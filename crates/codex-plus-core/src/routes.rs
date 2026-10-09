use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use crate::models::{DeleteResult, DeleteStatus, ExportResult, ExportStatus, SessionRef};
use crate::settings::{BackendSettings, SettingsStore};
use crate::status::StatusStore;
use crate::user_scripts::UserScriptManager;

pub type UserScriptEvaluator = Arc<dyn Fn(&str, &str) -> anyhow::Result<Value> + Send + Sync>;
pub type DevtoolsOpener = Arc<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>;

#[derive(Clone)]
pub struct BridgeContext {
    settings: Arc<dyn BridgeSettingsService>,
    runtime: Arc<dyn BridgeRuntimeService>,
    data: Arc<dyn BridgeDataService>,
}

impl BridgeContext {
    pub fn new(
        settings: Arc<dyn BridgeSettingsService>,
        runtime: Arc<dyn BridgeRuntimeService>,
        data: Arc<dyn BridgeDataService>,
    ) -> Self {
        Self {
            settings,
            runtime,
            data,
        }
    }

    pub fn core(runtime: Arc<dyn BridgeRuntimeService>) -> Self {
        Self::core_with_data(runtime, Arc::new(UnavailableDataService))
    }

    pub fn core_with_data(
        runtime: Arc<dyn BridgeRuntimeService>,
        data: Arc<dyn BridgeDataService>,
    ) -> Self {
        Self::new(Arc::new(CoreSettingsService::default()), runtime, data)
    }

    pub fn core_with_data_and_app_dir(
        runtime: Arc<dyn BridgeRuntimeService>,
        data: Arc<dyn BridgeDataService>,
        app_dir: PathBuf,
    ) -> Self {
        Self::new(
            Arc::new(CoreSettingsService::with_app_dir(app_dir)),
            runtime,
            data,
        )
    }
}

#[async_trait]
pub trait BridgeSettingsService: Send + Sync {
    async fn get_settings(&self) -> anyhow::Result<BackendSettings>;
    async fn set_settings(&self, payload: Value) -> anyhow::Result<BackendSettings>;

    async fn codex_app_version(&self) -> anyhow::Result<String> {
        Ok(String::new())
    }

    fn whale_ledger_path(&self) -> PathBuf {
        crate::paths::default_settings_path().with_file_name("whale-balance.sqlite3")
    }
}

