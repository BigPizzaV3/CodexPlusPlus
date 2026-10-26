//! Computer Use 的应用级审批。
//!
//! Codex 的原生 CU server 在「首次接管某个 app」时会发 `elicitation/create`
//! （`Allow ChatGPT to use "Finder"?`），并**阻塞到有人答复为止**。答复有三档：
//! `accept` / `decline` / `cancel`。
//!
//! 这里提供两种策略（`BackendSettings::computer_use_approval`）：
//!
//! - `auto`：代理自己答 `accept`，只留痕。对应 Codex 设为「从不询问」时不弹卡。
//! - `claude`（界面叫「弹卡确认」，取值沿用旧名）：同 Codex 的默认行为，把请求转给
//!   当前连接的 MCP 客户端弹卡。不限于 Claude，任何声明了 elicitation 的客户端都行。
//!
//! 「始终允许」和 Codex 一样交给 Computer Use 自己记：原生请求带
//! `_meta.persist: ["always"]` 时，转发的表单里加一个勾选项；勾上就回
//! `accept` + `_meta.persist: "always"`，原生写进它的 `ComputerUseAppApprovals.json`，
//! 以后任何会话（含 Codex）都不再问这个应用。实测见 `native_approvals_path`。
//!
//! 旧版的 `<state>/cu/approvals.json` 只读兼容：从来没有代码写过它，但手写的条目仍然放行。

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

const APPROVALS_FILE: &str = "cu/approvals.json";
const AUDIT_FILE: &str = "cu/audit.jsonl";
const LAST_CLIENT_FILE: &str = "cu/last-client.json";
/// 管理器自检握手时用的客户端名，见 `computer_use::run_probe`。
pub const SELF_CHECK_CLIENT: &str = "codex-plus";
/// 转发的审批表单里加的勾选项：勾上 = 「始终允许」。
pub const ALWAYS_FIELD: &str = "alwaysAllow";
/// Computer Use 自己记的「始终允许」（相对用户主目录）。Codex 和 Codex++ 共用这一份。
const NATIVE_APPROVALS: &str = "Library/Group Containers/2DC432GLL2.com.openai.sky.CUAService/Library/Application Support/Software/ComputerUseAppApprovals.json";

/// 记下最近一次连上来的 MCP 客户端，以及它**自己**有没有声明 elicitation。
///
/// `claude` 模式依赖客户端能弹审批卡；客户端不支持时代理会退回放行。
/// 这个降级必须让用户在页面上看得见，否则会误以为一直在手动审批。
pub fn record_client(client_info: Option<&serde_json::Value>, supports_elicitation: bool) {
    // 管理器自检也走这条入口（clientInfo.name = "codex-plus"），不能让它盖掉真实客户端。
    if client_info
        .and_then(|info| info.get("name"))
        .and_then(serde_json::Value::as_str)
        == Some(SELF_CHECK_CLIENT)
    {
        return;
    }
    let record = serde_json::json!({
        "at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or(0),
        "name": client_info.and_then(|info| info.get("name")).cloned(),
        "version": client_info.and_then(|info| info.get("version")).cloned(),
        "elicitation": supports_elicitation,
    });
    let path = crate::paths::default_app_state_dir().join(LAST_CLIENT_FILE);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format!("{record}\n"));
}

pub fn last_client() -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(crate::paths::default_app_state_dir().join(LAST_CLIENT_FILE))
        .ok()?;
    serde_json::from_str(&text).ok()
}

/// 审批策略。字符串形式与 `settings.json` 和前端下拉框一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    /// 代理就地放行。
    Auto,
    /// 转给 MCP 客户端弹卡（界面叫「弹卡确认」）。
    Claude,
}

impl ApprovalMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "claude" => Self::Claude,
            // 未知取值一律按默认处理：settings.json 里可能有旧版本或手写的值。
            _ => Self::Auto,
        }
    }

    /// 从设置文件现读。读失败按默认处理，不让审批链因为读配置挂掉。
    pub fn current() -> Self {
        let value = crate::settings::SettingsStore::default()
            .load()
            .map(|settings| settings.computer_use_approval)
            .unwrap_or_default();
        Self::parse(&value)
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Claude => "claude",
        }
    }
}

/// 记忆下来的决定。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppApproval {
    /// `allow` 或 `deny`。
    pub decision: String,
    /// `always`（跨会话）或 `session`（仅当次进程）。
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub decided_at: u64,
    /// `computer-use`：Computer Use 自己记的（与 Codex 共用）；空 = 旧版 Codex++ 文件。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalStore {
    #[serde(default = "default_version")]
    pub version: u32,
    /// bundle id -> 决定。没有 bundle id 的条目用显示名当键。
    #[serde(default)]
    pub apps: BTreeMap<String, AppApproval>,
}

