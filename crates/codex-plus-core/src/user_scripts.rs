use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::script_market::MarketScript;

pub const BOOTSTRAP_SCRIPT: &str = include_str!("../../../assets/inject/user-scripts-bootstrap.js");
const RUNTIME_SCRIPT: &str = include_str!("../../../assets/inject/user-scripts-runtime.js");

/// 拓展 API 契约版本，必须与 `91-extension-api.js` 的 `codexPlusExtensionApiVersion` 一致。
/// 拓展包在 `codexpp.json` 的 `engines.codexpp` 里声明需要的版本，高于此值的不加载。
pub const EXTENSION_API_VERSION: u64 = 1;

/// 拓展包的清单文件名。目录里有它，目录才被当成拓展包。
pub const EXTENSION_MANIFEST_FILE: &str = "codexpp.json";

/// 一次性连接当前 Codex 页面；不增加轮询，也不接管 launcher 的桥接连接。
pub async fn reload_live_scripts(
    debug_port: u16,
    manager: &UserScriptManager,
) -> anyhow::Result<Value> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let target = live_runtime_target(&targets)?;
    let websocket = target
        .web_socket_debugger_url
        .as_deref()
        .context("Codex renderer has no WebSocket URL")?;
    reload_scripts_at(websocket, manager).await
}

pub async fn reload_scripts_at(
    websocket: &str,
    manager: &UserScriptManager,
) -> anyhow::Result<Value> {
    // 拓展重载后旧后台的方法表、配置都过期了：先全部停掉，下次调用按新代码重起。
    crate::extension_backend::stop_all().await;
    apply_scripts_at(websocket, manager, &manager.build_reload_bundle()?).await
}

pub async fn load_scripts_at(
    websocket: &str,
    manager: &UserScriptManager,
) -> anyhow::Result<Value> {
    // 页面重新加载：上一个页面的后台不再有人用。
    crate::extension_backend::stop_all().await;
    apply_scripts_at(websocket, manager, &manager.build_initial_bundle()?).await
}

async fn apply_scripts_at(
    websocket: &str,
    manager: &UserScriptManager,
    bundle: &str,
) -> anyhow::Result<Value> {
    let response = crate::bridge::evaluate_script(websocket, bundle).await?;
    if let Some(exception) = response.pointer("/result/exceptionDetails") {
        anyhow::bail!(
            "脚本重载失败：{}",
            exception
                .pointer("/exception/description")
                .and_then(Value::as_str)
                .unwrap_or("renderer exception")
        );
    }
    let result = parse_live_runtime_status_response(&response)?;
    let mut inventory = manager.inventory_with_runtime_status(result.get("scripts"))?;
    inventory["reload_mode"] = result["mode"].clone();
    Ok(inventory)
}

/// Query the live renderer-side user-script registry through a one-shot CDP
/// connection. This is intentionally read-only and does not touch the
/// launcher-owned bridge websocket.
pub async fn live_runtime_status(debug_port: u16) -> anyhow::Result<Value> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let target = live_runtime_target(&targets)?;
    let websocket = target
        .web_socket_debugger_url
        .as_deref()
        .context("Codex renderer has no WebSocket URL")?;
    let response = crate::bridge::evaluate_script(
        websocket,
        "(() => JSON.stringify(window.__codexPlusUserScripts?.scripts || {}))()",
    )
    .await?;
    parse_live_runtime_status_response(&response)
}

fn live_runtime_target(targets: &[crate::cdp::CdpTarget]) -> anyhow::Result<crate::cdp::CdpTarget> {
    crate::cdp::pick_injectable_codex_page_target(targets)
}

