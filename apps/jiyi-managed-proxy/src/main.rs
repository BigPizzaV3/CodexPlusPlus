use anyhow::Result;
use codex_plus_core::local_backend::LocalBackendStore;
use codex_plus_core::managed_proxy::{ManagedProxyConfig, run_managed_proxy_with_store};

#[tokio::main]
async fn main() -> Result<()> {
    let config = ManagedProxyConfig::from_env()?;
    let store = LocalBackendStore::from_env();
    eprintln!(
        "极义托管代理启动：listen={} upstream={} backend_db={} upstream_key_configured={}",
        config.listen_addr,
        config.upstream_base_url,
        store.db_path().display(),
        !config.upstream_api_key.trim().is_empty()
    );
    run_managed_proxy_with_store(config, store).await
}
