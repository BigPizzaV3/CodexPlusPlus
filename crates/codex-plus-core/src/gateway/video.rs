//! 视频生成：把 Agent 发来的 OpenAI 风格异步 Videos 请求转给供应商池。
//!
//! 对外统一成 `POST /videos`、`GET /videos/{id}`、`GET /videos/{id}/content`。
//! 视频模型不参与聊天。创建任务时按 Agent 的选择尝试供应商；任务创建成功后，
//! 本地 ID 会固定到当时选中的供应商、Key 池和适配器，后续轮询不会换家。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::gateway::store::{GatewayProvider, GatewaySettings};
use crate::settings::RelayProfile;
use crate::tools::ToolId;

/// 关闭视频生成时存的值。
pub const VIDEO_OFF: &str = "off";

const VIDEO_CONTENT_TIMEOUT: Duration = Duration::from_secs(300);
const VIDEO_CONTENT_LIMIT: u64 = 512 * 1024 * 1024;
const ERROR_PREVIEW_LIMIT: usize = 1_000;

/// 视频模型的名字特征。图片模型先由 `images` 挑走，这里只负责视频专用名字；
/// 两边都命中的模型（例如 `wanx-video`）会按视频处理。
const VIDEO_MODEL_MARKS: &[&str] = &[
    "video",
    "sora",
    "veo",
    "kling",
    "hailuo",
    "seedance",
    "runway",
    "pika",
    "luma",
    "vidu",
    "mochi",
    "cogvideo",
    "dream-machine",
    "t2v",
    "i2v",
];

/// 按名字判断是不是视频模型（不分大小写，忽略供应商前缀）。
pub fn is_video_model(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    let name = model.rsplit('/').next().unwrap_or(&model);
    VIDEO_MODEL_MARKS.iter().any(|mark| name.contains(mark))
}

/// 上游协议。对外始终是 OpenAI 风格 Videos，供应商差异收在适配器里。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VideoAdapter {
    OpenAi,
    Xai,
    DashScope,
}

/// 界面上「视频模型」下拉的一项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoOption {
    /// 存进 `video_models` 的值：`<供应商 id>/<模型>`。
    pub id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub adapter: VideoAdapter,
}

/// 一家供应商能生成视频的模型。
pub fn provider_video_models(relay: &RelayProfile) -> Vec<String> {
    crate::gateway::route::relay_models(relay)
        .into_iter()
        .filter(|model| is_video_model(model))
        .collect()
}

/// 能生成视频的选项，按供应商池顺序排列。
pub fn options(gateway: &GatewaySettings) -> Vec<VideoOption> {
    let mut options = Vec::new();
    for provider in gateway.providers.iter().filter(|provider| provider.enabled) {
        for model in provider_video_models(&provider.relay) {
            options.push(VideoOption {
                id: format!("{}/{model}", provider.relay.id),
                provider_id: provider.relay.id.clone(),
                provider_name: provider.relay.name.clone(),
                adapter: detect_adapter(&provider.relay, &model),
                model,
            });
        }
    }
    options
}

/// 一个视频上游：发给哪家、请求里的模型名换成什么、使用哪个适配器。
#[derive(Debug, Clone)]
pub struct VideoTarget {
    pub relay: RelayProfile,
    pub model: String,
    pub adapter: VideoAdapter,
}

/// 按这个 Agent 的设置排出创建任务时要依次尝试的上游。
pub fn targets(
    gateway: &GatewaySettings,
    agent: &ToolId,
    requested: Option<&str>,
) -> Result<Vec<VideoTarget>, String> {
    let wanted = gateway.video_model_for(agent).trim();
    if wanted.eq_ignore_ascii_case(VIDEO_OFF) {
        return Err(
            "Codex++ 网关里这个 Agent 的视频生成已关闭：在「Agent 接入」里选一个视频模型"
                .to_string(),
        );
    }
    let target = |option: &VideoOption| {
        gateway
            .provider(&option.provider_id)
            .map(|provider| VideoTarget {
                relay: provider.relay.clone(),
                model: option.model.clone(),
                adapter: option.adapter,
            })
    };
    let all = options(gateway);
    if wanted.is_empty() {
        let mut targets = Vec::new();
        let mut seen = Vec::new();
        for option in &all {
            if seen.contains(&option.provider_id.as_str()) {
                continue;
            }
            seen.push(option.provider_id.as_str());
            let asked = requested.and_then(|requested| {
                all.iter().find(|candidate| {
                    candidate.provider_id == option.provider_id
                        && candidate.model.eq_ignore_ascii_case(requested.trim())
                })
            });
            targets.extend(target(asked.unwrap_or(option)));
        }
        if targets.is_empty() {
            return Err(
                "Codex++ 网关的供应商池里没有视频模型：给供应商手动加上视频模型名（如 wan2.7-video、grok-video、veo-3）"
                    .to_string(),
            );
        }
        return Ok(targets);
    }
    if let Some(option) = all
        .iter()
        .find(|option| option.id.eq_ignore_ascii_case(wanted))
    {
        return Ok(target(option).into_iter().collect());
    }
    // 名字没被识别，或那家暂时关闭/被删：供应商仍存在时按显式选择原样发送。
    let (provider_id, model) = wanted.split_once('/').unwrap_or(("", wanted));
    match gateway
        .providers
        .iter()
        .find(|provider| provider.enabled && provider.relay.id == provider_id)
    {
        Some(provider) => Ok(vec![VideoTarget {
            relay: provider.relay.clone(),
            model: model.to_string(),
            adapter: detect_adapter(&provider.relay, model),
        }]),
        None => Err(format!(
            "视频模型 {wanted} 所在的供应商已删除或关闭：在「Agent 接入」里换一个"
        )),
    }
}

