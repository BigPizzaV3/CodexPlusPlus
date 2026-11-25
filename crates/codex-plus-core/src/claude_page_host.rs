//! Claude 页面拓展的宿主端：常驻 helper 上的 `/claude-ext/v1/<op>`。
//!
//! Claude 页面里的拓展脚本（`claudeDesktop.renderer`）经它调后台。两条路都到这里：
//! - 完整功能模式：页面 → claude-local 的本地服务（校验同源和令牌）→ 这里；
//! - 补丁副本：页面 → 主进程 hook 的 binding（`assets/claude-localization/hook.js`）→ 这里。
//!
//! Code 会话里的按钮、工具、事件（Claude Code Mods，见 [`crate::extension_claude`]）也先转到这里，
//! 这样同一个拓展在页面和 Code 会话里用的是同一个后台实例；helper 不在线时它们退回各自进程里托管。
//!
//! 凭据（端口和令牌）写在状态目录的 [`RECEIPT_FILE`]，只有本人可读；令牌每次启动现生成，
//! 只在回环上的这组路由上有效。请求体、回应的形状见 ClaudeMom 仓库 `crates/claude-local/runtime/README.md`。

use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

use serde_json::{Value, json};

use crate::extension_backend::{self, DEFAULT_CALL_TIMEOUT};

/// 凭据文件名。主进程 hook（hook.js）和 claude-local 的本地服务按同一个名字读，改动要同步。
pub const RECEIPT_FILE: &str = "claude-ext-host.json";
pub const TOKEN_HEADER: &str = "x-codex-plus-claude-ext";
pub const ROUTE_PREFIX: &str = "/claude-ext/v1/";
const MAX_CALL_TIMEOUT: Duration = Duration::from_secs(24);
/// Claude 页面里开放的路由（与 claude-local 的 `PAGE_ROUTES` 相同）。
const PAGE_ROUTES: &[&str] = claude_local::extensions::PAGE_ROUTES;

static TOKEN: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());

/// 本进程的令牌。
pub fn token() -> &'static str {
    &TOKEN
}

pub fn receipt_path() -> PathBuf {
    crate::paths::default_app_state_dir().join(RECEIPT_FILE)
}

/// 令牌比对：长度相同才逐字节比，不因为前缀对上就早退。
pub fn token_matches(given: &str) -> bool {
    let expected = token().as_bytes();
    let given = given.as_bytes();
    given.len() == expected.len() && given.iter().zip(expected).fold(0u8, |diff, (left, right)| diff | (left ^ right)) == 0
}

/// 一次调用的失败：HTTP 状态码、给页面的错误码（`unsupported`、`not_enabled` 等）和原因。
#[derive(Debug)]
pub struct HostError {
    pub status: u16,
    pub code: Option<&'static str>,
    pub message: String,
}

impl HostError {
    fn new(status: u16, code: Option<&'static str>, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into() }
    }

    fn unsupported(message: impl Into<String>) -> Self {
        Self::new(403, Some("unsupported"), message)
    }

    pub fn reply(&self) -> Value {
        match self.code {
            Some(code) => json!({ "ok": false, "error": self.message, "code": code }),
            None => json!({ "ok": false, "error": self.message }),
        }
    }
}

fn text<'a>(body: &'a Value, name: &str) -> Result<&'a str, HostError> {
    body.get(name).and_then(Value::as_str).filter(|value| !value.is_empty()).ok_or_else(|| HostError::new(400, None, format!("缺少 {name}")))
}

/// 启用中的包。每次现查：拓展被停用、Claude 总开关关掉之后立刻调不到。
fn package(key: &str) -> Result<claude_local::extensions::PagePackage, HostError> {
    crate::claude_local_backend::ensure_host();
    claude_local::extensions::page_package(key).ok_or_else(|| HostError::new(403, Some("not_enabled"), "这个拓展没有启用"))
}

fn call_timeout(body: &Value) -> Duration {
    body.get("timeoutMs")
        .and_then(Value::as_u64)
        .filter(|millis| *millis > 0)
        .map_or(DEFAULT_CALL_TIMEOUT, |millis| Duration::from_millis(millis).min(MAX_CALL_TIMEOUT))
}

