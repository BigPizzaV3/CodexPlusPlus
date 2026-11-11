use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde_json::{Map, Value, json};

const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const GLOBAL_STATE_BACKUP_FILE: &str = ".codex-global-state.json.bak";
const BACKUP_ROOT: &str = "backups_state/app-state-sync";
const SNAPSHOT_FILE: &str = "latest-safe-state.json";
const SNAPSHOT_VERSION: u64 = 1;
const ACCOUNT_RENDERER_STATE_KEY: &str = "electron-account-renderer-state-v1";
/// 非官方登录时被挪走的账号隔离容器（见 [`reconcile_account_scope`]）。
const DETACHED_ACCOUNT_STATE_FILE: &str = "backups_state/account-scope/detached-renderer-state.json";
const DETACHED_ACCOUNT_STATE_VERSION: u64 = 1;
/// 合并旧账号项目时不带过来的键：排队中的追问搬到别的身份下可能被重放。
const ACCOUNT_SCOPE_MERGE_SKIP_KEYS: &[&str] = &["queued-follow-ups"];

const WORKSPACE_PATH_ARRAY_KEYS: &[&str] = &["electron-saved-workspace-roots", "project-order"];

const ACTIVE_WORKSPACE_ROOTS_KEY: &str = "active-workspace-roots";

const WORKSPACE_PATH_MAP_KEYS: &[&str] = &["electron-workspace-root-labels"];

const THREAD_STATE_MAP_KEYS: &[&str] = &[
    "thread-workspace-root-hints",
    "thread-projectless-output-directories",
    "thread-writable-roots",
];

const THREAD_ID_ARRAY_KEYS: &[&str] = &["projectless-thread-ids"];

const SAFE_TOP_LEVEL_KEYS: &[&str] = &[
    "electron-avatar-overlay-bounds",
    "electron-avatar-overlay-open",
    "electron-main-window-bounds",
];

/// 新版 Codex（26.10xx+）的项目列表/会话归属键：结构由 Codex 拥有，
/// Codex++ 不解析、不归一，但快照和 merge 时要原样带过——
/// 否则半截状态（Codex 写入中途被读到）再 merge 写回，
/// 这些键就永远从 state 里消失，对应 #2876 / #2707 的「项目消失」。
const PASS_THROUGH_STATE_KEYS: &[&str] = &[
    "local-projects",
    "selected-project",
    "app-server-project-id-by-legacy-project-id-by-host",
    "app-server-projects-migration-by-host",
    "project-appearances",
    "project-order",
    "sidebar-project-thread-orders",
    "thread-project-assignments",
    "thread-project-membership-host-ids",
    "remote-project-connection-backfill-completed",
];

/// Codex 输入框模型选择器的「最近使用」（持久化在 atom state 里，只留最近 3 条）。
const COMPOSER_RECENT_MODELS_KEY: &str = "composer-recent-model-configurations-v1";

const SAFE_ATOM_KEYS: &[&str] = &[
    "default-service-tier",
    "flat-project-sidebar-preferences-v1",
    "unified-sidebar-project-order-v1",
    "unified-sidebar-pinned-order-v1",
    "avatar-overlay-mascot-width-px",
    "composer-auto-context-enabled",
    "diff-filter",
    "enter-behavior",
    "first-awake-pet-notification-avatar-ids",
    "has-seen-codex-mobile-announcement",
    "has-seen-multi-agent-composer-banner",
    "has-user-changed-service-tier",
    "last_completed_onboarding",
    "preferred-non-full-access-agent-mode-by-host-id",
    "seen-model-upgrade-list",
    "sidebar-collapsed-groups",
    "sidebar-collapsed-sections-v1",
    "sidebar-width",
    "thread-summary-panel-section-expanded-progress",
    "unread-thread-ids-by-host-v1",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppStateSyncResult {
    pub changed: bool,
    pub changed_keys: Vec<String>,
    pub backup_path: Option<PathBuf>,
    pub snapshot_path: Option<PathBuf>,
}

pub fn capture_app_state_snapshot(home: &Path) -> anyhow::Result<Option<PathBuf>> {
    let Some(state) = load_global_state(home)? else {
        return Ok(None);
    };
    if state.contains_key(ACCOUNT_RENDERER_STATE_KEY) && account_renderer_state(&state).is_none() {
        anyhow::bail!("unrecognized account renderer state; refusing to capture");
    }
    let snapshot = safe_snapshot_from_state(&state);
    let snapshot_state = snapshot
        .get("state")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if snapshot_state.is_empty() {
        return Ok(None);
    }
    let path = snapshot_path(home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::settings::atomic_write(&path, serde_json::to_string_pretty(&snapshot)?.as_bytes())?;
    Ok(Some(path))
}

pub fn capture_app_state_snapshot_nonfatal(home: &Path, source: &str) {
    if let Err(error) = capture_app_state_snapshot(home) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "codex_app_state.snapshot_failed",
            json!({
                "source": source,
                "error": error.to_string(),
            }),
        );
    }
}

