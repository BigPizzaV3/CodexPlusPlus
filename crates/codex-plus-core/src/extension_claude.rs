//! Claude Desktop Mods 的声明式适配器。只运行清单显式声明的后台方法，不向 Claude 暴露 Codex 页面桥。

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::user_scripts::UserScriptManager;

pub const MCP_FLAG: &str = "--codex-plus-claude-extension-mcp";
pub const SERVER_NAME: &str = "codexpp-extensions";
pub const PLUGIN_NAME: &str = "codexpp-extension-host";
pub const TOOL_NAME: &str = "codexpp_extension_call";
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const REGISTER_TEMPLATE: &str = include_str!("../../../assets/claude-extensions/register.js");
const CONFIG_MARKER: &str = "/*CODEXPP_CONFIG*/{}";
const EVENTS: &[&str] = &["session.start", "turn.start", "turn.complete", "session.end"];

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaudeDesktopExtension {
    #[serde(default)]
    pub buttons: Vec<ClaudeButton>,
    #[serde(default)]
    pub events: BTreeMap<String, String>,
    #[serde(default)]
    pub tools: Vec<ClaudeTool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeButton {
    pub id: String,
    pub label: String,
    pub method: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaudeTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub method: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnabledClaudeExtension {
    pub key: String,
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub declaration: ClaudeDesktopExtension,
}

impl ClaudeDesktopExtension {
    pub fn parse(value: &Value) -> anyhow::Result<Self> {
        let mut declaration: Self = serde_json::from_value(value.clone())
            .context("claudeDesktop 必须声明 buttons、events 或 tools，不支持 renderer")?;
        if declaration.buttons.is_empty() && declaration.events.is_empty() && declaration.tools.is_empty() {
            anyhow::bail!("claudeDesktop 至少需要一个按钮、事件或工具");
        }
        if declaration.buttons.len() > 32 || declaration.tools.len() > 32 {
            anyhow::bail!("claudeDesktop 最多声明 32 个按钮和 32 个工具");
        }
        let mut buttons = BTreeSet::new();
        for button in &declaration.buttons {
            validate_name(&button.id, "按钮 id")?;
            validate_method(&button.method)?;
            if button.label.trim().is_empty() || button.label.len() > 256 {
                anyhow::bail!("claudeDesktop 按钮 label 不能为空或超过 256 字节");
            }
            if !buttons.insert(&button.id) {
                anyhow::bail!("claudeDesktop 按钮 id 重复：{}", button.id);
            }
        }
        for (event, method) in &declaration.events {
            if !EVENTS.contains(&event.as_str()) {
                anyhow::bail!("claudeDesktop 不支持事件 {event}");
            }
            validate_method(method)?;
        }
        let mut tools = BTreeSet::new();
        for tool in &mut declaration.tools {
            validate_name(&tool.name, "工具 name")?;
            if tool.name.len() > 48 {
                anyhow::bail!("claudeDesktop 工具 name 最多 48 字节");
            }
            validate_method(&tool.method)?;
            if !tools.insert(&tool.name) {
                anyhow::bail!("claudeDesktop 工具 name 重复：{}", tool.name);
            }
            if tool.description.len() > 4096 {
                anyhow::bail!("claudeDesktop 工具 description 超过 4096 字节");
            }
            if !tool.input_schema.is_object()
                || tool.input_schema.get("type").and_then(Value::as_str) != Some("object")
                || !tool.input_schema.get("properties").is_some_and(Value::is_object)
                || serde_json::to_vec(&tool.input_schema)?.len() > 64 * 1024
            {
                anyhow::bail!("claudeDesktop 工具 inputSchema 必须声明 type: object 与 properties，且不超过 64 KiB");
            }
            if tool.input_schema.get("additionalProperties").is_some_and(|value| value != &Value::Bool(false)) {
                anyhow::bail!("claudeDesktop 工具 inputSchema 不允许 additionalProperties");
            }
            if tool.input_schema["properties"].as_object().is_some_and(|properties| {
                properties.keys().any(|key| matches!(key.as_str(), "tool" | "agentId" | "toolUseId" | "tool_use_id"))
            }) {
                anyhow::bail!("claudeDesktop 工具输入不能使用 tool、agentId、toolUseId 或 tool_use_id 等宿主保留字段");
            }
            tool.input_schema["additionalProperties"] = json!(false);
        }
        Ok(declaration)
    }

    pub fn permits(&self, method: &str) -> bool {
        validate_method(method).is_ok()
            && (self.buttons.iter().any(|button| button.method == method)
                || self.events.values().any(|declared| declared == method)
                || self.tools.iter().any(|tool| tool.method == method))
    }
}

fn validate_name(value: &str, label: &str) -> anyhow::Result<()> {
    if value.is_empty() || value.len() > 64
        || !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        anyhow::bail!("claudeDesktop {label} 只能包含字母、数字、_、-，且长度为 1 到 64");
    }
    Ok(())
}

fn validate_method(method: &str) -> anyhow::Result<()> {
    if method.is_empty() || method.len() > 128 || method.starts_with("__")
        || matches!(method, "constructor" | "prototype")
        || !method.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        anyhow::bail!("claudeDesktop 后台方法名无效或属于内部方法：{method}");
    }
    Ok(())
}

