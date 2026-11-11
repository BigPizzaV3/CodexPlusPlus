use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

#[derive(Debug, thiserror::Error)]
#[error("用户脚本热重载请求超时，执行结果尚未确认，请检查实际效果后再操作。")]
pub struct ReloadOutcomeUnknown;

/// 超时后本地清单只能说明配置，不能确认页面里的运行状态。
pub fn mark_inventory_runtime_unknown(inventory: &mut Value) {
    if let Some(scripts) = inventory["scripts"].as_array_mut() {
        for script in scripts {
            script["status"] = json!("unknown");
        }
    }
}

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

/// 拓展配置改了以后先试着原地生效：页面里这个拓展的所有实例都用 `ctx.onConfigUpdate` 接住新配置时返回 `"applied"`，
/// 不必整体热重载；任何一个实例没接（或页面不支持）时返回 `"reload"`，由调用方照常热重载。
pub async fn update_live_config(debug_port: u16, manager: &UserScriptManager, key: &str) -> anyhow::Result<String> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let target = live_runtime_target(&targets)?;
    let websocket = target.web_socket_debugger_url.as_deref().context("Codex renderer has no WebSocket URL")?;
    let config = manager.effective_extension_config(key)?;
    let expression = format!(
        "(() => {{ const runtime = window.__codexPlusUserScripts; return typeof runtime?.updateConfig === \"function\" ? runtime.updateConfig({}, {}) : \"reload\"; }})()",
        serde_json::to_string(key)?,
        config
    );
    let response = crate::bridge::evaluate_script(websocket, &expression).await?;
    Ok(response.pointer("/result/result/value").and_then(Value::as_str).unwrap_or("reload").to_string())
}

pub async fn reload_scripts_at(
    websocket: &str,
    manager: &UserScriptManager,
) -> anyhow::Result<Value> {
    let bundle = manager.build_reload_bundle()?;
    // 拓展重载后旧后台的方法表、配置都过期了：先全部停掉，下次调用按新代码重起。
    crate::extension_backend::stop_all(manager).await;
    apply_scripts_at(websocket, manager, &bundle, Some(Duration::from_secs(15))).await
}

pub async fn load_scripts_at(
    websocket: &str,
    manager: &UserScriptManager,
) -> anyhow::Result<Value> {
    // 页面重新加载：上一个页面的后台不再有人用。
    crate::extension_backend::stop_all(manager).await;
    apply_scripts_at(websocket, manager, &manager.build_initial_bundle()?, None).await
}

