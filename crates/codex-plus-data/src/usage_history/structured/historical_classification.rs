//! 历史归类仅在读取已提交汇总时投影，不改写索引或严格归类结果。
use super::{observed_file_identity, unknown_model, unknown_provider};
use anyhow::{Context, ensure};
use chrono::{Local, NaiveDate, TimeZone};
use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

pub(super) const FILE_NAME: &str = "daily-historical-classification.json";
const MAX_BYTES: u64 = 4 * 1024 * 1024;

type Key<'a> = (&'a str, &'a str, &'a str, &'a str, &'a str, u8);
type Stamp = (u64, SystemTime, Option<(u64, u64)>);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    version: u32,
    cutoff_ms: i64,
    cutoff_day: String,
    rules: Vec<Rule>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Rule {
    day: String,
    session: String,
    model: String,
    provider: String,
    role: String,
    conflicts: u8,
    pub(super) estimated_model: Option<String>,
    pub(super) estimated_provider: Option<String>,
    pub(super) provider_split: Option<ProviderSplit>,
    #[serde(rename = "reason")]
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProviderSplit {
    pub(super) provider: String,
    pub(super) whole: SplitUsage,
    pub(super) matched: SplitUsage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SplitUsage {
    pub(super) records: usize,
    pub(super) input: u64,
    pub(super) cached: u64,
    pub(super) output: u64,
    pub(super) reasoning: Option<u64>,
}

impl Rule {
    fn key(&self) -> Key<'_> {
        (
            &self.day,
            &self.session,
            &self.model,
            &self.provider,
            &self.role,
            self.conflicts,
        )
    }
}

pub(super) struct Policy {
    cutoff_ms: i64,
    cutoff_day: String,
    rules: Vec<Rule>,
}

impl Policy {
    pub(super) fn find(&self, key: Key<'_>, latest_at: Option<i64>) -> Option<&Rule> {
        if key.0 >= self.cutoff_day.as_str()
            || latest_at.is_some_and(|timestamp| timestamp >= self.cutoff_ms)
        {
            return None;
        }
        let lookup = |key| {
            self.rules
                .binary_search_by(|rule| rule.key().cmp(&key))
                .ok()
                .map(|position| &self.rules[position])
        };
        lookup(key).or_else(|| lookup((key.0, "*", key.2, key.3, key.4, key.5)))
    }
}

pub(super) struct Loaded {
    pub(super) policy: Arc<Policy>,
    pub(super) revision: String,
}

struct Cached {
    path: PathBuf,
    stamp: Stamp,
    policy: Arc<Policy>,
    revision: String,
}

fn cache() -> &'static Mutex<Option<Cached>> {
    static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn stamp(path: &Path) -> anyhow::Result<Option<Stamp>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(metadata.is_file(), "历史归类策略不是普通文件");
    ensure!(metadata.len() <= MAX_BYTES, "历史归类策略超过 4 MiB");
    Ok(Some((
        metadata.len(),
        metadata.modified()?,
        observed_file_identity(path, &metadata)?,
    )))
}

