use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde_json::Value;

fn path() -> PathBuf {
    crate::paths::default_app_state_dir().join("relay-features.json")
}

fn load() -> anyhow::Result<BTreeMap<String, BTreeMap<String, bool>>> {
    let Ok(text) = fs::read_to_string(path()) else {
        return Ok(BTreeMap::new());
    };
    let value: Value = serde_json::from_str(&text).context("relay-features.json 解析失败")?;
    Ok(serde_json::from_value(value).context("relay-features.json 格式无效")?)
}

pub fn stabilize_prompt_prefix(profile_id: &str) -> bool {
    load()
        .ok()
        .and_then(|items| items.get(profile_id).cloned())
        .and_then(|flags| flags.get("stabilizePromptPrefix").copied())
        .unwrap_or(false)
}

pub fn set_stabilize_prompt_prefix(profile_id: &str, enabled: bool) -> anyhow::Result<()> {
    let profile_id = profile_id.trim();
    anyhow::ensure!(!profile_id.is_empty(), "供应商 ID 不能为空");
    let mut items = load()?;
    let flags = items.entry(profile_id.to_string()).or_default();
    if enabled {
        flags.insert("stabilizePromptPrefix".to_string(), true);
    } else {
        flags.remove("stabilizePromptPrefix");
        if flags.is_empty() {
            items.remove(profile_id);
        }
    }
    let path = path();
    if items.is_empty() {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::settings::atomic_write(&path, serde_json::to_string_pretty(&items)?.as_bytes())?;
    }
    Ok(())
}