#[async_trait]
pub trait BridgeRuntimeService: Send + Sync {
    async fn user_script_inventory(&self) -> anyhow::Result<Value>;
    async fn user_script_inventory_with_runtime_status(
        &self,
        _payload: Value,
    ) -> anyhow::Result<Value> {
        self.user_script_inventory().await
    }
    async fn set_user_scripts_enabled(&self, enabled: bool) -> anyhow::Result<Value>;
    async fn set_user_script_enabled(&self, key: String, enabled: bool) -> anyhow::Result<Value>;
    async fn delete_user_script(&self, key: String) -> anyhow::Result<Value>;
    async fn reload_user_scripts(&self) -> anyhow::Result<Value>;
    async fn load_user_scripts(&self) -> anyhow::Result<Value> {
        self.user_script_inventory().await
    }
    /// 拉市场清单，并合并本地已安装状态。
    async fn script_market_list(&self) -> anyhow::Result<Value> {
        anyhow::bail!("script market is unavailable")
    }
    /// 按 id 从市场安装脚本。
    async fn script_market_install(&self, _payload: Value) -> anyhow::Result<Value> {
        anyhow::bail!("script market is unavailable")
    }
    /// 独立插件市场只读取索引，安装时才获取目标插件。
    async fn plugin_market_list(&self, payload: Value) -> anyhow::Result<Value> {
        crate::plugin_market::list_plugins(
            &crate::codex_home::default_codex_home_dir(),
            payload
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("public"),
            payload
                .get("refresh")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        )
        .await
    }
    async fn plugin_market_install(&self, payload: Value) -> anyhow::Result<Value> {
        crate::plugin_market::install_plugin(
            &crate::codex_home::default_codex_home_dir(),
            payload
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("public"),
            payload
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .await
    }
    async fn plugin_market_install_status(&self, payload: Value) -> anyhow::Result<Value> {
        crate::plugin_market::install_status(
            &crate::codex_home::default_codex_home_dir(),
            payload
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("public"),
            payload
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
    }
    /// 注入层适配原生插件页面的列表、详情与安装协议。
    async fn plugin_market_native(&self, payload: Value) -> anyhow::Result<Value> {
        crate::plugin_market::native_request(
            &crate::codex_home::default_codex_home_dir(),
            payload
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            payload.get("params").cloned().unwrap_or_else(|| json!({})),
        )
        .await
    }
    async fn open_devtools(&self) -> anyhow::Result<Value>;
    async fn open_manager(&self, payload: Value) -> anyhow::Result<Value>;
    async fn open_transient_manager(&self, payload: Value) -> anyhow::Result<Value> {
        self.open_manager(payload).await
    }
    async fn backend_status(&self) -> anyhow::Result<Value>;
    async fn codex_model_catalog(&self) -> anyhow::Result<Value>;
    async fn ads(&self) -> anyhow::Result<Value>;
    async fn create_share(&self, payload: Value) -> anyhow::Result<Value> {
        crate::share::create_share(payload).await
    }
}

#[async_trait]
pub trait BridgeDataService: Send + Sync {
    async fn whale_history(&self, _payload: Value) -> anyhow::Result<Value> {
        Ok(json!({"status":"unavailable","message":"本机用量统计暂不可用"}))
    }
    async fn delete(&self, session: SessionRef) -> anyhow::Result<DeleteResult>;
    async fn undo(&self, undo_token: String) -> anyhow::Result<DeleteResult>;
    async fn export_markdown(&self, session: SessionRef) -> anyhow::Result<ExportResult>;
    async fn thread_usage_history(&self, session: SessionRef) -> anyhow::Result<Value>;
    async fn whale_session(&self, _session: SessionRef) -> anyhow::Result<Value> {
        Ok(json!({"status": "unavailable", "message": "当前后端暂不支持会话用量摘要"}))
    }
    async fn find_archived_thread_by_title(
        &self,
        title: String,
    ) -> anyhow::Result<Option<SessionRef>>;
    async fn recover_remote_control_session(&self, _thread_id: String) -> anyhow::Result<Value> {
        anyhow::bail!("Remote Control session recovery is unavailable")
    }
    async fn export_session_file(&self, _session: SessionRef) -> anyhow::Result<Value> {
        anyhow::bail!("Session file export is unavailable")
    }
    async fn import_session_file(&self, _payload: Value) -> anyhow::Result<Value> {
        anyhow::bail!("Session file import is unavailable")
    }
}

pub async fn handle_bridge_request(
    ctx: BridgeContext,
    path: &str,
    payload: Value,
) -> serde_json::Value {
    let started = Instant::now();
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "bridge.request",
        json!({
            "path": path,
            "payload_keys": payload
                .as_object()
                .map(|object| object.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        }),
    );
    let result = match path {
        "/settings/get" => settings_value(&ctx, ctx.settings.get_settings().await).await,
        "/dictation/status" => ctx.settings.get_settings().await.map(|settings| {
            let effective = crate::dictation::effective_settings(&settings);
            let mut status = crate::dictation::public_status(&effective);
            if effective.enabled {
                status["helperToken"] = json!(crate::dictation::helper_token());
            }
            status
        }),
        "/dictation/transcribe" => match ctx.settings.get_settings().await {
            Ok(settings) => crate::dictation::transcribe_bridge(
                &crate::dictation::effective_settings(&settings),
                &payload,
            )
            .await
            .map_err(anyhow::Error::from),
            Err(_) => Err(anyhow::anyhow!("无法读取语音设置")),
        },
        "/dictation/cancel" => {
            crate::dictation::cancel_bridge(&payload).map_err(anyhow::Error::from)
        }
        "/settings/set" => {
            settings_value(&ctx, ctx.settings.set_settings(payload.clone()).await).await
        }
        "/user-scripts/list" => {
            ctx.runtime
                .user_script_inventory_with_runtime_status(payload.clone())
                .await
        }
        "/user-scripts/set-enabled" => {
            let enabled = payload
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            ctx.runtime.set_user_scripts_enabled(enabled).await
        }
        "/user-scripts/set-script-enabled" => {
            let key = payload
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let enabled = payload
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            ctx.runtime.set_user_script_enabled(key, enabled).await
        }
        "/user-scripts/delete" => {
            let key = payload
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            ctx.runtime.delete_user_script(key).await
        }
        "/user-scripts/load" => ctx.runtime.load_user_scripts().await,
        "/user-scripts/reload" => ctx.runtime.reload_user_scripts().await,
        "/devtools/open" => ctx.runtime.open_devtools().await,
        "/manager/open" => ctx.runtime.open_manager(payload.clone()).await,
        "/manager/open-transient" => ctx.runtime.open_transient_manager(payload.clone()).await,
        "/backend/status" => backend_status_value(
            ctx.runtime.backend_status().await,
            ctx.settings.get_settings().await,
        ),
        "/codex-model-catalog" | "/codex-config-model" => ctx.runtime.codex_model_catalog().await,
        "/diagnostics/log" => diagnostic_log_value(payload.clone()),
        "/llm-proxy" => llm_proxy_value(payload.clone()).await,
        "/ads" => ctx.runtime.ads().await,
        "/share/create" => ctx.runtime.create_share(payload.clone()).await,
        "/script-market/list" => ctx.runtime.script_market_list().await,
        "/script-market/install" => ctx.runtime.script_market_install(payload.clone()).await,
        "/plugin-market/list" => ctx.runtime.plugin_market_list(payload.clone()).await,
        "/plugin-market/install" => ctx.runtime.plugin_market_install(payload.clone()).await,
        "/plugin-market/install-status" => {
            ctx.runtime
                .plugin_market_install_status(payload.clone())
                .await
        }
        "/plugin-market/native" => ctx.runtime.plugin_market_native(payload.clone()).await,
        "/stepwise/settings" => stepwise_settings_value(ctx.settings.get_settings().await),
        "/stepwise/generate" => {
            stepwise_generate_value(ctx.settings.get_settings().await, payload.clone()).await
        }
        "/stepwise/test" => {
            stepwise_test_value(ctx.settings.get_settings().await, payload.clone()).await
        }
        "/delete" => {
            let session = session_from_payload(&payload);
            match session.require_local_delete() {
                Ok(()) => result_value(ctx.data.delete(session).await),
                Err(error) => Err(error),
            }
        }
        "/undo" => {
            let undo_token = payload
                .get("undo_token")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            result_value(ctx.data.undo(undo_token).await)
        }
        "/export-markdown" => result_value(
            ctx.data
                .export_markdown(session_from_payload(&payload))
                .await,
        ),
        "/thread-usage-history" => {
            ctx.data
                .thread_usage_history(session_from_payload(&payload))
                .await
        }
        // 仅内置挂件使用，不加入第三方拓展路由白名单。
        "/whale/history" => match ctx.settings.get_settings().await {
            Ok(settings)
                if crate::whale::enabled(&settings)
                    && crate::whale_full::history_enabled(&ctx.settings.whale_ledger_path()) =>
            {
                ctx.data.whale_history(payload.clone()).await
            }
            Ok(_) => Ok(json!({"status":"disabled"})),
            Err(_) => Ok(json!({"status":"unavailable","message":"无法读取挂件设置"})),
        },
        "/whale/full" => match ctx.settings.get_settings().await {
            Ok(settings) => {
                let path = ctx.settings.whale_ledger_path();
                let requested_path = payload
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let history = if crate::whale::enabled(&settings)
                    && crate::whale_full::history_enabled(&path)
                    && matches!(
                        requested_path,
                        "/dsh-whale/usage-records.json" | "/dsh-whale/api-models.json"
                    ) {
                    ctx.data
                        .whale_history(crate::whale_full::history_query(&path))
                        .await
                        .unwrap_or_else(|_| json!({"status":"unavailable"}))
                } else {
                    Value::Null
                };
                Ok(crate::whale_full::handle(&settings, path, payload.clone(), history).await)
            }
            Err(_) => Ok(json!({"status":503,"body":{"ok":false,"error":"无法读取挂件设置"}})),
        },
        "/whale/balance" => match ctx.settings.get_settings().await {
            Ok(settings) => {
                Ok(crate::whale::balance(&settings, ctx.settings.whale_ledger_path()).await)
            }
            Err(_) => Ok(crate::whale::unavailable("无法读取挂件设置")),
        },
        "/whale/session" => match ctx.settings.get_settings().await {
            Ok(settings)
                if crate::whale::enabled(&settings)
                    && crate::whale_full::history_enabled(&ctx.settings.whale_ledger_path()) =>
            {
                ctx.data.whale_session(session_from_payload(&payload)).await
            }
            Ok(_) => Ok(json!({"status": "disabled"})),
            Err(_) => Ok(json!({"status": "unavailable", "message": "无法读取挂件设置"})),
        },
        "/archived-thread" => {
            let title = payload
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            archived_thread_value(ctx.data.find_archived_thread_by_title(title).await)
        }
        "/remote-control-session/recover" => {
            let thread_id = payload
                .get("thread_id")
                .or_else(|| payload.get("threadId"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            ctx.data.recover_remote_control_session(thread_id).await
        }
        "/session/export" => {
            ctx.data
                .export_session_file(session_from_payload(&payload))
                .await
        }
        "/session/import" => ctx.data.import_session_file(payload.clone()).await,
        _ => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.unknown_path",
                json!({
                    "path": path
                }),
            );
            return json!({
                "status": "failed",
                "session_id": "",
                "message": "Unknown bridge path"
            });
        }
    };

    let response = result.unwrap_or_else(|error| {
        if matches!(path, "/dictation/transcribe" | "/dictation/cancel") {
            failed_from_error(&Value::Null, error)
        } else {
            failed_from_error(&payload, error)
        }
    });
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "bridge.response",
        json!({
            "path": path,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "status": response.get("status").and_then(Value::as_str).unwrap_or("")
        }),
    );
    response
}

#[derive(Default)]
pub struct CoreSettingsService {
    store: SettingsStore,
    app_dir: Option<PathBuf>,
}

impl CoreSettingsService {
    fn with_app_dir(app_dir: PathBuf) -> Self {
        Self {
            store: SettingsStore::default(),
            app_dir: Some(app_dir),
        }
    }
}

#[async_trait]
impl BridgeSettingsService for CoreSettingsService {
    async fn get_settings(&self) -> anyhow::Result<BackendSettings> {
        self.store.load()
    }

