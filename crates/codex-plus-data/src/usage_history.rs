//! 本机用量索引。只解析 rollout 的用量与轮次元数据；费用仅按用户价格估算。

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, NaiveDate};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::{self, File, Metadata, ReadDir};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

const READ_BUDGET: u64 = 8 * 1024 * 1024;
const FILE_READ_BUDGET: u64 = 512 * 1024;
const MAX_LINE: usize = 256 * 1024;
const MAX_FILES: usize = 4096;
const MAX_RECORDS: usize = 20_000;
const MAX_TOTAL_RECORDS: usize = 100_000;
const DISCOVERY_BUDGET: usize = 4096;
const MAX_HOMES: usize = 2;
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;
static PENDING_BYTES: AtomicUsize = AtomicUsize::new(0);
static CACHE: OnceLock<Mutex<HistoryCache>> = OnceLock::new();

#[derive(Default)]
struct HistoryCache {
    homes: HashMap<PathBuf, CachedHome>,
    reaper_running: bool,
}
struct CachedHome {
    index: Arc<Mutex<HistoryIndex>>,
    touched: Instant,
}
impl HistoryCache {
    fn expire(&mut self) {
        self.homes.retain(|_, home| {
            home.touched.elapsed() < IDLE_TIMEOUT || Arc::strong_count(&home.index) > 1
        });
    }
    fn get(&mut self, home: PathBuf) -> Arc<Mutex<HistoryIndex>> {
        self.expire();
        if !self.homes.contains_key(&home) && self.homes.len() >= MAX_HOMES {
            if let Some(oldest) = self
                .homes
                .iter()
                .min_by_key(|(_, h)| h.touched)
                .map(|(p, _)| p.clone())
            {
                self.homes.remove(&oldest);
            }
        }
        let entry = self.homes.entry(home).or_insert_with(|| CachedHome {
            index: Arc::new(Mutex::new(HistoryIndex::new())),
            touched: Instant::now(),
        });
        entry.touched = Instant::now();
        Arc::clone(&entry.index)
    }
}

// 仅在首次查询后启动内存回收；不轮询文件，缓存清空后线程也退出。
fn expire_idle() {
    loop {
        let delay = {
            let Ok(mut cache) = CACHE.get().unwrap().lock() else {
                return;
            };
            cache.expire();
            if cache.homes.is_empty() {
                cache.reaper_running = false;
                return;
            }
            cache
                .homes
                .values()
                .map(|h| IDLE_TIMEOUT.saturating_sub(h.touched.elapsed()))
                .min()
                .unwrap_or(IDLE_TIMEOUT)
                .max(Duration::from_secs(1))
        };
        std::thread::sleep(delay);
    }
}

/// 只释放内存，不修改会话文件。
pub fn clear_history(codex_home: &Path) {
    if let Some(cache) = CACHE.get() {
        if let Ok(mut cache) = cache.lock() {
            if let Ok(home) = fs::canonicalize(codex_home) {
                cache.homes.remove(&home);
            }
        }
    }
}

pub(crate) fn normalize_session_id(id: &str) -> Option<String> {
    let id = id.strip_prefix("local:").unwrap_or(id);
    if id.len() != 36 {
        return None;
    }
    let normalized = uuid::Uuid::parse_str(id).ok()?.to_string();
    (normalized.eq_ignore_ascii_case(id)).then_some(normalized)
}

/// 查询来自受信任启动器的 Codex home；请求中不接受任何文件路径。
/// 每次扫描和读取均有预算，初次大目录返回 partial，后续请求从偏移续读。
pub fn query_history(codex_home: &Path, query: &Value) -> Value {
    let now = Local::now();
    let Some(options) = Query::parse(query, now.date_naive()) else {
        return json!({"status":"unavailable","complete":false,"message":"用量查询参数无效","records":[],"updatedAt":now.timestamp_millis()});
    };
    let Ok(home) = fs::canonicalize(codex_home) else {
        return json!({"status":"unavailable","complete":false,"message":"本机会话目录尚不可用","records":[],"updatedAt":now.timestamp_millis()});
    };
    let Ok(mut cache) = CACHE
        .get_or_init(|| Mutex::new(HistoryCache::default()))
        .lock()
    else {
        return json!({"status":"unavailable","complete":false,"message":"用量索引暂不可用","records":[],"updatedAt":now.timestamp_millis()});
    };
    let shared = cache.get(home.clone());
    if !cache.reaper_running {
        cache.reaper_running = std::thread::Builder::new()
            .name("usage-idle-release".to_owned())
            .spawn(expire_idle)
            .is_ok();
    }
    drop(cache);
    let Ok(mut index) = shared.lock() else {
        return json!({"status":"unavailable","complete":false,"message":"用量索引暂不可用","records":[],"updatedAt":now.timestamp_millis()});
    };
    index.update(&home);
    index.cached_response(&options, now.date_naive(), now.timestamp_millis())
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Tokens {
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) total_tokens: u64,
    reasoning_output_tokens: Option<u64>,
}
impl Tokens {
    fn new(input: u64, cached: u64, output: u64) -> Self {
        Self {
            input_tokens: input,
            cached_input_tokens: cached.min(input),
            output_tokens: output,
            total_tokens: input.saturating_add(output),
            reasoning_output_tokens: None,
        }
    }
    pub(crate) fn parse(value: &Value) -> Option<Self> {
        let mut tokens = Self::new(
            value["input_tokens"].as_u64()?,
            value["cached_input_tokens"].as_u64().unwrap_or(0),
            value["output_tokens"].as_u64()?,
        );
        tokens.reasoning_output_tokens = value["reasoning_output_tokens"]
            .as_u64()
            .map(|n| n.min(tokens.output_tokens));
        Some(tokens)
    }
    fn add(&mut self, other: Self) {
        let reasoning = if self.total_tokens == 0 {
            other.reasoning_output_tokens
        } else {
            self.reasoning_output_tokens
                .zip(other.reasoning_output_tokens)
                .map(|(a, b)| a.saturating_add(b))
        };
        *self = Self::new(
            self.input_tokens.saturating_add(other.input_tokens),
            self.cached_input_tokens
                .saturating_add(other.cached_input_tokens),
            self.output_tokens.saturating_add(other.output_tokens),
        );
        self.reasoning_output_tokens = reasoning;
    }
    fn delta(self, before: Self) -> Self {
        let mut tokens = Self::new(
            self.input_tokens.saturating_sub(before.input_tokens),
            self.cached_input_tokens
                .saturating_sub(before.cached_input_tokens),
            self.output_tokens.saturating_sub(before.output_tokens),
        );
        tokens.reasoning_output_tokens = self
            .reasoning_output_tokens
            .zip(before.reasoning_output_tokens)
            .map(|(a, b)| a.saturating_sub(b).min(tokens.output_tokens));
        tokens
    }

    fn is_monotonic_after(self, before: Self) -> bool {
        self.input_tokens >= before.input_tokens
            && self.output_tokens >= before.output_tokens
            && self.cached_input_tokens >= before.cached_input_tokens
            && self
                .reasoning_output_tokens
                .zip(before.reasoning_output_tokens)
                .is_none_or(|(next, previous)| next >= previous)
    }

    pub(crate) fn usage_delta(self, before: Option<Self>, last: Option<Self>) -> Self {
        match before {
            None => last.unwrap_or(self),
            // 子计数修正不是新消费，即使带着旧 last_usage 也不能重放。
            Some(before)
                if self.input_tokens == before.input_tokens
                    && self.output_tokens == before.output_tokens =>
            {
                Self::default()
            }
            Some(before) if self.is_monotonic_after(before) => self.delta(before),
            // 分量回退只能确认最后一次消费；缺失时不能回放整个累计值。
            Some(_) => last.unwrap_or_default(),
        }
    }
}

#[derive(Default)]
pub(crate) struct UsageHistoryBoundary {
    pub(crate) parent: Option<String>,
    start: Option<u64>,
}

impl UsageHistoryBoundary {
    pub(crate) fn from_meta(payload: &Value, session: &str) -> Self {
        Self {
            parent: payload["forked_from_id"]
                .as_str()
                .or_else(|| payload["parent_thread_id"].as_str())
                .and_then(normalize_session_id)
                .filter(|parent| parent != session),
            start: payload["subagent_history_start_ordinal"].as_u64(),
        }
    }

