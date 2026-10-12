//! 官方登录（ChatGPT OAuth）成员的凭据解析与刷新。
//!
//! 聚合代理选中官方登录成员时，请求要带着该账号的 OAuth access token 发往
//! chatgpt.com 后端（而不是供应商 API Key）。凭据保存在供应商快照的
//! `authContents`（auth.json 的 tokens 段）里；access token 即将过期时用
//! refresh_token 走 auth.openai.com 刷新，刷新结果写回快照，避免每个请求都重新刷新。
//!
//! 与 CC Switch 的 CodexOAuthManager 对齐：同一个 token 端点与公开 client_id，
//! 并优先采纳 live auth.json 里同一账号的更新凭据——用户同时在直连使用该账号时，
//! refresh token 可能已被 Codex 客户端轮换，旧代际刷新会直接失败。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde_json::Value;

use crate::settings::{RelayMode, RelayProfile, SettingsStore};

/// 官方登录的接入地址（relay_config 的官方测试兜底与此共用）。
pub const OFFICIAL_LOGIN_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// ChatGPT/Codex OAuth 的 token 端点与公开 client_id（与 Codex CLI 一致）。
pub const OFFICIAL_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const OFFICIAL_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// access token 到期前的刷新缓冲：提前 5 分钟刷新，避免请求在途中失效。
const REFRESH_AHEAD_MS: i64 = 5 * 60 * 1000;
/// 刷新失败后的负缓存窗口：避免每个请求都为同一个失效登录反复发起刷新。
const REFRESH_FAILURE_COOLDOWN_MS: u128 = 5 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialLogin {
    pub access_token: String,
    pub refresh_token: String,
    pub account_id: String,
}

/// 一次上游请求实际要用的凭据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialCredential {
    pub access_token: String,
    pub account_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RefreshedTokens {
    access_token: String,
    refresh_token: String,
    account_id: String,
}

/// 纯官方登录 profile（官方混合 API Key 走普通 API 通道，不算在这里）。
pub fn is_official_login_profile(profile: &RelayProfile) -> bool {
    profile.relay_mode == RelayMode::Official && !profile.official_mix_api_key
}

/// 官方登录接入地址；`CODEX_PLUS_OFFICIAL_BASE_URL` 仅供测试/诊断覆盖。
pub fn official_base_url() -> String {
    env_override("CODEX_PLUS_OFFICIAL_BASE_URL")
        .unwrap_or_else(|| OFFICIAL_LOGIN_BASE_URL.to_string())
}

fn oauth_token_url() -> String {
    env_override("CODEX_PLUS_OFFICIAL_TOKEN_URL")
        .unwrap_or_else(|| OFFICIAL_OAUTH_TOKEN_URL.to_string())
}

