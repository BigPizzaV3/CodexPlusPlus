//! 缓存最小请求证据与已核验的供应商归属，不保存会话用量历史。
use super::*;
use rusqlite::{Connection, params};

pub(super) fn storage_dir(home: &Path, settings_path: &Path) -> PathBuf {
    super::super::storage_dir(home, settings_path)
}

pub(super) fn database_path(storage: &Path) -> io::Result<PathBuf> {
    // Windows SQLite 需要扩展路径才能在长目录中打开数据库及其 WAL/SHM。
    #[cfg(windows)]
    let storage = fs::canonicalize(storage)?;
    #[cfg(not(windows))]
    let storage = storage.to_path_buf();
    Ok(storage.join("usage-provider-cache.sqlite"))
}

#[derive(serde::Serialize, Deserialize)]
struct ProxyObservationExtra {
    final_response_id: Option<String>,
    cached_tokens: Option<u64>,
    cached_tokens_known: Option<bool>,
}

fn apply_proxy_extra(row: &mut RequestEvidence, data: Option<&str>) -> anyhow::Result<()> {
    if let Some(data) = data {
        let extra: ProxyObservationExtra = serde_json::from_str(data)?;
        row.final_response_id = extra.final_response_id;
        if let Some(usage) = &mut row.usage {
            usage.cached_tokens = extra.cached_tokens;
            usage.cached_tokens_known = extra.cached_tokens_known;
        } else if extra.cached_tokens.is_some() || extra.cached_tokens_known.is_some() {
            anyhow::bail!("proxy cache counts without usage");
        }
    }
    Ok(())
}

fn stored_observation(data: &str, extra: Option<&str>) -> anyhow::Result<Option<RequestEvidence>> {
    let Ok(mut row) = serde_json::from_str::<RequestEvidence>(data) else {
        return Ok(None);
    };
    // 数据库只保存投影。未识别的持久证据不能像网关日志里的展示字段那样直接忽略。
    let encoded = serde_json::to_string(&row).expect("serialize request evidence");
    if encoded != data
        && serde_json::from_str::<Value>(data)? != serde_json::to_value(&row).unwrap()
    {
        return Ok(None);
    }
    apply_proxy_extra(&mut row, extra)?;
    Ok(row.valid_capture().then_some(row))
}

pub(super) struct Captured<T> {
    pub(super) rows: Vec<T>,
    pub(super) floor: i64,
    pub(super) complete: bool,
}

#[derive(Default)]
pub(super) struct ProviderCache {
    path: PathBuf,
    db: Option<Connection>,
    fingerprints: HashMap<[u8; 32], [u8; 32]>,
    #[cfg(test)]
    native_fingerprints: HashMap<[u8; 32], [u8; 32]>,
    pub(super) aliases: BTreeMap<String, String>,
    alias_seen: BTreeMap<String, i64>,
    cutoff: Option<NaiveDate>,
    has_proxy_extra: bool,
}

impl ProviderCache {
    pub(super) fn since(cutoff: NaiveDate) -> i64 {
        cutoff
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis()
            - 86_400_000
    }

