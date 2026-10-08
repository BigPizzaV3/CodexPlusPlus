//! 小鲸鱼的本机会话摘要。只读 rollout，不读取消息正文或凭据，不推断官方账单。

use crate::SQLiteStorageAdapter;
use chrono::{DateTime, Local, NaiveDate};
use codex_plus_core::models::SessionRef;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime};

const READ_BUDGET: u64 = 4 * 1024 * 1024;
const MAX_LINE: usize = 256 * 1024;
const CACHE_LIMIT: usize = 16;

pub(crate) fn normalize_session_id(id: &str) -> Option<String> {
    let id = id.strip_prefix("local:").unwrap_or(id);
    if id.len() != 36 {
        return None;
    }
    let normalized = uuid::Uuid::parse_str(id).ok()?.to_string();
    (normalized.eq_ignore_ascii_case(id)).then_some(normalized)
}

/// 每次最多读 4 MiB，后续请求从偏移续读；缓存有界且不包含会话正文。
pub fn session_summary(adapter: &SQLiteStorageAdapter, session: &SessionRef) -> Value {
    let now = Local::now();
    let Some(id) = normalize_session_id(&session.session_id) else {
        return unavailable(&session.session_id, "会话 ID 无效", now.timestamp_millis());
    };
    if session
        .host_id
        .as_deref()
        .is_some_and(|host| host != "local")
    {
        return unavailable(
            &id,
            "当前远程会话没有可读取的本机会话用量",
            now.timestamp_millis(),
        );
    }
    let Some(path) = adapter.whale_rollout_path(&id) else {
        return unavailable(&id, "未找到本机会话记录", now.timestamp_millis());
    };
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedRollout>>> = OnceLock::new();
    let Ok(mut cache) = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() else {
        return unavailable(&id, "会话摘要暂不可用", now.timestamp_millis());
    };
    if !cache.contains_key(&path) && cache.len() >= CACHE_LIMIT {
        if let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, item)| item.touched)
            .map(|(p, _)| p.clone())
        {
            cache.remove(&oldest);
        }
    }
    let reader = cache
        .entry(path.clone())
        .or_insert_with(|| CachedRollout::new(&id));
    reader.touched = Instant::now();
    if reader.read(&path, &id).is_err() {
        return unavailable(&id, "会话记录暂不可读", now.timestamp_millis());
    }
    reader.summary(now.date_naive(), now.timestamp_millis())
}

