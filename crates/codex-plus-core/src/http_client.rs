use std::sync::OnceLock;

/// Get or create a globally cached `reqwest::Client`.
///
/// The client is lazily initialized on the first call and reused for all subsequent
/// requests. The `user_agent` parameter is only consulted on the first call; after
/// that the cached client is returned regardless.
pub fn proxied_client(user_agent: &str) -> anyhow::Result<reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    let client = CLIENT.get_or_init(|| {
        let ua = if user_agent.trim().is_empty() {
            format!("CodexPlusPlus/{}", env!("CARGO_PKG_VERSION"))
        } else {
            user_agent.trim().to_string()
        };
        reqwest::Client::builder()
            .user_agent(ua)
            .build()
            .expect("reqwest::Client::build() only fails on TLS init — should never happen")
    });

    Ok(client.clone())
}
