use super::{
    BTreeMap, BTreeSet, Connection, DayGroup, Groups, Historical, Local, NaiveDate, NaiveTime,
    Query, Result, Store, TimeZone, Value, compact_range, day_groups, json, params,
};

pub(in super::super::super) const MINUTE_MS: i64 = 60_000;

pub(in super::super::super) fn update(
    db: &Connection,
    first: NaiveDate,
    last: NaiveDate,
    historical: Option<&Historical>,
    previous: Option<Option<&Historical>>,
    revision: i64,
    publication: &str,
) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS token_minute_publication (
             singleton INTEGER PRIMARY KEY CHECK(singleton=1),
             revision INTEGER NOT NULL, classification INTEGER, last_day TEXT NOT NULL,
             snapshot TEXT NOT NULL
         ) STRICT;
         CREATE TABLE IF NOT EXISTS token_published_minutes (
             day TEXT NOT NULL, minute INTEGER NOT NULL, groups TEXT NOT NULL,
             PRIMARY KEY(day,minute)
         ) WITHOUT ROWID, STRICT;
         CREATE TABLE IF NOT EXISTS token_published_scope_minutes (
             day TEXT NOT NULL, session TEXT NOT NULL, minute INTEGER NOT NULL, groups TEXT NOT NULL,
             PRIMARY KEY(day,session,minute)
         ) WITHOUT ROWID, STRICT;
         CREATE TABLE IF NOT EXISTS token_minute_dirty (
             day TEXT PRIMARY KEY
         ) WITHOUT ROWID, STRICT;
         CREATE TRIGGER IF NOT EXISTS token_minute_insert AFTER INSERT ON token_range_projection BEGIN
             INSERT OR IGNORE INTO token_minute_dirty VALUES(NEW.day);
         END;
         CREATE TRIGGER IF NOT EXISTS token_minute_update AFTER UPDATE ON token_range_projection BEGIN
             INSERT OR IGNORE INTO token_minute_dirty VALUES(OLD.day),(NEW.day);
         END;
         CREATE TRIGGER IF NOT EXISTS token_minute_delete AFTER DELETE ON token_range_projection BEGIN
             INSERT OR IGNORE INTO token_minute_dirty VALUES(OLD.day);
         END;",
    )?;
    let days_sql = if previous.is_none() {
        "SELECT DISTINCT day FROM token_range_projection WHERE day >= ?1 AND day <= ?2"
    } else {
        "SELECT day FROM token_minute_dirty WHERE day >= ?1 AND day <= ?2"
    };
    let mut dates = db.prepare(days_sql)?;
    let mut dates = dates.query(params![first.to_string(), last.to_string()])?;
    let mut dirty = BTreeSet::new();
    while let Some(row) = dates.next()? {
        dirty.insert(row.get_ref(0)?.as_str()?.parse::<NaiveDate>()?);
    }
    if let Some(previous) = previous {
        if let Some(current) = historical {
            current.minute_changes(previous, &mut dirty);
        }
        if let Some(old) = previous {
            old.minute_changes(historical, &mut dirty);
        }
    } else {
        db.execute_batch(
            "DELETE FROM token_published_minutes;
             DELETE FROM token_published_scope_minutes;",
        )?;
    }
    let mut prune = db.prepare("DELETE FROM token_published_minutes WHERE day=?1")?;
    let mut insert = db.prepare("INSERT INTO token_published_minutes VALUES(?1,?2,?3)")?;
    let mut prune_scope = db.prepare("DELETE FROM token_published_scope_minutes WHERE day=?1")?;
    let mut insert_scope =
        db.prepare("INSERT INTO token_published_scope_minutes VALUES(?1,?2,?3,?4)")?;
    for day in dirty
        .into_iter()
        .filter(|day| *day >= first && *day <= last)
    {
        let name = day.to_string();
        prune.execute([&name])?;
        prune_scope.execute([&name])?;
        let midnight = |date: NaiveDate| {
            Local
                .from_local_datetime(&date.and_time(NaiveTime::MIN))
                .single()
                .map(|time| time.timestamp_millis())
        };
        let Some((start, end)) = midnight(day).zip(day.succ_opt().and_then(midnight)) else {
            anyhow::bail!("cannot publish minute range for ambiguous local midnight: {day}");
        };
        let query =
            Query::parse(&json!({"from":name,"to":name}), day).expect("published minute query");
        let mut buckets = BTreeMap::<usize, BTreeMap<String, Groups>>::new();
        compact_range::visit(
            db,
            start,
            end,
            &query,
            historical.map(|history| &history.overlay),
            Some(MINUTE_MS),
            compact_range::Sessions::All,
            |session, _, key, totals, bucket| {
                buckets
                    .entry(bucket.unwrap())
                    .or_default()
                    .entry(session.to_owned())
                    .or_default()
                    .entry(key.clone())
                    .or_default()
                    .merge(totals);
            },
        )?;
        for (bucket, sessions) in buckets {
            let minute = start + bucket as i64 * MINUTE_MS;
            let mut all = Groups::new();
            for (session, groups) in sessions {
                for (key, totals) in &groups {
                    all.entry(key.clone()).or_default().merge(*totals);
                }
                insert_scope.execute(params![
                    name,
                    session,
                    minute,
                    serde_json::to_string(&day_groups(groups))?
                ])?;
            }
            insert.execute(params![
                name,
                minute,
                serde_json::to_string(&day_groups(all))?
            ])?;
        }
    }
    db.execute(
        "DELETE FROM token_published_minutes WHERE day < ?1 OR day > ?2",
        params![first.to_string(), last.to_string()],
    )?;
    db.execute(
        "DELETE FROM token_published_scope_minutes WHERE day < ?1 OR day > ?2",
        params![first.to_string(), last.to_string()],
    )?;
    db.execute("DELETE FROM token_minute_dirty", [])?;
    db.execute(
        "REPLACE INTO token_minute_publication VALUES(1,?1,?2,?3,?4)",
        params![
            revision,
            historical.map(|h| h.overlay.updated_at),
            last.to_string(),
            publication
        ],
    )?;
    Ok(())
}

