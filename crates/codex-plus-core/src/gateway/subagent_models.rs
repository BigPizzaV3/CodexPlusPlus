use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;

use super::route;
use super::store::GatewaySettings;
use crate::protocol_proxy::ProxySessionHeaders;
use crate::tools::ToolId;

const POLICY_MARKER: &str = "[Codex++ inherited model]";
const RETENTION_SECONDS: i64 = 90 * 24 * 60 * 60;
const CLEANUP_INTERVAL_SECONDS: u64 = 60 * 60;
static LAST_CLEANUP: AtomicU64 = AtomicU64::new(0);
static PENDING_ID: AtomicU64 = AtomicU64::new(1);
static UNRELIABLE: OnceLock<Mutex<HashMap<IdentityKey, RecordStatus>>> = OnceLock::new();
static BARRIER_LOCK: Mutex<()> = Mutex::new(());

fn barrier_path(path: &Path) -> PathBuf {
    path.with_extension("unreliable")
}

fn read_barrier(path: &Path) -> anyhow::Result<i64> {
    match std::fs::read_to_string(barrier_path(path)) {
        Ok(value) => Ok(value.trim().parse()?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

fn mark_registry_unreliable(path: &Path) -> anyhow::Result<()> {
    let _guard = BARRIER_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let previous = read_barrier(path)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(i64::MAX as u128) as i64;
    let version = now.max(previous.saturating_add(1));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::settings::atomic_write(&barrier_path(path), version.to_string().as_bytes())
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct IdentityKey {
    registry: PathBuf,
    home: PathBuf,
    sqlite_home: PathBuf,
    thread: String,
    turn: String,
}

#[derive(Default)]
struct RecordStatus {
    pending: HashSet<u64>,
    failed_before: Option<u64>,
}

fn identity_key(
    path: &Path,
    home: &Path,
    sqlite_home: &Path,
    thread: &str,
    turn: &str,
) -> IdentityKey {
    IdentityKey {
        registry: path.to_path_buf(),
        home: home.to_path_buf(),
        sqlite_home: sqlite_home.to_path_buf(),
        thread: thread.to_string(),
        turn: turn.to_string(),
    }
}

fn begin_record(key: &IdentityKey) -> u64 {
    let id = PENDING_ID.fetch_add(1, Ordering::Relaxed);
    let mut entries = UNRELIABLE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    entries.entry(key.clone()).or_default().pending.insert(id);
    id
}

fn finish_record(key: &IdentityKey, id: u64, success: bool) {
    let mut entries = UNRELIABLE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(status) = entries.get_mut(key) {
        if status.pending.remove(&id) {
            if success {
                if status
                    .failed_before
                    .is_some_and(|failed_before| id >= failed_before)
                {
                    status.failed_before = None;
                }
            } else {
                status.failed_before = Some(PENDING_ID.load(Ordering::Relaxed));
            }
        }
        if status.pending.is_empty() && status.failed_before.is_none() {
            entries.remove(key);
        }
    }
}

fn record_is_unreliable(key: &IdentityKey) -> bool {
    UNRELIABLE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains_key(key)
}

fn marked_child(headers: ProxySessionHeaders<'_>) -> bool {
    headers.client_identity.iter().any(|(name, value)| {
        (name.eq_ignore_ascii_case("x-openai-subagent") && value == "collab_spawn")
            || (name.eq_ignore_ascii_case("x-codex-parent-thread-id") && !value.trim().is_empty())
            || (name.eq_ignore_ascii_case("x-codex-turn-metadata")
                && serde_json::from_str::<Value>(value)
                    .ok()
                    .is_some_and(|metadata| {
                        metadata["subagent_kind"] == "thread_spawn"
                            || metadata["parent_thread_id"]
                                .as_str()
                                .is_some_and(|parent| !parent.trim().is_empty())
                    }))
    })
}

#[derive(Default)]
struct Decision {
    replacement: Option<String>,
    guidance: Option<String>,
}

fn diagnostic(reason: &str) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "gateway.subagent_model_inheritance",
        serde_json::json!({"reason": reason}),
    );
}

struct Identity {
    thread: String,
    turn: String,
    parent: String,
    parent_turn: String,
    child: bool,
}

impl Identity {
    fn read(headers: ProxySessionHeaders<'_>) -> anyhow::Result<Self> {
        let header = |name: &str| {
            headers
                .client_identity
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        };
        let metadata: Value = header("x-codex-turn-metadata")
            .map(serde_json::from_str)
            .transpose()
            .context("Codex 回合身份格式无效")?
            .unwrap_or(Value::Null);
        let field = |name: &str| metadata[name].as_str().unwrap_or("").trim().to_string();
        let checked = |value: Option<&str>, name: &str| -> anyhow::Result<String> {
            let value = value.unwrap_or("").trim();
            let meta = field(name);
            if !value.is_empty() && !meta.is_empty() && value != meta {
                bail!("Codex 请求头与回合身份不一致：{name}");
            }
            let value = if value.is_empty() {
                meta
            } else {
                value.to_string()
            };
            if value.len() > 256 || value.chars().any(char::is_control) {
                bail!("Codex 回合身份无效：{name}");
            }
            Ok(value)
        };
        let parent = checked(header("x-codex-parent-thread-id"), "parent_thread_id")?;
        let child = !parent.is_empty()
            || header("x-openai-subagent") == Some("collab_spawn")
            || metadata["subagent_kind"] == "thread_spawn";
        let thread = checked(headers.thread_id, "thread_id")?;
        let thread = if thread.is_empty() {
            checked(headers.session_id, "thread_id")?
        } else {
            thread
        };
        Ok(Self {
            thread,
            turn: checked(None, "turn_id")?,
            parent,
            parent_turn: checked(None, "parent_turn_id")?,
            child,
        })
    }
}

fn recorded_model(
    db: &Connection,
    scope: &str,
    thread: &str,
    turn: &str,
) -> anyhow::Result<Option<(String, String, String, String, i64)>> {
    Ok(db
        .query_row(
            "SELECT model, parent, parent_turn, model_kind, barrier FROM gateway_subagent_models
         WHERE scope=?1 AND thread=?2 AND turn=?3",
            params![scope, thread, turn],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?)
}

fn parent_rollout_model(
    home: &Path,
    sqlite_home: &Path,
    parent: &str,
    turn: &str,
) -> anyhow::Result<String> {
    for path in crate::codex_sqlite::codex_session_db_paths_in_home(sqlite_home) {
        let Ok(database) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        else {
            continue;
        };
        if database.busy_timeout(Duration::from_millis(500)).is_err() {
            continue;
        }
        let Ok(record) = database
            .query_row(
                "SELECT rollout_path, source FROM threads WHERE id=?1",
                [parent],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
        else {
            continue;
        };
        let Some((rollout_path, source)) = record else {
            continue;
        };
        if serde_json::from_str::<Value>(&source)
            .ok()
            .is_some_and(|source| source.get("subagent").is_some())
        {
            bail!("父回合来自子智能体，缺少其已继承的模型身份，不能按裸模型重新选择供应商");
        }
        let rollout = std::fs::canonicalize(rollout_path).context("父回合记录不可读取")?;
        if !rollout.starts_with(home) {
            bail!("父回合记录不在当前 Codex Home 中");
        }
        for line in BufReader::new(File::open(rollout)?).lines() {
            let line = line?;
            if !line.contains("\"turn_context\"") {
                continue;
            }
            let Ok(row) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if row["type"] == "turn_context" && row["payload"]["turn_id"] == turn {
                let model = row["payload"]["model"].as_str().unwrap_or("").trim();
                if !model.is_empty() {
                    return Ok(model.to_string());
                }
            }
        }
    }
    bail!("找不到父回合的模型身份，拒绝回退到默认供应商")
}

fn patch_spawn_tools(tools: &mut Value, model: &str, native: bool) {
    let Some(tools) = tools.as_array_mut() else {
        return;
    };
    for tool in tools {
        let name = tool["name"].as_str().unwrap_or("").to_string();
        let native = native
            || matches!(
                name.as_str(),
                "multi_agent_v1" | "multi_agent_v2" | "multi_agent"
            );
        if tool["type"] == "namespace" {
            patch_spawn_tools(&mut tool["tools"], model, native);
        } else if (native && name == "spawn_agent")
            || matches!(
                name.as_str(),
                "multi_agent_v1.spawn_agent" | "multi_agent_v2.spawn_agent"
            )
        {
            let original = tool["description"].as_str().unwrap_or("");
            let original = original
                .split(POLICY_MARKER)
                .next()
                .unwrap_or("")
                .trim_end();
            tool["description"] = Value::String(format!(
                "{original}\n{POLICY_MARKER} 子智能体默认继承父回合模型身份 {model}，创建时可省略 model；需要主动切换时请明确指定完整供应商/模型或 group/ 聚合组。"
            ));
        }
    }
}

pub(super) async fn prepare(
    gateway: &GatewaySettings,
    agent: &ToolId,
    request: &mut Value,
    headers: ProxySessionHeaders<'_>,
) -> anyhow::Result<()> {
    if *agent != ToolId::Codex || gateway.active_binding(agent).is_none() {
        return Ok(());
    }
    let gateway = gateway.clone();
    let agent = agent.clone();
    let model = request["model"].as_str().unwrap_or("").to_string();
    let session = headers.session_id.map(str::to_owned);
    let thread = headers.thread_id.map(str::to_owned);
    let opencode_session = headers.opencode_session.map(str::to_owned);
    let client_identity = headers.client_identity.to_vec();
    let result = tokio::task::spawn_blocking(move || {
        let home = crate::codex_home::default_codex_home_dir();
        let sqlite_home = crate::codex_sqlite::resolve_sqlite_home_home_or_default(&home);
        decide_at(
            &gateway,
            &agent,
            &model,
            ProxySessionHeaders {
                session_id: session.as_deref(),
                thread_id: thread.as_deref(),
                opencode_session: opencode_session.as_deref(),
                client_identity: &client_identity,
            },
            &crate::paths::default_app_state_dir().join("gateway-subagent-models.sqlite"),
            &home,
            &sqlite_home,
            &diagnostic,
        )
    })
    .await;
    let decision = match result {
        Ok(result) => result?,
        Err(_) => {
            let _ = tokio::task::spawn_blocking(|| {
                diagnostic("worker_failed");
                let path =
                    crate::paths::default_app_state_dir().join("gateway-subagent-models.sqlite");
                if mark_registry_unreliable(&path).is_err() {
                    diagnostic("unreliable_marker_failed");
                }
            })
            .await;
            if marked_child(headers) {
                bail!("子智能体模型身份处理失败，拒绝隐式切换供应商");
            }
            return Ok(());
        }
    };
    apply_decision(request, decision);
    Ok(())
}

fn apply_decision(request: &mut Value, decision: Decision) {
    if let Some(model) = decision.replacement {
        request["model"] = Value::String(model);
    }
    let Some(model) = decision.guidance else {
        return;
    };
    if let Some(tools) = request.get_mut("tools") {
        patch_spawn_tools(tools, &model, false);
    }
    if let Some(input) = request.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            if item["type"] == "tool_search_output" {
                patch_spawn_tools(&mut item["tools"], &model, false);
            }
        }
    }
}

#[cfg(test)]
fn prepare_at(
    gateway: &GatewaySettings,
    agent: &ToolId,
    request: &mut Value,
    headers: ProxySessionHeaders<'_>,
    path: &Path,
    home: &Path,
    sqlite_home: &Path,
) -> anyhow::Result<()> {
    let decision = decide_at(
        gateway,
        agent,
        request["model"].as_str().unwrap_or(""),
        headers,
        path,
        home,
        sqlite_home,
        &|_| {},
    )?;
    apply_decision(request, decision);
    Ok(())
}

fn decide_at(
    gateway: &GatewaySettings,
    agent: &ToolId,
    requested: &str,
    headers: ProxySessionHeaders<'_>,
    path: &Path,
    home: &Path,
    sqlite_home: &Path,
    report: &dyn Fn(&str),
) -> anyhow::Result<Decision> {
    if *agent != ToolId::Codex {
        return Ok(Decision::default());
    }
    let Some(binding) = gateway.active_binding(agent) else {
        return Ok(Decision::default());
    };
    let identity = match Identity::read(headers) {
        Ok(identity) => identity,
        Err(_) => {
            report("invalid_metadata");
            if marked_child(headers) {
                bail!("子智能体回合身份无效，拒绝按裸模型隐式切换供应商");
            }
            if mark_registry_unreliable(path).is_err() {
                report("unreliable_marker_failed");
            }
            return Ok(Decision::default());
        }
    };
    if identity.child
        && [
            &identity.thread,
            &identity.turn,
            &identity.parent,
            &identity.parent_turn,
        ]
        .iter()
        .any(|value| value.is_empty())
    {
        report("missing_child_identity");
        return Ok(Decision::default());
    }
    if identity.thread.is_empty() || identity.turn.is_empty() {
        return Ok(Decision::default());
    }
    if identity.child && identity.thread == identity.parent {
        report("invalid_child_identity");
        bail!("子智能体不能把自己声明为父回合");
    }
    let explicit =
        identity.child && route::has_explicit_model_identity(gateway, binding, requested);
    let key = identity_key(path, home, sqlite_home, &identity.thread, &identity.turn);
    let pending = (!identity.child || explicit).then(|| begin_record(&key));
    let parent = identity_key(
        path,
        home,
        sqlite_home,
        &identity.parent,
        &identity.parent_turn,
    );
    if identity.child && !explicit && record_is_unreliable(&key) {
        bail!("子回合模型登记状态不确定，拒绝使用可能过期的身份");
    }
    if identity.child && !explicit && record_is_unreliable(&parent) {
        bail!("父回合模型登记未完成，拒绝使用可能过期的身份");
    }
    let result = record_decision(
        gateway,
        binding,
        requested,
        &identity,
        explicit,
        path,
        home,
        sqlite_home,
        report,
    );
    if identity.child && !explicit && record_is_unreliable(&parent) {
        if result.is_ok() && mark_registry_unreliable(path).is_err() {
            let pending = begin_record(&key);
            finish_record(&key, pending, false);
            report("unreliable_marker_failed");
        }
        bail!("父回合模型登记未完成，拒绝使用可能过期的身份");
    }
    match result {
        Ok(decision) => {
            if let Some(pending) = pending {
                finish_record(&key, pending, true);
            }
            Ok(decision)
        }
        Err(error) if identity.child && !explicit => Err(error),
        Err(_) => {
            report("registry_unavailable");
            let invalidated = mark_registry_unreliable(path).is_ok();
            if !invalidated {
                report("unreliable_marker_failed");
            }
            if let Some(pending) = pending {
                finish_record(&key, pending, invalidated);
            }
            Ok(Decision::default())
        }
    }
}

fn record_decision(
    gateway: &GatewaySettings,
    binding: &super::store::AgentBinding,
    requested: &str,
    identity: &Identity,
    explicit: bool,
    path: &Path,
    home: &Path,
    sqlite_home: &Path,
    report: &dyn Fn(&str),
) -> anyhow::Result<Decision> {
    let home = std::fs::canonicalize(home)?;
    let sqlite_home = std::fs::canonicalize(sqlite_home)?;
    let scope = serde_json::to_string(&(&home, &sqlite_home))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?
    }
    let mut database = Connection::open(path)?;
    database.busy_timeout(Duration::from_millis(500))?;
    database.execute_batch(
        "CREATE TABLE IF NOT EXISTS gateway_subagent_models (
            scope TEXT NOT NULL, thread TEXT NOT NULL, turn TEXT NOT NULL,
            parent TEXT NOT NULL, parent_turn TEXT NOT NULL, model TEXT NOT NULL,
            updated_at INTEGER NOT NULL DEFAULT 0,
            model_kind TEXT NOT NULL DEFAULT '',
            barrier INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(scope, thread, turn)
        ) WITHOUT ROWID;",
    )?;
    if !has_column(&database, "updated_at")? {
        if let Err(error) = database.execute(
            "ALTER TABLE gateway_subagent_models ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0",
            [],
        ) {
            if !has_column(&database, "updated_at")? {
                return Err(error.into());
            }
        }
    }
    if !has_column(&database, "model_kind")? {
        if let Err(error) = database.execute(
            "ALTER TABLE gateway_subagent_models ADD COLUMN model_kind TEXT NOT NULL DEFAULT ''",
            [],
        ) {
            if !has_column(&database, "model_kind")? {
                return Err(error.into());
            }
        }
    }
    if !has_column(&database, "barrier")? {
        if let Err(error) = database.execute(
            "ALTER TABLE gateway_subagent_models ADD COLUMN barrier INTEGER NOT NULL DEFAULT 0",
            [],
        ) {
            if !has_column(&database, "barrier")? {
                return Err(error.into());
            }
        }
    }
    let barrier = read_barrier(path)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let fallback = if identity.child
        && !explicit
        && recorded_model(&database, &scope, &identity.thread, &identity.turn)?.is_none()
        && recorded_model(&database, &scope, &identity.parent, &identity.parent_turn)?.is_none()
    {
        if barrier != 0 {
            bail!("父回合模型登记在故障后尚未重新确认");
        }
        Some(parent_rollout_model(
            &home,
            &sqlite_home,
            &identity.parent,
            &identity.parent_turn,
        )?)
    } else {
        None
    };
    let transaction = database.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let saved = recorded_model(&transaction, &scope, &identity.thread, &identity.turn)?;
    if let Some((_, parent, parent_turn, _, _)) = &saved {
        if parent != &identity.parent || parent_turn != &identity.parent_turn {
            bail!("子智能体父回合身份与已记录的身份不一致");
        }
    }
    let (model, kind) = if identity.child && !explicit {
        if let Some((model, _, _, kind, saved_barrier)) = saved {
            if saved_barrier != barrier {
                let parent = recorded_model(
                    &transaction,
                    &scope,
                    &identity.parent,
                    &identity.parent_turn,
                )?;
                if !parent.is_some_and(|(parent_model, _, _, parent_kind, parent_barrier)| {
                    parent_barrier == barrier
                        && parent_model == model
                        && !parent_kind.is_empty()
                        && !kind.is_empty()
                        && parent_kind == kind
                }) {
                    bail!("子回合模型登记在故障后尚未重新确认");
                }
            }
            (model, kind)
        } else {
            match recorded_model(
                &transaction,
                &scope,
                &identity.parent,
                &identity.parent_turn,
            )? {
                Some((model, _, _, kind, saved_barrier)) => {
                    if saved_barrier != barrier {
                        bail!("父回合模型登记在故障后尚未重新确认");
                    }
                    (model, kind)
                }
                None => {
                    let model = fallback.context("父回合身份已变化，请重试")?;
                    let kind = route::model_identity_kind(gateway, binding, &model).to_string();
                    (model, kind)
                }
            }
        }
    } else {
        let model = route::effective_model(
            gateway,
            binding,
            &route::binding_default_model(gateway, binding),
            requested,
        );
        let kind = route::model_identity_kind(gateway, binding, &model).to_string();
        (model, kind)
    };
    let current_kind = route::model_identity_kind(gateway, binding, &model);
    let kind = if kind.is_empty() {
        current_kind.to_string()
    } else {
        kind
    };
    if identity.child
        && !explicit
        && (kind != current_kind
            || !route::model_is_routable(gateway, binding, &model)
            || route::effective_model(
                gateway,
                binding,
                &route::binding_default_model(gateway, binding),
                &model,
            ) != model)
    {
        bail!("父回合模型 {model} 的路由身份不可用或发生变化，拒绝隐式切换供应商");
    }
    transaction.execute(
        "INSERT INTO gateway_subagent_models(scope,thread,turn,parent,parent_turn,model,updated_at,model_kind,barrier)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(scope,thread,turn) DO UPDATE SET model=excluded.model, updated_at=excluded.updated_at, model_kind=excluded.model_kind, barrier=excluded.barrier",
        params![scope, identity.thread, identity.turn, identity.parent, identity.parent_turn, model, now, kind, barrier],
    )?;
    touch_ancestors(&transaction, &scope, &identity.thread, &identity.turn, now)?;
    transaction.commit()?;
    if identity.child && !explicit && read_barrier(path)? != barrier {
        bail!("父回合模型登记在请求期间发生变化");
    }
    maybe_cleanup(&database, now, report);
    Ok(Decision {
        replacement: (identity.child && !explicit && requested != model).then(|| model.clone()),
        guidance: route::has_explicit_model_identity(gateway, binding, &model).then_some(model),
    })
}