/// 处理一次调用。`op` 是路径里 [`ROUTE_PREFIX`] 之后的部分。
pub async fn handle(op: &str, body: &Value) -> Result<Value, HostError> {
    let manager = crate::user_scripts::default_user_script_manager();
    match op {
        // 页面里的拓展脚本调后台。
        "call" => {
            let key = text(body, "key")?;
            let method = text(body, "method")?;
            let package = package(key)?;
            if !claude_local::extensions::valid_method(method) || !package.methods.iter().any(|declared| declared == method) {
                return Err(HostError::unsupported(format!("拓展 {} 没有在 claudeDesktop.methods 里声明后台方法：{method}", package.id)));
            }
            let Some(backend) = package.backend.clone() else {
                return Err(HostError::unsupported(format!("拓展 {} 没有声明后台", package.id)));
            };
            let params = body.get("params").cloned().unwrap_or(Value::Null);
            let timeout = call_timeout(body);
            let result = if package.hosted {
                let instance = body.get("instance").and_then(Value::as_str);
                extension_backend::call_claude_page(&manager, key, method, params, timeout, instance).await
            } else {
                extension_backend::call_claude_local(&manager, key, (package.id, backend, package.config), package.data_dir, method, params, timeout)
                    .await
            };
            result.map_err(|error| HostError::new(502, None, format!("{error:#}")))
        }
        "events" => {
            let key = text(body, "key")?;
            package(key)?;
            let since = match body.get("since") {
                Some(Value::String(latest)) if latest == "latest" => None,
                other => Some(other.and_then(Value::as_u64).unwrap_or(0)),
            };
            let wait = Duration::from_millis(body.get("waitMs").and_then(Value::as_u64).unwrap_or(0));
            Ok(extension_backend::claude_events(&manager, key, since, wait).await)
        }
        // 页面实例卸载时发来的 stop 不停后台：Code 会话可能还在用同一个实例。
        "stop" => {
            package(text(body, "key")?)?;
            Ok(json!({ "stopped": false }))
        }
        "route" => {
            let package = package(text(body, "key")?)?;
            let route = text(body, "route")?;
            if !PAGE_ROUTES.contains(&route) {
                return Err(HostError::unsupported(format!("路由 {route} 在 Claude Desktop 里没有开放")));
            }
            if !package.routes.iter().any(|declared| declared == route) {
                return Err(HostError::unsupported(format!("拓展 {} 未在 codexpp.json 声明路由权限：{route}", package.id)));
            }
            if route == "/backend/status" {
                return Ok(json!({ "status": "ok", "host": "claudeDesktop", "connected": true, "version": env!("CARGO_PKG_VERSION") }));
            }
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "claude_extension.log",
                json!({ "extension": package.id, "payload": body.get("payload").cloned().unwrap_or(Value::Null) }),
            );
            Ok(json!({ "status": "ok" }))
        }
        // 页面回报各拓展的加载结果；顺手把已经不在启用列表里的后台停掉。
        "status" => {
            crate::claude_local_backend::ensure_host();
            claude_local::extensions::record_page_status(body).map_err(|error| HostError::new(500, None, format!("{error:#}")))?;
            // 留下的是：有页面部分的包，加上 Code 会话里还在用的包（只有按钮、工具、事件的包不在注入计划里）。
            // 读不出 Code 会话那份清单时不清理，免得误停。
            if let Ok(mods) = manager.enabled_claude_extensions() {
                let mut keep = claude_local::extensions::page_package_keys();
                keep.extend(mods.into_iter().map(|extension| extension.key));
                extension_backend::retain_claude(&manager, &keep).await;
            }
            Ok(json!({ "saved": true }))
        }
        "config" => {
            package(text(body, "key")?)?;
            Err(HostError::new(501, Some("unsupported"), "还不能在 Claude 页面里改拓展的配置，请到「万物皆拓展」页里改"))
        }
        // Code 会话里的按钮、工具、事件（Mods 的 MCP 进程转来的）：按它们各自的声明校验。
        "mods-call" => {
            let key = text(body, "key")?;
            let method = text(body, "method")?;
            let params = body.get("params").cloned().unwrap_or(Value::Null);
            let session = body.get("sessionId").and_then(Value::as_str);
            extension_backend::call_claude(&manager, key, method, params, call_timeout(body), session)
                .await
                .map_err(|error| HostError::new(502, None, format!("{error:#}")))
        }
        _ => Err(HostError::new(404, None, "没有这个接口")),
    }
}

/// 本机常驻 helper 的凭据；它没在运行（或凭据不是只有本人可读的普通文件）时为 `None`。
fn read_receipt() -> Option<(u16, String)> {
    let path = receipt_path();
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return None;
        }
    }
    let receipt: Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    let port = u16::try_from(receipt.get("port")?.as_u64()?).ok().filter(|port| *port > 0)?;
    Some((port, receipt.get("token")?.as_str()?.to_string()))
}

/// Mods 的 MCP 进程把后台调用转给常驻 helper。helper 不在线时返回 `None`，调用方退回本进程托管。
///
/// 只在 `manager` 就是本机默认的拓展目录时转：helper 管的是那一份，别的目录（测试、隔离实例）它不认识。
pub(crate) async fn forward_mods_call(
    manager: &crate::user_scripts::UserScriptManager,
    key: &str,
    method: &str,
    params: &Value,
    timeout: Duration,
    session: Option<&str>,
) -> Option<anyhow::Result<Value>> {
    if manager.config_path() != crate::user_scripts::default_user_script_manager().config_path() {
        return None;
    }
    let (port, token) = read_receipt()?;
    // 回环直连：进程环境里可能配了系统代理。
    let client = reqwest::Client::builder().no_proxy().timeout(timeout + Duration::from_secs(2)).build().ok()?;
    let body = json!({ "key": key, "method": method, "params": params, "timeoutMs": timeout.as_millis() as u64, "sessionId": session });
    let response = match client.post(format!("http://127.0.0.1:{port}{ROUTE_PREFIX}mods-call")).header(TOKEN_HEADER, token).json(&body).send().await {
        Ok(response) => response,
        // 连不上：helper 已经退了，凭据是上次留下的。
        Err(error) if error.is_connect() => return None,
        Err(error) => return Some(Err(anyhow::anyhow!("Claude Desktop 拓展调用失败：{error}"))),
    };
    let reply: Value = match response.json().await {
        Ok(reply) => reply,
        Err(error) => return Some(Err(anyhow::anyhow!("Claude Desktop 拓展调用失败：{error}"))),
    };
    // 令牌对不上（凭据是别的实例留下的）同样当作不在线。
    if reply.get("code").and_then(Value::as_str) == Some("unauthorized") {
        return None;
    }
    Some(if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(anyhow::anyhow!("{}", reply.get("error").and_then(Value::as_str).unwrap_or("Claude Desktop 拓展调用失败")))
    })
}
