//! 原生直连没有经过协议代理时，使用实际 HTTP 完成日志补充归属。
use super::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeSet;

const WINDOW_MS: i64 = 600_000;
const MAX_ROWS: usize = 20_000;
const MAX_SESSION_ROWS: usize = 2_000;

pub(super) fn timestamp_ms(seconds: i64, nanos: i64) -> Option<i64> {
    if !(0..1_000_000_000).contains(&nanos) {
        return None;
    }
    seconds.checked_mul(1000)?.checked_add(nanos / 1_000_000)
}

pub(super) fn gateway_config_path(settings: &Path) -> PathBuf {
    if settings == codex_plus_core::paths::default_settings_path() {
        codex_plus_core::gateway::store::GatewayStore::default_path()
    } else {
        settings.with_file_name("gateway.json")
    }
}

pub(super) type ConfigStamp = Vec<Option<(u64, Option<SystemTime>, Option<(u64, u64)>)>>;

pub(super) fn config_stamp(settings: &Path) -> ConfigStamp {
    [settings.to_owned(), gateway_config_path(settings)]
        .iter()
        .map(|path| {
            fs::metadata(path)
                .ok()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| {
                    (
                        metadata.len(),
                        metadata.modified().ok(),
                        file_identity(&metadata),
                    )
                })
        })
        .collect()
}

pub(super) fn profiles(
    path: &Path,
) -> anyhow::Result<Vec<codex_plus_core::settings::RelayProfile>> {
    #[derive(Deserialize)]
    struct Profiles {
        #[serde(rename = "relayProfiles", default)]
        profiles: Vec<codex_plus_core::settings::RelayProfile>,
    }
    fn read<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<Option<T>> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > 8 * 1024 * 1024 {
            anyhow::bail!("supplier settings exceed read budget");
        }
        Ok(Some(serde_json::from_reader(
            file.take(8 * 1024 * 1024 + 1),
        )?))
    }
    let before = config_stamp(path);
    let mut profiles = read::<Profiles>(path)?.map_or_else(Vec::new, |value| value.profiles);
    if let Some(gateway) =
        read::<codex_plus_core::gateway::store::GatewaySettings>(&gateway_config_path(path))?
    {
        // 导入后两份配置可各自修改；source 和显示名不能证明它们仍是同一供应商。
        profiles.extend(gateway.providers.into_iter().map(|provider| provider.relay));
    }
    anyhow::ensure!(
        before == config_stamp(path),
        "supplier settings changed during read"
    );
    Ok(profiles)
}

pub(super) fn log_paths(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths: Vec<_> = fs::read_dir(root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|s| s.to_str())
                .and_then(|name| name.strip_prefix("logs_")?.strip_suffix(".sqlite"))
                .is_some_and(|v| !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit()))
        })
        .collect();
    paths.sort();
    if paths.len() > 4 {
        anyhow::bail!("too many native log databases");
    }
    Ok(paths)
}

fn log_paths_for_home(home: &Path, explicit: Option<&Path>) -> anyhow::Result<Vec<PathBuf>> {
    // 显式目录尚未创建也不能回退，避免读取另一实例的日志。
    log_paths(
        explicit
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(home),
    )
}

pub(super) fn open_log(path: &Path) -> anyhow::Result<Connection> {
    #[cfg(windows)]
    let path = fs::canonicalize(path)?;
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(50))?;
    Ok(db)
}

fn open_count_cache(storage: &Path) -> anyhow::Result<Connection> {
    let db = Connection::open_with_flags(
        provider_cache::database_path(storage)?,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(100))?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS structured_count_guard(
            singleton INTEGER PRIMARY KEY CHECK(singleton=1), at INTEGER NOT NULL
        )",
    )?;
    Ok(db)
}

pub(super) fn cache_below_limit(storage: &Path, cutoff: NaiveDate) -> anyhow::Result<bool> {
    let db = open_count_cache(storage)?;
    let full: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_attributions WHERE at>=?1 LIMIT 1 OFFSET ?2)",
        params![
            provider_cache::ProviderCache::since(cutoff),
            (MAX_EVENTS - 1) as i64
        ],
        |row| row.get(0),
    )?;
    let guarded: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM structured_count_guard WHERE singleton=1 AND at>=?1)",
        [provider_cache::ProviderCache::since(cutoff)],
        |row| row.get(0),
    )?;
    Ok(!full && !guarded)
}

pub(super) fn source_identity(path: &Path) -> anyhow::Result<String> {
    let file = File::open(path)?;
    let (volume, index) = codex_plus_core::codex_sqlite::opened_file_identity(&file)?;
    Ok(format!(
        "{}:{volume}:{index}",
        fs::canonicalize(path)?.to_string_lossy()
    ))
}

#[derive(Clone, Deserialize, serde::Serialize, PartialEq, Eq)]
pub(super) struct Attribution {
    key: RequestMatchKey,
    pub(super) at: i64,
    turn: String,
    provider: String,
    #[serde(default)]
    version: u8,
    #[serde(default)]
    binding_since: i64,
    #[serde(default)]
    rejected: bool,
    // 0: 无计数冲突；1: 仅继承副本冲突；2: 原记录冲突。
    #[serde(default, skip_serializing_if = "no_count_conflict")]
    count_conflict: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    original_model: String,
}

fn no_count_conflict(level: &u8) -> bool {
    *level == 0
}

fn fingerprint(proof: &Attribution) -> [u8; 32] {
    Sha256::digest(serde_json::to_vec(proof).expect("attribution serialization")).into()
}

fn merge_persisted(current: &Attribution, incoming: &Attribution, observed: bool) -> Attribution {
    if current.version == 3 {
        if incoming.version != 3 {
            return current.clone();
        }
        if current.count_conflict > incoming.count_conflict
            && !(observed && current.count_conflict == 1 && incoming.count_conflict == 0)
        {
            return current.clone();
        }
    }
    if matches!(current.version, 2 | 4)
        && matches!(incoming.version, 2 | 4)
        && current.version != incoming.version
    {
        let mut retained = current.clone();
        if incoming.rejected
            || current.key != incoming.key
            || current.at != incoming.at
            || current.turn != incoming.turn
            || current.provider != incoming.provider
            || current.binding_since != incoming.binding_since
        {
            retained.rejected = true;
        }
        return retained;
    }
    if matches!(current.version, 2 | 4) && current.version == incoming.version {
        if current.rejected && !incoming.rejected {
            return current.clone();
        }
        if !observed {
            let mut previous = current.clone();
            let mut next = incoming.clone();
            previous.rejected = false;
            next.rejected = false;
            if previous != next {
                previous.rejected = true;
                return previous;
            }
        }
    }
    incoming.clone()
}