    async fn set_settings(&self, payload: Value) -> anyhow::Result<BackendSettings> {
        self.store.update(payload)
    }

    async fn codex_app_version(&self) -> anyhow::Result<String> {
        if let Some(app_dir) = self.app_dir.as_deref() {
            return Ok(crate::app_paths::codex_app_version(app_dir).unwrap_or_default());
        }
        let settings = self.store.load().unwrap_or_default();
        let app_dir = crate::app_paths::resolve_codex_app_dir_with_saved(
            None,
            Some(settings.codex_app_path.as_str()),
        );
        Ok(app_dir
            .as_deref()
            .and_then(crate::app_paths::codex_app_version)
            .unwrap_or_default())
    }
}

#[derive(Clone)]
pub struct CoreRuntimeService {
    debug_port: u16,
    status_store: StatusStore,
    user_scripts: Option<UserScriptManager>,
    websocket_url: Option<String>,
    user_script_evaluator: Option<UserScriptEvaluator>,
    devtools_opener: Option<DevtoolsOpener>,
    devtools_target_id: Option<String>,
}

impl CoreRuntimeService {
    pub fn new(debug_port: u16, status_store: StatusStore) -> Self {
        Self {
            debug_port,
            status_store,
            user_scripts: None,
            websocket_url: None,
            user_script_evaluator: None,
            devtools_opener: None,
            devtools_target_id: None,
        }
    }

