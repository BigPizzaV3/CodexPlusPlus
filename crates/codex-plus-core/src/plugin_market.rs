//! 独立插件市场：浏览只取索引，安装才拉取选中的 Git 目录。
//!
//! 不调用 Codex 的 plugin/install，避免安装包时自动发起服务授权。
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use toml_edit::{DocumentMut, Item, Table};

pub const MARKETPLACE_NAME: &str = "codex-plus-plugin-market";
const MAX_INDEX_BYTES: usize = 32 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PACKAGE_FILES: u64 = 100_000;
const INDEX_TIMEOUT: Duration = Duration::from_secs(35);
const GIT_TIMEOUT: Duration = Duration::from_secs(180);

/// 插件来源。现在只有完整库（`full`）一个仓库在用；`public` 是已归档的旧公开库，
/// 只为以前从它装上的插件保留（显示、卸载），不再用它浏览和安装。
#[derive(Debug, Clone, Copy)]
struct Source {
    id: &'static str,
    repository: &'static str,
    /// 仓库里的索引文件。完整库的 `plugin-index.json` 留给旧版本（它只收 8 MiB 以内的索引）。
    index_file: &'static str,
    private: bool,
}

impl Source {
    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "public" => Ok(Self {
                id: "public",
                repository: "BigPizzaV3/CodexPlusPlusPluginCache",
                index_file: "plugin-index.json",
                private: false,
            }),
            // 完整库已改为公开仓库（#2804）：和公开库一样匿名拉取，不需要 gh 登录。
            "full" => Ok(Self {
                id: "full",
                repository: "BigPizzaV3/CodexPlusPlusFullPluginCache",
                index_file: "plugin-index-v2.json",
                private: false,
            }),
            _ => bail!("未知插件市场来源，请选择 public 或 full"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginIndex {
    schema_version: u32,
    repository: String,
    revision: String,
    #[serde(default)]
    updated_at: String,
    plugins: Vec<PluginEntry>,
    /// 官方目录的分组，按官方顺序。旧索引没有。
    #[serde(default)]
    sections: Vec<IndexSection>,
}

/// 官方目录里的一个分组：标题、成员和组内顺序都照官方目录记下来。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexSection {
    slug: String,
    /// 语言 → 标题，至少有 `en`。
    title: BTreeMap<String, String>,
    /// 人工编排的合集（热门、新品推荐…）：成员同时还在各自的分类里。
    #[serde(default)]
    curated: bool,
    /// 成员的索引 id。
    plugins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginEntry {
    id: String,
    name: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    description: String,
    version: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    license: String,
    path: String,
    tree: String,
    bytes: u64,
    #[serde(default)]
    skills: u64,
    #[serde(default)]
    requires_auth: bool,
    /// 包内带 `.app.json`：连接器由 ChatGPT 托管，有 ChatGPT 登录才连得上。旧索引没有这两项。
    #[serde(default)]
    has_apps: bool,
    /// 包内带 `.mcp.json`：连接器是 MCP 服务，可以在本机登录。
    #[serde(default)]
    has_mcp: bool,
    /// 包清单里的 `interface.category`，原生插件页按它分组。旧索引没有这一项。
    #[serde(default)]
    category: String,
    /// 列表里显示的一句话简介；没有就用完整描述。
    #[serde(default)]
    short_description: String,
    /// 官方目录里的图标地址（深色界面另有一张的才有 `icon_url_dark`）。
    #[serde(default)]
    icon_url: String,
    #[serde(default)]
    icon_url_dark: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstalledPlugin {
    source: String,
    id: String,
    name: String,
    version: String,
    tree: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct InstalledState {
    #[serde(default)]
    plugins: BTreeMap<String, InstalledPlugin>,
}

fn market_root(home: &Path) -> PathBuf {
    home.join(".tmp").join(MARKETPLACE_NAME)
}

fn install_key(source: Source, id: &str) -> String {
    format!("{}:{id}", source.id)
}

fn plugin_market_name(source: Source, id: &str) -> String {
    let hash = format!("{:x}", Sha256::digest(install_key(source, id).as_bytes()));
    format!("{MARKETPLACE_NAME}-{}", &hash[..12])
}

fn plugin_market_root(home: &Path, source: Source, id: &str) -> PathBuf {
    market_root(home)
        .join("markets")
        .join(plugin_market_name(source, id))
}

fn package_root(home: &Path, source: Source, plugin: &PluginEntry) -> PathBuf {
    package_dir(home, source, &plugin.id, &plugin.name, &plugin.version)
}

fn package_dir(home: &Path, source: Source, id: &str, name: &str, version: &str) -> PathBuf {
    home.join("plugins")
        .join("cache")
        .join(plugin_market_name(source, id))
        .join(name)
        .join(version)
}

fn installed_dir(home: &Path, source: Source, installed: &InstalledPlugin) -> PathBuf {
    package_dir(home, source, &installed.id, &installed.name, &installed.version)
}

fn state_path(home: &Path) -> PathBuf {
    market_root(home).join("installed.json")
}

fn index_path(home: &Path, source: Source) -> PathBuf {
    // 本机缓存跟着仓库里的索引文件换名：旧文件里是旧索引，不能当新的读。
    let version = if source.index_file == "plugin-index.json" { "" } else { "-v2" };
    market_root(home).join(format!("index-{}{version}.json", source.id))
}

fn valid_component(value: &str, allow_plus: bool) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value != "."
        && value != ".."
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.')
                || (allow_plus && byte == b'+')
        })
}

fn valid_hash(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

// 远端 ID 是不透明标识，真实目录含有 `plugins~Plugin_...`。
// ID 仅用于散列市场名，不作为磁盘路径段；名称、版本、包路径仍保持严格规则。
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

fn validate_package_path(source: Source, value: &str) -> anyhow::Result<()> {
    let parts = value.split('/').collect::<Vec<_>>();
    if parts.iter().any(|part| !valid_component(part, false)) {
        bail!("插件路径含有不安全字符");
    }
    let valid = match source.id {
        "public" => parts.len() == 2 && parts[0] == "plugins",
        "full" => {
            parts.len() == 4
                && parts[0] == "markets"
                && parts[1].len() == 2
                && parts[1].bytes().all(|byte| byte.is_ascii_digit())
                && parts[2] == "plugins"
        }
        _ => false,
    };
    if !valid {
        bail!("插件路径不属于该市场的插件目录");
    }
    Ok(())
}

fn parse_index(bytes: &[u8], source: Source) -> anyhow::Result<PluginIndex> {
    if bytes.len() > MAX_INDEX_BYTES {
        bail!("插件索引超过 32 MiB 限制");
    }
    let index: PluginIndex = serde_json::from_slice(bytes).context("插件市场索引格式错误")?;
    if index.schema_version != 1 || index.repository != source.repository {
        bail!("插件索引版本或仓库来源不匹配");
    }
    if !valid_hash(&index.revision) || index.plugins.len() > 20_000 {
        bail!("插件索引 revision 或条目数量无效");
    }
    let mut ids = BTreeSet::new();
    for plugin in &index.plugins {
        if !valid_id(&plugin.id)
            || !valid_component(&plugin.name, false)
            || !valid_component(&plugin.version, true)
            || !valid_hash(&plugin.tree)
            || plugin.bytes > MAX_PACKAGE_BYTES
        {
            bail!("插件索引包含无效的名称、版本、校验值或大小");
        }
        if !ids.insert(&plugin.id) {
            bail!("插件索引包含重复 id");
        }
        validate_package_path(source, &plugin.path)?;
    }
    Ok(index)
}

/// 已有目录逐层检查，避免缓存路径通过符号链接写到市场之外。
fn ensure_directory(home: &Path, target: &Path) -> anyhow::Result<()> {
    let relative = target
        .strip_prefix(home)
        .context("插件缓存路径超出 CODEX_HOME")?;
    let mut current = home.to_path_buf();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("插件缓存路径包含非法目录段");
        }
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => bail!("插件缓存目录被文件或符号链接占用"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).context("无法创建插件缓存目录")?;
            }
            Err(error) => return Err(error).context("无法检查插件缓存目录"),
        }
    }
    Ok(())
}

fn prepare_home(home: &Path) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(home).context("无法创建 CODEX_HOME")?;
    let home = home.canonicalize().context("无法定位 CODEX_HOME")?;
    ensure_directory(&home, &market_root(&home))?;
    Ok(home)
}

fn check_regular_file(path: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => bail!("插件市场文件被目录或符号链接占用"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("无法检查插件市场文件"),
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    check_regular_file(path)?;
    crate::settings::atomic_write(path, &serde_json::to_vec_pretty(value)?)
        .context("无法保存插件市场状态")
}

fn read_state(home: &Path) -> anyhow::Result<InstalledState> {
    let path = state_path(home);
    check_regular_file(&path)?;
    match fs::read(&path) {
        Ok(bytes) => {
            let state: InstalledState =
                serde_json::from_slice(&bytes).context("插件安装记录损坏")?;
            for (key, plugin) in &state.plugins {
                let source = Source::parse(&plugin.source)?;
                if key != &install_key(source, &plugin.id)
                    || !valid_id(&plugin.id)
                    || !valid_component(&plugin.name, false)
                    || !valid_component(&plugin.version, true)
                    || !valid_hash(&plugin.tree)
                {
                    bail!("插件安装记录包含无效路径");
                }
            }
            Ok(state)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(InstalledState::default()),
        Err(error) => Err(error).context("无法读取插件安装记录"),
    }
}

fn gh_binary() -> PathBuf {
    if let Some(base) = directories::BaseDirs::new() {
        for local in [
            base.home_dir().join(".local/bin/gh"),
            PathBuf::from("/opt/homebrew/bin/gh"),
            PathBuf::from("/usr/local/bin/gh"),
        ] {
            if local.is_file() {
                return local;
            }
        }
    }
    PathBuf::from("gh")
}

/// 子进程输出有大小与时间上限；原始 stderr 可能含凭据，绝不向 UI 透传。
async fn command_output(
    command: &mut Command,
    limit: usize,
    timeout: Duration,
    error_message: &str,
) -> anyhow::Result<Vec<u8>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(crate::windows_create_no_window());
    let mut child = command.spawn().with_context(|| error_message.to_owned())?;
    let stdout = child.stdout.take().context("无法读取下载命令输出")?;
    let result = tokio::time::timeout(timeout, async {
        let mut output = Vec::new();
        stdout
            .take(limit as u64 + 1)
            .read_to_end(&mut output)
            .await?;
        if output.len() > limit {
            bail!("下载命令输出超过大小限制");
        }
        let status = child.wait().await?;
        if !status.success() {
            bail!("{error_message}");
        }
        Ok::<_, anyhow::Error>(output)
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => bail!("下载超时，请稍后重试"),
    }
}

async fn fetch_index(source: Source) -> anyhow::Result<Vec<u8>> {
    let raw = format!(
        "https://raw.githubusercontent.com/{}/main/{}",
        source.repository, source.index_file
    );
    // raw.githubusercontent.com 在部分网络里连不上，但 api.github.com 可达（#2804）：
    // 再匿名走 contents 接口取一次原文。
    let api = format!(
        "https://api.github.com/repos/{}/contents/{}?ref=main",
        source.repository, source.index_file
    );
    // 插件索引走 GitHub raw，和更新检查一样必须尊重系统代理，否则在需要代理的
    // 网络里会一直超时、表现为插件市场空白或加载失败（#1329 / #2442）。
    let client = crate::http_client::proxied_client_with_timeouts(
        "CodexPlusPlus/plugin-market",
        INDEX_TIMEOUT,
        INDEX_TIMEOUT,
    )
    .context("无法建立插件市场连接")?;
    if let Some(bytes) = fetch_index_over_http(&client, &[&raw, &api]).await? {
        return Ok(bytes);
    }
    if !source.private {
        bail!("无法获取插件市场索引，请检查网络或稍后重试");
    }
    let endpoint = format!(
        "repos/{}/contents/{}?ref=main",
        source.repository, source.index_file
    );
    command_output(
        Command::new(gh_binary()).args([
            "api",
            &endpoint,
            "-H",
            "Accept: application/vnd.github.raw+json",
        ]),
        MAX_INDEX_BYTES,
        INDEX_TIMEOUT,
        "完整市场需要 GitHub 仓库访问权限；请先安装 gh 并运行 gh auth login",
    )
    .await
}

/// 依次匿名请求各地址，返回第一个取到的索引；都取不到时返回 `None`。超过大小上限直接报错。
async fn fetch_index_over_http(
    client: &reqwest::Client,
    urls: &[&str],
) -> anyhow::Result<Option<Vec<u8>>> {
    'urls: for url in urls {
        let Ok(mut response) = client
            .get(*url)
            .header(reqwest::header::ACCEPT, "application/vnd.github.raw+json")
            .send()
            .await
        else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        if response.content_length().unwrap_or_default() > MAX_INDEX_BYTES as u64 {
            bail!("插件索引超过 32 MiB 限制");
        }
        let mut bytes = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if bytes.len() + chunk.len() > MAX_INDEX_BYTES {
                        bail!("插件索引超过 32 MiB 限制");
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(None) => return Ok(Some(bytes)),
                // 读到一半断开：换下一个地址再试。
                Err(_) => continue 'urls,
            }
        }
    }
    Ok(None)
}

async fn load_index(
    home: &Path,
    source: Source,
    refresh: bool,
) -> anyhow::Result<(PluginIndex, bool)> {
    let path = index_path(home, source);
    check_regular_file(&path)?;
    if !refresh {
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(index) = parse_index(&bytes, source) {
                return Ok((index, true));
            }
        }
    }
    let bytes = fetch_index(source).await?;
    let index = parse_index(&bytes, source)?;
    crate::settings::atomic_write(&path, &bytes).context("无法缓存插件市场索引")?;
    if source.id == "full" {
        // 旧版本缓存的完整库索引（另一个文件名）不再读，顺手清掉。
        let _ = fs::remove_file(market_root(home).join("index-full.json"));
    }
    Ok((index, false))
}

/// 旧公开库的安装记录是不是这个完整库条目的同一个插件。公开库里的 id 有两种：
/// 和完整库一样的官方 id，或者直接用插件名（来自 OpenAI 的 Git 市场的那些）。
fn legacy_install_of(installed: &InstalledPlugin, plugin: &PluginEntry) -> bool {
    installed.source == "public"
        && (installed.id == plugin.id || (installed.id == installed.name && installed.name == plugin.name))
}

/// 这个索引条目在本机的安装记录，连同它实际装在哪个来源名下。公开库已并入完整库：
/// 以前从公开库装的同一个插件也算装上了，它的文件和配置仍在公开库那个市场名下。
fn installed_record<'a>(
    home: &Path,
    source: Source,
    plugin: &PluginEntry,
    state: &'a InstalledState,
) -> Option<(Source, &'a InstalledPlugin)> {
    let present = |source: Source, installed: &InstalledPlugin| {
        fs::symlink_metadata(installed_dir(home, source, installed))
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    };
    if let Some(installed) = state
        .plugins
        .get(&install_key(source, &plugin.id))
        .filter(|installed| present(source, installed))
    {
        return Some((source, installed));
    }
    if source.id != "full" {
        return None;
    }
    let public = Source::parse("public").ok()?;
    state
        .plugins
        .values()
        .find(|installed| legacy_install_of(installed, plugin) && present(public, installed))
        .map(|installed| (public, installed))
}

fn plugin_payload(
    home: &Path,
    source: Source,
    plugin: &PluginEntry,
    state: &InstalledState,
) -> Value {
    let record = installed_record(home, source, plugin, state);
    let local = record.map(|(_, installed)| installed);
    let mut result = serde_json::to_value(plugin).unwrap_or_else(|_| json!({}));
    result["installed"] = json!(local.is_some());
    result["marketplaceName"] = json!(match record {
        Some((owner, installed)) => plugin_market_name(owner, &installed.id),
        None => plugin_market_name(source, &plugin.id),
    });
    result["installedVersion"] = json!(local.map(|item| item.version.as_str()).unwrap_or_default());
    result["updateAvailable"] = json!(
        local
            .map(|item| item.version != plugin.version
                || !item.tree.eq_ignore_ascii_case(&plugin.tree))
            .unwrap_or(false)
    );
    result
}

