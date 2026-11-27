//! Magpie（<https://github.com/yetone/magpie>，MIT）的供应商：读它的 `providers.json`，导入成直连供应商，或复制进共享供应商。
//!
//! 只读，不改 Magpie 的数据；导入之后两边各改各的，互不影响。
//! 规则对照它的源码（`internal/appdir`、`internal/provider`、`internal/gateway`、`internal/settings`），各处注明出处。
//!
//! `providers.json` 的格式（`internal/provider/provider.go` 的 `Provider`）：
//!
//! ```json
//! { "providers": [
//!   { "id": "demo", "name": "Demo", "key": "sk-…", "baseAPI": "responses", "responses": "https://example.com", "off": true }
//! ] }
//! ```
//!
//! 接口地址每种协议一个字段（`responses` / `chat` / `anthropic`），可以同时有几个；`baseAPI` 只是编辑时选的那一种。
//! `off` 是在 Magpie 里关掉了。没有地址的内置项（订阅登录的 `codex` 那一条）和 `hidden` 的删除记录不是供应商，跳过；
//! 指回 Codex++ 本地服务的条目（地址里带 `/gw/`）也跳过，导进来就绕圈了（Magpie 自己导入时同样不收指向它网关的地址）。
//!
//! 导入有两种方式，一次只能选一种（两种一起会有两条路通向同一批账号）：
//! - **复制它的供应商**：把地址和 Key 抄过来，之后不依赖 Magpie；
//! - **把 Magpie 当作一个上游**（[`GATEWAY_ID`]）：只加一条指向 Magpie 本地网关的供应商，请求经 Magpie 转发，
//!   用它里面的账号、路由和额度，Magpie 要开着。网关地址先读它运行时写的 `gateway.json`（`{"addr": "127.0.0.1:3425"}`），
//!   没有就按 `settings.json` 的 `port`，再没有是默认端口 3425（`settings.SavedAddr`）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde_json::Value;

use crate::ccs_import::CcsImportOrigin;
use crate::claude_provider_sources::SourceEntry;
use crate::settings::{BackendSettings, RelayProfile, RelayProtocol};

/// 导入来源记录（`ccs_import_sources`）里 Magpie 的 `kind`。
pub const ORIGIN_KIND: &str = "magpie";

/// 「把 Magpie 当作一个上游」那一条的 id。Magpie 自己的供应商 id 只有小写字母、数字、`-`、`_`，不会和它撞。
pub const GATEWAY_ID: &str = "@gateway";

/// Magpie 告诉各个 Agent 填的 Key（`gateway.Token`）。它的网关只听回环、什么 Key 都收，这个只是因为 Agent 非要一个。
const GATEWAY_LOOPBACK_KEY: &str = "magpie";

/// Magpie 网关的默认端口（`settings.DefaultPort`）。
const GATEWAY_DEFAULT_PORT: u64 = 3425;

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

/// Magpie 的配置目录（`appdir.installed`）：`XDG_CONFIG_HOME/magpie`（绝对路径时），否则 `~/.config/magpie`，
/// macOS、Linux、Windows 都一样。便携模式（程序旁边有 `data` 目录或 `.portable` 文件）下它的文件在那个 `data` 里，
/// 那个位置跟着程序走，这里找不到，读不出来。
pub fn magpie_config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| crate::computer_use::home_dir().join(".config"))
        .join("magpie")
}

pub fn default_providers_path() -> PathBuf {
    magpie_config_dir().join("providers.json")
}

pub fn list_providers() -> anyhow::Result<Vec<MagpieProvider>> {
    list_providers_at(&default_providers_path())
}

/// 「把 Magpie 当作一个上游」的那一条；没装 Magpie（配置目录不在）时没有。
pub fn gateway_provider() -> Option<MagpieProvider> {
    gateway_provider_at(&magpie_config_dir())
}

