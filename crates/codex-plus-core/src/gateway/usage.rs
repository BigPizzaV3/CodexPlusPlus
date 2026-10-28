//! 用量统计：每个请求结束时按「日期 × Agent × 供应商 × 模型」累加，存在 `<状态目录>/gateway-usage.json`。
//!
//! 请求日志会被裁剪到 1MB 左右，撑不起 30 天 / 全部的统计，所以单独存按天汇总的数字（不含请求内容）。
//! 统计覆盖经过 Codex++ 的全部流量：网关里的 Agent 按自己的名字，Codex 自己的请求记为 `codex`。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::gateway::store::{GatewaySettings, ProviderPrice};
use crate::request_log::RequestLogEntry;

const USAGE_FILE: &str = "gateway-usage.json";

/// 一组请求的累计数字。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCounts {
    #[serde(default)]
    pub calls: u64,
    #[serde(default)]
    pub errors: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    /// 输入里命中缓存的部分（已包含在 `input_tokens` 里）。
    #[serde(default)]
    pub cached_tokens: u64,
    /// 推理 token（已包含在 `output_tokens` 里）。
    #[serde(default)]
    pub reasoning_tokens: u64,
    /// 记了耗时的请求的总耗时（毫秒）；旧数据没有，平均耗时只按 `timed_calls` 算。
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub timed_calls: u64,
}

impl UsageCounts {
    fn add(&mut self, other: &UsageCounts) {
        self.calls += other.calls;
        self.errors += other.errors;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cached_tokens += other.cached_tokens;
        self.reasoning_tokens += other.reasoning_tokens;
        self.duration_ms += other.duration_ms;
        self.timed_calls += other.timed_calls;
    }
}

/// 某一天里某个 Agent 经某个供应商用某个模型的累计。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub agent: String,
    pub provider_id: String,
    pub provider: String,
    pub model: String,
    #[serde(flatten)]
    pub counts: UsageCounts,
    /// 不认识的字段（更新版本加的统计项）原样保留：回滚后这一版继续累加时不把它们丢掉。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageFile {
    #[serde(default)]
    version: u32,
    /// `YYYY-MM-DD`（本地日期）→ 当天各组合的累计。
    #[serde(default)]
    days: BTreeMap<String, Vec<UsageRow>>,
    /// 不认识的顶层字段原样保留。
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct UsageStore {
    pub path: PathBuf,
}

impl Default for UsageStore {
    fn default() -> Self {
        Self {
            path: crate::paths::default_app_state_dir().join(USAGE_FILE),
        }
    }
}

