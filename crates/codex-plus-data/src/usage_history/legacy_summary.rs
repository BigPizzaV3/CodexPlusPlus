//! 一次性导出的旧版日汇总只作为独立来源，不与原生用量相加。

use super::{NaiveDate, Query, storage_dir};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

const FILE_NAME: &str = "daily-legacy-summary.json";
const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyFile {
    version: u32,
    captured_at: String,
    scope: String,
    coverage: String,
    days: BTreeMap<String, Value>,
}

struct LegacySummary {
    captured_at: String,
    days: BTreeMap<NaiveDate, Value>,
}

struct CachedSummary {
    path: PathBuf,
    length: u64,
    modified: SystemTime,
    summary: Arc<LegacySummary>,
}

fn cache() -> &'static Mutex<Option<CachedSummary>> {
    static CACHE: OnceLock<Mutex<Option<CachedSummary>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn retain_fields(value: &mut Value, fields: &[&str]) -> Result<(), &'static str> {
    let object = value.as_object_mut().ok_or("旧摘要格式无效")?;
    object.retain(|key, _| fields.contains(&key.as_str()));
    Ok(())
}

fn sanitize_day(date: &str, value: &mut Value) -> Result<NaiveDate, &'static str> {
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| "旧摘要日期无效")?;
    if value["date"].as_str() != Some(date.to_string().as_str()) || !value["recordCount"].is_u64() {
        return Err("旧摘要日期或计数无效");
    }
    const USAGE: &[&str] = &[
        "inputTokens",
        "outputTokens",
        "cachedInputTokens",
        "reasoningOutputTokens",
        "totalTokens",
        "cachedKnown",
        "reasoningKnown",
        "inputKnown",
        "outputKnown",
        "totalKnown",
        "usageUncertain",
    ];
    fn usage(value: &mut Value) -> Result<(), &'static str> {
        if !value["inputTokens"].is_u64() || !value["outputTokens"].is_u64() {
            return Err("旧摘要用量无效");
        }
        retain_fields(value, USAGE)
    }
    usage(&mut value["usage"])?;
    for (field, allowed) in [
        (
            "models",
            &[
                "model",
                "usage",
                "recordCount",
                "modelInferred",
                "inferredModelTokens",
            ][..],
        ),
        (
            "breakdowns",
            &[
                "model",
                "provider",
                "providerName",
                "role",
                "usage",
                "recordCount",
                "modelConflict",
                "providerConflict",
                "providerInferred",
                "modelInferred",
                "inferredTokens",
                "inferredModelTokens",
                "providerSource",
                "modelSource",
            ][..],
        ),
    ] {
        let rows = value[field].as_array_mut().ok_or("旧摘要分类无效")?;
        for row in rows {
            if !row["recordCount"].is_u64() {
                return Err("旧摘要分类计数无效");
            }
            usage(&mut row["usage"])?;
            retain_fields(row, allowed)?;
        }
    }
    retain_fields(
        &mut value["legacyStatus"],
        &[
            "liveCaptureTruncated",
            "turnsTruncated",
            "pendingHistoryReplacement",
            "archiveDetailPending",
        ],
    )?;
    retain_fields(
        value,
        &[
            "date",
            "usage",
            "models",
            "breakdowns",
            "recordCount",
            "updatedAt",
            "legacyStatus",
        ],
    )?;
    Ok(date)
}

fn load(path: &Path) -> Result<Option<Arc<LegacySummary>>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取旧摘要状态失败: {error}")),
    };
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err("旧摘要不是普通文件或超过 8 MiB 内存预算".into());
    }
    let modified = metadata.modified().map_err(|error| error.to_string())?;
    let mut cache = cache().lock().expect("legacy summary cache poisoned");
    if let Some(cached) = cache.as_ref().filter(|cached| {
        cached.path == path && cached.length == metadata.len() && cached.modified == modified
    }) {
        return Ok(Some(cached.summary.clone()));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("读取旧摘要失败: {error}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("旧摘要超过 8 MiB 内存预算".into());
    }
    let legacy: LegacyFile =
        serde_json::from_slice(&bytes).map_err(|error| format!("旧摘要格式无效: {error}"))?;
    if legacy.version != 1
        || legacy.scope != "all"
        || legacy.coverage != "partial"
        || chrono::DateTime::parse_from_rfc3339(&legacy.captured_at).is_err()
    {
        return Err("旧摘要版本、作用域或覆盖状态无效".into());
    }
    let mut days = BTreeMap::new();
    for (key, mut value) in legacy.days {
        let date = sanitize_day(&key, &mut value).map_err(str::to_owned)?;
        if days.insert(date, value).is_some() {
            return Err("旧摘要存在重复日期".into());
        }
    }
    let summary = Arc::new(LegacySummary {
        captured_at: legacy.captured_at,
        days,
    });
    *cache = Some(CachedSummary {
        path: path.to_owned(),
        length: metadata.len(),
        modified,
        summary: summary.clone(),
    });
    Ok(Some(summary))
}