fn parse_live_runtime_status_response(response: &Value) -> anyhow::Result<Value> {
    let encoded = response
        .pointer("/result/result/value")
        .and_then(Value::as_str)
        .context("user-script runtime probe returned no value")?;
    let status: Value =
        serde_json::from_str(encoded).context("invalid user-script runtime status JSON")?;
    if !status.is_object() {
        anyhow::bail!("user-script runtime status must be a JSON object");
    }
    Ok(status)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserScriptConfig {
    pub enabled: bool,
    pub scripts: BTreeMap<String, bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub skip_next_start: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub market: BTreeMap<String, MarketScriptInstall>,
    /// 拓展包的配置值，键是脚本 key（`user:<id>`）。只存清单 schema 里声明过的键。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub configs: BTreeMap<String, Map<String, Value>>,
}

impl Default for UserScriptConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scripts: BTreeMap::new(),
            skip_next_start: false,
            market: BTreeMap::new(),
            configs: BTreeMap::new(),
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MarketScriptInstall {
    pub id: String,
    pub name: String,
    pub version: String,
    pub script_url: String,
    pub homepage: String,
    pub installed_at: String,
}

#[derive(Debug, Clone)]
pub struct UserScriptManager {
    builtin_dir: PathBuf,
    user_dir: PathBuf,
    config_path: PathBuf,
    config_lock: Arc<Mutex<()>>,
}

impl UserScriptManager {
    pub fn new(
        builtin_dir: impl Into<PathBuf>,
        user_dir: impl Into<PathBuf>,
        config_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            builtin_dir: builtin_dir.into(),
            user_dir: user_dir.into(),
            config_path: config_path.into(),
            config_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn load_config(&self) -> UserScriptConfig {
        let _guard = self.config_lock.lock().unwrap();
        self.load_config_unlocked()
    }

    fn load_config_unlocked(&self) -> UserScriptConfig {
        let Ok(text) = fs::read_to_string(&self.config_path) else {
            return UserScriptConfig::default();
        };
        let Ok(Value::Object(raw)) = serde_json::from_str::<Value>(&text) else {
            return UserScriptConfig::default();
        };
        config_from_object(&raw)
    }

    pub fn save_config(&self, config: &UserScriptConfig) -> anyhow::Result<()> {
        let _guard = self.config_lock.lock().unwrap();
        self.save_config_unlocked(config)
    }

    fn save_config_unlocked(&self, config: &UserScriptConfig) -> anyhow::Result<()> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create user script config directory {}",
                    parent.display()
                )
            })?;
        }
        crate::settings::atomic_write(
            &self.config_path,
            serde_json::to_string_pretty(config)?.as_bytes(),
        )
    }

    pub fn set_global_enabled(&self, enabled: bool) -> anyhow::Result<UserScriptConfig> {
        let _guard = self.config_lock.lock().unwrap();
        let mut config = self.load_config_unlocked();
        config.enabled = enabled;
        self.save_config_unlocked(&config)?;
        Ok(config)
    }

    pub fn set_skip_next_start(&self, skip: bool) -> anyhow::Result<UserScriptConfig> {
        let _guard = self.config_lock.lock().unwrap();
        let mut config = self.load_config_unlocked();
        config.skip_next_start = skip;
        self.save_config_unlocked(&config)?;
        Ok(config)
    }

    /// Consume the one-shot safe-start marker before building the first-page bundle.
    /// The marker is cleared only after the updated configuration is persisted.
    pub fn consume_skip_next_start(&self) -> anyhow::Result<bool> {
        let _guard = self.config_lock.lock().unwrap();
        let mut config = self.load_config_unlocked();
        if !config.skip_next_start {
            return Ok(false);
        }
        config.skip_next_start = false;
        self.save_config_unlocked(&config)?;
        Ok(true)
    }

    pub fn set_script_enabled(&self, key: &str, enabled: bool) -> anyhow::Result<UserScriptConfig> {
        let _guard = self.config_lock.lock().unwrap();
        let mut config = self.load_config_unlocked();
        config.scripts.insert(key.to_string(), enabled);
        self.save_config_unlocked(&config)?;
        Ok(config)
    }

    pub fn delete_user_script(&self, key: &str) -> anyhow::Result<UserScriptConfig> {
        let Some(file_name) = key.strip_prefix("user:").filter(|value| !value.is_empty()) else {
            anyhow::bail!("only user scripts can be deleted");
        };
        if file_name.contains(['/', '\\']) || file_name == "." || file_name == ".." {
            anyhow::bail!("invalid user script key");
        }
        let path = self.user_dir.join(file_name);
        // 拓展包是目录。删除只做单文件，目录交给用户在文件夹里自己处理。
        if path.is_dir() {
            anyhow::bail!("拓展包是一个文件夹，请在用户脚本目录中手动移除：{}", path.display());
        }
        let canonical_user_dir = self
            .user_dir
            .canonicalize()
            .or_else(|_| {
                fs::create_dir_all(&self.user_dir)?;
                self.user_dir.canonicalize()
            })
            .with_context(|| {
                format!(
                    "failed to resolve user script directory {}",
                    self.user_dir.display()
                )
            })?;
        if path.exists() {
            let canonical_path = path
                .canonicalize()
                .with_context(|| format!("failed to resolve user script {}", path.display()))?;
            if !canonical_path.starts_with(&canonical_user_dir) {
                anyhow::bail!("refusing to delete script outside user script directory");
            }
            fs::remove_file(&canonical_path).with_context(|| {
                format!("failed to delete user script {}", canonical_path.display())
            })?;
        }

        let _guard = self.config_lock.lock().unwrap();
        let mut config = self.load_config_unlocked();
        config.scripts.remove(key);
        config.market.remove(key);
        self.save_config_unlocked(&config)?;
        Ok(config)
    }

    pub fn user_script_path_for_market_id(&self, id: &str) -> PathBuf {
        self.user_dir.join(market_script_filename(id))
    }

    pub fn record_market_install(&self, script: &MarketScript) -> anyhow::Result<UserScriptConfig> {
        let _guard = self.config_lock.lock().unwrap();
        let mut config = self.load_config_unlocked();
        let key = format!("user:{}", market_script_filename(&script.id));
        config.scripts.entry(key.clone()).or_insert(true);
        config.market.insert(
            key,
            MarketScriptInstall {
                id: script.id.clone(),
                name: script.name.clone(),
                version: script.version.clone(),
                script_url: script.script_url.clone(),
                homepage: script.homepage.clone(),
                installed_at: current_unix_timestamp_string(),
            },
        );
        self.save_config_unlocked(&config)?;
        Ok(config)
    }

    /// 启用中、清单无误的拓展包声明的 Codex 钩子，附带拓展 id。全局关闭时为空。
    pub fn enabled_extension_hooks(&self) -> anyhow::Result<Vec<(String, ExtensionHook)>> {
        let config = self.load_config();
        if !config.enabled {
            return Ok(Vec::new());
        }
        Ok(self
            .scan_script_files(&config)?
            .into_iter()
            .filter(|script| script.enabled)
            .filter_map(|script| script.package)
            .filter(|package| package.error.is_none())
            .flat_map(|package| {
                let id = package.id.clone();
                let config = package.config.clone();
                package.hooks.into_iter().map(move |mut hook| {
                    hook.config = config.clone();
                    (id.clone(), hook)
                })
            })
            .collect())
    }

    /// 保存一个拓展包的配置：只接受清单 schema 里声明的键，类型不符即报错。返回生效配置。
    pub fn set_extension_config(&self, key: &str, values: &Map<String, Value>) -> anyhow::Result<Value> {
        let config = self.load_config();
        let package = self
            .scan_script_files(&config)?
            .into_iter()
            .find(|script| script.key == key)
            .and_then(|script| script.package)
            .with_context(|| format!("没有找到拓展包 {key}"))?;
        if let Some(error) = &package.error {
            anyhow::bail!("拓展包清单有误，不能保存配置：{error}");
        }
        let mut stored = Map::new();
        for (name, value) in values {
            let field = package
                .config_schema
                .iter()
                .find(|field| &field.key == name)
                .with_context(|| format!("拓展没有声明配置项 {name}"))?;
            if !field.accepts(value) {
                anyhow::bail!("配置项 {name} 的值与类型 {} 不符", field.kind);
            }
            stored.insert(name.clone(), value.clone());
        }
        let _guard = self.config_lock.lock().unwrap();
        let mut latest = self.load_config_unlocked();
        latest.configs.insert(key.to_string(), stored.clone());
        self.save_config_unlocked(&latest)?;
        Ok(resolve_extension_config(&package.config_schema, Some(&stored)))
    }

    /// 启用中、清单无误的拓展包的常驻后台：(拓展 id, 后台脚本, 生效配置)。
    pub fn enabled_extension_backend(&self, key: &str) -> anyhow::Result<(String, PathBuf, Value)> {
        let config = self.load_config();
        if !config.enabled {
            anyhow::bail!("本地拓展整体已关闭");
        }
        let script = self
            .scan_script_files(&config)?
            .into_iter()
            .find(|script| script.key == key)
            .with_context(|| format!("没有找到拓展 {key}"))?;
        if !script.enabled {
            anyhow::bail!("拓展 {key} 已停用");
        }
        let package = script.package.with_context(|| format!("{key} 不是拓展包，没有后台"))?;
        if let Some(error) = package.error {
            anyhow::bail!("拓展包清单有误：{error}");
        }
        let backend = package.backend.with_context(|| format!("拓展 {key} 没有声明 backend"))?;
        Ok((package.id, backend, package.config))
    }

    /// 启用中、清单无误的拓展包声明的模型工具，附带拓展 id。
    pub fn enabled_extension_tools(&self) -> anyhow::Result<Vec<(String, ExtensionTool)>> {
        let config = self.load_config();
        if !config.enabled {
            return Ok(Vec::new());
        }
        Ok(self
            .scan_script_files(&config)?
            .into_iter()
            .filter(|script| script.enabled)
            .filter_map(|script| script.package)
            .filter(|package| package.error.is_none())
            .flat_map(|package| {
                let id = package.id.clone();
                let config = package.config.clone();
                package.tools.into_iter().map(move |mut tool| {
                    tool.config = config.clone();
                    (id.clone(), tool)
                })
            })
            .collect())
    }

    /// 启用中、清单无误的拓展包声明的网关请求中间件，按拓展 key 排序（即执行顺序）。
    pub fn enabled_gateway_middlewares(&self) -> anyhow::Result<Vec<(String, ExtensionGatewayMiddleware)>> {
        self.enabled_gateway_list(|package| &package.gateway_request)
    }

    /// 启用中、清单无误的拓展包声明的网关响应流中间件，附带拓展 id，按拓展 key 顺序。
    pub fn enabled_gateway_response_middlewares(&self) -> anyhow::Result<Vec<(String, ExtensionGatewayMiddleware)>> {
        self.enabled_gateway_list(|package| &package.gateway_response)
    }

    fn enabled_gateway_list(
        &self,
        pick: impl Fn(&ExtensionPackage) -> &Vec<ExtensionGatewayMiddleware>,
    ) -> anyhow::Result<Vec<(String, ExtensionGatewayMiddleware)>> {
        let config = self.load_config();
        if !config.enabled {
            return Ok(Vec::new());
        }
        Ok(self
            .scan_script_files(&config)?
            .into_iter()
            .filter(|script| script.enabled)
            .filter_map(|script| script.package)
            .filter(|package| package.error.is_none())
            .flat_map(|package| {
                let id = package.id.clone();
                let config = package.config.clone();
                pick(&package).clone().into_iter().map(move |mut item| {
                    item.config = config.clone();
                    (id.clone(), item)
                })
            })
            .collect())
    }

    /// 用户脚本目录（拓展包就放在它下面）。
    pub fn user_dir(&self) -> &std::path::Path {
        &self.user_dir
    }

    /// 写入（或覆盖）一个用户拓展包：`user_dir/<id>/` 下的文件加生成的 `codexpp.json`。
    ///
    /// 供「对话造拓展」使用，所以校验从严：只能写 user 目录、只能写包内相对路径、
    /// 同名目录必须是同 id 的拓展包（不会接管别的目录）、单文件与总量有上限。
    /// 清单最后写，写入中途失败时旧清单仍在。不删除包里已有的其他文件。
    /// 返回脚本 key 与清单校验结果（`error` 非空表示包写好了但不会被加载）。
    pub fn write_extension_package(&self, write: &ExtensionWrite) -> anyhow::Result<Value> {
        validate_extension_id(&write.id)?;
        let file_count = write.files.len() + write.assets.len();
        if file_count == 0 || file_count > EXTENSION_WRITE_MAX_FILES {
            anyhow::bail!("files 需要 1~{EXTENSION_WRITE_MAX_FILES} 个文件");
        }
        // 先把二进制资源读进内存并校验，任何一个不合格就什么都不写。
        let mut asset_bytes = Vec::new();
        let mut asset_total = 0usize;
        for (path, source) in &write.assets {
            validate_package_relative_path(path)?;
            if write.files.contains_key(path) {
                anyhow::bail!("文件 {path} 重复出现");
            }
            if extension_asset_mime(path).is_none() {
                anyhow::bail!("{path} 不是支持的资源类型（图片、音频、字体）；文本文件请用 content");
            }
            if !source.is_absolute() {
                anyhow::bail!("{path} 的 sourcePath 必须是绝对路径");
            }
            let metadata = fs::metadata(source).with_context(|| format!("读不到 {}", source.display()))?;
            if !metadata.is_file() {
                anyhow::bail!("{} 不是普通文件", source.display());
            }
            if metadata.len() > EXTENSION_ASSET_MAX_BYTES as u64 {
                anyhow::bail!("{path} 超过单个资源上限 {} KiB，先压缩或缩小尺寸", EXTENSION_ASSET_MAX_BYTES / 1024);
            }
            let bytes = fs::read(source).with_context(|| format!("读不到 {}", source.display()))?;
            asset_total += bytes.len();
            asset_bytes.push((path.clone(), bytes));
        }
        if asset_total > EXTENSION_ASSET_MAX_TOTAL_BYTES {
            anyhow::bail!("资源总量超过 {} KiB", EXTENSION_ASSET_MAX_TOTAL_BYTES / 1024);
        }
        let mut total = 0usize;
        for (path, content) in &write.files {
            validate_package_relative_path(path)?;
            if path.eq_ignore_ascii_case(EXTENSION_MANIFEST_FILE) {
                anyhow::bail!("{EXTENSION_MANIFEST_FILE} 由清单参数生成，不能直接写");
            }
            if content.len() > EXTENSION_WRITE_MAX_FILE_BYTES {
                anyhow::bail!("{path} 超过单文件上限 {} KiB", EXTENSION_WRITE_MAX_FILE_BYTES / 1024);
            }
            total += content.len();
        }
        if total > EXTENSION_WRITE_MAX_TOTAL_BYTES {
            anyhow::bail!("文件总量超过 {} KiB", EXTENSION_WRITE_MAX_TOTAL_BYTES / 1024);
        }
        // 没指定页面入口时：有 renderer.js 就用它；只声明 hooks 的拓展可以没有页面入口。
        let renderer = if !write.renderer.is_empty() {
            write.renderer.clone()
        } else if write.files.contains_key("renderer.js")
            || (write.hooks.is_none() && write.gateway.is_none() && write.tools.is_none() && write.backend.is_none())
        {
            vec!["renderer.js".to_string()]
        } else {
            Vec::new()
        };
        fs::create_dir_all(&self.user_dir).with_context(|| {
            format!("failed to create user scripts directory {}", self.user_dir.display())
        })?;
        let root = self
            .user_dir
            .canonicalize()
            .context("无法解析用户脚本目录")?;
        let directory = root.join(&write.id);
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                anyhow::bail!("{} 已存在且不是普通目录，未写入", directory.display());
            }
            Ok(_) => {
                let manifest = directory.join(EXTENSION_MANIFEST_FILE);
                let existing_id = fs::read_to_string(&manifest)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .and_then(|value| value.get("id").and_then(Value::as_str).map(str::to_owned));
                if existing_id.as_deref() != Some(write.id.as_str()) {
                    anyhow::bail!(
                        "{} 已存在但不是 id 为 {} 的拓展包，未覆盖",
                        directory.display(),
                        write.id
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&directory)
                    .with_context(|| format!("无法创建拓展目录 {}", directory.display()))?;
            }
            Err(error) => return Err(error).context("无法检查拓展目录"),
        }
        for (path, content) in &write.files {
            let target = directory.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("无法创建目录 {}", parent.display()))?;
                // 中间目录可能是指向包外的软链：落盘前确认真实位置仍在包内。
                let parent = parent.canonicalize().context("无法解析拓展子目录")?;
                if !parent.starts_with(&directory) {
                    anyhow::bail!("{path} 指向拓展包以外，未写入");
                }
            }
            if fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                anyhow::bail!("{path} 是符号链接，未写入");
            }
            crate::settings::atomic_write(&target, content.as_bytes())
                .with_context(|| format!("无法写入 {path}"))?;
        }
        for (path, bytes) in &asset_bytes {
            let target = directory.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("无法创建目录 {}", parent.display()))?;
                let parent = parent.canonicalize().context("无法解析拓展子目录")?;
                if !parent.starts_with(&directory) {
                    anyhow::bail!("{path} 指向拓展包以外，未写入");
                }
            }
            if fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                anyhow::bail!("{path} 是符号链接，未写入");
            }
            crate::settings::atomic_write(&target, bytes).with_context(|| format!("无法写入 {path}"))?;
        }
        let manifest = json!({
            "id": write.id,
            "name": write.name,
            "version": if write.version.trim().is_empty() { "0.1.0" } else { write.version.trim() },
            "description": write.description,
            "engines": { "codexpp": EXTENSION_API_VERSION },
            "renderer": renderer,
            "permissions": { "routes": write.routes },
        });
        let mut manifest = manifest;
        if let Some(hooks) = &write.hooks {
            manifest["hooks"] = hooks.clone();
        }
        if let Some(gateway) = &write.gateway {
            manifest["gateway"] = gateway.clone();
        }
        if let Some(tools) = &write.tools {
            manifest["tools"] = tools.clone();
        }
        if let Some(config) = &write.config {
            manifest["config"] = config.clone();
        }
        if let Some(backend) = &write.backend {
            manifest["backend"] = backend.clone();
        }
        crate::settings::atomic_write(
            &directory.join(EXTENSION_MANIFEST_FILE),
            serde_json::to_string_pretty(&manifest)?.as_bytes(),
        )
        .context("无法写入拓展清单")?;
        let package = load_extension_package(&directory);
        Ok(json!({
            "key": extension_package_key(&write.id),
            "directory": directory.to_string_lossy(),
            "manifestError": package.error,
        }))
    }

    /// 读回一个用户拓展包的清单与文本文件，供对话里修改已有拓展。
    pub fn read_extension_package(&self, id: &str) -> anyhow::Result<Value> {
        validate_extension_id(id)?;
        let directory = self.user_dir.join(id);
        let manifest_path = directory.join(EXTENSION_MANIFEST_FILE);
        if fs::symlink_metadata(&directory).map(|metadata| !metadata.is_dir()).unwrap_or(true)
            || !manifest_path.is_file()
        {
            anyhow::bail!("没有 id 为 {id} 的用户拓展包");
        }
        let manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)
            .unwrap_or(Value::Null);
        let mut files = Vec::new();
        let mut total = 0usize;
        collect_package_files(&directory, &directory, 0, &mut files, &mut total);
        Ok(json!({
            "key": extension_package_key(id),
            "directory": directory.to_string_lossy(),
            "manifest": manifest,
            "files": files,
        }))
    }

    /// 用户拓展包是否存在（带清单的目录）。
    pub fn has_extension_package(&self, id: &str) -> bool {
        validate_extension_id(id).is_ok()
            && self.user_dir.join(id).join(EXTENSION_MANIFEST_FILE).is_file()
    }

    pub fn inventory(&self) -> anyhow::Result<Value> {
        self.inventory_with_runtime_status(None)
    }

    /// Build the script inventory and, when available, merge the status recorded
    /// by the renderer-side loader. The filesystem scan alone cannot tell
    /// whether a script was actually evaluated, so callers that have access to
    /// the live page should provide `window.__codexPlusUserScripts.scripts`.
    pub fn inventory_with_runtime_status(
        &self,
        runtime_status: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let config = self.load_config();
        let scripts = self.scan_scripts(&config, runtime_status)?;
        Ok(json!({
            "enabled": config.enabled,
            "skip_next_start": config.skip_next_start,
            "builtin_dir": self.builtin_dir.to_string_lossy(),
            "user_dir": self.user_dir.to_string_lossy(),
            "scripts": scripts
        }))
    }

    pub fn build_enabled_bundle(&self) -> anyhow::Result<String> {
        let config = self.load_config();
        if !config.enabled {
            return Ok(String::new());
        }
        let mut blocks = vec![RUNTIME_SCRIPT.to_string()];
        for script in self.scan_script_files(&config)? {
            if !script.enabled {
                continue;
            }
            let source = match &script.package {
                // 清单不合法或版本不兼容的包不注入，原因在管理页显示。
                Some(package) if package.error.is_some() => continue,
                Some(package) => package_source(package),
                None => fs::read_to_string(&script.path).unwrap_or_else(|error| {
                    format!("throw new Error({});", json!(error.to_string()))
                }),
            };
            blocks.push(wrap_script(&script, &source));
        }
        Ok(blocks.join("\n"))
    }

    /// 页面启动与桥接恢复只能初始化一次，不能清理旧脚本或触发刷新。
    pub fn build_initial_bundle(&self) -> anyhow::Result<String> {
        let skip_next_start = self.consume_skip_next_start()?;
        let bundle = if skip_next_start {
            String::new()
        } else {
            self.build_enabled_bundle()?
        };
        Ok(format!(
            r#"(() => {{
  {RUNTIME_SCRIPT}
  const runtime = window.__codexPlusUserScripts;
  if (!runtime.initialized && !runtime.refreshPending && !Object.keys(runtime.scripts).length) {{
    runtime.initialized = true;
    {bundle}
  }}
  return JSON.stringify({{ mode: "{mode}", scripts: runtime.scripts }});
}})()"#,
            mode = if skip_next_start {
                "skipped"
            } else {
                "scripts"
            }
        ))
    }

    /// 即便全部禁用也必须清理上一次运行的脚本。
    pub fn build_reload_bundle(&self) -> anyhow::Result<String> {
        let bundle = self.build_enabled_bundle()?;
        Ok(format!(
            r#"(() => {{
  {RUNTIME_SCRIPT}
  const mode = window.__codexPlusUserScripts.prepareReload();
  if (mode === "page") return JSON.stringify({{ mode }});
  {bundle}
  return JSON.stringify({{ mode, scripts: window.__codexPlusUserScripts.scripts }});
}})()"#
        ))
    }

    fn scan_scripts(
        &self,
        config: &UserScriptConfig,
        runtime_status: Option<&Value>,
    ) -> anyhow::Result<Vec<Value>> {
        let runtime_scripts = runtime_status
            .and_then(|value| value.get("scripts").or(Some(value)))
            .and_then(Value::as_object);
        Ok(self
            .scan_script_files(config)?
            .into_iter()
            .map(|script| {
                let market = config.market.get(&script.key);
                let package_error = script
                    .package
                    .as_ref()
                    .and_then(|package| package.error.as_deref());
                let fallback_status = if !config.enabled || !script.enabled {
                    "disabled"
                } else if package_error.is_some() {
                    "failed"
                } else {
                    "not_loaded"
                };
                let live = runtime_scripts.and_then(|items| items.get(&script.key));
                let status = if fallback_status == "disabled" || package_error.is_some() {
                    fallback_status
                } else {
                    live.and_then(|item| item.get("status").and_then(Value::as_str))
                        .unwrap_or(fallback_status)
                };
                let error = if fallback_status == "disabled" {
                    ""
                } else if let Some(error) = package_error {
                    error
                } else {
                    live.and_then(|item| item.get("error").and_then(Value::as_str))
                        .unwrap_or("")
                };
                json!({
                    "key": script.key,
                    "name": script.name,
                    "source": script.source,
                    "kind": if script.package.is_some() { "package" } else { "script" },
                    "package": script.package.as_ref().map(|package| package.summary()),
                    "enabled": script.enabled,
                    "status": status,
                    "error": error,
                    "market_id": market.as_ref().map(|item| item.id.as_str()).unwrap_or(""),
                    "version": market.as_ref().map(|item| item.version.as_str()).unwrap_or(""),
                    "installed": market.is_some(),
                    "source_url": market.as_ref().map(|item| item.script_url.as_str()).unwrap_or(""),
                    "homepage": market.as_ref().map(|item| item.homepage.as_str()).unwrap_or("")
                })
            })
            .collect())
    }

    fn scan_script_files(&self, config: &UserScriptConfig) -> anyhow::Result<Vec<UserScriptFile>> {
        fs::create_dir_all(&self.user_dir).with_context(|| {
            format!(
                "failed to create user scripts directory {}",
                self.user_dir.display()
            )
        })?;
        let mut scripts = Vec::new();
        self.append_scripts("builtin", &self.builtin_dir, config, &mut scripts)?;
        self.append_scripts("user", &self.user_dir, config, &mut scripts)?;
        Ok(scripts)
    }

    fn append_scripts(
        &self,
        source: &str,
        directory: &std::path::Path,
        config: &UserScriptConfig,
        scripts: &mut Vec<UserScriptFile>,
    ) -> anyhow::Result<()> {
        let Ok(entries) = fs::read_dir(directory) else {
            return Ok(());
        };
        // 单文件脚本是 `*.js`；拓展包是带 codexpp.json 的子目录。目录名以 .js 结尾的
        // 不当包处理，免得和同名脚本文件抢同一个 key。
        let mut paths = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                let is_js = path.extension().and_then(|value| value.to_str()) == Some("js");
                if path.is_dir() {
                    !is_js && path.join(EXTENSION_MANIFEST_FILE).is_file()
                } else {
                    is_js
                }
            })
            .collect::<Vec<_>>();
        paths.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        });

        for path in paths {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            let key = format!("{source}:{name}");
            let package = path.is_dir().then(|| {
                let mut package = load_extension_package(&path);
                package.config = resolve_extension_config(&package.config_schema, config.configs.get(&key));
                package
            });
            scripts.push(UserScriptFile {
                enabled: config.scripts.get(&key).copied().unwrap_or(true),
                key,
                name: package
                    .as_ref()
                    .and_then(|package| package.name.clone())
                    .unwrap_or(name),
                source: source.to_string(),
                path,
                package,
            });
        }
        Ok(())
    }
}

