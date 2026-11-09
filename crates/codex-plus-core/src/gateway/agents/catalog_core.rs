//! 通用 Agent 规格：OpenCode 系（OpenCode 与沿用它配置格式的 Agent）。
//!
//! 也是新增规格的参考写法：路径函数、探测函数、写入函数三件套，写入只改自己的供应商条目和
//! 选中的模型，用户的其它配置不动。

use std::path::PathBuf;

use serde_json::{Map, Value, json};

use crate::gateway::agents::{
    AgentEnv, AgentSpec, AgentWire, FileSet, PROVIDER_ID, PROVIDER_NAME, Wiring, object_entry,
};

pub static SPECS: &[AgentSpec] = &[OPENCODE];

/// 模型的输出上限：窗口的四分之一，最多 32K（OpenCode 的 `limit` 两项都要填）。
pub fn output_limit(window: u64) -> u64 {
    (window / 4).clamp(1, 32_000)
}

/// OpenCode 格式的模型表：`{ "<模型>": { name, limit? } }`。
pub fn opencode_models(wiring: &Wiring) -> Map<String, Value> {
    let mut models = Map::new();
    for entry in &wiring.models {
        let mut model = json!({ "name": entry.display_name });
        if let Some(window) = entry.context_window {
            model["limit"] = json!({ "context": window, "output": output_limit(window) });
        }
        models.insert(entry.id.clone(), model);
    }
    models
}

/// OpenCode 格式的供应商条目（`@ai-sdk/openai-compatible`）。
pub fn opencode_provider(wiring: &Wiring) -> Value {
    json!({
        "npm": "@ai-sdk/openai-compatible",
        "name": PROVIDER_NAME,
        "options": { "baseURL": wiring.v1(), "apiKey": wiring.token },
        "models": opencode_models(wiring),
    })
}

/// 目录里的 `<名字>.jsonc`，没有就用 `<名字>.json`。
pub fn json_or_jsonc(dir: PathBuf, stem: &str) -> PathBuf {
    let jsonc = dir.join(format!("{stem}.jsonc"));
    if jsonc.exists() {
        return jsonc;
    }
    dir.join(format!("{stem}.json"))
}

/// 写入 OpenCode 格式的配置：供应商 + `model` / `small_model`。
pub fn write_opencode_style(
    path: &std::path::Path,
    wiring: &Wiring,
    set: &mut FileSet,
) -> anyhow::Result<()> {
    let root = set.json(path)?;
    object_entry(root, "provider").insert(PROVIDER_ID.to_string(), opencode_provider(wiring));
    root.insert(
        "model".to_string(),
        json!(format!("{PROVIDER_ID}/{}", wiring.default_model)),
    );
    root.insert(
        "small_model".to_string(),
        json!(format!("{PROVIDER_ID}/{}", wiring.small_model)),
    );
    Ok(())
}

// ---------------------------------------------------------------- OpenCode

fn opencode_dir(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["OPENCODE_CONFIG_DIR"], || env.xdg_config().join("opencode"))
}

fn opencode_path(env: &AgentEnv) -> PathBuf {
    json_or_jsonc(opencode_dir(env), "opencode")
}

pub const OPENCODE: AgentSpec = AgentSpec {
    id: "opencode",
    name: "OpenCode",
    binary: "opencode",
    wire: AgentWire::Chat,
    detect: |env| opencode_dir(env).exists() || env.has_binary("opencode"),
    config_path: opencode_path,
    write: |env, wiring, set| write_opencode_style(&opencode_path(env), wiring, set),
    hint: "在 OpenCode 里用 /models 选 Codex++ 下的模型",
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::agents::ModelEntry;

    pub(crate) fn sample_wiring() -> Wiring {
        Wiring {
            agent: "x".to_string(),
            root: "http://127.0.0.1:57321/gw/x".to_string(),
            token: "cpx-gw-x-1".to_string(),
            default_model: "ds/deepseek-chat".to_string(),
            small_model: "ds/deepseek-chat".to_string(),
            models: vec![
                ModelEntry {
                    id: "ds/deepseek-chat".to_string(),
                    display_name: "ds/deepseek-chat".to_string(),
                    context_window: Some(128_000),
                    reasoning_levels: Vec::new(),
                },
                ModelEntry {
                    id: "group/daily".to_string(),
                    display_name: "group/daily".to_string(),
                    context_window: None,
                    reasoning_levels: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn opencode_keeps_user_providers() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let path = opencode_path(&env);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "{\n  // mine\n  \"provider\": { \"mine\": { \"npm\": \"x\" } },\n  \"theme\": \"dark\",\n}\n",
        )
        .unwrap();
        let mut set = FileSet::new();
        (OPENCODE.write)(&env, &sample_wiring(), &mut set).unwrap();
        let changes = set.changes().unwrap();
        let text = changes[0].after.as_deref().unwrap();
        let after = super::super::files::parse_jsonc(text).unwrap();
        assert!(text.contains("// mine"));
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["provider"]["mine"]["npm"], "x");
        assert_eq!(
            after["provider"]["codexplus"]["options"]["baseURL"],
            "http://127.0.0.1:57321/gw/x/v1"
        );
        assert_eq!(after["model"], "codexplus/ds/deepseek-chat");
        assert_eq!(
            after["provider"]["codexplus"]["models"]["ds/deepseek-chat"]["limit"]["context"],
            128_000
        );
    }
}
