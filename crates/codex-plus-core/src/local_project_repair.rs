use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const ASSIGNMENTS_KEY: &str = "thread-project-assignments";
const MAX_ROLLOUT_LINES: usize = 64;
const MAX_ROLLOUT_LINE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalProjectRepairReport {
    pub existing_assignments: usize,
    pub safe_assignments_found: usize,
    pub unmatched: usize,
    pub ambiguous: usize,
    pub missing_cwd: usize,
    pub duplicate_rollout_thread_ids: usize,
    pub malformed_rollout_lines: usize,
    pub applied_assignments: usize,
    pub backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct PlannedAssignment {
    thread_id: String,
    project_id: String,
    cwd: String,
}

#[derive(Debug, Clone)]
struct RepairPlan {
    report: LocalProjectRepairReport,
    assignments: Vec<PlannedAssignment>,
}

/// Scans Codex rollout metadata and returns only assignments that can be made safely.
pub fn analyze_local_project_assignments(home: &Path) -> anyhow::Result<LocalProjectRepairReport> {
    Ok(build_repair_plan(home)?.report)
}

/// Applies the safe assignment plan. This never replaces an existing assignment.
pub fn apply_local_project_assignments(home: &Path) -> anyhow::Result<LocalProjectRepairReport> {
    apply_local_project_assignments_with_writer(home, |path, bytes| {
        crate::settings::atomic_write(path, bytes)
    })
}

fn apply_local_project_assignments_with_writer<F>(
    home: &Path,
    write: F,
) -> anyhow::Result<LocalProjectRepairReport>
where
    F: Fn(&Path, &[u8]) -> anyhow::Result<()>,
{
    let plan = build_repair_plan(home)?;
    if plan.assignments.is_empty() {
        return Ok(plan.report);
    }

    let state_path = home.join(GLOBAL_STATE_FILE);
    let original_bytes = fs::read(&state_path)
        .with_context(|| format!("failed to read {}", state_path.display()))?;
    let mut state = parse_state(&original_bytes, &state_path)?;
    let assignments = state
        .get_mut(ASSIGNMENTS_KEY)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("{ASSIGNMENTS_KEY} must be a JSON object"))?;

    let mut applied = Vec::new();
    for item in &plan.assignments {
        if !assignments.contains_key(&item.thread_id) {
            assignments.insert(
                item.thread_id.clone(),
                json!({
                    "projectKind": "local",
                    "projectId": item.project_id,
                    "cwd": item.cwd,
                    "pendingCoreUpdate": false,
                }),
            );
            applied.push(item.clone());
        }
    }
    if applied.is_empty() {
        return Ok(plan.report);
    }

    let backup_path = create_backup(home, &original_bytes, applied.len())?;
    let next_bytes = serde_json::to_vec_pretty(&Value::Object(state))?;
    if let Err(error) = write(&state_path, &next_bytes) {
        let _ = restore_backup(&state_path, &original_bytes);
        return Err(error).context("failed to write repaired global state");
    }

    if let Err(error) = verify_assignments(&state_path, &applied) {
        let restore_result = restore_backup(&state_path, &original_bytes);
        return match restore_result {
            Ok(()) => Err(error).context("repair verification failed; restored original state"),
            Err(restore_error) => Err(error).context(format!(
                "repair verification failed and restoring {} also failed: {restore_error}",
                backup_path.display()
            )),
        };
    }

    let mut report = plan.report;
    report.applied_assignments = applied.len();
    report.backup_path = Some(backup_path);
    Ok(report)
}

