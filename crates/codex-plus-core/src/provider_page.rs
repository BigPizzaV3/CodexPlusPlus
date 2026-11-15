//! Codex 页面里的「供应商」页（注入分片 `43-provider-page.js`）用的后端。
//!
//! 页面只能调 bridge 路由，碰不到管理器的 Tauri 命令；而保存与切换供应商的完整流程
//! （归一化、聚合目录刷新、切换锁、live 文件回填与冲突处理、网关绑定、常驻 helper）都在管理器里。
//! 这里不复制那套流程：管理器启动时用 [`set_provider_page_host`] 注册它自己的保存 / 切换实现，
//! 页面与管理器走同一条代码路径。没有注册（例如独立运行的启动器）时路由明确报错。
//!
//! 字段 ↔ config.toml / auth.json 的联动在页面里由与管理器共用的 TS 纯函数完成
//! （`apps/codex-plus-manager/src/relay-profile-editing.ts`），这里只负责：
//! - 下发列表时把命名 Key 打码（`/settings/get` 本来就剥掉了 `apiKeys`，这里保持不下发明文）；
//! - 提交时按 id 把打码的 Key 还原成库里的真值，再交给宿主保存或切换；
//! - 网关接管条目（`codex-plus-gateway`）只能由网关维护，页面提交的版本一律以库里为准。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::gateway::apply::CODEX_GATEWAY_RELAY_ID;
use crate::relay_switch::LiveConflictResolution;
use crate::settings::{BackendSettings, RelayMode, RelayProfile, SettingsStore};
use crate::tools::ToolId;

/// 宿主切换的结果。`live_conflict` 有值时页面弹三选一，再带着选择重新提交。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderSwitchResult {
    pub ok: bool,
    pub message: String,
    pub live_conflict: Option<ProviderLiveConflict>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderLiveConflict {
    pub profile_id: String,
    pub profile_name: String,
}

/// 保存整份设置：`(settings, expected_revision) -> Ok(提示) / Err(原因)`。
pub type ProviderSaveHook =
    Arc<dyn Fn(BackendSettings, Option<String>) -> Result<String, String> + Send + Sync>;
/// 切换（并写 live 文件）：`(settings, previous_active_relay_id, resolution)`。
pub type ProviderSwitchHook = Arc<
    dyn Fn(BackendSettings, String, LiveConflictResolution) -> ProviderSwitchResult + Send + Sync,
>;

#[derive(Clone)]
pub struct ProviderPageHost {
    pub save: ProviderSaveHook,
    pub switch: ProviderSwitchHook,
    /// 网关供应商池：`(命令名, 参数) -> 命令返回的 JSON`。管理器按名字分发到它自己的
    /// `gateway_commands::*`（生效、价格填充、断开预览 token 都在那边），这里只做白名单与脱敏。
    pub gateway: ProviderGatewayHook,
}

/// 网关命令分发：`(command, args) -> {status, message, ...payload}`。
pub type ProviderGatewayHook = Arc<dyn Fn(&str, Value) -> Value + Send + Sync>;

/// 页面可以调的网关命令。按 Agent 生效的几项只允许作用在 Codex 上（见 [`gateway_command_allowed`]）。
pub const GATEWAY_PAGE_COMMANDS: &[&str] = &[
    "load_gateway",
    "create_gateway_provider",
    "update_gateway_provider",
    "delete_gateway_provider",
    "set_gateway_provider_enabled",
    "reorder_gateway_providers",
    "add_gateway_provider_key",
    "replace_gateway_provider_key",
    "delete_gateway_provider_key",
    "test_gateway_provider",
    "cancel_gateway_provider_test",
    "fetch_provider_models",
    "save_gateway_binding",
    "preview_gateway_disconnect",
    "disable_gateway_binding",
    "disable_gateway_binding_full_switch",
];

const GATEWAY_AGENT_COMMANDS: &[&str] = &[
    "save_gateway_binding",
    "preview_gateway_disconnect",
    "disable_gateway_binding",
    "disable_gateway_binding_full_switch",
];

/// 白名单内、且按 Agent 生效的命令只作用在 Codex 上（页面挂在 Codex 里，不该替别的 Agent 接入或断开）。
pub fn gateway_command_allowed(command: &str, args: &Value) -> Result<(), String> {
    if !GATEWAY_PAGE_COMMANDS.contains(&command) {
        return Err(format!("Codex 页面不支持网关命令 {command}"));
    }
    if GATEWAY_AGENT_COMMANDS.contains(&command)
        && args.get("agent").and_then(Value::as_str) != Some(ToolId::Codex.as_str())
    {
        return Err("Codex 页面只能接入或断开 Codex 自己的网关连接".to_string());
    }
    Ok(())
}