    pub fn with_user_scripts(mut self, user_scripts: UserScriptManager) -> Self {
        self.user_scripts = Some(user_scripts);
        self
    }

    pub fn with_websocket_url(mut self, websocket_url: impl Into<String>) -> Self {
        self.websocket_url = Some(websocket_url.into());
        self
    }

    pub fn with_user_script_evaluator(mut self, evaluator: UserScriptEvaluator) -> Self {
        self.user_script_evaluator = Some(evaluator);
        self
    }

    pub fn with_devtools_opener(mut self, opener: DevtoolsOpener) -> Self {
        self.devtools_opener = Some(opener);
        self
    }

    pub fn with_devtools_target_id(mut self, target_id: impl Into<String>) -> Self {
        self.devtools_target_id = Some(target_id.into());
        self
    }
}

#[async_trait]
impl BridgeRuntimeService for CoreRuntimeService {
    async fn user_script_inventory(&self) -> anyhow::Result<Value> {
        match &self.user_scripts {
            Some(user_scripts) => user_scripts.inventory(),
            None => Ok(empty_user_script_inventory()),
        }
    }

    async fn set_user_scripts_enabled(&self, enabled: bool) -> anyhow::Result<Value> {
        match &self.user_scripts {
            Some(user_scripts) => {
                user_scripts.set_global_enabled(enabled)?;
                user_scripts.inventory()
            }
            None => {
                let mut inventory = empty_user_script_inventory();
                inventory["enabled"] = json!(enabled);
                Ok(inventory)
            }
        }
    }

    async fn set_user_script_enabled(&self, key: String, enabled: bool) -> anyhow::Result<Value> {
        match &self.user_scripts {
            Some(user_scripts) => {
                user_scripts.set_script_enabled(&key, enabled)?;
                user_scripts.inventory()
            }
            None => Ok(empty_user_script_inventory()),
        }
    }

    async fn delete_user_script(&self, key: String) -> anyhow::Result<Value> {
        match &self.user_scripts {
            Some(user_scripts) => {
                user_scripts.delete_user_script(&key)?;
                user_scripts.inventory()
            }
            None => Ok(empty_user_script_inventory()),
        }
    }

