//! Magpie（`com.yetone.magpie`）的供应商：读它的 `providers.json`，导入成直连供应商，或复制进共享供应商。
//!
//! 只读，不改 Magpie 的数据；导入之后两边各改各的，互不影响。
//!
//! `providers.json` 的格式（按 0.1.4682 实测）：
//!
//! ```json
//! { "providers": [
//!   { "id": "demo", "name": "Demo", "key": "sk-…", "baseAPI": "responses", "responses": "https://example.com", "off": true }
//! ] }
//! ```
//!
//! `baseAPI` 是 `responses` / `chat` / `anthropic`，同名字段里放接口地址；`off` 是在 Magpie 里关掉了。
//! 没有地址的内置项（订阅登录的 `codex` 那一条）和 `hidden` 的删除记录不是供应商，跳过。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde_json::Value;

use crate::ccs_import::CcsImportOrigin;
use crate::claude_provider_sources::SourceEntry;
use crate::settings::{BackendSettings, RelayProfile, RelayProtocol};

/// 导入来源记录（`ccs_import_sources`）里 Magpie 的 `kind`。
pub const ORIGIN_KIND: &str = "magpie";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagpieProvider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub protocol: RelayProtocol,
    /// 在 Magpie 里关掉了（`off`）。照样能导入。
    pub disabled: bool,
}

/// Magpie 的配置目录：`XDG_CONFIG_HOME/magpie`（绝对路径时），否则 `~/.config/magpie`（macOS 实测也在这里）。
/// Windows 下的位置没有实测过：`~/.config/magpie` 没有时再看 `%APPDATA%\magpie`。
pub fn magpie_config_dir() -> PathBuf {
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("magpie"));
    let home = crate::computer_use::home_dir().join(".config").join("magpie");
    let appdata = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("magpie"));
    [xdg, Some(home.clone()), appdata]
        .into_iter()
        .flatten()
        .find(|dir| dir.join("providers.json").is_file())
        .unwrap_or(home)
}

pub fn default_providers_path() -> PathBuf {
    magpie_config_dir().join("providers.json")
}

pub fn list_providers() -> anyhow::Result<Vec<MagpieProvider>> {
    list_providers_at(&default_providers_path())
}

/// 没装 Magpie（文件不存在）是空列表；文件在但读不了、不是 JSON 才报错。单条坏掉不影响其它条。
pub fn list_providers_at(path: &Path) -> anyhow::Result<Vec<MagpieProvider>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read(path).with_context(|| format!("读取 Magpie 的供应商失败：{}", path.display()))?;
    let book: Value = serde_json::from_slice(&raw)
        .with_context(|| format!("Magpie 的供应商文件不是有效的 JSON：{}", path.display()))?;
    let mut seen = HashSet::new();
    Ok(book
        .get("providers")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(provider_from_value).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|provider| seen.insert(provider.id.clone()))
        .collect())
}

fn provider_from_value(value: &Value) -> Option<MagpieProvider> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    let id = text("id");
    if id.is_empty() || flag("hidden") {
        return None;
    }
    // 先按 baseAPI 指的那个字段取地址；没写或那个字段是空的，就用第一个有地址的。
    let preferred = text("baseAPI");
    let (protocol, base_url) = [preferred.as_str(), "responses", "chat", "anthropic"]
        .into_iter()
        .filter_map(|api| Some((protocol_of(api)?, text(api))))
        .find(|(_, url)| !url.is_empty())?;
    let name = text("name");
    Some(MagpieProvider {
        name: if name.is_empty() { id.clone() } else { name },
        id,
        base_url: normalize_base_url(&base_url, protocol),
        api_key: text("key"),
        protocol,
        disabled: flag("off"),
    })
}

fn protocol_of(api: &str) -> Option<RelayProtocol> {
    match api {
        "responses" => Some(RelayProtocol::Responses),
        "chat" => Some(RelayProtocol::ChatCompletions),
        "anthropic" => Some(RelayProtocol::AnthropicMessages),
        _ => None,
    }
}

/// Magpie 里 OpenAI 协议的地址常常只写到域名（`https://example.com`），它自己拼 `/v1/…`。
/// 直连供应商的地址是原样写进 Codex 配置的（Codex 只在后面接 `/responses`），所以只有域名时补上 `/v1`；
/// 已经带路径的（`…/api/v4`）不动。Anthropic 协议的地址本来就不带 `/v1`，不动。
fn normalize_base_url(url: &str, protocol: RelayProtocol) -> String {
    let base = url.trim().trim_end_matches('/');
    if protocol == RelayProtocol::AnthropicMessages {
        return base.to_string();
    }
    let origin_only = base.split_once("://").map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    if origin_only { format!("{base}/v1") } else { base.to_string() }
}

fn origin(path: &Path, id: &str) -> CcsImportOrigin {
    CcsImportOrigin {
        kind: ORIGIN_KIND.to_string(),
        database: path.canonicalize().unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().to_string(),
        source_id: id.to_string(),
    }
}