fn unavailable(id: &str, message: &str, now: i64) -> Value {
    json!({"status":"unavailable", "sessionId":id, "model":null, "today":null,
        "total":null, "lastTurn":null, "rateLimits":[], "updatedAt":now, "message":message})
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Tokens {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
}

impl Tokens {
    fn parse(value: &Value) -> Option<Self> {
        let input = value.get("input_tokens")?.as_u64()?;
        let output = value.get("output_tokens")?.as_u64()?;
        Some(Self::new(
            input,
            value
                .get("cached_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(input),
            output,
        ))
    }
    fn new(input: u64, cached: u64, output: u64) -> Self {
        Self {
            input_tokens: input,
            cached_input_tokens: cached.min(input),
            output_tokens: output,
            total_tokens: input.saturating_add(output),
        }
    }
    fn add(&mut self, other: Self) {
        *self = Self::new(
            self.input_tokens.saturating_add(other.input_tokens),
            self.cached_input_tokens
                .saturating_add(other.cached_input_tokens),
            self.output_tokens.saturating_add(other.output_tokens),
        );
    }
    fn delta(self, previous: Self) -> Self {
        Self::new(
            self.input_tokens.saturating_sub(previous.input_tokens),
            self.cached_input_tokens
                .saturating_sub(previous.cached_input_tokens),
            self.output_tokens.saturating_sub(previous.output_tokens),
        )
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Turn {
    id: String,
    status: &'static str,
    usage: Option<Tokens>,
    updated_at: String,
}

#[derive(Default)]
struct Summary {
    id: String,
    identity_seen: bool,
    verified: bool,
    wrong_session: bool,
    model: Option<String>,
    previous: Option<Tokens>,
    total: Tokens,
    days: BTreeMap<NaiveDate, Tokens>,
    unknown_day: bool,
    last_turn: Option<Turn>,
    rate_limits: Vec<Value>,
}

impl Summary {
    fn accept(&mut self, line: &[u8]) {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        let payload = &record["payload"];
        let kind = record["type"].as_str().unwrap_or("");
        if kind == "session_meta" {
            if !self.identity_seen {
                self.identity_seen = true;
                self.verified = payload["id"]
                    .as_str()
                    .or_else(|| payload["session_id"].as_str())
                    .and_then(normalize_session_id)
                    .as_deref()
                    == Some(&self.id);
                self.wrong_session = !self.verified;
            }
            return;
        }
        if !self.verified || self.wrong_session {
            return;
        }
        let timestamp = record["timestamp"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok());
        let updated_at = timestamp.map(|t| t.to_rfc3339()).unwrap_or_default();
        if kind == "turn_context" {
            if let Some(model) = payload["model"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 200)
            {
                self.model = Some(model.to_owned());
            }
            if let Some(id) = payload["turn_id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 200)
            {
                if self.last_turn.as_ref().is_none_or(|turn| turn.id != id) {
                    self.last_turn = Some(Turn {
                        id: id.to_owned(),
                        status: "unknown",
                        usage: None,
                        updated_at,
                    });
                }
            }
            return;
        }
        if kind != "event_msg" {
            return;
        }
        let event = payload["type"].as_str().unwrap_or("");
        let turn_id = payload["turn_id"]
            .as_str()
            .or_else(|| payload["turnId"].as_str())
            .filter(|id| !id.is_empty() && id.len() <= 200);
        match event {
            "task_started" | "turn_started" => {
                if let Some(id) = turn_id {
                    if self.last_turn.as_ref().is_none_or(|turn| turn.id != id) {
                        self.last_turn = Some(Turn {
                            id: id.to_owned(),
                            status: "running",
                            usage: None,
                            updated_at,
                        });
                    } else if let Some(turn) = self.last_turn.as_mut() {
                        turn.status = "running";
                        turn.updated_at = updated_at;
                    }
                } else if let Some(turn) = self.last_turn.as_mut() {
                    turn.status = "running";
                    turn.updated_at = updated_at;
                }
            }
            "task_complete" | "turn_completed" | "task_failed" | "turn_aborted"
            | "task_aborted" | "error" | "stream_error" => {
                let terminal_error = event == "task_failed"
                    || payload["fatal"] == true
                    || payload["will_retry"] == false
                    || event == "error" && payload["will_retry"] != true;
                if matches!(event, "error" | "stream_error") && !terminal_error {
                    return;
                }
                if self.last_turn.is_none() {
                    if let Some(id) = turn_id {
                        self.last_turn = Some(Turn {
                            id: id.to_owned(),
                            status: "unknown",
                            usage: None,
                            updated_at: String::new(),
                        });
                    }
                }
                if let Some(turn) = self
                    .last_turn
                    .as_mut()
                    .filter(|turn| turn_id.is_none_or(|id| id == turn.id))
                {
                    turn.status = if matches!(event, "turn_aborted" | "task_aborted")
                        || matches!(
                            payload["status"].as_str(),
                            Some("aborted" | "cancelled" | "canceled")
                        ) {
                        "aborted"
                    } else if terminal_error
                        || payload.get("error").is_some_and(|v| !v.is_null())
                        || matches!(payload["status"].as_str(), Some("failed" | "error"))
                    {
                        "failed"
                    } else if matches!(
                        payload["status"].as_str(),
                        Some("interrupted" | "incomplete")
                    ) {
                        "unknown"
                    } else {
                        "completed"
                    };
                    turn.updated_at = updated_at;
                }
            }
            "token_count" => {
                if let Some(limits) = payload
                    .get("rate_limits")
                    .filter(|limits| limits.is_object())
                {
                    self.rate_limits = parse_rate_limits(limits, timestamp.map(|t| t.timestamp()));
                }
                let Some(next) = Tokens::parse(&payload["info"]["total_token_usage"]) else {
                    return;
                };
                let delta = match self.previous {
                    Some(previous)
                        if next.input_tokens < previous.input_tokens
                            || next.output_tokens < previous.output_tokens =>
                    {
                        Tokens::parse(&payload["info"]["last_token_usage"]).unwrap_or(next)
                    }
                    Some(previous) => next.delta(previous),
                    None => next,
                };
                self.previous = Some(next);
                self.total.add(delta);
                if let Some(timestamp) = timestamp {
                    self.days
                        .entry(timestamp.with_timezone(&Local).date_naive())
                        .or_default()
                        .add(delta);
                } else if delta.total_tokens > 0 {
                    self.unknown_day = true;
                }
                if let Some(turn) = self
                    .last_turn
                    .as_mut()
                    .filter(|turn| turn_id.is_none_or(|id| id == turn.id))
                {
                    turn.usage.get_or_insert_with(Tokens::default).add(delta);
                    if !updated_at.is_empty() {
                        turn.updated_at = updated_at;
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_rate_limits(limits: &Value, observed_at: Option<i64>) -> Vec<Value> {
    let mut result = Vec::new();
    for (key, fallback) in [("primary", "主窗口"), ("secondary", "次窗口")] {
        let window = &limits[key];
        let Some(percent) = window["used_percent"]
            .as_f64()
            .filter(|p| p.is_finite() && (0.0..=100.0).contains(p))
        else {
            continue;
        };
        let Some(reset) = window["resets_at"].as_i64().filter(|r| *r > 0) else {
            continue;
        };
        let label = match window["window_minutes"].as_u64() {
            Some(300) => "5 小时".to_owned(),
            Some(10080) => "每周".to_owned(),
            Some(minutes) => format!("{minutes} 分钟"),
            None => fallback.to_owned(),
        };
        result.push(
            json!({"label":label,"usedPercent":percent,"resetAt":reset,"observedAt":observed_at}),
        );
    }
    result
}

struct CachedRollout {
    summary: Summary,
    offset: u64,
    pending: Vec<u8>,
    skipping_line: bool,
    modified: Option<SystemTime>,
    identity: Option<(u64, u64)>,
    created: Option<SystemTime>,
    catching_up: bool,
    touched: Instant,
}

impl CachedRollout {
    fn new(id: &str) -> Self {
        Self {
            summary: Summary {
                id: id.to_owned(),
                ..Summary::default()
            },
            offset: 0,
            pending: Vec::new(),
            skipping_line: false,
            modified: None,
            identity: None,
            created: None,
            catching_up: false,
            touched: Instant::now(),
        }
    }
    fn read(&mut self, path: &Path, id: &str) -> std::io::Result<()> {
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        let identity = file_identity(&metadata);
        let modified = metadata.modified().ok();
        let created = metadata.created().ok();
        if self.summary.id != id
            || metadata.len() < self.offset
            || self.offset > 0
                && (self.identity != identity
                    || self.created != created
                    || metadata.len() == self.offset && modified != self.modified)
        {
            *self = Self::new(id);
        }
        self.identity = identity;
        self.created = created;
        self.modified = modified;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        file.take(READ_BUDGET).read_to_end(&mut bytes)?;
        self.offset += bytes.len() as u64;
        self.catching_up = self.offset < metadata.len();
        for part in bytes.split_inclusive(|b| *b == b'\n') {
            let complete = part.last() == Some(&b'\n');
            if !self.skipping_line {
                if self.pending.len() + part.len() <= MAX_LINE {
                    self.pending.extend_from_slice(part);
                } else {
                    self.pending.clear();
                    self.skipping_line = true;
                }
            }
            if complete {
                if !self.skipping_line {
                    self.summary.accept(&self.pending);
                }
                self.pending.clear();
                self.skipping_line = false;
            }
        }
        Ok(())
    }
    fn summary(&self, today: NaiveDate, now: i64) -> Value {
        let ready = self.summary.verified && !self.summary.wrong_session && !self.catching_up;
        let tokens_ready = ready && self.summary.previous.is_some();
        let available = ready
            && (tokens_ready && !self.summary.unknown_day
                || !self.summary.rate_limits.is_empty()
                || self.summary.last_turn.is_some());
        let limits: Vec<Value> = self
            .summary
            .rate_limits
            .iter()
            .cloned()
            .map(|mut limit| {
                limit["expired"] = json!(
                    limit["resetAt"]
                        .as_i64()
                        .is_some_and(|reset| reset <= now / 1000)
                );
                limit
            })
            .collect();
        let mut value = json!({"status": if available {"ok"} else {"unavailable"},
            "sessionId":self.summary.id, "model":self.summary.model,
            "total":tokens_ready.then_some(self.summary.total),
            "today":(tokens_ready && !self.summary.unknown_day).then(|| self.summary.days.get(&today).copied().unwrap_or_default()),
            "lastTurn":if ready { self.summary.last_turn.as_ref() } else { None }, "rateLimits":limits, "updatedAt":now});
        if value["status"] == "unavailable" {
            value["message"] = json!(if self.catching_up {
                "正在读取会话用量"
            } else if self.summary.wrong_session {
                "会话记录与 ID 不匹配"
            } else if self.summary.unknown_day {
                "部分用量缺少日期，今日统计暂不可用"
            } else {
                "尚未观测到会话用量"
            });
        }
        value
    }
}

#[cfg(unix)]
fn file_identity(metadata: &Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((metadata.dev(), metadata.ino()))
}
#[cfg(not(unix))]
fn file_identity(_: &Metadata) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BackupStore;
    use chrono::{Duration, Utc};
    use rusqlite::Connection;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    const ID: &str = "019a11b9-193d-7c32-a71e-ae11fd3be78f";
    const OTHER: &str = "019a11b9-193d-7c32-a71e-ae11fd3be780";

    #[test]
    fn explicit_remote_scope_cannot_reuse_same_id_local_rollout() {
        let fixture = Fixture::new(true);
        fixture.write(&[json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":20}}}})]);
        let mut session = SessionRef::new(ID, "remote").unwrap();
        session.host_id = Some("remote-ssh:fixture".into());
        let result = session_summary(&fixture.adapter, &session);
        assert_eq!(result["status"], "unavailable");
        assert!(result["total"].is_null());
        assert!(result["message"].as_str().unwrap().contains("远程"));
        session.host_id = Some("local".into());
        assert_eq!(
            session_summary(&fixture.adapter, &session)["total"]["totalTokens"],
            120
        );
    }

    struct Fixture {
        _temp: TempDir,
        adapter: SQLiteStorageAdapter,
        path: PathBuf,
    }
    impl Fixture {
        fn new(database: bool) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path().join("sessions/2026/10/08");
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("rollout-2026-10-08T10-00-00-{ID}.jsonl"));
            let db_path = temp.path().join("state.sqlite");
            if database {
                let db = Connection::open(&db_path).unwrap();
                db.execute(
                    "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT)",
                    [],
                )
                .unwrap();
                db.execute(
                    "INSERT INTO threads VALUES (?1, ?2)",
                    [ID, path.to_str().unwrap()],
                )
                .unwrap();
            }
            let adapter =
                SQLiteStorageAdapter::new(&db_path, BackupStore::new(temp.path().join("backups")))
                    .with_codex_home(temp.path());
            Self {
                _temp: temp,
                adapter,
                path,
            }
        }
        fn write(&self, events: &[Value]) {
            let header = json!({"type":"session_meta","payload":{"id":ID}});
            let lines = std::iter::once(&header)
                .chain(events)
                .map(|v| format!("{v}\n"))
                .collect::<String>();
            fs::write(&self.path, lines).unwrap();
        }
        fn summary(&self) -> Value {
            session_summary(
                &self.adapter,
                &SessionRef::new(format!("local:{ID}"), "unused").unwrap(),
            )
        }
        fn append(&self, text: &str) {
            fs::OpenOptions::new()
                .append(true)
                .open(&self.path)
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        }
    }
    fn event(kind: &str, time: &str, mut payload: Value) -> Value {
        payload["type"] = json!(kind);
        json!({"type":"event_msg","timestamp":time,"payload":payload})
    }
    fn tokens(time: &str, input: u64, cached: u64, output: u64) -> Value {
        event(
            "token_count",
            time,
            json!({"info":{"total_token_usage":{
            "input_tokens":input,"cached_input_tokens":cached,"output_tokens":output,
            "reasoning_output_tokens":output,"total_tokens":999999}}}),
        )
    }
    fn context(time: &str, turn: &str) -> Value {
        json!({"type":"turn_context","timestamp":time,"payload":{"turn_id":turn,"model":"test-model"}})
    }

    #[test]
    fn today_uses_session_deltas_and_subsets_are_not_counted_twice() {
        let fixture = Fixture::new(true);
        let now = Utc::now().to_rfc3339();
        let previous = (Utc::now() - Duration::days(2)).to_rfc3339();
        fixture.write(&[
            tokens(&previous, 100, 40, 20),
            context(&now, "turn-2"),
            event("task_started", &now, json!({"turn_id":"turn-2"})),
            tokens(&now, 150, 60, 35),
            tokens(&now, 150, 60, 35),
            event("task_complete", &now, json!({"turn_id":"turn-2"})),
        ]);
        let result = fixture.summary();
        assert_eq!(result["status"], "ok");
        assert_eq!(result["model"], "test-model");
        assert_eq!(
            result["today"],
            json!({"inputTokens":50,"cachedInputTokens":20,"outputTokens":15,"totalTokens":65})
        );
        assert_eq!(result["total"]["totalTokens"], 185);
        assert_eq!(result["lastTurn"]["usage"], result["today"]);
        assert_eq!(result["lastTurn"]["status"], "completed");
        assert_eq!(fixture.summary()["total"], result["total"]);
    }

    #[test]
    fn cumulative_reset_uses_last_usage_without_underflow_or_duplicate_counting() {
        let fixture = Fixture::new(false);
        let now = Utc::now().to_rfc3339();
        let mut reset = tokens(&now, 10, 2, 3);
        reset["payload"]["info"]["last_token_usage"] =
            json!({"input_tokens":4,"cached_input_tokens":1,"output_tokens":2});
        fixture.write(&[tokens(&now, 100, 20, 50), reset.clone(), reset]);
        let result = fixture.summary();
        assert_eq!(
            result["total"],
            json!({"inputTokens":104,"cachedInputTokens":21,"outputTokens":52,"totalTokens":156})
        );
        assert!(!fixture._temp.path().join("state.sqlite").exists());
    }

    #[test]
    fn incremental_read_waits_for_complete_lines_and_handles_truncation() {
        let fixture = Fixture::new(false);
        let now = Utc::now().to_rfc3339();
        fixture.write(&[
            context(&now, "t"),
            event("task_started", &now, json!({"turn_id":"t"})),
            tokens(&now, 5, 2, 1),
        ]);
        assert_eq!(fixture.summary()["lastTurn"]["status"], "running");
        let next = tokens(&now, 10, 4, 2).to_string();
        fixture.append(&next[..next.len() / 2]);
        assert_eq!(fixture.summary()["total"]["totalTokens"], 6);
        fixture.append(&format!("{}\n", &next[next.len() / 2..]));
        assert_eq!(fixture.summary()["total"]["totalTokens"], 12);
        fixture.write(&[tokens(&now, 1, 0, 1)]);
        assert_eq!(fixture.summary()["total"]["totalTokens"], 2);
    }

    #[test]
    fn lifecycle_and_real_rate_limits_keep_unknown_information_explicit() {
        let fixture = Fixture::new(false);
        let now = Utc::now().to_rfc3339();
        fixture.write(&[]);
        assert_eq!(fixture.summary()["status"], "unavailable");
        fixture.write(&[context(&now, "t")]);
        let empty = fixture.summary();
        assert_eq!(empty["status"], "ok");
        assert!(empty["today"].is_null());
        assert!(empty["total"].is_null());
        assert_eq!(empty["lastTurn"]["status"], "unknown");
        assert!(empty["lastTurn"]["usage"].is_null());
        fixture.append(&format!(
            "{}\n",
            event("task_started", &now, json!({"turn_id":"t"}))
        ));
        let running = fixture.summary();
        assert_eq!(running["status"], "ok");
        assert_eq!(running["lastTurn"]["status"], "running");
        assert!(running["total"].is_null());
        let mut sample = tokens(&now, 0, 0, 0);
        sample["payload"]["rate_limits"] = json!({"primary":{"used_percent":32.5,"window_minutes":300,"resets_at":1800000000},"secondary":{"used_percent":101,"resets_at":1800000000}});
        fixture.append(&format!(
            "{sample}\n{}\n",
            event("task_failed", &now, json!({"turn_id":"t"}))
        ));
        let failed = fixture.summary();
        assert_eq!(failed["lastTurn"]["status"], "failed");
        assert_eq!(failed["rateLimits"][0]["usedPercent"], 32.5);
        assert_eq!(failed["rateLimits"][0]["resetAt"], 1800000000);
        assert_eq!(
            failed["rateLimits"][0]["observedAt"],
            DateTime::parse_from_rfc3339(&now).unwrap().timestamp()
        );
        fixture.append(&format!(
            "{}\n",
            event("turn_aborted", &now, json!({"turn_id":"t"}))
        ));
        assert_eq!(fixture.summary()["lastTurn"]["status"], "aborted");
    }

    #[test]
    fn bounded_scanning_resumes_and_does_not_publish_partial_totals() {
        let fixture = Fixture::new(false);
        let now = Utc::now().to_rfc3339();
        fixture.write(&[tokens(&now, 1, 0, 1)]);
        fixture.append(&" ".repeat(READ_BUDGET as usize));
        fixture.append(&format!("\n{}\n", tokens(&now, 3, 1, 2)));
        let first = fixture.summary();
        assert_eq!(first["status"], "unavailable");
        assert!(first["total"].is_null());
        let second = fixture.summary();
        assert_eq!(second["status"], "ok");
        assert_eq!(second["total"]["totalTokens"], 5);
    }

    #[test]
    fn rate_limit_only_snapshot_is_available_without_inventing_token_usage() {
        let fixture = Fixture::new(false);
        let now = Utc::now().to_rfc3339();
        fixture.write(&[event(
            "token_count",
            &now,
            json!({"info":null,"rate_limits":{
            "primary":{"used_percent":0,"window_minutes":300,"resets_at":1},
            "secondary":{"used_percent":17,"window_minutes":10080,"resets_at":1800500000}}}),
        )]);
        let result = fixture.summary();
        assert_eq!(result["status"], "ok");
        assert!(result["total"].is_null());
        assert!(result["today"].is_null());
        assert_eq!(result["rateLimits"].as_array().unwrap().len(), 2);
        assert_eq!(result["rateLimits"][1]["label"], "每周");
        assert_eq!(result["rateLimits"][0]["expired"], true);
        fixture.append(&format!(
            "{}\n",
            event("token_count", &now, json!({"info":null,"rate_limits":null}))
        ));
        assert_eq!(fixture.summary()["rateLimits"], result["rateLimits"]);
    }

    #[test]
    fn lookup_rejects_paths_wrong_session_metadata_and_database_escape() {
        let fixture = Fixture::new(true);
        let now = Utc::now().to_rfc3339();
        for id in [
            "../outside",
            "/tmp/secret.jsonl",
            "local:../../secret",
            "not-a-uuid",
        ] {
            let result = session_summary(&fixture.adapter, &SessionRef::new(id, "").unwrap());
            assert_eq!(result["status"], "unavailable");
            assert!(result["total"].is_null());
        }
        fs::write(
            &fixture.path,
            format!(
                "{}\n{}\n",
                json!({"type":"session_meta","payload":{"id":OTHER}}),
                tokens(&now, 99, 0, 1)
            ),
        )
        .unwrap();
        assert!(fixture.summary()["total"].is_null());
        let outside = fixture._temp.path().join("outside.jsonl");
        fs::write(&outside, "private content").unwrap();
        let db = Connection::open(fixture._temp.path().join("state.sqlite")).unwrap();
        db.execute(
            "INSERT INTO threads VALUES (?1, ?2)",
            [OTHER, outside.to_str().unwrap()],
        )
        .unwrap();
        assert!(fixture.adapter.whale_rollout_path(OTHER).is_none());
    }
}
