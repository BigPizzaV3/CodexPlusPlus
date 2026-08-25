//! MCP 服务器配置的结构化读写。
//!
//! 两个用途：把 `[mcp_servers.<id>]` 表体拆成表单字段再拼回去，以及把社区文档里
//! 常见的 Claude 风格 JSON 转成 Codex 的 TOML。
//!
//! # 为什么字段映射要显式写
//!
//! codex 的 `RawMcpServerConfig` 没有 `deny_unknown_fields`——未知字段会被 serde
//! **静默丢弃**，不报错也不生效。用 `codex -c '<key>=<错误类型的值>'` 实测（真字段
//! 会因类型不符报错，未知字段则毫无反应）：
//!
//! | 字段 | 结论 |
//! |---|---|
//! | `command` / `cwd` / `url` / `bearer_token` | 真字段（string）|
//! | `env` / `http_headers` | 真字段（map）|
//! | `args` | 真字段（list）|
//! | `enabled` | 真字段（bool）|
//! | `startup_timeout_sec` | 真字段（f64）|
//! | `type` / `headers` / `description` | **未知，静默丢弃** |
//!
//! 所以 Claude 风格 JSON 里的 `headers` 必须重命名成 `http_headers`，否则认证头
//! 会无声消失——配置看着是对的，运行时才失败。这类改写都会记进 warnings 告诉用户，
//! 而不是悄悄处理掉。

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue};

/// 表单直接支持的键。其余键一律进 `extra_toml` 原样保留，
/// 免得用户手写的 oauth / scopes 之类被表单一存就抹掉。
const FORM_KEYS: [&str; 9] = [
    "command",
    "args",
    "env",
    "cwd",
    "url",
    "http_headers",
    "bearer_token",
    "startup_timeout_sec",
    "enabled",
];

/// JSON 里出现就丢掉的键，附带说明用于 warning。
const DROPPED_JSON_KEYS: [(&str, &str); 3] = [
    ("type", "codex 由 url 是否存在推断传输方式"),
    ("description", "codex 不读这个字段"),
    ("disabled", "codex 用 enabled 表达启停"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    #[default]
    Stdio,
    Http,
}

/// MCP 条目的表单视图。空字符串统一表示「不写这个键」，
/// 与 RelayProfile 里 context_window / auto_compact_limit 的处理一致。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerForm {
    #[serde(default)]
    pub transport: McpTransport,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// 有序 Vec 而不是 map：保住用户写的顺序，diff 才稳定。
    #[serde(default)]
    pub env: Vec<McpKeyValue>,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub http_headers: Vec<McpKeyValue>,
    #[serde(default)]
    pub bearer_token: String,
    #[serde(default)]
    pub startup_timeout_sec: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 表单不认识的键，原样存放，保存时合并回去。
    #[serde(default)]
    pub extra_toml: String,
}

fn default_true() -> bool {
    true
}

