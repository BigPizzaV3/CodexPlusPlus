use anyhow::Context;

fn apply_configured_ca_certificate(
    mut builder: reqwest::ClientBuilder,
) -> anyhow::Result<reqwest::ClientBuilder> {
    let Some(value) = std::env::var_os("CODEX_CA_CERTIFICATE") else {
        return Ok(builder);
    };
    let value = value.to_string_lossy().trim().to_string();
    if value.is_empty() {
        return Ok(builder);
    }
    let pem = if value.contains("BEGIN CERTIFICATE") {
        value.into_bytes()
    } else {
        std::fs::read(&value).with_context(|| {
            format!("读取 CODEX_CA_CERTIFICATE 指定的证书文件失败：{value}")
        })?
    };
    let certificate = reqwest::Certificate::from_pem(&pem)
        .context("CODEX_CA_CERTIFICATE 不是有效的 PEM 证书")?;
    builder = builder.add_root_certificate(certificate);
    Ok(builder)
}

/// 建 client 时实际用哪种代理。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ProxyChoice {
    /// 供应商设成直连：系统代理、PAC 和代理环境变量一律不用（#2760）。
    Direct,
    /// 显式代理（供应商设置 / 系统代理 / 代理环境变量）。
    Explicit(String),
    /// 都没有：交给 reqwest 默认的系统代理处理。
    Auto,
}

fn resolve_proxy_choice(proxy_override: Option<&str>, target_url: Option<&str>) -> ProxyChoice {
    if proxy_override.is_some_and(crate::relay_proxy::is_direct_proxy) {
        return ProxyChoice::Direct;
    }
    proxy_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        // 已知目标地址时按 URL 求值，Windows PAC / 自动检测代理才能生效（#1329）
        .or_else(|| match target_url {
            Some(url) => crate::proxy::detect_system_proxy_for_url(url),
            None => crate::proxy::detect_system_proxy(),
        })
        .or_else(|| std::env::var("HTTPS_PROXY").ok())
        .or_else(|| std::env::var("https_proxy").ok())
        .or_else(|| std::env::var("ALL_PROXY").ok())
        .or_else(|| std::env::var("all_proxy").ok())
        .map_or(ProxyChoice::Auto, ProxyChoice::Explicit)
}

/// 除 UA 与代理外的 client 构建参数。
#[derive(Default)]
struct ClientOptions {
    connect_timeout: Option<std::time::Duration>,
    total_timeout: Option<std::time::Duration>,
    redirect: Option<reqwest::redirect::Policy>,
    pool_idle_timeout: Option<std::time::Duration>,
}

fn build_client_with_proxy_choice(
    user_agent: &str,
    proxy: &ProxyChoice,
    options: ClientOptions,
) -> anyhow::Result<reqwest::Client> {
    let ua = if user_agent.trim().is_empty() {
        format!("CodexPlusPlus/{}", env!("CARGO_PKG_VERSION"))
    } else {
        user_agent.trim().to_string()
    };
    let mut builder = reqwest::Client::builder().user_agent(ua);
    if let Some(timeout) = options.connect_timeout {
        builder = builder.connect_timeout(timeout);
    }
    if let Some(timeout) = options.total_timeout {
        builder = builder.timeout(timeout);
    }
    if let Some(policy) = options.redirect {
        builder = builder.redirect(policy);
    }
    if let Some(timeout) = options.pool_idle_timeout {
        builder = builder.pool_idle_timeout(timeout);
    }
    match proxy {
        ProxyChoice::Direct => builder = builder.no_proxy(),
        ProxyChoice::Explicit(proxy) => {
            // 显式代理会关掉 reqwest 自带的 NO_PROXY 处理，这里补回来；回环地址始终直连
            let no_proxy = reqwest::NoProxy::from_env()
                .or_else(|| reqwest::NoProxy::from_string("localhost,127.0.0.1,::1"));
            builder = builder.proxy(reqwest::Proxy::all(proxy.as_str())?.no_proxy(no_proxy));
        }
        ProxyChoice::Auto => {}
    }
    Ok(apply_configured_ca_certificate(builder)?.build()?)
}

