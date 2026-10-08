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
const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PACKAGE_FILES: u64 = 100_000;
const INDEX_TIMEOUT: Duration = Duration::from_secs(35);
const GIT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Copy)]
struct Source {
    id: &'static str,
    repository: &'static str,
    private: bool,
}

impl Source {
    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "public" => Ok(Self {
                id: "public",
                repository: "BigPizzaV3/CodexPlusPlusPluginCache",
                private: false,
            }),
            "full" => Ok(Self {
                id: "full",
                repository: "BigPizzaV3/CodexPlusPlusFullPluginCache",
                private: true,
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
    home.join("plugins")
        .join("cache")
        .join(plugin_market_name(source, &plugin.id))
        .join(&plugin.name)
        .join(&plugin.version)
}

fn state_path(home: &Path) -> PathBuf {
    market_root(home).join("installed.json")
}

fn index_path(home: &Path, source: Source) -> PathBuf {
    market_root(home).join(format!("index-{}.json", source.id))
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
        bail!("插件索引超过 8 MiB 限制");
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
    let url = format!(
        "https://raw.githubusercontent.com/{}/main/plugin-index.json",
        source.repository
    );
    let client = reqwest::Client::builder()
        .user_agent("CodexPlusPlus/plugin-market")
        .timeout(INDEX_TIMEOUT)
        .build()
        .context("无法建立插件市场连接")?;
    match client.get(&url).send().await {
        Ok(mut response) if response.status().is_success() => {
            if response.content_length().unwrap_or_default() > MAX_INDEX_BYTES as u64 {
                bail!("插件索引超过 8 MiB 限制");
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.context("无法读取插件索引")? {
                if bytes.len() + chunk.len() > MAX_INDEX_BYTES {
                    bail!("插件索引超过 8 MiB 限制");
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(bytes);
        }
        _ if source.private => {}
        _ => bail!("无法获取公开插件市场索引，请检查网络或稍后重试"),
    }
    let endpoint = format!(
        "repos/{}/contents/plugin-index.json?ref=main",
        source.repository
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
    Ok((index, false))
}

fn plugin_payload(
    home: &Path,
    source: Source,
    plugin: &PluginEntry,
    state: &InstalledState,
) -> Value {
    let local = state
        .plugins
        .get(&install_key(source, &plugin.id))
        .filter(|installed| {
            let mut entry = plugin.clone();
            entry.version.clone_from(&installed.version);
            fs::symlink_metadata(package_root(home, source, &entry))
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        });
    let mut result = serde_json::to_value(plugin).unwrap_or_else(|_| json!({}));
    result["installed"] = json!(local.is_some());
    result["marketplaceName"] = json!(plugin_market_name(source, &plugin.id));
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
                value["nativeResult"] = native_install_result();
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

fn git_command(work: &Path, token: Option<&str>) -> Command {
    let mut command = Command::new("git");
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
    if let Some(token) = token {
        // 凭据只传给本次 Git 子进程，不写 Git 配置，也不进入命令行或日志。
        let credentials =
            base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
            .env(
                "GIT_CONFIG_VALUE_0",
                format!("AUTHORIZATION: basic {credentials}"),
            );
    } else {
        command.env("GIT_CONFIG_COUNT", "0");
    }
    command
}

async fn run_git(work: &Path, token: Option<&str>, args: &[&str]) -> anyhow::Result<Vec<u8>> {
    command_output(
        git_command(work, token).args(args),
        64 * 1024,
        GIT_TIMEOUT,
        "Git 下载失败，请检查网络与仓库访问权限（完整市场需要 gh auth login）",
    )
    .await
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
    run_git(&work, token, &["init", "--quiet"]).await?;
    let url = format!("https://github.com/{}.git", source.repository);
    run_git(&work, token, &["remote", "add", "origin", &url]).await?;
    // 先取固定 revision 的树元数据，blob:none 保证没有整库插件文件下载。
    run_git(
        &work,
        token,
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
    let tree = run_git(&work, token, &["rev-parse", &tree_spec]).await?;
    if !String::from_utf8_lossy(&tree)
        .trim()
        .eq_ignore_ascii_case(&plugin.tree)
    {
        bail!("插件目录校验失败，仓库内容与索引不一致");
    }
    let files = command_output(
        git_command(&work, token).args(["ls-tree", "-r", "-z", &plugin.tree]),
        16 * 1024 * 1024,
        GIT_TIMEOUT,
        "无法读取插件 Git 文件清单",
    )
    .await?;
    validate_git_files(&files)?;
    run_git(&work, token, &["sparse-checkout", "init", "--no-cone"]).await?;
    // 非 cone 模式避免顺便检出根目录和祖先目录的其他文件。
    let pattern = format!("/{}/", plugin.path);
    run_git(
        &work,
        token,
        &["sparse-checkout", "set", "--no-cone", "--", &pattern],
    )
    .await?;
    run_git(
        &work,
        token,
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
        "category": "Productivity",
    })];
    json!({ "name": plugin_market_name(source, &plugin.id), "interface": { "displayName": format!("CodeX 插件市场 · {}", plugin.display_name) }, "plugins": plugins })
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
        if existing
            || (package_root(&home, source, plugin).is_dir()
                && receipt_matches(&home, source, plugin))
        {
            return commit_install(&home, source, plugin, None);
        }
        // 下载之前先验证当前配置，避免无意义下载或覆盖不可解析配置。
        updated_config(&home, source, plugin)?;
        set_status(&home, source, id, true, "fetching", "正在下载所选插件…")?;
        let downloaded = download_package(&home, source, &index, plugin).await?;
        set_status(&home, source, id, true, "fetching", "正在校验并启用插件…")?;
        commit_install(&home, source, plugin, Some(&downloaded))
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

fn native_install_result() -> Value {
    json!({ "appsNeedingAuth": [], "authPolicy": "ON_USE" })
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

fn native_summary(
    home: &Path,
    source: Source,
    revision: Option<&str>,
    plugin: &PluginEntry,
    alias: &str,
    state: &InstalledState,
    enabled: &BTreeMap<String, bool>,
) -> Value {
    let payload = plugin_payload(home, source, plugin, state);
    let installed = payload["installed"].as_bool().unwrap_or(false);
    let id = format!("{}@{}", plugin.name, plugin_market_name(source, &plugin.id));
    let local_version = payload["installedVersion"]
        .as_str()
        .filter(|value| !value.is_empty());
    let source_value = if let Some(revision) = revision {
        json!({ "type": "git", "url": format!("https://github.com/{}.git", source.repository), "path": plugin.path, "sha": revision, "refName": null })
    } else {
        let mut local = plugin.clone();
        if let Some(version) = local_version {
            local.version = version.to_owned();
        }
        json!({ "type": "local", "path": package_root(home, source, &local).to_string_lossy() })
    };
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
            "shortDescription": plugin.description, "longDescription": plugin.description,
            "developerName": plugin.author, "category": "Productivity", "capabilities": [],
            "screenshots": [], "screenshotUrls": [],
            "composerIcon": null, "logo": null, "logoDark": null,
            "composerIconUrl": null, "logoUrl": null, "logoUrlDark": null,
            "brandColor": null, "defaultPrompt": null, "privacyPolicyUrl": null,
            "termsOfServiceUrl": null, "websiteUrl": null,
        },
    })
}

fn native_market(home: &Path, source: Source, plugins: Vec<Value>) -> Value {
    json!({
        "name": virtual_market_name(source), "path": virtual_market_path(home, source).to_string_lossy(),
        "interface": { "displayName": if source.private { "CodeX 插件市场 · 完整库" } else { "CodeX 插件市场 · 公开库" } },
        "plugins": plugins,
    })
}

fn native_list_payload(
    home: &Path,
    results: &[(Source, anyhow::Result<(PluginIndex, bool)>)],
    state: &InstalledState,
    enabled: &BTreeMap<String, bool>,
) -> Value {
    // 公开库是完整库的子集；完整库可访问时只显示一次，已安装查询仍保留两来源。
    let full_available = results
        .iter()
        .any(|(source, result)| source.id == "full" && result.is_ok());
    let mut marketplaces = Vec::new();
    let mut errors = Vec::new();
    for (source, result) in results {
        if full_available && source.id != "full" {
            continue;
        }
        match result {
            Ok((index, _)) => {
                let aliases = native_aliases(index);
                let plugins = index.plugins.iter().map(|plugin| native_summary(home, *source, Some(&index.revision), plugin, aliases.get(&plugin.id).map(String::as_str).unwrap_or(&plugin.name), state, enabled)).collect();
                marketplaces.push(native_market(home, *source, plugins));
            }
            Err(error) => errors.push(json!({ "marketplacePath": virtual_market_path(home, *source).to_string_lossy(), "message": error.to_string() })),
        }
    }
    json!({ "marketplaces": marketplaces, "marketplaceLoadErrors": errors, "featuredPluginIds": [] })
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
    bail!("该请求不属于 CodeX 插件市场")
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
    if summary["installed"].as_bool().unwrap_or(false) {
        let mut local = plugin.clone();
        if let Some(version) = summary["localVersion"].as_str() {
            local.version = version.to_owned();
        }
        let directory = package_root(home, source, &local);
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
            let mcp_path = manifest
                .get("mcpServers")
                .and_then(Value::as_str)
                .unwrap_or(".mcp.json");
            if let Some(path) = safe_local_path(&directory, mcp_path) {
                if fs::metadata(&path).is_ok_and(|metadata| metadata.len() <= 1024 * 1024) {
                    if let Ok(bytes) = fs::read(path) {
                        if let Ok(config) = serde_json::from_slice::<Value>(&bytes) {
                            if let Some(servers) =
                                config.get("mcpServers").and_then(Value::as_object)
                            {
                                mcp_servers = servers.keys().cloned().collect();
                            }
                        }
                    }
                }
            }
        }
    }
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
    }
    plugin
}

/// 原生 UI 形状适配。虚拟市场的 read/install 由注入层独占拦截，不传给原生下载器。
pub async fn native_request(home: &Path, method: &str, params: Value) -> anyhow::Result<Value> {
    let home = prepare_home(home)?;
    let state = read_state(&home)?;
    let enabled = configured_plugin_enabled(&home)?;
    match method {
        "plugin/list" | "list-plugins" => {
            let public = Source::parse("public")?;
            let full = Source::parse("full")?;
            let refresh = params
                .get("forceRefetch")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let (public_result, full_result) = tokio::join!(
                load_index(&home, public, refresh),
                load_index(&home, full, refresh)
            );
            Ok(native_list_payload(
                &home,
                &[(public, public_result), (full, full_result)],
                &state,
                &enabled,
            ))
        }
        "plugin/installed"
        | "plugin/installed-list"
        | "list-installed-plugins"
        | "installed-plugins" => {
            // 提及、聊天初始化等高频路径只读本地已安装记录，不拉市场目录。
            let mut marketplaces = Vec::new();
            for name in ["public", "full"] {
                let source = Source::parse(name)?;
                let index = cached_index(&home, source);
                let aliases = index.as_ref().map(native_aliases).unwrap_or_default();
                let mut plugins = Vec::new();
                for installed in state
                    .plugins
                    .values()
                    .filter(|installed| installed.source == source.id)
                {
                    let fallback;
                    let entry = if let Some(entry) = index.as_ref().and_then(|index| {
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
                Ok(native_install_result())
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
        }
    }

    fn index(plugin: PluginEntry) -> PluginIndex {
        PluginIndex {
            schema_version: 1,
            repository: Source::parse("public").unwrap().repository.into(),
            revision: "b".repeat(40),
            updated_at: "2026-10-08".into(),
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
    async fn native_list_prefers_cached_full_index_without_duplicate_public_entries() {
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

    #[test]
    fn native_list_falls_back_to_public_with_explicit_full_access_error() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(temp.path()).unwrap();
        let public = Source::parse("public").unwrap();
        let full = Source::parse("full").unwrap();
        let results = [
            (public, Ok((index(entry()), true))),
            (
                full,
                Err(anyhow::anyhow!("完整市场需要 GitHub 仓库访问权限")),
            ),
        ];
        let payload = native_list_payload(
            &home,
            &results,
            &InstalledState::default(),
            &BTreeMap::new(),
        );
        assert_eq!(payload["marketplaces"].as_array().unwrap().len(), 1);
        assert_eq!(
            payload["marketplaces"][0]["name"],
            "codex-plus-index-public"
        );
        assert_eq!(
            payload["marketplaceLoadErrors"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            payload["marketplaceLoadErrors"][0]["marketplacePath"],
            virtual_market_path(&home, full).to_string_lossy().as_ref()
        );
    }
}
