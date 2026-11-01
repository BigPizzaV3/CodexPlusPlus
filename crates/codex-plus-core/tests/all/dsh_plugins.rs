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
            spec: "@deepseek-ai/dsh-tool-todo@0.2.0".to_string(),
            registry: None,
        }
    );
    // 自定义 registry：`#registry:<http(s) 地址>`；别的写法拒绝，不能借它注入 npm 选项。
    assert_eq!(
        dsh_plugins::parse_source("@acme/dsh-x@1.0.0#registry:https://npm.acme.dev/").unwrap(),
        PluginSource::Npm {
            spec: "@acme/dsh-x@1.0.0".to_string(),
            registry: Some("https://npm.acme.dev/".to_string()),
        }
    );
    assert!(dsh_plugins::parse_source("dsh-x#registry:--userconfig=/tmp/x").is_err());
    assert!(dsh_plugins::parse_source("dsh-x#registry:file:///etc").is_err());
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

    // cordis:group 的子行被展开；依赖工作流引擎的行不加载。
    let session = row(report, "session-tool");
    assert_eq!(session["status"], "incompatible");
    assert!(session["reasons"][0].as_str().unwrap().contains("workflowEngine"));
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
fn dsh_service_class_without_name_and_client_plugins_are_reported() {
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
    // Service 子类现在按 Cordis 语义实例化；没用 super(ctx, 名字) 登记服务名的提醒一句。
    let store = row(&record.report, "dsh-store");
    assert_eq!(store["status"], "partial", "{store}");
    assert_eq!(store["kind"], "service");
    assert!(store["notes"].to_string().contains("没有用 super(ctx, 名字) 登记服务名"), "{store}");
}

