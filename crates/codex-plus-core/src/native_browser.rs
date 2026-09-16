//! Opt-in adaptation of a pinned native Edge/Chrome identification callback.
//! Does not implement browser execution, cloud identity or approval decisions.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

#[derive(Clone)]
struct RuntimeContract {
    service_sha: String,
    files: Vec<(&'static str, String)>,
}

impl RuntimeContract {
    fn pinned() -> Self {
        Self {
            service_sha: ORIGINAL_SHA.into(),
            files: vec![
                ("bin/node_repl.exe", NATIVE_SHA.into()),
                (
                    "bin/node.exe",
                    "be14417b6c4b4a5af06be7c16bda58730f26b912c3e8c6489d12392ef08f35bf".into(),
                ),
                (
                    "manifest.json",
                    "ba3691b0717b6df8064c3841a75c784e8af9633c7b47f2fdb56d8de099efe6fc".into(),
                ),
                (
                    "bin/node_modules/@oai/cua-repl/bin/cua-repl.mjs",
                    "992174a5e637645aeb444adfdb1bae688e997bb84d7db07532f68e358e60f278".into(),
                ),
            ],
        }
    }
}

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

// Deny directory deletion/renaming while a Windows transaction uses its descendants.
// Open root-first with OPEN_REPARSE_POINT so no checked parent can become a junction.
fn pin_parents(path: &Path) -> Result<Vec<File>> {
    plain_path(path)?;
    let mut guards = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let mut parents: Vec<_> = path.ancestors().skip(1).collect();
        parents.reverse();
        for parent in parents {
            if !parent.exists() {
                break;
            }
            let guard = OpenOptions::new()
                .read(true)
                .share_mode(0x1 | 0x2) // FILE_SHARE_READ | FILE_SHARE_WRITE, never DELETE.
                .custom_flags(0x02000000 | 0x00200000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT.
                .open(parent)?;
            let meta = guard.metadata()?;
            ensure!(
                meta.is_dir() && meta.file_attributes() & 0x400 == 0,
                "Parent directory is a reparse point"
            );
            guards.push(guard);
        }
    }
    Ok(guards)
}

fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let _guards = pin_parents(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0x1).custom_flags(0x00200000);
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            meta.file_attributes() & 0x400 == 0,
            "File is a reparse point"
        );
    }
    ensure!(
        meta.is_file() && meta.len() <= limit,
        "Unexpected file type or size"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "File grew beyond the size limit"
    );
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<File> {
    let _guards = pin_parents(path)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(file)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write_with_modified(path, bytes, None)
}