async fn apply_scripts_at(
    websocket: &str,
    manager: &UserScriptManager,
    bundle: &str,
    reload_timeout: Option<Duration>,
) -> anyhow::Result<Value> {
    let response = if let Some(timeout) = reload_timeout {
        crate::bridge::evaluate_script_with_await_promise_timeout(websocket, bundle, false, timeout)
            .await
            .map_err(|error| {
                if error.is::<crate::bridge::CdpCommandTimeout>() {
                    error.context(ReloadOutcomeUnknown)
                } else {
                    error
                }
            })?
    } else {
        crate::bridge::evaluate_script(websocket, bundle).await?
    };
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
    /// Claude 的单项开关独立保存；空表也要写入，区分尚未迁移的旧配置。
    pub claude_scripts: BTreeMap<String, bool>,
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
            claude_scripts: BTreeMap::new(),
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
    /// 后台设置文件。内置拓展里有对应设置字段的，启停就读写这里（见 [`Self::with_settings_path`]）。
    settings_path: Option<PathBuf>,
    /// 随 Codex++ 发布的官方拓展包所在目录（来源 `bundled`），见 [`crate::bundled_extensions`]。
    bundled_dir: Option<PathBuf>,
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
            settings_path: None,
            bundled_dir: None,
        }
    }

    /// 指定官方拓展包目录（来源 `bundled`，脚本 key 为 `bundled:<目录名>`）。只扫描，不在这里写文件。
    pub fn with_bundled_dir(mut self, directory: impl Into<PathBuf>) -> Self {
        self.bundled_dir = Some(directory.into());
        self
    }

    /// 原来的内置桌宠改成了官方拓展包「Agent 桌宠」：第一次见到它时，把设置里的 `agentPet` 与
    /// `codexAppWhaleBalance*` 换算成它的配置，启停也照原来的开关。已经迁移过（配置或开关里有它）就不再动。
    pub fn migrate_bundled_agent_pet(&self, settings: &crate::settings::BackendSettings) -> anyhow::Result<bool> {
        self.migrate_bundled(
            crate::bundled_extensions::AGENT_PET_KEY,
            crate::bundled_extensions::agent_pet_config_from_settings(settings),
            settings.agent_pet.enabled_for("codex"),
        )
    }

    /// 原来的打字特效、背景图、用量状态栏、请求状态条也改成了官方拓展包：第一次见到时按旧设置写入配置与启停，迁移过就不再动。
    pub fn migrate_bundled_builtins(&self, settings: &crate::settings::BackendSettings) -> anyhow::Result<()> {
        let (config, enabled) = crate::bundled_extensions::typing_effects_migration(settings);
        self.migrate_bundled(crate::bundled_extensions::TYPING_EFFECTS_KEY, config, enabled)?;
        let (config, enabled) = crate::bundled_extensions::image_overlay_migration(settings);
        self.migrate_bundled(crate::bundled_extensions::IMAGE_OVERLAY_KEY, config, enabled)?;
        let (config, enabled) = crate::bundled_extensions::usage_statusline_migration(settings);
        self.migrate_bundled(crate::bundled_extensions::USAGE_STATUSLINE_KEY, config, enabled)?;
        let (config, enabled) = crate::bundled_extensions::proxy_request_status_migration(settings);
        self.migrate_bundled(crate::bundled_extensions::PROXY_REQUEST_STATUS_KEY, config, enabled)?;
        Ok(())
    }

    /// 官方拓展包的一次性迁移：配置与开关里都还没有它时写入；返回是否写了。
    fn migrate_bundled(&self, key: &str, values: Map<String, Value>, enabled: bool) -> anyhow::Result<bool> {
        let _guard = self.config_lock.lock().unwrap();
        let mut config = self.load_config_unlocked();
        if config.configs.contains_key(key) || config.scripts.contains_key(key) {
            return Ok(false);
        }
        config.configs.insert(key.to_string(), values);
        config.scripts.insert(key.to_string(), enabled);
        self.save_config_unlocked(&config)?;
        Ok(true)
    }

    /// 指定后台设置文件：有设置字段的内置拓展（会话删除、粘贴修复……）启停时读写它，
    /// 和管理器增强页、Codex 设置页是同一个开关。没指定时按设置默认值读、不能切换。
    pub fn with_settings_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.settings_path = Some(path.into());
        self
    }

    pub(crate) fn settings_path(&self) -> Option<&std::path::Path> {
        self.settings_path.as_deref()
    }

    /// 当前设置的 JSON 形态（读不到时用默认设置）。
    fn settings_value(&self) -> Value {
        let settings = match &self.settings_path {
            Some(path) => crate::settings::SettingsStore::new(path.clone()).load().unwrap_or_default(),
            None => crate::settings::BackendSettings::default(),
        };
        serde_json::to_value(settings).unwrap_or_default()
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
        let mut config = config_from_object(&raw);
        if !raw.contains_key("claude_scripts") {
            config.claude_scripts = config.scripts.clone();
            // 旧版的整体开关也约束 Claude；迁移后保留当时各项的关闭状态。
            if !config.enabled {
                if let Ok(scripts) = self.scan_script_files(&config) {
                    for script in scripts {
                        if script.package.as_ref().is_some_and(|package| package.claude_desktop.is_some()) {
                            config.claude_scripts.insert(script.key, false);
                        }
                    }
                }
            }
        }
        config
    }

    pub fn save_config(&self, config: &UserScriptConfig) -> anyhow::Result<()> {
        let _guard = self.config_lock.lock().unwrap();
        self.save_config_unlocked(config)
    }

    fn save_config_unlocked(&self, config: &UserScriptConfig) -> anyhow::Result<()> {
        crate::extension_backend::invalidate_scan_cache();
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
        // 有设置字段的内置拓展：开关就是那个设置项，写设置，不在用户脚本配置里另记一份
        // （顺手清掉旧版本记下的停用标记，免得它在没有入口的地方一直压着功能）。
        let setting = key
            .strip_prefix("builtin:")
            .and_then(crate::settings_catalog::builtin_extension_def)
            .and_then(|def| def.setting.as_deref().map(|setting| (def, setting)));
        if let Some((def, setting)) = setting {
            let path = self.settings_path.clone().context("没有设置文件，不能切换这个内置拓展")?;
            let value = if enabled { def.on_value.clone() } else { def.off_value.clone() };
            let patch = setting.rsplit('.').fold(value, |inner, segment| json!({ segment: inner }));
            crate::settings::SettingsStore::new(path).update(patch)?;
            if config.scripts.remove(key).is_some() {
                self.save_config_unlocked(&config)?;
            }
            return Ok(config);
        }
        config.scripts.insert(key.to_string(), enabled);
        self.save_config_unlocked(&config)?;
        Ok(config)
    }

    /// Claude 只接受显式适配的拓展包；不改 Codex 的单项开关或设置字段。
    pub fn set_claude_script_enabled(&self, key: &str, enabled: bool) -> anyhow::Result<UserScriptConfig> {
        let _guard = self.config_lock.lock().unwrap();
        let mut config = self.load_config_unlocked();
        let script = self.scan_script_files(&config)?
            .into_iter()
            .find(|script| script.key == key)
            .with_context(|| format!("没有找到拓展 {key}"))?;
        let package = script.package.context("Claude Desktop 只支持显式适配的拓展包")?;
        if package.claude_desktop.is_none() {
            anyhow::bail!("拓展没有声明 claudeDesktop");
        }
        if enabled {
            if let Some(error) = package.error {
                anyhow::bail!("拓展包清单有误：{error}");
            }
        }
        config.claude_scripts.insert(key.to_string(), enabled);
        self.save_config_unlocked(&config)?;
        Ok(config)
    }

    pub fn delete_user_script(&self, key: &str) -> anyhow::Result<UserScriptConfig> {
        self.delete_user_script_with_trash(key, &default_extension_trash_dir())
    }

    /// 同 [`Self::delete_user_script`]，拓展包（文件夹）移进 `trash_dir`。测试用它指定临时废纸篓。
    pub fn delete_user_script_with_trash(
        &self,
        key: &str,
        trash_dir: &std::path::Path,
    ) -> anyhow::Result<UserScriptConfig> {
        let Some(file_name) = key.strip_prefix("user:").filter(|value| !value.is_empty()) else {
            anyhow::bail!("only user scripts can be deleted");
        };
        if file_name.contains(['/', '\\']) || file_name == "." || file_name == ".." {
            anyhow::bail!("invalid user script key");
        }
        let path = self.user_dir.join(file_name);
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
            if canonical_path.is_dir() {
                // 拓展包是文件夹（#2919）：不做递归删除，整个移进废纸篓，可以找回。
                // 符号链接指向目录时 canonicalize 已解开，上面的目录归属检查同样适用。
                crate::gateway::sessions::move_to_trash(&canonical_path, trash_dir).with_context(|| {
                    format!("拓展包移进废纸篓失败：{}", canonical_path.display())
                })?;
            } else {
                fs::remove_file(&canonical_path).with_context(|| {
                    format!("failed to delete user script {}", canonical_path.display())
                })?;
            }
        }

        let _guard = self.config_lock.lock().unwrap();
        let mut config = self.load_config_unlocked();
        config.scripts.remove(key);
        config.claude_scripts.remove(key);
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
    /// 管理器面板：拓展包声明的 `manager.page` 的标题与 HTML 内容（拓展没启用或清单有误时报错）。
    pub fn extension_manager_page(&self, key: &str) -> anyhow::Result<(String, String)> {
        let config = self.load_config();
        let script = self
            .scan_script_files(&config)?
            .into_iter()
            .find(|script| script.key == key)
            .with_context(|| format!("没有找到拓展包 {key}"))?;
        if !script.enabled {
            anyhow::bail!("拓展已停用");
        }
        let package = script.package.with_context(|| format!("{key} 不是拓展包"))?;
        if let Some(error) = &package.error {
            anyhow::bail!("拓展包清单有误：{error}");
        }
        let page = package.manager_page.with_context(|| format!("拓展 {key} 没有声明管理器面板（manager.page）"))?;
        let (path, title) = (page.path, page.title);
        let html = fs::read_to_string(&path).with_context(|| format!("读取 {} 失败", path.display()))?;
        Ok((title, html))
    }

    /// 拓展自己的设置页（`settings.page`）的 HTML。拓展停用时也能打开：设置不依赖它正在运行。
    pub fn extension_settings_page(&self, key: &str) -> anyhow::Result<(String, String)> {
        let package = self.extension_package(key)?;
        let path = package.settings_page.clone().with_context(|| format!("拓展 {key} 没有声明设置页（settings.page）"))?;
        let html = fs::read_to_string(&path).with_context(|| format!("读取 {} 失败", path.display()))?;
        Ok((package.name.clone().unwrap_or_else(|| package.id.clone()), html))
    }
    /// 拓展的 README（Codex「拓展」详情页的「细节」页签）：包目录里的 README，优先中文版；单文件脚本没有 README。
    /// 回 `{status, markdown, file, truncated, assets}`。`assets` 是正文里引用到的包内 `assets/` 资源（路径 → data URL），
    /// 页面据此显示相对路径的图片；清单有误的包也照样给 README，方便看说明排查。
    pub fn extension_readme(&self, key: &str) -> anyhow::Result<Value> {
        let config = self.load_config();
        let script = self
            .scan_script_files(&config)?
            .into_iter()
            .find(|script| script.key == key)
            .with_context(|| format!("没有找到拓展 {key}"))?;
        let package = script.package.as_ref();
        let Some(path) = package.and_then(|package| package.root.as_deref()).and_then(find_extension_readme) else {
            return Ok(json!({ "status": "ok", "markdown": "", "file": "", "truncated": false, "assets": {} }));
        };
        let bytes = fs::read(&path).with_context(|| format!("读取 {} 失败", path.display()))?;
        let truncated = bytes.len() > EXTENSION_README_MAX_BYTES;
        let markdown = String::from_utf8_lossy(&bytes[..bytes.len().min(EXTENSION_README_MAX_BYTES)]).into_owned();
        let assets = package
            .map(|package| {
                package
                    .assets
                    .iter()
                    .filter(|(relative, _)| markdown.contains(relative.as_str()))
                    .map(|(relative, url)| (relative.clone(), Value::String(url.clone())))
                    .collect::<Map<String, Value>>()
            })
            .unwrap_or_default();
        Ok(json!({
            "status": "ok",
            "markdown": markdown,
            "file": path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
            "truncated": truncated,
            "assets": assets,
        }))
    }
    /// 清单无误的拓展包（不论启停）。
    fn extension_package(&self, key: &str) -> anyhow::Result<ExtensionPackage> {
        let config = self.load_config();
        let package = self
            .scan_script_files(&config)?
            .into_iter()
            .find(|script| script.key == key)
            .and_then(|script| script.package)
            .with_context(|| format!("没有找到拓展包 {key}"))?;
        if let Some(error) = &package.error {
            anyhow::bail!("拓展包清单有误：{error}");
        }
        Ok(package)
    }
    /// 管理器数据读操作在宿主侧复核清单，不信任页面传来的数据名或旧的清单快照。
    pub fn authorize_manager_data(&self, key: &str, name: &str) -> anyhow::Result<()> {
        if !MANAGER_DATA_SOURCES.contains(&name) {
            anyhow::bail!("未开放的管理器数据：{name}");
        }
        let config = self.load_config();
        if !config.enabled {
            anyhow::bail!("拓展已停用");
        }
        let script = self.scan_script_files(&config)?.into_iter()
            .find(|script| script.key == key && script.enabled)
            .with_context(|| format!("没有找到已启用的拓展包 {key}"))?;
        let package = script.package.context("只有拓展包可以读取管理器数据")?;
        if let Some(error) = &package.error {
            anyhow::bail!("拓展包清单有误：{error}");
        }
        let page = package.manager_page.context("拓展没有声明管理器面板")?;
        if !page.data.iter().any(|item| item == name) {
            anyhow::bail!("清单 manager.data 没有声明 {name}");
        }
        Ok(())
    }

    /// 拓展的设置页 / 管理器面板经 `codexPlusManager.call(route, payload)` 调数据路由：
    /// 只认 [`EXTENSION_PAGE_ROUTES`] 里的、且这个拓展在清单 `permissions.routes` 里声明过的。
    pub async fn extension_page_route(&self, key: &str, route: &str, payload: &Value) -> anyhow::Result<Value> {
        if !EXTENSION_PAGE_ROUTES.contains(&route) {
            anyhow::bail!("设置页不能调用路由 {route}");
        }
        let package = self.extension_package(key)?;
        if !package.routes.iter().any(|item| item == route) {
            anyhow::bail!("拓展没有在清单 permissions.routes 里声明 {route}");
        }
        crate::agent_pet::route(route, payload, &crate::agent_pet::default_cache_dir(), &crate::agent_pet::default_custom_dir())
            .await
            .with_context(|| format!("设置页不能调用路由 {route}"))
    }
    /// 拓展包 `assets/` 里的资源（data URL），给管理器面板的 `codexPlusManager.asset(path)`。
    pub fn extension_asset(&self, key: &str, path: &str) -> anyhow::Result<String> {
        self.extension_asset_for(key, path, true)
    }
    /// 同上；`require_enabled` 为 false 时停用的拓展也能取（它的设置页用）。
    pub fn extension_asset_for(&self, key: &str, path: &str, require_enabled: bool) -> anyhow::Result<String> {
        let config = self.load_config();
        let package = self
            .scan_script_files(&config)?
            .into_iter()
            .find(|script| script.key == key && (script.enabled || !require_enabled))
            .and_then(|script| script.package)
            .with_context(|| format!("没有找到已启用的拓展包 {key}"))?;
        let normalized = path.trim().trim_start_matches("./");
        let normalized = normalized.strip_prefix("assets/").unwrap_or(normalized);
        // 资源按包内路径登记（assets/<文件>）；写不写 assets/ 前缀都认。
        package
            .assets
            .get(&format!("assets/{normalized}"))
            .cloned()
            .with_context(|| format!("拓展包里没有资源 {path}"))
    }

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

    /// 一个拓展包的生效配置（默认值叠加已保存且类型正确的值）。
    pub fn effective_extension_config(&self, key: &str) -> anyhow::Result<Value> {
        let config = self.load_config();
        let package = self
            .scan_script_files(&config)?
            .into_iter()
            .find(|script| script.key == key)
            .and_then(|script| script.package)
            .with_context(|| format!("没有找到拓展包 {key}"))?;
        Ok(resolve_extension_config(&package.config_schema, config.configs.get(key)))
    }

    /// 对话里改配置：只覆盖给出的键，其余沿用已存值；secret 项传回打码值时保留原值。
    pub fn update_extension_config(&self, key: &str, values: &Map<String, Value>) -> anyhow::Result<Value> {
        let config = self.load_config();
        let schema = self
            .scan_script_files(&config)?
            .into_iter()
            .find(|script| script.key == key)
            .and_then(|script| script.package)
            .map(|package| package.config_schema)
            .unwrap_or_default();
        let mut merged = config.configs.get(key).cloned().unwrap_or_default();
        // 已存值里已不在声明中的键丢掉，免得校验报「未声明」；新给的未声明键照常报错。
        merged.retain(|name, _| schema.iter().any(|field| &field.key == name));
        for (name, value) in values {
            let secret = schema.iter().any(|field| &field.key == name && field.secret);
            if secret && value.as_str() == Some(SECRET_MASK) {
                continue;
            }
            merged.insert(name.clone(), value.clone());
        }
        self.set_extension_config(key, &merged)
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

    /// Claude 宿主总开关独立于 Codex，旧配置默认不接入。
    pub fn claude_extensions_enabled(&self) -> bool {
        self.settings_value().get("claudeAppExtensions").and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn set_claude_extensions_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let path = self.settings_path.clone().context("没有设置文件，不能切换 Claude Desktop 拓展")?;
        crate::settings::SettingsStore::new(path).update(json!({ "claudeAppExtensions": enabled }))?;
        Ok(())
    }

    /// 只下发 Claude Mods 的声明；不包含配置值、后台脚本或其它本机路径。
    pub fn enabled_claude_extensions(&self) -> anyhow::Result<Vec<crate::extension_claude::EnabledClaudeExtension>> {
        let config = self.load_config();
        if !self.claude_extensions_enabled() {
            return Ok(Vec::new());
        }
        Ok(self.scan_script_files(&config)?
            .into_iter()
            .filter(|script| config.claude_scripts.get(&script.key).copied().unwrap_or(true))
            .filter_map(|script| {
                let package = script.package.filter(|package| package.error.is_none())?;
                Some(crate::extension_claude::EnabledClaudeExtension {
                    key: script.key,
                    name: package.name.unwrap_or_else(|| package.id.clone()),
                    id: package.id,
                    declaration: package.claude_desktop?,
                })
            })
            .collect())
    }

    /// 每次 Claude 调用都重新确认宿主开关、拓展身份与清单方法白名单，不沿用 Codex 的扫描缓存。
    pub(crate) fn enabled_claude_extension_backend(
        &self,
        key: &str,
        method: &str,
    ) -> anyhow::Result<(String, PathBuf, Value)> {
        if !self.claude_extensions_enabled() {
            anyhow::bail!("Claude Desktop 拓展已关闭");
        }
        let config = self.load_config();
        let script = self.scan_script_files(&config)?
            .into_iter()
            .find(|script| script.key == key)
            .with_context(|| format!("没有找到拓展 {key}"))?;
        if !config.claude_scripts.get(key).copied().unwrap_or(true) {
            anyhow::bail!("拓展 {key} 已停用");
        }
        let package = script.package.with_context(|| format!("{key} 不是拓展包"))?;
        if let Some(error) = package.error {
            anyhow::bail!("拓展包清单有误：{error}");
        }
        let declaration = package.claude_desktop.context("拓展没有声明 claudeDesktop")?;
        if !declaration.permits(method) {
            anyhow::bail!("Claude Desktop 无权调用拓展 {key} 的方法 {method}");
        }
        Ok((
            package.id,
            package.backend.context("Claude Desktop 拓展没有声明 backend")?,
            package.config,
        ))
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

    /// 启用中的拓展关掉的内置网关环节：(拓展 id, 环节名)。
    pub fn enabled_gateway_replaces(&self) -> anyhow::Result<Vec<(String, String)>> {
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
            .flat_map(|package| package.gateway_replaces.iter().map(|name| (package.id.clone(), name.clone())).collect::<Vec<_>>())
            .collect())
    }

    /// 启用中、清单无误的拓展声明的模型供应商：(拓展 id, 供应商)，Key 已按配置解析好。
    pub fn enabled_extension_providers(&self) -> anyhow::Result<Vec<(String, ExtensionProvider)>> {
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
                package.providers.into_iter().map(move |mut provider| {
                    if let Some(key) = &provider.api_key_config {
                        provider.api_key = config.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
                    }
                    (id.clone(), provider)
                })
            })
            .collect())
    }

    /// 启用中、清单无误、声明了技能 / 子 agent / MCP 服务器 / 全局指令的拓展包。
    pub fn enabled_codex_assets(&self) -> anyhow::Result<Vec<crate::extension_codex::EnabledCodexAssets>> {
        let config = self.load_config();
        if !config.enabled {
            return Ok(Vec::new());
        }
        Ok(self
            .scan_script_files(&config)?
            .into_iter()
            .filter(|script| script.enabled)
            .filter_map(|script| script.package)
            .filter(|package| package.error.is_none() && !package.codex.is_empty())
            .filter_map(|package| {
                Some(crate::extension_codex::EnabledCodexAssets {
                    name: package.name.clone().unwrap_or_else(|| package.id.clone()),
                    id: package.id,
                    root: package.root?,
                    config: package.config,
                    assets: package.codex,
                })
            })
            .collect())
    }

    /// 这个拓展（启用中、清单无误、声明了 backend.http）是否对外处理 HTTP。
    pub fn extension_backend_http(&self, key: &str) -> bool {
        let config = self.load_config();
        config.enabled
            && self.scan_script_files(&config).ok().into_iter().flatten().any(|script| {
                script.key == key
                    && script.enabled
                    && script.package.as_ref().is_some_and(|package| package.error.is_none() && package.backend.is_some() && package.backend_http)
            })
    }

    /// 启用中、清单无误的拓展的定时任务：(拓展 key, 任务)。
    pub fn enabled_extension_schedules(&self) -> anyhow::Result<Vec<(String, ExtensionSchedule)>> {
        let config = self.load_config();
        if !config.enabled {
            return Ok(Vec::new());
        }
        Ok(self
            .scan_script_files(&config)?
            .into_iter()
            .filter(|script| script.enabled)
            .filter_map(|script| {
                let package = script.package.filter(|package| package.error.is_none() && package.backend.is_some())?;
                Some(package.schedules.into_iter().map(move |schedule| (script.key.clone(), schedule)))
            })
            .flatten()
            .collect())
    }

    /// 启用中的后台服务：服务名 → 提供它的拓展 key。同名时先扫描到的（按 key 顺序）生效。
    pub fn enabled_backend_services(&self) -> anyhow::Result<Vec<(String, String)>> {
        let config = self.load_config();
        if !config.enabled {
            return Ok(Vec::new());
        }
        let mut services: Vec<(String, String)> = Vec::new();
        for script in self.scan_script_files(&config)? {
            if !script.enabled {
                continue;
            }
            let Some(package) = script.package.filter(|package| package.error.is_none() && package.backend.is_some()) else {
                continue;
            };
            for name in package.backend_provides {
                if !services.iter().any(|(existing, _)| *existing == name) {
                    services.push((name, script.key.clone()));
                }
            }
        }
        Ok(services)
    }

    /// 启用中、清单无误的拓展包声明的失败重试中间件（gateway.error），附带拓展 id。
    pub fn enabled_gateway_error_middlewares(&self) -> anyhow::Result<Vec<(String, ExtensionGatewayMiddleware)>> {
        self.enabled_gateway_list(|package| &package.gateway_error)
    }

    /// 启用中、清单无误的拓展包声明的工具结果改写（toolPipeline.postExecute），附带拓展 id。
    pub fn enabled_tool_post_execute(&self) -> anyhow::Result<Vec<(String, ExtensionHook)>> {
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
                package.tool_post_execute.into_iter().map(move |mut hook| {
                    hook.config = config.clone();
                    (id.clone(), hook)
                })
            })
            .collect())
    }

    /// 拓展的工具代执行结果存放目录（钩子进程写、网关读）。
    pub fn tool_exec_dir(&self) -> PathBuf {
        self.user_dir
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| self.user_dir.clone())
            .join("extension-data")
            .join(".tool-exec")
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

    /// 用户脚本配置文件（启停、配置值都存在这里）。
    pub fn config_path(&self) -> &std::path::Path {
        &self.config_path
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
        crate::extension_backend::invalidate_scan_cache();
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
        let max_file = write.max_file_bytes.unwrap_or(EXTENSION_WRITE_MAX_FILE_BYTES);
        let max_total = EXTENSION_WRITE_MAX_TOTAL_BYTES.max(max_file.saturating_mul(2));
        let mut total = 0usize;
        let mut copied_total = 0usize;
        for (path, content) in &write.files {
            validate_package_relative_path(path)?;
            if path.eq_ignore_ascii_case(EXTENSION_MANIFEST_FILE) {
                anyhow::bail!("{EXTENSION_MANIFEST_FILE} 由清单参数生成，不能直接写");
            }
            if write.copied_text.contains(path) {
                if content.len() > EXTENSION_COPIED_TEXT_MAX_BYTES {
                    anyhow::bail!("{path} 超过复制文件上限 {} MiB", EXTENSION_COPIED_TEXT_MAX_BYTES / 1024 / 1024);
                }
                copied_total += content.len();
                continue;
            }
            if content.len() > max_file {
                anyhow::bail!("{path} 超过单文件上限 {} KiB；打包产物等大文件先写到本机，再用 sourcePath 复制进包", max_file / 1024);
            }
            total += content.len();
        }
        if total > max_total {
            anyhow::bail!("文件总量超过 {} KiB", max_total / 1024);
        }
        if copied_total > EXTENSION_COPIED_TEXT_MAX_TOTAL_BYTES {
            anyhow::bail!("复制进包的文本文件合计超过 {} MiB", EXTENSION_COPIED_TEXT_MAX_TOTAL_BYTES / 1024 / 1024);
        }
        // 没指定页面入口时：有 renderer.js 就用它；只声明 hooks 的拓展可以没有页面入口。
        let renderer = if !write.renderer.is_empty() {
            write.renderer.clone()
        } else if write.files.contains_key("renderer.js")
            || (write.hooks.is_none() && write.gateway.is_none() && write.tools.is_none() && write.backend.is_none() && write.tool_pipeline.is_none() && write.providers.is_none())
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
        if let Some(pipeline) = &write.tool_pipeline {
            manifest["toolPipeline"] = pipeline.clone();
        }
        if let Some(providers) = &write.providers {
            manifest["providers"] = providers.clone();
        }
        for (key, item) in &write.extra {
            if matches!(key.as_str(), "skills" | "agents" | "mcpServers" | "instructions" | "schedules" | "prompts" | "resources" | "manager" | "preload" | "launch" | "replaces" | "instances" | "dsh" | "claudeDesktop") {
                manifest[key.as_str()] = item.clone();
            }
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
            "builtins": self.builtin_extensions()?,
            "enabled": config.enabled,
            "claude_enabled": self.claude_extensions_enabled(),
            "skip_next_start": config.skip_next_start,
            "builtin_dir": self.builtin_dir.to_string_lossy(),
            "user_dir": self.user_dir.to_string_lossy(),
            "scripts": scripts
        }))
    }

    /// 启用中、清单无误的拓展包声明的页面最早期脚本：(脚本 key, 拓展 id, 生效配置, 脚本)，以及有没有拓展要求启动后重载一次。
    /// 用户脚本总开关关着、或「下次启动跳过拓展」开着时没有（不消费跳过标记，那由页面注入包消费）。
    pub fn enabled_preloads(&self) -> anyhow::Result<(Vec<(String, String, Value, Vec<PathBuf>)>, bool)> {
        let config = self.load_config();
        if !config.enabled || config.skip_next_start {
            return Ok((Vec::new(), false));
        }
        let mut reload = false;
        let entries = self
            .scan_script_files(&config)?
            .into_iter()
            .filter(|script| script.enabled)
            .filter_map(|script| {
                let package = script.package.filter(|package| package.error.is_none() && !package.preload.scripts.is_empty())?;
                reload |= package.preload.reload_on_launch;
                Some((script.key, package.id, package.config, package.preload.scripts))
            })
            .collect();
        Ok((entries, reload))
    }

    /// 启用中、清单无误的拓展包声明的启动前后：(拓展 id, 生效配置, launch)。
    pub fn enabled_launches(&self) -> anyhow::Result<Vec<(String, Value, crate::extension_launch::ExtensionLaunch)>> {
        let config = self.load_config();
        if !config.enabled || config.skip_next_start {
            return Ok(Vec::new());
        }
        Ok(self
            .scan_script_files(&config)?
            .into_iter()
            .filter(|script| script.enabled)
            .filter_map(|script| script.package)
            .filter(|package| package.error.is_none() && !package.launch.is_empty())
            .map(|package| (package.id, package.config, package.launch))
            .collect())
    }

    /// 有没有启用中、清单无误的拓展包在 `permissions.routes` 里声明了这条路由（用户脚本总开关关着时没有）。
    pub fn route_declared(&self, route: &str) -> bool {
        let config = self.load_config();
        if !config.enabled {
            return false;
        }
        self.scan_script_files(&config).is_ok_and(|scripts| {
            scripts.into_iter().any(|script| {
                script.enabled
                    && script.package.is_some_and(|package| package.error.is_none() && package.routes.iter().any(|item| item == route))
            })
        })
    }

    /// 声明了 `dsh` 的拓展包：(脚本 key, 拓展 id, dsh 插件目录, 是否启用, 版本)。不合法的包跳过。
    pub fn extension_dsh_plugins(&self) -> anyhow::Result<Vec<(String, String, PathBuf, bool, String)>> {
        let config = self.load_config();
        Ok(self
            .scan_script_files(&config)?
            .into_iter()
            .filter_map(|script| {
                let package = script.package.as_ref().filter(|package| package.error.is_none())?;
                let root = package.dsh.clone()?;
                Some((script.key.clone(), package.id.clone(), root, config.enabled && script.enabled, package.version.clone()))
            })
            .collect())
    }

    /// 内置拓展的停用 / 顶替状态：`{ disabled: [id], replacedBy: { id: 拓展 key } }`。
    /// `disabled` 只含没有设置字段、在用户脚本配置里停用的项；有设置字段的由功能自己读设置判断，
    /// 页面上切设置立刻生效，不必等注入包重发。
    pub fn builtin_extension_state(&self) -> anyhow::Result<Value> {
        let config = self.load_config();
        let disabled = crate::settings_catalog::builtin_extension_defs()
            .iter()
            .filter(|def| def.setting.is_none())
            .map(|def| def.id.as_str())
            .filter(|id| config.scripts.get(&builtin_extension_key(id)) == Some(&false))
            .collect::<Vec<_>>();
        let mut replaced_by = Map::new();
        if config.enabled {
            for script in self.scan_script_files(&config)? {
                let Some(package) = script.package.as_ref().filter(|package| script.enabled && package.error.is_none()) else {
                    continue;
                };
                for id in &package.replaces {
                    replaced_by.entry(id.clone()).or_insert_with(|| Value::String(script.key.clone()));
                }
            }
        }
        Ok(json!({ "disabled": disabled, "replacedBy": replaced_by }))
    }

    /// 内置拓展是否在用（没被停用、也没被启用的拓展包顶替）。给不在页面里跑的内置功能用，例如启动器驱动的桌宠跟随鼠标；
    /// 读不到配置时按启用算，和页面侧「不在停用集合里就算启用」一致。
    pub fn builtin_extension_active(&self, id: &str) -> bool {
        let Ok(state) = self.builtin_extension_state() else {
            return true;
        };
        let disabled = state["disabled"].as_array().is_some_and(|list| list.iter().any(|item| item == id));
        !disabled && state["replacedBy"].get(id).is_none()
    }

    /// 内置拓展列表（给管理器）：名称、说明、分组、是否启用、被哪个拓展顶替、对应的设置字段与子选项。
    /// 有设置字段的，「是否启用」就是那个设置项的值。
    pub fn builtin_extensions(&self) -> anyhow::Result<Value> {
        let state = self.builtin_extension_state()?;
        let settings = self.settings_value();
        Ok(Value::Array(
            crate::settings_catalog::builtin_extension_defs()
                .iter()
                .map(|def| {
                    let id = def.id.as_str();
                    let enabled = match &def.setting {
                        Some(setting) => def.is_on(setting.split('.').try_fold(&settings, |value, segment| value.get(segment))),
                        None => !state["disabled"].as_array().is_some_and(|list| list.iter().any(|item| item == id)),
                    };
                    json!({
                        "id": id,
                        "key": builtin_extension_key(id),
                        "name": def.name,
                        "description": def.description,
                        "group": def.group,
                        "enabled": enabled,
                        "replacedBy": state["replacedBy"].get(id).cloned().unwrap_or(Value::Null),
                        "setting": def.setting,
                        "params": def.params,
                        "restartRequired": def.restart,
                        "platforms": def.platforms,
                        "masterGated": def.master_gated,
                        "replaceable": def.replaceable,
                    })
                })
                .collect(),
        ))
    }

    pub fn build_enabled_bundle(&self) -> anyhow::Result<String> {
        let config = self.load_config();
        // 内置拓展的停用 / 顶替状态总要下发（用户脚本总开关关着也一样），页面据此装上 / 卸下内置功能。
        let builtin_state = format!(
            "window.__codexPlusBuiltinExtensionState = {}; try {{ window.__codexPlusSyncBuiltinExtensions?.(); }} catch {{}}",
            self.builtin_extension_state().unwrap_or_else(|_| json!({}))
        );
        if !config.enabled {
            return Ok(builtin_state);
        }
        let mut blocks = vec![builtin_state, RUNTIME_SCRIPT.to_string()];
        // 后台服务清单先交给页面：页面把它们登记成可 inject 的服务（调用经桥接转到提供方后台）。
        let services = self
            .enabled_backend_services()
            .unwrap_or_default()
            .into_iter()
            .map(|(name, key)| json!({ "name": name, "key": key }))
            .collect::<Vec<_>>();
        blocks.push(format!(
            "window.__codexPlusBackendServices = {}; try {{ window.__codexPlusSyncBackendServices?.(); }} catch {{}}",
            Value::Array(services)
        ));
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
                    "claude_enabled": script.package.as_ref()
                        .filter(|package| package.claude_desktop.is_some())
                        .map(|_| config.claude_scripts.get(&script.key).copied().unwrap_or(true)),
                    "replacedBy": script.replaced_by,
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
        if let Some(bundled) = &self.bundled_dir {
            self.append_scripts(crate::bundled_extensions::BUNDLED_SOURCE, bundled, config, &mut scripts)?;
        }
        self.append_scripts("user", &self.user_dir, config, &mut scripts)?;
        // 第三方拓展包的 replaces 写了改成官方拓展包的原内置拓展 id（如 usage-statusline）：
        // 它启用时对应的官方拓展包让位，停用时回来，和顶替内置拓展的效果一致。
        if config.enabled {
            let takeovers = scripts
                .iter()
                .filter(|script| script.enabled && script.source != crate::bundled_extensions::BUNDLED_SOURCE)
                .filter_map(|script| {
                    let package = script.package.as_ref().filter(|package| package.error.is_none())?;
                    Some((script.key.clone(), package))
                })
                .flat_map(|(key, package)| {
                    package
                        .replaces
                        .iter()
                        .filter_map(move |id| crate::bundled_extensions::retired_builtin_key(id).map(|bundled| (bundled, key.clone())))
                })
                .collect::<Vec<_>>();
            for (bundled, replacer) in takeovers {
                if let Some(script) = scripts.iter_mut().find(|script| script.key == bundled && script.replaced_by.is_none()) {
                    script.enabled = false;
                    script.replaced_by = Some(replacer);
                }
            }
        }
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
                replaced_by: None,
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
    /// 官方拓展包被哪个启用中的拓展包顶替（那个包的 `replaces` 写了它接替的原内置拓展 id）；顶替时 `enabled` 为 false。
    replaced_by: Option<String>,
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
    /// 图标（codexpp.json 的 `icon`）：包内 `assets/` 下的图片，或 https 地址。见 [`ExtensionPackage::icon_url`]。
    icon: Option<String>,
    engine: u64,
    routes: Vec<String>,
    renderer: Vec<PathBuf>,
    /// Claude Desktop Mods 的显式接入声明，不作为 Codex renderer 或后台服务登记。
    claude_desktop: Option<crate::extension_claude::ClaudeDesktopExtension>,
    hooks: Vec<ExtensionHook>,
    gateway_request: Vec<ExtensionGatewayMiddleware>,
    /// 关掉的内置网关环节（codexpp.json 的 `gateway.replaces`）。
    gateway_replaces: Vec<String>,
    /// 网关响应流中间件（codexpp.json 的 `gateway.response`）。
    gateway_response: Vec<ExtensionGatewayMiddleware>,
    /// 工具结果改写（codexpp.json 的 `toolPipeline.postExecute`），由网关请求侧执行。
    tool_post_execute: Vec<ExtensionHook>,
    /// 请求失败时决定是否重试（codexpp.json 的 `gateway.error`）。
    gateway_error: Vec<ExtensionGatewayMiddleware>,
    tools: Vec<ExtensionTool>,
    config_schema: Vec<ExtensionConfigField>,
    /// 页面脚本按实例各应用一次（codexpp.json 的 `instances`）：`[{ id, name, config }]`。
    instances: Vec<Value>,
    /// 常驻后台脚本（codexpp.json 的 `backend.script`）。
    backend: Option<PathBuf>,
    /// 后台对外提供的服务名（codexpp.json 的 `backend.provides`）。
    backend_provides: Vec<String>,
    /// 后台处理 `/ext/<id>/…` 的 HTTP 请求（codexpp.json 的 `backend.http: true`）。
    backend_http: bool,
    /// 定时调用后台方法（codexpp.json 的 `schedules`）。
    schedules: Vec<ExtensionSchedule>,
    /// 拓展声明的模型供应商（codexpp.json 的 `providers`），并入网关的供应商池。
    providers: Vec<ExtensionProvider>,
    /// 包内 `locale/<语言>.json`：语言 id → 词条（`meta.title` / `meta.description` 是本地化的名称与描述）。
    locales: std::collections::BTreeMap<String, Value>,
    /// 包内 `assets/` 的资源：包内路径 → data URL，交给页面的 `ctx.asset(path)`。
    assets: std::collections::BTreeMap<String, String>,
    /// 超出大小上限而没有下发的资源。
    skipped_assets: Vec<String>,
    /// 技能、子 agent、外部 MCP 服务器、全局指令（同步进 Codex home）。
    codex: crate::extension_codex::ExtensionCodexAssets,
    /// 顶替的内置拓展 id（codexpp.json 的 `replaces`）：本拓展启用时，这些内置拓展让位。
    replaces: Vec<String>,
    /// 包内按 dsh 插件写法写的插件目录（codexpp.json 的 `dsh`）：启用时由 dsh 兼容宿主运行。
    dsh: Option<PathBuf>,
    /// 管理器里的拓展面板（codexpp.json 的 `manager`）：包内 HTML 文件、标题、侧栏入口、顶替的管理器页面、可读的数据。
    manager_page: Option<ExtensionManagerPage>,
    /// 拓展自己的设置页（codexpp.json 的 `settings.page`）：管理器里「设置」打开它，代替通用配置表单。
    settings_page: Option<PathBuf>,
    /// 页面最早期脚本（codexpp.json 的 `preload`），见 [`crate::extension_launch`]。
    preload: crate::extension_launch::ExtensionPreload,
    /// 启动前后（codexpp.json 的 `launch`），见 [`crate::extension_launch`]。
    launch: crate::extension_launch::ExtensionLaunch,
    /// 包目录（绝对路径）。
    root: Option<PathBuf>,
    /// 生效配置：schema 默认值叠加 user_scripts.json 里存的值。扫描时算好。
    config: Value,
    error: Option<String>,
}

