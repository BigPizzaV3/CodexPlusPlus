use super::store::{
    Store,
    observation_encoding::Batch,
    projection::{ComponentInputLimit, SourceUpdate},
};
use crate::usage_history::structured::{
    FileState,
    index_budget::{Budget, CURSOR_BYTES, Meter},
};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

enum Encoding {
    Idle,
    Compacting(i64),
    Retry,
    FullComponent,
}

// 一份在读来源只保留当前批次。发生写入错误时下一轮重试同一批。
pub(super) struct SourceReader {
    pub(super) source: PathBuf,
    pub(super) state: FileState,
    revision: Option<i64>,
    offset: u64,
    committed_complete: bool,
    replacing: bool,
    ready: Option<bool>,
    encoding: Encoding,
    identity_probe: Option<Box<FileState>>,
}

impl SourceReader {
    pub(super) fn restore(store: &Store, source: &Path) -> Result<Self> {
        let (revision, mut state) = match store.checkpoint(source)? {
            Some((revision, state)) => (Some(revision), state),
            None => (None, FileState::default()),
        };
        state.executions = Some(Default::default());
        Ok(Self {
            source: source.to_path_buf(),
            offset: state.offset,
            committed_complete: revision.is_some() && !state.resume_pending,
            state,
            revision,
            replacing: false,
            ready: None,
            encoding: Encoding::Idle,
            identity_probe: None,
        })
    }

    pub(super) fn has_open_reader(&self) -> bool {
        self.state.reader.is_some() || self.identity_probe.is_some()
    }

    pub(super) fn states(&self) -> impl Iterator<Item = &FileState> {
        std::iter::once(&self.state).chain(self.identity_probe.as_deref())
    }

    pub(super) fn waiting_for_newline(&self) -> bool {
        self.ready.is_none()
            && !self.has_open_reader()
            && (self.state.committed_offset() < self.state.offset || self.state.skipping)
            && self.source.extension().is_some_and(|ext| ext == "jsonl")
            && self
                .state
                .stamp
                .is_some_and(|(size, _, _)| size == self.state.offset)
    }

    pub(super) fn can_yield_plain_batch(&self) -> bool {
        self.source.extension().is_some_and(|ext| ext == "jsonl")
            && self.revision.is_some()
            && !self.replacing
            && self.identity_probe.is_none()
            && self.ready.is_none()
            && matches!(self.encoding, Encoding::Idle)
            && !self.state.skipping
            && self.state.oversized.is_none()
            && self.offset == self.state.committed_offset()
    }

    pub(super) fn can_retry_write(&self) -> bool {
        self.ready.is_some() || self.identity_probe.as_ref().is_some_and(|probe| {
            probe.committed_offset() >= self.state.legacy_identity_prefix
        })
    }

    pub(super) fn retrying_closed_append(&self) -> bool {
        self.committed_complete
            && !self.replacing
            && self.ready == Some(true)
            && !self.has_count_gap()
    }

    pub(super) fn encoding_pending(&self) -> bool {
        !matches!(self.encoding, Encoding::Idle)
    }

    pub(super) fn closed_legacy_prefix(&self) -> Option<(super::SourceStamp, bool)> {
        let state = &self.state;
        let stamp @ (size, Some(_), Some(_)) = state.stamp? else { return None };
        // 历史扫描可能已提交整个新尾部，而旧前缀仍在分批读取。
        (state.legacy_identity_prefix != 0
            && self.committed_complete && self.ready.is_none() && !self.replacing
            && !state.resume_pending && !state.recheck_large_meter && !state.skipping
            && state.reader.is_none() && self.offset == state.committed_offset()
            && (self.source.extension().is_some_and(|ext| ext == "zst") || self.offset == size)
            && !state.session.is_empty() && !state.identity_conflict
            && state.issues == state.malformed_token_counts && (!state.invalid_counts || state.issues != 0))
            .then_some((stamp, !state.legacy_tools_only || state.issues != 0))
    }

    pub(super) fn has_count_gap(&self) -> bool {
        self.state.issues != 0 || self.state.invalid_counts || self.state.identity_conflict
            || self.state.legacy_identity_prefix != 0 && !self.state.legacy_tools_only
    }

    pub(super) fn has_only_malformed_token_counts(&self) -> bool {
        !self.state.session.is_empty()
            && self.state.legacy_identity_prefix == 0
            && !self.state.identity_conflict
            && self.state.issues != 0
            && self.state.issues == self.state.malformed_token_counts
    }

