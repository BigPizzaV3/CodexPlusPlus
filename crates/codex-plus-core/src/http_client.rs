pub fn proxied_client(user_agent: &str) -> anyhow::Result<reqwest::Client> {
    let ua = if user_agent.trim().is_empty() {
        format!("CodexPlusPlus/{}", env!("CARGO_PKG_VERSION"))
    } else {
        user_agent.trim().to_string()
    };
    // FIX: 为协议代理的流式请求配置合理的超时和连接池
    // - 连接超时 30s，防止慢连接卡死
    // - 读写超时 300s (5分钟)，流式 SSE 响应可能很长，避免中途断开
    // - 启用 TCP keepalive，防止防火墙/代理主动断开长连接
    // - 增加连接池大小，支持并发请求
    //
    // 注意：reqwest 0.12 不支持 http2_keep_alive_* 配置，这些在 hyper 0.x 中才有。
    // 对于 HTTP/1.1 流式响应，tcp_keepalive + 大超时已经足够。
    Ok(reqwest::Client::builder()
        .user_agent(ua)
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(300))
        .tcp_keepalive(std::time::Duration::from_secs(60))
        .pool_idle_timeout(std::time::Duration::from_secs(120))
        .pool_max_idle_per_host(32)
        .build()?)
}
