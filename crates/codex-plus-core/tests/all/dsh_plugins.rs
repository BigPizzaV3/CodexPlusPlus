use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use codex_plus_core::dsh_plugins::{self, DshPlugins, PluginSource, Toolchain};
use serde_json::{Value, json};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dsh")
        .join(name)
}

fn host_source(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/dsh-host")
        .join(name)
}

/// 宿主测试需要 Node；本机没有时跳过（CI 镜像与开发机都有）。
fn toolchain() -> Option<Toolchain> {
    let toolchain = Toolchain::detect();
    if toolchain.node.is_none() {
        eprintln!("跳过：没有找到 Node 运行时");
        return None;
    }
    Some(toolchain)
}

fn local_source(path: &Path) -> PluginSource {
    PluginSource::Local {
        path: path.to_string_lossy().into_owned(),
    }
}

fn row<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("报告里没有 {id} 行：{report}"))
}

/// 以 serve 模式启动宿主，依次发送请求，按 id 收集响应。
fn serve(plugins: &DshPlugins, toolchain: &Toolchain, requests: &[Value]) -> Vec<Value> {
    let host = plugins.ensure_host().unwrap();
    let mut child = Command::new(toolchain.node.as_ref().unwrap())
        .arg(host)
        .arg("serve")
        .arg(plugins.registry_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut responses = Vec::new();
    for request in requests {
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        if request.get("id").is_none() {
            continue;
        }
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        responses.push(serde_json::from_str::<Value>(&line).unwrap());
    }
    drop(stdin);
    let _ = child.wait();
    responses
}

fn call(id: u64, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
}

#[test]
fn dsh_parse_source_recognizes_local_npm_and_git() {
    assert_eq!(
        dsh_plugins::parse_source("/tmp/plugin").unwrap(),
        PluginSource::Local {
            path: "/tmp/plugin".to_string()
        }
    );
    assert_eq!(
        dsh_plugins::parse_source(" @deepseek-ai/dsh-tool-todo@0.2.0 ").unwrap(),
        PluginSource::Npm {
            spec: "@deepseek-ai/dsh-tool-todo@0.2.0".to_string()
        }
    );
    assert_eq!(
        dsh_plugins::parse_source("git+https://github.com/acme/tools.git#v1&path:packages/greet")
            .unwrap(),
        PluginSource::Git {
            url: "https://github.com/acme/tools.git".to_string(),
            reference: Some("v1".to_string()),
            path: Some("packages/greet".to_string()),
        }
    );
    assert!(matches!(
        dsh_plugins::parse_source("https://github.com/acme/tools").unwrap(),
        PluginSource::Git { reference: None, path: None, .. }
    ));
    // 选项注入、相对路径、越界子目录都拒绝。
    assert!(dsh_plugins::parse_source("--registry=evil").is_err());
    assert!(dsh_plugins::parse_source("./relative").is_err());
    assert!(dsh_plugins::parse_source("name with space").is_err());
    assert!(dsh_plugins::parse_source("https://x/y.git#path:../escape").is_err());
    assert!(dsh_plugins::parse_source("").is_err());
}

#[test]
fn dsh_plugin_id_from_package_name_is_filesystem_safe() {
    assert_eq!(
        dsh_plugins::plugin_id_from_name("@deepseek-ai/dsh-tool-todo"),
        "deepseek-ai__dsh-tool-todo"
    );
    assert_eq!(dsh_plugins::plugin_id_from_name("Greet Tools"), "greet-tools");
    assert_eq!(dsh_plugins::plugin_id_from_name("../.."), "plugin");
}

#[test]
fn dsh_managed_mcp_entry_points_at_codex_plus_flag() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("codex-plus");
    std::fs::write(&executable, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let config = codex_plus_core::managed_mcp::upsert_managed_context(
        &dsh_plugins::SPEC,
        "[mcp_servers.other]\ncommand = \"keep\"\n",
        &executable,
        directory.path(),
    )
    .unwrap();
    assert!(config.contains("[mcp_servers.other]"));
    assert!(config.contains("[mcp_servers.codex_plus_dsh]"));
    assert!(config.contains("--codex-plus-dsh-mcp"));
}

#[test]
fn dsh_import_bundle_reports_compatibility_per_row() {
    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let plugins = DshPlugins::new(state.path());
    let record = plugins
        .import(&local_source(&fixture("greet-bundle")), &toolchain)
        .unwrap();
    assert_eq!(record.id, "dsh-greet-bundle");
    assert!(!record.managed, "本地目录原地链接，不复制");
    let report = &record.report;
    assert_eq!(report["kind"], "bundle");
    assert_eq!(report["title"], "问候工具");

    let greet = row(report, "greet");
    assert_eq!(greet["status"], "compatible", "systemPrompt 经网关追加到 instructions，完整兼容：{greet}");
    assert_eq!(greet["mappings"], json!(["instructions"]));
    assert_eq!(greet["tools"][0]["name"], "greet");

    // cordis:group 的子行被展开；依赖子 agent 编排服务的行不加载。
    let session = row(report, "session-tool");
    assert_eq!(session["status"], "incompatible");
    assert!(session["reasons"][0].as_str().unwrap().contains("subagents"));
    assert_eq!(row(report, "official-todo")["status"], "incompatible");

    let mcp = dsh_plugins::mcp_servers_json(report).unwrap();
    assert_eq!(mcp["mcpServers"]["demo"]["command"], "npx");
    assert_eq!(mcp["mcpServers"]["demo"]["env"]["DEMO"], "1");

    let skills = dsh_plugins::report_skill_dirs(report);
    assert_eq!(skills.len(), 1);
    assert!(skills[0].join("SKILL.md").is_file());
    assert_eq!(dsh_plugins::report_tool_count(report), 1);

    let registry = plugins.load().unwrap();
    assert_eq!(registry.plugins.len(), 1);
}

#[test]
fn dsh_host_serves_imported_tools_over_mcp() {
    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let plugins = DshPlugins::new(state.path());
    plugins
        .import(&local_source(&fixture("greet-bundle")), &toolchain)
        .unwrap();
    let responses = serve(
        &plugins,
        &toolchain,
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            call(3, "greet", json!({"name": "Ada", "times": 2})),
            call(4, "greet", json!({"times": "x"})),
            call(5, "missing", json!({})),
        ],
    );
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "codex-plus-dsh");
    assert!(
        responses[0]["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("greet tool")
    );
    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["inputSchema"]["required"], json!(["name"]));
    assert_eq!(tools[0]["inputSchema"]["properties"]["times"]["type"], "integer");
    // bundle patch 的配置覆盖了 schema 默认值，未写的字段取默认值。
    assert_eq!(
        responses[2]["result"]["content"][0]["text"],
        "Hi there, Ada! Hi there, Ada!"
    );
    assert_eq!(responses[3]["result"]["isError"], true);
    assert!(
        responses[3]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("name is required")
    );
    assert_eq!(responses[4]["error"]["code"], -32602);
}

