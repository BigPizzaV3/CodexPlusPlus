//! Daily 消费者自己的封存数据，不是原生账单或归属证明，不参与 usage_history。

use chrono::{Duration as Days, Local, NaiveDate};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const CLIENT: &str = "daily";
const MAX_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_CORE_BYTES: usize = 512 * 1024;
const MAX_DETAIL_BYTES: usize = 8 * 1024 * 1024;
const MAX_COMPRESSED_DETAIL_BYTES: usize = MAX_DETAIL_BYTES + 64 * 1024;
const MAX_TOTAL_DETAIL_BYTES: usize = 128 * 1024 * 1024;
const MAX_SESSIONS: usize = 4096;

type Result<T> = std::result::Result<T, ArchiveError>;

#[derive(Debug)]
enum ArchiveError {
    Unavailable(&'static str),
    Failed(&'static str),
}

impl From<rusqlite::Error> for ArchiveError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Failed("storage-error")
    }
}

impl From<std::io::Error> for ArchiveError {
    fn from(_: std::io::Error) -> Self {
        Self::Failed("storage-error")
    }
}

impl From<serde_json::Error> for ArchiveError {
    fn from(_: serde_json::Error) -> Self {
        Self::Failed("invalid-data")
    }
}

fn require(condition: bool, code: &'static str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(ArchiveError::Failed(code))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    operation: Operation,
    date: Option<String>,
    expected_revision: Option<String>,
    entry: Option<Entry>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Operation {
    List,
    Read,
    Put,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    version: u32,
    date: String,
    core: Core,
    detail: String,
    protected_detail: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Core {
    all: Summary,
    sessions: Vec<SessionSummary>,
    session_info: BTreeMap<String, SessionInfo>,
    aliases: BTreeMap<String, String>,
    pending_live_events: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_accounting_fingerprint: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SessionSummary {
    session_id: String,
    summary: Summary,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SessionInfo {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
}

// 展开为独立的固定结构，避免 flatten 吞掉未知字段。
macro_rules! usage_record {
    ($name:ident { $($fields:tt)* }) => {
        #[derive(Clone, Deserialize, Serialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            input: u64,
            output: u64,
            cached: u64,
            reasoning: u64,
            total: u64,
            calls: u64,
            turns: u64,
            cached_known: bool,
            reasoning_known: bool,
            input_known: bool,
            output_known: bool,
            total_known: bool,
            usage_uncertain: bool,
            $($fields)*
        }

        impl $name {
            fn counts(&self) -> [u64; 7] {
                [self.input, self.output, self.cached, self.reasoning,
                 self.total, self.calls, self.turns]
            }

            fn validate_counts(&self) -> Result<()> {
                require(self.counts().iter().all(|n| *n <= MAX_INTEGER)
                    && self.input.checked_add(self.output) == Some(self.total)
                    && self.cached <= self.input && self.reasoning <= self.output,
                    "invalid-counts")
            }
        }
    };
}

usage_record!(Summary {
    models: Vec<ModelRow>,
    breakdowns: Vec<BreakdownRow>,
});

usage_record!(ModelRow {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_inferred: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inferred_model_tokens: Option<u64>,
});

usage_record!(BreakdownRow {
    model: String,
    provider: String,
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_conflict: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_conflict: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_inferred: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_inferred: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inferred_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inferred_model_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_source: Option<String>,
});

fn text(value: &str) -> Result<()> {
    require(value.chars().count() <= 256, "string-limit")
}

fn role(value: &str) -> Result<()> {
    require(
        matches!(value, "main" | "subagent" | "unknown"),
        "invalid-role",
    )
}

fn uuid(value: &str) -> Result<()> {
    require(
        value.len() == 36 && uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value),
        "invalid-session-id",
    )
}

fn date(value: &str) -> Result<NaiveDate> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| ArchiveError::Failed("invalid-date"))?;
    require(
        value.len() == 10 && parsed.to_string() == value,
        "invalid-date",
    )?;
    Ok(parsed)
}

fn revision(value: &str) -> Result<()> {
    require(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid-revision",
    )
}