#[derive(Debug)]
struct UserScriptFile {
    key: String,
    name: String,
    source: String,
    path: PathBuf,
    enabled: bool,
    package: Option<ExtensionPackage>,
}

/// 一个拓展包：目录 + `codexpp.json`。
///
/// 清单有问题时仍然返回包（`error` 非空），让它出现在管理页上并显示原因，
/// 而不是悄无声息地消失。
#[derive(Debug)]
struct ExtensionPackage {
    id: String,
    name: Option<String>,
    version: String,
    description: String,
    engine: u64,
    routes: Vec<String>,
    renderer: Vec<PathBuf>,
    hooks: Vec<ExtensionHook>,
    gateway_request: Vec<ExtensionGatewayMiddleware>,
    /// 网关响应流中间件（codexpp.json 的 `gateway.response`）。
    gateway_response: Vec<ExtensionGatewayMiddleware>,
    tools: Vec<ExtensionTool>,
    config_schema: Vec<ExtensionConfigField>,
    /// 常驻后台脚本（codexpp.json 的 `backend.script`）。
    backend: Option<PathBuf>,
    /// 包内 `locale/<语言>.json`：语言 id → 词条（`meta.title` / `meta.description` 是本地化的名称与描述）。
    locales: std::collections::BTreeMap<String, Value>,
    /// 包内 `assets/` 的资源：包内路径 → data URL，交给页面的 `ctx.asset(path)`。
    assets: std::collections::BTreeMap<String, String>,
    /// 超出大小上限而没有下发的资源。
    skipped_assets: Vec<String>,
    /// 生效配置：schema 默认值叠加 user_scripts.json 里存的值。扫描时算好。
    config: Value,
    error: Option<String>,
}