impl UsageStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn load(&self) -> anyhow::Result<UsageFile> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) if text.trim().is_empty() => Ok(UsageFile::default()),
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("无法解析 {}", self.path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(UsageFile::default()),
            Err(error) => Err(error).with_context(|| format!("无法读取 {}", self.path.display())),
        }
    }

    fn save(&self, file: &UsageFile) -> anyhow::Result<()> {
        let mut file = file.clone();
        file.version = 1;
        let text = serde_json::to_string(&file)?;
        crate::settings::atomic_write(&self.path, text.as_bytes())
            .with_context(|| format!("无法写入 {}", self.path.display()))
    }

    /// 记一个请求。`day` 是本地日期 `YYYY-MM-DD`。
    pub fn record(&self, day: &str, row: UsageRow) -> anyhow::Result<()> {
        let _guard = write_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut file = self.load()?;
        let rows = file.days.entry(day.to_string()).or_default();
        match rows.iter_mut().find(|known| {
            known.agent == row.agent
                && known.provider_id == row.provider_id
                && known.model == row.model
        }) {
            Some(known) => {
                known.counts.add(&row.counts);
                known.provider = row.provider;
            }
            None => rows.push(row),
        }
        self.save(&file)
    }

    /// 清空全部用量统计。
    pub fn clear(&self) -> anyhow::Result<()> {
        let _guard = write_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.save(&UsageFile::default())
    }

    /// 汇总 `days` 天（含今天；None 表示全部）的用量。`today` 是本地日期。
    pub fn summary(
        &self,
        today: &str,
        days: Option<u32>,
        gateway: &GatewaySettings,
    ) -> anyhow::Result<UsageSummary> {
        let file = self.load()?;
        let first = match days {
            Some(days) => shift_day(today, -(i64::from(days.max(1)) - 1)),
            None => file
                .days
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| today.to_string()),
        };
        let mut summary = UsageSummary::default();
        let mut day = first.clone();
        // 逐天补齐，没有请求的日子也画一根空柱子。
        let mut guard = 0;
        while day.as_str() <= today && guard < 3700 {
            let rows = file.days.get(&day).cloned().unwrap_or_default();
            let mut counts = UsageCounts::default();
            for row in &rows {
                counts.add(&row.counts);
                summary.totals.add(&row.counts);
                let price = gateway
                    .provider(&row.provider_id)
                    .and_then(|provider| provider.price_for(&row.model));
                add_group(
                    &mut summary.by_agent,
                    &row.agent,
                    &row.agent,
                    &row.counts,
                    price,
                );
                add_group(
                    &mut summary.by_provider,
                    &row.provider_id,
                    &row.provider,
                    &row.counts,
                    price,
                );
                add_group(
                    &mut summary.by_model,
                    &row.model,
                    &row.model,
                    &row.counts,
                    price,
                );
                if let Some(price) = price {
                    add_cost(&mut summary.costs, price, &row.counts);
                }
            }
            summary.days.push(UsageDay {
                date: day.clone(),
                counts,
            });
            day = shift_day(&day, 1);
            guard += 1;
        }
        for groups in [
            &mut summary.by_agent,
            &mut summary.by_provider,
            &mut summary.by_model,
        ] {
            groups.sort_by(|left, right| {
                (right.counts.input_tokens + right.counts.output_tokens)
                    .cmp(&(left.counts.input_tokens + left.counts.output_tokens))
                    .then(right.counts.calls.cmp(&left.counts.calls))
            });
        }
        Ok(summary)
    }
}

fn write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(Mutex::default)
}

fn add_group(
    groups: &mut Vec<UsageGroup>,
    key: &str,
    label: &str,
    counts: &UsageCounts,
    price: Option<&ProviderPrice>,
) {
    let index = match groups.iter().position(|group| group.key == key) {
        Some(index) => index,
        None => {
            groups.push(UsageGroup {
                key: key.to_string(),
                label: label.to_string(),
                ..UsageGroup::default()
            });
            groups.len() - 1
        }
    };
    let group = &mut groups[index];
    group.counts.add(counts);
    if let Some(price) = price {
        add_cost(&mut group.costs, price, counts);
    }
}