/// 手写 Default 而不是 derive：derive 会给 `enabled` 填 `false`，
/// 绕过 serde 的 `default_true`，于是每条新建的 MCP 都会莫名带上
/// `enabled = false`——建完就是停用的。
impl Default for McpServerForm {
    fn default() -> Self {
        Self {
            transport: McpTransport::default(),
            command: String::new(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: String::new(),
            url: String::new(),
            http_headers: Vec::new(),
            bearer_token: String::new(),
            startup_timeout_sec: String::new(),
            enabled: true,
            extra_toml: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct McpKeyValue {
    pub key: String,
    pub value: String,
}

impl McpKeyValue {
    fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// JSON 导入里的一条 server。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpJsonEntry {
    pub id: String,
    pub toml_body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpJsonImport {
    pub entries: Vec<McpJsonEntry>,
    /// 改写和丢弃都如实回报，不做无声处理。
    pub warnings: Vec<String>,
}

/// 把 `[mcp_servers.<id>]` 的表体拆成表单字段。
///
/// 表单不认识的键收进 `extra_toml`；传输方式由 `url` 是否存在推断。
pub fn parse_mcp_toml_body(body: &str) -> anyhow::Result<McpServerForm> {
    let doc: DocumentMut = body.parse().context("MCP 配置不是合法的 TOML")?;
    let table = doc.as_table();

    let mut form = McpServerForm {
        command: string_field(table, "command"),
        args: string_list_field(table, "args"),
        env: map_field(table, "env"),
        cwd: string_field(table, "cwd"),
        url: string_field(table, "url"),
        http_headers: map_field(table, "http_headers"),
        bearer_token: string_field(table, "bearer_token"),
        startup_timeout_sec: number_field(table, "startup_timeout_sec"),
        enabled: table.get("enabled").and_then(Item::as_bool).unwrap_or(true),
        ..Default::default()
    };
    form.transport = if form.url.is_empty() {
        McpTransport::Stdio
    } else {
        McpTransport::Http
    };

    // 剩下的键原样留着，保存时再合并回去
    let mut extra = doc.clone();
    for key in FORM_KEYS {
        extra.as_table_mut().remove(key);
    }
    let extra_text = extra.to_string();
    form.extra_toml = if extra_text.trim().is_empty() {
        String::new()
    } else {
        ensure_trailing_newline(extra_text)
    };

    Ok(form)
}

/// 表单字段拼回 TOML 表体。空值不写；`extra_toml` 里的键最后合并进来。
pub fn build_mcp_toml_body(form: &McpServerForm) -> anyhow::Result<String> {
    let mut doc = DocumentMut::new();
    let table = doc.as_table_mut();

    match form.transport {
        McpTransport::Stdio => {
            set_string(table, "command", &form.command);
            if !form.args.is_empty() {
                let mut array = Array::new();
                for arg in &form.args {
                    array.push(arg.as_str());
                }
                table["args"] = Item::Value(TomlValue::Array(array));
            }
            set_string(table, "cwd", &form.cwd);
            set_inline_map(table, "env", &form.env);
        }
        McpTransport::Http => {
            set_string(table, "url", &form.url);
            set_inline_map(table, "http_headers", &form.http_headers);
            set_string(table, "bearer_token", &form.bearer_token);
        }
    }

    let timeout = form.startup_timeout_sec.trim();
    if !timeout.is_empty() {
        let parsed: f64 = timeout
            .parse()
            .with_context(|| format!("启动超时必须是数字：{timeout}"))?;
        if !parsed.is_finite() || parsed <= 0.0 {
            anyhow::bail!("启动超时必须是正数：{timeout}");
        }
        // 整数写成整数：toml_edit::value(30.0) 会产出 `30.0`，
        // 而 codex 那边期望 f64，两种都收，但配置文件里 `30` 更干净。
        if parsed.fract() == 0.0 && parsed.abs() < 1e15 {
            table["startup_timeout_sec"] = toml_edit::value(parsed as i64);
        } else {
            table["startup_timeout_sec"] = toml_edit::value(parsed);
        }
    }

    // enabled = true 是默认值，不写进去省得每条都带一行噪音
    if !form.enabled {
        table["enabled"] = toml_edit::value(false);
    }

    if !form.extra_toml.trim().is_empty() {
        let extra: DocumentMut = form
            .extra_toml
            .parse()
            .context("高级 TOML 片段不是合法的 TOML")?;
        for (key, item) in extra.as_table().iter() {
            // 表单字段优先：extra 里若残留同名键，以表单为准
            if !FORM_KEYS.contains(&key) {
                table.insert(key, item.clone());
            }
        }
    }

    let text = doc.to_string();
    Ok(if text.trim().is_empty() {
        String::new()
    } else {
        ensure_trailing_newline(text)
    })
}

/// 解析社区文档里常见的 MCP JSON。
///
/// 接受四种外层形状：`{"mcpServers":{…}}`（Claude 标准）、`{"servers":{…}}`
/// （VS Code）、裸的 `{id: cfg}`，以及单个 server 对象（此时 id 留空由调用方填）。
pub fn parse_mcp_servers_json(json: &str) -> anyhow::Result<McpJsonImport> {
    let trimmed = json.trim();
    if trimmed.is_empty() {
        anyhow::bail!("请先粘贴 MCP 配置 JSON");
    }
    let root: Value = serde_json::from_str(trimmed).context("不是合法的 JSON")?;
    let Value::Object(root) = root else {
        anyhow::bail!("JSON 顶层必须是对象");
    };

    let servers = if let Some(Value::Object(map)) = root.get("mcpServers") {
        map.clone()
    } else if let Some(Value::Object(map)) = root.get("servers") {
        map.clone()
    } else if looks_like_single_server(&root) {
        // 单个 server 对象，没有 id，用占位 id 让用户在界面上改
        let mut map = Map::new();
        map.insert("mcp-server".to_string(), Value::Object(root));
        map
    } else {
        root
    };

    if servers.is_empty() {
        anyhow::bail!("JSON 里没有找到任何 MCP 服务器");
    }

    let mut import = McpJsonImport::default();
    for (id, config) in servers {
        let Value::Object(config) = config else {
            import.warnings.push(format!("已跳过 {id}：配置不是对象"));
            continue;
        };
        match json_server_to_toml(&id, &config, &mut import.warnings) {
            Ok(toml_body) => import.entries.push(McpJsonEntry { id, toml_body }),
            Err(error) => import.warnings.push(format!("已跳过 {id}：{error}")),
        }
    }

    if import.entries.is_empty() {
        anyhow::bail!("JSON 里没有可导入的 MCP 服务器");
    }
    Ok(import)
}

/// 只有 command 或 url、且没有嵌套的 server 对象，就当成单个 server。
fn looks_like_single_server(root: &Map<String, Value>) -> bool {
    (root.contains_key("command") || root.contains_key("url"))
        && !root.values().any(|value| {
            value
                .as_object()
                .map(|obj| obj.contains_key("command") || obj.contains_key("url"))
                .unwrap_or(false)
        })
}

fn json_server_to_toml(
    id: &str,
    config: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> anyhow::Result<String> {
    let mut form = McpServerForm::default();

    for (key, description) in DROPPED_JSON_KEYS {
        if config.contains_key(key) {
            // disabled 语义上等价于 enabled 取反，别只是丢掉
            if key == "disabled"
                && let Some(disabled) = config.get(key).and_then(Value::as_bool)
            {
                form.enabled = !disabled;
                warnings.push(format!("{id}：已把 disabled 转成 enabled"));
                continue;
            }
            warnings.push(format!("{id}：已忽略 {key} 字段（{description}）"));
        }
    }

    form.command = json_string(config, "command");
    form.args = config
        .get("args")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| json_scalar_to_string(item))
                .collect()
        })
        .unwrap_or_default();
    form.env = json_map(config, "env");
    form.cwd = json_string(config, "cwd");
    form.url = json_string(config, "url");
    form.bearer_token = json_string(config, "bearer_token");

    // 关键改写：Claude 风格的 headers 在 codex 里叫 http_headers，
    // 不改名的话认证头会被静默丢弃，运行时才发现连不上。
    form.http_headers = if config.contains_key("http_headers") {
        json_map(config, "http_headers")
    } else if config.contains_key("headers") {
        warnings.push(format!("{id}：headers 已重命名为 http_headers"));
        json_map(config, "headers")
    } else {
        Vec::new()
    };

    if let Some(timeout) = config.get("startup_timeout_sec").and_then(Value::as_f64) {
        form.startup_timeout_sec = format_number(timeout);
    }
    if let Some(enabled) = config.get("enabled").and_then(Value::as_bool) {
        form.enabled = enabled;
    }

    form.transport = if form.url.is_empty() {
        McpTransport::Stdio
    } else {
        McpTransport::Http
    };
    if form.transport == McpTransport::Stdio && form.command.trim().is_empty() {
        anyhow::bail!("缺少 command 或 url");
    }

    build_mcp_toml_body(&form)
}

fn json_string(config: &Map<String, Value>, key: &str) -> String {
    config
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn json_map(config: &Map<String, Value>, key: &str) -> Vec<McpKeyValue> {
    config
        .get(key)
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(k, v)| McpKeyValue::new(k, json_scalar_to_string(v)))
                .collect()
        })
        .unwrap_or_default()
}

/// env / args 里偶尔混进数字或布尔，转成字符串而不是整条丢掉。
fn json_scalar_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => String::new(),
    }
}