fn build_repair_plan(home: &Path) -> anyhow::Result<RepairPlan> {
    let state_path = home.join(GLOBAL_STATE_FILE);
    let state = parse_state(
        &fs::read(&state_path).with_context(|| format!("failed to read {}", state_path.display()))?,
        &state_path,
    )?;
    let assignments = required_object(&state, ASSIGNMENTS_KEY)?;
    let roots = project_roots(&state)?;
    let mut report = LocalProjectRepairReport {
        existing_assignments: assignments.len(),
        ..Default::default()
    };

    let mut rollouts: HashMap<String, Vec<Option<String>>> = HashMap::new();
    for path in rollout_files(&home.join("sessions"))? {
        let Some(thread_id) = rollout_thread_id(&path) else { continue };
        let (cwd, malformed) = rollout_cwd(&path)?;
        report.malformed_rollout_lines += malformed;
        rollouts.entry(thread_id).or_default().push(cwd);
    }

    let mut planned = Vec::new();
    for (thread_id, records) in rollouts {
        if records.len() != 1 {
            report.duplicate_rollout_thread_ids += 1;
            continue;
        }
        if assignments.contains_key(&thread_id) {
            continue;
        }
        let Some(cwd) = records.into_iter().next().flatten() else {
            report.missing_cwd += 1;
            continue;
        };
        let Some(normalized) = normalize_path(&cwd) else {
            report.missing_cwd += 1;
            continue;
        };
        match roots.get(&normalized) {
            Some(ids) if ids.len() == 1 => planned.push(PlannedAssignment {
                thread_id,
                project_id: ids.iter().next().expect("one project id").clone(),
                cwd,
            }),
            Some(_) => report.ambiguous += 1,
            None => report.unmatched += 1,
        }
    }
    report.safe_assignments_found = planned.len();
    Ok(RepairPlan { report, assignments: planned })
}

fn project_roots(state: &Map<String, Value>) -> anyhow::Result<BTreeMap<String, HashSet<String>>> {
    let local_projects = required_object(state, "local-projects")?;
    let order = required_array_of_strings(state, "project-order")?;
    let saved_roots = required_array_of_strings(state, "electron-saved-workspace-roots")?;
    if order.len() != saved_roots.len() {
        bail!("project-order and electron-saved-workspace-roots must have the same length");
    }

    let mut result: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    for (project_id, root) in order.iter().zip(saved_roots) {
        let Some(project) = local_projects.get(project_id).and_then(Value::as_object) else {
            continue;
        };
        if project.get("id").and_then(Value::as_str) != Some(project_id.as_str()) {
            continue;
        }
        let Some(root) = normalize_path(root) else { continue };
        result.entry(root).or_default().insert(project_id.clone());
    }
    Ok(result)
}

fn rollout_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() { return Ok(files); }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(rollout_files(&path)?);
        } else if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl")) {
            files.push(path);
        }
    }
    Ok(files)
}

fn rollout_thread_id(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    let candidate = stem.get(stem.len().checked_sub(36)?..)?;
    Uuid::parse_str(candidate).ok().map(|id| id.to_string())
}

fn rollout_cwd(path: &Path) -> anyhow::Result<(Option<String>, usize)> {
    let file = fs::File::open(path)?;
    let mut malformed = 0;
    let max_bytes = MAX_ROLLOUT_LINES as u64 * MAX_ROLLOUT_LINE_BYTES;
    for line in BufReader::new(file).take(max_bytes).lines().take(MAX_ROLLOUT_LINES) {
        let line = line?;
        if line.len() as u64 > MAX_ROLLOUT_LINE_BYTES { continue; }
        let record: Value = match serde_json::from_str(&line) {
            Ok(record) => record,
            Err(_) => { malformed += 1; continue; }
        };
        let payload = record.get("payload");
        let cwd = payload.and_then(|payload| payload.get("cwd")).and_then(Value::as_str)
            .or_else(|| payload.and_then(|payload| payload.pointer("/state/environments/environments/local/cwd")).and_then(Value::as_str));
        if let Some(cwd) = cwd.filter(|cwd| !cwd.trim().is_empty()) {
            return Ok((Some(cwd.to_string()), malformed));
        }
    }
    Ok((None, malformed))
}

fn parse_state(bytes: &[u8], path: &Path) -> anyhow::Result<Map<String, Value>> {
    serde_json::from_slice::<Value>(bytes)
        .with_context(|| format!("failed to parse {}", path.display()))?
        .as_object().cloned()
        .ok_or_else(|| anyhow::anyhow!("{} must be a JSON object", path.display()))
}

