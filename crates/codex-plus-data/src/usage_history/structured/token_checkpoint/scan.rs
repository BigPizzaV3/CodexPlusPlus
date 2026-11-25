use super::{
    SourceStamp,
    ingest::SourceReader,
    store::{
        Coverage, Store,
        executions::CompactionCursor,
        replicas::{Comparison, Probe},
    },
};
use crate::usage_history::structured::{FileState, discover, index_budget::Budget};
use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};

const SOURCES_PER_ROUND: usize = 64;
const LIVE_REVISIT_SOURCES: usize = 8;
pub(super) const RECLAIM_BATCH: u32 = 256;

#[derive(Default)]
pub(super) struct Scan {
    remaining: VecDeque<PathBuf>,
    missing: VecDeque<(PathBuf, i64)>,
    more_missing: bool,
    active: Option<SourceReader>,
    revisiting: Option<SourceReader>,
    comparing: Option<Box<Comparison>>,
    deferred: Option<SourceReader>,
    rescan: bool,
    live: BTreeMap<PathBuf, SourceStamp>,
    waiting_for_newline: BTreeMap<PathBuf, (SourceStamp, bool)>,
    next_live: usize,
    issues: usize,
    unknown_count_scope: bool,
    sealed_queue: HashSet<PathBuf>,
    scoped_count_gaps: BTreeSet<PathBuf>,
    maintenance_day: Option<NaiveDate>,
    details_pending: bool,
    activity_pending: bool,
    execution_cursor: Option<CompactionCursor>,
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
    pub(super) fn sealed_pending(&self) -> usize {
        self.sealed_queue.len()
    }

    pub(super) fn scoped_count_gaps(&self) -> &BTreeSet<PathBuf> {
        &self.scoped_count_gaps
    }

    fn retirement_blocked(&self) -> bool {
        self.unknown_count_scope
            || self.issues != self.scoped_count_gaps.len()
            || self.deferred.is_some()
            || self.revisiting.is_some()
            || !self.waiting_for_newline.is_empty()
    }