/// 拓展配置项（codexpp.json 的 `config.<key>`）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionConfigField {
    pub key: String,
    /// string / number / boolean / select
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub description: String,
    pub default: Value,
    /// select 的可选值：[{ value, label }]
    pub options: Vec<Value>,
    /// 界面上按密码框显示。值仍以明文存在本机 user_scripts.json。
    pub secret: bool,
}

impl ExtensionConfigField {
    /// 校验一个值是否合乎类型；select 必须是可选值之一。
    fn accepts(&self, value: &Value) -> bool {
        match self.kind.as_str() {
            "string" => value.is_string(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "select" => self.options.iter().any(|option| option.get("value") == Some(value)),
            _ => false,
        }
    }
}

/// 生效配置：每个声明的键取已存且合法的值，否则取默认值。
fn resolve_extension_config(schema: &[ExtensionConfigField], stored: Option<&Map<String, Value>>) -> Value {
    let mut resolved = Map::new();
    for field in schema {
        let value = stored
            .and_then(|values| values.get(&field.key))
            .filter(|value| field.accepts(value))
            .cloned()
            .unwrap_or_else(|| field.default.clone());
        resolved.insert(field.key.clone(), value);
    }
    Value::Object(resolved)
}

/// 拓展声明的一个模型工具（codexpp.json 的 `tools[]`），由 Codex++ 拓展 MCP 对外提供。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExtensionTool {
    /// 拓展内的工具名：字母、数字、_、-。对外名为 `<拓展 id>__<name>`。
    pub name: String,
    pub description: String,
    /// JSON Schema（object），原样交给 MCP。
    pub input_schema: Value,
    /// 包内脚本的绝对路径：默认导出 `async (args, context) => 结果`。
    pub script: PathBuf,
    pub timeout_secs: u64,
    /// 所属拓展的生效配置，作为 context.config 交给脚本。
    pub config: Value,
}

