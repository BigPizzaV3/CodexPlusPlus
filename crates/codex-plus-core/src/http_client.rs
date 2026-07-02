use std::sync::OnceLock;

/// Get or create a globally cached `reqwest::Client`.
///
/// The client is lazily initialized on the first call and reused for all subsequent
/// requests. Connection pooling (TCP/TLS) is shared across the process.
///
/// NOTE: The client carries NO default User-Agent header.
/// Callers MUST set the User-Agent on each request builder.
pub fn proxied_client() -> anyhow::Result<reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }

    let client = reqwest::Client::builder().build()?;
    Ok(CLIENT.get_or_init(|| client).clone())
}
