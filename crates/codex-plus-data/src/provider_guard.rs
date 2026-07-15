use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, value};
use url::Url;

use crate::{ProviderSyncStatus, run_provider_sync_with_target};

pub const STABLE_PROVIDER_ID: &str = "custom";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderBucket {
    pub provider: String,
    pub threads: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEndpoint {
    pub kind: String,
    pub loopback: bool,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderGuardFinding {
    pub code: String,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderGuardStatus {
    pub level: String,
    pub stable_provider: String,
    pub current_provider: String,
    pub stable_provider_configured: bool,
    pub total_threads: usize,
    pub databases_scanned: usize,
    pub provider_buckets: Vec<ProviderBucket>,
    pub endpoint: ProviderEndpoint,
    pub findings: Vec<ProviderGuardFinding>,
    pub can_repair: bool,
    pub repair_requires_native_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderGuardRepairResult {
    pub outcome: String,
    pub message: String,
    pub backup_dir: Option<PathBuf>,
    pub sync_backup_dir: Option<PathBuf>,
    pub changed_session_files: usize,
    pub sqlite_rows_updated: usize,
    pub guard: ProviderGuardStatus,
}

pub fn inspect_provider_guard(codex_home: Option<&Path>) -> anyhow::Result<ProviderGuardStatus> {
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(codex_plus_core::codex_home::default_codex_home_dir);
    let config_path = home.join("config.toml");
    let config_text = fs::read_to_string(&config_path).context("failed to read Codex config")?;
    let config = config_text
        .parse::<toml::Value>()
        .context("failed to parse Codex config")?;

    let current_provider = config
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
        .unwrap_or("openai")
        .to_string();
    let stable_provider_configured = provider_config(&config, STABLE_PROVIDER_ID).is_some();
    let endpoint = endpoint_for_provider(&config, &current_provider);

    let mut bucket_counts = BTreeMap::<String, usize>::new();
    let mut databases_scanned = 0;
    let mut database_failures = 0;
    for db_path in codex_plus_core::codex_sqlite::codex_session_db_paths_from_home(&home) {
        match read_provider_buckets(&db_path) {
            Ok(Some(buckets)) => {
                databases_scanned += 1;
                for (provider, count) in buckets {
                    *bucket_counts.entry(provider).or_default() += count;
                }
            }
            Ok(None) => {}
            Err(_) => database_failures += 1,
        }
    }
    let provider_buckets = bucket_counts
        .iter()
        .map(|(provider, threads)| ProviderBucket {
            provider: provider.clone(),
            threads: *threads,
        })
        .collect::<Vec<_>>();
    let total_threads = provider_buckets.iter().map(|bucket| bucket.threads).sum();

    let mut findings = Vec::new();
    if stable_provider_configured && current_provider != STABLE_PROVIDER_ID {
        findings.push(finding(
            "unstable_current_provider",
            "critical",
            format!(
                "Current model_provider is {current_provider:?}; stable session visibility requires {STABLE_PROVIDER_ID:?}."
            ),
        ));
    }
    if !stable_provider_configured {
        findings.push(finding(
            "stable_provider_missing",
            "warning",
            "The custom provider configuration is missing. Provider Guard will remain read-only and automatic repair is disabled.",
        ));
    }
    let foreign_threads = provider_buckets
        .iter()
        .filter(|bucket| bucket.provider != STABLE_PROVIDER_ID)
        .map(|bucket| bucket.threads)
        .sum::<usize>();
    if foreign_threads > 0 {
        findings.push(finding(
            "provider_buckets_diverged",
            "warning",
            format!(
                "{foreign_threads} thread index row(s) are stored outside the stable {STABLE_PROVIDER_ID:?} bucket."
            ),
        ));
    }
    if total_threads == 0 {
        findings.push(finding(
            "no_threads_detected",
            "warning",
            "No indexed threads were detected. Verify the active CODEX_HOME before repairing.",
        ));
    }
    if database_failures > 0 {
        findings.push(finding(
            "database_read_failed",
            "warning",
            format!("{database_failures} session database(s) could not be inspected read-only."),
        ));
    }
    if endpoint.port == Some(6269) {
        findings.push(finding(
            "known_cockpit_port",
            "warning",
            "The active provider uses local port 6269, which is commonly owned by Cockpit Tools on this machine.",
        ));
    }

    let level = if findings.iter().any(|item| item.severity == "critical") {
        "critical"
    } else if findings.iter().any(|item| item.severity == "warning") {
        "warning"
    } else {
        "ok"
    };
    Ok(ProviderGuardStatus {
        level: level.to_string(),
        stable_provider: STABLE_PROVIDER_ID.to_string(),
        current_provider,
        stable_provider_configured,
        total_threads,
        databases_scanned,
        provider_buckets,
        endpoint,
        findings,
        can_repair: stable_provider_configured,
        repair_requires_native_confirmation: true,
    })
}

pub fn repair_provider_guard(codex_home: Option<&Path>) -> anyhow::Result<ProviderGuardRepairResult> {
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(codex_plus_core::codex_home::default_codex_home_dir);
    let _lock = GuardLock::acquire(&home.join("tmp/provider-guard.lock"))?;
    let before = inspect_provider_guard(Some(&home))?;
    if !before.stable_provider_configured {
        anyhow::bail!("refusing repair because [model_providers.custom] is not configured");
    }

    let config_path = home.join("config.toml");
    let original_config = fs::read(&config_path)
        .context("failed to read Codex config")?;
    let backup_dir = create_guard_backup(&home, &original_config)?;
    let next_config = set_root_provider(&original_config, STABLE_PROVIDER_ID)?;
    if fs::read(&config_path).context("failed to re-check Codex config before repair")?
        != original_config
    {
        anyhow::bail!("Codex config changed during repair preparation; no changes were applied");
    }
    if next_config != original_config {
        codex_plus_core::settings::atomic_write(&config_path, &next_config)
            .context("failed to write stable model_provider")?;
    }

    let sync = run_provider_sync_with_target(Some(&home), Some(STABLE_PROVIDER_ID));
    if sync.status != ProviderSyncStatus::Synced {
        let current_config = fs::read(&config_path).unwrap_or_default();
        if current_config == next_config {
            codex_plus_core::settings::atomic_write(&config_path, &original_config)
                .context("provider repair failed and the original config could not be restored")?;
            anyhow::bail!("provider repair was rolled back: {}", sync.message);
        }
        anyhow::bail!(
            "provider repair failed and config changed externally; the safety backup was retained: {}",
            sync.message
        );
    }

    let guard = inspect_provider_guard(Some(&home))?;
    Ok(ProviderGuardRepairResult {
        outcome: "repaired".to_string(),
        message: "Provider guard repair completed with backups.".to_string(),
        backup_dir: Some(backup_dir),
        sync_backup_dir: sync.backup_dir,
        changed_session_files: sync.changed_session_files,
        sqlite_rows_updated: sync.sqlite_rows_updated,
        guard,
    })
}

fn provider_config<'a>(config: &'a toml::Value, provider: &str) -> Option<&'a toml::Value> {
    config
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .and_then(|providers| providers.get(provider))
}

fn endpoint_for_provider(config: &toml::Value, provider: &str) -> ProviderEndpoint {
    let base_url = provider_config(config, provider)
        .and_then(|provider| provider.get("base_url"))
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    let Ok(url) = Url::parse(base_url) else {
        return ProviderEndpoint {
            kind: "unknown".to_string(),
            loopback: false,
            port: None,
        };
    };
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    let port = url.port_or_known_default();
    let kind = match (loopback, port) {
        (true, Some(8317)) => "cpa",
        (true, Some(6269)) => "cockpit",
        (true, _) => "loopback",
        (false, _) => "remote",
    };
    ProviderEndpoint {
        kind: kind.to_string(),
        loopback,
        port,
    }
}

fn read_provider_buckets(path: &Path) -> anyhow::Result<Option<Vec<(String, usize)>>> {
    if !path.is_file() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let has_provider_column = connection
        .query_row(
            "SELECT 1 FROM pragma_table_info('threads') WHERE name = 'model_provider' LIMIT 1",
            [],
            |_| Ok(()),
        )
        .is_ok();
    if !has_provider_column {
        return Ok(None);
    }
    let mut statement = connection.prepare(
        "SELECT COALESCE(NULLIF(model_provider, ''), '<empty>'), COUNT(*) FROM threads GROUP BY COALESCE(NULLIF(model_provider, ''), '<empty>')",
    )?;
    let rows = statement
        .query_map([], |row| {
            let count = row.get::<_, i64>(1)?.max(0) as usize;
            Ok((row.get::<_, String>(0)?, count))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(rows))
}

fn create_guard_backup(home: &Path, config: &[u8]) -> anyhow::Result<PathBuf> {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let backup_dir = home
        .join("backups_state")
        .join("provider-guard")
        .join(format!("{timestamp}-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&backup_dir).context("failed to create provider guard backup directory")?;
    codex_plus_core::settings::atomic_write(&backup_dir.join("config.toml"), config)
        .context("failed to back up Codex config")?;
    Ok(backup_dir)
}

fn set_root_provider(config: &[u8], provider: &str) -> anyhow::Result<Vec<u8>> {
    let text = std::str::from_utf8(config).context("Codex config is not valid UTF-8")?;
    let mut document = text
        .parse::<DocumentMut>()
        .context("failed to parse Codex config for repair")?;
    document["model_provider"] = value(provider);
    Ok(document.to_string().into_bytes())
}

fn finding(code: &str, severity: &str, message: impl Into<String>) -> ProviderGuardFinding {
    ProviderGuardFinding {
        code: code.to_string(),
        severity: severity.to_string(),
        message: message.into(),
    }
}

struct GuardLock {
    path: PathBuf,
}

impl GuardLock {
    fn acquire(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("failed to create provider guard lock parent")?;
        }
        fs::create_dir(path).context("another Provider Guard repair is already running")?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for GuardLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_threads_db(path: &Path, providers: &[(&str, usize)]) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT)",
                [],
            )
            .unwrap();
        let mut id = 0;
        for (provider, count) in providers {
            for _ in 0..*count {
                id += 1;
                connection
                    .execute(
                        "INSERT INTO threads (id, model_provider) VALUES (?1, ?2)",
                        (format!("thread-{id}"), *provider),
                    )
                    .unwrap();
            }
        }
    }

    #[test]
    fn status_detects_provider_drift_without_exposing_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fs::write(
            home.join("config.toml"),
            r#"model_provider = "apex"

[model_providers.apex]
base_url = "https://user:password@example.test/v1?api_key=secret"

[model_providers.custom]
base_url = "http://127.0.0.1:8317/v1"
api_key = "top-secret"
"#,
        )
        .unwrap();
        write_threads_db(&home.join("state_5.sqlite"), &[("custom", 3), ("apex", 2)]);

        let status = inspect_provider_guard(Some(home)).unwrap();
        let serialized = serde_json::to_string(&status).unwrap();

        assert_eq!(status.level, "critical");
        assert_eq!(status.current_provider, "apex");
        assert_eq!(status.total_threads, 5);
        assert!(status.can_repair);
        assert!(status.findings.iter().any(|item| item.code == "unstable_current_provider"));
        assert!(!serialized.contains("top-secret"));
        assert!(!serialized.contains("password"));
        assert!(!serialized.contains("example.test"));
    }

    #[test]
    fn repair_refuses_when_custom_provider_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fs::write(
            home.join("config.toml"),
            "model_provider = \"apex\"\n[model_providers.apex]\nbase_url = \"https://example.test/v1\"\n",
        )
        .unwrap();
        write_threads_db(&home.join("state_5.sqlite"), &[("apex", 1)]);

        let error = repair_provider_guard(Some(home)).unwrap_err().to_string();

        assert!(error.contains("model_providers.custom"));
        assert!(!home.join("backups_state/provider-guard").exists());
    }

    #[test]
    fn repair_backs_up_config_and_normalizes_provider_buckets() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::write(
            home.join("config.toml"),
            r#"model_provider = "apex"
[model_providers.apex]
base_url = "https://example.test/v1"
[model_providers.custom]
base_url = "http://127.0.0.1:8317/v1"
"#,
        )
        .unwrap();
        fs::write(
            home.join("sessions/rollout-test.jsonl"),
            r#"{"type":"session_meta","payload":{"id":"thread-1","model_provider":"apex","cwd":"C:/workspace"}}
{"type":"event_msg","payload":{"type":"user_message","message":"hello"}}
"#,
        )
        .unwrap();
        write_threads_db(&home.join("state_5.sqlite"), &[("apex", 1)]);

        let result = repair_provider_guard(Some(home)).unwrap();

        assert_eq!(result.outcome, "repaired");
        assert_eq!(result.guard.current_provider, STABLE_PROVIDER_ID);
        assert_eq!(result.guard.provider_buckets[0].provider, STABLE_PROVIDER_ID);
        let backup = result.backup_dir.unwrap().join("config.toml");
        assert!(backup.is_file());
        assert!(fs::read_to_string(backup).unwrap().contains("model_provider = \"apex\""));
        assert!(
            fs::read_to_string(home.join("config.toml"))
                .unwrap()
                .contains("model_provider = \"custom\"")
        );
    }
}
