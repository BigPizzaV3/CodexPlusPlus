use super::{
    ingest::SourceReader,
    store::{Coverage, Store},
};
use crate::usage_history::structured::{discover, index_budget::Budget};
use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

const SOURCES_PER_ROUND: usize = 64;
const RECLAIM_BATCH: u32 = 256;

#[derive(Default)]
pub(super) struct Scan {
    remaining: VecDeque<PathBuf>,
    missing: VecDeque<(PathBuf, i64)>,
    more_missing: bool,
    active: Option<SourceReader>,
    deferred: Option<SourceReader>,
    rescan: bool,
    issues: usize,
    scoped_count_gaps: BTreeSet<PathBuf>,
    maintenance_day: Option<NaiveDate>,
    details_pending: bool,
    activity_pending: bool,
    observation_cursor: Option<(i64, (i64, i64))>,
}

pub(super) struct Report {
    pub(super) complete: bool,
    pub(super) pending_sources: usize,
    pub(super) issues: usize,
    pub(super) errors: Vec<(PathBuf, anyhow::Error)>,
    pub(super) maintenance_error: Option<anyhow::Error>,
}

impl Scan {
    pub(super) fn scoped_count_gaps(&self) -> &BTreeSet<PathBuf> {
        &self.scoped_count_gaps
    }

    fn maintain(&mut self, store: &mut Store, cutoff: NaiveDate) -> anyhow::Result<()> {
        if self.maintenance_day.is_none_or(|day| day < cutoff) {
            store.advance_retention(cutoff)?;
            self.maintenance_day = Some(cutoff);
            self.details_pending = true;
            self.activity_pending = true;
            self.observation_cursor = Some((0, (0, 0)));
        }
        if self.details_pending {
            self.details_pending =
                store.reclaim_expired_details(RECLAIM_BATCH)? == RECLAIM_BATCH as usize;
        }
        if self.activity_pending {
            self.activity_pending =
                store.reclaim_expired_activity(cutoff, RECLAIM_BATCH)? == RECLAIM_BATCH as usize;
        }
        if let Some(cursor) = &mut self.observation_cursor
            && !store.reclaim_next_source(cursor, RECLAIM_BATCH)?
        {
            self.observation_cursor = None;
        }
        Ok(())
    }

    pub(super) fn round(
        &mut self,
        store: &mut Store,
        home: &Path,
        cutoff: NaiveDate,
        provider_names: &BTreeMap<String, String>,
        reconciliation_limit: usize,
    ) -> Report {
        let maintenance_error = self.maintain(store, cutoff).err();
        let mut errors = Vec::new();
        if self.active.is_none() && self.remaining.is_empty() && self.missing.is_empty() {
            self.issues = 0;
            self.scoped_count_gaps.clear();
            self.more_missing = false;
            let paths = discover(home, !self.rescan, &mut self.issues);
            if self.issues == 0 {
                match store.missing_sources(&paths, SOURCES_PER_ROUND) {
                    Ok((missing, more)) => {
                        self.missing = missing;
                        self.more_missing = more;
                    }
                    Err(error) => {
                        self.issues += 1;
                        errors.push((home.to_owned(), error));
                    }
                }
            }
            self.remaining = paths.into();
            self.rescan = true;
            if let Some(reader) = &self.deferred {
                self.remaining.retain(|source| source != &reader.source);
            }
        }
        let mut budget = Budget::new(std::iter::empty());
        let mut retry = if self.active.is_none() {
            self.deferred.take()
        } else {
            None
        };
        for _ in 0..SOURCES_PER_ROUND {
            let mut reader = match self.active.take().or_else(|| retry.take()) {
                Some(source) => source,
                None => {
                    let Some(path) = self.remaining.pop_front() else {
                        // 先摄取完整的新来源，再处理旧路径，避免归档移动产生用量空窗。
                        if self.issues != 0 || self.deferred.is_some() {
                            if !self.missing.is_empty() || self.more_missing {
                                self.scoped_count_gaps.clear();
                            }
                            self.missing.clear();
                            self.more_missing = false;
                            break;
                        }
                        let Some((source, revision)) = self.missing.pop_front() else {
                            break;
                        };
                        match std::fs::symlink_metadata(&source) {
                            Ok(_) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                if let Err(error) = store.retire_source(
                                    &source,
                                    revision,
                                    cutoff,
                                    provider_names.clone(),
                                    reconciliation_limit,
                                ) {
                                    self.issues += 1;
                                    errors.push((source, error));
                                }
                            }
                            Err(error) => {
                                self.issues += 1;
                                errors.push((source, error.into()));
                            }
                        }
                        continue;
                    };
                    match SourceReader::restore(store, &path) {
                        Ok(reader) => reader,
                        Err(error) => {
                            self.issues += 1;
                            errors.push((path, error));
                            continue;
                        }
                    }
                }
            };
            let allowance = (budget.read_left, budget.source_left);
            match reader.step(
                store,
                cutoff,
                &mut budget,
                provider_names,
                reconciliation_limit,
            ) {
                Ok(true) => {
                    if reader.has_only_malformed_token_counts() {
                        self.scoped_count_gaps.insert(reader.source);
                        self.issues += 1;
                    } else {
                        self.issues += usize::from(reader.has_count_gap());
                    }
                }
                Ok(false)
                    if reader.has_open_reader()
                        && allowance != (budget.read_left, budget.source_left) =>
                {
                    self.active = Some(reader);
                    break;
                }
                Ok(false) => self.issues += 1,
                Err(error) => {
                    self.issues += 1;
                    errors.push((reader.source.clone(), error));
                    if reader.can_retry_write() && self.deferred.is_none() {
                        self.deferred = Some(reader);
                    }
                }
            }
            if budget.read_left == 0 || budget.source_left == 0 {
                break;
            }
        }
        let pending_sources = self.remaining.len()
            + self.missing.len()
            + usize::from(self.more_missing)
            + usize::from(self.active.is_some())
            + usize::from(self.deferred.is_some());
        let mut issues = self.issues;
        if let Err(error) = store.record_coverage(Coverage {
            pending_sources,
            issues,
            maintenance_blocked: maintenance_error.is_some(),
        }) {
            issues += 1;
            errors.push((home.to_owned(), error));
        }
        Report {
            complete: pending_sources == 0 && issues == 0 && maintenance_error.is_none(),
            pending_sources,
            issues,
            errors,
            maintenance_error,
        }
    }
}

#[cfg(test)]
mod tests;