/// 拓展配置项（codexpp.json 的 `config.<key>`）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionConfigField {
    pub key: String,
    /// string / text（多行）/ number / boolean / select / array / object
    #[serde(rename = "type")]
    pub kind: String,
    /// 字符串，或 `{ en, zh, ... }` 多语言文本（管理器按界面语言显示）。
    pub title: Value,
    pub description: Value,
    pub default: Value,
    /// select 的可选值：[{ value, label }]
    pub options: Vec<Value>,
    /// 界面上按密码框显示。值仍以明文存在本机 user_scripts.json。
    pub secret: bool,
    /// number 的取值范围。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// array 的元素类型（键为空的字段）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<ExtensionConfigField>>,
    /// object 的子字段。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<ExtensionConfigField>,
    /// string 的输入方式：`file` 表示本机文件路径，管理器显示「选择…」按钮。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// `format: "file"` 时选择器只列这些扩展名（不带点）；空表示不限。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

impl ExtensionConfigField {
    /// 校验一个值是否合乎类型；select 必须是可选值之一；对象可以缺键（缺的取默认），不能有未声明的键。
    fn accepts(&self, value: &Value) -> bool {
        match self.kind.as_str() {
            "string" | "text" => value.is_string(),
            "number" => value.as_f64().is_some_and(|number| {
                self.min.is_none_or(|min| number >= min) && self.max.is_none_or(|max| number <= max)
            }),
            "boolean" => value.is_boolean(),
            "select" => self.options.iter().any(|option| option.get("value") == Some(value)),
            "array" => value.as_array().is_some_and(|items| {
                items.len() <= 200 && self.items.as_ref().is_some_and(|field| items.iter().all(|item| field.accepts(item)))
            }),
            "object" => value.as_object().is_some_and(|map| {
                map.iter().all(|(key, item)| {
                    self.properties.iter().find(|field| &field.key == key).is_some_and(|field| field.accepts(item))
                })
            }),
            _ => false,
        }
    }