pub async fn list_plugins(home: &Path, source: &str, refresh: bool) -> anyhow::Result<Value> {
    let source = Source::parse(source)?;
    let home = prepare_home(home)?;
    let (index, cached) = load_index(&home, source, refresh).await?;
    let state = read_state(&home)?;
    let plugins = index
        .plugins
        .iter()
        .map(|plugin| plugin_payload(&home, source, plugin, &state))
        .collect::<Vec<_>>();
    Ok(json!({
        "status": "ok",
        "message": if cached { "已读取本地插件索引。" } else { "插件索引已刷新。" },
        "source": source.id,
        "repository": source.repository,
        "updatedAt": index.updated_at,
        "total": plugins.len(),
        "plugins": plugins,
        "cached": cached,
    }))
}

struct InstallLock {
    _file: File,
}

fn acquire_install_lock(home: &Path) -> anyhow::Result<InstallLock> {
    acquire_lock(
        home,
        "install.lock",
        "另一个窗口正在安装插件，请等待完成后重试",
    )
}

fn acquire_lock(home: &Path, name: &str, message: &str) -> anyhow::Result<InstallLock> {
    let path = market_root(home).join(name);
    check_regular_file(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .context("无法创建插件安装锁")?;
    file.try_lock_exclusive()
        .with_context(|| message.to_owned())?;
    Ok(InstallLock { _file: file })
}

fn set_status(
    home: &Path,
    source: Source,
    id: &str,
    busy: bool,
    stage: &str,
    message: &str,
) -> anyhow::Result<()> {
    write_json(
        &status_path(home, source, id),
        &json!({ "status": "ok", "source": source.id, "id": id, "busy": busy, "stage": stage, "message": message }),
    )
}

fn status_path(home: &Path, source: Source, id: &str) -> PathBuf {
    market_root(home).join(format!("status-{}.json", plugin_market_name(source, id)))
}

pub fn install_status(home: &Path, source: &str, id: &str) -> anyhow::Result<Value> {
    let source = Source::parse(source)?;
    if !valid_id(id) {
        bail!("插件 id 无效");
    }
    let home = prepare_home(home)?;
    let path = status_path(&home, source, id);
    check_regular_file(&path)?;
    let mut value: Value = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("安装状态格式错误")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(
                json!({ "status": "ok", "source": source.id, "id": id, "busy": false, "stage": "failed", "message": "没有该插件的安装记录，请重新安装。" }),
            );
        }
        Err(error) => return Err(error).context("无法读取安装状态"),
    };
    if value["source"] != source.id || value["id"] != id {
        return Ok(
            json!({ "status": "ok", "source": source.id, "id": id, "busy": false, "stage": "failed", "message": "没有该插件的安装记录，请重新安装。" }),
        );
    }
    if value["busy"].as_bool().unwrap_or(false) && acquire_install_lock(&home).is_ok() {
        value["busy"] = json!(false);
        value["stage"] = json!("failed");
        value["message"] = json!("上一次安装已中断，可重新安装。");
    }
    if value["stage"] == "complete" {
        if let Ok(state) = read_state(&home) {
            if let Some(plugin) = state
                .plugins
                .values()
                .find(|plugin| plugin.source == source.id && plugin.id == id)
            {
                value["installedVersion"] = json!(plugin.version);
                value["restartRequired"] = json!(true);
                value["nativeResult"] = native_install_result(&home, source, Some(plugin));
                if let Ok(bytes) = fs::read(index_path(&home, source)) {
                    if let Ok(index) = parse_index(&bytes, source) {
                        if let Some(entry) = index.plugins.iter().find(|entry| entry.id == id) {
                            value["plugin"] = plugin_payload(&home, source, entry, &state);
                        }
                    }
                }
            }
        }
    }
    Ok(value)
}

async fn github_token() -> Option<String> {
    let output = command_output(
        Command::new(gh_binary()).args(["auth", "token", "--hostname", "github.com"]),
        16 * 1024,
        Duration::from_secs(10),
        "GitHub 登录信息不可用",
    )
    .await
    .ok()?;
    let token = String::from_utf8(output).ok()?.trim().to_owned();
    if token.is_empty() || token.contains(['\r', '\n']) {
        None
    } else {
        Some(token)
    }
}

fn git_command(work: &Path, token: Option<&str>, proxy: Option<&str>) -> Command {
    git_command_with_binary(Path::new("git"), work, token, proxy)
}

fn git_command_with_binary(
    git: &Path,
    work: &Path,
    token: Option<&str>,
    proxy: Option<&str>,
) -> Command {
    let mut command = Command::new(git);
    command
        .current_dir(work)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_CONFIG")
        .env_remove("GIT_TEMPLATE_DIR")
        .env_remove("GIT_TRACE")
        .env_remove("GIT_TRACE_CURL")
        .env_remove("GIT_CURL_VERBOSE")
        .env_remove("GIT_TRACE2")
        .env_remove("GIT_TRACE2_EVENT")
        .env_remove("GIT_TRACE2_PERF")
        .env("GIT_TRACE_REDACT", "1")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.protectNTFS=true",
            "-c",
            "init.templateDir=",
        ])
        .arg("-c")
        .arg(format!(
            "core.hooksPath={}",
            work.join("no-hooks").display()
        ));
    let mut count = 0;
    if let Some(token) = token {
        // 凭据只传给本次 Git 子进程，不写 Git 配置，也不进入命令行或日志。
        let credentials =
            base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
        command
            .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
            .env(
                "GIT_CONFIG_VALUE_0",
                format!("AUTHORIZATION: basic {credentials}"),
            );
        count += 1;
    }
    if let Some(proxy) = proxy {
        command
            .env(format!("GIT_CONFIG_KEY_{count}"), "http.proxy")
            .env(format!("GIT_CONFIG_VALUE_{count}"), proxy);
        count += 1;
    }
    command.env("GIT_CONFIG_COUNT", count.to_string());
    command
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitFailure {
    Missing,
    Network,
    Access,
    LocalPermission,
    Other,
}

impl GitFailure {
    fn message(self, private: bool) -> &'static str {
        match self {
            Self::Missing => "未找到 Git，请安装 Git 并重新启动 Codex++",
            Self::Network => {
                "Git 网络连接失败，请检查代理、DNS、证书或网络连接；此错误不需要重新登录 gh"
            }
            Self::Access if private => {
                "完整市场仓库访问未授权，请运行 gh auth login 并确认该账号具有仓库访问权限"
            }
            Self::Access => {
                "插件市场仓库访问被拒绝，请检查仓库可用性或网络访问限制；插件市场不需要 gh 登录"
            }
            Self::LocalPermission => {
                "Git 无法读写本地插件缓存或配置，请检查目录权限、磁盘空间及安全软件拦截"
            }
            Self::Other => "Git 下载失败，请检查 Git 安装及插件缓存；请勿仅因该错误重新登录 gh",
        }
    }
}

fn classify_git_failure(stderr: &[u8]) -> GitFailure {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if [
        "could not create",
        "unable to create",
        "cannot create",
        "no space left",
    ]
    .iter()
    .any(|value| text.contains(value))
    {
        return GitFailure::LocalPermission;
    }
    if [
        "authentication failed",
        "repository not found",
        "could not read username",
        "terminal prompts disabled",
        "returned error: 401",
        "returned error: 403",
        "returned error: 404",
    ]
    .iter()
    .any(|value| text.contains(value))
    {
        return GitFailure::Access;
    }
    if ["permission denied", "access is denied"]
        .iter()
        .any(|value| text.contains(value))
    {
        return GitFailure::LocalPermission;
    }
    if [
        "could not resolve",
        "failed to connect",
        "couldn't connect",
        "connection timed out",
        "connection reset",
        "connection refused",
        "proxy connect",
        "proxy authentication",
        "proxy tunneling",
        "proxy after connect",
        "connect tunnel",
        "http code 407",
        "returned error: 407",
        "ssl certificate",
        "certificate verify",
        "schannel",
        "tls",
        "recv failure",
        "rpc failed",
        "http/2",
        "network is unreachable",
    ]
    .iter()
    .any(|value| text.contains(value))
    {
        return GitFailure::Network;
    }
    GitFailure::Other
}

struct GitOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

async fn git_output(
    command: &mut Command,
    limit: usize,
    private: bool,
) -> anyhow::Result<GitOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(crate::windows_create_no_window());
    // Linux 上刚写完的可执行文件可能还被并发 fork 出的子进程短暂持有写句柄，exec 会报
    // ETXTBSY（ExecutableFileBusy）；这只是瞬时状态，稍等重试几次。
    let mut busy_retries = 0_u64;
    let mut child = loop {
        match command.spawn() {
            Ok(child) => break child,
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy && busy_retries < 5 =>
            {
                busy_retries += 1;
                tokio::time::sleep(Duration::from_millis(20 * busy_retries)).await;
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    match error.kind() {
                        std::io::ErrorKind::NotFound => GitFailure::Missing,
                        std::io::ErrorKind::PermissionDenied => GitFailure::LocalPermission,
                        _ => GitFailure::Other,
                    }
                    .message(private)
                ));
            }
        }
    };
    let mut stdout = child.stdout.take().context("无法读取 Git 输出")?;
    let mut stderr = child.stderr.take().context("无法读取 Git 错误分类")?;
    tokio::time::timeout(GIT_TIMEOUT, async {
        let read_stdout = async {
            let mut bytes = Vec::new();
            (&mut stdout)
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .await?;
            anyhow::ensure!(bytes.len() <= limit, "Git 输出超过大小限制");
            Ok::<_, anyhow::Error>(bytes)
        };
        let read_stderr = async {
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let count = stderr.read(&mut chunk).await?;
                if count == 0 {
                    break;
                }
                let retained = count.min((16 * 1024_usize).saturating_sub(bytes.len()));
                bytes.extend_from_slice(&chunk[..retained]);
            }
            Ok::<_, anyhow::Error>(bytes)
        };
        let (stdout, stderr) = tokio::try_join!(read_stdout, read_stderr)?;
        Ok::<_, anyhow::Error>(GitOutput {
            status: child.wait().await?,
            stdout,
            stderr,
        })
    })
    .await
    .map_err(|_| anyhow::anyhow!("Git 下载超时，请检查网络与代理后重试"))?
}

async fn git_config_proxy(git: &Path, work: &Path, url: &str) -> anyhow::Result<Option<String>> {
    for scope in ["--global", "--system"] {
        // config 只读取代理白名单，不执行 hooks、credential helper 或用户脚本。
        let command = git_command(work, None, None);
        let mut query = Command::new(git);
        for (key, value) in command.as_std().get_envs() {
            // 查询沿用用户的配置来源，安装命令仍隔离 global/system。
            if key == "GIT_CONFIG_NOSYSTEM" || key == "GIT_CONFIG_GLOBAL" {
                continue;
            }
            match value {
                Some(value) => {
                    query.env(key, value);
                }
                None => {
                    query.env_remove(key);
                }
            }
        }
        query.current_dir(work).args([
            "config",
            scope,
            "--includes",
            "--null",
            "--get-urlmatch",
            "http.proxy",
            url,
        ]);
        let output = tokio::time::timeout(
            Duration::from_secs(10),
            git_output(&mut query, 16 * 1024, false),
        )
        .await
        .context("读取 Git 代理设置超时")??;
        if output.status.success() {
            let value = std::str::from_utf8(&output.stdout).context("Git 代理配置编码无效")?;
            let value = value.strip_suffix('\0').unwrap_or(value).to_string();
            anyhow::ensure!(!value.contains(['\r', '\n', '\0']), "Git 代理配置格式无效");
            return Ok(Some(value)); // 显式空值表示直连，不能再套用系统代理。
        }
        let text = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        if output.status.code() != Some(1)
            && !(text.contains("unable to read config file") && text.contains("no such file"))
        {
            anyhow::bail!("无法读取 Git 代理设置，请检查 Git 配置文件权限或语法");
        }
    }
    Ok(None)
}

async fn resolve_git_proxy_with(
    git: &Path,
    work: &Path,
    url: &str,
    has_proxy_env: bool,
    system_proxy: impl FnOnce() -> Option<String>,
) -> anyhow::Result<Option<String>> {
    if has_proxy_env {
        return Ok(None);
    }
    let proxy = git_config_proxy(git, work, url).await?;
    Ok(proxy.or_else(system_proxy))
}

async fn resolve_git_proxy(work: &Path, url: &str) -> anyhow::Result<Option<String>> {
    let has_proxy_env = has_git_proxy_environment(url, |key| std::env::var_os(key));
    resolve_git_proxy_with(
        Path::new("git"),
        work,
        url,
        has_proxy_env,
        // 按仓库地址求值，Windows PAC / 自动检测代理也能生效（#2475）
        || crate::proxy::detect_system_proxy_for_url(url),
    )
    .await
}

fn has_git_proxy_environment(
    url: &str,
    value: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> bool {
    // libcurl 的 HTTPS 请求不使用 HTTP_PROXY；不能因此跳过真正有效的 Git/system 代理。
    let keys = if url.starts_with("https://") {
        ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    };
    keys.iter()
        .any(|key| value(key).is_some_and(|value| !value.is_empty()))
}

async fn run_git(
    work: &Path,
    token: Option<&str>,
    proxy: Option<&str>,
    private: bool,
    args: &[&str],
) -> anyhow::Result<Vec<u8>> {
    let output = git_output(
        git_command(work, token, proxy).args(args),
        64 * 1024,
        private,
    )
    .await?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        classify_git_failure(&output.stderr).message(private)
    );
    Ok(output.stdout)
}

async fn download_package(
    home: &Path,
    source: Source,
    index: &PluginIndex,
    plugin: &PluginEntry,
) -> anyhow::Result<PathBuf> {
    let downloads = market_root(home).join("downloads");
    ensure_directory(home, &downloads)?;
    let work = downloads.join(uuid::Uuid::new_v4().to_string());
    ensure_directory(home, &work)?;
    let token = if source.private {
        github_token().await
    } else {
        None
    };
    let token = token.as_deref();
    let url = format!("https://github.com/{}.git", source.repository);
    let proxy = resolve_git_proxy(&work, &url).await?;
    let proxy = proxy.as_deref();
    run_git(&work, token, proxy, source.private, &["init", "--quiet"]).await?;
    run_git(
        &work,
        token,
        proxy,
        source.private,
        &["remote", "add", "origin", &url],
    )
    .await?;
    // 先取固定 revision 的树元数据，blob:none 保证没有整库插件文件下载。
    run_git(
        &work,
        token,
        proxy,
        source.private,
        &[
            "fetch",
            "--quiet",
            "--depth=1",
            "--filter=blob:none",
            "origin",
            &index.revision,
        ],
    )
    .await?;
    let tree_spec = format!("{}:{}", index.revision, plugin.path);
    let tree = run_git(
        &work,
        token,
        proxy,
        source.private,
        &["rev-parse", &tree_spec],
    )
    .await?;
    if !String::from_utf8_lossy(&tree)
        .trim()
        .eq_ignore_ascii_case(&plugin.tree)
    {
        bail!("插件目录校验失败，仓库内容与索引不一致");
    }
    let files = git_output(
        git_command(&work, token, proxy).args(["ls-tree", "-r", "-z", &plugin.tree]),
        16 * 1024 * 1024,
        source.private,
    )
    .await?;
    anyhow::ensure!(
        files.status.success(),
        "{}",
        classify_git_failure(&files.stderr).message(source.private)
    );
    validate_git_files(&files.stdout)?;
    run_git(
        &work,
        token,
        proxy,
        source.private,
        &["sparse-checkout", "init", "--no-cone"],
    )
    .await?;
    // 非 cone 模式避免顺便检出根目录和祖先目录的其他文件。
    let pattern = format!("/{}/", plugin.path);
    run_git(
        &work,
        token,
        proxy,
        source.private,
        &["sparse-checkout", "set", "--no-cone", "--", &pattern],
    )
    .await?;
    run_git(
        &work,
        token,
        proxy,
        source.private,
        &["checkout", "--quiet", "--detach", &index.revision],
    )
    .await?;
    Ok(work.join(&plugin.path))
}

