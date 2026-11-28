//! 本地 ChatGPT 后端兼容服务。
//!
//! ChatGPT 桌面版的侧栏入口（代码审查、图像、资料库、定时任务、插件等）只在
//! ChatGPT 登录下可用，纯 API Key 进不去。这里在 `https://localhost:8000` 起一个
//! 假的 ChatGPT 后端：桌面把 `CODEX_API_BASE_URL` 指过来后，就以为自己登录了
//! ChatGPT，那些入口随之可用。
//!
//! 应答是脱敏后的真实录制（[`recordings`]，构建时内嵌）。录制里没有的接口给最小的
//! 空响应；token 刷新本地签发。模型请求不经过这里，仍走用户在 `config.toml` 里选的供应商。
//!
//! 端口 8000 是桌面鉴权白名单写死的（只有 `localhost` / `localhost:8000` /
//! `*.openai.com` / `chatgpt.com` 会附带登录 token），不能改。

pub mod account_settings;
pub mod plugins;
mod recordings;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};

/// 桌面鉴权白名单里的端口，不能改。
pub const PORT: u16 = 8000;

/// 假账号的固定身份。录制里的真实 id 都已替换成这些值。
pub const ACCOUNT_ID: &str = "acc-local";
pub const USER_ID: &str = "user-local";

/// 本地账号的展示资料：名字、用户名、邮箱。管理器里可改，没填时用默认值。
/// 录制里个人资料处是固定占位（名字 `Local`、用户名 `local`、邮箱 `local@codexplus.invalid`），
/// 应答时按字段换成这里的值；插件、技能这些公开目录数据不动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub username: String,
    pub email: String,
}

pub const DEFAULT_NAME: &str = "Local";
pub const DEFAULT_USERNAME: &str = "local";
pub const DEFAULT_EMAIL: &str = "local@codexplus.invalid";

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: DEFAULT_NAME.to_string(),
            username: DEFAULT_USERNAME.to_string(),
            email: DEFAULT_EMAIL.to_string(),
        }
    }
}

impl Profile {
    /// 从设置里的三个值得到资料：空白的用默认值。
    pub fn from_settings(name: &str, username: &str, email: &str) -> Self {
        let pick = |value: &str, default: &str| {
            let value = value.trim();
            if value.is_empty() { default.to_string() } else { value.to_string() }
        };
        Self {
            name: pick(name, DEFAULT_NAME),
            username: pick(username, DEFAULT_USERNAME),
            email: pick(email, DEFAULT_EMAIL),
        }
    }
}

/// 服务当前用的资料。设置改了之后由 [`set_profile`] 更新，下一个请求就生效。
static PROFILE: Mutex<Option<Profile>> = Mutex::new(None);

pub fn set_profile(profile: Profile) {
    *PROFILE.lock().unwrap_or_else(|error| error.into_inner()) = Some(profile);
}

fn profile() -> Profile {
    PROFILE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
        .unwrap_or_default()
}

/// 个人资料所在的录制和字段（JSON pointer），值是名字 / 用户名 / 邮箱哪一个。
const PROFILE_FIELDS: &[(&str, &[(&str, ProfileField)])] = &[
    ("/backend-api/me", &[("/name", ProfileField::Name), ("/email", ProfileField::Email)]),
    (
        "/backend-api/profiles/me",
        &[
            ("/profile_details/display_name", ProfileField::Name),
            ("/profile_details/username", ProfileField::Username),
        ],
    ),
    (
        "/backend-api/wham/profiles/me",
        &[("/profile/display_name", ProfileField::Name), ("/profile/username", ProfileField::Username)],
    ),
    ("/backend-api/wham/usage", &[("/email", ProfileField::Email)]),
    ("/backend-api/accounts/optimized/check", &[("/account/name", ProfileField::Name)]),
    ("/backend-api/wham/accounts/check", &[("/accounts/0/name", ProfileField::Name)]),
    (
        "/backend-api/accounts/check/v4-2023-04-27",
        &[
            ("/accounts/acc-local/account/name", ProfileField::Name),
            ("/accounts/default/account/name", ProfileField::Name),
        ],
    ),
];

#[derive(Clone, Copy)]
enum ProfileField {
    Name,
    Username,
    Email,
}

/// 把录制里的个人资料占位换成当前资料。没有个人资料的录制原样返回。
fn personalize(path: &str, body: &'static str) -> String {
    let Some((_, fields)) = PROFILE_FIELDS.iter().find(|(known, _)| *known == path) else {
        return body.to_string();
    };
    let profile = profile();
    if profile == Profile::default() {
        return body.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<Value>(body) else {
        return body.to_string();
    };
    for (pointer, field) in *fields {
        if let Some(slot) = value.pointer_mut(pointer).filter(|slot| slot.is_string()) {
            *slot = json!(match field {
                ProfileField::Name => &profile.name,
                ProfileField::Username => &profile.username,
                ProfileField::Email => &profile.email,
            });
        }
    }
    serde_json::to_string(&value).unwrap_or_else(|_| body.to_string())
}

/// 假登录的套餐。入口的可见性按套餐判断，pro 能看到全部。
const PLAN: &str = "pro";

/// 证书在状态目录里的文件名。
const CERT_FILE: &str = "chatgpt-compat-cert.json";

/// 一年的秒数：本地签发的 token 有效期。
const TOKEN_TTL_SECS: u64 = 365 * 24 * 60 * 60;

/// 读请求头的上限。应答是固定的，正文用不上，读到头就够了。
const HEADER_LIMIT: usize = 64 * 1024;

/// 一条脱敏后的录制。
struct Recording {
    status: u16,
    body: &'static str,
}

/// 内嵌的脱敏录制，按 `方法 路径` 索引（路径不含查询串）。
fn recordings() -> &'static HashMap<String, Recording> {
    static RECORDINGS: OnceLock<HashMap<String, Recording>> = OnceLock::new();
    RECORDINGS.get_or_init(|| {
        recordings::FILES
            .iter()
            .map(|(method, path, body)| {
                (
                    format!("{method} {path}"),
                    Recording { status: 200, body },
                )
            })
            .collect()
    })
}

