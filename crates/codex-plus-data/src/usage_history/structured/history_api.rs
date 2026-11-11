//! 已提交持久索引的用量读取；扫描覆盖证明由调用方另行提供。
use super::{
    Aggregate, Query, Tokens, execution_api, historical_classification::Policy, unknown_model,
    unknown_provider,
};
use anyhow::{Context, ensure};
use chrono::{DateTime, Days, Local, TimeZone};
use rusqlite::{Connection, params_from_iter, types::Value as SqlValue};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
struct Day {
    total: Aggregate,
    models: BTreeMap<String, Aggregate>,
    breakdowns: BTreeMap<(String, String, String, bool, bool), Aggregate>,
}

#[derive(Clone, Copy)]
struct Classification<'a> {
    model: &'a str,
    provider: &'a str,
    role: &'a str,
    model_inferred: bool,
    provider_inferred: bool,
}

fn add(aggregate: &mut Aggregate, tokens: Tokens, records: usize) -> anyhow::Result<()> {
    aggregate.records = aggregate
        .records
        .checked_add(records)
        .context("用量记录数超出范围")?;
    aggregate.usage.add(tokens);
    Ok(())
}

impl Day {
    fn add(
        &mut self,
        classification: Classification<'_>,
        tokens: Tokens,
        records: usize,
    ) -> anyhow::Result<()> {
        add(&mut self.total, tokens, records)?;
        add(
            self.models.entry(classification.model.to_owned()).or_default(),
            tokens,
            records,
        )?;
        add(
            self.breakdowns
                .entry((
                    classification.model.to_owned(),
                    classification.provider.to_owned(),
                    classification.role.to_owned(),
                    classification.model_inferred,
                    classification.provider_inferred,
                ))
                .or_default(),
            tokens,
            records,
        )
    }

    fn value(self, provider_names: &BTreeMap<String, String>) -> Value {
        let mut value = self.total.value();
        value["models"] = json!(
            self.models
                .into_iter()
                .map(|(model, aggregate)| {
                    let mut row = aggregate.value();
                    row["model"] = json!(model);
                    row
                })
                .collect::<Vec<_>>()
        );
        value["breakdowns"] = json!(
            self.breakdowns
                .into_iter()
                .map(|((model, provider, role, model_inferred, provider_inferred), aggregate)| {
                    let mut row = aggregate.value();
                    row["providerName"] = json!(provider_names.get(&provider));
                    row["model"] = json!(model);
                    row["provider"] = json!(provider);
                    row["role"] = json!(role);
                    if model_inferred {
                        row["modelInferred"] = json!(true);
                    }
                    if provider_inferred {
                        row["providerInferred"] = json!(true);
                    }
                    row
                })
                .collect::<Vec<_>>()
        );
        value
    }
}

fn day_values(
    days: BTreeMap<String, Day>,
    provider_names: &BTreeMap<String, String>,
) -> Vec<Value> {
    days.into_iter()
        .map(|(date, day)| {
            let mut value = day.value(provider_names);
            value["date"] = json!(date);
            value
        })
        .collect()
}

struct Trend {
    start: i64,
    end: i64,
    interval: i64,
    day_precision: bool,
    buckets: Vec<(i64, i64, Day)>,
}

impl Trend {
    fn new(start: i64, end: i64, day_precision: bool) -> anyhow::Result<Self> {
        let span = end.checked_sub(start).context("用量时间范围超出支持范围")?;
        let minimum = ((span - 1) / 120 + 1).max(if day_precision { 86_400_000 } else { 60_000 });
        let interval = [1, 5, 15, 30, 60, 180, 360, 720, 1440, 2880, 10080, 43200]
            .into_iter()
            .map(|minutes| minutes * 60_000i64)
            .find(|interval| *interval >= minimum)
            .unwrap_or(((minimum - 1) / 86_400_000 + 1) * 86_400_000);
        let mut buckets = Vec::new();
        let mut at = start;
        while at < end {
            let next = if day_precision {
                // 日汇总按本地日历分桶，夏令时切换也不把相邻日期合并到同一桶。
                DateTime::from_timestamp_millis(at)
                    .context("用量时间范围超出支持范围")?
                    .with_timezone(&Local)
                    .checked_add_days(Days::new((interval / 86_400_000) as u64))
                    .context("用量日期桶超出支持范围")?
                    .timestamp_millis()
            } else {
                at.saturating_add(interval)
            };
            buckets.push((at, next.min(end), Day::default()));
            at = next;
        }
        ensure!(buckets.len() <= 120, "用量趋势桶超出范围");
        Ok(Self { start, end, interval, day_precision, buckets })
    }

    fn add(
        &mut self,
        at: i64,
        classification: Classification<'_>,
        tokens: Tokens,
        records: usize,
    ) -> anyhow::Result<()> {
        let position = self.buckets.partition_point(|(_, end, _)| *end <= at);
        if let Some((_, _, bucket)) = self.buckets.get_mut(position) {
            bucket.add(classification, tokens, records)?;
        }
        Ok(())
    }

    fn value(self, provider_names: &BTreeMap<String, String>) -> Value {
        let buckets: Vec<_> = self.buckets.into_iter().map(|(start, end, day)| {
            let mut value = day.value(provider_names);
            value["startAt"] = json!(start);
            value["endAt"] = json!(end);
            value
        }).collect();
        json!({
            "startAt":self.start,"endAt":self.end,"intervalMs":self.interval,
            "precision":if self.day_precision {"day"} else {"exact"},"buckets":buckets
        })
    }
}

/// 日范围和精确时间同时生效；所有状态与结果来自同一只读快照。
#[cfg(test)]
pub(super) fn query(
    db: &mut Connection,
    options: &Query,
    request: &Value,
    provider_names: &BTreeMap<String, String>,
) -> anyhow::Result<Value> {
    query_with_policy(db, options, request, provider_names, None)
}

