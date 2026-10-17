use anyhow::{Context, bail};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const HEADER_LINE_LIMIT: usize = 1024 * 1024;

/// Rollout 文件及其分页历史依赖。
///
/// `files` 按物理线程 ID 保存全部文件；continuation 文件的首行 `meta.id` 可能仍是
/// 旧线程，因此物理 ID 优先取文件名末尾 UUID。`dependencies` 的方向是
/// `当前线程 -> history_base.thread_id`。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct HistoryGraph {
    pub files: BTreeMap<String, Vec<PathBuf>>,
    pub dependencies: BTreeMap<String, BTreeSet<String>>,
}

impl HistoryGraph {
    pub fn scan(home: &Path, extra: &[(String, PathBuf)]) -> anyhow::Result<Self> {
        let canonical_home = fs::canonicalize(home)
            .with_context(|| format!("无法解析 Codex home：{}", home.display()))?;
        let mut candidates = BTreeMap::<PathBuf, Option<String>>::new();
        let mut visited_dirs = BTreeSet::new();

        for name in ["sessions", "archived_sessions"] {
            let root = home.join(name);
            if !root.exists() {
                continue;
            }
            collect_standard_files(&root, &canonical_home, &mut visited_dirs, &mut candidates)?;
        }

        // extra 是数据库明确给出的单文件映射，可以位于 home 外；不得从其父目录扩扫。
        for (thread_id, path) in extra {
            let canonical = canonical_regular_file(path)
                .with_context(|| format!("无法读取数据库声明的 rollout：{}", path.display()))?;
            let mapped = normalize_id(thread_id);
            match candidates.entry(canonical) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(mapped);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if entry.get().is_none() && mapped.is_some() {
                        entry.insert(mapped);
                    }
                }
            }
        }

        let mut graph = Self::default();
        for (path, extra_id) in candidates {
            let metadata = read_rollout_metadata(&path)?;
            let physical_id = physical_id_from_filename(&path)
                .or(extra_id)
                .or_else(|| metadata.meta_id.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!("无法确定 rollout 物理线程 ID：{}", path.display())
                })?;

            graph
                .files
                .entry(physical_id.clone())
                .or_default()
                .push(path);
            let edges = graph.dependencies.entry(physical_id).or_default();
            if let Some(base_id) = metadata.history_base_id {
                edges.insert(base_id);
            }
        }

        for paths in graph.files.values_mut() {
            paths.sort();
            paths.dedup();
        }
        Ok(graph)
    }

    pub fn reachable_from(&self, roots: impl IntoIterator<Item = String>) -> BTreeSet<String> {
        let mut reachable = BTreeSet::new();
        let mut pending = roots.into_iter().collect::<Vec<_>>();
        while let Some(id) = pending.pop() {
            if !reachable.insert(id.clone()) {
                continue;
            }
            if let Some(dependencies) = self.dependencies.get(&id) {
                pending.extend(dependencies.iter().cloned());
            }
        }
        reachable
    }

    pub fn closure_files(&self, id: &str) -> anyhow::Result<Vec<PathBuf>> {
        let root = normalize_id(id).ok_or_else(|| anyhow::anyhow!("历史线程 ID 为空"))?;
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        self.validate_closure(&root, &mut visiting, &mut visited)?;

        let mut files = Vec::new();
        for thread_id in visited {
            let Some(paths) = self.files.get(&thread_id) else {
                bail!("历史依赖缺失：{thread_id}");
            };
            files.extend(paths.iter().cloned());
        }
        files.sort();
        files.dedup();
        Ok(files)
    }

    fn validate_closure(
        &self,
        id: &str,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
    ) -> anyhow::Result<()> {
        if visited.contains(id) {
            return Ok(());
        }
        if !self.files.contains_key(id) {
            bail!("历史依赖缺失：{id}");
        }
        if !visiting.insert(id.to_string()) {
            bail!("历史依赖存在循环：{id}");
        }
        if let Some(dependencies) = self.dependencies.get(id) {
            for dependency in dependencies {
                self.validate_closure(dependency, visiting, visited)?;
            }
        }
        visiting.remove(id);
        visited.insert(id.to_string());
        Ok(())
    }
}

#[derive(Debug)]
struct RolloutMetadata {
    meta_id: Option<String>,
    history_base_id: Option<String>,
}

