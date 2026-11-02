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
    #[cfg(test)]
    pub(crate) fn with_test_settings(store: SettingsStore, app_dir: PathBuf) -> Self {
        Self::new(
            Arc::new(CoreSettingsService { store, app_dir: Some(app_dir.clone()) }),
            Arc::new(CoreRuntimeService::new(0, StatusStore::new(app_dir.join("status.json")))),
            Arc::new(UnavailableDataService),
        )
    }

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

    /// Agent 桌宠图片缓存目录；测试可覆盖成临时目录，避免写进真实状态目录。
    fn agent_pet_cache_dir(&self) -> PathBuf {
        crate::agent_pet::default_cache_dir()
    }

    /// Agent 桌宠自定义音效目录；测试可覆盖成临时目录。
    fn agent_pet_custom_dir(&self) -> PathBuf {
        crate::agent_pet::default_custom_dir()
    }

    fn whale_ledger_path(&self) -> PathBuf {
        // 账本固定在状态目录：迁移设置文件路径后不能跟着换位置，否则余额历史看似清空。
        crate::paths::default_app_state_dir().join("whale-balance.sqlite3")
    }

    /// 有没有启用中的拓展包在清单里声明了这条路由。桌宠 / 大肥鱼的数据路由（图片、音效、用量、余额）
    /// 只在有拓展用它时才干活：Agent 桌宠拓展停用后不联网、不扫会话日志、不查余额。测试可覆盖。
    fn extension_route_declared(&self, route: &str) -> bool {
        cached_route_declared(route)
    }
}

/// [`BridgeSettingsService::extension_route_declared`] 的默认实现：按默认拓展目录判断，3 秒缓存。
fn cached_route_declared(route: &str) -> bool {
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, std::collections::BTreeMap<String, bool>)>> =
        std::sync::Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.as_ref().is_none_or(|(at, _)| at.elapsed() >= std::time::Duration::from_secs(3)) {
        *cache = Some((std::time::Instant::now(), Default::default()));
    }
    let (_, entries) = cache.as_mut().expect("cache initialized");
    if let Some(value) = entries.get(route) {
        return *value;
    }
    let declared = crate::user_scripts::default_user_script_manager().route_declared(route);
    entries.insert(route.to_string(), declared);
    declared
}