fn env_override(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 从 auth.json 文本解析 ChatGPT 登录；没有 tokens 段或全为空时返回 None。
pub fn official_login_from_auth_contents(contents: &str) -> Option<OfficialLogin> {
    let value: Value = serde_json::from_str(contents).ok()?;
    official_login_from_auth_value(&value)
}

fn official_login_from_auth_value(value: &Value) -> Option<OfficialLogin> {
    let tokens = value.get("tokens")?;
    let access_token = string_field(tokens, "access_token");
    let refresh_token = string_field(tokens, "refresh_token");
    if access_token.is_empty() && refresh_token.is_empty() {
        return None;
    }
    Some(OfficialLogin {
        access_token,
        refresh_token,
        account_id: account_id_from_tokens(tokens),
    })
}

/// 快照里是否具备可用的官方登录凭据（同步、不联网）。刷新是否成功由请求路径决定。
pub fn official_login_available(profile: &RelayProfile) -> bool {
    is_official_login_profile(profile)
        && official_login_from_auth_contents(&profile.auth_contents).is_some()
}

/// 该 profile 的上游请求是否应走官方登录通道：纯官方 profile（归一化后没有
/// URL/Key）且快照里有登录凭据。带 URL/Key 的官方模式旧数据继续按 API 通道处理，
/// 保持与历史行为兼容。
pub fn member_uses_official_login_transport(profile: &RelayProfile) -> bool {
    if !is_official_login_profile(profile) {
        return false;
    }
    // 官方后端只认 Responses 协议；挂到 chat/completions 的异常 profile 不走官方通道。
    if profile.protocol != crate::settings::RelayProtocol::Responses {
        return false;
    }
    let has_api_transport = !profile.base_url.trim().is_empty()
        && (!profile.api_key.trim().is_empty() || profile.uses_no_auth());
    if has_api_transport {
        return false;
    }
    official_login_available(profile)
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// account id 是 chatgpt-account-id 请求头的值；优先 tokens.account_id，
/// 缺失时从 id_token / access_token 的 claims 取（旧版 auth.json 没有顶层字段）。
fn account_id_from_tokens(tokens: &Value) -> String {
    let direct = string_field(tokens, "account_id");
    if !direct.is_empty() {
        return direct;
    }
    for key in ["id_token", "access_token"] {
        let token = string_field(tokens, key);
        if token.is_empty() {
            continue;
        }
        if let Some(claims) = jwt_claims(&token) {
            if let Some(id) = claims
                .get("https://api.openai.com/auth")
                .and_then(|auth| auth.get("chatgpt_account_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return id.to_string();
            }
        }
    }
    String::new()
}

fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    use base64::Engine;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.as_bytes())
        .ok()
        .or_else(|| {
            base64::engine::general_purpose::URL_SAFE
                .decode(payload.as_bytes())
                .ok()
        })?;
    serde_json::from_slice(&decoded).ok()
}

fn jwt_exp_ms(token: &str) -> Option<i64> {
    jwt_claims(token)?
        .get("exp")?
        .as_i64()
        .map(|exp| exp.saturating_mul(1000))
}

/// access token 缺失、或 JWT exp 已进入刷新缓冲窗口时为 true。
/// 解析不出 exp 的 token 一律视为有效，交给上游 401 触发强制刷新。
fn token_needs_refresh(access_token: &str) -> bool {
    if access_token.trim().is_empty() {
        return true;
    }
    let Some(exp_ms) = jwt_exp_ms(access_token) else {
        return false;
    };
    exp_ms - now_ms() < REFRESH_AHEAD_MS
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// 读取 live auth.json 里同一账号的登录凭据（仅 chatgpt 登录态）。
pub fn live_chatgpt_login() -> Option<OfficialLogin> {
    live_chatgpt_login_from(&crate::codex_home::default_codex_home_dir())
}

fn live_chatgpt_login_from(home: &Path) -> Option<OfficialLogin> {
    let contents = std::fs::read_to_string(home.join("auth.json")).ok()?;
    let value: Value = serde_json::from_str(&contents).ok()?;
    let is_chatgpt = value
        .get("auth_mode")
        .and_then(Value::as_str)
        .map(|mode| mode.eq_ignore_ascii_case("chatgpt"))
        .unwrap_or(false);
    if !is_chatgpt {
        return None;
    }
    official_login_from_auth_value(&value)
}

/// 同一账号时优先使用 live 凭据：Codex 客户端每次使用都会刷新并轮换
/// refresh token，live 代际比快照新。账号不一致或 live 缺少 refresh token
/// 而快照有时，保留快照。
fn prefer_live_login(stored: OfficialLogin, live: Option<OfficialLogin>) -> OfficialLogin {
    let Some(live) = live else {
        return stored;
    };
    if live.account_id.is_empty() || live.account_id != stored.account_id {
        return stored;
    }
    if live.refresh_token.is_empty() && !stored.refresh_token.is_empty() {
        return stored;
    }
    if live.access_token == stored.access_token && live.refresh_token == stored.refresh_token {
        return stored;
    }
    live
}

fn credential(login: OfficialLogin) -> OfficialCredential {
    OfficialCredential {
        access_token: login.access_token,
        account_id: login.account_id,
    }
}

fn refresh_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn refresh_failures() -> &'static Mutex<HashMap<String, u128>> {
    static FAILURES: OnceLock<Mutex<HashMap<String, u128>>> = OnceLock::new();
    FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn refresh_in_cooldown(account_id: &str) -> bool {
    if account_id.trim().is_empty() {
        return false;
    }
    let mut failures = refresh_failures()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(&failed_at) = failures.get(account_id) else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    if now.saturating_sub(failed_at) >= REFRESH_FAILURE_COOLDOWN_MS {
        failures.remove(account_id);
        return false;
    }
    true
}

fn mark_refresh_failure(account_id: &str) {
    if account_id.trim().is_empty() {
        return;
    }
    let mut failures = refresh_failures()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    failures.insert(account_id.to_string(), now);
}

fn clear_refresh_failure(account_id: &str) {
    if account_id.trim().is_empty() {
        return;
    }
    refresh_failures()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(account_id);
}

/// 解析出可用于上游请求的官方凭据；access token 过期（或 `force_refresh`）
/// 时先刷新再返回。刷新结果写回供应商快照。
pub async fn resolve_official_credential(
    profile: &RelayProfile,
    force_refresh: bool,
) -> anyhow::Result<OfficialCredential> {
    resolve_official_credential_with(
        profile,
        force_refresh,
        &SettingsStore::default(),
        &crate::codex_home::default_codex_home_dir(),
        None,
    )
    .await
}

async fn resolve_official_credential_with(
    profile: &RelayProfile,
    force_refresh: bool,
    store: &SettingsStore,
    codex_home: &Path,
    token_url_override: Option<&str>,
) -> anyhow::Result<OfficialCredential> {
    if !is_official_login_profile(profile) {
        anyhow::bail!("供应商「{}」不是官方登录模式", profile.name);
    }
    let stored = official_login_from_auth_contents(&profile.auth_contents).ok_or_else(|| {
        anyhow::anyhow!(
            "官方登录成员「{}」缺少 ChatGPT 登录凭据，请先在管理器中完成或导入官方登录",
            profile.name
        )
    })?;
    let login = prefer_live_login(stored, live_chatgpt_login_from(codex_home));
    if !force_refresh && !token_needs_refresh(&login.access_token) {
        return Ok(credential(login));
    }

    // 串行刷新：并发请求只发一次刷新，其余等锁后直接用新凭据。
    let _guard = refresh_lock().lock().await;
    let stored = current_auth_contents(store, &profile.id)
        .and_then(|contents| official_login_from_auth_contents(&contents))
        .unwrap_or(login);
    let login = prefer_live_login(stored, live_chatgpt_login_from(codex_home));
    if !force_refresh && !token_needs_refresh(&login.access_token) {
        return Ok(credential(login));
    }
    if login.refresh_token.trim().is_empty() {
        anyhow::bail!(
            "官方登录成员「{}」的 access token 已过期且没有 refresh token，请重新登录该官方账号",
            profile.name
        );
    }
    let account_key = login.account_id.clone();
    if refresh_in_cooldown(&account_key) {
        anyhow::bail!(
            "官方登录凭据刷新刚失败过，暂时跳过（登录可能已在别处失效），请稍后重试或重新登录该官方账号"
        );
    }

    let token_url = token_url_override
        .map(str::to_string)
        .unwrap_or_else(oauth_token_url);
    let refreshed = match refresh_codex_tokens(&token_url, &login.refresh_token).await {
        Ok(tokens) => tokens,
        Err(error) => {
            // live 里可能是更新一代的 refresh token：换它再试一次。
            let live = live_chatgpt_login_from(codex_home).filter(|live| {
                !live.refresh_token.is_empty() && live.refresh_token != login.refresh_token
            });
            match live {
                Some(live) if live.account_id == login.account_id => {
                    match refresh_codex_tokens(&token_url, &live.refresh_token).await {
                        Ok(tokens) => tokens,
                        Err(retry_error) => {
                            mark_refresh_failure(&account_key);
                            return Err(retry_error);
                        }
                    }
                }
                _ => {
                    mark_refresh_failure(&account_key);
                    return Err(error);
                }
            }
        }
    };
    clear_refresh_failure(&account_key);
    let account_id = if refreshed.account_id.trim().is_empty() {
        login.account_id.clone()
    } else {
        refreshed.account_id.clone()
    };
    persist_refreshed_login(
        store,
        &profile.id,
        &refreshed.access_token,
        &refreshed.refresh_token,
        &account_id,
    );
    Ok(OfficialCredential {
        access_token: refreshed.access_token,
        account_id,
    })
}

fn current_auth_contents(store: &SettingsStore, profile_id: &str) -> Option<String> {
    let settings = store.load().ok()?;
    settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .map(|profile| profile.auth_contents.clone())
}

async fn refresh_codex_tokens(
    token_url: &str,
    refresh_token: &str,
) -> anyhow::Result<RefreshedTokens> {
    let client = reqwest::Client::builder()
        .user_agent(format!("CodexPlusPlus/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let response = client
        .post(token_url)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", OFFICIAL_OAUTH_CLIENT_ID),
            ("scope", "openid profile email"),
        ])
        .send()
        .await
        .context("官方登录 token 刷新请求失败")?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let code = extract_error_code(&text);
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
            || matches!(
                code.as_str(),
                "refresh_token_expired" | "refresh_token_reused" | "refresh_token_invalidated"
            )
        {
            anyhow::bail!(
                "官方登录 refresh token 已失效（{}），请在管理器中重新登录该官方账号",
                if code.is_empty() {
                    status.to_string()
                } else {
                    code
                }
            );
        }
        anyhow::bail!("官方登录 token 刷新失败：{status} {text}");
    }
    let value: Value = serde_json::from_str(&text).context("官方登录 token 响应解析失败")?;
    let access_token = string_field(&value, "access_token");
    if access_token.is_empty() {
        anyhow::bail!("官方登录 token 响应缺少 access_token");
    }
    let rotated = string_field(&value, "refresh_token");
    let refresh_token = if rotated.is_empty() {
        refresh_token.to_string()
    } else {
        rotated
    };
    let account_id = value
        .get("id_token")
        .and_then(Value::as_str)
        .and_then(|token| {
            let tokens = serde_json::json!({ "id_token": token });
            Some(account_id_from_tokens(&tokens))
        })
        .unwrap_or_default();
    Ok(RefreshedTokens {
        access_token,
        refresh_token,
        account_id,
    })
}

fn extract_error_code(text: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return String::new();
    };
    for pointer in ["/error/code", "/error", "/code"] {
        if let Some(code) = value.pointer(pointer).and_then(Value::as_str) {
            let code = code.trim();
            if !code.is_empty() {
                return code.to_string();
            }
        }
    }
    String::new()
}