/// 一个请求的应答。
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

/// 应答的来源，只用于日志，不进响应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplySource {
    /// 命中脱敏录制。
    Recorded,
    /// 本地签发的 token。
    Token,
    /// 合成的最小响应。
    Synthetic,
    /// 明确不存在（OAuth 发现、MCP 端点）。
    NotFound,
}

/// 决定一个请求怎么应答。
pub fn reply_for(method: &str, path: &str) -> (ReplySource, Reply) {
    let path_only = path.split('?').next().unwrap_or(path);
    if path_only.ends_with("/oauth/token") {
        return (ReplySource::Token, token_reply());
    }
    if path_only.contains("/.well-known/") || path_only.ends_with("/ps/mcp") {
        return (
            ReplySource::NotFound,
            Reply {
                status: 404,
                content_type: JSON,
                body: r#"{"error":"not_found"}"#.to_string(),
            },
        );
    }
    // 强调色跟着账号走：改的时候存下来、原样回写（见 [`account_settings`]）。
    if path_only.ends_with("/settings/account_user_setting") {
        let settings_user = recordings()
            .get(&format!("GET {SETTINGS_USER_PATH}"))
            .map(|recording| recording.body)
            .unwrap_or("{}");
        let (status, body) = account_settings::account_user_setting_reply(
            path,
            settings_user,
            &account_settings::store(),
        );
        return (
            ReplySource::Synthetic,
            Reply {
                status,
                content_type: JSON,
                body,
            },
        );
    }
    if let Some(recording) = recordings().get(&format!("{method} {path_only}")) {
        let mut body = personalize(path_only, recording.body);
        if path_only == SETTINGS_USER_PATH {
            // 录制里的强调色是录制那个账号的（black），换成本机记的，没改过是 default。
            body = account_settings::with_saved_theme(&body, &account_settings::store().chat_theme());
        }
        return (
            ReplySource::Recorded,
            Reply {
                status: recording.status,
                content_type: JSON,
                body,
            },
        );
    }
    (ReplySource::Synthetic, synthetic_reply(method, path_only))
}

/// 没录到的接口的最小响应。形状按客户端实际读取的字段给，给错会让页面报错。
fn synthetic_reply(method: &str, path: &str) -> Reply {
    let body = if path.ends_with("/aip/connectors/links/list_accessible") {
        r#"{"links":[]}"#.to_string()
    } else if path.ends_with("/aip/connectors/batch") {
        r#"{"connectors":[]}"#.to_string()
    } else if path.ends_with("/ps/apps/batch") || path.ends_with("/ps/plugins/batch") {
        r#"{"apps":[]}"#.to_string()
    } else if path.ends_with("/link") {
        r#"{"link":null}"#.to_string()
    } else if path.contains("/wham/analytics/") {
        // 设置页 / 个性化页挂载时会查 daily-code-review-metrics 等接口，直接调用 `data.some(...)`，
        // 缺 `data` 数组会让整个设置页触发 React ErrorBoundary（「ChatGPT 遇到了问题」）。
        r#"{"data":[]}"#.to_string()
    } else if path.ends_with("/wham/usage/chatpass/apps") {
        // 设置 → 使用情况页直接调用 `items.some(...)`，缺 `items` 会崩溃。
        r#"{"items":[]}"#.to_string()
    } else if path.ends_with("/wham/usage/daily-token-usage-breakdown")
        || path.ends_with("/wham/usage/daily-workspace-user-token-usage-breakdown")
    {
        r#"{"data":[],"units":"credits","data_freshness_ts":null}"#.to_string()
    } else if path.ends_with("/wham/usage/credit-usage-events")
        || path.ends_with("/wham/usage/daily-workspace-user-credit-usage")
    {
        r#"{"data":[],"data_freshness_ts":null}"#.to_string()
    } else if path.ends_with("/wham/usage/plan_limit_history") {
        r#"{"data_as_of":null,"coverage_start":null,"coverage_complete":true,"approximate":true,"boundary_tolerance_seconds":null,"periods":[]}"#.to_string()
    } else if path.ends_with("/wham/usage/thread_usage/query_v2") {
        r#"{"threads":[],"data_as_of":null}"#.to_string()
    } else if path.ends_with("/wham/rate-limit-reset-credits/history") {
        r#"{"events":[],"next_cursor":null}"#.to_string()
    } else if path.ends_with("/wham/rate-limit-reset-credits") {
        r#"{"credits":[],"items":[],"available_count":0,"applicable_available_count":0}"#.to_string()
    } else if path.ends_with("/subscriptions/credits/discount-offer") {
        r#"{"one_time":{"tiers":[]},"auto_reload":{"tiers":[]}}"#.to_string()
    } else if path.ends_with("/payments/payment_methods") {
        r#"{"payment_methods":[]}"#.to_string()
    } else if path.ends_with("/notifications/settings") {
        // 设置 → 通知页直接调用 `settings.map(...)`，缺 `settings` 数组会崩溃。
        r#"{"settings":[],"page_slack":null}"#.to_string()
    } else if path.ends_with("/personality_types") {
        // 设置 → 个性化页直接对返回值调用 `.find(...)`，必须是数组。
        "[]".to_string()
    } else if path.ends_with("/personality_trait_types") {
        // 设置 → 个性化页直接调用 `trait_types.filter(...)`。
        r#"{"trait_types":[]}"#.to_string()
    } else if path.ends_with("/settings/voices") {
        r#"{"voices":[]}"#.to_string()
    } else if path.ends_with("/pets") {
        "[]".to_string()
    } else if path.ends_with("/images/image-tags") {
        // 设置 → 个性化页（参考照片）直接解构 `{ items }` 返回给 React Query，缺 `items` 会触发告警。
        r#"{"items":[]}"#.to_string()
    } else if path.ends_with("/flora/browser/config") {
        // 设置 → 云电脑页直接读取 `config.origins.length`，缺 `origins` 会触发 React ErrorBoundary 白屏崩溃。
        r#"{"default_decision":"always_ask","origins":[],"email_verification_codes":"always_ask","passkeys":"always_ask"}"#.to_string()
    } else if path.ends_with("/flora/browser/cookies") {
        r#"{"target":{"type":"orbit"},"cookies":[]}"#.to_string()
    } else if path.ends_with("/saved-credentials") {
        // 设置 → 密码页用 Zod 严格校验 `{ items: [...], next_cursor }`。
        r#"{"items":[],"next_cursor":null}"#.to_string()
    } else if path.ends_with("/password-managers/1password") {
        r#"{"available":false,"state":"disconnected"}"#.to_string()
    } else if path.ends_with("/amphora/u18_graduation_unlink_setting_notices") {
        r#"{"notices":[]}"#.to_string()
    } else if path.ends_with("/profiles/me/page/pins") {
        r#"{"pins":{"items":[]},"page_version":1}"#.to_string()
    } else if path.ends_with("/profiles/me/page/pin_options") {
        r#"{"items":[],"next_cursor":null}"#.to_string()
    } else if path.ends_with("/profiles/me/page")
        || (path.contains("/profiles/") && path.ends_with("/page"))
    {
        profile_page_body(method)
    } else if path.ends_with("/trusted_contact/enabled") {
        r#"{"enabled":false}"#.to_string()
    } else if path.ends_with("/trusted_contact") {
        r#"{"contacts":[],"invitations":[]}"#.to_string()
    } else if path.ends_with("/plugins/featured") {
        "[]".to_string()
    } else if path.ends_with("/memories") {
        r#"{"memories":[],"memory_num_tokens":0,"memory_max_tokens":0}"#.to_string()
    } else if path.ends_with("/user_system_messages") {
        r#"{"enabled":false,"about_user_message":"","about_model_message":""}"#.to_string()
    } else if path.ends_with("/wham/github/operations/account") {
        r#"{"currentUser":{"status":"unauthenticated","account":null}}"#.to_string()
    } else if path.ends_with("/user_segments") {
        r#"{"segments":{}}"#.to_string()
    } else if path.contains("/ps/plugins/canonical-app/") {
        r#"{"plugin":null}"#.to_string()
    } else if method == "GET" && (path.ends_with("/items") || path.ends_with("/list")) {
        r#"{"items":[]}"#.to_string()
    } else {
        "{}".to_string()
    };
    Reply {
        status: 200,
        content_type: JSON,
        body,
    }
}

