//! Local, bounded observations from existing subscription refreshes.

use std::{path::Path, time::Duration};

use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::settings::RelayProfile;

use super::quota::{ProviderQuota, QuotaWindow};

const RETENTION_MS: u64 = 35 * 24 * 60 * 60 * 1000;
const MINUTE_MS: u64 = 60 * 1000;
const POINTS_PER_WINDOW: u32 = 384;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaObservation {
    pub window_minutes: Option<u64>,
    pub resets_at_ms: Option<u64>,
    pub observed_at_ms: u64,
    pub used_percent: f64,
}

fn database_path() -> std::path::PathBuf {
    crate::paths::default_app_state_dir().join("gateway-quota-history.sqlite3")
}

fn scope_parts(relay: &RelayProfile) -> Option<(String, String)> {
    if crate::official_login::member_uses_official_login_transport(relay) {
        return crate::native_auth::account_scope(&relay.auth_contents).map(|scope| ("official".into(), scope));
    }
    if let Some(kind) = super::accounts::relay_account_kind(relay) {
        let field = match kind.as_str() {
            "copilot" => "login",
            "grok" => "email",
            _ => return None,
        };
        let auth: serde_json::Value = serde_json::from_str(&relay.auth_contents).ok()?;
        let identity = auth.get(field)?.as_str()?.trim();
        return (!identity.is_empty()).then(|| (kind, identity.to_ascii_lowercase()));
    }
    super::coding_plan::plan_for_relay(relay)?;
    let secret = relay.api_key.trim();
    (!secret.is_empty()).then(|| (format!("api:{}", relay.base_url.trim()), secret.to_string()))
}

fn fingerprint(salt: &[u8], scope: &(String, String)) -> Vec<u8> {
    let mut digest = Sha256::new();
    for part in [salt, scope.0.as_bytes(), scope.1.as_bytes()] {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part);
    }
    digest.finalize().to_vec()
}

fn salt(connection: &Connection) -> rusqlite::Result<Vec<u8>> {
    connection.query_row("SELECT salt FROM history_meta WHERE id = 1", [], |row| {
        row.get(0)
    })
}

fn writable(path: &Path) -> anyhow::Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_millis(250))?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS history_meta (id INTEGER PRIMARY KEY, salt BLOB NOT NULL);
         INSERT OR IGNORE INTO history_meta VALUES (1, randomblob(32));
         CREATE TABLE IF NOT EXISTS history_points (
           scope BLOB NOT NULL, minutes INTEGER NOT NULL, bucket INTEGER NOT NULL,
           observed INTEGER NOT NULL, reset INTEGER, used REAL NOT NULL,
           PRIMARY KEY (scope, minutes, bucket)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS history_retention ON history_points (observed);",
    )?;
    Ok(connection)
}

fn point(window: &QuotaWindow, updated_at: u64) -> Option<QuotaObservation> {
    let observed = window.observed_at_ms.unwrap_or(updated_at);
    if observed == 0
        || i64::try_from(observed).is_err()
        || window
            .window_minutes
            .is_some_and(|minutes| minutes == 0 || minutes > 525_600)
        || window
            .resets_at_ms
            .is_some_and(|reset| i64::try_from(reset).is_err() || reset <= observed)
        || !(0.0..=100.0).contains(&window.used_percent)
    {
        return None;
    }
    Some(QuotaObservation {
        window_minutes: window.window_minutes,
        resets_at_ms: window.resets_at_ms,
        observed_at_ms: observed,
        used_percent: window.used_percent,
    })
}

