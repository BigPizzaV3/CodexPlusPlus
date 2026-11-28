use super::{
    SourceStamp,
    disk_full,
    ingest::SourceReader,
    store::{
        Coverage, Store,
        executions::CompactionCursor,
        replicas::{Comparison, Probe},
    },
};
use crate::usage_history::structured::{FileState, discover, discover_directories, index_budget::Budget};
use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};

const SOURCES_PER_ROUND: usize = 64;
pub(super) const RECLAIM_BATCH: u32 = 256;

#[derive(Clone, Copy)]
struct WaitingLine {
    stamp: SourceStamp,
    count_gap: bool,
    scoped: bool,
}

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
    discovered: HashSet<PathBuf>,
    live: BTreeMap<PathBuf, SourceStamp>,
    waiting_for_newline: BTreeMap<PathBuf, WaitingLine>,
    last_revisited: Option<PathBuf>,
    waiting_prefixes: HashSet<PathBuf>,
    issues: usize,
    unknown_count_scope: bool,
    pub(super) paired_only: bool,
    sealed_queue: HashSet<PathBuf>,
    legacy_prefixes: BTreeMap<PathBuf, (SourceStamp, bool)>,
    scoped_count_gaps: BTreeSet<PathBuf>,
    maintenance_day: Option<NaiveDate>,
    details_pending: bool,
    activity_pending: bool,
    execution_cursor: Option<CompactionCursor>,
    observation_cursor: Option<(i64, (i64, i64))>,
}

