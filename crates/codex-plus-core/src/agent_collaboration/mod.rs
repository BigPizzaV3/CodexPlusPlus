//! 本机 CLI 协作记录。每次状态转换均在 SQLite 写事务中完成；读取不会启动 agent。

mod dsh;
pub mod executor;
pub mod integration;
pub mod mcp;
pub mod native_refresh;
mod queue_owner;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_PROMPT_BYTES: usize = 64 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_TASKS: usize = 1000;
const WORKER_LEASE_MS: u64 = 30 * 1000;
const QUEUED_LEASE_MS: u64 = 60 * 1000;
/// 排队任务由提交它的进程定期续租；间隔远小于 QUEUED_LEASE_MS，进程失联后才会过期。
const QUEUE_HEARTBEAT_MS: u64 = 10 * 1000;
/// 同一 Agent 运行中的任务达到并发上限时新任务排队，最多排这么多个（不含正在运行的，#2772）。
pub const MAX_QUEUED_PER_AGENT: usize = 8;
/// 每个 Agent 默认同时只运行一个任务，与 2.1.3 的排队行为一致（#2800）。
pub const DEFAULT_MAX_CONCURRENT_TASKS: u32 = 1;
/// 每个 Agent 同时运行的任务数上限的可选范围上界（#2800）。
pub const MAX_CONCURRENT_TASKS_LIMIT: u32 = 4;
const QUEUE_ABANDONED_PROGRESS: &str = "排队任务未启动";

/// 排队任务轮询一次的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueTurn {
    /// 已轮到并认领执行权，状态已转为 running。
    Start,
    /// 仍在排队；ahead 为还要结束几个任务才轮到本任务：更早提交的排队任务数，
    /// 加上运行中的任务超出「并发上限 - 1」的部分。并发上限为 1 时即前面的未结束任务数。
    Wait { ahead: usize },
    /// 已取消、已结束或 Agent/协作已关闭，不再启动。
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub executable: String,
    pub enabled: bool,
    /// 空值沿用原生 CLI 的模型配置，不修改供应商配置。
    #[serde(default)]
    pub model: String,
    /// 仅 DSH 可选择由原生 CLI 复用已保存的认证，本工具不保存凭据。
    #[serde(default, skip_serializing_if = "is_false")]
    pub reuse_saved_auth: bool,
    /// 同一 Agent 最多同时运行的任务数（1–4），达到上限后新任务排队（#2800）。
    /// 旧配置没有此字段时取 1；取默认值时不写出，回退到 deny_unknown_fields 的旧版本仍能读取。
    #[serde(
        default = "default_max_concurrent_tasks",
        skip_serializing_if = "is_default_max_concurrent_tasks"
    )]
    pub max_concurrent_tasks: u32,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn default_max_concurrent_tasks() -> u32 {
    DEFAULT_MAX_CONCURRENT_TASKS
}

fn is_default_max_concurrent_tasks(value: &u32) -> bool {
    *value == DEFAULT_MAX_CONCURRENT_TASKS
}

