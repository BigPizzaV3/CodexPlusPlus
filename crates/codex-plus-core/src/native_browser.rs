//! Opt-in adaptation of a pinned native Edge identification callback.
//! Does not implement browser execution, cloud identity or approval decisions.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SERVICE: &str = "bin/node_modules/@oai/browser-desktop/scripts/browser-service.mjs";
const ORIGINAL_SHA: &str = "3e6fd4a8cf09f57549d63f2c9cbfa2abf42f0a6b0c09c3d6605fe07c8ba09e4a";
const NATIVE_SHA: &str = "ef53f8f0d957b7cf437020499b6b9d880dee381214788930107b549237f7949c";
const ANCHOR: &str = "new nf(r,this.clientApi,()=>ze(this.runtime),this.turnEndedTracker,cD)";
const HELPER: &str = include_str!("../../../assets/native-browser/require-identification.mjs");
const MAX_SERVICE: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct BrowserPaths {
    pub codex_home: PathBuf,
    pub runtime_root: PathBuf,
    pub state_root: PathBuf,
}

impl BrowserPaths {
    pub fn current() -> Result<Self> {
        ensure!(
            cfg!(windows),
            "Native browser compatibility is Windows-only"
        );
        let local = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable")?;
        Ok(Self {
            codex_home: crate::codex_home::default_codex_home_dir(),
            runtime_root: PathBuf::from(local).join("OpenAI/Codex/runtimes/cua_node"),
            state_root: crate::paths::default_app_state_dir().join("native-browser-identification"),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub state: String,
    pub detail: String,
}

impl BrowserStatus {
    fn new(state: &str, detail: &str) -> Self {
        Self {
            state: state.into(),
            detail: detail.into(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    schema: u32,
    original_sha: String,
    candidate_sha: String,
    modified_secs: u64,
    modified_nanos: u32,
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn key_valid(key: &str) -> bool {
    key.len() == 16 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

// Reject junctions as well as symlinks, including in parent directories.
fn plain_path(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "Expected an absolute local path");
    for ancestor in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(ancestor) {
            ensure!(
                !meta.file_type().is_symlink(),
                "Linked paths are unsupported"
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    meta.file_attributes() & 0x400 == 0,
                    "Reparse paths are unsupported"
                );
            }
        }
    }
    ensure!(
        !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "Parent traversal is unsupported"
    );
    Ok(())
}

fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>> {
    plain_path(path)?;
    let meta = fs::metadata(path)?;
    ensure!(
        meta.is_file() && meta.len() <= limit,
        "Unexpected file type or size"
    );
    Ok(fs::read(path)?)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    plain_path(path)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    plain_path(path)?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    write_new(&temp, bytes)?;
    #[cfg(windows)]
    let result = {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        use windows::core::PCWSTR;
        let source: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(anyhow::Error::from)
    };
    #[cfg(not(windows))]
    let result = fs::rename(&temp, path).map_err(anyhow::Error::from);
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn transform(source: &[u8], control: &Path) -> Result<Vec<u8>> {
    ensure!(
        sha(source) == ORIGINAL_SHA,
        "Unsupported native browser service hash"
    );
    transform_binding(source, control)
}

fn transform_binding(source: &[u8], control: &Path) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(source)?;
    ensure!(
        text.matches(ANCHOR).count() == 1,
        "Expected one callback binding"
    );
    ensure!(
        !text.contains("cppNativeIdentificationReader"),
        "Conflicting adapter"
    );
    let path = serde_json::to_string(&control.to_str().context("Non-Unicode control path")?)?;
    let replacement = format!(
        "new nf(r,this.clientApi,()=>ze(this.runtime),this.turnEndedTracker,cppNativeIdentificationReader(this.runtime,cD,ze,{path}))"
    );
    Ok(format!("{}\n{HELPER}", text.replacen(ANCHOR, &replacement, 1)).into_bytes())
}

fn selected_key(descriptor: &Value, root: &Path) -> Result<String> {
    let server = &descriptor["mcpServers"]["cua_repl"];
    let node = PathBuf::from(
        server["command"]
            .as_str()
            .context("Missing native Node command")?,
    );
    let relative = node
        .strip_prefix(root)
        .context("Native Node is outside the runtime cache")?;
    let parts: Vec<_> = relative.components().collect();
    ensure!(parts.len() == 3, "Unexpected native runtime layout");
    let key = parts[0]
        .as_os_str()
        .to_str()
        .context("Invalid runtime key")?;
    ensure!(
        key_valid(key) && relative == Path::new(key).join("bin/node.exe"),
        "Unexpected native runtime layout"
    );
    let env = &server["env"];
    ensure!(
        env["NODE_REPL_NODE_PATH"].as_str() == node.to_str(),
        "Conflicting native Node selection"
    );
    let services: Value = serde_json::from_str(
        env["NODE_REPL_TRUSTED_SERVICES"]
            .as_str()
            .context("Missing native service map")?,
    )?;
    ensure!(
        services["browser"] == "@oai/browser-desktop/service",
        "Original browser service is not selected"
    );
    ensure!(
        env["CUA_REPL_ENABLED_SURFACES"].as_str() == Some("browser"),
        "Unsupported native surface selection"
    );
    let args = server["args"]
        .as_array()
        .context("Missing native entry point")?;
    let entry = root
        .join(key)
        .join("bin/node_modules/@oai/cua-repl/bin/cua-repl.mjs");
    ensure!(
        args.len() == 1 && args[0].as_str() == entry.to_str(),
        "Unsupported native entry point"
    );
    plain_path(&node)?;
    Ok(key.to_owned())
}

fn discover(paths: &BrowserPaths) -> Result<Option<String>> {
    let plugins = paths
        .codex_home
        .join("plugins/cache/openai-bundled/unified-computer-use");
    plain_path(&plugins)?;
    if !plugins.exists() {
        return Ok(None);
    }
    let mut keys = BTreeSet::new();
    let mut count = 0;
    for entry in fs::read_dir(plugins)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        count += 1;
        ensure!(count <= 64, "Too many plugin descriptors");
        let descriptor = entry.path().join(".mcp.json");
        if !descriptor.exists() {
            continue;
        }
        let data: Value = serde_json::from_slice(&read_regular(&descriptor, 1024 * 1024)?)?;
        keys.insert(selected_key(&data, &paths.runtime_root)?);
    }
    ensure!(
        keys.len() <= 1,
        "Ambiguous runtime selection; no cache was modified"
    );
    Ok(keys.into_iter().next())
}

fn prepare(paths: &BrowserPaths, key: &str) -> Result<()> {
    ensure!(key_valid(key), "Invalid runtime key");
    let runtime = paths.runtime_root.join(key);
    let target = runtime.join(SERVICE);
    ensure!(
        sha(&read_regular(
            &runtime.join("bin/node_repl.exe"),
            128 * 1024 * 1024
        )?) == NATIVE_SHA,
        "Unsupported native worker hash"
    );
    let current = read_regular(&target, MAX_SERVICE)?;
    let backup_dir = paths.state_root.join(key);
    plain_path(&backup_dir)?;
    fs::create_dir_all(&backup_dir)?;
    let backup = backup_dir.join("original.mjs");
    let journal_path = backup_dir.join("journal.json");
    let control = paths.state_root.join("control.json");
    if !journal_path.exists() {
        let candidate = transform(&current, &control)?;
        if backup.exists() {
            ensure!(
                read_regular(&backup, MAX_SERVICE)? == current,
                "Unjournaled backup conflict"
            );
        } else {
            write_new(&backup, &current)?;
        }
        let modified = fs::metadata(&target)?
            .modified()?
            .duration_since(UNIX_EPOCH)?;
        let journal = Journal {
            schema: 1,
            original_sha: ORIGINAL_SHA.into(),
            candidate_sha: sha(&candidate),
            modified_secs: modified.as_secs(),
            modified_nanos: modified.subsec_nanos(),
        };
        // Durable original and journal precede any runtime write.
        atomic_write(&journal_path, &serde_json::to_vec(&journal)?)?;
    }
    let (journal, original, candidate) = recovery_material(paths, key)?;
    if current == candidate {
        return Ok(());
    }
    ensure!(
        current == original && sha(&current) == journal.original_sha,
        "Runtime changed outside Codex++; refusing to overwrite"
    );
    ensure!(
        read_regular(&target, MAX_SERVICE)? == current,
        "Concurrent runtime change"
    );
    atomic_write(&target, &candidate)?;
    ensure!(
        read_regular(&target, MAX_SERVICE)? == candidate,
        "Runtime write verification failed"
    );
    Ok(())
}

fn recovery_material(paths: &BrowserPaths, key: &str) -> Result<(Journal, Vec<u8>, Vec<u8>)> {
    ensure!(key_valid(key), "Invalid recovery key");
    let dir = paths.state_root.join(key);
    let journal: Journal = serde_json::from_slice(&read_regular(&dir.join("journal.json"), 4096)?)?;
    let original = read_regular(&dir.join("original.mjs"), MAX_SERVICE)?;
    let candidate = transform(&original, &paths.state_root.join("control.json"))?;
    ensure!(
        journal.schema == 1
            && journal.original_sha == ORIGINAL_SHA
            && journal.candidate_sha == sha(&candidate)
            && journal.modified_nanos < 1_000_000_000,
        "Recovery journal conflicts with verified content"
    );
    Ok((journal, original, candidate))
}

fn restore_all(paths: &BrowserPaths, keep: Option<&str>) -> Result<()> {
    for entry in fs::read_dir(&paths.state_root)? {
        let entry = entry?;
        let key = entry.file_name().to_string_lossy().to_string();
        if !key_valid(&key) || keep == Some(key.as_str()) {
            continue;
        }
        let dir = entry.path();
        plain_path(&dir)?;
        if !dir.join("journal.json").exists() {
            continue;
        }
        let (journal, original, candidate) = recovery_material(paths, &key)?;
        let target = paths.runtime_root.join(&key).join(SERVICE);
        if !target.exists() {
            continue; // Desktop owns cache deletion; never resurrect an obsolete runtime.
        }
        let current = read_regular(&target, MAX_SERVICE)?;
        ensure!(
            current == original || current == candidate,
            "External runtime change prevents recovery"
        );
        if current == candidate {
            ensure!(
                read_regular(&target, MAX_SERVICE)? == current,
                "Concurrent recovery change"
            );
            atomic_write(&target, &original)?;
        }
        let modified = UNIX_EPOCH
            .checked_add(Duration::new(journal.modified_secs, journal.modified_nanos))
            .context("Invalid recovery timestamp")?;
        File::options()
            .write(true)
            .open(&target)?
            .set_modified(modified)?;
        ensure!(
            read_regular(&target, MAX_SERVICE)? == original,
            "Recovery verification failed"
        );
    }
    Ok(())
}

/// No runtime operation occurs when this feature has never been enabled.
/// Call only from the owning launcher, never from settings save or status inspection.
pub fn reconcile(paths: &BrowserPaths, enabled: bool) -> Result<BrowserStatus> {
    plain_path(&paths.runtime_root)?;
    plain_path(&paths.state_root)?;
    ensure!(
        !paths.state_root.starts_with(&paths.runtime_root),
        "Backups must be outside the cache"
    );
    if !enabled && !paths.state_root.exists() {
        return Ok(BrowserStatus::new("disabled", "Not configured"));
    }
    fs::create_dir_all(&paths.state_root)?;
    let lock_path = paths.state_root.join("owner.lock");
    plain_path(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    lock.try_lock_exclusive()
        .context("Another compatibility transaction is active")?;
    let result = reconcile_locked(paths, enabled);
    if result.is_err() {
        // Fail closed for an already-loaded helper as well as future workers.
        let _ = atomic_write(
            &paths.state_root.join("control.json"),
            br#"{"schema":1,"requireIdentification":false}"#,
        );
    }
    result
}

fn reconcile_locked(paths: &BrowserPaths, enabled: bool) -> Result<BrowserStatus> {
    let control = paths.state_root.join("control.json");
    if !enabled {
        atomic_write(&control, br#"{"schema":1,"requireIdentification":false}"#)?;
        restore_all(paths, None)?;
        return Ok(BrowserStatus::new(
            "restored",
            "Service restored; extension identification may remain enabled",
        ));
    }
    let Some(key) = discover(paths)? else {
        atomic_write(&control, br#"{"schema":1,"requireIdentification":false}"#)?;
        return Ok(BrowserStatus::new(
            "waiting_for_runtime",
            "Waiting for a native browser runtime descriptor",
        ));
    };
    restore_all(paths, Some(&key))?;
    prepare(paths, &key)?;
    atomic_write(&control, br#"{"schema":1,"requireIdentification":true}"#)?;
    Ok(BrowserStatus::new(
        "prepared",
        "Prepared for a new native worker; browser operation is not yet verified",
    ))
}

pub fn read_status() -> BrowserStatus {
    let Ok(paths) = BrowserPaths::current() else {
        return BrowserStatus::new("unsupported", "Windows-only experimental compatibility");
    };
    let path = paths.state_root.join("status.json");
    if fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age > Duration::from_secs(90))
    {
        return BrowserStatus::new(
            "stale",
            "No recent launcher status; browser functionality is unverified",
        );
    }
    read_regular(&path, 8192)
        .and_then(|data| Ok(serde_json::from_slice(&data)?))
        .unwrap_or_else(|_| {
            BrowserStatus::new("not_started", "Waiting for the next Codex++ launcher start")
        })
}

/// The singleton launcher owns this task. Existing-instance activation does not start another one.
/// The startup snapshot intentionally requires a launcher restart to apply a saved choice.
pub async fn start_monitor(enabled: bool) -> Option<tokio::task::JoinHandle<()>> {
    let paths = BrowserPaths::current().ok()?;
    if !enabled && !paths.state_root.exists() {
        return None;
    }
    let initial = monitor_once(paths.clone(), enabled).await;
    Some(tokio::spawn(async move {
        let mut previous = initial.ok();
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let outcome = monitor_once(paths.clone(), enabled).await;
            if let Ok(status) = outcome {
                if previous.as_ref() != Some(&status) {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "native_browser.compatibility",
                        json!(status),
                    );
                    previous = Some(status);
                }
            }
        }
    }))
}

async fn monitor_once(paths: BrowserPaths, enabled: bool) -> Result<BrowserStatus> {
    tokio::task::spawn_blocking(move || {
        let status = reconcile(&paths, enabled).unwrap_or_else(|error| {
            BrowserStatus::new("blocked", &format!("Compatibility refused: {error}"))
        });
        if paths.state_root.exists() {
            let _ = atomic_write(
                &paths.state_root.join("status.json"),
                &serde_json::to_vec(&status).unwrap_or_default(),
            );
        }
        status
    })
    .await
    .map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(temp: &tempfile::TempDir) -> BrowserPaths {
        BrowserPaths {
            codex_home: temp.path().join("home"),
            runtime_root: temp.path().join("cache"),
            state_root: temp.path().join("state"),
        }
    }

    #[test]
    fn default_does_not_create_or_discover_anything() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        assert_eq!(reconcile(&paths, false).unwrap().state, "disabled");
        assert!(!paths.state_root.exists());
    }

    #[test]
    fn missing_runtime_waits_without_enabling() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        assert_eq!(
            reconcile(&paths, true).unwrap().state,
            "waiting_for_runtime"
        );
        let control: Value =
            serde_json::from_slice(&fs::read(paths.state_root.join("control.json")).unwrap())
                .unwrap();
        assert_eq!(control["requireIdentification"], false);
    }

    #[test]
    fn unknown_hash_is_rejected_even_with_matching_anchor() {
        assert!(transform(ANCHOR.as_bytes(), Path::new("C:/state/control.json")).is_err());
    }

    #[test]
    fn binding_requires_unique_anchor_and_preserves_other_code() {
        let path = Path::new("C:/unicode-\u{4e2d}/control.json");
        for source in ["no binding".to_string(), ANCHOR.repeat(2)] {
            assert!(transform_binding(source.as_bytes(), path).is_err());
        }
        let source = format!("prefix;{ANCHOR};suffix");
        let output =
            String::from_utf8(transform_binding(source.as_bytes(), path).unwrap()).unwrap();
        assert!(output.starts_with("prefix;new nf("));
        assert!(output.contains(";suffix\n"));
        assert!(output.ends_with(HELPER));
        assert!(!output.contains("turn_id:"));
    }

    fn descriptor(root: &Path, key: &str) -> Value {
        let node = root.join(key).join("bin/node.exe");
        json!({"mcpServers":{"cua_repl":{
            "command":node, "args":[root.join(key).join("bin/node_modules/@oai/cua-repl/bin/cua-repl.mjs")],
            "env":{"NODE_REPL_NODE_PATH":node,"NODE_REPL_TRUSTED_SERVICES":"{\"browser\":\"@oai/browser-desktop/service\"}",
                "CUA_REPL_ENABLED_SURFACES":"browser"}
        }}})
    }

    #[test]
    fn discovery_checks_native_backend_and_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let mut data = descriptor(&root, "0123456789abcdef");
        assert_eq!(selected_key(&data, &root).unwrap(), "0123456789abcdef");
        data["mcpServers"]["cua_repl"]["env"]["NODE_REPL_TRUSTED_SERVICES"] =
            json!("{\"browser\":\"other/backend\"}");
        assert!(selected_key(&data, &root).is_err());
        assert!(selected_key(&descriptor(&root, "../escape"), &root).is_err());
        assert!(selected_key(&descriptor(temp.path(), "0123456789abcdef"), &root).is_err());
    }

    #[test]
    fn ambiguous_descriptors_refuse_enablement() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        for (version, key) in [("one", "0123456789abcdef"), ("two", "fedcba9876543210")] {
            let dir = paths
                .codex_home
                .join("plugins/cache/openai-bundled/unified-computer-use")
                .join(version);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join(".mcp.json"),
                serde_json::to_vec(&descriptor(&paths.runtime_root, key)).unwrap(),
            )
            .unwrap();
        }
        assert!(reconcile(&paths, true).is_err());
        assert!(
            !fs::read_to_string(paths.state_root.join("control.json"))
                .unwrap()
                .contains(":true")
        );
    }

