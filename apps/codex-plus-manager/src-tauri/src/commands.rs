use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use codex_plus_core::install::SILENT_BINARY;
use codex_plus_core::models::{DeleteResult, SessionRef};
use codex_plus_core::script_market::{self, MarketScript, ScriptMarketManifest};
use codex_plus_core::settings::{BackendSettings, RelayProfile, SettingsStore};
use codex_plus_core::status::{LaunchStatus, StatusStore};
use codex_plus_core::user_scripts::UserScriptManager;
use codex_plus_core::zed_remote::{ZedOpenStrategy, ZedRemoteProject};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::install::{self, InstallActionResult, InstallOptions};

#[derive(Debug, Clone, Serialize)]
pub struct CommandResult<T>
where
    T: Serialize,
{
    pub status: String,
    pub message: String,
    #[serde(flatten)]
    pub payload: T,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionPayload {
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathState {
    pub status: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverviewPayload {
    pub codex_app: PathState,
    pub codex_version: Option<String>,
    pub silent_shortcut: PathState,
    pub management_shortcut: PathState,
    pub latest_launch: Option<LaunchStatus>,
    pub current_version: String,
    pub update_status: String,
    pub settings_path: String,
    pub logs_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsPayload {
    pub settings: BackendSettings,
    pub settings_path: String,
    pub user_scripts: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSessionsPayload {
    pub db_path: String,
    pub sessions: Vec<codex_plus_data::LocalSession>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZedRemoteProjectsPayload {
    pub projects: Vec<ZedRemoteProject>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZedRemoteOpenPayload {
    pub url: String,
    pub strategy: ZedOpenStrategy,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteLocalSessionRequest {
    pub session_id: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CcsProvidersPayload {
    pub db_path: String,
    pub providers: Vec<codex_plus_core::ccs_import::CcsProviderImport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayPayload {
    pub authenticated: bool,
    pub auth_source: String,
    pub account_label: Option<String>,
    pub config_path: String,
    pub configured: bool,
    pub requires_openai_auth: bool,
    pub has_bearer_token: bool,
    pub api_key_configured: bool,
    pub api_key_source: String,
    pub backup_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayFilesPayload {
    pub config_path: String,
    pub auth_path: String,
    pub config_contents: String,
    pub auth_contents: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsBackfillPayload {
    pub settings: BackendSettings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextEntriesPayload {
    pub settings: BackendSettings,
    pub entries: codex_plus_core::relay_config::CodexContextEntries,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveContextEntriesPayload {
    pub entries: codex_plus_core::relay_config::CodexContextEntries,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractRelayCommonConfigPayload {
    pub common_config_contents: String,
    pub profile_config_contents: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayProfileTestPayload {
    pub http_status: u16,
    pub endpoint: String,
    pub response_preview: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayProfileModelsPayload {
    pub models: Vec<String>,
    pub endpoint: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRelayFileRequest {
    pub kind: String,
    pub contents: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillRelayProfileRequest {
    pub settings: BackendSettings,
    pub profile_id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSettingsRequest {
    pub settings: BackendSettings,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextEntryRequest {
    pub settings: BackendSettings,
    pub kind: String,
    pub id: String,
    pub toml_body: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextDeleteRequest {
    pub settings: BackendSettings,
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractRelayCommonConfigRequest {
    pub config_contents: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRequest {
    #[serde(default)]
    pub app_path: String,
    #[serde(default = "default_debug_port")]
    pub debug_port: u16,
    #[serde(default = "default_helper_port")]
    pub helper_port: u16,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRequest {
    #[serde(default = "default_log_lines")]
    pub lines: usize,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmsCodeRequest {
    pub phone: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmsLoginRequest {
    pub phone: String,
    pub code: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmsProviderSettingsRequest {
    pub region: String,
    pub app_id: String,
    pub sign_name: String,
    pub template_id: String,
    pub ttl_minutes: i64,
    pub template_param_mode: String,
    pub dry_run: bool,
    #[serde(default)]
    pub secret_id: String,
    #[serde(default)]
    pub secret_key: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEntitlementUpdateRequest {
    pub plan_id: String,
    pub plan_name: String,
    pub daily_token_limit: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogsPayload {
    pub path: String,
    pub text: String,
    pub lines: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsPayload {
    pub report: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalIdentityExportPayload {
    pub report_path: String,
    pub user_count: usize,
    pub device_count: usize,
    pub entitlement_count: usize,
    pub usage_summary_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySyncRequestPayload {
    pub sync_request_path: String,
    pub report_path: String,
    pub endpoint: String,
    pub authorization: String,
    pub user_count: usize,
    pub device_count: usize,
    pub entitlement_count: usize,
    pub usage_summary_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySyncPostPayload {
    pub sync_request_path: String,
    pub report_path: String,
    pub response_audit_path: String,
    pub endpoint: String,
    pub http_status: u16,
    pub response_preview: String,
    pub user_count: usize,
    pub device_count: usize,
    pub entitlement_count: usize,
    pub usage_summary_count: usize,
    pub backend_session_token_ref: Option<String>,
    pub backend_session_configured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendApplyPayload {
    pub receipt: codex_plus_core::local_backend::LocalBackendSyncReceipt,
    pub state: codex_plus_core::local_backend::LocalBackendState,
    pub backend_session_token_ref: Option<String>,
    pub backend_session_configured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminConsolePayload {
    pub state: codex_plus_core::local_backend::LocalBackendState,
    pub users: codex_plus_core::local_backend::LocalBackendAdminUserList,
    pub teams: codex_plus_core::local_backend::LocalBackendAdminTeamList,
    pub renewals: codex_plus_core::local_backend::LocalBackendBillingRenewalList,
    pub audit_events: Vec<codex_plus_core::local_backend::LocalBackendAuditEvent>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdminConsoleQueryRequest {
    #[serde(default = "default_admin_console_limit")]
    pub limit: usize,
    #[serde(default)]
    pub event_type: String,
    #[serde(default)]
    pub actor_type: String,
    #[serde(default)]
    pub subject_user_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminConsoleUserAccessRequest {
    pub user_id: String,
    pub status: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminConsoleUserEntitlementRequest {
    pub user_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub daily_token_limit: i64,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminConsoleTeamEntitlementRequest {
    pub team_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub daily_token_limit: i64,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminConsoleBillingRenewalRequest {
    pub subject_type: String,
    pub subject_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub daily_token_limit: i64,
    pub amount_cents: i64,
    pub currency: String,
    pub payment_channel: String,
    #[serde(default)]
    pub external_order_id: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct IdentitySyncRequestFile {
    generated_at_ms: i64,
    schema_version: u32,
    endpoint: String,
    method: String,
    headers: BTreeMap<String, String>,
    pii_policy: String,
    body: codex_plus_core::local_backend::IdentitySyncBody,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct IdentitySyncResponseAuditFile {
    generated_at_ms: i64,
    schema_version: u32,
    endpoint: String,
    http_status: u16,
    response_preview: String,
    sync_request_path: String,
    report_path: String,
}

struct IdentitySyncRequestBuild {
    payload: IdentitySyncRequestPayload,
    request: IdentitySyncRequestFile,
    api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentitySyncServiceResponse {
    #[serde(default)]
    active_session: Option<IdentitySyncServiceActiveSession>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentitySyncServiceActiveSession {
    access_token: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WatcherPayload {
    pub enabled: bool,
    pub disabled_flag: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseReadinessItem {
    pub id: String,
    pub label: String,
    pub status: String,
    pub message: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseReadinessPayload {
    pub ready: bool,
    pub failures: usize,
    pub warnings: usize,
    pub checked_at_ms: i64,
    pub items: Vec<ReleaseReadinessItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficialCodexIsolationRepairPayload {
    pub official_home: String,
    pub app_support_paths: Vec<String>,
    pub backup_dir: Option<String>,
    pub scanned_files: Vec<String>,
    pub repaired_files: Vec<String>,
    pub remaining_contaminated_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedProxyRuntimePayload {
    pub running: bool,
    pub pid: Option<u32>,
    pub endpoint: String,
    pub listen_addr: String,
    pub binary_path: String,
    pub pid_path: String,
    pub log_path: String,
    pub health_checked: bool,
    pub health_http_status: Option<u16>,
    pub health_status: String,
    pub upstream_base_url: String,
    pub backend_db_path: String,
    pub upstream_key_configured: bool,
    pub identity_sync_key_configured: bool,
    pub admin_key_configured: bool,
    pub user_read_key_configured: bool,
    pub billing_key_configured: bool,
    pub payment_webhook_key_configured: bool,
    pub payment_webhook_signature_configured: bool,
    pub payment_webhook_alipay_signature_configured: bool,
    pub payment_webhook_wechatpay_signature_configured: bool,
    pub access_key_configured: bool,
    pub audit_key_configured: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedProxyHealthPayload {
    #[serde(default)]
    status: String,
    #[serde(default)]
    listen_addr: String,
    #[serde(default)]
    upstream_base_url: String,
    #[serde(default)]
    backend_db_path: String,
    #[serde(default)]
    upstream_key_configured: bool,
    #[serde(default)]
    identity_sync_key_configured: bool,
    #[serde(default)]
    admin_key_configured: bool,
    #[serde(default)]
    user_read_key_configured: bool,
    #[serde(default)]
    billing_key_configured: bool,
    #[serde(default)]
    payment_webhook_key_configured: bool,
    #[serde(default)]
    payment_webhook_signature_configured: bool,
    #[serde(default)]
    payment_webhook_alipay_signature_configured: bool,
    #[serde(default)]
    payment_webhook_wechatpay_signature_configured: bool,
    #[serde(default)]
    access_key_configured: bool,
    #[serde(default)]
    audit_key_configured: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdsPayload {
    pub version: u64,
    pub ads: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScriptMarketPayload {
    pub market: Value,
    pub user_scripts: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupPayload {
    pub show_update: bool,
    pub app_mode: String,
}

#[tauri::command]
pub fn backend_version() -> CommandResult<VersionPayload> {
    ok(
        "后端版本已读取。",
        VersionPayload {
            version: codex_plus_core::version::VERSION.to_string(),
        },
    )
}

#[tauri::command]
pub fn startup_options() -> CommandResult<StartupPayload> {
    ok(
        "启动参数已读取。",
        StartupPayload {
            show_update: startup_should_show_update(),
            app_mode: startup_app_mode(),
        },
    )
}

fn startup_app_mode() -> String {
    if let Ok(value) = std::env::var("JIYI_CODEX_APP_MODE") {
        match value.trim().to_ascii_lowercase().as_str() {
            "main" | "app" => return "main".to_string(),
            "manager" | "admin" => return "manager".to_string(),
            _ => {}
        }
    }
    for arg in std::env::args() {
        match arg.as_str() {
            "--main" | "--app-mode=main" | "--app-mode=app" => return "main".to_string(),
            "--manager" | "--app-mode=manager" | "--app-mode=admin" => {
                return "manager".to_string();
            }
            _ => {}
        }
    }
    let executable_path = std::env::current_exe().ok();
    let executable_text = executable_path
        .as_ref()
        .map(|path| path.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if executable_text.contains("极义codex 管理工具.app") {
        return "manager".to_string();
    }
    if executable_text.contains("极义codex.app") {
        return "main".to_string();
    }
    let executable = executable_path
        .as_ref()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(executable.as_str(), "jiyicodex" | "jiyicodex.bin") {
        "main".to_string()
    } else {
        "manager".to_string()
    }
}

pub fn startup_should_show_update() -> bool {
    should_show_update(
        std::env::args(),
        std::env::var("CODEX_PLUS_SHOW_UPDATE").ok().as_deref(),
    )
}

fn should_show_update<I, S>(args: I, env_value: Option<&str>) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|arg| arg.as_ref() == "--show-update") || env_value == Some("1")
}

#[tauri::command]
pub async fn load_overview() -> CommandResult<OverviewPayload> {
    let payload = tauri::async_runtime::spawn_blocking(load_overview_payload).await;
    let Ok((codex_app_path, entrypoints, latest_launch)) = payload else {
        return failed(
            "概览后台任务失败。",
            OverviewPayload {
                codex_app: path_state(None),
                codex_version: None,
                silent_shortcut: path_state(None),
                management_shortcut: path_state(None),
                latest_launch: None,
                current_version: codex_plus_core::version::VERSION.to_string(),
                update_status: "not_checked".to_string(),
                settings_path: codex_plus_core::paths::default_settings_path()
                    .to_string_lossy()
                    .to_string(),
                logs_path: codex_plus_core::paths::default_diagnostic_log_path()
                    .to_string_lossy()
                    .to_string(),
            },
        );
    };
    ok(
        "概览已加载。",
        OverviewPayload {
            codex_version: codex_app_path
                .as_deref()
                .and_then(codex_plus_core::app_paths::codex_app_version),
            codex_app: path_state(codex_app_path),
            silent_shortcut: shortcut_state(entrypoints.silent_shortcut),
            management_shortcut: shortcut_state(entrypoints.management_shortcut),
            latest_launch,
            current_version: codex_plus_core::version::VERSION.to_string(),
            update_status: "not_checked".to_string(),
            settings_path: codex_plus_core::paths::default_settings_path()
                .to_string_lossy()
                .to_string(),
            logs_path: codex_plus_core::paths::default_diagnostic_log_path()
                .to_string_lossy()
                .to_string(),
        },
    )
}

#[tauri::command]
pub fn launch_codex_plus(request: LaunchRequest) -> CommandResult<Value> {
    spawn_codex_plus_launch(request, "启动任务已在后台开始，可稍后查看概览状态。")
}

#[tauri::command]
pub fn restart_codex_plus(request: LaunchRequest) -> CommandResult<Value> {
    codex_plus_core::watcher::stop_launcher_processes();
    codex_plus_core::watcher::stop_codex_processes();
    spawn_codex_plus_launch(request, "极义codex 已请求重启，启动任务正在后台运行。")
}

#[tauri::command]
pub fn load_local_auth_state() -> CommandResult<codex_plus_core::local_account::LocalAuthState> {
    match codex_plus_core::local_account::LocalAccountStore::default().load_auth_state() {
        Ok(state) => ok("本地账号状态已读取。", state),
        Err(error) => failed(
            &format!("读取本地账号状态失败：{error}"),
            local_auth_fallback_state(),
        ),
    }
}

#[tauri::command]
pub fn load_local_usage_state() -> CommandResult<codex_plus_core::local_usage::LocalUsageSnapshot> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let policy = codex_plus_core::local_usage::LocalUsagePolicy::from_settings(&settings);
    match codex_plus_core::local_usage::LocalUsageStore::default().snapshot(policy.clone()) {
        Ok(state) => ok("本地用量状态已读取。", state),
        Err(error) => failed(
            &format!("读取本地用量状态失败：{error}"),
            codex_plus_core::local_usage::LocalUsageSnapshot {
                enabled: policy.enabled,
                daily_token_limit: policy.daily_token_limit,
                subject_id: policy.subject_id,
                plan_id: policy.plan_id,
                limit_source: policy.limit_source,
                day: String::new(),
                used_tokens: 0,
                request_count: 0,
                remaining_tokens: None,
                db_path: codex_plus_core::local_usage::default_usage_db_path()
                    .to_string_lossy()
                    .to_string(),
            },
        ),
    }
}

#[tauri::command]
pub async fn request_local_sms_code(
    request: SmsCodeRequest,
) -> CommandResult<codex_plus_core::local_account::SmsCodeIssue> {
    match codex_plus_core::local_account::LocalAccountStore::default()
        .request_sms_code(&request.phone)
        .await
    {
        Ok(issue) => {
            let message = if issue.dry_run {
                "验证码已在本地干跑模式生成。"
            } else {
                "验证码已通过腾讯云短信发送。"
            };
            ok(message, issue)
        }
        Err(error) => failed(
            &format!("发送验证码失败：{error}"),
            codex_plus_core::local_account::SmsCodeIssue {
                phone: String::new(),
                phone_masked: String::new(),
                expires_at_ms: 0,
                dry_run: true,
                dev_code: None,
                request_id: None,
            },
        ),
    }
}

#[tauri::command]
pub fn login_with_local_sms_code(
    request: SmsLoginRequest,
) -> CommandResult<codex_plus_core::local_account::LoginSession> {
    match codex_plus_core::local_account::LocalAccountStore::default()
        .login_with_sms_code(&request.phone, &request.code)
    {
        Ok(session) => ok("本地账号已登录。", session),
        Err(error) => failed(
            &format!("手机号登录失败：{error}"),
            codex_plus_core::local_account::LoginSession {
                user_id: String::new(),
                phone: String::new(),
                phone_masked: String::new(),
                login_at_ms: 0,
                expires_at_ms: 0,
                device_id: String::new(),
                session_ttl_hours: 24 * 30,
                entitlement: codex_plus_core::local_account::LocalEntitlementState {
                    user_id: None,
                    plan_id: "local_trial".to_string(),
                    plan_name: "本地试用".to_string(),
                    daily_token_limit: 0,
                    source: "fallback".to_string(),
                    updated_at_ms: None,
                },
            },
        ),
    }
}

#[tauri::command]
pub fn load_sms_provider_settings()
-> CommandResult<codex_plus_core::local_account::SmsProviderSettingsState> {
    match codex_plus_core::local_account::load_sms_provider_settings_state() {
        Ok(state) => ok("腾讯云短信配置已读取。", state),
        Err(error) => failed(
            &format!("读取腾讯云短信配置失败：{error}"),
            fallback_sms_provider_settings_state(),
        ),
    }
}

#[tauri::command]
pub fn save_sms_provider_settings(
    request: SmsProviderSettingsRequest,
) -> CommandResult<codex_plus_core::local_account::SmsProviderSettingsState> {
    if !request.secret_id.trim().is_empty() {
        if let Err(error) =
            codex_plus_core::secret_store::protect_tencent_sms_secret_id(&request.secret_id)
        {
            return failed(
                &format!("保存腾讯云短信 SecretId 失败：{error}"),
                fallback_sms_provider_settings_state(),
            );
        }
    }
    if !request.secret_key.trim().is_empty() {
        if let Err(error) =
            codex_plus_core::secret_store::protect_tencent_sms_secret_key(&request.secret_key)
        {
            return failed(
                &format!("保存腾讯云短信 SecretKey 失败：{error}"),
                fallback_sms_provider_settings_state(),
            );
        }
    }
    let settings = codex_plus_core::local_account::SmsProviderSettings {
        region: request.region,
        app_id: request.app_id,
        sign_name: request.sign_name,
        template_id: request.template_id,
        ttl_minutes: request.ttl_minutes,
        template_param_mode: request.template_param_mode,
        dry_run: request.dry_run,
    };
    let result = codex_plus_core::local_account::save_sms_provider_settings(&settings)
        .and_then(|_| codex_plus_core::local_account::load_sms_provider_settings_state());
    match result {
        Ok(state) => ok("腾讯云短信配置已保存。", state),
        Err(error) => failed(
            &format!("保存腾讯云短信配置失败：{error}"),
            fallback_sms_provider_settings_state(),
        ),
    }
}

#[tauri::command]
pub fn update_local_entitlement(
    request: LocalEntitlementUpdateRequest,
) -> CommandResult<codex_plus_core::local_account::LocalAuthState> {
    let store = codex_plus_core::local_account::LocalAccountStore::default();
    match store
        .update_active_entitlement(
            &request.plan_id,
            &request.plan_name,
            request.daily_token_limit,
        )
        .and_then(|_| store.load_auth_state())
    {
        Ok(state) => ok("本地套餐已更新。", state),
        Err(error) => failed(
            &format!("更新本地套餐失败：{error}"),
            local_auth_fallback_state(),
        ),
    }
}

#[tauri::command]
pub fn export_local_identity_report() -> CommandResult<LocalIdentityExportPayload> {
    match write_local_identity_report() {
        Ok(payload) => ok("本地账号迁移报告已导出。", payload),
        Err(error) => failed(
            &format!("导出本地账号迁移报告失败：{error}"),
            LocalIdentityExportPayload {
                report_path: String::new(),
                user_count: 0,
                device_count: 0,
                entitlement_count: 0,
                usage_summary_count: 0,
            },
        ),
    }
}

#[tauri::command]
pub fn prepare_identity_sync_request() -> CommandResult<IdentitySyncRequestPayload> {
    match write_identity_sync_request() {
        Ok(payload) => ok("极义服务端同步请求包已生成。", payload),
        Err(error) => failed(
            &format!("生成极义服务端同步请求包失败：{error}"),
            IdentitySyncRequestPayload {
                sync_request_path: String::new(),
                report_path: String::new(),
                endpoint: String::new(),
                authorization: "not_configured".to_string(),
                user_count: 0,
                device_count: 0,
                entitlement_count: 0,
                usage_summary_count: 0,
            },
        ),
    }
}

#[tauri::command]
pub async fn sync_identity_to_service() -> CommandResult<IdentitySyncPostPayload> {
    match post_identity_sync_request().await {
        Ok(payload) => ok("极义账号数据已同步到服务端。", payload),
        Err(error) => failed(
            &format!("同步极义账号数据失败：{error}"),
            IdentitySyncPostPayload {
                sync_request_path: String::new(),
                report_path: String::new(),
                response_audit_path: String::new(),
                endpoint: String::new(),
                http_status: 0,
                response_preview: String::new(),
                user_count: 0,
                device_count: 0,
                entitlement_count: 0,
                usage_summary_count: 0,
                backend_session_token_ref: None,
                backend_session_configured: false,
            },
        ),
    }
}

#[tauri::command]
pub fn load_local_backend_state() -> CommandResult<codex_plus_core::local_backend::LocalBackendState>
{
    match codex_plus_core::local_backend::LocalBackendStore::default().state() {
        Ok(state) => ok("本地账号服务端状态已读取。", state),
        Err(error) => failed(
            &format!("读取本地账号服务端状态失败：{error}"),
            codex_plus_core::local_backend::LocalBackendState {
                db_path: codex_plus_core::local_backend::default_backend_db_path()
                    .to_string_lossy()
                    .to_string(),
                initialized: false,
                batch_count: 0,
                user_count: 0,
                blocked_user_count: 0,
                device_count: 0,
                team_count: 0,
                team_member_count: 0,
                entitlement_count: 0,
                billing_renewal_count: 0,
                billing_payment_event_count: 0,
                usage_summary_count: 0,
                audit_event_count: 0,
                session_count: 0,
                active_session_count: 0,
                revoked_session_count: 0,
                last_synced_at_ms: None,
                last_audit_event_at_ms: None,
                last_billing_renewal_at_ms: None,
                last_billing_payment_event_at_ms: None,
                last_user_access_updated_at_ms: None,
                last_session_issued_at_ms: None,
                last_session_revoked_at_ms: None,
            },
        ),
    }
}

#[tauri::command]
pub fn apply_identity_sync_locally() -> CommandResult<LocalBackendApplyPayload> {
    match apply_identity_sync_to_local_backend() {
        Ok(payload) => ok("极义账号数据已同步到本地后端。", payload),
        Err(error) => failed(
            &format!("同步极义账号数据到本地后端失败：{error}"),
            LocalBackendApplyPayload {
                receipt: codex_plus_core::local_backend::LocalBackendSyncReceipt {
                    backend_db_path: codex_plus_core::local_backend::default_backend_db_path()
                        .to_string_lossy()
                        .to_string(),
                    batch_id: String::new(),
                    received_at_ms: 0,
                    users_upserted: 0,
                    devices_upserted: 0,
                    teams_upserted: 0,
                    team_members_upserted: 0,
                    entitlements_upserted: 0,
                    usage_summaries_upserted: 0,
                    sessions_issued: 0,
                    active_session: None,
                    total_user_count: 0,
                    total_device_count: 0,
                    total_team_count: 0,
                    total_team_member_count: 0,
                    total_entitlement_count: 0,
                    total_usage_summary_count: 0,
                    total_session_count: 0,
                },
                state: codex_plus_core::local_backend::LocalBackendState {
                    db_path: codex_plus_core::local_backend::default_backend_db_path()
                        .to_string_lossy()
                        .to_string(),
                    initialized: false,
                    batch_count: 0,
                    user_count: 0,
                    blocked_user_count: 0,
                    device_count: 0,
                    team_count: 0,
                    team_member_count: 0,
                    entitlement_count: 0,
                    billing_renewal_count: 0,
                    billing_payment_event_count: 0,
                    usage_summary_count: 0,
                    audit_event_count: 0,
                    session_count: 0,
                    active_session_count: 0,
                    revoked_session_count: 0,
                    last_synced_at_ms: None,
                    last_audit_event_at_ms: None,
                    last_billing_renewal_at_ms: None,
                    last_billing_payment_event_at_ms: None,
                    last_user_access_updated_at_ms: None,
                    last_session_issued_at_ms: None,
                    last_session_revoked_at_ms: None,
                },
                backend_session_token_ref: None,
                backend_session_configured: false,
            },
        ),
    }
}

#[tauri::command]
pub fn load_admin_console(request: AdminConsoleQueryRequest) -> CommandResult<AdminConsolePayload> {
    match admin_console_payload(&request) {
        Ok(payload) => ok("总后台数据已读取。", payload),
        Err(error) => failed(
            &format!("读取总后台数据失败：{error}"),
            empty_admin_console_payload(),
        ),
    }
}

#[tauri::command]
pub fn admin_console_set_user_access(
    request: AdminConsoleUserAccessRequest,
) -> CommandResult<AdminConsolePayload> {
    let store = codex_plus_core::local_backend::LocalBackendStore::default();
    let result = store.set_user_access_status_with_actor(
        &request.user_id,
        &request.status,
        text_option(&request.reason),
        "manager_admin_console",
        None,
    );
    match result.and_then(|_| admin_console_payload(&AdminConsoleQueryRequest::default())) {
        Ok(payload) => ok("用户访问状态已更新。", payload),
        Err(error) => failed(
            &format!("更新用户访问状态失败：{error}"),
            empty_admin_console_payload(),
        ),
    }
}

#[tauri::command]
pub fn admin_console_update_user_entitlement(
    request: AdminConsoleUserEntitlementRequest,
) -> CommandResult<AdminConsolePayload> {
    let store = codex_plus_core::local_backend::LocalBackendStore::default();
    let result = store.set_user_entitlement_with_actor(
        &request.user_id,
        &request.plan_id,
        &request.plan_name,
        request.daily_token_limit,
        text_option(&request.reason),
        "manager_admin_console",
        None,
    );
    match result.and_then(|_| admin_console_payload(&AdminConsoleQueryRequest::default())) {
        Ok(payload) => ok("用户套餐和额度已更新。", payload),
        Err(error) => failed(
            &format!("更新用户套餐失败：{error}"),
            empty_admin_console_payload(),
        ),
    }
}

#[tauri::command]
pub fn admin_console_update_team_entitlement(
    request: AdminConsoleTeamEntitlementRequest,
) -> CommandResult<AdminConsolePayload> {
    let store = codex_plus_core::local_backend::LocalBackendStore::default();
    let result = store.set_team_entitlement_with_actor(
        &request.team_id,
        &request.plan_id,
        &request.plan_name,
        request.daily_token_limit,
        text_option(&request.reason),
        "manager_admin_console",
        None,
    );
    match result.and_then(|_| admin_console_payload(&AdminConsoleQueryRequest::default())) {
        Ok(payload) => ok("团队套餐和额度已更新。", payload),
        Err(error) => failed(
            &format!("更新团队套餐失败：{error}"),
            empty_admin_console_payload(),
        ),
    }
}

#[tauri::command]
pub fn admin_console_record_billing_renewal(
    request: AdminConsoleBillingRenewalRequest,
) -> CommandResult<AdminConsolePayload> {
    let store = codex_plus_core::local_backend::LocalBackendStore::default();
    let result = store.record_billing_renewal_with_actor(
        &request.subject_type,
        &request.subject_id,
        &request.plan_id,
        &request.plan_name,
        request.daily_token_limit,
        request.amount_cents,
        &request.currency,
        &request.payment_channel,
        text_option(&request.external_order_id),
        text_option(&request.reason),
        "manager_admin_console",
        None,
    );
    match result.and_then(|_| admin_console_payload(&AdminConsoleQueryRequest::default())) {
        Ok(payload) => ok("续费记录已落账。", payload),
        Err(error) => failed(
            &format!("记录续费失败：{error}"),
            empty_admin_console_payload(),
        ),
    }
}

#[tauri::command]
pub fn admin_console_reconcile_billing() -> CommandResult<AdminConsolePayload> {
    let store = codex_plus_core::local_backend::LocalBackendStore::default();
    let result = store.reconcile_billing_payment_events_with_actor(
        default_admin_console_limit(),
        "manager_admin_console",
        None,
    );
    match result.and_then(|_| admin_console_payload(&AdminConsoleQueryRequest::default())) {
        Ok(payload) => ok("支付事件已重新对账。", payload),
        Err(error) => failed(
            &format!("支付事件对账失败：{error}"),
            empty_admin_console_payload(),
        ),
    }
}

fn admin_console_payload(
    request: &AdminConsoleQueryRequest,
) -> anyhow::Result<AdminConsolePayload> {
    let store = codex_plus_core::local_backend::LocalBackendStore::default();
    let limit = request.limit.clamp(1, 500);
    let audit_query = codex_plus_core::local_backend::LocalBackendAuditEventQuery {
        limit,
        event_type: text_option(&request.event_type).map(str::to_string),
        actor_type: text_option(&request.actor_type).map(str::to_string),
        subject_user_id: text_option(&request.subject_user_id).map(str::to_string),
    };
    Ok(AdminConsolePayload {
        state: store.state()?,
        users: store.admin_user_overviews(limit)?,
        teams: store.admin_team_overviews(limit)?,
        renewals: store.billing_renewals(limit)?,
        audit_events: store.audit_events(audit_query)?,
    })
}

fn empty_admin_console_payload() -> AdminConsolePayload {
    let state = codex_plus_core::local_backend::LocalBackendState {
        db_path: codex_plus_core::local_backend::default_backend_db_path()
            .to_string_lossy()
            .to_string(),
        initialized: false,
        batch_count: 0,
        user_count: 0,
        blocked_user_count: 0,
        device_count: 0,
        team_count: 0,
        team_member_count: 0,
        entitlement_count: 0,
        billing_renewal_count: 0,
        billing_payment_event_count: 0,
        usage_summary_count: 0,
        audit_event_count: 0,
        session_count: 0,
        active_session_count: 0,
        revoked_session_count: 0,
        last_synced_at_ms: None,
        last_audit_event_at_ms: None,
        last_billing_renewal_at_ms: None,
        last_billing_payment_event_at_ms: None,
        last_user_access_updated_at_ms: None,
        last_session_issued_at_ms: None,
        last_session_revoked_at_ms: None,
    };
    AdminConsolePayload {
        state,
        users: codex_plus_core::local_backend::LocalBackendAdminUserList {
            day: String::new(),
            users: Vec::new(),
        },
        teams: codex_plus_core::local_backend::LocalBackendAdminTeamList {
            day: String::new(),
            teams: Vec::new(),
        },
        renewals: codex_plus_core::local_backend::LocalBackendBillingRenewalList {
            renewals: Vec::new(),
        },
        audit_events: Vec::new(),
    }
}

fn text_option(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[tauri::command]
pub fn logout_local_auth() -> CommandResult<codex_plus_core::local_account::LocalAuthState> {
    let store = codex_plus_core::local_account::LocalAccountStore::default();
    let backend_logout_message = match revoke_active_local_backend_session() {
        Ok(true) => "本地账号已退出，服务端 session 已吊销。".to_string(),
        Ok(false) => "本地账号已退出，本地后端 session token 已清理。".to_string(),
        Err(error) => format!("本地账号已退出；本地后端 session 清理失败：{error}"),
    };
    match store.logout().and_then(|_| store.load_auth_state()) {
        Ok(state) => ok(&backend_logout_message, state),
        Err(error) => failed(
            &format!("退出本地账号失败：{error}"),
            local_auth_fallback_state(),
        ),
    }
}

#[tauri::command]
pub fn reset_local_auth_state() -> CommandResult<codex_plus_core::local_account::LocalAuthState> {
    let store = codex_plus_core::local_account::LocalAccountStore::default();
    let backend_clear_message = match revoke_active_local_backend_session() {
        Ok(true) => "本地后端 session 已吊销，".to_string(),
        Ok(false) => "本地后端 session 未命中，".to_string(),
        Err(error) => format!("本地后端 session 清理失败：{error}；"),
    };

    let hard_reset_message = match store.hard_reset() {
        Ok(_) => "本地账号数据库已重置。",
        Err(error) => {
            return failed(
                &format!("重置本地账号失败：{error}"),
                local_auth_fallback_state(),
            );
        }
    };

    let message = format!("{backend_clear_message}{hard_reset_message}");
    match store.load_auth_state() {
        Ok(state) => ok(&message, state),
        Err(error) => failed(
            &format!("重置本地账号失败：{error}"),
            local_auth_fallback_state(),
        ),
    }
}

#[tauri::command]
pub fn launch_embedded_codex(request: LaunchRequest) -> CommandResult<Value> {
    let auth = match codex_plus_core::local_account::LocalAccountStore::default().load_auth_state()
    {
        Ok(auth) => auth,
        Err(error) => {
            return failed(
                &format!("读取本地登录状态失败：{error}"),
                json!({ "appPath": null }),
            );
        }
    };
    if !auth.authenticated {
        return failed("请先完成手机号验证码登录。", json!({ "appPath": null }));
    }

    if let Err(error) = ensure_jiyi_native_api_config() {
        return failed(
            &format!("极义模型配置未就绪：{error}"),
            json!({
                "appPath": null,
                "debugPort": request.debug_port,
                "helperPort": request.helper_port
            }),
        );
    }

    match spawn_embedded_codex(&request) {
        Ok(app_path) => CommandResult {
            status: "accepted".to_string(),
            message: "已进入 Codex 使用界面。".to_string(),
            payload: json!({
                "appPath": app_path.to_string_lossy().to_string(),
                "debugPort": request.debug_port,
                "helperPort": request.helper_port
            }),
        },
        Err(error) => failed(
            &format!("进入 Codex 使用界面失败：{error}"),
            json!({
                "appPath": null,
                "debugPort": request.debug_port,
                "helperPort": request.helper_port
            }),
        ),
    }
}

fn spawn_embedded_codex(request: &LaunchRequest) -> anyhow::Result<PathBuf> {
    let app_path = embedded_codex_app_path()?;
    let mut launch_request = request.clone();
    launch_request.app_path = app_path.to_string_lossy().to_string();
    spawn_silent_launcher(&launch_request).map(|_| app_path)
}

fn ensure_jiyi_native_api_config() -> anyhow::Result<()> {
    let home = codex_plus_core::relay_config::default_codex_home_dir();
    let store = SettingsStore::default();
    let stored_settings = store.load().unwrap_or_default();
    #[cfg(not(test))]
    let stored_settings = {
        let mut stored_settings = stored_settings;
        if codex_plus_core::secret_store::protect_settings_secrets(&mut stored_settings)? {
            store.save(&stored_settings)?;
        }
        stored_settings
    };
    let settings = settings_with_live_ccs_profiles(stored_settings);
    if !settings.relay_profiles_enabled {
        anyhow::bail!("供应商配置总开关已关闭，请先启用极义默认供应商。");
    }

    let target = codex_plus_core::protocol_proxy::effective_relay_target(&settings)?;
    let mut relay = target.relay;
    let base_url = if target.managed_proxy {
        relay.base_url.clone()
    } else {
        first_non_empty(&[
            relay.base_url.as_str(),
            relay.upstream_base_url.as_str(),
            settings.relay_base_url.as_str(),
            codex_plus_core::settings::JIYI_DEFAULT_RELAY_BASE_URL,
            codex_plus_core::settings::JIYI_DEFAULT_RELAY_BASE_URL_FALLBACK,
        ])
    };
    let api_key = target.api_key;
    if api_key.is_empty() {
        anyhow::bail!("缺少阿里百炼 / 极义中转 API Key，已阻止回退到 ChatGPT 登录。");
    }

    relay.base_url = base_url.to_string();
    if target.managed_proxy {
        relay.api_key = codex_plus_core::secret_store::keychain_ref(
            codex_plus_core::secret_store::local_backend_session_token_account(),
        );
    } else {
        relay.api_key = api_key.to_string();
        codex_plus_core::secret_store::materialize_relay_profile_secrets(&mut relay, &api_key)?;
    }
    if target.managed_proxy {
        relay.protocol = codex_plus_core::settings::RelayProtocol::Responses;
    }
    relay.relay_mode = codex_plus_core::settings::RelayMode::PureApi;

    let force_local_proxy = target.managed_proxy || settings.jiyi_local_proxy_enabled;
    let result = if force_local_proxy {
        codex_plus_core::relay_config::apply_local_proxy_config_to_home(
            &home,
            codex_plus_core::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
        )?
    } else {
        codex_plus_core::relay_config::apply_pure_api_config_to_home_with_protocol(
            &home,
            &relay.base_url,
            &relay.api_key,
            relay.protocol,
            codex_plus_core::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
        )?
    };
    let status = codex_plus_core::relay_config::relay_status_from_home(&home);
    if !status.configured {
        anyhow::bail!("纯 API 配置写入后未生效，请检查 config.toml / auth.json。");
    }
    log_relay_apply_result(
        if target.managed_proxy {
            "manager.launch_embedded_codex.ensure_jiyi_native_api.managed_proxy_ok"
        } else if settings.jiyi_local_proxy_enabled {
            "manager.launch_embedded_codex.ensure_jiyi_native_api.local_proxy_ok"
        } else {
            "manager.launch_embedded_codex.ensure_jiyi_native_api.direct_ok"
        },
        &relay,
        &status,
        result.backup_path.as_ref(),
        None,
    );
    Ok(())
}

fn first_non_empty(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn embedded_codex_app_path() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe().context("无法定位当前极义codex可执行文件")?;
    let contents_dir = exe
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow::anyhow!("无法定位当前极义codex.app 的 Contents 目录"))?;
    embedded_codex_app_path_from_contents_dir(contents_dir)
}

fn embedded_codex_app_path_from_contents_dir(contents_dir: &Path) -> anyhow::Result<PathBuf> {
    let embedded = contents_dir.join("Resources").join("JiyiCodexClient.app");
    if !is_codex_app_bundle(&embedded) {
        anyhow::bail!(
            "未找到极义内置 JiyiCodexClient.app，请重新安装完整客户端版 DMG；为避免影响原版 Codex，不会使用 /Applications/Codex.app 兜底。"
        );
    }
    ensure_runtime_codex_app(&embedded)
}

fn is_codex_app_bundle(app_path: &Path) -> bool {
    app_path
        .join("Contents")
        .join("MacOS")
        .join("Codex")
        .is_file()
}

fn ensure_runtime_codex_app(source: &Path) -> anyhow::Result<PathBuf> {
    let runtime_root = runtime_codex_client_root();
    let runtime_app = runtime_root.join("JiyiCodexClient.app");
    if runtime_codex_app_is_current(source, &runtime_app) {
        normalize_runtime_codex_client_identity(&runtime_app)?;
        return Ok(runtime_app);
    }

    if runtime_app.exists() {
        fs::remove_dir_all(&runtime_app)?;
    }
    fs::create_dir_all(&runtime_root)?;
    let status = std::process::Command::new("/bin/cp")
        .arg("-R")
        .arg(source)
        .arg(&runtime_root)
        .status()?;
    if !status.success() {
        anyhow::bail!("复制内置 JiyiCodexClient.app 到运行目录失败");
    }
    let copied_app = runtime_root.join(
        source
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("JiyiCodexClient.app")),
    );
    if copied_app != runtime_app && copied_app.exists() {
        fs::rename(&copied_app, &runtime_app)?;
    }
    if !is_codex_app_bundle(&runtime_app) {
        anyhow::bail!("运行目录中的 JiyiCodexClient.app 不完整");
    }
    normalize_runtime_codex_client_identity(&runtime_app)?;
    Ok(runtime_app)
}

fn normalize_runtime_codex_client_identity(runtime_app: &Path) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let plist = runtime_app.join("Contents").join("Info.plist");
        if !plist.is_file() {
            return Ok(());
        }
        let desired_values = [
            ("CFBundleIdentifier", "com.jiyi.codex.client"),
            ("CFBundleName", "JiyiCodexClient"),
            ("CFBundleDisplayName", "极义codex"),
            ("CFBundleSignature", "JIYI"),
        ];
        let mut changed = false;
        for (key, value) in desired_values {
            if plist_value(&plist, key).as_deref() == Some(value) {
                continue;
            }
            set_plist_value(&plist, key, value)?;
            changed = true;
        }
        for key in ["CFBundleURLTypes", "SUPublicEDKey", "SUFeedURL"] {
            if delete_plist_key_if_present(&plist, key)? {
                changed = true;
            }
        }
        if changed {
            let _ = std::process::Command::new("xattr")
                .arg("-cr")
                .arg(runtime_app)
                .status();
            let status = std::process::Command::new("codesign")
                .args(["--force", "--deep", "--sign", "-"])
                .arg(runtime_app)
                .status()?;
            if !status.success() {
                anyhow::bail!("重签内置 Codex 客户端失败");
            }
        }
        if plist_value(&plist, "CFBundleIdentifier").as_deref() != Some("com.jiyi.codex.client") {
            anyhow::bail!("内置 Codex 客户端 bundle id 未隔离为 com.jiyi.codex.client");
        }
        if plist_key_exists(&plist, "CFBundleURLTypes") {
            anyhow::bail!("内置 Codex 客户端仍声明原版 codex URL Scheme");
        }
    }
    Ok(())
}

fn runtime_codex_client_root() -> PathBuf {
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("极义codex.noindex")
                .join("embedded-client");
        }
    }
    codex_plus_core::paths::default_app_state_dir().join("embedded-client")
}

fn runtime_codex_app_is_current(source: &Path, runtime_app: &Path) -> bool {
    if !is_codex_app_bundle(runtime_app) {
        return false;
    }
    let source_asar = source.join("Contents").join("Resources").join("app.asar");
    let runtime_asar = runtime_app
        .join("Contents")
        .join("Resources")
        .join("app.asar");
    match (fs::metadata(source_asar), fs::metadata(runtime_asar)) {
        (Ok(source_meta), Ok(runtime_meta)) => source_meta.len() == runtime_meta.len(),
        _ => false,
    }
}

fn spawn_codex_plus_launch(request: LaunchRequest, accepted_message: &str) -> CommandResult<Value> {
    let debug_port = request.debug_port;
    let helper_port = request.helper_port;
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "manager.launch_requested",
        json!({
            "debug_port": debug_port,
            "helper_port": helper_port,
            "app_path": request.app_path.trim()
        }),
    );
    match spawn_silent_launcher(&request) {
        Ok(()) => CommandResult {
            status: "accepted".to_string(),
            message: accepted_message.to_string(),
            payload: json!({
                "debugPort": debug_port,
                "helperPort": helper_port
            }),
        },
        Err(error) => failed(
            &format!("启动静默入口失败：{error}"),
            json!({
                "debugPort": debug_port,
                "helperPort": helper_port
            }),
        ),
    }
}

fn spawn_silent_launcher(request: &LaunchRequest) -> anyhow::Result<()> {
    let launcher = codex_plus_core::install::companion_binary_path(SILENT_BINARY);
    let mut command = std::process::Command::new(&launcher);
    command.env(
        "CODEX_HOME",
        codex_plus_core::relay_config::default_codex_home_dir(),
    );
    for key in codex_plus_core::launcher::jiyi_sensitive_environment_keys() {
        command.env_remove(key);
    }
    if !request.app_path.trim().is_empty() {
        command.arg("--app-path").arg(request.app_path.trim());
    }
    command
        .arg("--debug-port")
        .arg(request.debug_port.to_string())
        .arg("--helper-port")
        .arg(request.helper_port.to_string());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("无法启动 {}：{error}", launcher.to_string_lossy()))
}

#[tauri::command]
pub fn load_settings() -> CommandResult<SettingsPayload> {
    settings_payload("设置已加载。", "设置读取失败")
}

#[tauri::command]
pub fn get_config_coordination_status()
-> CommandResult<codex_plus_core::config_coordinator::CoordinationStatus> {
    let settings =
        settings_with_live_ccs_profiles(SettingsStore::default().load().unwrap_or_default());
    ok(
        "已读取配置协调状态。",
        codex_plus_core::config_coordinator::coordination_status(&settings),
    )
}

#[tauri::command]
pub fn save_settings(settings: BackendSettings) -> CommandResult<SettingsPayload> {
    let mut settings = normalize_settings_before_save(settings);
    #[cfg(not(test))]
    if let Err(error) = codex_plus_core::secret_store::protect_settings_secrets(&mut settings) {
        return failed(
            &format!("保存设置失败：API Key 写入 macOS 钥匙串失败：{error}"),
            SettingsPayload {
                settings,
                settings_path: codex_plus_core::paths::default_settings_path()
                    .to_string_lossy()
                    .to_string(),
                user_scripts: user_script_inventory(),
            },
        );
    }
    if settings.ccs_link_enabled {
        if let Err(error) = codex_plus_core::ccs_import::write_linked_profiles_to_default_db(
            &settings.relay_profiles,
        ) {
            let payload = SettingsPayload {
                settings,
                settings_path: codex_plus_core::paths::default_settings_path()
                    .to_string_lossy()
                    .to_string(),
                user_scripts: user_script_inventory(),
            };
            return failed(&format!("写回 cc-switch 供应商配置失败：{error}"), payload);
        }
        let active = settings.active_relay_profile();
        if !active.linked_ccs_provider_id.trim().is_empty() {
            if let Err(error) =
                codex_plus_core::ccs_import::set_current_codex_provider_in_default_db(
                    &active.linked_ccs_provider_id,
                )
            {
                let payload = SettingsPayload {
                    settings,
                    settings_path: codex_plus_core::paths::default_settings_path()
                        .to_string_lossy()
                        .to_string(),
                    user_scripts: user_script_inventory(),
                };
                return failed(&format!("同步 cc-switch 当前供应商失败：{error}"), payload);
            }
        }
    }
    remove_linked_ccs_profiles_for_local_storage(&mut settings);
    match SettingsStore::default().save(&settings) {
        Ok(()) => {
            let wrapper_message = refresh_cli_wrapper_after_settings_save(&settings);
            settings_payload(
                &format!("设置已保存。{wrapper_message}"),
                "设置保存后重新读取失败",
            )
        }
        Err(error) => failed(
            &format!("保存设置失败：{error}"),
            SettingsPayload {
                settings,
                settings_path: codex_plus_core::paths::default_settings_path()
                    .to_string_lossy()
                    .to_string(),
                user_scripts: user_script_inventory(),
            },
        ),
    }
}

#[tauri::command]
pub fn load_ccs_providers() -> CommandResult<CcsProvidersPayload> {
    let db_path = codex_plus_core::ccs_import::default_ccs_db_path();
    match codex_plus_core::ccs_import::list_codex_providers_from_db(&db_path) {
        Ok(providers) => ok(
            &format!("已读取外部 Codex 供应商配置：{} 个。", providers.len()),
            CcsProvidersPayload {
                db_path: db_path.to_string_lossy().to_string(),
                providers,
            },
        ),
        Err(error) => failed(
            &format!("读取外部供应商配置失败：{error}"),
            CcsProvidersPayload {
                db_path: db_path.to_string_lossy().to_string(),
                providers: Vec::new(),
            },
        ),
    }
}

#[tauri::command]
pub fn import_ccs_providers() -> CommandResult<SettingsPayload> {
    let store = SettingsStore::default();
    let mut settings = store.load().unwrap_or_default();
    let synced = match codex_plus_core::ccs_import::list_codex_providers_from_default_db() {
        Ok(providers) => providers.len(),
        Err(error) => {
            let payload = settings_payload_value()
                .map(|payload| payload)
                .unwrap_or_else(|(_, payload)| payload);
            return failed(&format!("读取外部供应商配置失败：{error}"), payload);
        }
    };
    settings.ccs_link_enabled = true;
    remove_linked_ccs_profiles_for_local_storage(&mut settings);

    if synced == 0 {
        return settings_payload("没有可联动的 cc-switch Codex 供应商配置。", "设置读取失败");
    }

    match store.save(&settings) {
        Ok(()) => settings_payload(
            &format!("已开启 cc-switch 联动：{synced} 个供应商将直接从 cc-switch 读取。"),
            "联动供应商配置后重新读取设置失败",
        ),
        Err(error) => failed(
            &format!("保存外部供应商配置失败：{error}"),
            settings_payload_value()
                .map(|payload| payload)
                .unwrap_or_else(|(_, payload)| payload),
        ),
    }
}

#[tauri::command]
pub fn list_local_sessions() -> CommandResult<LocalSessionsPayload> {
    let db_path = codex_plus_core::relay_config::default_codex_home_dir().join("state_5.sqlite");
    let adapter = local_session_adapter(&db_path);
    match adapter.list_local_sessions() {
        Ok(sessions) => ok(
            &format!("已读取 {} 个本地会话。", sessions.len()),
            LocalSessionsPayload {
                db_path: db_path.to_string_lossy().to_string(),
                sessions,
            },
        ),
        Err(error) => failed(
            &format!("读取本地会话失败：{error}"),
            LocalSessionsPayload {
                db_path: db_path.to_string_lossy().to_string(),
                sessions: Vec::new(),
            },
        ),
    }
}

#[tauri::command]
pub fn list_zed_remote_projects() -> CommandResult<ZedRemoteProjectsPayload> {
    let result = codex_plus_core::zed_remote::list_zed_remote_projects_response(&json!({}));
    if result.get("status").and_then(Value::as_str) == Some("ok") {
        let projects = serde_json::from_value::<Vec<ZedRemoteProject>>(
            result
                .get("projects")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .unwrap_or_default();
        return ok(
            &format!("已读取 {} 个 Zed 远程项目。", projects.len()),
            ZedRemoteProjectsPayload { projects },
        );
    }
    failed(
        result
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("读取 Zed 远程项目失败。"),
        ZedRemoteProjectsPayload {
            projects: Vec::new(),
        },
    )
}

#[tauri::command]
pub fn open_zed_remote(payload: Value) -> CommandResult<ZedRemoteOpenPayload> {
    let result = codex_plus_core::zed_remote::open_zed_remote(&payload);
    let strategy = result
        .get("strategy")
        .cloned()
        .and_then(|value| serde_json::from_value::<ZedOpenStrategy>(value).ok())
        .unwrap_or_default();
    let url = result
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if result.get("status").and_then(Value::as_str) == Some("ok") {
        return ok(
            "已在 Zed Remote 打开项目。",
            ZedRemoteOpenPayload { url, strategy },
        );
    }
    failed(
        result
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("无法在 Zed Remote 打开项目。"),
        ZedRemoteOpenPayload { url, strategy },
    )
}

#[tauri::command]
pub fn forget_zed_remote_project(id: String) -> CommandResult<ZedRemoteProjectsPayload> {
    let result =
        codex_plus_core::zed_remote::forget_zed_remote_project_response(&json!({ "id": id }));
    if result.get("status").and_then(Value::as_str) != Some("ok") {
        return failed(
            result
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("移除 Zed 远程项目失败。"),
            ZedRemoteProjectsPayload {
                projects: Vec::new(),
            },
        );
    }
    list_zed_remote_projects()
}

#[tauri::command]
pub fn delete_local_session(request: DeleteLocalSessionRequest) -> CommandResult<DeleteResult> {
    let session_id = request.session_id.trim();
    if session_id.is_empty() {
        return failed(
            "会话 ID 不能为空。",
            DeleteResult {
                status: codex_plus_core::models::DeleteStatus::Failed,
                session_id: String::new(),
                message: "会话 ID 不能为空。".to_string(),
                undo_token: None,
                backup_path: None,
            },
        );
    }
    let db_path = codex_plus_core::relay_config::default_codex_home_dir().join("state_5.sqlite");
    let adapter = local_session_adapter(&db_path);
    let session = SessionRef {
        session_id: session_id.to_string(),
        title: request.title,
    };
    let result = adapter.delete_local(&session);
    let status = if matches!(
        result.status,
        codex_plus_core::models::DeleteStatus::LocalDeleted
    ) {
        "ok"
    } else {
        "failed"
    };
    CommandResult {
        status: status.to_string(),
        message: result.message.clone(),
        payload: result,
    }
}

fn local_session_adapter(db_path: &Path) -> codex_plus_data::SQLiteStorageAdapter {
    codex_plus_data::SQLiteStorageAdapter::new(
        db_path,
        codex_plus_data::BackupStore::new(
            codex_plus_core::paths::default_app_state_dir().join("backups"),
        ),
    )
}

fn build_local_identity_export() -> anyhow::Result<codex_plus_core::local_backend::IdentitySyncBody>
{
    codex_plus_core::local_backend::build_identity_sync_body()
}

fn write_local_identity_report() -> anyhow::Result<LocalIdentityExportPayload> {
    let report = build_local_identity_export()?;
    let report_dir = codex_plus_core::paths::default_app_state_dir().join("reports");
    fs::create_dir_all(&report_dir)?;
    let report_path = report_dir.join(format!(
        "jiyi-local-identity-report-{}.json",
        report.generated_at_ms
    ));
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    Ok(LocalIdentityExportPayload {
        report_path: report_path.to_string_lossy().to_string(),
        user_count: report.account.users.len(),
        device_count: report.account.devices.len(),
        entitlement_count: report.account.entitlements.len(),
        usage_summary_count: report.usage.summaries.len(),
    })
}

fn write_identity_sync_request() -> anyhow::Result<IdentitySyncRequestPayload> {
    Ok(build_identity_sync_request()?.payload)
}

fn apply_identity_sync_to_local_backend() -> anyhow::Result<LocalBackendApplyPayload> {
    let body = build_local_identity_export()?;
    let store = codex_plus_core::local_backend::LocalBackendStore::default();
    let receipt = store.apply_identity_sync(&body)?;
    let backend_session_token_ref = receipt
        .active_session
        .as_ref()
        .map(|session| {
            codex_plus_core::secret_store::protect_local_backend_session_token(
                &session.access_token,
            )
        })
        .transpose()?;
    let state = store.state()?;
    Ok(LocalBackendApplyPayload {
        receipt,
        state,
        backend_session_configured: backend_session_token_ref.is_some(),
        backend_session_token_ref,
    })
}

fn revoke_active_local_backend_session() -> anyhow::Result<bool> {
    let token = codex_plus_core::secret_store::resolve_local_backend_session_token();
    let revoked = if token.trim().is_empty() {
        false
    } else {
        codex_plus_core::local_backend::LocalBackendStore::default()
            .revoke_session_token(&token)?
            .authenticated
    };
    codex_plus_core::secret_store::clear_local_backend_session_token()?;
    Ok(revoked)
}

fn build_identity_sync_request() -> anyhow::Result<IdentitySyncRequestBuild> {
    let store = SettingsStore::default();
    let settings = store.load().unwrap_or_default();
    #[cfg(not(test))]
    let settings = {
        let mut settings = settings;
        if codex_plus_core::secret_store::protect_settings_secrets(&mut settings)? {
            store.save(&settings)?;
        }
        settings
    };
    let endpoint = settings.jiyi_identity_sync_endpoint.trim().to_string();
    if endpoint.is_empty() {
        anyhow::bail!("请先在设置里填写极义服务端同步 Endpoint。");
    }
    if !endpoint.starts_with("https://") && !endpoint.starts_with("http://") {
        anyhow::bail!("极义服务端同步 Endpoint 必须是 http:// 或 https:// URL。");
    }

    let report_payload = write_local_identity_report()?;
    let report = build_local_identity_export()?;
    let api_key =
        codex_plus_core::secret_store::resolve_secret_value(&settings.jiyi_identity_sync_api_key);
    let has_api_key = !api_key.trim().is_empty();
    let authorization = if has_api_key {
        "bearer_configured"
    } else {
        "not_configured"
    };
    let mut headers = BTreeMap::new();
    headers.insert("content-type".to_string(), "application/json".to_string());
    headers.insert(
        "authorization".to_string(),
        if has_api_key {
            "Bearer <redacted>".to_string()
        } else {
            "<not-configured>".to_string()
        },
    );
    headers.insert("x-jiyi-client".to_string(), "jiyi-codex-macos".to_string());

    let request = IdentitySyncRequestFile {
        generated_at_ms: now_ms(),
        schema_version: 1,
        endpoint: endpoint.clone(),
        method: "POST".to_string(),
        headers,
        pii_policy:
            "同步体不包含明文手机号；授权密钥只保存于 macOS 钥匙串，请求包中仅保留脱敏占位。"
                .to_string(),
        body: report,
    };
    let report_dir = codex_plus_core::paths::default_app_state_dir().join("reports");
    fs::create_dir_all(&report_dir)?;
    let sync_request_path = report_dir.join(format!(
        "jiyi-identity-sync-request-{}.json",
        request.generated_at_ms
    ));
    fs::write(&sync_request_path, serde_json::to_vec_pretty(&request)?)?;

    Ok(IdentitySyncRequestBuild {
        payload: IdentitySyncRequestPayload {
            sync_request_path: sync_request_path.to_string_lossy().to_string(),
            report_path: report_payload.report_path,
            endpoint,
            authorization: authorization.to_string(),
            user_count: report_payload.user_count,
            device_count: report_payload.device_count,
            entitlement_count: report_payload.entitlement_count,
            usage_summary_count: report_payload.usage_summary_count,
        },
        request,
        api_key,
    })
}

async fn post_identity_sync_request() -> anyhow::Result<IdentitySyncPostPayload> {
    let build = build_identity_sync_request()?;
    let api_key = build.api_key.trim().to_string();
    if api_key.is_empty() {
        anyhow::bail!("请先在设置里填写极义服务端同步 API Key。");
    }

    let response = reqwest::Client::new()
        .post(&build.payload.endpoint)
        .bearer_auth(&api_key)
        .header("x-jiyi-client", "jiyi-codex-macos")
        .json(&build.request)
        .send()
        .await
        .with_context(|| "调用极义服务端同步接口失败")?;
    let http_status = response.status().as_u16();
    let response_text = response.text().await.unwrap_or_default();
    let response_preview = safe_response_preview(&response_text, &api_key);
    let report_dir = codex_plus_core::paths::default_app_state_dir().join("reports");
    fs::create_dir_all(&report_dir)?;
    let response_audit = IdentitySyncResponseAuditFile {
        generated_at_ms: now_ms(),
        schema_version: 1,
        endpoint: build.payload.endpoint.clone(),
        http_status,
        response_preview: response_preview.clone(),
        sync_request_path: build.payload.sync_request_path.clone(),
        report_path: build.payload.report_path.clone(),
    };
    let response_audit_path = report_dir.join(format!(
        "jiyi-identity-sync-response-{}.json",
        response_audit.generated_at_ms
    ));
    fs::write(
        &response_audit_path,
        serde_json::to_vec_pretty(&response_audit)?,
    )?;
    if !(200..300).contains(&http_status) {
        anyhow::bail!("极义服务端同步接口返回 HTTP {http_status}：{response_preview}");
    }
    let backend_session_token_ref = remote_backend_session_token_from_response(&response_text)
        .map(|token| codex_plus_core::secret_store::protect_local_backend_session_token(&token))
        .transpose()?;
    let backend_session_configured = backend_session_token_ref.is_some();

    Ok(IdentitySyncPostPayload {
        sync_request_path: build.payload.sync_request_path,
        report_path: build.payload.report_path,
        response_audit_path: response_audit_path.to_string_lossy().to_string(),
        endpoint: build.payload.endpoint,
        http_status,
        response_preview,
        user_count: build.payload.user_count,
        device_count: build.payload.device_count,
        entitlement_count: build.payload.entitlement_count,
        usage_summary_count: build.payload.usage_summary_count,
        backend_session_token_ref,
        backend_session_configured,
    })
}

fn remote_backend_session_token_from_response(response_text: &str) -> Option<String> {
    serde_json::from_str::<IdentitySyncServiceResponse>(response_text)
        .ok()
        .and_then(|response| response.active_session)
        .map(|session| session.access_token.trim().to_string())
        .filter(|token| !token.is_empty())
}

fn safe_response_preview(value: &str, api_key: &str) -> String {
    let mut preview = truncate_response_preview(value);
    let api_key = api_key.trim();
    if !api_key.is_empty() {
        preview = preview.replace(api_key, "<redacted>");
    }
    preview
}

fn truncate_response_preview(value: &str) -> String {
    const LIMIT: usize = 2000;
    let trimmed = value.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_string();
    }
    let preview = trimmed.chars().take(LIMIT).collect::<String>();
    format!("{preview}...")
}

fn local_auth_fallback_state() -> codex_plus_core::local_account::LocalAuthState {
    codex_plus_core::local_account::LocalAuthState {
        authenticated: false,
        user_id: None,
        phone: None,
        phone_masked: None,
        login_at_ms: None,
        expires_at_ms: None,
        device_id: None,
        session_ttl_hours: 24 * 30,
        session_expired: false,
        db_path: codex_plus_core::local_account::default_auth_db_path()
            .to_string_lossy()
            .to_string(),
        sms_config: codex_plus_core::local_account::SmsConfigState {
            configured: false,
            dry_run: true,
            region: "ap-guangzhou".to_string(),
            secret_id_set: false,
            secret_key_set: false,
            secret_id_source: "missing".to_string(),
            secret_key_source: "missing".to_string(),
            app_id_set: false,
            sign_name_set: false,
            template_id_set: false,
            ttl_minutes: 10,
            template_param_mode: "code_ttl".to_string(),
        },
        entitlement: codex_plus_core::local_account::LocalEntitlementState {
            user_id: None,
            plan_id: "local_trial".to_string(),
            plan_name: "本地试用".to_string(),
            daily_token_limit: 0,
            source: "fallback".to_string(),
            updated_at_ms: None,
        },
    }
}

fn fallback_sms_provider_settings_state() -> codex_plus_core::local_account::SmsProviderSettingsState
{
    codex_plus_core::local_account::SmsProviderSettingsState {
        settings_path: codex_plus_core::local_account::default_sms_provider_settings_path()
            .to_string_lossy()
            .to_string(),
        settings: codex_plus_core::local_account::SmsProviderSettings::default(),
        sms_config: local_auth_fallback_state().sms_config,
        secret_id_ref: codex_plus_core::secret_store::keychain_ref(
            codex_plus_core::secret_store::tencent_sms_secret_id_account(),
        ),
        secret_key_ref: codex_plus_core::secret_store::keychain_ref(
            codex_plus_core::secret_store::tencent_sms_secret_key_account(),
        ),
    }
}

fn normalize_settings_before_save(mut settings: BackendSettings) -> BackendSettings {
    if let Some(path) =
        codex_plus_core::app_paths::normalize_codex_app_path(Path::new(&settings.codex_app_path))
    {
        settings.codex_app_path = path.to_string_lossy().to_string();
    }
    settings.relay_common_config_contents =
        codex_plus_core::relay_config::sanitize_common_config_contents(
            &settings.relay_common_config_contents,
        );
    let (common_without_context, extracted_context) =
        split_relay_context_config_sections(&settings.relay_common_config_contents);
    settings.relay_common_config_contents = common_without_context;
    settings.relay_context_config_contents =
        relay_join_config_sections(&[&settings.relay_context_config_contents, &extracted_context]);
    settings.relay_context_config_contents =
        codex_plus_core::relay_config::sanitize_common_config_contents(
            &settings.relay_context_config_contents,
        );
    for profile in &mut settings.relay_profiles {
        if let Err(error) =
            codex_plus_core::relay_config::normalize_relay_profile_for_storage(profile)
        {
            log_manager_event(
                "manager.normalize_relay_profile_for_storage.failed",
                json!({
                    "profileId": profile.id.clone(),
                    "profileName": profile.name,
                    "error": error.to_string()
                }),
            );
        }
    }
    let common_config = relay_combined_common_config(&settings);
    if !common_config.trim().is_empty() {
        for profile in &mut settings.relay_profiles {
            if !profile.use_common_config || profile.config_contents.trim().is_empty() {
                continue;
            }
            match codex_plus_core::relay_config::strip_common_config_from_config(
                &profile.config_contents,
                &common_config,
            ) {
                Ok(stripped) => {
                    profile.config_contents =
                        strip_common_config_text_fallback(&stripped, &common_config);
                }
                Err(_) => {
                    profile.config_contents =
                        strip_common_config_text_fallback(&profile.config_contents, &common_config);
                }
            }
        }
    }
    settings.provider_sync_saved_providers =
        normalize_provider_sync_provider_list(settings.provider_sync_saved_providers);
    settings.provider_sync_manual_providers =
        normalize_provider_sync_provider_list(settings.provider_sync_manual_providers);
    settings.provider_sync_last_selected_provider = settings
        .provider_sync_last_selected_provider
        .trim()
        .to_string();
    settings
}

fn normalize_provider_sync_provider_list(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            result.push(trimmed.to_string());
        }
    }
    result.sort();
    result
}

fn settings_with_live_ccs_profiles(mut settings: BackendSettings) -> BackendSettings {
    if !settings.ccs_link_enabled {
        return settings;
    }
    remove_linked_ccs_profiles_for_local_storage(&mut settings);
    if let Err(error) = codex_plus_core::ccs_import::sync_linked_profiles_from_default_db(
        &mut settings.relay_profiles,
    ) {
        log_manager_event(
            "manager.settings_with_live_ccs_profiles.failed",
            json!({ "error": error.to_string() }),
        );
    }
    settings
}

fn remove_linked_ccs_profiles_for_local_storage(settings: &mut BackendSettings) {
    settings
        .relay_profiles
        .retain(|profile| profile.linked_ccs_provider_id.trim().is_empty());
    if !settings.ccs_link_enabled
        && !settings
            .relay_profiles
            .iter()
            .any(|profile| profile.id == settings.active_relay_id)
    {
        settings.active_relay_id = settings
            .relay_profiles
            .first()
            .map(|profile| profile.id.clone())
            .unwrap_or_else(codex_plus_core::settings::default_active_relay_id);
    }
}

fn relay_combined_common_config(settings: &BackendSettings) -> String {
    relay_join_config_sections(&[
        &settings.relay_common_config_contents,
        &settings.relay_context_config_contents,
    ])
}

fn relay_join_config_sections(sections: &[&str]) -> String {
    let sections = sections
        .iter()
        .map(|section| section.trim())
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>();
    if sections.is_empty() {
        String::new()
    } else {
        codex_plus_core::relay_config::normalize_config_text(&format!(
            "{}\n",
            sections.join("\n\n")
        ))
    }
}

fn split_relay_context_config_sections(config: &str) -> (String, String) {
    let mut common = Vec::new();
    let mut context = Vec::new();
    let mut in_context_table = false;

    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_context_table = trimmed.starts_with("[mcp_servers.")
                || trimmed.starts_with("[skills.")
                || trimmed.starts_with("[plugins.");
        }
        if in_context_table {
            context.push(line);
        } else {
            common.push(line);
        }
    }

    (
        relay_join_config_sections(&[&common.join("\n")]),
        relay_join_config_sections(&[&context.join("\n")]),
    )
}

fn strip_common_config_text_fallback(config_contents: &str, common_config: &str) -> String {
    let common = common_config_anchors(common_config);
    if common.root_keys.is_empty() && common.table_headers.is_empty() {
        return ensure_text_newline(config_contents.trim_end());
    }

    let mut kept = Vec::new();
    let mut skipping_table = false;

    for line in config_contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let header = trimmed.to_string();
            skipping_table = common.table_headers.contains(&header);
            if skipping_table {
                continue;
            }
        }

        if skipping_table {
            continue;
        }

        if let Some(key) = toml_key_from_line(trimmed) {
            if common.root_keys.contains(key) {
                continue;
            }
        }

        kept.push(line);
    }

    ensure_text_newline(kept.join("\n").trim_end())
}

struct CommonConfigAnchors {
    root_keys: std::collections::HashSet<String>,
    table_headers: std::collections::HashSet<String>,
}

fn common_config_anchors(common_config: &str) -> CommonConfigAnchors {
    let mut root_keys = std::collections::HashSet::new();
    let mut table_headers = std::collections::HashSet::new();
    let mut in_table = false;

    for line in common_config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_table = true;
            table_headers.insert(trimmed.to_string());
            continue;
        }
        if !in_table {
            if let Some(key) = toml_key_from_line(trimmed) {
                root_keys.insert(key.to_string());
            }
        }
    }

    CommonConfigAnchors {
        root_keys,
        table_headers,
    }
}

fn toml_key_from_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let (key, _) = trimmed.split_once('=')?;
    let key = key.trim();
    if key.is_empty() { None } else { Some(key) }
}

fn ensure_text_newline(value: &str) -> String {
    if value.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n", value.trim_end())
    }
}

#[tauri::command]
pub async fn load_provider_sync_targets() -> CommandResult<Value> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let result =
        tauri::async_runtime::spawn_blocking(|| codex_plus_data::load_provider_sync_targets(None))
            .await
            .map_err(|error| anyhow::anyhow!("provider target discovery task failed: {error}"));
    match result {
        Ok(mut targets) => {
            let manual = settings
                .provider_sync_manual_providers
                .iter()
                .chain(settings.provider_sync_saved_providers.iter())
                .filter_map(|value| {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                })
                .collect::<Vec<_>>();
            merge_manual_provider_sync_targets(&mut targets, &manual, &settings);
            ok(
                "Provider 同步目标已加载。",
                serde_json::to_value(targets).unwrap_or_else(|_| json!({})),
            )
        }
        Err(error) => failed(&format!("Provider 同步目标加载失败：{error}"), json!({})),
    }
}

fn merge_manual_provider_sync_targets(
    targets: &mut codex_plus_data::ProviderSyncTargetList,
    manual: &[String],
    settings: &BackendSettings,
) {
    for id in manual {
        if let Some(existing) = targets.targets.iter_mut().find(|target| target.id == *id) {
            if !existing
                .sources
                .contains(&codex_plus_data::ProviderSyncTargetSource::Manual)
            {
                existing
                    .sources
                    .push(codex_plus_data::ProviderSyncTargetSource::Manual);
                existing.sources.sort();
            }
            existing.is_manual = settings.provider_sync_manual_providers.contains(id);
            existing.is_saved = settings.provider_sync_saved_providers.contains(id);
        } else {
            targets
                .targets
                .push(codex_plus_data::ProviderSyncTargetOption {
                    id: id.clone(),
                    sources: vec![codex_plus_data::ProviderSyncTargetSource::Manual],
                    is_current_provider: *id == targets.current_provider,
                    is_manual: settings.provider_sync_manual_providers.contains(id),
                    is_saved: settings.provider_sync_saved_providers.contains(id),
                });
        }
    }
    targets.targets.sort_by(|left, right| {
        right
            .is_current_provider
            .cmp(&left.is_current_provider)
            .then_with(|| left.id.cmp(&right.id))
    });
}

#[tauri::command]
pub async fn sync_providers_now(target_provider: Option<String>) -> CommandResult<Value> {
    let target_provider = target_provider
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let target_for_settings = target_provider.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        codex_plus_data::run_provider_sync_with_target(None, target_provider.as_deref())
    })
    .await
    .map_err(|error| anyhow::anyhow!("provider sync task failed: {error}"));
    match result {
        Ok(sync) => {
            if is_success_sync_status(&sync.status) {
                persist_provider_sync_selection(
                    target_for_settings
                        .as_deref()
                        .unwrap_or(&sync.target_provider),
                );
            }
            ok(
                &format!(
                    "供应商已同步一次：{} 个会话文件，{} 行索引，跳过 {} 个占用文件。",
                    sync.changed_session_files,
                    sync.sqlite_rows_updated,
                    sync.skipped_locked_rollout_files.len()
                ),
                json!({
                    "syncStatus": sync.status,
                    "targetProvider": sync.target_provider,
                    "changedSessionFiles": sync.changed_session_files,
                    "skippedLockedRolloutFiles": sync.skipped_locked_rollout_files,
                    "sqliteRowsUpdated": sync.sqlite_rows_updated,
                    "sqliteProviderRowsUpdated": sync.sqlite_provider_rows_updated,
                    "sqliteUserEventRowsUpdated": sync.sqlite_user_event_rows_updated,
                    "sqliteCwdRowsUpdated": sync.sqlite_cwd_rows_updated,
                    "updatedWorkspaceRoots": sync.updated_workspace_roots,
                    "encryptedContentWarning": sync.encrypted_content_warning,
                    "backupDir": sync.backup_dir,
                    "syncMessage": sync.message,
                }),
            )
        }
        Err(error) => failed(&format!("供应商同步失败：{error}"), json!({})),
    }
}

fn is_success_sync_status(status: &codex_plus_data::ProviderSyncStatus) -> bool {
    matches!(status, codex_plus_data::ProviderSyncStatus::Synced)
}

fn persist_provider_sync_selection(provider: &str) {
    let trimmed = provider.trim();
    if trimmed.is_empty() {
        return;
    }
    let store = SettingsStore::default();
    let mut settings = store.load().unwrap_or_default();
    settings.provider_sync_last_selected_provider = trimmed.to_string();
    if !settings
        .provider_sync_saved_providers
        .iter()
        .any(|item| item == trimmed)
    {
        settings
            .provider_sync_saved_providers
            .push(trimmed.to_string());
    }
    settings.provider_sync_saved_providers =
        normalize_provider_sync_provider_list(settings.provider_sync_saved_providers);
    let _ = store.save(&settings);
}

#[tauri::command]
pub async fn load_ads() -> CommandResult<AdsPayload> {
    match codex_plus_core::ads::fetch_ad_list().await {
        Ok(payload) => ok("推荐内容已加载。", ads_payload(payload)),
        Err(error) => failed(
            &format!("推荐内容加载失败：{error}"),
            AdsPayload {
                version: 1,
                ads: Vec::new(),
            },
        ),
    }
}

#[tauri::command]
pub async fn refresh_script_market() -> CommandResult<ScriptMarketPayload> {
    match script_market::fetch_market_manifest(script_market::DEFAULT_MARKET_INDEX_URL).await {
        Ok(manifest) => ok(
            "脚本市场已刷新。",
            script_market_payload_from_manifest(&manifest, "ok", "脚本市场已刷新。"),
        ),
        Err(error) => failed(
            &format!("脚本市场加载失败：{error}"),
            failed_script_market_payload(&format!("脚本市场加载失败：{error}")),
        ),
    }
}

#[tauri::command]
pub async fn install_market_script(id: String) -> CommandResult<ScriptMarketPayload> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return failed(
            "脚本 id 不能为空。",
            failed_script_market_payload("脚本 id 不能为空。"),
        );
    }
    let manifest =
        match script_market::fetch_market_manifest(script_market::DEFAULT_MARKET_INDEX_URL).await {
            Ok(manifest) => manifest,
            Err(error) => {
                return failed(
                    &format!("脚本市场加载失败：{error}"),
                    failed_script_market_payload(&format!("脚本市场加载失败：{error}")),
                );
            }
        };
    let Some(script) = manifest.scripts.iter().find(|script| script.id == trimmed) else {
        return failed(
            "市场清单中未找到该脚本。",
            script_market_payload_from_manifest(&manifest, "failed", "市场清单中未找到该脚本。"),
        );
    };
    let manager = default_user_script_manager();
    match script_market::install_market_script(&manager, script).await {
        Ok(()) => ok(
            "脚本已安装。",
            script_market_payload_from_manifest(&manifest, "ok", "脚本已安装。"),
        ),
        Err(error) => failed(
            &format!("安装脚本失败：{error}"),
            script_market_payload_from_manifest(
                &manifest,
                "failed",
                &format!("安装脚本失败：{error}"),
            ),
        ),
    }
}

#[tauri::command]
pub fn set_user_script_enabled(key: String, enabled: bool) -> CommandResult<SettingsPayload> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return failed("脚本 key 不能为空。", fallback_settings_payload());
    }
    let manager = default_user_script_manager();
    match manager.set_script_enabled(trimmed, enabled) {
        Ok(_) => settings_payload(
            if enabled {
                "脚本已启用。"
            } else {
                "脚本已禁用。"
            },
            "脚本启停失败",
        ),
        Err(error) => failed(
            &format!("脚本启停失败：{error}"),
            fallback_settings_payload(),
        ),
    }
}

#[tauri::command]
pub fn delete_user_script(key: String) -> CommandResult<SettingsPayload> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return failed("脚本 key 不能为空。", fallback_settings_payload());
    }
    let manager = default_user_script_manager();
    match manager.delete_user_script(trimmed) {
        Ok(_) => settings_payload("脚本已删除。", "脚本删除失败"),
        Err(error) => failed(
            &format!("脚本删除失败：{error}"),
            fallback_settings_payload(),
        ),
    }
}

#[tauri::command]
pub fn open_external_url(url: String) -> CommandResult<Value> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return failed("只允许打开 http 或 https 链接。", json!({}));
    }
    match open_url(trimmed) {
        Ok(()) => ok("已在系统浏览器打开链接。", json!({ "url": trimmed })),
        Err(error) => failed(&format!("打开链接失败：{error}"), json!({ "url": trimmed })),
    }
}

#[tauri::command]
pub async fn install_entrypoints() -> InstallActionResult {
    tauri::async_runtime::spawn_blocking(install::install_entrypoints)
        .await
        .unwrap_or_else(|error| install_background_failure("安装入口", error))
}

#[tauri::command]
pub async fn uninstall_entrypoints(options: InstallOptions) -> InstallActionResult {
    tauri::async_runtime::spawn_blocking(move || install::uninstall_entrypoints(options))
        .await
        .unwrap_or_else(|error| install_background_failure("卸载入口", error))
}

#[tauri::command]
pub async fn repair_shortcuts() -> InstallActionResult {
    tauri::async_runtime::spawn_blocking(install::repair_shortcuts)
        .await
        .unwrap_or_else(|error| install_background_failure("修复快捷方式", error))
}

#[tauri::command]
pub fn repair_backend() -> CommandResult<SettingsPayload> {
    let settings =
        settings_with_live_ccs_profiles(SettingsStore::default().load().unwrap_or_default());
    let message = match codex_plus_core::cli_wrapper::ensure_cli_wrapper(&settings) {
        Ok(Some(install)) => format!(
            "后端已修复，命令包装器已指向 {}。",
            install.real_codex.to_string_lossy()
        ),
        Ok(None) => "后端已修复，命令包装器当前未启用。".to_string(),
        Err(error) => format!("后端修复部分失败：{error}"),
    };
    settings_payload(&message, "修复后重新读取设置失败")
}

#[tauri::command]
pub fn repair_official_codex_isolation() -> CommandResult<OfficialCodexIsolationRepairPayload> {
    match repair_official_codex_isolation_payload() {
        Ok(payload) => {
            let repaired = payload.repaired_files.len();
            let remaining = payload.remaining_contaminated_files.len();
            let status = if remaining > 0 { "warning" } else { "ok" };
            let message = if repaired == 0 && remaining == 0 {
                "原版 Codex 未检测到极义写入痕迹。".to_string()
            } else if remaining == 0 {
                format!("已修复 {repaired} 个原版 Codex 隔离项。")
            } else {
                format!(
                    "已修复 {repaired} 个隔离项，仍有 {remaining} 个文件需要关闭原版 Codex 后再处理。"
                )
            };
            CommandResult {
                status: status.to_string(),
                message,
                payload,
            }
        }
        Err(error) => failed(
            &format!("修复原版 Codex 隔离失败：{error}"),
            OfficialCodexIsolationRepairPayload {
                official_home: codex_plus_core::paths::default_official_codex_home_dir()
                    .to_string_lossy()
                    .to_string(),
                app_support_paths: official_codex_app_support_paths()
                    .into_iter()
                    .map(|path| path.to_string_lossy().to_string())
                    .collect(),
                backup_dir: None,
                scanned_files: Vec::new(),
                repaired_files: Vec::new(),
                remaining_contaminated_files: Vec::new(),
            },
        ),
    }
}

#[tauri::command]
pub async fn managed_proxy_status() -> CommandResult<ManagedProxyRuntimePayload> {
    match managed_proxy_runtime_payload().await {
        Ok(payload) => {
            let message = if payload.running {
                "极义本地托管代理正在运行。"
            } else {
                "极义本地托管代理未运行。"
            };
            let status = if payload.running { "ok" } else { "not_checked" };
            CommandResult {
                status: status.to_string(),
                message: message.to_string(),
                payload,
            }
        }
        Err(error) => failed(
            &format!("读取极义本地托管代理状态失败：{error}"),
            managed_proxy_runtime_fallback_payload(),
        ),
    }
}

#[tauri::command]
pub async fn start_managed_proxy() -> CommandResult<ManagedProxyRuntimePayload> {
    match start_managed_proxy_runtime().await {
        Ok(payload) => {
            let status = if payload.running
                && payload.upstream_key_configured
                && payload.identity_sync_key_configured
                && payload.admin_key_configured
            {
                "ok"
            } else {
                "warning"
            };
            let message = if !payload.running {
                "极义本地托管代理启动后未通过健康检查，请查看日志。".to_string()
            } else if !payload.upstream_key_configured {
                "极义本地托管代理已启动，但当前供应商未配置上游 API Key。".to_string()
            } else if !payload.identity_sync_key_configured {
                "极义本地托管代理已启动；账号同步 API Key 未配置，仅本机本地同步可用。".to_string()
            } else if !payload.admin_key_configured {
                "极义本地托管代理已启动；管理 API Key 未配置，远端封禁接口不可用。".to_string()
            } else {
                "极义本地托管代理已启动。".to_string()
            };
            CommandResult {
                status: status.to_string(),
                message,
                payload,
            }
        }
        Err(error) => failed(
            &format!("启动极义本地托管代理失败：{error}"),
            managed_proxy_runtime_fallback_payload(),
        ),
    }
}

#[tauri::command]
pub async fn stop_managed_proxy() -> CommandResult<ManagedProxyRuntimePayload> {
    match stop_managed_proxy_runtime().await {
        Ok(payload) => CommandResult {
            status: if payload.running { "warning" } else { "ok" }.to_string(),
            message: if payload.running {
                "已发送停止信号，但极义本地托管代理仍在运行。".to_string()
            } else {
                "极义本地托管代理已停止。".to_string()
            },
            payload,
        },
        Err(error) => failed(
            &format!("停止极义本地托管代理失败：{error}"),
            managed_proxy_runtime_fallback_payload(),
        ),
    }
}

const MANAGED_PROXY_BINARY: &str = "jiyi-managed-proxy";

async fn managed_proxy_runtime_payload() -> anyhow::Result<ManagedProxyRuntimePayload> {
    let settings = settings_with_live_ccs_profiles(SettingsStore::default().load()?);
    let listen_addr = managed_proxy_listen_addr_for_settings(&settings);
    let endpoint = managed_proxy_endpoint_from_listen_addr(&listen_addr);
    managed_proxy_runtime_payload_for_endpoint(&settings, &listen_addr, &endpoint).await
}

async fn managed_proxy_runtime_payload_for_endpoint(
    settings: &BackendSettings,
    listen_addr: &str,
    endpoint: &str,
) -> anyhow::Result<ManagedProxyRuntimePayload> {
    let pid = managed_proxy_live_recorded_pid();
    let (health_checked, health_http_status, health) = managed_proxy_probe_health(endpoint).await;
    let running = pid.is_some()
        || health_http_status == Some(200)
            && health
                .as_ref()
                .is_some_and(|payload| payload.status.eq_ignore_ascii_case("ok"));
    let upstream_base_url = health
        .as_ref()
        .map(|payload| payload.upstream_base_url.trim())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| managed_proxy_upstream_base_url(settings, endpoint));
    let listen_addr = health
        .as_ref()
        .map(|payload| payload.listen_addr.trim())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| listen_addr.to_string());
    let backend_db_path = health
        .as_ref()
        .map(|payload| payload.backend_db_path.trim())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            codex_plus_core::local_backend::default_backend_db_path()
                .to_string_lossy()
                .to_string()
        });
    let endpoint = managed_proxy_endpoint_from_listen_addr(&listen_addr);
    Ok(ManagedProxyRuntimePayload {
        running,
        pid,
        endpoint,
        listen_addr,
        binary_path: managed_proxy_runtime_binary_path()
            .to_string_lossy()
            .to_string(),
        pid_path: managed_proxy_pid_path().to_string_lossy().to_string(),
        log_path: managed_proxy_log_path().to_string_lossy().to_string(),
        health_checked,
        health_http_status,
        health_status: health
            .as_ref()
            .map(|payload| payload.status.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                if health_checked {
                    "unreachable".to_string()
                } else {
                    "not_checked".to_string()
                }
            }),
        upstream_base_url,
        backend_db_path,
        upstream_key_configured: health
            .as_ref()
            .map(|payload| payload.upstream_key_configured)
            .unwrap_or_else(|| {
                let active = settings.active_relay_profile();
                !codex_plus_core::protocol_proxy::resolved_relay_api_key(settings, &active)
                    .trim()
                    .is_empty()
            }),
        identity_sync_key_configured: health
            .as_ref()
            .map(|payload| payload.identity_sync_key_configured)
            .unwrap_or_else(|| {
                !codex_plus_core::secret_store::resolve_secret_value(
                    &settings.jiyi_identity_sync_api_key,
                )
                .trim()
                .is_empty()
            }),
        admin_key_configured: health
            .as_ref()
            .map(|payload| payload.admin_key_configured)
            .unwrap_or_else(|| {
                !codex_plus_core::secret_store::resolve_secret_value(
                    &settings.jiyi_identity_sync_api_key,
                )
                .trim()
                .is_empty()
            }),
        user_read_key_configured: health
            .as_ref()
            .map(|payload| payload.user_read_key_configured)
            .unwrap_or_else(|| {
                !codex_plus_core::secret_store::resolve_secret_value(
                    &settings.jiyi_identity_sync_api_key,
                )
                .trim()
                .is_empty()
            }),
        billing_key_configured: health
            .as_ref()
            .map(|payload| payload.billing_key_configured)
            .unwrap_or_else(|| {
                !codex_plus_core::secret_store::resolve_secret_value(
                    &settings.jiyi_identity_sync_api_key,
                )
                .trim()
                .is_empty()
            }),
        payment_webhook_key_configured: health
            .as_ref()
            .map(|payload| payload.payment_webhook_key_configured)
            .unwrap_or_else(|| {
                !codex_plus_core::secret_store::resolve_secret_value(
                    &settings.jiyi_identity_sync_api_key,
                )
                .trim()
                .is_empty()
            }),
        payment_webhook_signature_configured: health
            .as_ref()
            .map(|payload| payload.payment_webhook_signature_configured)
            .unwrap_or(false),
        payment_webhook_alipay_signature_configured: health
            .as_ref()
            .map(|payload| payload.payment_webhook_alipay_signature_configured)
            .unwrap_or(false),
        payment_webhook_wechatpay_signature_configured: health
            .as_ref()
            .map(|payload| payload.payment_webhook_wechatpay_signature_configured)
            .unwrap_or(false),
        access_key_configured: health
            .as_ref()
            .map(|payload| payload.access_key_configured)
            .unwrap_or_else(|| {
                !codex_plus_core::secret_store::resolve_secret_value(
                    &settings.jiyi_identity_sync_api_key,
                )
                .trim()
                .is_empty()
            }),
        audit_key_configured: health
            .as_ref()
            .map(|payload| payload.audit_key_configured)
            .unwrap_or_else(|| {
                !codex_plus_core::secret_store::resolve_secret_value(
                    &settings.jiyi_identity_sync_api_key,
                )
                .trim()
                .is_empty()
            }),
    })
}

fn managed_proxy_runtime_fallback_payload() -> ManagedProxyRuntimePayload {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let listen_addr = managed_proxy_listen_addr_for_settings(&settings);
    ManagedProxyRuntimePayload {
        running: false,
        pid: None,
        endpoint: managed_proxy_endpoint_from_listen_addr(&listen_addr),
        listen_addr,
        binary_path: managed_proxy_runtime_binary_path()
            .to_string_lossy()
            .to_string(),
        pid_path: managed_proxy_pid_path().to_string_lossy().to_string(),
        log_path: managed_proxy_log_path().to_string_lossy().to_string(),
        health_checked: false,
        health_http_status: None,
        health_status: "not_checked".to_string(),
        upstream_base_url: codex_plus_core::managed_proxy::DEFAULT_MANAGED_PROXY_UPSTREAM_BASE_URL
            .to_string(),
        backend_db_path: codex_plus_core::local_backend::default_backend_db_path()
            .to_string_lossy()
            .to_string(),
        upstream_key_configured: false,
        identity_sync_key_configured: false,
        admin_key_configured: false,
        user_read_key_configured: false,
        billing_key_configured: false,
        payment_webhook_key_configured: false,
        payment_webhook_signature_configured: false,
        payment_webhook_alipay_signature_configured: false,
        payment_webhook_wechatpay_signature_configured: false,
        access_key_configured: false,
        audit_key_configured: false,
    }
}

async fn start_managed_proxy_runtime() -> anyhow::Result<ManagedProxyRuntimePayload> {
    let store = SettingsStore::default();
    let mut settings = settings_with_live_ccs_profiles(store.load().unwrap_or_default());
    let listen_addr = managed_proxy_listen_addr_for_settings(&settings);
    let endpoint = managed_proxy_endpoint_from_listen_addr(&listen_addr);
    settings.jiyi_managed_proxy_enabled = true;
    settings.jiyi_managed_proxy_endpoint = endpoint.clone();
    if codex_plus_core::secret_store::protect_settings_secrets(&mut settings)? {
        store.save(&settings)?;
    } else {
        store.save(&settings)?;
    }

    let current =
        managed_proxy_runtime_payload_for_endpoint(&settings, &listen_addr, &endpoint).await?;
    if current.running {
        return Ok(current);
    }

    let binary = prepare_managed_proxy_runtime_binary()?;
    let upstream_base_url = managed_proxy_upstream_base_url(&settings, &endpoint);
    let active = settings.active_relay_profile();
    let upstream_api_key =
        codex_plus_core::protocol_proxy::resolved_relay_api_key(&settings, &active);
    let identity_sync_api_key =
        codex_plus_core::secret_store::resolve_secret_value(&settings.jiyi_identity_sync_api_key);
    let backend_db_path = codex_plus_core::local_backend::default_backend_db_path();
    let log_path = managed_proxy_log_path();
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("无法打开托管代理日志 {}", log_path.display()))?;
    writeln!(
        log_file,
        "[{}] starting {} listen={} upstream={} backend_db={}",
        now_ms(),
        MANAGED_PROXY_BINARY,
        listen_addr,
        upstream_base_url,
        backend_db_path.display()
    )
    .ok();
    let stderr = log_file.try_clone()?;

    let mut command = std::process::Command::new(&binary);
    for key in codex_plus_core::launcher::jiyi_sensitive_environment_keys() {
        command.env_remove(key);
    }
    command
        .env("JIYI_MANAGED_PROXY_LISTEN", &listen_addr)
        .env("JIYI_MANAGED_PROXY_UPSTREAM_BASE_URL", &upstream_base_url)
        .env("JIYI_MANAGED_PROXY_UPSTREAM_API_KEY", upstream_api_key)
        .env("JIYI_MANAGED_PROXY_SYNC_API_KEY", &identity_sync_api_key)
        .env("JIYI_MANAGED_PROXY_ADMIN_API_KEY", &identity_sync_api_key)
        .env(
            "JIYI_MANAGED_PROXY_USER_READ_API_KEY",
            &identity_sync_api_key,
        )
        .env("JIYI_MANAGED_PROXY_BILLING_API_KEY", &identity_sync_api_key)
        .env(
            "JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY",
            &identity_sync_api_key,
        )
        .env("JIYI_MANAGED_PROXY_ACCESS_API_KEY", &identity_sync_api_key)
        .env("JIYI_MANAGED_PROXY_AUDIT_API_KEY", &identity_sync_api_key)
        .env("JIYI_MANAGED_PROXY_DB_PATH", backend_db_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(stderr));
    let child = command
        .spawn()
        .with_context(|| format!("无法启动极义托管代理 {}", binary.display()))?;
    write_managed_proxy_pid(child.id())?;

    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(150));
        let payload =
            managed_proxy_runtime_payload_for_endpoint(&settings, &listen_addr, &endpoint).await?;
        if payload.health_http_status == Some(200) {
            return Ok(payload);
        }
    }
    managed_proxy_runtime_payload_for_endpoint(&settings, &listen_addr, &endpoint).await
}

async fn stop_managed_proxy_runtime() -> anyhow::Result<ManagedProxyRuntimePayload> {
    let pid_path = managed_proxy_pid_path();
    let Some(pid) = read_managed_proxy_pid() else {
        let _ = fs::remove_file(&pid_path);
        return managed_proxy_runtime_payload().await;
    };
    if !process_is_running(pid) {
        let _ = fs::remove_file(&pid_path);
        return managed_proxy_runtime_payload().await;
    }
    if !process_matches_managed_proxy(pid) {
        anyhow::bail!(
            "PID {} 不是 {}，为避免影响其它应用已拒绝停止。",
            pid,
            MANAGED_PROXY_BINARY
        );
    }
    terminate_process(pid)?;
    for _ in 0..30 {
        if !process_is_running(pid) {
            let _ = fs::remove_file(&pid_path);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    managed_proxy_runtime_payload().await
}

fn managed_proxy_packaged_binary_path() -> PathBuf {
    codex_plus_core::install::companion_binary_path(MANAGED_PROXY_BINARY)
}

fn managed_proxy_runtime_binary_path() -> PathBuf {
    codex_plus_core::paths::default_app_state_dir()
        .join("bin")
        .join(MANAGED_PROXY_BINARY)
}

fn prepare_managed_proxy_runtime_binary() -> anyhow::Result<PathBuf> {
    let source = managed_proxy_packaged_binary_path();
    if !source.is_file() {
        anyhow::bail!(
            "未找到 {}，请重新安装完整客户端版 DMG：{}",
            MANAGED_PROXY_BINARY,
            source.display()
        );
    }
    let runtime = managed_proxy_runtime_binary_path();
    let should_copy = match (fs::metadata(&source), fs::metadata(&runtime)) {
        (Ok(source_meta), Ok(runtime_meta)) => {
            source_meta.len() != runtime_meta.len()
                || source_meta.modified().ok() > runtime_meta.modified().ok()
        }
        (Ok(_), Err(_)) => true,
        _ => true,
    };
    if should_copy {
        if let Some(parent) = runtime.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &runtime).with_context(|| {
            format!(
                "无法准备极义托管代理运行副本 {} -> {}",
                source.display(),
                runtime.display()
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&runtime)?.permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&runtime, permissions)?;
        }
    }
    Ok(runtime)
}

fn managed_proxy_pid_path() -> PathBuf {
    codex_plus_core::paths::default_app_state_dir().join("jiyi-managed-proxy.pid")
}

fn managed_proxy_log_path() -> PathBuf {
    codex_plus_core::paths::default_app_state_dir().join("jiyi-managed-proxy.log")
}

fn read_managed_proxy_pid() -> Option<u32> {
    fs::read_to_string(managed_proxy_pid_path())
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
}

fn write_managed_proxy_pid(pid: u32) -> anyhow::Result<()> {
    let path = managed_proxy_pid_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{pid}\n"))?;
    Ok(())
}

fn managed_proxy_live_recorded_pid() -> Option<u32> {
    let pid = read_managed_proxy_pid()?;
    if process_is_running(pid) && process_matches_managed_proxy(pid) {
        Some(pid)
    } else {
        None
    }
}

fn managed_proxy_listen_addr_for_settings(settings: &BackendSettings) -> String {
    managed_proxy_loopback_listen_addr_from_endpoint(&settings.jiyi_managed_proxy_endpoint)
        .unwrap_or_else(|| {
            format!(
                "127.0.0.1:{}",
                codex_plus_core::managed_proxy::DEFAULT_MANAGED_PROXY_PORT
            )
        })
}

fn managed_proxy_endpoint_from_listen_addr(listen_addr: &str) -> String {
    let listen_addr = listen_addr.trim();
    if listen_addr.starts_with("http://") || listen_addr.starts_with("https://") {
        return listen_addr.trim_end_matches('/').to_string();
    }
    format!("http://{}", listen_addr.trim_end_matches('/'))
}

fn managed_proxy_loopback_listen_addr_from_endpoint(endpoint: &str) -> Option<String> {
    let mut value = endpoint.trim();
    value = value.strip_prefix("http://").unwrap_or(value);
    value = value.strip_prefix("https://").unwrap_or(value);
    value = value.split('/').next().unwrap_or_default().trim();
    if value.is_empty() {
        return None;
    }
    let normalized = value
        .strip_prefix("[::1]")
        .map(|port| format!("127.0.0.1{port}"));
    let value = normalized.as_deref().unwrap_or(value);
    if value == "localhost" {
        return Some(format!(
            "127.0.0.1:{}",
            codex_plus_core::managed_proxy::DEFAULT_MANAGED_PROXY_PORT
        ));
    }
    if let Some(port) = value.strip_prefix("localhost:") {
        return Some(format!("127.0.0.1:{}", port.trim()));
    }
    if value.starts_with("127.0.0.1:") {
        return Some(value.to_string());
    }
    None
}

fn managed_proxy_upstream_base_url(settings: &BackendSettings, local_endpoint: &str) -> String {
    let active = settings.active_relay_profile();
    let local_listen = managed_proxy_loopback_listen_addr_from_endpoint(local_endpoint);
    let local_endpoint = local_listen
        .as_deref()
        .map(managed_proxy_endpoint_from_listen_addr)
        .unwrap_or_else(|| local_endpoint.trim_end_matches('/').to_string());
    let candidates = [
        active.upstream_base_url.as_str(),
        active.base_url.as_str(),
        settings.relay_base_url.as_str(),
        codex_plus_core::managed_proxy::DEFAULT_MANAGED_PROXY_UPSTREAM_BASE_URL,
    ];
    candidates
        .into_iter()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .find(|value| {
            let value_endpoint = managed_proxy_loopback_listen_addr_from_endpoint(value)
                .as_deref()
                .map(managed_proxy_endpoint_from_listen_addr)
                .unwrap_or_else(|| value.trim_end_matches('/').to_string());
            value_endpoint != local_endpoint
        })
        .unwrap_or(codex_plus_core::managed_proxy::DEFAULT_MANAGED_PROXY_UPSTREAM_BASE_URL)
        .trim_end_matches('/')
        .to_string()
}

async fn managed_proxy_probe_health(
    endpoint: &str,
) -> (bool, Option<u16>, Option<ManagedProxyHealthPayload>) {
    let url = format!("{}/jiyi/v1/health", endpoint.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
    {
        Ok(client) => client,
        Err(_) => return (false, None, None),
    };
    let Ok(response) = client.get(url).send().await else {
        return (true, None, None);
    };
    let status = response.status().as_u16();
    let payload = response.json::<ManagedProxyHealthPayload>().await.ok();
    (true, Some(status), payload)
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    std::process::Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn process_is_running(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn process_matches_managed_proxy(pid: u32) -> bool {
    std::process::Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(MANAGED_PROXY_BINARY))
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn process_matches_managed_proxy(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn terminate_process(pid: u32) -> anyhow::Result<()> {
    let status = std::process::Command::new("/bin/kill")
        .arg(pid.to_string())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("kill {} 失败", pid)
    }
}

#[cfg(not(unix))]
fn terminate_process(_pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("当前平台暂不支持停止本地托管代理进程")
}

#[tauri::command]
pub async fn check_update() -> CommandResult<Value> {
    match codex_plus_core::update::check_for_update(codex_plus_core::version::VERSION).await {
        Ok(update) => {
            let status = if update.update_available {
                "ok"
            } else {
                "not_checked"
            };
            CommandResult {
                status: status.to_string(),
                message: if update.update_available {
                    "发现可用更新。".to_string()
                } else {
                    "当前已是最新版本。".to_string()
                },
                payload: json!({
                    "currentVersion": update.current_version,
                    "latestVersion": update.latest_version,
                    "releaseSummary": update.release_summary,
                    "assetName": update.asset_name,
                    "assetUrl": update.asset_url,
                    "updateAvailable": update.update_available,
                    "progress": 0
                }),
            }
        }
        Err(error) => failed(
            &format!("检查更新失败：{error}"),
            json!({
                "currentVersion": codex_plus_core::version::VERSION,
                "latestVersion": Value::Null,
                "releaseSummary": "",
                "assetName": Value::Null,
                "assetUrl": Value::Null,
                "updateAvailable": false,
                "progress": 0
            }),
        ),
    }
}

#[tauri::command]
pub async fn perform_update(
    release: Option<codex_plus_core::update::Release>,
) -> CommandResult<Value> {
    let Some(release) = release else {
        return failed(
            "请先检查更新并选择可下载的 Release asset。",
            json!({
                "currentVersion": codex_plus_core::version::VERSION,
                "progress": 0
            }),
        );
    };
    let download_dir = codex_plus_core::paths::default_app_state_dir().join("updates");
    match codex_plus_core::update::perform_update(&release, &download_dir).await {
        Ok(result) => ok(
            "安装包已下载并启动，请按安装向导完成更新。",
            json!({
                "currentVersion": codex_plus_core::version::VERSION,
                "latestVersion": result.release.version,
                "releaseSummary": result.release.body,
                "installedPath": result.installer_path.to_string_lossy(),
                "launched": result.launched,
                "progress": 100
            }),
        ),
        Err(error) => failed(
            &format!("安装更新失败：{error}"),
            json!({
                "currentVersion": codex_plus_core::version::VERSION,
                "latestVersion": release.version,
                "releaseSummary": release.body,
                "progress": 0
            }),
        ),
    }
}

#[tauri::command]
pub fn load_watcher_state() -> CommandResult<WatcherPayload> {
    ok("watcher 状态已加载。", watcher_payload())
}

#[tauri::command]
pub fn install_watcher() -> CommandResult<WatcherPayload> {
    let launcher_path =
        codex_plus_core::install::companion_binary_path(codex_plus_core::install::SILENT_BINARY);
    match codex_plus_core::watcher::install_watcher(&launcher_path, default_debug_port()) {
        Ok(()) => ok("watcher 已安装。", watcher_payload()),
        Err(error) => failed(&format!("安装 watcher 失败：{error}"), watcher_payload()),
    }
}

#[tauri::command]
pub fn uninstall_watcher() -> CommandResult<WatcherPayload> {
    match codex_plus_core::watcher::uninstall_watcher() {
        Ok(()) => ok("watcher 已移除。", watcher_payload()),
        Err(error) => failed(&format!("移除 watcher 失败：{error}"), watcher_payload()),
    }
}

#[tauri::command]
pub fn enable_watcher() -> CommandResult<WatcherPayload> {
    match codex_plus_core::watcher::enable_watcher() {
        Ok(()) => ok("watcher 已启用。", watcher_payload()),
        Err(error) => failed(&format!("启用 watcher 失败：{error}"), watcher_payload()),
    }
}

#[tauri::command]
pub fn disable_watcher() -> CommandResult<WatcherPayload> {
    match codex_plus_core::watcher::disable_watcher() {
        Ok(()) => ok("watcher 已禁用。", watcher_payload()),
        Err(error) => failed(&format!("禁用 watcher 失败：{error}"), watcher_payload()),
    }
}

#[tauri::command]
pub fn read_latest_logs(request: LogRequest) -> CommandResult<LogsPayload> {
    let path = codex_plus_core::paths::default_diagnostic_log_path();
    match read_tail(&path, request.lines) {
        Ok(text) => ok(
            "日志已读取。",
            LogsPayload {
                path: path.to_string_lossy().to_string(),
                text,
                lines: request.lines,
            },
        ),
        Err(error) => failed(
            &format!("读取日志失败：{error}"),
            LogsPayload {
                path: path.to_string_lossy().to_string(),
                text: String::new(),
                lines: request.lines,
            },
        ),
    }
}

#[tauri::command]
pub fn copy_diagnostics() -> CommandResult<DiagnosticsPayload> {
    ok(
        "诊断报告已生成。",
        DiagnosticsPayload {
            report: diagnostics_report(),
        },
    )
}

#[tauri::command]
pub fn release_readiness() -> CommandResult<ReleaseReadinessPayload> {
    let payload = release_readiness_payload();
    let status = if payload.failures > 0 {
        "failed"
    } else if payload.warnings > 0 {
        "warning"
    } else {
        "ok"
    };
    CommandResult {
        status: status.to_string(),
        message: if payload.ready {
            "发布前检查通过。".to_string()
        } else {
            format!(
                "发布前检查发现 {} 个阻断项、{} 个风险项。",
                payload.failures, payload.warnings
            )
        },
        payload,
    }
}

#[tauri::command]
pub fn reset_settings() -> CommandResult<SettingsPayload> {
    let settings = BackendSettings::default();
    match SettingsStore::default().save(&settings) {
        Ok(()) => settings_payload("设置已重置为默认值。", "设置重置后重新读取失败"),
        Err(error) => failed(
            &format!("重置设置失败：{error}"),
            SettingsPayload {
                settings,
                settings_path: codex_plus_core::paths::default_settings_path()
                    .to_string_lossy()
                    .to_string(),
                user_scripts: user_script_inventory(),
            },
        ),
    }
}

#[tauri::command]
pub fn relay_status() -> CommandResult<RelayPayload> {
    let status = codex_plus_core::relay_config::default_relay_status();
    let message = if status.configured {
        "极义 API 配置已就绪。"
    } else {
        "极义 API 配置未就绪，请配置阿里百炼或极义中转 API Key。"
    };
    ok(message, relay_payload(status, None))
}

#[tauri::command]
pub fn read_relay_files() -> CommandResult<RelayFilesPayload> {
    let home = codex_plus_core::relay_config::default_codex_home_dir();
    match relay_files_payload_from_home(&home) {
        Ok(payload) => ok("配置文件内容已读取。", payload),
        Err(error) => failed(
            &format!("读取配置文件失败：{error}"),
            RelayFilesPayload {
                config_path: home.join("config.toml").to_string_lossy().to_string(),
                auth_path: home.join("auth.json").to_string_lossy().to_string(),
                config_contents: String::new(),
                auth_contents: String::new(),
            },
        ),
    }
}

#[tauri::command]
pub fn save_relay_file(request: SaveRelayFileRequest) -> CommandResult<RelayFilesPayload> {
    let home = codex_plus_core::relay_config::default_codex_home_dir();
    match save_relay_file_in_home(&home, &request.kind, &request.contents)
        .and_then(|_| relay_files_payload_from_home(&home))
    {
        Ok(payload) => ok("配置文件已保存。", payload),
        Err(error) => failed(
            &format!("保存配置文件失败：{error}"),
            relay_files_payload_from_home(&home).unwrap_or_else(|_| RelayFilesPayload {
                config_path: home.join("config.toml").to_string_lossy().to_string(),
                auth_path: home.join("auth.json").to_string_lossy().to_string(),
                config_contents: String::new(),
                auth_contents: String::new(),
            }),
        ),
    }
}

#[tauri::command]
pub fn write_diagnostic_event(event: String, detail: Value) -> CommandResult<Value> {
    let event = sanitize_manager_event(&event);
    match codex_plus_core::diagnostic_log::append_diagnostic_log(&event, detail) {
        Ok(()) => ok("诊断日志已写入。", json!({})),
        Err(error) => failed(&format!("写入诊断日志失败：{error}"), json!({})),
    }
}

#[tauri::command]
pub fn backfill_relay_profile_from_live(
    request: BackfillRelayProfileRequest,
) -> CommandResult<SettingsBackfillPayload> {
    let home = codex_plus_core::relay_config::default_codex_home_dir();
    let mut settings = request.settings;
    let requested_profile_id = request.profile_id.clone();
    log_manager_event(
        "manager.backfill_relay_profile_from_live.start",
        json!({
            "profileId": requested_profile_id,
            "activeRelayId": settings.active_relay_id
        }),
    );
    let Some(profile) = settings
        .relay_profiles
        .iter_mut()
        .find(|profile| profile.id == request.profile_id)
    else {
        log_manager_event(
            "manager.backfill_relay_profile_from_live.missing_profile",
            json!({
                "profileId": requested_profile_id
            }),
        );
        return failed(
            "当前供应商已不在配置列表中，已停止切换以避免覆盖用户改动。",
            SettingsBackfillPayload { settings },
        );
    };

    match codex_plus_core::relay_config::backfill_relay_profile_from_home_with_common(
        &home,
        profile,
        &mut settings.relay_context_config_contents,
    ) {
        Ok(()) => {
            log_manager_event(
                "manager.backfill_relay_profile_from_live.ok",
                json!({
                    "profileId": requested_profile_id
                }),
            );
            ok(
                "当前供应商配置已从 live 文件回填。",
                SettingsBackfillPayload { settings },
            )
        }
        Err(error) => {
            log_manager_event(
                "manager.backfill_relay_profile_from_live.failed",
                json!({
                    "profileId": requested_profile_id,
                    "error": error.to_string()
                }),
            );
            failed(
                &format!("回填当前供应商配置失败：{error}"),
                SettingsBackfillPayload { settings },
            )
        }
    }
}

#[tauri::command]
pub fn list_context_entries(
    request: ContextSettingsRequest,
) -> CommandResult<ContextEntriesPayload> {
    match codex_plus_core::relay_config::list_context_entries_from_common_config(
        &request.settings.relay_context_config_contents,
    ) {
        Ok(entries) => ok(
            "工具与插件列表已读取。",
            ContextEntriesPayload {
                settings: request.settings,
                entries,
            },
        ),
        Err(error) => failed(
            &format!("读取工具与插件列表失败：{error}"),
            ContextEntriesPayload {
                settings: request.settings,
                entries: empty_context_entries(),
            },
        ),
    }
}

#[tauri::command]
pub fn read_live_context_entries() -> CommandResult<LiveContextEntriesPayload> {
    let home = codex_plus_core::relay_config::default_codex_home_dir();
    let config_path = home.join("config.toml");
    let config = read_optional_text_file(&config_path).unwrap_or_default();
    match codex_plus_core::relay_config::list_context_entries_from_common_config(&config) {
        Ok(entries) => ok(
            "live 工具与插件已读取。",
            LiveContextEntriesPayload { entries },
        ),
        Err(error) => failed(
            &format!("读取 live 工具与插件失败：{error}"),
            LiveContextEntriesPayload {
                entries: empty_context_entries(),
            },
        ),
    }
}

#[tauri::command]
pub fn upsert_context_entry(request: ContextEntryRequest) -> CommandResult<ContextEntriesPayload> {
    let mut settings = request.settings;
    match codex_plus_core::relay_config::upsert_context_entry_in_common_config(
        &settings.relay_context_config_contents,
        &request.kind,
        &request.id,
        &request.toml_body,
    ) {
        Ok(common) => {
            settings.relay_context_config_contents = common;
            list_context_entries(ContextSettingsRequest { settings })
        }
        Err(error) => failed(
            &format!("保存工具与插件失败：{error}"),
            ContextEntriesPayload {
                settings,
                entries: empty_context_entries(),
            },
        ),
    }
}

#[tauri::command]
pub fn sync_live_context_entries(
    request: ContextSettingsRequest,
) -> CommandResult<LiveContextEntriesPayload> {
    let home = codex_plus_core::relay_config::default_codex_home_dir();
    let config_path = home.join("config.toml");
    let current_config = match read_optional_text_file(&config_path) {
        Ok(config) => config,
        Err(error) => {
            return failed(
                &format!("读取 live config.toml 失败：{error}"),
                LiveContextEntriesPayload {
                    entries: empty_context_entries(),
                },
            );
        }
    };
    let updated_config = match codex_plus_core::relay_config::sync_live_config_context_entries(
        &current_config,
        &request.settings.relay_context_config_contents,
    ) {
        Ok(config) => config,
        Err(error) => {
            return failed(
                &format!("同步 live 工具与插件失败：{error}"),
                LiveContextEntriesPayload {
                    entries: empty_context_entries(),
                },
            );
        }
    };
    if let Some(parent) = config_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return failed(
                &format!("创建 Codex 配置目录失败：{error}"),
                LiveContextEntriesPayload {
                    entries: empty_context_entries(),
                },
            );
        }
    }
    if let Err(error) = std::fs::write(&config_path, &updated_config) {
        return failed(
            &format!("写入 live config.toml 失败：{error}"),
            LiveContextEntriesPayload {
                entries: empty_context_entries(),
            },
        );
    }
    match codex_plus_core::relay_config::list_context_entries_from_common_config(&updated_config) {
        Ok(entries) => ok(
            "live 工具与插件已同步。",
            LiveContextEntriesPayload { entries },
        ),
        Err(error) => failed(
            &format!("读取同步后的 live 工具与插件失败：{error}"),
            LiveContextEntriesPayload {
                entries: empty_context_entries(),
            },
        ),
    }
}

#[tauri::command]
pub fn delete_context_entry(request: ContextDeleteRequest) -> CommandResult<ContextEntriesPayload> {
    let mut settings = request.settings;
    match codex_plus_core::relay_config::delete_context_entry_from_common_config(
        &settings.relay_context_config_contents,
        &request.kind,
        &request.id,
    ) {
        Ok(common) => {
            settings.relay_context_config_contents = common;
            list_context_entries(ContextSettingsRequest { settings })
        }
        Err(error) => failed(
            &format!("删除工具与插件失败：{error}"),
            ContextEntriesPayload {
                settings,
                entries: empty_context_entries(),
            },
        ),
    }
}

#[tauri::command]
pub fn extract_relay_common_config(
    request: ExtractRelayCommonConfigRequest,
) -> CommandResult<ExtractRelayCommonConfigPayload> {
    match codex_plus_core::relay_config::extract_common_config_from_config(&request.config_contents)
        .and_then(|common_config_contents| {
            let profile_config_contents =
                codex_plus_core::relay_config::strip_common_config_from_config(
                    &request.config_contents,
                    &common_config_contents,
                )?;
            Ok(ExtractRelayCommonConfigPayload {
                common_config_contents,
                profile_config_contents,
            })
        }) {
        Ok(payload) => ok("通用配置已按兼容切换规则提取。", payload),
        Err(error) => failed(
            &format!("提取通用配置失败：{error}"),
            ExtractRelayCommonConfigPayload {
                common_config_contents: String::new(),
                profile_config_contents: request.config_contents,
            },
        ),
    }
}

#[tauri::command]
pub async fn test_relay_profile(profile: RelayProfile) -> CommandResult<RelayProfileTestPayload> {
    let profile_name = if profile.name.trim().is_empty() {
        "未命名供应商"
    } else {
        profile.name.trim()
    };
    let settings =
        settings_with_live_ccs_profiles(SettingsStore::default().load().unwrap_or_default());
    let test_model = if profile.test_model.trim().is_empty() {
        settings.relay_test_model.trim()
    } else {
        profile.test_model.trim()
    };
    match codex_plus_core::relay_config::test_relay_profile(&profile, test_model).await {
        Ok(result) => {
            let status = if result.http_status < 400 {
                "ok"
            } else {
                "failed"
            };
            log_manager_event(
                "manager.relay_profile_test",
                json!({
                    "profileId": profile.id.clone(),
                    "profileName": profile_name,
                    "testModel": test_model,
                    "httpStatus": result.http_status,
                    "endpoint": result.endpoint,
                    "status": status,
                }),
            );
            let preview = result.response_preview.trim();
            let detail = if preview.is_empty() {
                "响应内容为空".to_string()
            } else {
                format!("响应：{preview}")
            };
            CommandResult {
                status: status.to_string(),
                message: format!(
                    "已向「{profile_name}」用模型「{test_model}」发送 hi，HTTP {}。{detail}",
                    result.http_status
                ),
                payload: RelayProfileTestPayload {
                    http_status: result.http_status,
                    endpoint: result.endpoint,
                    response_preview: result.response_preview,
                },
            }
        }
        Err(error) => {
            log_manager_event(
                "manager.relay_profile_test.failed",
                json!({
                    "profileId": profile.id,
                    "profileName": profile_name,
                    "testModel": test_model,
                    "error": error.to_string(),
                }),
            );
            failed(
                &format!("测试「{profile_name}」失败：{error}"),
                RelayProfileTestPayload {
                    http_status: 0,
                    endpoint: String::new(),
                    response_preview: String::new(),
                },
            )
        }
    }
}

#[tauri::command]
pub async fn fetch_relay_profile_models(
    profile: RelayProfile,
) -> CommandResult<RelayProfileModelsPayload> {
    let profile_name = if profile.name.trim().is_empty() {
        "未命名供应商"
    } else {
        profile.name.trim()
    };
    match codex_plus_core::model_catalog::fetch_relay_profile_model_ids(&profile).await {
        Ok((models, endpoint)) => ok(
            &format!("已从「{profile_name}」获取 {} 个模型。", models.len()),
            RelayProfileModelsPayload { models, endpoint },
        ),
        Err(error) => failed(
            &format!("从「{profile_name}」获取模型失败：{error}"),
            RelayProfileModelsPayload {
                models: Vec::new(),
                endpoint: String::new(),
            },
        ),
    }
}

#[tauri::command]
pub fn apply_relay_injection() -> CommandResult<RelayPayload> {
    let home = codex_plus_core::relay_config::default_codex_home_dir();
    let settings =
        settings_with_live_ccs_profiles(SettingsStore::default().load().unwrap_or_default());
    if !settings.relay_profiles_enabled {
        let status = codex_plus_core::relay_config::relay_status_from_home(&home);
        return failed(
            "供应商配置总开关已关闭，未写入 config.toml / auth.json。",
            relay_payload(status, None),
        );
    }
    let mut relay = settings.active_relay_profile();
    let api_key = codex_plus_core::protocol_proxy::resolved_relay_api_key(&settings, &relay);
    if !api_key.trim().is_empty() {
        relay.api_key = api_key.clone();
        if let Err(error) =
            codex_plus_core::secret_store::materialize_relay_profile_secrets(&mut relay, &api_key)
        {
            let status = codex_plus_core::relay_config::relay_status_from_home(&home);
            return failed(
                &format!("读取钥匙串 API Key 失败：{error}"),
                relay_payload(status, None),
            );
        }
    }
    log_relay_apply_request("manager.apply_relay_injection", &settings, &relay);
    if relay.relay_mode != codex_plus_core::settings::RelayMode::PureApi {
        let status = codex_plus_core::relay_config::relay_status_from_home(&home);
        log_relay_apply_result(
            "manager.apply_relay_injection.blocked_jiyi_native",
            &relay,
            &status,
            None,
            Some("极义版已禁用官方登录和混合 API 模式".to_string()),
        );
        return failed(
            "极义codex 已禁用官方登录和混合 API 模式，请使用阿里百炼 / 极义中转纯 API。",
            relay_payload(status, None),
        );
    }
    if settings.jiyi_local_proxy_enabled {
        return match codex_plus_core::relay_config::apply_local_proxy_config_to_home(
            &home,
            codex_plus_core::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
        ) {
            Ok(result) => {
                let status = codex_plus_core::relay_config::relay_status_from_home(&home);
                log_relay_apply_result(
                    "manager.apply_relay_injection.local_proxy_ok",
                    &relay,
                    &status,
                    result.backup_path.as_ref(),
                    None,
                );
                ok(
                    "已写入极义本地代理配置；真实 API Key 保留在极义侧，不写入 Codex Home。",
                    relay_payload(status, result.backup_path),
                )
            }
            Err(error) => {
                let status = codex_plus_core::relay_config::relay_status_from_home(&home);
                log_relay_apply_result(
                    "manager.apply_relay_injection.local_proxy_failed",
                    &relay,
                    &status,
                    None,
                    Some(error.to_string()),
                );
                failed(
                    &format!("写入极义本地代理配置失败：{error}"),
                    relay_payload(status, None),
                )
            }
        };
    }
    if relay_has_complete_files(&relay) {
        return match codex_plus_core::relay_config::apply_relay_profile_to_home_with_switch_rules(
            &home,
            &relay,
            &relay_combined_common_config(&settings),
        ) {
            Ok(result) => {
                let status = codex_plus_core::relay_config::relay_status_from_home(&home);
                log_relay_apply_result(
                    "manager.apply_relay_injection.ok",
                    &relay,
                    &status,
                    result.backup_path.as_ref(),
                    None,
                );
                ok(
                    "已按兼容切换规则切换供应商。",
                    relay_payload(status, result.backup_path),
                )
            }
            Err(error) => {
                let status = codex_plus_core::relay_config::relay_status_from_home(&home);
                log_relay_apply_result(
                    "manager.apply_relay_injection.failed",
                    &relay,
                    &status,
                    None,
                    Some(error.to_string()),
                );
                failed(
                    &format!("切换完整中转配置失败：{error}"),
                    relay_payload(status, None),
                )
            }
        };
    }

    match codex_plus_core::relay_config::apply_pure_api_config_to_home_with_protocol(
        &home,
        &relay.base_url,
        &relay.api_key,
        relay.protocol,
        codex_plus_core::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
    ) {
        Ok(result) => {
            let status = codex_plus_core::relay_config::relay_status_from_home(&home);
            log_relay_apply_result(
                "manager.apply_relay_injection.ok",
                &relay,
                &status,
                result.backup_path.as_ref(),
                None,
            );
            ok(
                "极义纯 API 配置已写入，密钥未在界面明文显示。",
                relay_payload(status, result.backup_path),
            )
        }
        Err(error) => {
            let status = codex_plus_core::relay_config::relay_status_from_home(&home);
            log_relay_apply_result(
                "manager.apply_relay_injection.failed",
                &relay,
                &status,
                None,
                Some(error.to_string()),
            );
            failed(
                &format!("写入中转配置失败：{error}"),
                relay_payload(status, None),
            )
        }
    }
}

#[tauri::command]
pub fn apply_pure_api_injection() -> CommandResult<RelayPayload> {
    let home = codex_plus_core::relay_config::default_codex_home_dir();
    let settings =
        settings_with_live_ccs_profiles(SettingsStore::default().load().unwrap_or_default());
    if !settings.relay_profiles_enabled {
        let status = codex_plus_core::relay_config::relay_status_from_home(&home);
        return failed(
            "供应商配置总开关已关闭，未写入 config.toml / auth.json。",
            relay_payload(status, None),
        );
    }
    let mut relay = settings.active_relay_profile();
    let api_key = codex_plus_core::protocol_proxy::resolved_relay_api_key(&settings, &relay);
    if !api_key.trim().is_empty() {
        relay.api_key = api_key.clone();
        if let Err(error) =
            codex_plus_core::secret_store::materialize_relay_profile_secrets(&mut relay, &api_key)
        {
            let status = codex_plus_core::relay_config::relay_status_from_home(&home);
            return failed(
                &format!("读取钥匙串 API Key 失败：{error}"),
                relay_payload(status, None),
            );
        }
    }
    log_relay_apply_request("manager.apply_pure_api_injection", &settings, &relay);
    if settings.jiyi_local_proxy_enabled {
        return match codex_plus_core::relay_config::apply_local_proxy_config_to_home(
            &home,
            codex_plus_core::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
        ) {
            Ok(result) => {
                let status = codex_plus_core::relay_config::relay_status_from_home(&home);
                log_relay_apply_result(
                    "manager.apply_pure_api_injection.local_proxy_ok",
                    &relay,
                    &status,
                    result.backup_path.as_ref(),
                    None,
                );
                ok(
                    "纯 API 请求将通过极义本地代理转发；真实 API Key 不写入 Codex Home。",
                    relay_payload(status, result.backup_path),
                )
            }
            Err(error) => {
                let status = codex_plus_core::relay_config::relay_status_from_home(&home);
                log_relay_apply_result(
                    "manager.apply_pure_api_injection.local_proxy_failed",
                    &relay,
                    &status,
                    None,
                    Some(error.to_string()),
                );
                failed(
                    &format!("写入极义本地代理配置失败：{error}"),
                    relay_payload(status, None),
                )
            }
        };
    }
    if relay_has_complete_files(&relay) {
        return match codex_plus_core::relay_config::apply_relay_profile_to_home_with_switch_rules(
            &home,
            &relay,
            &relay_combined_common_config(&settings),
        ) {
            Ok(result) => {
                let status = codex_plus_core::relay_config::relay_status_from_home(&home);
                log_relay_apply_result(
                    "manager.apply_pure_api_injection.ok",
                    &relay,
                    &status,
                    result.backup_path.as_ref(),
                    None,
                );
                if !status.configured {
                    return failed(
                        "纯 API 配置写入后未检测到完整 custom provider，请检查 config.toml 和供应商 API Key。",
                        relay_payload(status, result.backup_path),
                    );
                }
                ok(
                    "已按兼容切换规则切换供应商。",
                    relay_payload(status, result.backup_path),
                )
            }
            Err(error) => {
                let status = codex_plus_core::relay_config::relay_status_from_home(&home);
                log_relay_apply_result(
                    "manager.apply_pure_api_injection.failed",
                    &relay,
                    &status,
                    None,
                    Some(error.to_string()),
                );
                failed(
                    &format!("切换纯 API 配置失败：{error}"),
                    relay_payload(status, None),
                )
            }
        };
    }

    match codex_plus_core::relay_config::apply_pure_api_config_to_home_with_protocol(
        &home,
        &relay.base_url,
        &relay.api_key,
        relay.protocol,
        codex_plus_core::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
    ) {
        Ok(result) => {
            let status = codex_plus_core::relay_config::relay_status_from_home(&home);
            log_relay_apply_result(
                "manager.apply_pure_api_injection.ok",
                &relay,
                &status,
                result.backup_path.as_ref(),
                None,
            );
            if !status.configured {
                return failed(
                    "纯 API 配置写入后未检测到完整 custom provider，请检查 config.toml 和供应商 API Key。",
                    relay_payload(status, result.backup_path),
                );
            }
            ok(
                "纯 API 模式已写入：config.toml 已写入 custom provider，auth.json 已切换为当前供应商。",
                relay_payload(status, result.backup_path),
            )
        }
        Err(error) => {
            let status = codex_plus_core::relay_config::relay_status_from_home(&home);
            log_relay_apply_result(
                "manager.apply_pure_api_injection.failed",
                &relay,
                &status,
                None,
                Some(error.to_string()),
            );
            failed(
                &format!("写入纯 API 模式失败：{error}"),
                relay_payload(status, None),
            )
        }
    }
}

#[tauri::command]
pub fn clear_relay_injection() -> CommandResult<RelayPayload> {
    let home = codex_plus_core::relay_config::default_codex_home_dir();
    let status = codex_plus_core::relay_config::relay_status_from_home(&home);
    log_manager_event(
        "manager.clear_relay_injection.blocked_jiyi_native",
        json!({
            "configured": status.configured
        }),
    );
    failed(
        "极义codex 是独立账号体系，已禁用切回官方 ChatGPT 登录模式。",
        relay_payload(status, None),
    )
}

fn relay_has_complete_files(relay: &codex_plus_core::settings::RelayProfile) -> bool {
    if relay.relay_mode == codex_plus_core::settings::RelayMode::Official
        && relay.official_mix_api_key
    {
        return !relay.config_contents.trim().is_empty();
    }
    !relay.config_contents.trim().is_empty() && !relay.auth_contents.trim().is_empty()
}

fn log_relay_apply_request(
    event: &str,
    settings: &BackendSettings,
    relay: &codex_plus_core::settings::RelayProfile,
) {
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        event,
        json!({
            "activeRelayId": settings.active_relay_id,
            "relayId": relay.id,
            "relayName": relay.name,
            "relayMode": relay.relay_mode,
            "protocol": relay.protocol,
            "baseUrl": relay.base_url,
            "hasConfigContents": !relay.config_contents.trim().is_empty(),
            "hasAuthContents": !relay.auth_contents.trim().is_empty(),
            "configContainsProxy": relay.config_contents.contains("127.0.0.1:57321")
        }),
    );
}

fn log_relay_apply_result(
    event: &str,
    relay: &codex_plus_core::settings::RelayProfile,
    status: &codex_plus_core::relay_config::RelayStatus,
    backup_path: Option<&String>,
    error: Option<String>,
) {
    log_manager_event(
        event,
        json!({
            "relayId": relay.id,
            "relayName": relay.name,
            "relayMode": relay.relay_mode,
            "protocol": relay.protocol,
            "configured": status.configured,
            "requiresOpenaiAuth": status.requires_openai_auth,
            "hasBearerToken": status.has_bearer_token,
            "backupPath": backup_path,
            "error": error
        }),
    );
}

fn log_manager_event(event: &str, detail: Value) {
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(event, detail);
}

fn sanitize_manager_event(event: &str) -> String {
    let suffix = event
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let suffix = suffix.trim_matches(['.', '_', '-']).trim();
    if suffix.is_empty() {
        "manager.ui.event".to_string()
    } else if suffix.starts_with("manager.") {
        suffix.to_string()
    } else {
        format!("manager.ui.{suffix}")
    }
}

fn refresh_cli_wrapper_after_settings_save(settings: &BackendSettings) -> String {
    match codex_plus_core::cli_wrapper::ensure_cli_wrapper(settings) {
        Ok(Some(install)) => format!(
            " 命令包装器已更新：{}。",
            install.real_codex.to_string_lossy()
        ),
        Ok(None) => String::new(),
        Err(error) => format!(" 但命令包装器更新失败：{error}。"),
    }
}

fn relay_payload(
    status: codex_plus_core::relay_config::RelayStatus,
    backup_path: Option<String>,
) -> RelayPayload {
    let settings =
        settings_with_live_ccs_profiles(SettingsStore::default().load().unwrap_or_default());
    let active = settings.active_relay_profile();
    let key_resolution =
        codex_plus_core::protocol_proxy::resolved_relay_api_key_details(&settings, &active);
    RelayPayload {
        authenticated: status.authenticated,
        auth_source: status.auth_source,
        account_label: status.account_label,
        config_path: status.config_path,
        configured: status.configured,
        requires_openai_auth: status.requires_openai_auth,
        has_bearer_token: status.has_bearer_token,
        api_key_configured: key_resolution.configured(),
        api_key_source: key_resolution.source,
        backup_path,
    }
}

fn empty_context_entries() -> codex_plus_core::relay_config::CodexContextEntries {
    codex_plus_core::relay_config::CodexContextEntries {
        mcp_servers: Vec::new(),
        skills: Vec::new(),
        plugins: Vec::new(),
    }
}

fn relay_files_payload_from_home(home: &std::path::Path) -> anyhow::Result<RelayFilesPayload> {
    let config_path = home.join("config.toml");
    let auth_path = home.join("auth.json");
    Ok(RelayFilesPayload {
        config_path: config_path.to_string_lossy().to_string(),
        auth_path: auth_path.to_string_lossy().to_string(),
        config_contents: read_optional_text_file(&config_path)?,
        auth_contents: read_optional_text_file(&auth_path)?,
    })
}

fn save_relay_file_in_home(
    home: &std::path::Path,
    kind: &str,
    contents: &str,
) -> anyhow::Result<()> {
    let path = match kind {
        "config" => home.join("config.toml"),
        "auth" => home.join("auth.json"),
        other => anyhow::bail!("未知配置文件类型：{other}"),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(())
}

fn read_optional_text_file(path: &std::path::Path) -> anyhow::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

fn ads_payload(payload: Value) -> AdsPayload {
    AdsPayload {
        version: payload.get("version").and_then(Value::as_u64).unwrap_or(1),
        ads: payload
            .get("ads")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    }
}

fn open_url(url: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        codex_plus_core::windows_open_url(url)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("启动系统浏览器失败：{error}"))
    }
}

fn settings_payload(message: &str, failure_context: &str) -> CommandResult<SettingsPayload> {
    match settings_payload_value() {
        Ok(payload) => ok(message, payload),
        Err((error, payload)) => failed(&format!("{failure_context}：{error}"), payload),
    }
}

fn settings_payload_value() -> Result<SettingsPayload, (anyhow::Error, SettingsPayload)> {
    let store = SettingsStore::default();
    let settings_path = codex_plus_core::paths::default_settings_path()
        .to_string_lossy()
        .to_string();
    match store.load() {
        Ok(settings) => Ok(SettingsPayload {
            settings: settings_with_live_ccs_profiles(settings),
            settings_path,
            user_scripts: user_script_inventory(),
        }),
        Err(error) => Err((
            error,
            SettingsPayload {
                settings: BackendSettings::default(),
                settings_path,
                user_scripts: user_script_inventory(),
            },
        )),
    }
}

fn fallback_settings_payload() -> SettingsPayload {
    SettingsPayload {
        settings: settings_with_live_ccs_profiles(
            SettingsStore::default().load().unwrap_or_default(),
        ),
        settings_path: codex_plus_core::paths::default_settings_path()
            .to_string_lossy()
            .to_string(),
        user_scripts: user_script_inventory(),
    }
}

fn user_script_inventory() -> Value {
    default_user_script_manager()
        .inventory()
        .unwrap_or_else(|error| {
            json!({
                "enabled": true,
                "scripts": [],
                "error": error.to_string()
            })
        })
}

fn failed_script_market_payload(message: &str) -> ScriptMarketPayload {
    ScriptMarketPayload {
        market: json!({
            "status": "failed",
            "message": message,
            "indexUrl": script_market::DEFAULT_MARKET_INDEX_URL,
            "updatedAt": "",
            "scripts": []
        }),
        user_scripts: user_script_inventory(),
    }
}

fn script_market_payload_from_manifest(
    manifest: &ScriptMarketManifest,
    status: &str,
    message: &str,
) -> ScriptMarketPayload {
    let user_scripts = user_script_inventory();
    let installed = installed_market_versions(&user_scripts);
    let scripts = manifest
        .scripts
        .iter()
        .map(|script| market_script_payload(script, &installed))
        .collect::<Vec<_>>();
    ScriptMarketPayload {
        market: json!({
            "status": status,
            "message": message,
            "indexUrl": script_market::DEFAULT_MARKET_INDEX_URL,
            "updatedAt": manifest.updated_at.clone().unwrap_or_default(),
            "scripts": scripts
        }),
        user_scripts,
    }
}

fn installed_market_versions(user_scripts: &Value) -> BTreeMap<String, String> {
    user_scripts
        .get("scripts")
        .and_then(Value::as_array)
        .map(|scripts| {
            scripts
                .iter()
                .filter_map(|script| {
                    let id = script.get("market_id").and_then(Value::as_str)?;
                    if id.is_empty() {
                        return None;
                    }
                    let version = script
                        .get("version")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    Some((id.to_string(), version))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn market_script_payload(script: &MarketScript, installed: &BTreeMap<String, String>) -> Value {
    let installed_version = installed.get(&script.id).cloned().unwrap_or_default();
    let is_installed = !installed_version.is_empty();
    json!({
        "id": script.id,
        "name": script.name,
        "description": script.description,
        "version": script.version,
        "author": script.author,
        "tags": script.tags,
        "homepage": script.homepage,
        "script_url": script.script_url,
        "sha256": script.sha256,
        "installed": is_installed,
        "installedVersion": installed_version,
        "updateAvailable": is_installed && installed.get(&script.id).map(|version| version != &script.version).unwrap_or(false)
    })
}

fn default_user_script_manager() -> UserScriptManager {
    let config_dir = user_scripts_config_dir();
    UserScriptManager::new(
        builtin_user_scripts_dir(),
        config_dir.join("user_scripts"),
        config_dir.join("user_scripts.json"),
    )
}

fn user_scripts_config_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(roaming) = std::env::var_os("APPDATA") {
            return PathBuf::from(roaming).join("Codex++");
        }
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("Codex++")
}

fn builtin_user_scripts_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .map(|path| path.join("user_scripts"))
        .unwrap_or_else(|| PathBuf::from("user_scripts"))
}

fn diagnostics_report() -> String {
    let (codex_app_path, entrypoints, latest_launch) = load_overview_payload();
    let overview = ok(
        "概览已加载。",
        OverviewPayload {
            codex_version: codex_app_path
                .as_deref()
                .and_then(codex_plus_core::app_paths::codex_app_version),
            codex_app: path_state(codex_app_path),
            silent_shortcut: shortcut_state(entrypoints.silent_shortcut),
            management_shortcut: shortcut_state(entrypoints.management_shortcut),
            latest_launch,
            current_version: codex_plus_core::version::VERSION.to_string(),
            update_status: "not_checked".to_string(),
            settings_path: codex_plus_core::paths::default_settings_path()
                .to_string_lossy()
                .to_string(),
            logs_path: codex_plus_core::paths::default_diagnostic_log_path()
                .to_string_lossy()
                .to_string(),
        },
    );
    let settings = SettingsStore::default().load().unwrap_or_default();
    let generated_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    serde_json::to_string_pretty(&json!({
        "generatedAtMs": generated_at_ms,
        "version": codex_plus_core::version::VERSION,
        "overview": overview.payload,
        "settings": settings,
        "logs": {
            "diagnosticLogPath": codex_plus_core::paths::default_diagnostic_log_path(),
            "latestStatusPath": codex_plus_core::paths::default_latest_status_path()
        },
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH
        }
    }))
    .unwrap_or_else(|error| format!("诊断报告序列化失败：{error}"))
}

fn load_overview_payload() -> (
    Option<PathBuf>,
    install::EntryPointState,
    Option<LaunchStatus>,
) {
    let settings = SettingsStore::default().load().unwrap_or_default();
    (
        codex_plus_core::app_paths::resolve_codex_app_dir_with_saved(
            None,
            Some(settings.codex_app_path.as_str()),
        ),
        install::inspect_entrypoints(),
        StatusStore::default().load_latest().unwrap_or(None),
    )
}

fn install_background_failure(action: &str, error: impl std::fmt::Display) -> InstallActionResult {
    let state = install::inspect_entrypoints();
    InstallActionResult {
        status: "failed".to_string(),
        message: format!("{action}后台任务失败：{error}"),
        silent_shortcut: state.silent_shortcut,
        management_shortcut: state.management_shortcut,
    }
}

fn watcher_payload() -> WatcherPayload {
    let flag = codex_plus_core::watcher::default_watcher_disabled_flag();
    WatcherPayload {
        enabled: !flag.exists(),
        disabled_flag: flag.to_string_lossy().to_string(),
    }
}

fn release_readiness_payload() -> ReleaseReadinessPayload {
    let mut items = Vec::new();
    items.extend(release_app_bundle_checks());
    items.extend(release_dmg_checks());
    items.extend(release_original_codex_isolation_checks());
    items.extend(release_local_account_checks());
    items.extend(release_sms_provider_checks());
    items.extend(release_local_entitlement_checks());
    items.extend(release_local_usage_checks());
    items.extend(release_local_backend_checks());
    items.extend(release_identity_sync_checks());
    items.extend(release_managed_proxy_checks());
    items.extend(release_api_key_risk_checks());
    items.extend(release_notarization_checks());

    let failures = items.iter().filter(|item| item.status == "failed").count();
    let warnings = items.iter().filter(|item| item.status == "warning").count();
    ReleaseReadinessPayload {
        ready: failures == 0 && warnings == 0,
        failures,
        warnings,
        checked_at_ms: now_ms(),
        items,
    }
}

fn release_app_bundle_checks() -> Vec<ReleaseReadinessItem> {
    let main_app = PathBuf::from("/Applications/极义codex.app");
    let manager_app = PathBuf::from("/Applications/极义codex 管理工具.app");
    let embedded_client = main_app
        .join("Contents")
        .join("Resources")
        .join("JiyiCodexClient.app");
    let mut items = vec![
        bundle_id_check(
            "main_bundle",
            "主应用 bundle id",
            &main_app,
            "com.jiyi.codex",
        ),
        bundle_id_check(
            "manager_bundle",
            "管理工具 bundle id",
            &manager_app,
            "com.jiyi.codex.manager",
        ),
        bundle_id_check(
            "embedded_client_bundle",
            "内置 Codex 客户端 bundle id",
            &embedded_client,
            "com.jiyi.codex.client",
        ),
        codesign_verify_check("main_codesign", "主应用签名校验", &main_app),
        codesign_verify_check("manager_codesign", "管理工具签名校验", &manager_app),
        codesign_verify_check(
            "embedded_client_codesign",
            "内置客户端签名校验",
            &embedded_client,
        ),
    ];
    items.push(embedded_client_runtime_isolation_check(&embedded_client));
    items.push(embedded_client_browser_user_data_isolation_check(
        &embedded_client,
    ));
    items.push(embedded_client_environment_isolation_check(
        &embedded_client,
    ));
    items.push(embedded_client_url_scheme_isolation_check(&embedded_client));
    items.push(managed_proxy_sidecar_check(&main_app, &manager_app));
    items.push(managed_proxy_launchd_deploy_check(&main_app, &manager_app));
    items.push(managed_proxy_remote_deploy_check(&main_app));
    items.push(developer_id_check(&main_app));
    items
}

fn managed_proxy_sidecar_check(main_app: &Path, manager_app: &Path) -> ReleaseReadinessItem {
    let main_proxy = main_app.join("Contents/MacOS/jiyi-managed-proxy");
    let manager_proxy = manager_app.join("Contents/MacOS/jiyi-managed-proxy");
    if main_proxy.is_file() && manager_proxy.is_file() {
        return ReleaseReadinessItem::ok(
            "managed_proxy_sidecar",
            "托管代理 sidecar",
            "主应用和管理工具均内置 jiyi-managed-proxy，本地部署阶段会复制到极义运行目录后启动。",
            Some(main_proxy),
        );
    }
    let missing = if !main_proxy.is_file() {
        main_proxy
    } else {
        manager_proxy
    };
    ReleaseReadinessItem::failed(
        "managed_proxy_sidecar",
        "托管代理 sidecar",
        "安装包缺少 jiyi-managed-proxy，启用极义托管代理后本机服务端无法启动。",
        Some(missing),
    )
}

fn managed_proxy_launchd_deploy_check(main_app: &Path, manager_app: &Path) -> ReleaseReadinessItem {
    let main_script =
        main_app.join("Contents/Resources/server/macos/install-managed-proxy-launchd.sh");
    let manager_script =
        manager_app.join("Contents/Resources/server/macos/install-managed-proxy-launchd.sh");
    let env_example =
        main_app.join("Contents/Resources/server/macos/jiyi-managed-proxy.env.example");
    if !main_script.is_file() || !manager_script.is_file() || !env_example.is_file() {
        let missing = if !main_script.is_file() {
            main_script
        } else if !manager_script.is_file() {
            manager_script
        } else {
            env_example
        };
        return ReleaseReadinessItem::failed(
            "managed_proxy_launchd_deploy",
            "托管代理本地服务部署",
            "安装包缺少 LaunchAgent 部署脚本或 env 示例，无法把 jiyi-managed-proxy 安装为本地常驻服务。",
            Some(missing),
        );
    }
    let script = fs::read_to_string(&main_script).unwrap_or_default();
    let env = fs::read_to_string(&env_example).unwrap_or_default();
    let script_ok = script.contains("com.jiyi.codex.managed-proxy")
        && script.contains("launchctl bootstrap")
        && script.contains("jiyi-managed-proxy.env")
        && script.contains("STATE_DIR/bin")
        && script.contains("jiyi-managed-proxy");
    let env_ok = env.contains("JIYI_MANAGED_PROXY_UPSTREAM_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_SYNC_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_ADMIN_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_USER_READ_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_BILLING_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET")
        && env.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY")
        && env.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH")
        && env.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY")
        && env.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH")
        && env.contains("JIYI_MANAGED_PROXY_ACCESS_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_AUDIT_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_DB_PATH");
    if script_ok && env_ok {
        ReleaseReadinessItem::ok(
            "managed_proxy_launchd_deploy",
            "托管代理本地服务部署",
            "安装包内置 LaunchAgent 安装脚本和 env 示例，会把 jiyi-managed-proxy 复制到极义运行目录并部署为 macOS 本地常驻服务。",
            Some(main_script),
        )
    } else {
        ReleaseReadinessItem::warning(
            "managed_proxy_launchd_deploy",
            "托管代理本地服务部署",
            "LaunchAgent 部署脚本存在，但缺少服务标签、launchctl 安装流程或关键环境变量示例。",
            Some(main_script),
        )
    }
}

fn managed_proxy_remote_deploy_check(main_app: &Path) -> ReleaseReadinessItem {
    let linux_installer =
        main_app.join("Contents/Resources/server/linux/install-managed-proxy-systemd.sh");
    let linux_service = main_app.join("Contents/Resources/server/linux/jiyi-managed-proxy.service");
    let linux_env = main_app.join("Contents/Resources/server/linux/jiyi-managed-proxy.env.example");
    let dockerfile = main_app.join("Contents/Resources/server/docker/Dockerfile");
    for path in [&linux_installer, &linux_service, &linux_env, &dockerfile] {
        if !path.is_file() {
            return ReleaseReadinessItem::failed(
                "managed_proxy_remote_deploy",
                "托管代理远端部署模板",
                "安装包缺少 Linux/systemd 或 Docker 部署模板，远端托管代理生产部署缺少标准入口。",
                Some(path.to_path_buf()),
            );
        }
    }

    let installer = fs::read_to_string(&linux_installer).unwrap_or_default();
    let service = fs::read_to_string(&linux_service).unwrap_or_default();
    let env = fs::read_to_string(&linux_env).unwrap_or_default();
    let docker = fs::read_to_string(&dockerfile).unwrap_or_default();
    let installer_ok = installer.contains("systemctl enable")
        && installer.contains("jiyi-managed-proxy")
        && installer.contains("/etc/jiyi-codex")
        && installer.contains("/var/lib/jiyi-codex");
    let service_ok = service.contains("EnvironmentFile=/etc/jiyi-codex/jiyi-managed-proxy.env")
        && service.contains("ExecStart=/usr/local/bin/jiyi-managed-proxy")
        && service.contains("NoNewPrivileges=true");
    let env_ok = env.contains("JIYI_MANAGED_PROXY_UPSTREAM_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_SYNC_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_ADMIN_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_USER_READ_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_BILLING_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET")
        && env.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY")
        && env.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH")
        && env.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY")
        && env.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH")
        && env.contains("JIYI_MANAGED_PROXY_ACCESS_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_AUDIT_API_KEY")
        && env.contains("JIYI_MANAGED_PROXY_DB_PATH");
    let docker_ok = docker.contains("cargo build --release -p jiyi-managed-proxy")
        && docker.contains("JIYI_MANAGED_PROXY_ADMIN_API_KEY")
        && docker.contains("JIYI_MANAGED_PROXY_USER_READ_API_KEY")
        && docker.contains("JIYI_MANAGED_PROXY_BILLING_API_KEY")
        && docker.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY")
        && docker.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET")
        && docker.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY")
        && docker.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH")
        && docker.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY")
        && docker.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH")
        && docker.contains("JIYI_MANAGED_PROXY_ACCESS_API_KEY")
        && docker.contains("JIYI_MANAGED_PROXY_AUDIT_API_KEY")
        && docker.contains("USER jiyi-codex")
        && docker.contains("EXPOSE 8080");

    if installer_ok && service_ok && env_ok && docker_ok {
        ReleaseReadinessItem::ok(
            "managed_proxy_remote_deploy",
            "托管代理远端部署模板",
            "安装包内置 Linux systemd、env 示例和 Dockerfile，可作为远端托管代理生产部署模板。",
            Some(linux_installer),
        )
    } else {
        ReleaseReadinessItem::warning(
            "managed_proxy_remote_deploy",
            "托管代理远端部署模板",
            "远端部署模板存在，但缺少 systemd、env 或 Docker 关键配置。",
            Some(linux_installer),
        )
    }
}

fn embedded_client_runtime_isolation_check(embedded_client: &Path) -> ReleaseReadinessItem {
    if !is_codex_app_bundle(embedded_client) {
        return ReleaseReadinessItem::failed(
            "embedded_client_no_official_fallback",
            "内置客户端无原版兜底",
            "未找到极义内置 JiyiCodexClient.app；主入口会阻止启动，避免打开 /Applications/Codex.app。",
            Some(embedded_client.to_path_buf()),
        );
    }
    match embedded_codex_app_path_from_contents_dir(
        embedded_client
            .parent()
            .and_then(Path::parent)
            .unwrap_or_else(|| Path::new("")),
    ) {
        Ok(runtime_app)
            if runtime_app
                .to_string_lossy()
                .contains("极义codex.noindex/embedded-client/JiyiCodexClient.app") =>
        {
            ReleaseReadinessItem::ok(
                "embedded_client_no_official_fallback",
                "内置客户端无原版兜底",
                "主入口只启动极义运行时客户端，不会回退 /Applications/Codex.app。",
                Some(runtime_app),
            )
        }
        Ok(runtime_app) => ReleaseReadinessItem::failed(
            "embedded_client_no_official_fallback",
            "内置客户端无原版兜底",
            "主入口解析到的客户端路径不是极义运行时目录。",
            Some(runtime_app),
        ),
        Err(error) => ReleaseReadinessItem::failed(
            "embedded_client_no_official_fallback",
            "内置客户端无原版兜底",
            format!("主入口客户端隔离检查失败：{error}"),
            Some(embedded_client.to_path_buf()),
        ),
    }
}

fn embedded_client_browser_user_data_isolation_check(
    embedded_client: &Path,
) -> ReleaseReadinessItem {
    let expected_dir = codex_plus_core::paths::default_jiyi_browser_user_data_dir();
    let expected_arg = format!("--user-data-dir={}", expected_dir.to_string_lossy());
    let command = codex_plus_core::launcher::build_macos_open_command(embedded_client, 9229, &[]);
    let uses_expected_dir = command.iter().any(|part| part == &expected_arg);
    let reuses_official_dir = command
        .iter()
        .any(|part| part.contains("Application Support/Codex"));

    if uses_expected_dir && !reuses_official_dir {
        ReleaseReadinessItem::ok(
            "embedded_client_user_data_isolation",
            "内置客户端浏览器数据隔离",
            "启动命令强制使用极义专用 --user-data-dir，不复用原版 Application Support/Codex。",
            Some(expected_dir),
        )
    } else {
        ReleaseReadinessItem::failed(
            "embedded_client_user_data_isolation",
            "内置客户端浏览器数据隔离",
            "启动命令未强制使用极义专用 --user-data-dir，可能复用原版 Codex 浏览器状态。",
            Some(expected_dir),
        )
    }
}

fn embedded_client_environment_isolation_check(embedded_client: &Path) -> ReleaseReadinessItem {
    let command = codex_plus_core::launcher::build_macos_open_command(embedded_client, 9229, &[]);
    let expected_codex_home = format!(
        "CODEX_HOME={}",
        codex_plus_core::relay_config::default_codex_home_dir().to_string_lossy()
    );
    let expected_home = format!(
        "HOME={}",
        codex_plus_core::paths::default_jiyi_unix_home_dir().to_string_lossy()
    );
    let expected_config_home = format!(
        "XDG_CONFIG_HOME={}",
        codex_plus_core::paths::default_jiyi_unix_home_dir()
            .join(".config")
            .to_string_lossy()
    );
    let clears_sensitive_env = codex_plus_core::launcher::jiyi_sensitive_environment_keys()
        .iter()
        .all(|key| command.iter().any(|part| part == &format!("{key}=")));
    let uses_isolated_home = command.iter().any(|part| part == &expected_codex_home)
        && command.iter().any(|part| part == &expected_home)
        && command.iter().any(|part| part == &expected_config_home);

    if uses_isolated_home && clears_sensitive_env {
        ReleaseReadinessItem::ok(
            "embedded_client_environment_isolation",
            "内置客户端环境隔离",
            "启动命令固定极义 CODEX_HOME/HOME/XDG_*，并清空通用 OpenAI/百炼/APIMart 环境变量。",
            Some(codex_plus_core::paths::default_jiyi_unix_home_dir()),
        )
    } else {
        ReleaseReadinessItem::failed(
            "embedded_client_environment_isolation",
            "内置客户端环境隔离",
            "启动命令未完整隔离 HOME/CODEX_HOME 或未清空通用 API 环境变量。",
            Some(codex_plus_core::paths::default_jiyi_unix_home_dir()),
        )
    }
}

fn embedded_client_url_scheme_isolation_check(embedded_client: &Path) -> ReleaseReadinessItem {
    let plist = embedded_client.join("Contents").join("Info.plist");
    if !plist.is_file() {
        return ReleaseReadinessItem::failed(
            "embedded_client_url_scheme_isolation",
            "内置客户端 URL Scheme 隔离",
            "未能读取内置客户端 Info.plist。",
            Some(embedded_client.to_path_buf()),
        );
    }
    if plist_key_exists(&plist, "CFBundleURLTypes") {
        return ReleaseReadinessItem::failed(
            "embedded_client_url_scheme_isolation",
            "内置客户端 URL Scheme 隔离",
            "内置客户端仍声明 URL Scheme，可能接管原版 Codex 的登录回调。",
            Some(plist),
        );
    }
    ReleaseReadinessItem::ok(
        "embedded_client_url_scheme_isolation",
        "内置客户端 URL Scheme 隔离",
        "内置客户端未声明 codex://，不会抢占原版 Codex 登录回调。",
        Some(plist),
    )
}

fn release_dmg_checks() -> Vec<ReleaseReadinessItem> {
    let Some(path) = find_release_dmg_path() else {
        return vec![ReleaseReadinessItem::warning(
            "dmg",
            "完整客户端 DMG",
            "未找到 DMG；设置 JIYI_CODEX_DMG_PATH 或在仓库 dist/macos 下生成。",
            None,
        )];
    };
    let size = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    if size >= 100 * 1024 * 1024 {
        vec![ReleaseReadinessItem::ok(
            "dmg",
            "完整客户端 DMG",
            format!("DMG 已存在，大小 {}M。", size / 1024 / 1024),
            Some(path),
        )]
    } else {
        vec![ReleaseReadinessItem::failed(
            "dmg",
            "完整客户端 DMG",
            format!(
                "DMG 只有 {}M，疑似未内置完整 Codex 客户端。",
                size / 1024 / 1024
            ),
            Some(path),
        )]
    }
}

fn release_original_codex_isolation_checks() -> Vec<ReleaseReadinessItem> {
    let home = codex_plus_core::paths::default_official_codex_home_dir();
    let candidates = official_codex_isolation_candidate_files();
    let contaminated = candidates
        .iter()
        .find(|path| official_codex_file_has_jiyi_contamination(path));
    if let Some(path) = contaminated {
        vec![ReleaseReadinessItem::failed(
            "official_codex_isolation",
            "原版 Codex 配置隔离",
            "原版 Codex 状态中仍检测到极义、百炼或 APIMart 写入痕迹，可在安装维护页执行“修复原版隔离”。",
            Some(path.to_path_buf()),
        )]
    } else {
        vec![ReleaseReadinessItem::ok(
            "official_codex_isolation",
            "原版 Codex 配置隔离",
            "原版 ~/.codex 和原版 Codex App Support 未检测到极义路径、百炼/APIMart、API Key 或 qwen3.7-plus。",
            Some(home),
        )]
    }
}

fn release_local_account_checks() -> Vec<ReleaseReadinessItem> {
    match codex_plus_core::local_account::LocalAccountStore::default().load_auth_state() {
        Ok(state) if state.session_ttl_hours > 0 => vec![ReleaseReadinessItem::ok(
            "local_account_session",
            "本地账号 session",
            format!(
                "本地账号 schema 可用，session TTL 为 {} 小时。",
                state.session_ttl_hours
            ),
            Some(PathBuf::from(state.db_path)),
        )],
        Ok(state) => vec![ReleaseReadinessItem::failed(
            "local_account_session",
            "本地账号 session",
            "本地账号 schema 可读但 session TTL 异常。",
            Some(PathBuf::from(state.db_path)),
        )],
        Err(error) => vec![ReleaseReadinessItem::failed(
            "local_account_session",
            "本地账号 session",
            format!("本地账号状态读取失败：{error}"),
            Some(codex_plus_core::local_account::default_auth_db_path()),
        )],
    }
}

fn release_sms_provider_checks() -> Vec<ReleaseReadinessItem> {
    let state = codex_plus_core::local_account::LocalAccountStore::default()
        .load_auth_state()
        .map(|state| state.sms_config);
    match state {
        Ok(state)
            if state.configured
                && !state.dry_run
                && state.app_id_set
                && state.sign_name_set
                && state.template_id_set =>
        {
            vec![ReleaseReadinessItem::ok(
                "tencent_sms_provider",
                "腾讯云短信生产配置",
                format!(
                    "腾讯云短信配置已完整，区域 {}，验证码有效期 {} 分钟。",
                    state.region, state.ttl_minutes
                ),
                None,
            )]
        }
        Ok(state) if state.configured && state.dry_run => vec![ReleaseReadinessItem::warning(
            "tencent_sms_provider",
            "腾讯云短信生产配置",
            "腾讯云短信参数已配置，但仍处于本地干跑模式；公开发布前需要关闭 JIYI_CODEX_SMS_DRY_RUN。",
            None,
        )],
        Ok(state) => {
            let mut missing = Vec::new();
            if !state.secret_id_set {
                missing.push("TENCENT_SMS_SECRET_ID 或默认 Keychain SecretId");
            }
            if !state.secret_key_set {
                missing.push("TENCENT_SMS_SECRET_KEY 或默认 Keychain SecretKey");
            }
            if !state.app_id_set {
                missing.push("TENCENT_SMS_APP_ID");
            }
            if !state.sign_name_set {
                missing.push("TENCENT_SMS_SIGN_NAME");
            }
            if !state.template_id_set {
                missing.push("TENCENT_SMS_TEMPLATE_ID");
            }
            vec![ReleaseReadinessItem::warning(
                "tencent_sms_provider",
                "腾讯云短信生产配置",
                format!(
                    "当前手机号登录仍使用本地干跑模式；公开发布前需要配置 {}。短信密钥也支持写入极义 Keychain 默认账号。",
                    missing.join(", ")
                ),
                None,
            )]
        }
        Err(error) => vec![ReleaseReadinessItem::failed(
            "tencent_sms_provider",
            "腾讯云短信生产配置",
            format!("读取短信配置失败：{error}"),
            Some(codex_plus_core::local_account::default_auth_db_path()),
        )],
    }
}

fn release_local_entitlement_checks() -> Vec<ReleaseReadinessItem> {
    let db_path = codex_plus_core::local_account::default_auth_db_path();
    let _ = codex_plus_core::local_account::LocalAccountStore::default().load_auth_state();
    let required_tables = ["local_users", "local_user_devices", "local_entitlements"];
    let result = rusqlite::Connection::open(&db_path).and_then(|db| {
        required_tables
            .iter()
            .map(|table| {
                db.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    rusqlite::params![table],
                    |row| row.get::<_, i64>(0),
                )
                .map(|count| (*table, count > 0))
            })
            .collect::<Result<Vec<_>, _>>()
    });
    match result {
        Ok(tables) if tables.iter().all(|(_, exists)| *exists) => vec![ReleaseReadinessItem::ok(
            "local_entitlement_model",
            "本地用户套餐模型",
            "本地用户、设备绑定和套餐额度 schema 可用。",
            Some(db_path),
        )],
        Ok(tables) => {
            let missing = tables
                .into_iter()
                .filter_map(|(table, exists)| (!exists).then_some(table))
                .collect::<Vec<_>>()
                .join(", ");
            vec![ReleaseReadinessItem::failed(
                "local_entitlement_model",
                "本地用户套餐模型",
                format!("缺少本地用户体系表：{missing}。"),
                Some(db_path),
            )]
        }
        Err(error) => vec![ReleaseReadinessItem::failed(
            "local_entitlement_model",
            "本地用户套餐模型",
            format!("本地用户套餐模型检查失败：{error}"),
            Some(db_path),
        )],
    }
}

fn release_local_usage_checks() -> Vec<ReleaseReadinessItem> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let policy = codex_plus_core::local_usage::LocalUsagePolicy::from_settings(&settings);
    match codex_plus_core::local_usage::LocalUsageStore::default().snapshot(policy) {
        Ok(snapshot) => vec![ReleaseReadinessItem::ok(
            "local_usage_meter",
            "本地用量记账",
            format!(
                "本地用量 schema 可用，今日已记录 {} 次请求、约 {} tokens。",
                snapshot.request_count, snapshot.used_tokens
            ),
            Some(PathBuf::from(snapshot.db_path)),
        )],
        Err(error) => vec![ReleaseReadinessItem::failed(
            "local_usage_meter",
            "本地用量记账",
            format!("本地用量状态读取失败：{error}"),
            Some(codex_plus_core::local_usage::default_usage_db_path()),
        )],
    }
}

fn release_local_backend_checks() -> Vec<ReleaseReadinessItem> {
    match codex_plus_core::local_backend::LocalBackendStore::default().state() {
        Ok(state) if state.initialized => vec![ReleaseReadinessItem::ok(
            "local_identity_backend",
            "本地账号服务端库",
            format!(
                "本地后端 schema 可用，已承接 {} 个用户、{} 个设备、{} 个团队、{} 个团队成员、{} 个套餐、{} 条续费记录、{} 个用量分组，服务端 session {} 个，其中有效 {} 个、已吊销 {} 个。",
                state.user_count,
                state.device_count,
                state.team_count,
                state.team_member_count,
                state.entitlement_count,
                state.billing_renewal_count,
                state.usage_summary_count,
                state.session_count,
                state.active_session_count,
                state.revoked_session_count
            ),
            Some(PathBuf::from(state.db_path)),
        )],
        Ok(state) => vec![ReleaseReadinessItem::failed(
            "local_identity_backend",
            "本地账号服务端库",
            "本地账号服务端库未初始化。",
            Some(PathBuf::from(state.db_path)),
        )],
        Err(error) => vec![ReleaseReadinessItem::failed(
            "local_identity_backend",
            "本地账号服务端库",
            format!("本地账号服务端库检查失败：{error}"),
            Some(codex_plus_core::local_backend::default_backend_db_path()),
        )],
    }
}

fn release_identity_sync_checks() -> Vec<ReleaseReadinessItem> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let endpoint = settings.jiyi_identity_sync_endpoint.trim();
    let api_key =
        codex_plus_core::secret_store::resolve_secret_value(&settings.jiyi_identity_sync_api_key);
    if endpoint.is_empty() {
        return vec![ReleaseReadinessItem::warning(
            "identity_sync_service",
            "极义账号服务端同步",
            "未配置极义服务端同步 Endpoint；本机可验收，但公开发布前需要远端账号、团队和额度承接服务。",
            Some(codex_plus_core::paths::default_settings_path()),
        )];
    }
    if !endpoint.starts_with("https://") && !endpoint.starts_with("http://") {
        return vec![ReleaseReadinessItem::failed(
            "identity_sync_service",
            "极义账号服务端同步",
            "极义服务端同步 Endpoint 必须是 http:// 或 https:// URL。",
            Some(codex_plus_core::paths::default_settings_path()),
        )];
    }
    if api_key.trim().is_empty() {
        return vec![ReleaseReadinessItem::warning(
            "identity_sync_service",
            "极义账号服务端同步",
            "已配置同步 Endpoint，但未配置同步 API Key；公开发布前需要服务端鉴权。",
            Some(codex_plus_core::paths::default_settings_path()),
        )];
    }
    vec![ReleaseReadinessItem::ok(
        "identity_sync_service",
        "极义账号服务端同步",
        "同步 Endpoint 和 API Key 已配置；请求包会使用脱敏账号、设备、套餐和用量摘要。",
        Some(codex_plus_core::paths::default_settings_path()),
    )]
}

fn release_managed_proxy_checks() -> Vec<ReleaseReadinessItem> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let endpoint = settings.jiyi_managed_proxy_endpoint.trim();
    if !settings.jiyi_managed_proxy_enabled {
        return vec![ReleaseReadinessItem::warning(
            "managed_proxy_service",
            "极义托管代理",
            "未启用极义托管代理；本机可继续用阿里百炼验收，APIMart 仅作为备选，公开发布前应走服务端代理或子 key。",
            Some(codex_plus_core::paths::default_settings_path()),
        )];
    }
    if endpoint.is_empty() {
        return vec![ReleaseReadinessItem::failed(
            "managed_proxy_service",
            "极义托管代理",
            "已启用极义托管代理，但 Endpoint 为空。",
            Some(codex_plus_core::paths::default_settings_path()),
        )];
    }
    if !endpoint.starts_with("https://") && !endpoint.starts_with("http://") {
        return vec![ReleaseReadinessItem::failed(
            "managed_proxy_service",
            "极义托管代理",
            "极义托管代理 Endpoint 必须是 http:// 或 https:// URL。",
            Some(codex_plus_core::paths::default_settings_path()),
        )];
    }
    if codex_plus_core::secret_store::resolve_local_backend_session_token()
        .trim()
        .is_empty()
    {
        return vec![ReleaseReadinessItem::warning(
            "managed_proxy_service",
            "极义托管代理",
            "托管代理 Endpoint 已配置；当前未检测到本机极义后端 session token，用户完成手机号登录并同步后才可请求模型。",
            Some(codex_plus_core::paths::default_settings_path()),
        )];
    }
    vec![ReleaseReadinessItem::ok(
        "managed_proxy_service",
        "极义托管代理",
        "托管代理已启用，模型请求将使用极义后端 session token 转发，不需要在客户端落百炼或中转站主 key。",
        Some(codex_plus_core::paths::default_settings_path()),
    )]
}

fn release_api_key_risk_checks() -> Vec<ReleaseReadinessItem> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let active = settings.active_relay_profile();
    let active_key = codex_plus_core::protocol_proxy::resolved_relay_api_key(&settings, &active);
    let has_plaintext_key =
        codex_plus_core::secret_store::settings_contain_plaintext_api_key(&settings);
    let has_keychain_ref = codex_plus_core::secret_store::settings_contain_keychain_ref(&settings);
    let codex_home = codex_plus_core::relay_config::default_codex_home_dir();
    let codex_home_auth = fs::read_to_string(codex_home.join("auth.json")).unwrap_or_default();
    let codex_home_config = fs::read_to_string(codex_home.join("config.toml")).unwrap_or_default();
    let codex_home_live = format!("{codex_home_auth}\n{codex_home_config}");
    let home_contains_active_key =
        !active_key.trim().is_empty() && codex_home_live.contains(active_key.trim());

    let mut items = Vec::new();
    items.push(if settings.jiyi_managed_proxy_enabled && !has_plaintext_key {
        ReleaseReadinessItem::ok(
            "api_key_distribution",
            "上游 Key 分发风险",
            "已启用极义托管代理，客户端模型请求使用极义后端 session token，不需要分发百炼或中转站主 key。",
            Some(codex_plus_core::paths::default_settings_path()),
        )
    } else if has_plaintext_key {
        ReleaseReadinessItem::warning(
            "api_key_distribution",
            "上游 Key 分发风险",
            if settings.jiyi_managed_proxy_enabled {
                "已启用极义托管代理，但本机设置仍存在明文 API Key；请清空百炼或中转站主 key 后保存设置。"
            } else if settings.jiyi_local_proxy_enabled {
                "本机设置仍存在明文 API Key；请保存一次设置或重新进入 Codex，让极义迁移到 macOS 钥匙串。"
            } else {
                "本机配置存在 API Key 且本地代理未开启。可本机验收，但公开发布前不能把主 key 随包分发。"
            },
            Some(codex_plus_core::paths::default_settings_path()),
        )
    } else if has_keychain_ref {
        ReleaseReadinessItem::ok(
            "api_key_distribution",
            "上游 Key 分发风险",
            "设置文件只保存 macOS 钥匙串引用；公开发布仍需接入服务端子 key 或请求代理。",
            Some(codex_plus_core::paths::default_settings_path()),
        )
    } else {
        ReleaseReadinessItem::ok(
            "api_key_distribution",
            "上游 Key 分发风险",
            "设置文件未保存明文 API Key；公开发布仍需接入服务端子 key 或请求代理。",
            Some(codex_plus_core::paths::default_settings_path()),
        )
    });
    items.push(if home_contains_active_key {
        ReleaseReadinessItem::failed(
            "codex_home_key_isolation",
            "极义 Codex Home Key 隔离",
            "极义隔离 Codex Home 仍含当前真实 API Key；应启用极义本地请求代理并重新进入 Codex。",
            Some(codex_home),
        )
    } else if settings.jiyi_local_proxy_enabled {
        ReleaseReadinessItem::ok(
            "codex_home_key_isolation",
            "极义 Codex Home Key 隔离",
            "极义隔离 Codex Home 未写入当前真实 API Key，本地代理负责转发请求。",
            Some(codex_home),
        )
    } else {
        ReleaseReadinessItem::warning(
            "codex_home_key_isolation",
            "极义 Codex Home Key 隔离",
            "本地代理未开启，极义 Codex Home 可能采用直写 Key 模式；公开分发前不建议使用。",
            Some(codex_home),
        )
    });
    items
}

fn release_notarization_checks() -> Vec<ReleaseReadinessItem> {
    let mut items = Vec::new();
    items.push(release_notarization_script_check());
    let developer_env_ready = env_present("APPLE_ID")
        && env_present("APPLE_APP_SPECIFIC_PASSWORD")
        && env_present("APPLE_TEAM_ID");
    let asc_env_ready =
        env_present("ASC_KEY_ID") && env_present("ASC_ISSUER_ID") && env_present("ASC_KEY_PATH");
    if developer_env_ready || asc_env_ready {
        items.push(ReleaseReadinessItem::ok(
            "notarization_env",
            "macOS 公证环境",
            "检测到 Apple 公证环境变量。",
            None,
        ));
    } else {
        items.push(ReleaseReadinessItem::warning(
            "notarization_env",
            "macOS 公证环境",
            "未检测到 Apple 公证环境变量；当前只能本机 ad-hoc 签名，公开发布前需要 Developer ID 签名和公证。",
            None,
        ));
    }
    items
}

fn release_notarization_script_check() -> ReleaseReadinessItem {
    let script = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.ancestors()
                .find(|ancestor| {
                    ancestor.join("scripts").is_dir() && ancestor.join("Cargo.toml").is_file()
                })
                .map(|root| root.join("scripts/installer/macos/package-dmg.sh"))
        })
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .unwrap_or_else(|| Path::new("."))
                .join("scripts/installer/macos/package-dmg.sh")
        });
    let Ok(content) = fs::read_to_string(&script) else {
        return ReleaseReadinessItem::failed(
            "notarization_packager",
            "正式签名/公证脚本",
            "未找到 macOS DMG 打包脚本，无法确认正式发布链路。",
            Some(script),
        );
    };
    let has_developer_id = content.contains("JIYI_CODESIGN_IDENTITY")
        && content.contains("--options runtime")
        && content.contains("--timestamp");
    let has_notary = content.contains("JIYI_NOTARIZE")
        && content.contains("xcrun notarytool submit")
        && content.contains("xcrun stapler staple");
    if has_developer_id && has_notary {
        ReleaseReadinessItem::ok(
            "notarization_packager",
            "正式签名/公证脚本",
            "打包脚本支持 Developer ID 签名、Hardened Runtime、DMG 签名、notarytool 公证和 stapler 固化。",
            Some(script),
        )
    } else {
        ReleaseReadinessItem::warning(
            "notarization_packager",
            "正式签名/公证脚本",
            "打包脚本尚未完整声明 Developer ID 签名和 Apple 公证流程。",
            Some(script),
        )
    }
}

fn bundle_id_check(id: &str, label: &str, app: &Path, expected: &str) -> ReleaseReadinessItem {
    let plist = app.join("Contents").join("Info.plist");
    match plist_value(&plist, "CFBundleIdentifier") {
        Some(actual) if actual == expected => ReleaseReadinessItem::ok(
            id,
            label,
            format!("bundle id 为 {expected}。"),
            Some(app.to_path_buf()),
        ),
        Some(actual) => ReleaseReadinessItem::failed(
            id,
            label,
            format!("bundle id 为 {actual}，预期 {expected}。"),
            Some(app.to_path_buf()),
        ),
        None => {
            ReleaseReadinessItem::failed(id, label, "未能读取 bundle id。", Some(app.to_path_buf()))
        }
    }
}

fn codesign_verify_check(id: &str, label: &str, app: &Path) -> ReleaseReadinessItem {
    let output = std::process::Command::new("codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=1"])
        .arg(app)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            ReleaseReadinessItem::ok(id, label, "codesign 校验通过。", Some(app.to_path_buf()))
        }
        Ok(output) => ReleaseReadinessItem::failed(
            id,
            label,
            format!(
                "codesign 校验失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Some(app.to_path_buf()),
        ),
        Err(error) => ReleaseReadinessItem::failed(
            id,
            label,
            format!("无法运行 codesign：{error}"),
            Some(app.to_path_buf()),
        ),
    }
}

fn developer_id_check(app: &Path) -> ReleaseReadinessItem {
    let output = std::process::Command::new("codesign")
        .args(["-dv", "--verbose=4"])
        .arg(app)
        .output();
    let stderr = output
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stderr).to_string())
        .unwrap_or_default();
    if stderr.contains("Authority=Developer ID Application") {
        ReleaseReadinessItem::ok(
            "developer_id_signature",
            "Developer ID 签名",
            "已检测到 Developer ID Application 签名。",
            Some(app.to_path_buf()),
        )
    } else {
        ReleaseReadinessItem::warning(
            "developer_id_signature",
            "Developer ID 签名",
            "未检测到 Developer ID Application；当前签名适合本机验收，不适合公开分发。",
            Some(app.to_path_buf()),
        )
    }
}

fn plist_value(plist: &Path, key: &str) -> Option<String> {
    let output = std::process::Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(format!("Print :{key}"))
        .arg(plist)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn set_plist_value(plist: &Path, key: &str, value: &str) -> anyhow::Result<()> {
    let set_command = format!("Set :{key} {value}");
    let status = std::process::Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(&set_command)
        .arg(plist)
        .status()?;
    if status.success() {
        return Ok(());
    }
    let add_command = format!("Add :{key} string {value}");
    let add_status = std::process::Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(&add_command)
        .arg(plist)
        .status()?;
    if add_status.success() {
        Ok(())
    } else {
        anyhow::bail!("更新内置 Codex 客户端身份失败：{set_command}");
    }
}

fn plist_key_exists(plist: &Path, key: &str) -> bool {
    std::process::Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(format!("Print :{key}"))
        .arg(plist)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn delete_plist_key_if_present(plist: &Path, key: &str) -> anyhow::Result<bool> {
    if !plist_key_exists(plist, key) {
        return Ok(false);
    }
    let status = std::process::Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(format!("Delete :{key}"))
        .arg(plist)
        .status()?;
    if status.success() {
        Ok(true)
    } else {
        anyhow::bail!("删除内置 Codex 客户端 plist 键失败：{key}");
    }
}

fn find_release_dmg_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("JIYI_CODEX_DMG_PATH")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
    {
        return Some(path);
    }
    let file_names = release_dmg_file_names();
    let mut dir = std::env::current_dir().ok();
    while let Some(current) = dir {
        for file_name in &file_names {
            let candidate = current.join("dist").join("macos").join(file_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        dir = current.parent().map(Path::to_path_buf);
    }
    None
}

fn release_dmg_file_names() -> Vec<String> {
    let arch = std::env::consts::ARCH;
    let labels: &[&str] = match arch {
        "aarch64" => &["arm64", "aarch64"],
        "x86_64" => &["x64", "x86_64"],
        other => &[other],
    };
    labels
        .iter()
        .map(|label| {
            format!(
                "JiyiCodex-{}-macos-{label}.dmg",
                codex_plus_core::version::VERSION
            )
        })
        .collect()
}

fn original_codex_file_has_jiyi_contamination(contents: &str) -> bool {
    contents.contains(".codex-session-delete")
        || contents.contains("JiyiCodex")
        || contents.contains("Jiyi")
        || contents.contains("极义codex")
        || contents.contains("dashscope.aliyuncs.com")
        || contents.contains("aliyuncs.com/compatible-mode")
        || contents.contains("DASHSCOPE_API_KEY")
        || contents.contains("BAILIAN_API_KEY")
        || contents.contains("ALIYUN_BAILIAN_API_KEY")
        || contents.contains("QWEN_API_KEY")
        || contents.contains("apimart.ai")
        || contents.contains("api.apimart.ai")
        || contents.contains("qwen3.7-plus")
        || contents.contains("gpt-5.5")
        || contents.contains("jiyi-local-proxy")
        || contents.contains("jiyi-keychain:")
}

fn official_codex_file_has_jiyi_contamination(path: &Path) -> bool {
    fs::read(path)
        .map(|bytes| original_codex_file_has_jiyi_contamination(&String::from_utf8_lossy(&bytes)))
        .unwrap_or(false)
}

fn official_codex_app_support_paths() -> Vec<PathBuf> {
    let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) else {
        return Vec::new();
    };
    vec![
        home.join("Library")
            .join("Application Support")
            .join("Codex"),
        home.join("Library")
            .join("Application Support")
            .join("com.openai.codex"),
    ]
}

fn official_codex_isolation_candidate_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let home = codex_plus_core::paths::default_official_codex_home_dir();
    files.push(home.join("config.toml"));
    files.push(home.join("auth.json"));

    for root in official_codex_app_support_paths() {
        for relative in [
            "Preferences",
            "Local State",
            "Network Persistent State",
            "Reporting and NEL",
            "Default/Preferences",
            "Default/Network Persistent State",
            "Default/Reporting and NEL",
        ] {
            files.push(root.join(relative));
        }
    }

    files.into_iter().filter(|path| path.is_file()).collect()
}

fn repair_official_codex_isolation_payload() -> anyhow::Result<OfficialCodexIsolationRepairPayload>
{
    let official_home = codex_plus_core::paths::default_official_codex_home_dir();
    let app_support_paths = official_codex_app_support_paths();
    let candidates = official_codex_isolation_candidate_files();
    let backup_dir = codex_plus_core::paths::default_app_state_dir()
        .join("original-codex-isolation-backups")
        .join(now_ms().to_string());

    let mut scanned_files = Vec::new();
    let mut repaired_files = Vec::new();
    let mut remaining_contaminated_files = Vec::new();
    let mut backup_created = false;

    for path in candidates {
        scanned_files.push(path.to_string_lossy().to_string());
        if !official_codex_file_has_jiyi_contamination(&path) {
            continue;
        }
        if !backup_created {
            fs::create_dir_all(&backup_dir)?;
            backup_created = true;
        }
        backup_official_codex_file(&path, &backup_dir)?;
        if repair_official_codex_file(&path).is_ok()
            && !official_codex_file_has_jiyi_contamination(&path)
        {
            repaired_files.push(path.to_string_lossy().to_string());
        } else {
            remaining_contaminated_files.push(path.to_string_lossy().to_string());
        }
    }

    Ok(OfficialCodexIsolationRepairPayload {
        official_home: official_home.to_string_lossy().to_string(),
        app_support_paths: app_support_paths
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        backup_dir: backup_created.then(|| backup_dir.to_string_lossy().to_string()),
        scanned_files,
        repaired_files,
        remaining_contaminated_files,
    })
}

fn backup_official_codex_file(path: &Path, backup_dir: &Path) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(backup_dir)?;
    let backup_path = backup_dir.join(safe_backup_file_name(path));
    fs::copy(path, &backup_path).with_context(|| {
        format!(
            "failed to back up {} to {}",
            path.display(),
            backup_path.display()
        )
    })?;
    Ok(backup_path)
}

fn safe_backup_file_name(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn repair_official_codex_file(path: &Path) -> anyhow::Result<()> {
    if is_ephemeral_official_codex_state_file(path) {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove cached state {}", path.display()))?;
        return Ok(());
    }

    let bytes = fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let sanitized = if looks_like_json_path(path) {
        sanitize_json_text_for_official_codex(&text)
            .unwrap_or_else(|| sanitize_text_for_official_codex(path, &text))
    } else {
        sanitize_text_for_official_codex(path, &text)
    };
    fs::write(path, sanitized)
        .with_context(|| format!("failed to write sanitized file {}", path.display()))
}

fn is_ephemeral_official_codex_state_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "Network Persistent State" | "Reporting and NEL"))
}

fn looks_like_json_path(path: &Path) -> bool {
    path.extension().and_then(|value| value.to_str()) == Some("json")
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| matches!(name, "Preferences" | "Local State" | "auth.json"))
}

fn sanitize_json_text_for_official_codex(text: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(text).ok()?;
    sanitize_json_value_for_official_codex(&mut value);
    serde_json::to_string_pretty(&value).ok().map(|mut output| {
        output.push('\n');
        output
    })
}

fn sanitize_json_value_for_official_codex(value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            let mut changed = false;
            for key in keys {
                let remove = original_codex_file_has_jiyi_contamination(&key)
                    || map.get(&key).is_some_and(json_value_is_direct_jiyi_scalar);
                if remove {
                    map.remove(&key);
                    changed = true;
                } else if let Some(nested) = map.get_mut(&key) {
                    changed |= sanitize_json_value_for_official_codex(nested);
                }
            }
            changed
        }
        Value::Array(items) => {
            let before = items.len();
            items.retain(|item| !json_value_is_direct_jiyi_scalar(item));
            let mut changed = items.len() != before;
            for item in items {
                changed |= sanitize_json_value_for_official_codex(item);
            }
            changed
        }
        Value::String(text) => {
            if original_codex_file_has_jiyi_contamination(text) {
                text.clear();
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn json_value_is_direct_jiyi_scalar(value: &Value) -> bool {
    match value {
        Value::String(text) => original_codex_file_has_jiyi_contamination(text),
        Value::Number(_) | Value::Bool(_) | Value::Null => false,
        Value::Array(_) | Value::Object(_) => false,
    }
}

fn sanitize_text_for_official_codex(path: &Path, text: &str) -> String {
    if path.extension().and_then(|value| value.to_str()) == Some("toml") {
        return sanitize_toml_text_for_official_codex(text);
    }
    let mut output = text
        .lines()
        .filter(|line| !original_codex_file_has_jiyi_contamination(line))
        .collect::<Vec<_>>()
        .join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    output
}

fn sanitize_toml_text_for_official_codex(text: &str) -> String {
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with('[') || blocks.is_empty() {
            blocks.push(Vec::new());
        }
        if let Some(block) = blocks.last_mut() {
            block.push(line);
        }
    }

    let removed_model_provider_section = blocks.iter().any(|block| {
        let contaminated = block
            .iter()
            .any(|line| original_codex_file_has_jiyi_contamination(line));
        let header = block.first().map(|line| line.trim()).unwrap_or_default();
        contaminated && header.starts_with("[model_providers.")
    });
    let mut kept_blocks: Vec<String> = Vec::new();
    for block in blocks {
        let contaminated = block
            .iter()
            .any(|line| original_codex_file_has_jiyi_contamination(line));
        let header = block
            .first()
            .map(|line| line.trim())
            .unwrap_or_default()
            .to_string();
        if contaminated && header.starts_with("[model_providers.") {
            continue;
        }
        let lines = block
            .into_iter()
            .filter(|line| {
                !original_codex_file_has_jiyi_contamination(line)
                    && !(removed_model_provider_section
                        && line.trim_start().starts_with("model_provider"))
            })
            .collect::<Vec<_>>();
        if !lines.is_empty() {
            kept_blocks.push(lines.join("\n"));
        }
    }

    let mut output = kept_blocks.join("\n\n");
    if !output.is_empty() {
        output.push('\n');
    }
    output
}

fn env_present(name: &str) -> bool {
    std::env::var(name)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

impl ReleaseReadinessItem {
    fn ok(
        id: impl Into<String>,
        label: impl Into<String>,
        message: impl Into<String>,
        path: Option<PathBuf>,
    ) -> Self {
        Self::new(id, label, "ok", message, path)
    }

    fn warning(
        id: impl Into<String>,
        label: impl Into<String>,
        message: impl Into<String>,
        path: Option<PathBuf>,
    ) -> Self {
        Self::new(id, label, "warning", message, path)
    }

    fn failed(
        id: impl Into<String>,
        label: impl Into<String>,
        message: impl Into<String>,
        path: Option<PathBuf>,
    ) -> Self {
        Self::new(id, label, "failed", message, path)
    }

    fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        status: impl Into<String>,
        message: impl Into<String>,
        path: Option<PathBuf>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            status: status.into(),
            message: message.into(),
            path: path.map(|path| path.to_string_lossy().to_string()),
        }
    }
}

fn read_tail(path: &Path, max_lines: usize) -> std::io::Result<String> {
    let contents = fs::read_to_string(path)?;
    let mut lines = contents.lines().rev().take(max_lines).collect::<Vec<_>>();
    lines.reverse();
    Ok(lines.join("\n"))
}

fn path_state(path: Option<PathBuf>) -> PathState {
    match path {
        Some(path) => PathState {
            status: "found".to_string(),
            path: Some(path.to_string_lossy().to_string()),
        },
        None => PathState {
            status: "missing".to_string(),
            path: None,
        },
    }
}

fn shortcut_state(shortcut: install::ShortcutState) -> PathState {
    PathState {
        status: if shortcut.installed {
            "installed".to_string()
        } else {
            "missing".to_string()
        },
        path: shortcut.path,
    }
}

fn ok<T: Serialize>(message: &str, payload: T) -> CommandResult<T> {
    CommandResult {
        status: "ok".to_string(),
        message: message.to_string(),
        payload,
    }
}

fn failed<T: Serialize>(message: &str, payload: T) -> CommandResult<T> {
    CommandResult {
        status: "failed".to_string(),
        message: message.to_string(),
        payload,
    }
}

fn default_debug_port() -> u16 {
    9229
}

fn default_helper_port() -> u16 {
    57321
}

fn default_log_lines() -> usize {
    200
}

fn default_admin_console_limit() -> usize {
    50
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_version_returns_structured_payload() {
        let result = backend_version();

        assert_eq!(result.status, "ok");
        assert!(!result.payload.version.is_empty());
    }

    #[test]
    fn startup_options_returns_structured_payload() {
        let result = startup_options();

        assert_eq!(result.status, "ok");
    }

    #[test]
    fn startup_options_honors_show_update_environment() {
        unsafe {
            std::env::set_var("CODEX_PLUS_SHOW_UPDATE", "1");
        }

        let result = startup_options();

        unsafe {
            std::env::remove_var("CODEX_PLUS_SHOW_UPDATE");
        }

        assert_eq!(result.status, "ok");
        assert!(result.payload.show_update);
    }

    #[test]
    fn startup_options_honors_show_update_argument() {
        assert!(should_show_update(
            ["codex-plus-plus-manager.exe", "--show-update"],
            None
        ));
    }

    #[test]
    fn overview_contains_expected_operational_fields() {
        let result = tauri::async_runtime::block_on(load_overview());

        assert_eq!(result.status, "ok");
        assert!(!result.payload.current_version.is_empty());
        assert!(
            result.payload.codex_version.is_none()
                || result
                    .payload
                    .codex_version
                    .as_deref()
                    .is_some_and(|version| !version.is_empty())
        );
        assert!(matches!(
            result.payload.codex_app.status.as_str(),
            "found" | "missing"
        ));
        assert!(matches!(
            result.payload.silent_shortcut.status.as_str(),
            "installed" | "missing"
        ));
    }

    #[test]
    fn update_install_requires_release_payload() {
        let result = tauri::async_runtime::block_on(perform_update(None));

        assert_eq!(result.status, "failed");
        assert!(result.message.contains("请先检查更新"));
    }

    #[test]
    fn watcher_state_returns_disabled_flag_path() {
        let result = load_watcher_state();

        assert_eq!(result.status, "ok");
        assert!(result.payload.disabled_flag.contains("watcher.disabled"));
    }

    #[test]
    fn release_contamination_detector_flags_jiyi_values() {
        assert!(original_codex_file_has_jiyi_contamination(
            r#"notify = ["/Users/lv/.codex-session-delete/codex-home/tool"]"#
        ));
        assert!(original_codex_file_has_jiyi_contamination(
            r#"base_url = "https://api.apimart.ai/v1""#
        ));
        assert!(!original_codex_file_has_jiyi_contamination(
            r#"notify = ["/Users/lv/.codex/computer-use/tool"]"#
        ));
        assert!(!original_codex_file_has_jiyi_contamination(
            r#"{"OPENAI_API_KEY":"sk-official-user-key"}"#
        ));
    }

    #[test]
    fn official_codex_json_sanitizer_removes_only_jiyi_scalars() {
        let sanitized = sanitize_json_text_for_official_codex(
            r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":"jiyi-local-proxy","tokens":{"id_token":"official-token"},"recent":["https://api.apimart.ai/v1","https://chatgpt.com"]}"#,
        )
        .unwrap();

        assert!(sanitized.contains("chatgpt"));
        assert!(sanitized.contains("official-token"));
        assert!(sanitized.contains("https://chatgpt.com"));
        assert!(!sanitized.contains("jiyi-local-proxy"));
        assert!(!sanitized.contains("api.apimart.ai"));
    }

    #[test]
    fn official_codex_toml_sanitizer_removes_contaminated_provider_block() {
        let sanitized = sanitize_toml_text_for_official_codex(
            r#"model_provider = "custom"
personality = "pragmatic"

[model_providers.custom]
name = "APIMart"
base_url = "https://api.apimart.ai/v1"

[projects."/Users/lv/Documents/codex二开"]
trust_level = "trusted"
"#,
        );

        assert!(sanitized.contains("personality = \"pragmatic\""));
        assert!(sanitized.contains("[projects.\"/Users/lv/Documents/codex二开\"]"));
        assert!(!sanitized.contains("model_provider = \"custom\""));
        assert!(!sanitized.contains("[model_providers.custom]"));
        assert!(!sanitized.contains("api.apimart.ai"));
    }

    #[test]
    fn release_readiness_item_builders_redact_paths_only() {
        let item = ReleaseReadinessItem::warning(
            "api_key_distribution",
            "上游 Key 分发风险",
            "本机配置存在 API Key。",
            Some(PathBuf::from("/tmp/settings.json")),
        );

        assert_eq!(item.status, "warning");
        assert_eq!(item.path.as_deref(), Some("/tmp/settings.json"));
        assert!(!item.message.contains("sk-"));
    }

    #[test]
    fn managed_proxy_endpoint_parser_accepts_loopback_only() {
        assert_eq!(
            managed_proxy_loopback_listen_addr_from_endpoint("http://127.0.0.1:57421/v1")
                .as_deref(),
            Some("127.0.0.1:57421")
        );
        assert_eq!(
            managed_proxy_loopback_listen_addr_from_endpoint("http://localhost:57422").as_deref(),
            Some("127.0.0.1:57422")
        );
        assert!(
            managed_proxy_loopback_listen_addr_from_endpoint("https://api.example.com/v1")
                .is_none()
        );
    }

    #[test]
    fn managed_proxy_upstream_does_not_point_to_local_endpoint() {
        let settings = BackendSettings {
            jiyi_managed_proxy_endpoint: "http://127.0.0.1:57421".to_string(),
            relay_base_url: "http://127.0.0.1:57421".to_string(),
            relay_profiles: vec![RelayProfile {
                base_url: "http://127.0.0.1:57421".to_string(),
                upstream_base_url: "http://127.0.0.1:57421".to_string(),
                ..RelayProfile::default()
            }],
            ..BackendSettings::default()
        };

        assert_eq!(
            managed_proxy_upstream_base_url(&settings, "http://127.0.0.1:57421"),
            codex_plus_core::managed_proxy::DEFAULT_MANAGED_PROXY_UPSTREAM_BASE_URL
        );
    }

    #[test]
    fn identity_sync_response_preview_redacts_api_key() {
        let preview = safe_response_preview(
            r#"{"error":"bad token sync-secret","detail":"sync-secret"}"#,
            "sync-secret",
        );

        assert!(preview.contains("<redacted>"));
        assert!(!preview.contains("sync-secret"));
    }

    #[test]
    fn identity_sync_response_extracts_remote_backend_session_token() {
        let token = remote_backend_session_token_from_response(
            r#"{"status":"ok","activeSession":{"userId":"user-1","deviceId":"device-1","accessToken":"jiyi-local-remote-token"}}"#,
        );

        assert_eq!(token.as_deref(), Some("jiyi-local-remote-token"));
        assert!(remote_backend_session_token_from_response(r#"{"status":"ok"}"#).is_none());
    }

    #[test]
    fn release_readiness_payload_contains_core_gates() {
        let payload = release_readiness_payload();
        let ids = payload
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();

        assert!(ids.contains(&"official_codex_isolation"));
        assert!(ids.contains(&"dmg"));
        assert!(ids.contains(&"embedded_client_no_official_fallback"));
        assert!(ids.contains(&"embedded_client_user_data_isolation"));
        assert!(ids.contains(&"embedded_client_environment_isolation"));
        assert!(ids.contains(&"embedded_client_url_scheme_isolation"));
        assert!(ids.contains(&"managed_proxy_sidecar"));
        assert!(ids.contains(&"managed_proxy_launchd_deploy"));
        assert!(ids.contains(&"managed_proxy_remote_deploy"));
        assert!(ids.contains(&"tencent_sms_provider"));
        assert!(ids.contains(&"local_entitlement_model"));
        assert!(ids.contains(&"local_usage_meter"));
        assert!(ids.contains(&"local_identity_backend"));
        assert!(ids.contains(&"identity_sync_service"));
        assert!(ids.contains(&"managed_proxy_service"));
        assert!(ids.contains(&"api_key_distribution"));
        assert!(ids.contains(&"codex_home_key_isolation"));
        assert!(ids.contains(&"notarization_packager"));
        assert!(ids.contains(&"notarization_env"));
    }

    #[test]
    fn embedded_client_resolution_does_not_fallback_to_official_codex() {
        let temp = tempfile::tempdir().expect("tempdir");
        let contents_dir = temp.path().join("极义codex.app").join("Contents");

        let error = embedded_codex_app_path_from_contents_dir(&contents_dir)
            .expect_err("missing embedded client should fail");

        let message = error.to_string();
        assert!(message.contains("不会使用 /Applications/Codex.app 兜底"));
    }

    #[test]
    fn release_dmg_file_names_include_packaging_aliases() {
        let names = release_dmg_file_names();

        if std::env::consts::ARCH == "aarch64" {
            assert!(names.iter().any(|name| name.ends_with("-macos-arm64.dmg")));
        }
        assert!(names.iter().all(|name| name.starts_with("JiyiCodex-")));
    }

    #[test]
    fn missing_logs_return_failed_status() {
        let result = read_latest_logs(LogRequest { lines: 25 });

        if result.payload.text.is_empty() {
            assert_eq!(result.status, "failed");
        }
    }

    #[test]
    fn relay_payload_does_not_expose_token_text() {
        let payload = relay_payload(
            codex_plus_core::relay_config::RelayStatus {
                authenticated: true,
                auth_source: "registry.json".to_string(),
                account_label: Some("user@example.test".to_string()),
                config_path: "config.toml".to_string(),
                configured: true,
                requires_openai_auth: true,
                has_bearer_token: true,
            },
            None,
        );
        let text = serde_json::to_string(&payload).unwrap();

        assert!(!text.contains("sk-"));
        assert!(text.contains("hasBearerToken"));
    }

    #[test]
    fn relay_files_payload_reads_config_and_auth_contents() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"custom\"\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("auth.json"),
            "{\"OPENAI_API_KEY\":\"sk-test\"}\n",
        )
        .unwrap();

        let payload = relay_files_payload_from_home(temp.path()).unwrap();

        assert!(payload.config_path.ends_with("config.toml"));
        assert!(payload.auth_path.ends_with("auth.json"));
        assert_eq!(payload.config_contents, "model_provider = \"custom\"\n");
        assert_eq!(payload.auth_contents, "{\"OPENAI_API_KEY\":\"sk-test\"}\n");
    }

    #[test]
    fn apply_relay_profile_to_home_with_switch_rules_preserves_custom_provider_id() {
        let temp = tempfile::tempdir().unwrap();
        let profile = RelayProfile {
            relay_mode: codex_plus_core::settings::RelayMode::PureApi,
            protocol: codex_plus_core::settings::RelayProtocol::Responses,
            config_contents: "model_provider = \"ai\"\nmodel = \"gpt-image-2\"\n\n[model_providers.ai]\nname = \"ai\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"https://ahg.codes\"\n"
                .to_string(),
            auth_contents: "{}\n".to_string(),
            ..RelayProfile::default()
        };

        codex_plus_core::relay_config::apply_relay_profile_to_home_with_switch_rules(
            temp.path(),
            &profile,
            "",
        )
        .unwrap();

        let applied = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
        assert!(applied.contains("model_provider = \"ai\""));
        assert!(applied.contains("[model_providers.ai]"));
        assert!(!applied.contains("[model_providers.custom]"));
    }

    #[test]
    fn save_relay_file_in_home_only_allows_known_files() {
        let temp = tempfile::tempdir().unwrap();

        save_relay_file_in_home(temp.path(), "config", "model = \"gpt-5\"\n").unwrap();
        save_relay_file_in_home(temp.path(), "auth", "{}\n").unwrap();

        assert_eq!(
            std::fs::read_to_string(temp.path().join("config.toml")).unwrap(),
            "model = \"gpt-5\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join("auth.json")).unwrap(),
            "{}\n"
        );
        assert!(save_relay_file_in_home(temp.path(), "../bad", "").is_err());
    }

    #[test]
    fn normalize_settings_before_save_preserves_profile_context_until_manual_extract() {
        let settings = BackendSettings {
            relay_common_config_contents: "[mcp_servers.context7]\ncommand = \"npx\"\n".to_string(),
            relay_profiles: vec![RelayProfile {
                use_common_config: false,
                relay_mode: codex_plus_core::settings::RelayMode::PureApi,
                config_contents: "model = \"gpt-5\"\n\n[mcp_servers.context7]\ncommand = \"npx\"\n"
                    .to_string(),
                ..RelayProfile::default()
            }],
            ..BackendSettings::default()
        };

        let normalized = normalize_settings_before_save(settings);

        assert!(
            normalized.relay_profiles[0]
                .config_contents
                .contains("model = \"gpt-5\"")
        );
        assert!(
            normalized.relay_profiles[0]
                .config_contents
                .contains("[mcp_servers.context7]")
        );
        assert!(
            normalized
                .relay_context_config_contents
                .contains("[mcp_servers.context7]")
        );
        assert!(
            !normalized
                .relay_common_config_contents
                .contains("[mcp_servers")
        );
    }

    #[test]
    fn normalize_settings_before_save_preserves_official_profile_auth() {
        let settings = BackendSettings {
            relay_profiles: vec![RelayProfile {
                relay_mode: codex_plus_core::settings::RelayMode::Official,
                official_mix_api_key: false,
                auth_contents: r#"{"auth_mode":"chatgpt","tokens":{"access_token":"edited"}}"#
                    .to_string(),
                config_contents: "model_provider = \"custom\"\n".to_string(),
                ..RelayProfile::default()
            }],
            ..BackendSettings::default()
        };

        let normalized = normalize_settings_before_save(settings);

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&normalized.relay_profiles[0].auth_contents)
                .unwrap(),
            serde_json::json!({"auth_mode":"chatgpt","tokens":{"access_token":"edited"}})
        );
        assert!(normalized.relay_profiles[0].config_contents.is_empty());
    }

    #[test]
    fn remove_linked_ccs_profiles_for_local_storage_drops_external_profiles() {
        let mut settings = BackendSettings {
            ccs_link_enabled: true,
            active_relay_id: "ccs-one".to_string(),
            relay_profiles: vec![
                RelayProfile {
                    id: "local".to_string(),
                    name: "Local".to_string(),
                    ..RelayProfile::default()
                },
                RelayProfile {
                    id: "ccs-one".to_string(),
                    linked_ccs_provider_id: "provider-one".to_string(),
                    name: "External".to_string(),
                    ..RelayProfile::default()
                },
            ],
            ..BackendSettings::default()
        };

        remove_linked_ccs_profiles_for_local_storage(&mut settings);

        assert_eq!(settings.relay_profiles.len(), 1);
        assert_eq!(settings.relay_profiles[0].id, "local");
        assert_eq!(settings.active_relay_id, "ccs-one");
    }

    #[test]
    fn normalize_settings_before_save_strips_common_from_enabled_profile() {
        let settings = BackendSettings {
            relay_common_config_contents: r#"model_reasoning_effort = "high"

[features]
goals = true

[plugins."superpowers@openai-curated"]
enabled = true
"#
            .to_string(),
            relay_profiles: vec![RelayProfile {
                use_common_config: true,
                relay_mode: codex_plus_core::settings::RelayMode::PureApi,
                config_contents: r#"model = "gpt-5"
model_reasoning_effort = "high"

[features]
goals = true
model_reasoning_effort = "high"

[plugins."superpowers@openai-curated"]
enabled = true
"#
                .to_string(),
                ..RelayProfile::default()
            }],
            ..BackendSettings::default()
        };

        let normalized = normalize_settings_before_save(settings);
        let config = &normalized.relay_profiles[0].config_contents;

        assert!(config.contains("model = \"gpt-5\""));
        assert!(!config.contains("model_reasoning_effort"));
        assert!(!config.contains("[features]"));
        assert!(!config.contains("[plugins.\"superpowers@openai-curated\"]"));
    }

    #[test]
    fn normalize_settings_before_save_repairs_invalid_profile_common_duplication() {
        let settings = BackendSettings {
            relay_common_config_contents: r#"model_reasoning_effort = "high"

[marketplaces.openai-bundled]
last_updated = "2026-05-25T11:52:46Z"
"#
            .to_string(),
            relay_profiles: vec![RelayProfile {
                use_common_config: true,
                relay_mode: codex_plus_core::settings::RelayMode::PureApi,
                config_contents: r#"model = "gpt-5"
model_reasoning_effort = "high"

[marketplaces.openai-bundled]
last_updated = "2026-05-25T11:52:46Z"

[marketplaces.openai-bundled]
last_updated = "2026-05-25T11:52:46Z"
"#
                .to_string(),
                ..RelayProfile::default()
            }],
            ..BackendSettings::default()
        };

        let normalized = normalize_settings_before_save(settings);
        let config = &normalized.relay_profiles[0].config_contents;

        assert!(config.contains("model = \"gpt-5\""));
        assert!(!config.contains("model_reasoning_effort"));
        assert!(!config.contains("[marketplaces.openai-bundled]"));
    }

    #[test]
    fn normalize_settings_before_save_removes_model_catalog_from_common_config() {
        let settings = BackendSettings {
            relay_common_config_contents: r#"model_catalog_json = "C:\\Users\\Administrator\\.codex\\model-catalogs\\relay-a.json"
model_catalog_json = 'C:\Users\Administrator\.codex\model-catalogs\relay-b.json'
model_reasoning_effort = "high"
"#
            .to_string(),
            ..BackendSettings::default()
        };

        let normalized = normalize_settings_before_save(settings);

        assert!(
            !normalized
                .relay_common_config_contents
                .contains("model_catalog_json")
        );
        assert!(
            normalized
                .relay_common_config_contents
                .contains("model_reasoning_effort = \"high\"")
        );
    }

    #[test]
    fn context_entry_commands_update_settings_payload() {
        let settings = BackendSettings::default();
        let upsert = upsert_context_entry(ContextEntryRequest {
            settings: settings.clone(),
            kind: "mcp".to_string(),
            id: "context7".to_string(),
            toml_body: "command = \"npx\"\n".to_string(),
        });

        assert_eq!(upsert.status, "ok");
        assert!(
            upsert
                .payload
                .settings
                .relay_context_config_contents
                .contains("[mcp_servers.context7]")
        );

        let listed = list_context_entries(ContextSettingsRequest {
            settings: upsert.payload.settings.clone(),
        });
        assert_eq!(listed.payload.entries.mcp_servers[0].id, "context7");

        let deleted = delete_context_entry(ContextDeleteRequest {
            settings: upsert.payload.settings,
            kind: "mcp".to_string(),
            id: "context7".to_string(),
        });
        assert_eq!(deleted.status, "ok");
        assert!(
            !deleted
                .payload
                .settings
                .relay_context_config_contents
                .contains("[mcp_servers.context7]")
        );
    }

    #[test]
    fn ads_payload_keeps_version_and_ad_items() {
        let payload = ads_payload(json!({
            "version": 1,
            "ads": [{"id": "ad-1", "type": "normal", "title": "Ad"}]
        }));

        assert_eq!(payload.version, 1);
        assert_eq!(payload.ads.len(), 1);
        assert_eq!(payload.ads[0]["id"], json!("ad-1"));
    }

    #[test]
    fn open_external_url_rejects_non_http_urls() {
        let result = open_external_url("file:///C:/Windows/win.ini".to_string());

        assert_eq!(result.status, "failed");
        assert!(result.message.contains("只允许打开 http 或 https 链接"));
    }
}
