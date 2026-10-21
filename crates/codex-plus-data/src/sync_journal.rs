//! 同步补偿日志：只恢复本操作改过且仍等于本操作后值的行/列，不恢复整份数据库。
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use rusqlite::{
    Connection, OptionalExtension, Transaction, params_from_iter,
    types::{Value, ValueRef},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;

const TABLES: &[&str] = &[
    "threads",
    "local_thread_catalog",
    "local_thread_catalog_metadata",
    "local_thread_catalog_sync_state",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Cell {
    Null,
    Integer(i64),
    Real(u64),
    Text(String),
    Blob(Vec<u8>),
}
impl Cell {
    fn read(value: ValueRef<'_>) -> anyhow::Result<Self> {
        Ok(match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(v) => Self::Integer(v),
            ValueRef::Real(v) => Self::Real(v.to_bits()),
            ValueRef::Text(v) => Self::Text(std::str::from_utf8(v)?.to_string()),
            ValueRef::Blob(v) => Self::Blob(v.to_vec()),
        })
    }
    fn sql(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Integer(v) => Value::Integer(*v),
            Self::Real(v) => Value::Real(f64::from_bits(*v)),
            Self::Text(v) => Value::Text(v.clone()),
            Self::Blob(v) => Value::Blob(v.clone()),
        }
    }
}
type Row = BTreeMap<String, Cell>;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Image {
    keys: Row,
    values: Row,
}
#[derive(Debug, Default)]
pub(crate) struct Snapshot(BTreeMap<String, BTreeMap<String, Image>>);

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