/// 自动时这次会先用哪一项（界面上显示「自动 · xxx」）。
pub fn auto_choice(gateway: &GatewaySettings) -> Option<VideoOption> {
    options(gateway).into_iter().next()
}

/// 请求 JSON 里的 `model`。
pub fn requested_model(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .get("model")?
        .as_str()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
}

/// 请求创建失败时带回 HTTP 状态。
#[derive(Debug, Clone)]
pub struct VideoError {
    pub status: u16,
    pub message: String,
}

impl VideoError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            message: message.into(),
        }
    }

    fn upstream(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: 502,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for VideoError {}

#[derive(Debug, Clone)]
struct VideoJob {
    id: String,
    agent: String,
    provider_id: String,
    upstream_id: String,
    model: String,
    adapter: VideoAdapter,
    created_at: u64,
    content_url: Option<String>,
}

static JOBS: OnceLock<Mutex<HashMap<String, VideoJob>>> = OnceLock::new();

fn jobs() -> &'static Mutex<HashMap<String, VideoJob>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remember(mut job: VideoJob) -> VideoJob {
    job.id = format!("video_{}", uuid::Uuid::new_v4().simple());
    let id = job.id.clone();
    let mut registry = jobs().lock().expect("video job registry poisoned");
    // 视频任务通常只存活几十分钟；顺手清掉一天前的记录，避免常驻进程无限增长。
    let now = now_secs();
    registry.retain(|_, item| now.saturating_sub(item.created_at) < 24 * 60 * 60);
    registry.insert(id, job.clone());
    job
}

fn job_for(agent: &ToolId, id: &str) -> Result<VideoJob, VideoError> {
    jobs()
        .lock()
        .map_err(|_| VideoError::bad_gateway("视频任务状态不可用，请重启 Codex++ 网关"))?
        .get(id)
        .filter(|job| job.agent == agent.as_str())
        .cloned()
        .ok_or_else(|| {
            VideoError::upstream(
                404,
                "视频任务不存在，或不属于这个 Agent。任务状态只在当前 Codex++ 网关进程内保留",
            )
        })
}

fn update_content_url(id: &str, content_url: Option<&str>) {
    if let Some(content_url) = content_url
        && let Ok(mut registry) = jobs().lock()
        && let Some(job) = registry.get_mut(id)
    {
        job.content_url = Some(content_url.to_string());
    }
}