    /// 生效值：合法的已存值，对象逐个子字段合并默认值；否则取默认值。
    fn resolve(&self, stored: Option<&Value>) -> Value {
        match (self.kind.as_str(), stored) {
            ("object", Some(Value::Object(map))) if self.accepts(&Value::Object(map.clone())) => {
                Value::Object(self.properties.iter().map(|field| (field.key.clone(), field.resolve(map.get(&field.key)))).collect())
            }
            ("object", _) => self.default.clone(),
            (_, Some(value)) if self.accepts(value) => value.clone(),
            _ => self.default.clone(),
        }
    }
}

/// 交给模型前 secret 配置项显示成的打码值。
pub const SECRET_MASK: &str = "******";

/// 把配置里 secret 项（含对象子字段、数组元素里的）换成打码值。`schema` 是序列化后的字段数组，
/// 交给模型的清单、配置结果都要过一遍，免得 API Key 进模型上下文和会话记录。
pub fn mask_secret_config(schema: &Value, config: &Value) -> Value {
    fn mask_field(field: &Value, value: &Value) -> Value {
        if field["secret"].as_bool() == Some(true) {
            return match value {
                Value::Null => Value::Null,
                Value::String(text) if text.is_empty() => value.clone(),
                _ => Value::String(SECRET_MASK.into()),
            };
        }
        match (field["type"].as_str(), value) {
            (Some("object"), Value::Object(_)) => mask_secret_config(&field["properties"], value),
            (Some("array"), Value::Array(items)) => Value::Array(items.iter().map(|item| mask_field(&field["items"], item)).collect()),
            _ => value.clone(),
        }
    }
    let (Some(fields), Some(map)) = (schema.as_array(), config.as_object()) else {
        return config.clone();
    };
    Value::Object(
        map.iter()
            .map(|(key, value)| {
                let masked = fields.iter().find(|field| field["key"] == *key).map_or_else(|| value.clone(), |field| mask_field(field, value));
                (key.clone(), masked)
            })
            .collect(),
    )
}

/// README 最多读这么多字节，超出的截断（页面上提示）。
const EXTENSION_README_MAX_BYTES: usize = 512 * 1024;
/// 按这个顺序找 README（不分大小写）：中文版优先，其次通用版。
const EXTENSION_README_NAMES: &[&str] = &["readme.zh-cn.md", "readme.zh.md", "readme.md", "readme.markdown", "readme"];