    #[test]
    fn recovery_rejects_forged_journal_and_backup() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let dir = paths.state_root.join("0123456789abcdef");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("journal.json"), br#"{"schema":1,"originalSha":"fake","candidateSha":"fake","modifiedSecs":0,"modifiedNanos":0}"#).unwrap();
        fs::write(dir.join("original.mjs"), ANCHOR).unwrap();
        assert!(reconcile(&paths, false).is_err());
    }

    #[test]
    fn concurrent_owner_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        fs::create_dir_all(&paths.state_root).unwrap();
        let file = File::create(paths.state_root.join("owner.lock")).unwrap();
        file.try_lock_exclusive().unwrap();
        assert!(reconcile(&paths, true).is_err());
        assert!(!paths.state_root.join("control.json").exists());
    }

    #[test]
    fn backup_must_be_outside_runtime_cache() {
        let temp = tempfile::tempdir().unwrap();
        let mut paths = paths(&temp);
        paths.state_root = paths.runtime_root.join("backup");
        assert!(reconcile(&paths, true).is_err());
    }

    #[test]
    fn atomic_replace_and_timestamp_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("value");
        write_new(&path, b"original").unwrap();
        assert!(write_new(&path, b"collision").is_err());
        atomic_write(&path, b"candidate").unwrap();
        assert_eq!(read_regular(&path, 50).unwrap(), b"candidate");
        let time = UNIX_EPOCH + Duration::new(1_789_145_796, 123_456_700);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(time)
            .unwrap();
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), time);
    }

    // The proprietary runtime is supplied locally, never committed or executed by this test.
    #[test]
    #[ignore = "requires CPP_NATIVE_BROWSER_FIXTURE containing the pinned service and worker"]
    fn pinned_fixture_transaction_recovery_and_external_change() {
        let fixture = PathBuf::from(std::env::var_os("CPP_NATIVE_BROWSER_FIXTURE").unwrap());
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let key = "0123456789abcdef";
        let runtime = paths.runtime_root.join(key);
        let service = runtime.join(SERVICE);
        fs::create_dir_all(service.parent().unwrap()).unwrap();
        fs::copy(fixture.join(SERVICE), &service).unwrap();
        fs::copy(
            fixture.join("bin/node_repl.exe"),
            runtime.join("bin/node_repl.exe"),
        )
        .unwrap();
        let original = fs::read(&service).unwrap();
        let modified = fs::metadata(&service).unwrap().modified().unwrap();
        let dir = paths
            .codex_home
            .join("plugins/cache/openai-bundled/unified-computer-use/test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(".mcp.json"),
            serde_json::to_vec(&descriptor(&paths.runtime_root, key)).unwrap(),
        )
        .unwrap();
        assert_eq!(reconcile(&paths, true).unwrap().state, "prepared");
        let candidate = fs::read(&service).unwrap();
        assert_ne!(candidate, original);
        assert_eq!(reconcile(&paths, true).unwrap().state, "prepared");
        assert_eq!(reconcile(&paths, false).unwrap().state, "restored");
        assert_eq!(fs::read(&service).unwrap(), original);
        assert_eq!(
            fs::metadata(&service).unwrap().modified().unwrap(),
            modified
        );
        // Simulate cache rebuild and interrupted deployment with a durable journal.
        assert_eq!(reconcile(&paths, true).unwrap().state, "prepared");
        fs::write(&service, &original).unwrap();
        assert_eq!(reconcile(&paths, true).unwrap().state, "prepared");
        assert_eq!(fs::read(&service).unwrap(), candidate);
        fs::write(&service, b"external edit").unwrap();
        assert!(reconcile(&paths, false).is_err());
        assert_eq!(fs::read(&service).unwrap(), b"external edit");
        fs::write(&service, &candidate).unwrap();
        reconcile(&paths, false).unwrap();
        assert_eq!(fs::read(&service).unwrap(), original);
    }
}