#[test]
fn dsh_row_config_and_toggles_apply_on_next_start() {
    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let plugins = DshPlugins::new(state.path());
    let record = plugins
        .import(&local_source(&fixture("greet-bundle")), &toolchain)
        .unwrap();

    plugins
        .set_row_config(&record.id, "greet", Some(json!({"greeting": "Yo"})))
        .unwrap();
    let responses = serve(&plugins, &toolchain, &[call(1, "greet", json!({"name": "Ada"}))]);
    assert_eq!(responses[0]["result"]["content"][0]["text"], "Yo, Ada!");

    // 非法配置在重新分析时报出来，而不是静默。
    plugins
        .set_row_config(&record.id, "greet", Some(json!({"greeting": 42})))
        .unwrap();
    let report = plugins.reinspect(&record.id, &toolchain).unwrap().report;
    assert_eq!(row(&report, "greet")["status"], "error");
    plugins.set_row_config(&record.id, "greet", None).unwrap();
    assert!(plugins.set_row_config(&record.id, "greet", Some(json!([1]))).is_err());

    plugins.set_row_enabled(&record.id, "greet", false).unwrap();
    let report = plugins.reinspect(&record.id, &toolchain).unwrap().report;
    assert_eq!(row(&report, "greet")["status"], "disabled");
    let responses = serve(&plugins, &toolchain, &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})]);
    assert_eq!(responses[0]["result"]["tools"], json!([]));

    plugins.set_row_enabled(&record.id, "greet", true).unwrap();
    plugins.set_enabled(&record.id, false).unwrap();
    let responses = serve(&plugins, &toolchain, &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})]);
    assert_eq!(responses[0]["result"]["tools"], json!([]), "整包停用后不加载");
}

#[test]
fn dsh_remove_local_plugin_keeps_source_directory() {
    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let plugins = DshPlugins::new(state.path());
    let record = plugins
        .import(&local_source(&fixture("greet-bundle")), &toolchain)
        .unwrap();
    plugins.remove(&record.id).unwrap();
    assert!(plugins.load().unwrap().plugins.is_empty());
    assert!(fixture("greet-bundle").join("package.json").is_file());
    assert!(plugins.remove(&record.id).is_err());
}

#[test]
fn dsh_plain_package_without_bundle_is_one_plugin_row() {
    let Some(toolchain) = toolchain() else { return };
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("package.json"),
        r#"{"name":"@acme/dsh-echo","version":"1.0.0","type":"module","exports":{".":{"types":"./x.d.ts","import":"./lib/index.js"}}}"#,
    )
    .unwrap();
    std::fs::create_dir(source.path().join("lib")).unwrap();
    std::fs::write(
        source.path().join("lib/index.js"),
        r#"import { defineTool } from '@deepseek-ai/dsh-tools'
export default function echo(ctx) {
  console.log('stdout 上的日志不能污染 MCP')
  ctx.tools.register(defineTool({
    name: 'echo',
    description: 'Echo the input.',
    parameters: { value: { type: 'json', required: true } },
    output: { schema: { type: 'json' }, render: (_args, value) => [{ type: 'text', text: JSON.stringify(value) }] },
    async execute(args, exec) { return { value: args.value, aborted: exec.signal.aborted } },
  }))
}
echo.inject = ['tools']
"#,
    )
    .unwrap();
    let state = tempfile::tempdir().unwrap();
    let plugins = DshPlugins::new(state.path());
    let record = plugins
        .import(&local_source(source.path()), &toolchain)
        .unwrap();
    assert_eq!(record.id, "acme__dsh-echo");
    assert_eq!(record.report["kind"], "plugin");
    assert_eq!(row(&record.report, "@acme/dsh-echo")["status"], "compatible");
    let responses = serve(&plugins, &toolchain, &[call(1, "echo", json!({"value": [1, {"a": true}]}))]);
    assert_eq!(
        responses[0]["result"]["content"][0]["text"],
        r#"{"value":[1,{"a":true}],"aborted":false}"#
    );
}

#[test]
fn dsh_service_class_and_client_plugins_are_reported_incompatible() {
    let Some(toolchain) = toolchain() else { return };
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("package.json"),
        r#"{"name":"dsh-store","type":"module","main":"index.js","dsh":{"client":{"platform":"web"}}}"#,
    )
    .unwrap();
    std::fs::write(
        source.path().join("index.js"),
        "import { Service } from '@deepseek-ai/cordis'\nexport default class Store extends Service {}\n",
    )
    .unwrap();
    let state = tempfile::tempdir().unwrap();
    let plugins = DshPlugins::new(state.path());
    let record = plugins
        .import(&local_source(source.path()), &toolchain)
        .unwrap();
    assert_eq!(record.report["client"], true);
    let store = row(&record.report, "dsh-store");
    assert_eq!(store["status"], "incompatible");
    assert_eq!(store["kind"], "service");
}

