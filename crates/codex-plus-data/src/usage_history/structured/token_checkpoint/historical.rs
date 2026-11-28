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
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const EVIDENCE_FILES: [&str; 2] = [
    "usage-provider-evidence.json",
    "usage-supplier-evidence-v2.json",
];

pub(super) type Stamps = [Option<SourceStamp>; EVIDENCE_FILES.len()];

pub(super) fn stamps(settings: &Path) -> Result<Stamps> {
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
    change_dates: HashMap<[u8; 32], NaiveDate>,
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
    legacy_document: Option<PreparedLegacy>,
    supplier: supplier_evidence::Cache,
}

#[derive(Clone)]
struct PreparedLegacy {
    stamp: Option<SourceStamp>,
    matches: Option<LegacyMatches>,
    timestamps: Arc<BTreeSet<i64>>,
}

impl EvidenceVerifier {
    fn prepare_legacy(&mut self, settings: &Path, before: Stamps) -> Result<PreparedLegacy> {
        if let Some(saved) = &self.legacy_document
            && saved.stamp == before[0]
        {
            return Ok(saved.clone());
        }
        let document = LegacyEvidenceDocument::read(settings)?;
        ensure!(stamps(settings)? == before, "historical evidence changed during read");

        let mut timestamps = BTreeSet::new();
        if let Some(document) = &document {
            document.timestamps(&mut timestamps);
        }
        let matches = self.legacy.prepare(Ok(document))?;
        let prepared = PreparedLegacy {
            stamp: before[0], matches, timestamps: Arc::new(timestamps),
        };
        self.legacy_document = Some(prepared.clone());
        Ok(prepared)
    }
}

impl EvidenceOverlay {
    fn from_archived_events(
        verifier: &Mutex<EvidenceVerifier>,
        settings: &Path,
        stamps_before: Stamps,
        events: Vec<UsageEvent>,
        legacy: Result<Option<LegacyMatches>>,
        supplier: Result<Option<supplier_evidence::Document>>,
    ) -> Result<Self> {
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
        LegacyMatches::apply_to_snapshot(legacy, &mut snapshot);
        verifier
            .supplier
            .apply_document(supplier, &mut snapshot);
        drop(verifier);
        let mut changes = HashMap::new();
        for (event, old) in snapshot.events.into_iter().zip(before) {
            let next = EventClassification::of(&event);
            if next != old {
                changes.insert(event.id, next);
            }
        }
        Ok(Self {
            settings: settings.to_owned(),
            stamps: stamps_before,
            updated_at: Local::now().timestamp_millis(),
            changes,
            provider_names: snapshot.provider_names,
            legacy_status: snapshot.provider_evidence,
            supplier_status: snapshot.supplier_evidence,
        })
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

        let saved = verifier.lock().map_err(|_| anyhow::anyhow!("historical verifier panicked"))?
            .prepare_legacy(settings, stamps_before);
        let supplier = supplier_evidence::read_document(settings);
        let mut times = saved.as_ref().map(|saved| Arc::clone(&saved.timestamps)).unwrap_or_default();
        if let Ok(Some(supplier)) = &supplier {
            supplier.timestamps(Arc::make_mut(&mut times));
        }
        let mut identities = EvidenceIdentities::default();
        if let Some(matches) = saved.as_ref().ok().and_then(|saved| saved.matches.as_ref())
            && !matches.records.is_empty()
        {
            Store::historical_identities(&read, &times, |id, alias| {
                if matches.records.contains_key(&id) {
                    identities.seen.insert(id);
                }
                if let Some(alias) = alias.filter(|id| matches.records.contains_key(id)) {
                    *identities.aliases.entry(alias).or_default() += 1;
                }
            })?;
        }
        let mut guard = verifier
            .lock()
            .map_err(|_| anyhow::anyhow!("historical verifier panicked"))?;
        ensure!(
            stamps(settings)? == stamps_before,
            "historical evidence changed during read"
        );
        let mut legacy = saved.map(|saved| saved.matches);
        let mut supplier = guard.supplier.prepare(supplier);
        let mut changes = HashMap::new();
        let mut affected = BTreeMap::<NaiveDate, HashSet<String>>::new();
        let mut change_dates = HashMap::new();
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
                change_dates.insert(event.id, event.date);
                affected.entry(event.date).or_default().insert(event.session);
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
        let days = Store::historical_days(&read, affected, &overlay)?;
        read.commit()?;
        Ok(Some(Arc::new(Self {
            revision,
            overlay,
            days,
            change_dates,
        })))
    }