/// 已经导入成直连供应商、而且那个供应商还在的 Magpie 条目 id。
pub fn imported_ids(settings: &BackendSettings, path: &Path) -> HashSet<String> {
    let database = origin(path, "").database;
    settings
        .ccs_import_sources
        .iter()
        .filter(|(profile_id, origin)| {
            origin.kind == ORIGIN_KIND
                && origin.database == database
                && settings.relay_profiles.iter().any(|profile| &profile.id == *profile_id)
        })
        .map(|(_, origin)| origin.source_id.clone())
        .collect()
}

/// 直连供应商：名字后面带「（Magpie）」，和 cc-switch 导入的「（ccswitch）」一个做法，列表里看得出来源。
fn direct_profile(provider: &MagpieProvider, existing_ids: &[String]) -> RelayProfile {
    crate::ccs_import::third_party_relay_profile(
        &format!("magpie-{}", provider.id),
        format!("{}（Magpie）", provider.name),
        &provider.base_url,
        &provider.api_key,
        provider.protocol,
        existing_ids,
    )
}

/// 把勾选的 Magpie 供应商导入成直连供应商。已经导入过（来源记录还在）的跳过，不覆盖已有配置。
/// 返回（新导入数，跳过数）。有一个 id 在 Magpie 里找不到就整批不导入。
pub fn import_into_settings(
    settings: &mut BackendSettings,
    path: &Path,
    selected: &[String],
) -> anyhow::Result<(usize, usize)> {
    let providers = list_providers_at(path)?;
    let already = imported_ids(settings, path);
    let mut next = settings.clone();
    let mut handled = HashSet::new();
    let (mut imported, mut skipped) = (0, 0);
    for id in selected {
        if !handled.insert(id.as_str()) {
            continue;
        }
        let Some(provider) = providers.iter().find(|provider| &provider.id == id) else {
            bail!("Magpie 里没有这个供应商：{id}");
        };
        if already.contains(id) {
            skipped += 1;
            continue;
        }
        let ids: Vec<String> = next.relay_profiles.iter().map(|profile| profile.id.clone()).collect();
        let profile = direct_profile(provider, &ids);
        next.ccs_import_sources.insert(profile.id.clone(), origin(path, id));
        next.relay_profiles.push(profile);
        imported += 1;
    }
    *settings = next;
    Ok((imported, skipped))
}

/// 共享供应商「导入」里列出来的 Magpie 条目；没装 Magpie 或读不了时为空。没有 Key 的进不了池，不列。
pub fn entries() -> Vec<SourceEntry> {
    entries_at(&default_providers_path())
}

fn entries_at(path: &Path) -> Vec<SourceEntry> {
    list_providers_at(path)
        .unwrap_or_default()
        .into_iter()
        .filter(|provider| !provider.api_key.is_empty())
        .map(|provider| SourceEntry { id: provider.id, name: provider.name })
        .collect()
}

/// 复制进共享供应商用的上游配置：名字不带后缀（池里靠来源标记区分），id 由加进池时重新起。
pub fn relay(id: &str) -> anyhow::Result<RelayProfile> {
    relay_at(&default_providers_path(), id)
}