fn gateway_provider_at(dir: &Path) -> Option<MagpieProvider> {
    if !dir.is_dir() {
        return None;
    }
    let json = |name: &str| {
        std::fs::read(dir.join(name))
            .ok()
            .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
    };
    // 正在跑的网关写的地址优先；没在跑就按设置里的端口，都没有是默认端口。端口范围同 `settings.CheckPort`。
    let addr = json("gateway.json")
        .and_then(|book| book.get("addr").and_then(Value::as_str).map(str::trim).map(str::to_string))
        .and_then(|addr| loopback_addr(&addr))
        .unwrap_or_else(|| {
            let port = json("settings.json")
                .and_then(|settings| settings.get("port").and_then(Value::as_u64))
                .filter(|port| (1024..=65535).contains(port))
                .unwrap_or(GATEWAY_DEFAULT_PORT);
            format!("127.0.0.1:{port}")
        });
    Some(MagpieProvider {
        id: GATEWAY_ID.to_string(),
        name: "Magpie".to_string(),
        base_url: format!("http://{addr}/v1"),
        api_key: GATEWAY_LOOPBACK_KEY.to_string(),
        protocol: RelayProtocol::Responses,
        disabled: false,
    })
}

/// `gateway.json` 里的监听地址换成本机能连的 `主机:端口`：监听在所有网卡（`0.0.0.0`、`[::]`、只有端口）时连回环。
/// 不是 `主机:端口` 的（带协议头、带路径、端口不是数字）不认。
fn loopback_addr(addr: &str) -> Option<String> {
    let (host, port) = addr.rsplit_once(':')?;
    if port.is_empty() || !port.chars().all(|ch| ch.is_ascii_digit()) || host.contains('/') {
        return None;
    }
    let host = match host {
        "" | "0.0.0.0" | "[::]" | "::" => "127.0.0.1",
        other => other,
    };
    Some(format!("{host}:{port}"))
}

/// 这个文件旁边能导入的全部条目：复制用的各家供应商，加上「把 Magpie 当作一个上游」那一条（排在最前）。
fn candidates_at(path: &Path) -> anyhow::Result<Vec<MagpieProvider>> {
    let mut all: Vec<MagpieProvider> = path.parent().and_then(gateway_provider_at).into_iter().collect();
    all.extend(list_providers_at(path)?);
    Ok(all)
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
    let (protocol, base_url) = [preferred.as_str(), "responses", "chat", "anthropic", "gemini"]
        .into_iter()
        .filter_map(|api| Some((protocol_of(api)?, text(api))))
        .find(|(_, url)| !url.is_empty())?;
    // 指回 Codex++ 本地服务的地址（`…/gw/<agent>`）：导进来请求就在两边之间绕圈。
    if base_url.contains("/gw/") {
        return None;
    }
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
        "gemini" => Some(RelayProtocol::Gemini),
        _ => None,
    }
}