fn default_version() -> u32 {
    1
}

fn path() -> PathBuf {
    crate::paths::default_app_state_dir().join(APPROVALS_FILE)
}

impl ApprovalStore {
    pub fn load() -> Self {
        let Ok(text) = std::fs::read_to_string(path()) else {
            return Self::default();
        };
        // 文件坏了就当空的：审批记录丢了只会多问一次，不会误放行。
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// 页面上列出的「始终允许」：Computer Use 自己记的在前，旧版文件里的接在后面。
    pub fn read_all() -> Vec<(String, AppApproval)> {
        let mut all: Vec<(String, AppApproval)> = read_native_approvals(&native_approvals_path())
            .into_iter()
            .map(|bundle| {
                let entry = AppApproval {
                    decision: "allow".into(),
                    mode: "always".into(),
                    display_name: String::new(),
                    decided_at: 0,
                    source: "computer-use".into(),
                };
                (bundle, entry)
            })
            .collect();
        for (key, entry) in Self::load().apps {
            if !all.iter().any(|(existing, _)| *existing == key) {
                all.push((key, entry));
            }
        }
        all
    }

    /// 记忆一个决定。`always` 才落盘，`session` 不写。
    pub fn remember(key: &str, approval: AppApproval) -> anyhow::Result<()> {
        if approval.mode != "always" {
            return Ok(());
        }
        let mut store = Self::load();
        store.version = 1;
        store.apps.insert(key.to_string(), approval);
        store.save()
    }

    /// 撤销一个应用：两份记录里都删。
    pub fn forget(key: &str) -> anyhow::Result<bool> {
        let native = forget_native_approvals(&native_approvals_path(), Some(key))?;
        let mut store = Self::load();
        if store.apps.remove(key).is_none() {
            return Ok(native);
        }
        store.save()?;
        Ok(true)
    }

    pub fn clear() -> anyhow::Result<()> {
        forget_native_approvals(&native_approvals_path(), None)?;
        let mut store = Self::load();
        if store.apps.is_empty() {
            return Ok(());
        }
        store.apps.clear();
        store.save()
    }

    fn save(&self) -> anyhow::Result<()> {
        let path = path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建目录 {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        // 先写临时文件再改名：审批文件被写坏会导致用户被反复追问。
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, format!("{text}\n"))
            .with_context(|| format!("无法写入 {}", temp.display()))?;
        std::fs::rename(&temp, &path).with_context(|| format!("无法替换 {}", path.display()))?;
        Ok(())
    }
}

/// Computer Use 自己的「始终允许」记录。
///
/// 实测（2026-10-25，`SkyComputerUseClient mcp`）：对 TextEdit 回 `accept` +
/// `_meta.persist: "always"` 后，这个文件变成 `{"approvedBundleIdentifiers":["com.apple.TextEdit"]}`，
/// 新开的进程再操作 TextEdit 不再发 `elicitation/create`。
pub fn native_approvals_path() -> PathBuf {
    crate::computer_use::home_dir().join(NATIVE_APPROVALS)
}

fn read_native_value(path: &std::path::Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn read_native_approvals(path: &std::path::Path) -> Vec<String> {
    read_native_value(path)
        .and_then(|value| value.get("approvedBundleIdentifiers").cloned())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect()
}

/// 从 Computer Use 的记录里删掉一个应用（`None` = 全部）。文件里的其它字段原样保留。
/// 返回是否删到了东西；文件不存在或本来就没有时不写。
pub fn forget_native_approvals(path: &std::path::Path, bundle: Option<&str>) -> anyhow::Result<bool> {
    let Some(mut value) = read_native_value(path) else {
        return Ok(false);
    };
    let Some(list) = value
        .get_mut("approvedBundleIdentifiers")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(false);
    };
    let before = list.len();
    list.retain(|item| bundle.is_some_and(|bundle| item.as_str() != Some(bundle)));
    if list.len() == before {
        return Ok(false);
    }
    // 先写临时文件再改名，避免写一半把 Computer Use 的记录弄坏。
    let temp = path.with_extension("json.codexplus-tmp");
    std::fs::write(&temp, serde_json::to_vec(&value)?)
        .with_context(|| format!("无法写入 {}", temp.display()))?;
    std::fs::rename(&temp, path).with_context(|| format!("无法替换 {}", path.display()))?;
    Ok(true)
}

/// 原生请求允许「始终允许」时，在要转给客户端的表单里加一个勾选项。
pub fn offer_always(params: &mut serde_json::Value) -> bool {
    let allowed = params
        .pointer("/_meta/persist")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item == "always"));
    let Some(object) = params.as_object_mut().filter(|_| allowed) else {
        return false;
    };
    let schema = object
        .entry("requestedSchema")
        .or_insert_with(|| serde_json::json!({ "type": "object" }));
    let Some(schema) = schema.as_object_mut() else {
        return false;
    };
    schema.entry("type").or_insert_with(|| "object".into());
    let Some(properties) = schema
        .entry("properties")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
    else {
        return false;
    };
    properties.insert(
        ALWAYS_FIELD.into(),
        serde_json::json!({
            "type": "boolean",
            "title": "始终允许（以后不再询问这个应用）",
            "description": "与 Codex 共用这份授权，可在 Codex++ 的 Computer Use 页撤销。",
            "default": false,
        }),
    );
    true
}