pub fn sync_app_state_after_provider_switch(home: &Path) -> anyhow::Result<AppStateSyncResult> {
    let Some(mut state) = load_global_state(home)? else {
        return Ok(AppStateSyncResult {
            changed: false,
            changed_keys: Vec::new(),
            backup_path: None,
            snapshot_path: None,
        });
    };
    // 读到疑似 Codex 写入中途的半截 state 时拒绝 merge 写回，
    // 避免把缺的项目键永久截掉（#2876 / #2707）。
    let account_scoped = state.contains_key(ACCOUNT_RENDERER_STATE_KEY);
    if account_scoped && account_renderer_state(&state).is_none() {
        anyhow::bail!("unrecognized account renderer state; refusing to merge");
    }
    if !account_scoped && looks_like_truncated_state(&state) {
        return Err(anyhow::anyhow!(
            "{} looks like a partially-written state (thread-* keys present but project keys missing); refusing to merge",
            state_path(home).display()
        ));
    }
    let original = Value::Object(state.clone());
    let mut changed_keys = BTreeSet::new();

    // 按账号状态原样保留，不使用旧版的跨身份目录合并规则。
    if !account_scoped {
        normalize_current_state(&mut state, &mut changed_keys);
    }
    if let Some(snapshot) = load_snapshot(home)? {
        merge_safe_snapshot(&mut state, &snapshot, &mut changed_keys);
    }

    let next = Value::Object(state);
    if next == original {
        let snapshot_path = capture_app_state_snapshot(home)?;
        return Ok(AppStateSyncResult {
            changed: false,
            changed_keys: Vec::new(),
            backup_path: None,
            snapshot_path,
        });
    }

    let backup_path = create_backup(home, &original)?;
    let text = serde_json::to_string_pretty(&next)?;
    let state_path = state_path(home);
    crate::settings::atomic_write(&state_path, text.as_bytes())?;
    if let Some(parent) = state_path.parent() {
        let _ =
            crate::settings::atomic_write(&parent.join(GLOBAL_STATE_BACKUP_FILE), text.as_bytes());
    }
    let snapshot_path = capture_app_state_snapshot(home)?;

    Ok(AppStateSyncResult {
        changed: true,
        changed_keys: changed_keys.into_iter().collect(),
        backup_path: Some(backup_path),
        snapshot_path,
    })
}