/// 包目录里的 README 文件（只看包根目录，不跟随符号链接）。
fn find_extension_readme(root: &std::path::Path) -> Option<PathBuf> {
    let files = fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| (entry.file_name().to_string_lossy().to_lowercase(), entry.path()))
        .collect::<Vec<_>>();
    EXTENSION_README_NAMES
        .iter()
        .find_map(|name| files.iter().find(|(file, _)| file == name).map(|(_, path)| path.clone()))
}

/// 生效配置：每个声明的键取已存且合法的值，否则取默认值（对象按子字段合并）。
fn resolve_extension_config(schema: &[ExtensionConfigField], stored: Option<&Map<String, Value>>) -> Value {
    let mut resolved = Map::new();
    for field in schema {
        resolved.insert(field.key.clone(), field.resolve(stored.and_then(|values| values.get(&field.key))));
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
    /// 请求中间件跑在哪一段：`request`（默认，Codex++ 自己的转换之前，改原始 Responses 请求）、
    /// `upstream`（Codex++ 的压缩、路由、协议转换之后，改真正发给上游的请求体）或
    /// `route`（轮换选好候选供应商之后，按拓展给的顺序重排 / 筛选候选）。
    #[serde(default)]
    pub stage: String,
    /// 同一段里的先后：越小越先，相同时按拓展顺序。
    #[serde(default)]
    pub order: i64,
}

/// 拓展能用 `gateway.replaces` 关掉的 Codex++ 内置网关环节（关掉后拓展自己在 request / route / upstream /
/// error / response 段里实现）。内置环节都能关；仍由 Codex++ 做的只有响应方向的协议转换（响应流中间件拿到的是
/// 转好的 Responses 事件）和官方登录凭据本身，原因见 EXTENSIONS.md「网关中间件」一节。
pub const BUILTIN_GATEWAY_SEGMENTS: &[&str] = &[
    "builtin:local-compaction",
    "builtin:reasoning-collapse",
    "builtin:exec-adapt",
    "builtin:relay-rotation",
    "builtin:gateway-failover",
    // 本地代理（中转与网关两种模式都经过）：
    "builtin:auto-retry",
    "builtin:foreign-reasoning",
    "builtin:unlisted-model-mapping",
    // 网关模式：
    "builtin:key-cooldown",
    "builtin:key-breaker",
    "builtin:local-rate-limit",
    "builtin:title-model",
    // 原先不开放的四个：关掉后由拓展在 upstream / response 段自己做，见 EXTENSIONS.md「网关中间件」。
    "builtin:request-conversion",
    "builtin:auth-headers",
    "builtin:official-shaping",
    "builtin:request-log",
];

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
    default_user_script_manager_for_settings(crate::paths::default_settings_path())
}

pub(crate) fn default_user_script_manager_for_settings(settings_path: PathBuf) -> UserScriptManager {
    let config_dir = default_user_scripts_config_dir();
    let builtin_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .map(|path| path.join("user_scripts"))
        .unwrap_or_else(|| PathBuf::from("user_scripts"));
    let bundled_dir = config_dir.join("bundled_extensions");
    let manager = UserScriptManager::new(builtin_dir, config_dir.join("user_scripts"), config_dir.join("user_scripts.json"))
        .with_settings_path(settings_path.clone())
        .with_bundled_dir(bundled_dir.clone());
    // 每个进程第一次用到时写出官方拓展包、迁移旧的内置桌宠 / 打字特效 / 背景图 / 用量状态栏 / 请求状态条设置；失败只记日志，不影响别的拓展。
    static BUNDLED_READY: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    BUNDLED_READY.get_or_init(|| {
        if let Err(error) = crate::bundled_extensions::materialize(&bundled_dir) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bundled_extensions.materialize_failed",
                json!({ "error": format!("{error:#}") }),
            );
        }
        let settings = crate::settings::SettingsStore::new(settings_path).load().unwrap_or_default();
        if let Err(error) = manager.migrate_bundled_agent_pet(&settings) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bundled_extensions.agent_pet_migration_failed",
                json!({ "error": format!("{error:#}") }),
            );
        }
        if let Err(error) = manager.migrate_bundled_builtins(&settings) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bundled_extensions.builtin_migration_failed",
                json!({ "error": format!("{error:#}") }),
            );
        }
    });
    manager
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
    /// 工具管道适配：`preExecute`（改参数 / 拦截）或 `execute`（代为执行）；None 是普通 Codex 钩子。
    pub adapter: Option<String>,
}

/// 拓展声明的一个模型供应商（codexpp.json 的 `providers[]`）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionProvider {
    /// 供应商短名，模型名写成 `<id>/<模型>`；只能是字母、数字、- 和 _。
    pub id: String,
    pub name: String,
    /// responses / chat / anthropic
    pub protocol: String,
    pub base_url: String,
    /// 字面值，或空串（这时取 api_key_config 指向的配置项）。
    #[serde(skip_serializing)]
    pub api_key: String,
    /// 从拓展配置的哪个键取 Key（`{ "config": "apiKey" }`）。
    pub api_key_config: Option<String>,
    pub models: Vec<String>,
    /// `discoverModels: true`：定期从上游 `/models` 拉模型列表，与 `models` 合并。
    pub discover_models: bool,
    /// `override: true`：与已有供应商同 id 时顶替它（拓展停用后恢复）；默认以已有的为准。
    #[serde(rename = "override")]
    pub override_existing: bool,
}

/// 拓展的一条定时任务：每隔 `every_minutes` 分钟（或每天 `at` 时刻）调用后台方法 `method`。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSchedule {
    pub method: String,
    pub every_minutes: Option<u64>,
    /// 每天的本地时刻 "HH:MM"。
    pub at: Option<String>,
}

/// 解析 `schedules`：`[{ method, everyMinutes } | { method, at: "09:30" }]`。
fn parse_extension_schedules(value: Option<&Value>) -> anyhow::Result<Vec<ExtensionSchedule>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value.as_array().context("schedules 必须是数组")?;
    if items.len() > 8 {
        anyhow::bail!("schedules 最多 8 条");
    }
    items
        .iter()
        .map(|item| {
            let method = item.get("method").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
            if method.is_empty() || method.starts_with("__") {
                anyhow::bail!("schedules 的每一项需要 method（后台导出的方法名）");
            }
            let every_minutes = match item.get("everyMinutes") {
                None => None,
                Some(value) => Some(value.as_u64().filter(|minutes| (1..=10080).contains(minutes)).context("everyMinutes 是 1~10080 的整数")?),
            };
            let at = match item.get("at") {
                None => None,
                Some(value) => {
                    let text = value.as_str().context("at 写成 \"HH:MM\"")?;
                    let valid = text.split_once(':').is_some_and(|(hour, minute)| {
                        hour.len() == 2 && minute.len() == 2 && hour.parse::<u32>().is_ok_and(|hour| hour < 24) && minute.parse::<u32>().is_ok_and(|minute| minute < 60)
                    });
                    if !valid {
                        anyhow::bail!("at 写成 \"HH:MM\"（24 小时制），不认识 {text}");
                    }
                    Some(text.to_string())
                }
            };
            if every_minutes.is_some() == at.is_some() {
                anyhow::bail!("schedules.{method} 需要 everyMinutes 或 at，二选一");
            }
            Ok(ExtensionSchedule { method, every_minutes, at })
        })
        .collect()
}

/// 解析 `providers`：`[{ id, name, protocol, baseUrl, apiKey: "字面值" | { config: "键" }, models: [...], discoverModels? }]`。
fn parse_extension_providers(value: Option<&Value>, schema: &[ExtensionConfigField]) -> anyhow::Result<Vec<ExtensionProvider>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value.as_array().context("providers 必须是数组")?;
    if items.len() > 8 {
        anyhow::bail!("providers 最多 8 个");
    }
    items
        .iter()
        .map(|item| {
            let item = item.as_object().context("providers 的每一项必须是对象")?;
            let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
            let id = text("id");
            if id.is_empty() || id.len() > 48 || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
                anyhow::bail!("providers 的 id 必填，只能是字母、数字、- 和 _：{id:?}");
            }
            let protocol = match text("protocol").as_str() {
                "" | "responses" => "responses",
                "chat" => "chat",
                "anthropic" => "anthropic",
                other => anyhow::bail!("providers.{id}.protocol 只能是 responses、chat、anthropic，不认识 {other}"),
            }
            .to_string();
            let base_url = text("baseUrl");
            if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
                anyhow::bail!("providers.{id}.baseUrl 必须是 http(s) 地址");
            }
            let (api_key, api_key_config) = match item.get("apiKey") {
                None => (String::new(), None),
                Some(Value::String(key)) => (key.clone(), None),
                Some(Value::Object(reference)) => {
                    let key = reference.get("config").and_then(Value::as_str).context("providers.apiKey 写成 { \"config\": \"配置键\" }")?;
                    if !schema.iter().any(|field| field.key == key && field.kind == "string") {
                        anyhow::bail!("providers.{id}.apiKey 引用的配置项 {key} 没有声明，或不是 string");
                    }
                    (String::new(), Some(key.to_string()))
                }
                Some(_) => anyhow::bail!("providers.{id}.apiKey 只能是字符串或 {{ \"config\": \"配置键\" }}"),
            };
            let models = item
                .get("models")
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(Value::as_str).map(str::trim).filter(|model| !model.is_empty()).map(str::to_string).collect::<Vec<_>>())
                .unwrap_or_default();
            let discover_models = item.get("discoverModels").and_then(Value::as_bool).unwrap_or(false);
            if models.is_empty() && !discover_models {
                anyhow::bail!("providers.{id}.models 至少要有一个模型（或设 discoverModels: true 从上游拉）");
            }
            let override_existing = item.get("override").and_then(Value::as_bool).unwrap_or(false);
            Ok(ExtensionProvider { name: if text("name").is_empty() { id.clone() } else { text("name") }, id, protocol, base_url, api_key, api_key_config, models, discover_models, override_existing })
        })
        .collect()
}

/// 拓展可以挂的 Codex 钩子事件。SessionEnd / Interrupt 的超时只有 1~3 秒，起 Node 来不及，不开放。
/// PreModelRequest / PostModelRequest 在 agent 每次请求模型前后触发（官方直连也触发），只能观察：
/// Codex 不采用它们的输出，拦不下请求，也加不了上下文。
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
    "PreModelRequest",
    "PostModelRequest",
];
/// 单个拓展钩子的超时上限（秒）。写进 Codex 配置的分发命令固定用 120 秒，要留出余量。
pub const EXTENSION_HOOK_MAX_TIMEOUT_SECS: u64 = 100;

impl ExtensionPackage {
    /// 页面能直接用的图标地址：https 地址原样给；包内路径（写不写 `assets/` 前缀都认）换成 `assets/` 里那张图的 data URL。
    /// 找不到、不是图片、超出资源大小上限都当没有图标，界面回退默认字形。
    fn icon_url(&self) -> Option<String> {
        let icon = self.icon.as_deref()?;
        if icon.starts_with("https://") {
            return Some(icon.to_string());
        }
        let normalized = icon.trim_start_matches("./");
        let normalized = normalized.strip_prefix("assets/").unwrap_or(normalized);
        self.assets
            .get(&format!("assets/{normalized}"))
            .filter(|url| url.starts_with("data:image/"))
            .cloned()
    }