/// 把 Magpie 里存的地址整理成直连供应商要的写法，规则同它自己的 `DetectBase`（`internal/provider/detect.go`）：
/// 先去掉粘上来的接口路径（`/chat/completions`、`/responses`、`/v1/messages`、`/messages`）；
/// Anthropic 的地址是根（`/v1/messages` 由请求方接），带了 `/v1` 就去掉；
/// Gemini 保留 `/v1beta`（裸域名补 `/v1beta`）；
/// OpenAI 两种协议只写到域名时补上 `/v1`（直连供应商的地址原样写进 Codex 配置，Codex 只在后面接 `/responses`），
/// 已经带路径的（`…/api/v4`）不动。
fn normalize_base_url(url: &str, protocol: RelayProtocol) -> String {
    if protocol == RelayProtocol::Gemini {
        return crate::gemini_bridge::normalize_gemini_base_url(url);
    }
    let mut base = url.trim().trim_end_matches('/');
    for suffix in ["/chat/completions", "/responses", "/v1/messages", "/messages"] {
        base = base.strip_suffix(suffix).unwrap_or(base);
    }
    let base = base.trim_end_matches('/');
    if protocol == RelayProtocol::AnthropicMessages {
        return base.strip_suffix("/v1").unwrap_or(base).to_string();
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

/// 直连供应商：复制来的名字后面带「（Magpie）」，和 cc-switch 导入的「（ccswitch）」一个做法，列表里看得出来源；
/// 「把 Magpie 当作一个上游」那一条就叫 Magpie。
fn direct_profile(provider: &MagpieProvider, existing_ids: &[String]) -> RelayProfile {
    let (id_base, name) = if provider.id == GATEWAY_ID {
        ("magpie-gateway".to_string(), provider.name.clone())
    } else {
        (format!("magpie-{}", provider.id), format!("{}（Magpie）", provider.name))
    };
    crate::ccs_import::third_party_relay_profile(
        &id_base,
        name,
        &provider.base_url,
        &provider.api_key,
        provider.protocol,
        existing_ids,
    )
}

/// 把勾选的条目导入成直连供应商：Magpie 的各家供应商（复制），或 [`GATEWAY_ID`]（把 Magpie 当作一个上游）。
/// 已经导入过（来源记录还在）的跳过，不覆盖已有配置。
/// 返回（新导入数，跳过数）。有一个 id 在 Magpie 里找不到就整批不导入。
pub fn import_into_settings(
    settings: &mut BackendSettings,
    path: &Path,
    selected: &[String],
) -> anyhow::Result<(usize, usize)> {
    ensure_one_way(selected.iter().map(String::as_str))?;
    let providers = candidates_at(path)?;
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

/// 「把 Magpie 当作一个上游」和「复制它的供应商」一次只能选一种：两种都在同一批里就拒绝。
pub fn ensure_one_way<'a>(ids: impl IntoIterator<Item = &'a str>) -> anyhow::Result<()> {
    let (mut upstream, mut copies) = (false, false);
    for id in ids {
        if id == GATEWAY_ID {
            upstream = true;
        } else {
            copies = true;
        }
    }
    if upstream && copies {
        bail!("把 Magpie 当作一个上游和复制它的供应商只能选一种，这次没有导入");
    }
    Ok(())
}

/// 共享供应商「导入」里列出来的 Magpie 条目：「把 Magpie 当作一个上游」那一条在最前，后面是可以复制的各家。
/// 没装 Magpie 或读不了时为空。没有 Key 的进不了池，不列。
pub fn entries() -> Vec<SourceEntry> {
    entries_at(&default_providers_path())
}

fn entries_at(path: &Path) -> Vec<SourceEntry> {
    candidates_at(path)
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
    let provider = candidates_at(path)?
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
    fn magpie_itself_can_be_imported_as_one_upstream() {
        let dir = tempfile::tempdir().unwrap();
        let path = book(dir.path(), BOOK);
        // 没有 gateway.json：用默认地址。
        assert_eq!(gateway_provider_at(dir.path()).unwrap().base_url, "http://127.0.0.1:3425/v1");
        std::fs::write(dir.path().join("gateway.json"), r#"{ "addr": "0.0.0.0:4567", "pid": 1, "serves": "app" }"#).unwrap();
        let gateway = gateway_provider_at(dir.path()).unwrap();
        // 监听在所有网卡时连回环；Key 是 Magpie 在回环上认的固定值。
        assert_eq!(gateway.base_url, "http://127.0.0.1:4567/v1");
        assert_eq!(gateway.api_key, "magpie");
        assert_eq!(gateway.protocol, RelayProtocol::Responses);
        // 没装 Magpie（目录不在）时没有这一条。
        assert!(gateway_provider_at(&dir.path().join("missing")).is_none());
        // 网关没在跑（没有 gateway.json）：按设置里的端口；端口不合规就回到默认。
        let idle = tempfile::tempdir().unwrap();
        std::fs::write(idle.path().join("settings.json"), r#"{ "theme": "system", "port": 4010 }"#).unwrap();
        assert_eq!(gateway_provider_at(idle.path()).unwrap().base_url, "http://127.0.0.1:4010/v1");
        std::fs::write(idle.path().join("settings.json"), r#"{ "port": 80 }"#).unwrap();
        assert_eq!(gateway_provider_at(idle.path()).unwrap().base_url, "http://127.0.0.1:3425/v1");

        let mut settings = BackendSettings::default();
        let before = settings.relay_profiles.len();
        // 两种方式一次只能选一种：一起提交整批拒绝，什么都不导入。
        let error = import_into_settings(&mut settings, &path, &[GATEWAY_ID.into(), "resp".into()]).unwrap_err();
        assert!(error.to_string().contains("只能选一种"));
        assert_eq!(settings.relay_profiles.len(), before);
        assert_eq!(import_into_settings(&mut settings, &path, &[GATEWAY_ID.into()]).unwrap(), (1, 0));
        let upstream = settings.relay_profiles.iter().find(|profile| profile.id == "magpie-gateway").unwrap();
        assert_eq!(upstream.name, "Magpie");
        assert_eq!(upstream.upstream_base_url, "http://127.0.0.1:4567/v1");
        assert_eq!(upstream.api_key, "magpie");
        assert!(imported_ids(&settings, &path).contains(GATEWAY_ID));
        // 再勾一次：已导入，跳过。
        assert_eq!(import_into_settings(&mut settings, &path, &[GATEWAY_ID.into()]).unwrap(), (0, 1));
        assert_eq!(settings.relay_profiles.len(), before + 1);
        // 之后想改成复制，可以另外再导一次。
        assert_eq!(import_into_settings(&mut settings, &path, &["resp".into()]).unwrap(), (1, 0));
        // 复制进共享供应商时也能选它。
        let relay = relay_at(&path, GATEWAY_ID).unwrap();
        assert_eq!(relay.name, "Magpie");
        assert_eq!(relay.upstream_base_url, "http://127.0.0.1:4567/v1");
    }

    #[test]
    fn base_urls_follow_magpies_own_normalisation() {
        let openai = RelayProtocol::Responses;
        let anthropic = RelayProtocol::AnthropicMessages;
        // 只写到域名的 OpenAI 协议地址补 /v1；带路径的不动。
        assert_eq!(normalize_base_url("https://a.example.com/", openai), "https://a.example.com/v1");
        assert_eq!(normalize_base_url("https://a.example.com/api/v4", openai), "https://a.example.com/api/v4");
        // 粘上来的接口路径去掉。
        assert_eq!(normalize_base_url("https://a.example.com/v1/chat/completions", RelayProtocol::ChatCompletions), "https://a.example.com/v1");
        assert_eq!(normalize_base_url("https://a.example.com/v1/responses", openai), "https://a.example.com/v1");
        // Anthropic 的地址是根：/v1、/v1/messages 都去掉。
        assert_eq!(normalize_base_url("https://c.example.com/v1/messages", anthropic), "https://c.example.com");
        assert_eq!(normalize_base_url("https://c.example.com/v1/", anthropic), "https://c.example.com");
        assert_eq!(normalize_base_url("https://c.example.com/api-relay", anthropic), "https://c.example.com/api-relay");
    }

    #[test]
    fn entries_pointing_back_at_our_own_local_service_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = book(dir.path(), r#"{ "providers": [
          { "id": "loop", "name": "Codex++", "key": "k", "anthropic": "http://127.0.0.1:55789/gw/claude", "baseAPI": "anthropic" },
          { "id": "ok", "name": "OK", "key": "k", "responses": "https://ok.example.com", "baseAPI": "responses" }
        ] }"#);
        assert_eq!(list_providers_at(&path).unwrap().iter().map(|provider| provider.id.as_str()).collect::<Vec<_>>(), ["ok"]);
    }

    #[test]
    fn gateway_addr_only_accepts_host_and_port() {
        assert_eq!(loopback_addr("127.0.0.1:3425").as_deref(), Some("127.0.0.1:3425"));
        assert_eq!(loopback_addr(":3425").as_deref(), Some("127.0.0.1:3425"));
        assert_eq!(loopback_addr("[::]:3425").as_deref(), Some("127.0.0.1:3425"));
        assert_eq!(loopback_addr("192.168.1.8:3425").as_deref(), Some("192.168.1.8:3425"));
        assert_eq!(loopback_addr("http://127.0.0.1:3425"), None);
        assert_eq!(loopback_addr("127.0.0.1"), None);
        assert_eq!(loopback_addr("127.0.0.1:abc"), None);
        assert_eq!(loopback_addr("evil.example/x:80"), None);
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
        assert_eq!(entries_at(&path).iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(), [GATEWAY_ID, "resp", "chat", "claude"]);
        let relay = relay_at(&path, "chat").unwrap();
        assert_eq!(relay.name, "Chat Relay");
        assert_eq!(relay.upstream_base_url, "https://chat.example.com/api/v4");
        assert_eq!(relay.protocol, RelayProtocol::ChatCompletions);
        assert!(relay_at(&path, "nokey").unwrap_err().to_string().contains("没有 Key"));
        assert!(relay_at(&path, "codex").is_err());
    }
}