/// 网关命令的返回给页面前去掉本地网关令牌（`gateway.bindings.*.token`）。池条目本身已是打码视图。
pub fn redact_gateway_result(mut value: Value) -> Value {
    fn strip_binding_tokens(value: &mut Value) {
        if let Some(bindings) = value.get_mut("bindings").and_then(Value::as_object_mut) {
            for binding in bindings.values_mut() {
                if let Some(binding) = binding.as_object_mut() {
                    binding.remove("token");
                }
            }
        }
    }
    if let Some(gateway) = value.get_mut("gateway") {
        strip_binding_tokens(gateway);
    }
    strip_binding_tokens(&mut value);
    value
}

/// Codex 当前是否经网关接入：与 `gateway::apply::codex_switch_blocked_by_gateway` 同一判定。
pub fn codex_gateway_mode(settings: &BackendSettings, codex_binding_enabled: bool) -> bool {
    settings.active_relay_id == CODEX_GATEWAY_RELAY_ID && settings.relay_profiles_enabled && codex_binding_enabled
}

fn codex_binding_enabled() -> bool {
    crate::gateway::store::GatewayStore::default()
        .load()
        .ok()
        .and_then(|gateway| gateway.binding(&ToolId::Codex).map(|binding| binding.enabled))
        .unwrap_or(false)
}

static PROVIDER_PAGE_HOST: Mutex<Option<ProviderPageHost>> = Mutex::new(None);

/// 管理器启动时注册（见 `apps/codex-plus-manager/src-tauri/src/lib.rs`）。
pub fn set_provider_page_host(host: ProviderPageHost) {
    if let Ok(mut slot) = PROVIDER_PAGE_HOST.lock() {
        *slot = Some(host);
    }
}

fn registered_host() -> anyhow::Result<ProviderPageHost> {
    PROVIDER_PAGE_HOST
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .ok_or_else(|| anyhow::anyhow!("Codex++ 管理器未运行，暂时不能在 Codex 里修改供应商；请先打开 Codex++。"))
}

/// 打码后的 Key 预览：只留前 3 位和末 4 位，短 Key 全部隐藏。
pub fn api_key_preview(key: &str) -> String {
    let key = key.trim();
    let chars: Vec<char> = key.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    if chars.len() <= 8 {
        return "••••".to_string();
    }
    let head: String = chars[..3].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// 单个供应商的页面载荷：命名 Key 只给 `id` / `name` / 预览，明文留在服务端。
fn masked_profile_value(profile: &RelayProfile) -> anyhow::Result<Value> {
    let mut value = serde_json::to_value(profile)?;
    if let Some(object) = value.as_object_mut() {
        let keys = profile
            .api_keys
            .iter()
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "name": entry.name,
                    "apiKey": "",
                    "stored": !entry.api_key.trim().is_empty(),
                    "preview": api_key_preview(&entry.api_key),
                })
            })
            .collect::<Vec<_>>();
        object.insert("apiKeys".to_string(), Value::Array(keys));
        object.insert(
            "gatewayManaged".to_string(),
            Value::Bool(profile.id == CODEX_GATEWAY_RELAY_ID),
        );
    }
    Ok(value)
}

/// `/providers/list` 的载荷。`settings` 只含供应商相关的顶层字段，形状与管理器的设置一致，
/// 页面直接交给共用的 TS 函数（`createRelayProfile(settings)` 等）。
pub fn provider_list_value(settings: &BackendSettings) -> anyhow::Result<Value> {
    let profiles = settings
        .relay_profiles
        .iter()
        .map(masked_profile_value)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let defaults = BackendSettings::default();
    Ok(json!({
        "status": "ok",
        "message": "",
        "revision": settings.settings_revision,
        "settings": {
            "relayProfiles": profiles,
            "activeRelayId": settings.active_relay_id,
            "aggregateRelayProfiles": serde_json::to_value(&settings.aggregate_relay_profiles)?,
            "activeAggregateRelayId": settings.active_aggregate_relay_id,
            "relayProfilesEnabled": settings.relay_profiles_enabled,
        },
        "defaults": {
            "relayBaseUrl": defaults.relay_base_url,
            "relayProfiles": defaults
                .relay_profiles
                .iter()
                .map(masked_profile_value)
                .collect::<anyhow::Result<Vec<_>>>()?,
        },
        "gatewayRelayId": CODEX_GATEWAY_RELAY_ID,
    }))
}