#[test]
fn dsh_patch_yaml_subset_parses_common_shapes() {
    let Some(toolchain) = toolchain() else { return };
    let yaml = host_source("yaml.mjs");
    let script = format!(
        r#"import {{ parseYaml }} from {url}
const doc = parseYaml(`# comment
- insert:
    - id: a   # trailing comment
      name: 'it''s'
      config:
        list: [1, "two", {{k: v}}]
        empty: {{}}
        flag: true
        nothing: ~
        url: http://127.0.0.1:3000/mcp
        expr: !!js ctx.foo
        text: |
          line one
          line two
    - id: b
      name: cordis:group
      config:
        - id: c
          name: "./c.js"
- remove:
  - x
`)
process.stdout.write(JSON.stringify(doc))"#,
        url = serde_json::to_string(&url_from_path(&yaml)).unwrap()
    );
    let output = Command::new(toolchain.node.as_ref().unwrap())
        .arg("--input-type=module")
        .arg("-e")
        .arg(script)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    let a = &doc[0]["insert"][0];
    assert_eq!(a["name"], "it's");
    assert_eq!(a["config"]["list"], json!([1, "two", {"k": "v"}]));
    assert_eq!(a["config"]["empty"], json!({}));
    assert_eq!(a["config"]["flag"], true);
    assert_eq!(a["config"]["nothing"], Value::Null);
    assert_eq!(a["config"]["url"], "http://127.0.0.1:3000/mcp");
    assert_eq!(a["config"]["expr"], json!({"tag": "!!js", "value": "ctx.foo"}));
    assert_eq!(a["config"]["text"], "line one\nline two\n");
    assert_eq!(doc[0]["insert"][1]["config"][0]["name"], "./c.js");
    assert_eq!(doc[1]["remove"], json!(["x"]));
}

fn url_from_path(path: &Path) -> String {
    let path = path.canonicalize().unwrap();
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

/// 联网：从 npm 导入真实发布的 dsh 工具插件，填配置后经 MCP 调用。
/// `cargo test -p codex-plus-core --test all dsh_plugins::dsh_npm_live -- --ignored`
#[test]
#[ignore = "需要网络与 npm"]
fn dsh_npm_live_import_runs_published_tool_plugin() {
    let Some(toolchain) = toolchain() else { return };
    assert!(toolchain.npm.is_some(), "需要 npm");
    let state = tempfile::tempdir().unwrap();
    let plugins = DshPlugins::new(state.path());
    let source = dsh_plugins::parse_source("@deepseek-ai/dsh-tool-workspace-dependencies").unwrap();
    let record = plugins.import(&source, &toolchain).unwrap();
    assert!(record.managed);
    assert!(Path::new(&record.root).join("node_modules/@deepseek-ai/schemastery").is_dir());
    let row_id = "@deepseek-ai/dsh-tool-workspace-dependencies";
    // 没有配置时，真实的 schemastery 会拒绝缺失的必填项。
    assert_eq!(row(&record.report, row_id)["status"], "error");

    let payload = tempfile::tempdir().unwrap();
    plugins
        .set_row_config(&record.id, row_id, Some(json!({"source": payload.path()})))
        .unwrap();
    let report = plugins.reinspect(&record.id, &toolchain).unwrap().report;
    assert_eq!(row(&report, row_id)["status"], "compatible", "{report}");
    let responses = serve(&plugins, &toolchain, &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})]);
    assert_eq!(responses[0]["result"]["tools"][0]["name"], "load_workspace_dependencies");

    // 重新导入同一个包视为更新：保留配置，旧目录进备份。
    let again = plugins.import(&source, &toolchain).unwrap();
    assert_eq!(again.configs.len(), 1);
    assert!(plugins.root().join("backups").read_dir().unwrap().next().is_some());
    plugins.remove(&record.id).unwrap();
    assert!(!Path::new(&record.root).exists());
}

/// 代码里注册的 skill（含依赖包里的）会被截下来写成 SKILL.md 目录；
/// 引用 dsh 运行时包的行判为不兼容，而不是「加载失败」。
#[test]
fn dsh_skills_registered_in_code_become_skill_directories() {
    let Some(toolchain) = toolchain() else { return };
    let source = tempfile::tempdir().unwrap();
    let root = source.path();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"dsh-skill-bundle","type":"module","dsh":{"bundle":{"patch":"./cordis.patch.yml"}}}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("cordis.patch.yml"),
        "- insert:\n    - id: badge\n      name: '@acme/dsh-skill-badge'\n    - id: llm-tool\n      name: ./llm.js\n",
    )
    .unwrap();
    std::fs::write(
        root.join("llm.js"),
        "import { LlmService } from '@deepseek-ai/dsh-llm'\nexport function apply() {}\n",
    )
    .unwrap();
    // skill 在依赖包里：只有代码注册，没有现成的 SKILL.md。
    let dependency = root.join("node_modules/@acme/dsh-skill-badge");
    std::fs::create_dir_all(dependency.join("assets")).unwrap();
    std::fs::write(
        dependency.join("package.json"),
        r#"{"name":"@acme/dsh-skill-badge","type":"module","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(dependency.join("assets/badge.md"), "Add the badge.").unwrap();
    std::fs::write(dependency.join("assets/badge.png"), "png").unwrap();
    std::fs::write(
        dependency.join("index.js"),
        r#"import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { BUNDLED_SKILL_RANK } from '@deepseek-ai/dsh-skill'
export const inject = ['skills']
const assets = fileURLToPath(new URL('./assets/', import.meta.url))
const candidate = {
  name: 'dsh-badge', description: 'Add the badge: use on every PR.', rank: BUNDLED_SKILL_RANK,
  invocation: { modelInvocable: true, userInvocable: true }, provider: 'badge', source: 'bundled',
  resourceBase: { kind: 'directory', path: assets }, locator: 'badge',
}
export function apply(ctx) {
  ctx.skills.registerProvider(() => ({
    name: 'badge',
    list: async () => [candidate],
    get: async (item) => ({ ...item, content: await readFile(new URL('./assets/badge.md', import.meta.url), 'utf8') }),
  }))
  ctx.skills.register({ name: 'inline-note', description: 'Inline skill', content: 'Inline body', source: 'runtime' })
}
"#,
    )
    .unwrap();

    let state = tempfile::tempdir().unwrap();
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(root), &toolchain).unwrap();
    let report = &record.report;

    let badge = row(report, "badge");
    assert_eq!(badge["status"], "skills", "{badge}");
    let llm = row(report, "llm-tool");
    assert_eq!(llm["status"], "incompatible", "{llm}");
    assert!(llm["reasons"][0].as_str().unwrap().contains("@deepseek-ai/dsh-llm"));

    let mut names: Vec<&str> = report["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| skill["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["dsh-badge", "inline-note"]);
    let dirs = dsh_plugins::report_skill_dirs(report);
    let badge_dir = dirs.iter().find(|dir| dir.ends_with("dsh-badge")).unwrap();
    let skill_md = std::fs::read_to_string(badge_dir.join("SKILL.md")).unwrap();
    assert!(skill_md.starts_with("---\nname: \"dsh-badge\"\ndescription: \"Add the badge: use on every PR.\"\n---"));
    assert!(skill_md.contains("Add the badge."));
    assert!(badge_dir.join("badge.png").is_file(), "资源目录的文件一并带上");
    assert!(dirs.iter().all(|dir| dir.starts_with(plugins.root())), "写在 Codex++ 自己的目录里");
    let (name, description) =
        codex_plus_core::skills::parse_skill_frontmatter(&skill_md, "fallback");
    assert_eq!((name.as_str(), description.as_str()), ("dsh-badge", "Add the badge: use on every PR."));
}