impl Snapshot {
    pub(crate) fn capture(db: &Connection) -> anyhow::Result<Self> {
        let mut tables = BTreeMap::new();
        for &table in TABLES {
            let mut info = db.prepare(&format!("PRAGMA table_info({})", quote(table)))?;
            let columns = info
                .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if columns.is_empty() {
                continue;
            }
            let mut keys = columns
                .iter()
                .filter(|(_, pk)| *pk > 0)
                .cloned()
                .collect::<Vec<_>>();
            keys.sort_by_key(|(_, pk)| *pk);
            let mut names = columns
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>();
            let key_names = if keys.is_empty() {
                // 无主键的本机元数据表使用 SQLite rowid；不猜测可重复的业务列。
                let alias = ["rowid", "_rowid_", "oid"]
                    .into_iter()
                    .find(|alias| !names.iter().any(|name| name.eq_ignore_ascii_case(alias)))
                    .ok_or_else(|| anyhow::anyhow!("table has no stable row identity: {table}"))?;
                names.push(alias.to_string());
                vec![alias.to_string()]
            } else {
                keys.into_iter().map(|(name, _)| name).collect()
            };
            let mut stmt = db.prepare(&format!(
                "SELECT {} FROM {}",
                names.iter().map(|n| quote(n)).collect::<Vec<_>>().join(","),
                quote(table)
            ))?;
            let mut rows = stmt.query([])?;
            let mut images = BTreeMap::new();
            while let Some(row) = rows.next()? {
                let mut values = Row::new();
                for (index, name) in names.iter().enumerate() {
                    values.insert(name.clone(), Cell::read(row.get_ref(index)?)?);
                }
                let keys: Row = key_names
                    .iter()
                    .map(|name| (name.clone(), values[name].clone()))
                    .collect();
                let identity = serde_json::to_string(&keys)?;
                if images.insert(identity, Image { keys, values }).is_some() {
                    anyhow::bail!("ambiguous row identity in {table}");
                }
            }
            tables.insert(table.to_string(), images);
        }
        Ok(Self(tables))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Change {
    table: String,
    keys: Row,
    before: Option<Row>,
    after: Option<Row>,
    #[serde(default)]
    restored: bool,
}
#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    database: PathBuf,
    state: String,
    changes: Vec<Change>,
}
#[derive(Debug, Serialize, Deserialize)]
struct FileChange {
    path: PathBuf,
    before_file: PathBuf,
    before_sha256: String,
    after_sha256: String,
    existed_before: bool,
    restored: bool,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct JournalRollout {
    pub path: PathBuf,
    pub before_sha256: String,
    pub after_sha256: String,
    pub original_meta_lines: Vec<String>,
    pub original_mtime: Option<std::time::SystemTime>,
    pub applied: Option<bool>,
    pub restored: bool,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SyncJournal {
    version: u32,
    state: String,
    entries: Vec<Entry>,
    #[serde(default)]
    files: Vec<FileChange>,
    #[serde(default)]
    rollouts: Vec<JournalRollout>,
    #[serde(skip)]
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSyncDatabaseOutcome {
    pub database: PathBuf,
    pub attempted_rows: usize,
    pub restored_rows: usize,
    pub remaining_rows: usize,
}

impl SyncJournal {
    pub(crate) fn load_for_recovery(
        path: &Path,
        home: &Path,
        allowed_databases: &[PathBuf],
    ) -> anyhow::Result<Self> {
        let mut journal: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        if journal.version != 1 || journal.state == "complete" {
            anyhow::bail!("journal is unsupported or belongs to a completed sync");
        }
        let allowed = allowed_databases
            .iter()
            .filter_map(|path| std::fs::canonicalize(path).ok())
            .collect::<Vec<_>>();
        for entry in &journal.entries {
            if !allowed.contains(&std::fs::canonicalize(&entry.database)?) {
                anyhow::bail!("journal database is outside current allowed candidates");
            }
            for change in &entry.changes {
                if !TABLES.contains(&change.table.as_str())
                    || change.keys.is_empty()
                    || (change.before.is_none() && change.after.is_none())
                {
                    anyhow::bail!("journal contains an unsupported row mutation");
                }
            }
        }
        let home = std::fs::canonicalize(home)?;
        for file in &journal.files {
            let name = file.path.file_name().and_then(|name| name.to_str());
            if !matches!(
                name,
                Some(".codex-global-state.json" | ".codex-global-state.json.bak")
            ) || file
                .path
                .parent()
                .map(std::fs::canonicalize)
                .transpose()?
                .as_deref()
                != Some(home.as_path())
            {
                anyhow::bail!("journal file is outside managed app state");
            }
            if !file
                .before_file
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
                || file.before_file.components().count() != 1
            {
                anyhow::bail!("journal recovery file escapes backup");
            }
            if std::fs::symlink_metadata(&file.path).is_ok_and(|meta| meta.file_type().is_symlink())
            {
                anyhow::bail!("journal source file is a symlink");
            }
            if file.existed_before {
                let source = path.parent().unwrap().join(&file.before_file);
                if std::fs::symlink_metadata(&source)?.file_type().is_symlink()
                    || std::fs::canonicalize(&source)?.parent()
                        != Some(std::fs::canonicalize(path.parent().unwrap())?.as_path())
                {
                    anyhow::bail!("journal recovery data escapes backup");
                }
            }
        }
        let roots = ["sessions", "archived_sessions"]
            .iter()
            .filter_map(|name| std::fs::canonicalize(home.join(name)).ok())
            .collect::<Vec<_>>();
        for rollout in &journal.rollouts {
            if rollout.restored || rollout.applied == Some(false) {
                continue;
            }
            let canonical = std::fs::canonicalize(&rollout.path)?;
            if !roots.iter().any(|root| canonical.starts_with(root)) {
                anyhow::bail!("journal rollout is outside managed session directories");
            }
        }
        journal.path = path.to_path_buf();
        Ok(journal)
    }
    pub(crate) fn new(backup: &Path) -> anyhow::Result<Self> {
        let journal = Self {
            version: 1,
            state: "prepared".into(),
            entries: Vec::new(),
            files: Vec::new(),
            rollouts: Vec::new(),
            path: backup.join("recovery-journal.json"),
        };
        journal.persist()?;
        Ok(journal)
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn persist(&self) -> anyhow::Result<()> {
        crate::backup::private_atomic_write(&self.path, &serde_json::to_vec(self)?)?;
        crate::backup::sync_file(&self.path)?;
        Ok(())
    }
    pub(crate) fn commit(
        &mut self,
        tx: Transaction<'_>,
        before: Snapshot,
        database: &Path,
    ) -> anyhow::Result<()> {
        let after = Snapshot::capture(&tx)?;
        let mut changes = Vec::new();
        for table in before
            .0
            .keys()
            .chain(after.0.keys())
            .collect::<BTreeSet<_>>()
        {
            let old = before.0.get(table);
            let new = after.0.get(table);
            let ids = old
                .into_iter()
                .flat_map(|v| v.keys())
                .chain(new.into_iter().flat_map(|v| v.keys()))
                .collect::<BTreeSet<_>>();
            for id in ids {
                let a = old.and_then(|v| v.get(id));
                let b = new.and_then(|v| v.get(id));
                if a != b {
                    changes.push(Change {
                        table: table.clone(),
                        keys: a.or(b).unwrap().keys.clone(),
                        before: a.map(|v| v.values.clone()),
                        after: b.map(|v| v.values.clone()),
                        restored: false,
                    });
                }
            }
        }
        if changes.is_empty() {
            tx.commit()?;
            return Ok(());
        }
        let index = self.entries.len();
        self.entries.push(Entry {
            database: database.to_path_buf(),
            state: "commit_intent".into(),
            changes,
        });
        // 意图和前后镜像先落盘；即便 commit 后写状态失败，也可按当前值判定并补偿。
        self.persist()?;
        tx.commit()?;
        self.entries[index].state = "committed".into();
        self.persist()
    }
    pub(crate) fn complete(&mut self) -> anyhow::Result<()> {
        self.state = "complete".into();
        self.persist()
    }
    pub(crate) fn prepare_rollouts(&mut self, rollouts: Vec<JournalRollout>) -> anyhow::Result<()> {
        self.rollouts = rollouts;
        self.persist()
    }
    pub(crate) fn mark_rollouts_applied(&mut self, applied: &[PathBuf]) -> anyhow::Result<()> {
        for rollout in &mut self.rollouts {
            rollout.applied = Some(applied.contains(&rollout.path));
        }
        self.persist()
    }
    pub(crate) fn remaining_rollouts(&self) -> usize {
        self.rollouts
            .iter()
            .filter(|r| !r.restored && r.applied != Some(false))
            .count()
    }
    pub(crate) fn prepare_file(
        &mut self,
        path: &Path,
        before: Option<&[u8]>,
        after: &[u8],
    ) -> anyhow::Result<()> {
        let relative = PathBuf::from(format!("recovery-file-{}.before", self.files.len()));
        if let Some(before) = before {
            let mut file =
                crate::backup::new_private_file(&self.path.parent().unwrap().join(&relative))?;
            file.write_all(before)?;
            file.sync_all()?;
        }
        self.files.push(FileChange {
            path: path.to_path_buf(),
            before_file: relative,
            before_sha256: format!("{:x}", Sha256::digest(before.unwrap_or_default())),
            after_sha256: format!("{:x}", Sha256::digest(after)),
            existed_before: before.is_some(),
            restored: false,
        });
        self.persist()
    }
    pub(crate) fn outcomes(&self) -> Vec<ProviderSyncDatabaseOutcome> {
        let mut outcomes = BTreeMap::<PathBuf, ProviderSyncDatabaseOutcome>::new();
        for entry in &self.entries {
            let out = outcomes.entry(entry.database.clone()).or_insert_with(|| {
                ProviderSyncDatabaseOutcome {
                    database: entry.database.clone(),
                    attempted_rows: 0,
                    restored_rows: 0,
                    remaining_rows: 0,
                }
            });
            out.attempted_rows += entry.changes.len();
            out.restored_rows += entry.changes.iter().filter(|c| c.restored).count();
            out.remaining_rows += entry.changes.iter().filter(|c| !c.restored).count();
        }
        outcomes.into_values().collect()
    }
    pub(crate) fn rollback(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        for index in (0..self.entries.len()).rev() {
            let entry = &mut self.entries[index];
            let applied = (|| -> anyhow::Result<()> {
                let mut db = Connection::open_with_flags(
                    &entry.database,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
                )?;
                db.busy_timeout(std::time::Duration::from_millis(250))?;
                let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let mut restored = Vec::new();
                for (row_index, change) in entry.changes.iter().enumerate().rev() {
                    if change.restored {
                        continue;
                    }
                    match restore_change(&tx, change) {
                        Ok(()) => restored.push(row_index),
                        Err(error) => errors.push(format!(
                            "{} / {}: {error:#}",
                            entry.database.display(),
                            change.table
                        )),
                    }
                }
                tx.commit()?;
                for row_index in restored {
                    entry.changes[row_index].restored = true;
                }
                entry.state = if entry.changes.iter().all(|c| c.restored) {
                    "rolled_back"
                } else {
                    "partial_failure"
                }
                .into();
                Ok(())
            })();
            if let Err(error) = applied {
                errors.push(format!("{}: {error:#}", entry.database.display()));
            }
            if let Err(error) = self.persist() {
                errors.push(format!("journal update: {error:#}"));
            }
        }
        for file in self.files.iter_mut().rev() {
            if file.restored {
                continue;
            }
            let restored = (|| -> anyhow::Result<()> {
                let current = match std::fs::read(&file.path) {
                    Ok(bytes) => bytes,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::NotFound && !file.existed_before =>
                    {
                        return Ok(());
                    }
                    Err(error) => return Err(error.into()),
                };
                let hash = format!("{:x}", Sha256::digest(&current));
                if file.existed_before && hash == file.before_sha256 {
                    return Ok(());
                }
                if hash != file.after_sha256 {
                    anyhow::bail!("file changed after sync, refused rollback");
                }
                if !file.existed_before {
                    std::fs::remove_file(&file.path)?;
                    return Ok(());
                }
                let before = std::fs::read(self.path.parent().unwrap().join(&file.before_file))?;
                if format!("{:x}", Sha256::digest(&before)) != file.before_sha256 {
                    anyhow::bail!("recovery file hash mismatch");
                }
                codex_plus_core::settings::atomic_write_with(&file.path, |target| {
                    if std::fs::read(&file.path)? != current {
                        return Err(std::io::Error::other("file changed before rollback"));
                    }
                    target.write_all(&before)
                })?;
                Ok(())
            })();
            match restored {
                Ok(()) => file.restored = true,
                Err(error) => errors.push(format!("{}: {error:#}", file.path.display())),
            }
        }
        for rollout in self.rollouts.iter_mut().rev() {
            if rollout.restored || rollout.applied == Some(false) {
                rollout.restored = true;
                continue;
            }
            match crate::provider_sync::restore_recovery_rollout(rollout) {
                Ok(()) => rollout.restored = true,
                Err(error) => errors.push(format!("{}: {error:#}", rollout.path.display())),
            }
        }
        self.state = if errors.is_empty() {
            "rolled_back"
        } else {
            "partial_failure"
        }
        .into();
        if let Err(error) = self.persist() {
            errors.push(format!("journal update: {error:#}"));
        }
        errors
    }
}

fn where_clause(keys: &Row) -> (String, Vec<Value>) {
    (
        keys.keys()
            .map(|key| format!("{} IS ?", quote(key)))
            .collect::<Vec<_>>()
            .join(" AND "),
        keys.values().map(Cell::sql).collect(),
    )
}
fn restore_change(db: &Connection, change: &Change) -> anyhow::Result<()> {
    let (key_sql, key_values) = where_clause(&change.keys);
    let columns = change
        .after
        .as_ref()
        .or(change.before.as_ref())
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let mut stmt = db.prepare(&format!(
        "SELECT {} FROM {} WHERE {key_sql}",
        columns
            .iter()
            .map(|n| quote(n))
            .collect::<Vec<_>>()
            .join(","),
        quote(&change.table)
    ))?;
    let current = stmt
        .query_row(params_from_iter(key_values.iter()), |row| {
            let mut values = Row::new();
            for (index, name) in columns.iter().enumerate() {
                values.insert(
                    name.clone(),
                    Cell::read(row.get_ref(index)?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?,
                );
            }
            Ok(values)
        })
        .optional()?;
    match (&change.before, &change.after) {
        (Some(before), Some(after)) => {
            let changed = after
                .iter()
                .filter(|(name, value)| before.get(*name) != Some(*value))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect::<Row>();
            let old = changed
                .keys()
                .map(|name| (name.clone(), before[name].clone()))
                .collect::<Row>();
            let current =
                current.ok_or_else(|| anyhow::anyhow!("rollback conflict: row was deleted"))?;
            if old
                .iter()
                .all(|(name, value)| current.get(name) == Some(value))
            {
                return Ok(());
            }
            if !changed
                .iter()
                .all(|(name, value)| current.get(name) == Some(value))
            {
                anyhow::bail!("rollback conflict: row was modified after sync");
            }
            let (after_sql, after_values) = where_clause(&changed);
            let mut values = old.values().map(Cell::sql).collect::<Vec<_>>();
            values.extend(key_values);
            values.extend(after_values);
            let count = db.execute(
                &format!(
                    "UPDATE {} SET {} WHERE {key_sql} AND {after_sql}",
                    quote(&change.table),
                    old.keys()
                        .map(|n| format!("{} = ?", quote(n)))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                params_from_iter(values),
            )?;
            if count != 1 {
                anyhow::bail!("rollback conflict: conditional update did not match");
            }
        }
        (None, Some(after)) => {
            if current.is_none() {
                return Ok(());
            }
            if current.as_ref() != Some(after) {
                anyhow::bail!("rollback conflict: inserted row changed");
            }
            let (sql, values) = where_clause(after);
            db.execute(
                &format!("DELETE FROM {} WHERE {sql}", quote(&change.table)),
                params_from_iter(values),
            )?;
        }
        (Some(before), None) => {
            if current.as_ref() == Some(before) {
                return Ok(());
            }
            if current.is_some() {
                anyhow::bail!("rollback conflict: deleted identity was reused");
            }
            db.execute(
                &format!(
                    "INSERT INTO {} ({}) VALUES ({})",
                    quote(&change.table),
                    before
                        .keys()
                        .map(|n| quote(n))
                        .collect::<Vec<_>>()
                        .join(","),
                    vec!["?"; before.len()].join(",")
                ),
                params_from_iter(before.values().map(Cell::sql)),
            )?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_creation_and_repeated_persist_publish_recoverable_states() {
        let temp = tempfile::tempdir().unwrap();
        let backup = temp.path().join("backup");
        crate::backup::private_directory(&backup, true).unwrap();
        let path = temp.path().join(".codex-global-state.json");
        std::fs::write(&path, b"before").unwrap();

        // Windows 上首次 new 和之后的原子替换都不能对只读句柄执行 sync_all。
        let mut journal = SyncJournal::new(&backup).unwrap();
        let initial: SyncJournal =
            serde_json::from_slice(&std::fs::read(journal.path()).unwrap()).unwrap();
        assert_eq!(initial.state, "prepared");
        assert!(initial.files.is_empty());
        journal
            .prepare_file(&path, Some(b"before"), b"after")
            .unwrap();
        journal.persist().unwrap();
        assert!(!journal.path().with_extension("json.tmp").exists());
        std::fs::write(&path, b"after").unwrap();

        let mut loaded = SyncJournal::load_for_recovery(journal.path(), temp.path(), &[]).unwrap();
        assert_eq!(loaded.files.len(), 1);
        assert_eq!(
            std::fs::read(backup.join(&loaded.files[0].before_file)).unwrap(),
            b"before"
        );
        assert!(loaded.rollback().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), b"before");
        let persisted: SyncJournal =
            serde_json::from_slice(&std::fs::read(loaded.path()).unwrap()).unwrap();
        assert_eq!(persisted.state, "rolled_back");
        assert!(persisted.files[0].restored);
        loaded.complete().unwrap();
        assert!(SyncJournal::load_for_recovery(loaded.path(), temp.path(), &[]).is_err());
    }

    #[test]
    fn journal_write_failure_prevents_database_commit_and_preserves_previous_journal() {
        let temp = tempfile::tempdir().unwrap();
        let database_path = temp.path().join("state.db");
        let mut db = database(&database_path);
        let mut journal = SyncJournal::new(temp.path()).unwrap();
        let journal_path = journal.path().to_path_buf();
        let original = std::fs::read(&journal_path).unwrap();
        let blocked = temp.path().join("not-a-directory");
        std::fs::write(&blocked, b"blocker").unwrap();
        journal.path = blocked.join("recovery-journal.json");

        let tx = db.transaction().unwrap();
        let before = Snapshot::capture(&tx).unwrap();
        tx.execute("UPDATE threads SET model_provider='new'", [])
            .unwrap();
        assert!(journal.commit(tx, before, &database_path).is_err());
        assert_eq!(
            db.query_row(
                "SELECT model_provider FROM threads WHERE id='t'",
                [],
                |row| { row.get::<_, String>(0) }
            )
            .unwrap(),
            "old"
        );
        assert_eq!(std::fs::read(&journal_path).unwrap(), original);
        assert_eq!(std::fs::read(blocked).unwrap(), b"blocker");
    }

    fn database(path: &Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, model_provider TEXT, title TEXT, payload BLOB); INSERT INTO threads VALUES('t','old','original',X'0001ff');").unwrap();
        db
    }
    #[test]
    fn conditional_rollback_preserves_unrelated_concurrent_column_and_blobs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.db");
        let mut db = database(&path);
        let mut journal = SyncJournal::new(temp.path()).unwrap();
        let tx = db.transaction().unwrap();
        let before = Snapshot::capture(&tx).unwrap();
        tx.execute("UPDATE threads SET model_provider='new'", [])
            .unwrap();
        journal.commit(tx, before, &path).unwrap();
        db.execute("UPDATE threads SET title='concurrent'", [])
            .unwrap();
        assert!(journal.rollback().is_empty());
        let row: (String, String, Vec<u8>) = db
            .query_row(
                "SELECT model_provider,title,payload FROM threads",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("old".into(), "concurrent".into(), vec![0, 1, 255]));
        assert_eq!(journal.outcomes()[0].remaining_rows, 0);
        assert!(journal.rollback().is_empty());
    }
    #[test]
    fn changed_owned_column_is_a_partial_failure_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.db");
        let mut db = database(&path);
        let mut journal = SyncJournal::new(temp.path()).unwrap();
        let tx = db.transaction().unwrap();
        let before = Snapshot::capture(&tx).unwrap();
        tx.execute("UPDATE threads SET model_provider='new'", [])
            .unwrap();
        journal.commit(tx, before, &path).unwrap();
        db.execute("UPDATE threads SET model_provider='concurrent'", [])
            .unwrap();
        assert!(!journal.rollback().is_empty());
        assert_eq!(
            db.query_row("SELECT model_provider FROM threads", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "concurrent"
        );
        assert_eq!(journal.outcomes()[0].remaining_rows, 1);
    }
    #[test]
    fn commit_intent_can_be_loaded_and_compensates_insert_delete_and_update() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.db");
        let mut db = database(&path);
        let mut journal = SyncJournal::new(temp.path()).unwrap();
        let tx = db.transaction().unwrap();
        let before = Snapshot::capture(&tx).unwrap();
        tx.execute("DELETE FROM threads WHERE id='t'", []).unwrap();
        tx.execute(
            "INSERT INTO threads VALUES('new','new','created',X'02')",
            [],
        )
        .unwrap();
        journal.commit(tx, before, &path).unwrap();
        let mut recovered: SyncJournal =
            serde_json::from_slice(&std::fs::read(journal.path()).unwrap()).unwrap();
        recovered.path = journal.path().to_path_buf();
        recovered.entries[0].state = "commit_intent".into();
        assert!(recovered.rollback().is_empty());
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM threads WHERE id='t' AND payload=X'0001ff'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM threads WHERE id='new'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn global_file_rollback_refuses_concurrent_changes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.json");
        let mut journal = SyncJournal::new(temp.path()).unwrap();
        std::fs::write(&path, b"old").unwrap();
        journal.prepare_file(&path, Some(b"old"), b"new").unwrap();
        std::fs::write(&path, b"other").unwrap();
        assert!(!journal.rollback().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), b"other");
    }
    #[cfg(unix)]
    #[test]
    fn file_journal_accepts_canonical_home_alias_and_keeps_recovery_images_private() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let alias = temp.path().join("alias");
        symlink(&home, &alias).unwrap();
        let backup = home.join("backup");
        crate::backup::private_directory(&backup, true).unwrap();
        let path = alias.join(".codex-global-state.json");
        std::fs::write(&path, b"before").unwrap();
        let mut journal = SyncJournal::new(&backup).unwrap();
        journal
            .prepare_file(&path, Some(b"before"), b"after")
            .unwrap();
        std::fs::write(&path, b"after").unwrap();
        assert_eq!(
            std::fs::metadata(journal.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(backup.join("recovery-file-0.before"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let mut loaded = SyncJournal::load_for_recovery(journal.path(), &home, &[]).unwrap();
        assert!(loaded.rollback().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), b"before");
    }
}
