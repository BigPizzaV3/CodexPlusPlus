//! 诊断报告的脱敏。
//!
//! 报告是给用户整段贴进 issue 的，里面的设置不能带任何凭据（#3129：贴出来的报告里有
//! 各家中转的明文 Key 和 ChatGPT 的登录 token）。这里按键名识别 Key / Token / 密码，
//! 内嵌的 `auth.json`、`config.toml` 文本解析后同样处理，解析不了的整段走兜底脱敏。

use serde_json::{Map, Value};

use crate::config_doctor::{mask_secret, redact_secrets};

/// 键名（去掉大小写、下划线和连字符后）里含这些词的，值按凭据处理。
const SECRET_WORDS: &[&str] = &[
    "apikey",
    "token",
    "secret",
    "password",
    "passwd",
    "authorization",
    "cookie",
    "credential",
    "privatekey",
    "accesskey",
];

/// 以这些词结尾的键存的是名字、编号或位置，不是凭据本身（`apiKeyEnv`、`activeApiKeyId`）。
const PLAIN_SUFFIXES: &[&str] = &["env", "envvar", "envkey", "id", "ids", "name", "header", "path", "file"];

/// 这些表里的值不看键名、整体按凭据处理：MCP 的 `env`、各类请求头（键名是用户起的，认不全）。
const SECRET_TABLES: &[&str] = &["env", "headers", "httpheaders", "envhttpheaders"];

/// 当前这一层的值怎么处理。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 键名看不出是凭据：只去掉一眼能认出的 Key 和网址里的账号密码。
    Plain,
    /// 在一个凭据字段下面（`apiKeys`、`tokens`）：打码，名字、编号这类字段除外。
    Secret,
    /// 在 [`SECRET_TABLES`] 里：全部打码。
    Table,
}

/// 把要放进诊断报告的 JSON 脱敏。结构和非凭据字段原样保留。
pub fn redact_report_value(value: Value) -> Value {
    redact(value, Mode::Plain)
}

fn redact(value: Value, mode: Mode) -> Value {
    match value {
        Value::String(text) if mode == Mode::Plain => Value::String(redact_plain_text(&text)),
        Value::String(text) => Value::String(mask_secret(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(|item| redact(item, mode)).collect()),
        Value::Object(object) => Value::Object(redact_object(object, mode)),
        other => other,
    }
}

fn redact_object(object: Map<String, Value>, mode: Mode) -> Map<String, Value> {
    // `{ key, value }` 形状的一行（自定义请求头）：名字留着，值按名字判断。
    if let (Some(Value::String(key)), Some(Value::String(_))) = (object.get("key"), object.get("value")) {
        let value_mode = match mode {
            Mode::Plain if has_secret_word(&normalized(key)) => Mode::Secret,
            mode => mode,
        };
        return object
            .into_iter()
            .map(|(name, value)| {
                let value = if name == "value" { redact(value, value_mode) } else { value };
                (name, value)
            })
            .collect();
    }
    object
        .into_iter()
        .map(|(name, value)| {
            let key = normalized(&name);
            let value = match value {
                Value::String(text) if key.ends_with("contents") => redact_embedded_file(&text),
                value => redact(value, child_mode(mode, &key)),
            };
            (name, value)
        })
        .collect()
}

fn child_mode(parent: Mode, key: &str) -> Mode {
    if parent == Mode::Table || SECRET_TABLES.contains(&key) {
        Mode::Table
    } else if PLAIN_SUFFIXES.iter().any(|suffix| key.ends_with(suffix)) {
        Mode::Plain
    } else if parent == Mode::Secret || has_secret_word(key) {
        Mode::Secret
    } else {
        Mode::Plain
    }
}

fn normalized(key: &str) -> String {
    key.chars()
        .filter(|c| !matches!(c, '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn has_secret_word(key: &str) -> bool {
    SECRET_WORDS.iter().any(|word| key.contains(word))
}

fn is_secret_key(key: &str) -> bool {
    has_secret_word(key) && !PLAIN_SUFFIXES.iter().any(|suffix| key.ends_with(suffix))
}

/// 设置里内嵌的文件正文（`authContents`、`configContents` 等）：解析成结构后按同样的规则脱敏。
fn redact_embedded_file(text: &str) -> Value {
    if text.trim().is_empty() {
        return Value::String(text.to_string());
    }
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return redact(value, Mode::Plain);
    }
    if let Some(value) = toml::from_str::<toml::Value>(text)
        .ok()
        .and_then(|value| serde_json::to_value(value).ok())
    {
        return redact(value, Mode::Plain);
    }
    Value::String(redact_secrets(text))
}

/// 键名看不出是凭据的字符串：去掉网址里的账号密码和查询串里的 Key，以及一眼能认出的 Key 形状。
fn redact_plain_text(text: &str) -> String {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("sk-") || lower.starts_with("bearer ") {
        return mask_secret(trimmed);
    }
    let Some((scheme, rest)) = text.split_once("://") else {
        return text.to_string();
    };
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
        return text.to_string();
    }
    let (address, query) = match rest.split_once('?') {
        Some((address, query)) => (address, Some(query)),
        None => (rest, None),
    };
    let authority_end = address.find('/').unwrap_or(address.len());
    let (authority, path) = address.split_at(authority_end);
    let authority = match authority.rsplit_once('@') {
        Some((_, host)) => format!("***@{host}"),
        None => authority.to_string(),
    };
    let mut out = format!("{scheme}://{authority}{path}");
    if let Some(query) = query {
        let (query, fragment) = match query.split_once('#') {
            Some((query, fragment)) => (query, Some(fragment)),
            None => (query, None),
        };
        let pairs: Vec<String> = query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((name, value)) if !value.is_empty() && is_secret_query_name(name) => format!("{name}=***"),
                _ => pair.to_string(),
            })
            .collect();
        out.push('?');
        out.push_str(&pairs.join("&"));
        if let Some(fragment) = fragment {
            out.push('#');
            out.push_str(fragment);
        }
    }
    out
}

fn is_secret_query_name(name: &str) -> bool {
    let key = normalized(name);
    key == "key" || is_secret_key(&key)
}
