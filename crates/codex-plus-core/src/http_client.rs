pub fn proxied_client(user_agent: &str) -> anyhow::Result<reqwest::Client> {
    let ua = if user_agent.trim().is_empty() {
        format!("CodexPlusPlus/{}", env!("CARGO_PKG_VERSION"))
    } else {
        user_agent.trim().to_string()
    };
    // 生产 client 尊重系统代理（HTTP_PROXY/HTTPS_PROXY/NO_PROXY env）。
    // 此前为绕过 macOS 系统代理拦截 127.0.0.1 测试请求而加的 .no_proxy() 会全断
    // 系统代理，影响所有靠代理上网的生产请求（Bug 1）；测试改用 NO_PROXY env 绕 localhost。
    Ok(reqwest::Client::builder().user_agent(ua).build()?)
}
