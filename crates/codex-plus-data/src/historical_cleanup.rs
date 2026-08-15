use base64::Engine;
use rusqlite::types::{ToSqlOutput, Value as SqlValue, ValueRef};
use rusqlite::{Connection, ToSql, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const FILE_NAMES: [&str; 3] = [
    "session_index.jsonl",
    ".codex-global-state.json",
    ".codex-global-state.json.bak",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalCleanupCandidate {
    pub id: String,
    pub thread_name: String,
    pub updated_at: String,
    pub workspace: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalCleanupPreview {
    pub snapshot_sha256: String,
    pub catalog_revision: i64,
    pub candidates: Vec<HistoricalCleanupCandidate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalCleanupResult {
    pub catalog_rows: usize,
    pub timeline_rows: usize,
    pub session_index_entries: usize,
    pub global_state_references: usize,
    pub global_state_backup_references: usize,
    pub skipped: usize,
    pub backup_dir: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct HistoricalCleanupError {
    pub message: String,
    pub backup_dir: Option<PathBuf>,
    pub partial_result: HistoricalCleanupResult,
}

pub type HistoricalCleanupApplyResult =
    Result<HistoricalCleanupResult, Box<HistoricalCleanupError>>;

#[derive(Clone)]
struct FileSnapshot {
    name: String,
    path: PathBuf,
    existed: bool,
    bytes: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DbRows {
    path: String,
    catalog_rows: Vec<Map<String, Value>>,
    timeline_rows: Vec<Map<String, Value>>,
}

struct CleanupPlan {
    snapshot_sha256: String,
    catalog_revision: i64,
    candidates: Vec<HistoricalCleanupCandidate>,
    files: Vec<FileSnapshot>,
    db_rows: Vec<DbRows>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupFileEntry {
    name: String,
    existed: bool,
    original_sha256: String,
    post_cleanup_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CleanupManifest {
    version: u32,
    namespace: String,
    codex_home: String,
    created_at: String,
    snapshot_sha256: String,
    selected_ids: Vec<String>,
    selected_candidates: Vec<HistoricalCleanupCandidate>,
    files: Vec<BackupFileEntry>,
    databases: Vec<DbRows>,
    deleted_counts: HistoricalCleanupResult,
}

#[derive(Clone)]
struct OwnedSqlValue(SqlValue);

impl ToSql for OwnedSqlValue {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(self.0.clone()))
    }
}

pub fn preview_historical_cleanup(home: Option<&Path>) -> anyhow::Result<HistoricalCleanupPreview> {
    let home = home
        .map(Path::to_path_buf)
        .unwrap_or_else(codex_plus_core::codex_home::default_codex_home_dir);
    let plan = build_plan(&home)?;
    Ok(HistoricalCleanupPreview {
        snapshot_sha256: plan.snapshot_sha256,
        catalog_revision: plan.catalog_revision,
        candidates: plan.candidates,
    })
}

pub fn apply_historical_cleanup(
    home: Option<&Path>,
    expected_snapshot_sha256: &str,
    selected_ids: &[String],
) -> HistoricalCleanupApplyResult {
    let require_stopped_app = home.is_none();
    ensure_codex_stopped(require_stopped_app, None)?;
    let home = home
        .map(Path::to_path_buf)
        .unwrap_or_else(codex_plus_core::codex_home::default_codex_home_dir);
    let lock = acquire_cleanup_lock(&home)?;
    let result = apply_locked(
        &home,
        expected_snapshot_sha256,
        selected_ids,
        require_stopped_app,
    );
    let _ = fs::remove_dir(&lock);
    result
}

fn apply_locked(
    home: &Path,
    expected_snapshot_sha256: &str,
    selected_ids: &[String],
    require_stopped_app: bool,
) -> HistoricalCleanupApplyResult {
    let plan = build_plan(home).map_err(|error| cleanup_error(error, None))?;
    if plan.snapshot_sha256 != expected_snapshot_sha256 {
        return Err(cleanup_error(
            "SQLite、任务索引或全局状态已在预览后发生变化；本次清理已中止，请重新预览",
            None,
        ));
    }
    let candidate_ids = plan
        .candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<HashSet<_>>();
    let selected = selected_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    if selected.is_empty() {
        return Ok(HistoricalCleanupResult::default());
    }
    if selected
        .iter()
        .any(|id| !candidate_ids.contains(id.as_str()))
    {
        return Err(cleanup_error(
            "确认列表已过期或包含非候选会话；本次清理未执行，请重新预览",
            None,
        ));
    }
    ensure_codex_stopped(require_stopped_app, None)?;

    let selected_set = selected.iter().cloned().collect::<HashSet<_>>();
    let (next_files, file_counts) =
        cleaned_files(&plan.files, &selected_set).map_err(|error| cleanup_error(error, None))?;
    let selected_db_rows = selected_database_rows(&plan.db_rows, &selected_set);
    let mut result = HistoricalCleanupResult {
        session_index_entries: file_counts[0],
        global_state_references: file_counts[1],
        global_state_backup_references: file_counts[2],
        ..HistoricalCleanupResult::default()
    };
    result.catalog_rows = selected_db_rows
        .iter()
        .map(|rows| rows.catalog_rows.len())
        .sum();
    result.timeline_rows = selected_db_rows
        .iter()
        .map(|rows| rows.timeline_rows.len())
        .sum();
    let backup_dir = create_backup(
        home,
        &plan,
        &selected,
        &next_files,
        &selected_db_rows,
        &result,
    )?;
    result.backup_dir = Some(backup_dir.clone());
    let mut completed = HistoricalCleanupResult {
        backup_dir: Some(backup_dir.clone()),
        ..HistoricalCleanupResult::default()
    };

    let current =
        build_plan(home).map_err(|error| cleanup_error(error, Some(backup_dir.clone())))?;
    if current.snapshot_sha256 != plan.snapshot_sha256 {
        return Err(cleanup_error(
            "数据在备份过程中发生变化；未继续写入，请重新预览",
            Some(backup_dir),
        ));
    }

    for (snapshot, next) in plan.files.iter().zip(next_files.iter()) {
        if snapshot.bytes == *next {
            continue;
        }
        codex_plus_core::settings::atomic_write(&snapshot.path, next).map_err(|error| {
            cleanup_error_with_progress(error, Some(backup_dir.clone()), completed.clone())
        })?;
        match snapshot.name.as_str() {
            "session_index.jsonl" => completed.session_index_entries = file_counts[0],
            ".codex-global-state.json" => completed.global_state_references = file_counts[1],
            ".codex-global-state.json.bak" => {
                completed.global_state_backup_references = file_counts[2];
            }
            _ => {}
        }
    }
    for rows in &selected_db_rows {
        delete_database_rows(rows, &selected_set).map_err(|error| {
            cleanup_error_with_progress(
                format!("数据库 {} 清理失败：{error}", rows.path),
                Some(backup_dir.clone()),
                completed.clone(),
            )
        })?;
        completed.catalog_rows += rows.catalog_rows.len();
        completed.timeline_rows += rows.timeline_rows.len();
    }
    Ok(result)
}

pub fn undo_historical_cleanup(
    home: Option<&Path>,
    backup_dir: &Path,
) -> HistoricalCleanupApplyResult {
    let require_stopped_app = home.is_none();
    ensure_codex_stopped(require_stopped_app, Some(backup_dir.to_path_buf()))?;
    let home = home
        .map(Path::to_path_buf)
        .unwrap_or_else(codex_plus_core::codex_home::default_codex_home_dir);
    let backup_dir = validate_backup_dir(&home, backup_dir)?;
    let manifest: CleanupManifest = serde_json::from_slice(
        &fs::read(backup_dir.join("manifest.json"))
            .map_err(|error| cleanup_error(error, Some(backup_dir.clone())))?,
    )
    .map_err(|error| cleanup_error(error, Some(backup_dir.clone())))?;
    let selected = manifest
        .selected_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let live = collect_real_thread_ids(&home, &database_paths(&home))
        .map_err(|error| cleanup_error(error, Some(backup_dir.clone())))?;
    if selected.iter().any(|id| live.contains(id)) {
        return Err(cleanup_error(
            "检测到同 ID 的新会话或 rollout，撤销已拒绝，未覆盖新内容",
            Some(backup_dir),
        ));
    }
    preflight_restore_databases(&manifest.databases, &selected)
        .map_err(|error| cleanup_error(error, Some(backup_dir.clone())))?;
    for file in &manifest.files {
        let path = home.join(&file.name);
        let current = fs::read(&path).unwrap_or_default();
        if sha256_hex(&current) != file.post_cleanup_sha256 {
            return Err(cleanup_error(
                format!("{} 已在清理后发生变化，撤销已拒绝", file.name),
                Some(backup_dir),
            ));
        }
    }
    for rows in &manifest.databases {
        restore_database_rows(rows)
            .map_err(|error| cleanup_error(error, Some(backup_dir.clone())))?;
    }
    for file in &manifest.files {
        let path = home.join(&file.name);
        if file.existed {
            let bytes = fs::read(backup_dir.join(&file.name))
                .map_err(|error| cleanup_error(error, Some(backup_dir.clone())))?;
            codex_plus_core::settings::atomic_write(&path, &bytes)
                .map_err(|error| cleanup_error(error, Some(backup_dir.clone())))?;
        } else if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| cleanup_error(error, Some(backup_dir.clone())))?;
        }
    }
    Ok(manifest.deleted_counts)
}

fn build_plan(home: &Path) -> anyhow::Result<CleanupPlan> {
    let paths = database_paths(home);
    let mut live_ids = collect_real_thread_ids(home, &paths)?;
    let mut db_rows = Vec::new();
    let mut candidates = BTreeMap::<String, HistoricalCleanupCandidate>::new();
    let mut catalog_revision = 0_i64;
    for path in &paths {
        if !path.exists() {
            continue;
        }
        let db = Connection::open(path)?;
        let host_id = local_host_id(&db)?;
        live_ids.extend(remote_catalog_thread_ids(&db, host_id.as_deref())?);
        let catalog_rows = select_thread_rows(&db, "local_thread_catalog", host_id.as_deref())?;
        let timeline_rows = select_thread_rows(&db, "thread_timeline_ledger", host_id.as_deref())?;
        catalog_revision += catalog_revision_value(&db)?;
        for row in &catalog_rows {
            let id = row_string(row, "thread_id");
            let source_detail = row_string(row, "source_detail");
            if !source_detail.is_empty() && Path::new(&source_detail).is_file() {
                live_ids.insert(id.clone());
            }
            merge_candidate(&mut candidates, row, "catalog");
        }
        for row in &timeline_rows {
            merge_candidate(&mut candidates, row, "timeline");
        }
        db_rows.push(DbRows {
            path: path.to_string_lossy().to_string(),
            catalog_rows,
            timeline_rows,
        });
    }
    let files = FILE_NAMES
        .iter()
        .map(|name| file_snapshot(home, name))
        .collect::<anyhow::Result<Vec<_>>>()?;
    add_file_sources(&files, &mut candidates)?;
    candidates.retain(|id, _| !live_ids.contains(id));
    for candidate in candidates.values_mut() {
        candidate.sources.sort();
        candidate.sources.dedup();
    }
    let candidates = candidates.into_values().collect::<Vec<_>>();
    let fingerprint = json!({
        "liveIds": live_ids.into_iter().collect::<BTreeSet<_>>(),
        "databases": db_rows,
        "files": files.iter().map(|file| json!({"name": file.name, "existed": file.existed, "sha256": sha256_hex(&file.bytes)})).collect::<Vec<_>>(),
        "catalogRevision": catalog_revision,
        "candidates": candidates,
    });
    Ok(CleanupPlan {
        snapshot_sha256: sha256_hex(&serde_json::to_vec(&fingerprint)?),
        catalog_revision,
        candidates,
        files,
        db_rows,
    })
}

fn database_paths(home: &Path) -> Vec<PathBuf> {
    codex_plus_core::codex_sqlite::codex_thread_reference_db_paths_from_home(home)
}

fn collect_real_thread_ids(home: &Path, paths: &[PathBuf]) -> anyhow::Result<HashSet<String>> {
    let mut ids = HashSet::new();
    for root_name in ["sessions", "archived_sessions"] {
        collect_rollout_ids(&home.join(root_name), &mut ids)?;
    }
    for path in paths {
        if !path.exists() {
            continue;
        }
        let db = Connection::open(path)?;
        for (table, column) in [
            ("threads", "id"),
            ("automation_runs", "thread_id"),
            ("inbox_items", "thread_id"),
            ("sessions", "id"),
            ("messages", "session_id"),
            ("thread_dynamic_tools", "thread_id"),
            ("thread_goals", "thread_id"),
            ("stage1_outputs", "thread_id"),
            ("agent_job_items", "assigned_thread_id"),
        ] {
            if !table_columns(&db, table)?.contains(column) {
                continue;
            }
            let sql =
                format!("SELECT DISTINCT {column} FROM {table} WHERE COALESCE({column}, '') <> ''");
            let mut stmt = db.prepare(&sql)?;
            for id in stmt.query_map([], |row| row.get::<_, String>(0))? {
                ids.insert(id?);
            }
        }
    }
    Ok(ids)
}

fn collect_rollout_ids(root: &Path, ids: &mut HashSet<String>) -> anyhow::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_rollout_ids(&path, ids)?;
            continue;
        }
        if rollout_id_from_name(&path).is_none() {
            continue;
        }
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        for line in text.lines() {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if value.get("type").and_then(Value::as_str) == Some("session_meta")
                && let Some(id) = value.pointer("/payload/id").and_then(Value::as_str)
            {
                ids.insert(id.to_string());
            }
        }
        if let Some(id) = rollout_id_from_name(&path) {
            ids.insert(id);
        }
    }
    Ok(())
}

fn rollout_id_from_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    (stem.len() >= 36).then(|| stem[stem.len() - 36..].to_string())
}