/// DSH 的临时模型配置需要同时指定供应商与模型，不能推测桌面当前选中的路由。
pub(super) fn parse_deepseek_model(value: &str) -> anyhow::Result<Option<(&str, &str)>> {
    if value.is_empty() {
        return Ok(None);
    }
    let (provider, model) = value
        .split_once('/')
        .context("DeepSeek 模型路由必须为 provider/model；留空沿用 headless 默认模型")?;
    if provider.is_empty()
        || !provider
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        || model.is_empty()
        || model.starts_with('-')
        || model.chars().any(char::is_control)
    {
        bail!("DeepSeek 模型路由格式无效，需填写有效的 provider/model");
    }
    Ok(Some((provider, model)))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollaborationConfig {
    pub enabled: bool,
    #[serde(default = "default_agent_id")]
    pub default_agent_id: String,
    pub agents: Vec<AgentConfig>,
    /// 单个子 Agent 任务的运行上限（分钟）。旧配置没有此字段时取默认 10 分钟（#2618）。
    /// 取默认值时不写出：v1.10.0 的结构带 `deny_unknown_fields`，回退旧版本时
    /// 多出的字段会让整份协作记录读不出来。
    #[serde(default = "default_max_runtime_minutes", skip_serializing_if = "is_default_max_runtime_minutes")]
    pub max_runtime_minutes: u32,
}

fn default_agent_id() -> String {
    "deepseek".to_string()
}

pub const DEFAULT_MAX_RUNTIME_MINUTES: u32 = 10;
/// 上限 4 小时：足够覆盖长任务，又不至于让卡死的 CLI 永久占着工作区。
pub const MAX_RUNTIME_MINUTES_LIMIT: u32 = 240;

fn default_max_runtime_minutes() -> u32 {
    DEFAULT_MAX_RUNTIME_MINUTES
}

fn is_default_max_runtime_minutes(value: &u32) -> bool {
    *value == DEFAULT_MAX_RUNTIME_MINUTES
}

impl Default for CollaborationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_agent_id: default_agent_id(),
            agents: [
                ("codex", "Codex"),
                ("deepseek", "DeepSeek Harness"),
                ("claude", "Claude Code"),
            ]
            .into_iter()
            .map(|(id, name)| AgentConfig {
                id: id.to_string(),
                name: name.to_string(),
                executable: String::new(),
                enabled: false,
                model: String::new(),
                reuse_saved_auth: false,
                max_concurrent_tasks: DEFAULT_MAX_CONCURRENT_TASKS,
            })
            .collect(),
            max_runtime_minutes: DEFAULT_MAX_RUNTIME_MINUTES,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskRequest {
    pub agent_id: String,
    pub prompt: String,
    pub cwd: String,
    #[serde(default)]
    pub continue_task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub agent_id: String,
    pub prompt: String,
    pub cwd: String,
    pub status: String,
    pub answer: String,
    pub error: String,
    pub session_id: String,
    pub parent_task_id: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub cancel_requested: bool,
    pub progress: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct State {
    config: CollaborationConfig,
    tasks: Vec<TaskRecord>,
    /// 排队任务的持有进程：任务 id → 持有者 id（见 [`queue_owner`]），只记录仍在排队的任务。
    /// 旧版本读取时忽略此字段，回退不受影响。
    #[serde(
        default,
        rename = "queueOwners",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    queue_owners: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Self::new(crate::paths::default_app_state_dir().join("agent-collaboration.sqlite3"))
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

pub fn is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled" | "interrupted")
}

impl Store {
    /// path 是独立的协作 SQLite 文件，不是 Codex 的会话或凭据数据库。
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn owners_dir(&self) -> PathBuf {
        queue_owner::owners_dir(&self.path)
    }

    fn connect(&self) -> anyhow::Result<Connection> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).context("无法创建协作记录目录")?;
        }
        if std::fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            bail!("协作状态文件不能是符号链接");
        }
        // 首次创建即使用私有权限，避免任务提示词短暂暴露在默认可读文件中。
        let mut options = std::fs::OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        drop(options.open(&self.path).context("无法打开协作记录")?);
        let connection = Connection::open(&self.path).context("无法连接协作数据库")?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS collaboration_state (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 payload TEXT NOT NULL
             );",
        )?;
        Ok(connection)
    }

    fn with_state<T>(
        &self,
        change: impl FnOnce(&mut State) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let payload: Option<String> = transaction
            .query_row(
                "SELECT payload FROM collaboration_state WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let mut state = parse_state(payload.as_deref())?;
        expire_abandoned_tasks(&self.owners_dir(), &mut state);
        let result = change(&mut state)?;
        let updated = serde_json::to_string(&state)?;
        // 状态轮询仍经同一事务读取，但内容未变时不反复重写完整任务记录。
        if payload.as_deref() != Some(updated.as_str()) {
            transaction.execute(
                "INSERT INTO collaboration_state(id, payload) VALUES(1, ?1)
                 ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
                params![updated],
            )?;
        }
        transaction.commit()?;
        Ok(result)
    }

    /// 只读取一份状态（不开写事务、不落盘）。租约过期只在这份内存视图里计算，
    /// 由之后的写事务落盘。
    fn read_state(&self) -> anyhow::Result<State> {
        let connection = self.connect()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload FROM collaboration_state WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let mut state = parse_state(payload.as_deref())?;
        expire_abandoned_tasks(&self.owners_dir(), &mut state);
        Ok(state)
    }

    pub fn config(&self) -> anyhow::Result<CollaborationConfig> {
        self.with_state(|state| Ok(state.config.clone()))
    }

    pub fn save_config(&self, config: CollaborationConfig) -> anyhow::Result<CollaborationConfig> {
        let config = normalize_config(config)?;
        self.with_state(|state| {
            for task in &mut state.tasks {
                if !is_terminal(&task.status)
                    && (!config.enabled
                        || !config
                            .agents
                            .iter()
                            .any(|agent| agent.id == task.agent_id && agent.enabled))
                {
                    task.cancel_requested = true;
                    task.updated_at_ms = now_ms();
                    if task.status == "queued" {
                        task.status = "cancelled".to_string();
                        task.progress = "协作已关闭，任务未启动".to_string();
                    }
                }
            }
            state.config = config;
            Ok(state.config.clone())
        })
    }

    pub fn snapshot(&self) -> anyhow::Result<Value> {
        self.with_state(|state| {
            let availability: Vec<Value> = state
                .config
                .agents
                .iter()
                .map(|agent| {
                    let resolution = executor::resolve_executable_detailed(agent);
                    // configuredMissing / recovered：已保存路径失效、以及是否已自动改用当前 CLI（#2751）。
                    json!({"id": agent.id, "available": resolution.path.is_some(),
                    "configuredMissing": resolution.configured_missing,
                    "recovered": resolution.recovered(),
                    "path": resolution.path.map(|path| path.to_string_lossy().into_owned())})
                })
                .collect();
            let mut tasks = state.tasks.clone();
            tasks.sort_by(|left, right| {
                is_terminal(&left.status)
                    .cmp(&is_terminal(&right.status))
                    .then_with(|| right.created_at_ms.cmp(&left.created_at_ms))
                    .then_with(|| right.id.cmp(&left.id))
            });
            let active_count = tasks
                .iter()
                .filter(|task| !is_terminal(&task.status))
                .count();
            tasks.truncate(50.max(active_count));
            Ok(
                json!({"config": state.config, "availability": availability, "tasks": tasks,
                "capabilities": {"cliTasks": true, "desktopSessionWake": false,
                    "maxQueuedPerAgent": MAX_QUEUED_PER_AGENT,
                    "maxConcurrentTasksLimit": MAX_CONCURRENT_TASKS_LIMIT}}),
            )
        })
    }

    pub fn submit(&self, request: TaskRequest) -> anyhow::Result<TaskRecord> {
        if request.prompt.trim().is_empty()
            || request.prompt.len() > MAX_PROMPT_BYTES
            || request.prompt.contains('\0')
        {
            bail!("任务内容不能为空，且不能超过 64 KiB 或包含空字符");
        }
        if !matches!(request.agent_id.as_str(), "codex" | "deepseek" | "claude") {
            bail!("不支持的 agent；仅支持 codex、deepseek、claude");
        }
        let path = Path::new(&request.cwd);
        if !path.is_absolute() || !path.is_dir() {
            bail!("工作目录必须是已存在的绝对目录");
        }
        let cwd = std::fs::canonicalize(path).context("无法确认任务工作目录")?;
        let cwd = cwd
            .to_str()
            .context("工作目录必须是有效 UTF-8 路径")?
            .to_string();
        self.with_state(|state| {
            if !state.config.enabled {
                bail!("请先在 Codex++ 中开启 agent 协作");
            }
            let agent = state
                .config
                .agents
                .iter()
                .find(|agent| agent.id == request.agent_id && agent.enabled)
                .context("此 agent 尚未启用")?;
            executor::resolve_executable(agent).context("未找到此 agent 的 CLI 可执行文件")?;
            if state.tasks.len() >= MAX_TASKS {
                bail!("协作记录已达到 1000 条上限，请使用新的协作状态文件");
            }
            // 同一 Agent 达到并发上限时排队而不是拒绝；排队数有上限，避免无界堆积（#2772）。
            let queued = state
                .tasks
                .iter()
                .filter(|task| task.agent_id == request.agent_id && task.status == "queued")
                .count();
            if queued >= MAX_QUEUED_PER_AGENT {
                bail!(
                    "此 agent 已有 {queued} 个排队任务，达到每个 Agent 最多排队 {MAX_QUEUED_PER_AGENT} 个的上限；请等待前面的任务结束，或取消部分排队任务后再提交"
                );
            }
            // 新任务排在所有已有任务之后（下标即 tasks.len()）。
            let ahead = tasks_ahead(
                &state.tasks,
                &request.agent_id,
                state.tasks.len(),
                concurrency_limit(&state.config, &request.agent_id),
            );
            let (parent_task_id, session_id) =
                if let Some(parent_id) = request.continue_task_id.as_deref() {
                    let parent = state
                        .tasks
                        .iter()
                        .find(|task| task.id == parent_id)
                        .context("找不到要继续的任务")?;
                    if parent.agent_id != request.agent_id || parent.cwd != cwd {
                        bail!("续聊必须使用同一个 agent 和工作目录");
                    }
                    if parent.status != "completed" || parent.session_id.is_empty() {
                        bail!("只能继续有原生会话 ID 的已完成任务");
                    }
                    if state.tasks.iter().any(|task| {
                        task.agent_id == parent.agent_id
                            && task.session_id == parent.session_id
                            && !is_terminal(&task.status)
                    }) {
                        bail!("此原生会话已有待执行或运行中的任务，请等待其结束");
                    }
                    (Some(parent.id.clone()), parent.session_id.clone())
                } else {
                    (None, String::new())
                };
            let timestamp = now_ms();
            let task = TaskRecord {
                id: uuid::Uuid::new_v4().to_string(),
                agent_id: request.agent_id,
                prompt: request.prompt,
                cwd,
                status: "queued".to_string(),
                answer: String::new(),
                error: String::new(),
                session_id,
                parent_task_id,
                created_at_ms: timestamp,
                updated_at_ms: timestamp,
                cancel_requested: false,
                progress: queue_progress(ahead),
            };
            state.tasks.push(task.clone());
            Ok(task)
        })
    }

    pub fn task(&self, id: &str) -> anyhow::Result<TaskRecord> {
        self.with_state(|state| Ok(find_task(state, id)?.clone()))
    }

    /// 事务内竞争唯一执行权，重复 spawn 不会重复运行同一任务；未轮到时返回 false。
    pub fn claim(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self.poll_queue(id)? == QueueTurn::Start)
    }

    /// 排队任务的单次轮询：轮到时认领执行权，否则刷新排队进度并续租。
    /// 同一 Agent 按提交顺序启动：运行中的任务数小于该 Agent 的并发上限、且前面没有更早的排队任务
    /// 时才轮到（#2800）；达到上限时，任一运行中的任务结束（完成、失败、取消、中断）后下一个轮到。
    /// 判断与认领在同一个 IMMEDIATE 写事务里，多个进程同时轮询也不会超过上限。
    /// 轮询的进程登记为该任务的持有者：之后租约因睡眠或卡顿过期时，只要持有进程还在就不判为失联。
    pub fn poll_queue(&self, id: &str) -> anyhow::Result<QueueTurn> {
        let owners_dir = self.owners_dir();
        self.with_state(|state| {
            let index = state
                .tasks
                .iter()
                .position(|task| task.id == id)
                .context("找不到协作任务")?;
            let task = &state.tasks[index];
            if task.status != "queued"
                || task.cancel_requested
                || !agent_enabled(&state.config, &task.agent_id)
            {
                return Ok(QueueTurn::Stop);
            }
            let ahead = tasks_ahead(
                &state.tasks,
                &task.agent_id,
                index,
                concurrency_limit(&state.config, &task.agent_id),
            );
            let timestamp = now_ms();
            if ahead == 0 {
                state.queue_owners.remove(id);
                let sharing = running_in_same_cwd(&state.tasks, index);
                let task = &mut state.tasks[index];
                task.status = "running".to_string();
                task.progress = if sharing == 0 {
                    "正在启动 CLI".to_string()
                } else {
                    format!("正在启动 CLI；{}", same_cwd_hint(sharing))
                };
                task.updated_at_ms = timestamp;
                return Ok(QueueTurn::Start);
            }
            if let Some(owner) = queue_owner::acquire(&owners_dir) {
                state.queue_owners.insert(id.to_string(), owner);
            }
            let task = &mut state.tasks[index];
            // 内容未变且租约仍新时不写库，避免排队轮询反复重写整份记录。
            if !queue_wait_is_current(task, ahead, timestamp) {
                task.progress = queue_progress(ahead);
                task.updated_at_ms = timestamp;
            }
            Ok(QueueTurn::Wait { ahead })
        })
    }

    /// 排队任务的只读检查（不开写事务）：仍要等、且进度和租约都不用更新时返回 Some(前面的任务数)；
    /// 轮到了、已不在排队、进度有变或该续租时返回 None，由调用方再走 [`Store::poll_queue`]。
    fn peek_queue(&self, id: &str) -> anyhow::Result<Option<usize>> {
        let state = self.read_state()?;
        let Some(index) = state.tasks.iter().position(|task| task.id == id) else {
            return Ok(None);
        };
        let task = &state.tasks[index];
        if task.status != "queued"
            || task.cancel_requested
            || !agent_enabled(&state.config, &task.agent_id)
        {
            return Ok(None);
        }
        let ahead = tasks_ahead(
            &state.tasks,
            &task.agent_id,
            index,
            concurrency_limit(&state.config, &task.agent_id),
        );
        Ok((ahead > 0 && queue_wait_is_current(task, ahead, now_ms())).then_some(ahead))
    }

    /// 与该任务同一工作目录、且可能与它同时运行的其他未结束任务数（只读，#2800）。
    /// 不同 Agent 的任务总可能并行；同一 Agent 只有并发上限大于 1 时才会并行。
    /// 不强制分目录，只用于给调用方一句提示。
    pub fn overlapping_cwd_tasks(&self, id: &str) -> anyhow::Result<usize> {
        let state = self.read_state()?;
        let task = state
            .tasks
            .iter()
            .find(|task| task.id == id)
            .context("找不到协作任务")?;
        let same_agent_parallel = concurrency_limit(&state.config, &task.agent_id) > 1;
        Ok(state
            .tasks
            .iter()
            .filter(|other| {
                other.id != task.id
                    && other.cwd == task.cwd
                    && !is_terminal(&other.status)
                    && (other.agent_id != task.agent_id || same_agent_parallel)
            })
            .count())
    }

    /// 执行进程退出时放弃仍在排队的任务：任务从未启动，标记失败并说明可重新提交。
    /// 已不在排队（已启动或已结束）时不做改动并返回 false。
    pub fn abandon_queued(&self, id: &str, reason: &str) -> anyhow::Result<bool> {
        self.abandon_before_launch(id, reason, false)
    }

    /// 同 [`Store::abandon_queued`]；`include_claimed` 时本进程已认领（running）但确定还没启动 CLI 的
    /// 任务也按未启动处理。只有确知 CLI 未启动的调用方（认领后、启动前被中止）才能传 true。
    fn abandon_before_launch(
        &self,
        id: &str,
        reason: &str,
        include_claimed: bool,
    ) -> anyhow::Result<bool> {
        self.with_state(|state| {
            let task = find_task(state, id)?;
            if task.status != "queued" && !(include_claimed && task.status == "running") {
                return Ok(false);
            }
            task.status = if task.cancel_requested {
                "cancelled"
            } else {
                "failed"
            }
            .to_string();
            task.error = bounded_text(reason, 8192);
            task.progress = QUEUE_ABANDONED_PROGRESS.to_string();
            task.updated_at_ms = now_ms();
            Ok(true)
        })
    }

    pub fn update_progress(&self, id: &str, text: &str) -> anyhow::Result<()> {
        self.with_state(|state| {
            let task = find_task(state, id)?;
            if !is_terminal(&task.status) {
                task.progress = bounded_text(text, 4096);
                task.updated_at_ms = now_ms();
            }
            Ok(())
        })
    }

    pub fn finish(
        &self,
        id: &str,
        status: &str,
        answer: &str,
        error: &str,
        session_id: &str,
    ) -> anyhow::Result<TaskRecord> {
        if !is_terminal(status) {
            bail!("任务结束状态无效");
        }
        if session_id.len() > 512 || session_id.chars().any(|value| value.is_control()) {
            bail!("CLI 会话 ID 格式无效");
        }
        self.with_state(|state| {
            let task = find_task(state, id)?;
            // 完成和取消并发时保留已提交的终态；取消一旦进入事务就不能被完成覆盖。
            if !is_terminal(&task.status) {
                let oversized = answer.len() > MAX_OUTPUT_BYTES;
                task.status = if task.cancel_requested {
                    "cancelled"
                } else if oversized {
                    "failed"
                } else {
                    status
                }
                .to_string();
                task.answer = if oversized {
                    String::new()
                } else {
                    answer.to_string()
                };
                task.error = if oversized {
                    "CLI 输出超过 256 KiB 上限，未保存不完整结果。".to_string()
                } else {
                    bounded_text(error, 8192)
                };
                if !session_id.is_empty() {
                    task.session_id = session_id.to_string();
                }
                task.updated_at_ms = now_ms();
                task.progress = match task.status.as_str() {
                    "completed" => "任务已完成",
                    "cancelled" => "任务已取消",
                    _ => "任务失败",
                }
                .to_string();
            }
            Ok(task.clone())
        })
    }

    pub fn cancel(&self, id: &str) -> anyhow::Result<TaskRecord> {
        self.with_state(|state| {
            let task = find_task(state, id)?;
            if !is_terminal(&task.status) {
                task.cancel_requested = true;
                task.updated_at_ms = now_ms();
                if task.status == "queued" {
                    task.status = "cancelled".to_string();
                    task.progress = "任务尚未启动，已取消".to_string();
                } else {
                    task.progress = "正在停止 CLI；已发生的操作不会撤销".to_string();
                }
            }
            Ok(task.clone())
        })
    }

    pub fn worker_should_stop(&self, id: &str) -> anyhow::Result<bool> {
        self.with_state(|state| {
            let task = state
                .tasks
                .iter()
                .find(|task| task.id == id)
                .context("找不到协作任务")?;
            Ok(task.cancel_requested
                || is_terminal(&task.status)
                || !state.config.enabled
                || !state
                    .config
                    .agents
                    .iter()
                    .any(|agent| agent.id == task.agent_id && agent.enabled))
        })
    }

    pub fn touch(&self, id: &str) -> anyhow::Result<()> {
        self.with_state(|state| {
            let task = find_task(state, id)?;
            if !is_terminal(&task.status) {
                task.updated_at_ms = now_ms();
            }
            Ok(())
        })
    }
}