/// 行引用的依赖包里放着现成的 SKILL.md：代码加载不了也要扫到；
/// 软链到包外的依赖不扫；数量有上限。
#[test]
fn dsh_skill_files_in_referenced_dependencies_are_scanned_with_limits() {
    let Some(toolchain) = toolchain() else { return };
    let source = tempfile::tempdir().unwrap();
    let root = source.path();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"dsh-memory","type":"module","dsh":{"bundle":{"patch":"./cordis.patch.yml"}}}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("cordis.patch.yml"),
        "- insert:\n    - id: memory\n      name: '@acme/dsh-memory-skills'\n    - id: outside\n      name: '@acme/linked'\n    - id: many\n      name: '@acme/many'\n",
    )
    .unwrap();
    let write_package = |directory: &Path, name: &str, code: &str| {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(
            directory.join("package.json"),
            format!(r#"{{"name":"{name}","type":"module","main":"index.js"}}"#),
        )
        .unwrap();
        std::fs::write(directory.join("index.js"), code).unwrap();
    };
    let write_skill = |directory: &Path, name: &str| {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name} skill\n---\n\nBody\n"),
        )
        .unwrap();
    };
    // 代码引用 dsh 运行时包、加载不了，但 skill 文件就在包里。
    let memory = root.join("node_modules/@acme/dsh-memory-skills");
    write_package(
        &memory,
        "@acme/dsh-memory-skills",
        "import '@deepseek-ai/dsh-skill-filesystem'\nexport function apply() {}\n",
    );
    write_skill(&memory.join("skills/recall"), "recall");
    write_skill(&memory.join("skills/remember"), "remember");
    // 软链到包外的依赖：不扫。
    let outside = tempfile::tempdir().unwrap();
    write_package(outside.path(), "@acme/linked", "export function apply() {}\n");
    write_skill(&outside.path().join("skills/escaped"), "escaped");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), root.join("node_modules/@acme/linked")).unwrap();
    // 超过上限的只列前面一部分。
    let many = root.join("node_modules/@acme/many");
    write_package(&many, "@acme/many", "export function apply() {}\n");
    for index in 0..70 {
        write_skill(&many.join(format!("skills/s{index:02}")), &format!("s{index:02}"));
    }

    let state = tempfile::tempdir().unwrap();
    let plugins = DshPlugins::new(state.path());
    let report = plugins.import(&local_source(root), &toolchain).unwrap().report;

    assert_eq!(row(&report, "memory")["status"], "incompatible", "代码加载不了只是不兼容");
    let names: Vec<&str> = report["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| skill["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"recall") && names.contains(&"remember"), "{names:?}");
    assert_eq!(names.len(), 64, "数量上限");
    let diagnostics = report["diagnostics"].to_string();
    assert!(diagnostics.contains("只列出前 64 个"), "{diagnostics}");
    #[cfg(unix)]
    {
        assert!(!names.contains(&"escaped"), "软链到包外的依赖不扫");
        assert!(diagnostics.contains("指向包外"), "{diagnostics}");
    }
}

