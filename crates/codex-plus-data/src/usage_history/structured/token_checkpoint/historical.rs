use super::{
    EventClassification, SourceStamp,
    store::{Store, projection::DayGroup},
};
use crate::usage_history::structured::{
    EvidenceIdentities, FileState, LegacyEvidenceCache, LegacyEvidenceDocument, LegacyMatches,
    Snapshot, UsageEvent, supplier_evidence,
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
        let times = files.timestamps();
        // Only legacy document IDs can be the target of an alias. The document bounds this set.
        let legacy_ids: std::collections::HashSet<[u8; 32]> = files
            .legacy
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .into_iter()
            .flat_map(|doc| &doc.records)
            .filter_map(|record| {
                if record.event_id.len() != 64 || !record.event_id.is_ascii() {
                    return None;
                }
                let mut id = [0; 32];
                for (i, byte) in id.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(&record.event_id[i * 2..i * 2 + 2], 16).ok()?;
                }
                Some(id)
            })
            .collect();
        let mut identities = EvidenceIdentities::default();
        Store::historical_candidates(&read, &times, |event| {
            // token_resolved's primary key already guarantees canonical ID uniqueness.
            if legacy_ids.contains(&event.id) {
                identities.seen.insert(event.id);
            }
            if let Some(alias) = event
                .legacy_identity
                .as_ref()
                .filter(|alias| legacy_ids.contains(&alias.id))
            {
                *identities.aliases.entry(alias.id).or_default() += 1;
            }
        })?;
        drop(legacy_ids);
        let mut guard = verifier
            .lock()
            .map_err(|_| anyhow::anyhow!("historical verifier panicked"))?;
        ensure!(
            stamps(settings)? == stamps_before,
            "historical evidence changed during read"
        );
        let mut legacy = guard.legacy.prepare(files.legacy);
        let mut supplier = guard.supplier.prepare(files.supplier);
        let mut changes = HashMap::new();
        let mut affected = BTreeSet::new();
        let mut legacy_names = BTreeMap::new();
        let mut supplier_names = BTreeMap::new();
        Store::historical_candidates(&read, &times, |mut event| {
            let prior = EventClassification::of(&event);
            if let Ok(Some(matches)) = &mut legacy {
                matches.apply(&mut event, &identities, &mut legacy_names);
            }
            if let Ok(Some(matches)) = &mut supplier {
                matches.apply(
                    &guard.supplier,
                    &mut event,
                    &identities.duplicates,
                    &mut supplier_names,
                );
            }
            let next = EventClassification::of(&event);
            if next != prior {
                changes.insert(event.id, next);
                affected.insert((event.date, event.session));
            }
        })?;
        drop(guard);
        for (provider, name) in supplier_names {
            legacy_names.entry(provider).or_insert(name);
        }
        let mut overlay = EvidenceOverlay {
            settings: settings.to_owned(),
            stamps: stamps_before,
            updated_at: Local::now().timestamp_millis(),
            changes,
            provider_names: legacy_names,
            legacy_status: LegacyMatches::report(legacy),
            supplier_status: supplier_evidence::Matches::report(supplier),
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};
    use serde_json::json;

    #[test]
    fn ambiguous_old_identity_is_checked_across_evidence_dates() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let settings = dir.path().join("settings.json");
        let mut store = Store::open(Connection::open_in_memory()?)?;
        store
            .db
            .execute("INSERT INTO token_sessions VALUES(1,'fixture')", [])?;
        let first = "2026-10-30T14:00:00Z";
        let second = "2026-10-31T14:00:00Z";
        let old = [9u8; 32];
        let legacy = |id: u8, time: &str| {
            json!({"eventId":format!("{id:02x}").repeat(32),
            "session":"fixture","timestamp":time,"model":"fixture","role":"main",
            "input":7,"output":1,"cached":0,"provider":"fixture-supplier",
            "providerName":"Fixture","source":"upstream-exact-bidirectional"})
        };
        std::fs::write(
            dir.path().join("usage-provider-evidence.json"),
            serde_json::to_vec(&json!({"version":1,"records":[legacy(9,first),legacy(8,second)]}))?,
        )?;
        let insert = |store: &Store,
                      id: u8,
                      time: &str,
                      alias: Option<[u8; 32]>|
         -> anyhow::Result<()> {
            let date = &time[..10];
            let counter = json!({"i":7,"c":0,"o":1,"t":8,"r":null,"k":true});
            let row = json!({"original_id":vec![id;32],"turn":"turn","model":"fixture",
                "timestamp":time,"date":date,"total":counter,"delta":counter,
                "last":null,"has_file_baseline":true,"count_uncertain":false,"count_incomplete":false,
                "count_pending":false,"rollout_position":null,"provider":"custom","role":"main",
                "inherited":false,"legacy_identity":alias.map(|id|json!({"id":id,"role":"main","session":"fixture"})),
                "context_conflicts":{"model":false,"provider":false,"role":false,"turn":false}});
            let at = chrono::DateTime::parse_from_rfc3339(time)?.timestamp_millis();
            store.db.execute(
                "INSERT INTO token_resolved VALUES(?1,1,?2,?3,?4)",
                params![[id; 32].as_slice(), at, date, serde_json::to_vec(&row)?],
            )?;
            Ok(())
        };
        let verifier = Mutex::new(EvidenceVerifier::default());
        insert(&store, 1, first, Some(old))?;
        let migrated = Historical::refresh(None, &mut store, &verifier, &settings)?.unwrap();
        assert_eq!(migrated.overlay.legacy_status["migratedEvents"], 1);
        for (id, alias) in [(2, Some(old)), (9, None)] {
            insert(&store, id, second, alias)?;
            let ambiguous = Historical::refresh(None, &mut store, &verifier, &settings)?.unwrap();
            assert!(ambiguous.overlay.changes.is_empty());
            assert!(ambiguous.days.is_empty());
            store.db.execute(
                "DELETE FROM token_resolved WHERE event_id=?1",
                [[id; 32].as_slice()],
            )?;
        }
        Ok(())
    }
}
