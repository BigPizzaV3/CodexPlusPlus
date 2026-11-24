use super::{
    EventClassification, SourceStamp,
    store::{Store, projection::DayGroup},
};
use crate::usage_history::structured::{
    FileState, Index, LegacyEvidenceDocument, Snapshot, UsageEvent, supplier_evidence,
};
use anyhow::{Result, ensure};
use chrono::{Local, NaiveDate};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    settings: PathBuf,
    stamps: Stamps,
    revision: i64,
    pub(super) updated_at: i64,
    pub(super) changes: HashMap<[u8; 32], EventClassification>,
    pub(super) days: BTreeMap<NaiveDate, HashMap<String, Vec<DayGroup>>>,
    pub(super) provider_names: BTreeMap<String, String>,
    pub(super) legacy_status: Value,
    pub(super) supplier_status: Value,
}

impl Historical {
    pub(super) fn refresh(
        previous: Option<&Arc<Self>>,
        store: &mut Store,
        index: &mut Index,
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
            && prior.stamps == stamps_before
        {
            return Ok(Some(Arc::clone(prior)));
        }

        let legacy = LegacyEvidenceDocument::read(settings);
        let supplier = supplier_evidence::read_document(settings);
        let mut times = BTreeSet::new();
        if let Ok(Some(document)) = &legacy {
            document.timestamps(&mut times);
        }
        if let Ok(Some(document)) = &supplier {
            document.timestamps(&mut times);
        }
        let mut snapshot = Snapshot::default();
        snapshot.events = Store::historical_candidates(&read, &times)?;
        ensure!(
            stamps(settings)? == stamps_before,
            "historical evidence changed during read"
        );

        let before: Vec<_> = snapshot
            .events
            .iter()
            .map(EventClassification::of)
            .collect();
        index.apply_legacy_evidence(legacy, &mut snapshot);
        index
            .supplier_evidence
            .apply_document(supplier, &mut snapshot);
        let mut changes = HashMap::new();
        let mut affected = BTreeSet::new();
        for (event, old) in snapshot.events.into_iter().zip(before) {
            let next = EventClassification::of(&event);
            if next != old {
                changes.insert(event.id, next);
                affected.insert((event.date, event.session));
            }
        }
        let updated_at = if let Some(prior) = previous
            && prior.changes == changes
            && prior.provider_names == snapshot.provider_names
        {
            prior.updated_at
        } else {
            Local::now().timestamp_millis()
        };
        let days = Store::historical_days(&read, &affected, &changes)?;
        read.commit()?;
        Ok(Some(Arc::new(Self {
            settings: settings.to_owned(),
            stamps: stamps_before,
            revision,
            updated_at,
            changes,
            days,
            provider_names: snapshot.provider_names,
            legacy_status: snapshot.provider_evidence,
            supplier_status: snapshot.supplier_evidence,
        })))
    }

    pub(super) fn is_current(&self, revision: i64) -> Result<bool> {
        Ok(self.revision == revision && self.stamps == stamps(&self.settings)?)
    }

    pub(super) fn apply(&self, event: &mut UsageEvent) {
        if let Some(classification) = self.changes.get(&event.id) {
            classification.apply(event);
        }
    }
}
