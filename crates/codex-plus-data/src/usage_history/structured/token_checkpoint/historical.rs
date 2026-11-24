use super::{
    EventClassification, SourceStamp,
    store::{Store, projection::DayGroup},
};
use crate::usage_history::structured::{
    FileState, LegacyEvidenceCache, LegacyEvidenceDocument, Snapshot, UsageEvent, supplier_evidence,
};
use anyhow::{Result, ensure};
use chrono::{Local, NaiveDate};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const EVIDENCE_FILES: [&str; 2] = [
    "usage-provider-evidence.json",
    "usage-supplier-evidence-v2.json",
];

type Stamps = [Option<SourceStamp>; EVIDENCE_FILES.len()];

fn stamps(settings: &Path) -> Result<Stamps> {
    let mut values = [None; EVIDENCE_FILES.len()];
    for (slot, name) in values.iter_mut().zip(EVIDENCE_FILES) {
        match FileState::source_stamp(&settings.with_file_name(name)) {
            Ok(stamp) => *slot = Some(stamp),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(values)
}

pub(super) struct Historical {
    pub(super) overlay: EvidenceOverlay,
    revision: i64,
    pub(super) days: BTreeMap<NaiveDate, HashMap<String, Vec<DayGroup>>>,
}

pub(super) struct EvidenceOverlay {
    settings: PathBuf,
    stamps: Stamps,
    pub(super) updated_at: i64,
    changes: HashMap<[u8; 32], EventClassification>,
    pub(super) provider_names: BTreeMap<String, String>,
    pub(super) legacy_status: Value,
    pub(super) supplier_status: Value,
}

#[derive(Default)]
pub(super) struct EvidenceVerifier {
    legacy: LegacyEvidenceCache,
    supplier: supplier_evidence::Cache,
}

struct EvidenceFiles {
    legacy: Result<Option<LegacyEvidenceDocument>>,
    supplier: Result<Option<supplier_evidence::Document>>,
}

impl EvidenceFiles {
    fn read(settings: &Path) -> Self {
        Self {
            legacy: LegacyEvidenceDocument::read(settings),
            supplier: supplier_evidence::read_document(settings),
        }
    }

    fn timestamps(&self) -> BTreeSet<i64> {
        let mut times = BTreeSet::new();
        if let Ok(Some(document)) = &self.legacy {
            document.timestamps(&mut times);
        }
        if let Ok(Some(document)) = &self.supplier {
            document.timestamps(&mut times);
        }
        times
    }

    fn apply(
        self,
        verifier: &Mutex<EvidenceVerifier>,
        settings: &Path,
        stamps_before: Stamps,
        events: Vec<UsageEvent>,
    ) -> Result<(EvidenceOverlay, BTreeSet<(NaiveDate, String)>)> {
        let before: Vec<_> = events.iter().map(EventClassification::of).collect();
        let mut verifier = verifier
            .lock()
            .map_err(|_| anyhow::anyhow!("historical verifier panicked"))?;
        ensure!(
            stamps(settings)? == stamps_before,
            "historical evidence changed during read"
        );
        let mut snapshot = Snapshot {
            events,
            ..Snapshot::default()
        };
        verifier.legacy.apply(self.legacy, &mut snapshot);
        verifier
            .supplier
            .apply_document(self.supplier, &mut snapshot);
        drop(verifier);
        let mut changes = HashMap::new();
        let mut affected = BTreeSet::new();
        for (event, old) in snapshot.events.into_iter().zip(before) {
            let next = EventClassification::of(&event);
            if next != old {
                changes.insert(event.id, next);
                affected.insert((event.date, event.session));
            }
        }
        Ok((
            EvidenceOverlay {
                settings: settings.to_owned(),
                stamps: stamps_before,
                updated_at: Local::now().timestamp_millis(),
                changes,
                provider_names: snapshot.provider_names,
                legacy_status: snapshot.provider_evidence,
                supplier_status: snapshot.supplier_evidence,
            },
            affected,
        ))
    }
}

impl Historical {
    pub(super) fn refresh(
        previous: Option<&Arc<Self>>,
        store: &mut Store,
        verifier: &Mutex<EvidenceVerifier>,
        settings: &Path,
    ) -> Result<Option<Arc<Self>>> {
        let stamps_before = stamps(settings)?;
        if stamps_before.iter().all(Option::is_none) {
            return Ok(None);
        }
        let read = store.db.transaction()?;
        let revision = read.query_row("SELECT revision FROM token_revision", [], |r| r.get(0))?;
        if let Some(prior) = previous
            && prior.revision == revision
            && prior.overlay.stamps == stamps_before
        {
            return Ok(Some(Arc::clone(prior)));
        }

        let files = EvidenceFiles::read(settings);
        let events = Store::historical_candidates(&read, &files.timestamps())?;
        let (mut overlay, affected) = files.apply(verifier, settings, stamps_before, events)?;
        if let Some(prior) = previous
            && prior.overlay.changes == overlay.changes
            && prior.overlay.provider_names == overlay.provider_names
        {
            overlay.updated_at = prior.overlay.updated_at;
        }
        let days = Store::historical_days(&read, &affected, &overlay.changes)?;
        read.commit()?;
        Ok(Some(Arc::new(Self {
            revision,
            overlay,
            days,
        })))
    }

    pub(super) fn is_current(&self, revision: i64) -> Result<bool> {
        Ok(self.revision == revision && self.overlay.is_current()?)
    }
}

impl EvidenceOverlay {
    pub(super) fn for_archive(
        verifier: &Mutex<EvidenceVerifier>,
        settings: &Path,
        events: &[UsageEvent],
        explicit_session: bool,
    ) -> Result<Self> {
        let before = stamps(settings)?;
        let files = EvidenceFiles::read(settings);
        let times = files.timestamps();
        let mut selected = Vec::new();
        for event in events
            .iter()
            .take(if times.is_empty() { 0 } else { events.len() })
        {
            let at = chrono::DateTime::parse_from_rfc3339(&event.timestamp)?.timestamp_millis();
            if times.contains(&at) {
                let mut event = event.clone();
                // 单会话回读不能证明旧别名在其他会话中没有竞争。
                if explicit_session {
                    event.legacy_identity = None;
                }
                selected.push(event);
            }
        }
        files
            .apply(verifier, settings, before, selected)
            .map(|(overlay, _)| overlay)
    }

    pub(super) fn is_current(&self) -> Result<bool> {
        Ok(self.stamps == stamps(&self.settings)?)
    }

    pub(super) fn apply(&self, event: &mut UsageEvent) -> bool {
        self.changes.get(&event.id).is_some_and(|classification| {
            classification.apply(event);
            true
        })
    }

    pub(super) fn contains(&self, event: &UsageEvent) -> bool {
        self.changes.contains_key(&event.id)
    }
}