fn collect_standard_files(
    path: &Path,
    canonical_home: &Path,
    visited_dirs: &mut BTreeSet<PathBuf>,
    output: &mut BTreeMap<PathBuf, Option<String>>,
) -> anyhow::Result<()> {
    let canonical =
        fs::canonicalize(path).with_context(|| format!("无法解析会话路径：{}", path.display()))?;
    if !canonical.starts_with(canonical_home) {
        bail!("会话路径越出 Codex home：{}", path.display());
    }
    let metadata = fs::metadata(&canonical)
        .with_context(|| format!("无法读取会话路径元数据：{}", path.display()))?;
    if metadata.is_dir() {
        if !visited_dirs.insert(canonical.clone()) {
            return Ok(());
        }
        for entry in fs::read_dir(&canonical)
            .with_context(|| format!("无法遍历会话目录：{}", canonical.display()))?
        {
            let entry = entry?;
            collect_standard_files(&entry.path(), canonical_home, visited_dirs, output)?;
        }
    } else if metadata.is_file()
        && canonical.extension().and_then(|value| value.to_str()) == Some("jsonl")
    {
        output.entry(canonical).or_insert(None);
    }
    Ok(())
}

fn canonical_regular_file(path: &Path) -> anyhow::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    if !fs::metadata(&canonical)?.is_file() {
        bail!("不是普通文件：{}", path.display());
    }
    Ok(canonical)
}

fn read_rollout_metadata(path: &Path) -> anyhow::Result<RolloutMetadata> {
    let file =
        fs::File::open(path).with_context(|| format!("无法打开 rollout：{}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut total_read = 0;
    loop {
        line.clear();
        let read = reader
            .by_ref()
            .take((HEADER_LINE_LIMIT + 1) as u64)
            .read_until(b'\n', &mut line)
            .with_context(|| format!("无法读取 rollout 首行：{}", path.display()))?;
        if read == 0 {
            bail!("rollout 缺少 session_meta：{}", path.display());
        }
        total_read += read;
        if total_read > HEADER_LINE_LIMIT {
            bail!("rollout 首个非空行超过长度上限：{}", path.display());
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        break;
    }

    let value: Value = serde_json::from_slice(&line)
        .with_context(|| format!("rollout 首个非空行不是合法 JSON：{}", path.display()))?;
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        bail!("rollout 首个非空行不是 session_meta：{}", path.display());
    }
    let payload = value
        .get("payload")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("session_meta 缺少 payload：{}", path.display()))?;
    let meta_id = payload
        .get("id")
        .and_then(Value::as_str)
        .and_then(normalize_id);
    let history_base_id = match payload.get("history_base") {
        None | Some(Value::Null) => None,
        Some(base) => Some(
            base.get("thread_id")
                .and_then(Value::as_str)
                .and_then(normalize_id)
                .ok_or_else(|| {
                    anyhow::anyhow!("history_base 缺少合法 thread_id：{}", path.display())
                })?,
        ),
    };
    Ok(RolloutMetadata {
        meta_id,
        history_base_id,
    })
}

fn physical_id_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    if stem.len() < 36 {
        return None;
    }
    let suffix = stem.get(stem.len() - 36..)?;
    Uuid::parse_str(suffix).ok().map(|id| id.to_string())
}