    async fn load_user_scripts(&self) -> anyhow::Result<Value> {
        if let (Some(user_scripts), Some(websocket_url), Some(evaluator)) = (
            &self.user_scripts,
            self.websocket_url.as_deref(),
            &self.user_script_evaluator,
        ) {
            evaluator(websocket_url, &user_scripts.build_initial_bundle()?)?;
        } else {
            anyhow::bail!("Codex 页面尚未连接");
        }
        self.user_script_inventory().await
    }

    async fn script_market_list(&self) -> anyhow::Result<Value> {
        let Some(user_scripts) = &self.user_scripts else {
            anyhow::bail!("用户脚本目录不可用");
        };
        crate::script_market::list_market_scripts(user_scripts).await
    }

    async fn script_market_install(&self, payload: Value) -> anyhow::Result<Value> {
        let Some(user_scripts) = &self.user_scripts else {
            anyhow::bail!("用户脚本目录不可用");
        };
        let id = payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if id.is_empty() {
            anyhow::bail!("脚本 id 不能为空");
        }
        crate::script_market::install_market_script_by_id(user_scripts, id).await
    }

    async fn reload_user_scripts(&self) -> anyhow::Result<Value> {
        if let (Some(user_scripts), Some(websocket_url), Some(evaluator)) = (
            &self.user_scripts,
            self.websocket_url.as_deref(),
            &self.user_script_evaluator,
        ) {
            let bundle = user_scripts.build_reload_bundle()?;
            evaluator(websocket_url, &bundle)?;
        } else {
            anyhow::bail!("Codex 页面尚未连接");
        }
        self.user_script_inventory().await
    }

    async fn open_devtools(&self) -> anyhow::Result<Value> {
        let target_id = self
            .devtools_target_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("No DevTools target configured"))?;
        let url = devtools_url(self.debug_port, target_id);
        if let Some(opener) = &self.devtools_opener {
            opener(&url)?;
        }
        Ok(json!({
            "status": "ok",
            "target_id": target_id,
            "url": url
        }))
    }

    async fn open_manager(&self, payload: Value) -> anyhow::Result<Value> {
        let navigation =
            crate::manager_navigation::save_pending_manager_navigation_from_payload(&payload)?;
        let target = crate::install::open_or_activate_manager().map_err(|error| {
            crate::manager_navigation::rollback_pending_manager_navigation_after_launch_failure(
                navigation.as_ref(),
                error,
            )
        })?;
        Ok(json!({
            "status": "ok",
            "path": target,
            "navigation": navigation
        }))
    }

    async fn open_transient_manager(&self, payload: Value) -> anyhow::Result<Value> {
        let navigation =
            crate::manager_navigation::save_pending_manager_navigation_from_payload(&payload)?;
        let target = crate::install::spawn_companion(
            crate::install::MANAGER_BINARY,
            ["--transient"],
        )
        .map_err(|error| {
            crate::manager_navigation::rollback_pending_manager_navigation_after_launch_failure(
                navigation.as_ref(),
                error,
            )
        })?;
        Ok(json!({
            "status": "ok",
            "path": target,
            "navigation": navigation
        }))
    }

    async fn backend_status(&self) -> anyhow::Result<Value> {
        let _ = self.status_store.load_latest();
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "bridge.backend_status_ok",
            json!({
                "debug_port": self.debug_port,
                "version": crate::version::VERSION
            }),
        );
        Ok(json!({"status": "ok", "message": "后端已连接", "version": crate::version::VERSION}))
    }

    async fn codex_model_catalog(&self) -> anyhow::Result<Value> {
        Ok(crate::model_catalog::read_codex_model_catalog().await)
    }

    async fn ads(&self) -> anyhow::Result<Value> {
        crate::ads::fetch_ad_list().await
    }
}

struct UnavailableDataService;

#[async_trait]
impl BridgeDataService for UnavailableDataService {
    async fn delete(&self, session: SessionRef) -> anyhow::Result<DeleteResult> {
        Ok(DeleteResult {
            status: DeleteStatus::Failed,
            session_id: session.session_id,
            message: "Delete service is not wired in core launcher hooks".to_string(),
            undo_token: None,
            backup_path: None,
        })
    }

    async fn undo(&self, undo_token: String) -> anyhow::Result<DeleteResult> {
        Ok(DeleteResult {
            status: DeleteStatus::Failed,
            session_id: String::new(),
            message: "Undo service is not wired in core launcher hooks".to_string(),
            undo_token: Some(undo_token),
            backup_path: None,
        })
    }

    async fn export_markdown(&self, session: SessionRef) -> anyhow::Result<ExportResult> {
        Ok(ExportResult {
            status: ExportStatus::Failed,
            session_id: session.session_id,
            message: "Markdown export service is not wired in core launcher hooks".to_string(),
            filename: None,
            markdown: None,
        })
    }