fn atomic_write_with_modified(
    path: &Path,
    bytes: &[u8],
    modified: Option<SystemTime>,
) -> Result<()> {
    let _guards = pin_parents(path)?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let file = write_new(&temp, bytes)?;
        // Publish bytes and timestamp together; never reopen the replaced target to set metadata.
        if let Some(modified) = modified {
            file.set_modified(modified)?;
            file.sync_all()?;
        }
        drop(file);
        #[cfg(windows)]
        {
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
        }
        #[cfg(not(windows))]
        {
            fs::rename(&temp, path).map_err(anyhow::Error::from)
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn transform(source: &[u8], control: &Path, contract: &RuntimeContract) -> Result<Vec<u8>> {
    ensure!(
        sha(source) == contract.service_sha,
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
        env["NODE_REPL_NODE_PATH"].as_str().map(Path::new) == Some(node.as_path()),
        "Conflicting native Node selection"
    );
    let worker = root.join(key).join("bin/node_repl.exe");
    ensure!(
        env["CUA_REPL_NODE_REPL_PATH"].as_str().map(Path::new) == Some(worker.as_path()),
        "Conflicting native worker selection"
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
        args.len() == 1 && args[0].as_str().map(Path::new) == Some(entry.as_path()),
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

fn prepare(paths: &BrowserPaths, key: &str, contract: &RuntimeContract) -> Result<()> {
    ensure!(key_valid(key), "Invalid runtime key");
    let runtime = paths.runtime_root.join(key);
    let target = runtime.join(SERVICE);
    let _runtime_guards = pin_parents(&target)?;
    for (file, expected) in &contract.files {
        ensure!(
            sha(&read_regular(&runtime.join(file), 128 * 1024 * 1024)?) == *expected,
            "Unsupported native runtime component: {file}"
        );
    }
    let mut current = read_regular(&target, MAX_SERVICE)?;
    let backup_dir = paths.state_root.join(key);
    plain_path(&backup_dir)?;
    fs::create_dir_all(&backup_dir)?;
    let _backup_guards = pin_parents(&backup_dir.join("journal.json"))?;
    let backup = backup_dir.join("original.mjs");
    let journal_path = backup_dir.join("journal.json");
    let control = paths.state_root.join("control.json");
    if journal_path.exists() {
        let (journal, original, recorded_candidate) = recovery_material(paths, key, contract)?;
        let candidate = transform(&original, &control, contract)?;
        if current == recorded_candidate {
            if candidate == recorded_candidate {
                return Ok(());
            }
            // Restore before upgrading the journal, so either journal can recover a crash.
            ensure!(
                read_regular(&target, MAX_SERVICE)? == current,
                "Concurrent adapter upgrade"
            );
            let modified = UNIX_EPOCH
                .checked_add(Duration::new(journal.modified_secs, journal.modified_nanos))
                .context("Invalid recovery timestamp")?;
            atomic_write_with_modified(&target, &original, Some(modified))?;
            current = original;
        }
        ensure!(
            sha(&current) == contract.service_sha,
            "Runtime changed outside Codex++"
        );
    }
    {
        let candidate = transform(&current, &control, contract)?;
        if backup.exists() {
            ensure!(
                read_regular(&backup, MAX_SERVICE)? == current,
                "Unjournaled backup conflict"
            );
        } else {
            write_new(&backup, &current)?;
        }
        let candidate_path = backup_dir.join(format!("candidate-{}.mjs", sha(&candidate)));
        if candidate_path.exists() {
            ensure!(
                read_regular(&candidate_path, MAX_SERVICE)? == candidate,
                "Candidate backup conflict"
            );
        } else {
            write_new(&candidate_path, &candidate)?;
        }
        let modified = fs::metadata(&target)?
            .modified()?
            .duration_since(UNIX_EPOCH)?;
        let journal = Journal {
            schema: 1,
            original_sha: contract.service_sha.clone(),
            candidate_sha: sha(&candidate),
            modified_secs: modified.as_secs(),
            modified_nanos: modified.subsec_nanos(),
        };
        // Durable original and journal precede any runtime write.
        atomic_write(&journal_path, &serde_json::to_vec(&journal)?)?;
    }
    let (journal, original, candidate) = recovery_material(paths, key, contract)?;
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

fn recovery_material(
    paths: &BrowserPaths,
    key: &str,
    contract: &RuntimeContract,
) -> Result<(Journal, Vec<u8>, Vec<u8>)> {
    ensure!(key_valid(key), "Invalid recovery key");
    let dir = paths.state_root.join(key);
    let journal: Journal = serde_json::from_slice(&read_regular(&dir.join("journal.json"), 4096)?)?;
    let original = read_regular(&dir.join("original.mjs"), MAX_SERVICE)?;
    ensure!(
        journal.candidate_sha.len() == 64
            && journal.candidate_sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid candidate hash"
    );
    let candidate = read_regular(
        &dir.join(format!("candidate-{}.mjs", journal.candidate_sha)),
        MAX_SERVICE,
    )?;
    ensure!(
        journal.schema == 1
            && (journal.original_sha == contract.service_sha
                || journal.original_sha == ORIGINAL_SHA)
            && sha(&original) == journal.original_sha
            && journal.candidate_sha == sha(&candidate)
            && journal.modified_nanos < 1_000_000_000,
        "Recovery journal conflicts with verified content"
    );
    Ok((journal, original, candidate))
}

fn restore_all(paths: &BrowserPaths, keep: Option<&str>, contract: &RuntimeContract) -> Result<()> {
    let mut pending = Vec::new();
    let mut guards = Vec::new();
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
        let target = paths.runtime_root.join(&key).join(SERVICE);
        if !target.exists() {
            continue; // Desktop owns cache deletion; never resurrect an obsolete runtime.
        }
        let (journal, original, candidate) = recovery_material(paths, &key, contract)?;
        guards.extend(pin_parents(&target)?);
        let current = read_regular(&target, MAX_SERVICE)?;
        ensure!(
            current == original || current == candidate,
            "External runtime change prevents recovery"
        );
        if current == candidate {
            let modified = UNIX_EPOCH
                .checked_add(Duration::new(journal.modified_secs, journal.modified_nanos))
                .context("Invalid recovery timestamp")?;
            pending.push((target, modified, original, current));
        }
    }
    // Preflight every cache before restoring any, independent of directory enumeration order.
    for (target, modified, original, current) in pending {
        ensure!(
            read_regular(&target, MAX_SERVICE)? == current,
            "Concurrent recovery change"
        );
        atomic_write_with_modified(&target, &original, Some(modified))?;
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
    reconcile_contract(paths, enabled, &RuntimeContract::pinned())
}

fn reconcile_contract(
    paths: &BrowserPaths,
    enabled: bool,
    contract: &RuntimeContract,
) -> Result<BrowserStatus> {
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
    let _guards = pin_parents(&paths.state_root.join("owner.lock"))?;
    let lock_path = paths.state_root.join("owner.lock");
    plain_path(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    lock.try_lock_exclusive()
        .context("Another compatibility transaction is active")?;
    let result = reconcile_locked(paths, enabled, contract);
    if result.is_err() {
        // Fail closed for an already-loaded helper as well as future workers.
        let _ = atomic_write(
            &paths.state_root.join("control.json"),
            br#"{"schema":1,"requireIdentification":false}"#,
        );
    }
    result
}

fn reconcile_locked(
    paths: &BrowserPaths,
    enabled: bool,
    contract: &RuntimeContract,
) -> Result<BrowserStatus> {
    let control = paths.state_root.join("control.json");
    if !enabled {
        atomic_write(&control, br#"{"schema":1,"requireIdentification":false}"#)?;
        restore_all(paths, None, contract)?;
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
    restore_all(paths, Some(&key), contract)?;
    prepare(paths, &key, contract)?;
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

pub struct BrowserMonitor {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl BrowserMonitor {
    pub async fn stop(self) {
        let _ = self.shutdown.send(());
        // A started blocking filesystem transaction must finish before its owner exits.
        if let Err(error) = self.task.await {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "native_browser.shutdown_failed",
                json!({"detail": error.to_string()}),
            );
        }
    }
}

/// The singleton launcher owns this task. Existing-instance activation does not start another one.
/// The startup snapshot intentionally requires a launcher restart to apply a saved choice.
pub async fn start_monitor(enabled: bool) -> Option<BrowserMonitor> {
    let paths = BrowserPaths::current().ok()?;
    if !enabled && !paths.state_root.exists() {
        return None;
    }
    match start_monitor_with_contract(paths, enabled, RuntimeContract::pinned()).await {
        Ok(monitor) => Some(monitor),
        Err(error) => {
            // Do not overwrite the active owner's control or status on a second launch.
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "native_browser.owner_refused",
                json!({"detail": error.to_string()}),
            );
            None
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MonitorReceipt {
    schema: u32,
    generation: String,
    state: String,
}

fn write_monitor_receipt(file: &mut File, generation: &str, state: &str) -> Result<()> {
    let bytes = serde_json::to_vec(&MonitorReceipt {
        schema: 1,
        generation: generation.into(),
        state: state.into(),
    })?;
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn verify_restored_state(paths: &BrowserPaths) -> Result<()> {
    if !paths.state_root.exists() {
        return Ok(());
    }
    let control = paths.state_root.join("control.json");
    if control.exists() {
        let value: Value = serde_json::from_slice(&read_regular(&control, 1024)?)?;
        ensure!(
            value["schema"] == 1 && value["requireIdentification"] == false,
            "Native browser compatibility is still enabled"
        );
    }
    for entry in fs::read_dir(&paths.state_root)? {
        let key = entry?.file_name().to_string_lossy().to_string();
        if !key_valid(&key) || !paths.state_root.join(&key).join("journal.json").exists() {
            continue;
        }
        let target = paths.runtime_root.join(&key).join(SERVICE);
        if target.exists() {
            let (_, original, _) = recovery_material(paths, &key, &RuntimeContract::pinned())?;
            ensure!(
                read_regular(&target, MAX_SERVICE)? == original,
                "Native browser service has not been restored"
            );
        }
    }
    Ok(())
}

fn acquire_monitor_owner(paths: &BrowserPaths) -> Result<File> {
    plain_path(&paths.state_root)?;
    ensure!(
        !paths.state_root.starts_with(&paths.runtime_root),
        "Backups must be outside the cache"
    );
    fs::create_dir_all(&paths.state_root)?;
    let path = paths.state_root.join("monitor.lock");
    let _guards = pin_parents(&path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0x1 | 0x2).custom_flags(0x00200000);
    }
    let owner = options.open(&path)?;
    let meta = owner.metadata()?;
    ensure!(meta.is_file(), "Unexpected monitor lock type");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(meta.file_attributes() & 0x400 == 0, "Monitor lock is a reparse point");
    }
    plain_path(&path)?;
    owner.try_lock_exclusive().context("Another native browser monitor is active")?;
    Ok(owner)
}

/// Called after Codex has been stopped, before the manager launches a replacement.
/// Never restores files itself or creates a lock for an older launcher.
pub fn wait_for_monitor_shutdown(timeout: Duration) -> Result<()> {
    if !cfg!(windows) {
        return Ok(());
    }
    let paths = BrowserPaths::current()?;
    wait_for_monitor_shutdown_at(&paths, timeout)
}

fn wait_for_monitor_shutdown_at(paths: &BrowserPaths, timeout: Duration) -> Result<()> {
    let path = paths.state_root.join("monitor.lock");
    let _guards = pin_parents(&path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0x1 | 0x2).custom_flags(0x00200000);
    }
    let mut file = match options.open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return verify_restored_state(paths);
        }
        Err(error) => return Err(error.into()),
    };
    plain_path(&path)?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => {
                ensure!(file.metadata()?.len() <= 1024, "Invalid native cleanup receipt");
                file.seek(SeekFrom::Start(0))?;
                let mut bytes = Vec::new();
                Read::by_ref(&mut file).take(1025).read_to_end(&mut bytes)?;
                let receipt: MonitorReceipt = serde_json::from_slice(&bytes)?;
                ensure!(
                    receipt.schema == 1 && uuid::Uuid::parse_str(&receipt.generation).is_ok()
                        && receipt.state == "restored",
                    "Native browser cleanup did not complete successfully"
                );
                return Ok(());
            }
            Err(error) if error.kind() == fs2::lock_contended_error().kind() => {
                ensure!(
                    std::time::Instant::now() < deadline,
                    "Native browser cleanup is still running; launcher was not terminated"
                );
                std::thread::sleep(Duration::from_millis(50).min(
                    deadline.saturating_duration_since(std::time::Instant::now()),
                ));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn start_monitor_with_contract(
    paths: BrowserPaths,
    enabled: bool,
    contract: RuntimeContract,
) -> Result<BrowserMonitor> {
    let mut owner = acquire_monitor_owner(&paths)?;
    let generation = uuid::Uuid::new_v4().to_string();
    write_monitor_receipt(&mut owner, &generation, "active")?;
    let initial = monitor_once(paths.clone(), enabled, None, contract.clone()).await;
    if let Ok((status, _)) = &initial {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "native_browser.compatibility",
            json!(status),
        );
    }
    let (shutdown, mut stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut previous = initial.ok();
        let started = std::time::Instant::now();
        loop {
            let delay = if started.elapsed() < Duration::from_secs(30)
                && previous
                    .as_ref()
                    .is_some_and(|(s, _)| s.state == "waiting_for_runtime")
            {
                Duration::from_millis(500)
            } else {
                Duration::from_secs(15)
            };
            tokio::select! {
                _ = &mut stopped => break,
                _ = tokio::time::sleep(delay) => {}
            }
            let outcome = monitor_once(paths.clone(), enabled, previous.clone(), contract.clone()).await;
            if let Ok((status, fingerprint)) = outcome {
                if previous.as_ref().map(|(s, _)| s) != Some(&status) {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "native_browser.compatibility",
                        json!(status),
                    );
                }
                previous = Some((status, fingerprint));
            }
        }
        // Keep lifetime ownership through recovery; a new monitor must not race this restore.
        match monitor_once(paths, false, None, contract).await {
            Ok((status, _)) => {
                if let Err(error) = write_monitor_receipt(&mut owner, &generation, &status.state) {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "native_browser.shutdown_failed",
                        json!({"detail": error.to_string()}),
                    );
                }
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "native_browser.compatibility",
                    json!(status),
                );
            }
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "native_browser.shutdown_failed",
                    json!({"detail": error.to_string()}),
                );
            }
        }
        drop(owner);
    });
    Ok(BrowserMonitor { shutdown, task })
}

fn error_status(error: &anyhow::Error) -> BrowserStatus {
    let retryable = error.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|error| {
            matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::WouldBlock
            ) || matches!(error.raw_os_error(), Some(32 | 33))
        }) || cause
            .downcast_ref::<serde_json::Error>()
            .is_some_and(|error| error.is_eof())
    });
    BrowserStatus::new(
        if retryable {
            "waiting_for_runtime"
        } else {
            "blocked"
        },
        &format!("Compatibility refused: {error}"),
    )
}