fn build_proxied_client_with_timeouts(
    user_agent: &str,
    proxy_override: Option<&str>,
    target_url: Option<&str>,
    connect_timeout: Option<std::time::Duration>,
    total_timeout: Option<std::time::Duration>,
    redirect: Option<reqwest::redirect::Policy>,
) -> anyhow::Result<reqwest::Client> {
    build_client_with_proxy_choice(
        user_agent,
        &resolve_proxy_choice(proxy_override, target_url),
        ClientOptions {
            connect_timeout,
            total_timeout,
            redirect,
            pool_idle_timeout: None,
        },
    )
}

/// 转发上游用的 client 最多缓存几个（按 UA × 代理 × 环境区分，一般只有个位数）。
const RELAY_CLIENT_CACHE_LIMIT: usize = 32;
/// 缓存 client 里空闲连接最多留多久。比 reqwest 默认的 90 秒短：一轮对话里相邻请求
/// 通常只隔几秒，30 秒足够复用；空闲更久的连接可能已被中转 / NAT 悄悄断开，不再拿来用。
const RELAY_CLIENT_POOL_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// 决定 client 行为的全部输入。任何一项变了就是另一个 client，旧的自然不再被取到。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RelayClientKey {
    user_agent: String,
    proxy: ProxyChoice,
    /// 影响 reqwest 代理行为的环境变量：显式代理时的 NO_PROXY，`Auto` 时 reqwest 自己读的 HTTP_PROXY。
    proxy_env: [Option<String>; 4],
    /// CODEX_CA_CERTIFICATE 的值；是文件路径时再带上文件大小和修改时间，证书换了能感知。
    ca_certificate: Option<(String, Option<(u64, std::time::SystemTime)>)>,
    /// 当前所在的 tokio 运行时。连接池里的连接由建立它的运行时驱动，那个运行时结束后
    /// （常驻 helper 换端口重启、临时运行时用完）再复用就会报 `dispatch task is gone`；
    /// 按运行时分开缓存，每个运行时只复用自己建的连接。不在运行时里调用时为 None。
    runtime: Option<tokio::runtime::Id>,
}

impl RelayClientKey {
    fn current(user_agent: &str, proxy: ProxyChoice) -> Self {
        let proxy_env = ["NO_PROXY", "no_proxy", "HTTP_PROXY", "http_proxy"]
            .map(|name| std::env::var(name).ok());
        let ca_certificate = std::env::var_os("CODEX_CA_CERTIFICATE")
            .map(|value| value.to_string_lossy().trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| {
                let file = (!value.contains("BEGIN CERTIFICATE"))
                    .then(|| std::fs::metadata(&value).ok())
                    .flatten()
                    .and_then(|meta| Some((meta.len(), meta.modified().ok()?)));
                (value, file)
            });
        Self {
            user_agent: user_agent.to_string(),
            proxy,
            proxy_env,
            ca_certificate,
            runtime: tokio::runtime::Handle::try_current()
                .ok()
                .map(|handle| handle.id()),
        }
    }
}

#[derive(Default)]
struct RelayClientCache {
    entries: std::collections::HashMap<RelayClientKey, (reqwest::Client, u64)>,
    tick: u64,
}