/// Service 子类插件：实例化后作为服务给同一个包里的插件 inject（消费方写在前面也行）；行上的 isolate 让组里拿到私有实例，
/// ctx.isolate / ctx.set 给出作用域服务；钩子常驻进程只映射消费方时也会把提供方一并加载。
#[tokio::test]
async fn dsh_service_class_plugins_provide_services_with_isolate() {
    use codex_plus_core::extension_hooks::dispatch;
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(scripts.path().join("builtin"), scripts.path().join("user"), scripts.path().join("user_scripts.json"));
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("service-bundle")), &toolchain).unwrap();
    let counter = row(&record.report, "counter");
    assert_eq!((counter["kind"].as_str(), counter["status"].as_str()), (Some("service"), Some("compatible")), "{counter}");
    assert_eq!(counter["provides"], json!(["counter"]));
    for id in ["consumer", "scoped-consumer", "scoped-counter", "memo"] {
        let item = row(&record.report, id);
        assert!(item["status"] == "compatible" || item["status"] == "partial", "{id}：{item}");
    }

    let responses = serve(&plugins, &toolchain, &[
        call(1, "count_next", json!({})),
        call(2, "count_next", json!({})),
        call(3, "count_scoped", json!({})),
        call(4, "memo_probe", json!({})),
    ]);
    let text = |index: usize| responses[index]["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string();
    assert_eq!((text(0), text(1), text(2)), ("1".to_string(), "2".to_string(), "101".to_string()), "{responses:?}");
    assert_eq!(serde_json::from_str::<Value>(&text(3)).unwrap(), json!({"outer": "outer", "inner": "inner", "viaGet": "outer"}));

    // 钩子常驻进程：只有消费方映射到 PreToolUse，提供方也要加载，否则 inject 不满足。
    plugins.sync_companion(&manager, &record.id).unwrap();
    let denied = dispatch(&manager, "PreToolUse", json!({"session_id": "s1", "tool_name": "counter_probe", "tool_input": {}})).await.unwrap();
    assert!(denied["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap_or_default().ends_with("counter=1"), "{denied}");
    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
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
    assert!(renderer.contains("config: {}"), "没填配置时前端插件拿到空对象");
    // 管理器里填的配置也交给前端插件的 apply。
    plugins.set_row_config(&record.id, "dsh-client-demo", Some(json!({"greeting": "hey"}))).unwrap();
    plugins.reinspect(&record.id, &toolchain).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
    let renderer = std::fs::read_to_string(dir.join("renderer.js")).unwrap();
    assert!(renderer.contains(r#"config: {"greeting":"hey"}"#), "{renderer}");

    let backend = dir.join("backend/main.mjs");
    let script = format!(
        "const {{ default: create }} = await import({}); const methods = create(); const reply = await methods.fetch({{ path: '/api/client-demo/echo', method: 'POST', headers: {{ 'content-type': 'application/json' }}, body: JSON.stringify({{ text: 'ping' }}) }}); const missing = await methods.fetch({{ path: '/api/nope' }}); const rpc = await methods.rpc({{ channel: '/client-demo', endpoint: 'ping', payload: {{ a: 1 }} }}); const denied = await methods.rpc({{ channel: '/client-demo', endpoint: 'secret' }}); const nobody = await methods.rpc({{ channel: '/other', endpoint: 'x' }}); process.stdout.write(JSON.stringify({{ reply, missing: missing.status, rpc, denied, nobody }})); process.exit(0)",
        json!(url_of(&backend))
    );
    let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    assert_eq!(result["reply"]["status"], 200, "{}", String::from_utf8_lossy(&output.stderr));
    let body: Value = serde_json::from_str(result["reply"]["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["echo"], "ping");
    assert_eq!(result["missing"], 404);
    // RPC：通道处理器给出结果，拦截器按 endpoint 优先，没有处理器时 NOT_FOUND。
    assert_eq!(result["rpc"], json!({"ok": true, "value": {"endpoint": "ping", "payload": {"a": 1}, "hits": 1}}), "{result}");
    assert_eq!(result["denied"]["error"]["code"], "DENIED");
    assert_eq!(result["nobody"]["error"]["code"], "NOT_FOUND");

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

/// bundle 里的插件之间用 ctx.emit / serial / waterfall / bail 互通；工具守卫、可见性限制与
/// tools/execute 代为执行经配套拓展包的钩子与网关生效。
#[tokio::test]
async fn dsh_bundle_rows_talk_through_events_and_tool_runtime() {
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
    let record = plugins.import(&local_source(&fixture("toolkit-bundle")), &toolchain).unwrap();
    let emitter = row(&record.report, "emitter");
    let mut mappings: Vec<&str> = emitter["mappings"].as_array().unwrap().iter().map(|item| item.as_str().unwrap()).collect();
    mappings.sort();
    assert_eq!(mappings, ["PreToolUse", "gateway:step"], "{emitter}");

    // 工具在 MCP 进程里执行：经 serial / waterfall / bail 拿到另一行插件的答复。
    let responses = serve(&plugins, &toolchain, &[
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        call(2, "count_up", json!({})),
        call(3, "count_up", json!({})),
    ]);
    assert_eq!(responses[0]["result"]["tools"][0]["name"], "count_up", "{responses:?}");
    assert_eq!(responses[1]["result"]["content"][0]["text"], "pong 1! (seen 1)", "{responses:?}");
    assert_eq!(responses[2]["result"]["content"][0]["text"], "pong 2! (seen 2)", "{responses:?}");

    let companion = plugins.sync_companion(&manager, &record.id).unwrap();
    assert_eq!(companion["manifestError"], Value::Null, "{companion}");
    // 守卫：pre-execute 放行后仍可拒绝。
    let guarded = dispatch(&manager, "PreToolUse", json!({"tool_name": "Bash", "tool_input": {"command": "shutdown now"}})).await.unwrap();
    assert!(guarded["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().ends_with("toolkit: shutdown is guarded"), "{guarded}");
    assert_eq!(dispatch(&manager, "PreToolUse", json!({"tool_name": "Bash", "tool_input": {"command": "ls"}})).await.unwrap(), json!({}));
    // 代为执行：原工具以「拒绝 + 结果」结束，下一次请求里网关把输出换成纯结果。
    let executed = dispatch(&manager, "PreToolUse", json!({"session_id": "t1", "tool_name": "Bash", "tool_use_id": "call-now", "tool_input": {"command": "toolkit-now"}})).await.unwrap();
    assert!(executed["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().contains("TOOLKIT-TIME"), "{executed}");
    let mut body = json!({"model": "gpt-x", "prompt_cache_key": "t1", "tools": [
        {"type": "function", "name": "exec_command"},
        {"type": "function", "name": "web_search"},
    ], "input": [
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "time?"}]},
        {"type": "function_call", "call_id": "call-now", "name": "exec_command", "arguments": "{\"cmd\":\"toolkit-now\"}"},
        {"type": "function_call_output", "call_id": "call-now", "output": "denied by hook"},
    ]});
    apply_request_with(&manager, &mut body, "/v1/responses").await;
    assert_eq!(body["input"][2]["output"], "TOOLKIT-TIME", "{body}");
    // 可见性限制：请求里的 web_search 被去掉。
    assert_eq!(body["tools"], json!([{"type": "function", "name": "exec_command"}]), "{body}");

    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
}

/// Rust 侧「宿主提供、不必 npm install」的包与 host.mjs 的替身表一致（schemastery 除外：声明了就装真包）。
#[test]
fn dsh_host_provided_packages_match_host_shims() {
    let host = std::fs::read_to_string(host_source("host.mjs")).unwrap();
    let start = host.find("const SHIMS = {").unwrap();
    let end = start + host[start..].find("\n}").unwrap();
    let mut shims: Vec<&str> = host[start..end]
        .lines()
        .filter_map(|line| line.trim().strip_prefix('\'')?.split('\'').next())
        .filter(|name| *name != "@deepseek-ai/schemastery")
        .collect();
    shims.sort();
    let mut provided = dsh_plugins::HOST_PROVIDED_PACKAGES.to_vec();
    provided.sort();
    assert_eq!(provided, shims);
}

/// 第二批替身服务：审批对上 PermissionRequest，提问 waterfall 能由插件自己答，system-prompt/assemble 拿到
/// Codex 真实的 instructions（改了它就整段替换），Stop 负载里的助手回复进镜像，spill / tokenMeter / storage 可用。
#[tokio::test]
async fn dsh_second_batch_services_and_events() {
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
    let record = plugins.import(&local_source(&fixture("service-plugin")), &toolchain).unwrap();
    let main = row(&record.report, "dsh-service-demo");
    assert!(main["status"] == "compatible" || main["status"] == "partial", "{main}");
    let plan = dsh_plugins::report_bridge(&record.report);
    assert!(plan.hooks.iter().any(|hook| hook == "PermissionRequest"), "{plan:?}");

    // 工具在 MCP 进程里用到的服务。
    let responses = serve(&plugins, &toolchain, &[call(1, "probe_services", json!({}))]);
    let probe: Value = serde_json::from_str(responses[0]["result"]["content"][0]["text"].as_str().unwrap_or("null")).unwrap_or(Value::Null);
    assert_eq!(probe, json!({"answer": "yes", "spillBytes": 100, "locator": true, "tokens": 2, "domain": true, "title": null, "telemetry": "disabled"}), "{responses:?}");

    plugins.sync_companion(&manager, &record.id).unwrap();
    // 审批：bash 放行、apply_patch 拒绝、其余不表态（交还 Codex 弹审批）。
    let allowed = dispatch(&manager, "PermissionRequest", json!({"session_id": "a1", "tool_name": "Bash", "tool_input": {"command": "make"}})).await.unwrap();
    assert_eq!(allowed["hookSpecificOutput"]["decision"]["behavior"], "allow", "{allowed}");
    let denied = dispatch(&manager, "PermissionRequest", json!({"session_id": "a1", "tool_name": "apply_patch", "tool_input": {}})).await.unwrap();
    assert_eq!(denied["hookSpecificOutput"]["decision"]["behavior"], "deny", "{denied}");
    let untouched = dispatch(&manager, "PermissionRequest", json!({"session_id": "a1", "tool_name": "web_fetch", "tool_input": {}})).await.unwrap();
    assert_eq!(untouched, json!({}));

    // 提示词组装：插件看得到 Codex 的 instructions，默认只追加。
    let mut body = json!({"model": "gpt-x", "instructions": "base rules", "input": []});
    apply_request_with(&manager, &mut body, "/v1/responses").await;
    assert_eq!(body["instructions"], "base rules\n\nservice-demo: be brief", "{body}");

    // Stop 负载里的助手回复进镜像，投影读得到；inbox 排队、改写与事件。
    dispatch(&manager, "UserPromptSubmit", json!({"session_id": "a1", "prompt": "hi"})).await.unwrap();
    dispatch(&manager, "Stop", json!({"session_id": "a1", "stop_hook_active": false, "last_assistant_message": "hello from codex"})).await.unwrap();
    let dir = scripts.path().join("user/dsh-dsh-service-demo");
    let script = format!(
        "const {{ default: create }} = await import({}); const methods = create(); process.stdout.write(JSON.stringify(await methods.command({{ name: 'replies', rawInput: '', sessionId: 'a1' }}))); process.exit(0)",
        json!(url_of(&dir.join("backend/main.mjs")))
    );
    let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    assert_eq!(result["kind"], "success", "{}", String::from_utf8_lossy(&output.stderr));
    let text: Value = serde_json::from_str(result["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, json!({"replies": ["hello from codex"], "inserted": 1, "replaced": true, "tokens": true}), "{text}");

    // 改了 Codex 那一段就整段替换 instructions。
    plugins.set_row_config(&record.id, "dsh-service-demo", Some(json!({"replaceCodex": true}))).unwrap();
    plugins.reinspect(&record.id, &toolchain).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
    let mut body = json!({"model": "gpt-x", "instructions": "base rules", "input": []});
    apply_request_with(&manager, &mut body, "/v1/responses").await;
    assert_eq!(body["instructions"], "BASE rules", "{body}");

    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
}

/// llm 替身：工具、图片、助手的工具调用与 tool 结果翻成 Responses 条目交给宿主调用；回复里的推理与工具调用
/// 按 dsh 的块顺序拆成流式块，finish 为 tool-calls。宿主调用用一个假的可执行文件代替。
#[cfg(unix)]
#[test]
fn dsh_llm_stand_in_passes_tools_and_splits_tool_calls() {
    use std::os::unix::fs::PermissionsExt;
    let Some(toolchain) = toolchain() else { return };
    let temp = tempfile::tempdir().unwrap();
    let fake = temp.path().join("host");
    let seen = temp.path().join("seen.json");
    std::fs::write(&fake, format!(
        "#!{}\nlet input='';process.stdin.on('data',c=>input+=c).on('end',()=>{{require('fs').writeFileSync({},input);process.stdout.write(JSON.stringify({{result:{{text:'ok',reasoning:'think',toolCalls:[{{callId:'c1',name:'weather',arguments:'{{}}'}}]}}}})+'\\n')}})\n",
        toolchain.node.as_ref().unwrap().display(),
        json!(seen.to_string_lossy()),
    )).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let script = format!(
        "const {{ createLlm }} = await import({}); const out = []; for await (const chunk of createLlm().stream({{ tools: [{{ name: 'weather', description: 'w', parameters: {{ type: 'object' }} }}], messages: [{{ role: 'user', content: [{{ type: 'text', text: 'hi' }}, {{ type: 'image', data: 'AAA', mimeType: 'image/png' }}] }}, {{ role: 'assistant', content: [{{ type: 'tool-call', id: 'c0', name: 'weather', arguments: '{{}}' }}] }}, {{ role: 'tool', toolCallId: 'c0', content: [{{ type: 'text', text: 'sunny' }}] }}] }})) out.push(chunk.type + ':' + (chunk.blockType ?? chunk.reason?.kind ?? '')); process.stdout.write(JSON.stringify(out)); process.exit(0)",
        json!(url_of(&host_source("services.mjs")))
    );
    let output = Command::new(toolchain.node.as_ref().unwrap())
        .args(["--input-type=module", "-e", &script])
        .env("CODEX_PLUS_HOST_EXE", &fake)
        .output()
        .unwrap();
    let chunks: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    assert_eq!(chunks, json!([
        "block-start:reasoning", "reasoning-delta:", "block-end:",
        "block-start:text", "text-delta:", "block-end:",
        "block-start:tool-call", "tool-call-delta:", "block-end:",
        "usage:", "finish:tool-calls"
    ]), "{}", String::from_utf8_lossy(&output.stderr));
    let request: Value = serde_json::from_str(&std::fs::read_to_string(&seen).unwrap()).unwrap();
    assert_eq!(request["params"]["input"], json!([
        {"role": "user", "content": [{"type": "input_text", "text": "hi"}, {"type": "input_image", "image_url": "data:image/png;base64,AAA"}]},
        {"type": "function_call", "call_id": "c0", "name": "weather", "arguments": "{}"},
        {"type": "function_call_output", "call_id": "c0", "output": "sunny"}
    ]));
    assert_eq!(request["params"]["tools"][0]["name"], "weather");
}

/// bundle patch 的完整语义：按 id 覆盖字段、`{ id, insert }` 插进组、include 文件与它自己的 patches、
/// name 对不上的覆盖跳过、`!!js` 在 vm 里求值完整表达式（ctx / env，超时按启用处理并诊断，结果按真假判停用）。
#[test]
fn dsh_bundle_patch_semantics_match_dsh() {
    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("patch-bundle")), &toolchain).unwrap();
    let rows: Vec<(String, String, Vec<String>)> = record.report["rows"].as_array().unwrap().iter().map(|row| (
        row["id"].as_str().unwrap().to_string(),
        row["status"].as_str().unwrap().to_string(),
        row["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap().to_string()).collect(),
    )).collect();
    let owned = |id: &str, status: &str, tools: &[&str]| (id.to_string(), status.to_string(), tools.iter().map(|tool| tool.to_string()).collect::<Vec<_>>());
    assert_eq!(rows, vec![
        owned("a", "compatible", &["say_two"]),
        owned("b", "compatible", &["say_bee"]),
        owned("c", "disabled", &[]),
        owned("d", "compatible", &["say_dee_loud"]),
        owned("e", "compatible", &["say_e-2-function-true"]),
        owned("f", "compatible", &["say_123"]),
        owned("g", "disabled", &[]),
    ], "{}", record.report);
    let diagnostics = record.report["diagnostics"].to_string();
    assert!(diagnostics.contains("name 对不上"), "{diagnostics}");
    assert!(diagnostics.contains("插件行 f 的 disabled 表达式 !!js") && diagnostics.contains("timed out"), "{diagnostics}");
}

/// 插件市场索引（格式 1）：npm 与 git 来源翻成导入能用的来源字符串，取最新版本，认不出的跳过。
#[test]
fn dsh_market_index_entries_become_import_sources() {
    let entries = dsh_plugins::parse_market_index(&json!({
        "format": 1,
        "title": "demo",
        "plugins": [
            { "name": "@scope/dsh-tool-a", "metadata": { "title": "工具 A", "description": "说明 A" }, "versions": [
                { "version": "0.1.0", "source": { "type": "npm" } },
                { "version": "0.2.0", "source": { "type": "npm", "registry": "https://npm.example.com" } }
            ] },
            { "name": "dsh-git-b", "title": "B", "versions": [{ "version": "1.0.0", "source": { "type": "git", "url": "https://github.com/x/y.git", "commit": "abc123", "path": "packages/b" } }] },
            { "name": "weird", "versions": [{ "version": "1", "source": { "type": "tarball" } }] },
            { "name": "empty", "versions": [] }
        ]
    })).unwrap();
    assert_eq!(entries.len(), 2);
    // 自定义 registry 随来源带上，导入时用它。
    assert_eq!(entries[0].source, "@scope/dsh-tool-a@0.2.0#registry:https://npm.example.com");
    assert_eq!(entries[0].title, "工具 A");
    assert_eq!(entries[0].note, None);
    assert_eq!(
        dsh_plugins::parse_source(&entries[0].source).unwrap(),
        PluginSource::Npm { spec: "@scope/dsh-tool-a@0.2.0".to_string(), registry: Some("https://npm.example.com".to_string()) }
    );
    assert_eq!(entries[1].source, "https://github.com/x/y.git#abc123&path:packages/b");
    assert!(matches!(dsh_plugins::parse_source(&entries[1].source).unwrap(), PluginSource::Git { reference: Some(ref commit), path: Some(ref path), .. } if commit == "abc123" && path == "packages/b"));
    assert!(dsh_plugins::parse_market_index(&json!({ "format": 2, "plugins": [] })).is_err());
}

/// 近似实现的运行时服务：计划模式在 PreToolUse 里拒绝写类工具，目标在 Stop 里要求继续（到完成或上限），
/// 插件自己的 fs 写入先问 fs/write-intent、读到 / 确认不存在时发 fs/observed。
#[tokio::test]
async fn dsh_runtime_stand_ins_plan_mode_goals_and_fs_intents() {
    use codex_plus_core::extension_hooks::dispatch;
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(scripts.path().join("builtin"), scripts.path().join("user"), scripts.path().join("user_scripts.json"));
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("runtime-plugin")), &toolchain).unwrap();
    let plan = dsh_plugins::report_bridge(&record.report);
    for hook in ["PreToolUse", "Stop"] {
        assert!(plan.hooks.iter().any(|item| item == hook), "{hook}：{plan:?}");
    }

    let responses = serve(&plugins, &toolchain, &[call(1, "probe_runtime", json!({ "dir": work.path().to_string_lossy() }))]);
    let probe: Value = serde_json::from_str(responses[0]["result"]["content"][0]["text"].as_str().unwrap_or("null")).unwrap_or(Value::Null);
    assert_eq!(probe, json!({ "second": "FS_STALE_VERSION", "observed": 3, "providers": ["codex"], "planClass": "function" }), "{responses:?}");

    plugins.sync_companion(&manager, &record.id).unwrap();
    let dir = scripts.path().join("user/dsh-dsh-runtime-demo");
    let command = |name: &str, input: &str| {
        let script = format!(
            "const {{ default: create }} = await import({}); process.stdout.write(JSON.stringify(await create().command({{ name: {}, rawInput: {}, sessionId: 'r1' }}))); process.exit(0)",
            json!(url_of(&dir.join("backend/main.mjs"))), json!(name), json!(input)
        );
        let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
        serde_json::from_slice::<Value>(&output.stdout).unwrap_or(Value::Null)
    };
    // 计划模式：apply_patch 被拒，读类工具放行；关掉后放行。
    assert_eq!(command("plan", "")["text"], "committed");
    let denied = dispatch(&manager, "PreToolUse", json!({"session_id": "r1", "tool_name": "apply_patch", "tool_input": {}})).await.unwrap();
    assert!(denied["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap_or_default().contains("计划模式"), "{denied}");
    assert_eq!(dispatch(&manager, "PreToolUse", json!({"session_id": "r1", "tool_name": "Bash", "tool_input": {"command": "ls"}})).await.unwrap(), json!({}));
    assert_eq!(command("plan", "off")["text"], "committed");
    let allowed = dispatch(&manager, "PreToolUse", json!({"session_id": "r1", "tool_name": "apply_patch", "tool_input": {}})).await.unwrap();
    assert!(allowed["hookSpecificOutput"].get("permissionDecision").is_none(), "{allowed}");
    assert!(allowed["hookSpecificOutput"]["additionalContext"].as_str().unwrap_or_default().contains("计划模式已关闭"), "切换时排下的说明随下一次钩子带上：{allowed}");
    // 目标：要停下时要求继续（钩子续过一轮后也照样），完成后放行。
    assert_eq!(command("goal", "ship it")["text"], "active");
    let stop = dispatch(&manager, "Stop", json!({"session_id": "r1", "stop_hook_active": false})).await.unwrap();
    assert!(stop["reason"].as_str().unwrap_or_default().contains("ship it"), "{stop}");
    let again = dispatch(&manager, "Stop", json!({"session_id": "r1", "stop_hook_active": true})).await.unwrap();
    assert_eq!(again["decision"], "block", "{again}");
    assert_eq!(command("done", "")["text"], "complete");
    assert_eq!(dispatch(&manager, "Stop", json!({"session_id": "r1", "stop_hook_active": true})).await.unwrap(), json!({}));

    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
}

/// 补上的零碎事件：goal/changed、domain/changed、credentials 写入后的事件、workspace/session-activity（服务里跑），
/// workspace/session-stop（Stop 钩子）与 agent/error（网关失败重试带上会话）。
#[tokio::test]
async fn dsh_remaining_events_are_dispatched() {
    use codex_plus_core::extension_hooks::dispatch;
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(scripts.path().join("builtin"), scripts.path().join("user"), scripts.path().join("user_scripts.json"));
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("events-plugin")), &toolchain).unwrap();
    let plan = dsh_plugins::report_bridge(&record.report);
    assert!(plan.gateway_error && plan.hooks.iter().any(|hook| hook == "Stop"), "{plan:?}");

    let responses = serve(&plugins, &toolchain, &[call(1, "probe_events", json!({}))]);
    let probe: Value = serde_json::from_str(responses[0]["result"]["content"][0]["text"].as_str().unwrap_or("null")).unwrap_or(Value::Null);
    assert_eq!(probe, json!({
        "seen": ["goal:create", "goal:complete", "domain:items:a:put", "domain:items:a:delete", "cred:TOKEN_X"],
        "source": "stored",
        "activity": [{ "kind": "working", "sessionId": "g1" }],
    }), "{responses:?}");

    plugins.sync_companion(&manager, &record.id).unwrap();
    dispatch(&manager, "Stop", json!({"session_id": "w1", "stop_hook_active": false})).await.unwrap();
    let dir = scripts.path().join("user/dsh-dsh-events-demo");
    let script = format!(
        "const {{ default: fail }} = await import({}); await fail({{ status: 500, body: 'boom', json: {{ error: {{ code: 'upstream_down' }} }}, session: 'w1' }}); const {{ default: create }} = await import({}); process.stdout.write(JSON.stringify(await create().command({{ name: 'seen', rawInput: '', sessionId: 'w1' }}))); process.exit(0)",
        json!(url_of(&dir.join("gateway/error.mjs"))), json!(url_of(&dir.join("backend/main.mjs")))
    );
    let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    let seen: Value = serde_json::from_str(result["text"].as_str().unwrap_or("null")).unwrap_or(Value::Null);
    assert_eq!(seen, json!(["stop:w1", "error:w1:upstream_down"]), "{}", String::from_utf8_lossy(&output.stderr));

    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
}

/// 网页路由（webServer → 配套拓展包后台的 HTTP）、MCP 资源（dsh 插件 MCP 的 resources/*）、附件与文件引用。
#[tokio::test]
async fn dsh_web_routes_mcp_resources_attachments_and_file_references() {
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(scripts.path().join("builtin"), scripts.path().join("user"), scripts.path().join("user_scripts.json"));
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("web-plugin")), &toolchain).unwrap();
    assert!(dsh_plugins::report_bridge(&record.report).webhook, "webServer 路由要挂后台 HTTP");

    let responses = serve(&plugins, &toolchain, &[
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "resources/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": "dsh-notes+note://readme"}}),
        call(4, "probe_files", json!({})),
    ]);
    assert!(responses[0]["result"]["capabilities"]["resources"].is_object(), "{:?}", responses[0]);
    assert_eq!(responses[1]["result"]["resources"][0]["uri"], "dsh-notes+note://readme");
    assert_eq!(responses[2]["result"]["contents"][0]["text"], "content of note://readme");
    assert_eq!(responses[2]["result"]["contents"][0]["uri"], "dsh-notes+note://readme");
    let probe: Value = serde_json::from_str(responses[3]["result"]["content"][0]["text"].as_str().unwrap_or("null")).unwrap_or(Value::Null);
    assert_eq!(probe, json!({ "bytes": 3, "rejected": true, "refs": [{ "path": "notes/readme.md", "kind": "file" }] }), "{responses:?}");

    plugins.sync_companion(&manager, &record.id).unwrap();
    let dir = scripts.path().join("user/dsh-dsh-web-demo");
    let script = format!(
        "const {{ default: create }} = await import({}); const reply = await create().http({{ path: '/ext/dsh-dsh-web-demo/hello', method: 'POST', headers: {{}}, body: 'there' }}); const missing = await create().http({{ path: '/ext/dsh-dsh-web-demo/nope', method: 'GET', headers: {{}} }}); process.stdout.write(JSON.stringify({{ reply, missing: missing.status }})); process.exit(0)",
        json!(url_of(&dir.join("backend/main.mjs")))
    );
    let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    assert_eq!(result["reply"], json!({ "status": 201, "headers": { "content-type": "text/plain" }, "body": "hi POST there" }), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(result["missing"], 404);

    // WebSocket 升级：Codex++ 本地代理的 /ext/<配套拓展>/ws 原样转到常驻进程里的 Node http 服务，插件自己的处理器完成握手与收发。
    {
        use futures_util::{SinkExt, StreamExt};
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server_manager = manager.clone();
        let server = tokio::spawn(async move {
            let mut handled = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let server_manager = server_manager.clone();
                handled.push(tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0_u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        stream.read_exact(&mut byte).await.unwrap();
                        head.push(byte[0]);
                    }
                    head.truncate(head.len() - 4);
                    let text = String::from_utf8_lossy(&head).to_string();
                    let path = text.lines().next().unwrap().split_whitespace().nth(1).unwrap().to_string();
                    codex_plus_core::extension_backend::route_upgrade_with(&server_manager, &mut stream, &path, &text, &head).await.unwrap();
                }));
            }
            for task in handled {
                task.await.unwrap();
            }
        });
        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ext/dsh-dsh-web-demo/ws")).await.unwrap();
        let hello = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next()).await.expect("等问候超时").unwrap().unwrap();
        assert_eq!(hello.into_text().unwrap().as_str(), "hello /ws");
        socket.send(tokio_tungstenite::tungstenite::Message::Text("ping".into())).await.unwrap();
        let echo = socket.next().await.unwrap().unwrap();
        assert_eq!(echo.into_text().unwrap().as_str(), "echo:ping");
        drop(socket);
        // 别的网页（Origin 与 Host 不同）发来的升级被拒绝。
        let mut request = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(format!("ws://127.0.0.1:{port}/ext/dsh-dsh-web-demo/ws")).unwrap();
        request.headers_mut().insert("origin", "https://evil.example".parse().unwrap());
        assert!(tokio_tungstenite::connect_async(request).await.is_err());
        server.await.unwrap();
    }

    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
}