pub(super) fn query_with_policy(
    db: &mut Connection,
    options: &Query,
    request: &Value,
    provider_names: &BTreeMap<String, String>,
    policy: Option<&Policy>,
) -> anyhow::Result<Value> {
    ensure!(
        options.turn_id.is_none() && !options.include_activity && !options.include_session_cursors,
        "持久索引暂不支持此用量查询"
    );
    let bounds = match (
        request.get("startAt").filter(|value| !value.is_null()),
        request.get("endAt").filter(|value| !value.is_null()),
    ) {
        (None, None) => None,
        (Some(start), Some(end)) => {
            let start = start.as_i64().context("用量开始时间无效")?;
            let end = end.as_i64().context("用量结束时间无效")?;
            ensure!(start < end, "用量时间范围无效");
            Some((start, end))
        }
        _ => anyhow::bail!("用量时间范围不完整"),
    };
    let scope = request
        .get("scopeSessionId")
        .map(|value| {
            value
                .as_str()
                .and_then(super::super::normalize_session_id)
                .context("会话范围 ID 无效")
        })
        .transpose()?;
    let tx = db.transaction()?;
    let revision: i64 = tx.query_row(
        "SELECT revision FROM usage_meta WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let day_bounds = bounds.and_then(|(start, end)| {
        let day = |timestamp| {
            let local = DateTime::from_timestamp_millis(timestamp)?.with_timezone(&Local);
            let midnight = local.date_naive().and_hms_opt(0, 0, 0)?;
            (local.naive_local() == midnight
                && Local.from_local_datetime(&midnight).single()?.timestamp_millis() == timestamp)
                .then_some(local.date_naive())
        };
        Some((day(start)?, day(end)?))
    });
    let mut daily = version >= 6 && (bounds.is_none() || day_bounds.is_some());
    let sessions = execution_api::scope_sessions(&tx, scope.as_deref())?;
    let (day_only, scoped_day_only, sealed_review, scoped_sealed_review) = if version >= 9 {
        let mut first = options.from;
        let mut last = options.to;
        if let Some((start, end)) = bounds {
            let local_day = |timestamp| {
                DateTime::from_timestamp_millis(timestamp)
                    .map(|time| time.with_timezone(&Local).date_naive())
                    .context("用量时间范围超出支持范围")
            };
            let start_day = local_day(start)?;
            let end_day = local_day(end - 1)?;
            first = Some(first.map_or(start_day, |day| day.max(start_day)));
            last = Some(last.map_or(end_day, |day| day.min(end_day)));
        }
        if first.zip(last).is_some_and(|(first, last)| first > last) {
            (false, false, false, false)
        } else {
            let mut values = Vec::<SqlValue>::new();
            let mut clauses = Vec::<String>::new();
            for (column, value) in [
                ("day >=", first.map(|day| day.to_string())),
                ("day <=", last.map(|day| day.to_string())),
                ("session =", options.session_id.clone()),
            ] {
                if let Some(value) = value {
                    values.push(SqlValue::Text(value));
                    clauses.push(format!("{column} ?{}", values.len()));
                }
            }
            let filter = if clauses.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", clauses.join(" AND "))
            };
            let precision_rows = if version >= 10 {
                "(SELECT day, session FROM usage_day_only UNION ALL
                  SELECT d.day, d.session FROM usage_daily d
                  JOIN usage_sealed_sessions s ON s.session = d.session AND d.day < s.cutoff)"
            } else {
                "usage_day_only"
            };
            let day_only = tx.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {precision_rows}{filter})"),
                params_from_iter(values.iter()),
                |row| row.get::<_, bool>(0),
            )?;
            let mut scoped_day_only = false;
            if day_only {
                if let Some(sessions) = &sessions {
                    let separator = if clauses.is_empty() { " WHERE " } else { " AND " };
                    let sql = format!(
                        "SELECT EXISTS(SELECT 1 FROM {precision_rows}{filter}{separator}session = ?{})",
                        values.len() + 1
                    );
                    let mut statement = tx.prepare(&sql)?;
                    for session in sessions {
                        values.push(SqlValue::Text(session.clone()));
                        scoped_day_only =
                            statement.query_row(params_from_iter(values.iter()), |row| row.get(0))?;
                        values.pop();
                        if scoped_day_only {
                            break;
                        }
                    }
                }
            }
            let mut sealed_review = false;
            let mut scoped_sealed_review = false;
            if version >= 10 {
                sealed_review = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM usage_sealed_sessions
                     WHERE review_reason IS NOT NULL AND (?1 IS NULL OR session = ?1))",
                    [&options.session_id],
                    |row| row.get(0),
                )?;
                if sealed_review {
                    if let Some(sessions) = &sessions {
                        let mut statement = tx.prepare(
                            "SELECT EXISTS(SELECT 1 FROM usage_sealed_sessions
                             WHERE review_reason IS NOT NULL AND session = ?1)",
                        )?;
                        for session in sessions {
                            if options.session_id.as_ref().is_some_and(|id| id != session) {
                                continue;
                            }
                            scoped_sealed_review =
                                statement.query_row([session], |row| row.get(0))?;
                            if scoped_sealed_review {
                                break;
                            }
                        }
                    }
                }
            }
            (day_only, scoped_day_only, sealed_review, scoped_sealed_review)
        }
    } else {
        (false, false, false, false)
    };
    let partial_day = bounds.is_some() && day_bounds.is_none();
    let global_unavailable = day_only && partial_day;
    let scoped_unavailable = scoped_day_only && partial_day;
    let include_trend = request.get("includeTrend")
        .map(|value| value.as_bool().context("用量趋势选项无效"))
        .transpose()?.unwrap_or(false);
    let trend_bounds = if include_trend { bounds } else { None };
    let mut trend = trend_bounds
        .filter(|_| !global_unavailable)
        .map(|(start, end)| Trend::new(start, end, day_only))
        .transpose()?;
    let mut scoped_trend = trend_bounds
        .filter(|_| scope.is_some() && !scoped_unavailable)
        .map(|(start, end)| Trend::new(start, end, scoped_day_only))
        .transpose()?;
    let fine_trend = trend.as_ref().or(scoped_trend.as_ref())
        .is_some_and(|trend| trend.interval < 86_400_000);
    if fine_trend && !day_only {
        daily = false;
    }
    // 有日精度历史或选择了多日桶时，共享同一日粒度查询。
    if daily && let Some((start, end)) = trend_bounds {
        trend = Some(Trend::new(start, end, true)?);
        scoped_trend = scope.as_ref().map(|_| Trend::new(start, end, true)).transpose()?;
    }
    if global_unavailable && (scope.is_none() || scoped_unavailable) {
        let mut response = json!({
            "status":"partial",
            "complete":false,
            "source":"persistent-local-rollout",
            "accountingVersion":2,
            "revision":revision,
            "rangeApplied":true,
            "days":[],
            "countStatus":{"complete":false,"uncertainEvents":null},
            "classificationStatus":{"complete":false,"uncertainEvents":null},
            "coverage":{"days":{},"precision":"day","reason":"retention-day-only","sealedReview":sealed_review},
            "providerNames":provider_names
        });
        if include_trend {
            response["trend"] = Value::Null;
        }
        if let Some(scope) = scope {
            response["scopedUsage"] = json!({
                "sessionId":scope,
                "sessionIds":sessions.unwrap_or_default(),
                "days":[],
                "countStatus":{"complete":false,"uncertainEvents":null},
                "classificationStatus":{"complete":false,"uncertainEvents":null},
                "coverage":{"precision":"day","reason":"retention-day-only","sealedReview":scoped_sealed_review}
            });
            if include_trend {
                response["scopedUsage"]["trend"] = Value::Null;
            }
        }
        tx.commit()?;
        return Ok(response);
    }
    let mut arguments = Vec::<SqlValue>::new();
    let mut filters = Vec::<String>::new();
    for (column, value) in [
        ("day >=", options.from.map(|date| date.to_string())),
        ("day <=", options.to.map(|date| date.to_string())),
        ("session =", options.session_id.clone()),
    ] {
        if let Some(value) = value {
            arguments.push(SqlValue::Text(value));
            filters.push(format!("{column} ?{}", arguments.len()));
        }
    }
    if let Some((start, end)) = bounds {
        if daily {
            let (first, after_last) = day_bounds.expect("daily bounds were checked");
            arguments.push(SqlValue::Text(first.to_string()));
            filters.push(format!("day >= ?{}", arguments.len()));
            arguments.push(SqlValue::Text(after_last.to_string()));
            filters.push(format!("day < ?{}", arguments.len()));
        } else {
            arguments.push(SqlValue::Integer(start));
            filters.push(format!("timestamp_ms >= ?{}", arguments.len()));
            arguments.push(SqlValue::Integer(end));
            filters.push(format!("timestamp_ms < ?{}", arguments.len()));
        }
    }
    if global_unavailable {
        let scoped_sessions = sessions.as_ref().expect("scope was checked");
        if scoped_sessions.is_empty() {
            filters.push("0".into());
        } else {
            let mut placeholders = Vec::with_capacity(scoped_sessions.len());
            for session in scoped_sessions {
                arguments.push(SqlValue::Text(session.clone()));
                placeholders.push(format!("?{}", arguments.len()));
            }
            filters.push(format!("session IN ({})", placeholders.join(", ")));
        }
    }
    let where_clause = if filters.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", filters.join(" AND "))
    };
    let bucket_expression = if !daily && let Some(trend) = trend.as_ref().or(scoped_trend.as_ref()) {
        arguments.push(SqlValue::Integer(trend.start));
        let start_parameter = arguments.len();
        arguments.push(SqlValue::Integer(trend.interval));
        format!("((timestamp_ms - ?{start_parameter}) / ?{})", arguments.len())
    } else {
        "NULL".into()
    };
    let sql = if daily {
        format!(
            "SELECT day, session, model, provider, role, context_conflicts,
                SUM(input), SUM(cached_known_sum), SUM(output), SUM(COALESCE(reasoning, 0)),
                SUM(records),
                SUM(CASE WHEN records > 0 AND cached IS NULL THEN records ELSE 0 END),
                SUM(CASE WHEN records > 0 AND reasoning IS NULL THEN records ELSE 0 END),
                SUM(uncertain_events), SUM(total_events), NULL, NULL
             FROM usage_daily {where_clause}
             GROUP BY day, session, model, provider, role, context_conflicts"
        )
    } else {
        let valid = "count_uncertain = 0 AND count_incomplete = 0 AND (input > 0 OR output > 0)";
        let latest = if policy.is_some() { "MAX(timestamp_ms)" } else { "NULL" };
        format!(
            "SELECT day, session, model, provider, role, context_conflicts,
            SUM(CASE WHEN {valid} THEN input ELSE 0 END),
            SUM(CASE WHEN {valid} THEN COALESCE(cached, 0) ELSE 0 END),
            SUM(CASE WHEN {valid} THEN output ELSE 0 END),
            SUM(CASE WHEN {valid} THEN COALESCE(reasoning, 0) ELSE 0 END),
            SUM(CASE WHEN {valid} THEN 1 ELSE 0 END),
            SUM(CASE WHEN {valid} AND cached IS NULL THEN 1 ELSE 0 END),
            SUM(CASE WHEN {valid} AND reasoning IS NULL THEN 1 ELSE 0 END),
            SUM(CASE WHEN count_uncertain != 0 OR count_incomplete != 0 THEN 1 ELSE 0 END),
            COUNT(*), {bucket_expression} AS trend_bucket, {latest}
         FROM usage_events {where_clause}
         GROUP BY day, session, model, provider, role, context_conflicts, trend_bucket"
        )
    };
    let mut days = BTreeMap::<String, Day>::new();
    let mut scoped_days = BTreeMap::<String, Day>::new();
    let mut uncertain_count = 0u64;
    let mut uncertain_classification = 0u64;
    let mut scoped_uncertain_count = 0u64;
    let mut scoped_uncertain_classification = 0u64;
    {
        let mut statement = tx.prepare(&sql)?;
        let mut rows = statement.query(params_from_iter(arguments.iter()))?;
        while let Some(row) = rows.next()? {
            let day: String = row.get(0)?;
            let session: String = row.get(1)?;
            let scoped = sessions
                .as_ref()
                .is_some_and(|sessions| sessions.binary_search(&session).is_ok());
            if global_unavailable && !scoped {
                continue;
            }
            let raw_model: String = row.get(2)?;
            let raw_provider: String = row.get(3)?;
            let raw_role: String = row.get(4)?;
            let conflicts: u8 = row.get(5)?;
            let missing_model = conflicts & 1 != 0 || unknown_model(&raw_model);
            let missing_provider = conflicts & 2 != 0 || unknown_provider(&raw_provider);
            let latest_at: Option<i64> = row.get(16)?;
            let estimate = if missing_model || missing_provider {
                policy.and_then(|policy| {
                    policy.find(
                        (&day, &session, &raw_model, &raw_provider, &raw_role, conflicts),
                        latest_at,
                    )
                })
            } else {
                None
            };
            let estimated_model = estimate.and_then(|rule| rule.estimated_model.as_deref());
            let estimated_provider = estimate.and_then(|rule| rule.estimated_provider.as_deref());
            let classification = Classification {
                model: if missing_model { estimated_model.unwrap_or("") } else { &raw_model },
                provider: if missing_provider {
                    estimated_provider.unwrap_or("")
                } else {
                    &raw_provider
                },
                role: if conflicts & 4 != 0 { "unknown" } else { &raw_role },
                model_inferred: missing_model && estimated_model.is_some(),
                provider_inferred: missing_provider && estimated_provider.is_some(),
            };
            let filter_model = if classification.model_inferred {
                classification.model
            } else {
                &raw_model
            };
            if options.model.as_ref().is_some_and(|model| model != filter_model)
                || !options.search.is_empty()
                    && !filter_model.to_lowercase().contains(&options.search)
                    && !day.contains(&options.search)
            {
                continue;
            }
            let records: usize = row.get(10)?;
            let count_issues: u64 = row.get(13)?;
            uncertain_count = uncertain_count
                .checked_add(count_issues)
                .context("不确定的用量记录数超出范围")?;
            if scoped {
                scoped_uncertain_count = scoped_uncertain_count
                    .checked_add(count_issues)
                    .context("会话范围内不确定的用量记录数超出范围")?;
            }
            if conflicts != 0 || unknown_model(&raw_model) || unknown_provider(&raw_provider) {
                let classification_issues: u64 = row.get(14)?;
                uncertain_classification = uncertain_classification
                    .checked_add(classification_issues)
                    .context("不确定的分类记录数超出范围")?;
                if scoped {
                    scoped_uncertain_classification = scoped_uncertain_classification
                        .checked_add(classification_issues)
                        .context("会话范围内不确定的分类记录数超出范围")?;
                }
            }
            if records == 0 {
                continue;
            }
            let mut tokens = Tokens::new(row.get(6)?, row.get(7)?, row.get(8)?);
            tokens.cached_known = row.get::<_, u64>(11)? == 0;
            tokens.reasoning_output_tokens = (row.get::<_, u64>(12)? == 0)
                .then(|| row.get(9))
                .transpose()?;
            let split = estimate.and_then(|rule| rule.provider_split.as_ref()).filter(|split| {
                daily && count_issues == 0 && tokens.cached_known
                    && split.whole.records == records
                    && split.whole.input == tokens.input_tokens
                    && split.whole.cached == tokens.cached_input_tokens
                    && split.whole.output == tokens.output_tokens
                    && split.whole.reasoning == tokens.reasoning_output_tokens
            });
            let mut parts = vec![(classification, tokens, records)];
            if let Some(split) = split {
                let matched = &split.matched;
                let mut matched_tokens = Tokens::new(matched.input, matched.cached, matched.output);
                matched_tokens.reasoning_output_tokens = matched.reasoning;
                let mut remainder = Tokens::new(
                    tokens.input_tokens - matched.input,
                    tokens.cached_input_tokens - matched.cached,
                    tokens.output_tokens - matched.output,
                );
                remainder.reasoning_output_tokens = tokens.reasoning_output_tokens
                    .zip(matched.reasoning).map(|(whole, part)| whole - part);
                parts = vec![
                    (classification, remainder, records - matched.records),
                    (Classification { provider: &split.provider, provider_inferred: true,
                        ..classification }, matched_tokens, matched.records),
                ];
            }
            let trend_at = if trend.is_some() || scoped_trend.is_some() {
                Some(if daily {
                    let date = chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d")?;
                    Local.from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
                        .earliest().context("用量日期桶无效")?.timestamp_millis()
                } else {
                    let bucket: i64 = row.get(15)?;
                    let layout = trend.as_ref().or(scoped_trend.as_ref()).unwrap();
                    layout.start + bucket * layout.interval
                })
            } else {
                None
            };
            for (classification, tokens, records) in parts {
                if let Some(at) = trend_at {
                    if let Some(trend) = &mut trend {
                        trend.add(at, classification, tokens, records)?;
                    }
                    if scoped && let Some(trend) = &mut scoped_trend {
                        trend.add(at, classification, tokens, records)?;
                    }
                }
                if !global_unavailable {
                    days.entry(day.clone())
                        .or_default()
                        .add(classification, tokens, records)?;
                }
                if scoped {
                    scoped_days.entry(day.clone()).or_default()
                        .add(classification, tokens, records)?;
                }
            }
        }
    }
    let rows = day_values(days, provider_names);
    let mut response = json!({
        "status":"partial",
        "complete":false,
        "source":"persistent-local-rollout",
        "accountingVersion":2,
        "revision":revision,
        "rangeApplied":bounds.is_some(),
        "days":rows,
        "countStatus":{"complete":false,"uncertainEvents":if global_unavailable {None} else {Some(uncertain_count)}},
        "classificationStatus":{"complete":false,"uncertainEvents":if global_unavailable {None} else {Some(uncertain_classification)}},
        "coverage":{"days":{},"precision":if day_only {"day"} else {"exact"},
            "reason":if global_unavailable {"retention-day-only"} else if sealed_review {"sealed-history-review"} else {"discovery-unconfirmed"},
            "sealedReview":sealed_review},
        "providerNames":provider_names
    });
    if include_trend {
        response["trend"] = trend.map(|trend| trend.value(provider_names)).unwrap_or(Value::Null);
    }
    if let Some(scope) = scope {
        response["scopedUsage"] = json!({
            "sessionId":scope,
            "sessionIds":sessions.unwrap_or_default(),
            "days":day_values(scoped_days, provider_names),
            "countStatus":{"complete":false,"uncertainEvents":scoped_uncertain_count},
            "classificationStatus":{"complete":false,"uncertainEvents":scoped_uncertain_classification},
            "coverage":{"precision":if scoped_day_only {"day"} else {"exact"},
                "reason":if scoped_sealed_review {"sealed-history-review"} else {"discovery-unconfirmed"},
                "sealedReview":scoped_sealed_review}
        });
        if include_trend {
            response["scopedUsage"]["trend"] =
                scoped_trend.map(|trend| trend.value(provider_names)).unwrap_or(Value::Null);
        }
    }
    tx.commit()?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rusqlite::params;

    const PARENT: &str = "00000000-0000-4000-8000-000000000001";
    const CHILD: &str = "00000000-0000-4000-8000-000000000002";
    const OTHER: &str = "00000000-0000-4000-8000-000000000003";

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE usage_meta(singleton INTEGER PRIMARY KEY, revision INTEGER NOT NULL);
             INSERT INTO usage_meta VALUES (1, 7);
             CREATE TABLE usage_sources(source BLOB PRIMARY KEY, scope_session TEXT,
                 scope_indexed INTEGER, scope_valid INTEGER);
             CREATE TABLE usage_scope_parents(source BLOB, session TEXT, parent TEXT);
             CREATE TABLE usage_scope_aliases(source BLOB, alias TEXT, session TEXT);
             INSERT INTO usage_sources VALUES
                 (X'61', '00000000-0000-4000-8000-000000000001', 1, 1),
                 (X'62', '00000000-0000-4000-8000-000000000002', 1, 1),
                 (X'63', '00000000-0000-4000-8000-000000000003', 1, 1);
             INSERT INTO usage_scope_parents VALUES
                 (X'61', '00000000-0000-4000-8000-000000000001', ''),
                 (X'62', '00000000-0000-4000-8000-000000000002',
                          '00000000-0000-4000-8000-000000000001'),
                 (X'63', '00000000-0000-4000-8000-000000000003', '');
             CREATE TABLE usage_events(
                 session TEXT, day TEXT, timestamp_ms INTEGER, model TEXT, provider TEXT, role TEXT,
                 context_conflicts INTEGER, count_uncertain INTEGER, count_incomplete INTEGER,
                 input INTEGER, cached INTEGER, output INTEGER, reasoning INTEGER
             );",
        )
        .unwrap();
        db
    }

    fn insert(
        db: &Connection,
        session: &str,
        timestamp: i64,
        model: &str,
        provider: &str,
        flags: (u8, bool, bool),
        counts: (u64, Option<u64>, u64),
    ) {
        db.execute(
            "INSERT INTO usage_events VALUES (?1, '2026-11-10', ?2, ?3, ?4, 'main',
                ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
            params![
                session, timestamp, model, provider, flags.0, flags.1, flags.2,
                counts.0, counts.1, counts.2
            ],
        )
        .unwrap();
    }

    #[test]
    fn scope_and_time_bounds_preserve_unknown_counts_without_mixing_other_sessions() {
        let mut db = database();
        insert(&db, PARENT, 1_000, "model-a", "provider-a", (0, false, false), (100, Some(10), 20));
        insert(&db, CHILD, 1_500, "", "", (0, false, false), (50, None, 5));
        insert(&db, CHILD, 2_000, "model-b", "provider-b", (0, false, false), (9, Some(1), 1));
        insert(&db, OTHER, 1_500, "model-c", "provider-c", (0, true, false), (700, Some(1), 0));
        let request = json!({"startAt":1000,"endAt":2000,"scopeSessionId":PARENT});
        let options = Query::parse(&request, NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()).unwrap();
        let result = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        assert_eq!(result["revision"], 7);
        assert_eq!(result["status"], "partial");
        assert_eq!(result["countStatus"]["uncertainEvents"], 1);
        assert_eq!(result["classificationStatus"]["uncertainEvents"], 1);
        assert_eq!(result["scopedUsage"]["countStatus"]["uncertainEvents"], 0);
        assert_eq!(result["scopedUsage"]["classificationStatus"]["uncertainEvents"], 1);
        assert_eq!(result["scopedUsage"]["sessionIds"], json!([PARENT, CHILD]));
        assert_eq!(result["scopedUsage"]["days"][0]["usage"]["inputTokens"], 150);
        assert_eq!(result["scopedUsage"]["days"][0]["usage"]["outputTokens"], 25);
        assert_eq!(result["scopedUsage"]["days"][0]["usage"]["cachedKnown"], false);
        assert_eq!(result["scopedUsage"]["days"][0]["recordCount"], 2);
        assert_eq!(result["scopedUsage"]["days"][0]["breakdowns"][0]["model"], "");
        assert_eq!(result["scopedUsage"]["days"][0]["breakdowns"][0]["usage"]["inputTokens"], 50);
    }

    #[test]
    fn retained_other_session_does_not_hide_an_exact_scoped_range() {
        let mut db = database();
        db.execute_batch(
            "CREATE TABLE usage_day_only(day TEXT NOT NULL, session TEXT NOT NULL);
             PRAGMA user_version = 9;",
        )
        .unwrap();
        let start = Local.with_ymd_and_hms(2026, 11, 10, 10, 0, 0).single().unwrap();
        insert(
            &db, PARENT, start.timestamp_millis(), "model-a", "provider-a",
            (0, false, false), (100, Some(10), 20),
        );
        insert(
            &db, CHILD, (start + chrono::Duration::minutes(1)).timestamp_millis(),
            "model-b", "provider-b", (0, false, false), (50, None, 5),
        );
        db.execute("INSERT INTO usage_day_only VALUES ('2026-11-10', ?1)", [OTHER])
            .unwrap();
        let request = json!({
            "startAt": start.timestamp_millis(),
            "endAt": (start + chrono::Duration::hours(1)).timestamp_millis(),
            "scopeSessionId": PARENT
        });
        let options = Query::parse(&request, NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()).unwrap();
        let result = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        assert_eq!(result["coverage"]["precision"], "day");
        assert_eq!(result["days"], json!([]));
        assert_eq!(result["countStatus"]["uncertainEvents"], Value::Null);
        assert_eq!(result["scopedUsage"]["coverage"]["precision"], "exact");
        assert_eq!(result["scopedUsage"]["days"][0]["usage"]["inputTokens"], 150);

        db.execute("INSERT INTO usage_day_only VALUES ('2026-11-10', ?1)", [CHILD])
            .unwrap();
        let result = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        assert_eq!(result["scopedUsage"]["coverage"]["precision"], "day");
        assert_eq!(result["scopedUsage"]["days"], json!([]));
        assert_eq!(result["scopedUsage"]["countStatus"]["uncertainEvents"], Value::Null);
    }

    #[test]
    fn count_conflicts_are_scoped_without_erasing_confirmed_usage() {
        let mut db = database();
        insert(&db, PARENT, 1_000, "model-a", "provider-a", (0, false, false), (100, Some(10), 20));
        insert(&db, PARENT, 1_500, "model-a", "", (2, true, false), (999, Some(5), 10));
        let request = json!({"startAt":1000,"endAt":2000});
        let options = Query::parse(&request, NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()).unwrap();
        let result = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        assert_eq!(result["countStatus"]["uncertainEvents"], 1);
        assert_eq!(result["classificationStatus"]["uncertainEvents"], 1);
        assert_eq!(result["days"][0]["usage"]["inputTokens"], 100);
        assert_eq!(result["days"][0]["recordCount"], 1);
    }

    #[test]
    fn sealed_precision_and_review_apply_before_pruning_and_to_missing_new_days() {
        let mut db = database();
        db.execute_batch(
            "CREATE TABLE usage_day_only(day TEXT, session TEXT);
             CREATE TABLE usage_daily(day TEXT, session TEXT);
             CREATE TABLE usage_sealed_sessions(session TEXT, cutoff TEXT, review_reason TEXT);
             PRAGMA user_version = 10;",
        ).unwrap();
        db.execute(
            "INSERT INTO usage_daily VALUES ('2026-08-01', ?1)", [PARENT],
        ).unwrap();
        db.execute(
            "INSERT INTO usage_sealed_sessions VALUES (?1, '2026-09-09', NULL)", [PARENT],
        ).unwrap();
        let old = Local.with_ymd_and_hms(2026, 8, 1, 10, 0, 0).single().unwrap();
        let old_request = json!({
            "startAt":old.timestamp_millis(),
            "endAt":(old + chrono::Duration::hours(1)).timestamp_millis(),
            "scopeSessionId":PARENT,
            "period":"all"
        });
        let options = Query::parse(&old_request, Local::now().date_naive()).unwrap();
        let result = query(&mut db, &options, &old_request, &BTreeMap::new()).unwrap();
        assert_eq!(result["coverage"]["precision"], "day");
        assert_eq!(result["scopedUsage"]["coverage"]["precision"], "day");
        assert_eq!(result["coverage"]["sealedReview"], false);
        assert_eq!(result["days"], json!([]));

        db.execute(
            "UPDATE usage_sealed_sessions SET review_reason = 'late conflicting source'", [],
        ).unwrap();
        let recent = Local.with_ymd_and_hms(2026, 11, 10, 10, 0, 0).single().unwrap();
        insert(
            &db, OTHER, recent.timestamp_millis(), "model-a", "provider-a",
            (0, false, false), (20, Some(2), 1),
        );
        let mut request = json!({
            "startAt":recent.timestamp_millis(),
            "endAt":(recent + chrono::Duration::hours(1)).timestamp_millis(),
            "scopeSessionId":PARENT
        });
        let options = Query::parse(&request, recent.date_naive()).unwrap();
        let result = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        assert_eq!(result["coverage"]["sealedReview"], true);
        assert_eq!(result["scopedUsage"]["coverage"]["reason"], "sealed-history-review");
        assert_eq!(result["scopedUsage"]["days"], json!([]));
        assert_eq!(result["days"][0]["usage"]["inputTokens"], 20);

        request["scopeSessionId"] = json!(OTHER);
        let result = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        assert_eq!(result["scopedUsage"]["coverage"]["sealedReview"], false);
        assert_eq!(result["scopedUsage"]["days"][0]["usage"]["inputTokens"], 20);
    }

    #[test]
    fn scoped_query_keeps_global_days_and_returns_separate_scoped_days() {
        let mut db = database();
        insert(&db, PARENT, 1_000, "model-a", "provider-a", (0, false, false), (100, Some(10), 20));
        insert(&db, CHILD, 1_500, "model-a", "provider-a", (0, false, false), (50, Some(5), 5));
        insert(&db, OTHER, 1_500, "model-b", "provider-b", (0, false, false), (30, Some(3), 3));
        let request = json!({"startAt":1000,"endAt":2000,"scopeSessionId":PARENT});
        let options = Query::parse(&request, NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()).unwrap();
        let result = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        assert_eq!(result["days"][0]["usage"]["inputTokens"], 180);
        assert_eq!(result["scopedUsage"]["days"][0]["usage"]["inputTokens"], 150);
    }

    #[test]
    fn minute_trend_reuses_filtered_counts_with_zero_buckets_and_separate_scope() {
        let mut db = database();
        let start = Local.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).single().unwrap()
            .timestamp_millis();
        insert(&db, PARENT, start, "model-a", "provider-a", (0, false, false), (100, Some(10), 20));
        insert(&db, CHILD, start + 60_000, "model-a", "provider-a", (0, false, false), (50, None, 5));
        insert(&db, OTHER, start + 60_000, "model-b", "provider-b", (0, false, false), (30, Some(3), 3));
        insert(&db, PARENT, start + 120_000, "model-a", "", (0, true, false), (999, Some(0), 1));
        insert(&db, PARENT, start + 3_600_000, "model-a", "provider-a", (0, false, false), (9, Some(1), 1));
        let request = json!({
            "startAt":start,"endAt":start + 3_600_000,"scopeSessionId":PARENT,"includeTrend":true
        });
        let options = Query::parse(&request, Local::now().date_naive()).unwrap();
        let names = BTreeMap::from([("provider-a".into(), "Supplier A".into())]);
        let result = query(&mut db, &options, &request, &names).unwrap();
        let buckets = result["trend"]["buckets"].as_array().unwrap();
        assert_eq!(buckets.len(), 60);
        assert_eq!(result["trend"]["intervalMs"], 60_000);
        assert_eq!(result["trend"]["precision"], "exact");
        assert_eq!(buckets[0]["usage"]["inputTokens"], 100);
        assert_eq!(buckets[1]["usage"]["inputTokens"], 80);
        assert_eq!(buckets[2]["usage"]["inputTokens"], 0);
        assert_eq!(buckets[59]["endAt"], start + 3_600_000);
        assert_eq!(buckets[0]["breakdowns"][0]["providerName"], "Supplier A");
        assert_eq!(result["scopedUsage"]["trend"]["buckets"][1]["usage"]["inputTokens"], 50);
        assert_eq!(result["scopedUsage"]["trend"]["buckets"][1]["usage"]["cachedKnown"], false);
        assert_eq!(result["countStatus"]["uncertainEvents"], 1);
        assert_eq!(buckets.iter().map(|row| row["usage"]["inputTokens"].as_u64().unwrap()).sum::<u64>(), 180);
        assert_eq!(result["days"][0]["usage"]["inputTokens"], 180);
    }

    #[test]
    fn historical_projection_preserves_counts_scope_trend_and_current_unknowns() {
        let mut db = database();
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().unwrap();
        let start = Local.from_local_datetime(&yesterday.and_hms_opt(0, 0, 0).unwrap())
            .single().unwrap().timestamp_millis();
        let cutoff = Local.from_local_datetime(&today.and_hms_opt(0, 0, 0).unwrap())
            .single().unwrap().timestamp_millis();
        let insert_row = |session: &str, day: NaiveDate, at: i64, model: &str, provider: &str,
                          conflicts: u8, input: u64| {
            db.execute(
                "INSERT INTO usage_events VALUES (?1, ?2, ?3, ?4, ?5, 'main', ?6, 0, 0, ?7, 2, 3, NULL)",
                params![session, day.to_string(), at, model, provider, conflicts, input],
            ).unwrap();
        };
        insert_row(PARENT, yesterday, start + 60_000, "", "custom", 0, 100);
        insert_row(CHILD, yesterday, start + 120_000, "", "custom", 0, 50);
        insert_row(OTHER, yesterday, start + 180_000, "model-a", "provider-a", 0, 30);
        insert_row(PARENT, yesterday, start + 240_000, "model-a", "provider-a", 0, 20);
        insert_row(PARENT, yesterday, start + 300_000, "recorded", "conflicted", 3, 10);
        insert_row(PARENT, today, cutoff + 60_000, "", "custom", 0, 40);
        let policy_file = tempfile::tempdir().unwrap();
        let path = policy_file.path().join(super::super::historical_classification::FILE_NAME);
        let document = json!({
            "version":1,"cutoffMs":cutoff,"cutoffDay":today.to_string(),
            "rules":[
                {"day":yesterday.to_string(),"session":"*","model":"","provider":"custom",
                 "role":"main","conflicts":0,"estimatedModel":"model-b","estimatedProvider":"provider-b"},
                {"day":yesterday.to_string(),"session":PARENT,"model":"","provider":"custom",
                 "role":"main","conflicts":0,"estimatedModel":"model-a","estimatedProvider":"provider-a"},
                {"day":yesterday.to_string(),"session":PARENT,"model":"recorded","provider":"conflicted",
                 "role":"main","conflicts":3,"estimatedModel":"model-c","estimatedProvider":"provider-c"}
            ]
        });
        std::fs::write(&path, document.to_string()).unwrap();
        let policy = super::super::historical_classification::load(&path).unwrap().unwrap();
        let request = json!({
            "startAt":start,"endAt":cutoff + 3_600_000,"scopeSessionId":PARENT,"includeTrend":true
        });
        let options = Query::parse(&request, today).unwrap();
        let raw = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        let projected = query_with_policy(
            &mut db, &options, &request, &BTreeMap::new(), Some(&policy.policy),
        ).unwrap();
        for (raw_day, projected_day) in raw["days"].as_array().unwrap()
            .iter().zip(projected["days"].as_array().unwrap())
        {
            assert_eq!(raw_day["usage"], projected_day["usage"]);
            assert_eq!(raw_day["recordCount"], projected_day["recordCount"]);
        }
        assert_eq!(raw["countStatus"], projected["countStatus"]);
        assert_eq!(raw["classificationStatus"], projected["classificationStatus"]);
        assert_eq!(raw["scopedUsage"]["classificationStatus"],
                   projected["scopedUsage"]["classificationStatus"]);
        let yesterday_rows = projected["days"][0]["breakdowns"].as_array().unwrap();
        let estimated_a = yesterday_rows.iter().find(|row|
            row["model"] == "model-a" && row["provider"] == "provider-a" &&
            row["modelInferred"] == true
        ).unwrap();
        assert_eq!(estimated_a["usage"]["inputTokens"], 100);
        assert_eq!(estimated_a["providerInferred"], true);
        let known_a = yesterday_rows.iter().find(|row|
            row["model"] == "model-a" && row["provider"] == "provider-a" &&
            row["modelInferred"].is_null()
        ).unwrap();
        assert_eq!(known_a["usage"]["inputTokens"], 50);
        assert!(yesterday_rows.iter().any(|row| row["model"] == "model-b"
            && row["provider"] == "provider-b" && row["usage"]["inputTokens"] == 50));
        assert!(yesterday_rows.iter().any(|row| row["model"] == "model-c"
            && row["provider"] == "provider-c" && row["usage"]["inputTokens"] == 10));
        let current = &projected["days"][1]["breakdowns"][0];
        assert_eq!(current["model"], "");
        assert_eq!(current["provider"], "");
        assert!(current["modelInferred"].is_null());

        let scoped = projected["scopedUsage"]["days"][0]["breakdowns"].as_array().unwrap();
        assert!(scoped.iter().any(|row| row["model"] == "model-a"
            && row["modelInferred"] == true && row["usage"]["inputTokens"] == 100));
        assert!(scoped.iter().any(|row| row["model"] == "model-b"
            && row["modelInferred"] == true && row["usage"]["inputTokens"] == 50));
        let buckets = projected["trend"]["buckets"].as_array().unwrap();
        assert_eq!(buckets.iter().map(|bucket| bucket["usage"]["inputTokens"].as_u64().unwrap())
            .sum::<u64>(), 250);
        assert!(buckets.iter().flat_map(|bucket| bucket["breakdowns"].as_array().unwrap())
            .any(|row| row["model"] == "model-b" && row["modelInferred"] == true));
        assert!(projected["scopedUsage"]["trend"]["buckets"].as_array().unwrap().iter()
            .flat_map(|bucket| bucket["breakdowns"].as_array().unwrap())
            .any(|row| row["model"] == "model-a" && row["modelInferred"] == true));
        let stored: (String, String, u64) = db.query_row(
            "SELECT model, provider, input FROM usage_events
             WHERE session = ?1 AND day = ?2 AND input = 100",
            params![PARENT, yesterday.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(stored, ("".into(), "custom".into(), 100));
    }

    #[test]
    fn retained_history_trend_uses_daily_totals_and_keeps_at_most_120_buckets() {
        let mut db = database();
        db.execute_batch(
            "CREATE TABLE usage_day_only(day TEXT, session TEXT);
             CREATE TABLE usage_daily(
                 day TEXT, session TEXT, model TEXT, provider TEXT, role TEXT,
                 context_conflicts INTEGER, input INTEGER, cached INTEGER,
                 cached_known_sum INTEGER, output INTEGER, reasoning INTEGER,
                 records INTEGER, uncertain_events INTEGER, total_events INTEGER
             );
             PRAGMA user_version = 9;",
        ).unwrap();
        db.execute(
            "INSERT INTO usage_daily VALUES
             ('2026-11-10', ?1, 'model-a', 'provider-a', 'main', 0, 123, 12, 12, 5, NULL, 2, 0, 2)",
            [PARENT],
        ).unwrap();
        db.execute("INSERT INTO usage_day_only VALUES ('2026-11-10', ?1)", [PARENT]).unwrap();
        let start = Local.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).single().unwrap();
        let end = Local.with_ymd_and_hms(2026, 11, 11, 0, 0, 0).single().unwrap();
        let request = json!({
            "startAt":start.timestamp_millis(),"endAt":end.timestamp_millis(),
            "scopeSessionId":PARENT,"includeTrend":true
        });
        let options = Query::parse(&request, Local::now().date_naive()).unwrap();
        let result = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        let buckets = result["trend"]["buckets"].as_array().unwrap();
        assert!(buckets.len() <= 120);
        assert_eq!(result["trend"]["intervalMs"], 172_800_000);
        assert_eq!(result["trend"]["precision"], "day");
        assert_eq!(result["scopedUsage"]["trend"]["precision"], "day");
        assert_eq!(buckets.iter().map(|row| row["recordCount"].as_u64().unwrap()).sum::<u64>(), 2);
        assert_eq!(buckets.iter().map(|row| row["usage"]["inputTokens"].as_u64().unwrap()).sum::<u64>(), 123);
        assert_eq!(result["days"][0]["usage"]["inputTokens"], 123);
    }

    #[test]
    fn historical_projection_applies_to_compacted_daily_totals() {
        let mut db = database();
        db.execute_batch(
            "CREATE TABLE usage_day_only(day TEXT, session TEXT);
             CREATE TABLE usage_daily(
                 day TEXT, session TEXT, model TEXT, provider TEXT, role TEXT,
                 context_conflicts INTEGER, input INTEGER, cached INTEGER,
                 cached_known_sum INTEGER, output INTEGER, reasoning INTEGER,
                 records INTEGER, uncertain_events INTEGER, total_events INTEGER
             );
             PRAGMA user_version = 9;",
        ).unwrap();
        let today = Local::now().date_naive();
        let old = today.checked_sub_days(Days::new(80)).unwrap();
        let cutoff = Local.from_local_datetime(&today.and_hms_opt(0, 0, 0).unwrap())
            .single().unwrap().timestamp_millis();
        db.execute(
            "INSERT INTO usage_daily VALUES (?1, ?2, '', 'custom', 'main', 0, 123, 12, 12, 5, NULL, 2, 0, 2)",
            params![old.to_string(), PARENT],
        ).unwrap();
        db.execute("INSERT INTO usage_day_only VALUES (?1, ?2)", params![old.to_string(), PARENT])
            .unwrap();
        let policy_file = tempfile::tempdir().unwrap();
        let path = policy_file.path().join(super::super::historical_classification::FILE_NAME);
        std::fs::write(&path, json!({
            "version":1,"cutoffMs":cutoff,"cutoffDay":today.to_string(),
            "rules":[{"day":old.to_string(),"session":PARENT,"model":"","provider":"custom",
                "role":"main","conflicts":0,"estimatedModel":"model-a","estimatedProvider":"provider-a"}]
        }).to_string()).unwrap();
        let policy = super::super::historical_classification::load(&path).unwrap().unwrap();
        let start = Local.from_local_datetime(&old.and_hms_opt(0, 0, 0).unwrap()).single().unwrap();
        let request = json!({
            "startAt":start.timestamp_millis(),"endAt":cutoff,
            "scopeSessionId":PARENT,"includeTrend":true
        });
        let options = Query::parse(&request, today).unwrap();
        let raw = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        let projected = query_with_policy(
            &mut db, &options, &request, &BTreeMap::new(), Some(&policy.policy),
        ).unwrap();
        assert_eq!(raw["days"][0]["usage"], projected["days"][0]["usage"]);
        assert_eq!(raw["days"][0]["recordCount"], projected["days"][0]["recordCount"]);
        assert_eq!(raw["classificationStatus"], projected["classificationStatus"]);
        let row = &projected["days"][0]["breakdowns"][0];
        assert_eq!(row["model"], "model-a");
        assert_eq!(row["provider"], "provider-a");
        assert_eq!(row["modelInferred"], true);
        assert_eq!(row["providerInferred"], true);
        assert_eq!(row["recordCount"], 2);
        assert_eq!(projected["scopedUsage"]["days"][0]["breakdowns"][0], *row);
        assert!(projected["trend"]["buckets"].as_array().unwrap().iter()
            .flat_map(|bucket| bucket["breakdowns"].as_array().unwrap())
            .any(|row| row["model"] == "model-a" && row["modelInferred"] == true));
    }

    #[test]
    fn historical_provider_split_preserves_daily_scope_and_trend() {
        let mut db = database();
        db.execute_batch(
            "CREATE TABLE usage_day_only(day TEXT, session TEXT);
             CREATE TABLE usage_daily(
                 day TEXT, session TEXT, model TEXT, provider TEXT, role TEXT,
                 context_conflicts INTEGER, input INTEGER, cached INTEGER,
                 cached_known_sum INTEGER, output INTEGER, reasoning INTEGER,
                 records INTEGER, uncertain_events INTEGER, total_events INTEGER
             );
             PRAGMA user_version = 9;",
        ).unwrap();
        let today = Local::now().date_naive();
        let old = today.checked_sub_days(Days::new(80)).unwrap();
        let start = Local.from_local_datetime(&old.and_hms_opt(0, 0, 0).unwrap())
            .single().unwrap().timestamp_millis();
        let cutoff = Local.from_local_datetime(&today.and_hms_opt(0, 0, 0).unwrap())
            .single().unwrap().timestamp_millis();
        db.execute(
            "INSERT INTO usage_daily VALUES (?1, ?2, 'model-a', 'custom', 'main', 0,
                 123, 12, 12, 5, 3, 2, 0, 2)",
            params![old.to_string(), PARENT],
        ).unwrap();
        db.execute("INSERT INTO usage_day_only VALUES (?1, ?2)", params![old.to_string(), PARENT])
            .unwrap();
        let policy_file = tempfile::tempdir().unwrap();
        let path = policy_file.path().join(super::super::historical_classification::FILE_NAME);
        std::fs::write(&path, json!({
            "version":1,"cutoffMs":cutoff,"cutoffDay":today.to_string(),
            "rules":[{"day":old.to_string(),"session":PARENT,"model":"model-a",
                "provider":"custom","role":"main","conflicts":0,"estimatedProvider":"provider-a",
                "providerSplit":{"provider":"provider-b",
                    "whole":{"records":2,"input":123,"cached":12,"output":5,"reasoning":3},
                    "matched":{"records":1,"input":80,"cached":8,"output":2,"reasoning":1}}}]
        }).to_string()).unwrap();
        let policy = super::super::historical_classification::load(&path).unwrap().unwrap();
        let request = json!({"startAt":start,"endAt":cutoff,
            "scopeSessionId":PARENT,"includeTrend":true});
        let options = Query::parse(&request, today).unwrap();
        let raw = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        let projected = query_with_policy(
            &mut db, &options, &request, &BTreeMap::new(), Some(&policy.policy),
        ).unwrap();
        assert_eq!(raw["days"][0]["usage"], projected["days"][0]["usage"]);
        assert_eq!(raw["days"][0]["recordCount"], projected["days"][0]["recordCount"]);
        assert_eq!(raw["scopedUsage"]["days"][0]["usage"],
            projected["scopedUsage"]["days"][0]["usage"]);
        let rows = projected["days"][0]["breakdowns"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        for (provider, input, cached, output) in [
            ("provider-a", 43, 4, 3),
            ("provider-b", 80, 8, 2),
        ] {
            let row = rows.iter().find(|row| row["provider"] == provider).unwrap();
            assert_eq!(row["recordCount"], 1);
            assert_eq!(row["usage"]["inputTokens"], input);
            assert_eq!(row["usage"]["cachedInputTokens"], cached);
            assert_eq!(row["usage"]["outputTokens"], output);
            assert_eq!(row["providerInferred"], true);
        }
        let buckets = projected["trend"]["buckets"].as_array().unwrap();
        assert_eq!(buckets.iter().map(|row| row["usage"]["inputTokens"].as_u64().unwrap())
            .sum::<u64>(), 123);
        assert!(buckets.iter().flat_map(|row| row["breakdowns"].as_array().unwrap())
            .any(|row| row["provider"] == "provider-b" && row["usage"]["inputTokens"] == 80));

        db.execute("UPDATE usage_daily SET input = 124 WHERE day = ?1", [old.to_string()]).unwrap();
        let changed = query_with_policy(
            &mut db, &options, &request, &BTreeMap::new(), Some(&policy.policy),
        ).unwrap();
        let rows = changed["days"][0]["breakdowns"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "a changed group must not reuse an old split");
        assert_eq!(rows[0]["provider"], "provider-a");
        assert_eq!(rows[0]["usage"]["inputTokens"], 124);
    }

    #[test]
    fn partial_day_retention_does_not_fabricate_trend_from_daily_counts() {
        let mut db = database();
        db.execute_batch(
            "CREATE TABLE usage_day_only(day TEXT, session TEXT);
             PRAGMA user_version = 9;",
        ).unwrap();
        db.execute("INSERT INTO usage_day_only VALUES ('2026-11-10', ?1)", [PARENT]).unwrap();
        let start = Local.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).single().unwrap();
        let request = json!({
            "startAt":start.timestamp_millis(),"endAt":(start + chrono::Duration::hours(1)).timestamp_millis(),
            "scopeSessionId":PARENT,"includeTrend":true
        });
        let options = Query::parse(&request, Local::now().date_naive()).unwrap();
        let result = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        assert_eq!(result["trend"], Value::Null);
        assert_eq!(result["scopedUsage"]["trend"], Value::Null);
        assert_eq!(result["coverage"]["precision"], "day");
    }

    #[test]
    fn null_time_bounds_match_the_existing_history_contract() {
        let mut db = database();
        insert(&db, PARENT, 1_000, "model-a", "provider-a", (0, false, false), (100, Some(10), 20));
        let request = json!({"startAt":null,"endAt":null});
        let options = Query::parse(&request, NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()).unwrap();
        let result = query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        assert_eq!(result["rangeApplied"], false);
        assert_eq!(result["days"][0]["usage"]["inputTokens"], 100);
    }
}
