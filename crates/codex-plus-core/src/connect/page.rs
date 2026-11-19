//! Codex 里的「微信连接」页（`45-weixin-connect-page.js`）读写的设置子集。
//!
//! 连接本体（扫码会话、运行中的连接）在管理器进程内存里，所以 bridge 路由 `/weixin-connect`
//! 由管理器登记成宿主路由，动作分发与管理器的 Tauri 命令共用同一组函数；这里只放两边都用得到、
//! 与进程状态无关的部分：页面能看到哪些字段、能改哪些字段、值域是什么。
//!
//! - 页面看不到登录凭据（只给「是否已登录」），也改不了凭据、账号、网关地址这类扫码后自动填的项。
//! - 改动走白名单：没列在这里的键一律拒绝，免得页面顺手把整份设置写回去。

use anyhow::bail;
use serde_json::{Value, json};

use crate::settings::BackendSettings;

/// bridge 路由路径。不在拓展路由白名单里：能扫码登录、启停连接、改微信设置。
pub const BRIDGE_ROUTE: &str = "/weixin-connect";

/// 沙箱权限的取值（与管理器微信页的下拉一致）。
pub const SANDBOX_VALUES: [&str; 3] = ["read-only", "workspace-write", "danger-full-access"];
/// 微信默认模式的取值：交给加加，或直连 Codex。
pub const DEFAULT_MODE_VALUES: [&str; 2] = ["jiajia", "direct"];

/// 页面能改的字符串字段。
const TEXT_FIELDS: [&str; 6] = [
    "weixinConnectAllowFrom",
    "weixinConnectWorkDir",
    "weixinConnectModel",
    "weixinConnectReasoningEffort",
    "weixinConnectDefaultMode",
    "weixinConnectSandbox",
];
/// 页面能改的开关字段。
const BOOL_FIELDS: [&str; 2] = [
    "weixinConnectPushToWeixin",
    "weixinConnectAllowOutboundAttachments",
];

/// 页面显示用的设置：可改的字段 + 只读的账号与登录状态，不含凭据。
pub fn page_settings(settings: &BackendSettings) -> Value {
    json!({
        "weixinConnectAllowFrom": settings.weixin_connect_allow_from,
        "weixinConnectWorkDir": settings.weixin_connect_work_dir,
        "weixinConnectModel": settings.weixin_connect_model,
        "weixinConnectReasoningEffort": settings.weixin_connect_reasoning_effort,
        "weixinConnectDefaultMode": settings.weixin_connect_default_mode,
        "weixinConnectSandbox": settings.weixin_connect_sandbox,
        "weixinConnectPushToWeixin": settings.weixin_connect_push_to_weixin,
        "weixinConnectAllowOutboundAttachments": settings.weixin_connect_allow_outbound_attachments,
        "weixinConnectAccountId": settings.weixin_connect_account_id,
        "weixinConnectEnabled": settings.weixin_connect_enabled,
        "loggedIn": !settings.weixin_connect_token.trim().is_empty(),
    })
}

/// 把页面提交的改动合进设置。只认白名单里的键，值的类型、取值不对就整体拒绝（不做部分写入）。
pub fn apply_page_settings(settings: &mut BackendSettings, patch: &Value) -> anyhow::Result<()> {
    let Some(patch) = patch.as_object() else {
        bail!("微信设置必须是对象");
    };
    // 先整体校验，再写入：任何一项不对都不动原设置。
    for (key, value) in patch {
        if TEXT_FIELDS.contains(&key.as_str()) {
            let Some(text) = value.as_str() else {
                bail!("{key} 必须是字符串");
            };
            match key.as_str() {
                "weixinConnectSandbox" if !SANDBOX_VALUES.contains(&text) => {
                    bail!("不支持的沙箱权限：{text}")
                }
                "weixinConnectDefaultMode" if !DEFAULT_MODE_VALUES.contains(&text) => {
                    bail!("不支持的微信默认模式：{text}")
                }
                _ => {}
            }
        } else if BOOL_FIELDS.contains(&key.as_str()) {
            if !value.is_boolean() {
                bail!("{key} 必须是开关值");
            }
        } else {
            bail!("不能在 Codex 里修改 {key}，请到 Codex++ 管理工具里改");
        }
    }
    for (key, value) in patch {
        let text = || value.as_str().unwrap_or_default().trim().to_string();
        let flag = value.as_bool().unwrap_or_default();
        match key.as_str() {
            "weixinConnectAllowFrom" => settings.weixin_connect_allow_from = text(),
            "weixinConnectWorkDir" => settings.weixin_connect_work_dir = text(),
            "weixinConnectModel" => settings.weixin_connect_model = text(),
            "weixinConnectReasoningEffort" => settings.weixin_connect_reasoning_effort = text(),
            "weixinConnectDefaultMode" => settings.weixin_connect_default_mode = text(),
            "weixinConnectSandbox" => settings.weixin_connect_sandbox = text(),
            "weixinConnectPushToWeixin" => settings.weixin_connect_push_to_weixin = flag,
            "weixinConnectAllowOutboundAttachments" => {
                settings.weixin_connect_allow_outbound_attachments = flag
            }
            _ => {}
        }
    }
    Ok(())
}