/// 拓展工具单次调用的超时上限（秒）。
pub const EXTENSION_TOOL_MAX_TIMEOUT_SECS: u64 = 300;

/// 拓展工具对外的名字。MCP 工具名只能是字母、数字、_、-，最长 64。
pub fn extension_tool_public_name(extension_id: &str, tool: &str) -> String {
    format!("{extension_id}__{tool}")
}

/// 拓展声明的一条网关请求中间件（codexpp.json 的 `gateway.request[]`）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExtensionGatewayMiddleware {
    /// 包内脚本的绝对路径：默认导出 `async (body, context) => 新 body | null`。
    pub script: PathBuf,
    pub timeout_secs: u64,
    /// 所属拓展的生效配置，作为 context.config 交给脚本。
    pub config: Value,
}

/// 网关请求中间件的超时上限（秒）：它挡在每一次模型请求前面，不能太久。
pub const EXTENSION_GATEWAY_MAX_TIMEOUT_SECS: u64 = 10;

/// Codex++ 的用户脚本配置目录：Windows 为 `%APPDATA%\\Codex++`，其余为 `$XDG_CONFIG_HOME/Codex++`
/// 或 `~/.config/Codex++`。管理器、启动器、本地代理、钩子分发共用这一份，不各自再算。
pub fn default_user_scripts_config_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(roaming) = std::env::var_os("APPDATA") {
            return PathBuf::from(roaming).join("Codex++");
        }
        if let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
            return home.join("AppData").join("Roaming").join("Codex++");
        }
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("Codex++")
}

/// 默认用户脚本位置：内置目录在程序旁，用户目录在配置目录下。
pub fn default_user_script_manager() -> UserScriptManager {
    let config_dir = default_user_scripts_config_dir();
    let builtin_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .map(|path| path.join("user_scripts"))
        .unwrap_or_else(|| PathBuf::from("user_scripts"));
    UserScriptManager::new(builtin_dir, config_dir.join("user_scripts"), config_dir.join("user_scripts.json"))
}

/// 拓展声明的一条 Codex 钩子（codexpp.json 的 `hooks.<事件>[]`）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExtensionHook {
    pub event: String,
    /// 正则，按 Codex 的规则匹配（工具名、SessionStart 的 source 等）；空或 `*` 表示全部。
    pub matcher: String,
    /// 包内脚本的绝对路径：默认导出 `async (payload, context) => output`。
    pub script: PathBuf,
    pub timeout_secs: u64,
    /// 所属拓展的生效配置，作为 context.config 交给脚本。
    pub config: Value,
}

/// 拓展可以挂的 Codex 钩子事件。SessionEnd / Interrupt 的超时只有 1~3 秒，起 Node 来不及，不开放。
pub const EXTENSION_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "PreCompact",
    "PostCompact",
    "SubagentStart",
    "SubagentStop",
    "Stop",
];
/// 单个拓展钩子的超时上限（秒）。写进 Codex 配置的分发命令固定用 120 秒，要留出余量。
pub const EXTENSION_HOOK_MAX_TIMEOUT_SECS: u64 = 100;

impl ExtensionPackage {
    fn summary(&self) -> Value {
        json!({
            "id": self.id,
            "version": self.version,
            "description": self.description,
            "engine": self.engine,
            "routes": self.routes,
            "hookEvents": self.hooks.iter().map(|hook| hook.event.as_str()).collect::<std::collections::BTreeSet<_>>(),
            "gatewayRequest": self.gateway_request.len(),
            "gatewayResponse": self.gateway_response.len(),
            "tools": self.tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>(),
            "configSchema": self.config_schema,
            "backend": self.backend.is_some(),
            "config": self.config,
            "assets": self.assets.keys().collect::<Vec<_>>(),
            "skippedAssets": self.skipped_assets,
            // 只给名称与描述的本地化版本，词条本身留给页面运行时。
            "locales": self.locales.iter().map(|(lang, messages)| (lang.clone(), json!({
                "title": messages.pointer("/meta/title").cloned().unwrap_or(Value::Null),
                "description": messages.pointer("/meta/description").cloned().unwrap_or(Value::Null),
            }))).collect::<serde_json::Map<_, _>>(),
        })
    }

    /// 交给 renderer 的清单：只放展示与权限需要的字段，不带本机路径。
    fn renderer_manifest(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "version": self.version,
            "description": self.description,
            "engines": { "codexpp": self.engine },
            "permissions": { "routes": self.routes },
            "config": self.config,
            "locales": self.locales,
            "assets": self.assets,
        })
    }
}

/// 对话写入拓展包的参数。`files` 的键是包内相对路径。
#[derive(Debug, Clone, Default)]
pub struct ExtensionWrite {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub routes: Vec<String>,
    pub renderer: Vec<String>,
    /// 原样写进清单的 `hooks`，加载时校验。
    pub hooks: Option<Value>,
    /// 原样写进清单的 `gateway`，加载时校验。
    pub gateway: Option<Value>,
    /// 原样写进清单的 `tools`，加载时校验。
    pub tools: Option<Value>,
    /// 原样写进清单的 `config`（配置 schema），加载时校验。
    pub config: Option<Value>,
    /// 原样写进清单的 `backend`，加载时校验。
    pub backend: Option<Value>,
    pub files: BTreeMap<String, String>,
    /// 二进制资源：包内路径 → 本机源文件，按字节复制（图片、音频、字体）。
    pub assets: BTreeMap<String, PathBuf>,
}

const EXTENSION_WRITE_MAX_FILES: usize = 32;
const EXTENSION_WRITE_MAX_FILE_BYTES: usize = 256 * 1024;
const EXTENSION_WRITE_MAX_TOTAL_BYTES: usize = 1024 * 1024;
/// 单个二进制资源（图片、音频、字体）的上限，以及一个包里资源的总上限。
/// 资源会以 data URL 随清单下发给页面，太大会拖慢每次加载。
const EXTENSION_ASSET_MAX_BYTES: usize = 1024 * 1024;
const EXTENSION_ASSET_MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;

/// 支持的资源类型（按扩展名），返回 MIME。
fn extension_asset_mime(path: &str) -> Option<&'static str> {
    let extension = path.rsplit('.').next()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "m4a" => "audio/mp4",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => return None,
    })
}