fn parse(bytes: &[u8]) -> anyhow::Result<Policy> {
    let mut document: Document = serde_json::from_slice(bytes).context("历史归类策略格式无效")?;
    ensure!(document.version == 1, "历史归类策略版本无效");
    let day = NaiveDate::parse_from_str(&document.cutoff_day, "%Y-%m-%d")
        .context("历史归类截止日期无效")?;
    ensure!(
        day.to_string() == document.cutoff_day,
        "历史归类截止日期无效"
    );
    let midnight = day.and_hms_opt(0, 0, 0).expect("midnight is valid");
    let cutoff = Local
        .from_local_datetime(&midnight)
        .single()
        .context("历史归类截止时间不是本地唯一午夜")?;
    ensure!(
        cutoff.timestamp_millis() == document.cutoff_ms,
        "历史归类截止日期和时间不一致"
    );
    ensure!(
        document.cutoff_ms <= Local::now().timestamp_millis(),
        "历史归类截止时间不能位于未来"
    );
    let valid_text = |text: &str| text.len() <= 256 && !text.chars().any(char::is_control);
    for rule in &document.rules {
        let day =
            NaiveDate::parse_from_str(&rule.day, "%Y-%m-%d").context("历史归类规则日期无效")?;
        ensure!(
            day.to_string() == rule.day && rule.day < document.cutoff_day,
            "历史归类规则超出截止日期"
        );
        ensure!(
            !rule.session.is_empty()
                && [&rule.session, &rule.model, &rule.provider, &rule.role]
                    .into_iter()
                    .all(|text| valid_text(text)),
            "历史归类规则身份无效"
        );
        ensure!(
            rule.estimated_model.is_some() || rule.estimated_provider.is_some(),
            "历史归类规则没有估计值"
        );
        if let Some(model) = &rule.estimated_model {
            ensure!(
                valid_text(model)
                    && model.trim() == model
                    && !unknown_model(model)
                    && (rule.conflicts & 1 != 0 || unknown_model(&rule.model)),
                "历史归类规则模型无效或覆盖明确模型"
            );
        }
        if let Some(provider) = &rule.estimated_provider {
            ensure!(
                valid_text(provider)
                    && provider.trim() == provider
                    && !unknown_provider(provider)
                    && (rule.conflicts & 2 != 0 || unknown_provider(&rule.provider)),
                "历史归类规则供应商无效或覆盖明确供应商"
            );
        }
        if let Some(split) = &rule.provider_split {
            let whole = &split.whole;
            let matched = &split.matched;
            ensure!(
                rule.estimated_provider.is_some()
                    && unknown_provider(&rule.provider)
                    && rule.conflicts & 2 == 0
                    && valid_text(&split.provider)
                    && split.provider.trim() == split.provider
                    && !unknown_provider(&split.provider)
                    && rule.estimated_provider.as_deref() != Some(&split.provider)
                    && matched.records > 0
                    && matched.records < whole.records
                    && matched.input <= whole.input
                    && matched.output <= whole.output
                    && matched.cached <= whole.cached
                    && whole.cached <= whole.input
                    && matched.cached <= matched.input
                    && whole.cached - matched.cached <= whole.input - matched.input
                    && match (whole.reasoning, matched.reasoning) {
                        (None, None) => true,
                        (Some(total), Some(part)) => {
                            part <= total && part <= matched.output
                                && total - part <= whole.output - matched.output
                        }
                        _ => false,
                    },
                "历史归类分组拆分无效"
            );
        }
        ensure!(
            rule.reason
                .as_ref()
                .is_none_or(|reason| reason.len() <= 1024),
            "历史归类规则说明超过长度限制"
        );
    }
    document
        .rules
        .sort_unstable_by(|a, b| a.key().cmp(&b.key()));
    ensure!(
        document
            .rules
            .windows(2)
            .all(|pair| pair[0].key() != pair[1].key()),
        "历史归类规则重复"
    );
    Ok(Policy {
        cutoff_ms: document.cutoff_ms,
        cutoff_day: document.cutoff_day,
        rules: document.rules,
    })
}