fn unsafe_payload_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == ".git"
        || name == ".env"
        || (name.starts_with(".env.")
            && !matches!(
                name.as_str(),
                ".env.example" | ".env.sample" | ".env.template" | ".env.dist"
            ))
}

fn validate_git_files(bytes: &[u8]) -> anyhow::Result<()> {
    let mut count = 0;
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|item| !item.is_empty())
    {
        count += 1;
        if count > MAX_PACKAGE_FILES {
            bail!("插件文件数量超过限制");
        }
        let record = std::str::from_utf8(record).context("插件 Git 文件名不是有效 UTF-8")?;
        let (metadata, path) = record
            .split_once('\t')
            .context("插件 Git 文件清单格式错误")?;
        let parts = metadata.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 3 || !matches!(parts[0], "100644" | "100755") || parts[1] != "blob" {
            bail!("插件包含符号链接、子模块或非普通 Git 文件，拒绝安装");
        }
        if path.starts_with('/')
            || path.contains('\\')
            || path.split('/').any(|part| {
                part.is_empty() || matches!(part, "." | "..") || unsafe_payload_name(part)
            })
        {
            bail!("插件包含不安全路径或凭据文件，拒绝安装");
        }
    }
    if count == 0 {
        bail!("插件 Git 目录为空");
    }
    Ok(())
}

fn inspect_payload(
    directory: &Path,
    depth: usize,
    bytes: &mut u64,
    files: &mut u64,
) -> anyhow::Result<()> {
    if depth > 64 {
        bail!("插件目录层级超过限制");
    }
    let metadata = fs::symlink_metadata(directory).context("无法检查插件目录")?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("插件包目录不是普通目录");
    }
    for entry in fs::read_dir(directory).context("无法读取插件目录")? {
        let entry = entry.context("无法读取插件文件")?;
        let name = entry.file_name();
        let name = name.to_str().context("插件文件名不是有效 UTF-8")?;
        if unsafe_payload_name(name) {
            bail!("插件包含 Git 目录或可能含凭据的 .env 文件，拒绝安装");
        }
        let metadata = fs::symlink_metadata(entry.path()).context("无法检查插件文件")?;
        if metadata.file_type().is_symlink() {
            bail!("插件包含符号链接，拒绝安装");
        }
        if metadata.is_dir() {
            inspect_payload(&entry.path(), depth + 1, bytes, files)?;
        } else if metadata.is_file() {
            *files += 1;
            *bytes = bytes.checked_add(metadata.len()).context("插件大小溢出")?;
            if *bytes > MAX_PACKAGE_BYTES || *files > MAX_PACKAGE_FILES {
                bail!("插件文件大小或数量超过限制");
            }
        } else {
            bail!("插件包含非普通文件，拒绝安装");
        }
    }
    Ok(())
}

fn plugin_manifest(directory: &Path) -> anyhow::Result<Value> {
    for relative in [
        ".codex-plugin/plugin.json",
        ".claude-plugin/plugin.json",
        "plugin.json",
    ] {
        let path = directory.join(relative);
        if path.exists() {
            check_regular_file(&path)?;
            let metadata = fs::metadata(&path).context("无法读取插件清单")?;
            if metadata.len() > 1024 * 1024 {
                bail!("插件清单超过大小限制");
            }
            return serde_json::from_slice(&fs::read(path)?).context("插件清单格式错误");
        }
    }
    bail!("插件包中没有 plugin.json 清单")
}

fn validate_payload(directory: &Path, plugin: &PluginEntry) -> anyhow::Result<()> {
    let mut bytes = 0;
    let mut files = 0;
    inspect_payload(directory, 0, &mut bytes, &mut files)?;
    let manifest = plugin_manifest(directory)?;
    if manifest["name"] != plugin.name || manifest["version"] != plugin.version {
        bail!("插件包的名称或版本与索引不一致");
    }
    Ok(())
}

fn ensure_table<'a>(document: &'a mut DocumentMut, key: &str) -> anyhow::Result<&'a mut Table> {
    if document.get(key).is_none() {
        document[key] = Item::Table(Table::new());
    }
    document[key]
        .as_table_mut()
        .context("现有插件配置不是 TOML 表，拒绝覆盖")
}

fn config_bytes(home: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    let path = home.join("config.toml");
    check_regular_file(&path)?;
    match fs::read(&path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => return Err(error).context("无法读取 Codex 配置"),
    }
}

fn updated_config(
    home: &Path,
    source: Source,
    plugin: &PluginEntry,
) -> anyhow::Result<(Option<Vec<u8>>, String)> {
    let original = config_bytes(home)?;
    let contents = std::str::from_utf8(original.as_deref().unwrap_or_default())
        .context("Codex 配置不是有效 UTF-8")?;
    // 不把解析错误携带的配置片段透传，避免错误信息暴露用户凭据。
    let mut document = contents
        .parse::<DocumentMut>()
        .map_err(|_| anyhow::anyhow!("Codex 配置格式错误，请先修复 config.toml"))?;
    let marketplaces = ensure_table(&mut document, "marketplaces")?;
    let market_name = plugin_market_name(source, &plugin.id);
    if marketplaces.get(&market_name).is_none() {
        marketplaces[&market_name] = Item::Table(Table::new());
    }
    let market = marketplaces[&market_name]
        .as_table_mut()
        .context("独立市场配置冲突，拒绝覆盖")?;
    if let Some(existing) = market.get("source") {
        if existing.as_str()
            != Some(
                plugin_market_root(home, source, &plugin.id)
                    .to_string_lossy()
                    .as_ref(),
            )
        {
            bail!("同名市场已指向其他目录，拒绝覆盖现有配置");
        }
    }
    market["source_type"] = toml_edit::value("local");
    market["source"] = toml_edit::value(
        plugin_market_root(home, source, &plugin.id)
            .to_string_lossy()
            .as_ref(),
    );
    let plugins = ensure_table(&mut document, "plugins")?;
    let key = format!("{}@{market_name}", plugin.name);
    if plugins.get(&key).is_none() {
        plugins[&key] = Item::Table(Table::new());
    }
    let plugin = plugins[&key]
        .as_table_mut()
        .context("插件启用配置冲突，拒绝覆盖")?;
    plugin["enabled"] = toml_edit::value(true);
    Ok((original, document.to_string()))
}

fn write_config_if_unchanged(
    home: &Path,
    original: Option<&[u8]>,
    updated: &str,
) -> anyhow::Result<()> {
    if config_bytes(home)?.as_deref() != original {
        bail!("Codex 配置已被其他窗口更新，请重试安装；未覆盖新的配置");
    }
    crate::settings::atomic_write(&home.join("config.toml"), updated.as_bytes())
        .context("无法启用已安装插件")
}

fn receipt_matches(home: &Path, source: Source, plugin: &PluginEntry) -> bool {
    let path = plugin_market_root(home, source, &plugin.id).join("ready.json");
    if check_regular_file(&path).is_err() {
        return false;
    }
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<InstalledPlugin>(&bytes).ok())
        .is_some_and(|item| {
            item.source == source.id
                && item.id == plugin.id
                && item.name == plugin.name
                && item.version == plugin.version
                && item.tree.eq_ignore_ascii_case(&plugin.tree)
        })
}

fn market_manifest(home: &Path, source: Source, plugin: &PluginEntry) -> Value {
    let path = package_root(home, source, plugin);
    let plugins = vec![json!({
        "name": plugin.name,
        "source": { "source": "local", "path": path.to_string_lossy() },
        "policy": { "installation": "AVAILABLE", "authentication": "ON_USE" },
        "category": native_category(plugin).unwrap_or("Productivity"),
    })];
    json!({ "name": plugin_market_name(source, &plugin.id), "interface": { "displayName": format!("Codex 插件市场 · {}", plugin.display_name) }, "plugins": plugins })
}

fn commit_install(
    home: &Path,
    source: Source,
    plugin: &PluginEntry,
    downloaded: Option<&Path>,
) -> anyhow::Result<Value> {
    let _config_lock = acquire_lock(
        home,
        "config.lock",
        "另一个窗口正在更新插件配置，请稍后重试",
    )?;
    let mut state = read_state(home)?;
    let target = package_root(home, source, plugin);
    let (original_config, config) = updated_config(home, source, plugin)?;
    ensure_directory(home, target.parent().context("插件缓存路径无父目录")?)?;
    let installed = InstalledPlugin {
        source: source.id.to_owned(),
        id: plugin.id.clone(),
        name: plugin.name.clone(),
        version: plugin.version.clone(),
        tree: plugin.tree.clone(),
    };
    let target_exists = match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => true,
        Ok(_) => bail!("插件安装位置被文件或符号链接占用，拒绝覆盖"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error).context("无法检查插件安装位置"),
    };
    if target_exists {
        let same = state
            .plugins
            .get(&install_key(source, &plugin.id))
            .is_some_and(|item| {
                item.version == plugin.version && item.tree.eq_ignore_ascii_case(&plugin.tree)
            });
        if !same && !receipt_matches(home, source, plugin) {
            bail!("同名同版本的插件缓存已存在，拒绝覆盖；请使用新的插件版本");
        }
        validate_payload(&target, plugin)?;
    } else {
        let downloaded = downloaded.context("插件下载目录不存在")?;
        validate_payload(downloaded, plugin)?;
        let receipt_dir = plugin_market_root(home, source, &plugin.id);
        ensure_directory(home, &receipt_dir)?;
        // 先记录已校验下载，配置或状态写失败后可以继续完成，避免覆盖缓存。
        write_json(&receipt_dir.join("ready.json"), &installed)?;
        // 仅移动本次下载目录；从不清除既有版本或用户文件。
        fs::rename(downloaded, &target).context("无法保存已下载的插件")?;
    }
    state
        .plugins
        .insert(install_key(source, &plugin.id), installed);
    let manifest_dir = plugin_market_root(home, source, &plugin.id).join(".agents/plugins");
    ensure_directory(home, &manifest_dir)?;
    write_json(
        &manifest_dir.join("marketplace.json"),
        &market_manifest(home, source, plugin),
    )?;
    write_config_if_unchanged(home, original_config.as_deref(), &config)?;
    write_json(&state_path(home), &state)?;
    Ok(json!({
        "status": "ok", "message": "插件文件已安装并启用，重启 Codex 后加载。",
        "plugin": plugin_payload(home, source, plugin, &state), "restartRequired": true,
    }))
}

pub async fn install_plugin(home: &Path, source: &str, id: &str) -> anyhow::Result<Value> {
    let source = Source::parse(source)?;
    if !valid_id(id) {
        bail!("插件 id 无效");
    }
    let home = prepare_home(home)?;
    let _lock = acquire_install_lock(&home)?;
    set_status(&home, source, id, true, "fetching", "正在读取插件索引…")?;
    let result = async {
        let (index, _) = load_index(&home, source, false).await?;
        let plugin = index
            .plugins
            .iter()
            .find(|plugin| plugin.id == id)
            .context("插件市场中未找到该插件，请刷新索引")?;
        let state = read_state(&home)?;
        let existing = state
            .plugins
            .get(&install_key(source, &plugin.id))
            .is_some_and(|item| {
                item.version == plugin.version
                    && item.tree.eq_ignore_ascii_case(&plugin.tree)
                    && package_root(&home, source, plugin).is_dir()
            });
        // 以前从旧公开库装的同一个插件：装上这一份后换掉它，不然会有两份同名插件。
        let legacy = (source.id == "full")
            .then(|| state.plugins.values().find(|item| legacy_install_of(item, plugin)))
            .flatten()
            .map(|item| item.id.clone());
        let mut installed = if existing
            || (package_root(&home, source, plugin).is_dir()
                && receipt_matches(&home, source, plugin))
        {
            commit_install(&home, source, plugin, None)?
        } else {
            // 下载之前先验证当前配置，避免无意义下载或覆盖不可解析配置。
            updated_config(&home, source, plugin)?;
            set_status(&home, source, id, true, "fetching", "正在下载所选插件…")?;
            let downloaded = download_package(&home, source, &index, plugin).await?;
            set_status(&home, source, id, true, "fetching", "正在校验并启用插件…")?;
            commit_install(&home, source, plugin, Some(&downloaded))?
        };
        if let Some(legacy) = legacy {
            if let Err(error) = remove_installed(&home, Source::parse("public")?, &legacy) {
                installed["message"] = json!(format!(
                    "插件文件已安装并启用，重启 Codex 后加载。以前从旧公开库装的那一份没能移除：{error}"
                ));
            }
        }
        anyhow::Ok(installed)
    }
    .await;
    match &result {
        Ok(_) => set_status(
            &home,
            source,
            id,
            false,
            "complete",
            "插件已安装，重启 Codex 后加载。",
        )?,
        Err(error) => {
            let _ = set_status(&home, source, id, false, "failed", &error.to_string());
        }
    }
    result
}

/// 判断市场名是否属于本插件市场的散列命名空间（`codex-plus-plugin-market-<12 位十六进制>`）。
fn owned_market_name(name: &str) -> bool {
    name.strip_prefix(MARKETPLACE_NAME)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|hash| hash.len() == 12 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// 只删除安装时创建的命名空间目录：必须是 `allowed_root` 的直接子目录、名称属于本市场，
/// 且从 CODEX_HOME 起的每一层都不是符号链接，避免顺着链接删到市场之外。
fn owned_directory_present(
    home: &Path,
    allowed_root: &Path,
    target: &Path,
) -> anyhow::Result<bool> {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .context("插件目录名称无效")?;
    if target.parent() != Some(allowed_root) || !owned_market_name(name) {
        bail!("插件目录不在插件市场的安装位置内，拒绝删除");
    }
    let relative = target
        .strip_prefix(home)
        .context("插件目录超出 CODEX_HOME，拒绝删除")?;
    let mut current = home.to_path_buf();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("插件目录包含非法目录段，拒绝删除");
        }
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => bail!("插件目录被文件或符号链接占用，拒绝删除"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error).context("无法检查插件目录"),
        }
    }
    Ok(true)
}

fn remove_owned_directory(home: &Path, allowed_root: &Path, target: &Path) -> anyhow::Result<bool> {
    if !owned_directory_present(home, allowed_root, target)? {
        return Ok(false);
    }
    // remove_dir_all 不跟随目录内的符号链接，只删除链接本身。
    fs::remove_dir_all(target).context("无法删除插件目录")?;
    Ok(true)
}

/// 生成去掉该插件市场条目与插件启用条目后的配置；无改动时返回 None。
fn config_without_plugin(
    home: &Path,
    source: Source,
    id: &str,
) -> anyhow::Result<(Option<Vec<u8>>, Option<String>)> {
    let original = config_bytes(home)?;
    let Some(bytes) = original.as_deref() else {
        return Ok((None, None));
    };
    let contents = std::str::from_utf8(bytes).context("Codex 配置不是有效 UTF-8")?;
    // 同安装：不透传解析错误里的配置片段。
    let mut document = contents
        .parse::<DocumentMut>()
        .map_err(|_| anyhow::anyhow!("Codex 配置格式错误，请先修复 config.toml"))?;
    let market_name = plugin_market_name(source, id);
    let mut changed = false;
    if let Some(marketplaces) = document
        .get_mut("marketplaces")
        .and_then(Item::as_table_like_mut)
    {
        // 市场名带 source:id 散列，只有本插件会用；仍核对 source 末段，防止误删同名的手写条目。
        let owned = marketplaces.get(&market_name).is_some_and(|market| {
            match market.get("source").and_then(Item::as_str) {
                None => true,
                Some(path) => {
                    Path::new(path).file_name().and_then(|name| name.to_str())
                        == Some(market_name.as_str())
                }
            }
        });
        if owned {
            marketplaces.remove(&market_name);
            changed = true;
        }
        if changed && marketplaces.is_empty() {
            document.remove("marketplaces");
        }
    }
    if let Some(plugins) = document
        .get_mut("plugins")
        .and_then(Item::as_table_like_mut)
    {
        let suffix = format!("@{market_name}");
        let keys = plugins
            .iter()
            .map(|(key, _)| key.to_owned())
            .filter(|key| key.ends_with(&suffix))
            .collect::<Vec<_>>();
        for key in &keys {
            plugins.remove(key);
        }
        if !keys.is_empty() {
            changed = true;
            if plugins.is_empty() {
                document.remove("plugins");
            }
        }
    }
    Ok((original, changed.then(|| document.to_string())))
}

