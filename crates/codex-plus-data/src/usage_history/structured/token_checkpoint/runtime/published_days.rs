use super::*;
use crate::usage_history::structured::token_checkpoint::historical::{Stamps, stamps};
use crate::usage_history::structured::token_checkpoint::store::projection::{
    published_minutes, published_scope,
};
use chrono::NaiveTime;
use serde::Serialize;
use std::collections::BTreeSet;
use std::io::{BufWriter, Write};

const FILE: &str = "published-usage-days-v1.json";

#[derive(Serialize, Deserialize)]
struct PublishedDays {
    first: String,
    last: String,
    evidence: Stamps,
    response: Value,
}

#[derive(Eq, PartialEq)]
pub(super) struct Publication {
    revision: i64,
    classification: Option<i64>,
    pending: i64,
    issues: i64,
    blocked: bool,
    last: NaiveDate,
    evidence: Stamps,
}

pub(super) fn save(
    store: &Store,
    storage: &Path,
    settings: &Path,
    state: &ReadState,
    previous: &mut Option<(Publication, Option<Arc<Historical>>)>,
) -> Result<()> {
    let (revision, first, pending, issues, blocked): (i64, String, i64, i64, bool) =
        store.db.query_row(
            "SELECT r.revision,r.precise_from,c.pending_sources,c.issues,c.maintenance_blocked
             FROM token_revision r JOIN token_coverage c USING(revision)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
    if first.is_empty() {
        return Ok(());
    }
    let evidence = stamps(settings)?;
    let historical = state.historical.as_deref();
    if historical.is_none() && evidence.iter().any(Option::is_some) {
        return Ok(());
    }
    let last = Local::now().date_naive();
    let publication = Publication {
        revision,
        classification: historical.map(|h| h.overlay.updated_at),
        pending,
        issues,
        blocked,
        last,
        evidence,
    };
    if previous
        .as_ref()
        .is_some_and(|(prior, _)| prior == &publication)
    {
        return Ok(());
    }
    let first = first.parse::<NaiveDate>()?;
    let midnight = |date: NaiveDate| {
        date.and_time(NaiveTime::MIN)
            .and_local_timezone(Local)
            .single()
            .map(|time| time.timestamp_millis())
    };
    let Some((start, end)) = midnight(first).zip(last.succ_opt().and_then(midnight)) else {
        return Ok(());
    };
    let request = json!({"from":first.to_string(),"to":last.to_string(),"startAt":start,"endAt":end,
        "includeFingerprints":false,"includeSessionMetadata":false});
    let (mut response, _) = Store::read_snapshot(
        &store.db,
        &Query::parse(&request, last).expect("published days query"),
        &request,
        None,
        &state.provider_names,
        &[],
        historical,
    )?;
    if response["scan"]["ready"] != true
        || response.get("historicalClassificationError").is_some()
        || publication.classification.is_some() && response["classificationRevision"][0].is_null()
        || publication.evidence != stamps(settings)?
    {
        return Ok(());
    }
    response["scan"]["retentionDays"] = json!(state.retention_days);
    let snapshot = PublishedDays {
        first: first.to_string(),
        last: last.to_string(),
        evidence: publication.evidence,
        response,
    };
    // Store both outputs of the same classified revision in one transaction.
    let tx = store.db.unchecked_transaction()?;
    published_minutes::update(
        &tx,
        first,
        last,
        historical,
        previous.as_ref().map(|(_, history)| history.as_deref()),
        revision,
        &serde_json::to_string(&snapshot)?,
    )?;
    published_scope::save(&tx, first, last, historical)?;
    ensure!(
        publication.evidence == stamps(settings)?,
        "published minute evidence changed during save"
    );
    tx.commit()?;
    *previous = Some((publication, state.historical.clone()));
    codex_plus_core::settings::atomic_write_with(&storage.join(FILE), |out| {
        let mut out = BufWriter::new(out);
        serde_json::to_writer(&mut out, &snapshot)?;
        out.flush()
    })?;
    Ok(())
}

impl ReadState {
    pub(in crate::usage_history::structured) fn read_published_days(
        &self,
        storage: &Path,
        settings: &Path,
        query: &Query,
        request: &Value,
        scope_session: Option<&str>,
    ) -> Option<Value> {
        let scoped_only = scope_session.filter(|_| request["scopeOnly"] == true);
        if (scoped_only.is_none()
            && (request["includeSessionMetadata"] != false || scope_session.is_some()))
            || request["includeFingerprints"] != false
            || request["includeTrend"] == true
            || request["liveReceipts"].is_array()
            || query.session_id.is_some()
            || query.turn_id.is_some()
            || query.model.is_some()
            || !query.search.is_empty()
            || query.include_activity
            || query.include_session_cursors
        {
            return None;
        }
        let local =
            |ms: i64| chrono::DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&Local));
        let start = local(request["startAt"].as_i64()?)?;
        let end = local(request["endAt"].as_i64()?)?;
        let clipped_start = start.time() != NaiveTime::MIN;
        let clipped_end = end.time() != NaiveTime::MIN;
        let from = start.date_naive().max(query.from.unwrap_or(NaiveDate::MIN));
        let to = local(end.timestamp_millis().checked_sub(1)?)?
            .date_naive()
            .min(query.to.unwrap_or(NaiveDate::MAX));
        let mut boundary_days = BTreeSet::new();
        if clipped_start && start.date_naive() >= from && start.date_naive() <= to {
            boundary_days.insert(start.date_naive().to_string());
        }
        if clipped_end && end.date_naive() >= from && end.date_naive() <= to {
            boundary_days.insert(end.date_naive().to_string());
        }
        let mut db = if boundary_days.is_empty() && scoped_only.is_none() {
            None
        } else {
            if [start.timestamp_millis(), end.timestamp_millis()]
                .into_iter()
                .any(|ms| ms.rem_euclid(published_minutes::MINUTE_MS) != 0)
            {
                return None;
            }
            match open_database(storage, false) {
                Ok(db) => Some(db),
                Err(error) => {
                    eprintln!("opening published usage minutes: {error:#}");
                    return None;
                }
            }
        };
        let tx = if let Some(db) = &mut db {
            match db.transaction() {
                Ok(tx) => Some(tx),
                Err(error) => {
                    eprintln!("reading published usage minutes: {error:#}");
                    return None;
                }
            }
        } else {
            None
        };
        let snapshot: PublishedDays = if let Some(tx) = &tx {
            match (|| -> Result<Option<_>> {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='token_minute_publication')",
                    [], |row| row.get(0))?;
                if !exists {
                    return Ok(None);
                }
                let snapshot: String = tx.query_row(
                    "SELECT snapshot FROM token_minute_publication WHERE singleton=1",
                    [],
                    |row| row.get(0),
                )?;
                Ok(Some(serde_json::from_str(&snapshot)?))
            })() {
                Ok(snapshot) => snapshot?,
                Err(error) => {
                    eprintln!("reading published usage minutes: {error:#}");
                    return None;
                }
            }
        } else {
            match fs::read(storage.join(FILE)) {
                Ok(input) => match serde_json::from_slice(&input) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        eprintln!("reading published usage days: {error}");
                        return None;
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
                Err(error) => {
                    eprintln!("opening published usage days: {error}");
                    return None;
                }
            }
        };
        let from_key = from.to_string();
        let to_key = to.to_string();
        if from_key < snapshot.first
            || to.min(Local::now().date_naive()).to_string() > snapshot.last
            || from > to
        {
            return None;
        }
        match stamps(settings) {
            Ok(current) if current == snapshot.evidence => {}
            Ok(_) => return None,
            Err(error) => {
                eprintln!("checking published usage evidence: {error:#}");
                return None;
            }
        }
        let mut response = snapshot.response;
        let within = |day: &str| from_key.as_str() <= day && day <= to_key.as_str();
        response["days"]
            .as_array_mut()
            .expect("published days array")
            .retain(|day| {
                let name = day["date"].as_str().expect("published day date");
                within(name) && !boundary_days.contains(name)
            });
        response["coverage"]["days"]
            .as_object_mut()
            .expect("published coverage map")
            .retain(|day, _| within(day));
        if let Some(tx) = tx {
            if let Some(scope_session) = scoped_only {
                match published_scope::read(
                    &tx,
                    &mut response,
                    scope_session,
                    query,
                    (from, to),
                    (start.timestamp_millis(), end.timestamp_millis()),
                    &boundary_days,
                ) {
                    Ok(true) => {}
                    Ok(false) => return None,
                    Err(error) => {
                        eprintln!("reading published session usage: {error:#}");
                        return None;
                    }
                }
                boundary_days.clear();
            }
            for day in boundary_days {
                if let Err(error) = published_minutes::append_day(
                    &tx,
                    &mut response,
                    day.parse().expect("formatted boundary date"),
                    start.timestamp_millis(),
                    end.timestamp_millis(),
                ) {
                    eprintln!("reading published usage minutes: {error:#}");
                    return None;
                }
            }
            response["days"]
                .as_array_mut()
                .unwrap()
                .sort_unstable_by(|a, b| a["date"].as_str().cmp(&b["date"].as_str()));
            match stamps(settings) {
                Ok(current) if current == snapshot.evidence => {}
                Ok(_) => return None,
                Err(error) => {
                    eprintln!("checking published usage evidence: {error:#}");
                    return None;
                }
            }
        }
        response["status"] = json!("partial");
        response["complete"] = json!(false);
        response["countStatus"] = json!({"complete":false});
        response["classificationStatus"] = json!({"complete":false});
        response["scan"]["processing"] = json!(true);
        response["scan"]["lastPublished"] = json!(true);
        response["liveAttributions"] = json!([]);
        Some(response)
    }
}