fn relay_client_cache() -> &'static std::sync::Mutex<RelayClientCache> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<RelayClientCache>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// 取（或建）可复用的 client。reqwest::Client 自带连接池：每个请求都新建 client，
/// 连接池随之丢弃，每次都要重新 DNS + TCP + TLS 握手（经代理还要多一次 CONNECT），
/// 还要重新加载系统根证书；复用后连续请求同一上游只握手一次（#2761）。
fn cached_relay_client(user_agent: &str, proxy: ProxyChoice) -> anyhow::Result<reqwest::Client> {
    let key = RelayClientKey::current(user_agent, proxy);
    let lock = || {
        relay_client_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    };
    {
        let mut cache = lock();
        cache.tick += 1;
        let tick = cache.tick;
        if let Some((client, used)) = cache.entries.get_mut(&key) {
            *used = tick;
            return Ok(client.clone());
        }
    }
    // 建 client 要加载系统根证书，比较慢：不持锁，别让并发请求排队等。
    // 构建失败（代理地址无效、证书读不了）不缓存，下次照旧报错。
    let client = build_client_with_proxy_choice(
        user_agent,
        &key.proxy,
        ClientOptions {
            pool_idle_timeout: Some(RELAY_CLIENT_POOL_IDLE_TIMEOUT),
            ..ClientOptions::default()
        },
    )?;
    let mut cache = lock();
    cache.tick += 1;
    let tick = cache.tick;
    // 并发时别的请求可能先建好了：用先放进去的那个，连接池只留一份。
    if let Some((existing, used)) = cache.entries.get_mut(&key) {
        *used = tick;
        return Ok(existing.clone());
    }
    if cache.entries.len() >= RELAY_CLIENT_CACHE_LIMIT
        && let Some(oldest) = cache
            .entries
            .iter()
            .min_by_key(|(_, (_, used))| *used)
            .map(|(key, _)| key.clone())
    {
        cache.entries.remove(&oldest);
    }
    cache.entries.insert(key, (client.clone(), tick));
    Ok(client)
}

fn build_proxied_client(
    user_agent: &str,
    proxy_override: Option<&str>,
) -> anyhow::Result<reqwest::Client> {
    build_proxied_client_with_timeouts(user_agent, proxy_override, None, None, None, None)
}

/// 走系统代理 / 环境变量代理的 client，并显式指定连接与总超时。
/// 更新下载、插件市场索引这类外部请求都应走这里（#1329）。
pub fn proxied_client_with_timeouts(
    user_agent: &str,
    connect_timeout: std::time::Duration,
    total_timeout: std::time::Duration,
) -> anyhow::Result<reqwest::Client> {
    build_proxied_client_with_timeouts(
        user_agent,
        None,
        None,
        Some(connect_timeout),
        Some(total_timeout),
        None,
    )
}

/// 同 `proxied_client_with_timeouts`，但按目标地址解析系统代理，
/// Windows 上配置了 PAC 脚本或「自动检测设置」时也能拿到实际代理。
/// 脚本市场、主题市场这类固定访问 GitHub raw 的请求走这里（#1329 / #1339）。
pub fn proxied_client_for_url_with_timeouts(
    user_agent: &str,
    target_url: &str,
    connect_timeout: std::time::Duration,
    total_timeout: std::time::Duration,
) -> anyhow::Result<reqwest::Client> {
    build_proxied_client_with_timeouts(
        user_agent,
        None,
        Some(target_url),
        Some(connect_timeout),
        Some(total_timeout),
        None,
    )
}

/// 同 `proxied_client_for_url_with_timeouts`，并指定重定向策略。
/// 用于需要自己限定跳转次数与协议的下载（例如 Agent 桌宠取图只允许 https）。
pub fn proxied_client_for_url_with_redirect(
    user_agent: &str,
    target_url: &str,
    connect_timeout: std::time::Duration,
    total_timeout: std::time::Duration,
    redirect: reqwest::redirect::Policy,
) -> anyhow::Result<reqwest::Client> {
    build_proxied_client_with_timeouts(
        user_agent,
        None,
        Some(target_url),
        Some(connect_timeout),
        Some(total_timeout),
        Some(redirect),
    )
}

pub fn proxied_client(user_agent: &str) -> anyhow::Result<reqwest::Client> {
    build_proxied_client(user_agent, None)
}

/// Provider-specific proxy override for opt-in routing. Set
/// `CODEX_PLUS_PROXY_<PROFILE_ID>` (non-alphanumeric characters become `_`),
/// or use `CODEX_PLUS_PROXY` as a process-wide fallback.
pub fn proxied_client_for_profile(
    user_agent: &str,
    profile_id: &str,
) -> anyhow::Result<reqwest::Client> {
    let proxy = crate::relay_proxy::configured_proxy_for_profile(profile_id);
    build_proxied_client(user_agent, proxy.as_deref())
}