fn required_object<'a>(state: &'a Map<String, Value>, key: &str) -> anyhow::Result<&'a Map<String, Value>> {
    state.get(key).and_then(Value::as_object).ok_or_else(|| anyhow::anyhow!("{key} must be a JSON object"))
}

fn required_array_of_strings(state: &Map<String, Value>, key: &str) -> anyhow::Result<Vec<String>> {
    state.get(key).and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("{key} must be an array"))?
        .iter().map(|value| value.as_str().map(str::to_string).ok_or_else(|| anyhow::anyhow!("{key} must contain only strings"))).collect()
}

fn normalize_path(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() { return None; }
    let value = expand_home(value)?;
    #[cfg(windows)]
    let separator = '\\';
    #[cfg(not(windows))]
    let separator = '/';
    let mut value = if cfg!(windows) { value.replace('/', "\\") } else { value.replace('\\', "/") };
    let prefix = if value.starts_with(separator) { separator.to_string() } else { String::new() };
    let mut parts = Vec::new();
    for part in value.split(separator) {
        if part.is_empty() || part == "." { continue; }
        if part == ".." { if !parts.is_empty() { parts.pop(); } continue; }
        parts.push(part);
    }
    value = format!("{}{}", prefix, parts.join(&separator.to_string()));
    #[cfg(windows)]
    { value = value.to_ascii_lowercase(); }
    Some(value)
}

fn expand_home(value: &str) -> Option<String> {
    if value == "~" || value.starts_with("~/") || value.starts_with("~\\") {
        let home = directories::BaseDirs::new()?.home_dir().to_string_lossy().to_string();
        Some(format!("{home}{}", &value[1..]))
    } else { Some(value.to_string()) }
}

fn create_backup(home: &Path, bytes: &[u8], assignment_count: usize) -> anyhow::Result<PathBuf> {
    let path = home.join("backups_state/local-project-assignment-repair").join(now_ms().to_string());
    fs::create_dir_all(&path)?;
    fs::write(path.join(GLOBAL_STATE_FILE), bytes)?;
    fs::write(path.join("metadata.json"), serde_json::to_vec_pretty(&json!({
        "managedBy": "Codex++ local project assignment repair",
        "createdAtMs": now_ms(),
        "assignmentCount": assignment_count,
    }))?)?;
    Ok(path)
}

fn verify_assignments(path: &Path, planned: &[PlannedAssignment]) -> anyhow::Result<()> {
    let state = parse_state(&fs::read(path)?, path)?;
    let assignments = required_object(&state, ASSIGNMENTS_KEY)?;
    for item in planned {
        let Some(value) = assignments.get(&item.thread_id).and_then(Value::as_object) else { bail!("assignment verification failed"); };
        if value.get("projectKind").and_then(Value::as_str) != Some("local")
            || value.get("projectId").and_then(Value::as_str) != Some(&item.project_id)
            || value.get("cwd").and_then(Value::as_str) != Some(&item.cwd)
            || value.get("pendingCoreUpdate").and_then(Value::as_bool) != Some(false) { bail!("assignment verification failed"); }
    }
    Ok(())
}

