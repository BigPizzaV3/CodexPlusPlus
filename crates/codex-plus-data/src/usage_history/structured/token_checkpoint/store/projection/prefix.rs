use super::*;

pub(super) struct ParentPrefix {
    pub(super) link: String,
    pub(super) path: PathBuf,
    pub(super) state: FileState,
    pub(super) bytes: usize,
}

impl ParentPrefix {
    pub(super) fn read(
        db: &Connection,
        update: SourceUpdate<'_>,
        cutoff: NaiveDate,
        limit: usize,
    ) -> Result<Option<Self>> {
        let state = update.state;
        let Some(base) = &state.history_base else {
            return Ok(None);
        };
        let Some(physical) = segments::rollout_id(update.source) else {
            return Ok(None);
        };
        if update.replace
            || !state.continuation
            || state.session.is_empty()
            || physical == base.rollout
            || state.invalid_counts
            || state.identity_conflict
            || state.rollout_out_of_order
            || state.issues != 0
            || state.events.iter().any(|e| e.session != state.session)
            || state
                .rollout_extent
                .is_none_or(|(start, _, _)| start != base.end_ordinal)
        {
            return Ok(None);
        }
        let link = format!("r:{}", base.rollout);
        let own = format!("r:{physical}");
        let mut sources = db.prepare_cached(
            "SELECT l.link, s.id, s.source, EXISTS(SELECT 1 FROM token_source_copies WHERE owner_id = s.id)
             FROM token_source_links l JOIN token_sources s ON s.id = l.source_id
             WHERE l.link IN (?1, ?2)",
        )?;
        let mut rows = sources.query(params![link, own])?;
        let mut parent = None;
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let name = row.get_ref(2)?.as_blob()?;
            bytes += name.len();
            if bytes > limit {
                return Ok(None);
            }
            let path = decode_path(name.to_vec())?;
            let id = segments::rollout_id(&path);
            if id == Some(physical) && (path != update.source || row.get::<_, bool>(3)?) {
                return Ok(None);
            }
            if id == Some(base.rollout) && row.get_ref(0)?.as_str()? == link {
                if parent.is_some() || row.get::<_, bool>(3)? {
                    return Ok(None);
                }
                parent = Some((row.get::<_, i64>(1)?, path));
            }
        }
        let Some((id, path)) = parent else {
            return Ok(None);
        };
        let (offset, checkpoint): (u64, Vec<u8>) = db.query_row(
            "SELECT offset, checkpoint FROM token_sources WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        bytes += checkpoint.len();
        if bytes > limit {
            return Ok(None);
        }
        let mut parent = serde_json::from_slice::<Checkpoint>(&checkpoint)?.restore(offset);
        let Some((start, last, end)) = parent.rollout_extent else {
            return Ok(None);
        };
        if parent.session.is_empty()
            || parent.session == state.session
            || parent.history_base.is_some()
            || parent.continuation
            || parent.identity_conflict
            || parent.invalid_counts
            || parent.rollout_out_of_order
            || parent.issues != 0
            || start != 0
            || end != offset
            || last.saturating_add(1) < base.end_ordinal
            || end < base.end_offset
            || last.saturating_add(1) == base.end_ordinal && end != base.end_offset
        {
            return Ok(None);
        }
        let mut tail = db.prepare_cached(
            "SELECT o.event_id, s.session, o.payload FROM token_observations o
             JOIN token_sessions s ON s.id = o.session_id WHERE o.source_id = ?1
             ORDER BY o.revision DESC, o.position DESC",
        )?;
        let mut rows = tail.query([id])?;
        while let Some(row) = rows.next()? {
            let data = row.get_ref(2)?.as_blob()?;
            bytes += data.len();
            if bytes > limit {
                return Ok(None);
            }
            let raw: Observation = serde_json::from_slice(data)?;
            let Some(position) = raw.rollout_position else {
                return Ok(None);
            };
            if position.0 >= base.end_ordinal {
                if position.1 < base.end_offset {
                    return Ok(None);
                }
                continue;
            }
            if position.2 > base.end_offset {
                return Ok(None);
            }
            if raw.terminal_response.is_some() {
                continue;
            }
            if raw.inherited
                || NaiveDate::parse_from_str(&raw.date, "%Y-%m-%d")? < cutoff
                || raw.count_uncertain
                || raw.count_incomplete
                || raw.count_pending
                || raw.context_conflicts.any()
                || parent
                    .expired_count
                    .is_some_and(|(older, _)| older >= position)
                || row.get_ref(1)?.as_str()? != parent.session
                || row.get_ref(0)?.as_blob()? != raw.original_id
            {
                return Ok(None);
            }
            let resolved = db.query_row(
                "SELECT r.payload FROM token_resolved r JOIN token_sessions s ON s.id = r.session_id
                 WHERE r.event_id = ?1 AND s.session = ?2
                 AND NOT EXISTS(SELECT 1 FROM token_collisions WHERE id = ?1)",
                params![raw.original_id.as_slice(), parent.session],
                |row| row.get::<_, Vec<u8>>(0),
            ).optional()?;
            let Some(resolved) = resolved else {
                return Ok(None);
            };
            bytes += resolved.len();
            if bytes > limit {
                return Ok(None);
            }
            let selected: Observation = serde_json::from_slice(&resolved)?;
            // resolve_counts marks every competing native total, even for zero deltas.
            if selected.original_id != raw.original_id
                || selected.terminal_response.is_some()
                || selected.timestamp != raw.timestamp
                || selected.turn != raw.turn
                || selected.total.0 != raw.total.0
                || selected.inherited
                || selected.context_conflicts.any()
                || selected.count_uncertain
                || selected
                    .classification_origin
                    .is_some_and(|origin| origin.count_uncertain)
                || selected.count_incomplete
                || selected.count_pending
            {
                return Ok(None);
            }
            parent.expired_count = Some((position, raw.total.0));
            return Ok(Some(Self {
                link,
                path,
                state: parent,
                bytes,
            }));
        }
        Ok(None)
    }