pub(super) struct Report {
    pub(super) complete: bool,
    pub(super) can_classify: bool,
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
            || self.waiting_for_newline.values().any(|line| !line.scoped)
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
        let mut budget = Budget::new(std::iter::empty());
        let batch_bytes = budget.read_left;
        let mut finish_appends = false;
        let backlogged = !self.remaining.is_empty();
        let recheck_missing = !self.missing.is_empty()
            && self.remaining.len() == self.sealed_queue.len()
            && !self.retirement_blocked();
        let mut current_inventory = false;
        let sessions_dir = home.join("sessions");
        if self.active.is_none()
            && self.comparing.is_none()
            && (self.remaining.is_empty() || recheck_missing)
        {
            self.issues = 0;
            let previous_queue = std::mem::take(&mut self.sealed_queue);
            self.legacy_prefixes.clear();
            self.waiting_prefixes.clear();
            self.scoped_count_gaps.clear();
            self.missing.clear();
            self.more_missing = false;
            let paths = discover(home, !self.rescan, &mut self.issues);
            self.discovered = paths.iter().cloned().collect();
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
                    if let Some(&line) = waiting.get(path)
                        && !line.count_gap
                        && FileState::source_stamp(path).is_ok_and(|current| current == line.stamp)
                    {
                        self.waiting_for_newline.insert(path.clone(), line);
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
                    Ok((sealed, gaps, growing, unchecked, prefixes)) => {
                        for (path, &stamp) in sealed.iter().chain(&unchecked) {
                            if path.starts_with(&sessions_dir)
                                && path.extension().is_some_and(|ext| ext == "jsonl")
                                && stamp.0 > 0 && !gaps.contains(path)
                            {
                                self.live.insert(path.clone(), stamp);
                            }
                        }
                        self.sealed_queue = sealed.into_keys().collect();
                        self.legacy_prefixes = prefixes;
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
                        // 不再增长的近期会话也可能在等待关联归档的旧前缀复核。
                        let recent_since = self.legacy_prefixes.values().any(|&(_, counts)| counts)
                            .then(|| chrono::Local::now().timestamp_millis() - super::RECENT_CLASSIFICATION_MS);
                        if (recent_since.is_some() || self.missing.is_empty() && !growing.is_empty())
                            && let Err(error) = store.prioritize_queue(&growing, recent_since, &mut self.remaining)
                        {
                            self.issues += 1;
                            self.unknown_count_scope = true;
                            errors.push((home.to_owned(), error));
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
                        } else if !growing.is_empty()
                            && !self.retirement_blocked()
                            && !self.more_missing
                            && self.remaining.len() == self.sealed_queue.len() + growing.len()
                        {
                            // 再留一个字节探测 EOF；每批仍按普通预算提交。
                            let observed = growing.values().sum::<u64>().saturating_add(1);
                            if observed > batch_bytes && observed <= reconciliation_limit as u64
                            {
                                budget.read_left = observed;
                                budget.source_left = observed;
                                finish_appends = true;
                            }
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
        let mut retry = if self.active.is_none() {
            self.deferred.take()
        } else {
            None
        };
        let mut compared = false;
        let mut verify_later = Vec::new();
        let mut changed = VecDeque::new();
        if !current_inventory && (backlogged || self.active.is_some() || self.comparing.is_some()) {
            self.live.retain(|path, stamp| {
                if self
                    .active
                    .iter()
                    .chain(&retry)
                    .chain(&self.deferred)
                    .chain(&self.revisiting)
                    .any(|r| &r.source == path)
                    || self.comparing.as_ref().is_some_and(|probe| probe.compares(path))
                {
                    return true;
                }
                let stamp = self.waiting_for_newline.get(path).map_or(*stamp, |line| line.stamp);
                match FileState::source_stamp(path) {
                    Ok(current) if current != stamp => {
                        changed.push_back(path.clone());
                        true
                    }
                    Ok(_) => true,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                    Err(error) => {
                        self.issues += 1;
                        self.unknown_count_scope = true;
                        errors.push((path.clone(), error.into()));
                        false
                    }
                }
            });
            if let Some(last) = &self.last_revisited {
                let next = changed.partition_point(|path| path <= last);
                changed.rotate_left(next);
            }
            // 新会话与已知文件追加共用实时名额，不必等无关归档扫描结束。
            let mut discovery_errors = 0;
            let paths = discover_directories(home, vec![sessions_dir.clone()], false, &mut discovery_errors);
            self.issues += discovery_errors;
            self.unknown_count_scope |= discovery_errors != 0;
            changed.extend(paths.into_iter().filter(|path| self.discovered.insert(path.clone())));
        }
        if !changed.is_empty() {
            if self.active.is_none() && retry.is_none() && self.comparing.is_none()
                && let Some(head) = self.remaining.front()
                && self.waiting_prefixes.remove(head)
            {
                changed.retain(|path| path != head);
            }
            let growing: HashSet<_> = changed.iter().collect();
            self.remaining.retain(|path| !growing.contains(path));
            for path in &changed {
                let was_waiting = self.waiting_for_newline.remove(path).is_some_and(|line| line.count_gap);
                let was_sealed = self.sealed_queue.remove(path);
                let was_prefix = self.legacy_prefixes.remove(path).is_some();
                let removed_gap = (was_waiting || was_sealed || was_prefix) && self.scoped_count_gaps.remove(path);
                if was_waiting || removed_gap {
                    self.issues -= 1;
                }
            }
        }
        let history_pending = self.active.is_some() || retry.is_some()
            || self.comparing.is_some() || !self.remaining.is_empty();
        let mut live_bytes = if history_pending { budget.read_left / 2 } else { budget.read_left };
        let mut live_source_bytes = if history_pending { budget.source_left / 2 } else { budget.source_left };
        'sources: for slot in 0..SOURCES_PER_ROUND {
            if budget.read_left == 0 || budget.source_left == 0 {
                break;
            }
            let revisit = live_bytes > 0 && live_source_bytes > 0
                && (!history_pending || slot + 1 < SOURCES_PER_ROUND)
                && if self.revisiting.is_some() { slot == 0 } else { !changed.is_empty() };
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
                    let live_path = if revisit {
                        let path = changed.pop_front().unwrap();
                        self.last_revisited = Some(path.clone());
                        Some(path)
                    } else {
                        None
                    };
                    let path = live_path
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
                    self.sealed_queue.remove(&path);
                    if self.scoped_count_gaps.remove(&path) {
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
            let mut slot_budget = budget.checkpoint_slot(parked.flat_map(|r| r.states()));
            if revisit {
                slot_budget.read_left = slot_budget.read_left.min(live_bytes);
                slot_budget.source_left = slot_budget.source_left.min(live_source_bytes);
            }
            slot_budget.read_left = slot_budget.read_left.min(batch_bytes);
            slot_budget.source_left = slot_budget.source_left.min(batch_bytes);
            let allowance = (slot_budget.read_left, slot_budget.source_left);
            let step = if revisit { SourceReader::step } else { SourceReader::step_in_history_slot };
            let result = step(
                &mut reader,
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
            if revisit {
                live_bytes -= consumed.0;
                live_source_bytes -= consumed.1;
            }
            if result.is_ok() && let Some(prefix) = reader.closed_legacy_prefix() {
                self.legacy_prefixes.insert(reader.source.clone(), prefix);
                if let Some(stamp) = self.live.get_mut(&reader.source) {
                    *stamp = prefix.0;
                }
            } else if reader.state.identity_conflict {
                self.legacy_prefixes.remove(&reader.source);
            }
            match result {
                Ok(true) if reader.state.legacy_identity_prefix != 0 => {
                    // 尾部已提交，旧前缀由历史队列复核，实时名额留给下一次追加。
                    if let Some(stamp) = self.live.get_mut(&reader.source) {
                        *stamp = reader.state.stamp.unwrap();
                    }
                    self.waiting_prefixes.insert(reader.source.clone());
                    verify_later.push(reader.source);
                }
                Ok(complete) if complete || reader.waiting_for_newline() => {
                    self.legacy_prefixes.remove(&reader.source);
                    if !complete {
                        self.waiting_for_newline.insert(
                            reader.source.clone(),
                            WaitingLine {
                                stamp: reader.state.stamp.unwrap(),
                                count_gap: reader.has_count_gap(),
                                scoped: reader.waiting_line_has_committed_scope(),
                            },
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
                    if finish_appends && (slot_budget.read_left == 0 || slot_budget.source_left == 0)
                    {
                        continue;
                    }
                    break;
                }
                Ok(_) => {
                    // 本轮没有读取名额时保留队列，不把等待当作来源损坏。
                    verify_later.push(reader.source);
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
        // 已变化但尚未获分配的来源仍属于未处理队列，不能沿用封存时的覆盖结论。
        self.remaining.extend(changed);
        let (prefixes, later): (Vec<_>, Vec<_>) = verify_later
            .into_iter()
            .partition(|path| self.waiting_prefixes.contains(path));
        self.remaining.extend(later);
        // 已追平的活跃来源优先补完旧前缀，不再排到全部无关归档之后。
        for path in prefixes.into_iter().rev() {
            self.remaining.push_front(path);
        }
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
        // 盘点之后，路径也可能被替换。旧前缀只对同一份已提交内容限定会话范围。
        let mut affected = self.scoped_count_gaps.clone();
        let mut rechecking = BTreeSet::new();
        let mut covered = 0;
        // 已登记的完整旧路径只影响其关联会话；未知的新来源仍会阻止归类。
        for (path, revision) in &self.missing {
            match store.checkpoint(path) {
                Ok(Some((saved_revision, state))) if saved_revision == *revision
                    && !state.session.is_empty()
                    && !state.resume_pending && !state.skipping
                    && !state.invalid_counts && !state.identity_conflict && state.issues == 0
                    && path.extension().is_some_and(|ext| ext == "jsonl")
                    && state.stamp.is_some_and(|(size, modified, identity)|
                        size == state.offset && modified.is_some() && identity.is_some()) =>
                {
                    affected.insert(path.clone());
                    covered += 1;
                }
                Ok(_) => {}
                Err(error) => {
                    issues += 1;
                    self.unknown_count_scope = true;
                    errors.push((path.clone(), error));
                }
            }
        }
        let unscoped_missing = self.more_missing || covered < self.missing.len();
        for path in self.remaining.iter().chain(self.active.iter().chain(&self.revisiting)
            .chain(&self.deferred).map(|reader| &reader.source)) {
            if self.sealed_queue.contains(path) {
                covered += 1;
            } else if let Some(&(stamp, affects_counts)) = self.legacy_prefixes.get(path) {
                match FileState::source_stamp(path) {
                    Ok(current) if current == stamp => {
                        covered += 1;
                        if affects_counts { rechecking.insert(path.clone()); }
                    }
                    Ok(_) => { self.legacy_prefixes.remove(path); }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        self.legacy_prefixes.remove(path);
                    }
                    Err(error) => {
                        self.unknown_count_scope = true;
                        issues += 1;
                        errors.push((path.clone(), error.into()));
                    }
                }
            }
        }
        // 普通追加的半行还不是冲突，只暂停该来源关联会话的重新归属。
        let mut unscoped_line = false;
        for (path, line) in &self.waiting_for_newline {
            if line.scoped {
                covered += 1;
                rechecking.insert(path.clone());
            } else {
                unscoped_line = true;
            }
        }
        let unknown = self.unknown_count_scope || unscoped_missing || unscoped_line
            || self.active.iter().chain(&self.revisiting).chain(&self.deferred)
                .any(|reader| reader.replacing || reader.state.issues != 0
                    || reader.state.invalid_counts || reader.state.identity_conflict);
        let can_publish_scope = !unknown && issues == self.scoped_count_gaps.len();
        // A queued source is not a conflict. Until it is read, new exact matches
        // require an independent response identity, not just matching counts.
        self.paired_only = pending_sources > covered + unpublished_append;
        let can_classify = can_publish_scope && unpublished_append == 0;
        let coverage = Coverage { pending_sources, issues, maintenance_blocked: maintenance_error.is_some() };
        let write_blocked = maintenance_error.as_ref().is_some_and(disk_full)
            || errors.iter().any(|(_, error)| disk_full(error));
        let recorded = if write_blocked {
            // 写入失败不是来源冲突，不发布据此推导的撤回。
            Ok(())
        } else if can_publish_scope {
            store.record_scoped_coverage(coverage, &affected, &rechecking)
        } else {
            store.record_coverage(coverage, unknown)
        };
        let coverage_error = recorded.err();
        issues += usize::from(coverage_error.is_some());
        let maintenance_error = coverage_error.or(maintenance_error);
        let write_blocked = write_blocked || maintenance_error.as_ref().is_some_and(disk_full);
        if write_blocked {
            // 下轮从已提交游标重新盘点，不能让临时错误滞留到旧队列耗尽。
            *self = Self::default();
        }
        Report {
            complete: !write_blocked && pending_sources == 0 && issues == 0 && maintenance_error.is_none(),
            can_classify: !write_blocked && can_classify && maintenance_error.is_none(),
            pending_sources,
            issues,
            errors,
            maintenance_error,
        }
    }
}

#[cfg(test)]
mod tests;