    fn summary(&self) -> Value {
        json!({
            "id": self.id,
            "version": self.version,
            "description": self.description,
            "icon": self.icon_url(),
            "engine": self.engine,
            "routes": self.routes,
            "renderer": self.renderer.len(),
            "claudeDesktop": self.claude_desktop,
            "hookEvents": self.hooks.iter().map(|hook| hook.event.as_str()).collect::<std::collections::BTreeSet<_>>(),
            "gatewayRequest": self.gateway_request.len(),
            "gatewayReplaces": self.gateway_replaces,
            "gatewayResponse": self.gateway_response.len(),
            "gatewayError": self.gateway_error.len(),
            "toolPipeline": {
                "preExecute": self.hooks.iter().filter(|hook| hook.adapter.as_deref() == Some("preExecute")).count(),
                "execute": self.hooks.iter().filter(|hook| hook.adapter.as_deref() == Some("execute")).count(),
                "postExecute": self.tool_post_execute.len(),
            },
            "tools": self.tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>(),
            "configSchema": self.config_schema,
            "backend": self.backend.is_some(),
            "http": self.backend_http,
            "schedules": self.schedules,
            "services": self.backend_provides,
            "providers": self.providers.iter().map(|provider| json!({ "id": provider.id, "name": provider.name, "models": provider.models })).collect::<Vec<_>>(),
            "codex": self.codex.summary(),
            "replaces": self.replaces,
            "dsh": self.dsh.is_some(),
            "settingsPage": self.settings_page.is_some(),
            "readme": self.root.as_deref().and_then(find_extension_readme).is_some(),
            "managerPage": self.manager_page.as_ref().map(|page| json!({
                "title": page.title,
                "nav": page.nav,
                "icon": page.icon,
                "replaces": page.replaces,
                "data": page.data,
            })),
            "preload": { "scripts": self.preload.scripts.len(), "reloadOnLaunch": self.preload.reload_on_launch },
            "launch": {
                "args": self.launch.args,
                "before": self.launch.before.is_some(),
                "after": self.launch.after.is_some(),
            },
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
            "instances": self.instances,
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
    /// 原样写进清单的 `toolPipeline`，加载时校验。
    pub tool_pipeline: Option<Value>,
    /// 原样写进清单的 `providers`，加载时校验。
    pub providers: Option<Value>,
    /// 原样写进清单的其余键：`skills`、`agents`、`mcpServers`、`instructions`、`schedules`、`prompts`、`resources`，加载时校验。
    pub extra: Map<String, Value>,
    pub files: BTreeMap<String, String>,
    /// 二进制资源：包内路径 → 本机源文件，按字节复制（图片、音频、字体）。
    pub assets: BTreeMap<String, PathBuf>,
    /// `files` 里从本机文件复制来的文本（打包产物等，不经模型逐字转写）：单个上限放宽到
    /// [`EXTENSION_COPIED_TEXT_MAX_BYTES`]，合计另算。
    pub copied_text: BTreeSet<String>,
    /// 单个文本文件的上限，默认（None）是对话造拓展的 256 KiB。Codex++ 自己生成的包（dsh 配套拓展包里
    /// 夹着前端插件的打包产物）可以放宽；文本文件总量上限随之放宽到它的 2 倍（至少 1 MiB）。
    pub max_file_bytes: Option<usize>,
}

const EXTENSION_WRITE_MAX_FILES: usize = 32;
/// 从本机复制来的文本文件（打包产物）的单个上限与合计上限。逐字写的 `content` 仍是 256 KiB：那是模型在一次
/// 工具调用里写出来的，再长多半是截断或乱码；复制来的不经模型，只受页面注入体积约束（与 dsh 前端插件入口一致）。
pub const EXTENSION_COPIED_TEXT_MAX_BYTES: usize = 4 * 1024 * 1024;
const EXTENSION_COPIED_TEXT_MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const EXTENSION_WRITE_MAX_FILE_BYTES: usize = 256 * 1024;
const EXTENSION_WRITE_MAX_TOTAL_BYTES: usize = 1024 * 1024;
/// 单个二进制资源（图片、音频、字体）的上限，以及一个包里资源的总上限。
/// 资源会以 data URL 随清单下发给页面，太大会拖慢每次加载。
const EXTENSION_ASSET_MAX_BYTES: usize = 1024 * 1024;
const EXTENSION_ASSET_MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;

/// 支持的资源类型（按扩展名），返回 MIME。
pub(crate) fn extension_asset_mime(path: &str) -> Option<&'static str> {
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
            // 每次扫描都会走到这里：按（路径, 大小, 修改时间）缓存编码结果，文件没变就不重读。
            let modified = metadata.modified().ok();
            let cached = ASSET_CACHE.lock().unwrap().get(&path).filter(|entry| entry.0 == metadata.len() && entry.1 == modified).map(|entry| entry.2.clone());
            let data_url = match cached {
                Some(data_url) => data_url,
                None => {
                    let Ok(bytes) = fs::read(&path) else {
                        continue;
                    };
                    let data_url = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
                    ASSET_CACHE.lock().unwrap().insert(path.clone(), (metadata.len(), modified, data_url.clone()));
                    data_url
                }
            };
            total += size;
            assets.insert(relative, data_url);
        }
    }
    (assets, skipped)
}

type AssetCacheEntry = (u64, Option<std::time::SystemTime>, String);
static ASSET_CACHE: std::sync::LazyLock<Mutex<std::collections::HashMap<PathBuf, AssetCacheEntry>>> = std::sync::LazyLock::new(Default::default);
/// 读回拓展包时的上限：只读文本、跳过大文件，避免把整个目录塞进对话。
const EXTENSION_READ_MAX_TOTAL_BYTES: usize = 512 * 1024;

/// 拓展包对应的脚本 key。
/// 卸载拓展包用的废纸篓：macOS 是用户的 `~/.Trash`，其他系统放在 Codex++ 状态目录里（可找回）。
pub fn default_extension_trash_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".Trash");
    }
    crate::paths::default_app_state_dir().join("deleted-extensions")
}

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
        icon: None,
        engine: EXTENSION_API_VERSION,
        routes: Vec::new(),
        renderer: Vec::new(),
        claude_desktop: None,
        hooks: Vec::new(),
        gateway_request: Vec::new(),
        gateway_replaces: Vec::new(),
        gateway_response: Vec::new(),
        tool_post_execute: Vec::new(),
        gateway_error: Vec::new(),
        backend_provides: Vec::new(),
        backend_http: false,
        schedules: Vec::new(),
        providers: Vec::new(),
        tools: Vec::new(),
        config_schema: Vec::new(),
        instances: Vec::new(),
        backend: None,
        locales: Default::default(),
        assets: Default::default(),
        skipped_assets: Vec::new(),
        codex: Default::default(),
        replaces: Vec::new(),
        dsh: None,
        manager_page: None,
        settings_page: None,
        preload: Default::default(),
        launch: Default::default(),
        root: None,
        config: json!({}),
        error: None,
    };
    if let Err(error) = parse_extension_manifest(directory, &mut package) {
        package.error = Some(error.to_string());
    }
    package
}

/// 管理器面板页的大小上限：页面整份读进 iframe 的 srcdoc，资源另经 `asset()` 取。
const EXTENSION_MANAGER_PAGE_MAX_BYTES: u64 = 1024 * 1024;

/// 管理器里能被拓展页面顶替的内置页面（管理器的路由 id）。「万物皆拓展」「内置拓展」「设置」三页不开放：
/// 它们是停用、改回拓展的入口，顶替后可能回不去。被顶替的页面上方仍有「查看内置页面」。
pub const MANAGER_REPLACEABLE_PAGES: &[&str] = &[
    "hub", "gatewayProviders", "gateway", "gatewayRouting", "gatewayUsage", "gatewaySettings", "requestLog",
    "overview", "relay", "multiLaunch", "relayEnvironment", "claude", "sessions", "context", "skills", "weixin",
    "dreamSkin", "pluginMarket", "recommendations", "agentCache", "agentCollaboration", "computerUse",
];
/// 拓展的设置页 / 管理器面板经 `codexPlusManager.call(route, payload)` 能调的数据路由（还要清单声明）。
pub const EXTENSION_PAGE_ROUTES: &[&str] = &crate::agent_pet::ROUTES;
/// 拓展页面经 `codexPlusManager.data(name, params)` 能读的管理器数据（都是只读、不含凭据）。
pub const MANAGER_DATA_SOURCES: &[&str] = &[
    "requestFlow", "requestLogs", "gatewayUsage", "localSessions",
    "gatewayProviders", "gatewayRouting", "gatewayHealth", "gatewayQuota",
    "installedSkills", "installedPlugins", "agentCollaboration",
];
/// 拓展页面的侧栏分组。
pub const MANAGER_NAV_GROUPS: &[&str] = &["gateway", "management", "tools"];

/// 管理器里的拓展页面（codexpp.json 的 `manager`）。
#[derive(Debug, Clone)]
struct ExtensionManagerPage {
    path: PathBuf,
    title: String,
    /// 侧栏入口所在分组；没有时只在拓展行菜单里打开。
    nav: Option<String>,
    /// 侧栏图标名（管理器认得的才用，否则用默认图标）。
    icon: Option<String>,
    /// 顶替的内置管理器页面。
    replaces: Vec<String>,
    /// 可读的管理器数据。
    data: Vec<String>,
}