/// 把页面提交的供应商里打码的 Key 还原成库里的真值。
///
/// `source` 是同 id 的已存供应商（复制出来的新供应商用 `keySources` 指向原供应商）。
/// `stored: true` 且 `apiKey` 为空的条目按 Key id 取回；取不到的（原 Key 已被删）丢弃，
/// 不留空 Key。用户新填的 Key（`apiKey` 非空）原样保留。
pub fn restore_masked_api_keys(profile: &mut Value, source: Option<&RelayProfile>) {
    let Some(object) = profile.as_object_mut() else {
        return;
    };
    object.remove("gatewayManaged");
    let Some(keys) = object.get_mut("apiKeys").and_then(Value::as_array_mut) else {
        return;
    };
    let restored = keys
        .drain(..)
        .filter_map(|entry| {
            let mut entry = entry.as_object()?.clone();
            let stored = entry.remove("stored").and_then(|value| value.as_bool()).unwrap_or(false);
            entry.remove("preview");
            let id = entry.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
            let plain = entry
                .get("apiKey")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default()
                .to_string();
            if !plain.is_empty() {
                return Some(Value::Object(entry));
            }
            if !stored {
                return None;
            }
            let secret = source?
                .api_keys
                .iter()
                .find(|candidate| candidate.id == id)
                .map(|candidate| candidate.api_key.clone())
                .filter(|value| !value.trim().is_empty())?;
            entry.insert("apiKey".to_string(), Value::String(secret));
            Some(Value::Object(entry))
        })
        .collect::<Vec<_>>();
    *keys = restored;
}

/// `/providers/commit` 的请求：整份供应商列表（含新增、删除、排序、编辑后的结果）。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCommitRequest {
    #[serde(default)]
    pub revision: String,
    pub relay_profiles: Vec<Value>,
    #[serde(default)]
    pub aggregate_relay_profiles: Option<Value>,
    #[serde(default)]
    pub active_aggregate_relay_id: Option<String>,
    /// 新供应商 id → 取 Key 的来源供应商 id（复制供应商时用）。
    #[serde(default)]
    pub key_sources: BTreeMap<String, String>,
    /// 提交后切到哪个供应商；不填表示保持当前。
    #[serde(default)]
    pub activate: Option<String>,
    #[serde(default)]
    pub conflict_resolution: LiveConflictResolution,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderCommitOutcome {
    pub switched: bool,
    pub message: String,
    pub live_conflict: Option<ProviderLiveConflict>,
}

/// 页面提交的整份供应商列表 → 待保存的设置。还原 Key、保住网关条目、校验当前供应商仍在。
pub fn settings_from_commit(
    stored: &BackendSettings,
    request: &ProviderCommitRequest,
) -> anyhow::Result<BackendSettings> {
    if !request.revision.trim().is_empty() && request.revision != stored.settings_revision {
        anyhow::bail!("settings_revision_conflict：供应商配置已在别处被修改，请刷新后再改。");
    }
    let stored_by_id: BTreeMap<&str, &RelayProfile> = stored
        .relay_profiles
        .iter()
        .map(|profile| (profile.id.as_str(), profile))
        .collect();
    let mut profiles = Vec::with_capacity(request.relay_profiles.len());
    let mut seen = std::collections::HashSet::new();
    for raw in &request.relay_profiles {
        let mut raw = raw.clone();
        let id = raw.get("id").and_then(Value::as_str).unwrap_or_default().trim().to_string();
        if id.is_empty() {
            anyhow::bail!("供应商缺少 id，未保存。");
        }
        if !seen.insert(id.clone()) {
            anyhow::bail!("供应商 id「{id}」重复，未保存。");
        }
        // 网关接管条目由网关生成与还原，页面不能改它，也不能删它（见下面补回）。
        if id == CODEX_GATEWAY_RELAY_ID {
            if let Some(existing) = stored_by_id.get(id.as_str()) {
                profiles.push((*existing).clone());
            }
            continue;
        }
        let source_id = request.key_sources.get(&id).map(String::as_str).unwrap_or(id.as_str());
        restore_masked_api_keys(&mut raw, stored_by_id.get(source_id).copied());
        let profile: RelayProfile = serde_json::from_value(raw)
            .with_context(|| format!("供应商「{id}」的格式不正确"))?;
        profiles.push(profile);
    }
    if let Some(gateway) = stored_by_id.get(CODEX_GATEWAY_RELAY_ID)
        && !profiles.iter().any(|profile| profile.id == CODEX_GATEWAY_RELAY_ID)
    {
        profiles.push((*gateway).clone());
    }
    if profiles.is_empty() {
        anyhow::bail!("至少要保留一个供应商。");
    }
    let target = request
        .activate
        .clone()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| stored.active_relay_id.clone());
    if !profiles.iter().any(|profile| profile.id == target) {
        anyhow::bail!("当前使用的供应商不能删除；请先切换到别的供应商。");
    }
    let mut next = stored.clone();
    next.relay_profiles = profiles;
    if let Some(aggregates) = &request.aggregate_relay_profiles {
        next.aggregate_relay_profiles =
            serde_json::from_value(aggregates.clone()).context("聚合供应商的格式不正确")?;
    }
    if let Some(active_aggregate) = &request.active_aggregate_relay_id {
        next.active_aggregate_relay_id = active_aggregate.clone();
    }
    next.active_relay_id = target;
    Ok(next)
}