/// 个人资料页（`/profiles/me/page`）的合成响应，随本地资料（名字 / 用户名）动态生成。
fn profile_page_body(method: &str) -> String {
    let display_settings = json!({
        "show_usage_stats_section": false,
        "show_activity_graph_section": false,
        "show_insights_section": false,
        "show_top_plugins_section": false,
        "show_site_artifacts_section": false,
    });
    if method.eq_ignore_ascii_case("PATCH") {
        return json!({
            "display_settings": display_settings,
            "page_version": 1,
        })
        .to_string();
    }
    let current = profile();
    let profile_url = format!("/u/{}", current.username);
    json!({
        "is_self": true,
        "can_edit": true,
        "has_public_personal_profile": false,
        "profile_details": {
            "username": current.username,
            "display_name": current.name,
            "bio": "",
            "profile_picture_url": null,
            "profile_url": profile_url,
            "photo_frame_style": "circle",
        },
        "page": {
            "workspace_id": null,
            "workspace_name": null,
            "page_version": 1,
            "visibility": {
                "value": "private",
                "selectable_options": ["private"],
            },
            "display_settings": display_settings,
            "workspace_display_settings": null,
            "pins": { "items": [] },
            "top_plugins": [],
            "top_plugins_by_remote_id": [],
            "stats": null,
            "activity_graph": null,
            "insights": null,
        }
    })
    .to_string()
}

const JSON: &str = "application/json";

/// 账号设置（强调色等）的接口路径。
const SETTINGS_USER_PATH: &str = "/backend-api/settings/user";

/// 本地签发的登录 token。桌面只解码 payload，不验签。
fn token_reply() -> Reply {
    let token = signed_token(PLAN);
    Reply {
        status: 200,
        content_type: JSON,
        body: serde_json::to_string(&json!({
            "access_token": token,
            "id_token": token,
            "refresh_token": "local-refresh",
            "token_type": "Bearer",
            "expires_in": TOKEN_TTL_SECS,
        }))
        .unwrap_or_else(|_| "{}".to_string()),
    }
}

/// 无签名的 JWT：桌面只读 claims，签名段填固定占位。
fn signed_token(plan: &str) -> String {
    let header = b64(&json!({ "alg": "none", "typ": "JWT" }));
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() + TOKEN_TTL_SECS)
        .unwrap_or(TOKEN_TTL_SECS);
    let payload = b64(&json!({
        "email": profile().email,
        "name": profile().name,
        "exp": exp,
        "https://api.openai.com/auth": {
            "chatgpt_account_id": ACCOUNT_ID,
            "chatgpt_plan_type": plan,
            "chatgpt_user_id": USER_ID,
            "user_id": USER_ID,
        },
    }));
    format!("{header}.{payload}.sig")
}

/// 登录 token 里的邮箱、名字是不是当前资料：改了资料要重写 `auth.json`，桌面才显示新名字。
pub fn token_matches_profile(token: &str) -> bool {
    use base64::Engine;
    let Some(claims) = token
        .split('.')
        .nth(1)
        .and_then(|payload| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return false;
    };
    let profile = profile();
    claims.get("email").and_then(Value::as_str) == Some(profile.email.as_str())
        && claims.get("name").and_then(Value::as_str) == Some(profile.name.as_str())
}

fn b64(value: &Value) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(value).unwrap_or_default())
}

/// `auth.json` 的 `tokens` 是否是本地账号的假登录（[`fake_auth_json`] 写的那份）。
pub fn is_local_login_tokens(tokens: &Value) -> bool {
    tokens.get("account_id").and_then(Value::as_str) == Some(ACCOUNT_ID)
}