/// 自定义 npm registry：`npm pack` 与依赖安装都带上 `--registry`（假的 npm 记下参数并造出 tarball）。
#[cfg(unix)]
#[test]
fn dsh_npm_import_uses_custom_registry_for_pack_and_install() {
    use std::os::unix::fs::PermissionsExt;
    let Some(mut toolchain) = toolchain() else { return };
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("src/package");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.json"), r#"{"name":"dsh-reg-demo","version":"1.0.0","type":"module","main":"index.js","dependencies":{"left-pad":"1.3.0"},"peerDependencies":{"@deepseek-ai/dsh-tools":"*"}}"#).unwrap();
    std::fs::write(package.join("index.js"), "import { defineTool } from '@deepseek-ai/dsh-tools'\nexport const name = 'reg'\nexport const inject = ['tools']\nexport function apply(ctx) {\n  ctx.tools.register(defineTool({ name: 'reg_ping', description: 'ping', parameters: {}, output: { schema: { type: 'string' }, render: (_a, v) => [{ type: 'text', text: v }] }, async execute() { return 'pong' } }))\n}\n").unwrap();
    let log = temp.path().join("npm-args.log");
    let fake = temp.path().join("npm");
    std::fs::write(&fake, format!(
        "#!{}\nconst {{ appendFileSync, mkdirSync }} = require('fs')\nconst {{ execFileSync }} = require('child_process')\nconst args = process.argv.slice(2)\nappendFileSync({}, JSON.stringify(args) + '\\n')\nif (args[0] === 'pack') {{\n  const dest = args[args.indexOf('--pack-destination') + 1]\n  execFileSync('tar', ['-czf', dest + '/dsh-reg-demo-1.0.0.tgz', '-C', {}, 'package'])\n  process.stdout.write(JSON.stringify([{{ filename: 'dsh-reg-demo-1.0.0.tgz' }}]))\n}} else {{\n  mkdirSync('node_modules/left-pad', {{ recursive: true }})\n}}\n",
        toolchain.node.as_ref().unwrap().display(),
        json!(log.to_string_lossy()),
        json!(temp.path().join("src").to_string_lossy()),
    )).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    toolchain.npm = Some(fake);

    let state = tempfile::tempdir().unwrap();
    let plugins = DshPlugins::new(state.path());
    let source = dsh_plugins::parse_source("dsh-reg-demo@1.0.0#registry:https://npm.acme.dev/").unwrap();
    let record = plugins.import(&source, &toolchain).unwrap();
    assert_eq!(row(&record.report, "dsh-reg-demo")["tools"][0]["name"], "reg_ping", "{}", record.report);
    let calls: Vec<Vec<String>> = std::fs::read_to_string(&log).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0][0], "pack");
    assert_eq!(calls[1][0], "install");
    for call in &calls {
        let at = call.iter().position(|arg| arg == "--registry").unwrap_or_else(|| panic!("缺少 --registry：{call:?}"));
        assert_eq!(call[at + 1], "https://npm.acme.dev/");
    }
    // 注册表里记着 registry，重新导入 / 更新时沿用。
    let saved = plugins.load().unwrap();
    assert!(matches!(&saved.plugins[0].source, PluginSource::Npm { registry: Some(registry), .. } if registry == "https://npm.acme.dev/"));
}

