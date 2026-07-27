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

const SAFE_ATOM_KEYS: &[&str] = &[
    "default-service-tier",
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
    let original = Value::Object(state.clone());
    let mut changed_keys = BTreeSet::new();

    normalize_current_state(&mut state, &mut changed_keys);
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

pub fn prepare_projectless_main_window(home: &Path) -> anyhow::Result<AppStateSyncResult> {
    if !projectless_startup_requested(home)? {
        return Ok(AppStateSyncResult {
            changed: false,
            changed_keys: Vec::new(),
            backup_path: None,
            snapshot_path: None,
        });
    }
    let Some(mut state) = load_global_state(home)? else {
        return Ok(AppStateSyncResult {
            changed: false,
            changed_keys: Vec::new(),
            backup_path: None,
            snapshot_path: None,
        });
    };
    let original = Value::Object(state.clone());
    let mut changed_keys = BTreeSet::new();
    replace_if_changed(
        &mut state,
        ACTIVE_WORKSPACE_ROOTS_KEY,
        Value::Array(Vec::new()),
        &mut changed_keys,
    );

    let mut atom = state
        .get("electron-persisted-atom-state")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    atom.insert(
        "electron:onboarding-projectless-completed".to_string(),
        Value::Bool(true),
    );
    atom.insert(
        "electron:onboarding-workspace-autolaunch-applied".to_string(),
        Value::Bool(true),
    );
    replace_if_changed(
        &mut state,
        "electron-persisted-atom-state",
        Value::Object(atom),
        &mut changed_keys,
    );

    let next = Value::Object(state);
    if next == original {
        return Ok(AppStateSyncResult {
            changed: false,
            changed_keys: Vec::new(),
            backup_path: None,
            snapshot_path: None,
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

    Ok(AppStateSyncResult {
        changed: true,
        changed_keys: changed_keys.into_iter().collect(),
        backup_path: Some(backup_path),
        snapshot_path: None,
    })
}

pub fn prepare_projectless_main_window_nonfatal(home: &Path, source: &str) {
    match prepare_projectless_main_window(home) {
        Ok(result) if result.changed => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "codex_app_state.projectless_main_window_prepared",
                json!({
                    "source": source,
                    "changedKeys": result.changed_keys,
                    "backupPath": result.backup_path.map(|path| path.to_string_lossy().to_string()),
                }),
            );
        }
        Ok(_) => {}
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "codex_app_state.projectless_main_window_failed",
                json!({
                    "source": source,
                    "error": error.to_string(),
                }),
            );
        }
    }
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

fn projectless_startup_requested(home: &Path) -> anyhow::Result<bool> {
    let path = home.join("config.toml");
    if !path.exists() {
        return Ok(false);
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let config = text
        .trim_start_matches('\u{feff}')
        .parse::<toml::Table>()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(config
        .get("desktop")
        .and_then(toml::Value::as_table)
        .and_then(|desktop| desktop.get("hotkey-window-projectless-default-enabled"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false))
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
    json!({
        "version": SNAPSHOT_VERSION,
        "state": safe,
    })
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
