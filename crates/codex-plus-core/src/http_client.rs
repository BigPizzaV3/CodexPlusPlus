use std::collections::HashMap;

pub fn proxied_client(user_agent: &str) -> anyhow::Result<reqwest::Client> {
    proxied_client_with(user_agent, crate::proxy::detect_local_proxy)
}

pub fn proxied_client_with(
    user_agent: &str,
    detect_proxy: impl FnOnce() -> Option<String>,
) -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder().user_agent(user_agent);
    let env = std::env::vars().collect();
    if let Some(proxy) = detected_proxy_for_env(&env, detect_proxy) {
        builder = builder.proxy(reqwest::Proxy::all(&proxy)?);
    }
    Ok(builder.build()?)
}

pub fn detected_proxy_for_env(
    env: &HashMap<String, String>,
    detect_proxy: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if crate::proxy::has_proxy_environment(env) {
        return None;
    }
    detect_proxy()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detected_proxy_is_used_when_env_is_empty() {
        let env = HashMap::new();

        assert_eq!(
            detected_proxy_for_env(&env, || Some("http://127.0.0.1:7890".to_string())),
            Some("http://127.0.0.1:7890".to_string())
        );
    }

    #[test]
    fn detected_proxy_is_ignored_when_proxy_env_exists() {
        let env = HashMap::from([(
            "HTTP_PROXY".to_string(),
            "http://127.0.0.1:7897".to_string(),
        )]);

        assert_eq!(
            detected_proxy_for_env(&env, || Some("http://127.0.0.1:7890".to_string())),
            None
        );
    }
}