/// 插件间事件跨包：dsh-bus-a 的命令在它的常驻进程里 emit / serial / waterfall / parallel / bail，只有监听器的 dsh-bus-b
/// （配套拓展包只有标记文件）在自己的常驻进程里收到，语义与包内一致；waterfall 的 next 回到发起方的链尾。
/// jobs 跨进程：常驻进程里起的任务，MCP 进程里看得到输出、能停掉并等到结束。
#[test]
fn dsh_events_cross_packages_and_jobs_cross_processes() {
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(scripts.path().join("builtin"), scripts.path().join("user"), scripts.path().join("user_scripts.json"));
    let plugins = DshPlugins::new(state.path());
    let a = plugins.import(&local_source(&fixture("bus-a")), &toolchain).unwrap();
    let b = plugins.import(&local_source(&fixture("bus-b")), &toolchain).unwrap();
    let listens = row(&b.report, "dsh-bus-b")["listens"].clone();
    assert_eq!(listens, json!(["demo/note", "demo/ask", "demo/flow", "demo/sync", "demo/bail", "demo/seen"]), "{}", b.report);
    let plan = dsh_plugins::report_bridge(&b.report);
    assert!(plan.events && plan.hooks.is_empty() && plan.commands.is_empty(), "{plan:?}");
    for record in [&a, &b] {
        let companion = plugins.sync_companion(&manager, &record.id).unwrap();
        assert_eq!(companion["manifestError"], Value::Null, "{companion}");
    }
    assert!(scripts.path().join("user/dsh-dsh-bus-b/dsh-bridge.json").is_file());

    let backend = url_of(&scripts.path().join("user/dsh-dsh-bus-a/backend/main.mjs"));
    let command = |name: &str| {
        let script = format!(
            "const {{ default: create }} = await import({}); process.stdout.write(JSON.stringify(await create().command({{ name: {}, rawInput: '', sessionId: 'b1' }}))); process.exit(0)",
            json!(backend), json!(name)
        );
        let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
        let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        assert_eq!(result["kind"], "success", "{result} {}", String::from_utf8_lossy(&output.stderr));
        result["text"].as_str().unwrap_or_default().to_string()
    };
    let ping: Value = serde_json::from_str(&command("ping")).unwrap();
    assert_eq!(ping, json!({
        "answer": "b answers q",
        "flow": { "trail": ["a", "b", "end"] },
        "bailed": 20,
        "seen": ["note:from-a", "sync:x"],
    }));

    // jobs：任务在 dsh-bus-a 的常驻进程里跑，MCP 进程里读到输出、停掉它。
    command("job-start");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let responses = serve(&plugins, &toolchain, &[call(1, "jobs_probe", json!({}))]);
    let probe: Value = serde_json::from_str(responses[0]["result"]["content"][0]["text"].as_str().unwrap_or("null")).unwrap_or(Value::Null);
    assert_eq!(probe, json!({ "found": true, "before": "running", "output": true, "kill": "requested", "after": "killed" }), "{responses:?}");

    for record in [&a, &b] {
        plugins.set_enabled(&record.id, false).unwrap();
        plugins.sync_companion(&manager, &record.id).unwrap();
    }
}