/// 写入 `~/.codex/auth.json` 的假登录。保留文件里已有的字段（API Key 等），
/// 只替换登录相关的键。
pub fn fake_auth_json(existing: Option<&str>) -> String {
    let mut value = existing
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .unwrap_or_else(|| json!({}));
    let object = value.as_object_mut().unwrap();
    let token = signed_token(PLAN);
    object.insert(
        "tokens".to_string(),
        json!({
            "id_token": token,
            "access_token": token,
            "refresh_token": "local-refresh",
            "account_id": ACCOUNT_ID,
        }),
    );
    object.insert("auth_mode".to_string(), json!("chatgpt"));
    object.insert("last_refresh".to_string(), json!(rfc3339_now()));
    serde_json::to_string_pretty(&value)
        .map(|text| format!("{text}\n"))
        .unwrap_or_else(|_| "{}\n".to_string())
}

/// RFC3339 时间，桌面的 `last_refresh` 用这个格式。
fn rfc3339_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let rem = (seconds % 86_400) as u32;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant 的 civil_from_days。
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// 证书文件的路径。
pub fn cert_path() -> PathBuf {
    crate::paths::default_app_state_dir().join(CERT_FILE)
}

/// 确保证书存在并覆盖 `localhost`，返回 PEM。
pub fn ensure_certificate() -> anyhow::Result<crate::remote_control::tls::StoredCertificate> {
    crate::remote_control::tls::ensure_certificate_at(&cert_path(), &[])
}

/// 证书公钥的 SHA-256（base64），给 Chromium 的 `--ignore-certificate-errors-spki-list`：
/// 窗口只额外信任这一张证书，其余站点照常校验。
pub fn spki_hash() -> anyhow::Result<String> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let cert = ensure_certificate()?;
    let key = rcgen::KeyPair::from_pem(&cert.key_pem)
        .map_err(|error| anyhow::anyhow!("读取证书密钥失败：{error}"))?;
    let spki = rcgen::PublicKeyData::subject_public_key_info(&key);
    Ok(base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&spki)))
}

/// 启动桌面时要带的环境变量：后端地址、证书、token 刷新地址。
///
/// 两个证书变量都是文件路径：app-server 的 `CODEX_CA_CERTIFICATE` 和 Chromium 的
/// `NODE_EXTRA_CA_CERTS` 都按路径读 PEM。
pub fn launch_env() -> anyhow::Result<Vec<(&'static str, String)>> {
    let cert = cert_file_path()?;
    let base = format!("https://localhost:{PORT}");
    Ok(vec![
        ("CODEX_API_BASE_URL", format!("{base}/backend-api")),
        ("CODEX_CA_CERTIFICATE", cert.clone()),
        ("NODE_EXTRA_CA_CERTS", cert),
        (
            "CODEX_REFRESH_TOKEN_URL_OVERRIDE",
            format!("{base}/oauth/token"),
        ),
    ])
}

/// 证书 PEM 落到磁盘上的路径。
fn cert_file_path() -> anyhow::Result<String> {
    let cert = ensure_certificate()?;
    let path = crate::paths::default_app_state_dir().join("chatgpt-compat-cert.pem");
    crate::settings::atomic_write(&path, cert.cert_pem.as_bytes())
        .map_err(|error| anyhow::anyhow!("写证书失败：{error}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// 兼容服务是否在跑。
pub fn running() -> bool {
    SERVER.lock().expect("chatgpt compat server lock").is_some()
}

struct ServerHandle {
    shutdown: tokio::sync::watch::Sender<bool>,
    generation: u64,
}

static SERVER: Mutex<Option<ServerHandle>> = Mutex::new(None);
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 启动兼容服务。已经在跑时直接返回。端口被占用时返回错误，由调用方提示。
///
/// 不以句柄判断「已经在跑」：存着的句柄可能比监听者活得久（任务被提前结束），
/// 只看句柄会让服务再也起不来——日志里就是 `started` 只出现过一次，之后端口早就没人听，
/// 每次启动都当成「已经在跑」直接返回。这里总是先绑端口，绑到了才算启动。
pub async fn start() -> anyhow::Result<()> {
    let cert = ensure_certificate()?;
    let mut config = (*crate::remote_control::tls::server_config(&cert)?).clone();
    // 只说 HTTP/1.1：Chromium 默认先协商 h2，这里只会按 HTTP/1.1 应答。
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await {
        Ok(listener) => listener,
        // 绑不上但句柄还在：就是本进程在听，按已启动处理。
        Err(_) if running() => return Ok(()),
        Err(error) => {
            anyhow::bail!("兼容服务绑定 127.0.0.1:{PORT} 失败：{error}（端口可能被占用）")
        }
    };
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let previous = SERVER
        .lock()
        .expect("chatgpt compat server lock")
        .replace(ServerHandle {
            shutdown: shutdown_tx,
            generation,
        });
    // 上一个句柄对应的监听者已经不在（否则上面绑不上）：把它的任务收掉。
    if let Some(previous) = previous {
        let _ = previous.shutdown.send(true);
    }
    tokio::spawn(async move {
        serve(listener, acceptor, shutdown_rx).await;
        // 被 stop 或任务自己结束时只清自己这一代，别把后来启动的那个句柄清掉。
        let mut server = SERVER.lock().expect("chatgpt compat server lock");
        if server
            .as_ref()
            .is_some_and(|handle| handle.generation == generation)
        {
            *server = None;
        }
    });
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "chatgpt_compat.started",
        json!({ "port": PORT, "recordings": recordings().len() }),
    );
    Ok(())
}

/// 停掉兼容服务。下次 [`start`] 可以重新拉起。
pub fn stop() {
    if let Some(handle) = SERVER.lock().expect("chatgpt compat server lock").take() {
        let _ = handle.shutdown.send(true);
    }
}

async fn serve(
    listener: tokio::net::TcpListener,
    acceptor: tokio_rustls::TlsAcceptor,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    if let Ok(stream) = acceptor.accept(stream).await {
                        let _ = handle_connection(stream).await;
                    }
                });
            }
        }
    }
}