fn file_snapshot(home: &Path, name: &str) -> anyhow::Result<FileSnapshot> {
    let path = home.join(name);
    let existed = path.exists();
    let bytes = if existed {
        fs::read(&path)?
    } else {
        Vec::new()
    };
    Ok(FileSnapshot {
        name: name.to_string(),
        path,
        existed,
        bytes,
    })
}

fn add_file_sources(
    files: &[FileSnapshot],
    candidates: &mut BTreeMap<String, HistoricalCleanupCandidate>,
) -> anyhow::Result<()> {
    for file in files {
        if !file.existed || file.name != "session_index.jsonl" {
            continue;
        }
        let text = std::str::from_utf8(&file.bytes)?;
        for line in text.lines() {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(object) = value.as_object() else {
                continue;
            };
            if object.len() != 3
                || !["id", "thread_name", "updated_at"]
                    .iter()
                    .all(|key| object.contains_key(*key))
            {
                continue;
            }
            let Some(id) = value
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
            else {
                continue;
            };
            let candidate =
                candidates
                    .entry(id.to_string())
                    .or_insert_with(|| HistoricalCleanupCandidate {
                        id: id.to_string(),
                        thread_name: String::new(),
                        updated_at: String::new(),
                        workspace: String::new(),
                        sources: Vec::new(),
                    });
            candidate.sources.push("session_index".to_string());
            fill_candidate_from_index(candidate, &value);
        }
    }
    let known = candidates.keys().cloned().collect::<Vec<_>>();
    for file in files {
        if !file.existed || file.name == "session_index.jsonl" {
            continue;
        }
        let value: Value = serde_json::from_slice(&file.bytes)?;
        for id in &known {
            if has_structural_reference(&value, id)
                && let Some(candidate) = candidates.get_mut(id)
            {
                candidate.sources.push(if file.name.ends_with(".bak") {
                    "global_state_bak".to_string()
                } else {
                    "global_state".to_string()
                });
            }
        }
    }
    Ok(())
}

