use codex_plus_core::relay_config::apply_relay_profile_to_home_with_switch_rules;
use codex_plus_core::settings::{RelayMode, RelayProfile};
use std::fs;

// live 侧：node_repl 四键齐全，env 子表独立。这是 Codex 自己维护的正确形状。
const LIVE: &str = r#"model_provider = "custom"
model = "deepseek-v4.1-flash"

[model_providers]
[model_providers.custom]
name = "custom"
wire_api = "responses"
base_url = "http://127.0.0.1:57321/v1"

[mcp_servers.node_repl]
args = []
command = 'C:\Users\shizi\AppData\Local\OpenAI\Codex\runtimes\cua_node\df473e5367fa2b42\bin\node_repl.exe'
env_vars = ["CODEX_WINDOWS_REGISTERED_CORE"]
startup_timeout_sec = 120

[mcp_servers.node_repl.env]
NODE_REPL_NODE_PATH = 'C:\Users\shizi\AppData\Local\OpenAI\Codex\runtimes\cua_node\df473e5367fa2b42\bin\node.exe'
CODEX_HOME = 'C:\Users\shizi\.codex'
"#;

// profile 侧：node_repl 是被压平过的坏形状，env 键混进了父表，
// 而 args/command/env_vars/startup_timeout_sec 这四个键不在。
// 修复前写盘逻辑「只补缺不覆盖」，live 里那份好的再也补不回来，坏形状被反复写出去。
const FLATTENED_PROFILE: &str = r#"model_provider = "custom"
model = "deepseek-v4.1-flash"

[model_providers]
[model_providers.custom]
name = "custom"
wire_api = "responses"
base_url = "http://127.0.0.1:57321/v1"

[mcp_servers.node_repl]
NODE_REPL_NODE_PATH = 'C:\Users\shizi\AppData\Local\OpenAI\Codex\runtimes\cua_node\df473e5367fa2b42\bin\node.exe'
CODEX_HOME = 'C:\Users\shizi\.codex'

[mcp_servers.node_repl.env]
NODE_REPL_NODE_PATH = 'C:\Users\shizi\AppData\Local\OpenAI\Codex\runtimes\cua_node\df473e5367fa2b42\bin\node.exe'
CODEX_HOME = 'C:\Users\shizi\.codex'
"#;

#[test]
fn flattened_profile_must_be_repaired_from_live() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("config.toml"), LIVE).unwrap();
    fs::write(temp.path().join("auth.json"), r#"{"OPENAI_API_KEY":"sk-test"}"#).unwrap();

    let profile = RelayProfile {
        id: "relay-test".to_string(),
        name: "test".to_string(),
        relay_mode: RelayMode::PureApi,
        use_common_config: true,
        config_contents: FLATTENED_PROFILE.to_string(),
        auth_contents: r#"{"OPENAI_API_KEY":"sk-test"}"#.to_string(),
        ..RelayProfile::default()
    };

    apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile, "").unwrap();

    let out = fs::read_to_string(temp.path().join("config.toml")).unwrap();
    let parsed: toml::Value = out.parse().unwrap();
    let nr = &parsed["mcp_servers"]["node_repl"];
    let keys: Vec<String> = nr
        .as_table()
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();
    println!("node_repl keys = {:?}", keys);

    for k in ["args", "command", "env_vars", "startup_timeout_sec"] {
        assert!(
            nr.get(k).is_some(),
            "node_repl missing {k}, actual keys = {keys:?}"
        );
    }
}