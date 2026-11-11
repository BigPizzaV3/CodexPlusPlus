//! 对当前可见来源做有界核验；结果不证明建索引前已删除的历史来源。
use super::{checkpoint, scan};
use crate::usage_history::structured::{log_file, observed_file_identity};
use anyhow::{Context, ensure};
use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;
use std::fs::{self, ReadDir};
use std::ops::Bound::{Excluded, Unbounded};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAX_DIRECTORY_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHECKPOINT_BYTES: i64 = 4 * 1024 * 1024;
const MAX_DIRECTORY_DEPTH: usize = 4;

/// 这些字段只描述在扫描区间内观察到的来源，不是历史完整性证明。
#[derive(Debug, PartialEq, Eq)]
pub(in crate::usage_history::structured) struct CurrentSourcesCoverage {
    pub(in crate::usage_history::structured) revision: i64,
    pub(in crate::usage_history::structured) started_at_ms: i64,
    pub(in crate::usage_history::structured) finished_at_ms: i64,
    pub(in crate::usage_history::structured) source_count: usize,
    pub(in crate::usage_history::structured) usage_since: Option<NaiveDate>,
    pub(in crate::usage_history::structured) counts_valid: bool,
    pub(in crate::usage_history::structured) scope_valid: bool,
    pub(in crate::usage_history::structured) executions_valid: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryStamp {
    identity: (u64, u64),
    modified: SystemTime,
    len: u64,
}

struct Directory {
    stamp: Option<DirectoryStamp>,
    seen_again: bool,
}

struct Frame {
    path: PathBuf,
    stamp: DirectoryStamp,
    entries: ReadDir,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    First,
    Second,
    Confirm,
    Finished,
}

/// 调用方应在后台分批调用，并且只将相同 revision 的证明用于已提交查询。
pub(in crate::usage_history::structured) struct CoverageProbe {
    home: PathBuf,
    revision: i64,
    started_at_ms: i64,
    phase: Phase,
    root_index: usize,
    stack: Vec<Frame>,
    directories: BTreeMap<PathBuf, Directory>,
    directory_bytes: usize,
    confirmation_cursor: Option<PathBuf>,
    first_sources: usize,
    second_sources: usize,
    second_directories: usize,
    usage_since: Option<NaiveDate>,
    unknown_cutoff: bool,
    counts_valid: bool,
    scope_valid: bool,
    executions_valid: bool,
}

impl CoverageProbe {
    pub(in crate::usage_history::structured) fn new(
        home: &Path,
        db: &Connection,
        started_at_ms: i64,
    ) -> anyhow::Result<Self> {
        let home = fs::canonicalize(home).context("cannot locate usage home")?;
        ensure!(home.is_dir(), "usage home is not a directory");
        Ok(Self {
            home,
            revision: revision(db)?,
            started_at_ms,
            phase: Phase::First,
            root_index: 0,
            stack: Vec::new(),
            directories: BTreeMap::new(),
            directory_bytes: 0,
            confirmation_cursor: None,
            first_sources: 0,
            second_sources: 0,
            second_directories: 0,
            usage_since: None,
            unknown_cutoff: false,
            counts_valid: true,
            scope_valid: true,
            executions_valid: true,
        })
    }