fn merge_candidate(
    candidates: &mut BTreeMap<String, HistoricalCleanupCandidate>,
    row: &Map<String, Value>,
    source: &str,
) {
    let id = row_string(row, "thread_id");
    if id.is_empty() {
        return;
    }
    let candidate = candidates
        .entry(id.clone())
        .or_insert_with(|| HistoricalCleanupCandidate {
            id,
            thread_name: String::new(),
            updated_at: String::new(),
            workspace: String::new(),
            sources: Vec::new(),
        });
    candidate.sources.push(source.to_string());
    for key in ["display_title", "title"] {
        if candidate.thread_name.is_empty() {
            candidate.thread_name = row_string(row, key);
        }
    }
    for key in ["source_updated_at", "updated_at", "updated_at_ms"] {
        if candidate.updated_at.is_empty() {
            candidate.updated_at = row.get(key).map(value_text).unwrap_or_default();
        }
    }
    if candidate.workspace.is_empty() {
        candidate.workspace = row_string(row, "cwd");
    }
}

fn fill_candidate_from_index(candidate: &mut HistoricalCleanupCandidate, value: &Value) {
    if candidate.thread_name.is_empty() {
        candidate.thread_name = value
            .get("thread_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    if candidate.updated_at.is_empty() {
        candidate.updated_at = value
            .get("updated_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
}

fn cleaned_files(
    files: &[FileSnapshot],
    selected: &HashSet<String>,
) -> anyhow::Result<(Vec<Vec<u8>>, [usize; 3])> {
    let mut next = Vec::new();
    let mut counts = [0; 3];
    for (index, file) in files.iter().enumerate() {
        if !file.existed {
            next.push(Vec::new());
        } else if file.name == "session_index.jsonl" {
            let text = std::str::from_utf8(&file.bytes)?;
            let mut output = String::new();
            for segment in text.split_inclusive('\n') {
                let (line, ending) = split_line_ending(segment);
                let remove = serde_json::from_str::<Value>(line)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("id")
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                    })
                    .is_some_and(|id| selected.contains(&id));
                if remove {
                    counts[index] += 1;
                } else {
                    output.push_str(line);
                    output.push_str(ending);
                }
            }
            next.push(output.into_bytes());
        } else {
            let mut value: Value = serde_json::from_slice(&file.bytes)?;
            counts[index] = prune_structural_references(&mut value, selected);
            next.push(serde_json::to_vec_pretty(&value)?);
        }
    }
    Ok((next, counts))
}

fn has_structural_reference(value: &Value, id: &str) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            key.contains(id) || value.as_str() == Some(id) || has_structural_reference(value, id)
        }),
        Value::Array(items) => items
            .iter()
            .any(|value| value.as_str() == Some(id) || has_structural_reference(value, id)),
        _ => false,
    }
}