fn sum_counts(values: impl Iterator<Item = [u64; 7]>) -> Result<[u64; 7]> {
    let mut sum = [0_u64; 7];
    for counts in values {
        for (total, value) in sum.iter_mut().zip(counts) {
            *total = total
                .checked_add(value)
                .filter(|n| *n <= MAX_INTEGER)
                .ok_or(ArchiveError::Failed("count-overflow"))?;
        }
    }
    Ok(sum)
}

impl Summary {
    fn validate(&self) -> Result<()> {
        self.validate_counts()?;
        for row in &self.models {
            row.validate_counts()?;
            text(&row.model)?;
            require(
                row.inferred_model_tokens.is_none_or(|n| n <= row.total),
                "invalid-inferred-counts",
            )?;
        }
        for row in &self.breakdowns {
            row.validate_counts()?;
            for value in [&row.model, &row.provider] {
                text(value)?;
            }
            role(&row.role)?;
            for value in [&row.provider_name, &row.provider_source, &row.model_source]
                .into_iter()
                .flatten()
            {
                text(value)?;
            }
            require(
                [row.inferred_tokens, row.inferred_model_tokens]
                    .into_iter()
                    .flatten()
                    .all(|n| n <= row.total),
                "invalid-inferred-counts",
            )?;
        }
        require(
            sum_counts(self.models.iter().map(ModelRow::counts))? == self.counts()
                && sum_counts(self.breakdowns.iter().map(BreakdownRow::counts))? == self.counts(),
            "inconsistent-summary",
        )
    }
}

impl Core {
    fn validate(&self) -> Result<()> {
        require(
            self.sessions.len() <= MAX_SESSIONS
                && self.session_info.len() <= MAX_SESSIONS
                && self.aliases.len() <= MAX_SESSIONS,
            "session-limit",
        )?;
        require(self.pending_live_events <= MAX_INTEGER, "invalid-counts")?;
        if let Some(value) = &self.source_accounting_fingerprint {
            text(value)?;
        }
        self.all.validate()?;
        let mut seen = HashSet::new();
        for session in &self.sessions {
            uuid(&session.session_id)?;
            require(
                seen.insert(&session.session_id)
                    && self.session_info.contains_key(&session.session_id),
                "invalid-session-membership",
            )?;
            session.summary.validate()?;
        }
        require(
            sum_counts(self.sessions.iter().map(|row| row.summary.counts()))? == self.all.counts(),
            "inconsistent-sessions",
        )?;
        for (id, info) in &self.session_info {
            uuid(id)?;
            role(&info.role)?;
            if let Some(parent) = &info.parent_session_id {
                uuid(parent)?;
                require(parent != id, "invalid-parent")?;
            }
            let mut visited = HashSet::new();
            let mut node = id.as_str();
            while let Some(parent) = self
                .session_info
                .get(node)
                .and_then(|info| info.parent_session_id.as_deref())
            {
                require(visited.insert(node), "parent-cycle")?;
                node = parent;
            }
        }
        for (alias, canonical) in &self.aliases {
            uuid(alias)?;
            uuid(canonical)?;
            require(
                self.aliases
                    .get(canonical)
                    .is_none_or(|next| next == canonical),
                "noncanonical-alias",
            )?;
        }
        require(
            serde_json::to_vec(self)?.len() <= MAX_CORE_BYTES,
            "core-limit",
        )
    }
}

fn canonical_json(value: &impl Serialize) -> Result<String> {
    fn ordered(value: Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(key, value)| (key, ordered(value)))
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            Value::Array(rows) => Value::Array(rows.into_iter().map(ordered).collect()),
            value => value,
        }
    }
    Ok(serde_json::to_string(&ordered(serde_json::to_value(
        value,
    )?))?)
}

struct Location {
    state: PathBuf,
    storage: PathBuf,
    home_binding: String,
    state_binding: String,
}

fn binding(path: &Path) -> String {
    format!("{:x}", Sha256::digest(path.as_os_str().as_encoded_bytes()))
}