    pub(crate) fn contains(&self, ordinal: Option<u64>) -> bool {
        self.parent.is_some()
            && self
                .start
                .zip(ordinal)
                .is_some_and(|(start, ordinal)| ordinal < start)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    id: String,
    session_id: String,
    turn_id: String,
    model: Option<String>,
    started_at: Option<String>,
    updated_at: Option<String>,
    #[serde(skip_serializing)]
    date: Option<NaiveDate>,
    status: &'static str,
    usage: Option<Tokens>,
    #[serde(skip)]
    observations: Vec<UsageObservation>,
}

#[derive(Clone)]
struct UsageObservation {
    sequence: u64,
    ordinal: Option<u64>,
    timestamp: Option<i64>,
    before: Option<Tokens>,
    total: Tokens,
    last: Option<Tokens>,
    continuation: bool,
}

impl Record {
    fn detail_size(&self) -> usize {
        1 + self.observations.len()
    }
}

#[derive(Clone, PartialEq)]
struct Price {
    currency: String,
    input: f64,
    cached: f64,
    output: f64,
}
#[derive(Clone, Serialize)]
struct Cost {
    currency: String,
    amount: f64,
}
impl Price {
    fn estimate(&self, usage: Tokens) -> Cost {
        let amount = ((usage.input_tokens - usage.cached_input_tokens) as f64 * self.input
            + usage.cached_input_tokens as f64 * self.cached
            + usage.output_tokens as f64 * self.output)
            / 1_000_000.0;
        Cost {
            currency: self.currency.clone(),
            amount: (amount * 100_000_000.0).round() / 100_000_000.0,
        }
    }
}

#[derive(Clone, PartialEq)]
struct Query {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    model: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    search: String,
    page: usize,
    page_size: usize,
    include_session_cursors: bool,
    prices: HashMap<String, Price>,
}
impl Query {
    fn parse(value: &Value, today: NaiveDate) -> Option<Self> {
        if !value.is_object() {
            return None;
        }
        if value.as_object()?.keys().any(|key| {
            !matches!(
                key.as_str(),
                "period"
                    | "from"
                    | "to"
                    | "model"
                    | "sessionId"
                    | "turnId"
                    | "search"
                    | "page"
                    | "pageSize"
                    | "limit"
                    | "includeSessionCursors"
                    | "prices"
            )
        }) {
            return None;
        }
        let mut from = match value["period"].as_str().unwrap_or("all") {
            "today" => Some(today),
            "week" => Some(today - ChronoDuration::days(6)),
            "month" => today.with_day(1),
            "all" => None,
            _ => return None,
        };
        let mut to = from.map(|_| today);
        for (name, field) in [("from", &mut from), ("to", &mut to)] {
            if !value[name].is_null() {
                *field = Some(NaiveDate::parse_from_str(value[name].as_str()?, "%Y-%m-%d").ok()?);
            }
        }
        if from.zip(to).is_some_and(|(f, t)| f > t) {
            return None;
        }
        let model = value["model"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if model.as_ref().is_some_and(|s| s.len() > 200) {
            return None;
        }
        let session_id = if value["sessionId"].is_null() {
            None
        } else {
            Some(normalize_session_id(value["sessionId"].as_str()?)?)
        };
        let turn_id = if value["turnId"].is_null() {
            None
        } else {
            Some(value["turnId"].as_str()?.to_owned())
        };
        if turn_id
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 200)
        {
            return None;
        }
        let search = value["search"].as_str().unwrap_or("").trim().to_lowercase();
        if search.len() > 200 {
            return None;
        }
        let page = value["page"].as_u64().unwrap_or(1).clamp(1, 100_000) as usize;
        let page_size = value["pageSize"]
            .as_u64()
            .or_else(|| value["limit"].as_u64())
            .unwrap_or(50)
            .clamp(1, 500) as usize;
        let include_session_cursors = match value.get("includeSessionCursors") {
            None => false,
            Some(value) => value.as_bool()?,
        };
        let mut prices = HashMap::new();
        if let Some(rows) = value["prices"].as_array() {
            if rows.len() > 1024 {
                return None;
            }
            for row in rows {
                let name = row["model"].as_str()?.trim();
                let currency = row["currency"].as_str().unwrap_or("").to_ascii_uppercase();
                // 价目表可逐项填写。缺项保持未知，不让未填缓存价导致整份用量不可读。
                if name.is_empty()
                    || currency.is_empty()
                    || ["input", "cachedInput", "output"]
                        .iter()
                        .any(|key| row[key].is_null())
                {
                    continue;
                }
                if name.is_empty()
                    || name.len() > 200
                    || currency.len() != 3
                    || !currency.bytes().all(|c| c.is_ascii_uppercase())
                {
                    return None;
                }
                let amount = |key: &str| {
                    row[key]
                        .as_f64()
                        .filter(|n| n.is_finite() && (0.0..=1e9).contains(n))
                };
                prices.insert(
                    name.to_owned(),
                    Price {
                        currency,
                        input: amount("input")?,
                        cached: amount("cachedInput")?,
                        output: amount("output")?,
                    },
                );
            }
        }
        Some(Self {
            from,
            to,
            model,
            session_id,
            turn_id,
            search,
            page,
            page_size,
            include_session_cursors,
            prices,
        })
    }
    fn price_for(&self, model: &str) -> Option<&Price> {
        if unknown_model(model) {
            return None;
        }
        if let Some(price) = self.prices.get(model) {
            return Some(price);
        }
        let name = model.to_lowercase();
        self.prices
            .iter()
            .filter(|(key, _)| {
                key.eq_ignore_ascii_case(model)
                    || key.len() >= 3 && name.contains(&key.to_lowercase())
            })
            .max_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| b.cmp(a)))
            .map(|(_, price)| price)
    }
    fn accepts(&self, record: &Record) -> bool {
        if self
            .session_id
            .as_ref()
            .is_some_and(|id| id != &record.session_id)
            || self
                .turn_id
                .as_ref()
                .is_some_and(|id| id != &record.turn_id)
        {
            return false;
        }
        if self
            .from
            .is_some_and(|from| record.date.is_none_or(|d| d < from))
            || self.to.is_some_and(|to| record.date.is_none_or(|d| d > to))
        {
            return false;
        }
        if !self.accepts_model(record.model.as_deref().unwrap_or_default()) {
            return false;
        }
        self.search.is_empty()
            || record
                .model
                .as_deref()
                .unwrap_or("unknown")
                .to_lowercase()
                .contains(&self.search)
            || record
                .date
                .is_some_and(|d| d.to_string().contains(&self.search))
    }
    fn accepts_model(&self, model: &str) -> bool {
        self.model.as_ref().is_none_or(|selected| {
            if unknown_model(selected) {
                unknown_model(model)
            } else {
                selected == model
            }
        })
    }
}

fn unknown_model(model: &str) -> bool {
    let model = model.trim();
    model.is_empty()
        || model.eq_ignore_ascii_case("unknown")
        || model.eq_ignore_ascii_case("unattributed")
}

#[derive(Default)]
struct Archived {
    usage: Option<Tokens>,
    record_count: usize,
    unknown_usage_records: usize,
    unknown_model_records: usize,
}
impl Archived {
    fn add(&mut self, record: &Record) {
        self.record_count += 1;
        self.unknown_model_records += usize::from(record.model.is_none());
        if let Some(usage) = record.usage {
            self.usage.get_or_insert_with(Tokens::default).add(usage);
        } else {
            self.unknown_usage_records += 1;
        }
    }
}