fn restore_backup(state_path: &Path, bytes: &[u8]) -> anyhow::Result<()> { crate::settings::atomic_write(state_path, bytes) }
fn now_ms() -> u128 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() }

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const PROJECT_A: &str = "11111111-1111-1111-1111-111111111111";
    const PROJECT_B: &str = "22222222-2222-2222-2222-222222222222";
    const THREAD_A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const THREAD_B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    fn state() -> Value {
        let mut projects = Map::new();
        projects.insert(PROJECT_A.to_string(), json!({ "id": PROJECT_A }));
        projects.insert(PROJECT_B.to_string(), json!({ "id": PROJECT_B }));
        json!({
            "local-projects": projects,
            "project-order": [PROJECT_A, PROJECT_B],
            "electron-saved-workspace-roots": ["/workspace/one", "/workspace/two"],
            "thread-project-assignments": {}, "unrelated": {"keep": true}
        })
    }
    fn setup(entries: &[(&str, &str)]) -> tempfile::TempDir {
        let temp = tempdir().unwrap(); let home = temp.path();
        fs::write(home.join(GLOBAL_STATE_FILE), serde_json::to_vec(&state()).unwrap()).unwrap();
        let sessions = home.join("sessions/2026"); fs::create_dir_all(&sessions).unwrap();
        for (id, line) in entries { fs::write(sessions.join(format!("rollout-{id}.jsonl")), line).unwrap(); }
        temp
    }
    fn cwd(value: &str) -> String { json!({"payload":{"cwd":value}}).to_string() }

    #[test] fn exact_match_and_nested_cwd_are_planned() { let temp = setup(&[(THREAD_A, &cwd("/workspace/one/../one")), (THREAD_B, &json!({"payload":{"state":{"environments":{"environments":{"local":{"cwd":"/workspace/two"}}}}}}).to_string())]); let r = analyze_local_project_assignments(temp.path()).unwrap(); assert_eq!(r.safe_assignments_found, 2); }
    #[test] fn existing_assignment_is_preserved_and_apply_is_idempotent() { let temp = setup(&[(THREAD_A, &cwd("/workspace/one"))]); let path=temp.path().join(GLOBAL_STATE_FILE); let mut value:Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap(); value[ASSIGNMENTS_KEY][THREAD_A]=json!({"projectId":"unchanged"}); fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap(); assert_eq!(analyze_local_project_assignments(temp.path()).unwrap().safe_assignments_found,0); assert_eq!(apply_local_project_assignments(temp.path()).unwrap().applied_assignments,0); }
    #[test] fn unmatched_missing_and_malformed_are_skipped() { let temp=setup(&[(THREAD_A, "not json\n"), (THREAD_B, &cwd("/no/match"))]); let r=analyze_local_project_assignments(temp.path()).unwrap(); assert_eq!(r.safe_assignments_found,0); assert_eq!(r.unmatched,1); assert_eq!(r.missing_cwd,1); assert_eq!(r.malformed_rollout_lines,1); }
    #[test] fn duplicate_roots_and_duplicate_rollouts_are_safe() { let temp=setup(&[(THREAD_A,&cwd("/workspace/one")), (THREAD_B,&cwd("/workspace/one"))]); let path=temp.path().join(GLOBAL_STATE_FILE); let mut value:Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap(); value["electron-saved-workspace-roots"]=json!(["/workspace/one","/workspace/one"]); fs::write(&path,serde_json::to_vec(&value).unwrap()).unwrap(); let r=analyze_local_project_assignments(temp.path()).unwrap(); assert_eq!(r.ambiguous,2); fs::write(temp.path().join("sessions/rollout-copy-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa.jsonl"),cwd("/workspace/one")).unwrap(); assert_eq!(analyze_local_project_assignments(temp.path()).unwrap().duplicate_rollout_thread_ids,1); }
    #[test] fn dry_run_preserves_state_backup_and_unrelated_fields() { let temp=setup(&[(THREAD_A,&cwd("/workspace/one"))]); let path=temp.path().join(GLOBAL_STATE_FILE); let before=fs::read(&path).unwrap(); analyze_local_project_assignments(temp.path()).unwrap(); assert_eq!(fs::read(&path).unwrap(),before); let r=apply_local_project_assignments(temp.path()).unwrap(); assert_eq!(r.applied_assignments,1); assert!(r.backup_path.unwrap().is_dir()); let value:Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap(); assert_eq!(value["unrelated"]["keep"],true); assert_eq!(apply_local_project_assignments(temp.path()).unwrap().applied_assignments,0); }
    #[test] fn verification_failure_restores_original_state() { let temp=setup(&[(THREAD_A,&cwd("/workspace/one"))]); let path=temp.path().join(GLOBAL_STATE_FILE); let original=fs::read(&path).unwrap(); let error=apply_local_project_assignments_with_writer(temp.path(), |path,_| crate::settings::atomic_write(path,b"{}")).unwrap_err(); assert!(error.to_string().contains("verification")); assert_eq!(fs::read(&path).unwrap(),original); }
}