impl Location {
    fn new(home: &Path, settings: &Path) -> Result<Self> {
        let home =
            fs::canonicalize(home).map_err(|_| ArchiveError::Unavailable("home-unavailable"))?;
        let state = fs::canonicalize(settings.parent().unwrap_or_else(|| Path::new(".")))
            .map_err(|_| ArchiveError::Unavailable("state-unavailable"))?;
        require(home.is_dir() && state.is_dir(), "invalid-owner")?;
        Ok(Self {
            storage: crate::usage_history::storage_dir(&home, &state.join("settings.json")),
            home_binding: binding(&home),
            state_binding: binding(&state),
            state,
        })
    }

    fn database(&self, create: bool) -> Result<PathBuf> {
        for directory in [self.state.join("usage-history"), self.storage.clone()] {
            match fs::symlink_metadata(&directory) {
                Ok(metadata) => require(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    "invalid-storage-path",
                )?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !create {
                        return Err(ArchiveError::Unavailable("archive-missing"));
                    }
                    match fs::create_dir(&directory) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            }
            require(
                fs::canonicalize(&directory)? == directory
                    && fs::symlink_metadata(&directory)?.is_dir(),
                "invalid-storage-path",
            )?;
        }
        let path = self.storage.join("usage-summary.sqlite");
        match fs::symlink_metadata(&path) {
            Ok(metadata) => require(
                metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && fs::canonicalize(&path)? == path,
                "invalid-storage-path",
            )?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ArchiveError::Unavailable("archive-missing"));
            }
            Err(error) => return Err(error.into()),
        }
        Ok(path)
    }
}

fn verify_owner(db: &Connection, location: &Location) -> Result<()> {
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    require(version == 1, "unsupported-schema")?;
    let owner: (String, String, String) = db.query_row(
        "SELECT home_binding,state_binding,client FROM archive_owner WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    require(
        owner.0 == location.home_binding && owner.1 == location.state_binding && owner.2 == CLIENT,
        "owner-mismatch",
    )
}

fn initialize(db: &Connection, location: &Location) -> Result<()> {
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 0 {
        let tables: i64 = db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        require(tables == 0, "unsupported-schema")?;
        db.execute_batch(
            "CREATE TABLE archive_owner(
                singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                home_binding TEXT NOT NULL, state_binding TEXT NOT NULL, client TEXT NOT NULL
            );
            CREATE TABLE archive_days(
                client TEXT NOT NULL, date TEXT NOT NULL, revision TEXT NOT NULL,
                core TEXT NOT NULL, detail BLOB,
                protected_detail INTEGER NOT NULL CHECK(protected_detail IN (0,1)),
                PRIMARY KEY(client,date)
            ) WITHOUT ROWID;
            PRAGMA user_version=1;",
        )?;
        db.execute(
            "INSERT INTO archive_owner VALUES(1,?1,?2,?3)",
            params![location.home_binding, location.state_binding, CLIENT],
        )?;
    }
    verify_owner(db, location)
}

fn metadata(
    day: String,
    saved_revision: String,
    core_json: String,
    detail_available: bool,
    protected: i64,
) -> Result<Value> {
    date(&day)?;
    revision(&saved_revision)?;
    let core: Core = serde_json::from_str(&core_json)?;
    core.validate()?;
    require(
        matches!(protected, 0 | 1) && (core.pending_live_events == 0 || protected == 1),
        "invalid-stored-entry",
    )?;
    Ok(json!({
        "date":day,"revision":saved_revision,"core":core,
        "detailAvailable":detail_available,"protectedDetail":protected == 1
    }))
}

fn compress_detail(detail: &str) -> Result<Vec<u8>> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1)?;
    encoder.include_checksum(true)?;
    encoder.write_all(detail.as_bytes())?;
    let compressed = encoder.finish()?;
    require(
        compressed.len() <= MAX_COMPRESSED_DETAIL_BYTES,
        "detail-limit",
    )?;
    Ok(compressed)
}