/// 按供应商取代理：网关供应商用它自己的设置（`gateway_proxy`），直连模式的按 id 查代理表。
///
/// 网关和协议代理每个上游请求都走这里，返回的是按构建参数缓存的 client，
/// 连续请求同一上游复用已建立的连接（#2761）；代理、UA、证书等输入变了自动换新的。
pub fn proxied_client_for_relay(
    user_agent: &str,
    relay: &crate::settings::RelayProfile,
) -> anyhow::Result<reqwest::Client> {
    let proxy = crate::relay_proxy::proxy_for_relay(relay);
    cached_relay_client(user_agent, resolve_proxy_choice(proxy.as_deref(), None))
}

/// 代理取法与 [`proxied_client_for_relay`] 完全一致，但不进缓存，并使用调用方给的重定向策略。
/// 中转站倍率 / 余额识别会同时带 Bearer 和 `x-api-key`，reqwest 跨主机重定向只去掉
/// Authorization / Cookie，不去 `x-api-key`，所以识别要用不跟随跨主机重定向的 client。
pub fn relay_client_with_redirect(
    user_agent: &str,
    relay: &crate::settings::RelayProfile,
    redirect: reqwest::redirect::Policy,
) -> anyhow::Result<reqwest::Client> {
    let proxy = crate::relay_proxy::proxy_for_relay(relay);
    build_client_with_proxy_choice(
        user_agent,
        &resolve_proxy_choice(proxy.as_deref(), None),
        ClientOptions {
            redirect: Some(redirect),
            ..ClientOptions::default()
        },
    )
}

/// 用调用方配好的 builder（超时、UA 等不变）建 client，并按给定的供应商代理设置走代理：
/// 填了代理就走它，`direct` 时不走任何代理，没填（None）时保持 reqwest 默认的系统代理 / 环境变量。
/// 账号类供应商的 token 刷新、额度 / 账号查询、模型列表这些属于某个网关供应商的请求都要走这里，
/// 否则供应商单独填的代理管不到它们，没开系统代理时直连 OpenAI 失败（#2775）。
pub fn build_with_relay_proxy(
    builder: reqwest::ClientBuilder,
    proxy: Option<&str>,
) -> anyhow::Result<reqwest::Client> {
    let proxy = proxy.map(str::trim).filter(|value| !value.is_empty());
    let builder = match proxy {
        Some(proxy) if crate::relay_proxy::is_direct_proxy(proxy) => builder.no_proxy(),
        Some(proxy) => {
            // 与 build_proxied_client_with_timeouts 一致：补回 NO_PROXY，回环地址始终直连。
            let no_proxy = reqwest::NoProxy::from_env()
                .or_else(|| reqwest::NoProxy::from_string("localhost,127.0.0.1,::1"));
            builder.proxy(
                reqwest::Proxy::all(proxy)
                    .context("供应商代理地址无效")?
                    .no_proxy(no_proxy),
            )
        }
        None => builder,
    };
    Ok(apply_configured_ca_certificate(builder)?.build()?)
}

/// 同 [`build_with_relay_proxy`]，代理取这个供应商自己的设置（[`crate::relay_proxy::proxy_for_relay`]）。
pub fn build_for_relay(
    builder: reqwest::ClientBuilder,
    relay: &crate::settings::RelayProfile,
) -> anyhow::Result<reqwest::Client> {
    build_with_relay_proxy(builder, crate::relay_proxy::proxy_for_relay(relay).as_deref())
}

/// VLM 专用 HTTP client（带超时）。
/// 不复用通用 proxied_client，避免 VLM 服务无响应时永久阻塞整个代理。
pub fn vlm_http_client() -> anyhow::Result<reqwest::Client> {
    vlm_http_client_with_timeout(
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(30),
    )
}

pub(crate) fn vlm_http_client_with_timeout(
    connect: std::time::Duration,
    total: std::time::Duration,
) -> anyhow::Result<reqwest::Client> {
    Ok(apply_configured_ca_certificate(reqwest::Client::builder()
        .user_agent(format!("CodexPlusPlus-VLM/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(connect)
        .timeout(total))?
        .build()?)
}
