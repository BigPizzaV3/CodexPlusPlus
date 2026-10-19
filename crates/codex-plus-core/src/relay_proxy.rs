use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde_json::Value;

fn path() -> PathBuf {
    crate::paths::default_app_state_dir().join("relay-proxies.json")
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