/// 生成受管理的本机 Claude 插件；不修改 Claude 的用户插件配置。
pub fn prepare_managed_plugin() -> anyhow::Result<PathBuf> {
    prepare_managed_plugin_at(
        &crate::user_scripts::default_user_script_manager(),
        &crate::paths::default_app_state_dir(),
        &std::env::current_exe().context("无法定位 Codex++ 可执行文件")?,
    )
}

/// 独立路径入口供本地实例和测试使用；生成文件均同目录原子替换，不清理目录里的其它文件。
pub fn prepare_managed_plugin_at(
    manager: &UserScriptManager,
    state_dir: &Path,
    executable: &Path,
) -> anyhow::Result<PathBuf> {
    if !state_dir.is_absolute() || !executable.is_absolute() {
        anyhow::bail!("Claude 拓展插件需要绝对状态目录与可执行文件路径");
    }
    let settings_path = manager.settings_path().map(Path::to_path_buf)
        .unwrap_or_else(crate::paths::default_settings_path);
    if !settings_path.is_absolute() {
        anyhow::bail!("Claude 拓展插件需要绝对设置路径");
    }
    let config_dir = crate::user_scripts::default_user_scripts_config_dir();
    let config_base = config_dir.parent().context("Claude 拓展配置目录没有父目录")?;
    let config_env = if cfg!(windows) { "APPDATA" } else { "XDG_CONFIG_HOME" };
    let extensions = manager.enabled_claude_extensions()?;
    let config = json!({
        "serverName": format!("plugin:{PLUGIN_NAME}:{SERVER_NAME}"),
        "toolName": TOOL_NAME,
        "extensions": extensions,
    });
    if REGISTER_TEMPLATE.matches(CONFIG_MARKER).count() != 1 {
        anyhow::bail!("Claude 拓展注册模板缺少唯一配置标记");
    }
    let registration = REGISTER_TEMPLATE.replace(CONFIG_MARKER, &serde_json::to_string(&config)?);
    let root = state_dir.join("claude-extensions").join("plugin");
    std::fs::create_dir_all(&root).context("无法创建 Claude 拓展插件目录")?;
    let root = root.canonicalize().context("无法解析 Claude 拓展插件目录")?;
    let manifest = json!({
        "name": PLUGIN_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Codex++ extensions for Claude Desktop",
        "author": {"name": "CodexPlusPlus"},
    });
    let mcp = json!({
        "mcpServers": {
            (SERVER_NAME): {
                "command": executable,
                "args": [MCP_FLAG, "--state-dir", state_dir, "--settings-path", settings_path],
                "env": {(config_env): config_base},
            }
        }
    });
    write_plugin_file(&root, ".claude-plugin/plugin.json", &serde_json::to_vec_pretty(&manifest)?)?;
    write_plugin_file(&root, ".mcp.json", &serde_json::to_vec_pretty(&mcp)?)?;
    write_plugin_file(&root, "hooks/hooks.json", br#"{"modules":["./register.js"]}"#)?;
    write_plugin_file(&root, "hooks/register.js", registration.as_bytes())?;
    Ok(root)
}

fn write_plugin_file(root: &Path, relative: &str, contents: &[u8]) -> anyhow::Result<()> {
    let target = root.join(relative);
    let parent = target.parent().context("Claude 插件文件没有父目录")?;
    std::fs::create_dir_all(parent)?;
    if !parent.canonicalize()?.starts_with(root)
        || std::fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        anyhow::bail!("Claude 插件文件 {relative} 指向受管理目录之外");
    }
    crate::settings::atomic_write(&target, contents)
}

pub fn run_mcp_cli(settings_path: Option<PathBuf>) -> anyhow::Result<()> {
    let manager = crate::user_scripts::default_user_script_manager_for_settings(
        settings_path.unwrap_or_else(crate::paths::default_settings_path),
    );
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(serve(manager))
}

