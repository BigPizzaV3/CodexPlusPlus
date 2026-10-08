//! 小鲸鱼的全机用量索引。只解析本地 rollout 的用量与轮次元数据；费用仅按用户价格估算。

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, NaiveDate};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::{self, File, Metadata, ReadDir};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

const READ_BUDGET: u64 = 8 * 1024 * 1024;
const FILE_READ_BUDGET: u64 = 512 * 1024;
const MAX_LINE: usize = 256 * 1024;
const MAX_FILES: usize = 4096;
const MAX_RECORDS: usize = 20_000;
const MAX_TOTAL_RECORDS: usize = 100_000;
const DISCOVERY_BUDGET: usize = 4096;
const MAX_HOMES: usize = 2;

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
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, HistoryIndex>>> = OnceLock::new();
    let Ok(mut cache) = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() else {
        return json!({"status":"unavailable","complete":false,"message":"用量索引暂不可用","records":[],"updatedAt":now.timestamp_millis()});
    };
    if !cache.contains_key(&home) && cache.len() >= MAX_HOMES {
        if let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, index)| index.touched)
            .map(|(p, _)| p.clone())
        {
            cache.remove(&oldest);
        }
    }
    let index = cache.entry(home.clone()).or_insert_with(HistoryIndex::new);
    index.touched = Instant::now();
    index.update(&home);
    index.response(&options, now.date_naive(), now.timestamp_millis())
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Tokens {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
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
    fn parse(value: &Value) -> Option<Self> {
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
}

#[derive(Clone)]
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

struct Query {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    model: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    search: String,
    page: usize,
    page_size: usize,
    prices: HashMap<String, Price>,
}
impl Query {
    fn parse(value: &Value, today: NaiveDate) -> Option<Self> {
        if !value.is_object() {
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
            Some(crate::whale_usage::normalize_session_id(
                value["sessionId"].as_str()?,
            )?)
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
            prices,
        })
    }
    fn price_for(&self, model: &str) -> Option<&Price> {
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
        if self
            .model
            .as_ref()
            .is_some_and(|m| record.model.as_ref() != Some(m))
        {
            return false;
        }
        self.search.is_empty()
            || record
                .model
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains(&self.search)
            || record
                .date
                .is_some_and(|d| d.to_string().contains(&self.search))
    }
}