/// 归一化后的供应商，用来判断当前供应商「实质上」有没有变（变了要重写 live 文件）。
fn normalized_for_compare(profile: &RelayProfile) -> Option<Value> {
    let mut profile = profile.clone();
    crate::relay_config::normalize_relay_profile_for_storage(&mut profile).ok()?;
    let mut value = serde_json::to_value(&profile).ok()?;
    if let Some(object) = value.as_object_mut() {
        // 只读的派生字段不参与比较。
        object.remove("revision");
    }
    Some(value)
}

fn active_profile_changed(stored: &BackendSettings, next: &BackendSettings) -> bool {
    let find = |settings: &BackendSettings| {
        settings
            .relay_profiles
            .iter()
            .find(|profile| profile.id == stored.active_relay_id)
            .cloned()
    };
    match (find(stored), find(next)) {
        (Some(before), Some(after)) => normalized_for_compare(&before) != normalized_for_compare(&after),
        _ => true,
    }
}

/// 提交供应商列表：当前供应商没变时只保存；切换或改了当前供应商时走宿主的切换（重写 live 文件）。
pub fn commit_providers(
    store: &SettingsStore,
    host: &ProviderPageHost,
    request: &ProviderCommitRequest,
) -> anyhow::Result<ProviderCommitOutcome> {
    let stored = store.load().context("读取供应商设置失败")?;
    let next = settings_from_commit(&stored, request)?;
    let switching = next.active_relay_id != stored.active_relay_id;
    let needs_apply = switching || active_profile_changed(&stored, &next);
    if needs_apply && !next.relay_profiles_enabled {
        if switching {
            anyhow::bail!("供应商配置总开关已关闭，不能切换；请先在 Codex++ 里打开总开关。");
        }
    } else if needs_apply {
        let result = (host.switch)(next, stored.active_relay_id.clone(), request.conflict_resolution);
        crate::settings::notify_external_settings_change();
        if !result.ok && result.live_conflict.is_none() {
            anyhow::bail!("{}", result.message);
        }
        return Ok(ProviderCommitOutcome {
            switched: result.ok && switching,
            message: result.message,
            live_conflict: result.live_conflict,
        });
    }
    let message = (host.save)(next, Some(stored.settings_revision.clone()))
        .map_err(|error| anyhow::anyhow!(error))?;
    crate::settings::notify_external_settings_change();
    Ok(ProviderCommitOutcome { switched: false, message, live_conflict: None })
}

/// 测试连接 / 拉模型用的草稿：还原 Key 后归一化（补齐地址、模型、当前 Key），失败时用原样草稿。
pub fn draft_profile_for_probe(stored: &BackendSettings, payload: &Value) -> anyhow::Result<RelayProfile> {
    let mut raw = payload.get("profile").cloned().ok_or_else(|| anyhow::anyhow!("缺少供应商草稿"))?;
    let id = raw.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    let source_id = payload
        .get("keySourceId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(id.as_str())
        .to_string();
    let source = stored.relay_profiles.iter().find(|profile| profile.id == source_id);
    restore_masked_api_keys(&mut raw, source);
    let draft: RelayProfile = serde_json::from_value(raw).context("供应商草稿的格式不正确")?;
    let mut normalized = draft.clone();
    Ok(match crate::relay_config::normalize_relay_profile_for_storage(&mut normalized) {
        Ok(()) => normalized,
        Err(_) => draft,
    })
}

fn profile_display_name(profile: &RelayProfile) -> &str {
    if profile.name.trim().is_empty() { "未命名供应商" } else { profile.name.trim() }
}

fn conflict_value(conflict: &Option<ProviderLiveConflict>) -> Value {
    conflict
        .as_ref()
        .map(|conflict| json!({ "profileId": conflict.profile_id, "profileName": conflict.profile_name }))
        .unwrap_or(Value::Null)
}

// ---- bridge 路由 ----

pub fn route_list() -> anyhow::Result<Value> {
    let settings = SettingsStore::default().load().context("读取供应商设置失败")?;
    let mut value = provider_list_value(&settings)?;
    insert_gateway_mode(&mut value, &settings);
    Ok(value)
}

fn insert_gateway_mode(value: &mut Value, settings: &BackendSettings) {
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "gatewayMode".to_string(),
            Value::Bool(codex_gateway_mode(settings, codex_binding_enabled())),
        );
    }
}