    async fn thread_usage_history(&self, session: SessionRef) -> anyhow::Result<Value> {
        Ok(json!({
            "status": "failed",
            "session_id": session.session_id,
            "message": "Thread usage history service is not wired in core launcher hooks",
            "history": []
        }))
    }

    async fn find_archived_thread_by_title(
        &self,
        _title: String,
    ) -> anyhow::Result<Option<SessionRef>> {
        Ok(None)
    }
}

fn settings_payload_value(
    mut settings: BackendSettings,
    codex_app_version: String,
) -> anyhow::Result<Value> {
    let active_relay_session_provider = settings.active_relay_session_provider();
    let active_relay_codex_provider = crate::model_catalog::codex_model_provider_for_relay_profile(
        &crate::relay_config::default_codex_home_dir(),
        &settings.active_relay_profile(),
    );
    settings.dictation = crate::dictation::effective_settings(&settings);
    let mut value = serde_json::to_value(settings)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("codexAppStepwiseApiKey");
        if let Some(dictation) = object.get_mut("dictation").and_then(Value::as_object_mut) {
            let configured = dictation
                .remove("apiKey")
                .and_then(|value| value.as_str().map(|value| !value.trim().is_empty()))
                .unwrap_or(false);
            dictation.insert("apiKeyConfigured".to_string(), json!(configured));
        }
        object.insert(
            "activeRelaySessionProvider".to_string(),
            Value::String(active_relay_session_provider.as_str().to_string()),
        );
        object.insert(
            "activeRelayCodexProvider".to_string(),
            Value::String(active_relay_codex_provider),
        );
        object.insert(
            "codexAppVersion".to_string(),
            Value::String(codex_app_version),
        );
    }
    Ok(value)
}

#[cfg(test)]
mod dictation_privacy_tests {
    use super::*;

    #[test]
    fn settings_bridge_preserves_session_share_choice_for_renderer_master_gating() {
        for enabled in [false, true] {
            let settings = BackendSettings {
                codex_app_session_share: enabled,
                enhancements_enabled: false,
                ..BackendSettings::default()
            };
            let result = settings_payload_value(settings, String::new()).unwrap();
            assert_eq!(result["codexAppSessionShare"], enabled);
            assert_eq!(result["enhancementsEnabled"], false);
        }
    }

    #[test]
    fn dictation_settings_bridge_removes_asr_key() {
        let mut settings = BackendSettings::default();
        settings.dictation.api_key = "fake-asr-private-key".to_string();
        let result = settings_payload_value(settings, String::new()).unwrap();
        assert!(result["dictation"].get("apiKey").is_none());
        assert_eq!(result["dictation"]["apiKeyConfigured"], true);
        assert!(!result.to_string().contains("fake-asr-private-key"));
    }
    #[test]
    fn dictation_settings_bridge_applies_enhancement_switch_without_mutating_settings() {
        let mut settings = BackendSettings::default();
        settings.enhancements_enabled = false;
        settings.dictation.enabled = true;
        settings.dictation.api_key = "fake-asr-private-key".to_string();
        let result = settings_payload_value(settings.clone(), String::new()).unwrap();
        assert_eq!(result["dictation"]["enabled"], false);
        assert_eq!(result["dictation"]["apiKeyConfigured"], true);
        assert!(settings.dictation.enabled);
        assert_eq!(settings.dictation.api_key, "fake-asr-private-key");
    }
}

async fn settings_value(
    ctx: &BridgeContext,
    result: anyhow::Result<BackendSettings>,
) -> anyhow::Result<Value> {
    let settings = result?;
    let codex_app_version = ctx.settings.codex_app_version().await.unwrap_or_default();
    settings_payload_value(settings, codex_app_version)
}

fn backend_status_value(
    status: anyhow::Result<Value>,
    settings: anyhow::Result<BackendSettings>,
) -> anyhow::Result<Value> {
    let mut status = status?;
    if let Some(object) = status.as_object_mut() {
        let hide = settings
            .map(|settings| crate::assets::hide_official_usage_alert_config(&settings))
            .unwrap_or(false);
        object.insert("hideOfficialUsageAlert".to_string(), Value::Bool(hide));
    }
    Ok(status)
}

fn result_value<T>(result: anyhow::Result<T>) -> anyhow::Result<Value>
where
    T: serde::Serialize,
{
    Ok(serde_json::to_value(result?)?)
}

fn stepwise_settings_value(result: anyhow::Result<BackendSettings>) -> anyhow::Result<Value> {
    let settings = result?;
    Ok(json!({
        "status": "ok",
        "settings": crate::stepwise::public_settings(&settings),
    }))
}