/// 按每百万 token 的价格估算费用：未命中缓存的输入、命中缓存的输入、输出分别计价。
fn add_cost(costs: &mut Vec<UsageCost>, price: &ProviderPrice, counts: &UsageCounts) {
    let uncached = counts.input_tokens.saturating_sub(counts.cached_tokens) as f64;
    let amount = (uncached * price.input
        + counts.cached_tokens as f64 * price.cached_input
        + counts.output_tokens as f64 * price.output)
        / 1_000_000.0;
    let currency = if price.currency.trim().is_empty() {
        "USD".to_string()
    } else {
        price.currency.trim().to_ascii_uppercase()
    };
    match costs.iter_mut().find(|cost| cost.currency == currency) {
        Some(cost) => cost.amount += amount,
        None => costs.push(UsageCost { currency, amount }),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDay {
    pub date: String,
    #[serde(flatten)]
    pub counts: UsageCounts,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroup {
    pub key: String,
    pub label: String,
    #[serde(flatten)]
    pub counts: UsageCounts,
    /// 这一组里填了价格的部分估算出的费用。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub costs: Vec<UsageCost>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCost {
    pub currency: String,
    pub amount: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub totals: UsageCounts,
    pub days: Vec<UsageDay>,
    pub by_agent: Vec<UsageGroup>,
    pub by_provider: Vec<UsageGroup>,
    pub by_model: Vec<UsageGroup>,
    /// 只统计填了价格的网关供应商。
    pub costs: Vec<UsageCost>,
}

/// 本地日期（`YYYY-MM-DD`）。
pub fn local_day(timestamp_ms: u64) -> String {
    rusqlite::Connection::open_in_memory()
        .and_then(|connection| {
            connection.query_row(
                "SELECT date(?1 / 1000, 'unixepoch', 'localtime')",
                [timestamp_ms as i64],
                |row| row.get::<_, String>(0),
            )
        })
        .unwrap_or_else(|_| "1970-01-01".to_string())
}

/// 日期加减天数。
pub fn shift_day(day: &str, delta: i64) -> String {
    rusqlite::Connection::open_in_memory()
        .and_then(|connection| {
            connection.query_row(
                "SELECT date(?1, ?2 || ' days')",
                rusqlite::params![day, delta],
                |row| row.get::<_, String>(0),
            )
        })
        .unwrap_or_else(|_| day.to_string())
}

/// 请求结束时记一笔（从请求日志条目取数）。失败不影响请求本身。
pub fn record_entry(entry: &RequestLogEntry) {
    let _ = record_entry_in(&UsageStore::default(), entry);
}

pub fn record_entry_in(store: &UsageStore, entry: &RequestLogEntry) -> anyhow::Result<()> {
    if entry.state.as_deref() == Some("running") {
        return Ok(());
    }
    let usage = entry.usage.clone().unwrap_or_default();
    let model = if entry.upstream_model.trim().is_empty() {
        entry.model.trim()
    } else {
        entry.upstream_model.trim()
    };
    let row = UsageRow {
        agent: entry
            .agent
            .clone()
            .filter(|agent| !agent.trim().is_empty())
            .unwrap_or_else(|| "codex".to_string()),
        provider_id: entry.relay_id.clone(),
        provider: entry.relay_name.clone(),
        model: model.to_string(),
        counts: UsageCounts {
            calls: 1,
            errors: u64::from(!entry.success),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_tokens: usage.cached_tokens,
            reasoning_tokens: usage.reasoning_tokens,
            duration_ms: entry.duration_ms,
            timed_calls: u64::from(entry.duration_ms > 0),
        },
        extra: Default::default(),
    };
    let timestamp = if entry.timestamp_ms > 0 {
        entry.timestamp_ms
    } else {
        entry.started_at_ms
    };
    store.record(&local_day(timestamp), row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::store::GatewayProvider;
    use crate::request_log::RequestUsage;
    use crate::settings::RelayProfile;

    fn entry(
        agent: Option<&str>,
        relay: &str,
        input: u64,
        output: u64,
        cached: u64,
    ) -> RequestLogEntry {
        RequestLogEntry {
            id: "id".to_string(),
            timestamp_ms: 1_790_000_000_000,
            started_at_ms: 0,
            duration_ms: 1200,
            method: "POST".to_string(),
            path: "/v1/messages".to_string(),
            status_code: 200,
            success: true,
            stream: true,
            model: "claude-sonnet-5".to_string(),
            thread_id: None,
            upstream_model: "deepseek-v4-pro".to_string(),
            relay_id: relay.to_string(),
            relay_name: relay.to_uppercase(),
            wire_api: "ChatCompletions".to_string(),
            attempts: 1,
            auto_retries: 0,
            retry_reasons: Vec::new(),
            attempt_details: Vec::new(),
            state: None,
            first_token_ms: None,
            estimated_output_tokens: None,
            phase: None,
            error: None,
            stream_outcome: None,
            usage: Some(RequestUsage {
                input_tokens: input,
                output_tokens: output,
                cached_tokens: cached,
                reasoning_tokens: 1,
            }),
            cache_hit_ratio: None,
            agent: agent.map(str::to_string),
        }
    }

    #[test]
    fn days_shift_and_format() {
        assert_eq!(shift_day("2026-10-01", -1), "2026-09-30");
        assert_eq!(shift_day("2026-12-31", 1), "2027-01-01");
        assert_eq!(local_day(0).len(), 10);
    }

    #[test]
    fn records_accumulate_per_day_and_summarize_with_costs() {
        let temp = tempfile::tempdir().unwrap();
        let store = UsageStore::at(temp.path().join("usage.json"));
        record_entry_in(
            &store,
            &entry(Some("claude"), "ds", 1_000_000, 200_000, 400_000),
        )
        .unwrap();
        record_entry_in(&store, &entry(Some("claude"), "ds", 1_000_000, 0, 0)).unwrap();
        record_entry_in(&store, &entry(None, "codex-relay", 10, 5, 0)).unwrap();
        let mut failed = entry(Some("grok"), "ds", 0, 0, 0);
        failed.success = false;
        failed.usage = None;
        record_entry_in(&store, &failed).unwrap();

        let day = local_day(1_790_000_000_000);
        let mut gateway = GatewaySettings::default();
        gateway.providers.push(GatewayProvider {
            enabled: true,
            source: String::new(),
            model_prices: Default::default(),
            model_windows: Default::default(),
            context_window: None,
            proxy: String::new(),
            price: Some(ProviderPrice {
                currency: "cny".to_string(),
                input: 2.0,
                cached_input: 0.5,
                output: 8.0,
            }),
            upstream_new: Vec::new(),
            upstream_gone: Vec::new(),
            models_checked_at_ms: 0,
            relay: RelayProfile {
                id: "ds".to_string(),
                ..RelayProfile::default()
            },
            extra: Default::default(),
        });
        let summary = store.summary(&day, Some(7), &gateway).unwrap();
        assert_eq!(summary.days.len(), 7);
        assert_eq!(summary.days.last().unwrap().date, day);
        assert_eq!(summary.totals.calls, 4);
        assert_eq!(summary.totals.errors, 1);
        assert_eq!(summary.totals.input_tokens, 2_000_010);
        assert_eq!(summary.totals.cached_tokens, 400_000);
        let agents = summary
            .by_agent
            .iter()
            .map(|group| (group.key.as_str(), group.counts.calls))
            .collect::<Vec<_>>();
        assert_eq!(agents, vec![("claude", 2), ("codex", 1), ("grok", 1)]);
        // (2M - 0.4M) × 2 + 0.4M × 0.5 + 0.2M × 8 = 3.2 + 0.2 + 1.6 = 5.0 元
        assert_eq!(summary.costs.len(), 1);
        assert_eq!(summary.costs[0].currency, "CNY");
        assert!((summary.costs[0].amount - 5.0).abs() < 1e-9);
        let ds = summary
            .by_provider
            .iter()
            .find(|group| group.key == "ds")
            .unwrap();
        assert!((ds.costs[0].amount - 5.0).abs() < 1e-9, "每组也算出费用");
        assert!(
            summary
                .by_provider
                .iter()
                .find(|group| group.key == "codex-relay")
                .unwrap()
                .costs
                .is_empty()
        );
        assert_eq!(summary.totals.timed_calls, 4);
        assert_eq!(summary.totals.duration_ms, 4 * 1200);

        let all = store.summary(&day, None, &gateway).unwrap();
        assert_eq!(all.days.len(), 1);
        store.clear().unwrap();
        assert_eq!(store.summary(&day, None, &gateway).unwrap().totals.calls, 0);
    }

    /// 更新版本加的统计项、顶层字段：这一版继续记账时原样保留。
    #[test]
    fn unknown_fields_survive_recording() {
        let temp = tempfile::tempdir().unwrap();
        let store = UsageStore::at(temp.path().join("gateway-usage.json"));
        let day = local_day(1_790_000_000_000);
        let text = serde_json::json!({
            "version": 9,
            "futureTop": "keep",
            "days": { day.clone(): [{
                "agent": "claude", "providerId": "ds", "provider": "DS",
                "model": "deepseek-v4-pro", "calls": 2, "cacheWriteTokens": 77
            }]}
        });
        std::fs::write(&store.path, text.to_string()).unwrap();
        record_entry_in(&store, &entry(Some("claude"), "ds", 10, 5, 0)).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&store.path).unwrap()).unwrap();
        assert_eq!(saved["futureTop"], "keep");
        let row = &saved["days"][&day][0];
        assert_eq!(row["calls"], 3);
        assert_eq!(row["cacheWriteTokens"], 77);
    }
}