impl Attribution {
    pub(super) fn valid(&self) -> bool {
        if self.version == 3 {
            // 冲突屏障只保存事件摘要和时间，不落盘会话、模型、供应商或计数。
            return self.rejected
                && self.count_conflict <= 2
                && self.at > 0
                && self.key.session.is_empty()
                && self.key.model.is_empty()
                && self.key.input == 0
                && self.key.output == 0
                && self.key.cached == 0
                && self.turn.is_empty()
                && self.provider.is_empty()
                && self.binding_since == 0
                && self.original_model.is_empty();
        }
        matches!(self.version, 2 | 4)
            && if self.version == 2 {
                self.original_model.is_empty()
            } else {
                !self.original_model.is_empty()
                    && self.original_model.len() <= 256
                    && request_base_model(&self.original_model)
                        .eq_ignore_ascii_case(&self.key.model)
            }
            && self.count_conflict == 0
            && self.binding_since > 0
            && self.at >= self.binding_since
            && !unknown_provider(&self.provider)
            && [
                &self.key.session,
                &self.key.model,
                &self.turn,
                &self.provider,
            ]
            .iter()
            .all(|s| !s.is_empty() && s.len() <= 256)
            && self.key.cached <= self.key.input
            && self
                .key
                .input
                .checked_add(self.key.output)
                .is_some_and(|n| n > 0)
    }
}

#[derive(Default)]
pub(super) struct Cache {
    pub(super) verified: HashMap<[u8; 32], Attribution>,
    persisted: HashMap<[u8; 32], [u8; 32]>,
    pending_overflow_at: Option<i64>,
    negative: HashMap<[u8; 32], Instant>,
    binding_since: i64,
    cursor: usize,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, PartialEq, Eq)]
pub(super) struct Request {
    pub(super) session: String,
    turn: String,
    model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original_model: Option<String>,
    pub(super) at: i64,
    provider: Option<String>,
    pub(super) binding_since: i64,
    identity: [u8; 32],
    #[serde(default)]
    source: String,
    #[serde(default)]
    row_id: i64,
}

impl Request {
    pub(super) fn valid(&self) -> bool {
        self.at > 0
            && self.binding_since > 0
            && [&self.session, &self.turn, &self.model]
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 256)
            && self.original_model.as_ref().is_none_or(|model| {
                !model.is_empty()
                    && model.len() <= 256
                    && request_base_model(model).eq_ignore_ascii_case(&self.model)
            })
            && self.provider.as_ref().is_none_or(|p| {
                !unknown_provider(p) && p.len() <= 256 && self.at >= self.binding_since
            })
    }
}

// 日志字段只解析独立键；正文、请求头和错误文本不作为身份来源。
fn field(text: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let mut found = None;
    for (start, _) in text.match_indices(&needle) {
        if start > 0 && !matches!(text.as_bytes()[start - 1], b' ' | b'{' | b'\n') {
            continue;
        }
        let value = &text[start + needle.len()..];
        let value = if value.starts_with('"') {
            let mut escaped = false;
            let end = value.char_indices().skip(1).find_map(|(i, ch)| {
                if escaped {
                    escaped = false;
                    return None;
                }
                if ch == '\\' {
                    escaped = true;
                    return None;
                }
                (ch == '"').then_some(i)
            })?;
            serde_json::from_str::<String>(&value[..=end]).ok()?
        } else {
            value
                .split(|c: char| c.is_whitespace() || matches!(c, '}' | ','))
                .next()?
                .to_owned()
        };
        if value.is_empty() || value.len() > 2048 {
            return None;
        }
        if found.as_ref().is_some_and(|previous| previous != &value) {
            return None;
        }
        found = Some(value);
    }
    found
}

fn public_endpoint(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    !authority.is_empty()
        && !authority.contains('@')
        && !value.contains(['?', '#', '\\'])
        && !value.chars().any(char::is_control)
        && !["localhost", "127.0.0.1", "[::1]"]
            .iter()
            .any(|host| authority == *host || authority.starts_with(&format!("{host}:")))
}

pub(super) fn endpoints(
    profiles: &[codex_plus_core::settings::RelayProfile],
) -> BTreeMap<String, Option<String>> {
    let mut identities = BTreeMap::<&str, BTreeSet<String>>::new();
    for profile in profiles {
        if profile.relay_mode != codex_plus_core::settings::RelayMode::Aggregate
            && !unknown_provider(&profile.id)
        {
            let base = codex_plus_core::relay_config::relay_profile_base_url(profile);
            identities
                .entry(&profile.id)
                .or_default()
                .insert(codex_plus_core::protocol_proxy::responses_url(&base));
        }
    }
    let mut result = BTreeMap::new();
    for profile in profiles {
        if profile.relay_mode == codex_plus_core::settings::RelayMode::Aggregate
            || unknown_provider(&profile.id)
        {
            continue;
        }
        let base = codex_plus_core::relay_config::relay_profile_base_url(profile);
        let endpoint = codex_plus_core::protocol_proxy::responses_url(&base);
        if !public_endpoint(&endpoint) {
            continue;
        }
        let provider = (identities[profile.id.as_str()].len() == 1).then(|| profile.id.clone());
        result
            .entry(endpoint)
            .and_modify(|previous: &mut Option<String>| {
                if previous != &provider {
                    *previous = None;
                }
            })
            .or_insert(provider);
    }
    result
}

pub(super) fn request(
    body: &str,
    session: &str,
    timestamp: (i64, i64),
    endpoints: &BTreeMap<String, Option<String>>,
    binding_since: i64,
    source: &str,
    id: i64,
) -> Option<Request> {
    let at = timestamp_ms(timestamp.0, timestamp.1)?;
    let (spans, message) = body.rsplit_once("}: Request completed ")?;
    // 只读取完成消息的固定字段；后续响应头中同名字段不能覆盖这些值。
    let message = message.split(" headers=").next()?;
    if field(message, "method")?.as_str() != "POST" {
        return None;
    }
    let status = field(message, "status")?.parse::<u16>().ok()?;
    if !(200..300).contains(&status) {
        return None;
    }
    let url = field(message, "url")?;
    if !url.ends_with("/responses") {
        return None;
    }
    let thread = field(spans, "thread.id").or_else(|| field(spans, "thread_id"))?;
    if thread != session {
        return None;
    }
    let turn = field(spans, "turn.id")?;
    let model = field(spans, "model")?;
    Some(Request {
        session: session.to_owned(),
        turn,
        model: request_base_model(&model).to_ascii_lowercase(),
        original_model: Some(model),
        at,
        provider: endpoints
            .get(&url)
            .cloned()
            .flatten()
            .filter(|_| at >= binding_since),
        binding_since,
        identity: Sha256::digest(body.as_bytes()).into(),
        source: source.to_owned(),
        row_id: id,
    })
}

fn event_key(
    event: &UsageEvent,
    aliases: &RequestSessionAliases,
) -> Option<(RequestMatchKey, i64)> {
    if event.count_incomplete || !event.delta.cached_known {
        return None;
    }
    Some((
        RequestMatchKey {
            session: aliases.resolve(&event.session)?,
            model: request_base_model(&event.model).to_ascii_lowercase(),
            input: event.delta.input_tokens,
            output: event.delta.output_tokens,
            cached: event.delta.cached_input_tokens,
        },
        DateTime::parse_from_rfc3339(&event.timestamp)
            .ok()?
            .timestamp_millis(),
    ))
}