/// 包内的 HTML 页面：必须在包目录内、是 .html 文件、不超过 1 MiB。`field` 只用于报错。
fn resolve_package_html(root: &std::path::Path, page: &str, field: &str) -> anyhow::Result<PathBuf> {
    let path = root
        .join(page)
        .canonicalize()
        .with_context(|| format!("{field} 指向的文件不存在：{page}"))?;
    if !path.starts_with(root) {
        anyhow::bail!("{field} 必须在拓展包目录内：{page}");
    }
    let is_html = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"));
    if !path.is_file() || !is_html {
        anyhow::bail!("{field} 必须是 .html 文件：{page}");
    }
    if fs::metadata(&path)?.len() > EXTENSION_MANAGER_PAGE_MAX_BYTES {
        anyhow::bail!("{field} 超过 1 MiB：{page}（图片等资源放进 assets/，在页面里用 codexPlusManager.asset() 取）");
    }
    Ok(path)
}
/// 解析 `settings: { page }`：拓展自己的设置页，和 manager.page 一样放进沙箱 iframe、用同一套 codexPlusManager 接口。
fn parse_extension_settings_page(root: &std::path::Path, raw: &Map<String, Value>) -> anyhow::Result<Option<PathBuf>> {
    let Some(settings) = raw.get("settings") else {
        return Ok(None);
    };
    let page = settings
        .as_object()
        .and_then(|settings| settings.get("page"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|page| !page.is_empty())
        .context("settings 必须是对象且带 page，如 {\"page\": \"settings.html\"}")?;
    resolve_package_html(root, page, "settings.page").map(Some)
}
/// 解析 `manager: { page, title, nav, icon, replaces, data }`：页面必须是包内存在的 .html 文件，不超过 1 MiB。
fn parse_extension_manager_page(
    root: &std::path::Path,
    raw: &Map<String, Value>,
    default_title: &str,
) -> anyhow::Result<Option<ExtensionManagerPage>> {
    let Some(manager) = raw.get("manager") else {
        return Ok(None);
    };
    let Value::Object(manager) = manager else {
        anyhow::bail!("manager 必须是对象，如 {{\"page\": \"manager.html\"}}");
    };
    let page = manager
        .get("page")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|page| !page.is_empty())
        .context("manager.page 必填：包内的 HTML 文件路径")?;
    let path = resolve_package_html(root, page, "manager.page")?;
    let title = manager
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(default_title)
        .to_string();
    let nav = match manager.get("nav") {
        None | Some(Value::Bool(false)) | Some(Value::Null) => None,
        Some(Value::Bool(true)) => Some("management".to_string()),
        Some(Value::String(group)) if MANAGER_NAV_GROUPS.contains(&group.as_str()) => Some(group.clone()),
        Some(_) => anyhow::bail!("manager.nav 只能是 true 或分组名（{}）", MANAGER_NAV_GROUPS.join("、")),
    };
    let icon = match manager.get("icon") {
        None => None,
        Some(Value::String(icon)) if !icon.is_empty() && icon.len() <= 32 && icon.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-') => Some(icon.clone()),
        Some(_) => anyhow::bail!("manager.icon 只能是小写字母、数字与 - 组成的图标名（如 bar-chart）"),
    };
    let names = |key: &str, allowed: &[&str]| -> anyhow::Result<Vec<String>> {
        let Some(value) = manager.get(key) else {
            return Ok(Vec::new());
        };
        let items = value.as_array().with_context(|| format!("manager.{key} 只能是字符串数组"))?;
        let mut list: Vec<String> = Vec::new();
        for item in items {
            let name = item.as_str().with_context(|| format!("manager.{key} 只能是字符串数组"))?;
            if !allowed.contains(&name) {
                anyhow::bail!("manager.{key} 里的 {name} 不认识（可用：{}）", allowed.join("、"));
            }
            if !list.iter().any(|existing| existing == name) {
                list.push(name.to_string());
            }
        }
        Ok(list)
    };
    let replaces = names("replaces", MANAGER_REPLACEABLE_PAGES)?;
    let data = names("data", MANAGER_DATA_SOURCES)?;
    Ok(Some(ExtensionManagerPage { path, title, nav, icon, replaces, data }))
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
    package.icon = string_field(&raw, "icon").map(|icon| icon.trim().to_string()).filter(|icon| !icon.is_empty());
    package.id = string_field(&raw, "id").unwrap_or_default();
    if package.id.is_empty()
        || !package
            .id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("清单的 id 必填，且只能包含字母、数字、- 和 _");
    }
    // engines.codexpp：整数是拓展 API 版本；字符串是对 Codex++ 版本的 semver 范围（如 "^2.1"、">=2.1.0 <3"）。
    let mut engine_range = None;
    match raw.get("engines").and_then(|engines| engines.get("codexpp")) {
        None => {}
        Some(Value::String(range)) => engine_range = Some(range.clone()),
        Some(engine) => {
            package.engine = engine
                .as_u64()
                .context("engines.codexpp 必须是正整数（拓展 API 版本）或 semver 范围字符串（如 \"^2.1\"）")?;
        }
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
    let dsh_declared = raw.get("dsh").is_some();
    let hooks_declared = raw.get("hooks").is_some_and(|hooks| hooks.as_object().is_some_and(|map| !map.is_empty()));
    let gateway_declared = raw.get("gateway").is_some();
    let tools_declared = raw.get("tools").is_some_and(|tools| tools.as_array().is_some_and(|list| !list.is_empty()));
    let backend_declared = raw.get("backend").is_some();
    let manager_declared = raw.get("manager").is_some();
    let preload_declared = raw.get("preload").is_some();
    let launch_declared = raw.get("launch").is_some();
    let pipeline_declared = raw.get("toolPipeline").is_some();
    let providers_declared = raw.get("providers").is_some_and(|value| value.as_array().is_some_and(|list| !list.is_empty()));
    let root = directory
        .canonicalize()
        .with_context(|| format!("无法解析拓展目录 {}", directory.display()))?;
    package.root = Some(root.clone());
    package.codex = crate::extension_codex::parse(&root, &raw)?;
    if entries.is_empty() && !dsh_declared && !hooks_declared && !gateway_declared && !tools_declared && !backend_declared && !pipeline_declared && !providers_declared && !manager_declared && !preload_declared && !launch_declared && package.codex.is_empty() {
        anyhow::bail!("清单没有声明 renderer 入口，也没有声明 hooks、gateway、tools、toolPipeline、providers、backend、manager、preload、launch、skills、agents、mcpServers 或 instructions");
    }
    package.preload = crate::extension_launch::parse_preload(&root, raw.get("preload"))?;
    package.launch = crate::extension_launch::parse_launch(&root, raw.get("launch"))?;
    package.manager_page = parse_extension_manager_page(&root, &raw, package.name.as_deref().unwrap_or(&package.id))?;
    package.settings_page = parse_extension_settings_page(&root, &raw)?;
    package.renderer = entries
        .iter()
        .map(|entry| resolve_package_entry(&root, entry))
        .collect::<anyhow::Result<Vec<_>>>()?;
    package.hooks = parse_extension_hooks(&root, raw.get("hooks"))?;
    let (tool_hooks, tool_post_execute) = parse_tool_pipeline(&root, raw.get("toolPipeline"))?;
    package.hooks.extend(tool_hooks);
    package.tool_post_execute = tool_post_execute;
    (package.gateway_request, package.gateway_response, package.gateway_error) = parse_gateway_middlewares(&root, raw.get("gateway"))?;
    package.gateway_replaces = parse_gateway_replaces(raw.get("gateway"))?;
    package.tools = parse_extension_tools(&root, &package.id, raw.get("tools"))?;
    package.config_schema = parse_extension_config_schema(raw.get("config"))?;
    package.instances = parse_extension_instances(raw.get("instances"), &package.config_schema)?;
    package.backend = match raw.get("backend") {
        None => None,
        Some(Value::Object(backend)) => {
            // provides：这个后台对外提供的服务名，页面、其它后台、模型工具都能按名字调用。
            package.backend_provides = match backend.get("provides") {
                None => Vec::new(),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|item| {
                        let name = item.as_str().context("backend.provides 只能是字符串数组")?;
                        let valid = name.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic())
                            && name.len() <= 64
                            && name.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'));
                        if !valid {
                            anyhow::bail!("服务名不合法：{name}（字母开头，只含字母、数字、_ . -）");
                        }
                        Ok(name.to_string())
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?,
                Some(_) => anyhow::bail!("backend.provides 只能是字符串数组"),
            };
            package.backend_http = match backend.get("http") {
                None => false,
                Some(Value::Bool(flag)) => *flag,
                Some(_) => anyhow::bail!("backend.http 只能是 true / false"),
            };
            Some(resolve_package_entry(
                &root,
                backend.get("script").and_then(Value::as_str).context("backend 缺少 script")?,
            )?)
        }
        Some(_) => anyhow::bail!("backend 必须是对象：{{ \"script\": \"backend/main.mjs\" }}"),
    };
    package.claude_desktop = raw.get("claudeDesktop")
        .map(crate::extension_claude::ClaudeDesktopExtension::parse)
        .transpose()?;
    if package.claude_desktop.is_some() && package.backend.is_none() {
        anyhow::bail!("claudeDesktop 调用后台方法，需要同时声明 backend.script");
    }
    package.replaces = parse_extension_replaces(raw.get("replaces"))?;
    package.dsh = match raw.get("dsh") {
        None => None,
        Some(Value::String(entry)) => {
            let path = resolve_package_path(&root, entry)?;
            if !path.join("package.json").is_file() {
                anyhow::bail!("dsh 指向的目录 {entry} 里没有 package.json");
            }
            Some(path)
        }
        Some(_) => anyhow::bail!("dsh 只能是包内目录（里面放按 dsh 插件写法写的 package.json）"),
    };
    package.schedules = parse_extension_schedules(raw.get("schedules"))?;
    if !package.schedules.is_empty() && package.backend.is_none() {
        anyhow::bail!("schedules 调用的是后台方法，需要同时声明 backend");
    }
    package.locales = load_extension_locales(&root)?;
    package.providers = parse_extension_providers(raw.get("providers"), &package.config_schema)?;
    (package.assets, package.skipped_assets) = load_extension_assets(&root);
    // 版本校验放在最后：清单本身的格式错误更具体，优先报。
    if let Some(range) = engine_range {
        let current = crate::extension_semver::codexpp_version();
        let ok = crate::extension_semver::satisfies(current, &range)
            .with_context(|| format!("engines.codexpp 的版本范围写法不对：{range}"))?;
        if !ok {
            anyhow::bail!("需要 Codex++ {range}，当前是 {current}，请升级 Codex++ 或换用兼容的拓展版本");
        }
    }
    if package.engine > EXTENSION_API_VERSION {
        anyhow::bail!(
            "需要拓展 API v{}，当前 Codex++ 提供 v{}，请升级 Codex++",
            package.engine,
            EXTENSION_API_VERSION
        );
    }
    Ok(())
}

/// 内置拓展在配置里的 key。清单见 `settings_catalog::builtin_extension_defs`（来自 enhancement-settings.json），
/// 页面侧清单在 `assets/inject/renderer-inject/96-builtin-extensions.js`，两边 id 要一致。
pub fn builtin_extension_key(id: &str) -> String {
    format!("builtin:{id}")
}

fn parse_extension_replaces(value: Option<&Value>) -> anyhow::Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value.as_array().context("replaces 只能是内置拓展 id 的字符串数组")?;
    let mut replaces = Vec::new();
    for item in items {
        let id = item.as_str().context("replaces 只能是内置拓展 id 的字符串数组")?;
        let id = id.strip_prefix("builtin:").unwrap_or(id);
        // 改成官方拓展包的内置拓展：原 id 仍可写，启用时让对应的官方拓展包让位。
        if crate::bundled_extensions::retired_builtin_key(id).is_some() {
            if !replaces.iter().any(|existing| existing == id) {
                replaces.push(id.to_string());
            }
            continue;
        }
        let Some(def) = crate::settings_catalog::builtin_extension_def(id) else {
            let known = crate::settings_catalog::builtin_extension_defs()
                .iter()
                .filter(|def| def.replaceable)
                .map(|def| def.id.as_str())
                .collect::<Vec<_>>()
                .join("、");
            anyhow::bail!("replaces 里的 {id} 不是内置拓展（可用：{known}）");
        };
        if !def.replaceable {
            anyhow::bail!("内置拓展 {id} 只在关闭增强（拓展不加载）时生效，不能被拓展顶替；可以在管理器里停用它");
        }
        if !replaces.iter().any(|existing| existing == id) {
            replaces.push(id.to_string());
        }
    }
    Ok(replaces)
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
                adapter: None,
            });
        }
    }
    Ok(hooks)
}

/// 解析 `toolPipeline`：`{ preExecute: [...], execute: [...], postExecute: [...] }`，每项
/// `{ matcher, script, timeout }`。preExecute / execute 落到 Codex 的 PreToolUse 钩子（matcher 匹配
/// 钩子里的工具名）；postExecute 由网关在下一次请求里改写工具结果（matcher 匹配模型侧的工具名）。
fn parse_tool_pipeline(
    root: &std::path::Path,
    value: Option<&Value>,
) -> anyhow::Result<(Vec<ExtensionHook>, Vec<ExtensionHook>)> {
    let Some(value) = value else {
        return Ok((Vec::new(), Vec::new()));
    };
    let map = value
        .as_object()
        .context("toolPipeline 必须是对象：{ \"preExecute\": [...], \"execute\": [...], \"postExecute\": [...] }")?;
    if let Some(key) = map.keys().find(|key| !["preExecute", "execute", "postExecute"].contains(&key.as_str())) {
        anyhow::bail!("toolPipeline 只支持 preExecute、execute、postExecute，不认识 {key}");
    }
    let parse = |stage: &str, default_timeout: u64, max_timeout: u64| -> anyhow::Result<Vec<ExtensionHook>> {
        let Some(list) = map.get(stage) else {
            return Ok(Vec::new());
        };
        list.as_array()
            .with_context(|| format!("toolPipeline.{stage} 必须是数组"))?
            .iter()
            .map(|item| {
                let item = item.as_object().with_context(|| format!("toolPipeline.{stage} 的每一项必须是对象"))?;
                let script = item
                    .get("script")
                    .and_then(Value::as_str)
                    .with_context(|| format!("toolPipeline.{stage} 缺少 script"))?;
                let matcher = match item.get("matcher") {
                    None => String::new(),
                    Some(Value::String(matcher)) => matcher.clone(),
                    Some(_) => anyhow::bail!("toolPipeline.{stage} 的 matcher 必须是字符串"),
                };
                let timeout_secs = match item.get("timeout") {
                    None => default_timeout,
                    Some(value) => value
                        .as_u64()
                        .filter(|seconds| (1..=max_timeout).contains(seconds))
                        .with_context(|| format!("toolPipeline.{stage} 的 timeout 必须是 1~{max_timeout} 秒"))?,
                };
                Ok(ExtensionHook {
                    event: if stage == "postExecute" { "postExecute".to_string() } else { "PreToolUse".to_string() },
                    matcher,
                    script: resolve_package_entry(root, script)?,
                    timeout_secs,
                    config: Value::Null,
                    adapter: Some(stage.to_string()),
                })
            })
            .collect()
    };
    let mut pre = parse("preExecute", 30, EXTENSION_HOOK_MAX_TIMEOUT_SECS)?;
    pre.extend(parse("execute", 60, EXTENSION_HOOK_MAX_TIMEOUT_SECS)?);
    // 结果改写挡在每次模型请求前面，和网关中间件一样要快。
    let post = parse("postExecute", 5, EXTENSION_GATEWAY_MAX_TIMEOUT_SECS)?;
    Ok((pre, post))
}