/// 把刷新后的 tokens 写回供应商快照的 authContents。写失败只记录日志：
/// 当前请求仍用新 token，下一次请求会再刷新。
fn persist_refreshed_login(
    store: &SettingsStore,
    profile_id: &str,
    access_token: &str,
    refresh_token: &str,
    account_id: &str,
) {
    let result = (|| -> anyhow::Result<()> {
        let mut settings = store.load()?;
        let Some(profile) = settings
            .relay_profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
        else {
            return Ok(());
        };
        if !is_official_login_profile(profile) {
            return Ok(());
        }
        let mut value: Value = serde_json::from_str(&profile.auth_contents)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| serde_json::json!({ "auth_mode": "chatgpt" }));
        if !value.get("tokens").is_some_and(Value::is_object) {
            value["tokens"] = serde_json::json!({});
        }
        value["tokens"]["access_token"] = serde_json::json!(access_token);
        value["tokens"]["refresh_token"] = serde_json::json!(refresh_token);
        if !account_id.trim().is_empty() {
            value["tokens"]["account_id"] = serde_json::json!(account_id);
        }
        profile.auth_contents = serde_json::to_string_pretty(&value)?;
        store.save(&settings)
    })();
    if let Err(error) = result {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "official_login.refresh_persist_failed",
            serde_json::json!({
                "profileId": profile_id,
                "error": error.to_string()
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fake_jwt(payload: Value) -> String {
        use base64::Engine;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(br#"{"alg":"none"}"#);
        let body = engine.encode(serde_json::to_vec(&payload).unwrap());
        format!("{header}.{body}.")
    }

    fn official_profile(auth_contents: &str) -> RelayProfile {
        RelayProfile {
            id: "official".to_string(),
            name: "官方登录".to_string(),
            relay_mode: RelayMode::Official,
            auth_contents: auth_contents.to_string(),
            ..RelayProfile::default()
        }
    }

    #[test]
    fn parses_login_from_auth_contents() {
        let login = official_login_from_auth_contents(
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"a","refresh_token":"r","account_id":"acct-1"}}"#,
        )
        .unwrap();
        assert_eq!(login.access_token, "a");
        assert_eq!(login.refresh_token, "r");
        assert_eq!(login.account_id, "acct-1");
    }

    #[test]
    fn rejects_auth_without_tokens() {
        assert!(official_login_from_auth_contents(r#"{"OPENAI_API_KEY":"sk-x"}"#).is_none());
        assert!(
            official_login_from_auth_contents(r#"{"tokens":{"access_token":"","refresh_token":""}}"#)
                .is_none()
        );
    }

    #[test]
    fn derives_account_id_from_id_token_claims() {
        let id_token = fake_jwt(json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct-claims" }
        }));
        let contents = json!({
            "auth_mode": "chatgpt",
            "tokens": { "access_token": "a", "refresh_token": "r", "id_token": id_token }
        })
        .to_string();
        let login = official_login_from_auth_contents(&contents).unwrap();
        assert_eq!(login.account_id, "acct-claims");
    }

    #[test]
    fn treats_expired_access_token_as_needing_refresh() {
        let expired = fake_jwt(json!({ "exp": now_ms() / 1000 - 60 }));
        let fresh = fake_jwt(json!({ "exp": now_ms() / 1000 + 3600 }));
        assert!(token_needs_refresh(&expired));
        assert!(!token_needs_refresh(&fresh));
        // 无法解析 exp 的 token 不主动刷新，交给 401 触发。
        assert!(!token_needs_refresh("not-a-jwt"));
        assert!(token_needs_refresh(""));
    }

    #[test]
    fn prefers_live_login_for_same_account() {
        let stored = OfficialLogin {
            access_token: "old".to_string(),
            refresh_token: "old-refresh".to_string(),
            account_id: "acct-1".to_string(),
        };
        let live = OfficialLogin {
            access_token: "new".to_string(),
            refresh_token: "new-refresh".to_string(),
            account_id: "acct-1".to_string(),
        };
        let picked = prefer_live_login(stored.clone(), Some(live.clone()));
        assert_eq!(picked.access_token, "new");

        let other_account = OfficialLogin {
            account_id: "acct-2".to_string(),
            ..live.clone()
        };
        assert_eq!(
            prefer_live_login(stored.clone(), Some(other_account)).access_token,
            "old"
        );

        let live_without_refresh = OfficialLogin {
            access_token: "new".to_string(),
            refresh_token: String::new(),
            account_id: "acct-1".to_string(),
        };
        assert_eq!(
            prefer_live_login(stored.clone(), Some(live_without_refresh)).access_token,
            "old"
        );
        assert_eq!(prefer_live_login(stored.clone(), None), stored);
    }

    #[test]
    fn extracts_refresh_error_code() {
        assert_eq!(
            extract_error_code(r#"{"error":{"code":"refresh_token_reused"}}"#),
            "refresh_token_reused"
        );
        assert_eq!(
            extract_error_code(r#"{"error":"invalid_grant"}"#),
            "invalid_grant"
        );
        assert_eq!(extract_error_code("plain text"), "");
    }

    #[tokio::test]
    async fn resolve_refreshes_expired_token_and_persists_rotation() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        // 本地 token 端点：返回轮换后的 access/refresh token。
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 8192];
            let mut request = Vec::new();
            loop {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n")
                    && String::from_utf8_lossy(&request).contains("grant_type=refresh_token")
                {
                    break;
                }
            }
            let body = r#"{"access_token":"fresh-access","refresh_token":"fresh-refresh"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).to_string()
        });

        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let store = SettingsStore::new(settings_path.clone());
        let expired = fake_jwt(json!({ "exp": now_ms() / 1000 - 60 }));
        let profile = official_profile(&json!({
            "auth_mode": "chatgpt",
            "tokens": { "access_token": expired, "refresh_token": "old-refresh", "account_id": "acct-1" }
        })
        .to_string());
        let mut settings = crate::settings::BackendSettings::default();
        settings.relay_profiles = vec![profile.clone()];
        store.save(&settings).unwrap();

        let credential = resolve_official_credential_with(
            &profile,
            false,
            &store,
            temp.path(),
            Some(&format!("http://{address}/oauth/token")),
        )
        .await
        .unwrap();
        assert_eq!(credential.access_token, "fresh-access");
        assert_eq!(credential.account_id, "acct-1");

        let request = server.join().unwrap();
        assert!(request.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"), "{request}");
        assert!(request.contains("refresh_token=old-refresh"), "{request}");

        // 刷新结果（含轮换后的 refresh token）写回快照。
        let persisted = SettingsStore::new(settings_path)
            .load()
            .unwrap()
            .relay_profiles
            .into_iter()
            .find(|profile| profile.id == "official")
            .unwrap();
        let login = official_login_from_auth_contents(&persisted.auth_contents).unwrap();
        assert_eq!(login.access_token, "fresh-access");
        assert_eq!(login.refresh_token, "fresh-refresh");
    }
}