    pub(super) fn maintain(&mut self, store: &mut Store, cutoff: NaiveDate) -> anyhow::Result<()> {
        let now = chrono::Local::now().timestamp_millis();
        store.expire_execution_locators(now, RECLAIM_BATCH)?;
        store.compact_execution_metadata(now, RECLAIM_BATCH)?;
        if self.maintenance_day.is_none_or(|day| day < cutoff) {
            store.advance_retention(cutoff)?;
            self.maintenance_day = Some(cutoff);
            self.details_pending = true;
            self.activity_pending = true;
            self.execution_cursor = None;
            self.observation_cursor = Some((0, (0, 0)));
        }
        if self.details_pending {
            self.details_pending =
                store.reclaim_expired_details(RECLAIM_BATCH)? == RECLAIM_BATCH as usize;
        }
        self.activity_pending |=
            store.compact_executions(cutoff, &mut self.execution_cursor, RECLAIM_BATCH)? != 0;
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
        let backlogged = !self.remaining.is_empty();
        let recheck_missing = !self.missing.is_empty()
            && self.remaining.len() == self.sealed_queue.len()
            && !self.retirement_blocked();
        let mut current_inventory = false;
        if self.active.is_none()
            && self.comparing.is_none()
            && (self.remaining.is_empty() || recheck_missing)
        {
            self.issues = 0;
            let previous_queue = std::mem::take(&mut self.sealed_queue);
            self.scoped_count_gaps.clear();
            self.missing.clear();
            self.more_missing = false;
            let paths = discover(home, !self.rescan, &mut self.issues);
            self.unknown_count_scope = self.issues != 0;
            if self.issues == 0 {
                match store.missing_sources(&paths, SOURCES_PER_ROUND) {
                    Ok((missing, more)) => {
                        self.missing = missing;
                        self.more_missing = more;
                    }
                    Err(error) => {
                        self.issues += 1;
                        self.unknown_count_scope = true;
                        errors.push((home.to_owned(), error));
                    }
                }
            }
            let waiting = std::mem::take(&mut self.waiting_for_newline);
            self.remaining = paths
                .into_iter()
                .filter(|path| {
                    // 有计数缺口的前缀需从持久检查点恢复诊断，只有干净的半行可以跳过。
                    if let Some(&(stamp, false)) = waiting.get(path)
                        && FileState::source_stamp(path).is_ok_and(|current| current == stamp)
                    {
                        self.waiting_for_newline.insert(path.clone(), (stamp, false));
                        false
                    } else {
                        true
                    }
                })
                .collect();
            self.rescan = true;
            for reader in self.deferred.iter().chain(&self.revisiting) {
                self.remaining.retain(|source| source != &reader.source);
            }
            if self.issues == 0 {
                match store.sealed_queued_sources(self.remaining.make_contiguous()) {
                    Ok((sealed, gaps, growing)) => {
                        self.sealed_queue = sealed;
                        self.issues += gaps.len();
                        self.scoped_count_gaps = gaps;
                        current_inventory = true;
                        if recheck_missing {
                            // 已处理过、且本轮仍证实封存的路径无需重新排队。
                            self.remaining.retain(|path| {
                                !self.sealed_queue.contains(path) || previous_queue.contains(path)
                            });
                            self.sealed_queue
                                .retain(|path| previous_queue.contains(path));
                        }
                        if !self.missing.is_empty() {
                            let names: HashSet<_> = self.missing.iter()
                                .filter_map(|(path, _)| path.file_name())
                                .collect();
                            self.remaining.make_contiguous().sort_by_key(|path| {
                                (
                                    self.sealed_queue.contains(path),
                                    !path.file_name().is_some_and(|name| names.contains(name)),
                                )
                            });
                        } else if !growing.is_empty() {
                            // 优先解析追加内容，读完并提交后才可按新的计数范围保留归属。
                            self.remaining
                                .make_contiguous()
                                .sort_by_key(|path| !growing.contains(path));
                        }
                    }
                    Err(error) => {
                        self.issues += 1;
                        self.unknown_count_scope = true;
                        errors.push((home.to_owned(), error));
                    }
                }
            }
        }
        let mut budget = Budget::new(std::iter::empty());
        let mut retry = if self.active.is_none() {
            self.deferred.take()
        } else {
            None
        };
        let mut compared = false;
        let mut verify_later = Vec::new();
        let sessions_dir = home.join("sessions");
        let mut live_path = None;
        if (backlogged || self.active.is_some() || self.comparing.is_some())
            && self.revisiting.is_none()
        {
            for _ in 0..self.live.len() {
                let index = self.next_live % self.live.len();
                self.next_live = index + 1;
                let (path, stamp) = self.live.iter().nth(index).unwrap();
                if self
                    .active
                    .iter()
                    .chain(&retry)
                    .chain(&self.deferred)
                    .any(|r| &r.source == path)
                    || self.comparing.as_ref().is_some_and(|probe| probe.compares(path))
                {
                    continue;
                }
                let stamp = self.waiting_for_newline.get(path).map_or(*stamp, |tail| tail.0);
                let path = path.clone();
                match FileState::source_stamp(&path) {
                    Ok(current) if current != stamp => {
                        self.remaining.retain(|p| p != &path);
                        if let Some((_, true)) = self.waiting_for_newline.remove(&path) {
                            self.issues -= 1;
                            self.scoped_count_gaps.remove(&path);
                        }
                        live_path = Some(path);
                        break;
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        self.live.remove(&path);
                    }
                    Err(error) => {
                        self.live.remove(&path);
                        self.issues += 1;
                        self.unknown_count_scope = true;
                        errors.push((path, error.into()));
                    }
                }
            }
        }
        'sources: for slot in 0..SOURCES_PER_ROUND {
            if budget.read_left == 0 || budget.source_left == 0 {
                break;
            }
            let revisit = slot == 0 && (live_path.is_some() || self.revisiting.is_some());
            let resumed = if revisit {
                self.revisiting.take()
            } else {
                self.active.take().or_else(|| retry.take())
            };
            let mut reader = match resumed {
                Some(source) => source,
                None => {
                    let comparing = if compared || revisit {
                        None
                    } else {
                        self.comparing.take()
                    };
                    let blocked_retirement = self.retirement_blocked();
                    // 只在本轮已核实所有潜在迁移目的地时优先处理缺失路径。
                    let retire_first = current_inventory
                        && !blocked_retirement
                        && !self.missing.is_empty()
                        && self.remaining.len() == self.sealed_queue.len()
                        && self.comparing.is_none()
                        && verify_later.is_empty();
                    let path = live_path
                        .take()
                        .or_else(|| comparing.as_ref().map(|check| check.path().to_owned()))
                        .or_else(|| {
                            if retire_first { None } else { self.remaining.pop_front() }
                        });
                    let Some(path) = path else {
                        // 先摄取完整的新来源，再处理旧路径，避免归档移动产生用量空窗。
                        if self.comparing.is_some() || !verify_later.is_empty() {
                            break;
                        }
                        if blocked_retirement {
                            if !self.missing.is_empty() || self.more_missing {
                                self.unknown_count_scope = true;
                                self.scoped_count_gaps.clear();
                            }
                            self.missing.clear();
                            self.more_missing = false;
                            break;
                        }
                        if !current_inventory {
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
                                    self.unknown_count_scope = true;
                                    errors.push((source, error));
                                }
                            }
                            Err(error) => {
                                self.issues += 1;
                                self.unknown_count_scope = true;
                                errors.push((source, error.into()));
                            }
                        }
                        continue;
                    };
                    if self.sealed_queue.remove(&path) && self.scoped_count_gaps.remove(&path) {
                        self.issues -= 1;
                    }
                    if self
                        .comparing
                        .as_ref()
                        .is_some_and(|probe| probe.holds(&path))
                    {
                        verify_later.push(path);
                        continue;
                    }
                    let probe = if let Some(comparing) = comparing {
                        compared = true;
                        comparing.step(store, &mut budget)
                    } else {
                        for i in 0..self.missing.len() {
                            let (missing, revision) = &self.missing[i];
                            match store.relocate_source(missing, *revision, &path) {
                                Ok(true) => {
                                    self.missing.remove(i);
                                    break;
                                }
                                Ok(false) => {}
                                Err(error) => {
                                    self.issues += 1;
                                    self.unknown_count_scope = true;
                                    errors.push((path, error));
                                    continue 'sources;
                                }
                            }
                        }
                        store.probe_copy(&path).and_then(|probe| match probe {
                            Probe::Compare(comparison) if !compared && self.comparing.is_none() => {
                                compared = true;
                                comparison.step(store, &mut budget)
                            }
                            other => Ok(other),
                        })
                    };
                    match probe {
                        Ok(Probe::Compare(comparing)) => {
                            if self.comparing.is_none() {
                                self.comparing = Some(comparing);
                            } else {
                                verify_later.push(path);
                            }
                            continue;
                        }
                        Ok(Probe::Covered { moved_from }) => {
                            if let Some(old) = moved_from {
                                self.missing.retain(|(source, _)| *source != old);
                            }
                            continue;
                        }
                        Ok(Probe::WaitForOwner(owner)) => {
                            if let Some(i) = self
                                .remaining
                                .iter()
                                .position(|candidate| *candidate == owner)
                            {
                                self.remaining.remove(i);
                                self.remaining.push_front(path);
                                self.remaining.push_front(owner);
                                continue;
                            }
                        }
                        Ok(Probe::ReadSource) => {}
                        Err(error) => {
                            self.issues += 1;
                            self.unknown_count_scope = true;
                            errors.push((path, error));
                            continue;
                        }
                    }
                    if budget.read_left == 0 || budget.source_left == 0 {
                        self.remaining.push_front(path);
                        break;
                    }
                    match SourceReader::restore(store, &path) {
                        Ok(reader) => reader,
                        Err(error) => {
                            self.issues += 1;
                            self.unknown_count_scope = true;
                            errors.push((path, error));
                            continue;
                        }
                    }
                }
            };
            let parked = self
                .active
                .iter()
                .chain(&self.revisiting)
                .chain(&self.deferred)
                .chain(&retry);
            let other_work = self.active.is_some() || retry.is_some() || self.comparing.is_some() || !self.remaining.is_empty();
            let mut slot_budget =
                budget.checkpoint_slot(parked.map(|r| &r.state), revisit && other_work);
            let allowance = (slot_budget.read_left, slot_budget.source_left);
            let result = reader.step(
                store,
                cutoff,
                &mut slot_budget,
                provider_names,
                reconciliation_limit,
            );
            let consumed = (
                allowance.0 - slot_budget.read_left,
                allowance.1 - slot_budget.source_left,
            );
            budget.read_left -= consumed.0;
            budget.source_left -= consumed.1;
            match result {
                Ok(complete) if complete || reader.waiting_for_newline() => {
                    if !complete {
                        self.waiting_for_newline.insert(
                            reader.source.clone(),
                            (reader.state.stamp.unwrap(), reader.has_count_gap()),
                        );
                    }
                    if reader.has_only_malformed_token_counts() {
                        self.scoped_count_gaps.insert(reader.source);
                        self.issues += 1;
                    } else if reader.has_count_gap() {
                        self.issues += 1;
                        self.unknown_count_scope = true;
                    } else if reader.source.starts_with(&sessions_dir)
                        && reader.source.extension().is_some_and(|ext| ext == "jsonl")
                        && let Some(stamp) = reader.state.stamp
                        && stamp.0 > 0
                    {
                        self.live.insert(reader.source, stamp);
                        if self.live.len() > LIVE_REVISIT_SOURCES {
                            let oldest = self
                                .live
                                .iter()
                                .min_by_key(|(_, stamp)| stamp.1)
                                .unwrap()
                                .0
                                .clone();
                            self.live.remove(&oldest);
                        }
                    }
                }
                Ok(false) if revisit && consumed != (0, 0) && reader.can_yield_plain_batch() => {
                    // 已提交的普通前缀可直接 seek；下一轮把实时名额交给下一个变化的源。
                    verify_later.push(reader.source);
                }
                Ok(false)
                    if revisit
                        && (reader.encoding_pending()
                            || reader.has_open_reader() && consumed != (0, 0)) =>
                {
                    self.revisiting = Some(reader);
                }
                Ok(false) if reader.encoding_pending() => {
                    if self.deferred.is_none() {
                        self.deferred = Some(reader);
                    } else {
                        verify_later.push(reader.source);
                    }
                }
                Ok(false) if reader.has_open_reader() && consumed != (0, 0) => {
                    self.active = Some(reader);
                    break;
                }
                Ok(_) => {
                    self.issues += 1;
                    self.unknown_count_scope = true;
                }
                Err(error) => {
                    self.issues += 1;
                    self.unknown_count_scope |= !reader.can_retry_write();
                    errors.push((reader.source.clone(), error));
                    if revisit && reader.can_retry_write() {
                        self.revisiting = Some(reader);
                    } else if reader.can_retry_write() && self.deferred.is_none() {
                        self.deferred = Some(reader);
                    }
                }
            }
        }
        self.remaining.extend(verify_later);
        let pending_sources = self.remaining.len()
            + self.waiting_for_newline.len()
            + self.missing.len()
            + usize::from(self.more_missing)
            + usize::from(self.active.is_some())
            + usize::from(self.revisiting.is_some())
            + usize::from(self.comparing.is_some())
            + usize::from(self.deferred.is_some());
        let mut issues = self.issues;
        // The previous snapshot stays intact until this known append actually commits.
        let unpublished_append = self
            .deferred
            .iter()
            .chain(&self.revisiting)
            .filter(|reader| reader.retrying_closed_append())
            .count();
        if let Err(error) = store.record_coverage(
            Coverage {
                pending_sources,
                issues,
                maintenance_blocked: maintenance_error.is_some(),
            },
            self.unknown_count_scope
                || pending_sources != self.sealed_pending() + unpublished_append,
        ) {
            issues += 1;
            return Report {
                complete: false,
                pending_sources,
                issues,
                errors,
                maintenance_error: Some(error),
            };
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