/// 排在 index 处的 agent_id 任务还要等几个任务结束才轮到：更早提交的排队任务数，
/// 加上运行中的任务超出「并发上限 - 1」的部分（#2800）。为 0 即运行中的任务数小于上限、
/// 且前面没有更早的排队任务，可以启动。并发上限为 1 时等于前面的未结束任务数，与 #2772 一致。
/// 任务按提交顺序追加且不删除，向量顺序即排队顺序；index 可以等于 tasks.len()（尚未写入的新任务）。
fn tasks_ahead(tasks: &[TaskRecord], agent_id: &str, index: usize, limit: usize) -> usize {
    let (mut running, mut earlier_queued) = (0usize, 0usize);
    for (position, task) in tasks.iter().enumerate() {
        if position == index || task.agent_id != agent_id || is_terminal(&task.status) {
            continue;
        }
        if task.status != "queued" {
            running += 1;
        } else if position < index {
            earlier_queued += 1;
        }
    }
    earlier_queued + (running + 1).saturating_sub(limit.max(1))
}

/// 该 Agent 的并发上限；未知 Agent 按 1 处理。
fn concurrency_limit(config: &CollaborationConfig, agent_id: &str) -> usize {
    config
        .agents
        .iter()
        .find(|agent| agent.id == agent_id)
        .map_or(DEFAULT_MAX_CONCURRENT_TASKS, |agent| agent.max_concurrent_tasks)
        .max(1) as usize
}

