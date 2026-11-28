use super::*;

#[derive(Deserialize, Serialize)]
struct Membership {
    sessions: HashSet<String>,
    aliases: BTreeMap<String, String>,
    scope_parents: BTreeMap<String, String>,
}

pub(in super::super::super) fn save(
    db: &Connection,
    first: NaiveDate,
    last: NaiveDate,
    historical: Option<&Historical>,
) -> Result<()> {
    let scope = Store::scope(db, &BTreeMap::new(), false)?;
    let membership = Membership {
        sessions: scope.sessions,
        aliases: scope.aliases,
        scope_parents: scope.scope_parents,
    };
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS token_published_scope (
             singleton INTEGER PRIMARY KEY CHECK(singleton=1), membership TEXT NOT NULL
         ) STRICT;
         CREATE TABLE IF NOT EXISTS token_published_scope_days (
             session TEXT NOT NULL, day TEXT NOT NULL, groups BLOB NOT NULL,
             PRIMARY KEY(session,day)
         ) WITHOUT ROWID, STRICT;
         DELETE FROM token_published_scope_days;",
    )?;
    db.execute(
        "INSERT INTO token_published_scope_days
         SELECT s.session,d.day,d.groups FROM token_days d JOIN token_sessions s ON s.id=d.session_id
         WHERE d.day BETWEEN ?1 AND ?2",
        params![first.to_string(), last.to_string()],
    )?;
    if let Some(historical) = historical {
        let mut insert = db.prepare("REPLACE INTO token_published_scope_days VALUES(?1,?2,?3)")?;
        for (day, sessions) in historical.days.range(first..=last) {
            let day = day.to_string();
            for (session, groups) in sessions {
                insert.execute(params![session, day, serde_json::to_vec(groups)?])?;
            }
        }
    }
    db.execute(
        "REPLACE INTO token_published_scope VALUES(1,?1)",
        [serde_json::to_string(&membership)?],
    )?;
    Ok(())
}

pub(in super::super::super) fn read(
    db: &Connection,
    response: &mut Value,
    requested: &str,
    query: &Query,
    dates: (NaiveDate, NaiveDate),
    bounds: (i64, i64),
    clipped: &BTreeSet<String>,
) -> Result<bool> {
    let Some(scope) = membership(db, None)? else {
        return Ok(false);
    };
    if !clipped.is_empty() && !published_minutes::has_scope(db)? {
        return Ok(false);
    }
    let (canonical, sessions) = scope.scope_members(requested);
    let members = serde_json::to_string(&sessions)?;
    let mut statement = db.prepare(
        "SELECT day,groups FROM token_published_scope_days
         WHERE session IN (SELECT value FROM json_each(?1)) AND day BETWEEN ?2 AND ?3
         AND day NOT IN (SELECT value FROM json_each(?4))
         ORDER BY day,session",
    )?;
    let (from, to) = dates;
    let mut rows = statement.query(params![
        members,
        from.to_string(),
        to.to_string(),
        serde_json::to_string(clipped)?,
    ])?;
    let mut totals = BTreeMap::<NaiveDate, Groups>::new();
    while let Some(row) = rows.next()? {
        let groups = totals
            .entry(row.get_ref(0)?.as_str()?.parse()?)
            .or_default();
        for group in serde_json::from_slice::<Vec<DayGroup>>(row.get_ref(1)?.as_blob()?)? {
            groups
                .entry((group.model, group.provider, group.role, group.conflicts))
                .or_default()
                .merge(group.totals);
        }
    }
    for day in clipped {
        let day = day.parse()?;
        published_minutes::visit_scoped_day(db, day, bounds.0, bounds.1, &members, |_, group| {
            totals
                .entry(day)
                .or_default()
                .entry((group.model, group.provider, group.role, group.conflicts))
                .or_default()
                .merge(group.totals);
        })?;
    }
    let revision = response["revision"].as_i64().expect("published revision");
    let precise_from = response["scan"]["preciseFrom"]
        .as_str()
        .expect("published retention cutoff");
    let mut scoped = Store::format_selection(revision, None, precise_from, false, totals, None);
    for day in &mut scoped.days {
        for row in day["breakdowns"].as_array_mut().unwrap() {
            row["providerName"] =
                response["providerNames"][row["provider"].as_str().unwrap()].clone();
        }
    }
    response["sessionAliases"] = json!({requested:canonical});
    response["days"] = json!([]);
    response["scopedUsage"] = json!({
        "sessionId":canonical,
        "sessionIds":ordered_scope_ids(canonical, &sessions),
        "coverage":scoped.range_coverage(query, Some(bounds), false)?,
        "countStatus":{"complete":false,"uncertainEvents":scoped.unresolved},
        "classificationStatus":{"complete":false,"uncertainEvents":scoped.unclassified},
        "complete":false,
        "days":scoped.days,
    });
    Ok(true)
}

pub(super) fn membership(db: &Connection, revision: Option<i64>) -> Result<Option<Snapshot>> {
    let exists = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='token_published_scope')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists {
        return Ok(None);
    }
    let mut query = db.prepare_cached(
        "SELECT s.membership FROM token_published_scope s JOIN token_minute_publication p USING(singleton)
         WHERE ?1 IS NULL OR p.revision=?1",
    )?;
    let mut rows = query.query([revision])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let membership: Membership = serde_json::from_str(row.get_ref(0)?.as_str()?)?;
    Ok(Some(Snapshot {
        sessions: membership.sessions,
        aliases: membership.aliases,
        scope_parents: membership.scope_parents,
        ..Snapshot::default()
    }))
}