/// llm.stream 真流式：宿主调用通道是行式流，第一段文字增量在回复结束前就到插件手里；
/// 插件注册的 adapter 在 provider 对得上时接管 stream。宿主调用用一个假的可执行文件代替（分两段写、中间停 400ms）。
#[cfg(unix)]
#[test]
fn dsh_llm_stream_yields_deltas_before_the_reply_finishes() {
    use std::os::unix::fs::PermissionsExt;
    let Some(toolchain) = toolchain() else { return };
    let temp = tempfile::tempdir().unwrap();
    let fake = temp.path().join("host");
    std::fs::write(&fake, format!(
        "#!{}\nlet input='';process.stdin.on('data',c=>input+=c).on('end',()=>{{const req=JSON.parse(input);if(req.method!=='stream'||!req.stream){{process.stdout.write(JSON.stringify({{error:'not streamed'}})+'\\n');return}}const w=(v)=>process.stdout.write(JSON.stringify(v)+'\\n');w({{delta:{{type:'reasoning',text:'th'}}}});w({{delta:{{type:'text',text:'Hel'}}}});setTimeout(()=>{{w({{delta:{{type:'text',text:'lo'}}}});w({{result:{{text:'Hello',reasoning:'th',usage:{{output_tokens:2}}}}}})}},400)}})\n",
        toolchain.node.as_ref().unwrap().display(),
    )).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let script = format!(
        "const {{ createLlm }} = await import({}); const llm = createLlm(); const start = Date.now(); const out = []; let firstText; for await (const chunk of llm.stream({{ messages: [{{ role: 'user', content: [{{ type: 'text', text: 'hi' }}] }}] }})) {{ if (chunk.type === 'text-delta' && firstText === undefined) firstText = Date.now() - start; out.push(chunk.type + ':' + (chunk.text ?? chunk.blockType ?? chunk.reason?.kind ?? chunk.block?.text ?? '')) }} const total = Date.now() - start; const off = llm.registerAdapter({{ id: 'mine', async *stream(options) {{ yield {{ type: 'text-delta', index: 0, text: 'from adapter ' + options.model }} }} }}); const via = []; for await (const chunk of llm.stream({{ provider: 'mine', model: 'm1', messages: [] }})) via.push(chunk.text); const providers = llm.listProviders().map((p) => p.id); off(); process.stdout.write(JSON.stringify({{ out, early: total - firstText >= 300, via, providers, after: llm.listProviders().length }})); process.exit(0)",
        json!(url_of(&host_source("services.mjs")))
    );
    let output = Command::new(toolchain.node.as_ref().unwrap())
        .args(["--input-type=module", "-e", &script])
        .env("CODEX_PLUS_HOST_EXE", &fake)
        .output()
        .unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    assert_eq!(result["out"], json!([
        "block-start:reasoning", "reasoning-delta:th", "block-end:th",
        "block-start:text", "text-delta:Hel", "text-delta:lo", "block-end:Hello",
        "usage:", "finish:stop"
    ]), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(result["early"], true, "第一段文字要在回复结束前到达：{result}");
    assert_eq!(result["via"], json!(["from adapter m1"]));
    assert_eq!(result["providers"], json!(["codex", "mine"]));
    assert_eq!(result["after"], 1, "撤销后 adapter 不在了");
}

/// typert 类型化 RPC：包导出 ./typert 时自动登记它的 TYPERT 清单，前端 ctx.remote 经 connection.rpc 的 /api 调到宿主：
/// 按描述把线上参数对上方法参数（严格 codec 校验、没 schema 放行、lookup 解析成宿主对象），this.ctx.invocation 可用；
/// stream 方法经 rpcOpen / rpcPush / rpcPull 分批收发（上行按 uplink codec 校验）。
#[test]
fn dsh_typert_methods_are_callable_through_connection_rpc() {
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(scripts.path().join("builtin"), scripts.path().join("user"), scripts.path().join("user_scripts.json"));
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("typert-plugin")), &toolchain).unwrap();
    let main = row(&record.report, "dsh-typert-demo");
    assert_eq!(main["kind"], "service", "{main}");
    assert!(main["mappings"].as_array().unwrap().iter().any(|item| item == "connection"), "{main}");
    assert!(dsh_plugins::report_bridge(&record.report).connection);
    plugins.sync_companion(&manager, &record.id).unwrap();

    let script = format!(r#"
const {{ default: create }} = await import({backend})
const api = create()
const rpc = (endpoint, payload) => api.rpc({{ channel: '/api', endpoint, payload }})
const out = {{}}
out.descriptors = (await rpc('$codexpp/typert-descriptors', null)).value.map((d) => `${{d.namespace}}/${{d.method}}${{d.mode ? ':' + d.mode : ''}}`)
out.added = (await rpc('notes/add', {{ args: {{ text: 'hello' }} }})).value
out.positional = (await rpc('notes/add', {{ positional: ['world'] }})).value
out.invalid = (await rpc('notes/add', {{ args: {{ text: 42 }} }})).error.code
out.extra = (await rpc('notes/add', {{ args: {{ text: 'x', nope: 1 }} }})).error.code
out.list = (await rpc('notes/list', {{ args: {{}} }})).value
out.read = (await rpc('notes/read', {{ args: {{ noteId: 1 }} }})).value
out.missing = (await rpc('notes/read', {{ args: {{ noteId: 9 }} }})).error.code
out.unaryOnStream = (await rpc('notes/watch', {{ args: {{ prefix: '>' }} }})).error.code
const opened = await api.rpcOpen({{ channel: '/api', endpoint: 'notes/watch', payload: {{ args: {{ prefix: '>' }} }} }})
await api.rpcPush({{ id: opened.id, items: ['a', 'b'] }})
await api.rpcPush({{ id: opened.id, items: [], end: true }})
const items = []
for (;;) {{
  const batch = await api.rpcPull({{ id: opened.id, waitMs: 500 }})
  items.push(...batch.items)
  if (batch.done) break
}}
out.stream = items
const bad = await api.rpcOpen({{ channel: '/api', endpoint: 'notes/watch', payload: {{ args: {{ prefix: '!' }} }} }})
await api.rpcPush({{ id: bad.id, items: [7], end: true }})
let failed
for (;;) {{
  const batch = await api.rpcPull({{ id: bad.id, waitMs: 500 }})
  if (batch.done) {{ failed = batch.error?.message ?? null; break }}
}}
out.badUplink = /expected string/.test(failed ?? '')
process.stdout.write(JSON.stringify(out))
process.exit(0)
"#, backend = json!(url_of(&scripts.path().join("user/dsh-dsh-typert-demo/backend/main.mjs"))));
    let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    assert_eq!(result, json!({
        "descriptors": ["notes/add", "notes/list", "notes/read", "notes/watch:stream"],
        "added": 1,
        "positional": 2,
        "invalid": "gateway/arguments-invalid",
        "extra": "gateway/arguments-invalid",
        "list": ["hello", "world"],
        "read": "note: world（read）",
        "missing": "gateway/lookup-not-found",
        "unaryOnStream": "gateway/binding-invalid",
        "stream": [">a", ">b"],
        "badUplink": true,
    }), "{}", String::from_utf8_lossy(&output.stderr));

    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
}