/// 一个 Claude 会话保持一个 stdio 服务，后台可在按钮、工具和事件之间保留状态。
pub async fn serve(manager: UserScriptManager) -> anyhow::Result<()> {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        while let Ok(Some(line)) = read_bounded_line(&mut reader) {
            if sender.blocking_send(line).is_err() {
                break;
            }
        }
    });
    let result = async {
        while let Some(line) = receiver.recv().await {
            let response = match line {
                None => Some(rpc_error(Value::Null, -32600, "请求行超过 1 MiB 上限")),
                Some(line) if line.iter().all(u8::is_ascii_whitespace) => None,
                Some(line) => match serde_json::from_slice(&line) {
                    Ok(request) => handle_request(&manager, request).await,
                    Err(_) => Some(rpc_error(Value::Null, -32700, "JSON 解析失败")),
                },
            };
            if let Some(response) = response {
                let mut output = std::io::stdout().lock();
                serde_json::to_writer(&mut output, &response)?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
        anyhow::Ok(())
    }.await;
    crate::extension_backend::stop_all_claude(&manager).await;
    result
}

pub async fn handle_request(manager: &UserScriptManager, request: Value) -> Option<Value> {
    let Some(object) = request.as_object() else {
        return Some(rpc_error(Value::Null, -32600, "请求必须是 JSON-RPC 对象"));
    };
    let id = object.get("id").cloned()?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(rpc_error(id, -32600, "JSON-RPC 请求格式无效"));
    }
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match object.get("method").and_then(Value::as_str) {
        Some("initialize") => {
            let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18");
            let version = if matches!(requested, "2024-11-05" | "2025-03-26" | "2025-06-18" | "2025-11-25") {
                requested
            } else {
                "2025-06-18"
            };
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
            })
        }
        Some("ping") => json!({}),
        Some("tools/list") => json!({"tools": [{
            "name": TOOL_NAME,
            "description": "Call a backend method explicitly declared by an enabled Codex++ Claude Desktop extension.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": {"type": "string"},
                    "method": {"type": "string"},
                    "params": {},
                    "sessionId": {"type": "string"},
                },
                "required": ["key", "method"],
                "additionalProperties": false,
            },
        }]}),
        Some("tools/call") if params.get("name").and_then(Value::as_str) == Some(TOOL_NAME) => {
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match call(manager, &arguments).await {
                Ok(value) => json!({
                    "content": [{"type": "text", "text": serde_json::to_string(&value).unwrap_or_default()}],
                    "structuredContent": {"result": value},
                    "isError": false,
                }),
                Err(error) => json!({
                    "content": [{"type": "text", "text": format!("{error:#}")}],
                    "isError": true,
                }),
            }
        }
        Some("tools/call") => return Some(rpc_error(id, -32602, "没有这个 Claude 拓展工具")),
        Some(_) => return Some(rpc_error(id, -32601, "未实现的 MCP 方法")),
        None => return Some(rpc_error(id, -32600, "缺少请求方法")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

pub async fn call(manager: &UserScriptManager, arguments: &Value) -> anyhow::Result<Value> {
    let object = arguments.as_object().context("Claude 拓展调用参数必须是对象")?;
    if object.keys().any(|key| !matches!(key.as_str(), "key" | "method" | "params" | "sessionId")) {
        anyhow::bail!("Claude 拓展调用包含未声明的参数");
    }
    let key = object.get("key").and_then(Value::as_str).context("缺少拓展 key")?;
    let method = object.get("method").and_then(Value::as_str).context("缺少后台 method")?;
    let session_id = match object.get("sessionId") {
        Some(Value::String(value)) if !value.is_empty() && value.len() <= 256 => Some(value.as_str()),
        None => None,
        _ => anyhow::bail!("sessionId 必须是 1 到 256 字节的字符串"),
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let timeout = if params.get("host").and_then(Value::as_str) == Some("claudeDesktop") {
        match params.get("event").and_then(Value::as_str) {
            Some("session.end") => std::time::Duration::from_millis(750),
            Some(event) if EVENTS.contains(&event) => std::time::Duration::from_secs(3),
            _ => crate::extension_backend::DEFAULT_CALL_TIMEOUT,
        }
    } else {
        crate::extension_backend::DEFAULT_CALL_TIMEOUT
    };
    match tokio::time::timeout(timeout, crate::extension_backend::call_claude(
        manager,
        key,
        method,
        params,
        timeout,
        session_id,
    )).await {
        Ok(result) => result,
        Err(_) => {
            crate::extension_backend::stop_claude(manager, key).await;
            anyhow::bail!("Claude Desktop 拓展调用超时");
        }
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn read_bounded_line(reader: &mut impl BufRead) -> std::io::Result<Option<Option<Vec<u8>>>> {
    let mut line = Vec::new();
    let read = reader.by_ref().take(MAX_REQUEST_BYTES as u64 + 1).read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > MAX_REQUEST_BYTES && line.last() != Some(&b'\n') {
        // 丢弃超长行时也使用固定大小缓冲，避免无换行输入无限增长。
        loop {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                break;
            }
            let end = buffer.iter().position(|byte| *byte == b'\n').map(|index| index + 1);
            let consumed = end.unwrap_or(buffer.len());
            reader.consume(consumed);
            if end.is_some() {
                break;
            }
        }
        return Ok(Some(None));
    }
    Ok(Some(Some(line)))
}