/// 系统提示词与能对上 Codex 钩子的 dsh 事件：导入后生成配套拓展包，经 Codex++ 的钩子分发与网关中间件
/// 跑通插件的监听器（拦截、补上下文、拒绝输入、要求继续），并随插件启停、配置与移除同步。
#[tokio::test]
async fn dsh_events_and_system_prompt_run_through_companion_extension() {
    use codex_plus_core::extension_gateway::apply_request_with;
    use codex_plus_core::extension_hooks::dispatch;
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        scripts.path().join("builtin"),
        scripts.path().join("user"),
        scripts.path().join("user_scripts.json"),
    );
    let plugins = DshPlugins::new(state.path());
    let record = plugins
        .import(&local_source(&fixture("policy-plugin")), &toolchain)
        .unwrap();
    let main = row(&record.report, "dsh-policy-demo");
    assert_eq!(main["status"], "compatible", "{main}");
    let mut mappings: Vec<&str> = main["mappings"].as_array().unwrap().iter().map(|item| item.as_str().unwrap()).collect();
    mappings.sort();
    assert_eq!(mappings, ["PostToolUse", "PreToolUse", "Stop", "UserPromptSubmit", "gateway:step", "instructions"]);

    let companion = plugins.sync_companion(&manager, &record.id).unwrap();
    assert_eq!(companion["id"], "dsh-dsh-policy-demo");
    assert_eq!(companion["manifestError"], Value::Null, "{companion}");
    assert_eq!(companion["staticPrompt"], true);
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(scripts.path().join("user/dsh-dsh-policy-demo/codexpp.json")).unwrap(),
    )
    .unwrap();
    assert!(manifest["hooks"]["PreToolUse"].is_array() && manifest["hooks"]["Stop"].is_array(), "{manifest}");
    assert!(manifest["gateway"]["request"].is_array(), "{manifest}");
    assert_eq!(manifest["renderer"], json!([]), "只有钩子与网关，不往页面注入脚本");
    let hooks = manager.enabled_extension_hooks().unwrap();
    assert_eq!(hooks.len(), 4, "四个事件各一条钩子");

    // PreToolUse：dsh 的 deny → Codex 的 permissionDecision deny；放行时不表态。
    let denied = dispatch(&manager, "PreToolUse", json!({"tool_name": "Bash", "tool_input": {"command": "rm -rf build"}})).await.unwrap();
    assert_eq!(denied["hookSpecificOutput"]["permissionDecision"], "deny", "{denied}");
    assert!(denied["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().contains("rm -rf is blocked"));
    let allowed = dispatch(&manager, "PreToolUse", json!({"tool_name": "Bash", "tool_input": {"command": "ls"}})).await.unwrap();
    assert_eq!(allowed, json!({}));
    // PostToolUse：dsh 的 block → decision block + reason。
    let blocked = dispatch(&manager, "PostToolUse", json!({"tool_name": "Bash", "tool_input": {"command": "cat .env"}, "tool_response": "SECRET=1"})).await.unwrap();
    assert_eq!(blocked["decision"], "block", "{blocked}");
    assert!(blocked["reason"].as_str().unwrap().contains("withheld"));
    // 插件的内存状态跨钩子进程保留（常驻进程），并按会话区分 agent。
    let ls = json!({"session_id": "s1", "tool_name": "Bash", "tool_input": {"command": "ls"}, "tool_response": "a"});
    assert_eq!(dispatch(&manager, "PostToolUse", ls.clone()).await.unwrap(), json!({}));
    let repeated = dispatch(&manager, "PostToolUse", ls.clone()).await.unwrap();
    assert_eq!(repeated["hookSpecificOutput"]["additionalContext"], "policy-demo: \"ls\" repeated 2 times", "{repeated}");
    let other_session = dispatch(&manager, "PostToolUse", json!({"session_id": "s2", "tool_name": "Bash", "tool_input": {"command": "ls"}, "tool_response": "a"})).await.unwrap();
    assert_eq!(other_session, json!({}), "别的会话是另一个 agent");
    // UserPromptSubmit：reject → block；新增消息 → additionalContext。
    let rejected = dispatch(&manager, "UserPromptSubmit", json!({"prompt": "please forbidden-word"})).await.unwrap();
    assert_eq!(rejected["decision"], "block", "{rejected}");
    let enriched = dispatch(&manager, "UserPromptSubmit", json!({"prompt": "hello"})).await.unwrap();
    assert_eq!(enriched["hookSpecificOutput"]["additionalContext"], "policy-demo: remember the house rules", "{enriched}");
    // Stop：steer → block；已经多走过一轮（stop_hook_active）时不再拦。
    let stop = dispatch(&manager, "Stop", json!({"stop_hook_active": false})).await.unwrap();
    assert_eq!(stop["decision"], "block", "{stop}");
    assert!(stop["reason"].as_str().unwrap().contains("summarize"));
    let again = dispatch(&manager, "Stop", json!({"stop_hook_active": true})).await.unwrap();
    assert_eq!(again, json!({}));

    // 网关：静态提示词追加到 instructions，重复请求不重复追加。
    let mut body = json!({"model": "gpt-x", "instructions": "base", "input": []});
    apply_request_with(&manager, &mut body, "/v1/responses").await;
    assert_eq!(body["instructions"], "base\n\nNever run destructive shell commands without asking.");
    apply_request_with(&manager, &mut body, "/v1/responses").await;
    assert_eq!(body["instructions"], "base\n\nNever run destructive shell commands without asking.");

    // tools/post-execute 的 block 钩子里只能拦下，模型看到的工具结果在下一次请求里换成插件给的反馈；
    // 工具调用之后的每一步也跑 agent/pre-step（这个插件每步补一句提醒）。
    let secret = json!({"session_id": "s3", "tool_name": "Bash", "tool_input": {"command": "cat .env"}, "tool_use_id": "call-secret", "tool_response": "SECRET=1"});
    assert_eq!(dispatch(&manager, "PostToolUse", secret).await.unwrap()["decision"], "block");
    let mut body = json!({"model": "gpt-x", "instructions": "base", "prompt_cache_key": "s3", "input": [
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "read env"}]},
        {"type": "function_call", "call_id": "call-secret", "name": "shell", "arguments": "{\"command\":\"cat .env\"}"},
        {"type": "function_call_output", "call_id": "call-secret", "output": "SECRET=1"},
    ]});
    apply_request_with(&manager, &mut body, "/v1/responses").await;
    assert!(body["input"][2]["output"].as_str().unwrap().contains("withheld"), "{body}");
    assert_eq!(body["input"][3]["content"][0]["text"], "policy-demo: remember the house rules", "{body}");
    // 同一条结果在后续请求里（没经过钩子的新进程也一样）照旧换掉，不再追加重复的提醒以外的东西。
    let mut again = json!({"model": "gpt-x", "instructions": "base", "prompt_cache_key": "s3", "input": body["input"].as_array().unwrap()[..3].to_vec()});
    again["input"][2]["output"] = json!("SECRET=1");
    apply_request_with(&manager, &mut again, "/v1/responses").await;
    assert!(again["input"][2]["output"].as_str().unwrap().contains("withheld"), "{again}");

    // 动态片段：每次请求加载插件求值，变量按请求的模型取值。
    plugins
        .set_row_config(&record.id, "dsh-policy-demo", Some(json!({"dynamicPrompt": true, "requireSummary": false})))
        .unwrap();
    plugins.reinspect(&record.id, &toolchain).unwrap();
    let companion = plugins.sync_companion(&manager, &record.id).unwrap();
    assert_eq!(companion["staticPrompt"], false, "{companion}");
    let mut body = json!({"model": "gpt-dyn", "instructions": "base"});
    apply_request_with(&manager, &mut body, "/v1/responses").await;
    assert_eq!(body["instructions"], "base\n\nNever run destructive shell commands without asking. (model: gpt-dyn)");
    let stop = dispatch(&manager, "Stop", json!({"stop_hook_active": false})).await.unwrap();
    assert_eq!(stop, json!({}), "改了配置后按新配置运行");

    // 停用插件：配套拓展包一并停用，钩子与网关都不再生效。
    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
    assert!(manager.enabled_extension_hooks().unwrap().is_empty());
    let mut body = json!({"instructions": "base"});
    apply_request_with(&manager, &mut body, "/v1/responses").await;
    assert_eq!(body["instructions"], "base");

    // 移除：配套拓展包移入备份，不留在拓展目录里。
    plugins.remove(&record.id).unwrap();
    assert!(plugins.remove_companion(&manager, &record.id).unwrap());
    assert!(!scripts.path().join("user/dsh-dsh-policy-demo").exists());
    assert!(std::fs::read_dir(plugins.root().join("backups")).unwrap().any(|entry| {
        entry.unwrap().file_name().to_string_lossy().starts_with("companion-dsh-dsh-policy-demo")
    }));
}