fn write(path: &Path, scope: &(String, String), quota: &ProviderQuota) -> anyhow::Result<()> {
    let mut connection = writable(path)?;
    let scope = fingerprint(&salt(&connection)?, scope);
    let transaction = connection.transaction()?;
    for observation in quota
        .windows
        .iter()
        .filter_map(|window| point(window, quota.updated_at_ms))
    {
        let observed = observation.observed_at_ms;
        let minutes = observation.window_minutes.unwrap_or(0);
        let interval = match minutes {
            1..=720 => 5 * MINUTE_MS,
            721..=10_080 => 30 * MINUTE_MS,
            _ => 150 * MINUTE_MS,
        };
        transaction.execute(
            "INSERT INTO history_points VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(scope, minutes, bucket) DO UPDATE SET
               observed = excluded.observed, reset = excluded.reset, used = excluded.used
             WHERE excluded.observed > history_points.observed",
            params![
                scope,
                minutes,
                observed / interval * interval,
                observed,
                observation.resets_at_ms,
                observation.used_percent
            ],
        )?;
        transaction.execute(
            "DELETE FROM history_points WHERE scope = ?1 AND minutes = ?2 AND bucket IN (
                SELECT bucket FROM history_points WHERE scope = ?1 AND minutes = ?2
                ORDER BY bucket DESC LIMIT -1 OFFSET ?3
             )",
            params![scope, minutes, POINTS_PER_WINDOW],
        )?;
    }
    transaction.execute(
        "DELETE FROM history_points WHERE observed < ?1",
        [quota.updated_at_ms.saturating_sub(RETENTION_MS)],
    )?;
    transaction.commit()?;
    Ok(())
}

#[cfg(not(test))]
pub(super) async fn record_refresh(relay: &RelayProfile, quota: &ProviderQuota) {
    if quota.windows.is_empty() {
        return;
    }
    let Some(scope) = scope_parts(relay) else {
        return;
    };
    let quota = quota.clone();
    let _ = tokio::task::spawn_blocking(move || write(&database_path(), &scope, &quota)).await;
}

fn read(path: &Path, scope: &(String, String), now: u64) -> anyhow::Result<Vec<QuotaObservation>> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::from_millis(100))?;
    let scope = fingerprint(&salt(&connection)?, scope);
    let mut statement = connection.prepare(
        "SELECT minutes, observed, reset, used FROM history_points
         WHERE scope = ?1 AND observed BETWEEN ?2 AND ?3
         ORDER BY bucket DESC, minutes LIMIT 1536",
    )?;
    let observations = statement.query_map(
        params![scope, now.saturating_sub(RETENTION_MS), now],
        |row| {
            let minutes = row.get::<_, u64>(0)?;
            Ok(QuotaObservation {
                window_minutes: (minutes != 0).then_some(minutes),
                observed_at_ms: row.get(1)?,
                resets_at_ms: row.get(2)?,
                used_percent: row.get(3)?,
            })
        },
    )?;
    Ok(observations.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn for_relay(relay: &RelayProfile) -> anyhow::Result<Vec<QuotaObservation>> {
    let Some(scope) = scope_parts(relay) else {
        return Ok(Vec::new());
    };
    let path = database_path();
    if !path.try_exists()? {
        return Ok(Vec::new());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    read(&path, &scope, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_survive_reopen_and_are_scoped_to_the_credential() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("quota.sqlite3");
        let first = ("test".into(), "first".into());
        let other = ("test".into(), "other".into());
        let mut quota = ProviderQuota {
            updated_at_ms: 1000,
            windows: vec![QuotaWindow {
                window_minutes: Some(300),
                used_percent: 10.0,
                resets_at_ms: Some(9_000_000),
                observed_at_ms: None,
            }],
            ..ProviderQuota::default()
        };
        write(&path, &first, &quota).unwrap();
        quota.updated_at_ms = 2000;
        quota.windows[0].used_percent = 11.0;
        write(&path, &first, &quota).unwrap();
        quota.updated_at_ms = 1500;
        quota.windows[0].used_percent = 9.0;
        write(&path, &first, &quota).unwrap();
        assert!(read(&path, &other, 3000).unwrap().is_empty());
        let points = read(&path, &first, 3000).unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].used_percent, 11.0);
        assert!(read(&path, &first, RETENTION_MS + 3000).unwrap().is_empty());
    }
}