impl Cache {
    pub(super) fn restore_after_failed_load(&mut self, records: HashMap<[u8; 32], Attribution>) {
        let previously_persisted = std::mem::take(&mut self.persisted);
        self.persisted = records
            .iter()
            .map(|(id, proof)| (*id, fingerprint(proof)))
            .collect();
        let pending = std::mem::replace(&mut self.verified, records);
        for (id, saved) in pending {
            if !saved.rejected {
                continue;
            }
            if saved.version == 3 {
                let locally_changed = previously_persisted.get(&id) != Some(&fingerprint(&saved));
                if saved.count_conflict == 1
                    && !locally_changed
                    && self.verified.get(&id).is_some_and(|restored| {
                        restored.version == 3 && restored.count_conflict == 0
                    })
                {
                    continue;
                }
                let stronger = self
                    .verified
                    .get(&id)
                    .is_some_and(|restored| restored.count_conflict > saved.count_conflict);
                if stronger {
                    continue;
                }
                if self.verified.contains_key(&id) || self.verified.len() < MAX_EVENTS {
                    self.verified.insert(id, saved);
                } else {
                    self.pending_overflow_at =
                        Some(self.pending_overflow_at.unwrap_or(saved.at).max(saved.at));
                }
            } else if let Some(restored) = self.verified.get_mut(&id) {
                restored.rejected = true;
            } else if self.verified.len() < MAX_EVENTS {
                self.verified.insert(id, saved);
            } else {
                self.pending_overflow_at =
                    Some(self.pending_overflow_at.unwrap_or(saved.at).max(saved.at));
            }
        }
    }

    pub(super) fn save_bounded(
        &mut self,
        storage: &Path,
        cutoff: NaiveDate,
        events: &[UsageEvent],
    ) -> anyhow::Result<(usize, bool)> {
        self.persisted
            .retain(|id, _| self.verified.contains_key(id));
        let mut changed = Vec::new();
        for (id, proof) in &self.verified {
            let digest = fingerprint(proof);
            if self.persisted.get(id) != Some(&digest) {
                changed.push((*id, proof.clone()));
            }
        }
        let mut overflow_at = events
            .iter()
            .filter(|event| {
                event.count_uncertain
                    && self
                        .verified
                        .get(&event.id)
                        .is_none_or(|proof| proof.count_conflict == 0)
            })
            .filter_map(|event| DateTime::parse_from_rfc3339(&event.timestamp).ok())
            .map(|at| at.timestamp_millis())
            .max();
        if let Some(at) = self.pending_overflow_at {
            overflow_at = Some(overflow_at.unwrap_or(at).max(at));
        }
        if changed.is_empty() && overflow_at.is_none() {
            return Ok((0, cache_below_limit(storage, cutoff)?));
        }
        let mut db = open_count_cache(storage)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut rows: usize = tx.query_row(
            "SELECT count(*) FROM (SELECT 1 FROM native_attributions LIMIT ?1)",
            [(MAX_EVENTS + 1) as i64],
            |row| row.get(0),
        )?;
        let mut resolved = Vec::new();
        let mut written = 0;
        for (id, incoming) in changed {
            let current: Option<(i64, String)> = tx
                .query_row(
                    "SELECT at,data FROM native_attributions WHERE id=?1",
                    [id.as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if current.is_none() && rows >= MAX_EVENTS {
                if incoming.rejected {
                    overflow_at = Some(overflow_at.unwrap_or(incoming.at).max(incoming.at));
                }
                continue;
            }
            let merged = if let Some((at, data)) = &current {
                let proof: Attribution = serde_json::from_str(data)?;
                anyhow::ensure!(proof.valid() && proof.at == *at, "invalid native evidence");
                let digest: [u8; 32] = Sha256::digest(data.as_bytes()).into();
                merge_persisted(&proof, &incoming, self.persisted.get(&id) == Some(&digest))
            } else {
                incoming
            };
            let data = serde_json::to_string(&merged)?;
            if current
                .as_ref()
                .is_none_or(|(at, old)| *at != merged.at || *old != data)
            {
                tx.execute(
                    "INSERT INTO native_attributions(id,at,data) VALUES(?1,?2,?3)
                     ON CONFLICT(id) DO UPDATE SET at=excluded.at,data=excluded.data",
                    params![id.as_slice(), merged.at, data],
                )?;
                written += 1;
            }
            rows += usize::from(current.is_none());
            resolved.push((id, fingerprint(&merged), merged));
        }
        if let Some(at) = overflow_at {
            // 缺失屏障不能只留在内存；单条全局屏障至少保留到受影响事件过期。
            let at = at.max(Local::now().timestamp_millis());
            tx.execute(
                "INSERT INTO structured_count_guard(singleton,at) VALUES(1,?1)
                 ON CONFLICT(singleton) DO UPDATE SET at=max(at,excluded.at)",
                [at],
            )?;
        }
        let guarded: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM structured_count_guard WHERE singleton=1 AND at>=?1)",
            [provider_cache::ProviderCache::since(cutoff)],
            |row| row.get(0),
        )?;
        tx.commit()?;
        self.pending_overflow_at = None;
        for (id, digest, proof) in resolved {
            self.persisted.insert(id, digest);
            self.verified.insert(id, proof);
        }
        Ok((written, rows < MAX_EVENTS && !guarded))
    }

    pub(super) fn observe_count_conflicts(
        &mut self,
        events: &mut [UsageEvent],
        scan_complete: bool,
    ) -> bool {
        let cutoff = provider_cache::ProviderCache::since(
            Local::now().date_naive() - Days::days(RETENTION_DAYS - 1),
        );
        self.verified.retain(|_, proof| proof.at >= cutoff);
        let mut available = true;
        for event in events {
            let previous = self
                .verified
                .get(&event.id)
                .map_or(0, |saved| saved.count_conflict);
            let observed = if event.count_uncertain {
                if event.inherited { 1 } else { 2 }
            } else {
                0
            };
            let retained = if previous == 1 && !event.inherited && scan_complete {
                0
            } else {
                previous
            };
            let conflict = retained.max(observed);
            event.count_uncertain = conflict > 0;
            if conflict == previous {
                continue;
            }
            if !self.verified.contains_key(&event.id) && self.verified.len() >= MAX_EVENTS {
                available = false;
                continue;
            }
            let Ok(at) = DateTime::parse_from_rfc3339(&event.timestamp) else {
                available = false;
                continue;
            };
            self.verified.insert(
                event.id,
                Attribution {
                    key: RequestMatchKey {
                        session: String::new(),
                        model: String::new(),
                        input: 0,
                        output: 0,
                        cached: 0,
                    },
                    at: at.timestamp_millis(),
                    turn: String::new(),
                    provider: String::new(),
                    version: 3,
                    binding_since: 0,
                    rejected: true,
                    count_conflict: conflict,
                    original_model: String::new(),
                },
            );
        }
        // 加载上限达到时，无法证明是否还有未加载的冲突屏障。
        available && self.verified.len() < MAX_EVENTS
    }

    pub(super) fn unobserved_count_conflicts(&self, seen: &HashMap<[u8; 32], usize>) -> usize {
        self.verified
            .iter()
            .filter(|(id, proof)| proof.count_conflict > 0 && !seen.contains_key(*id))
            .count()
    }

