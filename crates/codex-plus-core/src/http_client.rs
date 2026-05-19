pub fn proxied_client(user_agent: &str) -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder().user_agent(user_agent).build()?)
}