fn prune_structural_references(value: &mut Value, selected: &HashSet<String>) -> usize {
    match value {
        Value::Object(map) => {
            let remove = map
                .iter()
                .filter(|(key, value)| {
                    selected.iter().any(|id| key.contains(id))
                        || value.as_str().is_some_and(|text| selected.contains(text))
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            let mut count = remove.len();
            for key in remove {
                map.remove(&key);
            }
            count += map
                .values_mut()
                .map(|value| prune_structural_references(value, selected))
                .sum::<usize>();
            count
        }
        Value::Array(items) => {
            let before = items.len();
            items.retain(|value| !value.as_str().is_some_and(|text| selected.contains(text)));
            before - items.len()
                + items
                    .iter_mut()
                    .map(|value| prune_structural_references(value, selected))
                    .sum::<usize>()
        }
        _ => 0,
    }
}

fn selected_database_rows(rows: &[DbRows], selected: &HashSet<String>) -> Vec<DbRows> {
    rows.iter()
        .filter_map(|rows| {
            let catalog_rows = rows
                .catalog_rows
                .iter()
                .filter(|row| selected.contains(&row_string(row, "thread_id")))
                .cloned()
                .collect::<Vec<_>>();
            let timeline_rows = rows
                .timeline_rows
                .iter()
                .filter(|row| selected.contains(&row_string(row, "thread_id")))
                .cloned()
                .collect::<Vec<_>>();
            (!catalog_rows.is_empty() || !timeline_rows.is_empty()).then(|| DbRows {
                path: rows.path.clone(),
                catalog_rows,
                timeline_rows,
            })
        })
        .collect()
}

fn create_backup(
    home: &Path,
    plan: &CleanupPlan,
    selected: &BTreeSet<String>,
    next_files: &[Vec<u8>],
    databases: &[DbRows],
    counts: &HistoricalCleanupResult,
) -> Result<PathBuf, Box<HistoricalCleanupError>> {
    let root = home.join("backups_state/history-cleanup");
    let mut dir = root.join(chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f").to_string());
    let mut suffix = 0;
    while dir.exists() {
        suffix += 1;
        dir = root.join(format!(
            "{}-{suffix}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f")
        ));
    }
    fs::create_dir_all(&dir).map_err(|error| cleanup_error(error, None))?;
    let mut files = Vec::new();
    for (snapshot, next) in plan.files.iter().zip(next_files.iter()) {
        if snapshot.existed {
            fs::write(dir.join(&snapshot.name), &snapshot.bytes)
                .map_err(|error| cleanup_error(error, Some(dir.clone())))?;
        }
        files.push(BackupFileEntry {
            name: snapshot.name.clone(),
            existed: snapshot.existed,
            original_sha256: sha256_hex(&snapshot.bytes),
            post_cleanup_sha256: sha256_hex(next),
        });
    }
    let manifest = CleanupManifest {
        version: 1,
        namespace: "codex-plus-history-cleanup".to_string(),
        codex_home: home.to_string_lossy().to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        snapshot_sha256: plan.snapshot_sha256.clone(),
        selected_ids: selected.iter().cloned().collect(),
        selected_candidates: plan
            .candidates
            .iter()
            .filter(|candidate| selected.contains(&candidate.id))
            .cloned()
            .collect(),
        files,
        databases: databases.to_vec(),
        deleted_counts: counts.clone(),
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| cleanup_error(error, Some(dir.clone())))?;
    fs::write(dir.join("manifest.json"), manifest_bytes)
        .map_err(|error| cleanup_error(error, Some(dir.clone())))?;
    Ok(dir)
}

fn delete_database_rows(rows: &DbRows, selected: &HashSet<String>) -> anyhow::Result<()> {
    let mut db = Connection::open(&rows.path)?;
    let tx = db.transaction()?;
    let host = local_host_id(&tx)?;
    let mut catalog_deleted = 0;
    for table in ["local_thread_catalog", "thread_timeline_ledger"] {
        if !table_columns(&tx, table)?.contains("thread_id") {
            continue;
        }
        for id in selected {
            let count = if table_columns(&tx, table)?.contains("host_id") {
                tx.execute(
                    &format!("DELETE FROM {table} WHERE host_id = ?1 AND thread_id = ?2"),
                    (host.as_deref().unwrap_or("local"), id),
                )?
            } else {
                tx.execute(&format!("DELETE FROM {table} WHERE thread_id = ?1"), [id])?
            };
            if table == "local_thread_catalog" {
                catalog_deleted += count;
            }
        }
    }
    increment_catalog_revision(&tx, catalog_deleted)?;
    tx.commit()?;
    Ok(())
}

fn preflight_restore_databases(rows: &[DbRows], selected: &HashSet<String>) -> anyhow::Result<()> {
    for rows in rows {
        let db = Connection::open(&rows.path)?;
        for table in ["local_thread_catalog", "thread_timeline_ledger"] {
            if !table_columns(&db, table)?.contains("thread_id") {
                continue;
            }
            for id in selected {
                let exists: i64 = db.query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE thread_id = ?1"),
                    [id],
                    |row| row.get(0),
                )?;
                if exists > 0 {
                    anyhow::bail!("restore conflict: {table} already contains {id}");
                }
            }
        }
        let source_ids = real_ids_in_db(&db)?;
        if selected.iter().any(|id| source_ids.contains(id)) {
            anyhow::bail!("restore conflict: a real session with the same ID exists");
        }
    }
    Ok(())
}

fn restore_database_rows(rows: &DbRows) -> anyhow::Result<()> {
    let mut db = Connection::open(&rows.path)?;
    let tx = db.transaction()?;
    for row in &rows.catalog_rows {
        insert_row(&tx, "local_thread_catalog", row)?;
    }
    for row in &rows.timeline_rows {
        insert_row(&tx, "thread_timeline_ledger", row)?;
    }
    increment_catalog_revision(&tx, rows.catalog_rows.len())?;
    tx.commit()?;
    Ok(())
}

fn validate_backup_dir(home: &Path, dir: &Path) -> Result<PathBuf, Box<HistoricalCleanupError>> {
    let root = fs::canonicalize(home.join("backups_state/history-cleanup"))
        .map_err(|error| cleanup_error(error, None))?;
    let dir = fs::canonicalize(dir).map_err(|error| cleanup_error(error, None))?;
    if !dir.starts_with(&root) {
        return Err(cleanup_error("备份目录不属于历史残留清理", None));
    }
    Ok(dir)
}

fn acquire_cleanup_lock(home: &Path) -> Result<PathBuf, Box<HistoricalCleanupError>> {
    let lock = home.join("tmp/history-cleanup.lock");
    if let Some(parent) = lock.parent() {
        fs::create_dir_all(parent).map_err(|error| cleanup_error(error, None))?;
    }
    fs::create_dir(&lock).map_err(|_| cleanup_error("已有历史残留清理正在运行", None))?;
    Ok(lock)
}

fn ensure_codex_stopped(
    required: bool,
    backup: Option<PathBuf>,
) -> Result<(), Box<HistoricalCleanupError>> {
    if !required {
        return Ok(());
    }
    let pids = codex_plus_core::watcher::find_session_index_cleanup_blocking_processes();
    if pids.is_empty() {
        return Ok(());
    }
    Err(cleanup_error(
        format!(
            "Codex App / ChatGPT 仍在运行（进程：{}）；请完全退出后重新预览",
            pids.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        backup,
    ))
}

fn cleanup_error(
    message: impl std::fmt::Display,
    backup_dir: Option<PathBuf>,
) -> Box<HistoricalCleanupError> {
    Box::new(HistoricalCleanupError {
        message: message.to_string(),
        backup_dir,
        partial_result: HistoricalCleanupResult::default(),
    })
}

fn cleanup_error_with_progress(
    message: impl std::fmt::Display,
    backup_dir: Option<PathBuf>,
    partial_result: HistoricalCleanupResult,
) -> Box<HistoricalCleanupError> {
    Box::new(HistoricalCleanupError {
        message: message.to_string(),
        backup_dir,
        partial_result,
    })
}

fn select_thread_rows(
    db: &Connection,
    table: &str,
    host: Option<&str>,
) -> anyhow::Result<Vec<Map<String, Value>>> {
    let columns = table_columns(db, table)?;
    if !columns.contains("thread_id") {
        return Ok(Vec::new());
    }
    let (sql, args): (String, Vec<OwnedSqlValue>) = if columns.contains("host_id") {
        (
            format!(
                "SELECT * FROM {table} WHERE host_id = ?1 AND COALESCE(thread_id, '') <> '' ORDER BY thread_id"
            ),
            vec![OwnedSqlValue(SqlValue::Text(
                host.unwrap_or("local").to_string(),
            ))],
        )
    } else {
        (
            format!("SELECT * FROM {table} WHERE COALESCE(thread_id, '') <> '' ORDER BY thread_id"),
            Vec::new(),
        )
    };
    select_rows(db, &sql, &args)
}

fn remote_catalog_thread_ids(
    db: &Connection,
    local_host: Option<&str>,
) -> anyhow::Result<HashSet<String>> {
    let columns = table_columns(db, "local_thread_catalog")?;
    if !columns.contains("thread_id") || !columns.contains("host_id") {
        return Ok(HashSet::new());
    }
    let mut stmt = db.prepare(
        "SELECT DISTINCT thread_id FROM local_thread_catalog
         WHERE host_id <> ?1 AND COALESCE(thread_id, '') <> ''",
    )?;
    Ok(stmt
        .query_map([local_host.unwrap_or("local")], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<HashSet<_>>>()?)
}

fn select_rows(
    db: &Connection,
    sql: &str,
    args: &[OwnedSqlValue],
) -> anyhow::Result<Vec<Map<String, Value>>> {
    let mut stmt = db.prepare(sql)?;
    let columns = stmt
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    let refs = args
        .iter()
        .map(|value| value as &dyn ToSql)
        .collect::<Vec<_>>();
    Ok(stmt
        .query_map(refs.as_slice(), |row| {
            let mut map = Map::new();
            for (index, column) in columns.iter().enumerate() {
                map.insert(column.clone(), sql_to_json(row.get_ref(index)?));
            }
            Ok(map)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn table_columns(db: &Connection, table: &str) -> anyhow::Result<HashSet<String>> {
    let mut stmt = db.prepare(&format!(
        "PRAGMA table_info(\"{}\")",
        table.replace('"', "\"\"")
    ))?;
    Ok(stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<HashSet<_>>>()?)
}

fn local_host_id(db: &Connection) -> anyhow::Result<Option<String>> {
    let columns = table_columns(db, "local_thread_catalog_hosts")?;
    if !columns.contains("host_id") {
        return Ok(Some("local".to_string()));
    }
    let sql = if columns.contains("host_kind") {
        "SELECT host_id FROM local_thread_catalog_hosts WHERE LOWER(COALESCE(host_kind, '')) = 'local' ORDER BY host_id LIMIT 1"
    } else {
        "SELECT host_id FROM local_thread_catalog_hosts WHERE host_id = 'local' LIMIT 1"
    };
    Ok(db
        .query_row(sql, [], |row| row.get::<_, String>(0))
        .ok()
        .or_else(|| Some("local".to_string())))
}

fn catalog_revision_value(db: &Connection) -> anyhow::Result<i64> {
    if !table_columns(db, "local_thread_catalog_metadata")?.contains("catalog_revision") {
        return Ok(0);
    }
    Ok(db.query_row(
        "SELECT COALESCE(MAX(catalog_revision), 0) FROM local_thread_catalog_metadata",
        [],
        |row| row.get(0),
    )?)
}

fn increment_catalog_revision(db: &Connection, amount: usize) -> anyhow::Result<()> {
    if amount == 0
        || !table_columns(db, "local_thread_catalog_metadata")?.contains("catalog_revision")
    {
        return Ok(());
    }
    let updated = db.execute(
        "UPDATE local_thread_catalog_metadata SET catalog_revision = catalog_revision + ?1",
        [amount as i64],
    )?;
    if updated == 0 {
        let columns = table_columns(db, "local_thread_catalog_metadata")?;
        if columns.len() == 1 {
            db.execute(
                "INSERT INTO local_thread_catalog_metadata (catalog_revision) VALUES (?1)",
                [amount as i64],
            )?;
        } else if columns.contains("id") {
            db.execute(
                "INSERT INTO local_thread_catalog_metadata (id, catalog_revision) VALUES (1, ?1)",
                [amount as i64],
            )?;
        } else if columns.contains("host_id") {
            db.execute(
                "INSERT INTO local_thread_catalog_metadata (host_id, catalog_revision) VALUES (?1, ?2)",
                (local_host_id(db)?.as_deref().unwrap_or("local"), amount as i64),
            )?;
        }
    }
    Ok(())
}

fn real_ids_in_db(db: &Connection) -> anyhow::Result<HashSet<String>> {
    let mut ids = HashSet::new();
    for (table, column) in [
        ("threads", "id"),
        ("automation_runs", "thread_id"),
        ("inbox_items", "thread_id"),
        ("sessions", "id"),
        ("messages", "session_id"),
        ("thread_dynamic_tools", "thread_id"),
        ("thread_goals", "thread_id"),
        ("stage1_outputs", "thread_id"),
        ("agent_job_items", "assigned_thread_id"),
    ] {
        if !table_columns(db, table)?.contains(column) {
            continue;
        }
        let mut stmt = db.prepare(&format!(
            "SELECT {column} FROM {table} WHERE COALESCE({column}, '') <> ''"
        ))?;
        for id in stmt.query_map([], |row| row.get::<_, String>(0))? {
            ids.insert(id?);
        }
    }
    Ok(ids)
}

fn insert_row(db: &Connection, table: &str, row: &Map<String, Value>) -> anyhow::Result<()> {
    let columns = row.keys().collect::<Vec<_>>();
    let quoted = columns
        .iter()
        .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let marks = (1..=columns.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let values = columns
        .iter()
        .map(|column| OwnedSqlValue(json_to_sql(&row[*column])))
        .collect::<Vec<_>>();
    db.execute(
        &format!("INSERT INTO {table} ({quoted}) VALUES ({marks})"),
        params_from_iter(values),
    )?;
    Ok(())
}

fn sql_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(v) => json!(v),
        ValueRef::Real(v) => json!(v),
        ValueRef::Text(v) => Value::String(String::from_utf8_lossy(v).to_string()),
        ValueRef::Blob(v) => {
            json!({"__sqlite_blob_b64": base64::engine::general_purpose::STANDARD.encode(v)})
        }
    }
}

fn json_to_sql(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(v) => SqlValue::Integer(i64::from(*v)),
        Value::Number(v) if v.is_i64() => SqlValue::Integer(v.as_i64().unwrap()),
        Value::Number(v) => SqlValue::Real(v.as_f64().unwrap_or_default()),
        Value::String(v) => SqlValue::Text(v.clone()),
        Value::Object(map) if map.len() == 1 && map.contains_key("__sqlite_blob_b64") => {
            let bytes = map
                .get("__sqlite_blob_b64")
                .and_then(Value::as_str)
                .and_then(|encoded| {
                    base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .ok()
                })
                .unwrap_or_default();
            SqlValue::Blob(bytes)
        }
        other => SqlValue::Text(other.to_string()),
    }
}

fn row_string(row: &Map<String, Value>, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| value.to_string())
}
fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn split_line_ending(segment: &str) -> (&str, &str) {
    if let Some(line) = segment.strip_suffix("\r\n") {
        (line, "\r\n")
    } else if let Some(line) = segment.strip_suffix('\n') {
        (line, "\n")
    } else {
        (segment, "")
    }
}
