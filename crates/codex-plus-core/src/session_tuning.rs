//! Codex 页面「上下文与压缩」（96-context-tuning.js）的后端：把 Codex 原生的会话调优键同步进
//! Codex++ 的通用配置。
//!
//! 为什么要同步：Codex++ 切换供应商 / 重启时会按「供应商快照 + 通用配置」重写 config.toml，
//! 页面只用 `config/batchWrite` 写进 live 的话，切走时它会被回存进当前供应商的快照、换到别的
//! 供应商就没了。写进通用配置后：合并时通用配置覆盖快照、回存时同值会被剥掉，于是对所有
//! 开了「使用通用配置」的供应商一致生效。
//!
//! 页面先经 Codex 的 `config/batchWrite` 写 live（Codex 自己校验、新会话立即生效），回读通过后
//! 再调这里；所以这里只认白名单里的键，并按类型和范围再校验一遍，避免写出 Codex 拒载的值
//! （例如阈值 150 会让每次 `thread/start` 都失败）。

use anyhow::Context;
use serde_json::{Map, Value, json};
use toml_edit::{DocumentMut, Item};

use crate::settings::SettingsStore;

/// 一个可同步键的取值约束。
#[derive(Clone, Copy)]
enum KeyKind {
    Text { max_chars: usize },
    Int { min: i64, max: i64 },
    Bool,
}

/// 允许经页面写进通用配置的键。只放实测过「写进 config.toml 新会话就生效」的。
const SESSION_TUNING_KEYS: &[(&str, KeyKind)] = &[
    ("compact_prompt", KeyKind::Text { max_chars: 20_000 }),
    // Codex 自己的校验是 0~100，0 等于关闭；页面用「不设置」表示关闭，所以这里从 1 起。
    ("model_post_turn_compact_threshold_percent", KeyKind::Int { min: 1, max: 100 }),
    ("tool_output_token_limit", KeyKind::Int { min: 1, max: 10_000_000 }),
    ("include_environment_context", KeyKind::Bool),
    ("include_environment_context_time", KeyKind::Bool),
    ("include_permissions_instructions", KeyKind::Bool),
    ("include_apps_instructions", KeyKind::Bool),
    ("include_collaboration_mode_instructions", KeyKind::Bool),
];

fn key_kind(key: &str) -> Option<KeyKind> {
    SESSION_TUNING_KEYS.iter().find(|(name, _)| *name == key).map(|(_, kind)| *kind)
}

fn parse_common(text: &str) -> anyhow::Result<DocumentMut> {
    text.parse::<DocumentMut>()
        .context("通用配置不是合法的 TOML，请先在 Codex++ 管理器「通用配置」里修好")
}

fn item_to_json(item: &Item) -> Value {
    let Some(value) = item.as_value() else {
        return Value::Null;
    };
    if let Some(text) = value.as_str() {
        return json!(text);
    }
    if let Some(number) = value.as_integer() {
        return json!(number);
    }
    if let Some(flag) = value.as_bool() {
        return json!(flag);
    }
    Value::Null
}

/// 把页面给的值按键的约束转成 TOML 值；`null` 表示删掉这个键。
fn toml_value(key: &str, kind: KeyKind, value: &Value) -> anyhow::Result<Option<toml_edit::Value>> {
    if value.is_null() {
        return Ok(None);
    }
    match kind {
        KeyKind::Text { max_chars } => {
            let text = value.as_str().with_context(|| format!("{key} 必须是文本"))?;
            if text.trim().is_empty() {
                return Ok(None);
            }
            if text.chars().count() > max_chars {
                anyhow::bail!("{key} 太长（最多 {max_chars} 字）");
            }
            Ok(Some(text.into()))
        }
        KeyKind::Int { min, max } => {
            let number = value.as_i64().with_context(|| format!("{key} 必须是整数"))?;
            if number < min || number > max {
                anyhow::bail!("{key} 必须在 {min} 到 {max} 之间");
            }
            Ok(Some(number.into()))
        }
        KeyKind::Bool => {
            let flag = value.as_bool().with_context(|| format!("{key} 必须是开或关"))?;
            Ok(Some(flag.into()))
        }
    }
}