fn decode_detail(value: ValueRef<'_>) -> Result<Option<String>> {
    let bytes = match value {
        ValueRef::Null => return Ok(None),
        // 兼容既有 schema 1 的 TEXT；读取不转存，也不解释消费者自己的编码。
        ValueRef::Text(bytes) => {
            require(bytes.len() <= MAX_DETAIL_BYTES, "invalid-stored-entry")?;
            bytes.to_vec()
        }
        ValueRef::Blob(bytes) => {
            require(
                bytes.len() <= MAX_COMPRESSED_DETAIL_BYTES,
                "invalid-stored-entry",
            )?;
            let mut decoder = zstd::stream::read::Decoder::with_buffer(bytes)
                .map_err(|_| ArchiveError::Failed("invalid-stored-entry"))?;
            decoder
                .window_log_max(23)
                .map_err(|_| ArchiveError::Failed("invalid-stored-entry"))?;
            let mut decoded = Vec::new();
            // 输出最多读取上限加一字节，不能信任压缩帧声明的原始长度。
            decoder
                .take((MAX_DETAIL_BYTES + 1) as u64)
                .read_to_end(&mut decoded)
                .map_err(|_| ArchiveError::Failed("invalid-stored-entry"))?;
            require(decoded.len() <= MAX_DETAIL_BYTES, "invalid-stored-entry")?;
            decoded
        }
        _ => return Err(ArchiveError::Failed("invalid-stored-entry")),
    };
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| ArchiveError::Failed("invalid-stored-entry"))
}

fn read_entry(db: &Connection, day: &str) -> Result<Option<Value>> {
    let mut statement = db.prepare(
        "SELECT revision,core,detail,protected_detail FROM archive_days
         WHERE client=?1 AND date=?2",
    )?;
    let mut rows = statement.query(params![CLIENT, day])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let detail = decode_detail(row.get_ref(2)?)?;
    let mut entry = metadata(
        day.to_owned(),
        row.get(0)?,
        row.get(1)?,
        detail.is_some(),
        row.get(3)?,
    )?;
    entry["detail"] = json!(detail);
    Ok(Some(entry))
}

fn check_detail_budget(db: &Connection, total_limit: usize) -> Result<()> {
    let mut statement =
        db.prepare("SELECT detail FROM archive_days WHERE client=?1 AND detail IS NOT NULL")?;
    let mut rows = statement.query([CLIENT])?;
    let mut bytes = 0_usize;
    while let Some(row) = rows.next()? {
        let detail = decode_detail(row.get_ref(0)?)?;
        bytes = bytes
            .checked_add(detail.as_ref().map_or(0, String::len))
            .filter(|bytes| *bytes <= total_limit)
            .ok_or(ArchiveError::Failed("detail-budget-exceeded"))?;
    }
    Ok(())
}