#[derive(Default)]
struct Aggregate {
    usage: Option<Tokens>,
    records: usize,
    unknown: usize,
    unknown_model: usize,
    unpriced: usize,
    costs: BTreeMap<String, (f64, usize)>,
}
impl Aggregate {
    fn add(&mut self, record: &Record, price: Option<&Price>) {
        self.records += 1;
        self.unknown_model += usize::from(record.model.is_none());
        if let Some(usage) = record.usage {
            self.usage.get_or_insert_with(Tokens::default).add(usage);
            if let Some(price) = price {
                let cost = price.estimate(usage);
                let value = self.costs.entry(cost.currency).or_default();
                value.0 += cost.amount;
                value.1 += 1;
            } else {
                self.unpriced += 1;
            }
        } else {
            self.unknown += 1;
            self.unpriced += 1;
        }
    }
    fn add_archive(&mut self, archived: &Archived, price: Option<&Price>) {
        self.records += archived.record_count;
        self.unknown += archived.unknown_usage_records;
        self.unknown_model += archived.unknown_model_records;
        if let Some(usage) = archived.usage {
            self.usage.get_or_insert_with(Tokens::default).add(usage);
            if let Some(price) = price {
                let cost = price.estimate(usage);
                let value = self.costs.entry(cost.currency).or_default();
                value.0 += cost.amount;
                value.1 += archived.record_count - archived.unknown_usage_records;
                self.unpriced += archived.unknown_usage_records;
            } else {
                self.unpriced += archived.record_count;
            }
        } else {
            self.unpriced += archived.record_count;
        }
    }
    fn value(&self) -> Value {
        let costs: Vec<_> = self.costs.iter().map(|(currency,(amount, count))| json!({"currency":currency,"amount":(amount*1e8).round()/1e8,"pricedRecords":count})).collect();
        json!({"usage":self.usage,"recordCount":self.records,"unknownUsageRecords":self.unknown,"unknownModelRecords":self.unknown_model,"unpricedRecords":self.unpriced,"estimatedCosts":costs})
    }
}