pub fn sync_app_state_after_provider_switch_nonfatal(home: &Path, source: &str) {
    match sync_app_state_after_provider_switch(home) {
        Ok(result) => {
            if result.changed {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "codex_app_state.synced",
                    json!({
                        "source": source,
                        "changedKeys": result.changed_keys,
                        "backupPath": result.backup_path.map(|path| path.to_string_lossy().to_string()),
                        "snapshotPath": result.snapshot_path.map(|path| path.to_string_lossy().to_string()),
                    }),
                );
            }
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "codex_app_state.sync_failed",
                json!({
                    "source": source,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

/// 切换供应商/聚合（或启动 Codex）后清掉模型选择器里的失效残留。
/// `valid_models` 是当前活动供应商公布的模型 ID 集合；为空时什么都不做——
/// 算不出目录时宁可留着，也不能把用户的「最近使用」误删。
pub fn prune_stale_recent_model_configurations(
    home: &Path,
    valid_models: &HashSet<String>,
) -> anyhow::Result<usize> {
    if valid_models.is_empty() {
        return Ok(0);
    }
    let Some(mut state) = load_global_state(home)? else {
        return Ok(0);
    };
    let original = Value::Object(state.clone());
    let Some(entries) = state
        .get_mut("electron-persisted-atom-state")
        .and_then(Value::as_object_mut)
        .and_then(|atom| atom.get_mut(COMPOSER_RECENT_MODELS_KEY))
        .and_then(Value::as_array_mut)
    else {
        return Ok(0);
    };
    let before = entries.len();
    entries.retain(|entry| match entry.get("model").and_then(Value::as_str) {
        Some(model) => valid_models.contains(model.trim()),
        // 结构不认识就原样保留，不替 Codex 做决定。
        None => true,
    });
    let removed = before - entries.len();
    if removed == 0 {
        return Ok(0);
    }

    let next = Value::Object(state);
    create_backup(home, &original)?;
    let text = serde_json::to_string_pretty(&next)?;
    let state_path = state_path(home);
    crate::settings::atomic_write(&state_path, text.as_bytes())?;
    if let Some(parent) = state_path.parent() {
        let _ =
            crate::settings::atomic_write(&parent.join(GLOBAL_STATE_BACKUP_FILE), text.as_bytes());
    }
    Ok(removed)
}

/// 切换/启动流程调用的非致命包装：按当前活动供应商公布的模型集合清理残留引用，
/// 并把结果写进诊断日志。任何失败都只记录，不阻断切换或启动。
pub fn prune_stale_model_references_after_switch_nonfatal(
    home: &Path,
    settings: &crate::settings::BackendSettings,
    source: &str,
) {
    let valid_models = crate::model_catalog::active_published_model_ids(settings);
    if valid_models.is_empty() {
        return;
    }
    match prune_stale_recent_model_configurations(home, &valid_models) {
        Ok(0) => {}
        Ok(removed) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "codex_app_state.pruned_stale_models",
                json!({
                    "source": source,
                    "removed": removed,
                    "publishedModelCount": valid_models.len(),
                }),
            );
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "codex_app_state.prune_stale_models_failed",
                json!({
                    "source": source,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

/// [`reconcile_account_scope`] 做了什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountScopeOutcome {
    /// 没有需要处理的情况。
    Unchanged,
    /// 容器已挪到旁路文件；`source_owner` 是项目被并入顶层的那个身份（顶层原本就属于它时也记它）。
    Detached {
        source_owner: Option<String>,
        root_project_count: usize,
        backup_path: PathBuf,
    },
    /// 容器已放回状态文件；顶层项目归属 `owner`。
    Reattached {
        owner: Option<String>,
        backup_path: PathBuf,
    },
}

/// 新版 Codex（26.1103+）按 ChatGPT 账号隔离本地项目：登录过一次就会在
/// `.codex-global-state.json` 里建 `electron-account-renderer-state-v1` 容器。
/// Codex 每次启动都会在构造函数里调用 `selectRendererOwner(null)`：
/// owner 是账号时把顶层项目存进 `inactive` 再清空顶层，owner 已是 null 时直接清空顶层。
/// 没有 ChatGPT 身份的 API Key / 网关 / 聚合模式一直停在匿名身份，于是项目列表永远是空的，
/// 在匿名身份下新建的项目下次启动也会消失（#2947 / #3018 / #3011，原生 Codex 同样复现）。
///
/// 容器不存在时构造函数什么都不做，所以：
/// - `keep_projects` 为真（开关打开且当前不是官方登录）：把容器原样挪到旁路文件，
///   并把最近活跃的那个账号的项目合并到顶层（只加不删）；
/// - `keep_projects` 为假：有旁路文件就把容器放回去，顶层项目归还给当初合并来源的账号。
///
/// 只能在 Codex 未运行时调用，否则 Codex 会用内存里的状态覆盖回去。写之前整份备份。
pub fn reconcile_account_scope(
    home: &Path,
    keep_projects: bool,
) -> anyhow::Result<AccountScopeOutcome> {
    let Some(mut state) = load_global_state(home)? else {
        return Ok(AccountScopeOutcome::Unchanged);
    };
    let detached = load_detached_account_state(home)?;
    if keep_projects {
        if !state.contains_key(ACCOUNT_RENDERER_STATE_KEY) {
            return Ok(AccountScopeOutcome::Unchanged);
        }
        let Some(accounts) = account_renderer_state(&state).cloned() else {
            anyhow::bail!("unrecognized account renderer state; refusing to detach");
        };
        let original = Value::Object(state.clone());
        // 已经挪过一次、Codex 又建了新容器（例如在 Codex 里直接登录过）：旧旁路里的身份也要留住。
        let mut container = accounts;
        if let Some(previous) = detached.as_ref() {
            merge_missing_inactive_scopes(&mut container, &previous.container);
        }
        let source_owner = match container["owner"].as_str() {
            // 顶层就是这个账号的数据，原样保留即可。
            Some(owner) => Some(owner.to_string()),
            None => {
                let source = latest_project_scope(&container);
                if let Some(owner) = source.as_deref() {
                    let scope = container["inactive"][owner].clone();
                    merge_scope_into_root(&mut state, &scope);
                }
                source
            }
        };
        state.remove(ACCOUNT_RENDERER_STATE_KEY);
        let backup_path = create_backup(home, &original)?;
        write_detached_account_state(
            home,
            &DetachedAccountState {
                container: Value::Object(container),
                source_owner: source_owner.clone(),
            },
        )?;
        write_global_state(home, &Value::Object(state.clone()))?;
        return Ok(AccountScopeOutcome::Detached {
            source_owner,
            root_project_count: project_count(&state),
            backup_path,
        });
    }

    let Some(detached) = detached else {
        return Ok(AccountScopeOutcome::Unchanged);
    };
    let original = Value::Object(state.clone());
    let owner = if let Some(current) = account_renderer_state(&state).cloned() {
        // Codex 自己重建了容器（登录过 ChatGPT），顶层已归属当前账号；只补回旁路里有、文件里没有的身份。
        let mut current = current;
        merge_missing_inactive_scopes(&mut current, &detached.container);
        let owner = current["owner"].as_str().map(str::to_owned);
        state.insert(ACCOUNT_RENDERER_STATE_KEY.to_string(), Value::Object(current));
        owner
    } else if state.contains_key(ACCOUNT_RENDERER_STATE_KEY) {
        anyhow::bail!("unrecognized account renderer state; refusing to reattach");
    } else {
        let mut container = detached
            .container
            .as_object()
            .cloned()
            .context("detached account state has no container")?;
        let owner = detached.source_owner.clone();
        // 顶层（含非官方模式期间新建的项目）归还给合并来源的账号；它在 inactive 里的旧副本已并进顶层。
        if let Some(owner) = owner.as_deref()
            && let Some(inactive) = container.get_mut("inactive").and_then(Value::as_object_mut)
        {
            inactive.remove(owner);
        }
        container.insert("owner".to_string(), json!(owner));
        state.insert(ACCOUNT_RENDERER_STATE_KEY.to_string(), Value::Object(container));
        if account_renderer_state(&state).is_none() {
            anyhow::bail!("detached account state is malformed; refusing to reattach");
        }
        owner
    };
    let backup_path = create_backup(home, &original)?;
    write_global_state(home, &Value::Object(state))?;
    let _ = fs::remove_file(detached_account_state_path(home));
    Ok(AccountScopeOutcome::Reattached { owner, backup_path })
}

/// 启动 Codex 前调用的非致命包装。`codex_running` 为真时不动文件（Codex 会用内存状态覆盖回去）。
pub fn reconcile_account_scope_nonfatal(
    home: &Path,
    settings: &crate::settings::BackendSettings,
    codex_running: bool,
    source: &str,
) {
    // 供应商管理关着时不知道 Codex 用的是哪种登录，不介入。
    if !settings.relay_profiles_enabled {
        return;
    }
    let keep_projects = settings.keep_projects_without_account
        && !crate::official_login::is_official_login_profile(&settings.active_relay_profile());
    if codex_running {
        if keep_projects || detached_account_state_path(home).exists() {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "codex_app_state.account_scope_deferred",
                json!({ "source": source, "reason": "codex_running" }),
            );
        }
        return;
    }
    match reconcile_account_scope(home, keep_projects) {
        Ok(AccountScopeOutcome::Unchanged) => {}
        Ok(AccountScopeOutcome::Detached {
            source_owner,
            root_project_count,
            backup_path,
        }) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "codex_app_state.account_scope_detached",
                json!({
                    "source": source,
                    "hasSourceOwner": source_owner.is_some(),
                    "rootProjectCount": root_project_count,
                    "backupPath": backup_path.to_string_lossy(),
                }),
            );
        }
        Ok(AccountScopeOutcome::Reattached { owner, backup_path }) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "codex_app_state.account_scope_reattached",
                json!({
                    "source": source,
                    "hasOwner": owner.is_some(),
                    "backupPath": backup_path.to_string_lossy(),
                }),
            );
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "codex_app_state.account_scope_failed",
                json!({ "source": source, "error": error.to_string() }),
            );
        }
    }
}