/// 网关中间件三类：请求改写、响应流改写、请求失败时决定是否重试。
type GatewayMiddlewareLists = (Vec<ExtensionGatewayMiddleware>, Vec<ExtensionGatewayMiddleware>, Vec<ExtensionGatewayMiddleware>);

/// 解析 `gateway`：`{ request: [...], response: [...], error: [...] }`，写法相同（script + timeout）。
fn parse_gateway_middlewares(
    root: &std::path::Path,
    value: Option<&Value>,
) -> anyhow::Result<GatewayMiddlewareLists> {
    let Some(value) = value else {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    };
    let map = value.as_object().context("gateway 必须是对象：{ \"request\": [...], \"response\": [...], \"error\": [...] }")?;
    if let Some(key) = map.keys().find(|key| !["request", "response", "error", "replaces"].contains(&key.as_str())) {
        anyhow::bail!("gateway 只支持 request（请求中间件）、response（响应流中间件）、error（失败重试）与 replaces（关掉内置环节），不认识 {key}");
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
                let stage = match item.get("stage").and_then(Value::as_str) {
                    None | Some("request") => "request",
                    Some("upstream") if kind == "request" => "upstream",
                    Some("route") if kind == "request" => "route",
                    Some(other) => anyhow::bail!("gateway.{kind} 的 stage 只能是 request、upstream 或 route（只有 request 中间件能用后两种），不认识 {other}"),
                };
                let order = match item.get("order") {
                    None => 0,
                    Some(value) => value.as_i64().with_context(|| format!("gateway.{kind} 的 order 必须是整数"))?,
                };
                Ok(ExtensionGatewayMiddleware {
                    script: resolve_package_entry(root, script)?,
                    timeout_secs,
                    config: Value::Null,
                    stage: stage.to_string(),
                    order,
                })
            })
            .collect()
    };
    Ok((parse("request")?, parse("response")?, parse("error")?))
}

/// 解析 `instances`：同一个拓展的页面脚本开几份，每份 `{ id, name?, config? }`，`config` 只能写清单 `config` 里声明过的键。
fn parse_extension_instances(value: Option<&Value>, schema: &[ExtensionConfigField]) -> anyhow::Result<Vec<Value>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value.as_array().context("instances 必须是数组：[{ \"id\": \"work\", \"config\": { … } }]")?;
    if items.len() > 16 {
        anyhow::bail!("instances 最多 16 个");
    }
    let mut seen = std::collections::BTreeSet::new();
    items
        .iter()
        .map(|item| {
            let item = item.as_object().context("instances 的每一项必须是对象")?;
            let id = item.get("id").and_then(Value::as_str).context("instances 的每一项要有 id")?;
            if id.is_empty() || id.len() > 32 || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
                anyhow::bail!("instances 的 id {id:?} 只能是 1~32 个字母、数字、_、-");
            }
            if !seen.insert(id.to_string()) {
                anyhow::bail!("instances 的 id {id} 重复");
            }
            let config = item.get("config").cloned().unwrap_or_else(|| json!({}));
            let config_map = config.as_object().with_context(|| format!("instances.{id}.config 必须是对象"))?;
            if let Some(key) = config_map.keys().find(|key| !schema.iter().any(|field| &field.key == *key)) {
                anyhow::bail!("instances.{id}.config 里的 {key} 没有在清单 config 里声明");
            }
            Ok(json!({ "id": id, "name": item.get("name").and_then(Value::as_str).unwrap_or(id), "config": config }))
        })
        .collect()
}

/// 解析 `gateway.replaces`：要关掉的内置网关环节，只能是 [`BUILTIN_GATEWAY_SEGMENTS`] 里的名字。
fn parse_gateway_replaces(value: Option<&Value>) -> anyhow::Result<Vec<String>> {
    let Some(list) = value.and_then(|gateway| gateway.get("replaces")) else {
        return Ok(Vec::new());
    };
    let items = list.as_array().context("gateway.replaces 必须是数组")?;
    items
        .iter()
        .map(|item| {
            let name = item.as_str().context("gateway.replaces 的每一项必须是字符串")?;
            if !BUILTIN_GATEWAY_SEGMENTS.contains(&name) {
                anyhow::bail!("gateway.replaces 只能是 {}，不认识 {name}", BUILTIN_GATEWAY_SEGMENTS.join("、"));
            }
            Ok(name.to_string())
        })
        .collect()
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
    map.iter().map(|(key, field)| parse_config_field(key, &format!("config.{key}"), field, 0)).collect()
}

/// 解析一个配置字段。`path` 用于报错（如 `config.proxy.host`），`depth` 限制对象嵌套层数。
fn parse_config_field(key: &str, path: &str, field: &Value, depth: usize) -> anyhow::Result<ExtensionConfigField> {
    if !key.is_empty() && !key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
        anyhow::bail!("配置键 {key:?} 只能包含字母、数字、_、-");
    }
    let field = field.as_object().with_context(|| format!("{path} 必须是对象"))?;
    let kind = field.get("type").and_then(Value::as_str).unwrap_or("string").to_string();
    if !["string", "text", "number", "boolean", "select", "array", "object"].contains(&kind.as_str()) {
        anyhow::bail!("{path}.type 只能是 string、text、number、boolean、select、array、object");
    }
    // title / description：字符串，或 { 语言: 文本 } 多语言对象。
    let text = |name: &str, fallback: &str| -> anyhow::Result<Value> {
        match field.get(name) {
            None => Ok(json!(fallback)),
            Some(Value::String(text)) => Ok(json!(text)),
            Some(Value::Object(map)) if !map.is_empty() && map.values().all(Value::is_string) => Ok(Value::Object(map.clone())),
            Some(_) => anyhow::bail!("{path}.{name} 必须是字符串或 {{ 语言: 文本 }}"),
        }
    };
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
                _ => anyhow::bail!("{path}.options 的每一项要么是值，要么是 {{ value, label }}"),
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        Some(_) => anyhow::bail!("{path}.options 必须是数组"),
    };
    if kind == "select" && options.is_empty() {
        anyhow::bail!("{path} 是 select，必须给 options");
    }
    let bound = |name: &str| -> anyhow::Result<Option<f64>> {
        match field.get(name) {
            None => Ok(None),
            Some(value) => value.as_f64().map(Some).with_context(|| format!("{path}.{name} 必须是数字")),
        }
    };
    let items = if kind == "array" {
        let items = field.get("items").with_context(|| format!("{path} 是 array，必须给 items（元素类型）"))?;
        let item = parse_config_field("", &format!("{path}.items"), items, depth + 1)?;
        if matches!(item.kind.as_str(), "array" | "object") {
            anyhow::bail!("{path}.items 只能是简单类型（string、text、number、boolean、select）");
        }
        Some(Box::new(item))
    } else {
        None
    };
    let properties = if kind == "object" {
        if depth >= 2 {
            anyhow::bail!("{path} 嵌套过深（对象最多 3 层）");
        }
        let map = field
            .get("properties")
            .and_then(Value::as_object)
            .filter(|map| !map.is_empty())
            .with_context(|| format!("{path} 是 object，必须给 properties（子字段）"))?;
        if map.len() > 32 {
            anyhow::bail!("{path}.properties 最多 32 项");
        }
        map.iter()
            .map(|(child, value)| parse_config_field(child, &format!("{path}.{child}"), value, depth + 1))
            .collect::<anyhow::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let format = match field.get("format") {
        None => None,
        Some(Value::String(format)) if format == "file" && kind == "string" => Some(format.clone()),
        Some(_) => anyhow::bail!("{path}.format 目前只支持 \"file\"，且只能用在 string 上"),
    };
    let extensions = match field.get("extensions") {
        None => Vec::new(),
        Some(Value::Array(items)) if format.is_some() && items.len() <= 20 => items
            .iter()
            .map(|item| match item.as_str() {
                Some(name) if !name.is_empty() && name.len() <= 10 && name.chars().all(|ch| ch.is_ascii_alphanumeric()) => {
                    Ok(name.to_ascii_lowercase())
                }
                _ => anyhow::bail!("{path}.extensions 的每一项是不带点的扩展名，如 \"png\""),
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        Some(_) => anyhow::bail!("{path}.extensions 只能和 format: \"file\" 一起用，最多 20 项"),
    };
    let mut result = ExtensionConfigField {
        key: key.to_string(),
        kind: kind.clone(),
        title: text("title", key)?,
        description: text("description", "")?,
        default: Value::Null,
        options,
        secret: field.get("secret").and_then(Value::as_bool).unwrap_or(false),
        min: bound("min")?,
        max: bound("max")?,
        items,
        properties,
        format,
        extensions,
    };
    result.default = match kind.as_str() {
        // 对象的默认值由子字段默认值组成；清单里写的 default 只用来覆盖其中的键。
        "object" => {
            let given = field.get("default").and_then(Value::as_object).cloned().unwrap_or_default();
            Value::Object(result.properties.iter().map(|child| (child.key.clone(), child.resolve(given.get(&child.key)))).collect())
        }
        _ => {
            let fallback = match kind.as_str() {
                "number" => match result.min { Some(min) if min > 0.0 => json!(min), _ => json!(0) },
                "boolean" => json!(false),
                "select" => result.options[0]["value"].clone(),
                "array" => json!([]),
                _ => json!(""),
            };
            match field.get("default") {
                Some(value) if result.accepts(value) => value.clone(),
                Some(_) => anyhow::bail!("{path}.default 与 type 不符"),
                None => fallback,
            }
        }
    };
    Ok(result)
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
/// 包内相对路径（文件或目录）→ 绝对路径；不许跳出包目录。
pub fn resolve_package_path(root: &std::path::Path, entry: &str) -> anyhow::Result<PathBuf> {
    let relative = std::path::Path::new(entry);
    if entry.trim().is_empty()
        || relative.is_absolute()
        || relative.components().any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        anyhow::bail!("{entry} 必须是包内的相对路径");
    }
    let resolved = root.join(relative).canonicalize().with_context(|| format!("包内没有 {entry}"))?;
    if !resolved.starts_with(root) {
        anyhow::bail!("{entry} 必须在包目录里");
    }
    Ok(resolved)
}

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
  window.__codexPlusUserScripts.scripts[key] = {{ key, name: {name}, source: {source_name}, manifest: {manifest}, status: "loading", error: "", loadedAt: new Date().toISOString() }};{headless}
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
        source = source,
        // 没有页面入口的拓展包（只有钩子、工具、网关或后台）在页面上没有要撤销的东西：登记一个空清理函数，
        // 否则热重载会把它当成「没登记清理的旧脚本」而退回整页刷新。
        headless = if script.package.as_ref().is_some_and(|package| package.renderer.is_empty()) {
            "\n  window.__codexPlusUserScripts.scripts[key].headless = true;\n  window.__codexPlusUserScripts.scripts[key].cleanups = [() => {}];"
        } else {
            ""
        }
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
        claude_scripts: raw
            .get("claude_scripts")
            .and_then(Value::as_object)
            .map(|items| {
                items.iter()
                    .filter_map(|(key, value)| Some((key.clone(), value.as_bool()?)))
                    .collect()
            })
            .unwrap_or_default(),
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