    /// 超出预算时返回 None；任何错误都表示本次扫描不能产生覆盖证明。
    pub(in crate::usage_history::structured) fn step(
        &mut self,
        db: &Connection,
        max_steps: usize,
        now_ms: i64,
    ) -> anyhow::Result<Option<CurrentSourcesCoverage>> {
        ensure!(self.phase != Phase::Finished, "coverage probe already finished");
        ensure!(
            revision(db)? == self.revision,
            "usage index changed during coverage probe"
        );
        for _ in 0..max_steps {
            if self.advance(db)? {
                ensure!(
                    revision(db)? == self.revision,
                    "usage index changed during coverage probe"
                );
                let count_gaps: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM usage_count_gaps)
                            OR EXISTS(SELECT 1 FROM usage_unindexed_checkpoints)",
                    [],
                    |row| row.get(0),
                )?;
                ensure!(
                    revision(db)? == self.revision,
                    "usage index changed during coverage probe"
                );
                self.phase = Phase::Finished;
                return Ok(Some(CurrentSourcesCoverage {
                    revision: self.revision,
                    started_at_ms: self.started_at_ms,
                    finished_at_ms: now_ms,
                    source_count: self.first_sources,
                    usage_since: if self.unknown_cutoff {
                        None
                    } else {
                        self.usage_since
                    },
                    counts_valid: self.counts_valid && !count_gaps,
                    scope_valid: self.scope_valid,
                    executions_valid: self.executions_valid,
                }));
            }
        }
        ensure!(
            revision(db)? == self.revision,
            "usage index changed during coverage probe"
        );
        Ok(None)
    }

    fn advance(&mut self, db: &Connection) -> anyhow::Result<bool> {
        if self.phase == Phase::Confirm {
            let next = match &self.confirmation_cursor {
                Some(cursor) => self
                    .directories
                    .range((Excluded(cursor.clone()), Unbounded))
                    .next(),
                None => self.directories.iter().next(),
            };
            let Some((path, directory)) = next else {
                return Ok(true);
            };
            ensure!(
                directory_stamp(path)? == directory.stamp,
                "usage source directory changed during coverage probe"
            );
            self.confirmation_cursor = Some(path.clone());
            return Ok(false);
        }

        if self.stack.is_empty() {
            if self.root_index < 2 {
                let name = ["sessions", "archived_sessions"][self.root_index];
                self.root_index += 1;
                self.enter_directory(self.home.join(name))?;
                return Ok(false);
            }
            if self.phase == Phase::First {
                self.phase = Phase::Second;
                self.root_index = 0;
                return Ok(false);
            }
            ensure!(
                self.second_directories == self.directories.len()
                    && self.first_sources == self.second_sources,
                "usage source set changed during coverage probe"
            );
            let stored: usize =
                db.query_row("SELECT COUNT(*) FROM usage_sources", [], |row| row.get(0))?;
            ensure!(
                stored == self.first_sources,
                "usage source set differs from the committed index"
            );
            self.phase = Phase::Confirm;
            return Ok(false);
        }

        let next = self.stack.last_mut().unwrap().entries.next();
        let Some(entry) = next else {
            let frame = self.stack.pop().unwrap();
            ensure!(
                directory_stamp(&frame.path)? == Some(frame.stamp),
                "usage source directory changed while reading"
            );
            return Ok(false);
        };
        let entry = entry.context("cannot enumerate usage source directory")?;
        let path = entry.path();
        let kind = entry.file_type().context("cannot inspect usage source")?;
        ensure!(!kind.is_symlink(), "usage source contains a symbolic link");
        if kind.is_dir() {
            let depth = path
                .strip_prefix(&self.home)
                .context("usage source directory is outside its home")?
                .components()
                .count();
            ensure!(
                depth <= MAX_DIRECTORY_DEPTH,
                "usage source directory exceeds discovery depth"
            );
            self.enter_directory(path)?;
        } else if kind.is_file() && log_file(&path) {
            let path = fs::canonicalize(path).context("cannot locate usage source")?;
            ensure!(
                path.starts_with(&self.home),
                "usage source is outside its home"
            );
            self.check_source(db, &path)?;
            if self.phase == Phase::First {
                self.first_sources += 1;
            } else {
                self.second_sources += 1;
            }
        }
        Ok(false)
    }

    fn enter_directory(&mut self, path: PathBuf) -> anyhow::Result<()> {
        let stamp = directory_stamp(&path)?;
        if self.phase == Phase::First {
            let charge = path.as_os_str().len() * 2 + 256;
            ensure!(
                self.directory_bytes + charge <= MAX_DIRECTORY_BYTES,
                "usage coverage directory budget exhausted"
            );
            self.directory_bytes += charge;
            ensure!(
                self.directories
                    .insert(
                        path.clone(),
                        Directory {
                            stamp: stamp.clone(),
                            seen_again: false,
                        },
                    )
                    .is_none(),
                "usage source directory was visited twice"
            );
        } else {
            let saved = self
                .directories
                .get_mut(&path)
                .context("usage source directory appeared during coverage probe")?;
            ensure!(
                saved.stamp == stamp && !saved.seen_again,
                "usage source directory changed during coverage probe"
            );
            saved.seen_again = true;
            self.second_directories += 1;
        }
        if let Some(stamp) = stamp {
            let entries = fs::read_dir(&path).context("cannot read usage source directory")?;
            self.stack.push(Frame {
                path,
                stamp,
                entries,
            });
        }
        Ok(())
    }

    fn check_source(&mut self, db: &Connection, path: &Path) -> anyhow::Result<()> {
        let before = file_stamp(path)?;
        let row = db
            .query_row(
                "SELECT identity, offset, parser_state, complete, usage_cutoff,
                        scope_indexed, scope_valid, execution_indexed, execution_valid
                 FROM usage_sources
                 WHERE source = ?1 AND length(parser_state) <= ?2",
                rusqlite::params![scan::encode_source(path), MAX_CHECKPOINT_BYTES],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, bool>(5)?,
                        row.get::<_, bool>(6)?,
                        row.get::<_, bool>(7)?,
                        row.get::<_, bool>(8)?,
                    ))
                },
            )
            .optional()?
            .context("usage source is not indexed or its checkpoint exceeds the probe budget")?;
        let state = checkpoint::restore(&row.2).context("cannot restore usage source checkpoint")?;
        let offset = u64::try_from(row.1).context("invalid usage source offset")?;
        let compressed = path.extension().is_some_and(|extension| extension == "zst");
        ensure!(
            row.3
                && state.offset == offset
                && !state.needs_resume
                && state.pending.is_empty()
                && !state.skipping
                && !state.identity_conflict
                && state.stamp == Some(before)
                && row.0 == scan::encode_identity(before.2.unwrap())
                && (compressed || offset == before.0),
            "usage source is not fully committed at its observed identity"
        );
        ensure!(
            file_stamp(path)? == before,
            "usage source changed during coverage probe"
        );
        match row.4 {
            Some(cutoff) => {
                let cutoff = NaiveDate::parse_from_str(&cutoff, "%Y-%m-%d")
                    .context("invalid usage source cutoff")?;
                self.usage_since = Some(self.usage_since.map_or(cutoff, |old| old.max(cutoff)));
            }
            None => self.unknown_cutoff = true,
        }
        self.counts_valid &= state.issues == 0
            && !state.unlocalized_invalid_counts
            && !state.pending_count_gap;
        self.scope_valid &= row.5 && row.6;
        self.executions_valid &= row.7 && row.8 && !state.execution_incomplete;
        Ok(())
    }
}