async fn stepwise_generate_value(
    result: anyhow::Result<BackendSettings>,
    payload: Value,
) -> anyhow::Result<Value> {
    let settings = result?;
    let request = payload.get("request").cloned().unwrap_or(payload);
    let request =
        serde_json::from_value::<crate::stepwise::StepwiseRequest>(request).unwrap_or_default();
    crate::stepwise::generate(request, &settings).await
}

async fn stepwise_test_value(
    result: anyhow::Result<BackendSettings>,
    payload: Value,
) -> anyhow::Result<Value> {
    let settings = crate::stepwise::settings_with_payload(result?, &payload);
    crate::stepwise::test_connection(&settings).await
}

async fn llm_proxy_value(payload: Value) -> anyhow::Result<Value> {
    let url = validate_llm_proxy_url(
        payload
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )?;
    let method = payload
        .get("method")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("POST");
    if !method.eq_ignore_ascii_case("POST") {
        anyhow::bail!("LLM Bridge 仅支持 POST 请求");
    }

    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(60_000)
        .clamp(1_000, 60_000);
    let body = payload
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if body.len() > 1_048_576 {
        anyhow::bail!("LLM Bridge 请求体过大");
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client
        .post(url)
        .headers(llm_proxy_headers(&payload)?)
        .body(body)
        .send()
        .await?;
    let http_status = response.status().as_u16();
    let ok = response.status().is_success();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if let Some(length) = response.content_length() {
        if length > 4 * 1024 * 1024 {
            anyhow::bail!("LLM Bridge 响应体过大");
        }
    }
    let bytes = response.bytes().await?;
    if bytes.len() > 4 * 1024 * 1024 {
        anyhow::bail!("LLM Bridge 响应体过大");
    }
    let body_text = String::from_utf8_lossy(&bytes).to_string();
    let body_json = if content_type
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("application/json"))
    {
        serde_json::from_slice::<Value>(&bytes).ok()
    } else {
        None
    };

    Ok(json!({
        "status": "ok",
        "http_status": http_status,
        "ok": ok,
        "body_text": body_text,
        "body_json": body_json,
    }))
}

fn validate_llm_proxy_url(raw: &str) -> anyhow::Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| anyhow::anyhow!("Base URL 格式无效"))?;
    if url.scheme() != "https" {
        anyhow::bail!("Base URL 必须使用 HTTPS");
    }
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("Base URL 不得包含用户名或密码");
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("Base URL 缺少主机名"))?;
    if is_blocked_llm_proxy_host(host) {
        anyhow::bail!("Base URL 不得指向本机或私有网络");
    }
    Ok(url)
}

fn is_blocked_llm_proxy_host(host: &str) -> bool {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
    {
        return true;
    }
    if let Ok(ip) = std::net::IpAddr::from_str(&host) {
        return match ip {
            std::net::IpAddr::V4(ip) => {
                ip.is_loopback()
                    || ip.is_private()
                    || ip.is_link_local()
                    || ip.is_multicast()
                    || ip.is_broadcast()
                    || ip.is_documentation()
                    || ip.octets()[0] == 0
                    || ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1])
            }
            std::net::IpAddr::V6(ip) => {
                ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_multicast()
                    || (ip.segments()[0] & 0xfe00) == 0xfc00
                    || (ip.segments()[0] & 0xffc0) == 0xfe80
            }
        };
    }
    false
}

fn llm_proxy_headers(payload: &Value) -> anyhow::Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let Some(raw_headers) = payload.get("headers").and_then(Value::as_object) else {
        return Ok(headers);
    };
    for (name, value) in raw_headers {
        if !is_allowed_llm_proxy_header(name) {
            continue;
        }
        let Some(value) = value.as_str() else {
            continue;
        };
        let header_name = HeaderName::from_bytes(name.as_bytes())?;
        let header_value = HeaderValue::from_str(value)?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

fn is_allowed_llm_proxy_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "accept"
            | "api-key"
            | "anthropic-beta"
            | "anthropic-version"
            | "authorization"
            | "content-type"
            | "openai-organization"
            | "openai-project"
            | "x-api-key"
    )
}

fn diagnostic_log_value(payload: Value) -> anyhow::Result<Value> {
    let event = payload
        .get("event")
        .and_then(Value::as_str)
        .map(sanitize_diagnostic_event)
        .unwrap_or_else(|| "event".to_string());
    crate::diagnostic_log::append_diagnostic_log(&format!("renderer.{event}"), payload)?;
    Ok(json!({
        "status": "ok",
        "message": "日志已记录"
    }))
}

fn sanitize_diagnostic_event(event: &str) -> String {
    let sanitized = event
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "event".to_string()
    } else {
        sanitized
    }
}

