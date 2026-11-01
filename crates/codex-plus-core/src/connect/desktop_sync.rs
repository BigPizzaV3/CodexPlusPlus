use std::path::Path;
use std::sync::Mutex;

use anyhow::Context;
use serde::{Deserialize, Serialize};

const FILE: &str = ".codex-plus-weixin-desktop-sync.json";
const LIMIT: usize = 128;
const MAX_BYTES: usize = 64 * 1024;
static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    home: String,
    database: String,
    entries: Vec<Entry>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    thread_id: String,
    revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project: Option<Project>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    id: String,
    name: String,
    root_paths: Vec<String>,
}

impl Project {
    fn from_response(value: &serde_json::Value) -> anyhow::Result<Self> {
        let id = value["id"].as_str().context("微信项目缺少 ID")?;
        uuid::Uuid::parse_str(id).context("微信项目 ID 无效")?;
        let name = value["name"].as_str().context("微信项目缺少名称")?;
        let roots = value["roots"].as_array().context("微信项目缺少目录")?;
        let root_paths = roots
            .iter()
            .map(|root| {
                root["path"]
                    .as_str()
                    .filter(|path| !path.is_empty())
                    .map(str::to_owned)
                    .context("微信项目目录无效")
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        anyhow::ensure!(!root_paths.is_empty(), "微信项目目录为空");
        Ok(Self {
            id: id.to_owned(),
            name: name.to_owned(),
            root_paths,
        })
    }
}

fn load(home: &Path, database: &Path) -> anyhow::Result<Snapshot> {
    let file = home.join(FILE);
    if std::fs::metadata(&file)
        .map(|value| value.len() > MAX_BYTES as u64)
        .unwrap_or(false)
    {
        anyhow::bail!("微信桌面同步记录超过上限");
    }
    let text = match std::fs::read(&file) {
        Ok(text) if text.len() <= MAX_BYTES => text,
        Ok(_) => anyhow::bail!("微信桌面同步记录超过上限"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Snapshot::default());
        }
        Err(error) => return Err(error.into()),
    };
    let snapshot: Snapshot = serde_json::from_slice(&text)?;
    // 同一 home 可配不同 SQLite；只通知拥有对应数据库的桌面实例。
    if snapshot.home
        != home
            .canonicalize()
            .unwrap_or_else(|_| home.to_path_buf())
            .to_string_lossy()
        || snapshot.database
            != database
                .canonicalize()
                .unwrap_or_else(|_| database.to_path_buf())
                .to_string_lossy()
    {
        return Ok(Snapshot::default());
    }
    Ok(snapshot)
}

pub fn pending() -> Vec<Entry> {
    let home = crate::codex_home::default_codex_home_dir();
    let database = crate::codex_sqlite::codex_session_db_path();
    pending_at(&home, &database).unwrap_or_default()
}

fn pending_at(home: &Path, database: &Path) -> anyhow::Result<Vec<Entry>> {
    let snapshot = load(home, database)?;
    if snapshot.entries.is_empty() {
        return Ok(Vec::new());
    }
    let connection = rusqlite::Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    connection.busy_timeout(std::time::Duration::from_millis(100))?;
    let mut exists =
        connection.prepare("SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND archived=0)")?;
    let mut entries = Vec::new();
    for entry in snapshot.entries.into_iter().take(LIMIT) {
        // 不能因历史同步记录复活已删除或归档的会话。
        if exists.query_row([&entry.thread_id], |row| row.get::<_, bool>(0))? {
            entries.push(entry);
        }
    }
    Ok(entries)
}

pub fn record(thread_id: &str, project: Option<&serde_json::Value>) -> anyhow::Result<()> {
    let home = crate::codex_home::default_codex_home_dir();
    let database = crate::codex_sqlite::codex_session_db_path();
    record_at(
        &home,
        &database,
        thread_id,
        project.map(Project::from_response).transpose()?,
    )
}

fn record_at(
    home: &Path,
    database: &Path,
    thread_id: &str,
    project: Option<Project>,
) -> anyhow::Result<()> {
    uuid::Uuid::parse_str(thread_id).context("微信同步线程 ID 无效")?;
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let mut snapshot = load(home, database)?;
    snapshot.home = home
        .canonicalize()
        .unwrap_or_else(|_| home.to_path_buf())
        .to_string_lossy()
        .into_owned();
    snapshot.database = database
        .canonicalize()
        .unwrap_or_else(|_| database.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let project = project.or_else(|| {
        snapshot
            .entries
            .iter()
            .find(|entry| entry.thread_id == thread_id)
            .and_then(|entry| entry.project.clone())
    });
    snapshot
        .entries
        .retain(|entry| entry.thread_id != thread_id);
    snapshot.entries.insert(
        0,
        Entry {
            thread_id: thread_id.to_string(),
            revision: uuid::Uuid::new_v4().to_string(),
            project,
        },
    );
    snapshot.entries.truncate(LIMIT);
    let mut bytes = serde_json::to_vec(&snapshot)?;
    while bytes.len() > MAX_BYTES && snapshot.entries.len() > 1 {
        snapshot.entries.pop();
        bytes = serde_json::to_vec(&snapshot)?;
    }
    anyhow::ensure!(bytes.len() <= MAX_BYTES, "微信桌面同步记录超过上限");
    crate::settings::atomic_write(&home.join(FILE), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_are_bounded_deduplicated_and_database_scoped() {
        let home = tempfile::tempdir().unwrap();
        let database = home.path().join("state_5.sqlite");
        let id = uuid::Uuid::new_v4().to_string();
        record_at(home.path(), &database, &id, None).unwrap();
        let first = load(home.path(), &database).unwrap().entries[0]
            .revision
            .clone();
        record_at(home.path(), &database, &id, None).unwrap();
        let next = load(home.path(), &database).unwrap();
        assert_eq!(next.entries.len(), 1);
        assert_ne!(next.entries[0].revision, first);
        assert!(
            load(home.path(), &home.path().join("other.sqlite"))
                .unwrap()
                .entries
                .is_empty()
        );
        let other_home = tempfile::tempdir().unwrap();
        std::fs::copy(home.path().join(FILE), other_home.path().join(FILE)).unwrap();
        assert!(
            load(other_home.path(), &database)
                .unwrap()
                .entries
                .is_empty()
        );
        for _ in 0..LIMIT {
            record_at(
                home.path(),
                &database,
                &uuid::Uuid::new_v4().to_string(),
                None,
            )
            .unwrap();
        }
        let snapshot = load(home.path(), &database).unwrap();
        assert_eq!(snapshot.entries.len(), LIMIT);
        assert!(!snapshot.entries.iter().any(|entry| entry.thread_id == id));
        assert!(record_at(home.path(), &database, "not-a-thread", None).is_err());
        let json = serde_json::to_value(&snapshot.entries).unwrap();
        assert_eq!(json[0].as_object().unwrap().len(), 2);
    }

    #[test]
    fn archived_or_deleted_threads_are_not_republished() {
        let home = tempfile::tempdir().unwrap();
        let database = home.path().join("state_5.sqlite");
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, archived INTEGER);")
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        record_at(home.path(), &database, &id, None).unwrap();
        assert!(pending_at(home.path(), &database).unwrap().is_empty());
        connection
            .execute("INSERT INTO threads VALUES(?1,0)", [&id])
            .unwrap();
        assert_eq!(pending_at(home.path(), &database).unwrap().len(), 1);
        connection
            .execute("UPDATE threads SET archived=1", [])
            .unwrap();
        assert!(pending_at(home.path(), &database).unwrap().is_empty());
        connection.execute("DELETE FROM threads", []).unwrap();
        assert!(pending_at(home.path(), &database).unwrap().is_empty());
    }

    #[test]
    fn project_identity_is_preserved_and_old_records_remain_readable() {
        let home = tempfile::tempdir().unwrap();
        let database = home.path().join("state_5.sqlite");
        let thread = uuid::Uuid::new_v4().to_string();
        let project_id = uuid::Uuid::new_v4().to_string();
        let response = serde_json::json!({
            "id": project_id, "name": "Renamed Weixin",
            "roots": [{"path": "/workspace/weixin"}],
            "metadata": {"codexPlusWeixin": "codex-plus-weixin-reserved-v1"},
            "unrelated": "must not be published"
        });
        let project = Project::from_response(&response).unwrap();
        record_at(home.path(), &database, &thread, Some(project)).unwrap();
        let snapshot = load(home.path(), &database).unwrap();
        let project = snapshot.entries[0].project.as_ref().unwrap();
        assert_eq!(project.id, project_id);
        assert_eq!(project.name, "Renamed Weixin");
        let serialized = serde_json::to_value(&snapshot).unwrap();
        assert!(
            serialized["entries"][0]["project"]
                .get("unrelated")
                .is_none()
        );
        let mut selected = response;
        selected["metadata"] = serde_json::json!({});
        assert_eq!(Project::from_response(&selected).unwrap().id, project_id);
        record_at(home.path(), &database, &thread, None).unwrap();
        assert_eq!(
            load(home.path(), &database).unwrap().entries[0]
                .project
                .as_ref()
                .unwrap()
                .id,
            project_id
        );
        let old: Entry = serde_json::from_value(serde_json::json!({
            "threadId": thread, "revision": uuid::Uuid::new_v4().to_string()
        }))
        .unwrap();
        assert!(old.project.is_none());
    }

    #[test]
    fn project_metadata_respects_snapshot_byte_limit() {
        let home = tempfile::tempdir().unwrap();
        let database = home.path().join("state_5.sqlite");
        let project = Project {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Weixin".into(),
            root_paths: vec!["x".repeat(4096)],
        };
        for _ in 0..32 {
            record_at(
                home.path(),
                &database,
                &uuid::Uuid::new_v4().to_string(),
                Some(project.clone()),
            )
            .unwrap();
        }
        assert!(std::fs::metadata(home.path().join(FILE)).unwrap().len() <= MAX_BYTES as u64);
        let snapshot = load(home.path(), &database).unwrap();
        assert!(!snapshot.entries.is_empty() && snapshot.entries.len() < 32);
        let oversized = Project {
            root_paths: vec!["x".repeat(MAX_BYTES)],
            ..project
        };
        assert!(
            record_at(
                home.path(),
                &database,
                &uuid::Uuid::new_v4().to_string(),
                Some(oversized)
            )
            .is_err()
        );
        assert_eq!(
            load(home.path(), &database).unwrap().entries.len(),
            snapshot.entries.len()
        );
    }
}