struct DetachedAccountState {
    container: Value,
    source_owner: Option<String>,
}

fn detached_account_state_path(home: &Path) -> PathBuf {
    home.join(DETACHED_ACCOUNT_STATE_FILE)
}

fn load_detached_account_state(home: &Path) -> anyhow::Result<Option<DetachedAccountState>> {
    let path = detached_account_state_path(home);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let container = value
        .get("container")
        .filter(|container| {
            let mut probe = Map::new();
            probe.insert(ACCOUNT_RENDERER_STATE_KEY.to_string(), (*container).clone());
            account_renderer_state(&probe).is_some()
        })
        .cloned()
        .with_context(|| format!("{} has no valid container", path.display()))?;
    Ok(Some(DetachedAccountState {
        container,
        source_owner: value
            .get("sourceOwner")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }))
}

fn write_detached_account_state(home: &Path, detached: &DetachedAccountState) -> anyhow::Result<()> {
    let path = detached_account_state_path(home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = json!({
        "version": DETACHED_ACCOUNT_STATE_VERSION,
        "managedBy": "Codex++ keepProjectsWithoutAccount",
        "detachedAtMs": now_ms() as u64,
        "sourceOwner": detached.source_owner,
        "container": detached.container,
    });
    crate::settings::atomic_write(&path, serde_json::to_string_pretty(&body)?.as_bytes())
}

/// 旁路里有、当前容器里没有的 inactive 身份补进来；当前 owner 不补（它的数据在顶层）。
fn merge_missing_inactive_scopes(target: &mut Map<String, Value>, saved: &Value) {
    let owner = target.get("owner").and_then(Value::as_str).map(str::to_owned);
    let Some(saved) = saved.get("inactive").and_then(Value::as_object) else {
        return;
    };
    let Some(inactive) = target.get_mut("inactive").and_then(Value::as_object_mut) else {
        return;
    };
    for (id, scope) in saved {
        if owner.as_deref() != Some(id.as_str()) && !inactive.contains_key(id) {
            inactive.insert(id.clone(), scope.clone());
        }
    }
}

/// 有项目的身份里，项目最近更新过的那个（同为 0 时取项目多的）。
fn latest_project_scope(container: &Map<String, Value>) -> Option<String> {
    container["inactive"]
        .as_object()?
        .iter()
        .filter_map(|(owner, scope)| {
            let projects = scope["globals"].get("local-projects")?.as_object()?;
            if projects.is_empty() {
                return None;
            }
            let latest = projects
                .values()
                .filter_map(|project| project.get("updatedAt").and_then(Value::as_u64))
                .max()
                .unwrap_or(0);
            Some((latest, projects.len(), owner.clone()))
        })
        .max()
        .map(|(_, _, owner)| owner)
}

/// 把身份的 globals / atoms 并进顶层：顶层已有的条目优先，只加不删。
fn merge_scope_into_root(state: &mut Map<String, Value>, scope: &Value) {
    if let Some(globals) = scope.get("globals").and_then(Value::as_object) {
        for (key, saved) in globals {
            if ACCOUNT_SCOPE_MERGE_SKIP_KEYS.contains(&key.as_str()) {
                continue;
            }
            let merged = match state.get(key) {
                None | Some(Value::Null) => saved.clone(),
                Some(current) => merge_root_first(current, saved),
            };
            state.insert(key.clone(), merged);
        }
    }
    if let Some(atoms) = scope.get("atoms").and_then(Value::as_object)
        && !atoms.is_empty()
    {
        let root = state
            .entry("electron-persisted-atom-state".to_string())
            .or_insert_with(|| json!({}));
        if let Some(root) = root.as_object_mut() {
            for (key, value) in atoms {
                root.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }
}

fn merge_root_first(current: &Value, saved: &Value) -> Value {
    match (current, saved) {
        (Value::Object(current), Value::Object(saved)) => {
            let mut merged = saved.clone();
            for (key, value) in current {
                merged.insert(key.clone(), value.clone());
            }
            Value::Object(merged)
        }
        // 列表（项目顺序、无项目会话）：来源账号的在前，顶层多出来的接在后面。
        (Value::Array(current), Value::Array(saved)) => {
            let mut merged = saved.clone();
            for value in current {
                if !merged.contains(value) {
                    merged.push(value.clone());
                }
            }
            Value::Array(merged)
        }
        _ => current.clone(),
    }
}

fn project_count(state: &Map<String, Value>) -> usize {
    state
        .get("local-projects")
        .and_then(Value::as_object)
        .map_or(0, Map::len)
}

fn write_global_state(home: &Path, state: &Value) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(state)?;
    let path = state_path(home);
    crate::settings::atomic_write(&path, text.as_bytes())?;
    if let Some(parent) = path.parent() {
        let _ =
            crate::settings::atomic_write(&parent.join(GLOBAL_STATE_BACKUP_FILE), text.as_bytes());
    }
    Ok(())
}

fn load_global_state(home: &Path) -> anyhow::Result<Option<Map<String, Value>>> {
    let path = state_path(home);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    value
        .as_object()
        .cloned()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("{} must be a JSON object", path.display()))
}

/// 检测疑似「Codex 新版（26.10xx+）写入中途被读到」的 state：
/// 新版项目归属用 `thread-project-*` / `local-projects` / `app-server-projects-*`，
/// 这些键在写入时一起出现。如果 state 有 `thread-project-*` 数据但所有顶层项目键
/// （`local-projects`、`selected-project`、`app-server-projects-migration-by-host`）都缺，
/// 说明读到的是写了一半的文件，继续 merge 写回会把项目归属永久截掉（#2876 / #2707）。
/// 旧版 Codex（electron-saved-workspace-roots）没有这些键是正常状态，不拦。
fn looks_like_truncated_state(state: &Map<String, Value>) -> bool {
    let has_new_thread_keys = state
        .keys()
        .any(|key| key.starts_with("thread-project-") || key.starts_with("sidebar-project-"));
    if !has_new_thread_keys {
        return false;
    }
    !state.contains_key("local-projects")
        && !state.contains_key("selected-project")
        && !state.contains_key("app-server-projects-migration-by-host")
}

fn load_snapshot(home: &Path) -> anyhow::Result<Option<Map<String, Value>>> {
    let path = snapshot_path(home);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let state = value
        .get("state")
        .and_then(Value::as_object)
        .or_else(|| value.as_object())
        .cloned()
        .unwrap_or_default();
    Ok(Some(state))
}

fn safe_snapshot_from_state(state: &Map<String, Value>) -> Value {
    let mut safe = safe_state_fields(state);
    if let Some(accounts) = account_renderer_state(state) {
        let mut inactive = Map::new();
        for (owner, scope) in accounts["inactive"].as_object().unwrap() {
            let mut fields = scope["globals"].as_object().unwrap().clone();
            fields.insert(
                "electron-persisted-atom-state".to_string(),
                scope["atoms"].clone(),
            );
            let mut globals = safe_state_fields(&fields);
            let atoms = globals
                .remove("electron-persisted-atom-state")
                .unwrap_or_else(|| json!({}));
            inactive.insert(owner.clone(), json!({"globals": globals, "atoms": atoms}));
        }
        safe.insert(
            ACCOUNT_RENDERER_STATE_KEY.to_string(),
            json!({"owner": accounts["owner"], "inactive": inactive}),
        );
    }
    json!({"version": SNAPSHOT_VERSION, "state": safe})
}

fn safe_state_fields(state: &Map<String, Value>) -> Map<String, Value> {
    let mut safe = Map::new();
    for key in WORKSPACE_PATH_ARRAY_KEYS {
        if let Some(value) = state.get(*key) {
            safe.insert((*key).to_string(), json!(dedupe_paths(path_array(value))));
        }
    }
    if let Some(value) = state.get(ACTIVE_WORKSPACE_ROOTS_KEY) {
        safe.insert(
            ACTIVE_WORKSPACE_ROOTS_KEY.to_string(),
            normalize_active_workspace_roots(value),
        );
    }
    for key in WORKSPACE_PATH_MAP_KEYS {
        if let Some(value) = state.get(*key).and_then(Value::as_object) {
            safe.insert(
                (*key).to_string(),
                Value::Object(normalize_path_keyed_map(value)),
            );
        }
    }
    for key in THREAD_STATE_MAP_KEYS {
        if let Some(value) = state.get(*key).and_then(Value::as_object) {
            safe.insert(
                (*key).to_string(),
                Value::Object(normalize_string_keyed_map(value)),
            );
        }
    }
    for key in THREAD_ID_ARRAY_KEYS {
        if let Some(value) = state.get(*key) {
            safe.insert(
                (*key).to_string(),
                json!(dedupe_strings(string_array(value))),
            );
        }
    }
    for key in SAFE_TOP_LEVEL_KEYS {
        if let Some(value) = state.get(*key) {
            safe.insert((*key).to_string(), value.clone());
        }
    }
    for key in PASS_THROUGH_STATE_KEYS {
        if let Some(value) = state.get(*key) {
            safe.insert((*key).to_string(), value.clone());
        }
    }
    if let Some(atom) = state
        .get("electron-persisted-atom-state")
        .and_then(Value::as_object)
    {
        let atom = safe_atom_state(atom);
        if !atom.is_empty() {
            safe.insert(
                "electron-persisted-atom-state".to_string(),
                Value::Object(atom),
            );
        }
    }
    safe
}

fn normalize_current_state(state: &mut Map<String, Value>, changed: &mut BTreeSet<String>) {
    for key in WORKSPACE_PATH_ARRAY_KEYS {
        if let Some(value) = state.get(*key).cloned() {
            let next = json!(dedupe_paths(path_array(&value)));
            replace_if_changed(state, key, next, changed);
        }
    }
    if let Some(value) = state.get(ACTIVE_WORKSPACE_ROOTS_KEY).cloned() {
        replace_if_changed(
            state,
            ACTIVE_WORKSPACE_ROOTS_KEY,
            normalize_active_workspace_roots(&value),
            changed,
        );
    }
    for key in WORKSPACE_PATH_MAP_KEYS {
        if let Some(value) = state.get(*key).and_then(Value::as_object) {
            let next = Value::Object(normalize_path_keyed_map(value));
            replace_if_changed(state, key, next, changed);
        }
    }
    for key in THREAD_STATE_MAP_KEYS {
        if let Some(value) = state.get(*key).and_then(Value::as_object) {
            let next = Value::Object(normalize_string_keyed_map(value));
            replace_if_changed(state, key, next, changed);
        }
    }
    for key in THREAD_ID_ARRAY_KEYS {
        if let Some(value) = state.get(*key).cloned() {
            let next = json!(dedupe_strings(string_array(&value)));
            replace_if_changed(state, key, next, changed);
        }
    }
    if let Some(value) = state
        .get("electron-persisted-atom-state")
        .and_then(Value::as_object)
        .cloned()
    {
        let mut atom = value;
        normalize_atom_state(&mut atom);
        replace_if_changed(
            state,
            "electron-persisted-atom-state",
            Value::Object(atom),
            changed,
        );
    }
}

fn merge_safe_snapshot(
    target: &mut Map<String, Value>,
    snapshot: &Map<String, Value>,
    changed: &mut BTreeSet<String>,
) {
    if target.contains_key(ACCOUNT_RENDERER_STATE_KEY)
        || snapshot.contains_key(ACCOUNT_RENDERER_STATE_KEY)
    {
        merge_account_scoped_snapshot(target, snapshot, changed);
        return;
    }
    for key in WORKSPACE_PATH_ARRAY_KEYS {
        let mut paths = target.get(*key).map(path_array).unwrap_or_default();
        paths.extend(snapshot.get(*key).map(path_array).unwrap_or_default());
        if !paths.is_empty() {
            replace_if_changed(target, key, json!(dedupe_paths(paths)), changed);
        }
    }
    let mut active_paths = target
        .get(ACTIVE_WORKSPACE_ROOTS_KEY)
        .map(path_array)
        .unwrap_or_default();
    active_paths.extend(
        snapshot
            .get(ACTIVE_WORKSPACE_ROOTS_KEY)
            .map(path_array)
            .unwrap_or_default(),
    );
    let active_paths = dedupe_paths(active_paths);
    if !active_paths.is_empty() {
        let target_is_array = target
            .get(ACTIVE_WORKSPACE_ROOTS_KEY)
            .is_some_and(Value::is_array);
        let snapshot_is_array = snapshot
            .get(ACTIVE_WORKSPACE_ROOTS_KEY)
            .is_some_and(Value::is_array);
        let next = if target_is_array || snapshot_is_array || active_paths.len() > 1 {
            json!(active_paths)
        } else {
            json!(active_paths[0])
        };
        replace_if_changed(target, ACTIVE_WORKSPACE_ROOTS_KEY, next, changed);
    }
    for key in WORKSPACE_PATH_MAP_KEYS {
        let snapshot_map = snapshot.get(*key).and_then(Value::as_object);
        let current_map = target.get(*key).and_then(Value::as_object);
        let merged = merge_path_keyed_maps(snapshot_map, current_map);
        if !merged.is_empty() {
            replace_if_changed(target, key, Value::Object(merged), changed);
        }
    }
    for key in THREAD_STATE_MAP_KEYS {
        let snapshot_map = snapshot.get(*key).and_then(Value::as_object);
        let current_map = target.get(*key).and_then(Value::as_object);
        let merged = merge_string_keyed_maps(snapshot_map, current_map);
        if !merged.is_empty() {
            replace_if_changed(target, key, Value::Object(merged), changed);
        }
    }
    for key in THREAD_ID_ARRAY_KEYS {
        let mut ids = target.get(*key).map(string_array).unwrap_or_default();
        ids.extend(snapshot.get(*key).map(string_array).unwrap_or_default());
        if !ids.is_empty() {
            replace_if_changed(target, key, json!(dedupe_strings(ids)), changed);
        }
    }
    for key in SAFE_TOP_LEVEL_KEYS {
        if let Some(value) = snapshot.get(*key) {
            replace_if_changed(target, key, value.clone(), changed);
        }
    }
    // 新版 Codex 的项目键：快照里有就带回来。当前 state 缺或比快照旧就用快照那份，
    // 半截状态被 merge 时这些键不至于凭空消失（#2876 / #2707）。
    for key in PASS_THROUGH_STATE_KEYS {
        if let Some(value) = snapshot.get(*key) {
            replace_if_changed(target, key, value.clone(), changed);
        }
    }
    if let Some(snapshot_atom) = snapshot
        .get("electron-persisted-atom-state")
        .and_then(Value::as_object)
    {
        let mut atom = target
            .get("electron-persisted-atom-state")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, value) in safe_atom_state(snapshot_atom) {
            atom.insert(key, value);
        }
        normalize_atom_state(&mut atom);
        replace_if_changed(
            target,
            "electron-persisted-atom-state",
            Value::Object(atom),
            changed,
        );
    }
}