/// 把客户端对审批表单的答复换成给原生的答复。返回（答复，是否始终允许）。
///
/// 勾了 `ALWAYS_FIELD`，或客户端自己带了 `_meta.persist: "always"`（Codex 的做法），
/// 都按「始终允许」回原生；勾选项本身不传给原生。
pub fn native_reply(result: Option<&serde_json::Value>) -> (serde_json::Value, bool) {
    let action = result
        .and_then(|result| result.get("action"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("cancel");
    let mut content = result
        .and_then(|result| result.get("content"))
        .filter(|content| content.is_object())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let ticked = content
        .as_object_mut()
        .and_then(|content| content.remove(ALWAYS_FIELD))
        .and_then(|value| value.as_bool())
        == Some(true);
    let client_always = result
        .and_then(|result| result.pointer("/_meta/persist"))
        .and_then(serde_json::Value::as_str)
        == Some("always");
    let always = action == "accept" && (ticked || client_always);
    let mut reply = serde_json::json!({ "action": action, "content": content });
    if always {
        reply["_meta"] = serde_json::json!({ "persist": "always" });
    }
    (reply, always)
}

/// 一次审批的结局，用于留痕。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 记忆命中，直接放行。
    Remembered,
    /// `auto` 模式就地放行。
    AutoAccepted,
    /// 客户端批准（仅本会话）。
    ClientAccepted,
    /// 客户端批准并勾了「始终允许」，由 Computer Use 自己记住。
    ClientAcceptedAlways,
    /// 客户端拒绝。
    ClientDeclined,
    /// 客户端取消或超时。
    ClientCanceled,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remembered => "remembered",
            Self::AutoAccepted => "auto-accepted",
            Self::ClientAccepted => "client-accepted",
            Self::ClientAcceptedAlways => "client-accepted-always",
            Self::ClientDeclined => "client-declined",
            Self::ClientCanceled => "client-canceled",
        }
    }

    pub fn accepted(self) -> bool {
        matches!(
            self,
            Self::Remembered
                | Self::AutoAccepted
                | Self::ClientAccepted
                | Self::ClientAcceptedAlways
        )
    }
}