async fn handle_connection(
    stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut stream = stream;
    let Some(req) = read_request(&mut stream).await? else {
        return Ok(());
    };
    let request = req.headers;
    let body = req.body;
    let mut lines = request.lines();
    let Some(start) = lines.next() else { return Ok(()) };
    let mut parts = start.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let path_only = path.split('?').next().unwrap_or(&path);
    // 原生语音听写（`POST /backend-api/transcribe`）：复用 Codex++ 语音转写后端
    // （同时支持云端 OpenAI 兼容 ASR 与本地 SenseVoice 多格式解码）。音频与文本不落日志。
    if method.eq_ignore_ascii_case("POST") && path_only.ends_with("/transcribe") {
        let (status, reply_body) = if req.body_too_large {
            (
                413,
                json!({
                    "detail": {
                        "code": "transcription_failed",
                        "message": "语音上传超过 25 MiB 限制",
                    }
                })
                .to_string(),
            )
        } else {
            transcribe_reply(&request, &body).await
        };
        write_json(&mut stream, status, &reply_body).await?;
        stream.shutdown().await?;
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "chatgpt_compat.request",
            json!({
                "method": method,
                "path": path_only,
                "source": "Dictation",
                "status": status,
            }),
        );
        return Ok(());
    }
    // 插件目录和插件包用 Codex++ 的缓存现答（见 [`plugins`]），其余走录制 / 合成应答。
    if let Some(reply) = plugins::reply(&method, &path, &request, &body).await {
        write_body(&mut stream, reply.status, reply.content_type, &reply.body).await?;
        stream.shutdown().await?;
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "chatgpt_compat.request",
            json!({
                "method": method,
                // 插件接口的查询串（分页、范围、搜索词）留着，排查 Codex 实际怎么请求时要看。
                "path": path,
                "source": "Plugins",
                "status": reply.status,
            }),
        );
        return Ok(());
    }
    let (source, reply) = reply_for(&method, &path);
    write_json(&mut stream, reply.status, &reply.body).await?;
    stream.shutdown().await?;
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "chatgpt_compat.request",
        json!({
            "method": method,
            "path": path_only,
            "source": format!("{source:?}"),
        }),
    );
    Ok(())
}

/// 从请求头提取指定头部（大小写不敏感）。
fn request_header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then_some(value.trim())
    })
}

async fn transcribe_reply(headers: &str, body: &[u8]) -> (u16, String) {
    let settings = match crate::settings::SettingsStore::default().load() {
        Ok(settings) => settings,
        Err(_) => {
            return (
                500,
                json!({
                    "detail": {
                        "code": "transcription_failed",
                        "message": "无法读取 Codex++ 语音输入设置",
                    }
                })
                .to_string(),
            );
        }
    };
    let effective = crate::dictation::effective_settings(&settings);
    transcribe_reply_with_settings(&effective, headers, body).await
}

async fn transcribe_reply_with_settings(
    settings: &crate::settings::DictationSettings,
    headers: &str,
    body: &[u8],
) -> (u16, String) {
    if !settings.enabled {
        return (
            403,
            json!({
                "detail": {
                    "code": "transcription_failed",
                    "message": "请先在 Codex++「设置 → 语音输入」中启用并配置语音服务",
                }
            })
            .to_string(),
        );
    }
    let content_type = request_header(headers, "content-type").unwrap_or("");
    match crate::dictation::transcribe(settings, body, content_type).await {
        Ok(value) => (200, value.to_string()),
        Err(error) => (
            error.status,
            json!({
                "detail": {
                    "code": "transcription_failed",
                    "message": error.message,
                }
            })
            .to_string(),
        ),
    }
}

/// 普通接口正文最多读这么多（插件批量查询的 id 列表用得到，别的应答不看正文）。
const BODY_LIMIT: usize = 1024 * 1024;
/// 原生语音听写上传的音频正文上限（25 MiB 音频 + multipart 头部余量）。
const TRANSCRIBE_BODY_LIMIT: usize = crate::dictation::MAX_AUDIO_BODY_BYTES + 256 * 1024;

struct IncomingRequest {
    headers: String,
    body: Vec<u8>,
    body_too_large: bool,
}

fn body_limit_for_headers(headers: &str) -> usize {
    let path = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let path_only = path.split('?').next().unwrap_or(path);
    if path_only.ends_with("/transcribe") {
        TRANSCRIBE_BODY_LIMIT
    } else {
        BODY_LIMIT
    }
}

async fn read_chunked_body<S>(
    stream: &mut S,
    initial: Vec<u8>,
    limit: usize,
) -> anyhow::Result<(Vec<u8>, bool)>
where
    S: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut raw = initial;
    let mut decoded = Vec::new();
    let mut cursor = 0usize;
    let mut buf = [0u8; 8192];
    loop {
        let line_end = loop {
            if let Some(pos) = raw[cursor..].windows(2).position(|w| w == b"\r\n") {
                break cursor + pos;
            }
            let read = stream.read(&mut buf).await?;
            if read == 0 {
                return Ok((decoded, false));
            }
            raw.extend_from_slice(&buf[..read]);
            if raw.len().saturating_sub(cursor) > 16 * 1024 {
                return Ok((Vec::new(), true));
            }
        };
        let size_line = String::from_utf8_lossy(&raw[cursor..line_end]);
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let Ok(chunk_len) = usize::from_str_radix(size_hex, 16) else {
            return Ok((decoded, false));
        };
        cursor = line_end + 2;
        if chunk_len == 0 {
            return Ok((decoded, false));
        }
        if decoded.len().saturating_add(chunk_len) > limit {
            return Ok((Vec::new(), true));
        }
        let needed = cursor + chunk_len + 2;
        while raw.len() < needed {
            let read = stream.read(&mut buf).await?;
            if read == 0 {
                let avail = raw.len().saturating_sub(cursor).min(chunk_len);
                decoded.extend_from_slice(&raw[cursor..cursor + avail]);
                return Ok((decoded, false));
            }
            raw.extend_from_slice(&buf[..read]);
        }
        decoded.extend_from_slice(&raw[cursor..cursor + chunk_len]);
        cursor = needed;
        if cursor > 64 * 1024 {
            raw.drain(..cursor);
            cursor = 0;
        }
    }
}

