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
    if let Some(recording) = recordings().get(&format!("{method} {path_only}")) {
        return (
            ReplySource::Recorded,
            Reply {
                status: recording.status,
                content_type: JSON,
                body: personalize(path_only, recording.body),
            },
        );
    }
    (ReplySource::Synthetic, synthetic_reply(method, path_only))
}

/// 没录到的接口的最小响应。形状按客户端实际读取的字段给，给错会让页面报错。
fn synthetic_reply(method: &str, path: &str) -> Reply {
    let body = if path.ends_with("/aip/connectors/links/list_accessible") {
        r#"{"links":[]}"#
    } else if path.ends_with("/ps/apps/batch") || path.ends_with("/ps/plugins/batch") {
        r#"{"apps":[]}"#
    } else if path.ends_with("/link") {
        r#"{"link":null}"#
    } else if method == "GET" && (path.ends_with("/items") || path.ends_with("/list")) {
        r#"{"items":[]}"#
    } else {
        "{}"
    };
    Reply {
        status: 200,
        content_type: JSON,
        body: body.to_string(),
    }
}

const JSON: &str = "application/json";

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
}

static SERVER: Mutex<Option<ServerHandle>> = Mutex::new(None);

/// 启动兼容服务。已经在跑时直接返回。端口被占用时返回错误，由调用方提示。
pub async fn start() -> anyhow::Result<()> {
    if running() {
        return Ok(());
    }
    let cert = ensure_certificate()?;
    let mut config = (*crate::remote_control::tls::server_config(&cert)?).clone();
    // 只说 HTTP/1.1：Chromium 默认先协商 h2，这里只会按 HTTP/1.1 应答。
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", PORT))
        .await
        .map_err(|error| {
            anyhow::anyhow!("兼容服务绑定 127.0.0.1:{PORT} 失败：{error}（端口可能被占用）")
        })?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    *SERVER.lock().expect("chatgpt compat server lock") = Some(ServerHandle {
        shutdown: shutdown_tx,
    });
    tokio::spawn(serve(listener, acceptor, shutdown_rx));
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
    let Some((request, body)) = read_request(&mut stream).await? else {
        return Ok(());
    };
    let mut lines = request.lines();
    let Some(start) = lines.next() else { return Ok(()) };
    let mut parts = start.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();
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
            "path": path.split('?').next().unwrap_or(&path),
            "source": format!("{source:?}"),
        }),
    );
    Ok(())
}

/// 正文最多读这么多（插件批量查询的 id 列表用得到，别的应答不看正文）。
const BODY_LIMIT: usize = 1024 * 1024;

/// 读请求头（以空行分隔），再按 `content-length` 读正文（超过上限的不读）。
async fn read_request(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) -> anyhow::Result<Option<(String, Vec<u8>)>> {
    use tokio::io::AsyncReadExt;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
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
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim().eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = buffer[header_end.min(buffer.len())..].to_vec();
    if length <= BODY_LIMIT {
        while body.len() < length {
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read]);
        }
        body.truncate(length);
    } else {
        body.clear();
    }
    Ok(Some((headers, body)))
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
}