fn normalize_id(id: &str) -> Option<String> {
    let id = id.trim();
    if id.is_empty() {
        None
    } else {
        Some(
            Uuid::parse_str(id)
                .map(|value| value.to_string())
                .unwrap_or_else(|_| id.to_string()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    const BASE: &str = "01a140ba-41ac-7830-85c2-6f49e71c5234";
    const CHILD: &str = "01a14443-39fe-7de2-82e8-966cf95f2d87";

    fn write_meta(path: &Path, meta_id: &str, history_base: Option<&str>, forked: Option<&str>) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut payload = json!({"id": meta_id});
        if let Some(base) = history_base {
            payload["history_base"] = json!({
                "thread_id": base,
                "end_ordinal_exclusive": 42,
                "end_byte_offset": 4096
            });
        }
        if let Some(parent) = forked {
            payload["forked_from_id"] = json!(parent);
        }
        fs::write(
            path,
            format!("\n{}\n", json!({"type":"session_meta","payload":payload})),
        )
        .unwrap();
    }

    #[test]
    fn continuation_uses_filename_id_and_history_base_edge() {
        let temp = tempdir().unwrap();
        let home = temp.path();
        let base = home
            .join("sessions")
            .join(format!("rollout-2026-10-16T10-00-00-{BASE}.jsonl"));
        let child = home
            .join("sessions")
            .join(format!("rollout-2026-10-16T10-01-00-{BASE}_{CHILD}.jsonl"));
        write_meta(&base, BASE, None, None);
        // continuation 首行仍写旧 ID，物理 ID 必须取文件名末尾 CHILD。
        write_meta(&child, BASE, Some(BASE), None);

        let graph = HistoryGraph::scan(home, &[]).unwrap();

        assert_eq!(graph.files[CHILD], vec![fs::canonicalize(&child).unwrap()]);
        assert_eq!(
            graph.dependencies[CHILD],
            BTreeSet::from([BASE.to_string()])
        );
        assert_eq!(
            graph.closure_files(CHILD).unwrap(),
            vec![
                fs::canonicalize(&base).unwrap(),
                fs::canonicalize(&child).unwrap()
            ]
        );
    }

    #[test]
    fn forked_from_id_without_history_base_is_not_a_dependency() {
        let temp = tempdir().unwrap();
        let path = temp
            .path()
            .join("sessions")
            .join(format!("rollout-{CHILD}.jsonl"));
        write_meta(&path, CHILD, None, Some(BASE));

        let graph = HistoryGraph::scan(temp.path(), &[]).unwrap();

        assert!(graph.dependencies[CHILD].is_empty());
        assert_eq!(graph.closure_files(CHILD).unwrap().len(), 1);
    }

    #[test]
    fn explicit_extra_outside_home_uses_mapping_then_meta_fallback() {
        let home = tempdir().unwrap();
        fs::create_dir_all(home.path().join("sessions")).unwrap();
        let outside = tempdir().unwrap();
        let mapped = outside.path().join("custom-name.jsonl");
        let fallback = outside.path().join("other-name.jsonl");
        write_meta(&mapped, BASE, None, None);
        write_meta(&fallback, CHILD, None, None);

        let graph = HistoryGraph::scan(
            home.path(),
            &[
                (CHILD.to_string(), mapped.clone()),
                (String::new(), fallback.clone()),
            ],
        )
        .unwrap();

        assert!(graph.files[CHILD].contains(&fs::canonicalize(mapped).unwrap()));
        assert!(graph.files[CHILD].contains(&fs::canonicalize(fallback).unwrap()));
    }

    #[test]
    fn closure_rejects_missing_dependency() {
        let temp = tempdir().unwrap();
        let child = temp
            .path()
            .join("sessions")
            .join(format!("rollout-{CHILD}.jsonl"));
        write_meta(&child, CHILD, Some(BASE), None);

        let graph = HistoryGraph::scan(temp.path(), &[]).unwrap();

        assert!(
            graph
                .closure_files(CHILD)
                .unwrap_err()
                .to_string()
                .contains(BASE)
        );
    }

    #[test]
    fn closure_rejects_dependency_cycle() {
        let temp = tempdir().unwrap();
        let base = temp
            .path()
            .join("sessions")
            .join(format!("rollout-{BASE}.jsonl"));
        let child = temp
            .path()
            .join("sessions")
            .join(format!("rollout-{CHILD}.jsonl"));
        write_meta(&base, BASE, Some(CHILD), None);
        write_meta(&child, CHILD, Some(BASE), None);

        let graph = HistoryGraph::scan(temp.path(), &[]).unwrap();

        assert!(
            graph
                .closure_files(CHILD)
                .unwrap_err()
                .to_string()
                .contains("循环")
        );
    }

    #[test]
    fn scan_rejects_damaged_or_oversized_metadata() {
        let damaged_home = tempdir().unwrap();
        let damaged = damaged_home
            .path()
            .join("sessions")
            .join(format!("rollout-{CHILD}.jsonl"));
        fs::create_dir_all(damaged.parent().unwrap()).unwrap();
        fs::write(&damaged, "{broken\n").unwrap();
        assert!(HistoryGraph::scan(damaged_home.path(), &[]).is_err());

        let oversized_home = tempdir().unwrap();
        let oversized = oversized_home
            .path()
            .join("sessions")
            .join(format!("rollout-{CHILD}.jsonl"));
        fs::create_dir_all(oversized.parent().unwrap()).unwrap();
        fs::write(&oversized, vec![b'x'; HEADER_LINE_LIMIT + 1]).unwrap();
        assert!(HistoryGraph::scan(oversized_home.path(), &[]).is_err());
    }
}