/// 读请求头（以空行分隔），再按 `transfer-encoding: chunked` 或 `content-length` 读正文。
async fn read_request(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) -> anyhow::Result<Option<IncomingRequest>> {
    use tokio::io::AsyncReadExt;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break buffer.len();
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if buffer.len() >= HEADER_LIMIT {
            break buffer.len();
        }
    };
    if buffer.is_empty() {
        return Ok(None);
    }
    let headers = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let limit = body_limit_for_headers(&headers);
    let initial = buffer[header_end.min(buffer.len())..].to_vec();
    let is_chunked = request_header(&headers, "transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"));
    if is_chunked {
        let (body, body_too_large) = read_chunked_body(stream, initial, limit).await?;
        return Ok(Some(IncomingRequest {
            headers,
            body,
            body_too_large,
        }));
    }
    let length = request_header(&headers, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if length > limit {
        return Ok(Some(IncomingRequest {
            headers,
            body: Vec::new(),
            body_too_large: true,
        }));
    }
    let mut body = initial;
    while body.len() < length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length);
    Ok(Some(IncomingRequest {
        headers,
        body,
        body_too_large: false,
    }))
}

async fn write_json(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    status: u16,
    body: &str,
) -> anyhow::Result<()> {
    write_body(stream, status, JSON, body.as_bytes()).await
}