    pub(super) fn independent(
        &self,
        files: &BTreeMap<PathBuf, FileState>,
        update: SourceUpdate<'_>,
        cutoff: NaiveDate,
    ) -> bool {
        if files.contains_key(&self.path) {
            return false;
        }
        if let Some(prior) = files.get(update.source) {
            let (Some(old), Some(now)) = (&prior.history_base, &update.state.history_base) else {
                return false;
            };
            if prior.session != update.state.session
                || prior.issues != 0
                || prior.identity_conflict
                || prior.invalid_counts
                || prior.rollout_out_of_order
                || (old.rollout, old.end_ordinal, old.end_offset)
                    != (now.rollout, now.end_ordinal, now.end_offset)
            {
                return false;
            }
        }
        // With one origin per native event the local graph cannot spend the full
        // resolver's multiple-predecessor budget. Equal latest times with different
        // totals would let a change in topological order affect terminal receipts.
        let mut ids = HashSet::new();
        let mut latest = HashMap::<&str, (&str, _, bool)>::new();
        for event in files
            .values()
            .chain([update.state])
            .filter(|file| {
                !file.identity_conflict && (!file.session.is_empty() || file.stamp.is_none())
            })
            .flat_map(|file| &file.events)
            .filter(|event| event.terminal_response.is_none() && event.date >= cutoff)
        {
            if !ids.insert(event.original_id) {
                return false;
            }
            let last =
                latest
                    .entry(&event.session)
                    .or_insert((&event.timestamp, event.total, false));
            match event.timestamp.as_str().cmp(last.0) {
                std::cmp::Ordering::Greater => *last = (&event.timestamp, event.total, false),
                std::cmp::Ordering::Equal if last.1 != event.total => last.2 = true,
                _ => {}
            }
        }
        latest.values().all(|(_, _, ambiguous)| !ambiguous)
    }
}
