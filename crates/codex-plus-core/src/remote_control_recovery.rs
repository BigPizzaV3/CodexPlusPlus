use crate::settings::{RelayProfile, atomic_write};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRemoteControlRecovery {
    pub thread_id: String,
    pub profile_id: String,
    pub target_provider: String,
    pub config_generation: String,
    pub created_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingRemoteControlRecoveryState {
    #[serde(default = "state_version")]
    version: u32,
    #[serde(default)]
    requests: Vec<PendingRemoteControlRecovery>,
}

fn state_version() -> u32 {
    STATE_VERSION
}

fn state_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn config_generation(profile: &RelayProfile, target_provider: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(profile.id.trim().as_bytes());
    digest.update([0]);
    digest.update(profile.config_contents.as_bytes());
    digest.update([0]);
    digest.update(target_provider.trim().as_bytes());
    format!("{:x}", digest.finalize())
}

pub fn load_pending_remote_control_recoveries(
    path: Option<&Path>,
) -> anyhow::Result<Vec<PendingRemoteControlRecovery>> {
    let path = pending_path(path);
    let _guard = state_lock()
        .lock()
        .map_err(|_| anyhow::anyhow!("Remote Control recovery state lock poisoned"))?;
    Ok(load_state(&path)?.requests)
}

pub fn enqueue_pending_remote_control_recovery(
    path: Option<&Path>,
    request: PendingRemoteControlRecovery,
) -> anyhow::Result<()> {
    validate_request(&request)?;
    let path = pending_path(path);
    let _guard = state_lock()
        .lock()
        .map_err(|_| anyhow::anyhow!("Remote Control recovery state lock poisoned"))?;
    let mut state = load_state(&path)?;
    // The first observed profile/provider snapshot is authoritative. Retries for the same
    // thread must not overwrite it after the user switches relay profiles.
    if state
        .requests
        .iter()
        .any(|existing| existing.thread_id == request.thread_id)
    {
        return Ok(());
    }
    state.requests.push(request);
    save_state(&path, &state)
}

pub fn complete_pending_remote_control_recovery(
    path: Option<&Path>,
    thread_id: &str,
) -> anyhow::Result<()> {
    let path = pending_path(path);
    let _guard = state_lock()
        .lock()
        .map_err(|_| anyhow::anyhow!("Remote Control recovery state lock poisoned"))?;
    let mut state = load_state(&path)?;
    let original_len = state.requests.len();
    state
        .requests
        .retain(|request| request.thread_id != thread_id);
    if state.requests.len() == original_len {
        return Ok(());
    }
    save_state(&path, &state)
}

fn pending_path(path: Option<&Path>) -> PathBuf {
    path.map(Path::to_path_buf)
        .unwrap_or_else(crate::paths::default_pending_remote_control_recovery_path)
}

fn load_state(path: &Path) -> anyhow::Result<PendingRemoteControlRecoveryState> {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(state) => Ok(state),
            Err(_) => {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let corrupt_path = path.with_file_name(format!(
                    "{}.corrupt-{}-{timestamp}",
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("pending.json"),
                    std::process::id()
                ));
                let _ = std::fs::rename(path, corrupt_path);
                Ok(PendingRemoteControlRecoveryState {
                    version: STATE_VERSION,
                    requests: Vec::new(),
                })
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(PendingRemoteControlRecoveryState {
                version: STATE_VERSION,
                requests: Vec::new(),
            })
        }
        Err(error) => Err(error.into()),
    }
}

fn save_state(path: &Path, state: &PendingRemoteControlRecoveryState) -> anyhow::Result<()> {
    if state.requests.is_empty() {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
    atomic_write(path, serde_json::to_string_pretty(state)?.as_bytes())
}

fn validate_request(request: &PendingRemoteControlRecovery) -> anyhow::Result<()> {
    if request.thread_id.trim().is_empty() || request.thread_id.len() > 128 {
        anyhow::bail!("Remote Control recovery requires a valid thread id");
    }
    if request.profile_id.trim().is_empty() || request.target_provider.trim().is_empty() {
        anyhow::bail!("Remote Control recovery requires profile and provider provenance");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn request(thread_id: &str) -> PendingRemoteControlRecovery {
        PendingRemoteControlRecovery {
            thread_id: thread_id.to_string(),
            profile_id: "official-mix".to_string(),
            target_provider: "custom".to_string(),
            config_generation: "generation".to_string(),
            created_at: 1,
        }
    }

    #[test]
    fn pending_recovery_state_deduplicates_and_completes_by_thread() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pending.json");
        enqueue_pending_remote_control_recovery(Some(&path), request("one")).unwrap();
        let replacement = request("one");
        enqueue_pending_remote_control_recovery(Some(&path), replacement.clone()).unwrap();
        enqueue_pending_remote_control_recovery(Some(&path), request("two")).unwrap();

        assert_eq!(
            load_pending_remote_control_recoveries(Some(&path)).unwrap(),
            vec![request("one"), request("two")]
        );

        complete_pending_remote_control_recovery(Some(&path), "one").unwrap();
        assert_eq!(
            load_pending_remote_control_recoveries(Some(&path)).unwrap(),
            vec![request("two")]
        );
        complete_pending_remote_control_recovery(Some(&path), "two").unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn corrupt_pending_recovery_state_is_quarantined() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pending.json");
        std::fs::write(&path, "{broken").unwrap();

        assert!(
            load_pending_remote_control_recoveries(Some(&path))
                .unwrap()
                .is_empty()
        );
        assert!(!path.exists());
        assert!(std::fs::read_dir(dir.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("pending.json.corrupt-")
        }));
    }
}