#[derive(Default)]
struct Archived {
    usage: Option<Tokens>,
    record_count: usize,
    unknown_usage_records: usize,
}
impl Archived {
    fn add(&mut self, record: &Record) {
        self.record_count += 1;
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
    unpriced: usize,
    costs: BTreeMap<String, (f64, usize)>,
}
impl Aggregate {
    fn add(&mut self, record: &Record, price: Option<&Price>) {
        self.records += 1;
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
        json!({"usage":self.usage,"recordCount":self.records,"unknownUsageRecords":self.unknown,"unpricedRecords":self.unpriced,"estimatedCosts":costs})
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
    touched: Instant,
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
            touched: Instant::now(),
            truncated: false,
            discovery_errors: false,
        }
    }
    fn update(&mut self, home: &Path) {
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
                }
            }
        }
        // 已删除的会话应从统计移除。无法访问的文件保留未知标记，避免把错误当成零。
        self.files.retain(|path, _| !matches!(fs::metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound));
        let paths: Vec<_> = self.files.keys().cloned().collect();
        if paths.is_empty() {
            return;
        }
        let mut budget = READ_BUDGET;
        let start = self.next_file;
        for step in 0..paths.len() {
            let position = (start + step) % paths.len();
            let file = self.files.get_mut(&paths[position]).unwrap();
            let allowed = budget.min(FILE_READ_BUDGET);
            let valid = fs::canonicalize(&paths[position]).ok().is_some_and(|path| {
                path == paths[position] && self.roots.iter().any(|root| path.starts_with(root))
            }) && fs::symlink_metadata(&paths[position])
                .ok()
                .is_some_and(|meta| meta.file_type().is_file());
            if !valid {
                file.unreadable = true;
                continue;
            }
            match file.read(&paths[position], allowed) {
                Ok(consumed) => budget = budget.saturating_sub(consumed),
                Err(_) => file.unreadable = true,
            }
            self.next_file = (position + 1) % paths.len();
            if budget == 0 {
                break;
            }
        }
        let count: usize = self.files.values().map(|file| file.records.len()).sum();
        if count > MAX_TOTAL_RECORDS {
            let mut remove = count - MAX_TOTAL_RECORDS;
            for file in self.files.values_mut() {
                let n = remove.min(file.records.len());
                if n > 0 {
                    for _ in 0..n {
                        if let Some(record) = file.records.pop_front() {
                            file.archive_record(record);
                        }
                    }
                    remove -= n;
                }
                if remove == 0 {
                    break;
                }
            }
        }
    }
    fn response(&self, query: &Query, today: NaiveDate, now: i64) -> Value {
        let mut effective: BTreeMap<&str, &FileIndex> = BTreeMap::new();
        for file in self.files.values().filter(|f| f.verified) {
            let id = file.session.as_deref().unwrap();
            if effective.get(id).is_none_or(|old| {
                file.modified > old.modified
                    || file.modified == old.modified && file.offset > old.offset
            }) {
                effective.insert(id, file);
            }
        }
        let effective: Vec<_> = effective.into_values().collect();
        let mut record_map: BTreeMap<String, Record> = BTreeMap::new();
        let mut limits = Vec::new();
        let mut latest_limit = i64::MIN;
        let mut plan_type = None;
        let mut pending = 0;
        let mut indexed = 0;
        let mut incomplete_records = false;
        let mut archived_records = 0;
        for file in self.files.values() {
            if file.catching_up || file.unreadable || !file.verified {
                pending += 1;
            } else {
                indexed += 1;
            }
            incomplete_records |=
                file.truncated || file.invalid_usage || file.pending.len() > 0 || file.skipping;
            if effective
                .iter()
                .any(|candidate| std::ptr::eq(*candidate, file))
            {
                archived_records += file
                    .archived_all
                    .values()
                    .map(|a| a.record_count)
                    .sum::<usize>();
                incomplete_records |= file.undated_archived > 0;
                for record in &file.records {
                    let replace = record_map.get(&record.id).is_none_or(|previous| {
                        record_timestamp(record) > record_timestamp(previous)
                            || record_timestamp(record) == record_timestamp(previous)
                                && record.usage.map(|u| u.total_tokens)
                                    > previous.usage.map(|u| u.total_tokens)
                    });
                    if replace {
                        record_map.insert(record.id.clone(), record.clone());
                    }
                }
                if file.limits_observed > latest_limit {
                    latest_limit = file.limits_observed;
                    limits = file.rate_limits.clone();
                    plan_type = file.plan_type.clone();
                }
            }
        }
        let mut records: Vec<_> = record_map.into_values().collect();
        records.sort_by(|a, b| {
            record_timestamp(b)
                .cmp(&record_timestamp(a))
                .then_with(|| b.id.cmp(&a.id))
        });
        let week = today - ChronoDuration::days(6);
        let month = today.with_day(1).unwrap();
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
        let selected: Vec<_> = records.iter().filter(|r| query.accepts(r)).collect();
        let mut models: BTreeMap<String, Aggregate> = BTreeMap::new();
        let mut days: BTreeMap<NaiveDate, Aggregate> = BTreeMap::new();
        let mut selected_day_models: BTreeMap<NaiveDate, BTreeMap<String, Aggregate>> =
            BTreeMap::new();
        for record in &selected {
            let price = record.model.as_ref().and_then(|m| query.price_for(m));
            models
                .entry(record.model.clone().unwrap_or_default())
                .or_default()
                .add(record, price);
            if let Some(day) = record.date {
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
            if query
                .session_id
                .as_ref()
                .is_some_and(|id| Some(id) != file.session.as_ref())
                || query.turn_id.is_some()
            {
                continue;
            }
            if query.from.is_none() && query.to.is_none() {
                for (model, archive) in &file.archived_all {
                    if query.model.as_ref().is_some_and(|m| m != model)
                        || !query.search.is_empty() && !model.to_lowercase().contains(&query.search)
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
                    || query.model.as_ref().is_some_and(|m| m != model)
                {
                    continue;
                }
                if !query.search.is_empty()
                    && !model.to_lowercase().contains(&query.search)
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
                    || !query.search.is_empty() && !model.to_lowercase().contains(&query.search)
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
        json!({"ok":true,"status":if complete {"ok"} else {"partial"},"complete":complete,"updatedAt":now,
            "source":"local-rollout-estimate","today":today_summary,"days7":days7,"all":{"days":daily,"events":rows,"pagination":{"page":page,"pageSize":query.page_size,"total":selected.len(),"pages":pages}},"machineSummary":machine_summary,"codex":codex,"periods":periods.iter().map(|(name,aggregate)|(*name,aggregate.value())).collect::<BTreeMap<_,_>>(),
            "models":model_rows,"days":day_rows,"records":rows,"pagination":{"page":page,"pageSize":query.page_size,"total":selected.len(),"pages":pages},"rateLimits":limits,
            "archive":{"recordCount":archived_records,"detailRetentionDays":90,"dailyRetentionDays":365,"recordsPerSessionLimit":MAX_RECORDS,"totalDetailLimit":MAX_TOTAL_RECORDS},
            "scan":{"discoveredFiles":self.files.len(),"indexedFiles":indexed,"pendingFiles":pending,"truncated":self.truncated||incomplete_records||self.discovery_errors,"discovering":self.discovering},
            "message":if complete { Value::Null } else { json!("正在分批索引，或部分记录缺少日期/用量；以下仅汇总已观测数据") }})
    }
}

struct FileIndex {
    session: Option<String>,
    expected_id: Option<String>,
    verified: bool,
    previous: Option<Tokens>,
    current_model: Option<String>,
    current_turn: Option<String>,
    records: VecDeque<Record>,
    archived_all: BTreeMap<String, Archived>,
    archived_days: BTreeMap<(NaiveDate, String), Archived>,
    undated_archived: usize,
    offset: u64,
    pending: Vec<u8>,
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
            current_model: None,
            current_turn: None,
            records: VecDeque::new(),
            archived_all: BTreeMap::new(),
            archived_days: BTreeMap::new(),
            undated_archived: 0,
            offset: 0,
            pending: Vec::new(),
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
    fn read(&mut self, path: &Path, budget: u64) -> std::io::Result<u64> {
        let before = fs::symlink_metadata(path)?;
        if !before.file_type().is_file() || fs::canonicalize(path)? != path {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "rollout path changed",
            ));
        }
        let mut file = File::open(path)?;
        let meta = file.metadata()?;
        if file_identity(&meta) != file_identity(&before) || fs::canonicalize(path)? != path {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "rollout identity changed",
            ));
        }
        let identity = file_identity(&meta);
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
        self.offset += bytes.len() as u64;
        self.catching_up = self.offset < meta.len();
        for part in bytes.split_inclusive(|b| *b == b'\n') {
            if !self.skipping {
                if self.pending.len() + part.len() <= MAX_LINE {
                    self.pending.extend_from_slice(part);
                } else {
                    self.pending.clear();
                    self.skipping = true;
                    self.invalid_usage = true;
                }
            }
            if part.last() == Some(&b'\n') {
                if !self.skipping {
                    let line = std::mem::take(&mut self.pending);
                    self.accept(&line);
                }
                self.pending.clear();
                self.skipping = false;
            }
        }
        // 追平前不按年龄归档：同一历史轮次可能跨越两次读取预算。
        if !self.catching_up {
            let cutoff = Local::now().date_naive() - ChronoDuration::days(89);
            let mut retained = VecDeque::new();
            while let Some(record) = self.records.pop_front() {
                if record.date.is_some_and(|day| day < cutoff) {
                    self.archive_record(record);
                } else {
                    retained.push_back(record);
                }
            }
            self.records = retained;
        }
        let daily_cutoff = Local::now().date_naive() - ChronoDuration::days(364);
        self.archived_days
            .retain(|(day, _), _| *day >= daily_cutoff);
        Ok(bytes.len() as u64)
    }
    fn archive_record(&mut self, record: Record) {
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
        if kind == "session_meta" {
            if self.session.is_none() {
                self.session = payload["id"]
                    .as_str()
                    .or_else(|| payload["session_id"].as_str())
                    .and_then(crate::whale_usage::normalize_session_id);
                self.verified = self.session.is_some() && self.session == self.expected_id;
            }
            return;
        }
        if !self.verified {
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
            if let Some(model) = payload["model"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 200)
            {
                self.current_model = Some(model.to_owned());
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
        let delta = match self.previous {
            Some(before)
                if next.input_tokens < before.input_tokens
                    || next.output_tokens < before.output_tokens =>
            {
                Tokens::parse(&payload["info"]["last_token_usage"]).unwrap_or(next)
            }
            Some(before) => next.delta(before),
            None => next,
        };
        self.previous = Some(next);
        if delta.total_tokens == 0 && self.records.back().is_some_and(|r| r.usage.is_some()) {
            return;
        }
        if let Some(id) = turn_id {
            self.current_turn = Some(id.to_owned());
        }
        let row = self.current_record(timestamp, day);
        row.usage.get_or_insert_with(Tokens::default).add(delta);
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
        if self.records.len() >= MAX_RECORDS {
            if let Some(record) = self.records.pop_front() {
                self.archive_record(record);
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
        });
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
fn rollout_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    crate::whale_usage::normalize_session_id(id)
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
#[cfg(not(unix))]
fn file_identity(_: &Metadata) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    const A: &str = "019a11b9-193d-7c32-a71e-ae11fd3be78f";
    const B: &str = "019a11b9-193d-7c32-a71e-ae11fd3be780";
    struct Fixture {
        temp: TempDir,
    }
    impl Fixture {
        fn new() -> Self {
            Self {
                temp: tempfile::tempdir().unwrap(),
            }
        }
        fn write(&self, id: &str, events: &[Value]) -> PathBuf {
            let dir = self.temp.path().join("sessions/2026/10/08");
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("rollout-2026-10-08T10-00-00-{id}.jsonl"));
            let header = json!({"type":"session_meta","payload":{"id":id}});
            fs::write(
                &path,
                std::iter::once(&header)
                    .chain(events)
                    .map(|v| format!("{v}\n"))
                    .collect::<String>(),
            )
            .unwrap();
            path
        }
        fn query(&self, q: Value) -> Value {
            query_history(self.temp.path(), &q)
        }
    }
    fn event(time: &str, kind: &str, turn: &str) -> Value {
        json!({"type":"event_msg","timestamp":time,"payload":{"type":kind,"turn_id":turn}})
    }
    fn context(time: &str, model: &str, turn: &str) -> Value {
        json!({"type":"turn_context","timestamp":time,"payload":{"model":model,"turn_id":turn}})
    }
    fn tokens(time: &str, input: u64, cached: u64, output: u64) -> Value {
        json!({"type":"event_msg","timestamp":time,"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"cached_input_tokens":cached,"output_tokens":output,"reasoning_output_tokens":output,"total_tokens":999999}}}})
    }
    fn times() -> (String, String) {
        let now = Local::now();
        (
            now.to_rfc3339(),
            (now - ChronoDuration::days(2)).to_rfc3339(),
        )
    }

    #[test]
    fn whole_machine_periods_models_costs_and_pagination_use_incremental_deltas() {
        let f = Fixture::new();
        let (now, old) = times();
        f.write(
            A,
            &[
                context(&old, "model-a", "old"),
                tokens(&old, 100, 40, 20),
                context(&now, "model-a", "new"),
                event(&now, "task_started", "new"),
                tokens(&now, 150, 60, 35),
                tokens(&now, 150, 60, 35),
                event(&now, "task_complete", "new"),
            ],
        );
        f.write(
            B,
            &[context(&now, "model-b", "other"), tokens(&now, 200, 50, 40)],
        );
        let result=f.query(json!({"period":"today","pageSize":1,"prices":[{"model":"model-a","currency":"USD","input":2.0,"cachedInput":0.2,"output":6.0}]}));
        assert_eq!(result["status"], "ok");
        assert_eq!(result["periods"]["all"]["usage"]["totalTokens"], 425);
        assert_eq!(result["periods"]["today"]["usage"]["totalTokens"], 305);
        assert_eq!(result["periods"]["today"]["usage"]["cachedInputTokens"], 70);
        assert_eq!(
            result["periods"]["today"]["usage"]["reasoningOutputTokens"],
            55
        );
        assert_eq!(result["pagination"]["total"], 2);
        assert_eq!(result["pagination"]["pages"], 2);
        assert_eq!(result["models"].as_array().unwrap().len(), 2);
        assert_eq!(result["days"][0]["models"].as_array().unwrap().len(), 2);
        let model_a = result["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["model"] == "model-a")
            .unwrap();
        assert_eq!(model_a["estimatedCosts"][0]["amount"], 0.000154);
        let result2 = f.query(json!({"period":"today","model":"model-b"}));
        assert_eq!(result2["records"][0]["estimatedCost"], Value::Null);
        assert_eq!(result2["records"][0]["usage"]["totalTokens"], 240);
        let result3 = f.query(json!({"search":"model-a"}));
        assert_eq!(result3["pagination"]["total"], 2);
        let result4 = f.query(json!({"search":Local::now().date_naive().to_string()}));
        assert_eq!(result4["pagination"]["total"], 2);
        assert_eq!(result4["machineSummary"]["totalTokens"], 425);
    }

    #[test]
    fn missing_usage_is_unknown_and_subscriptions_are_observed_snapshots() {
        let f = Fixture::new();
        let (now, _) = times();
        f.write(A,&[context(&now,"model-a","turn"),event(&now,"task_started","turn"),event(&now,"task_complete","turn"),json!({"type":"event_msg","timestamp":now,"payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":25.0,"window_minutes":300,"resets_at":Local::now().timestamp()+900}}}})]);
        let result = f.query(json!({}));
        assert_eq!(result["records"][0]["usage"], Value::Null);
        assert_eq!(result["records"][0]["estimatedCost"], Value::Null);
        assert_eq!(result["periods"]["all"]["usage"], Value::Null);
        assert_eq!(result["periods"]["all"]["unknownUsageRecords"], 1);
        assert_eq!(result["rateLimits"][0]["usedPercent"], 25.0);
        assert_eq!(result["rateLimits"][0]["expired"], false);
        assert!(result["rateLimits"][0]["observedAt"].is_i64());
    }

    #[test]
    fn appended_partial_lines_do_not_count_before_completion_and_truncation_restarts() {
        let f = Fixture::new();
        let (now, _) = times();
        let path = f.write(
            A,
            &[context(&now, "model-a", "turn"), tokens(&now, 100, 40, 20)],
        );
        assert_eq!(
            f.query(json!({}))["periods"]["all"]["usage"]["totalTokens"],
            120
        );
        let line = tokens(&now, 150, 60, 30).to_string();
        let split = line.len() / 2;
        let mut append = fs::OpenOptions::new().append(true).open(&path).unwrap();
        append.write_all(line[..split].as_bytes()).unwrap();
        let partial = f.query(json!({}));
        assert_eq!(partial["complete"], false);
        assert_eq!(partial["periods"]["all"]["usage"]["totalTokens"], 120);
        append
            .write_all(format!("{}\n", &line[split..]).as_bytes())
            .unwrap();
        assert_eq!(
            f.query(json!({}))["periods"]["all"]["usage"]["totalTokens"],
            180
        );
        f.write(
            A,
            &[
                context(&now, "model-a", "replacement"),
                tokens(&now, 20, 0, 5),
            ],
        );
        assert_eq!(
            f.query(json!({}))["periods"]["all"]["usage"]["totalTokens"],
            25
        );
    }

    #[test]
    fn counter_reset_uses_last_usage_and_model_switch_is_not_misattributed() {
        let f = Fixture::new();
        let (now, _) = times();
        let mut reset = tokens(&now, 10, 3, 2);
        reset["payload"]["info"]["last_token_usage"] =
            json!({"input_tokens":4,"cached_input_tokens":1,"output_tokens":1});
        f.write(
            A,
            &[
                context(&now, "a", "first"),
                tokens(&now, 100, 20, 10),
                context(&now, "b", "second"),
                event(&now, "task_started", "second"),
                reset,
            ],
        );
        let r = f.query(json!({}));
        assert_eq!(r["periods"]["all"]["usage"]["totalTokens"], 115);
        let b = r["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["model"] == "b")
            .unwrap();
        assert_eq!(b["usage"]["totalTokens"], 5);
    }

    #[test]
    fn file_budget_resumes_without_reporting_large_log_as_zero() {
        let f = Fixture::new();
        let (now, _) = times();
        let path = f.write(A, &[]);
        let mut append = fs::OpenOptions::new().append(true).open(path).unwrap();
        let noise = json!({"type":"ignored","payload":{"content":"x".repeat(1000)}}).to_string();
        for _ in 0..1200 {
            writeln!(append, "{noise}").unwrap();
        }
        writeln!(append, "{}", context(&now, "model-a", "turn")).unwrap();
        writeln!(append, "{}", tokens(&now, 10, 0, 5)).unwrap();
        let first = f.query(json!({}));
        assert_eq!(first["complete"], false);
        assert_eq!(first["periods"]["all"]["usage"], Value::Null);
        let mut latest = first;
        for _ in 0..5 {
            latest = f.query(json!({}));
            if latest["complete"] == true {
                break;
            }
        }
        assert_eq!(latest["complete"], true);
        assert_eq!(latest["periods"]["all"]["usage"]["totalTokens"], 15);
    }

    #[test]
    fn invalid_identity_query_and_external_symlinks_are_rejected() {
        let f = Fixture::new();
        let (now, _) = times();
        let path = f.write(A, &[tokens(&now, 10, 0, 5)]);
        let text = fs::read_to_string(&path).unwrap().replace(A, B);
        fs::write(path, text).unwrap();
        let result = f.query(json!({}));
        assert_eq!(result["periods"]["all"]["usage"], Value::Null);
        assert_eq!(result["complete"], false);
        assert_eq!(
            f.query(json!({"from":"2026-99-99"}))["status"],
            "unavailable"
        );
        assert_eq!(f.query(json!({"prices":[{"model":"a","currency":"USD","input":-1,"cachedInput":1,"output":1}]}))["status"],"unavailable");
        #[cfg(unix)]
        {
            let external = Fixture::new();
            let outside = external.write(B, &[tokens(&now, 10000, 0, 1000)]);
            std::os::unix::fs::symlink(outside, f.temp.path().join("sessions/escape.jsonl"))
                .unwrap();
            assert_eq!(f.query(json!({}))["periods"]["all"]["usage"], Value::Null);
        }
    }
    #[test]
    fn old_details_roll_into_daily_and_lifetime_aggregates_without_losing_cost() {
        let f = Fixture::new();
        let now = Local::now();
        let old = (now - ChronoDuration::days(120)).to_rfc3339();
        let ancient = (now - ChronoDuration::days(400)).to_rfc3339();
        f.write(
            A,
            &[
                context(&ancient, "model-a", "ancient"),
                tokens(&ancient, 100, 20, 10),
                context(&old, "model-a", "archived"),
                tokens(&old, 150, 30, 15),
                context(&now.to_rfc3339(), "model-a", "current"),
                tokens(&now.to_rfc3339(), 170, 40, 17),
            ],
        );
        let r=f.query(json!({"prices":[{"model":"model-a","currency":"USD","input":2,"cachedInput":0.2,"output":6}]}));
        assert_eq!(r["status"], "ok");
        assert_eq!(r["periods"]["all"]["usage"]["totalTokens"], 187);
        assert_eq!(r["periods"]["all"]["recordCount"], 3);
        assert_eq!(r["pagination"]["total"], 1);
        assert_eq!(r["archive"]["recordCount"], 2);
        assert_eq!(r["all"]["days"].as_array().unwrap().len(), 2);
        assert_eq!(r["models"][0]["usage"]["totalTokens"], 187);
        assert_eq!(r["models"][0]["recordCount"], 3);
        assert_eq!(r["models"][0]["estimatedCosts"][0]["amount"], 0.00037);
        let twice = f.query(json!({}));
        assert_eq!(twice["periods"]["all"]["usage"]["totalTokens"], 187);
    }

    #[test]
    fn duplicate_active_archived_rollout_is_counted_once_and_turn_filter_is_exact() {
        let f = Fixture::new();
        let (now, _) = times();
        let path = f.write(
            A,
            &[
                context(&now, "a", "turn-1"),
                tokens(&now, 100, 0, 10),
                context(&now, "b", "turn-2"),
                tokens(&now, 150, 0, 20),
            ],
        );
        let archive = f.temp.path().join("archived_sessions");
        fs::create_dir_all(&archive).unwrap();
        fs::copy(&path, archive.join(path.file_name().unwrap())).unwrap();
        let r = f.query(json!({"turnId":"turn-2","sessionId":A}));
        assert_eq!(r["periods"]["all"]["usage"]["totalTokens"], 170);
        assert_eq!(r["pagination"]["total"], 1);
        assert_eq!(r["records"][0]["turnId"], "turn-2");
        assert_eq!(r["records"][0]["usage"]["totalTokens"], 60);
    }

    #[test]
    fn partially_configured_price_keeps_usage_visible_and_amount_unknown() {
        let f = Fixture::new();
        let (now, _) = times();
        f.write(
            A,
            &[context(&now, "model-a", "turn"), tokens(&now, 100, 40, 10)],
        );
        let result = f.query(json!({"prices":[{"model":"model-a","currency":"USD","input":2,"cachedInput":null,"output":6}]}));
        assert_eq!(result["status"], "ok");
        assert_eq!(result["periods"]["all"]["usage"]["totalTokens"], 110);
        assert_eq!(result["records"][0]["estimatedCost"], Value::Null);
        assert_eq!(result["periods"]["all"]["unpricedRecords"], 1);
    }
    #[test]
    fn explicit_model_prices_use_exact_then_longest_case_insensitive_match() {
        let q = Query::parse(
            &json!({"prices":[
                {"model":"gpt","currency":"USD","input":10,"cachedInput":1,"output":10},
                {"model":"gpt-5","currency":"USD","input":2,"cachedInput":0.2,"output":4},
                {"model":"gpt-5-mini","currency":"USD","input":1,"cachedInput":0.1,"output":2}
            ]}),
            Local::now().date_naive(),
        )
        .unwrap();
        assert_eq!(q.price_for("gpt-5-mini").unwrap().input, 1.0);
        assert_eq!(q.price_for("GPT-5-pro").unwrap().input, 2.0);
        assert_eq!(q.price_for("gpt-other").unwrap().input, 10.0);
        assert!(q.price_for("unpriced-model").is_none());
    }
}