// owner 只作为 Codex 已保存的作用域标识；不读取 auth.json，也不推算或创建身份。
fn account_renderer_state(state: &Map<String, Value>) -> Option<&Map<String, Value>> {
    let accounts = state.get(ACCOUNT_RENDERER_STATE_KEY)?.as_object()?;
    let owner = accounts.get("owner")?;
    if !owner.is_string() && !owner.is_null() {
        return None;
    }
    let inactive = accounts.get("inactive")?.as_object()?;
    if inactive.values().any(|scope| {
        scope.get("globals").and_then(Value::as_object).is_none()
            || scope.get("atoms").and_then(Value::as_object).is_none()
    }) {
        return None;
    }
    if owner
        .as_str()
        .is_some_and(|owner| inactive.contains_key(owner))
    {
        return None;
    }
    Some(accounts)
}

fn snapshot_for_owner(snapshot: &Map<String, Value>, owner: &str) -> Option<Map<String, Value>> {
    let accounts = account_renderer_state(snapshot)?;
    if accounts["owner"].as_str() == Some(owner) {
        return Some(safe_state_fields(snapshot));
    }
    let scope = accounts["inactive"].as_object()?.get(owner)?;
    let mut fields = scope["globals"].as_object()?.clone();
    fields.insert(
        "electron-persisted-atom-state".to_string(),
        scope["atoms"].clone(),
    );
    Some(safe_state_fields(&fields))
}