struct DirectoryCursor {
    depth: usize,
    entries: ReadDir,
}
struct HistoryIndex {
    files: BTreeMap<PathBuf, FileIndex>,
    discovery: Vec<DirectoryCursor>,
    discovering: bool,
    roots: Vec<PathBuf>,
    last_scan: Instant,
    next_file: usize,
    cached: Option<(Query, NaiveDate, Value)>,
    retained_on: NaiveDate,
    truncated: bool,
    discovery_errors: bool,
}
impl HistoryIndex {
    fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            discovery: Vec::new(),
            discovering: false,
            roots: Vec::new(),
            last_scan: Instant::now() - Duration::from_secs(60),
            next_file: 0,
            cached: None,
            retained_on: Local::now().date_naive(),
            truncated: false,
            discovery_errors: false,
        }
    }
    fn update(&mut self, home: &Path) {
        let scan_before = (self.discovering, self.truncated, self.discovery_errors);
        let mut changed = false;
        if !self.discovering && self.last_scan.elapsed() >= Duration::from_secs(10) {
            self.roots = ["sessions", "archived_sessions"]
                .into_iter()
                .filter_map(|p| fs::canonicalize(home.join(p)).ok())
                .filter(|p| p.starts_with(home))
                .collect();
            self.discovery = self
                .roots
                .iter()
                .filter_map(|p| {
                    fs::read_dir(p)
                        .ok()
                        .map(|entries| DirectoryCursor { depth: 0, entries })
                })
                .collect();
            self.discovering = true;
            self.discovery_errors = false;
            self.last_scan = Instant::now();
        }
        for _ in 0..DISCOVERY_BUDGET {
            let Some(cursor) = self.discovery.last_mut() else {
                self.discovering = false;
                break;
            };
            let depth = cursor.depth;
            let Some(entry) = cursor.entries.next() else {
                self.discovery.pop();
                continue;
            };
            let Ok(entry) = entry else {
                self.discovery_errors = true;
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                self.discovery_errors = true;
                continue;
            };
            if kind.is_dir() && depth < 4 {
                if let Ok(entries) = fs::read_dir(entry.path()) {
                    self.discovery.push(DirectoryCursor {
                        depth: depth + 1,
                        entries,
                    });
                } else {
                    self.discovery_errors = true;
                }
            } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "jsonl") {
                let Ok(path) = fs::canonicalize(entry.path()) else {
                    continue;
                };
                if !self.roots.iter().any(|root| path.starts_with(root)) {
                    continue;
                }
                if !self.files.contains_key(&path) {
                    if rollout_id(&path).is_none() {
                        continue;
                    }
                    if self.files.len() >= MAX_FILES {
                        self.truncated = true;
                        continue;
                    }
                    self.files.insert(path, FileIndex::new());
                    changed = true;
                }
            }
        }
        changed |= scan_before != (self.discovering, self.truncated, self.discovery_errors);
        let paths: Vec<_> = self.files.keys().cloned().collect();
        if paths.is_empty() {
            if changed {
                self.cached = None;
            }
            return;
        }
        let mut budget = READ_BUDGET;
        let start = self.next_file;
        for step in 0..paths.len() {
            let position = (start + step) % paths.len();
            // 每文件只取一次元数据；内容变化时再做规范路径及打开后身份校验。
            let metadata = fs::symlink_metadata(&paths[position]);
            if matches!(&metadata, Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
                self.files.remove(&paths[position]);
                changed = true;
                continue;
            }
            let file = self.files.get_mut(&paths[position]).unwrap();
            let before = file.revision();
            let allowed = budget.min(FILE_READ_BUDGET);
            match metadata.and_then(|meta| file.read_observed(&paths[position], allowed, &meta)) {
                Ok(consumed) => budget = budget.saturating_sub(consumed),
                Err(_) => file.unreadable = true,
            }
            changed |= before != file.revision();
            self.next_file = (position + 1) % paths.len();
            if budget == 0 {
                break;
            }
        }
        let count: usize = self.files.values().map(|file| file.detail_size).sum();
        if count > MAX_TOTAL_RECORDS {
            changed = true;
            let mut remove = count - MAX_TOTAL_RECORDS;
            for file in self.files.values_mut() {
                while remove > 0 {
                    let Some(record) = file.records.pop_front() else {
                        break;
                    };
                    remove = remove.saturating_sub(record.detail_size());
                    file.archive_record(record);
                }
                if remove == 0 {
                    break;
                }
            }
        }
        if changed {
            self.cached = None;
        }
    }
    fn cached_response(&mut self, query: &Query, today: NaiveDate, now: i64) -> Value {
        // 保留期随日期推进，与文件是否变化无关；归档只收拢明细，累计总量保留。
        if self.retained_on != today {
            for file in self.files.values_mut() {
                file.retain_recent(today);
            }
            self.retained_on = today;
            self.cached = None;
        }
        if self
            .cached
            .as_ref()
            .is_none_or(|(old, date, _)| old != query || *date != today)
        {
            self.cached = Some((query.clone(), today, self.response(query, today, now)));
        }
        let mut response = self.cached.as_ref().unwrap().2.clone();
        response["updatedAt"] = json!(now);
        // 时间状态不依赖文件变化，缓存命中时仍按当前时间更新。
        for field in ["rateLimits", "machineSummary"] {
            let limits = if field == "rateLimits" {
                response.get_mut(field)
            } else {
                response.get_mut(field).and_then(|v| v.get_mut("windows"))
            };
            if let Some(limits) = limits.and_then(Value::as_array_mut) {
                for limit in limits {
                    limit["expired"] =
                        json!(limit["resetAt"].as_i64().is_some_and(|t| t <= now / 1000));
                }
            }
        }
        if let Some(windows) = response["codex"]["windows"].as_object_mut() {
            for window in windows.values_mut() {
                window["expired"] = json!(window["resetAt"].as_i64().is_some_and(|t| t <= now));
            }
        }
        response["codex"]["stale"] = json!(
            response["machineSummary"]["rateLimitsTs"]
                .as_i64()
                .is_none_or(|t| now / 1000 - t > 120)
        );
        response
    }
    fn response(&self, query: &Query, today: NaiveDate, now: i64) -> Value {
        // 同一会话可有多个分页分段；仅合并用量流完全相同的物理副本。
        let mut effective: BTreeMap<(&str, [u8; 32]), &FileIndex> = BTreeMap::new();
        for file in self.files.values().filter(|f| f.verified) {
            let id = (
                file.session.as_deref().unwrap(),
                file.usage_digest.clone().finalize().into(),
            );
            if effective.get(&id).is_none_or(|old| {
                file.modified > old.modified
                    || file.modified == old.modified && file.offset > old.offset
            }) {
                effective.insert(id, file);
            }
        }
        let mut record_map: BTreeMap<String, Record> = BTreeMap::new();
        let mut observations = BTreeMap::new();
        let mut successors: BTreeMap<_, HashSet<_>> = BTreeMap::new();
        let mut observed_rows = HashSet::new();
        let mut limits = Vec::new();
        let mut latest_limit = i64::MIN;
        let mut plan_type = None;
        let mut pending = 0;
        let mut indexed = 0;
        let mut ignored = 0;
        let mut identity_mismatches = 0;
        let mut incomplete_records = false;
        let mut archived_records = 0;
        for file in self.files.values() {
            if file.catching_up || file.unreadable {
                pending += 1;
            } else if file.verified {
                indexed += 1;
            } else {
                ignored += 1;
            }
            identity_mismatches += usize::from(file.verified && file.session != file.expected_id);
            incomplete_records |= file.truncated
                || file.invalid_usage
                || file.pending.len() > 0
                || file.skipping
                || !file.verified;
            if effective
                .values()
                .any(|candidate| std::ptr::eq(*candidate, file))
            {
                archived_records += file
                    .archived_all
                    .values()
                    .map(|a| a.record_count)
                    .sum::<usize>();
                incomplete_records |= file.undated_archived > 0;
                let mut file_order = Vec::new();
                for record in &file.records {
                    let replace = record_map.get(&record.id).is_none_or(|previous| {
                        record_timestamp(record) > record_timestamp(previous)
                            || record_timestamp(record) == record_timestamp(previous)
                                && record.usage.map(|u| u.total_tokens)
                                    > previous.usage.map(|u| u.total_tokens)
                    });
                    if replace {
                        let mut row = record.clone();
                        row.observations.clear();
                        if !record.observations.is_empty() {
                            row.usage = None;
                        }
                        record_map.insert(record.id.clone(), row);
                    }
                    for observation in &record.observations {
                        observed_rows.insert(record.id.as_str());
                        let key = (
                            record.session_id.as_str(),
                            observation.timestamp,
                            observation.ordinal,
                            observation.total.input_tokens,
                            observation.total.output_tokens,
                            observation.total.cached_input_tokens,
                            observation.total.reasoning_output_tokens,
                            record.turn_id.as_str(),
                        );
                        file_order.push((observation.sequence, key));
                        let candidate =
                            observations
                                .entry(key)
                                .or_insert((record, observation, false));
                        candidate.2 |= candidate.0.model.is_some()
                            && record.model.is_some()
                            && candidate.0.model != record.model;
                        if candidate.0.model.is_none() && record.model.is_some() {
                            candidate.0 = record;
                        }
                        if observation.before.is_some_and(|before| {
                            candidate.1.before.is_none()
                                || observation.total.is_monotonic_after(before)
                                    && candidate
                                        .1
                                        .before
                                        .is_some_and(|old| before.is_monotonic_after(old))
                        }) {
                            candidate.1 = observation;
                        }
                    }
                }
                file_order.sort_unstable_by_key(|(sequence, _)| *sequence);
                for pair in file_order.windows(2) {
                    if pair[0].1 != pair[1].1 {
                        successors.entry(pair[0].1).or_default().insert(pair[1].1);
                    }
                }
                if file.limits_observed > latest_limit {
                    latest_limit = file.limits_observed;
                    limits = file.rate_limits.clone();
                    plan_type = file.plan_type.clone();
                }
            }
        }
        let effective: Vec<_> = effective.into_values().collect();
        let mut source_counts = BTreeMap::new();
        for file in &effective {
            *source_counts
                .entry(file.session.as_deref().unwrap())
                .or_insert(0usize) += 1;
        }
        let compacted_overlap: HashSet<_> = effective
            .iter()
            .filter(|file| !file.archived_all.is_empty())
            .map(|file| file.session.as_deref().unwrap())
            .filter(|session| source_counts[session] > 1)
            .collect();
        // 超过明细限额后无法证明跨文件旧汇总互不重叠，只返回可去重的观测下界。
        incomplete_records |= !compacted_overlap.is_empty();
        // 每份文件内部的观测先后比时钟或跨文件 ordinal 更可靠；共享事件连接各分页。
        fn component_root(roots: &mut [usize], mut index: usize) -> usize {
            while roots[index] != index {
                roots[index] = roots[roots[index]];
                index = roots[index];
            }
            index
        }
        let mut incoming: BTreeMap<_, _> = observations
            .keys()
            .enumerate()
            .map(|(index, key)| (*key, (0usize, index)))
            .collect();
        let mut roots: Vec<_> = (0..incoming.len()).collect();
        for (previous, nexts) in &successors {
            for next in nexts {
                let from = component_root(&mut roots, incoming[previous].1);
                let to = component_root(&mut roots, incoming[next].1);
                roots[to] = from;
                incoming.get_mut(next).unwrap().0 += 1;
            }
        }
        let mut ready: BTreeSet<_> = incoming
            .iter()
            .filter_map(|(key, (count, _))| (*count == 0).then_some(*key))
            .collect();
        let mut ordered = Vec::new();
        while let Some(key) = ready.pop_first() {
            let component = component_root(&mut roots, incoming[&key].1);
            ordered.push((observations.remove(&key).unwrap(), component));
            for next in successors.get(&key).into_iter().flatten() {
                let count = &mut incoming.get_mut(next).unwrap().0;
                *count -= 1;
                if *count == 0 {
                    ready.insert(*next);
                }
            }
        }
        // 相互矛盾的物理来源不能强行排序后宣称完整。
        incomplete_records |= !observations.is_empty();
        let mut cursors = BTreeMap::new();
        let mut stream_cursors = BTreeMap::new();
        let mut selected_rows = HashSet::new();
        for ((source, observation, model_conflict), component) in ordered {
            // 只有共享原生观测证明相连的计数流，才可共用累计水位。
            let previous = stream_cursors.get(&component).copied().or_else(|| {
                (observation.continuation && observation.last.is_none())
                    .then(|| cursors.get(source.session_id.as_str()).copied())
                    .flatten()
            });
            let before = if observation.before.is_none() && observation.last.is_some() {
                None
            } else {
                previous
                    .filter(|previous| {
                        observation.total.is_monotonic_after(*previous)
                            && observation
                                .before
                                .is_none_or(|before| previous.is_monotonic_after(before))
                    })
                    .or(observation.before)
            };
            let delta = if compacted_overlap.contains(source.session_id.as_str()) {
                // 已丢失重叠依据时，只累计每条唯一观测明确提供的末次消费。
                observation.last
            } else if before.is_none() && observation.last.is_none() && observation.continuation {
                incomplete_records = true;
                None
            } else {
                Some(observation.total.usage_delta(before, observation.last))
            };
            stream_cursors.insert(component, observation.total);
            cursors.insert(source.session_id.as_str(), observation.total);
            let mut row = source.clone();
            row.observations.clear();
            if model_conflict {
                incomplete_records = true;
                row.model = None;
                row.id = format!(
                    "{}:{}::{}",
                    row.session_id,
                    row.turn_id,
                    row.date.map(|date| date.to_string()).unwrap_or_default()
                );
            }
            let row = record_map.entry(row.id.clone()).or_insert_with(|| {
                row.usage = None;
                row
            });
            selected_rows.insert(row.id.clone());
            if let Some(delta) = delta {
                row.usage.get_or_insert_with(Tokens::default).add(delta);
            }
        }
        record_map
            .retain(|id, _| !observed_rows.contains(id.as_str()) || selected_rows.contains(id));
        let mut records: Vec<_> = record_map.into_values().collect();
        records.sort_by(|a, b| {
            record_timestamp(b)
                .cmp(&record_timestamp(a))
                .then_with(|| b.id.cmp(&a.id))
        });
        let week = today - ChronoDuration::days(6);
        let month = today.with_day(1).unwrap();
        let detail_cutoff = today - ChronoDuration::days(89);
        let daily_cutoff = today - ChronoDuration::days(364);
        archived_records += records
            .iter()
            .filter(|record| record.date.is_some_and(|date| date < detail_cutoff))
            .count();
        let mut periods: BTreeMap<&str, Aggregate> = ["today", "week", "month", "all"]
            .into_iter()
            .map(|s| (s, Aggregate::default()))
            .collect();
        let mut all_models: BTreeMap<String, Aggregate> = BTreeMap::new();
        let mut all_days: BTreeMap<NaiveDate, Aggregate> = BTreeMap::new();
        let mut day_models: BTreeMap<NaiveDate, BTreeMap<String, Aggregate>> = BTreeMap::new();
        for record in &records {
            let price = record.model.as_ref().and_then(|m| query.price_for(m));
            periods.get_mut("all").unwrap().add(record, price);
            let model = record.model.clone().unwrap_or_default();
            all_models
                .entry(model.clone())
                .or_default()
                .add(record, price);
            if let Some(day) = record.date {
                if day < daily_cutoff {
                    continue;
                }
                all_days.entry(day).or_default().add(record, price);
                day_models
                    .entry(day)
                    .or_default()
                    .entry(model)
                    .or_default()
                    .add(record, price);
                for (name, from) in [("today", today), ("week", week), ("month", month)] {
                    if day >= from && day <= today {
                        periods.get_mut(name).unwrap().add(record, price);
                    }
                }
            } else {
                incomplete_records = true;
            }
        }
        for file in &effective {
            if compacted_overlap.contains(file.session.as_deref().unwrap()) {
                continue;
            }
            for (model, archive) in &file.archived_all {
                let price = query.price_for(model);
                periods.get_mut("all").unwrap().add_archive(archive, price);
                all_models
                    .entry(model.clone())
                    .or_default()
                    .add_archive(archive, price);
            }
            for ((day, model), archive) in &file.archived_days {
                let price = query.price_for(model);
                all_days
                    .entry(*day)
                    .or_default()
                    .add_archive(archive, price);
                day_models
                    .entry(*day)
                    .or_default()
                    .entry(model.clone())
                    .or_default()
                    .add_archive(archive, price);
                for (name, from) in [("today", today), ("week", week), ("month", month)] {
                    if *day >= from && *day <= today {
                        periods.get_mut(name).unwrap().add_archive(archive, price);
                    }
                }
            }
        }
        let selected: Vec<_> = records
            .iter()
            .filter(|r| r.date.is_none_or(|date| date >= detail_cutoff) && query.accepts(r))
            .collect();
        let mut models: BTreeMap<String, Aggregate> = BTreeMap::new();
        let mut days: BTreeMap<NaiveDate, Aggregate> = BTreeMap::new();
        let mut selected_day_models: BTreeMap<NaiveDate, BTreeMap<String, Aggregate>> =
            BTreeMap::new();
        for record in records.iter().filter(|record| {
            query.accepts(record)
                && (query.from.is_none() && query.to.is_none()
                    || record.date.is_none_or(|date| date >= daily_cutoff))
        }) {
            let price = record.model.as_ref().and_then(|m| query.price_for(m));
            models
                .entry(record.model.clone().unwrap_or_default())
                .or_default()
                .add(record, price);
            if let Some(day) = record.date {
                if day < daily_cutoff {
                    continue;
                }
                days.entry(day).or_default().add(record, price);
                selected_day_models
                    .entry(day)
                    .or_default()
                    .entry(record.model.clone().unwrap_or_default())
                    .or_default()
                    .add(record, price);
            }
        }
        for file in &effective {
            if compacted_overlap.contains(file.session.as_deref().unwrap())
                || query
                    .session_id
                    .as_ref()
                    .is_some_and(|id| Some(id) != file.session.as_ref())
                || query.turn_id.is_some()
            {
                continue;
            }
            if query.from.is_none() && query.to.is_none() {
                for (model, archive) in &file.archived_all {
                    if !query.accepts_model(model)
                        || !query.search.is_empty()
                            && !model_search_name(model).contains(&query.search)
                    {
                        continue;
                    }
                    models
                        .entry(model.clone())
                        .or_default()
                        .add_archive(archive, query.price_for(model));
                }
            }
            for ((day, model), archive) in &file.archived_days {
                if query.from.is_some_and(|f| *day < f)
                    || query.to.is_some_and(|t| *day > t)
                    || !query.accepts_model(model)
                {
                    continue;
                }
                if !query.search.is_empty()
                    && !model_search_name(model).contains(&query.search)
                    && !day.to_string().contains(&query.search)
                {
                    continue;
                }
                days.entry(*day)
                    .or_default()
                    .add_archive(archive, query.price_for(model));
                selected_day_models
                    .entry(*day)
                    .or_default()
                    .entry(model.clone())
                    .or_default()
                    .add_archive(archive, query.price_for(model));
                if query.from.is_some()
                    || query.to.is_some()
                    || !query.search.is_empty() && !model_search_name(model).contains(&query.search)
                {
                    models
                        .entry(model.clone())
                        .or_default()
                        .add_archive(archive, query.price_for(model));
                }
            }
        }
        let complete = !self.discovering
            && pending == 0
            && !self.truncated
            && !self.discovery_errors
            && !incomplete_records;
        let pages = selected.len().div_ceil(query.page_size).max(1);
        let page = query.page.min(pages);
        let rows: Vec<_> = selected
            .iter()
            .skip((page - 1) * query.page_size)
            .take(query.page_size)
            .map(|record| {
                let mut row = serde_json::to_value(record).unwrap();
                row["date"] = json!(record.date.map(|date| date.to_string()));
                let cost = record.usage.and_then(|usage| {
                    record
                        .model
                        .as_ref()
                        .and_then(|m| query.price_for(m))
                        .map(|p| p.estimate(usage))
                });
                row["estimatedCost"] = json!(cost);
                row
            })
            .collect();
        let model_rows: Vec<_> = models
            .iter()
            .map(|(model, aggregate)| {
                let mut v = aggregate.value();
                v["model"] = if model.is_empty() {
                    Value::Null
                } else {
                    json!(model)
                };
                v
            })
            .collect();
        let day_rows: Vec<_> = days
            .iter()
            .map(|(day, aggregate)| {
                let mut v = aggregate.value();
                v["date"] = json!(day.to_string());
                v["models"] = json!(
                    selected_day_models
                        .get(day)
                        .map(|models| models
                            .iter()
                            .map(|(model, a)| {
                                let mut m = a.value();
                                m["model"] = if model.is_empty() {
                                    Value::Null
                                } else {
                                    json!(model)
                                };
                                m
                            })
                            .collect::<Vec<_>>())
                        .unwrap_or_default()
                );
                v
            })
            .collect();
        for limit in &mut limits {
            limit["expired"] = json!(
                limit["resetAt"]
                    .as_i64()
                    .is_some_and(|reset| reset <= now / 1000)
            );
        }
        let daily: Vec<Value> = all_days
            .iter()
            .map(|(date, aggregate)| {
                let mut row = aggregate.value();
                row["date"] = json!(date.to_string());
                row["models"] = json!(
                    day_models
                        .get(date)
                        .map(|models| models
                            .iter()
                            .map(|(model, a)| {
                                let mut m = a.value();
                                m["model"] = if model.is_empty() {
                                    Value::Null
                                } else {
                                    json!(model)
                                };
                                m
                            })
                            .collect::<Vec<_>>())
                        .unwrap_or_default()
                );
                row
            })
            .collect();
        let today_summary = daily
            .iter()
            .find(|row| row["date"] == today.to_string())
            .cloned()
            .unwrap_or_else(|| {
                let mut a = Aggregate::default().value();
                a["date"] = json!(today.to_string());
                a["models"] = json!([]);
                a
            });
        let days7: Vec<_> = daily
            .iter()
            .filter(|row| {
                row["date"].as_str().is_some_and(|day| {
                    day >= week.to_string().as_str() && day <= today.to_string().as_str()
                })
            })
            .cloned()
            .collect();
        let by_model: BTreeMap<_,_>=all_models.iter().map(|(model,a)| (model.clone(),json!({"tokens":a.usage.map(|u|u.total_tokens),"input":a.usage.map(|u|u.input_tokens),"out":a.usage.map(|u|u.output_tokens),"cached":a.usage.map(|u|u.cached_input_tokens),"reason":a.usage.and_then(|u|u.reasoning_output_tokens),"turns":a.records,"unknownUsageRecords":a.unknown}))).collect();
        let total_usage = periods["all"].usage;
        let mut windows = serde_json::Map::new();
        for limit in &limits {
            if let Some(key) = limit["key"].as_str() {
                windows.insert(key.to_owned(),json!({"usedPct":limit["usedPercent"],"windowMinutes":limit["windowMinutes"],"resetAt":limit["resetAt"].as_i64().map(|v|v.saturating_mul(1000)),"observedAt":limit["observedAt"].as_i64().map(|v|v.saturating_mul(1000)),"expired":limit["expired"]}));
            }
        }
        let codex = json!({"ok":true,"complete":complete,"sessions":self.files.len(),"todayTokens":periods["today"].usage.map(|u|u.total_tokens),"monthTokens":periods["month"].usage.map(|u|u.total_tokens),"totalTokens":total_usage.map(|u|u.total_tokens),"days7":days7.iter().map(|day|json!({"date":day["date"],"tokens":day["usage"]["totalTokens"]})).collect::<Vec<_>>(),"windows":windows,"planType":plan_type,"stale":latest_limit==i64::MIN||now/1000-latest_limit>120});
        let machine_summary = json!({"ok":true,"complete":complete,"sessions":self.files.len(),"deferred":pending,"todayTokens":periods["today"].usage.map(|u|u.total_tokens),"monthTokens":periods["month"].usage.map(|u|u.total_tokens),"totalTokens":total_usage.map(|u|u.total_tokens),"outTokens":total_usage.map(|u|u.output_tokens),"reasonTokens":total_usage.and_then(|u|u.reasoning_output_tokens),"cachedTokens":total_usage.map(|u|u.cached_input_tokens),"byModel":by_model,"days7":days7.iter().map(|day|json!({"date":day["date"],"tokens":day["usage"]["totalTokens"],"turns":day["recordCount"],"models":day["models"]})).collect::<Vec<_>>(),"windows":limits,"rateLimitsTs":if latest_limit==i64::MIN {Value::Null} else {json!(latest_limit)}});
        let mut response = json!({"ok":true,"status":if complete {"ok"} else {"partial"},"complete":complete,"updatedAt":now,
            "source":"local-rollout-estimate","today":today_summary,"days7":days7,"all":{"days":daily,"events":rows,"pagination":{"page":page,"pageSize":query.page_size,"total":selected.len(),"pages":pages}},"machineSummary":machine_summary,"codex":codex,"periods":periods.iter().map(|(name,aggregate)|(*name,aggregate.value())).collect::<BTreeMap<_,_>>(),
            "models":model_rows,"days":day_rows,"records":rows,"pagination":{"page":page,"pageSize":query.page_size,"total":selected.len(),"pages":pages},"rateLimits":limits,
            "archive":{"recordCount":archived_records,"detailRetentionDays":90,"dailyRetentionDays":365,"recordsPerSessionLimit":MAX_RECORDS,"totalDetailLimit":MAX_TOTAL_RECORDS},
            "scan":{"discoveredFiles":self.files.len(),"indexedFiles":indexed,"pendingFiles":pending,"ignoredFiles":ignored,"identityMismatchFiles":identity_mismatches,"compactedOverlapSessions":compacted_overlap.len(),"truncated":self.truncated||incomplete_records||self.discovery_errors,"discovering":self.discovering},
            "message":if complete { Value::Null } else if !compacted_overlap.is_empty() { json!("部分分段的去重证据超过内存限额；无法确认重叠的旧汇总未计入，当前仅返回可核验的用量下界") } else if ignored > 0 { json!("部分会话缺少有效会话头或包含冲突的会话 ID；未确认部分未计入汇总") } else { json!("正在分批索引，或部分记录缺少日期/用量；以下仅汇总已观测数据") }});
        if query.include_session_cursors {
            // 游标是当前索引的会话累计水位，不按日期或模型切分，也不受明细分页影响。
            response["sessionCursors"] = json!(
                effective
                    .iter()
                    .filter_map(|file| {
                        file.session
                            .as_deref()
                            .zip(file.previous)
                            .map(|(session, total)| (session, (file.latest_count_at, total)))
                    })
                    .fold(BTreeMap::new(), |mut cursors, (session, candidate)| {
                        let selected = cursors.entry(session).or_insert(candidate);
                        if candidate.0 > selected.0
                            || candidate.0 == selected.0
                                && candidate.1.is_monotonic_after(selected.1)
                        {
                            *selected = candidate;
                        }
                        cursors
                    })
                    .into_iter()
                    .filter(|(session, _)| query
                        .session_id
                        .as_ref()
                        .is_none_or(|id| *session == id))
                    .map(|(session, (_, total))| json!({"sessionId":session,"total":total}))
                    .take(MAX_FILES)
                    .collect::<Vec<_>>()
            );
        }
        response
    }
}

