use super::store::{
    Store,
    observation_encoding::Batch,
    projection::{ComponentInputLimit, SourceUpdate},
};
use crate::usage_history::structured::{FileState, index_budget::Budget};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

enum Encoding {
    Idle,
    Compacting(i64),
    Retry,
    Exhausted,
}

// 一份在读来源只保留当前批次。发生写入错误时下一轮重试同一批。
pub(super) struct SourceReader {
    pub(super) source: PathBuf,
    state: FileState,
    revision: Option<i64>,
    offset: u64,
    committed_complete: bool,
    replacing: bool,
    ready: Option<bool>,
    encoding: Encoding,
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
        })
    }

    pub(super) fn has_open_reader(&self) -> bool {
        self.state.reader.is_some()
    }

    pub(super) fn scanned_stamp(&self) -> Option<(u64, std::time::SystemTime)> {
        let (size, modified, _) = self.state.stamp?;
        Some((size, modified?))
    }

    pub(super) fn can_retry_write(&self) -> bool {
        self.ready.is_some() && !matches!(self.encoding, Encoding::Exhausted)
    }

    pub(super) fn retrying_closed_append(&self) -> bool {
        self.committed_complete
            && !self.replacing
            && self.ready == Some(true)
            && !self.has_count_gap()
    }

    pub(super) fn encoding_pending(&self) -> bool {
        matches!(self.encoding, Encoding::Compacting(_) | Encoding::Retry)
    }

    pub(super) fn has_count_gap(&self) -> bool {
        self.state.issues != 0 || self.state.invalid_counts || self.state.identity_conflict
    }

    pub(super) fn has_only_malformed_token_counts(&self) -> bool {
        !self.state.session.is_empty()
            && !self.state.identity_conflict
            && self.state.issues != 0
            && self.state.issues == self.state.malformed_token_counts
    }

    pub(super) fn step(
        &mut self,
        store: &mut Store,
        cutoff: NaiveDate,
        budget: &mut Budget,
        provider_names: &BTreeMap<String, String>,
        reconciliation_limit: usize,
    ) -> Result<bool> {
        if self.ready.is_some() {
            let stamp = FileState::source_stamp(&self.source)?;
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
                if self.state.skipping || self.replacing && !complete {
                    return Ok(false);
                }
                let offset = self.state.offset - self.state.pending.len() as u64;
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
            reconciliation_limit,
        ) {
            Ok(prepared) => prepared,
            Err(error) if error.is::<ComponentInputLimit>() => {
                if let Some(revision) = self.revision
                    && !self.replacing
                {
                    if matches!(self.encoding, Encoding::Idle)
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
                    self.encoding = Encoding::Exhausted;
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        self.revision = Some(store.publish(prepared)?);
        self.offset = self.state.offset - self.state.pending.len() as u64;
        self.committed_complete = complete;
        self.state.events = Vec::new();
        self.state.tools = Vec::new();
        self.state.executions.as_mut().unwrap().rows = Vec::new();
        self.ready = None;
        self.replacing = false;
        self.encoding = Encoding::Idle;
        Ok(complete)
    }
}