    fn check_prefix(
        &mut self,
        store: &mut Store,
        cutoff: NaiveDate,
        budget: &mut Budget,
    ) -> Result<bool> {
        if self.state.legacy_tools_only {
            let probe = self.identity_probe.get_or_insert_with(|| {
                Box::new(FileState {
                    replay_tool_activity: true,
                    executions: Some(Default::default()),
                    ..FileState::default()
                })
            });
            probe.executions.as_mut().unwrap().now_ms = chrono::Local::now().timestamp_millis();
            let (complete, _) = probe.read_source_batch(&self.source, cutoff, budget)?;
            if self.state.stamp != Some(FileState::source_stamp(&self.source)?) {
                self.identity_probe = None;
                return Ok(false);
            }
            if self.state.identity_conflict {
                probe.tools.clear();
                probe.executions.as_mut().unwrap().rows.clear();
            }
            if !probe.tools.is_empty() || !probe.executions.as_ref().unwrap().rows.is_empty() || complete {
                // 原始会话和计量已经核验。分批补回同一文件位置的工具记录。
                store.repair_tool_batch(&self.source, self.revision.unwrap(), probe, complete)?;
                probe.tools.clear();
                probe.executions.as_mut().unwrap().rows.clear();
            }
            if complete {
                self.state.legacy_identity_prefix = 0;
                self.state.legacy_tools_only = false;
                self.state.activity_truncated = probe.activity_truncated;
                self.state.calls = std::mem::take(&mut probe.calls);
                self.state.exec_skills = std::mem::take(&mut probe.exec_skills);
                self.state.exec_polls = std::mem::take(&mut probe.exec_polls);
                self.identity_probe = None;
            } else if probe.reader.is_some() {
                Meter::new(probe).refresh(probe, CURSOR_BYTES);
            }
            return Ok(complete);
        }
        let probe = self.identity_probe.get_or_insert_with(|| {
            Box::new(FileState { identity_probe: Some(false), ..FileState::default() })
        });
        // 只检查旧解析器已经提交的前缀，尾部的新数据仍由正常路径优先追加。
        let end = self.state.legacy_identity_prefix;
        if probe.identity_probe != Some(true) && probe.committed_offset() < end {
            let allowance = budget.read_left;
            budget.read_left = allowance.min(end.saturating_sub(probe.offset).max(1));
            let reserved = allowance - budget.read_left;
            let result = probe.read_source_batch(&self.source, cutoff, budget);
            budget.read_left += reserved;
            let (complete, _) = result?;
            probe.events.clear();
            probe.tools.clear();
            anyhow::ensure!(!complete || probe.committed_offset() >= end,
                "source ended before its saved identity-check prefix");
        }
        if probe.identity_probe == Some(true) {
            if budget.read_left == 0 || budget.source_left == 0 {
                return Ok(false);
            }
            self.state.recheck_large_meter = true;
            self.state.resume_pending = true;
            self.identity_probe = None;
        } else if probe.committed_offset() >= end {
            if self.state.stamp != Some(FileState::source_stamp(&self.source)?) {
                return Ok(false);
            }
            store.finish_identity_probe(&self.source, self.revision.unwrap())?;
            self.state.legacy_identity_prefix = 0;
            self.identity_probe = None;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    pub(super) fn step_in_history_slot(
        &mut self,
        store: &mut Store,
        cutoff: NaiveDate,
        budget: &mut Budget,
        provider_names: &BTreeMap<String, String>,
        reconciliation_limit: usize,
    ) -> Result<bool> {
        if self.state.legacy_identity_prefix == 0
            || !self.source.extension().is_some_and(|ext| ext == "jsonl")
        {
            return self.step(store, cutoff, budget, provider_names, reconciliation_limit);
        }
        let current = FileState::source_stamp(&self.source)?;
        if self.state.stamp == Some(current) && current.0 == self.offset {
            return self.step(store, cutoff, budget, provider_names, reconciliation_limit);
        }
        // 同一个文件持续追加时，历史名额同时服务新尾部和已经固定的旧前缀。
        let reserved = (budget.read_left / 2, budget.source_left / 2);
        budget.read_left -= reserved.0;
        budget.source_left -= reserved.1;
        let before = self.offset;
        let tail = self.step(store, cutoff, budget, provider_names, reconciliation_limit);
        budget.read_left += reserved.0;
        budget.source_left += reserved.1;
        let closed = tail?;
        if self.offset == before || self.state.legacy_identity_prefix == 0 || self.replacing
            || self.state.legacy_tools_only && !self.committed_complete
            || self.state.stamp != Some(FileState::source_stamp(&self.source)?)
        {
            return Ok(closed);
        }
        let checked = self.check_prefix(store, cutoff, budget)?;
        if self.state.recheck_large_meter {
            return self.step(store, cutoff, budget, provider_names, reconciliation_limit);
        }
        Ok(checked && self.committed_complete)
    }

    pub(super) fn step(
        &mut self,
        store: &mut Store,
        cutoff: NaiveDate,
        budget: &mut Budget,
        provider_names: &BTreeMap<String, String>,
        reconciliation_limit: usize,
    ) -> Result<bool> {
        if self.state.legacy_identity_prefix > 0 && self.ready.is_none()
            && self.state.reader.is_none() && !self.state.resume_pending
            && !self.replacing && self.state.stamp == Some(FileState::source_stamp(&self.source)?)
            && !self.check_prefix(store, cutoff, budget)?
        {
            return Ok(false);
        }
        if self.ready.is_some() {
            let stamp = match FileState::source_stamp(&self.source) {
                Ok(stamp) => stamp,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    *self = Self::restore(store, &self.source)?;
                    return Err(error.into());
                }
                Err(error) => return Err(error.into()),
            };
            let compressed = self.source.extension().is_some_and(|ext| ext == "zst");
            if self.state.source_replaced(stamp, compressed) {
                *self = Self::restore(store, &self.source)?;
            }
        }
        if let Encoding::Compacting(revision) = self.encoding {
            match store.compact_source_observations(
                &self.source,
                revision,
                &mut budget.read_left,
            )? {
                Batch::More => (),
                Batch::Finished { .. } => self.encoding = Encoding::Retry,
                Batch::SourceChanged => *self = Self::restore(store, &self.source)?,
            }
            if matches!(self.encoding, Encoding::Compacting(_) | Encoding::Retry) {
                return Ok(false);
            }
        }
        let complete = match self.ready {
            Some(complete) => complete,
            None => {
                self.state.executions.as_mut().unwrap().now_ms =
                    chrono::Local::now().timestamp_millis();
                let result = self.state.read_source_batch(&self.source, cutoff, budget);
                let (complete, replaced) = match result {
                    Ok(result) => result,
                    Err(error) => {
                        *self = Self::restore(store, &self.source).with_context(|| {
                            format!("restoring checkpoint after read failure: {error}")
                        })?;
                        return Err(error.into());
                    }
                };
                self.replacing |= replaced && self.revision.is_some();
                if replaced {
                    self.identity_probe = None;
                }
                if self.state.skipping || self.replacing && !complete {
                    return Ok(false);
                }
                let offset = self.state.committed_offset();
                if !self.replacing
                    && offset == self.offset
                    && (!complete || self.committed_complete)
                {
                    return Ok(complete);
                }
                self.ready = Some(complete);
                complete
            }
        };
        let prepared = match store.prepare(
            SourceUpdate {
                source: &self.source,
                state: &self.state,
                previous_revision: self.revision,
                replace: self.replacing,
            },
            cutoff,
            provider_names.clone(),
            if matches!(self.encoding, Encoding::FullComponent) {
                usize::MAX
            } else {
                reconciliation_limit
            },
        ) {
            Ok(prepared) => prepared,
            Err(error) if error.is::<ComponentInputLimit>()
                && !matches!(self.encoding, Encoding::FullComponent) => {
                if let Some(revision) = self.revision
                    && !self.replacing
                    && matches!(self.encoding, Encoding::Idle)
                    && store.has_encoding_work(&self.source, revision)?
                {
                    match store.compact_source_observations(
                        &self.source,
                        revision,
                        &mut budget.read_left,
                    )? {
                        Batch::More | Batch::SourceChanged => {
                            self.encoding = Encoding::Compacting(revision);
                            return Ok(false);
                        }
                        Batch::Finished { changed: true } => {
                            self.encoding = Encoding::Retry;
                            return Ok(false);
                        }
                        Batch::Finished { changed: false } => (),
                    }
                }
                // 整组必须一并核对。保留当前批次，下一轮按完整输入计算。
                self.encoding = Encoding::FullComponent;
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        self.revision = Some(store.publish(prepared)?);
        self.offset = self.state.committed_offset();
        self.committed_complete = complete;
        self.state.events = Vec::new();
        self.state.tools = Vec::new();
        self.state.executions.as_mut().unwrap().rows = Vec::new();
        if !complete && self.has_open_reader() {
            Meter::new(&self.state).refresh(&mut self.state, CURSOR_BYTES);
        }
        self.ready = None;
        self.replacing = false;
        self.encoding = Encoding::Idle;
        Ok(complete && self.identity_probe.is_none())
    }
}