fn string_field(table: &Table, key: &str) -> String {
    table
        .get(key)
        .and_then(Item::as_str)
        .unwrap_or_default()
        .to_string()
}

fn number_field(table: &Table, key: &str) -> String {
    match table.get(key) {
        Some(item) if item.as_float().is_some() => format_number(item.as_float().unwrap_or(0.0)),
        Some(item) if item.as_integer().is_some() => item.as_integer().unwrap_or(0).to_string(),
        _ => String::new(),
    }
}

fn string_list_field(table: &Table, key: &str) -> Vec<String> {
    table
        .get(key)
        .and_then(Item::as_array)
        .map(|array| {
            array
                .iter()
                .map(|value| toml_scalar_to_string(value))
                .collect()
        })
        .unwrap_or_default()
}

/// env / http_headers 既可能写成行内表也可能是独立子表，两种都要认。
fn map_field(table: &Table, key: &str) -> Vec<McpKeyValue> {
    let Some(item) = table.get(key) else {
        return Vec::new();
    };
    let Some(table_like) = item.as_table_like() else {
        return Vec::new();
    };
    table_like
        .iter()
        .map(|(k, v)| {
            let value = v.as_value().map(toml_scalar_to_string).unwrap_or_default();
            McpKeyValue::new(k, value)
        })
        .collect()
}