/// 读包内 `assets/` 下的资源，转成 data URL 交给页面（`ctx.asset(path)`）。超限的跳过并记下原因。
fn load_extension_assets(root: &std::path::Path) -> (std::collections::BTreeMap<String, String>, Vec<String>) {
    use base64::Engine;
    let mut assets = std::collections::BTreeMap::new();
    let mut skipped = Vec::new();
    let mut total = 0usize;
    let mut stack = vec![root.join("assets")];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(relative) = path.strip_prefix(root).ok().map(|relative| relative.to_string_lossy().replace('\\', "/")) else {
                continue;
            };
            let Some(mime) = extension_asset_mime(&relative) else {
                continue;
            };
            let size = metadata.len() as usize;
            if size > EXTENSION_ASSET_MAX_BYTES || total + size > EXTENSION_ASSET_MAX_TOTAL_BYTES {
                skipped.push(relative);
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            total += bytes.len();
            assets.insert(relative, format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)));
        }
    }
    (assets, skipped)
}
/// 读回拓展包时的上限：只读文本、跳过大文件，避免把整个目录塞进对话。
const EXTENSION_READ_MAX_TOTAL_BYTES: usize = 512 * 1024;

/// 拓展包对应的脚本 key。
pub fn extension_package_key(id: &str) -> String {
    format!("user:{id}")
}

pub fn validate_extension_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || id.len() > 64
        || id.ends_with(".js")
        || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("拓展 id 只能包含字母、数字、- 和 _，最长 64 个字符");
    }
    Ok(())
}

fn validate_package_relative_path(path: &str) -> anyhow::Result<()> {
    let relative = std::path::Path::new(path);
    if path.trim().is_empty()
        || path.contains('\\')
        || relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        anyhow::bail!("文件路径 {path} 必须是包内的相对路径");
    }
    Ok(())
}

fn collect_package_files(
    root: &std::path::Path,
    directory: &std::path::Path,
    depth: usize,
    files: &mut Vec<Value>,
    total: &mut usize,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut paths = entries.filter_map(Result::ok).map(|entry| entry.path()).collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        let relative = path
            .strip_prefix(root)
            .map(|value| value.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if metadata.file_type().is_symlink() || relative == EXTENSION_MANIFEST_FILE {
            continue;
        }
        if metadata.is_dir() {
            if depth < 4 {
                collect_package_files(root, &path, depth + 1, files, total);
            }
            continue;
        }
        let size = metadata.len() as usize;
        let content = (size <= EXTENSION_WRITE_MAX_FILE_BYTES
            && *total + size <= EXTENSION_READ_MAX_TOTAL_BYTES)
            .then(|| fs::read_to_string(&path).ok())
            .flatten();
        if let Some(content) = &content {
            *total += content.len();
        }
        files.push(json!({ "path": relative, "size": size, "content": content }));
    }
}

fn load_extension_package(directory: &std::path::Path) -> ExtensionPackage {
    let mut package = ExtensionPackage {
        id: String::new(),
        name: None,
        version: String::new(),
        description: String::new(),
        engine: EXTENSION_API_VERSION,
        routes: Vec::new(),
        renderer: Vec::new(),
        hooks: Vec::new(),
        gateway_request: Vec::new(),
        gateway_response: Vec::new(),
        tools: Vec::new(),
        config_schema: Vec::new(),
        backend: None,
        locales: Default::default(),
        assets: Default::default(),
        skipped_assets: Vec::new(),
        config: json!({}),
        error: None,
    };
    if let Err(error) = parse_extension_manifest(directory, &mut package) {
        package.error = Some(error.to_string());
    }
    package
}

fn parse_extension_manifest(
    directory: &std::path::Path,
    package: &mut ExtensionPackage,
) -> anyhow::Result<()> {
    let text = fs::read_to_string(directory.join(EXTENSION_MANIFEST_FILE))
        .with_context(|| format!("读取 {EXTENSION_MANIFEST_FILE} 失败"))?;
    let Value::Object(raw) = serde_json::from_str::<Value>(&text)
        .with_context(|| format!("{EXTENSION_MANIFEST_FILE} 不是合法的 JSON"))?
    else {
        anyhow::bail!("{EXTENSION_MANIFEST_FILE} 顶层必须是对象");
    };
    package.name = string_field(&raw, "name");
    package.version = string_field(&raw, "version").unwrap_or_default();
    package.description = string_field(&raw, "description").unwrap_or_default();
    package.id = string_field(&raw, "id").unwrap_or_default();
    if package.id.is_empty()
        || !package
            .id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("清单的 id 必填，且只能包含字母、数字、- 和 _");
    }
    if let Some(engine) = raw.get("engines").and_then(|engines| engines.get("codexpp")) {
        package.engine = engine
            .as_u64()
            .context("engines.codexpp 必须是正整数（拓展 API 版本）")?;
    }
    package.routes = match raw.get("permissions").and_then(|value| value.get("routes")) {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(ToOwned::to_owned))
            .collect::<Option<Vec<_>>>()
            .context("permissions.routes 只能是字符串数组")?,
        Some(_) => anyhow::bail!("permissions.routes 只能是字符串数组"),
    };
    let entries = match raw.get("renderer") {
        None => Vec::new(),
        Some(Value::String(entry)) => vec![entry.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(ToOwned::to_owned))
            .collect::<Option<Vec<_>>>()
            .context("renderer 只能是字符串或字符串数组")?,
        Some(_) => anyhow::bail!("renderer 只能是字符串或字符串数组"),
    };
    let hooks_declared = raw.get("hooks").is_some_and(|hooks| hooks.as_object().is_some_and(|map| !map.is_empty()));
    let gateway_declared = raw.get("gateway").is_some();
    let tools_declared = raw.get("tools").is_some_and(|tools| tools.as_array().is_some_and(|list| !list.is_empty()));
    let backend_declared = raw.get("backend").is_some();
    if entries.is_empty() && !hooks_declared && !gateway_declared && !tools_declared && !backend_declared {
        anyhow::bail!("清单没有声明 renderer 入口，也没有声明 hooks、gateway、tools 或 backend");
    }
    let root = directory
        .canonicalize()
        .with_context(|| format!("无法解析拓展目录 {}", directory.display()))?;
    package.renderer = entries
        .iter()
        .map(|entry| resolve_package_entry(&root, entry))
        .collect::<anyhow::Result<Vec<_>>>()?;
    package.hooks = parse_extension_hooks(&root, raw.get("hooks"))?;
    (package.gateway_request, package.gateway_response) = parse_gateway_middlewares(&root, raw.get("gateway"))?;
    package.tools = parse_extension_tools(&root, &package.id, raw.get("tools"))?;
    package.config_schema = parse_extension_config_schema(raw.get("config"))?;
    package.backend = match raw.get("backend") {
        None => None,
        Some(Value::Object(backend)) => Some(resolve_package_entry(
            &root,
            backend.get("script").and_then(Value::as_str).context("backend 缺少 script")?,
        )?),
        Some(_) => anyhow::bail!("backend 必须是对象：{{ \"script\": \"backend/main.mjs\" }}"),
    };
    package.locales = load_extension_locales(&root)?;
    (package.assets, package.skipped_assets) = load_extension_assets(&root);
    // 版本校验放在最后：清单本身的格式错误更具体，优先报。
    if package.engine > EXTENSION_API_VERSION {
        anyhow::bail!(
            "需要拓展 API v{}，当前 Codex++ 提供 v{}，请升级 Codex++",
            package.engine,
            EXTENSION_API_VERSION
        );
    }
    Ok(())
}

/// 单个语言文件的大小上限。
const EXTENSION_LOCALE_MAX_BYTES: u64 = 256 * 1024;