/// 卸载与安装对称，只撤销安装写入的内容：
/// `plugins/cache/<市场名>/`、`.tmp/codex-plus-plugin-market/markets/<市场名>/`、
/// config.toml 中的 `[marketplaces.<市场名>]` 与 `[plugins."<插件>@<市场名>"]`、安装记录和状态文件。
pub fn uninstall_plugin(home: &Path, source: &str, id: &str) -> anyhow::Result<Value> {
    let source = Source::parse(source)?;
    if !valid_id(id) {
        bail!("插件 id 无效");
    }
    let home = prepare_home(home)?;
    let _lock = acquire_lock(&home, "install.lock", "正在安装插件，请等待完成后再卸载")?;
    // 这个条目装在旧公开库名下时，卸的是那一份。
    let state = read_state(&home)?;
    let legacy = (source.id == "full" && !state.plugins.contains_key(&install_key(source, id)))
        .then(|| cached_index(&home, source))
        .flatten()
        .and_then(|index| {
            let entry = index.plugins.iter().find(|entry| entry.id == id)?;
            let installed = state.plugins.values().find(|item| legacy_install_of(item, entry))?;
            Some(installed.id.clone())
        });
    match legacy {
        Some(legacy) => remove_installed(&home, Source::parse("public")?, &legacy)?,
        None => remove_installed(&home, source, id)?,
    }
    let state = read_state(&home)?;
    let plugin = cached_index(&home, source).and_then(|index| {
        index
            .plugins
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| plugin_payload(&home, source, entry, &state))
    });
    Ok(json!({
        "status": "ok", "message": "插件已卸载，重启 Codex 后生效。",
        "source": source.id, "id": id, "plugin": plugin, "restartRequired": true,
    }))
}

/// 移除一份安装：配置、缓存目录、市场清单和安装记录。调用方持有安装锁。
fn remove_installed(home: &Path, source: Source, id: &str) -> anyhow::Result<()> {
    let _config_lock = acquire_lock(
        home,
        "config.lock",
        "另一个窗口正在更新插件配置，请稍后重试",
    )?;
    let mut state = read_state(home)?;
    let market_name = plugin_market_name(source, id);
    let cache_parent = home.join("plugins").join("cache");
    let cache_root = cache_parent.join(&market_name);
    let markets_parent = market_root(home).join("markets");
    let receipt_root = plugin_market_root(home, source, id);
    let (original_config, config) = config_without_plugin(home, source, id)?;
    let recorded = state.plugins.remove(&install_key(source, id)).is_some();
    // 改动任何东西之前先校验两个目录，路径异常时整个卸载不生效。
    let cache_present = owned_directory_present(home, &cache_parent, &cache_root)?;
    let receipt_present = owned_directory_present(home, &markets_parent, &receipt_root)?;
    if !recorded && config.is_none() && !cache_present && !receipt_present {
        bail!("该插件未通过插件市场安装，无需卸载");
    }
    // 先撤销配置，Codex 不再加载该插件；配置被其他窗口改过时不删任何文件。
    if let Some(config) = config {
        if config_bytes(home)?.as_deref() != original_config.as_deref() {
            bail!("Codex 配置已被其他窗口更新，请重试卸载；未覆盖新的配置");
        }
        crate::settings::atomic_write(&home.join("config.toml"), config.as_bytes())
            .context("无法移除插件配置")?;
    }
    remove_owned_directory(home, &cache_parent, &cache_root)?;
    remove_owned_directory(home, &markets_parent, &receipt_root)?;
    if recorded {
        write_json(&state_path(home), &state)?;
    }
    let status = status_path(home, source, id);
    check_regular_file(&status)?;
    match fs::remove_file(&status) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("无法清除插件安装状态"),
    }
}

/// Codex 的 ChatGPT 登录在不在（auth.json）。在的话原生插件页直接读官方云端目录，托管连接器也能连。
fn codex_signed_in(home: &Path) -> bool {
    crate::relay_config::chatgpt_auth_status_from_home(home).authenticated
}

/// 已安装包里声明的连接器：`.mcp.json` 的服务名，以及有没有 ChatGPT 托管的 `.app.json`。
fn package_connectors(directory: &Path, manifest: &Value) -> (Vec<String>, bool) {
    let object_at = |key: &str, default: &str, field: &str| {
        let relative = manifest.get(key).and_then(Value::as_str).unwrap_or(default);
        let path = safe_local_path(directory, relative)?;
        if !fs::metadata(&path).is_ok_and(|metadata| metadata.len() <= 1024 * 1024) {
            return None;
        }
        let config = serde_json::from_slice::<Value>(&fs::read(path).ok()?).ok()?;
        config.get(field)?.as_object().cloned()
    };
    let servers = object_at("mcpServers", ".mcp.json", "mcpServers")
        .map(|servers| servers.keys().cloned().collect())
        .unwrap_or_default();
    let has_apps = object_at("apps", ".app.json", "apps").is_some_and(|apps| !apps.is_empty());
    (servers, has_apps)
}

/// 原生界面只读 authPolicy 和 appsNeedingAuth；codexPlus* 两项给注入层决定装完提示什么。
/// 不走原生 plugin/install：它会不经询问直接弹出浏览器授权页。
fn native_install_result(home: &Path, source: Source, installed: Option<&InstalledPlugin>) -> Value {
    let (servers, has_apps) = installed
        .map(|plugin| package_dir(home, source, &plugin.id, &plugin.name, &plugin.version))
        .and_then(|directory| {
            let manifest = plugin_manifest(&directory).ok()?;
            Some(package_connectors(&directory, &manifest))
        })
        .unwrap_or_default();
    let hosted_only = has_apps && servers.is_empty() && !codex_signed_in(home);
    json!({
        "appsNeedingAuth": [], "authPolicy": "ON_USE",
        "codexPlusMcpServers": servers, "codexPlusHostedOnly": hosted_only,
    })
}

/// 没有 ChatGPT 登录时，在原生插件页上说明这个插件的连接器怎么连。
fn annotate_connectors(summary: &mut Value, plugin: &PluginEntry) {
    let (tag, note) = if plugin.has_mcp {
        ("", "连接器是 MCP 服务：安装后在 Codex 的「设置 → MCP」里登录。")
    } else if plugin.has_apps {
        (
            "需 ChatGPT 登录 · ",
            "连接器由 ChatGPT 托管，要有 ChatGPT 登录才能连接；插件里的 Skills 可以直接使用。",
        )
    } else if plugin.requires_auth {
        // 旧索引只知道「带连接器」，分不出是哪一种。
        ("", "这个插件带连接器或 MCP 服务，可能需要另行登录。")
    } else {
        return;
    };
    let interface = &mut summary["interface"];
    let short = interface["shortDescription"].as_str().unwrap_or_default();
    interface["shortDescription"] = json!(format!("{tag}{short}"));
    let long = interface["longDescription"].as_str().unwrap_or_default();
    interface["longDescription"] = json!(format!("{note}\n\n{long}"));
}

/// 原生插件页拿分类当分组标题；索引里没有或不像标题时不给，页面归到 "Other"。
fn native_category(plugin: &PluginEntry) -> Option<&str> {
    let category = plugin.category.trim();
    (!category.is_empty() && category.chars().count() <= 40 && !category.chars().any(char::is_control))
        .then_some(category)
}

/// 图标只认 https 地址；别的一律当没有，页面画默认图标。
fn native_icon(url: &str) -> Option<&str> {
    (url.starts_with("https://")
        && url.len() <= 600
        && !url.chars().any(|c| c.is_whitespace() || c.is_control() || "\"'<>\\".contains(c)))
    .then_some(url)
}

/// 交给注入层排版的官方分组（见 62-plugin-market-adapter 的 codexPlusPluginNativeArrange）。
/// 标题会进页面，这里只放行像标题的文字。
fn native_sections(index: &PluginIndex) -> Vec<Value> {
    let titled = |text: &str| {
        let text = text.trim();
        (!text.is_empty() && text.chars().count() <= 40 && !text.chars().any(char::is_control))
            .then(|| text.to_owned())
    };
    index
        .sections
        .iter()
        .filter_map(|section| {
            let slug_ok = !section.slug.is_empty()
                && section.slug.len() <= 80
                && section.slug.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
            let title: BTreeMap<&str, String> = section
                .title
                .iter()
                .filter_map(|(lang, text)| Some((lang.as_str(), titled(text)?)))
                .collect();
            (slug_ok && title.contains_key("en")).then(|| {
                json!({ "slug": section.slug, "title": title, "curated": section.curated, "plugins": section.plugins })
            })
        })
        .collect()
}

/// 目录是缓存快照，不是官方实时目录：在插件详情里写明快照日期。
fn annotate_snapshot(summary: &mut Value, index: &PluginIndex) {
    let Some(date) = snapshot_date(&index.updated_at) else {
        return;
    };
    let interface = &mut summary["interface"];
    let long = interface["longDescription"].as_str().unwrap_or_default();
    interface["longDescription"] = json!(format!(
        "{long}\n\n来自 Codex++ 缓存的官方目录快照（{date}），不是实时目录。"
    ));
}

/// 索引的快照日期（YYYY-MM-DD）；格式不对就不显示。
fn snapshot_date(updated_at: &str) -> Option<&str> {
    let date = updated_at.get(..10)?;
    date.bytes()
        .enumerate()
        .all(|(index, byte)| if index == 4 || index == 7 { byte == b'-' } else { byte.is_ascii_digit() })
        .then_some(date)
}

fn virtual_market_name(source: Source) -> String {
    format!("codex-plus-index-{}", source.id)
}

/// 哨兵绝对路径只交给渲染层拦截；不创建清单，不注册给原生下载器。
fn virtual_market_path(home: &Path, source: Source) -> PathBuf {
    market_root(home)
        .join(format!("virtual-{}", source.id))
        .join(".agents/plugins/marketplace.json")
}

fn native_aliases(index: &PluginIndex) -> BTreeMap<String, String> {
    let mut names = BTreeMap::<&str, usize>::new();
    for plugin in &index.plugins {
        *names.entry(plugin.name.as_str()).or_default() += 1;
    }
    index
        .plugins
        .iter()
        .map(|plugin| {
            let alias = if names.get(plugin.name.as_str()).copied().unwrap_or_default() > 1 {
                let hash = format!("{:x}", Sha256::digest(plugin.id.as_bytes()));
                format!("{}--{}", plugin.name, &hash[..8])
            } else {
                plugin.name.clone()
            };
            (plugin.id.clone(), alias)
        })
        .collect()
}

fn configured_plugin_enabled(home: &Path) -> anyhow::Result<BTreeMap<String, bool>> {
    let bytes = config_bytes(home)?;
    let contents = std::str::from_utf8(bytes.as_deref().unwrap_or_default())
        .context("Codex 配置不是有效 UTF-8")?;
    let document = contents
        .parse::<DocumentMut>()
        .map_err(|_| anyhow::anyhow!("Codex 配置格式错误，请先修复 config.toml"))?;
    Ok(document
        .get("plugins")
        .and_then(Item::as_table)
        .map(|plugins| {
            plugins
                .iter()
                .filter_map(|(name, entry)| {
                    entry
                        .as_table()
                        .and_then(|table| table.get("enabled"))
                        .and_then(Item::as_bool)
                        .map(|enabled| (name.to_owned(), enabled))
                })
                .collect()
        })
        .unwrap_or_default())
}