/// 依赖 sessions / agents / sessionProjections / commands 的插件：钩子事件喂进会话镜像，投影随之前进；
/// agent/created 对上 SessionStart，钩子之外排下的上下文在下一次钩子带上；agent/request 经网关改推理强度；
/// 斜杠命令生成页面脚本与后台，后台经 bridge 转给插件。
#[tokio::test]
async fn dsh_session_services_run_on_codex_session_mirror() {
    use codex_plus_core::extension_gateway::apply_request_with;
    use codex_plus_core::extension_hooks::dispatch;
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        scripts.path().join("builtin"),
        scripts.path().join("user"),
        scripts.path().join("user_scripts.json"),
    );
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("session-plugin")), &toolchain).unwrap();
    let main = row(&record.report, "dsh-session-demo");
    assert_eq!(main["status"], "partial", "镜像服务有说明，按部分兼容：{main}");
    let plan = dsh_plugins::report_bridge(&record.report);
    assert!(plan.gateway_step && !plan.instructions, "{plan:?}");
    assert_eq!(plan.commands.len(), 1);
    for hook in ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"] {
        assert!(plan.hooks.iter().any(|item| item == hook), "会话镜像要 {hook}：{plan:?}");
    }

    let companion = plugins.sync_companion(&manager, &record.id).unwrap();
    assert_eq!(companion["manifestError"], Value::Null, "{companion}");
    let dir = scripts.path().join("user/dsh-dsh-session-demo");
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("codexpp.json")).unwrap()).unwrap();
    assert_eq!(manifest["renderer"], json!(["renderer.js"]), "{manifest}");
    assert_eq!(manifest["backend"]["script"], "backend/main.mjs", "{manifest}");
    assert!(std::fs::read_to_string(dir.join("renderer.js")).unwrap().contains("\"calls\""));

    let session = |extra: Value| {
        let mut payload = json!({"session_id": "mirror-1", "cwd": "/tmp"});
        payload.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        payload
    };
    let start = dispatch(&manager, "SessionStart", session(json!({"source": "startup"}))).await.unwrap();
    assert_eq!(start["hookSpecificOutput"]["additionalContext"], "session-demo: hello mirror-1 (startup)", "{start}");
    dispatch(&manager, "UserPromptSubmit", session(json!({"prompt": "hi"}))).await.unwrap();
    for index in 0..3 {
        let call = session(json!({"tool_name": "Bash", "tool_input": {"command": "ls"}, "tool_use_id": format!("c{index}"), "tool_response": "ok"}));
        dispatch(&manager, "PreToolUse", call.clone()).await.unwrap();
        dispatch(&manager, "PostToolUse", call).await.unwrap();
    }
    // 第三次工具结果后排下的上下文（钩子之外的 inject）随下一次钩子带上。
    let next = dispatch(&manager, "PreToolUse", session(json!({"tool_name": "Bash", "tool_input": {"command": "pwd"}, "tool_use_id": "c3"}))).await.unwrap();
    assert_eq!(next["hookSpecificOutput"]["additionalContext"], "session-demo: 3 tool calls so far", "{next}");

    // 网关：agent/request 把推理强度改成 high。
    let mut body = json!({"model": "gpt-x", "prompt_cache_key": "mirror-1", "reasoning": {"effort": "low"}, "input": []});
    apply_request_with(&manager, &mut body, "/v1/responses").await;
    assert_eq!(body["reasoning"]["effort"], "high", "{body}");

    // 斜杠命令：后台脚本经 bridge 转给常驻进程，投影读的是镜像的状态。
    let backend = dir.join("backend/main.mjs");
    let script = format!(
        "const {{ default: create }} = await import({}); const methods = create(); process.stdout.write(JSON.stringify(await methods.command({{ name: 'calls', rawInput: '', sessionId: 'mirror-1' }}))); process.exit(0)",
        json!(url_of(&backend))
    );
    let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    assert_eq!(result["kind"], "success", "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(result["text"], "calls=4 turns=1 events=9", "{result}");

    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
    assert!(manager.enabled_extension_hooks().unwrap().is_empty());
}