/// 与 index 处任务同一工作目录、正在运行的其他任务数（任意 Agent）。
fn running_in_same_cwd(tasks: &[TaskRecord], index: usize) -> usize {
    let cwd = &tasks[index].cwd;
    tasks
        .iter()
        .enumerate()
        .filter(|(position, task)| {
            *position != index
                && task.cwd == *cwd
                && !is_terminal(&task.status)
                && task.status != "queued"
        })
        .count()
}

/// 并发任务共用工作目录时的提示：不阻止执行，只提醒调用方分目录。
pub fn same_cwd_hint(count: usize) -> String {
    format!(
        "提示：同一工作目录还有 {count} 个任务可能同时运行，可能互相修改相同文件；并发任务建议使用不同的工作目录（例如各自的 git worktree）"
    )
}

fn agent_enabled(config: &CollaborationConfig, agent_id: &str) -> bool {
    config.enabled
        && config
            .agents
            .iter()
            .any(|agent| agent.enabled && agent.id == agent_id)
}

/// 排队任务的进度文字与 ahead 一致、且租约还新，不需要写库。
fn queue_wait_is_current(task: &TaskRecord, ahead: usize, timestamp: u64) -> bool {
    task.progress == queue_progress(ahead)
        && timestamp.saturating_sub(task.updated_at_ms) < QUEUE_HEARTBEAT_MS
}

