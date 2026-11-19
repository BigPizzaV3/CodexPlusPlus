//! 加加的 MCP：Codex 以 `--codex-plus-jiajia-mcp` 拉起的 stdio 进程，只做转发。
//!
//! 工具清单编在程序里（和服务端同一份 [`super::tools::tool_definitions`]），调用经本机回环转给远程控制服务的
//! `/api/jiajia/tool`（地址与令牌来自加加会话配置覆盖里的环境变量，不写进 config.toml）。
//! 真正干活的在服务进程里：它持有会话跟随、后台执行、聊天记录这些状态。

use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Value, json};

use super::{ENV_TOKEN, ENV_URL};

/// 新开会话、给桌面会话发消息可能要等一会儿。
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

const INSTRUCTIONS: &str = "加加的工具：查看和管理用户电脑上 Codex 的会话、盯会话、约回访、记事。";

pub fn serve() -> anyhow::Result<()> {
    let url = std::env::var(ENV_URL).context("缺少转发地址")?;
    let token = std::env::var(ENV_TOKEN).context("缺少令牌")?;
    let target = Target::parse(&url)?;
    let stdin = std::io::stdin();
    let mut output = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle_request(&request, |name, arguments| target.call(&token, name, arguments)),
            Err(_) => Some(rpc_error(Value::Null, -32700, "JSON 解析失败")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// 处理一条 MCP 请求；通知没有响应。`call` 执行工具，返回给模型的文本。
pub fn handle_request(request: &Value, call: impl Fn(&str, &Value) -> anyhow::Result<String>) -> Option<Value> {
    let id = request.get("id").cloned()?;
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18");
            let version = if matches!(requested, "2024-11-05" | "2025-03-26" | "2025-06-18") { requested } else { "2025-06-18" };
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "codex-plus-jiajia", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": super::tools::tool_definitions() }),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(rpc_error(id, -32602, "tools/call 缺少工具名"));
            };
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match call(name, &arguments) {
                Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
                Err(error) => json!({ "content": [{ "type": "text", "text": format!("{error:#}") }], "isError": true }),
            }
        }
        "resources/list" => json!({ "resources": [] }),
        "resources/templates/list" => json!({ "resourceTemplates": [] }),
        "prompts/list" => json!({ "prompts": [] }),
        _ => return Some(rpc_error(id, -32601, "未实现的 MCP 方法")),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// 转发目标：只认本机回环的 http 地址。
struct Target {
    port: u16,
    path: String,
}

impl Target {
    fn parse(url: &str) -> anyhow::Result<Self> {
        let rest = url.strip_prefix("http://127.0.0.1:").context("转发地址只能是本机回环")?;
        let (port, path) = rest.split_once('/').context("转发地址缺少路径")?;
        Ok(Self { port: port.parse().context("端口无效")?, path: format!("/{path}") })
    }

    fn call(&self, token: &str, name: &str, arguments: &Value) -> anyhow::Result<String> {
        let body = serde_json::to_vec(&json!({ "name": name, "arguments": arguments }))?;
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).context("连不上 Codex++ 远程控制服务")?;
        stream.set_read_timeout(Some(CALL_TIMEOUT))?;
        let head = format!(
            "POST {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.path,
            self.port,
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(&body)?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        let split = response.windows(4).position(|window| window == b"\r\n\r\n").context("响应格式不对")?;
        let payload: Value = serde_json::from_slice(&response[split + 4..]).context("响应不是 JSON")?;
        if payload.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(payload.pointer("/data/text").and_then(Value::as_str).unwrap_or_default().to_string());
        }
        bail!("{}", payload.get("error").and_then(Value::as_str).unwrap_or("工具调用失败"))
    }
}