/// 大肥鱼数据路由用的设置：拓展用到了就按「选中大肥鱼」处理，余额接口按请求里带的 `balance`（来自 Agent 桌宠拓展的配置）。
fn whale_settings(mut settings: BackendSettings, payload: &Value) -> BackendSettings {
    settings.agent_pet = crate::settings::AgentPetSettings::whale_widget();
    if let Some(balance) = payload.get("balance").and_then(Value::as_object) {
        if let Some(protocol) = balance.get("protocol").and_then(Value::as_str) {
            settings.codex_app_whale_balance_protocol = match protocol {
                "custom" | "off" => protocol.to_string(),
                _ => "auto".to_string(),
            };
        }
        if let Some(path) = balance.get("path").and_then(Value::as_str) {
            settings.codex_app_whale_balance_path = path.trim().to_string();
        }
        if let Some(field) = balance.get("field").and_then(Value::as_str) {
            settings.codex_app_whale_balance_field = field.trim().to_string();
        }
        if let Some(currency) = balance.get("currency").and_then(Value::as_str) {
            settings.codex_app_whale_balance_currency = currency.trim().to_ascii_uppercase();
        }
        if let Some(scale) = balance.get("scale").and_then(Value::as_f64).filter(|scale| scale.is_finite() && *scale > 0.0) {
            settings.codex_app_whale_balance_scale = scale;
        }
    }
    settings
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
    /// 保存一个拓展包的配置（Codex 里「拓展」详情页的设置面板用），回 `{status, config}`，config 是生效配置。
    async fn set_extension_config(&self, key: String, values: serde_json::Map<String, Value>) -> anyhow::Result<Value> {
        let config = crate::user_scripts::default_user_script_manager().set_extension_config(&key, &values)?;
        Ok(json!({ "status": "ok", "config": config }))
    }
    /// 拓展的 README（Codex 里「拓展」详情页的「细节」页签用），见 [`crate::user_scripts::UserScriptManager::extension_readme`]。
    async fn extension_readme(&self, key: String) -> anyhow::Result<Value> {
        crate::user_scripts::default_user_script_manager().extension_readme(&key)
    }
    async fn reload_user_scripts(&self) -> anyhow::Result<Value>;
    async fn load_user_scripts(&self) -> anyhow::Result<Value> {
        self.user_script_inventory().await
    }
    /// 列出拓展可注入的 Codex 页面。默认关闭，具体 runtime 再接 CDP。
    async fn cdp_targets(&self) -> anyhow::Result<Value> {
        anyhow::bail!("CDP targets are unavailable")
    }
    /// 在指定 Codex 页面执行表达式。默认关闭。
    async fn cdp_evaluate(&self, _payload: Value) -> anyhow::Result<Value> {
        anyhow::bail!("CDP evaluate is unavailable")
    }
    /// 按需开启 app-server 客户端捕获（拓展的 codex/request 中间件需要）。默认不可用。
    async fn request_app_server_client_capture(&self) -> anyhow::Result<Value> {
        anyhow::bail!("app-server client capture is unavailable")
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
    /// Codex 输入框下方状态栏的轻量用量快照：只给最近的 token 事件，不返回全量历史。
    /// 内置状态栏和拓展（路由白名单里的只读项）共用。
    async fn thread_usage_live(&self, _session: SessionRef) -> anyhow::Result<Value> {
        Ok(json!({"status": "unavailable", "message": "本机用量快照暂不可用", "events": []}))
    }
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
}

/// 拓展脚本在独立 Codex 页面（例如宠物 overlay）执行表达式。
/// 只走 Runtime.evaluate，且必须指定 kind 或 target_id，避免误打到管理页。
pub const EXTENSION_CDP_EXPRESSION_MAX_BYTES: usize = 256 * 1024;

fn nonempty_payload_str<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub async fn evaluate_extension_cdp(debug_port: u16, payload: &Value) -> anyhow::Result<Value> {
    let expression = nonempty_payload_str(payload, "expression")
        .ok_or_else(|| anyhow::anyhow!("expression 不能为空"))?;
    if expression.len() > EXTENSION_CDP_EXPRESSION_MAX_BYTES {
        anyhow::bail!(
            "expression 超过 {} 字节上限",
            EXTENSION_CDP_EXPRESSION_MAX_BYTES
        );
    }
    let target_id = nonempty_payload_str(payload, "target_id")
        .or_else(|| nonempty_payload_str(payload, "targetId"));
    let kind = nonempty_payload_str(payload, "kind");
    let targets = crate::cdp::list_targets(debug_port).await?;
    let selected = crate::cdp::select_extension_cdp_targets(&targets, target_id, kind)?;
    let mut results = Vec::new();
    for target in selected {
        let websocket = target
            .web_socket_debugger_url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("CDP 页面没有 WebSocket URL"))?;
        let result = match crate::bridge::evaluate_script_return_by_value(websocket, expression)
            .await
        {
            Ok(response) => json!({
                "id": target.id,
                "ok": true,
                "value": response.pointer("/result/result/value").cloned().unwrap_or(Value::Null),
            }),
            Err(error) => json!({
                "id": target.id,
                "ok": false,
                "error": error.to_string(),
            }),
        };
        results.push(result);
    }
    Ok(json!({ "status": "ok", "results": results }))
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
            let result = ctx.settings.set_settings(payload.clone()).await;
            // Codex 设置页改了设置：通知管理器重新读取，两边开关保持一致。
            if result.is_ok() {
                crate::settings::notify_external_settings_change();
            }
            settings_value(&ctx, result).await
        }
        // 只读：设置项清单（标题、说明、分组等），注入页据此生成设置行。
        "/settings/catalog" => crate::settings_catalog::enhancement_settings_catalog(),
        // 开放给拓展的设置只读子集：当前供应商的名称、模式、模型与上下文窗口。整份设置（/settings/get）仍不开放。
        "/settings/public" => match ctx.settings.get_settings().await {
            Ok(settings) => {
                let codex_app_version = ctx.settings.codex_app_version().await.unwrap_or_default();
                Ok(public_settings_value(&settings, codex_app_version))
            }
            Err(error) => Err(error),
        },
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
        // Codex++ 内部用：「拓展」详情页里拓展自己的设置面板保存配置。只写这一个拓展清单里声明过的配置项，
        // 不进拓展路由白名单（拓展经 ctx.registerSettingsPanel 拿到的 saveConfig 只能存它自己的）。
        "/user-scripts/set-config" => {
            let key = payload.get("key").and_then(Value::as_str).unwrap_or_default().trim().to_string();
            let values = payload.get("values").and_then(Value::as_object).cloned().unwrap_or_default();
            if key.is_empty() {
                Ok(json!({ "status": "failed", "message": "拓展 key 不能为空" }))
            } else {
                match ctx.runtime.set_extension_config(key, values).await {
                    Ok(value) => Ok(value),
                    Err(error) => Ok(json!({ "status": "failed", "message": format!("{error:#}") })),
                }
            }
        }
        // Codex++ 内部用：「拓展」详情页「细节」页签读拓展的 README。只读，不进拓展路由白名单。
        "/user-scripts/readme" => {
            let key = payload.get("key").and_then(Value::as_str).unwrap_or_default().trim().to_string();
            if key.is_empty() {
                Ok(json!({ "status": "failed", "message": "拓展 key 不能为空" }))
            } else {
                match ctx.runtime.extension_readme(key).await {
                    Ok(value) => Ok(value),
                    Err(error) => Ok(json!({ "status": "failed", "message": format!("{error:#}") })),
                }
            }
        }
        "/user-scripts/load" => ctx.runtime.load_user_scripts().await,
        "/user-scripts/reload" => ctx.runtime.reload_user_scripts().await,
        "/cdp/targets" => ctx.runtime.cdp_targets().await,
        "/cdp/evaluate" => ctx.runtime.cdp_evaluate(payload.clone()).await,
        // 系统鼠标位置：拓展写「桌宠跟随鼠标」这类功能用（页面里拿不到窗口外的鼠标）。只读。
        "/system/cursor-position" => crate::launcher::cursor_position()
            .map(|(x, y)| json!({ "status": "ok", "x": x, "y": y })),
        // Codex++ 内部用：拓展要改写发往 app-server 的请求时补开客户端捕获。不进拓展路由白名单。
        "/extensions/request-capture" => ctx.runtime.request_app_server_client_capture().await,
        // 拓展常驻后台：页面侧 ctx.backend 用。进程由处理桥接的这个进程托管，与具体运行时无关。
        "/extensions/backend/call" => crate::extension_backend::route_call(&payload).await,
        "/extensions/backend/events" => crate::extension_backend::route_events(&payload).await,
        "/extensions/backend/stop" => crate::extension_backend::route_stop(&payload).await,
        "/extensions/services/call" => crate::extension_backend::route_service_call(&payload).await,
        // 拓展自己调模型（ctx.llm）：经协议代理用当前供应商，不跑拓展网关中间件。
        "/extensions/llm/complete" => crate::extension_llm::route_complete(&payload).await,
        "/extensions/llm/models" => crate::extension_llm::route_models(),
        "/extensions/history" => {
            let payload = payload.clone();
            tokio::task::spawn_blocking(move || crate::extension_history::route(&payload))
                .await
                .map_err(anyhow::Error::from)
                .and_then(|result| result)
        }
        "/devtools/open" => ctx.runtime.open_devtools().await,
        "/manager/open" => ctx.runtime.open_manager(payload.clone()).await,
        "/manager/open-transient" => ctx.runtime.open_transient_manager(payload.clone()).await,
        "/backend/status" => backend_status_value(
            ctx.runtime.backend_status().await,
            ctx.settings.get_settings().await,
        ),
        "/codex-model-catalog" | "/codex-config-model" => ctx.runtime.codex_model_catalog().await,
        // 开放给拓展的模型目录：只给模型列表与每模型元数据（含上下文窗口），去掉来源地址、config 路径。
        "/model-catalog/public" => ctx.runtime.codex_model_catalog().await.map(public_model_catalog_value),
        "/diagnostics/log" => diagnostic_log_value(payload.clone()),
        "/llm-proxy" => llm_proxy_value(payload.clone()).await,
        "/ads" => ctx.runtime.ads().await,
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
        "/thread-usage-live" => {
            ctx.data
                .thread_usage_live(session_from_payload(&payload))
                .await
        }
        // 大肥鱼（Agent 桌宠拓展）的数据路由：拓展白名单里开放，但只有启用中的拓展在清单里声明了它才干活。
        // 本机用量统计与余额查询都在 Codex++ 里做，供应商的 Key 不交给页面。
        "/whale/media" if ctx.settings.extension_route_declared(path) => Ok(crate::assets::whale_media()),
        "/whale/media" => Ok(json!({"status":"disabled"})),
        "/whale/history" => match ctx.settings.get_settings().await {
            Ok(_)
                if ctx.settings.extension_route_declared(path)
                    && crate::whale_full::history_enabled(&ctx.settings.whale_ledger_path()) =>
            {
                ctx.data.whale_history(payload.clone()).await
            }
            Ok(_) => Ok(json!({"status":"disabled"})),
            Err(_) => Ok(json!({"status":"unavailable","message":"无法读取挂件设置"})),
        },
        "/whale/full" if !ctx.settings.extension_route_declared(path) => {
            Ok(json!({"status":403,"body":{"ok":false,"error":"Codex 用量挂件已关闭"}}))
        }
        "/whale/full" => match ctx.settings.get_settings().await {
            Ok(settings) => {
                let settings = whale_settings(settings, &payload);
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
            Ok(settings) if ctx.settings.extension_route_declared(path) => {
                let settings = whale_settings(settings, &payload);
                Ok(crate::whale::balance(&settings, ctx.settings.whale_ledger_path()).await)
            }
            Ok(_) => Ok(json!({"status":"disabled","provider":{"id":"","name":"","accountId":""},"balances":[],"updatedAt":0,"stale":false})),
            Err(_) => Ok(crate::whale::unavailable("无法读取挂件设置")),
        },
        "/whale/session" => match ctx.settings.get_settings().await {
            Ok(_)
                if ctx.settings.extension_route_declared(path)
                    && crate::whale_full::history_enabled(&ctx.settings.whale_ledger_path()) =>
            {
                ctx.data.whale_session(session_from_payload(&payload)).await
            }
            Ok(_) => Ok(json!({"status": "disabled"})),
            Err(_) => Ok(json!({"status": "unavailable", "message": "无法读取挂件设置"})),
        },
        // Agent 桌宠拓展的数据路由（图库、图片、音效、导入自定义音效）：参数来自拓展配置，按原来的设置规则校验；
        // 下载走 Codex++ 的地址校验（只 https、拦截本机地址）与缓存。只有启用中的拓展声明了它才干活。
        "/agent-pet/catalog" | "/agent-pet/image" | "/agent-pet/sound" | "/agent-pet/import-sound" => {
            if ctx.settings.extension_route_declared(path) {
                Ok(crate::agent_pet::route(path, &payload, &ctx.settings.agent_pet_cache_dir(), &ctx.settings.agent_pet_custom_dir())
                    .await
                    .unwrap_or_else(|| json!({"status": "failed", "message": "未知的桌宠路由"})))
            } else {
                Ok(json!({"status": "disabled"}))
            }
        }
        // 背景图拓展的读图路由：只读这个拓展配置里的那张本机图片（页面拿不到本机文件）。
        // 只有启用中的拓展在清单里声明了它才干活。
        "/image-overlay/image" => {
            if ctx.settings.extension_route_declared(path) {
                tokio::task::spawn_blocking(|| {
                    crate::bundled_extensions::image_overlay_image(&crate::user_scripts::default_user_script_manager())
                })
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))
            } else {
                Ok(json!({"status": "disabled", "dataUrl": ""}))
            }
        }
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
        "/side-chat/source" | "/side-chat/save" => {
            let request = payload.clone();
            let operation = if path == "/side-chat/source" {
                crate::side_chat::source_snapshot
            } else {
                crate::side_chat::save
            };
            tokio::task::spawn_blocking(move || {
                operation(&crate::codex_home::default_codex_home_dir(), &request)
            })
            .await
            .unwrap_or_else(|error| Err(anyhow::anyhow!(error.to_string())))
        }
        "/request-log/recent" => {
            // 只读：最近已完成的模型请求（与托盘「实时请求」同一份 request-log.jsonl），
            // 开放给拓展做请求提示这类功能。进行中的请求只在代理进程内存里，这里不返回，
            // 需要的话走 /proxy-status。
            let limit = payload
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(20)
                .clamp(1, 100) as usize;
            tokio::task::spawn_blocking(move || {
                crate::request_log::read_recent_request_logs_at(
                    &crate::request_log::request_log_path(),
                    limit,
                )
            })
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .and_then(|result| result.map_err(|error| anyhow::anyhow!(error.to_string())))
            .map(|entries| json!({ "status": "ok", "entries": entries }))
        }
        "/proxy-status" => {
            // 代理进程的实时请求状态只存在管理器内存里；桥接进程通过本地 helper
            // 的只读路由取一份快照，供 Codex 页面里的状态条轮询。
            let client = reqwest::Client::new();
            match client
                .post("http://127.0.0.1:57321/proxy-status")
                .json(&serde_json::json!({}))
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
            {
                Ok(response) => response
                    .json::<Value>()
                    .await
                    .map_err(|error| anyhow::anyhow!(error.to_string())),
                Err(error) => Ok(serde_json::json!({
                    "status": "failed",
                    "message": error.to_string()
                })),
            }
        }
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
                let result = user_scripts.set_script_enabled(&key, enabled);
                // 与管理器一致：拓展带的钩子 / Codex 资产跟着启停写回 Codex 配置。
                crate::extension_hooks::sync_default(user_scripts);
                result?;
                user_scripts.inventory()
            }
            None => Ok(empty_user_script_inventory()),
        }
    }

    async fn extension_readme(&self, key: String) -> anyhow::Result<Value> {
        match &self.user_scripts {
            Some(user_scripts) => user_scripts.extension_readme(&key),
            None => anyhow::bail!("没有找到拓展 {key}"),
        }
    }

    async fn delete_user_script(&self, key: String) -> anyhow::Result<Value> {
        match &self.user_scripts {
            Some(user_scripts) => {
                let result = user_scripts.delete_user_script(&key);
                crate::extension_hooks::sync_default(user_scripts);
                result?;
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

    async fn cdp_targets(&self) -> anyhow::Result<Value> {
        crate::cdp::list_extension_cdp_targets(self.debug_port).await
    }

    async fn cdp_evaluate(&self, payload: Value) -> anyhow::Result<Value> {
        evaluate_extension_cdp(self.debug_port, &payload).await
    }

    async fn request_app_server_client_capture(&self) -> anyhow::Result<Value> {
        let websocket_url = self
            .websocket_url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Codex 页面尚未连接"))?;
        let started = crate::bridge::request_app_server_client_capture(websocket_url);
        Ok(json!({ "status": "ok", "started": started }))
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
    let managed_session_connection = managed_session_connection(&settings);
    let active_relay_session_provider = settings.active_relay_session_provider();
    let active_relay_codex_provider = crate::model_catalog::codex_model_provider_for_relay_profile(
        &crate::relay_config::default_codex_home_dir(),
        &settings.active_relay_profile(),
    );
    settings.dictation = crate::dictation::effective_settings(&settings);
    let native_fast = crate::native_fast::runtime_settings(&settings);
    // 页面心跳比较这两项，发拓展事件 relayProfile/switched、modelCatalog/changed（见 40-backend-settings.js）。
    let active_relay_profile_public = public_active_relay_profile(&settings);
    let catalog_fingerprint = model_catalog_fingerprint(&settings);
    let mut value = serde_json::to_value(settings)?;
    fn remove_named_keys(value: &mut Value) {
        match value {
            Value::Object(object) => {
                object.remove("apiKeys");
                for child in object.values_mut() {
                    remove_named_keys(child);
                }
            }
            Value::Array(array) => {
                for child in array {
                    remove_named_keys(child);
                }
            }
            _ => {}
        }
    }
    remove_named_keys(&mut value);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "weixinDesktopSync".to_string(),
            serde_json::to_value(crate::connect::desktop_sync::pending())?,
        );
        object.remove("codexAppStepwiseApiKey");
        if let Some(dictation) = object.get_mut("dictation").and_then(Value::as_object_mut) {
            let configured = dictation
                .remove("apiKey")
                .and_then(|value| value.as_str().map(|value| !value.trim().is_empty()))
                .unwrap_or(false);
            dictation.insert("apiKeyConfigured".to_string(), json!(configured));
        }
        object.insert(
            "managedSessionConnection".to_string(),
            managed_session_connection,
        );
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
        object.insert("activeRelayProfilePublic".to_string(), active_relay_profile_public);
        object.insert("modelCatalogFingerprint".to_string(), Value::String(catalog_fingerprint));
        object.insert(
            "nativeFastLocalApiConfigured".to_string(),
            Value::Bool(native_fast.native_fast_local_api_configured),
        );
        // 原生输入预测与 Fast 共用同一份本机自定义 API 判定，只读一次 config.toml。
        object.insert(
            "nativePredictionsLocalApiConfigured".to_string(),
            Value::Bool(native_fast.native_fast_local_api_configured),
        );
    }
    Ok(value)
}

/// 当前供应商里拓展能看的部分（`/settings/public` 与设置载荷的 `activeRelayProfilePublic` 共用）。
/// 只有名称、模式、模型和窗口这类展示用字段：不含地址、Key、config.toml / auth.json 内容。
fn public_active_relay_profile(settings: &BackendSettings) -> Value {
    let profile = settings.active_relay_profile();
    json!({
        "id": profile.id,
        "name": profile.name,
        "relayMode": profile.relay_mode,
        "protocol": profile.protocol,
        "model": crate::relay_config::relay_profile_model(&profile),
        "contextWindow": profile.context_window,
        "autoCompactLimit": profile.auto_compact_limit,
        "aggregate": settings.active_aggregate_relay_profile().is_some(),
    })
}

/// 当前供应商的模型目录指纹：决定 Codex 模型目录的字段（模型列表、每模型窗口 / 压缩 / 元数据、聚合成员、换供应商）
/// 变了它就变。页面比较前后两次的值发 `modelCatalog/changed`；只用来判断「变没变」，不承诺跨版本稳定。
fn model_catalog_fingerprint(settings: &BackendSettings) -> String {
    use std::hash::{Hash, Hasher};
    let profile = settings.active_relay_profile();
    let aggregate = settings
        .active_aggregate_relay_profile()
        .and_then(|aggregate| serde_json::to_string(&aggregate).ok())
        .unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (
        settings.relay_profiles_enabled,
        profile.id.as_str(),
        profile.name.as_str(),
        crate::relay_config::relay_profile_model(&profile),
        profile.model_list.as_str(),
        profile.model_windows.as_str(),
        profile.model_auto_compact.as_str(),
        profile.model_metadata.as_str(),
        profile.context_window.as_str(),
        profile.auto_compact_limit.as_str(),
        aggregate,
    )
        .hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// `/settings/public`：拓展可读的设置子集。
fn public_settings_value(settings: &BackendSettings, codex_app_version: String) -> Value {
    json!({
        "status": "ok",
        "codexPlusVersion": crate::version::VERSION,
        "codexAppVersion": codex_app_version,
        "enhancementsEnabled": settings.enhancements_enabled,
        "relayProfilesEnabled": settings.relay_profiles_enabled,
        "activeProfile": public_active_relay_profile(settings),
        "modelCatalogFingerprint": model_catalog_fingerprint(settings),
    })
}

/// `/model-catalog/public` 保留的字段。`sources`（带供应商地址）、`path`（本机 config 路径）、`responses_api` 不给拓展。
const PUBLIC_MODEL_CATALOG_KEYS: &[&str] = &[
    "status",
    "message",
    "service_tier",
    "model",
    "model_provider",
    "codex_model_provider",
    "provider_name",
    "default_model",
    "models",
    "modelMetadata",
];

fn public_model_catalog_value(catalog: Value) -> Value {
    let mut public = serde_json::Map::new();
    if let Some(object) = catalog.as_object() {
        for key in PUBLIC_MODEL_CATALOG_KEYS {
            if let Some(value) = object.get(*key) {
                public.insert((*key).to_string(), value.clone());
            }
        }
    }
    Value::Object(public)
}

fn managed_session_connection(settings: &BackendSettings) -> Value {
    // 固定所有自定义会话聚合的本地传输，模型聚合与故障转移共用这一入口。
    // 不要把成员的真实认证信息下发给 renderer。
    let Some(aggregate) = settings.active_aggregate_relay_profile() else {
        return Value::Null;
    };
    if !settings.relay_profiles_enabled
        || aggregate.session_provider != crate::settings::RelaySessionProvider::Custom
        || aggregate.sync_default_member_login
    {
        return Value::Null;
    }
    json!({
        "profileId": settings.active_relay_id,
        "provider": "codex_plus_managed",
        "baseUrl": crate::protocol_proxy::local_responses_proxy_base_url(
            crate::protocol_proxy::protocol_proxy_port()
        ),
    })
}

#[cfg(test)]
mod dictation_privacy_tests {
    use super::*;

    #[test]
    fn managed_session_connection_uses_settings_and_local_proxy_only() {
        use crate::settings::{AggregateRelayProfile, RelayMode, RelayProfile, RelaySessionProvider};
        let mut settings = BackendSettings {
            relay_profiles_enabled: true,
            active_relay_id: "aggregate".to_string(),
            relay_profiles: vec![RelayProfile {
                id: "aggregate".to_string(),
                relay_mode: RelayMode::Aggregate,
                ..RelayProfile::default()
            }],
            aggregate_relay_profiles: vec![AggregateRelayProfile {
                id: "aggregate".to_string(),
                name: String::new(),
                session_provider: RelaySessionProvider::Custom,
                model_aggregation_enabled: true,
                strategy: Default::default(),
                members: Vec::new(),
                routes: Vec::new(),
                default_relay_id: String::new(),
                sync_default_member_login: false,
                auto_review_relay_id: String::new(),
            }],
            ..BackendSettings::default()
        };
        let connection = managed_session_connection(&settings);
        assert_eq!(connection["profileId"], "aggregate");
        assert_eq!(connection["provider"], "codex_plus_managed");
        assert_eq!(
            connection["baseUrl"],
            crate::protocol_proxy::local_responses_proxy_base_url(
                crate::protocol_proxy::protocol_proxy_port()
            )
        );
        assert_eq!(connection.as_object().unwrap().len(), 3);
        // 模型聚合开关、故障转移策略及默认选择不改变本地会话连接地址。
        settings.aggregate_relay_profiles[0].model_aggregation_enabled = false;
        settings.aggregate_relay_profiles[0].strategy =
            crate::settings::AggregateRelayStrategy::Failover;
        settings.aggregate_relay_profiles[0].default_relay_id = "new-default".to_string();
        settings.relay_profiles[0].model = "new-model".to_string();
        assert_eq!(managed_session_connection(&settings), connection);
        settings.aggregate_relay_profiles[0].sync_default_member_login = true;
        assert!(managed_session_connection(&settings).is_null());
        settings.aggregate_relay_profiles[0].sync_default_member_login = false;
        settings.aggregate_relay_profiles[0].session_provider = RelaySessionProvider::Openai;
        assert!(managed_session_connection(&settings).is_null());
        settings.aggregate_relay_profiles[0].session_provider = RelaySessionProvider::Custom;
        settings.relay_profiles_enabled = false;
        assert!(managed_session_connection(&settings).is_null());
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
    fn settings_bridge_never_returns_named_key_lists() {
        let mut settings = BackendSettings::default();
        settings.relay_profiles[0].api_keys = vec![crate::settings::RelayApiKey {
            id: "backup".into(), name: "备用".into(), api_key: "fake-backup-secret".into(),
        }];
        let result = settings_payload_value(settings, String::new()).unwrap();
        assert!(result["relayProfiles"][0].get("apiKeys").is_none());
        assert!(!result.to_string().contains("fake-backup-secret"));
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
        let settings = settings.ok();
        let hide = settings
            .as_ref()
            .map(crate::assets::hide_official_usage_alert_config)
            .unwrap_or(false);
        object.insert("hideOfficialUsageAlert".to_string(), Value::Bool(hide));
        // 设置快照的版本。前端 5 秒一次的心跳靠它判断设置有没有变：没变就不用再拉一次整份
        // /settings/get（166 KB 的 JSON，编解码开销全落在渲染进程主线程上）。
        let revision = settings
            .as_ref()
            .map(|settings| settings.settings_revision.clone())
            .unwrap_or_default();
        object.insert("settingsRevision".to_string(), Value::from(revision));
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