/// 工具的 registerOptions.when：MCP 调用时判断（没有 agent），经代理的每一步请求里按 agent 判断，不满足就从 tools 去掉；
/// exec.deferContext 的上下文并进工具结果；sessionQuery.observeSession 对镜像里的会话给 live 切面。
#[tokio::test]
async fn dsh_tool_when_defer_context_and_live_observation() {
    use codex_plus_core::extension_gateway::apply_request_with;
    use codex_plus_core::extension_hooks::dispatch;
    use codex_plus_core::user_scripts::UserScriptManager;

    let Some(toolchain) = toolchain() else { return };
    let state = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(scripts.path().join("builtin"), scripts.path().join("user"), scripts.path().join("user_scripts.json"));
    let plugins = DshPlugins::new(state.path());
    let record = plugins.import(&local_source(&fixture("tool-extras")), &toolchain).unwrap();
    assert!(dsh_plugins::report_bridge(&record.report).gateway_step, "when 要在每一步的请求里判断");

    let responses = serve(&plugins, &toolchain, &[call(1, "gated", json!({})), call(2, "deferring", json!({}))]);
    assert_eq!(responses[0]["result"]["isError"], true, "{responses:?}");
    assert!(responses[0]["result"]["content"][0]["text"].as_str().unwrap().contains("when 条件不满足"));
    let deferred: Vec<&str> = responses[1]["result"]["content"].as_array().unwrap().iter().map(|block| block["text"].as_str().unwrap()).collect();
    assert_eq!(deferred, ["done", "remember the deferred note"]);

    plugins.sync_companion(&manager, &record.id).unwrap();
    let tools = json!([
        {"type": "function", "name": "mcp__codex_plus_dsh__gated"},
        {"type": "function", "name": "mcp__codex_plus_dsh__deferring"},
        {"type": "function", "name": "exec_command"},
    ]);
    let input = json!([{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}]);
    let mut allowed = json!({"model": "gpt-x", "prompt_cache_key": "allowed", "tools": tools, "input": input});
    apply_request_with(&manager, &mut allowed, "/v1/responses").await;
    assert_eq!(allowed["tools"].as_array().unwrap().len(), 3, "{allowed}");
    let mut other = json!({"model": "gpt-x", "prompt_cache_key": "other", "tools": tools, "input": input});
    apply_request_with(&manager, &mut other, "/v1/responses").await;
    let names: Vec<&str> = other["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["mcp__codex_plus_dsh__deferring", "exec_command"], "{other}");

    // 会话 allowed 经钩子进了镜像：observeSession 给 live 切面。
    dispatch(&manager, "UserPromptSubmit", json!({"session_id": "allowed", "prompt": "hello"})).await.unwrap();
    let script = format!(
        "const {{ default: create }} = await import({}); process.stdout.write(JSON.stringify(await create().command({{ name: 'observe', rawInput: '', sessionId: 'allowed' }}))); process.exit(0)",
        json!(url_of(&scripts.path().join("user/dsh-dsh-tool-extras/backend/main.mjs")))
    );
    let output = Command::new(toolchain.node.as_ref().unwrap()).args(["--input-type=module", "-e", &script]).output().unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    let observed: Value = serde_json::from_str(result["text"].as_str().unwrap_or("null")).unwrap_or(Value::Null);
    assert_eq!(observed["source"], "live", "{result} {}", String::from_utf8_lossy(&output.stderr));
    assert!(observed["types"].as_array().unwrap().iter().any(|item| item == "user/message"), "{observed}");
    assert_eq!(observed["cursor"].as_i64().unwrap() + 1, observed["types"].as_array().unwrap().len() as i64);

    plugins.set_enabled(&record.id, false).unwrap();
    plugins.sync_companion(&manager, &record.id).unwrap();
}
