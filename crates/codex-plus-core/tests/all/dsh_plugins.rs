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
    assert_eq!(greet["status"], "partial", "用了 systemPrompt，应标为部分兼容：{greet}");
    assert_eq!(greet["tools"][0]["name"], "greet");

    // cordis:group 的子行被展开；依赖会话服务的行不加载。
    let session = row(report, "session-tool");
    assert_eq!(session["status"], "incompatible");
    assert!(session["reasons"][0].as_str().unwrap().contains("sessions"));
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