/// 带前端半边（dsh.client）的插件：配套拓展包的页面脚本夹着插件的 client.js 与胶水，
/// 前端发往 /api 的请求经后台 → bridge → 常驻进程交给插件注册的路由。
#[test]
fn dsh_client_plugin_gets_renderer_glue_and_routed_fetch() {
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        scripts.path().join("builtin"),
        scripts.path().join("user"),
        scripts.path().join("user_scripts.json"),
    );
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("client-plugin")), &toolchain).unwrap();
    let plan = dsh_plugins::report_bridge(&record.report);
    assert!(plan.connection && plan.client.is_some(), "{plan:?}");

    let companion = plugins.sync_companion(&manager, &record.id).unwrap();
    assert_eq!(companion["manifestError"], Value::Null, "{companion}");
    let dir = scripts.path().join("user/dsh-dsh-client-demo");
    let renderer = std::fs::read_to_string(dir.join("renderer.js")).unwrap();
    assert!(renderer.contains("window.__ModuleLoader__.load({"), "插件的前端入口原样夹在里面");
    assert!(renderer.contains("codexPlus.dshClient(ctx, modules"), "交给页面的 dsh 前端运行时");
    assert!(renderer.contains("ctx.backend.call('fetch'"), "/api 请求经后台转发");

    let backend = dir.join("backend/main.mjs");
    let script = format!(
        "const {{ default: create }} = await import({}); const methods = create(); const reply = await methods.fetch({{ path: '/api/client-demo/echo', method: 'POST', headers: {{ 'content-type': 'application/json' }}, body: JSON.stringify({{ text: 'ping' }}) }}); const missing = await methods.fetch({{ path: '/api/nope' }}); process.stdout.write(JSON.stringify({{ reply, missing: missing.status }})); process.exit(0)",
        json!(url_of(&backend))
    );
    let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    assert_eq!(result["reply"]["status"], 200, "{}", String::from_utf8_lossy(&output.stderr));
    let body: Value = serde_json::from_str(result["reply"]["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["echo"], "ping");
    assert_eq!(result["missing"], 404);

    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
}

/// llm/stream 改写回复流（文字增量与收尾里的完整文本都换成插件的）、agent/assistant-stream 观察到每一帧。
#[tokio::test]
async fn dsh_stream_events_rewrite_the_response_stream() {
    use codex_plus_core::extension_gateway::ResponseFilter;
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        scripts.path().join("builtin"),
        scripts.path().join("user"),
        scripts.path().join("user_scripts.json"),
    );
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("stream-plugin")), &toolchain).unwrap();
    let plan = dsh_plugins::report_bridge(&record.report);
    assert!(plan.gateway_response, "{plan:?}");
    let companion = plugins.sync_companion(&manager, &record.id).unwrap();
    assert_eq!(companion["manifestError"], Value::Null, "{companion}");

    let events = [
        json!({"type": "response.created", "response": {"id": "r1"}}),
        json!({"type": "response.content_part.added", "item_id": "m1", "output_index": 0, "content_index": 0, "part": {"type": "output_text", "text": ""}}),
        json!({"type": "response.output_text.delta", "item_id": "m1", "output_index": 0, "content_index": 0, "delta": "hello "}),
        json!({"type": "response.output_text.delta", "item_id": "m1", "output_index": 0, "content_index": 0, "delta": "world"}),
        json!({"type": "response.output_text.done", "item_id": "m1", "output_index": 0, "content_index": 0, "text": "hello world"}),
        json!({"type": "response.output_item.done", "output_index": 0, "item": {"type": "message", "id": "m1", "content": [{"type": "output_text", "text": "hello world"}]}}),
        json!({"type": "response.completed", "response": {"id": "r1", "output": [{"type": "message", "id": "m1", "content": [{"type": "output_text", "text": "hello world"}]}], "usage": {"input_tokens": 3, "output_tokens": 2}}}),
    ];
    let bytes: Vec<u8> = events.iter().map(|event| format!("event: {}\ndata: {}\n\n", event["type"].as_str().unwrap(), event)).collect::<String>().into_bytes();
    let middlewares = manager.enabled_gateway_response_middlewares().unwrap();
    let request = json!({"model": "gpt", "prompt_cache_key": "stream-1", "input": []});
    let mut filter = ResponseFilter::start_with(&middlewares, "/v1/responses", Some(&request)).await.expect("应建立会话");
    let mut out = filter.push(&bytes).await;
    out.extend(filter.finish().await);
    let text = String::from_utf8_lossy(&out).to_string();
    let parsed: Vec<Value> = text.split("\n\n").filter_map(|block| block.lines().find_map(|line| line.strip_prefix("data: "))).filter_map(|data| serde_json::from_str(data).ok()).collect();
    let deltas: String = parsed.iter().filter(|event| event["type"] == "response.output_text.delta").map(|event| event["delta"].as_str().unwrap().to_string()).collect();
    assert_eq!(deltas, "HELLO WORLD", "{text}");
    let done = parsed.iter().find(|event| event["type"] == "response.output_text.done").unwrap();
    assert_eq!(done["text"], "HELLO WORLD");
    let completed = parsed.iter().find(|event| event["type"] == "response.completed").unwrap();
    assert_eq!(completed["response"]["output"][0]["content"][0]["text"], "HELLO WORLD");
    let order: Vec<&str> = parsed.iter().map(|event| event["type"].as_str().unwrap()).collect();
    assert_eq!(order.last(), Some(&"response.completed"), "{order:?}");

    // 观察：常驻进程里的插件收到了 start / chunk / end 帧。
    let dir = scripts.path().join("user/dsh-dsh-stream-demo");
    let script = format!(
        "const {{ default: create }} = await import({}); process.stdout.write(JSON.stringify(await create().command({{ name: 'frames', rawInput: '' }}))); process.exit(0)",
        json!(url_of(&dir.join("backend/main.mjs")))
    );
    let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    let frames: Value = serde_json::from_str(result["text"].as_str().unwrap_or("{}")).unwrap_or(Value::Null);
    assert_eq!(frames["start"], 1, "{result} {}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(frames["end"], 1);
    assert!(frames["chunk"].as_u64().unwrap() >= 4, "{frames}");

    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
}