// A process-local observation cache avoids repeated large-file hashing at idle.
// It is never trusted to authorize a write; reconcile still verifies the full contract.
fn observation(paths: &BrowserPaths) -> Result<String> {
    let mut files = BTreeSet::new();
    let mut keys = BTreeSet::new();
    if let Some(key) = discover(paths)? {
        keys.insert(key);
    }
    let plugin = paths
        .codex_home
        .join("plugins/cache/openai-bundled/unified-computer-use");
    if plugin.exists() {
        for entry in fs::read_dir(plugin)? {
            files.insert(entry?.path().join(".mcp.json"));
        }
    }
    files.insert(paths.state_root.join("control.json"));
    if paths.state_root.exists() {
        for entry in fs::read_dir(&paths.state_root)? {
            let entry = entry?;
            let key = entry.file_name().to_string_lossy().to_string();
            if key_valid(&key) {
                plain_path(&entry.path())?;
                keys.insert(key);
                for file in fs::read_dir(entry.path())? {
                    files.insert(file?.path());
                    ensure!(files.len() <= 1024, "Too many recovery records");
                }
            }
        }
    }
    for key in keys {
        let runtime = paths.runtime_root.join(key);
        files.insert(runtime.join(SERVICE));
        for (file, _) in RuntimeContract::pinned().files {
            files.insert(runtime.join(file));
        }
    }
    let mut hash = Sha256::new();
    for path in files {
        plain_path(&path)?;
        hash.update(path.to_string_lossy().as_bytes());
        if !path.exists() {
            hash.update(b"missing");
            continue;
        }
        let file = File::open(&path)?;
        let meta = file.metadata()?;
        ensure!(meta.is_file(), "Unexpected observation path");
        hash.update(format!(
            "{:?}:{:?}:{}",
            meta.created()?,
            meta.modified()?,
            meta.len()
        ));
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows::Win32::Foundation::HANDLE;
            use windows::Win32::Storage::FileSystem::{
                BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
            };
            let mut identity = BY_HANDLE_FILE_INFORMATION::default();
            unsafe {
                GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut identity)?;
            }
            ensure!(
                identity.dwFileAttributes & 0x400 == 0,
                "Reparse observation file"
            );
            hash.update(identity.dwVolumeSerialNumber.to_le_bytes());
            hash.update(identity.nFileIndexHigh.to_le_bytes());
            hash.update(identity.nFileIndexLow.to_le_bytes());
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

async fn monitor_once(
    paths: BrowserPaths,
    enabled: bool,
    cache: Option<(BrowserStatus, Option<String>)>,
    contract: RuntimeContract,
) -> Result<(BrowserStatus, Option<String>)> {
    tokio::task::spawn_blocking(move || {
        let before = observation(&paths).ok();
        let cached = cache.filter(|(status, observed)| {
            matches!(status.state.as_str(), "prepared" | "restored" | "blocked")
                && before.is_some()
                && &before == observed
        });
        let (status, fingerprint) = match cached {
            Some((status, _)) => (status, before),
            None => {
                let status =
                    reconcile_contract(&paths, enabled, &contract).unwrap_or_else(|error| error_status(&error));
                let fingerprint = observation(&paths).ok();
                (status, fingerprint)
            }
        };
        if paths.state_root.exists() {
            let _ = atomic_write(
                &paths.state_root.join("status.json"),
                &serde_json::to_vec(&status).unwrap_or_default(),
            );
        }
        (status, fingerprint)
    })
    .await
    .map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `plain_path` 拒绝祖先链上含软链的路径（防 junction / symlink 攻击，见该函数注释）。
    /// macOS 上 `/var` 是指向 `/private/var` 的系统软链，而 `tempfile` 默认建在
    /// `/var/folders/...` 下——直接用 `temp.path()` 会让所有测试都撞上这条校验。
    /// 这里 canonicalize 到真实路径，既保留被校验路径的生产语义，又让测试可跨平台运行。
    /// Windows 的 junction 重定向（如被重定向的 TEMP）不在此 helper 的处理范围内，
    /// 那属于 `plain_path` 自身需要收紧的地方。
    fn temp_root(temp: &tempfile::TempDir) -> PathBuf {
        let canonical = temp.path().canonicalize().expect("temp dir should canonicalize");
        // Windows 的 canonicalize 会加上 `\\?\` verbatim 前缀。测试构造的路径要参与
        // 字符串形态断言（分隔符风格、路径比较），带上这个前缀会改变语义，所以剥掉。
        #[cfg(windows)]
        {
            let text = canonical.to_string_lossy().to_string();
            if let Some(stripped) = text.strip_prefix(r"\\?\") {
                return PathBuf::from(stripped);
            }
        }
        canonical
    }

    fn paths(temp: &tempfile::TempDir) -> BrowserPaths {
        let root = temp_root(temp);
        BrowserPaths {
            codex_home: root.join("home"),
            runtime_root: root.join("cache"),
            state_root: root.join("state"),
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
        assert!(
            transform(
                ANCHOR.as_bytes(),
                Path::new("C:/state/control.json"),
                &RuntimeContract::pinned()
            )
            .is_err()
        );
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
            "env":{"NODE_REPL_NODE_PATH":node,"CUA_REPL_NODE_REPL_PATH":root.join(key).join("bin/node_repl.exe"),"NODE_REPL_TRUSTED_SERVICES":"{\"browser\":\"@oai/browser-desktop/service\"}",
                "CUA_REPL_ENABLED_SURFACES":"browser"}
        }}})
    }

    #[test]
    fn discovery_checks_native_backend_and_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("cache");
        let mut data = descriptor(&root, "0123456789abcdef");
        assert_eq!(selected_key(&data, &root).unwrap(), "0123456789abcdef");
        data["mcpServers"]["cua_repl"]["env"]["NODE_REPL_TRUSTED_SERVICES"] =
            json!("{\"browser\":\"other/backend\"}");
        assert!(selected_key(&data, &root).is_err());
        assert!(selected_key(&descriptor(&root, "../escape"), &root).is_err());
        assert!(selected_key(&descriptor(&temp_root(&temp), "0123456789abcdef"), &root).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_generated_descriptor_accepts_backslash_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("OpenAI/Codex/runtimes/cua_node");
        let key = "0123456789abcdef";
        let base = format!(r"{}\{key}", root.display().to_string().replace('/', "\\"));
        // Desktop writes backslashes independently of our PathBuf joins.
        let data = json!({"mcpServers":{"cua_repl":{
            "command":format!(r"{base}\bin\node.exe"),
            "args":[format!(r"{base}\bin\node_modules\@oai\cua-repl\bin\cua-repl.mjs")],
            "env":{
                "NODE_REPL_NODE_PATH":format!(r"{base}\bin\node.exe"),
                "CUA_REPL_NODE_REPL_PATH":format!(r"{base}\bin\node_repl.exe"),
                "NODE_REPL_TRUSTED_SERVICES":"{\"browser\":\"@oai/browser-desktop/service\"}",
                "CUA_REPL_ENABLED_SURFACES":"browser"
            }
        }}});
        assert_eq!(selected_key(&data, &root).unwrap(), key);
    }

    #[cfg(windows)]
    #[test]
    fn windows_descriptor_accepts_independent_separator_styles() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("OpenAI/Codex/runtimes/cua_node");
        let key = "0123456789abcdef";
        let fields = [
            "/mcpServers/cua_repl/command",
            "/mcpServers/cua_repl/env/NODE_REPL_NODE_PATH",
            "/mcpServers/cua_repl/env/CUA_REPL_NODE_REPL_PATH",
            "/mcpServers/cua_repl/args/0",
        ];
        for mask in 0..16 {
            let mut data = descriptor(&root, key);
            for (index, field) in fields.iter().enumerate() {
                let value = data.pointer_mut(field).unwrap();
                let path = value.as_str().unwrap().replace('\\', "/");
                *value = json!(if mask & (1 << index) == 0 {
                    path
                } else {
                    path.replace('/', "\\")
                });
            }
            assert_eq!(selected_key(&data, &root).unwrap(), key, "mask {mask}");
        }
    }

    #[test]
    fn descriptor_rejects_conflicting_or_malformed_runtime_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("cache");
        let key = "0123456789abcdef";
        for field in [
            "/mcpServers/cua_repl/command",
            "/mcpServers/cua_repl/env/NODE_REPL_NODE_PATH",
            "/mcpServers/cua_repl/env/CUA_REPL_NODE_REPL_PATH",
            "/mcpServers/cua_repl/args/0",
        ] {
            for invalid in [
                Value::Null,
                json!(42),
                json!(""),
                json!("bin/node_repl.exe"),
                json!(root.join("fedcba9876543210/bin/node_repl.exe")),
                json!(root.join(key).join("bin/../bin/node_repl.exe")),
            ] {
                let mut data = descriptor(&root, key);
                *data.pointer_mut(field).unwrap() = invalid;
                assert!(selected_key(&data, &root).is_err(), "{field}");
            }
        }
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
        let target = paths.runtime_root.join("0123456789abcdef").join(SERVICE);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, b"unrecognized service").unwrap();
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
        let path = temp_root(&temp).join("value");
        write_new(&path, b"original").unwrap();
        assert!(write_new(&path, b"collision").is_err());
        atomic_write(&path, b"candidate").unwrap();
        assert_eq!(read_regular(&path, 50).unwrap(), b"candidate");
        let time = UNIX_EPOCH + Duration::new(1_789_145_796, 123_456_700);
        atomic_write_with_modified(&path, b"original", Some(time)).unwrap();
        assert_eq!(read_regular(&path, 50).unwrap(), b"original");
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), time);
    }

    #[cfg(windows)]
    #[test]
    fn failed_atomic_restore_preserves_target_bytes_and_timestamp() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp_root(&temp).join("service.mjs");
        write_new(&path, b"candidate").unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        let restored = UNIX_EPOCH + Duration::new(1_789_145_796, 123_456_700);
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0x1)
            .open(&path)
            .unwrap();
        assert!(atomic_write_with_modified(&path, b"original", Some(restored)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"candidate");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
        assert_eq!(fs::read_dir(temp_root(&temp)).unwrap().count(), 1);
        drop(held);
        atomic_write_with_modified(&path, b"original", Some(restored)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), restored);
    }

    fn synthetic(temp: &tempfile::TempDir) -> (BrowserPaths, RuntimeContract, PathBuf) {
        let paths = paths(temp);
        let key = "0123456789abcdef";
        let service = paths.runtime_root.join(key).join(SERVICE);
        let original = format!("fixture;{ANCHOR};original");
        fs::create_dir_all(service.parent().unwrap()).unwrap();
        fs::write(&service, &original).unwrap();
        let contract = RuntimeContract {
            service_sha: sha(original.as_bytes()),
            files: vec![("bin/node.exe", sha(b"fixture-node"))],
        };
        fs::write(
            paths.runtime_root.join(key).join("bin/node.exe"),
            b"fixture-node",
        )
        .unwrap();
        let dir = paths
            .codex_home
            .join("plugins/cache/openai-bundled/unified-computer-use/test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(".mcp.json"),
            serde_json::to_vec(&descriptor(&paths.runtime_root, key)).unwrap(),
        )
        .unwrap();
        (paths, contract, service)
    }

    #[test]
    fn synthetic_transaction_and_rebuilt_generation_timestamp() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let original = fs::read(&service).unwrap();
        assert_eq!(
            reconcile_contract(&paths, true, &contract).unwrap().state,
            "prepared"
        );
        let candidate = fs::read(&service).unwrap();
        reconcile_contract(&paths, true, &contract).unwrap();
        assert_eq!(fs::read(&service).unwrap(), candidate);
        reconcile_contract(&paths, false, &contract).unwrap();
        assert_eq!(fs::read(&service).unwrap(), original);
        let rebuilt_time = UNIX_EPOCH + Duration::new(1_789_145_800, 700);
        File::options()
            .write(true)
            .open(&service)
            .unwrap()
            .set_modified(rebuilt_time)
            .unwrap();
        // A disabled reconcile must not overwrite an already-original generation's timestamp.
        reconcile_contract(&paths, false, &contract).unwrap();
        assert_eq!(
            fs::metadata(&service).unwrap().modified().unwrap(),
            rebuilt_time
        );
        reconcile_contract(&paths, true, &contract).unwrap();
        reconcile_contract(&paths, false, &contract).unwrap();
        assert_eq!(
            fs::metadata(&service).unwrap().modified().unwrap(),
            rebuilt_time
        );
    }

    #[test]
    fn stored_candidate_allows_adapter_upgrade_without_overwriting_external_changes() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        reconcile_contract(&paths, true, &contract).unwrap();
        let key = "0123456789abcdef";
        let (mut journal, original, current_candidate) =
            recovery_material(&paths, key, &contract).unwrap();
        let old_candidate = [
            current_candidate.as_slice(),
            b"\n// previous adapter revision\n",
        ]
        .concat();
        journal.candidate_sha = sha(&old_candidate);
        let backup = paths.state_root.join(key);
        fs::write(
            backup.join(format!("candidate-{}.mjs", journal.candidate_sha)),
            &old_candidate,
        )
        .unwrap();
        fs::write(
            backup.join("journal.json"),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        fs::write(&service, &old_candidate).unwrap();
        reconcile_contract(&paths, true, &contract).unwrap();
        assert_eq!(fs::read(&service).unwrap(), current_candidate);
        fs::write(&service, b"third-party change").unwrap();
        assert!(reconcile_contract(&paths, false, &contract).is_err());
        assert_eq!(fs::read(&service).unwrap(), b"third-party change");
        fs::write(&service, current_candidate).unwrap();
        reconcile_contract(&paths, false, &contract).unwrap();
        assert_eq!(fs::read(service).unwrap(), original);
    }

    #[test]
    fn all_restores_are_preflighted_before_any_runtime_write() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        reconcile_contract(&paths, true, &contract).unwrap();
        let candidate = fs::read(&service).unwrap();
        let other = "fedcba9876543210";
        let other_backup = paths.state_root.join(other);
        fs::create_dir_all(&other_backup).unwrap();
        for entry in fs::read_dir(paths.state_root.join("0123456789abcdef")).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), other_backup.join(entry.file_name())).unwrap();
        }
        let other_service = paths.runtime_root.join(other).join(SERVICE);
        fs::create_dir_all(other_service.parent().unwrap()).unwrap();
        fs::write(&other_service, b"external change").unwrap();
        assert!(reconcile_contract(&paths, false, &contract).is_err());
        assert_eq!(fs::read(&service).unwrap(), candidate);
        let control: Value =
            serde_json::from_slice(&fs::read(paths.state_root.join("control.json")).unwrap())
                .unwrap();
        assert_eq!(control["requireIdentification"], false);
        fs::write(&other_service, &candidate).unwrap();
        let journal_path = other_backup.join("journal.json");
        let mut journal: Journal =
            serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
        journal.modified_secs = u64::MAX;
        fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
        assert!(reconcile_contract(&paths, false, &contract).is_err());
        assert_eq!(fs::read(&service).unwrap(), candidate);
        assert_eq!(fs::read(&other_service).unwrap(), candidate);
    }

    #[test]
    fn changed_entry_component_is_refused_before_service_write() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let original = fs::read(&service).unwrap();
        fs::write(
            paths.runtime_root.join("0123456789abcdef/bin/node.exe"),
            b"unknown-node",
        )
        .unwrap();
        assert!(reconcile_contract(&paths, true, &contract).is_err());
        assert_eq!(fs::read(&service).unwrap(), original);
    }

    #[test]
    fn interrupted_backup_stage_can_be_resumed_and_tampering_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let dir = paths.state_root.join("0123456789abcdef");
        fs::create_dir_all(&dir).unwrap();
        fs::copy(&service, dir.join("original.mjs")).unwrap();
        reconcile_contract(&paths, true, &contract).unwrap();
        let candidate = fs::read(&service).unwrap();
        let journal: Journal =
            serde_json::from_slice(&fs::read(dir.join("journal.json")).unwrap()).unwrap();
        fs::write(
            dir.join(format!("candidate-{}.mjs", journal.candidate_sha)),
            b"corrupt backup",
        )
        .unwrap();
        assert!(reconcile_contract(&paths, false, &contract).is_err());
        assert_eq!(fs::read(service).unwrap(), candidate);
    }

    #[cfg(windows)]
    #[test]
    fn pinned_parent_cannot_be_renamed_during_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp_root(&temp).join("parent");
        let destination = temp_root(&temp).join("renamed");
        fs::create_dir(&parent).unwrap();
        let guards = pin_parents(&parent.join("service.mjs")).unwrap();
        assert!(fs::rename(&parent, &destination).is_err());
        drop(guards);
        fs::rename(&parent, &destination).unwrap();
    }

    #[test]
    fn removed_cache_does_not_require_obsolete_recovery_records() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let dir = paths.state_root.join("0123456789abcdef");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("journal.json"),
            b"obsolete record without a target",
        )
        .unwrap();
        assert_eq!(reconcile(&paths, false).unwrap().state, "restored");
    }

    #[test]
    fn transient_generation_errors_retry_but_unknown_contracts_do_not() {
        let missing = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert_eq!(error_status(&missing).state, "waiting_for_runtime");
        let partial = serde_json::from_str::<Value>("{").unwrap_err();
        assert_eq!(error_status(&partial.into()).state, "waiting_for_runtime");
        assert_eq!(
            error_status(&anyhow::anyhow!("Unsupported runtime hash")).state,
            "blocked"
        );
    }

    #[tokio::test]
    async fn idle_observation_does_not_rewrite_control_or_reconcile() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        reconcile_contract(&paths, true, &contract).unwrap();
        let control = paths.state_root.join("control.json");
        let modified = fs::metadata(&control).unwrap().modified().unwrap();
        let observed = observation(&paths).unwrap();
        let cached = Some((
            BrowserStatus::new("prepared", "fixture"),
            Some(observed.clone()),
        ));
        let (status, after) =
            monitor_once(paths.clone(), true, cached, RuntimeContract::pinned()).await.unwrap();
        assert_eq!(status.state, "prepared"); // A full pinned-contract reconcile would reject this fixture.
        assert_eq!(after.as_deref(), Some(observed.as_str()));
        assert_eq!(fs::metadata(control).unwrap().modified().unwrap(), modified);
        fs::write(service, b"external change").unwrap();
        assert_ne!(observation(&paths).unwrap(), observed);
    }

    #[tokio::test]
    async fn monitor_stop_restores_original_content_timestamp_and_control() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let original = fs::read(&service).unwrap();
        let modified = fs::metadata(&service).unwrap().modified().unwrap();
        let monitor = start_monitor_with_contract(paths.clone(), true, contract).await.unwrap();
        assert_ne!(fs::read(&service).unwrap(), original);
        monitor.stop().await;
        assert_eq!(sha(&fs::read(&service).unwrap()), sha(&original));
        assert_eq!(fs::metadata(&service).unwrap().modified().unwrap(), modified);
        let control: Value =
            serde_json::from_slice(&fs::read(paths.state_root.join("control.json")).unwrap()).unwrap();
        assert_eq!(control["requireIdentification"], false);
        let status: BrowserStatus =
            serde_json::from_slice(&fs::read(paths.state_root.join("status.json")).unwrap()).unwrap();
        assert_eq!(status.state, "restored");
    }

    #[tokio::test]
    async fn second_monitor_cannot_disable_or_restore_the_active_owner() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let first = start_monitor_with_contract(paths.clone(), true, contract.clone()).await.unwrap();
        let candidate = sha(&fs::read(&service).unwrap());
        let control = fs::read(paths.state_root.join("control.json")).unwrap();
        let status = fs::read(paths.state_root.join("status.json")).unwrap();
        assert!(start_monitor_with_contract(paths.clone(), false, contract.clone()).await.is_err());
        assert_eq!(sha(&fs::read(&service).unwrap()), candidate);
        assert_eq!(fs::read(paths.state_root.join("control.json")).unwrap(), control);
        assert_eq!(fs::read(paths.state_root.join("status.json")).unwrap(), status);
        first.stop().await;
        let next = start_monitor_with_contract(paths.clone(), true, contract).await.unwrap();
        assert_eq!(sha(&fs::read(&service).unwrap()), candidate);
        next.stop().await;
    }

    #[tokio::test]
    async fn monitor_shutdown_preserves_external_edits_and_reports_recovery_failure() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let monitor = start_monitor_with_contract(paths.clone(), true, contract).await.unwrap();
        fs::write(&service, b"external edit").unwrap();
        monitor.stop().await;
        assert_eq!(fs::read(&service).unwrap(), b"external edit");
        let control: Value =
            serde_json::from_slice(&fs::read(paths.state_root.join("control.json")).unwrap()).unwrap();
        assert_eq!(control["requireIdentification"], false);
        let status: BrowserStatus =
            serde_json::from_slice(&fs::read(paths.state_root.join("status.json")).unwrap()).unwrap();
        assert_eq!(status.state, "blocked");
        assert!(status.detail.contains("External runtime change"));
        assert!(wait_for_monitor_shutdown_at(&paths, Duration::ZERO).is_err());
        assert!(acquire_monitor_owner(&paths).is_ok());
    }

    #[tokio::test]
    async fn dropped_monitor_sender_still_runs_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let original = sha(&fs::read(&service).unwrap());
        let BrowserMonitor { shutdown, task } =
            start_monitor_with_contract(paths.clone(), true, contract).await.unwrap();
        drop(shutdown);
        task.await.unwrap();
        assert_eq!(sha(&fs::read(&service).unwrap()), original);
        assert!(acquire_monitor_owner(&paths).is_ok());
    }

    #[test]
    fn manager_wait_is_read_only_and_refuses_an_active_monitor() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        wait_for_monitor_shutdown_at(&paths, Duration::ZERO).unwrap();
        assert!(!paths.state_root.exists());
        let mut owner = acquire_monitor_owner(&paths).unwrap();
        write_monitor_receipt(&mut owner, &uuid::Uuid::new_v4().to_string(), "restored").unwrap();
        assert!(wait_for_monitor_shutdown_at(&paths, Duration::ZERO).is_err());
        assert!(!paths.state_root.join("control.json").exists());
        drop(owner);
        wait_for_monitor_shutdown_at(&paths, Duration::ZERO).unwrap();
    }

    #[test]
    fn manager_rejects_incomplete_receipts_and_legacy_enabled_state() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        fs::create_dir_all(&paths.state_root).unwrap();
        fs::write(paths.state_root.join("control.json"),
            br#"{"schema":1,"requireIdentification":true}"#).unwrap();
        assert!(wait_for_monitor_shutdown_at(&paths, Duration::ZERO).is_err());
        let mut owner = acquire_monitor_owner(&paths).unwrap();
        let generation = uuid::Uuid::new_v4().to_string();
        for state in ["active", "blocked"] {
            write_monitor_receipt(&mut owner, &generation, state).unwrap();
            FileExt::unlock(&owner).unwrap();
            assert!(wait_for_monitor_shutdown_at(&paths, Duration::ZERO).is_err());
            owner.try_lock_exclusive().unwrap();
        }
        owner.set_len(0).unwrap();
        drop(owner);
        assert!(wait_for_monitor_shutdown_at(&paths, Duration::ZERO).is_err());
    }

    #[tokio::test]
    async fn manager_wait_finishes_only_after_original_is_restored() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let original = sha(&fs::read(&service).unwrap());
        let monitor = start_monitor_with_contract(paths.clone(), true, contract).await.unwrap();
        let wait_paths = paths.clone();
        let waiter = tokio::task::spawn_blocking(move || {
            wait_for_monitor_shutdown_at(&wait_paths, Duration::from_secs(5)).unwrap();
            assert_eq!(sha(&fs::read(service).unwrap()), original);
        });
        monitor.stop().await;
        waiter.await.unwrap();
    }

    // The proprietary runtime is supplied locally, never committed or executed by this test.
    #[test]
    #[ignore = "requires CPP_NATIVE_BROWSER_FIXTURE and CPP_NATIVE_BROWSER_DESCRIPTOR"]
    fn pinned_fixture_transaction_recovery_and_external_change() {
        let fixture = PathBuf::from(std::env::var_os("CPP_NATIVE_BROWSER_FIXTURE").unwrap());
        let generated = PathBuf::from(std::env::var_os("CPP_NATIVE_BROWSER_DESCRIPTOR").unwrap());
        let mut data: Value = serde_json::from_slice(&fs::read(&generated).unwrap()).unwrap();
        let source_key = selected_key(&data, fixture.parent().unwrap()).unwrap();
        assert_eq!(
            Some(source_key.as_str()),
            fixture.file_name().unwrap().to_str()
        );
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let key = "0123456789abcdef";
        let runtime = paths.runtime_root.join(key);
        // Preserve Desktop's descriptor spelling; only relocate its checked paths.
        for field in [
            "/mcpServers/cua_repl/command",
            "/mcpServers/cua_repl/env/NODE_REPL_NODE_PATH",
            "/mcpServers/cua_repl/env/CUA_REPL_NODE_REPL_PATH",
            "/mcpServers/cua_repl/args/0",
        ] {
            let value = data.pointer_mut(field).unwrap();
            let relative = Path::new(value.as_str().unwrap())
                .strip_prefix(&fixture)
                .unwrap();
            *value = json!(runtime.join(relative));
        }
        let service = runtime.join(SERVICE);
        fs::create_dir_all(service.parent().unwrap()).unwrap();
        fs::copy(fixture.join(SERVICE), &service).unwrap();
        for (file, _) in RuntimeContract::pinned().files {
            let target = runtime.join(file);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(file), target).unwrap();
        }
        let original = fs::read(&service).unwrap();
        let modified = fs::metadata(&service).unwrap().modified().unwrap();
        let dir = paths
            .codex_home
            .join("plugins/cache/openai-bundled/unified-computer-use/test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(".mcp.json"),
            serde_json::to_vec(&data).unwrap(),
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
