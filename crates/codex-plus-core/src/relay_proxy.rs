use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde_json::Value;

fn path() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_PATH.with(|path| path.borrow().clone()) {
        return path;
    }
    crate::paths::default_app_state_dir().join("relay-proxies.json")
}

#[cfg(test)]
thread_local! {
    static TEST_PATH: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct ProxyPathTestScope(Option<PathBuf>);

#[cfg(test)]
impl ProxyPathTestScope {
    pub(crate) fn new(dir: &std::path::Path) -> Self {
        Self(TEST_PATH.with(|path| path.replace(Some(dir.join("relay-proxies.json")))))
    }
}

#[cfg(test)]
impl Drop for ProxyPathTestScope {
    fn drop(&mut self) {
        TEST_PATH.with(|path| path.replace(self.0.take()));
    }
}

fn load() -> anyhow::Result<BTreeMap<String, String>> {
    let path = path();
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(BTreeMap::new());
    };
    let value: Value = serde_json::from_str(&text).context("relay-proxies.json 解析失败")?;
    Ok(value
        .as_object()
        .into_iter()
        .flat_map(|object| object.iter())
        .filter_map(|(id, value)| {
            let proxy = value.as_str()?.trim();
            (!proxy.is_empty()).then(|| (id.clone(), proxy.to_string()))
        })
        .collect())
}

pub fn proxy_for_profile(profile_id: &str) -> Option<String> {
    load().ok().and_then(|items| items.get(profile_id).cloned())
}

/// 与实际 HTTP client 共用解析规则，配置生成不能只检查 sidecar 而漏掉环境覆盖。
pub fn configured_proxy_for_profile(profile_id: &str) -> Option<String> {
    let suffix = profile_id
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_uppercase() } else { '_' })
        .collect::<String>();
    proxy_for_profile(profile_id)
        .or_else(|| non_empty_proxy(std::env::var(format!("CODEX_PLUS_PROXY_{suffix}")).ok()))
        .or_else(|| non_empty_proxy(std::env::var("CODEX_PLUS_PROXY").ok()))
}

fn non_empty_proxy(value: Option<String>) -> Option<String> {
    value.map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

pub fn set_proxy_for_profile(profile_id: &str, proxy: &str) -> anyhow::Result<()> {
    let profile_id = profile_id.trim();
    anyhow::ensure!(!profile_id.is_empty(), "供应商 ID 不能为空");
    let mut items = load()?;
    let proxy = proxy.trim();
    if proxy.is_empty() {
        items.remove(profile_id);
    } else {
        reqwest::Proxy::all(proxy).context("供应商代理地址无效")?;
        items.insert(profile_id.to_string(), proxy.to_string());
    }
    let path = path();
    if items.is_empty() {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::settings::atomic_write(&path, serde_json::to_string_pretty(&items)?.as_bytes())?;
    }
    Ok(())
}