/// `/providers/gateway`：`{command, args}` → 管理器的网关命令，白名单 + 脱敏。
pub fn route_gateway(payload: &Value) -> anyhow::Result<Value> {
    let command = payload.get("command").and_then(Value::as_str).unwrap_or_default().to_string();
    let args = payload.get("args").cloned().unwrap_or_else(|| json!({}));
    gateway_command_allowed(&command, &args).map_err(|message| anyhow::anyhow!(message))?;
    let host = registered_host()?;
    Ok(redact_gateway_result((host.gateway)(&command, args)))
}

pub fn route_commit(payload: &Value) -> anyhow::Result<Value> {
    let host = registered_host()?;
    let request: ProviderCommitRequest =
        serde_json::from_value(payload.clone()).context("提交的供应商列表格式不正确")?;
    let store = SettingsStore::default();
    let outcome = commit_providers(&store, &host, &request)?;
    let settings = store.load().context("保存后读取供应商设置失败")?;
    let mut value = provider_list_value(&settings)?;
    insert_gateway_mode(&mut value, &settings);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "status".to_string(),
            json!(if outcome.live_conflict.is_some() { "conflict" } else { "ok" }),
        );
        object.insert("message".to_string(), json!(outcome.message));
        object.insert("switched".to_string(), json!(outcome.switched));
        object.insert("liveConflict".to_string(), conflict_value(&outcome.live_conflict));
    }
    Ok(value)
}

pub async fn route_test(payload: &Value) -> anyhow::Result<Value> {
    let stored = SettingsStore::default().load().context("读取供应商设置失败")?;
    let profile = draft_profile_for_probe(&stored, payload)?;
    let name = profile_display_name(&profile).to_string();
    let model = crate::relay_config::relay_profile_test_model(&profile);
    match crate::relay_config::test_relay_profile(&profile, &model).await {
        Ok(result) => {
            let status = if !result.protocol_valid {
                "failed"
            } else if result.completion_status != "completed" {
                "warning"
            } else {
                "ok"
            };
            let preview = result.response_preview.trim();
            let mut detail =
                if preview.is_empty() { "响应内容为空".to_string() } else { format!("响应：{preview}") };
            if let Some(hint) =
                crate::relay_config::relay_test_auth_failure_hint(&profile, result.http_status)
            {
                detail = format!("{detail} {hint}");
            }
            Ok(json!({
                "status": status,
                "message": format!("已向「{name}」用模型「{model}」发送 hi，HTTP {}。{detail}", result.http_status),
                "httpStatus": result.http_status,
                "endpoint": result.endpoint,
            }))
        }
        Err(error) => Ok(json!({ "status": "failed", "message": format!("测试「{name}」失败：{error}") })),
    }
}

pub async fn route_models(payload: &Value) -> anyhow::Result<Value> {
    let stored = SettingsStore::default().load().context("读取供应商设置失败")?;
    let profile = draft_profile_for_probe(&stored, payload)?;
    let name = profile_display_name(&profile).to_string();
    if profile.relay_mode == RelayMode::Aggregate {
        anyhow::bail!("聚合供应商没有自己的模型列表，请在成员供应商上获取。");
    }
    match crate::model_catalog::fetch_relay_profile_model_ids(&profile).await {
        Ok((models, endpoint)) => Ok(json!({
            "status": "ok",
            "message": format!("已从「{name}」获取 {} 个模型。", models.len()),
            "models": models,
            "endpoint": endpoint,
        })),
        Err(error) => Ok(json!({
            "status": "failed",
            "message": format!("从「{name}」获取模型失败：{error}"),
            "models": Vec::<String>::new(),
        })),
    }
}