/// 创建视频任务。仅在创建阶段按供应商池顺序尝试；成功后的轮询绑定到这个任务。
pub async fn create(
    gateway: &GatewaySettings,
    agent: &ToolId,
    body: &[u8],
    user_agent: Option<&str>,
) -> Result<Value, VideoError> {
    let request: Value =
        serde_json::from_slice(body).map_err(|_| VideoError::bad_request("视频请求必须是 JSON"))?;
    if !request.is_object() {
        return Err(VideoError::bad_request("视频请求必须是 JSON 对象"));
    }
    let requested = request
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty());
    let targets = targets(gateway, agent, requested)
        .map_err(VideoError::bad_request)?
        .into_iter()
        .collect::<Vec<_>>();
    let count = targets.len();
    let mut last_error = String::new();
    for (index, target) in targets.into_iter().enumerate() {
        let has_more = index + 1 < count;
        let payload = create_payload(target.adapter, &target.model, &request);
        let endpoint = create_endpoint(&target.relay, target.adapter, &target.model);
        let response = match send_with_provider_key(
            gateway,
            &target.relay,
            reqwest::Method::POST,
            &endpoint,
            Some(&payload),
            user_agent,
            target.adapter,
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                last_error = format!("{}：{error}", target.relay.name);
                continue;
            }
        };
        if has_more && should_try_next(response.status) {
            last_error = format!(
                "{} 返回 {}：{}",
                target.relay.name,
                response.status,
                preview(&response.body)
            );
            continue;
        }
        if !(200..300).contains(&response.status) {
            return Err(VideoError::upstream(
                response.status,
                format!(
                    "视频上游 {} 返回 {}：{}",
                    target.relay.name,
                    response.status,
                    preview(&response.body)
                ),
            ));
        }
        let upstream: Value = match serde_json::from_slice(&response.body) {
            Ok(value) => value,
            Err(_) => {
                last_error = format!("{}：视频上游没有返回 JSON", target.relay.name);
                continue;
            }
        };
        let Some(upstream_id) = create_job_id(target.adapter, &upstream) else {
            last_error = format!("{}：视频上游没有返回任务 ID", target.relay.name);
            continue;
        };
        let normalized = normalize_status(target.adapter, &upstream, target.model.as_str());
        let content_url = extract_content_url(&upstream);
        let job = remember(VideoJob {
            id: String::new(),
            agent: agent.as_str().to_string(),
            provider_id: target.relay.id.clone(),
            upstream_id,
            model: target.model.clone(),
            adapter: target.adapter,
            created_at: now_secs(),
            content_url,
        });
        return Ok(public_status(&job, normalized));
    }
    Err(VideoError::bad_gateway(format!(
        "视频任务创建失败：{last_error}"
    )))
}

/// 轮询一个已经创建的视频任务。
pub async fn status(
    gateway: &GatewaySettings,
    agent: &ToolId,
    id: &str,
    user_agent: Option<&str>,
) -> Result<Value, VideoError> {
    let job = job_for(agent, id)?;
    let provider = gateway.provider(&job.provider_id).ok_or_else(|| {
        VideoError::upstream(
            410,
            format!(
                "视频任务绑定的供应商 {} 已被删除，无法继续轮询",
                job.provider_id
            ),
        )
    })?;
    let endpoint = status_endpoint(&provider.relay, job.adapter, &job.upstream_id);
    let response = send_with_provider_key(
        gateway,
        &provider.relay,
        reqwest::Method::GET,
        &endpoint,
        None,
        user_agent,
        job.adapter,
    )
    .await
    .map_err(VideoError::bad_gateway)?;
    if !(200..300).contains(&response.status) {
        return Err(VideoError::upstream(
            response.status,
            format!(
                "视频状态上游 {} 返回 {}：{}",
                provider.relay.name,
                response.status,
                preview(&response.body)
            ),
        ));
    }
    let upstream: Value = serde_json::from_slice(&response.body)
        .map_err(|_| VideoError::bad_gateway("视频状态上游没有返回 JSON"))?;
    let content_url = extract_content_url(&upstream);
    update_content_url(id, content_url.as_deref());
    Ok(public_status(
        &job,
        normalize_status(job.adapter, &upstream, &job.model),
    ))
}

/// 视频文件。优先代理状态里给出的签名链接；标准 OpenAI 兼容供应商没有链接时，
/// 再请求 `/videos/{id}/content`。
pub async fn content(
    gateway: &GatewaySettings,
    agent: &ToolId,
    id: &str,
    user_agent: Option<&str>,
) -> Result<VideoContent, VideoError> {
    let mut job = job_for(agent, id)?;
    let provider = gateway.provider(&job.provider_id).ok_or_else(|| {
        VideoError::upstream(
            410,
            format!("视频任务绑定的供应商 {} 已被删除", job.provider_id),
        )
    })?;
    if job.content_url.is_none() {
        let _ = status(gateway, agent, id, user_agent).await?;
        job = job_for(agent, id)?;
    }
    if let Some(url) = job.content_url.as_deref() {
        let response = download_content_url(url, user_agent, &provider.relay).await?;
        return Ok(VideoContent {
            content_type: response.content_type,
            body: response.body,
        });
    }
    if job.adapter != VideoAdapter::OpenAi {
        return Err(VideoError::upstream(
            409,
            "视频任务还没有生成文件链接，请稍后再轮询状态",
        ));
    }
    let endpoint = content_endpoint(&provider.relay, &job.upstream_id);
    let response = send_with_provider_key(
        gateway,
        &provider.relay,
        reqwest::Method::GET,
        &endpoint,
        None,
        user_agent,
        job.adapter,
    )
    .await
    .map_err(VideoError::bad_gateway)?;
    if !(200..300).contains(&response.status) {
        return Err(VideoError::upstream(
            response.status,
            format!(
                "视频文件上游 {} 返回 {}：{}",
                provider.relay.name,
                response.status,
                preview(&response.body)
            ),
        ));
    }
    Ok(VideoContent {
        content_type: response.content_type,
        body: response.body,
    })
}