/// 从 elicitation 参数里认出「针对哪个 app」。
///
/// 原生发的是 `Allow ChatGPT to use "Finder"?`，`_meta` 里没有 bundle id；
/// 更完整的一版（code-mode 路径）带 `tool_params.app`。两条都认。
pub fn subject_of(params: &serde_json::Value) -> (String, String) {
    let meta = params.get("_meta");
    let bundle = meta
        .and_then(|meta| meta.get("tool_params"))
        .and_then(|params| params.get("app"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let display = meta
        .and_then(|meta| meta.get("tool_params_display"))
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("value"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| app_name_in(params.get("message").and_then(serde_json::Value::as_str)))
        .unwrap_or_default();
    let key = if bundle.is_empty() {
        display.clone()
    } else {
        bundle.to_string()
    };
    (key, display)
}

/// 从 `Allow ChatGPT to use "Finder"?` 里取出 `Finder`。
/// 新版原生不加引号（`Allow ChatGPT to use TextEdit?`），取 ` use ` 之后、问号之前的部分。
fn app_name_in(message: Option<&str>) -> Option<String> {
    let message = message?;
    let name = if let Some(start) = message.find('"') {
        let rest = &message[start + 1..];
        &rest[..rest.find('"')?]
    } else {
        let rest = &message[message.rfind(" use ")? + " use ".len()..];
        rest.strip_suffix('?').unwrap_or(rest)
    };
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// 追加一条审计记录。只记 app 与结果，不记输入内容。
pub fn audit(params: &serde_json::Value, mode: ApprovalMode, outcome: Outcome) {
    let (key, display) = subject_of(params);
    let record = serde_json::json!({
        "at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or(0),
        "app": key,
        "displayName": display,
        "mode": mode.id(),
        "outcome": outcome.as_str(),
    });
    let path = crate::paths::default_app_state_dir().join(AUDIT_FILE);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write as _;
        let _ = writeln!(file, "{record}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_app_name_from_native_message() {
        let params = serde_json::json!({ "message": "Allow ChatGPT to use \"Finder\"?" });
        let (key, display) = subject_of(&params);
        assert_eq!(key, "Finder");
        assert_eq!(display, "Finder");
    }

    #[test]
    fn parses_unquoted_app_name_from_current_native_message() {
        let params = serde_json::json!({ "message": "Allow ChatGPT to use TextEdit?" });
        assert_eq!(subject_of(&params).0, "TextEdit");
    }

    #[test]
    fn offers_always_only_when_native_allows_it() {
        let mut params = serde_json::json!({
            "_meta": { "persist": ["always"] },
            "message": "Allow ChatGPT to use TextEdit?",
            "requestedSchema": { "properties": {}, "type": "object" }
        });
        assert!(offer_always(&mut params));
        assert_eq!(params["requestedSchema"]["properties"][ALWAYS_FIELD]["type"], "boolean");

        let mut session_only = serde_json::json!({ "_meta": { "persist": ["session"] }, "message": "x" });
        assert!(!offer_always(&mut session_only));
        assert!(session_only.get("requestedSchema").is_none());
    }

    #[test]
    fn native_reply_persists_only_accepted_always() {
        let ticked = serde_json::json!({ "action": "accept", "content": { ALWAYS_FIELD: true } });
        let (reply, always) = native_reply(Some(&ticked));
        assert!(always);
        assert_eq!(reply["_meta"]["persist"], "always");
        // 勾选项不传给原生。
        assert!(reply["content"].get(ALWAYS_FIELD).is_none());

        let plain = serde_json::json!({ "action": "accept", "content": { ALWAYS_FIELD: false } });
        let (reply, always) = native_reply(Some(&plain));
        assert!(!always);
        assert!(reply.get("_meta").is_none());

        // 客户端自己带 persist（Codex 的做法）也算。
        let codex = serde_json::json!({ "action": "accept", "content": {}, "_meta": { "persist": "always" } });
        assert!(native_reply(Some(&codex)).1);

        // 拒绝时勾了也不记。
        let declined = serde_json::json!({ "action": "decline", "content": { ALWAYS_FIELD: true } });
        let (reply, always) = native_reply(Some(&declined));
        assert!(!always);
        assert_eq!(reply["action"], "decline");
        assert_eq!(native_reply(None).0["action"], "cancel");
    }

    #[test]
    fn native_approvals_read_and_forget_keep_other_fields() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ComputerUseAppApprovals.json");
        assert!(read_native_approvals(&path).is_empty());
        assert!(!forget_native_approvals(&path, None).unwrap());

        std::fs::write(
            &path,
            r#"{"approvedBundleIdentifiers":["com.apple.TextEdit","com.apple.finder"],"other":1}"#,
        )
        .unwrap();
        assert_eq!(read_native_approvals(&path), vec!["com.apple.TextEdit", "com.apple.finder"]);
        assert!(!forget_native_approvals(&path, Some("com.example.none")).unwrap());
        assert!(forget_native_approvals(&path, Some("com.apple.TextEdit")).unwrap());
        assert_eq!(read_native_approvals(&path), vec!["com.apple.finder"]);
        let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["other"], 1);
        assert!(forget_native_approvals(&path, None).unwrap());
        assert!(read_native_approvals(&path).is_empty());
    }

    #[test]
    fn prefers_bundle_id_when_meta_carries_it() {
        let params = serde_json::json!({
            "message": "Allow Computer Use to use \"Finder\"?",
            "_meta": {
                "tool_params": { "app": "com.apple.finder" },
                "tool_params_display": [{ "name": "app", "value": "Finder" }]
            }
        });
        let (key, display) = subject_of(&params);
        assert_eq!(key, "com.apple.finder");
        assert_eq!(display, "Finder");
    }

    #[test]
    fn missing_message_yields_empty_subject() {
        let params = serde_json::json!({});
        assert_eq!(subject_of(&params).0, "");
    }

    #[test]
    fn approval_mode_parses_known_values_and_defaults_otherwise() {
        assert_eq!(ApprovalMode::parse("auto"), ApprovalMode::Auto);
        assert_eq!(ApprovalMode::parse("claude"), ApprovalMode::Claude);
        // 手写或旧版本的值不能让审批链挂掉。
        assert_eq!(ApprovalMode::parse(""), ApprovalMode::Auto);
        assert_eq!(ApprovalMode::parse("whatever"), ApprovalMode::Auto);
    }

    #[test]
    fn outcome_acceptance_matches_semantics() {
        assert!(Outcome::Remembered.accepted());
        assert!(Outcome::AutoAccepted.accepted());
        assert!(Outcome::ClientAccepted.accepted());
        assert!(Outcome::ClientAcceptedAlways.accepted());
        assert!(!Outcome::ClientDeclined.accepted());
        assert!(!Outcome::ClientCanceled.accepted());
    }
}