fn relay_at(path: &Path, id: &str) -> anyhow::Result<RelayProfile> {
    let provider = list_providers_at(path)?
        .into_iter()
        .find(|provider| provider.id == id)
        .ok_or_else(|| anyhow::anyhow!("Magpie 里没有这个供应商：{id}"))?;
    if provider.api_key.is_empty() {
        bail!("Magpie 的供应商「{}」没有 Key，无法导入", provider.name);
    }
    let mut profile = direct_profile(&provider, &[]);
    profile.name = provider.name;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("providers.json");
        std::fs::write(&path, body).unwrap();
        path
    }

    const BOOK: &str = r#"{
      "providers": [
        { "id": "resp", "name": "Resp Relay", "icon": "generic", "key": "sk-resp", "responses": "https://resp.example.com", "baseAPI": "responses", "off": true },
        { "id": "chat", "name": "Chat Relay", "key": "sk-chat", "chat": "https://chat.example.com/api/v4/", "baseAPI": "chat" },
        { "id": "claude", "name": "", "key": "sk-claude", "anthropic": "https://claude.example.com", "baseAPI": "anthropic" },
        { "id": "nokey", "name": "No Key", "key": "", "responses": "https://nokey.example.com/v1", "baseAPI": "responses" },
        { "id": "codex", "name": "", "key": "", "off": true },
        { "id": "gone", "hidden": true, "responses": "https://gone.example.com" },
        "not an object"
      ],
      "deleted": [{ "id": "old", "name": "old" }]
    }"#;

    #[test]
    fn reads_three_protocols_and_skips_entries_that_are_not_providers() {
        let dir = tempfile::tempdir().unwrap();
        let providers = list_providers_at(&book(dir.path(), BOOK)).unwrap();
        assert_eq!(providers.iter().map(|provider| provider.id.as_str()).collect::<Vec<_>>(), ["resp", "chat", "claude", "nokey"]);
        // 只有域名的 OpenAI 协议地址补 /v1；带路径的、Anthropic 的不动。
        assert_eq!(providers[0].base_url, "https://resp.example.com/v1");
        assert_eq!(providers[0].protocol, RelayProtocol::Responses);
        assert!(providers[0].disabled);
        assert_eq!(providers[1].base_url, "https://chat.example.com/api/v4");
        assert_eq!(providers[1].protocol, RelayProtocol::ChatCompletions);
        assert!(!providers[1].disabled);
        assert_eq!(providers[2].base_url, "https://claude.example.com");
        assert_eq!(providers[2].protocol, RelayProtocol::AnthropicMessages);
        // 没起名字的用 id。
        assert_eq!(providers[2].name, "claude");
        assert_eq!(providers[3].base_url, "https://nokey.example.com/v1");
    }

    #[test]
    fn missing_file_is_empty_and_broken_json_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_providers_at(&dir.path().join("providers.json")).unwrap().is_empty());
        let error = list_providers_at(&book(dir.path(), "{ not json")).unwrap_err();
        assert!(format!("{error:#}").contains("不是有效的 JSON"));
    }

    #[test]
    fn falls_back_to_the_first_endpoint_when_base_api_is_missing_or_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = book(dir.path(), r#"{ "providers": [
          { "id": "a", "key": "k", "chat": "https://a.example.com" },
          { "id": "b", "key": "k", "baseAPI": "responses", "responses": "", "anthropic": "https://b.example.com" }
        ] }"#);
        let providers = list_providers_at(&path).unwrap();
        assert_eq!(providers[0].protocol, RelayProtocol::ChatCompletions);
        assert_eq!(providers[1].protocol, RelayProtocol::AnthropicMessages);
    }

    #[test]
    fn imports_selected_providers_once_and_keeps_existing_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let path = book(dir.path(), BOOK);
        let mut settings = BackendSettings::default();
        let before = settings.relay_profiles.len();
        let (imported, skipped) = import_into_settings(&mut settings, &path, &["resp".into(), "claude".into(), "resp".into()]).unwrap();
        assert_eq!((imported, skipped), (2, 0));
        assert_eq!(settings.relay_profiles.len(), before + 2);
        let resp = settings.relay_profiles.iter().find(|profile| profile.id == "magpie-resp").unwrap();
        assert_eq!(resp.name, "Resp Relay（Magpie）");
        assert_eq!(resp.upstream_base_url, "https://resp.example.com/v1");
        assert_eq!(resp.api_key, "sk-resp");
        assert_eq!(resp.protocol, RelayProtocol::Responses);
        assert!(resp.config_contents.contains("base_url = \"https://resp.example.com/v1\""));
        let claude = settings.relay_profiles.iter().find(|profile| profile.id == "magpie-claude").unwrap();
        assert_eq!(claude.protocol, RelayProtocol::AnthropicMessages);
        assert_eq!(imported_ids(&settings, &path), HashSet::from(["resp".to_string(), "claude".to_string()]));

        // 再导一次：已导入的跳过，新勾的照常导入；改过名字的已有供应商不被覆盖。
        settings.relay_profiles.iter_mut().find(|profile| profile.id == "magpie-resp").unwrap().name = "我改过名".into();
        let (imported, skipped) = import_into_settings(&mut settings, &path, &["resp".into(), "chat".into()]).unwrap();
        assert_eq!((imported, skipped), (1, 1));
        assert_eq!(settings.relay_profiles.len(), before + 3);
        assert!(settings.relay_profiles.iter().any(|profile| profile.name == "我改过名"));

        // 删掉导入的那个供应商后可以重新导入。
        settings.relay_profiles.retain(|profile| profile.id != "magpie-chat");
        assert!(!imported_ids(&settings, &path).contains("chat"));
    }

    #[test]
    fn unknown_id_imports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = book(dir.path(), BOOK);
        let mut settings = BackendSettings::default();
        let before = settings.relay_profiles.len();
        let error = import_into_settings(&mut settings, &path, &["resp".into(), "missing".into()]).unwrap_err();
        assert!(error.to_string().contains("missing"));
        assert_eq!(settings.relay_profiles.len(), before);
    }

    #[test]
    fn pool_entries_need_a_key_and_copies_keep_the_plain_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = book(dir.path(), BOOK);
        assert_eq!(entries_at(&path).iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(), ["resp", "chat", "claude"]);
        let relay = relay_at(&path, "chat").unwrap();
        assert_eq!(relay.name, "Chat Relay");
        assert_eq!(relay.upstream_base_url, "https://chat.example.com/api/v4");
        assert_eq!(relay.protocol, RelayProtocol::ChatCompletions);
        assert!(relay_at(&path, "nokey").unwrap_err().to_string().contains("没有 Key"));
        assert!(relay_at(&path, "codex").is_err());
    }
}