/// 读包内 `locale/*.json`。文件名就是语言 id（如 `en`、`zh`、`zh-TW`），内容是 JSON 对象；
/// `meta.title` / `meta.description` 是本地化的名称与描述，其余键是拓展自己的词条。
fn load_extension_locales(root: &std::path::Path) -> anyhow::Result<std::collections::BTreeMap<String, Value>> {
    let mut locales = std::collections::BTreeMap::new();
    let directory = root.join("locale");
    let Ok(entries) = fs::read_dir(&directory) else {
        return Ok(locales);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let lang = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_string();
        let valid = !lang.is_empty()
            && lang.split('-').enumerate().all(|(index, part)| {
                (if index == 0 { (2..=3).contains(&part.len()) } else { (2..=8).contains(&part.len()) })
                    && part.chars().all(|ch| ch.is_ascii_alphanumeric())
            });
        if !valid {
            anyhow::bail!("locale/{lang}.json 的文件名不是合法的语言 id（如 en、zh、zh-TW）");
        }
        // 必须在包内：拒绝指向包外的软链。
        let resolved = path.canonicalize().with_context(|| format!("无法读取 locale/{lang}.json"))?;
        if !resolved.starts_with(root) {
            anyhow::bail!("locale/{lang}.json 指向包外");
        }
        if fs::metadata(&resolved).map(|meta| meta.len()).unwrap_or(0) > EXTENSION_LOCALE_MAX_BYTES {
            anyhow::bail!("locale/{lang}.json 超过 256 KiB");
        }
        let value: Value = serde_json::from_str(&fs::read_to_string(&resolved)?)
            .with_context(|| format!("locale/{lang}.json 不是合法的 JSON"))?;
        if !value.is_object() {
            anyhow::bail!("locale/{lang}.json 顶层必须是对象");
        }
        locales.insert(lang, value);
    }
    Ok(locales)
}

/// 解析 `hooks`：`{ "PreToolUse": [{ "matcher": "^Bash$", "script": "hooks/guard.mjs", "timeout": 30 }] }`。
fn parse_extension_hooks(
    root: &std::path::Path,
    value: Option<&Value>,
) -> anyhow::Result<Vec<ExtensionHook>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let map = value.as_object().context("hooks 必须是对象：{ 事件名: [钩子...] }")?;
    let mut hooks = Vec::new();
    for (event, list) in map {
        if !EXTENSION_HOOK_EVENTS.contains(&event.as_str()) {
            anyhow::bail!(
                "hooks 不支持事件 {event}，可用：{}",
                EXTENSION_HOOK_EVENTS.join("、")
            );
        }
        let items = list
            .as_array()
            .with_context(|| format!("hooks.{event} 必须是数组"))?;
        for item in items {
            let item = item
                .as_object()
                .with_context(|| format!("hooks.{event} 的每一项必须是对象"))?;
            let script = item
                .get("script")
                .and_then(Value::as_str)
                .with_context(|| format!("hooks.{event} 缺少 script"))?;
            let matcher = match item.get("matcher") {
                None => String::new(),
                Some(Value::String(matcher)) => matcher.clone(),
                Some(_) => anyhow::bail!("hooks.{event} 的 matcher 必须是字符串"),
            };
            let timeout_secs = match item.get("timeout") {
                None => 30,
                Some(value) => value
                    .as_u64()
                    .filter(|seconds| (1..=EXTENSION_HOOK_MAX_TIMEOUT_SECS).contains(seconds))
                    .with_context(|| {
                        format!("hooks.{event} 的 timeout 必须是 1~{EXTENSION_HOOK_MAX_TIMEOUT_SECS} 秒")
                    })?,
            };
            hooks.push(ExtensionHook {
                event: event.clone(),
                matcher,
                script: resolve_package_entry(root, script)?,
                timeout_secs,
                config: Value::Null,
            });
        }
    }
    Ok(hooks)
}

/// 解析 `gateway`：`{ "request": [{ "script": "gateway/request.mjs", "timeout": 5 }] }`。
/// 解析 `gateway`：`{ request: [...], response: [...] }`，两种中间件写法相同（script + timeout）。
fn parse_gateway_middlewares(
    root: &std::path::Path,
    value: Option<&Value>,
) -> anyhow::Result<(Vec<ExtensionGatewayMiddleware>, Vec<ExtensionGatewayMiddleware>)> {
    let Some(value) = value else {
        return Ok((Vec::new(), Vec::new()));
    };
    let map = value.as_object().context("gateway 必须是对象：{ \"request\": [...], \"response\": [...] }")?;
    if let Some(key) = map.keys().find(|key| key.as_str() != "request" && key.as_str() != "response") {
        anyhow::bail!("gateway 只支持 request（请求中间件）与 response（响应流中间件），不认识 {key}");
    }
    let parse = |kind: &str| -> anyhow::Result<Vec<ExtensionGatewayMiddleware>> {
        let Some(list) = map.get(kind) else {
            return Ok(Vec::new());
        };
        let items = list.as_array().with_context(|| format!("gateway.{kind} 必须是数组"))?;
        items
            .iter()
            .map(|item| {
                let item = item.as_object().with_context(|| format!("gateway.{kind} 的每一项必须是对象"))?;
                let script = item
                    .get("script")
                    .and_then(Value::as_str)
                    .with_context(|| format!("gateway.{kind} 缺少 script"))?;
                let timeout_secs = match item.get("timeout") {
                    None => 5,
                    Some(value) => value
                        .as_u64()
                        .filter(|seconds| (1..=EXTENSION_GATEWAY_MAX_TIMEOUT_SECS).contains(seconds))
                        .with_context(|| {
                            format!("gateway.{kind} 的 timeout 必须是 1~{EXTENSION_GATEWAY_MAX_TIMEOUT_SECS} 秒")
                        })?,
                };
                Ok(ExtensionGatewayMiddleware {
                    script: resolve_package_entry(root, script)?,
                    timeout_secs,
                    config: Value::Null,
                })
            })
            .collect()
    };
    Ok((parse("request")?, parse("response")?))
}

/// 解析 `config`：`{ "minutes": { "type": "number", "title": "时长", "default": 25 } }`。
fn parse_extension_config_schema(value: Option<&Value>) -> anyhow::Result<Vec<ExtensionConfigField>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let map = value.as_object().context("config 必须是对象：{ 键: { type, title, default } }")?;
    if map.len() > 32 {
        anyhow::bail!("config 最多 32 项");
    }
    map.iter()
        .map(|(key, field)| {
            if key.is_empty() || !key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
                anyhow::bail!("配置键 {key:?} 只能包含字母、数字、_、-");
            }
            let field = field.as_object().with_context(|| format!("config.{key} 必须是对象"))?;
            let kind = field.get("type").and_then(Value::as_str).unwrap_or("string").to_string();
            let options = match field.get("options") {
                None => Vec::new(),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|item| match item {
                        Value::Object(object) if object.contains_key("value") => Ok(json!({
                            "value": object["value"],
                            "label": object.get("label").and_then(Value::as_str).map(str::to_owned)
                                .unwrap_or_else(|| object["value"].to_string().trim_matches('"').to_string()),
                        })),
                        Value::String(_) | Value::Number(_) | Value::Bool(_) => Ok(json!({ "value": item, "label": item.to_string().trim_matches('"') })),
                        _ => anyhow::bail!("config.{key}.options 的每一项要么是值，要么是 {{ value, label }}"),
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?,
                Some(_) => anyhow::bail!("config.{key}.options 必须是数组"),
            };
            if !["string", "number", "boolean", "select"].contains(&kind.as_str()) {
                anyhow::bail!("config.{key}.type 只能是 string、number、boolean、select");
            }
            if kind == "select" && options.is_empty() {
                anyhow::bail!("config.{key} 是 select，必须给 options");
            }
            let mut result = ExtensionConfigField {
                key: key.clone(),
                kind: kind.clone(),
                title: field.get("title").and_then(Value::as_str).unwrap_or(key).to_string(),
                description: field.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
                default: Value::Null,
                options,
                secret: field.get("secret").and_then(Value::as_bool).unwrap_or(false),
            };
            let fallback = match kind.as_str() {
                "number" => json!(0),
                "boolean" => json!(false),
                "select" => result.options[0]["value"].clone(),
                _ => json!(""),
            };
            result.default = match field.get("default") {
                Some(value) if result.accepts(value) => value.clone(),
                Some(_) => anyhow::bail!("config.{key}.default 与 type 不符"),
                None => fallback,
            };
            Ok(result)
        })
        .collect()
}