    pub(super) fn data_version(&self) -> anyhow::Result<i64> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        Ok(db.pragma_query_value(None, "data_version", |row| row.get(0))?)
    }

    pub(super) fn read_snapshot(storage: &Path, cutoff: NaiveDate) -> anyhow::Result<Self> {
        let path = database_path(storage)?;
        if !fs::symlink_metadata(&path)?.is_file() {
            anyhow::bail!("provider cache is not a regular file");
        }
        let db = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        db.busy_timeout(Duration::from_millis(100))?;
        // 所有别名、覆盖边界和拒绝身份来自同一只读快照，不创建、迁移或淘汰数据。
        db.execute_batch("BEGIN DEFERRED;")?;
        let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != 2 {
            anyhow::bail!("unsupported provider cache version");
        }
        let has_proxy_extra = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table'
             AND name='completed_proxy_request_extra')",
            [],
            |row| row.get(0),
        )?;
        let mut aliases = BTreeMap::new();
        {
            let mut statement =
                db.prepare("SELECT id,target FROM aliases WHERE last_seen>=?1")?;
            let rows = statement
                .query_map([Self::since(cutoff)], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
            for row in rows {
                let (id, target) = row?;
                aliases.insert(id, target);
            }
        }
        Ok(Self {
            path,
            db: Some(db),
            aliases,
            cutoff: Some(cutoff),
            has_proxy_extra,
            ..Self::default()
        })
    }

    pub(super) fn open(&mut self, home: &Path, cutoff: NaiveDate) -> anyhow::Result<bool> {
        fs::create_dir_all(home)?;
        let path = database_path(home)?;
        if self.path == path && self.db.is_some() {
            self.expire(cutoff)?;
            return Ok(false);
        }
        *self = Self {
            path: path.clone(),
            ..Self::default()
        };
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
            anyhow::bail!("provider cache is not a regular file");
        }
        // 参数由调用方指定缓存根目录，不在此处查询全局路径。
        fs::create_dir_all(home)?;
        let mut db = Connection::open(path)?;
        db.busy_timeout(Duration::from_millis(100))?;
        let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 2 {
            anyhow::bail!("unsupported provider cache version");
        }
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS verified_requests(
                 id BLOB PRIMARY KEY NOT NULL, at INTEGER NOT NULL, data TEXT NOT NULL
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS verified_requests_at ON verified_requests(at);
             CREATE TABLE IF NOT EXISTS event_identity_collisions(
                 id BLOB PRIMARY KEY NOT NULL, at INTEGER NOT NULL
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS aliases(
                 id TEXT PRIMARY KEY NOT NULL, target TEXT NOT NULL
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS native_attributions(
                 id BLOB PRIMARY KEY NOT NULL, at INTEGER NOT NULL, data TEXT NOT NULL
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS native_attributions_at ON native_attributions(at);
             CREATE TABLE IF NOT EXISTS native_endpoint_binding(
                 singleton INTEGER PRIMARY KEY CHECK(singleton=1), fingerprint TEXT NOT NULL, since INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS completed_native_requests(
                 id BLOB PRIMARY KEY NOT NULL, at INTEGER NOT NULL, session TEXT NOT NULL, data TEXT NOT NULL
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS completed_native_session_at ON completed_native_requests(session,at);
             CREATE INDEX IF NOT EXISTS completed_native_at ON completed_native_requests(at);
             CREATE TABLE IF NOT EXISTS native_capture_cursors(
                 source TEXT PRIMARY KEY NOT NULL, position INTEGER NOT NULL, at INTEGER NOT NULL,
                 complete INTEGER NOT NULL DEFAULT 0, floor INTEGER NOT NULL DEFAULT 0,
                 anchor TEXT NOT NULL DEFAULT ''
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS completed_proxy_requests(
                 id TEXT PRIMARY KEY NOT NULL, at INTEGER NOT NULL, session TEXT NOT NULL, data TEXT NOT NULL
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS completed_proxy_session_at ON completed_proxy_requests(session,at);
             CREATE INDEX IF NOT EXISTS completed_proxy_at ON completed_proxy_requests(at);
             CREATE TABLE IF NOT EXISTS completed_proxy_request_extra(
                 id TEXT PRIMARY KEY NOT NULL, data TEXT NOT NULL
             ) WITHOUT ROWID;
             CREATE TRIGGER IF NOT EXISTS completed_proxy_extra_delete AFTER DELETE ON completed_proxy_requests
             BEGIN DELETE FROM completed_proxy_request_extra WHERE id=old.id; END;
             CREATE TRIGGER IF NOT EXISTS completed_proxy_extra_invalidate AFTER UPDATE OF data ON completed_proxy_requests
             BEGIN DELETE FROM completed_proxy_request_extra WHERE id=old.id; END;
             CREATE TABLE IF NOT EXISTS rejected_proxy_ids(
                 id TEXT PRIMARY KEY NOT NULL, at INTEGER NOT NULL
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS rejected_proxy_ids_at ON rejected_proxy_ids(at);
             CREATE TABLE IF NOT EXISTS capture_retention(
                 kind TEXT PRIMARY KEY NOT NULL, floor INTEGER NOT NULL
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS proxy_replayed_windows(
                 first INTEGER PRIMARY KEY, last INTEGER NOT NULL
             );
             "
        )?;
        db.execute(
            "INSERT OR IGNORE INTO capture_retention VALUES('proxy_legacy_until',?1)",
            [if version == 0 {
                0
            } else {
                Local::now().timestamp_millis()
            }],
        )?;
        if version < 2 {
            let tx = db.transaction()?;
            tx.execute_batch(
                "ALTER TABLE aliases ADD COLUMN last_seen INTEGER NOT NULL DEFAULT 0;",
            )?;
            tx.execute(
                "UPDATE aliases SET last_seen=?1",
                [Local::now().timestamp_millis()],
            )?;
            tx.execute_batch("PRAGMA user_version=2;")?;
            tx.commit()?;
        }
        {
            let mut statement = db.prepare("SELECT id,target,last_seen FROM aliases WHERE last_seen>=?1")?;
            let rows = statement.query_map([Self::since(cutoff)], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
            for row in rows {
                let (id, target, seen): (String, String, i64) = row?;
                self.alias_seen.insert(id.clone(), seen);
                self.aliases.insert(id, target);
            }
        }
        self.db = Some(db);
        self.has_proxy_extra = true;
        self.expire(cutoff)?;
        Ok(true)
    }

    fn expire(&mut self, cutoff: NaiveDate) -> anyhow::Result<()> {
        if self.cutoff == Some(cutoff) {
            return Ok(());
        }
        let since = Self::since(cutoff);
        let db = self
            .db
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let tx = db.transaction()?;
        let expired: Vec<Vec<u8>> = {
            let mut statement = tx.prepare("SELECT id FROM verified_requests WHERE at<?1")?;
            statement
                .query_map([since], |row| row.get(0))?
                .collect::<Result<_, _>>()?
        };
        tx.execute("DELETE FROM verified_requests WHERE at<?1", [since])?;
        tx.execute("DELETE FROM native_attributions WHERE at<?1", [since])?;
        tx.execute("DELETE FROM completed_native_requests WHERE at<?1", [since])?;
        tx.execute("DELETE FROM completed_proxy_requests WHERE at<?1", [since])?;
        tx.execute("DELETE FROM proxy_replayed_windows WHERE last<?1", [since])?;
        tx.execute("DELETE FROM rejected_proxy_ids WHERE at<?1", [since])?;
        tx.execute("DELETE FROM native_capture_cursors WHERE at<?1", [since])?;
        tx.execute("DELETE FROM aliases WHERE last_seen<?1", [since])?;
        tx.commit()?;
        for id in expired {
            if let Ok(id) = <[u8; 32]>::try_from(id) {
                self.fingerprints.remove(&id);
            }
        }
        self.alias_seen.retain(|_, at| *at >= since);
        #[cfg(test)]
        self.native_fingerprints.clear();
        self.aliases
            .retain(|id, _| self.alias_seen.contains_key(id));
        self.cutoff = Some(cutoff);
        Ok(())
    }

    pub(super) fn load_verified(
        &mut self,
        cutoff: NaiveDate,
    ) -> anyhow::Result<Vec<([u8; 32], VerifiedAttribution)>> {
        let since = Self::since(cutoff);
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let mut statement = db.prepare("SELECT id,data FROM verified_requests WHERE at>=?1")?;
        let rows = statement.query_map([since], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut records = Vec::new();
        for row in rows {
            let (id, data) = row?;
            let id = id
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid cached request identity"))?;
            let saved = serde_json::from_str(&data)?;
            self.fingerprints
                .insert(id, Sha256::digest(data.as_bytes()).into());
            records.push((id, saved));
        }
        Ok(records)
    }

    // 确认碰撞后保留拆分身份；分段暂缺或重启不能恢复旧的无轮次 ID。
    pub(super) fn event_collisions(
        &mut self,
        observed: &HashMap<[u8; 32], i64>,
        cutoff: NaiveDate,
    ) -> anyhow::Result<HashSet<[u8; 32]>> {
        let db = self
            .db
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let tx = db.transaction()?;
        tx.execute(
            "DELETE FROM event_identity_collisions WHERE at<?1",
            [Self::since(cutoff)],
        )?;
        for (id, at) in observed {
            tx.execute(
                "INSERT INTO event_identity_collisions(id,at) VALUES(?1,?2)
                ON CONFLICT(id) DO UPDATE SET at=max(at,excluded.at)",
                params![id.as_slice(), at],
            )?;
        }
        let count: usize = tx.query_row(
            "SELECT count(*) FROM event_identity_collisions",
            [],
            |row| row.get(0),
        )?;
        if count > MAX_EVENTS {
            anyhow::bail!("event identity collision capacity");
        }
        let collisions = {
            let mut statement = tx.prepare("SELECT id FROM event_identity_collisions LIMIT ?1")?;
            statement
                .query_map([MAX_EVENTS as i64], |row| row.get::<_, Vec<u8>>(0))?
                .map(|row| row?.try_into().map_err(|_| rusqlite::Error::InvalidQuery))
                .collect::<rusqlite::Result<HashSet<[u8; 32]>>>()?
        };
        tx.commit()?;
        Ok(collisions)
    }

    pub(super) fn save(
        &mut self,
        records: &HashMap<[u8; 32], VerifiedAttribution>,
        aliases: &BTreeMap<String, String>,
    ) -> anyhow::Result<usize> {
        let mut changed = Vec::new();
        for (id, saved) in records {
            let data = serde_json::to_string(saved)?;
            let fingerprint = Sha256::digest(data.as_bytes()).into();
            if self.fingerprints.get(id) != Some(&fingerprint) {
                changed.push((*id, saved.at, data, fingerprint));
            }
        }
        let today = Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        let alias_changes: Vec<_> = aliases
            .iter()
            .filter(|(id, target)| {
                self.aliases.get(*id) != Some(*target)
                    || self.alias_seen.get(*id).is_none_or(|at| *at < today)
            })
            .collect();
        if changed.is_empty() && alias_changes.is_empty() {
            return Ok(0);
        }
        let db = self
            .db
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let tx = db.transaction()?;
        {
            let mut statement = tx.prepare_cached(
                "INSERT INTO verified_requests(id,at,data) VALUES(?1,?2,?3)
                 ON CONFLICT(id) DO UPDATE SET at=excluded.at,data=excluded.data",
            )?;
            for (id, at, data, _) in &changed {
                statement.execute(params![id.as_slice(), at, data])?;
            }
            for (id, target) in &alias_changes {
                tx.execute("INSERT INTO aliases(id,target,last_seen) VALUES(?1,?2,?3)
                    ON CONFLICT(id) DO UPDATE SET target=excluded.target,last_seen=excluded.last_seen",
                    params![id, target, today])?;
            }
        }
        tx.commit()?;
        let writes = changed.len() + alias_changes.len();
        for (id, _, _, fingerprint) in changed {
            self.fingerprints.insert(id, fingerprint);
        }
        for (id, target) in alias_changes {
            self.aliases.insert(id.clone(), target.clone());
            self.alias_seen.insert(id.clone(), today);
        }
        Ok(writes)
    }

    pub(super) fn load_native(
        &mut self,
        cutoff: NaiveDate,
    ) -> anyhow::Result<HashMap<[u8; 32], native_endpoints::Attribution>> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let mut statement =
            db.prepare("SELECT id,data FROM native_attributions WHERE at>=?1")?;
        let rows = statement.query_map([Self::since(cutoff)], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut records = HashMap::new();
        for row in rows {
            let (id, data) = row?;
            let id = id
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid native evidence identity"))?;
            let proof: native_endpoints::Attribution = serde_json::from_str(&data)?;
            if !proof.valid() {
                continue;
            }
            #[cfg(test)]
            self.native_fingerprints
                .insert(id, Sha256::digest(data.as_bytes()).into());
            records.insert(id, proof);
        }
        Ok(records)
    }

    pub(super) fn endpoint_binding_since(
        &mut self,
        endpoints: &BTreeMap<String, Option<String>>,
    ) -> anyhow::Result<i64> {
        use rusqlite::OptionalExtension;
        let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(endpoints)?));
        let db = self
            .db
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let tx = db.transaction()?;
        let previous: Option<(String, i64)> = tx
            .query_row(
                "SELECT fingerprint,since FROM native_endpoint_binding WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let now = Local::now().timestamp_millis();
        let since = previous
            .filter(|(old, since)| *old == fingerprint && *since > 0 && *since <= now)
            .map(|(_, since)| since)
            .unwrap_or(now);
        tx.execute("INSERT INTO native_endpoint_binding VALUES(1,?1,?2)
            ON CONFLICT(singleton) DO UPDATE SET fingerprint=excluded.fingerprint,since=excluded.since
            WHERE fingerprint<>excluded.fingerprint OR since<>excluded.since", params![fingerprint,since])?;
        tx.commit()?;
        Ok(since)
    }

    #[cfg(test)]
    pub(super) fn save_native(
        &mut self,
        records: &HashMap<[u8; 32], native_endpoints::Attribution>,
    ) -> anyhow::Result<usize> {
        let mut changed = Vec::new();
        for (id, proof) in records {
            let data = serde_json::to_string(proof)?;
            let fingerprint = Sha256::digest(data.as_bytes()).into();
            if self.native_fingerprints.get(id) != Some(&fingerprint) {
                changed.push((*id, proof.at, data, fingerprint));
            }
        }
        if changed.is_empty() {
            return Ok(0);
        }
        let db = self
            .db
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let tx = db.transaction()?;
        {
            let mut statement = tx.prepare_cached(
                "INSERT INTO native_attributions(id,at,data) VALUES(?1,?2,?3)
                 ON CONFLICT(id) DO UPDATE SET at=excluded.at,data=excluded.data",
            )?;
            for (id, at, data, _) in &changed {
                statement.execute(params![id.as_slice(), at, data])?;
            }
        }
        tx.commit()?;
        let count = changed.len();
        for (id, _, _, fingerprint) in changed {
            self.native_fingerprints.insert(id, fingerprint);
        }
        Ok(count)
    }

    pub(super) fn retire_native_sources(
        &mut self,
        identities: &HashSet<String>,
        observed: i64,
    ) -> anyhow::Result<bool> {
        let db = self
            .db
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let identities = serde_json::to_string(identities)?;
        let tx = db.transaction()?;
        let changed = tx.execute(
            "UPDATE native_capture_cursors SET complete=1,floor=max(floor,?2),at=?2
             WHERE source IN (
                 SELECT source FROM native_capture_cursors
                 WHERE complete=0 AND source NOT IN (SELECT value FROM json_each(?1))
                 ORDER BY source LIMIT 64
             )",
            params![identities, observed],
        )?;
        tx.commit()?;
        Ok(changed == 64)
    }

    pub(super) fn capture_cursor(
        &self,
        source: &str,
    ) -> anyhow::Result<Option<(i64, String, i64, Option<i64>)>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?
            .query_row(
                "SELECT position,anchor,floor,at FROM native_capture_cursors WHERE source=?1",
                [source],
                |row| {
                    let anchor: String = row.get(1)?;
                    let (anchor, legacy_at) = match anchor.strip_prefix("full:") {
                        Some(full) => (full.to_owned(), None),
                        None => (anchor, Some(row.get(3)?)),
                    };
                    Ok((row.get(0)?, anchor, row.get(2)?, legacy_at))
                },
            )
            .optional()?)
    }

    pub(super) fn capture_native(
        &mut self,
        source: &str,
        position: i64,
        rows: &[([u8; 32], native_endpoints::Request)],
        complete: bool,
        floor: i64,
        anchor: &str,
    ) -> anyhow::Result<()> {
        let db = self
            .db
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let tx = db.transaction()?;
        for (id, row) in rows {
            if !row.valid() {
                continue;
            }
            // 首次观测的映射不可被后来的配置重解释；重复读取不写入。
            tx.execute(
                "INSERT OR IGNORE INTO completed_native_requests VALUES(?1,?2,?3,?4)",
                params![
                    id.as_slice(),
                    row.at,
                    row.session,
                    serde_json::to_string(row)?
                ],
            )?;
        }
        // 替换旧游标前记下受旧正文截断影响的时段，保留原始证据供复核。
        tx.execute(
            "INSERT INTO capture_retention(kind,floor)
             SELECT 'native_legacy_until',at FROM native_capture_cursors
             WHERE source=?1 AND anchor NOT GLOB 'full:*' AND position>0
             ON CONFLICT(kind) DO UPDATE SET floor=max(floor,excluded.floor)",
            [source],
        )?;
        tx.execute(
            "INSERT INTO native_capture_cursors VALUES(?1,?2,?3,?4,?5,?6)
            ON CONFLICT(source) DO UPDATE SET position=excluded.position,at=excluded.at,
                complete=excluded.complete,floor=max(floor,excluded.floor),anchor=excluded.anchor",
            params![
                source,
                position,
                Local::now().timestamp_millis(),
                complete,
                floor,
                format!("full:{anchor}")
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn captured_native(
        &self,
        session: &str,
        since: i64,
        until: i64,
    ) -> anyhow::Result<Captured<native_endpoints::Request>> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let tx = db.unchecked_transaction()?;
        let (floor, complete): (i64, bool) = tx.query_row(
            "SELECT coalesce(max(CASE WHEN anchor NOT GLOB 'full:*' AND position>0
                 THEN max(at,floor) ELSE floor END),0),coalesce(min(complete),1)
             FROM native_capture_cursors",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let floor = floor.max(self.capture_floor("completed_native_requests")?);
        let mut statement = tx.prepare(
            "SELECT data FROM completed_native_requests WHERE session=?1 AND at>?2 AND at<=?3
             ORDER BY at DESC,id",
        )?;
        let rows = statement.query_map(params![session, since.max(floor), until], |row| {
            row.get::<_, String>(0)
        })?;
        let mut result = Vec::new();
        for row in rows {
            let row: native_endpoints::Request = serde_json::from_str(&row?)?;
            if !row.valid() {
                anyhow::bail!("invalid captured native request");
            }
            result.push(row);
        }
        drop(statement);
        tx.commit()?;
        Ok(Captured {
            rows: result,
            floor,
            complete,
        })
    }

    pub(super) fn native_legacy_until(&self) -> anyhow::Result<i64> {
        let db = self.db.as_ref().ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        Ok(db.query_row(
            "SELECT max(coalesce((SELECT floor FROM capture_retention WHERE kind='native_legacy_until'),0),
                        coalesce((SELECT max(at) FROM native_capture_cursors
                                  WHERE anchor NOT GLOB 'full:*' AND position>0),0))",
            [], |row| row.get(0),
        )?)
    }

    #[cfg(test)]
    pub(super) fn capture_proxy(&mut self, rows: &[RequestEvidence]) -> anyhow::Result<usize> {
        self.capture_proxy_log(rows, None)
    }

    pub(super) fn capture_proxy_log(
        &mut self,
        rows: &[RequestEvidence],
        replay: Option<(i64, i64)>,
    ) -> anyhow::Result<usize> {
        let db = self
            .db
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let tx = db.transaction()?;
        let mut writes = 0;
        let mut ids = HashSet::new();
        let mut duplicates = HashSet::new();
        for row in rows {
            if !ids.insert(row.id.as_str()) {
                duplicates.insert(row.id.as_str());
            }
        }
        {
            let mut previous = tx.prepare_cached(
                "SELECT
                (SELECT data FROM completed_proxy_requests WHERE id=?1),
                (SELECT data FROM completed_proxy_request_extra WHERE id=?1),
                EXISTS(SELECT 1 FROM rejected_proxy_ids WHERE id=?1)",
            )?;
            let mut insert = tx.prepare_cached(
                "INSERT OR IGNORE INTO completed_proxy_requests VALUES(?1,?2,?3,?4)",
            )?;
            let mut reject = tx.prepare_cached(
                "INSERT INTO rejected_proxy_ids VALUES(?1,?2)
                ON CONFLICT(id) DO UPDATE SET at=max(at,excluded.at) WHERE at<?3",
            )?;
            let mut invalidate = tx.prepare_cached(
                "UPDATE completed_proxy_requests SET data=?2 WHERE id=?1 AND data<>?2",
            )?;
            let mut save_extra = tx.prepare_cached(
                "INSERT INTO completed_proxy_request_extra VALUES(?1,?2)
                ON CONFLICT(id) DO UPDATE SET data=excluded.data",
            )?;
            let now = Local::now().timestamp_millis();
            let today = now.div_euclid(86_400_000) * 86_400_000;
            for row in rows {
                if row.id.is_empty() {
                    continue;
                }
                let (old, extra, denied): (Option<String>, Option<String>, bool) = previous
                    .query_row([&row.id], |entry| {
                        Ok((entry.get(0)?, entry.get(1)?, entry.get(2)?))
                    })?;
                let stored = old
                    .as_deref()
                    .map(|data| stored_observation(data, extra.as_deref()))
                    .transpose()?
                    .flatten();
                let merged = stored
                    .as_ref()
                    .and_then(|saved| saved.merge_observation(row));
                let denied = denied
                    || duplicates.contains(row.id.as_str())
                    || !row.valid_capture()
                    || old.is_some() && merged.is_none();
                let at = if row.timestamp_ms > 0 {
                    row.timestamp_ms
                } else {
                    now
                };
                let rejected = RequestEvidence {
                    id: row.id.clone(),
                    thread_id: row.thread_id.clone(),
                    timestamp_ms: at,
                    ..Default::default()
                };
                let mut extra = None;
                if old.is_none() {
                    let projection = if denied {
                        rejected.clone()
                    } else {
                        // 旧程序整段比较这列，重读时也不能更换最初的计数投影。
                        row.legacy_projection()
                    };
                    if !denied && projection != *row {
                        extra = Some(row);
                    }
                    writes += insert.execute(params![
                        row.id,
                        at,
                        RequestSessionAliases::identity(&rejected.thread_id),
                        serde_json::to_string(&projection)?
                    ])?;
                }
                if denied {
                    // 拒绝身份独立于首次请求的会话和时间索引；重复观测每日续期一次。
                    writes += reject.execute(params![row.id, now.max(at), today])?;
                    writes +=
                        invalidate.execute(params![row.id, serde_json::to_string(&rejected)?])?;
                } else {
                    if let Some(std::borrow::Cow::Owned(enriched)) = &merged {
                        extra = Some(enriched);
                    }
                    if let Some(extra) = extra {
                        let data = serde_json::to_string(&ProxyObservationExtra {
                            final_response_id: extra.final_response_id.clone(),
                            cached_tokens: extra.usage.and_then(|u| u.cached_tokens),
                            cached_tokens_known: extra.usage.and_then(|u| u.cached_tokens_known),
                        })?;
                        writes += save_extra.execute(params![row.id, data])?;
                    }
                }
            }
        }
        if let Some((first, last)) = replay {
            let legacy_until: i64 = tx.query_row(
                "SELECT floor FROM capture_retention WHERE kind='proxy_legacy_until'",
                [],
                |r| r.get(0),
            )?;
            if first <= legacy_until {
                let (first, last): (i64, i64) = tx.query_row(
                    "SELECT min(?1,coalesce(min(first),?1)),max(?2,coalesce(max(last),?2))
                     FROM proxy_replayed_windows WHERE last>=?1 AND first<=?2",
                    params![first, last.min(legacy_until)],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                tx.execute(
                    "DELETE FROM proxy_replayed_windows WHERE first>?1 AND last<=?2",
                    params![first, last],
                )?;
                tx.execute(
                    "INSERT INTO proxy_replayed_windows VALUES(?1,?2)
                     ON CONFLICT(first) DO UPDATE SET last=excluded.last WHERE last<excluded.last",
                    params![first, last],
                )?;
            }
        }
        tx.commit()?;
        Ok(writes)
    }

    pub(super) fn proxy_coverage(&self) -> anyhow::Result<Vec<(i64, i64)>> {
        use rusqlite::OptionalExtension;
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let until: Option<i64> = db
            .query_row(
                "SELECT floor FROM capture_retention WHERE kind='proxy_legacy_until'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        // 未迁移的只读库尚未复证旧读取器可能跳过的日志范围。
        let Some(until) = until else {
            return Ok(Vec::new());
        };
        let mut windows = vec![(until.saturating_add(1), i64::MAX)];
        if until > 0 {
            let mut stmt =
                db.prepare("SELECT first,last FROM proxy_replayed_windows ORDER BY first DESC")?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
                let (first, last) = row?;
                let next = windows.last_mut().unwrap();
                if last.saturating_add(1) >= next.0 {
                    next.0 = first.min(next.0);
                } else {
                    windows.push((first, last));
                }
            }
        }
        Ok(windows)
    }

    pub(super) fn captured_proxy(
        &self,
        session: &str,
        since: i64,
        until: i64,
    ) -> anyhow::Result<Captured<RequestEvidence>> {
        self.captured_proxy_context(session, since, until, false)
    }

    pub(super) fn captured_proxy_context(
        &self,
        session: &str,
        since: i64,
        until: i64,
        include_neighbors: bool,
    ) -> anyhow::Result<Captured<RequestEvidence>> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let tx = db
            .is_autocommit()
            .then(|| db.unchecked_transaction())
            .transpose()?;
        let floor = self
            .capture_floor("completed_proxy_requests")?
            .saturating_add(1);
        let extra = self.proxy_extra_select();
        let mut statement = db.prepare(&format!(
            "SELECT data,{extra} FROM completed_proxy_requests AS observed
            WHERE session=?1 AND at>=?4 AND (at BETWEEN ?2 AND ?3
                OR (?5 AND at=(SELECT max(at) FROM completed_proxy_requests
                    WHERE session=?1 AND at<?2 AND at>=?4))
                OR (?5 AND at=(SELECT min(at) FROM completed_proxy_requests
                    WHERE session=?1 AND at>?3)))
            ORDER BY at DESC,id",
        ))?;
        let rows = statement.query_map(params![session, since.max(floor), until, floor, include_neighbors], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let rows = rows
            .map(|row| {
                let (data, extra) = row?;
                let mut row = serde_json::from_str(&data)?;
                apply_proxy_extra(&mut row, extra.as_deref())?;
                Ok(row)
            })
            .collect::<anyhow::Result<_>>()?;
        drop(statement);
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(Captured {
            rows,
            floor,
            complete: true,
        })
    }

    pub(super) fn rejected_proxy_ids<'a>(
        &self,
        rows: impl IntoIterator<Item = &'a RequestEvidence>,
    ) -> anyhow::Result<HashSet<String>> {
        self.check_proxy_ids(rows, true)
    }

    pub(super) fn rejected_proxy_ids_read_only<'a>(
        &self,
        rows: impl IntoIterator<Item = &'a RequestEvidence>,
    ) -> anyhow::Result<HashSet<String>> {
        self.check_proxy_ids(rows, false)
    }

    fn check_proxy_ids<'a>(
        &self,
        rows: impl IntoIterator<Item = &'a RequestEvidence>,
        persist: bool,
    ) -> anyhow::Result<HashSet<String>> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let mut current = BTreeMap::<&str, std::borrow::Cow<RequestEvidence>>::new();
        let mut rejected = HashSet::new();
        for row in rows {
            if row.id.is_empty() {
                continue;
            }
            let prior = current
                .entry(&row.id)
                .or_insert(std::borrow::Cow::Borrowed(row));
            if !row.valid_capture() {
                rejected.insert(row.id.clone());
            } else {
                match prior.merge_observation(row) {
                    None => {
                        rejected.insert(row.id.clone());
                    }
                    Some(std::borrow::Cow::Owned(merged)) => {
                        *prior = std::borrow::Cow::Owned(merged)
                    }
                    Some(std::borrow::Cow::Borrowed(_)) => {}
                }
            }
        }
        // 批量检查三类输入，避免逐事件查询；也发现采集线程尚未处理的 ID 复用。
        let ids: Vec<_> = current.keys().copied().collect();
        let now = Local::now().timestamp_millis();
        let today = now.div_euclid(86_400_000) * 86_400_000;
        let mut fresh = HashSet::new();
        let tx = db
            .is_autocommit()
            .then(|| db.unchecked_transaction())
            .transpose()?;
        let extra = self.proxy_extra_select();
        for batch in ids.chunks(256) {
            let slots = std::iter::repeat_n("?", batch.len())
                .collect::<Vec<_>>()
                .join(",");
            let mut statement = db.prepare(&format!(
                "SELECT id,data,{extra} FROM completed_proxy_requests AS observed WHERE id IN ({slots})"
            ))?;
            let stored = statement.query_map(rusqlite::params_from_iter(batch.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            for row in stored {
                let (id, data, extra) = row?;
                let matches = stored_observation(&data, extra.as_deref())?
                    .is_some_and(|saved| saved.merge_observation(&current[id.as_str()]).is_some());
                if !matches {
                    rejected.insert(id);
                }
            }
            let mut statement = db.prepare(&format!(
                "SELECT id,at FROM rejected_proxy_ids WHERE id IN ({slots})"
            ))?;
            let denied = statement.query_map(rusqlite::params_from_iter(batch.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in denied {
                let (id, at) = row?;
                if persist && at >= today {
                    fresh.insert(id.clone());
                }
                rejected.insert(id);
            }
        }
        if let Some(tx) = tx {
            tx.commit()?;
        }
        // 拒绝证据只会累加。先释放读快照，再在单独的短事务中登记或续期。
        if persist && rejected.len() > fresh.len() {
            let tx = db.unchecked_transaction()?;
            let mut statement = tx.prepare_cached(
                "INSERT INTO rejected_proxy_ids VALUES(?1,?2)
                ON CONFLICT(id) DO UPDATE SET at=excluded.at WHERE at<?3",
            )?;
            for id in rejected.difference(&fresh) {
                statement.execute(params![id, now, today])?;
            }
            drop(statement);
            tx.commit()?;
        }
        Ok(rejected)
    }

    fn proxy_extra_select(&self) -> &'static str {
        if self.has_proxy_extra {
            "(SELECT data FROM completed_proxy_request_extra WHERE id=observed.id)"
        } else {
            "NULL"
        }
    }

    fn capture_floor(&self, kind: &str) -> anyhow::Result<i64> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let floor: i64 = db.query_row(
            "SELECT coalesce(max(floor),0) FROM capture_retention WHERE kind=?1",
            [kind],
            |row| row.get(0),
        )?;
        Ok(floor.max(self.cutoff.map(Self::since).unwrap_or(0)))
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;

    #[test]
    fn captures_past_the_old_row_limit_keep_competitors_and_existing_floors() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let cutoff = Local::now().date_naive() - Days::days(5);
        let at = Local::now().timestamp_millis();
        let mut cache = ProviderCache::default();
        cache.open(root.path(), cutoff)?;
        let db = cache.db.as_ref().unwrap();
        for table in ["completed_native_requests", "completed_proxy_requests"] {
            db.execute(
                &format!("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?1)
                  INSERT INTO {table}(id,at,session,data)
                  SELECT printf('%032d',i),?2,'unrelated','{{}}' FROM n"),
                params![MAX_EVENTS - 1, at],
            )?;
            db.execute(
                "INSERT INTO capture_retention(kind,floor) VALUES(?1,?2)",
                params![table, at - 60_000],
            )?;
        }
        for id in [1u8, 2] {
            let native = serde_json::from_value(json!({
                "session":"wanted","turn":"turn","model":"model","at":at,"binding_since":at-1,
                "identity":([id; 32]),"provider":"supplier"
            }))?;
            cache.capture_native("source", i64::from(id), &[([id; 32], native)], true, 0, "test")?;
            let proxy = serde_json::from_value(json!({
                "id":id.to_string(),"threadId":"wanted","model":"model","upstreamModel":"model",
                "relayId":"supplier","success":true,"state":"success","statusCode":200,
                "timestampMs":at,"startedAtMs":at-10,"attempts":1,"autoRetries":0,
                "usage":{"inputTokens":10,"outputTokens":2,"cachedTokens":0}
            }))?;
            cache.capture_proxy(&[proxy])?;
        }
        drop(cache);
        let mut cache = ProviderCache::default();
        cache.open(root.path(), cutoff)?;
        for table in ["completed_native_requests", "completed_proxy_requests"] {
            assert_eq!(cache.capture_floor(table)?, at - 60_000);
            let count: usize = cache.db.as_ref().unwrap().query_row(
                &format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)
            )?;
            assert_eq!(count, MAX_EVENTS + 1);
        }
        assert_eq!(cache.captured_native("wanted", at - 1, at)?.rows.len(), 2);
        let proxies = cache.captured_proxy("wanted", at - 1, at)?.rows;
        assert_eq!(proxies.len(), 2);
        assert!(proxies.iter().all(|row| row.valid_capture()));
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn long_windows_storage_paths_support_wal_and_count_cache() {
        let root = tempfile::tempdir().unwrap();
        let storage = root.path().join("a".repeat(120)).join("b".repeat(120));
        fs::create_dir_all(&storage).unwrap();
        assert!(
            storage
                .join("usage-provider-cache.sqlite")
                .as_os_str()
                .len()
                > 260
        );

        let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
        let mut cache = ProviderCache::default();
        assert!(cache.open(&storage, cutoff).unwrap());
        assert!(native_endpoints::count_evidence_available(&storage, cutoff).unwrap());
        assert!(!cache.open(&storage, cutoff).unwrap());

        let mut second = ProviderCache::default();
        assert!(second.open(&storage, cutoff).unwrap());
        assert!(native_endpoints::count_evidence_available(&storage, cutoff).unwrap());

        let log = storage.join("logs_1.sqlite");
        let log_db =
            Connection::open(fs::canonicalize(&storage).unwrap().join("logs_1.sqlite")).unwrap();
        log_db
            .execute("CREATE TABLE logs (id INTEGER)", [])
            .unwrap();
        drop(log_db);
        let read_only = native_endpoints::open_log(&log).unwrap();
        let count: i64 = read_only
            .query_row("SELECT count(*) FROM logs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