fn toml_scalar_to_string(value: &TomlValue) -> String {
    match value {
        TomlValue::String(text) => text.value().to_string(),
        TomlValue::Integer(number) => number.value().to_string(),
        TomlValue::Float(number) => format_number(*number.value()),
        TomlValue::Boolean(flag) => flag.value().to_string(),
        _ => String::new(),
    }
}

fn set_string(table: &mut Table, key: &str, value: &str) {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        table[key] = toml_edit::value(trimmed);
    }
}

fn set_inline_map(table: &mut Table, key: &str, pairs: &[McpKeyValue]) {
    let usable: Vec<&McpKeyValue> = pairs
        .iter()
        .filter(|pair| !pair.key.trim().is_empty())
        .collect();
    if usable.is_empty() {
        return;
    }
    let mut inline = InlineTable::new();
    for pair in usable {
        inline.insert(pair.key.trim(), pair.value.as_str().into());
    }
    table[key] = Item::Value(TomlValue::InlineTable(inline));
}

/// 整数值不带 `.0` 后缀，避免 30 被写成 30.0。
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

fn ensure_trailing_newline(mut text: String) -> String {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(pairs: &[(&str, &str)]) -> Vec<McpKeyValue> {
        pairs
            .iter()
            .map(|(k, v)| McpKeyValue::new(*k, *v))
            .collect()
    }

    #[test]
    fn parses_a_stdio_entry_into_form_fields() {
        let form = parse_mcp_toml_body(
            r#"command = "npx"
args = ["-y", "@upstash/context7-mcp"]
env = { KEY = "value" }
"#,
        )
        .unwrap();

        assert_eq!(form.transport, McpTransport::Stdio);
        assert_eq!(form.command, "npx");
        assert_eq!(form.args, vec!["-y", "@upstash/context7-mcp"]);
        assert_eq!(form.env, kv(&[("KEY", "value")]));
        assert!(form.enabled);
        assert!(form.extra_toml.is_empty());
    }

    /// url 存在就是 HTTP，codex 自己也是这么区分的。
    #[test]
    fn infers_http_transport_from_url() {
        let form = parse_mcp_toml_body(
            r#"url = "https://example.com/mcp"
http_headers = { "X-Api-Key" = "secret" }
"#,
        )
        .unwrap();

        assert_eq!(form.transport, McpTransport::Http);
        assert_eq!(form.url, "https://example.com/mcp");
        assert_eq!(form.http_headers, kv(&[("X-Api-Key", "secret")]));
    }

    /// env 写成独立子表（[mcp_servers.x.env]）时同样要认。
    #[test]
    fn reads_env_written_as_a_child_table() {
        let form = parse_mcp_toml_body(
            r#"command = "node"

[env]
A = "1"
B = "2"
"#,
        )
        .unwrap();

        assert_eq!(form.env, kv(&[("A", "1"), ("B", "2")]));
    }

    /// 表单一存就把用户手写的高级配置抹掉是最难查的那类 bug，这里钉死。
    #[test]
    fn round_trip_preserves_fields_the_form_does_not_know() {
        let original = r#"command = "npx"
scopes = ["read", "write"]

[oauth]
client_id = "abc"
"#;

        let form = parse_mcp_toml_body(original).unwrap();
        assert!(form.extra_toml.contains("scopes"));
        assert!(form.extra_toml.contains("[oauth]"));

        let rebuilt = build_mcp_toml_body(&form).unwrap();
        assert!(rebuilt.contains(r#"command = "npx""#));
        assert!(rebuilt.contains("scopes"));
        assert!(rebuilt.contains("[oauth]"));
        assert!(rebuilt.contains(r#"client_id = "abc""#));
    }

    #[test]
    fn empty_fields_are_not_written() {
        let body = build_mcp_toml_body(&McpServerForm {
            transport: McpTransport::Stdio,
            command: "npx".to_string(),
            ..Default::default()
        })
        .unwrap();

        assert_eq!(body, "command = \"npx\"\n");
        assert!(!body.contains("args"));
        assert!(!body.contains("env"));
        assert!(!body.contains("cwd"));
        // enabled = true 是默认，不该写出来
        assert!(!body.contains("enabled"));
    }

    #[test]
    fn disabled_entries_write_the_enabled_flag() {
        let body = build_mcp_toml_body(&McpServerForm {
            command: "npx".to_string(),
            enabled: false,
            ..Default::default()
        })
        .unwrap();

        assert!(body.contains("enabled = false"));
    }

    #[test]
    fn http_transport_only_writes_http_fields() {
        let body = build_mcp_toml_body(&McpServerForm {
            transport: McpTransport::Http,
            // 传输切成 HTTP 后，stdio 那些字段不该跟着写出去
            command: "npx".to_string(),
            args: vec!["-y".to_string()],
            url: "https://example.com/mcp".to_string(),
            http_headers: kv(&[("X-Key", "v")]),
            ..Default::default()
        })
        .unwrap();

        assert!(body.contains("url ="));
        assert!(body.contains("http_headers"));
        assert!(!body.contains("command"));
        assert!(!body.contains("args"));
    }

    #[test]
    fn startup_timeout_rejects_non_numeric_values() {
        let error = build_mcp_toml_body(&McpServerForm {
            command: "npx".to_string(),
            startup_timeout_sec: "abc".to_string(),
            ..Default::default()
        })
        .unwrap_err();

        assert!(error.to_string().contains("启动超时"));
    }

    #[test]
    fn integer_timeouts_do_not_gain_a_decimal_suffix() {
        let body = build_mcp_toml_body(&McpServerForm {
            command: "npx".to_string(),
            startup_timeout_sec: "30".to_string(),
            ..Default::default()
        })
        .unwrap();

        assert!(body.contains("startup_timeout_sec = 30"));
        assert!(!body.contains("30.0"));
    }

    /// 最关键的一条：headers 不改名的话认证头会被 codex 静默丢弃。
    #[test]
    fn json_import_renames_headers_to_http_headers_and_warns() {
        let import = parse_mcp_servers_json(
            r#"{"mcpServers":{"remote":{"url":"https://example.com/mcp","headers":{"Authorization":"Bearer x"}}}}"#,
        )
        .unwrap();

        assert_eq!(import.entries.len(), 1);
        let body = &import.entries[0].toml_body;
        assert!(body.contains("http_headers"));
        assert!(body.contains("Authorization"));
        assert!(!body.contains("\nheaders"));
        assert!(
            import
                .warnings
                .iter()
                .any(|warning| warning.contains("http_headers"))
        );
    }

    #[test]
    fn json_import_drops_unknown_keys_and_says_so() {
        let import = parse_mcp_servers_json(
            r#"{"mcpServers":{"fetch":{"type":"stdio","command":"uvx","args":["mcp-server-fetch"],"description":"demo"}}}"#,
        )
        .unwrap();

        let body = &import.entries[0].toml_body;
        assert!(body.contains(r#"command = "uvx""#));
        assert!(!body.contains("type"));
        assert!(!body.contains("description"));
        assert!(import.warnings.iter().any(|w| w.contains("type")));
        assert!(import.warnings.iter().any(|w| w.contains("description")));
    }

    /// disabled 是语义等价的，转成 enabled 而不是直接丢。
    #[test]
    fn json_import_converts_disabled_into_enabled() {
        let import =
            parse_mcp_servers_json(r#"{"mcpServers":{"x":{"command":"npx","disabled":true}}}"#)
                .unwrap();

        assert!(import.entries[0].toml_body.contains("enabled = false"));
        assert!(import.warnings.iter().any(|w| w.contains("disabled")));
    }

    #[test]
    fn json_import_accepts_the_common_wrapper_shapes() {
        let claude = parse_mcp_servers_json(r#"{"mcpServers":{"a":{"command":"x"}}}"#).unwrap();
        assert_eq!(claude.entries[0].id, "a");

        let vscode = parse_mcp_servers_json(r#"{"servers":{"b":{"command":"x"}}}"#).unwrap();
        assert_eq!(vscode.entries[0].id, "b");

        let bare = parse_mcp_servers_json(r#"{"c":{"command":"x"}}"#).unwrap();
        assert_eq!(bare.entries[0].id, "c");

        let single = parse_mcp_servers_json(r#"{"command":"x","args":["y"]}"#).unwrap();
        assert_eq!(single.entries.len(), 1);
        assert!(single.entries[0].toml_body.contains(r#"command = "x""#));
    }

    #[test]
    fn json_import_handles_multiple_servers_at_once() {
        let import = parse_mcp_servers_json(
            r#"{"mcpServers":{
                "a":{"command":"npx","args":["-y","pkg-a"]},
                "b":{"url":"https://b.example/mcp"}
            }}"#,
        )
        .unwrap();

        assert_eq!(import.entries.len(), 2);
        let ids: Vec<&str> = import.entries.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
    }

    /// 一条坏的不该让整批导入失败。
    #[test]
    fn json_import_skips_bad_entries_but_keeps_the_rest() {
        let import = parse_mcp_servers_json(
            r#"{"mcpServers":{"good":{"command":"npx"},"bad":{"note":"没有 command 也没有 url"}}}"#,
        )
        .unwrap();

        assert_eq!(import.entries.len(), 1);
        assert_eq!(import.entries[0].id, "good");
        assert!(import.warnings.iter().any(|w| w.contains("bad")));
    }

    #[test]
    fn json_import_stringifies_non_string_env_values() {
        let import =
            parse_mcp_servers_json(r#"{"mcpServers":{"x":{"command":"n","env":{"PORT":8080}}}}"#)
                .unwrap();

        assert!(import.entries[0].toml_body.contains(r#"PORT = "8080""#));
    }

    #[test]
    fn json_import_rejects_input_it_cannot_use() {
        assert!(parse_mcp_servers_json("").is_err());
        assert!(parse_mcp_servers_json("not json").is_err());
        assert!(parse_mcp_servers_json("[]").is_err());
        assert!(parse_mcp_servers_json(r#"{"mcpServers":{}}"#).is_err());
    }

    /// 导入产物必须能被我们自己的表单解析器读回来。
    #[test]
    fn imported_bodies_parse_back_into_the_form() {
        let import = parse_mcp_servers_json(
            r#"{"mcpServers":{"x":{"command":"npx","args":["-y","pkg"],"env":{"K":"V"}}}}"#,
        )
        .unwrap();

        let form = parse_mcp_toml_body(&import.entries[0].toml_body).unwrap();
        assert_eq!(form.command, "npx");
        assert_eq!(form.args, vec!["-y", "pkg"]);
        assert_eq!(form.env, kv(&[("K", "V")]));
    }
}