/// 仅返回 Codex++ 插件市场托管的本地安装记录，不枚举其他来源的 Codex 插件。
/// 不检查包目录、不拉取索引、不创建缓存；source 仅为 public/full，不包含仓库或路径。
pub fn installed_summary(home: &Path) -> anyhow::Result<Value> {
    let state = read_state(home)?;
    let enabled = configured_plugin_enabled(home)?;
    let plugins = state
        .plugins
        .values()
        .map(|plugin| {
            let source = Source::parse(&plugin.source)?;
            let id = format!("{}@{}", plugin.name, plugin_market_name(source, &plugin.id));
            Ok(json!({
                "enabled": enabled.get(&id).copied().unwrap_or(true),
                "id": id,
                "name": plugin.name,
                "version": plugin.version,
                "source": source.id,
            }))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(json!({ "plugins": plugins }))
}

fn native_summary(
    home: &Path,
    source: Source,
    revision: Option<&str>,
    plugin: &PluginEntry,
    alias: &str,
    state: &InstalledState,
    enabled: &BTreeMap<String, bool>,
) -> Value {
    // 装在旧公开库名下的，id 和本地目录都按那个市场名算：配置里的启用状态、Codex 加载的插件用的是它。
    let record = installed_record(home, source, plugin, state);
    let installed = record.is_some();
    let id = match record {
        Some((owner, local)) => format!("{}@{}", local.name, plugin_market_name(owner, &local.id)),
        None => format!("{}@{}", plugin.name, plugin_market_name(source, &plugin.id)),
    };
    let local_version = record.map(|(_, local)| local.version.as_str());
    let source_value = if let Some(revision) = revision {
        json!({ "type": "git", "url": format!("https://github.com/{}.git", source.repository), "path": plugin.path, "sha": revision, "refName": null })
    } else {
        let path = match record {
            Some((owner, local)) => installed_dir(home, owner, local),
            None => package_root(home, source, plugin),
        };
        json!({ "type": "local", "path": path.to_string_lossy() })
    };
    // 列表里一行显示的简介：包里没写就取完整描述的开头。
    let short: String = if plugin.short_description.trim().is_empty() {
        plugin.description.chars().take(300).collect()
    } else {
        plugin.short_description.clone()
    };
    let logo = native_icon(&plugin.icon_url);
    let logo_dark = native_icon(&plugin.icon_url_dark);
    json!({
        "id": id, "name": alias, "source": source_value,
        "codexPlusIndexId": plugin.id,
        "installed": installed, "enabled": installed && enabled.get(&id).copied().unwrap_or(true),
        "authPolicy": "ON_USE", "installPolicy": "AVAILABLE", "installPolicySource": null,
        "availability": "AVAILABLE", "disabledReason": null, "eligiblePlanTypes": null,
        "version": plugin.version, "localVersion": local_version, "keywords": plugin.tags,
        "remotePluginId": null, "shareContext": null, "installedAt": null,
        "mustShowInstallationInterstitial": false,
        "interface": {
            "displayName": if plugin.display_name.is_empty() { &plugin.name } else { &plugin.display_name },
            "shortDescription": short, "longDescription": plugin.description,
            "developerName": plugin.author, "category": native_category(plugin), "capabilities": [],
            "screenshots": [], "screenshotUrls": [],
            "composerIcon": null, "logo": null, "logoDark": null,
            "composerIconUrl": null, "logoUrl": logo, "logoUrlDark": logo_dark,
            "brandColor": null, "defaultPrompt": null, "privacyPolicyUrl": null,
            "termsOfServiceUrl": null, "websiteUrl": null,
        },
    })
}

/// 原生插件页按市场名分页签：显示名里的字母数字连起来是 `codex-official` 的市场进「公开」页签，
/// 和官方插件一起按分类分组；其他市场都在「个人」页签里各占一组、不分类。
/// 所以这个名字里除了 Codex、official 不能再有别的字母或数字（快照日期写在插件详情里）。
fn native_market(home: &Path, source: Source, plugins: Vec<Value>) -> Value {
    let name = if source.id == "full" { "Codex official 目录快照" } else { "Codex official 目录快照 · 旧公开库" };
    json!({
        "name": virtual_market_name(source), "path": virtual_market_path(home, source).to_string_lossy(),
        "interface": { "displayName": name },
        "plugins": plugins,
    })
}

fn native_list_payload(
    home: &Path,
    source: Source,
    result: anyhow::Result<(PluginIndex, bool)>,
    state: &InstalledState,
    enabled: &BTreeMap<String, bool>,
) -> Value {
    let (index, _) = match result {
        Ok(loaded) => loaded,
        Err(error) => {
            let errors = [json!({ "marketplacePath": virtual_market_path(home, source).to_string_lossy(), "message": error.to_string() })];
            return json!({ "marketplaces": [], "marketplaceLoadErrors": errors, "featuredPluginIds": [], "codexPlusSections": [] });
        }
    };
    let aliases = native_aliases(&index);
    let plugins = index
        .plugins
        .iter()
        .map(|plugin| {
            let alias = aliases.get(&plugin.id).map(String::as_str).unwrap_or(&plugin.name);
            let mut summary = native_summary(home, source, Some(&index.revision), plugin, alias, state, enabled);
            // 这份目录只在没有 ChatGPT 登录时给出（见 native_request）。
            annotate_connectors(&mut summary, plugin);
            // 列表有上万条，完整描述只在详情（plugin/read）里给。
            summary["interface"]["longDescription"] = Value::Null;
            summary
        })
        .collect();
    json!({
        "marketplaces": [native_market(home, source, plugins)], "marketplaceLoadErrors": [],
        "featuredPluginIds": [], "codexPlusSections": native_sections(&index),
    })
}

fn cached_index(home: &Path, source: Source) -> Option<PluginIndex> {
    let path = index_path(home, source);
    check_regular_file(&path).ok()?;
    parse_index(&fs::read(path).ok()?, source).ok()
}

fn managed_source(home: &Path, params: &Value) -> anyhow::Result<Source> {
    for name in ["public", "full"] {
        let source = Source::parse(name)?;
        let path = virtual_market_path(home, source);
        if params.get("marketplacePath").and_then(Value::as_str)
            == Some(path.to_string_lossy().as_ref())
            || params.get("remoteMarketplaceName").and_then(Value::as_str)
                == Some(virtual_market_name(source).as_str())
        {
            return Ok(source);
        }
    }
    bail!("该请求不属于 Codex 插件市场")
}

fn managed_entry<'a>(
    index: &'a PluginIndex,
    source: Source,
    params: &Value,
) -> anyhow::Result<(&'a PluginEntry, String)> {
    let aliases = native_aliases(index);
    let name = params
        .get("pluginName")
        .and_then(Value::as_str)
        .context("缺少插件名称")?;
    let id = params
        .get("pluginId")
        .or_else(|| params.get("id"))
        .and_then(Value::as_str);
    if let Some(id) = id {
        return index
            .plugins
            .iter()
            .find_map(|plugin| {
                let expected_id =
                    format!("{}@{}", plugin.name, plugin_market_name(source, &plugin.id));
                (id == expected_id).then(|| {
                    (
                        plugin,
                        aliases
                            .get(&plugin.id)
                            .cloned()
                            .unwrap_or_else(|| plugin.name.clone()),
                    )
                })
            })
            .context("插件市场中未找到指定 ID");
    }
    index
        .plugins
        .iter()
        .find_map(|plugin| {
            let alias = aliases.get(&plugin.id)?;
            if alias == name {
                Some((plugin, alias.clone()))
            } else {
                None
            }
        })
        .context("插件市场中未找到该插件，请刷新目录并选择对应条目")
}

fn safe_local_path(root: &Path, relative: &str) -> Option<PathBuf> {
    if relative.contains('\\') || relative.contains('\0') {
        return None;
    }
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let path = root.join(relative);
    let canonical = path.canonicalize().ok()?;
    if canonical.starts_with(root.canonicalize().ok()?) {
        Some(canonical)
    } else {
        None
    }
}

fn installed_skill_summaries(root: &Path, manifest: &Value, enabled: bool) -> Vec<Value> {
    fn scan(root: &Path, directory: &Path, depth: usize, enabled: bool, skills: &mut Vec<Value>) {
        if depth > 8 || skills.len() >= 1000 {
            return;
        }
        let path = directory.join("SKILL.md");
        if path.is_file() && path.canonicalize().is_ok_and(|path| path.starts_with(root)) {
            if let Ok(metadata) = fs::metadata(&path) {
                if metadata.len() <= 256 * 1024 {
                    if let Ok(text) = fs::read_to_string(&path) {
                        let fallback = directory
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("skill");
                        let (name, description) =
                            crate::skills::parse_skill_frontmatter(&text, fallback);
                        skills.push(json!({ "name": name, "description": description, "shortDescription": description, "enabled": enabled, "path": path.to_string_lossy(), "interface": null }));
                    }
                }
            }
            return;
        }
        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten() {
                if entry
                    .file_type()
                    .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                {
                    scan(root, &entry.path(), depth + 1, enabled, skills);
                }
            }
        }
    }
    let root = match root.canonicalize() {
        Ok(root) => root,
        Err(_) => return vec![],
    };
    let paths = match manifest.get("skills") {
        Some(Value::String(path)) => vec![path.as_str()],
        Some(Value::Array(paths)) => paths.iter().filter_map(Value::as_str).collect(),
        _ => vec!["skills"],
    };
    let mut result = Vec::new();
    for relative in paths {
        if let Some(directory) = safe_local_path(&root, relative) {
            if directory.is_dir() {
                scan(&root, &directory, 0, enabled, &mut result);
            }
        }
    }
    result
}

fn native_detail(
    home: &Path,
    source: Source,
    index: &PluginIndex,
    plugin: &PluginEntry,
    alias: &str,
    state: &InstalledState,
    enabled: &BTreeMap<String, bool>,
) -> Value {
    let mut summary = native_summary(
        home,
        source,
        Some(&index.revision),
        plugin,
        alias,
        state,
        enabled,
    );
    let mut skills = Vec::new();
    let mut mcp_servers = Vec::new();
    let mut declared = None;
    if let Some((owner, record)) = installed_record(home, source, plugin, state) {
        let mut local = plugin.clone();
        local.version.clone_from(&record.version);
        let directory = installed_dir(home, owner, record);
        if let Ok(manifest) = plugin_manifest(&directory) {
            skills = installed_skill_summaries(
                &directory,
                &manifest,
                summary["enabled"].as_bool().unwrap_or(false),
            );
            if let Some(interface) = manifest.get("interface").and_then(Value::as_object) {
                for key in [
                    "displayName",
                    "shortDescription",
                    "longDescription",
                    "developerName",
                    "brandColor",
                    "category",
                ] {
                    if let Some(value) = interface.get(key).filter(|value| value.is_string()) {
                        summary["interface"][key] = value.clone();
                    }
                }
                for key in ["logo", "logoDark", "composerIcon"] {
                    if let Some(path) = interface
                        .get(key)
                        .and_then(Value::as_str)
                        .and_then(|path| safe_local_path(&directory, path))
                        .filter(|path| path.is_file())
                    {
                        summary["interface"][key] = json!(path.to_string_lossy());
                    }
                }
            }
            let (servers, has_apps) = package_connectors(&directory, &manifest);
            // 装好的包以包里的声明为准，比索引里的标记准。
            local.has_mcp = !servers.is_empty();
            local.has_apps = has_apps;
            local.requires_auth = local.has_mcp || has_apps;
            mcp_servers = servers;
            declared = Some(local);
        }
    }
    if !codex_signed_in(home) {
        annotate_connectors(&mut summary, declared.as_ref().unwrap_or(plugin));
    }
    annotate_snapshot(&mut summary, index);
    json!({ "plugin": {
        "summary": summary, "marketplaceName": virtual_market_name(source), "marketplacePath": virtual_market_path(home, source).to_string_lossy(),
        "description": plugin.description, "skills": skills, "mcpServers": mcp_servers,
        "hooks": [], "apps": [], "appTemplates": [], "shareUrl": null, "scheduledTasks": null, "onboardingSkill": null,
    } })
}