async fn write_body(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let reason = if (200..300).contains(&status) { "OK" } else { "Error" };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 资料是进程级的，改它的用例串行。
    static PROFILE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn recordings_are_embedded() {
        assert!(recordings().len() > 40, "脱敏录制应随构建内嵌");
        let (source, reply) = reply_for("GET", "/backend-api/me");
        assert_eq!(source, ReplySource::Recorded);
        assert!(reply.body.contains(ACCOUNT_ID) || reply.body.contains("local"));
    }

    #[test]
    fn token_reply_decodes_to_the_fake_account() {
        let (_, reply) = reply_for("POST", "/oauth/token");
        let body: Value = serde_json::from_str(&reply.body).unwrap();
        let token = body["id_token"].as_str().unwrap();
        let payload = token.split('.').nth(1).unwrap();
        use base64::Engine;
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap();
        let claims: Value = serde_json::from_slice(&decoded).unwrap();
        let auth = &claims["https://api.openai.com/auth"];
        assert_eq!(auth["chatgpt_account_id"], ACCOUNT_ID);
        assert_eq!(auth["chatgpt_plan_type"], PLAN);
    }

    #[test]
    fn unknown_get_list_returns_empty_items() {
        let (source, reply) = reply_for("GET", "/backend-api/somewhere/items?x=1");
        assert_eq!(source, ReplySource::Synthetic);
        assert_eq!(reply.body, r#"{"items":[]}"#);
    }

    #[test]
    fn connector_link_is_null_not_missing() {
        let (_, reply) = reply_for("GET", "/backend-api/aip/connectors/abc/link");
        assert_eq!(reply.body, r#"{"link":null}"#);
    }

    #[test]
    fn apps_batch_returns_empty_apps() {
        let (_, reply) = reply_for("POST", "/backend-api/ps/apps/batch");
        assert_eq!(reply.body, r#"{"apps":[]}"#);
    }

    #[test]
    fn well_known_is_not_found() {
        let (source, reply) = reply_for("GET", "/.well-known/oauth-authorization-server");
        assert_eq!(source, ReplySource::NotFound);
        assert_eq!(reply.status, 404);
    }

    #[test]
    fn recorded_models_are_served_not_forwarded() {
        let (source, reply) = reply_for("GET", "/backend-api/models");
        assert_eq!(source, ReplySource::Recorded);
        assert!(reply.body.contains("slug"));
    }

    #[test]
    fn spki_hash_is_base64_sha256() {
        let _guard = crate::paths::settings_path_test_guard();
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os(crate::paths::STATE_DIR_ENV);
        unsafe { std::env::set_var(crate::paths::STATE_DIR_ENV, dir.path()) };
        let hash = spki_hash();
        unsafe {
            match previous {
                Some(value) => std::env::set_var(crate::paths::STATE_DIR_ENV, value),
                None => std::env::remove_var(crate::paths::STATE_DIR_ENV),
            }
        }
        let hash = hash.unwrap();
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(&hash).unwrap();
        assert_eq!(bytes.len(), 32);
    }

    /// 句柄比监听者活得久时也要能把服务重新拉起来：以前只按句柄判断「已经在跑」，
    /// 端口早就没人听也会直接返回 Ok，服务从此再也起不来。
    #[tokio::test]
    async fn start_rebinds_after_the_previous_listener_is_gone() {
        // 端口是进程级固定资源：本机已经有服务在听（开发机常常开着）就跳过。
        let Ok(probe) = tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await else {
            return;
        };
        drop(probe);
        // 证书落在状态目录里：拿进程级的设置路径锁，再用自己的临时目录，别和并行用例抢同一个文件。
        let _guard = crate::paths::settings_path_test_guard();
        let state = tempfile::tempdir().unwrap();
        struct RestoreStateDir(Option<std::ffi::OsString>);
        impl Drop for RestoreStateDir {
            fn drop(&mut self) {
                // SAFETY: 整个用例持着设置路径锁，没有别的用例同时读这个变量。
                unsafe {
                    match self.0.take() {
                        Some(previous) => std::env::set_var(crate::paths::STATE_DIR_ENV, previous),
                        None => std::env::remove_var(crate::paths::STATE_DIR_ENV),
                    }
                }
            }
        }
        let _restore = {
            let previous = std::env::var_os(crate::paths::STATE_DIR_ENV);
            // SAFETY: 同上。
            unsafe { std::env::set_var(crate::paths::STATE_DIR_ENV, state.path()) };
            RestoreStateDir(previous)
        };
        stop();
        // 假装表里还留着一个句柄，但它对应的监听者已经不在了。
        let (shutdown, _receiver) = tokio::sync::watch::channel(false);
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *SERVER.lock().expect("chatgpt compat server lock") = Some(ServerHandle {
            shutdown,
            generation,
        });
        assert!(running(), "前提：表里留着一个陈旧句柄");

        start().await.unwrap();
        let rebound = tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await;
        assert!(rebound.is_err(), "start() 应该真的绑上端口，而不是信陈旧句柄");
        stop();
    }

    /// 在随机端口真起服务，用只信任这张证书的 TLS 客户端连上去读应答。
    #[tokio::test(flavor = "multi_thread")]
    async fn server_answers_over_tls_with_the_embedded_recordings() {
        let dir = tempfile::tempdir().unwrap();
        let cert = crate::remote_control::tls::ensure_certificate_at(&dir.path().join("c.json"), &[]).unwrap();
        let mut config = (*crate::remote_control::tls::server_config(&cert).unwrap()).clone();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
        let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await else { return };
        let port = listener.local_addr().unwrap().port();
        let (_tx, rx) = tokio::sync::watch::channel(false);
        tokio::spawn(serve(listener, acceptor, rx));

        let client = reqwest::Client::builder()
            .add_root_certificate(reqwest::Certificate::from_pem(cert.cert_pem.as_bytes()).unwrap())
            .no_proxy()
            .build()
            .unwrap();
        let base = format!("https://localhost:{port}");
        let me: Value = client.get(format!("{base}/backend-api/me")).send().await.unwrap().json().await.unwrap();
        assert_eq!(me["email"], DEFAULT_EMAIL);
        let token: Value = client
            .post(format!("{base}/oauth/token"))
            .body("grant_type=refresh_token")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(token["access_token"].as_str().unwrap().contains('.'));
        let missing = client.get(format!("{base}/.well-known/openid-configuration")).send().await.unwrap();
        assert_eq!(missing.status().as_u16(), 404);
        // 1.3MB 的 statsig 录制要完整读回来。
        let statsig = client
            .post(format!("{base}/backend-api/wham/statsig/bootstrap"))
            .body("{}")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(statsig.len() > 1_000_000);
    }

    #[test]
    fn profile_replaces_only_personal_fields() {
        let _guard = PROFILE_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        set_profile(Profile::from_settings("小黄", "  ", "me@example.com"));
        let (_, me) = reply_for("GET", "/backend-api/me");
        let me: Value = serde_json::from_str(&me.body).unwrap();
        assert_eq!(me["name"], "小黄");
        assert_eq!(me["email"], "me@example.com");
        let (_, profile) = reply_for("GET", "/backend-api/wham/profiles/me");
        let profile: Value = serde_json::from_str(&profile.body).unwrap();
        assert_eq!(profile["profile"]["display_name"], "小黄");
        assert_eq!(profile["profile"]["username"], DEFAULT_USERNAME);
        // 插件目录是公开数据，名字不跟着改。
        let (_, plugins) = reply_for("GET", "/backend-api/ps/plugins/installed");
        assert!(plugins.body.contains("\"GitHub\""));
        assert!(!plugins.body.contains("小黄"));
        set_profile(Profile::default());
    }

    #[test]
    fn fake_auth_keeps_existing_api_key() {
        let existing = r#"{"OPENAI_API_KEY":"sk-real","auth_mode":"apikey"}"#;
        let merged: Value = serde_json::from_str(&fake_auth_json(Some(existing))).unwrap();
        assert_eq!(merged["OPENAI_API_KEY"], "sk-real");
        assert_eq!(merged["auth_mode"], "chatgpt");
        assert!(merged["tokens"]["id_token"].as_str().unwrap().contains('.'));
        assert!(merged["last_refresh"].as_str().unwrap().contains('T'));
    }

    #[test]
    fn virtual_account_recordings_report_active_pro_plan() {
        let (_, check) = reply_for("GET", "/backend-api/accounts/check/v4-2023-04-27");
        let check: Value = serde_json::from_str(&check.body).unwrap();
        for key in ["acc-local", "default"] {
            assert_eq!(check["accounts"][key]["account"]["plan_type"], "pro");
            assert_eq!(check["accounts"][key]["account"]["plan_display_name"], "Pro");
            assert_eq!(check["accounts"][key]["entitlement"]["has_active_subscription"], true);
            assert_eq!(check["accounts"][key]["has_eligible_promotion"], false);
        }
        let (_, wham_check) = reply_for("GET", "/backend-api/wham/accounts/check");
        let wham_check: Value = serde_json::from_str(&wham_check.body).unwrap();
        assert_eq!(wham_check["accounts"][0]["plan_type"], "pro");

        let (_, usage) = reply_for("GET", "/backend-api/wham/usage");
        let usage: Value = serde_json::from_str(&usage.body).unwrap();
        assert_eq!(usage["plan_type"], "pro");

        let (_, storage) = reply_for("GET", "/backend-api/files/library/storage/usage");
        let storage: Value = serde_json::from_str(&storage.body).unwrap();
        assert_eq!(storage["plan_type"], "pro");
        assert_eq!(storage["limit_tier"], "pro");

        let (_, statsig) = reply_for("POST", "/backend-api/wham/statsig/bootstrap");
        assert!(statsig.body.contains(r#"\"plan_type\":\"pro\""#));
    }

    #[test]
    fn settings_personalization_and_profile_endpoints_return_safe_shapes() {
        let _guard = PROFILE_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        set_profile(Profile::from_settings("测试用户", "tester", "t@example.com"));
        let (_, metrics) = reply_for("GET", "/backend-api/wham/analytics/daily-code-review-metrics?days=30");
        let metrics: Value = serde_json::from_str(&metrics.body).unwrap();
        assert!(metrics["data"].is_array());

        let (_, profile_page) = reply_for("GET", "/backend-api/profiles/me/page");
        let profile_page: Value = serde_json::from_str(&profile_page.body).unwrap();
        assert_eq!(profile_page["is_self"], true);
        assert_eq!(profile_page["profile_details"]["username"], "tester");
        assert_eq!(profile_page["profile_details"]["display_name"], "测试用户");
        assert_eq!(profile_page["page"]["visibility"]["value"], "private");
        assert!(profile_page["page"]["top_plugins_by_remote_id"].is_array());

        let (_, personality_types) = reply_for("GET", "/backend-api/personality_types");
        let personality_types: Value = serde_json::from_str(&personality_types.body).unwrap();
        assert!(personality_types.is_array());

        let (_, personality_traits) = reply_for("GET", "/backend-api/personality_trait_types");
        let personality_traits: Value = serde_json::from_str(&personality_traits.body).unwrap();
        assert!(personality_traits["trait_types"].is_array());

        let (_, notifications) = reply_for("GET", "/backend-api/notifications/settings");
        let notifications: Value = serde_json::from_str(&notifications.body).unwrap();
        assert!(notifications["settings"].is_array());

        let (_, chatpass) = reply_for("GET", "/backend-api/wham/usage/chatpass/apps");
        let chatpass: Value = serde_json::from_str(&chatpass.body).unwrap();
        assert!(chatpass["items"].is_array());

        let (_, voices) = reply_for("GET", "/backend-api/settings/voices");
        let voices: Value = serde_json::from_str(&voices.body).unwrap();
        assert!(voices["voices"].is_array());

        let (_, browser_cfg) = reply_for("GET", "/backend-api/flora/browser/config");
        let browser_cfg: Value = serde_json::from_str(&browser_cfg.body).unwrap();
        assert!(browser_cfg["origins"].is_array());

        let (_, browser_cookies) = reply_for("GET", "/backend-api/flora/browser/cookies?surface=Orbit");
        let browser_cookies: Value = serde_json::from_str(&browser_cookies.body).unwrap();
        assert!(browser_cookies["target"].is_object());

        let (_, saved_creds) = reply_for("GET", "/backend-api/saved-credentials");
        let saved_creds: Value = serde_json::from_str(&saved_creds.body).unwrap();
        assert!(saved_creds["items"].is_array());

        let (_, image_tags) = reply_for("GET", "/backend-api/images/image-tags");
        let image_tags: Value = serde_json::from_str(&image_tags.body).unwrap();
        assert!(image_tags["items"].is_array());

        let (_, memories) = reply_for("GET", "/backend-api/memories?include_memory_entries=true");
        let memories: Value = serde_json::from_str(&memories.body).unwrap();
        assert!(memories["memories"].is_array());

        let (_, trusted) = reply_for("GET", "/backend-api/trusted_contact/enabled");
        let trusted: Value = serde_json::from_str(&trusted.body).unwrap();
        assert_eq!(trusted["enabled"], false);

        let (_, featured) = reply_for("GET", "/backend-api/plugins/featured");
        let featured: Value = serde_json::from_str(&featured.body).unwrap();
        assert!(featured.is_array());

        let (_, github_acct) = reply_for("POST", "/backend-api/wham/github/operations/account");
        let github_acct: Value = serde_json::from_str(&github_acct.body).unwrap();
        assert_eq!(github_acct["currentUser"]["status"], "unauthenticated");
        set_profile(Profile::default());
    }

    #[tokio::test]
    async fn transcribe_endpoint_forwards_native_multipart_and_handles_disabled() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "你好世界" })))
            .mount(&server)
            .await;

        let boundary = "----codex-transcribe-test-1234";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"codex.webm\"\r\nContent-Type: audio/webm;codecs=opus\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(b"fake-webm-bytes");
        body.extend_from_slice(
            format!(
                "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\nzh\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"dictation_session_id\"\r\n\r\nsess-1\r\n--{boundary}--\r\n"
            )
            .as_bytes(),
        );
        let headers = format!(
            "POST /backend-api/transcribe HTTP/1.1\r\nHost: localhost:8000\r\nContent-Type: multipart/form-data; boundary={boundary}\r\n\r\n"
        );

        let disabled = crate::settings::DictationSettings::default();
        let (status, err_body) = transcribe_reply_with_settings(&disabled, &headers, &body).await;
        assert_eq!(status, 403);
        let err_json: Value = serde_json::from_str(&err_body).unwrap();
        assert!(err_json["detail"]["message"].as_str().unwrap().contains("语音输入"));

        let enabled = crate::settings::DictationSettings {
            enabled: true,
            provider: crate::settings::DictationProvider::Api,
            base_url: format!("{}/v1", server.uri()),
            api_key: "sk-test-asr".to_string(),
            model: "whisper-large-v3-turbo".to_string(),
            ..Default::default()
        };
        let (status, ok_body) = transcribe_reply_with_settings(&enabled, &headers, &body).await;
        assert_eq!(status, 200, "body={ok_body}");
        let ok_json: Value = serde_json::from_str(&ok_body).unwrap();
        assert_eq!(ok_json["text"], "你好世界");
    }

    #[tokio::test]
    async fn chunked_body_reader_and_transcribe_limit_work() {
        assert_eq!(
            body_limit_for_headers("POST /backend-api/transcribe HTTP/1.1\r\n\r\n"),
            TRANSCRIBE_BODY_LIMIT
        );
        assert_eq!(
            body_limit_for_headers("POST /backend-api/ps/apps/batch HTTP/1.1\r\n\r\n"),
            BODY_LIMIT
        );

        let chunked = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let mut cursor = std::io::Cursor::new(&chunked[..]);
        let (decoded, too_large) = read_chunked_body(&mut cursor, Vec::new(), 64)
            .await
            .unwrap();
        assert!(!too_large);
        assert_eq!(decoded, b"hello world");

        let mut cursor = std::io::Cursor::new(&chunked[..]);
        let (_, too_large) = read_chunked_body(&mut cursor, Vec::new(), 8).await.unwrap();
        assert!(too_large);
    }
}