// 只为跨读取边界的半行保留空间；所有 home 共用总量上限，Drop 同步归还。
#[derive(Default)]
struct PendingLine(Vec<u8>);
impl PendingLine {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn extend(&mut self, part: &[u8]) -> bool {
        if self.0.capacity() == 0 {
            if PENDING_BYTES
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    n.checked_add(MAX_LINE).filter(|n| *n <= MAX_PENDING_BYTES)
                })
                .is_err()
            {
                return false;
            }
            self.0 = Vec::with_capacity(MAX_LINE);
        }
        self.0.extend_from_slice(part);
        true
    }
    fn clear(&mut self) {
        *self = Self::default();
    }
}
impl Drop for PendingLine {
    fn drop(&mut self) {
        if self.0.capacity() != 0 {
            PENDING_BYTES.fetch_sub(MAX_LINE, Ordering::AcqRel);
        }
    }
}

// 仅辨认 JSON 顶层和 payload 的 type，不匹配图片、文本里的同名字符串。
// 无法从有限前缀确认类型时保持保守，避免跳过真实用量后声称完整。
struct UsagePrefix<'a> {
    payload: bool,
    result: &'a std::cell::Cell<Option<bool>>,
}
impl<'de> serde::de::DeserializeSeed<'de> for UsagePrefix<'_> {
    type Value = bool;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<bool, D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de> serde::de::Visitor<'de> for UsagePrefix<'_> {
    type Value = bool;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("rollout event")
    }
    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<bool, M::Error> {
        let mut event_msg = false;
        while let Some(key) = map.next_key::<String>()? {
            if key == "type" {
                let kind = map.next_value::<String>()?;
                if self.payload {
                    let relevant = matches!(
                        kind.as_str(),
                        "token_count"
                            | "task_started"
                            | "turn_started"
                            | "task_complete"
                            | "turn_completed"
                            | "task_failed"
                            | "turn_aborted"
                            | "task_aborted"
                            | "error"
                            | "stream_error"
                    );
                    self.result.set(Some(relevant));
                    return Ok(relevant);
                }
                match kind.as_str() {
                    "session_meta" | "turn_context" => {
                        self.result.set(Some(true));
                        return Ok(true);
                    }
                    "event_msg" => event_msg = true,
                    _ => {
                        self.result.set(Some(false));
                        return Ok(false);
                    }
                }
            } else if key == "payload" && event_msg {
                return map.next_value_seed(UsagePrefix {
                    payload: true,
                    result: self.result,
                });
            } else {
                map.next_value::<serde::de::IgnoredAny>()?;
            }
        }
        Ok(true)
    }
}
fn oversized_may_affect_usage(prefix: &[u8]) -> bool {
    let result = std::cell::Cell::new(None);
    // 前缀可能没有闭合括号；已确认的类型独立保存，不依赖完整 JSON 解析成功。
    let _ = serde::Deserializer::deserialize_map(
        &mut serde_json::Deserializer::from_slice(prefix),
        UsagePrefix {
            payload: false,
            result: &result,
        },
    );
    result.get().unwrap_or(true)
}