fn parse_state(payload: Option<&str>) -> anyhow::Result<State> {
    match payload {
        Some(payload) => serde_json::from_str(payload).context("协作记录损坏，未覆盖现有数据"),
        None => Ok(State::default()),
    }
}

fn queue_progress(ahead: usize) -> String {
    if ahead == 0 {
        "等待启动 CLI".to_string()
    } else {
        format!("排队中：同一 Agent 前面还有 {ahead} 个任务，轮到后自动启动")
    }
}

fn find_task<'a>(state: &'a mut State, id: &str) -> anyhow::Result<&'a mut TaskRecord> {
    state
        .tasks
        .iter_mut()
        .find(|task| task.id == id)
        .context("找不到协作任务")
}

fn normalize_config(config: CollaborationConfig) -> anyhow::Result<CollaborationConfig> {
    let mut normalized = CollaborationConfig::default();
    normalized.enabled = config.enabled;
    normalized.default_agent_id = config.default_agent_id.trim().to_string();
    if !matches!(
        normalized.default_agent_id.as_str(),
        "codex" | "deepseek" | "claude"
    ) {
        bail!("默认子 Agent 必须是 Codex、DeepSeek Harness 或 Claude Code");
    }
    if !(1..=MAX_RUNTIME_MINUTES_LIMIT).contains(&config.max_runtime_minutes) {
        bail!("子 Agent 运行上限必须在 1 到 {MAX_RUNTIME_MINUTES_LIMIT} 分钟之间");
    }
    normalized.max_runtime_minutes = config.max_runtime_minutes;
    let mut seen = std::collections::HashSet::new();
    for mut agent in config.agents {
        if !seen.insert(agent.id.clone()) {
            bail!("agent 配置不能重复");
        }
        let target = normalized
            .agents
            .iter_mut()
            .find(|value| value.id == agent.id)
            .context("不支持的 agent 配置")?;
        agent.name = agent.name.trim().to_string();
        agent.executable = agent.executable.trim().to_string();
        agent.model = agent.model.trim().to_string();
        if agent.id != "deepseek" && agent.reuse_saved_auth {
            bail!("仅 DeepSeek Harness 支持复用 DSH 已保存的认证");
        }
        if agent.name.len() > 160 || agent.name.chars().any(|value| value.is_control()) {
            bail!("agent 名称格式无效");
        }
        if agent.executable.len() > 4096 || agent.executable.chars().any(|value| value.is_control())
        {
            bail!("CLI 路径格式无效");
        }
        if !agent.executable.is_empty() && !Path::new(&agent.executable).is_absolute() {
            bail!("CLI 只能填写绝对可执行文件路径；留空自动探测，不能包含参数或 shell 表达式");
        }
        if agent.model.len() > 256
            || agent.model.starts_with('-')
            || agent.model.chars().any(char::is_control)
        {
            bail!("模型名称格式无效，不能超过 256 字节或包含控制字符");
        }
        if agent.id == "deepseek" {
            parse_deepseek_model(&agent.model)?;
        }
        if !(1..=MAX_CONCURRENT_TASKS_LIMIT).contains(&agent.max_concurrent_tasks) {
            bail!("同一 Agent 同时执行的任务数必须在 1 到 {MAX_CONCURRENT_TASKS_LIMIT} 之间");
        }
        if agent.name.is_empty() {
            agent.name.clone_from(&target.name);
        }
        *target = agent;
    }
    Ok(normalized)
}