    pub(super) fn apply_count_conflicts(&self, events: &mut [UsageEvent]) {
        for event in events {
            event.count_uncertain |= self
                .verified
                .get(&event.id)
                .is_some_and(|proof| proof.count_conflict > 0);
        }
    }

    pub(super) fn attribute(
        &mut self,
        home: &Path,
        events: &mut [UsageEvent],
        endpoints: &BTreeMap<String, Option<String>>,
        binding_since: i64,
        session_aliases: &BTreeMap<String, String>,
        captured: &provider_cache::ProviderCache,
        scan_complete: bool,
    ) -> Value {
        let aliases = RequestSessionAliases::new(session_aliases);
        let blocked_sessions: HashSet<_> = events
            .iter()
            .filter(|event| event.count_incomplete || !event.delta.cached_known)
            .filter_map(|event| aliases.resolve(&event.session))
            .collect();
        let keys: Vec<_> = events
            .iter()
            .map(|event| event_key(event, &aliases))
            .collect();
        let now = Local::now().timestamp_millis();
        let cutoff = provider_cache::ProviderCache::since(
            Local::now().date_naive() - Days::days(RETENTION_DAYS - 1),
        );
        self.verified.retain(|_, proof| proof.at >= cutoff);
        if self.binding_since != binding_since {
            self.negative.clear();
            self.binding_since = binding_since;
        }
        self.negative
            .retain(|_, at| at.elapsed() < Duration::from_secs(300));
        let mut targets: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        let mut live_targets = HashSet::new();
        for (event, key) in events.iter().zip(&keys) {
            let Some((key, at)) = key else {
                continue;
            };
            if !scan_complete
                || event.count_uncertain
                || event.context_conflicts.any()
                || blocked_sessions.contains(&key.session)
                || !unknown_provider(&event.provider)
                || event.turn.is_empty()
            {
                continue;
            }
            if self.negative.contains_key(&event.id) {
                continue;
            }
            let saved = self.verified.get(&event.id);
            let retained = saved.is_some_and(|s| {
                !s.rejected
                    && &s.key == key
                    && s.at == *at
                    && s.turn == event.turn
                    && (s.version != 4 || s.original_model.eq_ignore_ascii_case(&event.model))
            }) && now.saturating_sub(*at) > WINDOW_MS;
            if !retained {
                live_targets.insert(key.session.clone());
            }
            targets
                .entry(key.session.clone())
                .and_modify(|(since, until)| {
                    *since = (*since).min(*at);
                    *until = (*until).max(*at);
                })
                .or_insert((*at, *at));
        }
        let mut records = Vec::new();
        let mut reads = 0usize;
        let mut limited = false;
        let mut unavailable = false;
        let mut coverage = BTreeMap::new();
        let mut checked = HashSet::new();
        let mut captured_reads = 0usize;
        let ordered: Vec<_> = targets.iter().collect();
        let selected: Vec<_> = (0..ordered.len().min(256))
            .map(|offset| ordered[(self.cursor + offset) % ordered.len()])
            .collect();
        // 反向展开已解析的别名；查询原始日志 ID，匹配时统一使用规范 ID。
        let mut query_sessions: BTreeMap<String, BTreeSet<String>> = selected
            .iter()
            .map(|(session, _)| ((*session).clone(), BTreeSet::from([(*session).clone()])))
            .collect();
        for (raw, canonical) in &aliases.canonical {
            if let Some(sessions) = canonical.as_ref().and_then(|id| query_sessions.get_mut(id)) {
                sessions.insert(raw.clone());
            }
        }
        for raw in session_aliases
            .keys()
            .chain(session_aliases.values())
            .chain(events.iter().map(|event| &event.session))
        {
            if let Some(sessions) = aliases
                .resolve(raw)
                .and_then(|id| query_sessions.get_mut(&id))
            {
                sessions.insert(raw.clone());
            }
        }
        // 已采集的请求可早于当前配置的观测 epoch；每条证据保留自己的原始映射。
        for (session, (since, until)) in &selected {
            let mut floor = i64::MIN;
            let mut complete = true;
            let mut available = false;
            for raw in &query_sessions[*session] {
                match captured.captured_native(raw, since.saturating_sub(WINDOW_MS), *until) {
                    Ok(batch) if records.len() + batch.rows.len() <= MAX_ROWS => {
                        available = true;
                        floor = floor.max(batch.floor);
                        complete &= batch.complete;
                        captured_reads += batch.rows.len();
                        records.extend(batch.rows);
                    }
                    Ok(_) => {
                        available = true;
                        limited = true;
                        floor = i64::MAX;
                        complete = false;
                    }
                    Err(_) => complete = false,
                }
            }
            if available {
                coverage.insert((*session).clone(), (floor, complete));
            }
        }
        if !targets.is_empty() {
            let explicit = std::env::var_os("CODEX_SQLITE_HOME").map(PathBuf::from);
            let paths = match log_paths_for_home(home, explicit.as_deref()) {
                Ok(paths) => paths,
                Err(_) => {
                    unavailable = true;
                    Vec::new()
                }
            };
            let mut live_complete = !paths.is_empty();
            for path in paths {
                let result = (|| -> anyhow::Result<()> {
                    let source = source_identity(&path)?;
                    let db = open_log(&path)?;
                    let snapshot = db.unchecked_transaction()?;
                    let mut statement = db.prepare(
                        "SELECT ts,ts_nanos,substr(feedback_log_body,1,16385),thread_id,id
                         FROM logs WHERE thread_id=?1 AND ts>=?2 AND ts<=?3
                         AND target='codex_http_client::client' ORDER BY ts,id LIMIT ?4",
                    )?;
                    for (session, (since, until)) in &selected {
                        if !live_targets.contains(*session) {
                            continue;
                        }
                        if reads >= MAX_ROWS {
                            limited = true;
                            coverage.insert((*session).clone(), (i64::MAX, false));
                            continue;
                        }
                        let mut session_reads = 0;
                        for raw in &query_sessions[*session] {
                            let rows = statement.query_map(
                                params![
                                    raw,
                                    since.saturating_sub(WINDOW_MS) / 1000,
                                    until / 1000 + 1,
                                    (MAX_SESSION_ROWS + 1) as i64
                                ],
                                |row| {
                                    Ok((
                                        row.get::<_, i64>(0)?,
                                        row.get::<_, i64>(1)?,
                                        row.get::<_, String>(2)?,
                                        row.get::<_, String>(3)?,
                                        row.get::<_, i64>(4)?,
                                    ))
                                },
                            )?;
                            for row in rows {
                                let (seconds, nanos, body, raw_session, id) = row?;
                                reads += 1;
                                session_reads += 1;
                                if session_reads > MAX_SESSION_ROWS
                                    || reads > MAX_ROWS
                                    || body.len() > 16384
                                {
                                    limited = true;
                                    coverage.insert((*session).clone(), (i64::MAX, false));
                                    break;
                                }
                                records.extend(request(
                                    &body,
                                    &raw_session,
                                    (seconds, nanos),
                                    endpoints,
                                    binding_since,
                                    &source,
                                    id,
                                ));
                            }
                            if session_reads > MAX_SESSION_ROWS || reads > MAX_ROWS {
                                break;
                            }
                        }
                        checked.insert((*session).clone());
                    }
                    drop(statement);
                    snapshot.commit()?;
                    anyhow::ensure!(
                        source_identity(&path)? == source,
                        "native log replaced during read"
                    );
                    Ok(())
                })();
                unavailable |= result.is_err();
                live_complete &= result.is_ok();
            }
            if live_complete {
                for (session, _) in &selected {
                    if live_targets.contains(*session) {
                        coverage
                            .entry((*session).clone())
                            .or_insert((i64::MIN, true));
                    }
                }
            }
            limited |= targets.len() > 256;
            self.cursor = self.cursor.wrapping_add(targets.len().min(256));
        }
        // 部分读取不能建立唯一性；旧的独立证据仍可保留。
        if unavailable {
            // 成功读到的反证必须保留；其他来源失败仅禁止建立新的唯一性证明。
            for (_, complete) in coverage.values_mut() {
                *complete = false;
            }
            checked.clear();
        }
        // 同一请求可能同时来自完成缓存和尚未轮转的日志，保留采集时的映射。
        let mut unique = BTreeMap::new();
        for mut row in records {
            let Some(session) = aliases.resolve(&row.session) else {
                continue;
            };
            row.session = session;
            unique
                .entry((row.source.clone(), row.row_id, row.at, row.identity))
                .and_modify(|previous: &mut Request| {
                    if previous.original_model.is_none() {
                        previous.original_model.clone_from(&row.original_model);
                    }
                })
                .or_insert(row);
        }
        let records: Vec<_> = unique.into_values().collect();
        let stats = self.apply(
            events,
            &keys,
            &records,
            binding_since,
            &coverage,
            scan_complete,
            &blocked_sessions,
        );
        for (event, key) in events.iter().zip(&keys) {
            if key.as_ref().is_some_and(|(key, at)| {
                checked.contains(&key.session)
                    && coverage.get(&key.session).is_some_and(|(floor, complete)| {
                        *complete && at.saturating_sub(WINDOW_MS) >= *floor
                    })
            }) && unknown_provider(&event.provider)
                && now.saturating_sub(key.as_ref().unwrap().1) > WINDOW_MS
                && self.negative.len() < MAX_EVENTS
            {
                self.negative.insert(event.id, Instant::now());
            }
        }
        json!({"source":"native-http-endpoint","recoveredEvents":stats.0,"recoveredTokens":stats.1,
            "logRowsRead":reads,"capturedRequestsRead":captured_reads,"limited":limited,"unavailable":unavailable,
            "cachedEvents":self.verified.len()})
    }