/// 解析 `tools`：`[{ "name": "lookup", "description": "…", "inputSchema": {…}, "script": "tools/lookup.mjs", "timeout": 30 }]`。
fn parse_extension_tools(
    root: &std::path::Path,
    extension_id: &str,
    value: Option<&Value>,
) -> anyhow::Result<Vec<ExtensionTool>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value.as_array().context("tools 必须是数组")?;
    let mut names = std::collections::BTreeSet::new();
    items
        .iter()
        .map(|item| {
            let item = item.as_object().context("tools 的每一项必须是对象")?;
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let public = extension_tool_public_name(extension_id, &name);
            if name.is_empty()
                || !name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
                || public.len() > 64
            {
                anyhow::bail!("工具名 {name:?} 只能包含字母、数字、_、-，且「拓展id__工具名」不超过 64 个字符");
            }
            if !names.insert(name.clone()) {
                anyhow::bail!("工具名 {name} 重复");
            }
            let description = item
                .get("description")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .with_context(|| format!("工具 {name} 缺少 description"))?
                .to_string();
            let input_schema = match item.get("inputSchema") {
                None => serde_json::json!({ "type": "object", "properties": {} }),
                Some(schema) if schema.get("type").and_then(Value::as_str) == Some("object") => schema.clone(),
                Some(_) => anyhow::bail!("工具 {name} 的 inputSchema 必须是 type 为 object 的 JSON Schema"),
            };
            let script = item
                .get("script")
                .and_then(Value::as_str)
                .with_context(|| format!("工具 {name} 缺少 script"))?;
            let timeout_secs = match item.get("timeout") {
                None => 30,
                Some(value) => value
                    .as_u64()
                    .filter(|seconds| (1..=EXTENSION_TOOL_MAX_TIMEOUT_SECS).contains(seconds))
                    .with_context(|| format!("工具 {name} 的 timeout 必须是 1~{EXTENSION_TOOL_MAX_TIMEOUT_SECS} 秒"))?,
            };
            Ok(ExtensionTool {
                name,
                description,
                input_schema,
                script: resolve_package_entry(root, script)?,
                timeout_secs,
                config: Value::Null,
            })
        })
        .collect()
}

/// 入口只能指向包内的文件：拒绝绝对路径、`..`，以及经软链逃出包目录的路径。
fn resolve_package_entry(root: &std::path::Path, entry: &str) -> anyhow::Result<PathBuf> {
    let relative = std::path::Path::new(entry);
    if entry.trim().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        anyhow::bail!("renderer 入口 {entry} 必须是包内的相对路径");
    }
    let resolved = root
        .join(relative)
        .canonicalize()
        .with_context(|| format!("renderer 入口 {entry} 不存在"))?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        anyhow::bail!("renderer 入口 {entry} 必须是包内的文件");
    }
    Ok(resolved)
}

/// 包的各个入口按清单顺序拼进同一个作用域，相当于一次简单打包。
fn package_source(package: &ExtensionPackage) -> String {
    package
        .renderer
        .iter()
        .map(|path| {
            fs::read_to_string(path)
                .unwrap_or_else(|error| format!("throw new Error({});", json!(error.to_string())))
        })
        .collect::<Vec<_>>()
        .join("\n;\n")
}

fn wrap_script(script: &UserScriptFile, source: &str) -> String {
    format!(
        r#"
(() => {{
  const codexPlusIsNodeTestHarness = typeof process === "object" && !!process.versions?.node;
  if (!codexPlusIsNodeTestHarness && (window.top !== window || window.self !== window || !window.electronBridge || !/^app:\/\/\-\//i.test(window.location.href))) return;
  window.__codexPlusUserScripts = window.__codexPlusUserScripts || {{ scripts: {{}} }};
  const key = {key};
  window.__codexPlusUserScripts.currentKey = key;
  window.__codexPlusUserScripts.scripts[key] = {{ key, name: {name}, source: {source_name}, manifest: {manifest}, status: "loading", error: "", loadedAt: new Date().toISOString() }};
  try {{
{source}
    window.__codexPlusUserScripts.scripts[key].status = "loaded";
    window.__codexPlusUserScripts.scripts[key].loadedAt = new Date().toISOString();
  }} catch (error) {{
    window.__codexPlusUserScripts.scripts[key].status = "failed";
    window.__codexPlusUserScripts.scripts[key].error = String(error && (error.stack || error.message) || error);
  }} finally {{
    window.__codexPlusUserScripts.currentKey = null;
  }}
}})();
"#,
        key = json!(script.key).to_string(),
        name = json!(script.name).to_string(),
        source_name = json!(script.source).to_string(),
        manifest = script
            .package
            .as_ref()
            .map(ExtensionPackage::renderer_manifest)
            .unwrap_or(Value::Null),
        source = source
    )
}

fn config_from_object(raw: &Map<String, Value>) -> UserScriptConfig {
    let enabled = raw.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    let skip_next_start = raw
        .get("skip_next_start")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let scripts = raw
        .get("scripts")
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_bool()?)))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let market = raw
        .get("market")
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), market_install_from_value(value)?)))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let configs = raw
        .get("configs")
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_object()?.clone())))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    UserScriptConfig {
        enabled,
        scripts,
        skip_next_start,
        market,
        configs,
    }
}

pub fn market_script_filename(id: &str) -> String {
    let sanitized = sanitize_market_id(id);
    format!(
        "market-{}.js",
        if sanitized.is_empty() {
            "script".to_string()
        } else {
            sanitized
        }
    )
}

fn market_install_from_value(value: &Value) -> Option<MarketScriptInstall> {
    let raw = value.as_object()?;
    Some(MarketScriptInstall {
        id: string_field(raw, "id")?,
        name: string_field(raw, "name").unwrap_or_default(),
        version: string_field(raw, "version")?,
        script_url: string_field(raw, "script_url")?,
        homepage: string_field(raw, "homepage").unwrap_or_default(),
        installed_at: string_field(raw, "installed_at").unwrap_or_default(),
    })
}

fn string_field(raw: &Map<String, Value>, key: &str) -> Option<String> {
    raw.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn sanitize_market_id(id: &str) -> String {
    id.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn current_unix_timestamp_string() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod runtime_status_tests {
    use super::*;

    fn target(id: &str, title: &str, url: &str, websocket: &str) -> crate::cdp::CdpTarget {
        crate::cdp::CdpTarget {
            id: id.to_string(),
            target_type: "page".to_string(),
            title: title.to_string(),
            url: url.to_string(),
            web_socket_debugger_url: Some(websocket.to_string()),
        }
    }

    #[test]
    fn extension_api_version_matches_renderer_contract() {
        let api = include_str!("../../../assets/inject/renderer-inject/91-extension-api.js");
        let expected = format!("const codexPlusExtensionApiVersion = {EXTENSION_API_VERSION};");
        assert!(
            api.contains(&expected),
            "EXTENSION_API_VERSION 与 91-extension-api.js 不一致"
        );
    }

    #[test]
    fn parses_live_runtime_status_response() {
        let response = json!({
            "result": { "result": { "value": r#"{"user:test.js":{"status":"loaded","error":""}}"# } }
        });

        let status = parse_live_runtime_status_response(&response).unwrap();

        assert_eq!(status["user:test.js"]["status"], "loaded");
    }

    #[test]
    fn rejects_missing_or_invalid_live_runtime_status() {
        assert!(parse_live_runtime_status_response(&json!({})).is_err());
        assert!(
            parse_live_runtime_status_response(&json!({
                "result": { "result": { "value": "not json" } }
            }))
            .is_err()
        );
        assert!(
            parse_live_runtime_status_response(&json!({
                "result": { "result": { "value": "[]" } }
            }))
            .is_err()
        );
    }

    #[test]
    fn live_runtime_probe_selects_codex_page_over_manager() {
        let targets = vec![
            target(
                "manager",
                "Codex++ 管理工具",
                "http://127.0.0.1:1420/",
                "ws://manager",
            ),
            target("main", "Codex", "app://-/index.html", "ws://main"),
        ];

        let selected = live_runtime_target(&targets).unwrap();

        assert_eq!(selected.id, "main");
    }
}