fn bounded_text(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn expire_abandoned_tasks(owners_dir: &Path, state: &mut State) {
    let timestamp = now_ms();
    let State {
        tasks,
        queue_owners,
        ..
    } = state;
    // 持有者记录只对仍在排队的任务有意义。
    queue_owners.retain(|id, _| {
        tasks
            .iter()
            .any(|task| task.id == *id && task.status == "queued")
    });
    for task in tasks.iter_mut() {
        let lease = if task.status == "queued" {
            QUEUED_LEASE_MS
        } else {
            WORKER_LEASE_MS
        };
        if !is_terminal(&task.status) && timestamp.saturating_sub(task.updated_at_ms) > lease {
            if task.status == "queued" {
                // 租约过期不一定是进程没了：电脑睡眠或进程卡顿也会让续租停下。持有进程还在
                // 就继续等它下一次轮询续租，不能把它的任务判成失联（#2772）。
                if queue_owners
                    .get(&task.id)
                    .is_some_and(|owner| queue_owner::is_alive(owners_dir, owner))
                {
                    continue;
                }
                queue_owners.remove(&task.id);
                // 排队任务从未启动 CLI，结果是确定的：没有执行任何操作，可以放心重新提交。
                task.status = if task.cancel_requested {
                    "cancelled"
                } else {
                    "failed"
                }
                .to_string();
                task.error = "排队期间提交此任务的执行进程（Codex++ 或 MCP）已退出或失联，任务未启动，没有执行任何操作；可以重新提交。".to_string();
                task.progress = QUEUE_ABANDONED_PROGRESS.to_string();
                task.updated_at_ms = timestamp;
                continue;
            }
            task.status = if task.cancel_requested {
                "cancelled"
            } else {
                "interrupted"
            }
            .to_string();
            task.error =
                "执行进程已失联；任务结果不确定，请先检查工作目录，不会自动重试。".to_string();
            task.progress = "执行进程已失联".to_string();
            task.updated_at_ms = timestamp;
        }
    }
}
