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
    native_count: Option<usize>,
    proxy_count: Option<usize>,
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
        let mut aliases = BTreeMap::new();
        {
            let mut statement =
                db.prepare("SELECT id,target FROM aliases WHERE last_seen>=?1 LIMIT ?2")?;
            let rows = statement
                .query_map(params![Self::since(cutoff), MAX_EVENTS + 1], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
            for row in rows {
                let (id, target) = row?;
                aliases.insert(id, target);
            }
        }
        if aliases.len() > MAX_EVENTS {
            anyhow::bail!("provider alias limit");
        }
        Ok(Self {
            path,
            db: Some(db),
            aliases,
            cutoff: Some(cutoff),
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
             CREATE TABLE IF NOT EXISTS rejected_proxy_ids(
                 id TEXT PRIMARY KEY NOT NULL, at INTEGER NOT NULL
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS rejected_proxy_ids_at ON rejected_proxy_ids(at);
             CREATE TABLE IF NOT EXISTS capture_retention(
                 kind TEXT PRIMARY KEY NOT NULL, floor INTEGER NOT NULL
             ) WITHOUT ROWID;
             "
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
            let mut statement = db.prepare("SELECT id,target,last_seen FROM aliases LIMIT ?1")?;
            let rows = statement.query_map([MAX_EVENTS as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
            for row in rows {
                let (id, target, seen): (String, String, i64) = row?;
                self.alias_seen.insert(id.clone(), seen);
                self.aliases.insert(id, target);
            }
        }
        self.db = Some(db);
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
        self.native_count = None;
        self.proxy_count = None;
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
        let mut statement = db.prepare(
            "SELECT id,data FROM verified_requests WHERE at>=?1 ORDER BY at DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![since, MAX_EVENTS as i64], |row| {
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
            db.prepare("SELECT id,data FROM native_attributions WHERE at>=?1 LIMIT ?2")?;
        let rows = statement.query_map(params![Self::since(cutoff), MAX_EVENTS as i64], |row| {
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
        for (id, proof) in records.iter().take(MAX_EVENTS) {
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
    ) -> anyhow::Result<Option<(i64, String, i64)>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?
            .query_row(
                "SELECT position,anchor,floor FROM native_capture_cursors WHERE source=?1",
                [source],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
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
        let mut added = 0;
        for (id, row) in rows {
            if !row.valid() {
                continue;
            }
            // 首次观测的映射不可被后来的配置重解释；重复读取不写入。
            added += tx.execute(
                "INSERT OR IGNORE INTO completed_native_requests VALUES(?1,?2,?3,?4)",
                params![
                    id.as_slice(),
                    row.at,
                    row.session,
                    serde_json::to_string(row)?
                ],
            )?;
        }
        let count = trim_completed(&tx, "completed_native_requests", self.native_count, added)?;
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
                anchor
            ],
        )?;
        tx.commit()?;
        self.native_count = Some(count);
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
            "SELECT coalesce(max(floor),0),coalesce(min(complete),1) FROM native_capture_cursors",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut floor = floor.max(self.capture_floor("completed_native_requests")?);
        let mut statement = tx.prepare(
            "SELECT data FROM completed_native_requests WHERE session=?1 AND at>?2 AND at<=?3
             ORDER BY at DESC,id LIMIT 2001",
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
        if result.len() > 2000 {
            floor = floor.max(result.last().unwrap().at);
            result.retain(|row| row.at > floor);
        }
        drop(statement);
        tx.commit()?;
        Ok(Captured {
            rows: result,
            floor,
            complete,
        })
    }

    pub(super) fn capture_proxy(&mut self, rows: &[RequestEvidence]) -> anyhow::Result<usize> {
        let db = self
            .db
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let tx = db.transaction()?;
        let mut writes = 0;
        let mut added = 0;
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
            let now = Local::now().timestamp_millis();
            let today = now.div_euclid(86_400_000) * 86_400_000;
            for row in rows {
                if row.id.is_empty() || row.id.len() > 256 {
                    continue;
                }
                let data = serde_json::to_string(row)?;
                let (old, denied): (Option<String>, bool) =
                    previous.query_row([&row.id], |entry| Ok((entry.get(0)?, entry.get(1)?)))?;
                let denied = denied
                    || duplicates.contains(row.id.as_str())
                    || row.keys().is_empty()
                    || old.as_ref().is_some_and(|old| old != &data);
                let at = if row.timestamp_ms > 0 {
                    row.timestamp_ms
                } else {
                    now
                };
                let rejected = RequestEvidence {
                    id: row.id.clone(),
                    thread_id: row.thread_id.chars().take(256).collect(),
                    timestamp_ms: at,
                    ..Default::default()
                };
                let rejected_data = denied
                    .then(|| serde_json::to_string(&rejected))
                    .transpose()?;
                let inserted = insert.execute(params![
                    row.id,
                    at,
                    RequestSessionAliases::identity(&rejected.thread_id),
                    rejected_data.as_ref().unwrap_or(&data)
                ])?;
                added += inserted;
                writes += inserted;
                if denied {
                    // 拒绝身份独立于首次请求的会话和时间索引；重复观测每日续期一次。
                    writes += reject.execute(params![row.id, now.max(at), today])?;
                    writes += invalidate.execute(params![row.id, rejected_data])?;
                }
            }
        }
        let count = trim_completed(&tx, "completed_proxy_requests", self.proxy_count, added)?;
        tx.commit()?;
        self.proxy_count = Some(count);
        Ok(writes)
    }

    pub(super) fn captured_proxy(
        &self,
        session: &str,
        since: i64,
        until: i64,
    ) -> anyhow::Result<Captured<RequestEvidence>> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("provider cache unavailable"))?;
        let tx = db
            .is_autocommit()
            .then(|| db.unchecked_transaction())
            .transpose()?;
        let mut floor = self
            .capture_floor("completed_proxy_requests")?
            .saturating_add(1);
        let mut statement = db.prepare(
            "SELECT at,data FROM completed_proxy_requests
            WHERE session=?1 AND at>=?2 AND at<=?3 AND at>=?4 ORDER BY at DESC,id LIMIT 8193",
        )?;
        let rows = statement.query_map(params![session, since, until, floor], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut rows: Vec<(i64, RequestEvidence)> = rows
            .map(|row| {
                let (at, data) = row?;
                Ok((at, serde_json::from_str(&data)?))
            })
            .collect::<anyhow::Result<_>>()?;
        if rows.len() > MAX_REQUEST_EVIDENCE {
            // floor 是首个完整毫秒；不能保留 LIMIT 截断的边界毫秒内的半组请求。
            floor = floor.max(rows.last().unwrap().0.saturating_add(1));
            rows.retain(|(at, _)| *at >= floor);
        }
        drop(statement);
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(Captured {
            rows: rows.into_iter().map(|(_, row)| row).collect(),
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
        let mut current: BTreeMap<&str, [u8; 32]> = BTreeMap::new();
        let mut rejected = HashSet::new();
        for row in rows {
            if row.id.is_empty() || row.id.len() > 256 {
                continue;
            }
            let fingerprint = Sha256::digest(serde_json::to_vec(row)?).into();
            if row.keys().is_empty()
                || current
                    .insert(row.id.as_str(), fingerprint)
                    .is_some_and(|old| old != fingerprint)
            {
                rejected.insert(row.id.clone());
            }
        }
        // 批量检查三类输入，避免逐事件查询；也发现采集线程尚未处理的 ID 复用。
        let ids: Vec<_> = current.keys().copied().collect();
        let tx = db
            .is_autocommit()
            .then(|| db.unchecked_transaction())
            .transpose()?;
        for batch in ids.chunks(256) {
            let slots = std::iter::repeat_n("?", batch.len())
                .collect::<Vec<_>>()
                .join(",");
            let mut statement = db.prepare(&format!(
                "SELECT id,data FROM completed_proxy_requests WHERE id IN ({slots})"
            ))?;
            let stored = statement.query_map(rusqlite::params_from_iter(batch.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in stored {
                let (id, data) = row?;
                let fingerprint: [u8; 32] = Sha256::digest(data.as_bytes()).into();
                if current.get(id.as_str()) != Some(&fingerprint) {
                    rejected.insert(id);
                }
            }
            let mut statement = db.prepare(&format!(
                "SELECT id FROM rejected_proxy_ids WHERE id IN ({slots})"
            ))?;
            let denied = statement.query_map(rusqlite::params_from_iter(batch.iter()), |row| {
                row.get::<_, String>(0)
            })?;
            for id in denied {
                rejected.insert(id?);
            }
        }
        if persist {
            let now = Local::now().timestamp_millis();
            let today = now.div_euclid(86_400_000) * 86_400_000;
            let mut statement = db.prepare_cached(
                "INSERT INTO rejected_proxy_ids VALUES(?1,?2)
                ON CONFLICT(id) DO UPDATE SET at=excluded.at WHERE at<?3",
            )?;
            for id in &rejected {
                statement.execute(params![id, now, today])?;
            }
        }
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(rejected)
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

fn trim_completed(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    count: Option<usize>,
    added: usize,
) -> anyhow::Result<usize> {
    // 表名只来自本模块固定调用点；按整毫秒淘汰，避免保留半组矛盾请求。
    let count = match count {
        Some(count) => count.saturating_add(added),
        None => tx.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })?,
    };
    if count <= MAX_EVENTS {
        return Ok(count);
    }
    let at: i64 = tx.query_row(
        &format!("SELECT at FROM {table} ORDER BY at DESC LIMIT 1 OFFSET ?1"),
        [(MAX_EVENTS - 1) as i64],
        |row| row.get(0),
    )?;
    let removed = tx.execute(&format!("DELETE FROM {table} WHERE at<=?1"), [at])?;
    tx.execute(
        "INSERT INTO capture_retention VALUES(?1,?2)
        ON CONFLICT(kind) DO UPDATE SET floor=max(floor,excluded.floor)",
        params![table, at],
    )?;
    Ok(count.saturating_sub(removed))
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
        assert!(native_endpoints::cache_below_limit(&storage, cutoff).unwrap());
        assert!(!cache.open(&storage, cutoff).unwrap());

        let mut second = ProviderCache::default();
        assert!(second.open(&storage, cutoff).unwrap());
        assert!(native_endpoints::cache_below_limit(&storage, cutoff).unwrap());

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