pub(super) fn matches(
    db: &Connection,
    revision: i64,
    historical: Option<&Historical>,
    last: NaiveDate,
    scoped: bool,
) -> Result<bool> {
    let exists = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='token_minute_publication')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists || scoped && !has_scope(db)? {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM token_minute_publication
                       WHERE revision=?1 AND classification IS ?2 AND last_day >= ?3)",
        params![
            revision,
            historical.map(|h| h.overlay.updated_at),
            last.to_string()
        ],
        |row| row.get(0),
    )?)
}

pub(super) fn has_scope(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='token_published_scope_minutes')",
        [],
        |row| row.get(0),
    )?)
}

pub(super) fn visit_scoped_day(
    db: &Connection,
    day: NaiveDate,
    start: i64,
    end: i64,
    members: &str,
    mut visit: impl FnMut(&str, DayGroup),
) -> Result<()> {
    let mut query = db.prepare_cached(
        "SELECT session,groups FROM token_published_scope_minutes
         WHERE day=?1 AND session IN (SELECT value FROM json_each(?2)) AND minute>=?3 AND minute<?4
         ORDER BY session,minute",
    )?;
    let mut rows = query.query(params![day.to_string(), members, start, end])?;
    while let Some(row) = rows.next()? {
        let session = row.get_ref(0)?.as_str()?;
        let groups: Vec<DayGroup> = serde_json::from_str(row.get_ref(1)?.as_str()?)?;
        for group in groups {
            visit(session, group);
        }
    }
    Ok(())
}

pub(super) fn visit_day(
    db: &Connection,
    day: NaiveDate,
    start: i64,
    end: i64,
    mut visit: impl FnMut(DayGroup),
) -> Result<()> {
    let mut query = db.prepare_cached(
        "SELECT groups FROM token_published_minutes
         WHERE day=?1 AND minute>=?2 AND minute<?3 ORDER BY minute",
    )?;
    let mut rows = query.query(params![day.to_string(), start, end])?;
    while let Some(row) = rows.next()? {
        let groups: Vec<DayGroup> = serde_json::from_str(row.get_ref(0)?.as_str()?)?;
        groups.into_iter().for_each(&mut visit);
    }
    Ok(())
}

pub(in super::super::super) fn append_day(
    db: &Connection,
    response: &mut Value,
    day: NaiveDate,
    start: i64,
    end: i64,
) -> Result<()> {
    let mut totals = Groups::new();
    visit_day(db, day, start, end, |group| {
        totals
            .entry((group.model, group.provider, group.role, group.conflicts))
            .or_default()
            .merge(group.totals);
    })?;
    if !totals.is_empty() {
        let (mut line, _) = Store::format_day(totals, None);
        line["date"] = json!(day.to_string());
        for row in line["breakdowns"].as_array_mut().unwrap() {
            row["providerName"] =
                response["providerNames"][row["provider"].as_str().unwrap()].clone();
        }
        response["days"].as_array_mut().unwrap().push(line);
    }
    Ok(())
}