fn revision(db: &Connection) -> anyhow::Result<i64> {
    Ok(db.query_row(
        "SELECT revision FROM usage_meta WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?)
}

fn file_stamp(path: &Path) -> anyhow::Result<(u64, Option<SystemTime>, Option<(u64, u64)>)> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !is_reparse_point(&metadata),
        "usage source is not a regular file"
    );
    ensure!(
        fs::canonicalize(path)? == path,
        "usage source path changed"
    );
    let modified = metadata.modified().context("usage source has no modification time")?;
    let identity = observed_file_identity(path, &metadata)?
        .context("usage source has no stable file identity")?;
    Ok((metadata.len(), Some(modified), Some(identity)))
}

fn directory_stamp(path: &Path) -> anyhow::Result<Option<DirectoryStamp>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_dir() && !is_reparse_point(&metadata),
        "usage source directory is not a regular directory"
    );
    ensure!(
        fs::canonicalize(path)? == path,
        "usage source directory path changed"
    );
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        fs::OpenOptions::new()
            .access_mode(0)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
            .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0)
            .open(path)?
    };
    #[cfg(not(windows))]
    let file = fs::File::open(path)?;
    let opened = file.metadata()?;
    let identity = codex_plus_core::codex_sqlite::opened_file_identity(&file)?;
    ensure!(
        opened.is_dir()
            && opened.modified()? == metadata.modified()?
            && opened.len() == metadata.len(),
        "usage source directory changed while opening"
    );
    Ok(Some(DirectoryStamp {
        identity,
        modified: opened.modified()?,
        len: opened.len(),
    }))
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
}

#[cfg(not(windows))]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests;