struct FileIndex {
    session: Option<String>,
    expected_id: Option<String>,
    verified: bool,
    previous: Option<Tokens>,
    history_boundary: UsageHistoryBoundary,
    current_model: Option<String>,
    current_turn: Option<String>,
    records: VecDeque<Record>,
    detail_size: usize,
    usage_digest: Sha256,
    continuation: bool,
    latest_count_at: Option<i64>,
    usage_sequence: u64,
    archived_all: BTreeMap<String, Archived>,
    archived_days: BTreeMap<(NaiveDate, String), Archived>,
    undated_archived: usize,
    offset: u64,
    pending: PendingLine,
    skipping: bool,
    identity: Option<(u64, u64)>,
    created: Option<SystemTime>,
    modified: Option<SystemTime>,
    catching_up: bool,
    truncated: bool,
    unreadable: bool,
    invalid_usage: bool,
    rate_limits: Vec<Value>,
    limits_observed: i64,
    plan_type: Option<String>,
}
impl FileIndex {
    fn new() -> Self {
        Self {
            session: None,
            expected_id: None,
            verified: false,
            previous: None,
            history_boundary: UsageHistoryBoundary::default(),
            current_model: None,
            current_turn: None,
            records: VecDeque::new(),
            detail_size: 0,
            usage_digest: Sha256::new(),
            continuation: false,
            latest_count_at: None,
            usage_sequence: 0,
            archived_all: BTreeMap::new(),
            archived_days: BTreeMap::new(),
            undated_archived: 0,
            offset: 0,
            pending: PendingLine::default(),
            skipping: false,
            identity: None,
            created: None,
            modified: None,
            catching_up: true,
            truncated: false,
            unreadable: false,
            invalid_usage: false,
            rate_limits: Vec::new(),
            limits_observed: i64::MIN,
            plan_type: None,
        }
    }
    fn revision(
        &self,
    ) -> (
        u64,
        Option<(u64, u64)>,
        Option<SystemTime>,
        Option<SystemTime>,
        bool,
        bool,
        bool,
        usize,
        bool,
    ) {
        (
            self.offset,
            self.identity,
            self.created,
            self.modified,
            self.unreadable,
            self.catching_up,
            self.verified,
            self.pending.len(),
            self.skipping,
        )
    }
    fn read_observed(
        &mut self,
        path: &Path,
        budget: u64,
        before: &Metadata,
    ) -> std::io::Result<u64> {
        let observed_identity = observed_file_identity(path, before)?;
        if before.file_type().is_file()
            && self.expected_id.is_some()
            && !self.unreadable
            && before.len() == self.offset
            && self.identity == observed_identity
            && self.created == before.created().ok()
            && self.modified == before.modified().ok()
        {
            return Ok(0);
        }
        if !before.file_type().is_file() || fs::canonicalize(path)? != path {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "rollout path changed",
            ));
        }
        let mut file = File::open(path)?;
        let meta = file.metadata()?;
        let identity = opened_file_identity(&file, &meta)?;
        if identity != observed_identity || fs::canonicalize(path)? != path {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "rollout identity changed",
            ));
        }
        let created = meta.created().ok();
        let modified = meta.modified().ok();
        if meta.len() < self.offset
            || self.offset > 0
                && (self.identity != identity
                    || self.created != created
                    || meta.len() == self.offset && self.modified != modified)
        {
            *self = Self::new();
        }
        self.expected_id = rollout_id(path);
        self.identity = identity;
        self.created = created;
        self.modified = modified;
        self.unreadable = false;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        file.take(budget).read_to_end(&mut bytes)?;
        let mut processed = 0;
        for part in bytes.split_inclusive(|b| *b == b'\n') {
            if !self.skipping
                && self.pending.len() == 0
                && part.len() <= MAX_LINE
                && part.last() == Some(&b'\n')
            {
                self.accept(part);
                processed += part.len();
                continue;
            }
            if !self.skipping {
                if self.pending.len() + part.len() <= MAX_LINE {
                    // 缓冲预算不足时停在本段之前；之后继续，不丢弃半行或用量。
                    if !self.pending.extend(part) {
                        break;
                    }
                } else {
                    let remaining = MAX_LINE - self.pending.len();
                    if self.pending.len() == 0 {
                        self.invalid_usage |= oversized_may_affect_usage(&part[..MAX_LINE]);
                    } else {
                        self.pending.extend(&part[..remaining]);
                        self.invalid_usage |= oversized_may_affect_usage(&self.pending.0);
                    }
                    self.pending.clear();
                    self.skipping = true;
                }
            }
            if part.last() == Some(&b'\n') {
                if !self.skipping {
                    let line = std::mem::take(&mut self.pending);
                    self.accept(&line.0);
                }
                self.pending.clear();
                self.skipping = false;
            }
            processed += part.len();
        }
        self.offset += processed as u64;
        self.catching_up = self.offset < meta.len();
        self.retain_recent(Local::now().date_naive());
        Ok(bytes.len() as u64)
    }
    fn retain_recent(&mut self, today: NaiveDate) {
        // 限额内保留用量去重证据；响应仍只开放 90 天明细和 365 天日汇总。
        // 过早压成按日总数会使分页副本的重叠部分无法核对。
        let daily_cutoff = today - ChronoDuration::days(364);
        self.archived_days
            .retain(|(day, _), _| *day >= daily_cutoff);
    }
    fn archive_record(&mut self, mut record: Record) {
        self.detail_size = self.detail_size.saturating_sub(record.detail_size());
        if record.observations.iter().any(|observation| {
            observation.continuation && observation.before.is_none() && observation.last.is_none()
        }) {
            // 丢弃去重依据前也不能把缺少前段基线的累计值当成本段消费。
            record.usage = None;
            self.invalid_usage = true;
        }
        let model = record.model.clone().unwrap_or_default();
        self.archived_all
            .entry(model.clone())
            .or_default()
            .add(&record);
        if let Some(day) = record.date {
            if day >= Local::now().date_naive() - ChronoDuration::days(364) {
                self.archived_days
                    .entry((day, model))
                    .or_default()
                    .add(&record);
            }
        } else {
            self.undated_archived += 1;
        }
    }
    fn accept(&mut self, line: &[u8]) {
        let Ok(event) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        let kind = event["type"].as_str().unwrap_or("");
        let payload = &event["payload"];
        let inherited = self.history_boundary.contains(event["ordinal"].as_u64());
        if !inherited
            && (matches!(kind, "session_meta" | "turn_context")
                || kind == "event_msg"
                    && matches!(
                        payload["type"].as_str(),
                        Some(
                            "token_count"
                                | "task_started"
                                | "turn_started"
                                | "task_complete"
                                | "turn_completed"
                                | "task_failed"
                                | "turn_aborted"
                                | "task_aborted"
                                | "error"
                                | "stream_error"
                        )
                    ))
        {
            self.usage_digest.update(event.to_string().as_bytes());
        }
        if kind == "session_meta" {
            if inherited {
                return;
            }
            let session = payload["id"]
                .as_str()
                .and_then(normalize_session_id)
                .or_else(|| {
                    payload["session_id"]
                        .as_str()
                        .and_then(normalize_session_id)
                });
            if self.session.is_none() {
                // 分页历史或迁移文件的名字可能沿用旧 ID；有效会话头才是实际归属。
                self.session = session;
                self.verified = self.session.is_some();
                self.history_boundary =
                    UsageHistoryBoundary::from_meta(payload, self.session.as_deref().unwrap_or(""));
                self.continuation = payload["history_base"]["end_ordinal_exclusive"]
                    .as_u64()
                    .is_some_and(|ordinal| ordinal > 0);
            } else if session.is_some() && session != self.session {
                // 复制的继承头已在上方排除；两个自有头不能混记为同一会话。
                self.verified = false;
                self.invalid_usage = true;
            }
            return;
        }
        if !self.verified {
            return;
        }
        if inherited {
            if kind == "event_msg" && payload["type"] == "token_count" {
                if let Some(total) = Tokens::parse(&payload["info"]["total_token_usage"]) {
                    self.previous = Some(total);
                }
            }
            return;
        }
        let time = event["timestamp"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok());
        let timestamp = time.map(|t| t.to_rfc3339());
        let day = time.map(|t| t.with_timezone(&Local).date_naive());
        let turn_id = payload["turn_id"]
            .as_str()
            .or_else(|| payload["turnId"].as_str())
            .filter(|s| !s.is_empty() && s.len() <= 200);
        if kind == "turn_context" {
            if turn_id.is_some_and(|id| self.current_turn.as_deref() != Some(id)) {
                self.current_model = None;
            }
            if let Some(model) = payload["model"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 200)
            {
                self.current_model = (!unknown_model(model)).then(|| model.to_owned());
            }
            if let Some(id) = turn_id {
                self.current_turn = Some(id.to_owned());
            }
            if let Some(record) = self
                .records
                .back_mut()
                .filter(|r| Some(&r.turn_id) == self.current_turn.as_ref() && r.usage.is_none())
            {
                record.model = self.current_model.clone();
            }
            return;
        }
        if kind != "event_msg" {
            return;
        }
        let event_type = payload["type"].as_str().unwrap_or("");
        if matches!(event_type, "task_started" | "turn_started") {
            if let Some(id) = turn_id {
                if self.current_turn.as_deref() != Some(id) {
                    self.current_model = None;
                }
                self.current_turn = Some(id.to_owned());
            }
            let row = self.current_record(timestamp, day);
            row.status = "running";
            return;
        }
        if matches!(
            event_type,
            "task_complete"
                | "turn_completed"
                | "task_failed"
                | "turn_aborted"
                | "task_aborted"
                | "error"
                | "stream_error"
        ) {
            let fatal = event_type == "task_failed"
                || payload["fatal"] == true
                || payload["will_retry"] == false
                || event_type == "error" && payload["will_retry"] != true;
            if matches!(event_type, "error" | "stream_error") && !fatal {
                return;
            }
            if let Some(id) = turn_id {
                self.current_turn = Some(id.to_owned());
            }
            let turn = self.current_turn.clone();
            let status = if matches!(event_type, "turn_aborted" | "task_aborted")
                || matches!(
                    payload["status"].as_str(),
                    Some("aborted" | "cancelled" | "canceled")
                ) {
                "aborted"
            } else if fatal
                || payload.get("error").is_some_and(|v| !v.is_null())
                || matches!(payload["status"].as_str(), Some("failed" | "error"))
            {
                "failed"
            } else if matches!(
                payload["status"].as_str(),
                Some("interrupted" | "incomplete")
            ) {
                "unknown"
            } else {
                "completed"
            };
            if self
                .records
                .iter()
                .any(|r| Some(&r.turn_id) == turn.as_ref())
            {
                for row in self
                    .records
                    .iter_mut()
                    .filter(|r| Some(&r.turn_id) == turn.as_ref())
                {
                    row.status = status;
                    if timestamp.is_some() {
                        row.updated_at = timestamp.clone();
                    }
                }
            } else {
                self.current_record(timestamp, day).status = status;
            }
            return;
        }
        if event_type != "token_count" {
            return;
        }
        if let Some(limits) = payload.get("rate_limits").filter(|v| v.is_object()) {
            let observed = time.map(|t| t.timestamp());
            let parsed = parse_limits(limits, observed);
            if !parsed.is_empty() {
                self.rate_limits = parsed;
                self.limits_observed = observed.unwrap_or(i64::MIN);
                self.plan_type = limits["plan_type"]
                    .as_str()
                    .filter(|s| s.len() <= 64)
                    .map(str::to_owned);
            }
        }
        let Some(next) = Tokens::parse(&payload["info"]["total_token_usage"]) else {
            return;
        };
        let observation = UsageObservation {
            sequence: self.usage_sequence,
            ordinal: event["ordinal"].as_u64(),
            timestamp: time.map(|time| time.timestamp_millis()),
            before: self.previous,
            total: next,
            last: Tokens::parse(&payload["info"]["last_token_usage"]),
            continuation: self.continuation,
        };
        let delta = next.usage_delta(observation.before, observation.last);
        self.previous = Some(next);
        self.usage_sequence = self.usage_sequence.saturating_add(1);
        self.latest_count_at = observation.timestamp;
        if delta.total_tokens == 0 && self.records.back().is_some_and(|r| r.usage.is_some()) {
            return;
        }
        if let Some(id) = turn_id {
            if self.current_turn.as_deref() != Some(id) {
                self.current_model = None;
            }
            self.current_turn = Some(id.to_owned());
        }
        let row = self.current_record(timestamp, day);
        row.usage.get_or_insert_with(Tokens::default).add(delta);
        row.observations.push(observation);
        self.detail_size += 1;
        while self.detail_size > MAX_RECORDS {
            let Some(record) = self.records.pop_front() else {
                break;
            };
            self.archive_record(record);
        }
    }
    fn current_record(&mut self, timestamp: Option<String>, day: Option<NaiveDate>) -> &mut Record {
        let turn = self
            .current_turn
            .clone()
            .unwrap_or_else(|| "unattributed".to_owned());
        let model = self.current_model.clone();
        let pos = self
            .records
            .iter()
            .rposition(|r| r.turn_id == turn && r.model == model && r.date == day);
        if let Some(pos) = pos {
            let row = self.records.get_mut(pos).unwrap();
            if timestamp.is_some() {
                row.updated_at = timestamp;
            }
            return row;
        }
        while self.detail_size >= MAX_RECORDS {
            if let Some(record) = self.records.pop_front() {
                self.archive_record(record);
            } else {
                break;
            }
        }
        let session = self.session.clone().unwrap();
        self.records.push_back(Record {
            id: format!(
                "{session}:{turn}:{}:{}",
                model.as_deref().unwrap_or(""),
                day.map(|d| d.to_string()).unwrap_or_default()
            ),
            session_id: session,
            turn_id: turn,
            model,
            started_at: timestamp.clone(),
            updated_at: timestamp,
            date: day,
            status: "unknown",
            usage: None,
            observations: Vec::new(),
        });
        self.detail_size += 1;
        self.records.back_mut().unwrap()
    }
}
fn record_timestamp(record: &Record) -> Option<i64> {
    record
        .updated_at
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp_millis())
}
fn model_search_name(model: &str) -> String {
    if unknown_model(model) {
        "unknown".to_owned()
    } else {
        model.to_lowercase()
    }
}
fn rollout_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    normalize_session_id(id)
}
fn parse_limits(value: &Value, observed: Option<i64>) -> Vec<Value> {
    ["primary", "secondary"]
        .into_iter()
        .filter_map(|key| {
            let v = &value[key];
            let percent = v["used_percent"]
                .as_f64()
                .filter(|p| p.is_finite() && (0.0..=100.0).contains(p))?;
            let reset = v["resets_at"].as_i64().filter(|r| *r > 0)?;
            let label = match v["window_minutes"].as_u64() {
                Some(300) => "5 小时".to_owned(),
                Some(10080) => "每周".to_owned(),
                Some(n) => format!("{n} 分钟"),
                None => if key == "primary" {
                    "主窗口"
                } else {
                    "次窗口"
                }
                .to_owned(),
            };
            Some(json!({"key":key,"label":label,"usedPercent":percent,"windowMinutes":v["window_minutes"],"resetAt":reset,"observedAt":observed}))
        })
        .collect()
}
#[cfg(unix)]
fn file_identity(meta: &Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}
#[cfg(not(any(unix, windows)))]
fn file_identity(_: &Metadata) -> Option<(u64, u64)> {
    None
}

fn observed_file_identity(path: &Path, meta: &Metadata) -> std::io::Result<Option<(u64, u64)>> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        // 零访问权限只查文件身份，避免未变文件打开内容；NTFS 轮转可能保留创建时间。
        let file = fs::OpenOptions::new()
            .access_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)?;
        opened_file_identity(&file, meta)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(file_identity(meta))
    }
}
fn opened_file_identity(file: &File, meta: &Metadata) -> std::io::Result<Option<(u64, u64)>> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let _ = meta;
        let mut identity = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut identity) }
            .map_err(std::io::Error::other)?;
        Ok(Some((
            identity.dwVolumeSerialNumber as u64,
            (identity.nFileIndexHigh as u64) << 32 | identity.nFileIndexLow as u64,
        )))
    }
    #[cfg(not(windows))]
    {
        let _ = file;
        Ok(file_identity(meta))
    }
}

#[cfg(test)]
mod tests;