pub struct VideoContent {
    pub content_type: String,
    pub body: Vec<u8>,
}

struct RawResponse {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

async fn send_with_provider_key(
    gateway: &GatewaySettings,
    relay: &RelayProfile,
    method: reqwest::Method,
    endpoint: &str,
    body: Option<&Value>,
    user_agent: Option<&str>,
    adapter: VideoAdapter,
) -> Result<RawResponse, String> {
    if crate::official_login::member_uses_official_login_transport(relay) {
        return Err(
            "视频接口需要带 Base URL 和 Key 的 API 供应商；订阅登录账号暂不支持".to_string(),
        );
    }
    let (keys, _) = crate::gateway::key_pool::order_keys_with(
        relay,
        gateway.key_policy.mode,
        None,
        std::time::Instant::now(),
        true,
    );
    let limits = gateway
        .provider(&relay.id)
        .map(GatewayProvider::key_limits)
        .unwrap_or_default();
    let Some((key, _slot)) = keys.into_iter().find_map(|key| {
        crate::gateway::key_pool::try_acquire(&key.key, limits, std::time::Instant::now())
            .map(|slot| (key, slot))
    }) else {
        return Err(format!(
            "{} 的 Key 都在冷却、熔断中或达到了 Codex++ 本地限速上限",
            relay.name
        ));
    };
    let relay = crate::gateway::accounts::prepare_relay(&key.relay)
        .await
        .map_err(|error| format!("{error:#}"))?;
    let user_agent = user_agent
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(relay.user_agent.as_str());
    let apply_dashscope_async = should_send_dashscope_async(adapter, &method);
    let client = crate::http_client::proxied_client_for_relay(user_agent, &relay)
        .map_err(|error| format!("{error:#}"))?;
    let mut request = client
        .request(method, endpoint)
        .timeout(Duration::from_secs(120));
    if apply_dashscope_async {
        request = request.header("X-DashScope-Async", "enable");
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let request = crate::relay_headers::apply_for_protocol(request, &relay);
    let response = request.send().await.map_err(|error| format!("{error:#}"))?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json; charset=utf-8")
        .to_string();
    let content_length = response.content_length();
    if content_length.is_some_and(|length| length > VIDEO_CONTENT_LIMIT) {
        return Err(format!(
            "视频文件超过 {} MB 的大小上限",
            VIDEO_CONTENT_LIMIT / 1024 / 1024
        ));
    }
    let body = read_response_body(response, VIDEO_CONTENT_LIMIT).await?;
    Ok(RawResponse {
        status,
        content_type,
        body,
    })
}

fn should_send_dashscope_async(adapter: VideoAdapter, method: &reqwest::Method) -> bool {
    adapter == VideoAdapter::DashScope && method == reqwest::Method::POST
}

async fn download_content_url(
    url: &str,
    user_agent: Option<&str>,
    relay: &RelayProfile,
) -> Result<RawResponse, VideoError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| VideoError::bad_gateway("视频上游返回了无效的文件链接"))?;
    if !matches!(parsed.scheme(), "http" | "https") || is_private_host(parsed.host_str()) {
        return Err(VideoError::bad_gateway(
            "视频上游返回的文件链接不是可代理的公网 HTTP(S) 地址",
        ));
    }
    let user_agent = user_agent
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("CodexPlusPlus/VideoGateway");
    let client = crate::http_client::proxied_client_for_relay(user_agent, relay)
        .map_err(|error| VideoError::bad_gateway(format!("{error:#}")))?;
    let response = client
        .get(parsed)
        .timeout(VIDEO_CONTENT_TIMEOUT)
        .send()
        .await
        .map_err(|error| VideoError::bad_gateway(format!("下载视频失败：{error:#}")))?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("video/mp4")
        .to_string();
    if !(200..300).contains(&status) {
        return Err(VideoError::upstream(
            status,
            format!("视频文件链接返回 {status}"),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > VIDEO_CONTENT_LIMIT)
    {
        return Err(VideoError::bad_gateway(format!(
            "视频文件超过 {} MB 的大小上限",
            VIDEO_CONTENT_LIMIT / 1024 / 1024
        )));
    }
    let body = read_response_body(response, VIDEO_CONTENT_LIMIT)
        .await
        .map_err(VideoError::bad_gateway)?;
    Ok(RawResponse {
        status,
        content_type,
        body,
    })
}

async fn read_response_body(
    mut response: reqwest::Response,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("{error:#}"))?
    {
        let next_len = (body.len() as u64).saturating_add(chunk.len() as u64);
        if next_len > limit {
            return Err(format!(
                "视频文件超过 {} MB 的大小上限",
                limit / 1024 / 1024
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn is_private_host(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return true;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let Ok(ip) = host.parse::<std::net::IpAddr>() else {
        return false;
    };
    match ip {
        std::net::IpAddr::V4(ip) => {
            ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified()
        }
        std::net::IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

fn detect_adapter(relay: &RelayProfile, model: &str) -> VideoAdapter {
    let model = model.trim().to_ascii_lowercase();
    let base_url = relay_base_url(relay).to_ascii_lowercase();
    if base_url.contains("dashscope")
        || base_url.contains("aliyuncs.com")
        || model.starts_with("wan")
        || model.contains("wanx-video")
        || model.contains("qwen-video")
    {
        return VideoAdapter::DashScope;
    }
    if base_url.contains("api.x.ai") || model.contains("grok") {
        return VideoAdapter::Xai;
    }
    VideoAdapter::OpenAi
}

fn relay_base_url(relay: &RelayProfile) -> String {
    if relay.upstream_base_url.trim().is_empty() {
        crate::relay_config::relay_profile_base_url(relay)
    } else {
        relay.upstream_base_url.trim().to_string()
    }
}

fn create_endpoint(relay: &RelayProfile, adapter: VideoAdapter, model: &str) -> String {
    let base = relay_base_url(relay);
    match adapter {
        VideoAdapter::OpenAi => api_url(&base, "videos"),
        VideoAdapter::Xai => api_url(&base, "videos/generations"),
        VideoAdapter::DashScope => {
            dashscope_endpoint_from_base(&base, "services/aigc/video-generation/video-synthesis")
                .unwrap_or_else(|| api_url(&base, "services/aigc/video-generation/video-synthesis"))
        }
    }
    .replace("{model}", model)
}

fn status_endpoint(relay: &RelayProfile, adapter: VideoAdapter, id: &str) -> String {
    let base = relay_base_url(relay);
    match adapter {
        VideoAdapter::OpenAi | VideoAdapter::Xai => api_url(&base, &format!("videos/{id}")),
        VideoAdapter::DashScope => dashscope_endpoint_from_base(&base, &format!("tasks/{id}"))
            .unwrap_or_else(|| api_url(&base, &format!("tasks/{id}"))),
    }
}

fn content_endpoint(relay: &RelayProfile, id: &str) -> String {
    api_url(&relay_base_url(relay), &format!("videos/{id}/content"))
}

fn api_url(base_url: &str, path: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base
        .to_ascii_lowercase()
        .ends_with(&format!("/{}", path.to_ascii_lowercase()))
    {
        return base.to_string();
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    let has_version = has_version_suffix(base);
    let mut url = if skip_version_prefix || has_version || !origin_only {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    url
}

fn has_version_suffix(base: &str) -> bool {
    let path = base.rsplit_once("://").map_or(base, |(_, rest)| rest);
    let last = path
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    last == "v1"
        || last == "v1beta"
        || last == "v2"
        || last == "v3"
        || last == "v4"
        || last == "openai"
}

fn dashscope_endpoint_from_base(base_url: &str, path: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(base_url.trim().trim_end_matches('#')).ok()?;
    let path = path.trim_matches('/');
    let host = url.host_str()?.to_ascii_lowercase();
    if host.ends_with("aliyuncs.com") {
        url.set_path(&format!("/api/v1/{path}"));
    } else {
        let current = url.path().trim_end_matches('/');
        if current.ends_with("/api/v1") {
            url.set_path(&format!("{current}/{path}"));
        } else if current.ends_with("/compatible-mode/v1") {
            let origin = &current[..current.len() - "/compatible-mode/v1".len()];
            url.set_path(&format!("{origin}/api/v1/{path}"));
        } else {
            return None;
        }
    }
    url.set_query(None);
    Some(url.to_string())
}

fn create_payload(adapter: VideoAdapter, model: &str, request: &Value) -> Value {
    match adapter {
        VideoAdapter::OpenAi | VideoAdapter::Xai => {
            let mut value = request.clone();
            value["model"] = Value::String(model.to_string());
            value
        }
        VideoAdapter::DashScope => {
            let source = request.as_object().cloned().unwrap_or_default();
            let mut parameters = Map::new();
            copy_between(&source, &mut parameters, "duration", "duration");
            copy_between(&source, &mut parameters, "seconds", "duration");
            copy_between(&source, &mut parameters, "resolution", "resolution");
            copy_between(
                &source,
                &mut parameters,
                "negative_prompt",
                "negative_prompt",
            );
            copy_between(&source, &mut parameters, "seed", "seed");
            copy_between(&source, &mut parameters, "prompt_extend", "prompt_extend");
            copy_between(&source, &mut parameters, "watermark", "watermark");
            if model.to_ascii_lowercase().starts_with("wan2.7") {
                if source.get("ratio").is_some() {
                    copy_between(&source, &mut parameters, "ratio", "ratio");
                } else {
                    copy_between(&source, &mut parameters, "aspect_ratio", "ratio");
                }
            } else {
                copy_between(&source, &mut parameters, "size", "size");
                copy_between(&source, &mut parameters, "aspect_ratio", "aspect_ratio");
            }
            let prompt = source
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut input = Map::new();
            input.insert("prompt".to_string(), Value::String(prompt));
            copy_between(&source, &mut input, "image", "img_url");
            copy_between(&source, &mut input, "image_url", "img_url");
            json!({
                "model": model,
                "input": input,
                "parameters": parameters
            })
        }
    }
}

fn copy_between(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
    from: &str,
    to: &str,
) {
    if let Some(value) = source.get(from) {
        target.insert(to.to_string(), value.clone());
    }
}

fn should_try_next(status: u16) -> bool {
    matches!(status, 401 | 403 | 404 | 408 | 409 | 425 | 429) || status >= 500
}

fn create_job_id(adapter: VideoAdapter, value: &Value) -> Option<String> {
    let candidates: &[&[&str]] = match adapter {
        VideoAdapter::DashScope => &[&["output", "task_id"], &["task_id"], &["id"]],
        VideoAdapter::OpenAi | VideoAdapter::Xai => {
            &[&["id"], &["request_id"], &["output", "task_id"]]
        }
    };
    candidates.iter().find_map(|path| {
        value_at_path(value, path)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    })
}

fn raw_status(value: &Value) -> Option<&str> {
    value
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| value.get("state").and_then(Value::as_str))
        .or_else(|| value.get("output")?.get("task_status")?.as_str())
}

fn normalized_status(value: &Value) -> &'static str {
    match raw_status(value)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "completed" | "complete" | "succeeded" | "success" | "done" | "finished" => "completed",
        "failed" | "failure" | "error" | "cancelled" | "canceled" => "failed",
        "processing" | "running" | "in_progress" | "in-progress" | "generating" => "in_progress",
        _ => "queued",
    }
}

fn normalize_status(adapter: VideoAdapter, value: &Value, model: &str) -> Value {
    let status = normalized_status(value);
    let progress = value
        .get("progress")
        .or_else(|| {
            value
                .get("output")
                .and_then(|output| output.get("progress"))
        })
        .and_then(|progress| {
            progress
                .as_u64()
                .or_else(|| progress.as_str().and_then(|value| value.parse().ok()))
        })
        .unwrap_or_else(|| match status {
            "completed" => 100,
            "in_progress" => 50,
            _ => 0,
        })
        .min(100);
    let mut response = json!({
        "id": Value::Null,
        "object": "video",
        "model": model,
        "status": status,
        "progress": progress,
        "upstreamAdapter": match adapter {
            VideoAdapter::OpenAi => "openai",
            VideoAdapter::Xai => "xai",
            VideoAdapter::DashScope => "dashscope",
        }
    });
    if let Some(error) = value
        .get("error")
        .and_then(|error| error.get("message").or(Some(error)))
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("output")
                .and_then(|output| output.get("message"))
                .and_then(Value::as_str)
        })
    {
        response["error"] = json!({ "message": error });
    } else if status == "failed" {
        response["error"] =
            json!({ "message": preview(&serde_json::to_vec(value).unwrap_or_default()) });
    }
    if let Some(url) = extract_content_url(value) {
        response["url"] = Value::String(url);
    }
    for key in [
        "created_at",
        "completed_at",
        "expires_at",
        "seconds",
        "size",
    ] {
        if let Some(item) = value.get(key) {
            response[key] = item.clone();
        }
    }
    response
}

fn public_status(job: &VideoJob, mut status: Value) -> Value {
    status["id"] = Value::String(job.id.clone());
    status["created_at"] = Value::Number(job.created_at.into());
    status
}

fn extract_content_url(value: &Value) -> Option<String> {
    [
        &["url"][..],
        &["content_url"],
        &["video_url"],
        &["video", "url"],
        &["output", "video_url"],
        &["output", "video", "url"],
        &["data", "0", "url"],
        &["result", "video_url"],
        &["results", "0", "video_url"],
    ]
    .iter()
    .find_map(|path| {
        value_at_path(value, path)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
            .map(str::to_string)
    })
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| {
        current.get(*key).or_else(|| {
            key.parse::<usize>()
                .ok()
                .and_then(|index| current.as_array()?.get(index))
        })
    })
}

fn preview(body: &[u8]) -> String {
    String::from_utf8_lossy(body)
        .trim()
        .chars()
        .take(ERROR_PREVIEW_LIMIT)
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, models: &str) -> GatewayProvider {
        GatewayProvider {
            enabled: true,
            source: String::new(),
            price: None,
            model_prices: Default::default(),
            proxy: String::new(),
            key_rpm: None,
            key_concurrency: None,
            upstream_new: Vec::new(),
            upstream_gone: Vec::new(),
            models_checked_at_ms: 0,
            newapi_access: None,
            subscription_policy: None,
            upstream_meta: Default::default(),
            auto_compact: Default::default(),
            model_windows: Default::default(),
            context_window: None,
            relay: RelayProfile {
                id: id.to_string(),
                name: id.to_string(),
                base_url: format!("https://{id}.test/v1"),
                api_key: format!("sk-{id}"),
                model: models.lines().next().unwrap_or_default().to_string(),
                model_list: models.to_string(),
                ..RelayProfile::default()
            },
            extra: Default::default(),
        }
    }

    #[test]
    fn video_models_are_detected_without_taking_images() {
        for model in [
            "wan2.7-video",
            "wan2.5-t2v-preview",
            "grok-video",
            "sora-2",
            "veo-3",
            "kling-2.1",
        ] {
            assert!(is_video_model(model), "{model}");
        }
        for model in [
            "gpt-image-2",
            "qwen-image-3.0-pro",
            "gpt-5.6",
            "deepseek-v4-pro",
        ] {
            assert!(!is_video_model(model), "{model}");
        }
        assert!(!crate::gateway::images::is_image_model("wan2.7-video"));
        let relay = provider("mix", "deepseek-v4-pro\nwan2.7-video\nqwen-image-3.0-pro").relay;
        assert_eq!(
            crate::gateway::route::chat_models(&relay),
            ["deepseek-v4-pro"]
        );
    }

    #[test]
    fn auto_tries_one_video_model_per_provider_in_pool_order() {
        let gateway = GatewaySettings {
            providers: vec![
                provider("ds", "deepseek-v4-pro"),
                provider("qwen", "wan2.7-video\nwan2.7-video-pro"),
                provider("x", "grok-video"),
            ],
            ..GatewaySettings::default()
        };
        let picked = targets(&gateway, &ToolId::Codex, None)
            .unwrap()
            .into_iter()
            .map(|target| (target.relay.id, target.model, target.adapter))
            .collect::<Vec<_>>();
        assert_eq!(
            picked,
            [
                (
                    "qwen".to_string(),
                    "wan2.7-video".to_string(),
                    VideoAdapter::DashScope
                ),
                ("x".to_string(), "grok-video".to_string(), VideoAdapter::Xai),
            ]
        );
        assert_eq!(auto_choice(&gateway).unwrap().id, "qwen/wan2.7-video");
    }

    #[test]
    fn explicit_and_off_selections_explain_themselves() {
        let mut gateway = GatewaySettings {
            providers: vec![provider("oa", "sora-2\nveo-3")],
            ..GatewaySettings::default()
        };
        gateway
            .video_models
            .insert("codex".to_string(), "oa/veo-3".to_string());
        let picked = targets(&gateway, &ToolId::Codex, Some("sora-2")).unwrap();
        assert_eq!(picked[0].model, "veo-3");
        gateway
            .video_models
            .insert("codex".to_string(), VIDEO_OFF.to_string());
        assert!(
            targets(&gateway, &ToolId::Codex, None)
                .unwrap_err()
                .contains("已关闭")
        );
    }

    #[test]
    fn adapters_map_the_public_contract_to_upstream_protocols() {
        assert_eq!(
            create_endpoint(
                &provider("oa", "sora-2").relay,
                VideoAdapter::OpenAi,
                "sora-2"
            ),
            "https://oa.test/v1/videos"
        );
        assert_eq!(
            create_endpoint(
                &provider("x", "grok-video").relay,
                VideoAdapter::Xai,
                "grok-video"
            ),
            "https://x.test/v1/videos/generations"
        );
        let dashscope = RelayProfile {
            base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1".to_string(),
            ..provider("qwen", "wan2.7-video").relay
        };
        assert_eq!(
            create_endpoint(&dashscope, VideoAdapter::DashScope, "wan2.7-video"),
            "https://dashscope-intl.aliyuncs.com/api/v1/services/aigc/video-generation/video-synthesis"
        );
        assert_eq!(
            status_endpoint(&dashscope, VideoAdapter::DashScope, "task-1"),
            "https://dashscope-intl.aliyuncs.com/api/v1/tasks/task-1"
        );
    }

    #[test]
    fn china_token_plan_uses_dashscope_endpoints_and_only_enables_async_on_create() {
        let relay = RelayProfile {
            base_url: "https://token-plan.maas.qianwenaiapi.com/compatible-mode/v1".to_string(),
            ..provider("qwen", "wan3.0-video").relay
        };
        assert_eq!(
            create_endpoint(&relay, VideoAdapter::DashScope, "wan3.0-video"),
            "https://token-plan.maas.qianwenaiapi.com/api/v1/services/aigc/video-generation/video-synthesis"
        );
        assert_eq!(
            status_endpoint(&relay, VideoAdapter::DashScope, "task-1"),
            "https://token-plan.maas.qianwenaiapi.com/api/v1/tasks/task-1"
        );
        assert!(should_send_dashscope_async(
            VideoAdapter::DashScope,
            &reqwest::Method::POST
        ));
        assert!(!should_send_dashscope_async(
            VideoAdapter::DashScope,
            &reqwest::Method::GET
        ));
        assert!(!should_send_dashscope_async(
            VideoAdapter::OpenAi,
            &reqwest::Method::POST
        ));
    }

    #[test]
    fn dashscope_payload_uses_native_input_and_parameters() {
        let payload = create_payload(
            VideoAdapter::DashScope,
            "wan2.6-t2v",
            &json!({
                "model": "ignored",
                "prompt": "waves",
                "duration": 5,
                "size": "1280*720"
            }),
        );
        assert_eq!(payload["model"], "wan2.6-t2v");
        assert_eq!(payload["input"]["prompt"], "waves");
        assert_eq!(payload["parameters"]["duration"], 5);
        assert_eq!(payload["parameters"]["size"], "1280*720");
    }

    #[test]
    fn dashscope_wan2_7_uses_ratio_instead_of_size() {
        let payload = create_payload(
            VideoAdapter::DashScope,
            "wan2.7-t2v",
            &json!({
                "model": "ignored",
                "prompt": "waves",
                "resolution": "1080p",
                "aspect_ratio": "16:9",
                "size": "1920*1080"
            }),
        );
        assert_eq!(payload["parameters"]["resolution"], "1080p");
        assert_eq!(payload["parameters"]["ratio"], "16:9");
        assert!(payload["parameters"].get("size").is_none());
    }

    #[test]
    fn dashscope_result_video_url_is_normalized() {
        let value = json!({
            "status": "SUCCEEDED",
            "results": [{"video_url": "https://cdn.test/result.mp4"}]
        });
        assert_eq!(
            extract_content_url(&value).as_deref(),
            Some("https://cdn.test/result.mp4")
        );
        assert_eq!(normalized_status(&value), "completed");
    }

    #[test]
    fn native_statuses_normalize_to_the_public_contract() {
        let xai = normalize_status(
            VideoAdapter::Xai,
            &json!({"status": "done", "video": {"url": "https://cdn.test/v.mp4"}}),
            "grok-video",
        );
        assert_eq!(xai["status"], "completed");
        assert_eq!(xai["progress"], 100);
        assert_eq!(xai["url"], "https://cdn.test/v.mp4");
        let dashscope = normalize_status(
            VideoAdapter::DashScope,
            &json!({
                "output": {
                    "task_status": "SUCCEEDED",
                    "video_url": "https://cdn.test/q.mp4"
                }
            }),
            "wan2.7-video",
        );
        assert_eq!(dashscope["status"], "completed");
        assert_eq!(dashscope["url"], "https://cdn.test/q.mp4");
    }

    #[test]
    fn accepted_jobs_keep_the_creation_target() {
        let saved = remember(VideoJob {
            id: String::new(),
            agent: "codex".to_string(),
            provider_id: "qwen".to_string(),
            upstream_id: "task-1".to_string(),
            model: "wan2.7-video".to_string(),
            adapter: VideoAdapter::DashScope,
            created_at: now_secs(),
            content_url: None,
        });
        assert!(saved.id.starts_with("video_"));
        let found = job_for(&ToolId::Codex, &saved.id).unwrap();
        assert_eq!(found.provider_id, "qwen");
        assert_eq!(found.adapter, VideoAdapter::DashScope);
    }

    #[test]
    fn model_is_read_from_json_only() {
        assert_eq!(
            requested_model(br#"{"model":"wan2.7-video","prompt":"x"}"#).as_deref(),
            Some("wan2.7-video")
        );
        assert_eq!(requested_model(b"not-json"), None);
    }
}
