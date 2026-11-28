use super::*;

const ROWS: usize = 256;
const BYTES: usize = 1024 * 1024;

pub(in super::super) enum Batch {
    More,
    Finished { changed: bool },
    SourceChanged,
}

impl Store {
    pub(in super::super) fn has_encoding_work(&self, source: &Path, revision: i64) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM token_sources s
             LEFT JOIN token_observation_encoding e ON e.source_id=s.id
             WHERE s.source=?1 AND s.revision=?2 AND EXISTS(
                SELECT 1 FROM token_observations o WHERE o.source_id=s.id
                AND (o.revision,o.position)>(coalesce(e.revision,-1),coalesce(e.position,-1))))",
            params![encode_path(source), revision],
            |r| r.get(0),
        )?)
    }

    pub(in super::super) fn compact_source_observations(
        &mut self,
        source: &Path,
        revision: i64,
        read_left: &mut u64,
    ) -> Result<Batch> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner: Option<(i64,i64,i64)> = tx.query_row(
            "SELECT s.id, coalesce(e.revision,-1), coalesce(e.position,-1) FROM token_sources s
             LEFT JOIN token_observation_encoding e ON e.source_id=s.id WHERE s.source=?1 AND s.revision=?2",
            params![encode_path(source),revision], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional()?;
        let Some((id, rev, pos)) = owner else {
            return Ok(Batch::SourceChanged);
        };
        // Inspect lengths by primary key before fetching any observation payload.
        let page = tx.prepare_cached(
            "SELECT revision,position,length(payload) FROM token_observations WHERE source_id=?1
             AND (revision,position)>(?2,?3) ORDER BY revision,position LIMIT ?4",
        )?.query_map(params![id,rev,pos,ROWS],|r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,usize>(2)?)))?
          .collect::<rusqlite::Result<Vec<_>>>()?;
        let allowance = (*read_left).min(BYTES as u64) as usize;
        let mut used = 0;
        let mut after = (rev, pos);
        let mut end = page.len() < ROWS;
        let mut changed = false;
        {
            let mut get = tx.prepare_cached("SELECT payload FROM token_observations WHERE source_id=?1 AND revision=?2 AND position=?3")?;
            let mut put = tx.prepare_cached(
                "UPDATE token_observations SET payload=CAST(json_remove(CAST(?4 AS TEXT),?5,?6) AS BLOB)
                 WHERE source_id=?1 AND revision=?2 AND position=?3",
            )?;
            for (rev, pos, bytes) in page {
                // An oversized row remains intact. Its successors may still have recoverable space.
                if bytes <= BYTES {
                    if bytes > allowance - used {
                        end = false;
                        break;
                    }
                    let raw: Vec<u8> = get.query_row(params![id, rev, pos], |r| r.get(0))?;
                    used += bytes;
                    *read_left -= bytes as u64;
                    let object: serde_json::Map<String, serde_json::Value> =
                        serde_json::from_slice(&raw)?;
                    let null =
                        |field: &str| object.get(field).is_some_and(serde_json::Value::is_null);
                    let paths = match (null("legacy_identity"), null("provider_source")) {
                        (true, true) => Some(("$.legacy_identity", "$.provider_source")),
                        (true, false) => Some(("$.legacy_identity", "$.legacy_identity")),
                        (false, true) => Some(("$.provider_source", "$.provider_source")),
                        (false, false) => None,
                    };
                    if let Some((first, second)) = paths {
                        put.execute(params![id, rev, pos, raw, first, second])?;
                        changed = true;
                    }
                }
                after = (rev, pos);
            }
        }
        if after != (rev, pos) {
            tx.execute("INSERT INTO token_observation_encoding VALUES(?1,?2,?3)
                ON CONFLICT(source_id) DO UPDATE SET revision=excluded.revision,position=excluded.position",
                params![id,after.0,after.1])?;
        }
        tx.commit()?;
        Ok(if end {
            Batch::Finished { changed }
        } else {
            Batch::More
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn bounded_legacy_encoding_resumes_and_preserves_evidence() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("db");
        let source = Path::new("legacy");
        let mut store = Store::open(Connection::open(&file)?)?;
        store.db.execute("INSERT INTO token_sources(id,source,revision,offset,checkpoint,activity_truncated) VALUES(1,?1,7,42,x'7b7d',0)",[encode_path(source)])?;
        store
            .db
            .execute("INSERT INTO token_sessions VALUES(1,'session')", [])?;
        for n in 0..300 {
            let mut value = json!({"legacy_identity":null,"provider_source":null,"extra":[7]});
            match n {
                0 => value["legacy_identity"] = json!({"id":[8],"role":"main"}),
                1 => value["provider_source"] = json!("upstream-exact"),
                2 => value["large"] = json!("x".repeat(BYTES + 1)),
                3 | 4 => value["large"] = json!("x".repeat(BYTES / 2 + 10)),
                _ => (),
            }
            store.db.execute(
                "INSERT INTO token_observations VALUES(1,7,?1,1,x'12',?2)",
                params![n, serde_json::to_vec(&value)?],
            )?;
        }
        store.db.execute_batch(
            "CREATE TRIGGER fail BEFORE UPDATE ON token_observations WHEN NEW.position=1
                                BEGIN SELECT RAISE(ABORT,'forced rollback'); END;",
        )?;
        let mut limit = u64::MAX;
        assert!(
            store
                .compact_source_observations(source, 7, &mut limit)
                .is_err()
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM token_observation_encoding", [], |r| r
                    .get::<_, i64>(0))?,
            0
        );
        let payload = |store: &Store, n| -> Result<Value> {
            let bytes=store.db.query_row("SELECT payload FROM token_observations WHERE source_id=1 AND revision=7 AND position=?1",[n],|r|r.get::<_,Vec<u8>>(0))?;
            Ok(serde_json::from_slice(&bytes)?)
        };
        assert!(payload(&store, 0)?.get("provider_source").is_some());
        store.db.execute_batch("DROP TRIGGER fail")?;
        assert!(matches!(
            store.compact_source_observations(source, 6, &mut limit)?,
            Batch::SourceChanged
        ));
        let mut left = 2 * BYTES as u64;
        assert!(matches!(
            store.compact_source_observations(source, 7, &mut left)?,
            Batch::More
        ));
        assert!(left >= BYTES as u64);
        assert_eq!(
            store
                .db
                .query_row("SELECT position FROM token_observation_encoding", [], |r| r
                    .get::<_, i64>(0))?,
            3
        );
        assert!(payload(&store, 0)?.get("provider_source").is_none());
        assert_eq!(payload(&store, 0)?["legacy_identity"]["id"], json!([8]));
        assert_eq!(payload(&store, 1)?["provider_source"], "upstream-exact");
        assert!(payload(&store, 1)?.get("legacy_identity").is_none());
        assert!(payload(&store, 2)?.get("provider_source").is_some());
        drop(store);
        let mut store = Store::open(Connection::open(&file)?)?;
        assert!(store.has_encoding_work(source, 7)?);
        assert!(matches!(
            store.compact_source_observations(source, 7, &mut limit)?,
            Batch::More
        ));
        assert_eq!(
            store
                .db
                .query_row("SELECT position FROM token_observation_encoding", [], |r| r
                    .get::<_, i64>(0))?,
            259
        );
        assert!(matches!(
            store.compact_source_observations(source, 7, &mut limit)?,
            Batch::Finished { changed: true }
        ));
        assert!(!store.has_encoding_work(source, 7)?);
        assert_eq!(
            store
                .db
                .query_row("SELECT revision,offset FROM token_sources", [], |r| Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?
                )))?,
            (7, 42)
        );
        Ok(())
    }
}