// 只补缺失键。已有空值可能来自用户删除或重排，不能当成损坏而用旧快照覆盖。
fn restore_missing_fields(target: &mut Map<String, Value>, snapshot: Map<String, Value>) {
    for (key, value) in snapshot {
        if key == "electron-persisted-atom-state" {
            let atom = target.entry(key).or_insert_with(|| json!({}));
            if let (Some(atom), Some(saved)) = (atom.as_object_mut(), value.as_object()) {
                for (key, value) in saved {
                    atom.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
        } else {
            target.entry(key).or_insert(value);
        }
    }
}

fn merge_account_scoped_snapshot(
    target: &mut Map<String, Value>,
    snapshot: &Map<String, Value>,
    changed: &mut BTreeSet<String>,
) {
    let Some(accounts) = account_renderer_state(target) else {
        return;
    };
    let owner = accounts["owner"].as_str().map(str::to_owned);
    let mut accounts = accounts.clone();
    if let Some(saved) = owner
        .as_deref()
        .and_then(|owner| snapshot_for_owner(snapshot, owner))
    {
        let before = target.clone();
        restore_missing_fields(target, saved);
        for (key, value) in target.iter() {
            if before.get(key) != Some(value) {
                changed.insert(key.clone());
            }
        }
    }
    // 仅恢复当前文件仍然保存的账号，不复活已退出并移除的账号。
    for (owner, scope) in accounts["inactive"].as_object_mut().unwrap() {
        let Some(mut saved) = snapshot_for_owner(snapshot, owner) else {
            continue;
        };
        let atoms = saved
            .remove("electron-persisted-atom-state")
            .unwrap_or_else(|| json!({}));
        let globals = scope["globals"].as_object_mut().unwrap();
        restore_missing_fields(globals, saved);
        let current_atoms = scope["atoms"].as_object_mut().unwrap();
        for (key, value) in atoms.as_object().unwrap() {
            current_atoms
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
    replace_if_changed(
        target,
        ACCOUNT_RENDERER_STATE_KEY,
        Value::Object(accounts),
        changed,
    );
}

fn replace_if_changed(
    target: &mut Map<String, Value>,
    key: &str,
    value: Value,
    changed: &mut BTreeSet<String>,
) {
    if target.get(key) != Some(&value) {
        target.insert(key.to_string(), value);
        changed.insert(key.to_string());
    }
}

fn safe_atom_state(atom: &Map<String, Value>) -> Map<String, Value> {
    atom.iter()
        .filter(|(key, _)| is_safe_atom_key(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn normalize_atom_state(atom: &mut Map<String, Value>) {
    if let Some(value) = atom.remove("service-tier-default") {
        atom.entry("default-service-tier".to_string())
            .or_insert(value);
    }
}

fn is_safe_atom_key(key: &str) -> bool {
    SAFE_ATOM_KEYS.contains(&key)
        || key.starts_with("app-shell:right-panel-width:")
        || key.starts_with("avatar-overlay-")
        || key.starts_with("electron:onboarding-")
        || key.starts_with("first-awake-pet-notification-")
        || key.starts_with("sidebar-project-expanded-")
        || key.starts_with("thread-summary-panel-section-expanded-")
}

fn merge_path_keyed_maps(
    snapshot: Option<&Map<String, Value>>,
    current: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let mut merged = Map::new();
    if let Some(snapshot) = snapshot {
        for (key, value) in normalize_path_keyed_map(snapshot) {
            merged.insert(key, value);
        }
    }
    if let Some(current) = current {
        for (key, value) in normalize_path_keyed_map(current) {
            merged.insert(key, value);
        }
    }
    merged
}

fn merge_string_keyed_maps(
    snapshot: Option<&Map<String, Value>>,
    current: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let mut merged = Map::new();
    if let Some(snapshot) = snapshot {
        for (key, value) in normalize_string_keyed_map(snapshot) {
            merged.insert(key, value);
        }
    }
    if let Some(current) = current {
        for (key, value) in normalize_string_keyed_map(current) {
            merged.insert(key, value);
        }
    }
    merged
}

fn normalize_path_keyed_map(map: &Map<String, Value>) -> Map<String, Value> {
    let mut next = Map::new();
    for (key, value) in map {
        if let Some(path) = normalize_desktop_path(key) {
            next.insert(path, value.clone());
        }
    }
    next
}

fn normalize_string_keyed_map(map: &Map<String, Value>) -> Map<String, Value> {
    let mut next = Map::new();
    for (key, value) in map {
        let key = key.trim();
        if !key.is_empty() {
            next.insert(key.to_string(), value.clone());
        }
    }
    next
}

fn path_array(value: &Value) -> Vec<String> {
    if let Some(items) = value.as_array() {
        items
            .iter()
            .filter_map(Value::as_str)
            .filter_map(normalize_desktop_path)
            .collect()
    } else if let Some(value) = value.as_str() {
        normalize_desktop_path(value).into_iter().collect()
    } else {
        Vec::new()
    }
}

fn normalize_active_workspace_roots(value: &Value) -> Value {
    let normalized = dedupe_paths(path_array(value));
    if value.is_array() {
        json!(normalized)
    } else if let Some(first) = normalized.first() {
        json!(first)
    } else {
        value.clone()
    }
}

fn dedupe_paths(paths: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for path in paths {
        let comparable = path
            .replace('/', r"\")
            .trim_end_matches('\\')
            .to_ascii_lowercase();
        if seen.insert(comparable) {
            result.push(path);
        }
    }
    result
}

fn string_array(value: &Value) -> Vec<String> {
    if let Some(items) = value.as_array() {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect()
    } else if let Some(value) = value.as_str() {
        let value = value.trim();
        if value.is_empty() {
            Vec::new()
        } else {
            vec![value.to_string()]
        }
    } else {
        Vec::new()
    }
}

fn dedupe_strings(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for item in items {
        if seen.insert(item.clone()) {
            result.push(item);
        }
    }
    result
}

fn normalize_desktop_path(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut path = trimmed.replace('/', r"\");
    while path.len() > 3 && path.ends_with('\\') {
        path.pop();
    }
    Some(path)
}

fn create_backup(home: &Path, original: &Value) -> anyhow::Result<PathBuf> {
    let root = home.join(BACKUP_ROOT).join(format!("{}", now_ms()));
    fs::create_dir_all(&root)?;
    fs::write(
        root.join(GLOBAL_STATE_FILE),
        serde_json::to_string_pretty(original)?,
    )?;
    fs::write(
        root.join("metadata.json"),
        serde_json::to_string_pretty(&json!({
            "version": SNAPSHOT_VERSION,
            "managedBy": "Codex++ app state sync",
            "createdAtMs": now_ms(),
        }))?,
    )?;
    Ok(root)
}

fn state_path(home: &Path) -> PathBuf {
    home.join(GLOBAL_STATE_FILE)
}

fn snapshot_path(home: &Path) -> PathBuf {
    home.join(BACKUP_ROOT).join(SNAPSHOT_FILE)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
