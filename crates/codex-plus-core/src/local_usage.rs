use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::settings::BackendSettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalUsagePolicy {
    pub enabled: bool,
    pub daily_token_limit: i64,
    pub subject_id: Option<String>,
    pub plan_id: Option<String>,
    pub limit_source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalUsageSnapshot {
    pub enabled: bool,
    pub daily_token_limit: i64,
    pub subject_id: Option<String>,
    pub plan_id: Option<String>,
    pub limit_source: String,
    pub day: String,
    pub used_tokens: i64,
    pub request_count: i64,
    pub remaining_tokens: Option<i64>,
    pub db_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalUsageExport {
    pub generated_at_ms: i64,
    pub db_path: String,
    pub summaries: Vec<LocalUsageSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalUsageSummary {
    pub day: String,
    pub subject_id: String,
    pub plan_id: Option<String>,
    pub request_count: i64,
    pub request_bytes: i64,
    pub response_bytes: i64,
    pub estimated_tokens: i64,
    pub reported_total_tokens: i64,
    pub effective_total_tokens: i64,
    pub first_seen_at_ms: i64,
    pub last_seen_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct LocalUsageEvent {
    pub method: String,
    pub path: String,
    pub upstream_protocol: String,
    pub status_code: u16,
    pub request_bytes: usize,
    pub response_bytes: usize,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone)]
pub struct LocalUsageStore {
    db_path: PathBuf,
}

impl Default for LocalUsageStore {
    fn default() -> Self {
        Self::new(default_usage_db_path())
    }
}

impl LocalUsageStore {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    pub fn preflight_request(
        &self,
        policy: LocalUsagePolicy,
        request_bytes: usize,
    ) -> anyhow::Result<LocalUsageSnapshot> {
        let snapshot = self.snapshot(policy.clone())?;
        if !policy.enabled || policy.daily_token_limit <= 0 {
            return Ok(snapshot);
        }
        let request_estimate = estimate_tokens_from_bytes(request_bytes, 0);
        if snapshot.used_tokens + request_estimate > policy.daily_token_limit {
            anyhow::bail!(
                "本地每日额度已用尽：今日已用约 {} tokens，当前请求预计 {} tokens，上限 {} tokens。",
                snapshot.used_tokens,
                request_estimate,
                policy.daily_token_limit
            );
        }
        Ok(snapshot)
    }

    pub fn snapshot(&self, policy: LocalUsagePolicy) -> anyhow::Result<LocalUsageSnapshot> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let day = day_key(now_ms());
        let (used_tokens, request_count) = if let Some(subject_id) = policy.subject_id.as_deref() {
            db.query_row(
                "SELECT COALESCE(SUM(COALESCE(reported_total_tokens, estimated_tokens)), 0), COUNT(*)
                 FROM local_usage_events
                 WHERE day = ?1 AND subject_id = ?2",
                params![day, subject_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0))
        } else {
            db.query_row(
                "SELECT COALESCE(SUM(COALESCE(reported_total_tokens, estimated_tokens)), 0), COUNT(*)
                 FROM local_usage_events
                 WHERE day = ?1",
                params![day],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0))
        };
        Ok(LocalUsageSnapshot {
            enabled: policy.enabled,
            daily_token_limit: policy.daily_token_limit,
            subject_id: policy.subject_id,
            plan_id: policy.plan_id,
            limit_source: policy.limit_source,
            day,
            used_tokens,
            request_count,
            remaining_tokens: (policy.enabled && policy.daily_token_limit > 0)
                .then_some((policy.daily_token_limit - used_tokens).max(0)),
            db_path: self.db_path.to_string_lossy().to_string(),
        })
    }

    pub fn export_summary(&self) -> anyhow::Result<LocalUsageExport> {
        self.ensure_schema()?;
        let generated_at_ms = now_ms();
        let db = Connection::open(&self.db_path)?;
        let mut statement = db.prepare(
            "SELECT
                day,
                subject_id,
                plan_id,
                COUNT(*),
                COALESCE(SUM(request_bytes), 0),
                COALESCE(SUM(response_bytes), 0),
                COALESCE(SUM(estimated_tokens), 0),
                COALESCE(SUM(reported_total_tokens), 0),
                COALESCE(SUM(COALESCE(reported_total_tokens, estimated_tokens)), 0),
                MIN(created_at_ms),
                MAX(created_at_ms)
             FROM local_usage_events
             GROUP BY day, subject_id, plan_id
             ORDER BY day DESC, subject_id ASC, plan_id ASC",
        )?;
        let summaries = statement
            .query_map([], |row| {
                Ok(LocalUsageSummary {
                    day: row.get(0)?,
                    subject_id: row.get(1)?,
                    plan_id: row.get(2)?,
                    request_count: row.get(3)?,
                    request_bytes: row.get(4)?,
                    response_bytes: row.get(5)?,
                    estimated_tokens: row.get(6)?,
                    reported_total_tokens: row.get(7)?,
                    effective_total_tokens: row.get(8)?,
                    first_seen_at_ms: row.get(9)?,
                    last_seen_at_ms: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(LocalUsageExport {
            generated_at_ms,
            db_path: self.db_path.to_string_lossy().to_string(),
            summaries,
        })
    }

    pub fn record_event(&self, event: LocalUsageEvent) -> anyhow::Result<()> {
        self.ensure_schema()?;
        let now = now_ms();
        let estimated_tokens =
            estimate_tokens_from_bytes(event.request_bytes, event.response_bytes);
        let usage = event.token_usage;
        let entitlement = crate::local_account::LocalAccountStore::default()
            .load_active_entitlement()
            .ok()
            .flatten();
        let subject_id = entitlement
            .as_ref()
            .and_then(|value| value.user_id.clone())
            .unwrap_or_else(|| "local-anonymous".to_string());
        let plan_id = entitlement.map(|value| value.plan_id);
        let db = Connection::open(&self.db_path)?;
        db.execute(
            "INSERT INTO local_usage_events (
                id, created_at_ms, day, subject_id, plan_id, method, path, upstream_protocol, status_code,
                request_bytes, response_bytes, estimated_tokens,
                reported_input_tokens, reported_output_tokens, reported_total_tokens
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                Uuid::new_v4().to_string(),
                now,
                day_key(now),
                subject_id,
                plan_id,
                event.method,
                event.path,
                event.upstream_protocol,
                i64::from(event.status_code),
                event.request_bytes as i64,
                event.response_bytes as i64,
                estimated_tokens,
                usage.and_then(|value| value.input_tokens),
                usage.and_then(|value| value.output_tokens),
                usage.and_then(|value| value.total_tokens),
            ],
        )?;
        Ok(())
    }

    fn ensure_schema(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Connection::open(&self.db_path)?;
        db.execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS local_usage_events (
	  id TEXT PRIMARY KEY,
	  created_at_ms INTEGER NOT NULL,
	  day TEXT NOT NULL,
	  subject_id TEXT NOT NULL DEFAULT 'local-legacy',
	  plan_id TEXT,
	  method TEXT NOT NULL,
	  path TEXT NOT NULL,
  upstream_protocol TEXT NOT NULL,
  status_code INTEGER NOT NULL,
  request_bytes INTEGER NOT NULL,
  response_bytes INTEGER NOT NULL,
  estimated_tokens INTEGER NOT NULL,
  reported_input_tokens INTEGER,
  reported_output_tokens INTEGER,
  reported_total_tokens INTEGER
);

	CREATE INDEX IF NOT EXISTS idx_local_usage_events_day_created
	  ON local_usage_events(day, created_at_ms DESC);

	CREATE INDEX IF NOT EXISTS idx_local_usage_events_subject_day
	  ON local_usage_events(subject_id, day, created_at_ms DESC);
	"#,
        )?;
        ensure_column(
            &db,
            "local_usage_events",
            "subject_id",
            "ALTER TABLE local_usage_events ADD COLUMN subject_id TEXT NOT NULL DEFAULT 'local-legacy'",
        )?;
        ensure_column(
            &db,
            "local_usage_events",
            "plan_id",
            "ALTER TABLE local_usage_events ADD COLUMN plan_id TEXT",
        )?;
        Ok(())
    }
}

impl LocalUsagePolicy {
    pub fn from_settings(settings: &BackendSettings) -> Self {
        Self::from_settings_with_account_store(
            settings,
            &crate::local_account::LocalAccountStore::default(),
        )
    }

    pub fn from_settings_with_account_store(
        settings: &BackendSettings,
        account_store: &crate::local_account::LocalAccountStore,
    ) -> Self {
        let settings_limit = settings.jiyi_daily_token_limit.max(0);
        let entitlement = account_store.load_active_entitlement().ok().flatten();
        if let Some(entitlement) = entitlement {
            let entitlement_limit = entitlement.daily_token_limit.max(0);
            return Self {
                enabled: settings.jiyi_local_usage_meter_enabled,
                daily_token_limit: if entitlement_limit > 0 {
                    entitlement_limit
                } else {
                    settings_limit
                },
                subject_id: entitlement.user_id,
                plan_id: Some(entitlement.plan_id),
                limit_source: if entitlement_limit > 0 {
                    "local_entitlement".to_string()
                } else if settings_limit > 0 {
                    "settings".to_string()
                } else {
                    "unlimited".to_string()
                },
            };
        }
        Self {
            enabled: settings.jiyi_local_usage_meter_enabled,
            daily_token_limit: settings_limit,
            subject_id: None,
            plan_id: None,
            limit_source: if settings_limit > 0 {
                "settings".to_string()
            } else {
                "unlimited".to_string()
            },
        }
    }
}

pub fn default_usage_db_path() -> PathBuf {
    crate::local_account::default_auth_db_path()
}

pub fn estimate_tokens_from_bytes(request_bytes: usize, response_bytes: usize) -> i64 {
    let total = request_bytes.saturating_add(response_bytes).max(1);
    total.div_ceil(4) as i64
}

pub fn token_usage_from_value(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let input_tokens = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(Value::as_i64);
    let output_tokens = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(Value::as_i64);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_i64)
        .or_else(|| {
            input_tokens
                .zip(output_tokens)
                .map(|(input, output)| input + output)
        });
    (input_tokens.is_some() || output_tokens.is_some() || total_tokens.is_some()).then_some(
        TokenUsage {
            input_tokens,
            output_tokens,
            total_tokens,
        },
    )
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn day_key(timestamp_ms: i64) -> String {
    let days = timestamp_ms.div_euclid(86_400_000);
    let date = time::Date::from_julian_day((days + 2_440_588) as i32).unwrap_or(time::Date::MIN);
    date.to_string()
}

fn ensure_column(
    db: &Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> anyhow::Result<()> {
    let mut statement = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(());
        }
    }
    db.execute(alter_sql, [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_usage_and_prefers_reported_total_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalUsageStore::new(temp.path().join("usage.sqlite"));
        let policy = LocalUsagePolicy {
            enabled: true,
            daily_token_limit: 1_000,
            subject_id: None,
            plan_id: None,
            limit_source: "settings".to_string(),
        };

        store
            .record_event(LocalUsageEvent {
                method: "POST".to_string(),
                path: "/v1/responses".to_string(),
                upstream_protocol: "responses".to_string(),
                status_code: 200,
                request_bytes: 80,
                response_bytes: 120,
                token_usage: Some(TokenUsage {
                    input_tokens: Some(11),
                    output_tokens: Some(17),
                    total_tokens: Some(28),
                }),
            })
            .unwrap();

        let snapshot = store.snapshot(policy).unwrap();
        assert_eq!(snapshot.used_tokens, 28);
        assert_eq!(snapshot.request_count, 1);
        assert_eq!(snapshot.remaining_tokens, Some(972));
    }

    #[test]
    fn preflight_blocks_when_daily_limit_would_be_exceeded() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalUsageStore::new(temp.path().join("usage.sqlite"));
        let policy = LocalUsagePolicy {
            enabled: true,
            daily_token_limit: 10,
            subject_id: None,
            plan_id: None,
            limit_source: "settings".to_string(),
        };

        let error = store.preflight_request(policy, 80).unwrap_err();

        assert!(error.to_string().contains("本地每日额度已用尽"));
    }

    #[test]
    fn extracts_usage_from_responses_or_chat_json() {
        let responses = serde_json::json!({
            "usage": { "input_tokens": 9, "output_tokens": 3, "total_tokens": 12 }
        });
        let chat = serde_json::json!({
            "usage": { "prompt_tokens": 5, "completion_tokens": 7 }
        });

        assert_eq!(
            token_usage_from_value(&responses).unwrap().total_tokens,
            Some(12)
        );
        assert_eq!(
            token_usage_from_value(&chat).unwrap().total_tokens,
            Some(12)
        );
    }

    #[test]
    fn policy_prefers_active_local_entitlement_limit() {
        let temp = tempfile::tempdir().unwrap();
        let auth_path = temp.path().join("auth.sqlite");
        let account_store = crate::local_account::LocalAccountStore::new(auth_path.clone());
        account_store.load_auth_state().unwrap();
        let db = Connection::open(auth_path).unwrap();
        let now = now_ms();
        db.execute(
            "INSERT OR REPLACE INTO auth_state (id, user_id, phone, session_token, login_at_ms, session_expires_at_ms, device_id)
             VALUES (1, 'user-1', '+8613812345678', 'token-1', ?1, ?2, 'device-1')",
            params![now, now + 60_000],
        )
        .unwrap();
        db.execute(
            "INSERT OR REPLACE INTO local_entitlements (user_id, plan_id, plan_name, daily_token_limit, updated_at_ms)
             VALUES ('user-1', 'team_trial', '团队试用', 123, ?1)",
            params![now],
        )
        .unwrap();
        let settings = BackendSettings {
            jiyi_daily_token_limit: 999,
            ..BackendSettings::default()
        };

        let policy = LocalUsagePolicy::from_settings_with_account_store(&settings, &account_store);

        assert_eq!(policy.daily_token_limit, 123);
        assert_eq!(policy.subject_id.as_deref(), Some("user-1"));
        assert_eq!(policy.plan_id.as_deref(), Some("team_trial"));
        assert_eq!(policy.limit_source, "local_entitlement");
    }

    #[test]
    fn export_summary_groups_usage_by_day_subject_and_plan() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("usage.sqlite");
        let store = LocalUsageStore::new(db_path.clone());
        store.ensure_schema().unwrap();
        let db = Connection::open(db_path).unwrap();
        db.execute(
            "INSERT INTO local_usage_events (
                id, created_at_ms, day, subject_id, plan_id, method, path, upstream_protocol, status_code,
                request_bytes, response_bytes, estimated_tokens,
                reported_input_tokens, reported_output_tokens, reported_total_tokens
             )
             VALUES ('event-1', 1000, '2026-06-09', 'user-1', 'team_basic', 'POST', '/v1/responses', 'responses', 200, 80, 120, 50, 10, 20, 30)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO local_usage_events (
                id, created_at_ms, day, subject_id, plan_id, method, path, upstream_protocol, status_code,
                request_bytes, response_bytes, estimated_tokens,
                reported_input_tokens, reported_output_tokens, reported_total_tokens
             )
             VALUES ('event-2', 2000, '2026-06-09', 'user-1', 'team_basic', 'POST', '/v1/responses', 'responses', 200, 40, 60, 25, NULL, NULL, NULL)",
            [],
        )
        .unwrap();

        let export = store.export_summary().unwrap();

        assert_eq!(export.summaries.len(), 1);
        let summary = &export.summaries[0];
        assert_eq!(summary.day, "2026-06-09");
        assert_eq!(summary.subject_id, "user-1");
        assert_eq!(summary.plan_id.as_deref(), Some("team_basic"));
        assert_eq!(summary.request_count, 2);
        assert_eq!(summary.reported_total_tokens, 30);
        assert_eq!(summary.effective_total_tokens, 55);
        assert_eq!(summary.first_seen_at_ms, 1000);
        assert_eq!(summary.last_seen_at_ms, 2000);
    }
}