pub(super) fn load(path: &Path) -> anyhow::Result<Option<Loaded>> {
    let Some(before) = stamp(path)? else {
        let mut cached = cache()
            .lock()
            .expect("historical classification cache poisoned");
        if cached.as_ref().is_some_and(|entry| entry.path == path) {
            *cached = None;
        }
        return Ok(None);
    };
    let mut cached = cache()
        .lock()
        .expect("historical classification cache poisoned");
    if let Some(entry) = cached
        .as_ref()
        .filter(|entry| entry.path == path && entry.stamp == before)
    {
        return Ok(Some(Loaded {
            policy: entry.policy.clone(),
            revision: entry.revision.clone(),
        }));
    }
    let mut bytes = Vec::with_capacity(before.0 as usize);
    File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "历史归类策略超过 4 MiB");
    ensure!(
        bytes.len() as u64 == before.0 && stamp(path)?.as_ref() == Some(&before),
        "历史归类策略读取时发生变化"
    );
    let policy = Arc::new(parse(&bytes)?);
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let revision = format!("v1-{:016x}", hasher.finish());
    *cached = Some(Cached {
        path: path.to_owned(),
        stamp: before,
        policy: policy.clone(),
        revision: revision.clone(),
    });
    Ok(Some(Loaded { policy, revision }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Days;
    use serde_json::{Value, json};

    fn document() -> Value {
        let cutoff = Local::now().date_naive();
        let midnight = Local
            .from_local_datetime(&cutoff.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .unwrap();
        json!({
            "version":1,
            "cutoffMs":midnight.timestamp_millis(),
            "cutoffDay":cutoff.to_string(),
            "rules":[{
                "day":cutoff.checked_sub_days(Days::new(1)).unwrap().to_string(),
                "session":"*","model":"","provider":"custom","role":"main","conflicts":0,
                "estimatedModel":"model-a","estimatedProvider":"provider-a",
                "reason":"上游主用模型"
            }]
        })
    }

    #[test]
    fn wildcard_is_bounded_and_exact_session_takes_precedence() {
        let mut value = document();
        let day = value["rules"][0]["day"].as_str().unwrap().to_owned();
        let mut exact = value["rules"][0].clone();
        exact["session"] = json!("session-a");
        exact["estimatedModel"] = json!("model-b");
        value["rules"].as_array_mut().unwrap().push(exact);
        let policy = parse(value.to_string().as_bytes()).unwrap();
        let key = (day.as_str(), "session-a", "", "custom", "main", 0);
        assert_eq!(
            policy.find(key, None).unwrap().estimated_model.as_deref(),
            Some("model-b")
        );
        assert_eq!(
            policy
                .find((key.0, "session-b", key.2, key.3, key.4, key.5), None)
                .unwrap()
                .estimated_model
                .as_deref(),
            Some("model-a")
        );
        assert!(policy.find(key, Some(policy.cutoff_ms)).is_none());
        assert!(
            policy
                .find((key.0, key.1, key.2, key.3, "subagent", key.5), None)
                .is_none()
        );
        assert!(
            policy
                .find(
                    (&policy.cutoff_day, key.1, key.2, key.3, key.4, key.5),
                    None
                )
                .is_none()
        );
    }

    #[test]
    fn rejects_future_cutoff_today_rules_and_overwriting_known_identity() {
        let mut today = document();
        today["rules"][0]["day"] = today["cutoffDay"].clone();
        assert!(parse(today.to_string().as_bytes()).is_err());

        let mut future = document();
        let tomorrow = Local::now()
            .date_naive()
            .checked_add_days(Days::new(1))
            .unwrap();
        let midnight = Local
            .from_local_datetime(&tomorrow.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .unwrap();
        future["cutoffDay"] = json!(tomorrow.to_string());
        future["cutoffMs"] = json!(midnight.timestamp_millis());
        assert!(parse(future.to_string().as_bytes()).is_err());

        for (field, known) in [("model", "known-model"), ("provider", "known-provider")] {
            let mut value = document();
            value["rules"][0][field] = json!(known);
            assert!(parse(value.to_string().as_bytes()).is_err());
        }

        let mut conflict = document();
        conflict["rules"][0]["model"] = json!("known-model");
        conflict["rules"][0]["conflicts"] = json!(1);
        assert!(parse(conflict.to_string().as_bytes()).is_ok());

        let mut duplicate = document();
        let row = duplicate["rules"][0].clone();
        duplicate["rules"].as_array_mut().unwrap().push(row);
        assert!(parse(duplicate.to_string().as_bytes()).is_err());
    }

    #[test]
    fn rejects_invalid_historical_provider_splits() {
        let mut value = document();
        value["rules"][0]["providerSplit"] = json!({
            "provider":"provider-b",
            "whole":{"records":2,"input":100,"cached":40,"output":10,"reasoning":5},
            "matched":{"records":1,"input":60,"cached":20,"output":6,"reasoning":3}
        });
        assert!(parse(value.to_string().as_bytes()).is_ok());
        for (pointer, invalid) in [
            ("/rules/0/providerSplit/matched/records", json!(2)),
            ("/rules/0/providerSplit/matched/cached", json!(80)),
            ("/rules/0/providerSplit/matched/reasoning", json!(null)),
            ("/rules/0/providerSplit/provider", json!("provider-a")),
        ] {
            let mut bad = value.clone();
            *bad.pointer_mut(pointer).unwrap() = invalid;
            assert!(parse(bad.to_string().as_bytes()).is_err());
        }
    }

    #[test]
    fn policy_revision_changes_on_update_and_deletion_is_reversible() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        assert!(load(&path).unwrap().is_none());
        let mut value = document();
        fs::write(&path, value.to_string()).unwrap();
        let first = load(&path).unwrap().unwrap();
        assert_eq!(load(&path).unwrap().unwrap().revision, first.revision);

        value["rules"][0]["estimatedModel"] = json!("different-model");
        fs::write(&path, value.to_string()).unwrap();
        let second = load(&path).unwrap().unwrap();
        assert_ne!(second.revision, first.revision);
        fs::remove_file(&path).unwrap();
        assert!(load(&path).unwrap().is_none());
        fs::write(&path, "{invalid").unwrap();
        assert!(load(&path).is_err());
        fs::write(&path, value.to_string()).unwrap();
        assert_eq!(load(&path).unwrap().unwrap().revision, second.revision);
    }
}
