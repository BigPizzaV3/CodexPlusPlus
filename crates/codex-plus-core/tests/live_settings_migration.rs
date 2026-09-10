//! 拿真实的 settings.json 过一遍迁移，确认「升级不丢配置」。
//!
//! 这个测试只在设置了 `CODEX_PLUS_LIVE_SETTINGS` 时才跑（默认 skip），因为它
//! 读的是本机真实文件。刻意用文件副本、并且只写副本，原文件全程只读。
//!
//!   CODEX_PLUS_LIVE_SETTINGS=~/.codex-session-delete/settings.json \
//!     cargo test -p codex-plus-core --test live_settings_migration

use codex_plus_core::settings::{BackendSettings, SettingsStore};
use codex_plus_core::tools::ToolId;

fn live_settings_path() -> Option<std::path::PathBuf> {
    let raw = std::env::var_os("CODEX_PLUS_LIVE_SETTINGS")?;
    let path = std::path::PathBuf::from(raw);
    path.is_file().then_some(path)
}

/// 迁移前 → 迁移后，扁平字段一个都不能变；分片要跟扁平字段一致。
#[test]
fn migrating_a_real_settings_file_loses_nothing() {
    let Some(live) = live_settings_path() else {
        eprintln!("跳过：未设置 CODEX_PLUS_LIVE_SETTINGS");
        return;
    };

    let original_text = std::fs::read_to_string(&live).expect("读真实 settings.json");
    let original: serde_json::Value = serde_json::from_str(&original_text).expect("解析真实 settings");

    // 只在临时目录里动手，原文件只读。
    let temp = tempfile::tempdir().unwrap();
    let working = temp.path().join("settings.json");
    std::fs::write(&working, &original_text).unwrap();

    let store = SettingsStore::new(working.clone());
    let loaded = store.load().expect("迁移前应当能正常加载");
    store.save(&loaded).expect("保存应当成功");

    let migrated_text = std::fs::read_to_string(&working).unwrap();
    let migrated: serde_json::Value = serde_json::from_str(&migrated_text).unwrap();

    // 1) 老版本读的扁平字段：BackendSettings 认识的键逐个比对。
    //
    // 两类已知的、与本次改动无关的既有行为，这里显式列出而不是绕过：
    //   - 不认识的键会被 `normalize_settings_config_sections` 丢掉（任何一次保存都会）
    //   - `relay*ConfigContents` 是配置文本，保存时会做 TOML 归一化（删空行等）
    // 除这两类外，其余字段必须逐字节不变。
    const TEXT_FIELDS: &[&str] = &[
        "relayCommonConfigContents",
        "relayContextConfigContents",
    ];
    let original_obj = original.as_object().unwrap();
    let migrated_obj = migrated.as_object().unwrap();
    let mut unknown_keys = Vec::new();
    let mut normalized_keys = Vec::new();
    for (key, before) in original_obj {
        // tools / activeTool 是本次新增的，原文件里本来就没有。
        if key == "tools" || key == "activeTool" {
            continue;
        }
        match migrated_obj.get(key) {
            Some(after) if TEXT_FIELDS.contains(&key.as_str()) => {
                let (Some(before_text), Some(after_text)) = (before.as_str(), after.as_str())
                else {
                    panic!("{key} 应当是字符串");
                };
                // 归一化只允许丢空行/行尾空白，内容行必须完整保留。
                let content_lines = |text: &str| -> Vec<String> {
                    text.lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .map(ToString::to_string)
                        .collect()
                };
                assert_eq!(
                    content_lines(after_text),
                    content_lines(before_text),
                    "{key} 归一化后内容行发生了变化"
                );
                normalized_keys.push(key.clone());
            }
            Some(after) => assert_eq!(
                after, before,
                "扁平字段 {key} 在迁移后发生了变化：{before:?} → {after:?}"
            ),
            None => unknown_keys.push(key.clone()),
        }
    }
    if !normalized_keys.is_empty() {
        eprintln!("以下配置文本字段做了 TOML 归一化（既有行为）：{normalized_keys:?}");
    }
    if !unknown_keys.is_empty() {
        eprintln!(
            "注意：以下键不在 BackendSettings 里，保存时会被丢弃（既有行为）：{unknown_keys:?}"
        );
    }

    // 2) 分片被补出来，并且与扁平字段一致。
    let codex = migrated
        .get("tools")
        .and_then(|tools| tools.get("codex"))
        .expect("迁移后必须有 tools.codex");
    assert_eq!(codex.get("activeRelayId"), original.get("activeRelayId"));
    assert_eq!(
        codex.get("relayProfiles").and_then(|value| value.as_array()).map(Vec::len),
        original.get("relayProfiles").and_then(|value| value.as_array()).map(Vec::len),
        "分片里的供应商数量必须与扁平字段一致"
    );

    // 3) 再读一次应当得到与迁移前语义等价的配置（幂等）。
    let reloaded: BackendSettings = store.load().unwrap();
    assert_eq!(reloaded.active_relay_id, loaded.active_relay_id);
    assert_eq!(reloaded.relay_profiles.len(), loaded.relay_profiles.len());
    assert_eq!(
        reloaded.tool_config(&ToolId::Codex).relay_profiles.len(),
        loaded.relay_profiles.len()
    );

    eprintln!(
        "迁移检查通过：{} 个顶层字段、{} 个供应商，往返无损",
        original_obj.len(),
        loaded.relay_profiles.len()
    );
}