    pub(super) fn minute_changes(&self, other: Option<&Self>, days: &mut BTreeSet<NaiveDate>) {
        if let Some(other) = other
            && std::ptr::eq(self, other)
        {
            return;
        }
        for (id, date) in &self.change_dates {
            if other.is_none_or(|other| {
                other.overlay.changes.get(id) != Some(&self.overlay.changes[id])
                    || other.change_dates.get(id) != Some(date)
            }) {
                days.insert(*date);
            }
        }
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
        let saved = verifier.lock().map_err(|_| anyhow::anyhow!("historical verifier panicked"))?
            .prepare_legacy(settings, before);
        let supplier = supplier_evidence::read_document(settings);
        let mut times = saved.as_ref().map(|saved| Arc::clone(&saved.timestamps)).unwrap_or_default();
        if let Ok(Some(supplier)) = &supplier {
            supplier.timestamps(Arc::make_mut(&mut times));
        }
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
        Self::from_archived_events(verifier, settings, before, selected,
            saved.map(|saved| saved.matches), supplier)
    }

    pub(super) fn is_current(&self) -> Result<bool> {
        Ok(self.stamps == stamps(&self.settings)?)
    }

    pub(super) fn apply(&self, event: &mut UsageEvent) -> bool {
        self.classification(&event.id).is_some_and(|classification| {
            classification.apply(event);
            true
        })
    }