fn installed_fallback(home: &Path, source: Source, installed: &InstalledPlugin) -> PluginEntry {
    let mut plugin = PluginEntry {
        id: installed.id.clone(),
        name: installed.name.clone(),
        display_name: installed.name.clone(),
        description: String::new(),
        version: installed.version.clone(),
        author: String::new(),
        tags: vec![],
        license: String::new(),
        path: String::new(),
        tree: installed.tree.clone(),
        bytes: 0,
        skills: 0,
        requires_auth: false,
        has_apps: false,
        has_mcp: false,
        category: String::new(),
        short_description: String::new(),
        icon_url: String::new(),
        icon_url_dark: String::new(),
    };
    if let Ok(manifest) = plugin_manifest(&package_root(home, source, &plugin)) {
        plugin.display_name = manifest
            .pointer("/interface/displayName")
            .and_then(Value::as_str)
            .unwrap_or(&installed.name)
            .to_owned();
        plugin.description = manifest
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        plugin.author = manifest
            .pointer("/author/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        plugin.category = manifest
            .pointer("/interface/category")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
    }
    plugin
}

/// 原生 UI 形状适配。虚拟市场的 read/install 由注入层独占拦截，不传给原生下载器。
pub async fn native_request(home: &Path, method: &str, params: Value) -> anyhow::Result<Value> {
    let home = prepare_home(home)?;
    crate::relay_config::cleanup_legacy_official_marketplace_overrides_in_home(&home)?;
    let state = read_state(&home)?;
    let enabled = configured_plugin_enabled(&home)?;
    match method {
        "plugin/list" | "list-plugins" => {
            // 有 ChatGPT 登录时原生插件页直接读官方云端目录，不再并入缓存快照；
            // 已经从快照装上的插件仍由下面的 plugin/installed 给出。
            if codex_signed_in(&home) {
                return Ok(json!({ "marketplaces": [], "marketplaceLoadErrors": [], "featuredPluginIds": [] }));
            }
            let full = Source::parse("full")?;
            let refresh = params
                .get("forceRefetch")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let result = load_index(&home, full, refresh).await;
            Ok(native_list_payload(&home, full, result, &state, &enabled))
        }
        "plugin/installed"
        | "plugin/installed-list"
        | "list-installed-plugins"
        | "installed-plugins" => {
            // 提及、聊天初始化等高频路径只读本地已安装记录，不拉市场目录。
            let mut marketplaces = Vec::new();
            let full_index = cached_index(&home, Source::parse("full")?);
            // 以前从公开库装的、完整库里有同一个插件的，算在完整库名下。
            let merged = |installed: &InstalledPlugin| {
                full_index.as_ref().and_then(|index| {
                    index.plugins.iter().find(|entry| legacy_install_of(installed, entry))
                })
            };
            for name in ["public", "full"] {
                let source = Source::parse(name)?;
                let index = if name == "full" { full_index.clone() } else { cached_index(&home, source) };
                let aliases = index.as_ref().map(native_aliases).unwrap_or_default();
                let mut plugins = Vec::new();
                for installed in state.plugins.values().filter(|installed| {
                    if name == "full" {
                        installed.source == "full" || merged(installed).is_some()
                    } else {
                        installed.source == "public" && merged(installed).is_none()
                    }
                }) {
                    let fallback;
                    let entry = if let Some(entry) = merged(installed).filter(|_| name == "full") {
                        entry
                    } else if let Some(entry) = index.as_ref().and_then(|index| {
                        index.plugins.iter().find(|entry| entry.id == installed.id)
                    }) {
                        entry
                    } else {
                        fallback = installed_fallback(&home, source, installed);
                        &fallback
                    };
                    let summary = native_summary(
                        &home,
                        source,
                        index.as_ref().map(|index| index.revision.as_str()),
                        entry,
                        aliases
                            .get(&entry.id)
                            .map(String::as_str)
                            .unwrap_or(&entry.name),
                        &state,
                        &enabled,
                    );
                    if summary["installed"].as_bool().unwrap_or(false) {
                        plugins.push(summary);
                    }
                }
                if !plugins.is_empty() {
                    marketplaces.push(native_market(&home, source, plugins));
                }
            }
            Ok(json!({ "marketplaces": marketplaces, "marketplaceLoadErrors": [] }))
        }
        "plugin/read" | "read-plugin" | "plugin/install" | "install-plugin" => {
            let source = managed_source(&home, &params)?;
            let (index, _) = load_index(&home, source, false).await?;
            let (plugin, alias) = managed_entry(&index, source, &params)?;
            if matches!(method, "plugin/install" | "install-plugin") {
                install_plugin(&home, source.id, &plugin.id).await?;
                let state = read_state(&home)?;
                Ok(native_install_result(
                    &home,
                    source,
                    state.plugins.get(&install_key(source, &plugin.id)),
                ))
            } else {
                Ok(native_detail(
                    &home, source, &index, plugin, &alias, &state, &enabled,
                ))
            }
        }
        _ => bail!("独立插件市场不支持该原生方法"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_market_is_public_and_reads_its_own_index_file() {
        // 完整库已公开（#2804）：不再要求 gh 登录。
        let full = Source::parse("full").unwrap();
        assert!(!full.private);
        // 仓库里的 plugin-index.json 留给旧版本，新索引和它的本机缓存都另起文件名。
        assert_eq!(full.index_file, "plugin-index-v2.json");
        assert!(index_path(Path::new("/tmp/home"), full).ends_with("index-full-v2.json"));
        let public = Source::parse("public").unwrap();
        assert!(index_path(Path::new("/tmp/home"), public).ends_with("index-public.json"));
    }

    #[tokio::test]
    async fn plugins_installed_from_the_old_public_market_count_as_the_full_entry() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let public = Source::parse("public").unwrap();
        // 旧公开库里来自 Git 市场的插件直接拿名字当 id。
        let mut legacy = entry();
        legacy.id = "notes".into();
        legacy.name = "notes".into();
        let download = home.join(".tmp/notes");
        write_payload(&download, &legacy);
        commit_install(&home, public, &legacy, Some(&download)).unwrap();
        let legacy_market = plugin_market_name(public, "notes");

        let mut same = entry();
        same.id = "plugin_notes".into();
        same.name = "notes".into();
        same.version = "2.0.0".into();
        let mut other = entry();
        other.id = "plugin_other".into();
        other.name = "other".into();
        let full = cache_indexes(&home, vec![same.clone(), other]);

        let listed = list_plugins(&home, "full", false).await.unwrap();
        let notes = &listed["plugins"][0];
        assert_eq!(notes["id"], "plugin_notes");
        assert_eq!(notes["installed"], true);
        assert_eq!(notes["installedVersion"], "1.0.0");
        assert_eq!(notes["updateAvailable"], true);
        assert_eq!(notes["marketplaceName"], legacy_market.as_str());
        assert_eq!(listed["plugins"][1]["installed"], false);

        // 原生页面上它的 id 仍是装的那一份的（配置里的启用状态按它记），并且只列在完整库名下。
        let native = native_request(&home, "plugin/list", json!({})).await.unwrap();
        let summary = &native["marketplaces"][0]["plugins"][0];
        assert_eq!(summary["id"], format!("notes@{legacy_market}"));
        assert_eq!(summary["installed"], true);
        assert_eq!(summary["localVersion"], "1.0.0");
        let installed = native_request(&home, "installed-plugins", json!({})).await.unwrap();
        assert_eq!(installed["marketplaces"].as_array().unwrap().len(), 1);
        assert_eq!(installed["marketplaces"][0]["name"], "codex-plus-index-full");
        assert_eq!(installed["marketplaces"][0]["plugins"][0]["id"], format!("notes@{legacy_market}"));
        let params = json!({ "marketplacePath": virtual_market_path(&home, full), "pluginName": "notes" });
        let detail = native_request(&home, "plugin/read", params).await.unwrap();
        assert_eq!(detail["plugin"]["skills"].as_array().unwrap().len(), 1);

        // 装上完整库的这一份后，旧的那一份被换掉。
        let download = home.join(".tmp/notes-full");
        write_payload(&download, &same);
        commit_install(&home, full, &same, Some(&download)).unwrap();
        install_plugin(&home, "full", "plugin_notes").await.unwrap();
        let state = read_state(&home).unwrap();
        assert_eq!(state.plugins.keys().collect::<Vec<_>>(), [&install_key(full, "plugin_notes")]);
        assert!(!home.join("plugins/cache").join(&legacy_market).exists());
        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(!config.contains(&legacy_market), "{config}");
        assert!(config.contains(&plugin_market_name(full, "plugin_notes")), "{config}");
    }

    #[tokio::test]
    async fn uninstalling_a_full_entry_removes_the_copy_from_the_old_public_market() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let public = Source::parse("public").unwrap();
        // 两个仓库里 id 相同的插件。
        let plugin = entry();
        let download = home.join(".tmp/sample");
        write_payload(&download, &plugin);
        commit_install(&home, public, &plugin, Some(&download)).unwrap();
        cache_indexes(&home, vec![plugin.clone()]);
        let result = uninstall_plugin(&home, "full", &plugin.id).unwrap();
        assert_eq!(result["plugin"]["installed"], false);
        assert!(read_state(&home).unwrap().plugins.is_empty());
        assert!(!home.join("plugins/cache").join(plugin_market_name(public, &plugin.id)).exists());
        // 没装过的照旧报错。
        assert!(uninstall_plugin(&home, "full", &plugin.id).is_err());
    }

    #[test]
    fn market_names_land_in_the_public_tab_of_the_native_page() {
        // Codex 26.1117 的插件页：市场显示名去掉首尾空白、转小写、非 [a-z0-9] 连成一个 `-`、
        // 再去掉首尾的 `-`，得到 codex-official 的进「公开」页签。
        let normalised = |name: &str| {
            let mut result = String::new();
            for part in name
                .trim()
                .to_lowercase()
                .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
                .filter(|part| !part.is_empty())
            {
                if !result.is_empty() {
                    result.push('-');
                }
                result.push_str(part);
            }
            result
        };
        for source in ["public", "full"] {
            let market = native_market(Path::new("/tmp/home"), Source::parse(source).unwrap(), Vec::new());
            assert_eq!(
                normalised(market["interface"]["displayName"].as_str().unwrap()),
                "codex-official",
                "{source}"
            );
        }
    }

    #[tokio::test]
    async fn native_list_groups_plugins_by_the_category_in_the_index() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let mut tools = entry();
        tools.category = " Developer Tools ".into();
        let mut unknown = entry();
        unknown.id = "plugin_unknown".into();
        unknown.name = "unknown".into();
        let mut odd = entry();
        odd.id = "plugin_odd".into();
        odd.name = "odd".into();
        odd.category = "x".repeat(41);
        cache_indexes(&home, vec![tools, unknown, odd]);
        let payload = native_request(&home, "plugin/list", json!({}))
            .await
            .unwrap();
        let category = |name: &str| {
            payload["marketplaces"][0]["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .find(|plugin| plugin["name"] == name)
                .unwrap()["interface"]["category"]
                .clone()
        };
        assert_eq!(category("sample"), "Developer Tools");
        // 旧索引没有分类、或分类不像标题：不给，原生页面归到 "Other"。
        assert!(category("unknown").is_null());
        assert!(category("odd").is_null());
        // 旧索引也没有官方分组。
        assert_eq!(payload["codexPlusSections"], json!([]));
    }

    #[tokio::test]
    async fn native_list_carries_the_official_layout_icons_and_short_descriptions() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let mut listed = entry();
        listed.short_description = "Short".into();
        listed.icon_url = "https://files.example.com/content?id=1&sig=ab".into();
        listed.icon_url_dark = "https://files.example.com/content?id=2&sig=cd".into();
        let mut plain = entry();
        plain.id = "plugin_plain".into();
        plain.name = "plain".into();
        plain.icon_url = "javascript:alert(1)".into();
        plain.icon_url_dark = "https://files.example.com/a b".into();
        let full = cache_indexes(&home, vec![listed, plain]);
        let path = index_path(&home, full);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["sections"] = json!([
            { "slug": "featured", "title": { "en": " Featured ", "zh": "热门" }, "curated": true, "plugins": ["plugin_test"] },
            { "slug": "productivity", "title": { "en": "Productivity", "zh": "x".repeat(41) }, "plugins": ["plugin_test", "plugin_plain"] },
            { "slug": "Bad Slug", "title": { "en": "Bad" }, "plugins": [] },
            { "slug": "untitled", "title": { "zh": "没有英文标题" }, "plugins": [] },
        ]);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let payload = native_request(&home, "plugin/list", json!({}))
            .await
            .unwrap();
        assert_eq!(
            payload["codexPlusSections"],
            json!([
                { "slug": "featured", "title": { "en": "Featured", "zh": "热门" }, "curated": true, "plugins": ["plugin_test"] },
                { "slug": "productivity", "title": { "en": "Productivity" }, "curated": false, "plugins": ["plugin_test", "plugin_plain"] },
            ])
        );
        let interface = |name: &str| {
            payload["marketplaces"][0]["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .find(|plugin| plugin["name"] == name)
                .unwrap()["interface"]
                .clone()
        };
        let listed = interface("sample");
        assert_eq!(listed["shortDescription"], "Short");
        assert_eq!(listed["logoUrl"], "https://files.example.com/content?id=1&sig=ab");
        assert_eq!(listed["logoUrlDark"], "https://files.example.com/content?id=2&sig=cd");
        // 没有简介的用完整描述；不是 https 的图标地址不给。
        let plain = interface("plain");
        assert_eq!(plain["shortDescription"], "sample plugin");
        assert!(plain["logoUrl"].is_null() && plain["logoUrlDark"].is_null());
    }

    #[tokio::test]
    async fn index_falls_back_to_github_api_when_raw_is_unreachable() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // raw.githubusercontent.com 连不上、api.github.com 可达（#2804）：换 contents 接口取原文。
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/raw/plugin-index.json"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/x/contents/plugin-index.json"))
            .and(header("accept", "application/vnd.github.raw+json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"plugins\":[]}"))
            .mount(&server)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let raw = format!("{}/raw/plugin-index.json", server.uri());
        let api = format!("{}/repos/x/contents/plugin-index.json", server.uri());

        let bytes = fetch_index_over_http(&client, &[&raw, &api]).await.unwrap();
        assert_eq!(bytes.as_deref(), Some(&b"{\"plugins\":[]}"[..]));
        // 匿名请求：不带任何凭据。
        let requests = server.received_requests().await.unwrap();
        assert!(requests.iter().all(|request| !request.headers.contains_key("authorization")));

        // 两个地址都取不到：交给调用方报错。
        let missing = format!("{}/missing", server.uri());
        assert_eq!(fetch_index_over_http(&client, &[&raw, &missing]).await.unwrap(), None);

        // 超过大小上限直接报错，不再试下一个地址。
        let large = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b' '; MAX_INDEX_BYTES + 1]))
            .mount(&large)
            .await;
        let error = fetch_index_over_http(&client, &[&large.uri(), &api]).await.unwrap_err();
        assert!(error.to_string().contains("32 MiB"), "{error:#}");
    }

    #[test]
    fn git_proxy_and_credentials_are_ephemeral_while_hooks_and_templates_stay_isolated() {
        let temp = tempfile::tempdir().unwrap();
        let command = git_command(
            temp.path(),
            Some("fixture-token"),
            Some("http://fixture:proxy-secret@127.0.0.1:7890"),
        );
        let args = command
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.iter().any(|value| value == "init.templateDir="));
        assert!(
            args.iter()
                .any(|value| value.starts_with("core.hooksPath="))
        );
        assert!(
            !args
                .iter()
                .any(|value| value.contains("fixture-token") || value.contains("proxy-secret"))
        );
        let env = command
            .as_std()
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(env["GIT_CONFIG_NOSYSTEM"].as_deref(), Some("1"));
        assert_eq!(env["GIT_CONFIG_COUNT"].as_deref(), Some("2"));
        assert_eq!(
            env["GIT_CONFIG_KEY_0"].as_deref(),
            Some("http.https://github.com/.extraheader")
        );
        assert_eq!(env["GIT_CONFIG_KEY_1"].as_deref(), Some("http.proxy"));
        assert!(
            env["GIT_CONFIG_VALUE_0"]
                .as_deref()
                .unwrap()
                .starts_with("AUTHORIZATION: basic ")
        );
        assert_eq!(env["GIT_TRACE_CURL"], None);
        assert_eq!(env["GIT_CONFIG_PARAMETERS"], None);
        assert_eq!(env["GIT_TEMPLATE_DIR"], None);
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
        ] {
            assert!(
                !env.contains_key(key),
                "existing network environment must be inherited"
            );
        }
        assert!(!temp.path().join(".gitconfig").exists());
    }

    #[test]
    fn git_failure_messages_classify_reasons_without_echoing_credentials_or_misleading_public_auth()
    {
        for (stderr, kind) in [
            (
                "fatal: unable to access https://fixture-secret@github.com/repo: Could not resolve proxy: proxy-secret",
                GitFailure::Network,
            ),
            (
                "fatal: Authentication failed for https://fixture-secret@github.com/repo",
                GitFailure::Access,
            ),
            (
                "fatal: could not create work tree dir '/cache/proxy-plugin': Permission denied fixture-secret",
                GitFailure::LocalPermission,
            ),
        ] {
            assert_eq!(classify_git_failure(stderr.as_bytes()), kind);
            for private in [false, true] {
                assert!(!kind.message(private).contains("fixture-secret"));
                assert!(!kind.message(private).contains("proxy-secret"));
            }
        }
        assert!(!GitFailure::Access.message(false).contains("gh auth login"));
        assert!(!GitFailure::Network.message(true).contains("gh auth login"));
        assert!(GitFailure::Access.message(true).contains("gh auth login"));
    }

    #[cfg(unix)]
    fn fake_git(temp: &tempfile::TempDir, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = temp.path().join("fake-git");
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn git_proxy_lookup_reads_only_proxy_and_respects_environment_explicit_direct_and_system_fallback()
     {
        let temp = tempfile::tempdir().unwrap();
        let url = "https://github.com/BigPizzaV3/CodexPlusPlusPluginCache.git";
        let git = fake_git(
            &temp,
            r#"
[ "$1" = config ] && [ "$3" = --includes ] && [ "$4" = --null ] && [ "$5" = --get-urlmatch ] && [ "$6" = http.proxy ] || exit 9
if [ "$2" = --global ]; then printf 'http://127.0.0.1:7890\000'; exit 0; fi
exit 1
"#,
        );
        assert_eq!(
            resolve_git_proxy_with(&git, temp.path(), url, false, || panic!(
                "global proxy should win"
            ))
            .await
            .unwrap()
            .as_deref(),
            Some("http://127.0.0.1:7890")
        );
        let http_only = |key: &str| (key == "HTTP_PROXY").then(|| "http://127.0.0.1:7999".into());
        assert!(!has_git_proxy_environment(url, http_only));
        assert!(has_git_proxy_environment("http://github.com/", http_only));
        assert_eq!(
            resolve_git_proxy_with(
                &git,
                temp.path(),
                url,
                has_git_proxy_environment(url, http_only),
                || panic!("HTTPS Git proxy should still be read")
            )
            .await
            .unwrap()
            .as_deref(),
            Some("http://127.0.0.1:7890")
        );
        let git = fake_git(
            &temp,
            "if [ \"$2\" = --global ]; then exit 1; fi; printf 'http://127.0.0.1:7891\\000'",
        );
        assert_eq!(
            resolve_git_proxy_with(&git, temp.path(), url, false, || panic!(
                "system Git proxy should win"
            ))
            .await
            .unwrap()
            .as_deref(),
            Some("http://127.0.0.1:7891")
        );
        let missing_git = temp.path().join("missing-git");
        assert!(
            resolve_git_proxy_with(&missing_git, temp.path(), url, true, || panic!(
                "environment proxy should win"
            ))
            .await
            .unwrap()
            .is_none()
        );
        let git = fake_git(&temp, "printf '\\000'; exit 0");
        assert_eq!(
            resolve_git_proxy_with(&git, temp.path(), url, false, || panic!(
                "explicit empty proxy means direct"
            ))
            .await
            .unwrap(),
            Some(String::new())
        );
        let git = fake_git(&temp, "exit 1");
        assert_eq!(
            resolve_git_proxy_with(&git, temp.path(), url, false, || Some(
                "http://127.0.0.1:8888".into()
            ))
            .await
            .unwrap()
            .as_deref(),
            Some("http://127.0.0.1:8888")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn git_process_failures_preserve_only_fixed_diagnostics_and_missing_git_is_explicit() {
        let temp = tempfile::tempdir().unwrap();
        let git = fake_git(
            &temp,
            "printf 'fatal: Could not resolve proxy: fixture-secret proxy-secret' >&2; exit 128",
        );
        let output = git_output(
            git_command_with_binary(&git, temp.path(), None, Some("http://127.0.0.1:7890"))
                .arg("fetch"),
            64,
            false,
        )
        .await
        .unwrap();
        assert!(!output.status.success());
        let message = classify_git_failure(&output.stderr).message(false);
        assert_eq!(classify_git_failure(&output.stderr), GitFailure::Network);
        assert!(!message.contains("fixture-secret") && !message.contains("proxy-secret"));
        let error = git_output(
            &mut Command::new(temp.path().join("missing-git")),
            64,
            false,
        )
        .await
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("未找到 Git"));
        assert!(!error.contains("gh auth login"));
        let git = fake_git(
            &temp,
            "printf 'fatal: Could not resolve proxy\\n' >&2; i=0; while [ \"$i\" -lt 6000 ]; do printf 'proxy-secret' >&2; i=$((i + 1)); done; printf 'partial'; exit 128",
        );
        let output = git_output(&mut Command::new(git), 64, false).await.unwrap();
        assert_eq!(output.stdout, b"partial");
        assert_eq!(output.stderr.len(), 16 * 1024);
        assert_eq!(classify_git_failure(&output.stderr), GitFailure::Network);
    }

    fn entry() -> PluginEntry {
        PluginEntry {
            id: "plugin_test".into(),
            name: "sample".into(),
            display_name: "Sample".into(),
            description: "sample plugin".into(),
            version: "1.0.0".into(),
            author: "Test".into(),
            tags: vec![],
            license: "MIT".into(),
            path: "plugins/sample".into(),
            tree: "a".repeat(40),
            bytes: 128,
            skills: 1,
            requires_auth: false,
            has_apps: false,
            has_mcp: false,
            category: String::new(),
            short_description: String::new(),
            icon_url: String::new(),
            icon_url_dark: String::new(),
        }
    }

    fn index(plugin: PluginEntry) -> PluginIndex {
        PluginIndex {
            schema_version: 1,
            repository: Source::parse("public").unwrap().repository.into(),
            revision: "b".repeat(40),
            updated_at: "2026-10-08".into(),
            sections: Vec::new(),
            plugins: vec![plugin],
        }
    }

    fn write_payload(path: &Path, plugin: &PluginEntry) {
        fs::create_dir_all(path.join(".codex-plugin")).unwrap();
        fs::write(
            path.join(".codex-plugin/plugin.json"),
            serde_json::to_vec(
                &json!({ "name": plugin.name, "version": plugin.version, "skills": "./skills/" }),
            )
            .unwrap(),
        )
        .unwrap();
        fs::create_dir_all(path.join("skills/sample")).unwrap();
        fs::write(
            path.join("skills/sample/SKILL.md"),
            "---\nname: sample\ndescription: test\n---\nTest skill\n",
        )
        .unwrap();
    }

    #[test]
    fn index_rejects_traversal_repository_mismatch_and_duplicate_ids() {
        let source = Source::parse("public").unwrap();
        let mut manifest = index(entry());
        assert!(parse_index(&serde_json::to_vec(&manifest).unwrap(), source).is_ok());
        for path in [
            "plugins/../secrets",
            "plugins/../../auth.json",
            "plugins\\sample",
            "/plugins/sample",
            "plugins/[sample]",
        ] {
            manifest.plugins[0].path = path.into();
            assert!(
                parse_index(&serde_json::to_vec(&manifest).unwrap(), source).is_err(),
                "{path}"
            );
        }
        manifest.plugins[0] = entry();
        manifest.repository = "somebody/else".into();
        assert!(parse_index(&serde_json::to_vec(&manifest).unwrap(), source).is_err());
        manifest.repository = source.repository.into();
        manifest.plugins.push(entry());
        assert!(parse_index(&serde_json::to_vec(&manifest).unwrap(), source).is_err());
    }

    #[tokio::test]
    async fn listing_cached_index_does_not_create_or_download_packages() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = Source::parse("public").unwrap();
        fs::write(
            index_path(&home, source),
            serde_json::to_vec(&index(entry())).unwrap(),
        )
        .unwrap();
        let payload = list_plugins(&home, "public", false).await.unwrap();
        assert_eq!(payload["total"], 1);
        assert_eq!(payload["cached"], true);
        assert_eq!(payload["plugins"][0]["installed"], false);
        assert!(!home.join("plugins").exists());
        assert!(!market_root(&home).join("downloads").exists());
    }

    #[test]
    fn install_preserves_other_config_and_only_installs_selected_package() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let original = "# Keep this comment\nmodel = \"original-model\"\n\n[model_providers.relay]\nbase_url = \"https://example.test/v1\"\n\n[plugins.\"other@existing-market\"]\nenabled = false\n";
        fs::write(home.join("config.toml"), original).unwrap();
        let source = Source::parse("public").unwrap();
        let plugin = entry();
        let download = home.join(".tmp/downloaded");
        write_payload(&download, &plugin);
        let untouched = home.join(".tmp/other-plugin");
        fs::create_dir_all(&untouched).unwrap();
        fs::write(untouched.join("keep.txt"), "kept").unwrap();
        let payload = commit_install(&home, source, &plugin, Some(&download)).unwrap();
        assert_eq!(payload["plugin"]["installed"], true);
        assert!(
            package_root(&home, source, &plugin)
                .join("skills/sample/SKILL.md")
                .is_file()
        );
        assert!(untouched.join("keep.txt").is_file());
        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("# Keep this comment"));
        let document = config.parse::<DocumentMut>().unwrap();
        assert_eq!(document["model"].as_str(), Some("original-model"));
        assert_eq!(
            document["plugins"]["other@existing-market"]["enabled"].as_bool(),
            Some(false)
        );
        let plugin_key = format!("sample@{}", plugin_market_name(source, &plugin.id));
        assert_eq!(
            document["plugins"][&plugin_key]["enabled"].as_bool(),
            Some(true)
        );
        assert!(commit_install(&home, source, &plugin, None).is_ok());
        assert_eq!(read_state(&home).unwrap().plugins.len(), 1);
    }

    #[test]
    fn uninstall_after_install_restores_files_config_and_records() {
        // #2701：卸载要把安装写入的缓存、市场目录、配置条目与记录全部撤销，其他内容原样保留。
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let original = "# Keep this comment\nmodel = \"original-model\"\n\n[model_providers.relay]\nbase_url = \"https://example.test/v1\"\n\n[marketplaces.user-market]\nsource_type = \"local\"\nsource = \"/somewhere/user-market\"\n\n[plugins.\"other@existing-market\"]\nenabled = false\n";
        fs::write(home.join("config.toml"), original).unwrap();
        let source = Source::parse("public").unwrap();
        let plugin = entry();
        fs::write(
            index_path(&home, source),
            serde_json::to_vec(&index(plugin.clone())).unwrap(),
        )
        .unwrap();
        // 安装前已存在的其他插件缓存与市场目录不能被卸载波及。
        let other_cache = home.join("plugins/cache/existing-market/other/1.0.0");
        fs::create_dir_all(&other_cache).unwrap();
        fs::write(other_cache.join("keep.txt"), "kept").unwrap();
        let other_market = market_root(&home).join("markets/user-market");
        fs::create_dir_all(&other_market).unwrap();
        fs::write(other_market.join("keep.txt"), "kept").unwrap();

        let download = home.join(".tmp/downloaded");
        write_payload(&download, &plugin);
        commit_install(&home, source, &plugin, Some(&download)).unwrap();
        set_status(&home, source, &plugin.id, false, "complete", "done").unwrap();
        let market_name = plugin_market_name(source, &plugin.id);
        assert!(package_root(&home, source, &plugin).is_dir());
        assert!(plugin_market_root(&home, source, &plugin.id).is_dir());
        assert!(
            fs::read_to_string(home.join("config.toml"))
                .unwrap()
                .contains(&market_name)
        );

        let payload = uninstall_plugin(&home, "public", &plugin.id).unwrap();
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["plugin"]["installed"], false);
        assert_eq!(payload["plugin"]["installedVersion"], "");
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).unwrap(),
            original
        );
        assert!(!home.join("plugins/cache").join(&market_name).exists());
        assert!(!plugin_market_root(&home, source, &plugin.id).exists());
        assert!(read_state(&home).unwrap().plugins.is_empty());
        assert!(!status_path(&home, source, &plugin.id).exists());
        assert_eq!(
            install_status(&home, "public", &plugin.id).unwrap()["stage"],
            "failed"
        );
        assert!(other_cache.join("keep.txt").is_file());
        assert!(other_market.join("keep.txt").is_file());
        let listed = plugin_payload(&home, source, &plugin, &read_state(&home).unwrap());
        assert_eq!(listed["installed"], false);
        // 已卸载后再次卸载要明确报错，不碰任何文件。
        assert!(uninstall_plugin(&home, "public", &plugin.id).is_err());
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).unwrap(),
            original
        );
        // 卸载后可以重新安装。
        let download = home.join(".tmp/reinstall");
        write_payload(&download, &plugin);
        commit_install(&home, source, &plugin, Some(&download)).unwrap();
        assert!(package_root(&home, source, &plugin).is_dir());
    }

    #[test]
    fn uninstall_without_prior_config_drops_created_tables_and_keeps_other_variant() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = Source::parse("public").unwrap();
        let first = entry();
        let mut second = first.clone();
        second.id = "plugin_second".into();
        second.tree = "c".repeat(40);
        let first_download = home.join(".tmp/first");
        let second_download = home.join(".tmp/second");
        write_payload(&first_download, &first);
        write_payload(&second_download, &second);
        commit_install(&home, source, &first, Some(&first_download)).unwrap();
        commit_install(&home, source, &second, Some(&second_download)).unwrap();

        uninstall_plugin(&home, "public", &second.id).unwrap();
        assert!(package_root(&home, source, &first).is_dir());
        assert!(!package_root(&home, source, &second).exists());
        let document = fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert_eq!(document["marketplaces"].as_table().unwrap().len(), 1);
        assert_eq!(document["plugins"].as_table().unwrap().len(), 1);
        let first_key = format!("sample@{}", plugin_market_name(source, &first.id));
        assert_eq!(
            document["plugins"][&first_key]["enabled"].as_bool(),
            Some(true)
        );
        let state = read_state(&home).unwrap();
        assert_eq!(state.plugins.len(), 1);
        assert!(state.plugins.contains_key(&install_key(source, &first.id)));

        uninstall_plugin(&home, "public", &first.id).unwrap();
        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(!config.contains("marketplaces"));
        assert!(!config.contains("plugins"));
        assert!(
            fs::read_dir(home.join("plugins/cache"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    #[test]
    fn uninstall_refuses_unowned_paths_and_rejects_invalid_ids() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        assert!(uninstall_plugin(&home, "public", "../../auth.json").is_err());
        assert!(uninstall_plugin(&home, "elsewhere", "plugin_test").is_err());
        assert!(uninstall_plugin(&home, "public", "plugin_test").is_err());
        assert!(owned_market_name("codex-plus-plugin-market-0123456789ab"));
        assert!(!owned_market_name("codex-plus-plugin-market"));
        assert!(!owned_market_name(
            "codex-plus-plugin-market-0123456789ab-extra"
        ));
        assert!(!owned_market_name("existing-market"));
        let cache = home.join("plugins/cache");
        let foreign = cache.join("existing-market");
        fs::create_dir_all(&foreign).unwrap();
        assert!(remove_owned_directory(&home, &cache, &foreign).is_err());
        let nested = cache.join("codex-plus-plugin-market-0123456789ab");
        assert!(remove_owned_directory(&home, &home, &nested).is_err());
        assert!(foreign.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn uninstall_does_not_follow_symlinked_plugin_directories() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path().join("home").as_path()).unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "kept").unwrap();
        let source = Source::parse("public").unwrap();
        let plugin = entry();
        let download = home.join(".tmp/downloaded");
        write_payload(&download, &plugin);
        commit_install(&home, source, &plugin, Some(&download)).unwrap();
        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        // 安装后缓存目录被替换成指向外部的链接：卸载必须整体拒绝，不跟随删除，也不先改配置。
        let cache_root = home
            .join("plugins/cache")
            .join(plugin_market_name(source, &plugin.id));
        fs::rename(&cache_root, temp.path().join("moved")).unwrap();
        symlink(&outside, &cache_root).unwrap();
        assert!(uninstall_plugin(&home, "public", &plugin.id).is_err());
        assert!(outside.join("keep.txt").is_file());
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).unwrap(),
            config
        );
        assert_eq!(read_state(&home).unwrap().plugins.len(), 1);
        assert!(plugin_market_root(&home, source, &plugin.id).is_dir());
    }

    #[test]
    fn failed_payload_does_not_mark_plugin_installed_or_change_config() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        fs::write(home.join("config.toml"), "model = \"original\"\n").unwrap();
        let plugin = entry();
        let download = home.join(".tmp/downloaded");
        write_payload(&download, &plugin);
        fs::write(download.join(".env"), "SECRET=dummy").unwrap();
        assert!(
            commit_install(
                &home,
                Source::parse("public").unwrap(),
                &plugin,
                Some(&download)
            )
            .is_err()
        );
        let source = Source::parse("public").unwrap();
        assert_eq!(
            plugin_payload(&home, source, &plugin, &read_state(&home).unwrap())["installed"],
            false
        );
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).unwrap(),
            "model = \"original\"\n"
        );
        assert!(!package_root(&home, source, &plugin).exists());
    }

    #[test]
    fn invalid_toml_error_does_not_echo_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        fs::write(
            home.join("config.toml"),
            "api_key = \"private-test-value\"\nbroken = [\n",
        )
        .unwrap();
        let error = updated_config(&home, Source::parse("public").unwrap(), &entry())
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private-test-value"));
    }

    #[cfg(unix)]
    #[test]
    fn payload_rejects_symlinks_and_preserves_executable_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let plugin = entry();
        let download = home.join(".tmp/downloaded");
        write_payload(&download, &plugin);
        symlink("/tmp", download.join("outside")).unwrap();
        assert!(validate_payload(&download, &plugin).is_err());
        // 链接包与可执行包分别使用独立下载目录，不删除用户文件。
        let executable = home.join(".tmp/executable");
        write_payload(&executable, &plugin);
        fs::write(executable.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(executable.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        commit_install(
            &home,
            Source::parse("public").unwrap(),
            &plugin,
            Some(&executable),
        )
        .unwrap();
        assert_eq!(
            fs::metadata(
                package_root(&home, Source::parse("public").unwrap(), &plugin).join("run.sh")
            )
            .unwrap()
            .permissions()
            .mode()
                & 0o777,
            0o755
        );
    }

    #[test]
    fn simultaneous_installs_are_locked_and_stale_status_is_detected() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = Source::parse("public").unwrap();
        let lock = acquire_install_lock(&home).unwrap();
        assert!(acquire_install_lock(&home).is_err());
        set_status(
            &home,
            source,
            "plugin_test",
            true,
            "fetching",
            "downloading",
        )
        .unwrap();
        assert_eq!(
            install_status(&home, "public", "plugin_test").unwrap()["busy"],
            true
        );
        drop(lock);
        let status = install_status(&home, "public", "plugin_test").unwrap();
        assert_eq!(status["busy"], false);
        assert_eq!(status["stage"], "failed");
    }

    #[test]
    fn same_name_variants_keep_separate_marketplaces_and_install_records() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = Source::parse("public").unwrap();
        let first = entry();
        let mut second = first.clone();
        second.id = "plugin_second".into();
        second.tree = "c".repeat(40);
        let first_download = home.join(".tmp/first");
        let second_download = home.join(".tmp/second");
        write_payload(&first_download, &first);
        write_payload(&second_download, &second);
        commit_install(&home, source, &first, Some(&first_download)).unwrap();
        commit_install(&home, source, &second, Some(&second_download)).unwrap();
        assert_ne!(
            package_root(&home, source, &first),
            package_root(&home, source, &second)
        );
        assert_eq!(read_state(&home).unwrap().plugins.len(), 2);
        let document = fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert_eq!(document["plugins"].as_table().unwrap().len(), 2);
        assert_eq!(document["marketplaces"].as_table().unwrap().len(), 2);
    }

    #[test]
    fn enabling_a_plugin_preserves_mcp_servers_and_other_user_tables() {
        // #353：切换插件启用状态时只应增改 marketplaces/plugins，用户手写的
        // mcp_servers、skills 等表必须原样保留。
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = Source::parse("public").unwrap();
        let plugin = entry();
        let original = r#"model = "qwen3-coder"

[mcp_servers.memory]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-memory"]

[skills.custom]
enabled = true
"#;
        fs::write(home.join("config.toml"), original).unwrap();

        let (_, updated) = updated_config(&home, source, &plugin).unwrap();
        let document = updated.parse::<DocumentMut>().unwrap();
        assert_eq!(
            document["mcp_servers"]["memory"]["command"].as_str(),
            Some("npx")
        );
        assert_eq!(
            document["mcp_servers"]["memory"]["args"]
                .as_array()
                .map(|values| values.len()),
            Some(2)
        );
        assert_eq!(document["skills"]["custom"]["enabled"].as_bool(), Some(true));
        assert_eq!(document["model"].as_str(), Some("qwen3-coder"));
    }

    #[test]
    fn changed_config_is_not_overwritten_and_ready_cache_can_resume() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = Source::parse("public").unwrap();
        let plugin = entry();
        fs::write(home.join("config.toml"), "model = \"first\"\n").unwrap();
        let (original, updated) = updated_config(&home, source, &plugin).unwrap();
        fs::write(home.join("config.toml"), "model = \"newer\"\n").unwrap();
        assert!(write_config_if_unchanged(&home, original.as_deref(), &updated).is_err());
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).unwrap(),
            "model = \"newer\"\n"
        );
        let download = home.join(".tmp/downloaded");
        write_payload(&download, &plugin);
        commit_install(&home, source, &plugin, Some(&download)).unwrap();
        // 模拟已完成缓存/配置但尚未提交安装记录，可从 ready receipt 恢复。
        write_json(&state_path(&home), &InstalledState::default()).unwrap();
        assert!(commit_install(&home, source, &plugin, None).is_ok());
        assert_eq!(read_state(&home).unwrap().plugins.len(), 1);
    }

    #[test]
    fn git_tree_rejects_symlinks_submodules_and_credential_paths_before_checkout() {
        let hash = "a".repeat(40);
        assert!(
            validate_git_files(format!("100755 blob {hash}\tscripts/run.sh\0").as_bytes()).is_ok()
        );
        for record in [
            format!("120000 blob {hash}\tout\0"),
            format!("160000 commit {hash}\tsubmodule\0"),
            format!("100644 blob {hash}\t.env\0"),
            format!("100644 blob {hash}\t../auth.json\0"),
        ] {
            assert!(validate_git_files(record.as_bytes()).is_err());
        }
    }

    #[tokio::test]
    async fn native_read_uses_index_without_materializing_uninstalled_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = Source::parse("public").unwrap();
        let plugin = entry();
        fs::write(
            index_path(&home, source),
            serde_json::to_vec(&index(plugin)).unwrap(),
        )
        .unwrap();
        let params = json!({ "marketplacePath": virtual_market_path(&home, source), "pluginName": "sample" });
        let response = native_request(&home, "plugin/read", params).await.unwrap();
        assert_eq!(response["plugin"]["summary"]["installed"], false);
        assert_eq!(response["plugin"]["summary"]["authPolicy"], "ON_USE");
        assert_eq!(response["plugin"]["skills"], json!([]));
        assert!(!home.join("plugins").exists());
        assert!(!virtual_market_path(&home, source).exists());
        assert!(response["plugin"]["summary"]["remotePluginId"].is_null());
    }

    #[tokio::test]
    async fn native_installed_is_local_only_and_uses_real_ids_for_enabled_state() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = Source::parse("public").unwrap();
        // 空白 CODEX_HOME 的 installed 查询应立即返回，不产生索引下载。
        let empty = native_request(&home, "plugin/installed", json!({}))
            .await
            .unwrap();
        assert_eq!(empty["marketplaces"], json!([]));
        assert!(!index_path(&home, source).exists());
        let plugin = entry();
        let download = home.join(".tmp/downloaded");
        write_payload(&download, &plugin);
        commit_install(&home, source, &plugin, Some(&download)).unwrap();
        let response = native_request(&home, "plugin/installed", json!({}))
            .await
            .unwrap();
        let summary = &response["marketplaces"][0]["plugins"][0];
        assert_eq!(
            summary["id"],
            format!("sample@{}", plugin_market_name(source, &plugin.id))
        );
        assert_eq!(summary["codexPlusIndexId"], "plugin_test");
        assert_eq!(summary["installed"], true);
        assert_eq!(summary["source"]["type"], "local");
        assert!(!index_path(&home, source).exists());
        let mut document = fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        document["plugins"][summary["id"].as_str().unwrap()]["enabled"] = toml_edit::value(false);
        fs::write(home.join("config.toml"), document.to_string()).unwrap();
        let response = native_request(&home, "plugin/installed", json!({}))
            .await
            .unwrap();
        assert_eq!(response["marketplaces"][0]["plugins"][0]["enabled"], false);
    }

    #[test]
    fn native_variants_and_sources_are_unambiguous() {
        let public = Source::parse("public").unwrap();
        let full = Source::parse("full").unwrap();
        let first = entry();
        let mut second = first.clone();
        second.id = "plugin_second".into();
        let mut manifest = index(first.clone());
        manifest.plugins.push(second.clone());
        let aliases = native_aliases(&manifest);
        assert_ne!(aliases.get(&first.id), aliases.get(&second.id));
        assert_ne!(
            plugin_market_name(public, &first.id),
            plugin_market_name(full, &first.id)
        );
        for entry in [&first, &second] {
            let params = json!({ "pluginName": aliases.get(&entry.id).unwrap() });
            assert_eq!(
                managed_entry(&manifest, public, &params).unwrap().0.id,
                entry.id
            );
        }
    }

    #[test]
    fn install_status_is_kept_separately_for_each_plugin() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = Source::parse("public").unwrap();
        set_status(&home, source, "plugin_first", false, "complete", "done").unwrap();
        let _lock = acquire_install_lock(&home).unwrap();
        set_status(
            &home,
            source,
            "plugin_second",
            true,
            "fetching",
            "downloading",
        )
        .unwrap();
        assert_eq!(
            install_status(&home, "public", "plugin_first").unwrap()["stage"],
            "complete"
        );
        assert_eq!(
            install_status(&home, "public", "plugin_second").unwrap()["busy"],
            true
        );
        assert_eq!(
            install_status(&home, "public", "plugin_missing").unwrap()["stage"],
            "failed"
        );
    }

    #[tokio::test]
    async fn opaque_remote_ids_with_tilde_are_supported_without_relaxing_paths() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = Source::parse("public").unwrap();
        let mut plugin = entry();
        plugin.id = "plugins~Plugin_f1b845ac33888191ac156169c58733c2".into();
        let manifest = index(plugin.clone());
        let bytes = serde_json::to_vec(&manifest).unwrap();
        assert!(parse_index(&bytes, source).is_ok());
        fs::write(index_path(&home, source), &bytes).unwrap();
        let result = list_plugins(&home, "public", false).await.unwrap();
        assert_eq!(result["plugins"][0]["id"], plugin.id);
        let download = home.join(".tmp/opaque");
        write_payload(&download, &plugin);
        commit_install(&home, source, &plugin, Some(&download)).unwrap();
        assert!(
            read_state(&home)
                .unwrap()
                .plugins
                .contains_key(&install_key(source, &plugin.id))
        );
        set_status(&home, source, &plugin.id, false, "complete", "done").unwrap();
        assert_eq!(
            install_status(&home, "public", &plugin.id).unwrap()["stage"],
            "complete"
        );
        assert!(!valid_component(&plugin.id, false));
        assert!(!valid_id("../../auth.json"));
    }

    #[tokio::test]
    async fn native_list_reads_only_the_full_index() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let public = Source::parse("public").unwrap();
        let full = Source::parse("full").unwrap();
        fs::write(
            index_path(&home, public),
            serde_json::to_vec(&index(entry())).unwrap(),
        )
        .unwrap();
        let mut full_index = index(entry());
        full_index.repository = full.repository.into();
        full_index.plugins[0].path = "markets/01/plugins/sample".into();
        let mut extra = entry();
        extra.id = "plugin_full_only".into();
        extra.name = "full-only".into();
        extra.path = "markets/02/plugins/full-only".into();
        full_index.plugins.push(extra);
        fs::write(
            index_path(&home, full),
            serde_json::to_vec(&full_index).unwrap(),
        )
        .unwrap();
        let payload = native_request(&home, "plugin/list", json!({}))
            .await
            .unwrap();
        assert_eq!(payload["marketplaces"].as_array().unwrap().len(), 1);
        assert_eq!(payload["marketplaces"][0]["name"], "codex-plus-index-full");
        assert_eq!(
            payload["marketplaces"][0]["plugins"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(payload["marketplaceLoadErrors"], json!([]));
        assert!(!home.join("plugins").exists());
        assert!(!market_root(&home).join("downloads").exists());
    }

    fn sign_in(home: &Path) {
        fs::write(
            home.join("auth.json"),
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"token"}}"#,
        )
        .unwrap();
    }

    /// 两个来源的索引都写进缓存，列表请求就不会去联网。
    fn cache_indexes(home: &Path, plugins: Vec<PluginEntry>) -> Source {
        let full = Source::parse("full").unwrap();
        fs::write(
            index_path(home, Source::parse("public").unwrap()),
            serde_json::to_vec(&index(entry())).unwrap(),
        )
        .unwrap();
        let mut manifest = index(entry());
        manifest.repository = full.repository.into();
        manifest.plugins = plugins;
        for plugin in &mut manifest.plugins {
            plugin.path = format!("markets/01/plugins/{}", plugin.name);
        }
        fs::write(
            index_path(home, full),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        full
    }

    #[tokio::test]
    async fn native_list_leaves_the_catalog_to_codex_when_chatgpt_is_signed_in() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = cache_indexes(&home, vec![entry()]);
        let before = native_request(&home, "plugin/list", json!({}))
            .await
            .unwrap();
        assert_eq!(before["marketplaces"].as_array().unwrap().len(), 1);

        sign_in(&home);
        let after = native_request(&home, "plugin/list", json!({}))
            .await
            .unwrap();
        assert_eq!(
            after,
            json!({ "marketplaces": [], "marketplaceLoadErrors": [], "featuredPluginIds": [] })
        );
        // 已经装上的仍能打开详情。
        let params = json!({ "marketplacePath": virtual_market_path(&home, source), "pluginName": "sample" });
        let detail = native_request(&home, "plugin/read", params).await.unwrap();
        assert_eq!(detail["plugin"]["summary"]["name"], "sample");
    }

    #[tokio::test]
    async fn native_page_shows_snapshot_date_and_how_connectors_connect() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let mut hosted = entry();
        hosted.requires_auth = true;
        hosted.has_apps = true;
        let mut local = entry();
        local.id = "plugin_local".into();
        local.name = "local".into();
        local.requires_auth = true;
        local.has_apps = true;
        local.has_mcp = true;
        let mut plain = entry();
        plain.id = "plugin_plain".into();
        plain.name = "plain".into();
        let source = cache_indexes(&home, vec![hosted, local, plain]);
        let payload = native_request(&home, "plugin/list", json!({}))
            .await
            .unwrap();
        let market = &payload["marketplaces"][0];
        let interface = |name: &str| {
            market["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .find(|plugin| plugin["name"] == name)
                .unwrap()["interface"]
                .clone()
        };
        assert_eq!(
            interface("sample")["shortDescription"],
            "需 ChatGPT 登录 · sample plugin"
        );
        assert_eq!(interface("local")["shortDescription"], "sample plugin");
        // 完整描述和连接器说明在详情里，列表不带。
        assert!(interface("sample")["longDescription"].is_null());
        for (name, start) in [("sample", "连接器由 ChatGPT 托管"), ("local", "连接器是 MCP 服务")] {
            let params = json!({ "marketplacePath": virtual_market_path(&home, source), "pluginName": name });
            let detail = native_request(&home, "plugin/read", params).await.unwrap();
            let long = detail["plugin"]["summary"]["interface"]["longDescription"].as_str().unwrap();
            assert!(long.starts_with(start), "{long}");
            assert!(long.contains("\n\nsample plugin\n\n"), "{long}");
        }

        // 快照日期写在详情里（市场名要留给页签判定，放不了日期）。
        let params = json!({ "marketplacePath": virtual_market_path(&home, source), "pluginName": "plain" });
        let detail = native_request(&home, "plugin/read", params).await.unwrap();
        assert_eq!(
            detail["plugin"]["summary"]["interface"]["longDescription"],
            "sample plugin\n\n来自 Codex++ 缓存的官方目录快照（2026-10-08），不是实时目录。"
        );

        // ChatGPT 登录后托管连接器能连，详情里不再提示。
        sign_in(&home);
        let params = json!({ "marketplacePath": virtual_market_path(&home, source), "pluginName": "sample" });
        let detail = native_request(&home, "plugin/read", params).await.unwrap();
        assert_eq!(
            detail["plugin"]["summary"]["interface"]["shortDescription"],
            "sample plugin"
        );
    }

    #[test]
    fn snapshot_date_is_shown_only_when_it_is_a_date() {
        assert_eq!(snapshot_date("2026-10-08"), Some("2026-10-08"));
        assert_eq!(snapshot_date("2026-10-08T08:29:33Z"), Some("2026-10-08"));
        for value in ["", "2026-10", "<script>xx", "2026/10/08", "二〇二六年十月八日"] {
            assert_eq!(snapshot_date(value), None, "{value}");
        }
    }

    #[test]
    fn index_connector_fields_are_optional() {
        let source = Source::parse("public").unwrap();
        let mut value = serde_json::to_value(index(entry())).unwrap();
        let plugin = value["plugins"][0].as_object_mut().unwrap();
        plugin.remove("hasApps");
        plugin.remove("hasMcp");
        plugin.remove("category");
        let old = parse_index(&serde_json::to_vec(&value).unwrap(), source).unwrap();
        assert!(!old.plugins[0].has_apps && !old.plugins[0].has_mcp);
        assert_eq!(old.plugins[0].category, "");
        value["plugins"][0]["hasApps"] = json!(true);
        value["plugins"][0]["hasMcp"] = json!(true);
        let new = parse_index(&serde_json::to_vec(&value).unwrap(), source).unwrap();
        assert!(new.plugins[0].has_apps && new.plugins[0].has_mcp);
    }

    #[test]
    fn install_result_tells_which_connectors_the_package_declares() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let source = Source::parse("public").unwrap();
        let install = |id: &str, name: &str, files: &[(&str, &str)]| {
            let mut plugin = entry();
            plugin.id = id.into();
            plugin.name = name.into();
            let download = home.join(".tmp").join(name);
            write_payload(&download, &plugin);
            for (file, contents) in files {
                fs::write(download.join(file), contents).unwrap();
            }
            commit_install(&home, source, &plugin, Some(&download)).unwrap();
            set_status(&home, source, id, false, "complete", "done").unwrap();
            install_status(&home, "public", id).unwrap()["nativeResult"].clone()
        };
        let apps = r#"{"apps":{"demo":{"id":"connector_demo"}}}"#;
        let mcp = r#"{"mcpServers":{"demo":{"type":"http","url":"https://mcp.example.com/mcp"}}}"#;

        let both = install("plugin_both", "both", &[(".app.json", apps), (".mcp.json", mcp)]);
        assert_eq!(both["authPolicy"], "ON_USE");
        assert_eq!(both["appsNeedingAuth"], json!([]));
        assert_eq!(both["codexPlusMcpServers"], json!(["demo"]));
        assert_eq!(both["codexPlusHostedOnly"], false);

        let hosted = install("plugin_hosted", "hosted", &[(".app.json", apps)]);
        assert_eq!(hosted["codexPlusMcpServers"], json!([]));
        assert_eq!(hosted["codexPlusHostedOnly"], true);

        let plain = install("plugin_plain", "plain", &[]);
        assert_eq!(plain["codexPlusMcpServers"], json!([]));
        assert_eq!(plain["codexPlusHostedOnly"], false);

        // 旧索引只标了「带连接器」；装好以后详情页按包里的声明说清楚是哪一种。
        let note = |id: &str, name: &str| {
            let mut plugin = entry();
            plugin.id = id.into();
            plugin.name = name.into();
            plugin.requires_auth = true;
            let detail = native_detail(
                &home,
                source,
                &index(plugin.clone()),
                &plugin,
                name,
                &read_state(&home).unwrap(),
                &configured_plugin_enabled(&home).unwrap(),
            );
            let summary = &detail["plugin"]["summary"];
            assert_eq!(summary["installed"], true, "{name}");
            summary["interface"]["longDescription"].as_str().unwrap().to_owned()
        };
        assert!(note("plugin_both", "both").starts_with("连接器是 MCP 服务"));
        assert!(note("plugin_hosted", "hosted").starts_with("连接器由 ChatGPT 托管"));
        assert!(!note("plugin_plain", "plain").contains("连接器"));

        // 有 ChatGPT 登录时托管连接器能连，不用提示。
        sign_in(&home);
        let signed_in = install_status(&home, "public", "plugin_hosted").unwrap();
        assert_eq!(signed_in["nativeResult"]["codexPlusHostedOnly"], false);
    }

    #[test]
    fn native_list_reports_why_the_index_is_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let full = Source::parse("full").unwrap();
        let payload = native_list_payload(
            &home,
            full,
            Err(anyhow::anyhow!("无法获取插件市场索引，请检查网络或稍后重试")),
            &InstalledState::default(),
            &BTreeMap::new(),
        );
        // 旧公开库不再用来兜底。
        assert_eq!(payload["marketplaces"], json!([]));
        assert_eq!(
            payload["marketplaceLoadErrors"],
            json!([{
                "marketplacePath": virtual_market_path(&home, full).to_string_lossy(),
                "message": "无法获取插件市场索引，请检查网络或稍后重试",
            }])
        );
    }
}
