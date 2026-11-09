//! REA 私有运行时的只读更新检查。
use super::*;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaUpdatePreview {
    pub current_version: String,
    pub skill_versions: Vec<ReaSkillVersion>,
    pub target_version: String,
    pub node_version: String,
    pub node_engines: String,
    pub node_compatible: bool,
    pub update_available: bool,
    pub prefix: PathBuf,
    pub clients: Vec<String>,
    pub client_targets: Vec<ReaClientTarget>,
    pub skill_paths: Vec<PathBuf>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaSkillVersion {
    pub client: String,
    pub path: PathBuf,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaClientTarget {
    pub client: String,
    pub path: PathBuf,
}

#[derive(Clone)]
struct ReaInstallation {
    prefix: PathBuf,
    node: PathBuf,
    entry: PathBuf,
    package: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReaProbe {
    node_version: String,
    skill_versions: Vec<ReaSkillVersion>,
    node_compatible: bool,
    update_available: bool,
    clients: Vec<String>,
    client_targets: Vec<ReaClientTarget>,
    skill_paths: Vec<PathBuf>,
    unresolved: Vec<String>,
}

/// 只接受 Codex 当前明确注册的私有 Node 和 REA 入口。
fn registered_installation(config: &str) -> anyhow::Result<ReaInstallation> {
    let document: toml::Value = config.parse().context("Codex 配置格式错误")?;
    let server = document.get("mcp_servers").and_then(|value| value.get("rea"))
        .and_then(toml::Value::as_table).context("未找到 Codex 中注册的 REA MCP 入口")?;
    anyhow::ensure!(server.get("enabled").and_then(toml::Value::as_bool) != Some(false),
        "REA MCP 入口已禁用");
    anyhow::ensure!(
        server.get("env").map_or(true, |env| env.as_table().is_some_and(|env| env.is_empty()))
            && server.get("env_vars").map_or(true, |env| env.as_array().is_some_and(|env| env.is_empty())),
        "REA MCP 使用了环境变量覆盖，无法可靠确定维护范围"
    );
    let node = PathBuf::from(server.get("command").and_then(toml::Value::as_str)
        .context("REA MCP 缺少 Node 路径")?);
    let args = server.get("args").and_then(toml::Value::as_array).context("REA MCP 缺少启动参数")?;
    anyhow::ensure!(args.len() == 2 && args[1].as_str() == Some("mcp"), "REA MCP 启动参数不受支持");
    let entry = PathBuf::from(args[0].as_str().context("REA MCP 入口无效")?);
    anyhow::ensure!(node.is_absolute() && entry.is_absolute(), "REA MCP 路径必须是绝对路径");
    let package = entry.parent().and_then(Path::parent).context("REA 入口路径无效")?;
    let modules = package.parent().context("REA 包路径无效")?;
    let prefix = modules.parent().context("REA 前缀路径无效")?;
    anyhow::ensure!(package.file_name().is_some_and(|name| name == "rea-agents")
        && modules.file_name().is_some_and(|name| name == "node_modules")
        && entry == package.join("scripts").join("rea.mjs"), "REA MCP 入口不是支持的私有包布局");
    #[cfg(windows)]
    let expected_node = modules.join("node").join("bin").join("node.exe");
    #[cfg(not(windows))]
    let expected_node = modules.join("node").join("bin").join("node");
    let canonical_prefix = prefix.canonicalize().context("无法读取 REA 前缀")?;
    let canonical_node = node.canonicalize().context("无法读取 REA Node")?;
    anyhow::ensure!(canonical_node == expected_node.canonicalize().context("无法读取私有 Node")?,
        "REA MCP 未使用同一前缀的私有 Node");
    anyhow::ensure!(canonical_node.starts_with(&canonical_prefix)
        && entry.canonicalize()?.starts_with(&canonical_prefix), "REA 入口位于私有前缀之外");
    Ok(ReaInstallation {
        prefix: prefix.to_path_buf(), node, entry: entry.clone(), package: package.to_path_buf(),
    })
}

impl SkillsManager {
    /// 检查注册入口、npm 最新版本与现有维护范围；不会安装、刷新或重启。
    pub async fn preview_rea_update(&self) -> anyhow::Result<ReaUpdatePreview> {
        let config = std::fs::read_to_string(self.codex_home.join("config.toml"))
            .context("无法读取 Codex 配置")?;
        let installation = registered_installation(&config)?;
        let package_path = installation.package.join("package.json");
        let installed: Value = serde_json::from_slice(&std::fs::read(&package_path)?)
            .context("REA 包元数据无效")?;
        anyhow::ensure!(installed.get("name").and_then(Value::as_str) == Some("rea-agents"),
            "已注册的 REA 包身份不匹配");
        let current = installed.get("version").and_then(Value::as_str).context("缺少 REA 已安装版本")?;
        let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
        let release: Value = client.get("https://registry.npmjs.org/rea-agents/latest")
            .send().await.context("无法查询 npm 最新 REA 版本")?
            .error_for_status()?.json().await.context("npm 返回的 REA 元数据无效")?;
        anyhow::ensure!(release.get("name").and_then(Value::as_str) == Some("rea-agents"),
            "npm 返回的包身份不匹配");
        let target = release.get("version").and_then(Value::as_str).context("npm 未返回精确版本")?;
        let engines = release.get("engines").and_then(|value| value.get("node"))
            .and_then(Value::as_str).context("npm 未提供 Node 兼容范围")?;
        let helper = installation.package.join("dist").join("application").join("UpdateMaintenance.js");
        anyhow::ensure!(helper.is_file(), "该 REA 版本不提供可核验的维护范围接口");
        let home_helper = installation.package.join("dist").join("config").join("homeDirectory.js");
        anyhow::ensure!(home_helper.is_file(), "该 REA 版本不提供可核验的用户目录接口");
        let expected_scope = serde_json::json!({
            "codex": self.codex_home,
            "shared": self.shared_skills_dir,
            "agents": self.agent_dirs.iter().map(|(id, path)| (id, path))
                .collect::<BTreeMap<_, _>>(),
        });
        let script = REA_PROBE_SCRIPT;
        let output = std::process::Command::new(&installation.node)
            .arg("-e").arg(script).arg(&package_path).arg(&helper).arg(&home_helper).arg(&installation.entry)
            .arg(current).arg(target).arg(engines)
            .arg(expected_scope.to_string())
            .output().context("无法运行已注册的 REA Node")?;
        anyhow::ensure!(output.status.success(), "REA 维护范围检查失败：{}",
            String::from_utf8_lossy(&output.stderr).trim());
        let probe: ReaProbe = serde_json::from_slice(&output.stdout)
            .context("REA 维护范围返回了无法识别的数据")?;
        let mut blockers = probe.unresolved;
        if !probe.node_compatible {
            blockers.push("当前私有 Node 不满足新版本的 engines 要求".into());
        }
        if probe.update_available {
            blockers.push("暂不支持自动更新：尚未实现保护现有 MCP 进程与私有 Node 的版本切换".into());
        }
        Ok(ReaUpdatePreview {
            current_version: current.into(), target_version: target.into(),
            skill_versions: probe.skill_versions,
            node_version: probe.node_version, node_engines: engines.into(),
            node_compatible: probe.node_compatible, update_available: probe.update_available,
            prefix: installation.prefix, clients: probe.clients, client_targets: probe.client_targets,
            skill_paths: probe.skill_paths, blockers,
        })
    }
}

const REA_PROBE_SCRIPT: &str = r#"
const {createRequire} = require("node:module");
const {pathToFileURL} = require("node:url");
const path = require("node:path");
const fs = require("node:fs");
const [pkg, helper, homeHelper, entry, current, target, engines, expectedJson] = process.argv.slice(1);
const load = createRequire(pkg);
const semver = load("semver");
if (semver.valid(current) !== current || semver.valid(target) !== target || !semver.validRange(engines))
  throw new Error("Invalid version metadata");
Promise.all([
  import(pathToFileURL(helper).href),
  import(pathToFileURL(homeHelper).href),
  import(pathToFileURL(path.join(path.dirname(helper), "SupportedClients.js")).href),
]).then(async ([maintenance, home, supported]) => {
  const expected = JSON.parse(expectedJson);
  const userHome = home.homeDirectoryFromEnvironment(process.env, process.platform);
  const normalize = value => {
    if (!value || !path.isAbsolute(value)) throw new Error("REA 维护范围包含非绝对路径");
    let resolved = path.resolve(value);
    const suffix = [];
    while (true) {
      try {
        resolved = path.join(fs.realpathSync.native(resolved), ...suffix.reverse());
        break;
      } catch (error) {
        if (error.code !== "ENOENT") throw error;
        const parent = path.dirname(resolved);
        if (parent === resolved) throw error;
        suffix.push(path.basename(resolved));
        resolved = parent;
      }
    }
    return process.platform === "win32" ? resolved.toLowerCase() : resolved;
  };
  const same = (left, right) => normalize(left) === normalize(right);
  const inside = (target, root) => {
    const relative = path.relative(normalize(root), normalize(target));
    return relative === "" || (!path.isAbsolute(relative) && relative !== ".." && !relative.startsWith(`..${path.sep}`));
  };
  if (!expected.shared || !same(expected.shared, path.join(userHome, ".agents", "skills")))
    throw new Error("REA 用户目录与管理器共享 Skill 目录不一致");
  const clients = supported.supportedClients(userHome, process.platform, process.env);
  const codex = clients.find(client => client.name === "codex");
  if (!codex || !same(codex.configPath, path.join(expected.codex, "config.toml")))
    throw new Error("REA Codex 目录与管理器不一致");
  for (const [id, reaId] of [["claude", "claude_code"], ["grok", "grok_build"]]) {
    if (!expected.agents[id]) continue;
    const client = clients.find(item => item.name === reaId);
    const root = reaId === "claude_code"
      ? supported.clientSkillDirectories(userHome, [reaId], process.env, process.platform).directories[0]?.directory
      : client && path.join(path.dirname(client.configPath), "skills");
    if (!root || !same(root, expected.agents[id]))
      throw new Error(`REA ${id} 目录与管理器不一致`);
  }
  const roots = [userHome, expected.codex, ...Object.values(expected.agents).map(value => path.dirname(value))];
  for (const client of clients) {
    for (const candidate of [client.configPath, ...(client.configPaths ?? []), client.markerPath, client.zcodeFallbackPath].filter(Boolean)) {
      if (!roots.some(root => inside(candidate, root)))
        throw new Error(`REA ${client.name} 路径超出管理器范围：${candidate}`);
    }
  }
  const selection = supported.clientSkillDirectories(userHome, undefined, process.env, process.platform);
  for (const destination of selection.directories) {
    if (!roots.some(root => inside(destination.directory, root)))
      throw new Error(`REA Skill 路径超出管理器范围：${destination.directory}`);
  }
  const scope = await maintenance.existingMaintenanceScope(userHome, entry, process.env);
  const unresolved = [
    ...(scope.unresolved ?? []).map(x => `${x.client}: ${x.config_path}`),
    ...(scope.skillIssues ?? []).map(x => `${x.client ?? "skill"}: ${x.path}`),
  ];
  const clientTargets = scope.clients.map(id => {
    const client = clients.find(candidate => candidate.name === id);
    if (!client || !client.configPath) throw new Error(`无法确定 REA ${id} 的配置路径`);
    return {client: id, path: client.configPath};
  });
  const skillVersions = (scope.skillDestinations ?? []).map(({client, path: root}) => {
    if (typeof client !== "string" || !client) throw new Error(`无法确定 REA Skill 归属：${root}`);
    const manifest = path.join(root, "SKILL.md");
    const text = fs.readFileSync(manifest, "utf8");
    const header = text.match(/^---\s*\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/);
    if (!header) throw new Error(`Skill frontmatter 无效：${manifest}`);
    const document = load("yaml").parseDocument(header[1]);
    if (document.errors.length) throw new Error(`Skill 元数据无效：${manifest}: ${document.errors.map(error => error.message).join("; ")}`);
    const version = document.toJS()?.metadata?.version;
    if (version !== undefined && version !== null && typeof version !== "string" && typeof version !== "number")
      throw new Error(`Skill 版本格式无效：${manifest}`);
    return {client, path: root, version: version == null ? null : String(version)};
  });
  process.stdout.write(JSON.stringify({
    nodeVersion: process.versions.node,
    skillVersions,
    nodeCompatible: semver.satisfies(process.versions.node, engines),
    updateAvailable: semver.gt(target, current),
    clients: scope.clients,
    clientTargets,
    skillPaths: (scope.skillDestinations ?? []).map(x => x.path),
    unresolved,
  }));
}).catch(error => { console.error(error.message); process.exitCode = 1; });
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(node: &Path, entry: &Path, extra: &str) -> String {
        format!("[mcp_servers.rea]\ncommand={}\nargs=[{},'mcp']\n{extra}\n",
            toml::Value::String(node.to_string_lossy().into_owned()),
            toml::Value::String(entry.to_string_lossy().into_owned()))
    }

    #[test]
    fn only_accepts_registered_private_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let modules = temp.path().join("node_modules");
        #[cfg(windows)]
        let node = modules.join("node/bin/node.exe");
        #[cfg(not(windows))]
        let node = modules.join("node/bin/node");
        let entry = modules.join("rea-agents/scripts/rea.mjs");
        std::fs::create_dir_all(node.parent().unwrap()).unwrap();
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(&node, "fixture node").unwrap();
        std::fs::write(&entry, "fixture installer").unwrap();
        let installed = registered_installation(&registration(&node, &entry, "")).unwrap();
        assert_eq!(installed.prefix, temp.path());
        assert_eq!(installed.node, node);
        assert_eq!(installed.entry, entry);
        assert!(registered_installation(&registration(&node, &entry, "enabled=false")).is_err());
        assert!(registered_installation(&registration(&node, &entry, "[mcp_servers.rea.env]\nHOME='other'")).is_err());
        assert!(registered_installation(&registration(&node, &entry, "env_vars=['HOME']")).is_err());
        assert!(registered_installation(&registration(&temp.path().join("other-node"), &entry, "")).is_err());
        assert!(registered_installation("").is_err());
        assert!(registered_installation("[mcp_servers.rea]\ncommand='node'\nargs=['rea','mcp']").is_err());
        assert!(registered_installation("[mcp_servers.rea]\ncommand='node'\nargs=['rea','setup']").is_err());
    }

    #[test]
    fn probe_rejects_mismatched_scope_and_reports_each_skill_version() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let package = root.join("package");
        let application = package.join("dist/application");
        let home = root.join("home");
        let shared = home.join(".agents/skills/reverse-engineer-anything");
        let claude = home.join(".claude/skills/reverse-engineer-anything");
        for dir in [&application, &shared, &claude, &package.join("node_modules/semver"), &package.join("node_modules/yaml")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(package.join("package.json"), r#"{"type":"module"}"#).unwrap();
        std::fs::write(package.join("node_modules/semver/index.js"),
            "exports.valid=x=>x;exports.validRange=x=>x;exports.satisfies=()=>true;exports.gt=()=>true;").unwrap();
        std::fs::write(package.join("node_modules/yaml/index.js"), r#"
exports.parseDocument = value => ({
  errors: value.includes("[invalid") ? [new Error("invalid yaml")] : [],
  toJS: () => ({metadata: {version: value.match(/version: (.+)/)?.[1]}}),
});
"#).unwrap();
        let helper = application.join("UpdateMaintenance.js");
        std::fs::write(&helper, r#"
import path from "node:path";
export const existingMaintenanceScope = async home => ({
  clients: ["codex"],
  skillDestinations: [".agents", ".claude"].map((name, index) => ({
    client: index === 0 ? "shared" : "claude_code",
    path: path.join(home, name, "skills", "reverse-engineer-anything"),
  })),
});
"#).unwrap();
        std::fs::write(application.join("SupportedClients.js"), r#"
import path from "node:path";
export const supportedClients = (home, platform, env) => [
  {name:"codex",configPath:path.join(env.CODEX_HOME,"config.toml")},
  {name:"claude_code",configPath:path.join(home,".claude.json")},
  {name:"grok_build",configPath:path.join(home,".grok","config.toml")},
];
export const clientSkillDirectories = home => ({directories:[
  {directory:path.join(home,".claude","skills")},
]});
"#).unwrap();
        let home_helper = package.join("home.js");
        std::fs::write(&home_helper, "export const homeDirectoryFromEnvironment = env => env.USERPROFILE;").unwrap();
        std::fs::write(shared.join(SKILL_MANIFEST_FILE), "---\nmetadata:\n  version: 49\n---\n").unwrap();
        std::fs::write(claude.join(SKILL_MANIFEST_FILE), "---\nmetadata:\n  version: 48\n---\n").unwrap();
        let expected = serde_json::json!({
            "codex": home.join(".codex"),
            "shared": home.join(".agents/skills"),
            "agents": {"claude": home.join(".claude/skills"), "grok": home.join(".grok/skills")},
        });
        let run = |scope: &Value| {
            let mut command = std::process::Command::new("node");
            command.env_clear();
            #[cfg(windows)]
            for name in ["SystemRoot", "WINDIR"] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
            command.env("USERPROFILE", &home)
                .env("HOME", &home)
                .env("APPDATA", home.join("AppData/Roaming"))
                .env("LOCALAPPDATA", home.join("AppData/Local"))
                .env("CODEX_HOME", home.join(".codex"))
                .arg("-e").arg(REA_PROBE_SCRIPT)
                .arg(package.join("package.json")).arg(&helper).arg(&home_helper)
                .arg(package.join("scripts/rea.mjs")).args(["1.0.0", "2.0.0", ">=22"])
                .arg(scope.to_string()).output().unwrap()
        };
        let result = run(&expected);
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        let probe: ReaProbe = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(probe.skill_versions.len(), 2);
        assert_eq!(probe.skill_versions[0].client, "shared");
        assert_eq!(probe.skill_versions[0].version.as_deref(), Some("49"));
        assert_eq!(probe.skill_versions[1].version.as_deref(), Some("48"));
        assert_eq!(probe.client_targets.len(), 1);
        assert_eq!(probe.client_targets[0].client, "codex");
        assert_eq!(probe.client_targets[0].path, home.join(".codex/config.toml"));

        let mut mismatched = expected.clone();
        mismatched["shared"] = serde_json::json!(root.join("other/skills"));
        let result = run(&mismatched);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("共享 Skill"));

        mismatched = expected.clone();
        mismatched["codex"] = serde_json::json!(root.join("other/.codex"));
        let result = run(&mismatched);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("Codex 目录"));

        mismatched = expected.clone();
        mismatched["agents"]["claude"] = serde_json::json!(root.join("other/.claude/skills"));
        let result = run(&mismatched);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("claude 目录"));

        std::fs::write(claude.join(SKILL_MANIFEST_FILE), "---\nmetadata: [invalid\n---\n").unwrap();
        let result = run(&expected);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("Skill 元数据无效"));
    }
}