pub(super) fn attach(
    response: &mut Value,
    home: &Path,
    settings: &Path,
    query: &Query,
    request: &Value,
) {
    if request["includeLegacySummary"] != true
        || response["status"] == "unavailable"
        || !request["scopeSessionId"].is_null()
        || query.session_id.is_some()
        || query.model.is_some()
        || query.turn_id.is_some()
        || !query.search.is_empty()
    {
        return;
    }
    match load(&storage_dir(home, settings).join(FILE_NAME)) {
        Ok(Some(summary)) => {
            let days: BTreeMap<_, _> = summary
                .days
                .iter()
                .filter(|(date, _)| query.from.is_none_or(|from| **date >= from))
                .filter(|(date, _)| query.to.is_none_or(|to| **date <= to))
                .map(|(date, row)| (date.to_string(), row))
                .collect();
            response["legacyDays"] = json!(days);
            response["legacyCapturedAt"] = json!(summary.captured_at);
            response["legacyStatus"] = json!({"available":true,"scope":"all","coverage":"partial"});
        }
        Ok(None) => {}
        Err(error) => {
            response["legacyStatus"] = json!({"available":false,"message":error});
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    fn fixture() -> Value {
        json!({
            "version":1,"capturedAt":"2026-11-10T04:00:00Z","scope":"all",
            "coverage":"partial","days":{
                "2026-11-09":{"date":"2026-11-09","usage":{"inputTokens":80,"outputTokens":20},
                    "recordCount":1,"models":[],"breakdowns":[],"legacyStatus":{}},
                "2026-11-10":{"date":"2026-11-10","usage":{"inputTokens":5,"outputTokens":2},
                    "recordCount":1,"models":[],"breakdowns":[],"legacyStatus":{},
                    "privateCommand":"must not be returned"}
            }
        })
    }

    fn setup(value: &Value) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        fs::create_dir(&home).unwrap();
        let settings = root.path().join("settings.json");
        let dir = storage_dir(&home, &settings);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE_NAME), serde_json::to_vec(value).unwrap()).unwrap();
        (root, home, settings)
    }

    fn response(home: &Path, settings: &Path, request: Value) -> Value {
        let query = Query::parse(&request, Local::now().date_naive()).unwrap();
        let mut response =
            json!({"status":"partial","days":[{"date":"2026-11-10","recordCount":4}]});
        attach(&mut response, home, settings, &query, &request);
        response
    }

    #[test]
    fn legacy_is_separate_scoped_and_filtered_by_date() {
        let (_root, home, settings) = setup(&fixture());
        let result = response(
            &home,
            &settings,
            json!({"from":"2026-11-10","to":"2026-11-10","includeLegacySummary":true}),
        );
        assert_eq!(result["days"][0]["recordCount"], 4);
        assert_eq!(result["legacyDays"]["2026-11-10"]["recordCount"], 1);
        assert!(result["legacyDays"]["2026-11-09"].is_null());
        assert!(result.to_string().find("must not be returned").is_none());
        assert_eq!(result["legacyStatus"]["coverage"], "partial");
    }

    #[test]
    fn legacy_is_not_returned_for_filtered_scope() {
        let (_root, home, settings) = setup(&fixture());
        for request in [
            json!({"scopeSessionId":"00000000-0000-0000-0000-000000000001","includeLegacySummary":true}),
            json!({"sessionId":"00000000-0000-0000-0000-000000000001","includeLegacySummary":true}),
            json!({"model":"example","includeLegacySummary":true}),
            json!({"turnId":"turn","includeLegacySummary":true}),
            json!({"search":"example","includeLegacySummary":true}),
            json!({}),
        ] {
            assert!(response(&home, &settings, request)["legacyDays"].is_null());
        }
    }

    #[test]
    fn invalid_summary_reports_error_without_changing_native_counts() {
        let mut invalid = fixture();
        invalid["scope"] = json!("current");
        let (_root, home, settings) = setup(&invalid);
        let result = response(&home, &settings, json!({"includeLegacySummary":true}));
        assert_eq!(result["days"][0]["recordCount"], 4);
        assert_eq!(result["legacyStatus"]["available"], false);
        assert!(result["legacyDays"].is_null());
    }

    #[test]
    fn malformed_date_is_rejected_instead_of_relabelled() {
        let mut invalid = fixture();
        invalid["days"]["2026-11-10"]["date"] = json!("2026-11-09");
        let (_root, home, settings) = setup(&invalid);
        let result = response(&home, &settings, json!({"includeLegacySummary":true}));
        assert_eq!(result["legacyStatus"]["available"], false);
        assert!(result["legacyDays"].is_null());
    }
}