fn put(
    location: &Location,
    entry: Entry,
    expected: Option<String>,
    today: NaiveDate,
    total_limit: usize,
) -> Result<Value> {
    require(entry.version == 1, "unsupported-entry-version")?;
    date(&entry.date)?;
    entry.core.validate()?;
    require(entry.detail.len() <= MAX_DETAIL_BYTES, "detail-limit")?;
    require(
        entry.core.pending_live_events == 0 || entry.protected_detail,
        "pending-detail-must-be-protected",
    )?;
    if let Some(expected) = &expected {
        revision(expected)?;
    }
    let wanted_revision = format!("{:x}", Sha256::digest(canonical_json(&entry)?.as_bytes()));
    let core = canonical_json(&entry.core)?;
    let mut db = Connection::open_with_flags(
        location.database(true)?,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    db.busy_timeout(Duration::from_secs(2))?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    initialize(&tx, location)?;
    let current: Option<String> = tx
        .query_row(
            "SELECT revision FROM archive_days WHERE client=?1 AND date=?2",
            params![CLIENT, entry.date],
            |row| row.get(0),
        )
        .optional()?;
    // 提交成功但回复丢失时，原请求可原样重试；不能要求调用方先猜测写入结果。
    if current.as_deref() != Some(wanted_revision.as_str()) && current != expected {
        return Ok(json!({"status":"conflict","date":entry.date,"revision":current}));
    }
    if current.as_deref() != Some(wanted_revision.as_str()) {
        let detail = compress_detail(&entry.detail)?;
        tx.execute(
            "INSERT INTO archive_days(client,date,revision,core,detail,protected_detail)
             VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(client,date) DO UPDATE SET revision=excluded.revision,
                 core=excluded.core,detail=excluded.detail,protected_detail=excluded.protected_detail",
            params![
                CLIENT, entry.date, wanted_revision, core, detail, entry.protected_detail
            ],
        )?;
    }
    // 清理与新写入同一事务；限额失败也会回滚清理，旧数据保持原样。
    tx.execute(
        "UPDATE archive_days SET detail=NULL
         WHERE client=?1 AND protected_detail=0 AND date<?2 AND detail IS NOT NULL",
        params![CLIENT, (today - Days::days(30)).to_string()],
    )?;
    check_detail_budget(&tx, total_limit)?;
    let saved =
        read_entry(&tx, &entry.date)?.ok_or(ArchiveError::Failed("missing-committed-entry"))?;
    tx.commit()?;
    Ok(json!({"status":"ok","entry":saved}))
}

fn dispatch(
    home: &Path,
    settings: &Path,
    request: &Value,
    today: NaiveDate,
    total_limit: usize,
) -> Result<Value> {
    // 在复制大字符串或创建目录之前检查最外层大小。
    if let Some(detail) = request["entry"]["detail"].as_str() {
        require(detail.len() <= MAX_DETAIL_BYTES, "detail-limit")?;
    }
    if let Some(core) = request["entry"].get("core") {
        require(
            serde_json::to_vec(core)?.len() <= MAX_CORE_BYTES,
            "core-limit",
        )?;
    }
    let request: Request = serde_json::from_value(request.clone())?;
    let location = Location::new(home, settings)?;
    match request.operation {
        Operation::Put => {
            let entry = request
                .entry
                .ok_or(ArchiveError::Failed("entry-required"))?;
            require(
                request.date.as_ref().is_none_or(|day| day == &entry.date),
                "date-mismatch",
            )?;
            put(
                &location,
                entry,
                request.expected_revision,
                today,
                total_limit,
            )
        }
        operation => {
            require(
                request.entry.is_none() && request.expected_revision.is_none(),
                "invalid-operation-fields",
            )?;
            if let Some(day) = &request.date {
                date(day)?;
            }
            if matches!(operation, Operation::List) {
                require(request.date.is_none(), "invalid-operation-fields")?;
            } else {
                require(request.date.is_some(), "date-required")?;
            }
            let mut db = Connection::open_with_flags(
                location.database(false)?,
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            db.busy_timeout(Duration::from_secs(2))?;
            // 使用回滚日志数据库，读连接不创建数据库、迁移、采集器或 WAL/SHM。
            let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
            verify_owner(&tx, &location)?;
            if let Operation::Read = operation {
                let entry = read_entry(&tx, request.date.as_deref().unwrap())?
                    .ok_or(ArchiveError::Unavailable("entry-missing"))?;
                Ok(json!({"status":"ok","entry":entry}))
            } else {
                let mut statement = tx.prepare(
                    "SELECT date,revision,core,detail IS NOT NULL,protected_detail
                     FROM archive_days WHERE client=?1 ORDER BY date",
                )?;
                let rows = statement.query_map([CLIENT], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })?;
                let mut entries = Vec::new();
                for row in rows {
                    let (day, saved_revision, core, detail, protected) = row?;
                    entries.push(metadata(day, saved_revision, core, detail, protected)?);
                }
                Ok(json!({"status":"ok","entries":entries}))
            }
        }
    }
}

/// 路由先执行 usageHistoryEnabled 门控；这两个路径只能由宿主提供，不能来自请求。
pub fn query_archive(home: &Path, settings: &Path, request: &Value) -> Value {
    match dispatch(
        home,
        settings,
        request,
        Local::now().date_naive(),
        MAX_TOTAL_DETAIL_BYTES,
    ) {
        Ok(value) => value,
        Err(ArchiveError::Unavailable(code)) => json!({"status":"unavailable","code":code}),
        Err(ArchiveError::Failed(code)) => json!({"status":"failed","code":code}),
    }
}

#[cfg(test)]
mod tests;