fn archived_thread_value(result: anyhow::Result<Option<SessionRef>>) -> anyhow::Result<Value> {
    Ok(match result? {
        Some(session) => json!({"session_id": session.session_id, "title": session.title}),
        None => json!({"session_id": "", "title": ""}),
    })
}

fn failed_from_error(payload: &Value, error: anyhow::Error) -> Value {
    json!({
        "status": "failed",
        "session_id": payload
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "message": error.to_string()
    })
}

fn session_from_payload(payload: &Value) -> SessionRef {
    let parse_host = |value: &Value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(ToString::to_string)
    };
    // 双名称不一致同样属于未知来源，不能通过优先读取某个字段放宽归属。
    let host_id = match (payload.get("host_id"), payload.get("hostId")) {
        (Some(left), Some(right)) => {
            let left = parse_host(left);
            let right = parse_host(right);
            if left == right { left } else { None }
        }
        (Some(value), None) | (None, Some(value)) => parse_host(value),
        _ => None,
    };
    SessionRef {
        session_id: payload
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        title: payload
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        host_id,
    }
}

pub fn devtools_url(debug_port: u16, target_id: &str) -> String {
    format!(
        "http://127.0.0.1:{debug_port}/devtools/inspector.html?ws=127.0.0.1:{debug_port}/devtools/page/{target_id}"
    )
}

fn empty_user_script_inventory() -> Value {
    json!({
        "enabled": true,
        "scripts": []
    })
}

#[cfg(test)]
mod dictation_bridge_route_tests {
    use super::*;

    struct IsolatedSettings(BackendSettings);

    #[async_trait]
    impl BridgeSettingsService for IsolatedSettings {
        async fn get_settings(&self) -> anyhow::Result<BackendSettings> {
            Ok(self.0.clone())
        }
        async fn set_settings(&self, _payload: Value) -> anyhow::Result<BackendSettings> {
            anyhow::bail!("test settings are read-only")
        }
    }

    fn context() -> BridgeContext {
        BridgeContext::new(
            Arc::new(IsolatedSettings(BackendSettings::default())),
            Arc::new(CoreRuntimeService::new(
                0,
                StatusStore::new(PathBuf::from("unused-test-status.json")),
            )),
            Arc::new(UnavailableDataService),
        )
    }

    #[tokio::test]
    async fn dictation_bridge_routes_return_sanitized_failure_without_payload_echo() {
        for route in ["/dictation/transcribe", "/dictation/cancel"] {
            let result = handle_bridge_request(
                context(),
                route,
                json!({
                    "helperToken": "fake-secret-token",
                    "audioBase64": "fake-secret-audio",
                    "session_id": "fake-secret-session"
                }),
            )
            .await;
            assert_eq!(result["status"], "failed");
            assert_eq!(result["session_id"], "");
            assert!(!result.to_string().contains("fake-secret"));
            assert!(result["message"].as_str().unwrap().contains("未授权"));
        }
    }
}

#[cfg(test)]
mod dictation_disabled_route_tests {
    use super::*;
    struct DisabledSettings;
    #[async_trait]
    impl BridgeSettingsService for DisabledSettings {
        async fn get_settings(&self) -> anyhow::Result<BackendSettings> {
            Ok(BackendSettings {
                enhancements_enabled: false,
                dictation: crate::settings::DictationSettings {
                    enabled: true,
                    api_key: "fake-asr-key".into(),
                    ..Default::default()
                },
                ..Default::default()
            })
        }
        async fn set_settings(&self, _payload: Value) -> anyhow::Result<BackendSettings> {
            anyhow::bail!("read-only test")
        }
    }
    #[tokio::test]
    async fn dictation_bridge_disables_status_and_transcribe_when_enhancements_are_off() {
        let ctx = BridgeContext::new(
            Arc::new(DisabledSettings),
            Arc::new(CoreRuntimeService::new(
                0,
                StatusStore::new(PathBuf::from("unused-test-status.json")),
            )),
            Arc::new(UnavailableDataService),
        );
        let status = handle_bridge_request(ctx.clone(), "/dictation/status", json!({})).await;
        assert_eq!(status["enabled"], false);
        assert!(status.get("helperToken").is_none());
        let response = handle_bridge_request(ctx, "/dictation/transcribe", json!({
            "requestId": uuid::Uuid::new_v4().to_string(), "helperToken": crate::dictation::helper_token(), "audioBase64": "AA==", "mimeType": "audio/wav"
        })).await;
        assert_eq!(response["status"], "failed");
        assert!(response["message"].as_str().unwrap().contains("未启用"));
    }
}