fn values_of(doc: &DocumentMut) -> Value {
    let mut values = Map::new();
    for (key, _) in SESSION_TUNING_KEYS {
        values.insert((*key).to_string(), doc.get(key).map(item_to_json).unwrap_or(Value::Null));
    }
    Value::Object(values)
}

/// 当前供应商会不会套用通用配置。不会时页面要提示「设置只跟着这个供应商走」。
fn common_config_applies(settings: &crate::settings::BackendSettings) -> bool {
    !settings.relay_profiles_enabled || settings.active_relay_profile().use_common_config
}

fn state_value(settings: &crate::settings::BackendSettings, doc: &DocumentMut) -> Value {
    let active = settings.active_relay_profile();
    json!({
        "status": "ok",
        "values": values_of(doc),
        "commonConfigApplies": common_config_applies(settings),
        "activeProfileName": if active.name.trim().is_empty() { active.id.clone() } else { active.name.clone() },
    })
}

/// 切走供应商时 live 会被回存进它的快照，于是快照里留着一份通用配置值的副本。通用配置在时
/// 合并会覆盖它，但通用配置里的键改掉 / 删掉后，换回那个供应商时旧副本会复活。
/// 所以改一个键前，把各供应商快照里「和旧通用值相同」的副本删掉；值不同的是用户给单个供应商
/// 专门设的，不动。只看套用通用配置的供应商。
fn forget_snapshot_copies(settings: &mut crate::settings::BackendSettings, common: &DocumentMut, key: &str) {
    let Some(old) = common.get(key).and_then(Item::as_value).map(|value| item_to_json(&Item::Value(value.clone()))) else {
        return;
    };
    if old.is_null() {
        return;
    }
    for profile in settings.relay_profiles.iter_mut().filter(|profile| profile.use_common_config) {
        let Ok(mut snapshot) = profile.config_contents.parse::<DocumentMut>() else {
            continue;
        };
        if snapshot.get(key).map(item_to_json).as_ref() != Some(&old) {
            continue;
        }
        snapshot.remove(key);
        profile.config_contents = crate::relay_config::normalize_config_text(&snapshot.to_string());
    }
}

/// `/session-tuning/read`：通用配置里这些键当前的值。
pub fn read(store: &SettingsStore) -> anyhow::Result<Value> {
    let settings = store.load().context("读取 Codex++ 设置失败")?;
    let doc = parse_common(&settings.relay_common_config_contents)?;
    Ok(state_value(&settings, &doc))
}

/// `/session-tuning/write`：`{ edits: [{ key, value }] }`，`value: null` 删除。
/// 白名单外的键整批拒绝，不做部分写入。
pub fn write(store: &SettingsStore, payload: &Value) -> anyhow::Result<Value> {
    let edits = payload
        .get("edits")
        .and_then(Value::as_array)
        .context("缺少 edits")?;
    let mut planned = Vec::with_capacity(edits.len());
    for edit in edits {
        let key = edit.get("key").and_then(Value::as_str).context("edits 里缺少 key")?;
        let kind = key_kind(key).with_context(|| format!("{key} 不能在这里修改"))?;
        let value = toml_value(key, kind, edit.get("value").unwrap_or(&Value::Null))?;
        planned.push((key.to_string(), value));
    }
    // 设置文件被别处同时改了（revision 冲突）就重读再来一次。
    let mut last_error = None;
    for _ in 0..2 {
        let mut settings = store.load().context("读取 Codex++ 设置失败")?;
        let mut doc = parse_common(&settings.relay_common_config_contents)?;
        for (key, value) in &planned {
            forget_snapshot_copies(&mut settings, &doc, key);
            match value {
                Some(value) => doc[key.as_str()] = Item::Value(value.clone()),
                None => {
                    doc.remove(key);
                }
            }
        }
        settings.relay_common_config_contents = crate::relay_config::normalize_config_text(&doc.to_string());
        match store.save(&settings) {
            Ok(()) => {
                crate::settings::notify_external_settings_change();
                let saved = store.load().context("保存后读取 Codex++ 设置失败")?;
                let saved_doc = parse_common(&saved.relay_common_config_contents)?;
                return Ok(state_value(&saved, &saved_doc));
            }
            Err(error) if format!("{error:#}").contains("settings_revision_conflict") => last_error = Some(error),
            Err(error) => return Err(error).context("保存通用配置失败"),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("保存通用配置失败")))
}