    pub(super) fn classification(&self, id: &[u8; 32]) -> Option<&EventClassification> {
        self.changes.get(id)
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
    fn unchanged_legacy_document_reuses_prepared_index_until_file_is_removed() -> Result<()> {
        let root = tempfile::tempdir()?;
        let settings = root.path().join("settings.json");
        let file = settings.with_file_name(EVIDENCE_FILES[0]);
        std::fs::write(&file, json!({"version":1,"records":[{
            "eventId":"A2".repeat(32),"session":"fixture","timestamp":"2026-05-17T14:00:00Z",
            "model":"fixture","role":"main","input":7,"output":1,"cached":0,
            "provider":"fixture-supplier","providerName":"Fixture","source":"session-context-inferred",
        }]}).to_string())?;
        let mut verifier = EvidenceVerifier::default();
        let before = stamps(&settings)?;
        let first = verifier.prepare_legacy(&settings, before)?;
        let second = verifier.prepare_legacy(&settings, before)?;
        let first = first.matches.unwrap();
        let second = second.matches.unwrap();
        assert!(Arc::ptr_eq(&first.records, &second.records));
        assert!(second.records.contains_key(&[0xa2; 32]));
        std::fs::remove_file(&file)?;
        let removed = verifier.prepare_legacy(&settings, stamps(&settings)?)?;
        assert!(removed.matches.is_none() && removed.timestamps.is_empty());
        Ok(())
    }

    #[test]
    fn ambiguous_old_identity_is_checked_across_evidence_dates() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let settings = dir.path().join("settings.json");
        let mut store = Store::open(Connection::open_in_memory()?)?;
        store
            .db
            .execute("INSERT INTO token_sessions VALUES(1,'fixture')", [])?;
        let local_noon = |day| {
            NaiveDate::from_ymd_opt(2026, 10, day)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap()
                .and_local_timezone(Local)
                .single()
                .unwrap()
                .to_rfc3339()
        };
        let first = local_noon(30);
        let second = local_noon(31);
        let old = [9u8; 32];
        let legacy = |id: u8, time: &str| {
            json!({"eventId":format!("{id:02x}").repeat(32),
            "session":"fixture","timestamp":time,"model":"fixture","role":"main",
            "input":7,"output":1,"cached":0,"provider":"fixture-supplier",
            "providerName":"Fixture","source":"upstream-exact-bidirectional"})
        };
        let mut later = legacy(8, &second);
        later["providerName"] = json!("Later name");
        std::fs::write(
            dir.path().join("usage-provider-evidence.json"),
            serde_json::to_vec(&json!({"version":1,"records":[legacy(9,&first),later]}))?,
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
                "count_pending":false,"rollout_position":null,"provider":"inferred-supplier",
                "provider_source":"session-context-inferred","role":"main",
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
        insert(&store, 1, &first, Some(old))?;
        insert(&store, 8, &second, None)?;
        let migrated = Historical::refresh(None, &mut store, &verifier, &settings)?.unwrap();
        assert_eq!(migrated.overlay.legacy_status["migratedEvents"], 1);
        assert_eq!(migrated.overlay.legacy_status["appliedEvents"], 2);
        assert_eq!(migrated.overlay.provider_names["fixture-supplier"], "Fixture");
        for day in [&first, &second] {
            let groups = &migrated.days[&day[..10].parse::<NaiveDate>()?]["fixture"];
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].provider, "fixture-supplier");
            assert_eq!(groups[0].totals.records, 1);
            assert_eq!(groups[0].totals.counts.0.total_tokens, 8);
        }
        let at = chrono::DateTime::parse_from_rfc3339(&first)?.timestamp_millis();
        let request = json!({"startAt":at-1,"endAt":at+1,"includeFingerprints":false});
        let query = crate::usage_history::Query::parse(&request, Local::now().date_naive()).unwrap();
        let check_range = |store: &mut Store, history: &Historical, provider: &str| -> Result<()> {
            let fast = Store::read_response_with_archive(
                &mut store.db, &query, &request, None, &BTreeMap::new(), None, Some(history),
            )?.0;
            let mut exact = request.clone();
            exact["includeTrend"] = json!(true);
            let slow = Store::read_response_with_archive(
                &mut store.db, &query, &exact, None, &BTreeMap::new(), None, Some(history),
            )?.0;
            assert_eq!(fast["days"], slow["days"]);
            assert_eq!(fast["days"][0]["breakdowns"][0]["provider"], provider);
            Ok(())
        };
        check_range(&mut store, &migrated, "fixture-supplier")?;
        for (id, alias) in [(2, Some(old)), (9, None)] {
            insert(&store, id, &second, alias)?;
            let ambiguous = Historical::refresh(None, &mut store, &verifier, &settings)?.unwrap();
            assert_eq!(ambiguous.overlay.legacy_status["migratedEvents"], 0);
            assert_eq!(ambiguous.overlay.legacy_status["appliedEvents"], 1);
            assert_eq!(ambiguous.overlay.changes.len(), 1);
            assert!(ambiguous.overlay.changes.contains_key(&[8; 32]));
            assert_eq!(ambiguous.overlay.provider_names["fixture-supplier"], "Later name");
            assert_eq!(ambiguous.days.len(), 1);
            let groups = &ambiguous.days[&second[..10].parse::<NaiveDate>()?]["fixture"];
            assert_eq!(groups.len(), 2);
            assert_eq!(groups.iter().map(|g| g.totals.records).sum::<u64>(), 2);
            let mut dirty = BTreeSet::new();
            ambiguous.minute_changes(Some(&migrated), &mut dirty);
            migrated.minute_changes(Some(&ambiguous), &mut dirty);
            assert_eq!(dirty, BTreeSet::from([first[..10].parse()?]));
            store.db.execute(
                "DELETE FROM token_resolved WHERE event_id=?1",
                [[id; 32].as_slice()],
            )?;
        }
        std::fs::write(
            dir.path().join("usage-provider-evidence.json"),
            br#"{"version":1,"records":[]}"#,
        )?;
        let revoked =
            Historical::refresh(Some(&migrated), &mut store, &verifier, &settings)?.unwrap();
        assert!(revoked.overlay.changes.is_empty());
        assert!(revoked.days.is_empty());
        check_range(&mut store, &revoked, "inferred-supplier")?;
        Ok(())
    }
}