    fn apply(
        &mut self,
        events: &mut [UsageEvent],
        keys: &[Option<(RequestMatchKey, i64)>],
        records: &[Request],
        binding_since: i64,
        coverage: &BTreeMap<String, (i64, bool)>,
        scan_complete: bool,
        blocked_sessions: &HashSet<String>,
    ) -> (usize, u64) {
        type Group = (String, String, String);
        let mut requests: BTreeMap<Group, Vec<&Request>> = BTreeMap::new();
        for row in records {
            requests
                .entry((row.session.clone(), row.turn.clone(), row.model.clone()))
                .or_default()
                .push(row);
        }
        for rows in requests.values_mut() {
            rows.sort_by_key(|row| row.at);
        }
        let mut times: BTreeMap<(String, String), Vec<i64>> = BTreeMap::new();
        for (event, key) in events.iter().zip(keys) {
            if let Some((key, at)) = key {
                times
                    .entry((key.session.clone(), event.turn.clone()))
                    .or_default()
                    .push(*at);
            }
        }
        for rows in times.values_mut() {
            rows.sort_unstable();
        }
        let (mut recovered, mut tokens) = (0, 0u64);
        for (event, key) in events.iter_mut().zip(keys) {
            if event.context_conflicts.any() || event.count_uncertain {
                if let Some(saved) = self.verified.get_mut(&event.id) {
                    saved.rejected = true;
                }
                continue;
            }
            if !scan_complete || !unknown_provider(&event.provider) || event.turn.is_empty() {
                continue;
            }
            let Some((key, at)) = key else {
                continue;
            };
            if blocked_sessions.contains(&key.session) {
                continue;
            }
            let group = (key.session.clone(), event.turn.clone(), key.model.clone());
            let boundaries = &times[&(key.session.clone(), event.turn.clone())];
            let position = boundaries.partition_point(|time| time < at);
            if boundaries.partition_point(|time| time <= at) - position != 1 {
                continue;
            }
            let floor = position
                .checked_sub(1)
                .map(|i| boundaries[i])
                .unwrap_or(i64::MIN)
                .max(at.saturating_sub(WINDOW_MS));
            let empty = Vec::new();
            let candidates = requests.get(&group).unwrap_or(&empty);
            let start = candidates.partition_point(|row| row.at <= floor);
            let end = candidates.partition_point(|row| row.at <= *at);
            let observed = &candidates[start..end];
            let original_model = event.model.as_str();
            let legacy = self
                .verified
                .get(&event.id)
                .is_some_and(|saved| saved.version == 2);
            let uncertain = !legacy && observed.iter().any(|row| row.original_model.is_none());
            let rows: Vec<_> = if legacy {
                observed.to_vec()
            } else if uncertain {
                Vec::new()
            } else {
                observed
                    .iter()
                    .copied()
                    .filter(|row| {
                        row.original_model
                            .as_deref()
                            .is_some_and(|model| model.eq_ignore_ascii_case(original_model))
                    })
                    .collect()
            };
            let candidate = rows
                .first()
                .and_then(|first| first.provider.as_ref())
                .filter(|provider| {
                    rows.iter().all(|row| {
                        row.provider.as_ref() == Some(*provider)
                            && row.at
                                >= if row.binding_since > 0 {
                                    row.binding_since
                                } else {
                                    binding_since
                                }
                    })
                });
            if let Some(saved) = self.verified.get_mut(&event.id) {
                if saved.key != *key
                    || saved.at != *at
                    || saved.turn != event.turn
                    || saved.version == 4
                        && !saved.original_model.eq_ignore_ascii_case(original_model)
                    || saved.version == 4 && uncertain
                    || !rows.is_empty() && candidate != Some(&saved.provider)
                {
                    saved.rejected = true;
                }
            }
            let complete = coverage
                .get(&key.session)
                .is_some_and(|(since, complete)| *complete && floor >= *since);
            if let Some(provider) =
                candidate.filter(|_| complete && self.verified.len() < MAX_EVENTS)
            {
                let since = rows
                    .iter()
                    .map(|row| {
                        if row.binding_since > 0 {
                            row.binding_since
                        } else {
                            binding_since
                        }
                    })
                    .max()
                    .unwrap_or(binding_since);
                self.verified
                    .entry(event.id)
                    .or_insert_with(|| Attribution {
                        key: key.clone(),
                        at: *at,
                        turn: event.turn.clone(),
                        provider: provider.clone(),
                        version: 4,
                        binding_since: since,
                        rejected: false,
                        count_conflict: 0,
                        original_model: original_model.to_owned(),
                    });
            }
            if let Some(saved) = self.verified.get(&event.id).filter(|s| !s.rejected) {
                event.provider.clone_from(&saved.provider);
                recovered += 1;
                tokens = tokens.saturating_add(event.delta.total_tokens);
            }
        }
        (recovered, tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_model_is_required_for_new_proofs_and_legacy_proofs_are_retained() {
        let at = Local::now().timestamp_millis();
        let event = UsageEvent {
            id: [71; 32],
            session: "session".into(),
            turn: "turn".into(),
            model: "cpx-a/model".into(),
            timestamp: DateTime::from_timestamp_millis(at).unwrap().to_rfc3339(),
            date: Local::now().date_naive(),
            total: Tokens::new(100, 20, 10),
            delta: Tokens::new(100, 20, 10),
            has_file_baseline: false,
            last: None,
            count_uncertain: false,
            count_incomplete: false,
            provider: String::new(),
            provider_source: None,
            role: "main".into(),
            inherited: false,
            legacy_identity: None,
            context_conflicts: ContextConflicts::default(),
        };
        let aliases = RequestSessionAliases::new(&BTreeMap::new());
        let key = event_key(&event, &aliases).unwrap();
        let row = |model: &str, provider: &str| Request {
            session: event.session.clone(),
            turn: event.turn.clone(),
            model: request_base_model(model).to_owned(),
            original_model: Some(model.to_owned()),
            at: at - 10,
            provider: Some(provider.to_owned()),
            binding_since: at - 100,
            identity: [0; 32],
            source: String::new(),
            row_id: 0,
        };
        let exact = row("cpx-a/model", "supplier-a");
        let other = row("cpx-b/model", "supplier-b");
        let coverage = BTreeMap::from([(event.session.clone(), (i64::MIN, true))]);
        let mut cache = Cache::default();
        let mut fresh = vec![event.clone()];
        cache.apply(
            &mut fresh,
            &[Some(key.clone())],
            &[exact.clone(), other],
            at - 100,
            &coverage,
            true,
            &HashSet::new(),
        );
        assert_eq!(fresh[0].provider, "supplier-a");
        assert_eq!(fresh[0].model, "cpx-a/model");
        assert_eq!(cache.verified[&event.id].version, 4);
        assert_eq!(cache.verified[&event.id].original_model, event.model);

        let root = tempfile::tempdir().unwrap();
        let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
        let mut storage = provider_cache::ProviderCache::default();
        storage.open(root.path(), cutoff).unwrap();
        storage.save_native(&cache.verified).unwrap();
        drop(storage);
        let mut storage = provider_cache::ProviderCache::default();
        storage.open(root.path(), cutoff).unwrap();
        cache.verified = storage.load_native(cutoff).unwrap();
        let mut restored = vec![event.clone()];
        cache.apply(
            &mut restored,
            &[Some(key.clone())],
            &[],
            at - 100,
            &coverage,
            true,
            &HashSet::new(),
        );
        assert_eq!(restored[0].provider, "supplier-a");

        let mut conflict = vec![event.clone()];
        cache.apply(
            &mut conflict,
            &[Some(key.clone())],
            &[row("cpx-a/model", "supplier-b")],
            at - 100,
            &coverage,
            true,
            &HashSet::new(),
        );
        assert!(unknown_provider(&conflict[0].provider));
        assert!(cache.verified[&event.id].rejected);

        let mut old_request = exact.clone();
        old_request.original_model = None;
        let mut uncertain_cache = Cache::default();
        let mut first = vec![event.clone()];
        uncertain_cache.apply(
            &mut first,
            &[Some(key.clone())],
            &[exact, old_request.clone()],
            at - 100,
            &coverage,
            true,
            &HashSet::new(),
        );
        assert!(unknown_provider(&first[0].provider));
        assert!(!uncertain_cache.verified.contains_key(&event.id));

        let mut uncertain_cache = Cache::default();
        uncertain_cache
            .verified
            .insert(event.id, cache.verified[&event.id].clone());
        uncertain_cache
            .verified
            .get_mut(&event.id)
            .unwrap()
            .rejected = false;
        let mut disputed = vec![event.clone()];
        uncertain_cache.apply(
            &mut disputed,
            &[Some(key.clone())],
            &[old_request.clone()],
            at - 100,
            &coverage,
            true,
            &HashSet::new(),
        );
        assert!(unknown_provider(&disputed[0].provider));
        assert!(uncertain_cache.verified[&event.id].rejected);

        let mut cache = Cache::default();
        let mut unverified = vec![event.clone()];
        cache.apply(
            &mut unverified,
            &[Some(key.clone())],
            &[old_request],
            at - 100,
            &coverage,
            true,
            &HashSet::new(),
        );
        assert!(unknown_provider(&unverified[0].provider));
        assert!(!cache.verified.contains_key(&event.id));

        cache.verified.insert(
            event.id,
            Attribution {
                key: key.0.clone(),
                at,
                turn: event.turn.clone(),
                provider: "supplier-old".into(),
                version: 2,
                binding_since: at - 100,
                rejected: false,
                count_conflict: 0,
                original_model: String::new(),
            },
        );
        let old = cache.verified[&event.id].clone();
        let mut historical = vec![event.clone()];
        cache.apply(
            &mut historical,
            &[Some(key)],
            &[],
            at - 100,
            &coverage,
            true,
            &HashSet::new(),
        );
        assert_eq!(historical[0].provider, "supplier-old");
        assert!(cache.verified[&event.id] == old);

        for model in ["cpx-a/Model", "CPX-a/Model"] {
            let mut case_event = event.clone();
            case_event.model = model.to_owned();
            let body = format!(
                "{{thread.id=session turn.id=turn model={model}}}: Request completed method=POST url=https://one.example/v1/responses status=200 headers={{}}"
            );
            let row = request(
                &body,
                "session",
                (at.div_euclid(1000), at.rem_euclid(1000) * 1_000_000),
                &BTreeMap::from([(
                    "https://one.example/v1/responses".into(),
                    Some("supplier-a".into()),
                )]),
                at - 100,
                "source",
                1,
            )
            .unwrap();
            assert_eq!(row.original_model.as_deref(), Some(model));
            assert!(row.valid());
            let mut case_cache = Cache::default();
            let mut events = vec![case_event.clone()];
            case_cache.apply(
                &mut events,
                &[event_key(&case_event, &aliases)],
                &[row],
                at - 100,
                &coverage,
                true,
                &HashSet::new(),
            );
            assert_eq!(events[0].provider, "supplier-a");
            assert_eq!(case_cache.verified[&event.id].original_model, model);
            assert!(case_cache.verified[&event.id].valid());
        }
    }

    #[test]
    fn concurrent_old_and_new_native_proofs_preserve_rejection_and_conflicts() {
        let at = Local::now().timestamp_millis();
        let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
        let id = [72; 32];
        for (first_version, second_version, rejected, conflicting) in [
            (2, 4, false, false),
            (4, 2, false, false),
            (2, 4, true, false),
            (4, 2, true, false),
            (2, 4, false, true),
            (4, 2, false, true),
        ] {
            let proof = |version| Attribution {
                key: RequestMatchKey {
                    session: "session".into(),
                    model: "model".into(),
                    input: 100,
                    output: 10,
                    cached: 20,
                },
                at,
                turn: "turn".into(),
                provider: "supplier-a".into(),
                version,
                binding_since: at - 1,
                rejected: false,
                count_conflict: 0,
                original_model: if version == 4 {
                    "cpx-a/model".into()
                } else {
                    String::new()
                },
            };
            let root = tempfile::tempdir().unwrap();
            let mut storage = provider_cache::ProviderCache::default();
            storage.open(root.path(), cutoff).unwrap();
            drop(storage);
            let mut first = Cache::default();
            let mut second = Cache::default();
            first.verified.insert(id, proof(first_version));
            let mut incoming = proof(second_version);
            incoming.rejected = rejected;
            if conflicting {
                incoming.provider = "supplier-b".into();
            }
            second.verified.insert(id, incoming);
            first.save_bounded(root.path(), cutoff, &[]).unwrap();
            second.save_bounded(root.path(), cutoff, &[]).unwrap();

            let mut storage = provider_cache::ProviderCache::default();
            storage.open(root.path(), cutoff).unwrap();
            let saved = storage.load_native(cutoff).unwrap();
            assert_eq!(saved[&id].version, first_version);
            assert_eq!(saved[&id].rejected, rejected || conflicting);
        }
    }

    #[test]
    fn unknown_cache_blocks_native_matching_without_revoking_saved_evidence() {
        let at = Local::now().timestamp_millis();
        let key = RequestMatchKey {
            session: "session".into(),
            model: "model-a".into(),
            input: 100,
            output: 10,
            cached: 20,
        };
        let event = UsageEvent {
            id: [7; 32],
            session: key.session.clone(),
            turn: "turn".into(),
            model: key.model.clone(),
            timestamp: DateTime::from_timestamp_millis(at).unwrap().to_rfc3339(),
            date: Local::now().date_naive(),
            total: Tokens::new(100, 20, 10),
            delta: Tokens::new(100, 20, 10),
            has_file_baseline: false,
            last: None,
            count_uncertain: false,
            count_incomplete: false,
            provider: String::new(),
            provider_source: None,
            role: "main".into(),
            inherited: false,
            legacy_identity: None,
            context_conflicts: ContextConflicts::default(),
        };
        let aliases = RequestSessionAliases::new(&BTreeMap::new());
        let mut unknown = event.clone();
        unknown.delta.cached_known = false;
        assert!(event_key(&unknown, &aliases).is_none());
        let mut cache = Cache::default();
        cache.verified.insert(
            event.id,
            Attribution {
                key: key.clone(),
                at,
                turn: event.turn.clone(),
                provider: "supplier-a".into(),
                version: 2,
                binding_since: at - 1,
                rejected: false,
                count_conflict: 0,
                original_model: String::new(),
            },
        );
        let mut events = vec![event.clone(), unknown];
        let result = cache.apply(
            &mut events,
            &[Some((key, at)), None],
            &[],
            at - 1,
            &BTreeMap::new(),
            true,
            &HashSet::from(["session".to_string()]),
        );
        assert_eq!(result.0, 0);
        assert!(unknown_provider(&events[0].provider));
        assert!(!cache.verified[&event.id].rejected);
    }

    #[test]
    fn observed_conflict_rejects_native_proof_for_later_partial_scans() {
        let at = Local::now().timestamp_millis();
        let key = RequestMatchKey {
            session: "session".into(),
            model: "model-a".into(),
            input: 100,
            output: 10,
            cached: 20,
        };
        let event = UsageEvent {
            id: [8; 32],
            session: key.session.clone(),
            turn: "turn".into(),
            model: key.model.clone(),
            timestamp: DateTime::from_timestamp_millis(at).unwrap().to_rfc3339(),
            date: Local::now().date_naive(),
            total: Tokens::new(100, 20, 10),
            delta: Tokens::new(100, 20, 10),
            has_file_baseline: false,
            last: None,
            count_uncertain: false,
            count_incomplete: false,
            provider: String::new(),
            provider_source: None,
            role: "main".into(),
            inherited: false,
            legacy_identity: None,
            context_conflicts: ContextConflicts::default(),
        };
        let mut cache = Cache::default();
        cache.verified.insert(
            event.id,
            Attribution {
                key: key.clone(),
                at,
                turn: event.turn.clone(),
                provider: "supplier-a".into(),
                version: 2,
                binding_since: at - 1,
                rejected: false,
                count_conflict: 0,
                original_model: String::new(),
            },
        );
        let mut conflicted = event.clone();
        conflicted.context_conflicts.provider = true;
        for candidate_key in [Some((key.clone(), at)), None] {
            cache.verified.get_mut(&event.id).unwrap().rejected = false;
            cache.apply(
                &mut [conflicted.clone()],
                &[candidate_key],
                &[],
                at - 1,
                &BTreeMap::new(),
                false,
                &HashSet::new(),
            );
            assert!(cache.verified[&event.id].rejected);
        }
        let root = tempfile::tempdir().unwrap();
        let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
        let mut storage = provider_cache::ProviderCache::default();
        storage.open(root.path(), cutoff).unwrap();
        storage.save_native(&cache.verified).unwrap();
        drop(storage);
        let mut storage = provider_cache::ProviderCache::default();
        storage.open(root.path(), cutoff).unwrap();
        cache.verified = storage.load_native(cutoff).unwrap();
        let mut incomplete = vec![event.clone()];
        cache.apply(
            &mut incomplete,
            &[Some((key.clone(), at))],
            &[],
            at - 1,
            &BTreeMap::new(),
            false,
            &HashSet::new(),
        );
        eprintln!(
            "rejected={}, later_provider={}",
            cache.verified[&event.id].rejected, incomplete[0].provider
        );
        assert!(cache.verified[&event.id].rejected);
        assert_eq!(incomplete[0].provider, "");
        let mut complete = vec![event.clone()];
        cache.apply(
            &mut complete,
            &[Some((key, at))],
            &[],
            at - 1,
            &BTreeMap::new(),
            true,
            &HashSet::new(),
        );
        assert_eq!(complete[0].provider, "");
    }

    #[test]
    fn incomplete_scan_does_not_apply_or_reject_an_unconflicted_native_proof() {
        let at = Local::now().timestamp_millis();
        let key = RequestMatchKey {
            session: "session".into(),
            model: "model-a".into(),
            input: 100,
            output: 10,
            cached: 20,
        };
        let event = UsageEvent {
            id: [9; 32],
            session: key.session.clone(),
            turn: "turn".into(),
            model: key.model.clone(),
            timestamp: DateTime::from_timestamp_millis(at).unwrap().to_rfc3339(),
            date: Local::now().date_naive(),
            total: Tokens::new(100, 20, 10),
            delta: Tokens::new(100, 20, 10),
            has_file_baseline: false,
            last: None,
            count_uncertain: false,
            count_incomplete: false,
            provider: String::new(),
            provider_source: None,
            role: "main".into(),
            inherited: false,
            legacy_identity: None,
            context_conflicts: ContextConflicts::default(),
        };
        let mut cache = Cache::default();
        cache.verified.insert(
            event.id,
            Attribution {
                key: key.clone(),
                at,
                turn: event.turn.clone(),
                provider: "supplier-a".into(),
                version: 2,
                binding_since: at - 1,
                rejected: false,
                count_conflict: 0,
                original_model: String::new(),
            },
        );
        let mut partial = vec![event.clone()];
        cache.apply(
            &mut partial,
            &[Some((key.clone(), at))],
            &[],
            at - 1,
            &BTreeMap::new(),
            false,
            &HashSet::new(),
        );
        assert_eq!(partial[0].provider, "");
        assert!(!cache.verified[&event.id].rejected);
        let mut complete = vec![event];
        cache.apply(
            &mut complete,
            &[Some((key, at))],
            &[],
            at - 1,
            &BTreeMap::new(),
            true,
            &HashSet::new(),
        );
        assert_eq!(complete[0].provider, "supplier-a");
    }

    #[test]
    fn count_conflict_markers_expire_and_saturation_fails_closed() {
        let at = Local::now().timestamp_millis();
        let event = UsageEvent {
            id: [255; 32],
            session: "session".into(),
            turn: "turn".into(),
            model: "model-a".into(),
            timestamp: DateTime::from_timestamp_millis(at).unwrap().to_rfc3339(),
            date: Local::now().date_naive(),
            total: Tokens::new(100, 20, 10),
            delta: Tokens::new(100, 20, 10),
            has_file_baseline: false,
            last: None,
            count_uncertain: true,
            count_incomplete: false,
            provider: String::new(),
            provider_source: None,
            role: "main".into(),
            inherited: false,
            legacy_identity: None,
            context_conflicts: ContextConflicts::default(),
        };
        let mut cache = Cache::default();
        assert!(cache.observe_count_conflicts(&mut [event.clone()], true));
        let mut marker = cache.verified[&event.id].clone();
        assert!(marker.valid());
        let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
        marker.at = provider_cache::ProviderCache::since(cutoff) - 1;
        let root = tempfile::tempdir().unwrap();
        let mut storage = provider_cache::ProviderCache::default();
        storage.open(root.path(), cutoff).unwrap();
        storage
            .save_native(&HashMap::from([(event.id, marker.clone())]))
            .unwrap();
        drop(storage);
        let mut storage = provider_cache::ProviderCache::default();
        storage.open(root.path(), cutoff).unwrap();
        assert!(storage.load_native(cutoff).unwrap().is_empty());

        marker.at = at;
        cache.verified.clear();
        for n in 0..MAX_EVENTS {
            let mut id = [0; 32];
            id[..8].copy_from_slice(&(n as u64).to_le_bytes());
            cache.verified.insert(id, marker.clone());
        }
        let mut disputed = event;
        assert!(!cache.observe_count_conflicts(std::slice::from_mut(&mut disputed), true));
        assert!(disputed.count_uncertain);
        assert_eq!(cache.verified.len(), MAX_EVENTS);
        assert!(!cache.verified.contains_key(&disputed.id));
    }

    #[test]
    fn failed_load_retry_preserves_pending_conflicts_and_rejections() {
        let at = Local::now().timestamp_millis();
        let event = UsageEvent {
            id: [10; 32],
            session: "session".into(),
            turn: "turn".into(),
            model: "model-a".into(),
            timestamp: DateTime::from_timestamp_millis(at).unwrap().to_rfc3339(),
            date: Local::now().date_naive(),
            total: Tokens::new(100, 20, 10),
            delta: Tokens::new(100, 20, 10),
            has_file_baseline: false,
            last: None,
            count_uncertain: true,
            count_incomplete: false,
            provider: String::new(),
            provider_source: None,
            role: "main".into(),
            inherited: false,
            legacy_identity: None,
            context_conflicts: ContextConflicts::default(),
        };
        let proof = Attribution {
            key: RequestMatchKey {
                session: event.session.clone(),
                model: event.model.clone(),
                input: 100,
                output: 10,
                cached: 20,
            },
            at,
            turn: event.turn.clone(),
            provider: "supplier-a".into(),
            version: 2,
            binding_since: at - 1,
            rejected: false,
            count_conflict: 0,
            original_model: String::new(),
        };
        assert!(
            serde_json::to_value(&proof)
                .unwrap()
                .get("count_conflict")
                .is_none()
        );
        let mut cache = Cache::default();
        assert!(cache.observe_count_conflicts(&mut [event.clone()], true));
        cache.restore_after_failed_load(HashMap::from([(event.id, proof.clone())]));
        assert_eq!(cache.verified[&event.id].count_conflict, 2);
        assert_eq!(cache.verified[&event.id].version, 3);

        let original_conflict = cache.verified[&event.id].clone();
        let mut inherited_conflict = original_conflict.clone();
        inherited_conflict.count_conflict = 1;
        cache.verified.insert(event.id, inherited_conflict);
        cache.restore_after_failed_load(HashMap::from([(event.id, original_conflict)]));
        assert_eq!(cache.verified[&event.id].count_conflict, 2);

        let mut rejected = proof.clone();
        rejected.rejected = true;
        cache.verified.insert(event.id, rejected);
        cache.restore_after_failed_load(HashMap::from([(event.id, proof)]));
        assert!(cache.verified[&event.id].rejected);
    }

    #[test]
    fn conflicting_duplicate_cannot_restore_cached_native_attribution() {
        let at = Local::now().timestamp_millis();
        let key = RequestMatchKey {
            session: "session".into(),
            model: "model-a".into(),
            input: 100,
            output: 10,
            cached: 20,
        };
        let event = UsageEvent {
            id: [7; 32],
            session: key.session.clone(),
            turn: "turn".into(),
            model: key.model.clone(),
            timestamp: DateTime::from_timestamp_millis(at).unwrap().to_rfc3339(),
            date: Local::now().date_naive(),
            total: Tokens::new(100, 20, 10),
            delta: Tokens::new(100, 20, 10),
            has_file_baseline: false,
            last: None,
            count_uncertain: false,
            count_incomplete: false,
            provider: String::new(),
            provider_source: None,
            role: "main".into(),
            inherited: false,
            legacy_identity: None,
            context_conflicts: ContextConflicts::default(),
        };
        for conflicted in [false, true] {
            let mut cache = Cache::default();
            cache.verified.insert(
                event.id,
                Attribution {
                    key: key.clone(),
                    at,
                    turn: event.turn.clone(),
                    provider: "supplier-a".into(),
                    version: 2,
                    binding_since: at - 1,
                    rejected: false,
                    count_conflict: 0,
                    original_model: String::new(),
                },
            );
            let mut candidate = event.clone();
            candidate.context_conflicts.provider = conflicted;
            let mut events = vec![candidate];
            let result = cache.apply(
                &mut events,
                &[Some((key.clone(), at))],
                &[],
                at - 1,
                &BTreeMap::new(),
                true,
                &HashSet::new(),
            );
            assert_eq!(
                events[0].provider,
                if conflicted { "" } else { "supplier-a" }
            );
            assert_eq!(result.0, usize::from(!conflicted));
        }
    }
}
