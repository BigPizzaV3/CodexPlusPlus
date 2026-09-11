use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfiguredModel {
    #[serde(default)]
    id: String,
    model: String,
    provider: String,
}

#[derive(Clone, Debug)]
pub struct RemoteModelConfig {
    pub id: String,
    pub model: String,
    pub provider: String,
}

pub fn load_remote_models(_codex_home: &Path) -> Vec<RemoteModelConfig> {
    let path = config_path();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let Ok(models) = serde_json::from_value::<Vec<ConfiguredModel>>(
        value
            .pointer("/mobile/models")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    ) else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    models
        .into_iter()
        .filter_map(|model| {
            let name = strip_context_suffix(model.model.trim());
            let provider = model.provider.trim();
            if name.is_empty()
                || name.len() > 256
                || provider.is_empty()
                || provider.len() > 256
                || !seen.insert((provider.to_owned(), name.to_owned()))
            {
                return None;
            }
            let id = if valid_id(model.id.trim()) {
                model.id.trim().to_owned()
            } else {
                format!(
                    "{:x}",
                    Sha256::digest(
                        format!("xuanplus-mobile-model-v1\n{provider}\n{name}").as_bytes()
                    )
                )
            };
            Some(RemoteModelConfig {
                id,
                model: name.to_owned(),
                provider: provider.to_owned(),
            })
        })
        .take(100)
        .collect()
}

pub fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("XUAN_PLUGINS_CONFIG") {
        return PathBuf::from(path);
    }
    config_root().join("xuan-plugins.json")
}

fn config_root() -> PathBuf {
    if let Some(path) = std::env::var_os("XUAN_HOME") {
        return PathBuf::from(path);
    }
    if cfg!(windows) {
        if let Some(path) = std::env::var_os("APPDATA") {
            return PathBuf::from(path).join("XuanPlusPlus");
        }
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".config").join("xuan-plus-plus"))
        .unwrap_or_else(|| PathBuf::from(".xuan-plus-plus"))
}

fn strip_context_suffix(value: &str) -> &str {
    let Some(open) = value.rfind('[') else {
        return value;
    };
    let Some(suffix) = value.get(open + 1..value.len().saturating_sub(1)) else {
        return value;
    };
    if !value.ends_with(']') || suffix.len() < 2 {
        return value;
    }
    let (digits, unit) = suffix.split_at(suffix.len() - 1);
    if digits.chars().all(|ch| ch.is_ascii_digit()) && matches!(unit, "K" | "k" | "M" | "m") {
        value[..open].trim_end()
    } else {
        value
    }
}

fn valid_id(value: &str) -> bool {
    (16..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_context_window_suffixes() {
        assert_eq!(strip_context_suffix("deepseek-v4[1M]"), "deepseek-v4");
        assert_eq!(strip_context_suffix("model[preview]"), "model[preview]");
    }
}