/// schedule / webhookRuntime / jobs / workspaceRegistry：工具里建的定时任务落在插件数据目录，配套拓展包的后台
/// 每分钟 tick；webhook 规则经后台的 HTTP 路由触发；jobs 在进程里跑完；定时规则的匹配按时区算。
#[test]
fn dsh_schedule_webhook_and_jobs_services() {
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        scripts.path().join("builtin"),
        scripts.path().join("user"),
        scripts.path().join("user_scripts.json"),
    );
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("schedule-plugin")), &toolchain).unwrap();
    let plan = dsh_plugins::report_bridge(&record.report);
    assert!(plan.schedule && plan.webhook, "{plan:?}");
    let companion = plugins.sync_companion(&manager, &record.id).unwrap();
    assert_eq!(companion["manifestError"], Value::Null, "{companion}");
    let dir = scripts.path().join("user/dsh-dsh-schedule-demo");
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("codexpp.json")).unwrap()).unwrap();
    assert_eq!(manifest["schedules"], json!([{ "method": "tick", "everyMinutes": 1 }]), "{manifest}");
    assert_eq!(manifest["backend"]["http"], true);

    // 工具经 MCP 调用：建一个每分钟的定时任务、跑一个后台任务。
    let responses = serve(&plugins, &toolchain, &[
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        call(2, "remind_me", json!({"session": "s-sched", "prompt": "看看构建"})),
    ]);
    let value: Value = serde_json::from_str(responses[1]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(value, json!({ "schedule": "every", "listed": 1, "job": "completed", "workspaces": true }), "{responses:?}");
    let stored = std::fs::read_to_string(plugins.root().join("data").join(&record.id).join("schedules.json")).unwrap();
    assert!(stored.contains("看看构建"), "{stored}");

    // 后台：tick 还没到点不触发；webhook 交给规则（没有 Codex++ 宿主时新建会话失败，记成 error）。
    let script = format!(
        "const {{ default: create }} = await import({}); const m = create(); const ticked = await m.tick(); const hook = await m.http({{ method: 'POST', path: '/webhook/demo/push', headers: {{}}, body: JSON.stringify({{ action: 'go', ref: 'main' }}) }}); const other = await m.http({{ method: 'POST', path: '/webhook/nope', headers: {{}}, body: '{{}}' }}); process.stdout.write(JSON.stringify({{ ticked, hook, other }})); process.exit(0)",
        json!(url_of(&dir.join("backend/main.mjs")))
    );
    let output = Command::new(toolchain.node.as_ref().unwrap()).env_remove("CODEX_PLUS_HOST_EXE").args(["--input-type=module", "-e", &script]).output().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    assert_eq!(result["ticked"], json!([]), "{} {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(result["hook"]["status"], 202, "{result}");
    assert_eq!(result["hook"]["json"]["started"][0]["rule"], "demo-rule");
    assert!(result["hook"]["json"]["started"][0]["error"].as_str().unwrap().contains("Codex++"), "{result}");
    assert_eq!(result["other"]["status"], 404);

    // 规则匹配：daily / weekly / cron 按时区，every 按上次触发时间。
    let services = url_of(&host_source("services.mjs"));
    let script = format!(
        "const {{ scheduleDue }} = await import({services:?}); const at = new Date('2026-11-02T01:30:10Z'); const out = [
          scheduleDue({{ kind: 'daily', hour: 9, minute: 30, timeZone: 'Asia/Shanghai' }}, at),
          scheduleDue({{ kind: 'daily', hour: 9, minute: 31, timeZone: 'Asia/Shanghai' }}, at),
          scheduleDue({{ kind: 'weekly', hour: 9, minute: 30, timeZone: 'Asia/Shanghai', weekdays: ['mon'] }}, at),
          scheduleDue({{ kind: 'weekly', hour: 9, minute: 30, timeZone: 'Asia/Shanghai', weekdays: ['tue'] }}, at),
          scheduleDue({{ kind: 'cron', expression: '*/15 9 * * 1-5', timeZone: 'Asia/Shanghai' }}, at),
          scheduleDue({{ kind: 'cron', expression: '0 9 * * *', timeZone: 'Asia/Shanghai' }}, at),
          scheduleDue({{ kind: 'every', everySeconds: 60, scheduledAt: at.getTime() - 61000 }}, at),
          scheduleDue({{ kind: 'every', everySeconds: 60, scheduledAt: at.getTime() - 10000 }}, at),
          scheduleDue({{ kind: 'after', dueAt: at.getTime() - 1, lastFiredAt: 0 }}, at),
        ]; process.stdout.write(JSON.stringify(out)); process.exit(0)"
    );
    let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
    let due: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    assert_eq!(due, json!([true, false, true, false, true, false, true, false, true]), "{}", String::from_utf8_lossy(&output.stderr));

    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
}

/// 拓展包清单里的 `dsh` 指向包内一个按 dsh 写法写的插件：随拓展启用导入、停用停用、删除移除。
#[test]
fn extension_package_can_carry_a_dsh_plugin() {
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        scripts.path().join("builtin"),
        scripts.path().join("user"),
        scripts.path().join("user_scripts.json"),
    );
    let package = scripts.path().join("user/kit");
    std::fs::create_dir_all(package.join("dsh")).unwrap();
    for name in ["package.json", "index.js"] {
        std::fs::copy(fixture("policy-plugin").join(name), package.join("dsh").join(name)).unwrap();
    }
    std::fs::write(package.join("codexpp.json"), json!({ "id": "kit", "name": "套件", "version": "1.0.0", "dsh": "dsh" }).to_string()).unwrap();
    let inventory = manager.inventory().unwrap();
    let entry = inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == "user:kit").cloned().unwrap();
    assert_eq!(entry["error"], "", "{entry}");
    assert_eq!(entry["package"]["dsh"], true);

    let plugins = DshPlugins::new(state.path());
    let report = plugins.sync_extension_plugins(&manager, &toolchain).unwrap();
    assert_eq!(report["changed"], json!(["ext-kit"]), "{report}");
    let record = plugins.load().unwrap().plugins.into_iter().find(|plugin| plugin.id == "ext-kit").unwrap();
    assert_eq!(record.owner.as_deref(), Some("user:kit"));
    assert!(scripts.path().join("user/dsh-ext-kit/codexpp.json").is_file(), "配套拓展包生成了");
    assert!(!manager.enabled_extension_hooks().unwrap().is_empty());
    // 没变化时不重新分析。
    assert_eq!(plugins.sync_extension_plugins(&manager, &toolchain).unwrap()["changed"], json!([]));

    manager.set_script_enabled("user:kit", false).unwrap();
    plugins.sync_extension_plugins(&manager, &toolchain).unwrap();
    assert!(!plugins.load().unwrap().plugins.iter().find(|plugin| plugin.id == "ext-kit").unwrap().enabled);
    assert!(manager.enabled_extension_hooks().unwrap().is_empty(), "配套拓展包随之停用");

    std::fs::remove_file(package.join("codexpp.json")).unwrap();
    plugins.sync_extension_plugins(&manager, &toolchain).unwrap();
    assert!(plugins.load().unwrap().plugins.iter().all(|plugin| plugin.id != "ext-kit"));
    assert!(!scripts.path().join("user/dsh-ext-kit").exists());
    assert!(package.join("dsh/index.js").is_file(), "插件源码原地不动");
}

fn url_of(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') { format!("file://{text}") } else { format!("file:///{text}") }
}

/// 只注册工具的插件不生成配套拓展包；替身缺的导出判为不兼容而不是加载失败。
#[test]
fn dsh_tool_only_plugins_need_no_companion() {
    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = codex_plus_core::user_scripts::UserScriptManager::new(
        scripts.path().join("builtin"),
        scripts.path().join("user"),
        scripts.path().join("user_scripts.json"),
    );
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("greet-bundle")), &toolchain).unwrap();
    let plan = dsh_plugins::report_bridge(&record.report);
    // greet-bundle 只有工具与静态提示词片段：提示词仍要追加，但没有钩子。
    assert!(plan.hooks.is_empty(), "{plan:?}");
    assert!(plan.instructions);
    let companion = plugins.sync_companion(&manager, &record.id).unwrap();
    assert!(companion["hooks"].as_array().unwrap().is_empty());
    assert!(manager.enabled_extension_hooks().unwrap().is_empty());
}