fn has_column(database: &Connection, name: &str) -> anyhow::Result<bool> {
    Ok(database.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('gateway_subagent_models') WHERE name=?1)",
        [name],
        |row| row.get(0),
    )?)
}

fn touch_ancestors(
    database: &Connection,
    scope: &str,
    thread: &str,
    turn: &str,
    now: i64,
) -> anyhow::Result<()> {
    database.execute(
        "WITH RECURSIVE lineage(thread, turn, parent, parent_turn) AS (
            SELECT thread, turn, parent, parent_turn FROM gateway_subagent_models
              WHERE scope=?1 AND thread=?2 AND turn=?3
            UNION
            SELECT entry.thread, entry.turn, entry.parent, entry.parent_turn
              FROM gateway_subagent_models AS entry JOIN lineage
              ON entry.scope=?1 AND entry.thread=lineage.parent AND entry.turn=lineage.parent_turn
         ) UPDATE gateway_subagent_models SET updated_at=?4
           WHERE scope=?1 AND (thread, turn) IN (SELECT thread, turn FROM lineage)",
        params![scope, thread, turn, now],
    )?;
    Ok(())
}

fn cleanup_expired(database: &Connection, now: i64) -> anyhow::Result<()> {
    database.execute(
        "UPDATE gateway_subagent_models SET updated_at=?1 WHERE updated_at=0",
        [now],
    )?;
    database.execute(
        "DELETE FROM gateway_subagent_models WHERE updated_at < ?1",
        [now.saturating_sub(RETENTION_SECONDS)],
    )?;
    Ok(())
}

fn maybe_cleanup(database: &Connection, now: i64, report: &dyn Fn(&str)) {
    let now = now.max(0) as u64;
    let last = LAST_CLEANUP.load(Ordering::Relaxed);
    if now.saturating_sub(last) >= CLEANUP_INTERVAL_SECONDS
        && LAST_CLEANUP
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        && cleanup_expired(database, now as i64).is_err()
    {
        report("cleanup_failed");
    }
}

#[cfg(test)]
#[path = "subagent_models_tests.rs"]
mod tests;
