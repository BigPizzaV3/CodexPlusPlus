//! 仅从相互印证的原始事件生成候选；不把自动续执行当作新指令。
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::BufRead,
};
#[cfg(test)]
use std::{fs::File, io::BufReader, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TurnBoundary {
    pub start_ordinal: i64,
    pub started_at: i64,
    pub end_ordinal: i64,
    pub completed_at: i64,
    pub status: String,
    pub error_json: Option<String>,
    pub duration_ms: Option<i64>,
    pub first_user_projection: Option<(String, i64)>,
    pub first_user_response_ordinal: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Candidate {
    pub thread: String,
    pub turn: String,
    pub text: String,
    pub ordinal: i64,
    pub created: i64,
    pub goal: bool,
    #[serde(default)]
    pub goal_key: Option<String>,
    #[serde(default)]
    pub projection: Option<(String, i64)>,
    #[serde(default)]
    pub following_turn: Option<(String, i64)>,
    #[serde(default)]
    pub turn_boundary: Option<TurnBoundary>,
    #[serde(default)]
    pub start_ordinal: Option<i64>,
    #[serde(default)]
    pub first_user_projection: Option<(String, i64)>,
    #[serde(default)]
    pub first_user_response_ordinal: Option<i64>,
    #[serde(default)]
    pub first_user_evidence_valid: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ScanResult {
    pub candidates: Vec<Candidate>,
    pub issues: Vec<String>,
}

fn event_time(row: &Value, field: &str) -> Option<i64> {
    let payload = &row["payload"][field];
    if !payload.is_null() {
        return payload.as_i64().filter(|time| *time >= 0);
    }
    chrono::DateTime::parse_from_rfc3339(row["timestamp"].as_str()?)
        .ok()
        .map(|time| time.timestamp())
        .filter(|time| *time >= 0)
}

fn text_content(value: &Value) -> Option<String> {
    let parts = value.as_array()?;
    if parts.is_empty() {
        return None;
    }
    let mut text = String::new();
    for part in parts {
        if !matches!(part["type"].as_str(), Some("text" | "input_text" | "Text")) {
            return None;
        }
        text.push_str(part["text"].as_str()?);
    }
    (!text.is_empty()).then_some(text)
}

fn ordinary(text: &str) -> bool {
    let text = text.trim_start();
    !text.is_empty()
        && ![
            "<codex_internal_context",
            "<environment_context",
            "<permissions instructions>",
            "# AGENTS.md instructions",
            "<INSTRUCTIONS>",
        ]
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

#[cfg(test)]
pub(crate) fn scan(path: &Path) -> anyhow::Result<ScanResult> {
    scan_reader(BufReader::new(File::open(path)?))
}

pub(crate) fn scan_reader(mut reader: impl BufRead) -> anyhow::Result<ScanResult> {
    let mut line = String::new();
    let mut thread = None::<String>;
    let mut inherited_from = HashSet::<String>::new();
    let mut own_history_start = None::<i64>;
    let mut current_turn = None::<String>;
    let mut last_started_turn = None::<String>;
    let mut following_turns = HashMap::<String, Option<(String, i64)>>::new();
    let mut active_boundary = None::<(String, i64, i64)>;
    let mut started_turns = HashSet::<String>::new();
    let mut duplicate_starts = HashSet::<String>::new();
    let mut start_ordinals = HashMap::<String, i64>::new();
    let mut ended_turns = HashSet::<String>::new();
    let mut invalid_boundaries = HashSet::<String>::new();
    let mut invalid_first_user_evidence = HashSet::<String>::new();
    let mut boundaries = HashMap::<String, TurnBoundary>::new();
    let mut first_user_projections = HashMap::<String, (String, i64)>::new();
    let mut first_user_responses = HashMap::<String, i64>::new();
    let mut responses = HashMap::<String, Vec<(Candidate, usize)>>::new();
    let mut evidence = HashMap::<(String, String), Vec<usize>>::new();
    let mut terminal_positions = HashMap::<String, Vec<usize>>::new();
    let mut projections = HashMap::<(String, String), Vec<(String, i64)>>::new();
    let mut goal = None::<(String, i64, String, Option<String>)>;
    let mut seen_goals = HashSet::<(String, String)>::new();
    let mut result = Vec::new();
    let mut issues = Vec::new();
    let mut stream_position = 0usize;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        stream_position = stream_position.saturating_add(1);
        let row: Value = match serde_json::from_str(&line) {
            Ok(row) => row,
            Err(_) if !line.ends_with('\n') => bail!("会话记录尾行不完整"),
            Err(error) => return Err(error).context("会话记录包含无效 JSON 行"),
        };
        let p = &row["payload"];
        if row["type"] == "session_meta" {
            let id = p["id"].as_str().context("会话标识缺失")?;
            if thread.as_deref().is_some_and(|old| old != id) {
                // 子任务日志包含父任务快照；只有元数据声明的父任务及边界内记录可忽略。
                if inherited_from.contains(id)
                    && own_history_start.is_some_and(|start| {
                        row["ordinal"]
                            .as_i64()
                            .is_some_and(|ordinal| ordinal >= 0 && ordinal < start)
                    })
                {
                    continue;
                }
                bail!("文件混入其他会话");
            }
            if thread.is_none() {
                own_history_start = p["subagent_history_start_ordinal"]
                    .as_i64()
                    .filter(|v| *v >= 0);
                for key in ["parent_thread_id", "forked_from_id"] {
                    if let Some(parent) = p[key].as_str() {
                        inherited_from.insert(parent.to_owned());
                    }
                }
            }
            thread = Some(id.to_owned());
            continue;
        }
        let thread = thread.as_ref().context("首条记录缺少 session_meta")?;
        if let Some(start) = own_history_start {
            let ordinal = row["ordinal"]
                .as_i64()
                .context("分叉记录缺少序号，无法确认历史边界")?;
            if ordinal < start {
                continue;
            }
        }
        if p["thread_id"].as_str().is_some_and(|id| id != thread)
            || p["threadId"].as_str().is_some_and(|id| id != thread)
        {
            continue;
        }
        if row["type"] == "event_msg" {
            if p["type"] == "item_completed"
                && matches!(
                    p["item"]["type"].as_str(),
                    Some("UserMessage" | "userMessage")
                )
            {
                if let Some(turn) = p["turn_id"].as_str().or(current_turn.as_deref()) {
                    if let (Some(id), Some(ordinal)) = (
                        p["item"]["id"].as_str().filter(|id| !id.is_empty()),
                        row["ordinal"].as_i64().filter(|ordinal| *ordinal >= 0),
                    ) {
                        if ended_turns.contains(turn) {
                            invalid_boundaries.insert(turn.to_owned());
                            invalid_first_user_evidence.insert(turn.to_owned());
                        } else if p["thread_id"].as_str() == Some(thread)
                            && p["turn_id"].as_str() == Some(turn)
                        {
                            let first = first_user_projections
                                .entry(turn.to_owned())
                                .or_insert((id.to_owned(), ordinal));
                            if ordinal < first.1 {
                                *first = (id.to_owned(), ordinal);
                            } else if ordinal == first.1 && id != first.0 {
                                invalid_boundaries.insert(turn.to_owned());
                                invalid_first_user_evidence.insert(turn.to_owned());
                            }
                        } else {
                            invalid_boundaries.insert(turn.to_owned());
                            invalid_first_user_evidence.insert(turn.to_owned());
                        }
                    } else {
                        invalid_boundaries.insert(turn.to_owned());
                        invalid_first_user_evidence.insert(turn.to_owned());
                    }
                }
            }
            match p["type"].as_str() {
                Some("task_started") => {
                    if let Some((interrupted, _, _)) = active_boundary.take() {
                        invalid_boundaries.insert(interrupted);
                    }
                    current_turn = p["turn_id"].as_str().map(str::to_owned);
                    let started = current_turn.as_ref().filter(|id| !id.is_empty());
                    if let Some(turn) = started {
                        if !started_turns.insert(turn.clone()) {
                            duplicate_starts.insert(turn.clone());
                            invalid_boundaries.insert(turn.clone());
                        }
                        if ended_turns.contains(turn) {
                            invalid_boundaries.insert(turn.clone());
                        }
                        if let Some(ordinal) =
                            row["ordinal"].as_i64().filter(|ordinal| *ordinal >= 0)
                        {
                            start_ordinals.entry(turn.clone()).or_insert(ordinal);
                        }
                        if let (Some(ordinal), Some(time)) = (
                            row["ordinal"].as_i64().filter(|ordinal| *ordinal >= 0),
                            event_time(&row, "started_at"),
                        ) {
                            active_boundary = Some((turn.clone(), ordinal, time));
                        }
                    }
                    if let Some(previous) = last_started_turn.take() {
                        if started.is_none_or(|id| id != &previous) {
                            // 只接受紧接的真实开始边界；缺失信息不得跳到更晚的轮次补猜。
                            let following = started.and_then(|id| {
                                row["ordinal"]
                                    .as_i64()
                                    .filter(|ordinal| *ordinal >= 0)
                                    .map(|ordinal| (id.clone(), ordinal))
                            });
                            following_turns.entry(previous).or_insert(following);
                        }
                    }
                    last_started_turn = started.cloned();
                    if let Some((_, _, _, turn)) = goal.as_mut() {
                        if turn.is_none() {
                            *turn = current_turn.clone();
                        }
                    }
                }
                Some(kind @ ("task_complete" | "task_failed" | "turn_aborted")) => {
                    let terminal_turn = p["turn_id"].as_str().or(current_turn.as_deref());
                    if let Some(turn) = terminal_turn {
                        terminal_positions
                            .entry(turn.to_owned())
                            .or_default()
                            .push(stream_position);
                        if !ended_turns.insert(turn.to_owned()) {
                            invalid_boundaries.insert(turn.to_owned());
                        }
                        if let Some((active, start_ordinal, started_at)) = active_boundary.take() {
                            if active != turn
                                || (!p["started_at"].is_null()
                                    && p["started_at"].as_i64() != Some(started_at))
                            {
                                invalid_boundaries.insert(active);
                                invalid_boundaries.insert(turn.to_owned());
                            } else if let (Some(end_ordinal), Some(completed_at)) = (
                                row["ordinal"]
                                    .as_i64()
                                    .filter(|ordinal| *ordinal > start_ordinal),
                                event_time(&row, "completed_at").filter(|time| *time >= started_at),
                            ) {
                                let error_json =
                                    (!p["error"].is_null()).then(|| p["error"].to_string());
                                let status = match kind {
                                    "turn_aborted" => "interrupted",
                                    "task_failed" => "failed",
                                    _ if error_json.is_some() => "failed",
                                    _ => "completed",
                                };
                                boundaries.insert(
                                    turn.to_owned(),
                                    TurnBoundary {
                                        start_ordinal,
                                        started_at,
                                        end_ordinal,
                                        completed_at,
                                        status: status.to_owned(),
                                        error_json,
                                        duration_ms: p["duration_ms"]
                                            .as_i64()
                                            .filter(|duration| *duration >= 0),
                                        first_user_projection: None,
                                        first_user_response_ordinal: None,
                                    },
                                );
                            }
                        }
                    } else if let Some((active, _, _)) = active_boundary.take() {
                        invalid_boundaries.insert(active);
                    }
                    current_turn = None;
                }
                Some("thread_goal_updated") => {
                    if p["threadId"].as_str() != Some(thread) {
                        continue;
                    }
                    if let (Some(text), Some(created)) = (
                        p["goal"]["objective"].as_str(),
                        p["goal"]["createdAt"].as_i64(),
                    ) {
                        let goal_key = p["goal"]["id"]
                            .as_str()
                            .filter(|id| !id.is_empty())
                            .map(|id| format!("id:{id}:created:{created}"))
                            .unwrap_or_else(|| format!("created:{created}"));
                        if !text.is_empty()
                            && !seen_goals.contains(&(goal_key.clone(), text.to_owned()))
                        {
                            goal = created
                                .checked_mul(1000)
                                .map(|created| (text.to_owned(), created, goal_key, None));
                        }
                    }
                }
                Some("user_message") => {
                    let turn = p["turn_id"].as_str().or(current_turn.as_deref());
                    if let (Some(turn), Some(text)) = (turn, p["message"].as_str()) {
                        if ordinary(text)
                            && (current_turn.as_deref() == Some(turn)
                                || (current_turn.is_none() && !ended_turns.contains(turn)))
                        {
                            evidence
                                .entry((turn.to_owned(), text.to_owned()))
                                .or_default()
                                .push(stream_position);
                        }
                    }
                }
                Some("item_completed")
                    if p["thread_id"].as_str() == Some(thread)
                        && matches!(
                            p["item"]["type"].as_str(),
                            Some("UserMessage" | "userMessage")
                        ) =>
                {
                    if let (Some(turn), Some(text)) =
                        (p["turn_id"].as_str(), text_content(&p["item"]["content"]))
                    {
                        if ordinary(&text) {
                            if ended_turns.contains(turn) {
                                invalid_boundaries.insert(turn.to_owned());
                                invalid_first_user_evidence.insert(turn.to_owned());
                            } else if let (Some(id), Some(ordinal)) =
                                (p["item"]["id"].as_str(), row["ordinal"].as_i64())
                            {
                                if !id.is_empty() && ordinal >= 0 {
                                    projections
                                        .entry((turn.to_owned(), text.clone()))
                                        .or_default()
                                        .push((id.to_owned(), ordinal));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if row["type"] != "response_item" || p["role"] != "user" {
            continue;
        }
        let meta = &p["internal_chat_message_metadata_passthrough"];
        // 附件消息也占据首条用户输入位置，不能因无法恢复为纯文本而被后续 steering 越过。
        let parts = p["content"].as_array();
        let visible_text = parts
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<String>()
            })
            .unwrap_or_default();
        let has_attachment = parts.is_some_and(|parts| {
            parts
                .iter()
                .any(|part| !matches!(part["type"].as_str(), Some("text" | "input_text" | "Text")))
        });
        if ordinary(&visible_text) || has_attachment {
            if let Some(turn) = meta["turn_id"].as_str().or(current_turn.as_deref()) {
                if ended_turns.contains(turn) {
                    invalid_boundaries.insert(turn.to_owned());
                    invalid_first_user_evidence.insert(turn.to_owned());
                }
                if let Some(ordinal) = row["ordinal"].as_i64().filter(|ordinal| *ordinal >= 0) {
                    if meta["turn_id"].as_str() == Some(turn) {
                        let first = first_user_responses
                            .entry(turn.to_owned())
                            .or_insert(ordinal);
                        *first = (*first).min(ordinal);
                    } else {
                        invalid_boundaries.insert(turn.to_owned());
                        invalid_first_user_evidence.insert(turn.to_owned());
                    }
                } else {
                    invalid_boundaries.insert(turn.to_owned());
                    invalid_first_user_evidence.insert(turn.to_owned());
                }
            }
        }
        let (Some(turn), Some(ordinal), Some(text)) = (
            meta["turn_id"].as_str(),
            row["ordinal"].as_i64(),
            text_content(&p["content"]),
        ) else {
            continue;
        };
        if ordinal < 0 {
            continue;
        }
        if let Some((objective, created, goal_key, Some(goal_turn))) = &goal {
            if turn == goal_turn
                && text.starts_with("<codex_internal_context")
                && (text.contains(&format!("<objective>\n{objective}\n</objective>"))
                    || text.contains(&format!("<objective>\r\n{objective}\r\n</objective>")))
            {
                result.push(Candidate {
                    thread: thread.clone(),
                    turn: turn.to_owned(),
                    text: objective.clone(),
                    ordinal,
                    created: *created,
                    goal: true,
                    goal_key: Some(goal_key.clone()),
                    projection: None,
                    following_turn: None,
                    turn_boundary: None,
                    start_ordinal: None,
                    first_user_projection: None,
                    first_user_response_ordinal: None,
                    first_user_evidence_valid: false,
                });
                seen_goals.insert((goal_key.clone(), objective.clone()));
                goal = None;
                continue;
            }
        }
        if !ordinary(&text) {
            continue;
        }
        let created = meta["create_time"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|v| (v * 1000.0) as i64)
            .or_else(|| {
                chrono::DateTime::parse_from_rfc3339(row["timestamp"].as_str()?)
                    .ok()
                    .map(|t| t.timestamp_millis())
            });
        if let Some(created) = created {
            responses.entry(turn.to_owned()).or_default().push((
                Candidate {
                    thread: thread.clone(),
                    turn: turn.to_owned(),
                    text,
                    ordinal,
                    created,
                    goal: false,
                    goal_key: None,
                    projection: None,
                    following_turn: None,
                    turn_boundary: None,
                    start_ordinal: None,
                    first_user_projection: None,
                    first_user_response_ordinal: None,
                    first_user_evidence_valid: false,
                },
                stream_position,
            ));
        }
    }
    for candidates in responses.into_values() {
        let mut response_counts = HashMap::<String, usize>::new();
        for (candidate, _) in &candidates {
            *response_counts.entry(candidate.text.clone()).or_default() += 1;
        }
        let mut ambiguous_events = HashSet::new();
        if let Some((first, _)) = candidates.first() {
            for (text, expected) in &response_counts {
                if evidence
                    .get(&(first.turn.clone(), text.clone()))
                    .is_some_and(|items| items.len() != *expected)
                {
                    ambiguous_events.insert(text.clone());
                }
            }
        }
        for (response_index, (source, source_position)) in candidates.iter().enumerate() {
            let mut candidate = source.clone();
            let key = (candidate.turn.clone(), candidate.text.clone());
            let next_response_position = candidates[response_index + 1..]
                .iter()
                .find(|(c, _)| c.text == candidate.text)
                .map(|(_, position)| *position);
            let previous_response_position = candidates[..response_index]
                .iter()
                .rev()
                .find(|(c, _)| c.text == candidate.text)
                .map(|(_, position)| *position)
                .unwrap_or(0);
            let matched_event = evidence
                .get_mut(&key)
                .filter(|items| !items.is_empty() && !ambiguous_events.contains(&candidate.text))
                .and_then(|items| {
                    items
                        .iter()
                        .position(|event_position| {
                            *event_position > previous_response_position
                                && next_response_position.is_none_or(|next| *event_position < next)
                                && !terminal_positions.get(&candidate.turn).is_some_and(
                                    |terminals| {
                                        terminals.iter().any(|terminal| {
                                            *terminal > (*event_position).min(*source_position)
                                                && *terminal
                                                    <= (*event_position).max(*source_position)
                                        })
                                    },
                                )
                        })
                        .map(|index| items.remove(index))
                })
                .is_some();
            let next = candidates[response_index + 1..]
                .iter()
                .find(|(c, _)| c.text == candidate.text)
                .map(|(c, _)| c.ordinal);
            if let Some(items) = projections.get_mut(&key) {
                // 每条 response 只匹配其后、下一次同文 response 之前的完成事件。
                let matches = items
                    .iter()
                    .filter(|(_, ordinal)| {
                        *ordinal > candidate.ordinal && next.is_none_or(|next| *ordinal < next)
                    })
                    .count();
                if matches > 1 {
                    issues.push(format!(
                        "{} / {} / {}：同一用户响应对应多个完成事件，无法可靠确认原生消息身份",
                        candidate.thread, candidate.turn, candidate.ordinal
                    ));
                    continue;
                }
                if let Some(index) = items.iter().position(|(_, ordinal)| {
                    *ordinal > candidate.ordinal && next.is_none_or(|next| *ordinal < next)
                }) {
                    candidate.projection = Some(items.remove(index));
                }
            }
            if candidate.projection.is_none() && ambiguous_events.contains(&candidate.text) {
                issues.push(format!(
                    "{} / {} / {}：重复文本用户事件数量与响应不一致，无法可靠匹配",
                    candidate.thread, candidate.turn, candidate.ordinal
                ));
                continue;
            }
            if candidate.projection.is_some() || matched_event {
                result.push(candidate);
            }
        }
    }
    for (turn, boundary) in &mut boundaries {
        boundary.first_user_projection = first_user_projections.get(turn).cloned();
        boundary.first_user_response_ordinal = first_user_responses.get(turn).copied();
        if boundary
            .first_user_projection
            .as_ref()
            .is_some_and(|(_, ordinal)| {
                *ordinal <= boundary.start_ordinal || *ordinal >= boundary.end_ordinal
            })
            || boundary.first_user_response_ordinal.is_some_and(|ordinal| {
                ordinal <= boundary.start_ordinal || ordinal >= boundary.end_ordinal
            })
        {
            invalid_boundaries.insert(turn.clone());
        }
    }
    for candidate in &mut result {
        candidate.start_ordinal = start_ordinals
            .get(&candidate.turn)
            .filter(|ordinal| {
                !duplicate_starts.contains(&candidate.turn) && **ordinal < candidate.ordinal
            })
            .copied();
        candidate.first_user_projection = first_user_projections.get(&candidate.turn).cloned();
        candidate.first_user_response_ordinal = first_user_responses.get(&candidate.turn).copied();
        candidate.first_user_evidence_valid = if candidate.goal {
            candidate.start_ordinal.is_some()
                && !duplicate_starts.contains(&candidate.turn)
                && !invalid_first_user_evidence.contains(&candidate.turn)
        } else {
            candidate
                .first_user_response_ordinal
                .is_some_and(|response| {
                    !duplicate_starts.contains(&candidate.turn)
                        && !invalid_first_user_evidence.contains(&candidate.turn)
                        && candidate.start_ordinal.is_none_or(|start| start < response)
                        && candidate
                            .first_user_projection
                            .as_ref()
                            .is_none_or(|(_, projection)| response < *projection)
                })
        };
        candidate.turn_boundary = boundaries
            .get(&candidate.turn)
            .filter(|boundary| {
                !invalid_boundaries.contains(&candidate.turn)
                    && boundary.start_ordinal < candidate.ordinal
                    && candidate.ordinal < boundary.end_ordinal
                    && candidate.projection.as_ref().is_none_or(|(_, ordinal)| {
                        boundary.start_ordinal < *ordinal && *ordinal < boundary.end_ordinal
                    })
            })
            .cloned();
        candidate.following_turn = following_turns
            .get(&candidate.turn)
            .and_then(|following| following.as_ref())
            .filter(|(id, ordinal)| {
                candidate.start_ordinal.is_some()
                    && !duplicate_starts.contains(id)
                    && id != &candidate.turn
                    && *ordinal > candidate.ordinal
            })
            .cloned();
    }
    result.sort_by_key(|candidate| candidate.ordinal);
    issues.sort();
    Ok(ScanResult {
        candidates: result,
        issues,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    /// 只读现场核验，路径与目标轮次均由调用方显式提供；不输出消息原文。
    #[test]
    #[ignore = "需要显式指定 CODEX_INDEX_VERIFY_ROLLOUT、CODEX_INDEX_VERIFY_TURN 和 CODEX_INDEX_VERIFY_TEXT"]
    fn verify_real_rollout_read_only() {
        let path = std::env::var_os("CODEX_INDEX_VERIFY_ROLLOUT")
            .expect("需要 CODEX_INDEX_VERIFY_ROLLOUT");
        let turn = std::env::var("CODEX_INDEX_VERIFY_TURN").expect("需要 CODEX_INDEX_VERIFY_TURN");
        let text = std::env::var("CODEX_INDEX_VERIFY_TEXT").expect("需要 CODEX_INDEX_VERIFY_TEXT");
        assert!(!text.is_empty(), "核验原文不能为空");
        let candidates = scan(Path::new(&path)).unwrap().candidates;
        let found = candidates
            .iter()
            .any(|candidate| candidate.turn == turn && candidate.text.starts_with(&text));
        eprintln!("candidates={}, confirmed_message={found}", candidates.len());
        assert!(found, "未找到指定轮次的待核验原文");
    }

    fn scan_rows(rows: Vec<Value>, tail: &str) -> anyhow::Result<Vec<Candidate>> {
        scan_rows_with_issues(rows, tail).map(|result| result.candidates)
    }

    fn scan_rows_with_issues(rows: Vec<Value>, tail: &str) -> anyhow::Result<ScanResult> {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for row in rows {
            writeln!(file, "{row}").unwrap();
        }
        write!(file, "{tail}").unwrap();
        scan(file.path())
    }
    fn message(turn: &str, text: &str, ordinal: i64) -> Value {
        json!({"type":"response_item","ordinal":ordinal,"timestamp":"2026-09-14T12:24:30Z", "payload":{"role":"user","content":[{"type":"input_text","text":text}],"internal_chat_message_metadata_passthrough":{"turn_id":turn}}})
    }
    fn start(turn: &str) -> Value {
        json!({"type":"event_msg","payload":{"type":"task_started","turn_id":turn}})
    }
    fn goal(thread: &str) -> Value {
        goal_at(thread, 100)
    }
    fn goal_at(thread: &str, created: i64) -> Value {
        json!({"type":"event_msg","payload":{"type":"thread_goal_updated","threadId":thread,"goal":{"objective":"原文","createdAt":created}}})
    }
    fn meta() -> Value {
        json!({"type":"session_meta","payload":{"id":"t"}})
    }

    fn start_at(turn: &str, ordinal: i64) -> Value {
        let mut row = start(turn);
        row["ordinal"] = json!(ordinal);
        row
    }

    fn confirmed_message_rows() -> Vec<Value> {
        vec![
            meta(),
            start_at("a", 1),
            message("a", "原文", 2),
            json!({"type":"event_msg","ordinal":3,"payload":{"type":"user_message","turn_id":"a","message":"原文"}}),
        ]
    }

    #[test]
    fn duplicate_task_starts_invalidate_ordinary_message_evidence() {
        let rows = vec![
            meta(),
            start_at("a", 1),
            message("a", "第一条", 2),
            json!({"type":"event_msg","ordinal":3,"payload":{"type":"user_message","turn_id":"a","message":"第一条"}}),
            start_at("a", 4),
            message("a", "第二条", 5),
            json!({"type":"event_msg","ordinal":6,"payload":{"type":"user_message","turn_id":"a","message":"第二条"}}),
        ];
        let candidates = scan_rows(rows, "").unwrap();
        assert_eq!(candidates.len(), 2);
        assert!(
            candidates
                .iter()
                .all(|candidate| { !candidate.first_user_evidence_valid })
        );
    }

    #[test]
    fn user_event_after_terminal_boundary_cannot_confirm_an_old_response() {
        let rows = vec![
            meta(),
            start_at("a", 1),
            message("a", "迟到", 2),
            json!({"type":"event_msg","ordinal":3,"payload":{"type":"task_complete","turn_id":"a"}}),
            json!({"type":"event_msg","ordinal":4,"payload":{"type":"user_message","turn_id":"a","message":"迟到"}}),
        ];
        assert!(scan_rows(rows, "").unwrap().is_empty());
    }

    #[test]
    fn completed_item_after_terminal_boundary_cannot_confirm_an_old_response() {
        let rows = vec![
            meta(),
            start_at("a", 1),
            message("a", "迟到完成", 2),
            json!({"type":"event_msg","ordinal":3,"payload":{"type":"task_complete","turn_id":"a"}}),
            json!({"type":"event_msg","ordinal":4,"payload":{"type":"item_completed","thread_id":"t","turn_id":"a","item":{"id":"late","type":"UserMessage","content":[{"type":"text","text":"迟到完成"}]}}}),
        ];
        let result = scan_rows_with_issues(rows, "").unwrap();
        assert!(result.candidates.is_empty());
        assert!(result.issues.is_empty());
    }

    #[test]
    fn following_turn_uses_immediate_task_started_even_without_completion() {
        for completed in [false, true] {
            let mut rows = confirmed_message_rows();
            if completed {
                rows.push(json!({"type":"event_msg","ordinal":4,"payload":{"type":"task_complete","turn_id":"a"}}));
            }
            rows.extend([start_at("b", 5), start_at("c", 8)]);
            let candidates = scan_rows(rows, "").unwrap();
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0].following_turn, Some(("b".into(), 5)));
        }
    }

    #[test]
    fn following_turn_requires_distinct_valid_boundary_after_candidate() {
        for next in [
            None,
            Some(start("b")),
            Some(start_at("b", -1)),
            Some(start_at("b", 2)),
            Some(start_at("a", 5)),
            Some(
                json!({"type":"event_msg","ordinal":5,"payload":{"type":"task_complete","turn_id":"b"}}),
            ),
            Some(
                json!({"type":"event_msg","ordinal":5,"payload":{"type":"task_started","turn_id":"b","thread_id":"other"}}),
            ),
            Some(
                json!({"type":"event_msg","ordinal":5,"payload":{"type":"task_started","turn_id":"b","threadId":"other"}}),
            ),
        ] {
            let mut rows = confirmed_message_rows();
            rows.extend(next);
            let candidates = scan_rows(rows, "").unwrap();
            assert_eq!(candidates.len(), 1);
            assert!(candidates[0].following_turn.is_none());
        }
    }

    #[test]
    fn following_turn_does_not_skip_an_unverifiable_intermediate_start() {
        for next in [
            start("b"),
            start_at("b", -1),
            start_at("", 5),
            json!({"type":"event_msg","ordinal":5,"payload":{"type":"task_started"}}),
        ] {
            let mut rows = confirmed_message_rows();
            rows.extend([next, start_at("c", 8)]);
            let candidates = scan_rows(rows, "").unwrap();
            assert_eq!(candidates.len(), 1);
            assert!(candidates[0].following_turn.is_none());
        }
    }

    #[test]
    fn following_turn_ignores_inherited_history_and_foreign_thread_events() {
        let rows = vec![
            json!({"type":"session_meta","ordinal":0,"payload":{"id":"t","parent_thread_id":"parent","subagent_history_start_ordinal":5}}),
            json!({"type":"session_meta","ordinal":1,"payload":{"id":"parent"}}),
            start_at("parent-a", 2),
            start_at("parent-b", 3),
            start_at("a", 5),
            message("a", "原文", 6),
            json!({"type":"event_msg","ordinal":7,"payload":{"type":"user_message","turn_id":"a","message":"原文"}}),
            json!({"type":"event_msg","ordinal":8,"payload":{"type":"task_started","turn_id":"parent-c","thread_id":"parent"}}),
            start_at("b", 9),
        ];
        let candidates = scan_rows(rows, "").unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].following_turn, Some(("b".into(), 9)));
    }

    #[test]
    fn following_turn_is_optional_in_older_cached_candidates() {
        let candidate: Candidate = serde_json::from_value(json!({
            "thread":"t", "turn":"a", "text":"原文", "ordinal":2,
            "created":1000, "goal":false,
        }))
        .unwrap();
        assert!(candidate.following_turn.is_none());
        assert!(candidate.turn_boundary.is_none());
        assert!(candidate.start_ordinal.is_none());
        assert!(candidate.goal_key.is_none());
        assert!(candidate.first_user_projection.is_none());
        assert!(candidate.first_user_response_ordinal.is_none());
        assert!(!candidate.first_user_evidence_valid);
    }

    #[test]
    fn following_turn_rejects_repeated_source_or_destination_starts() {
        for extra in [start_at("a", 8), start_at("b", 8)] {
            let mut rows = confirmed_message_rows();
            rows.extend([start_at("b", 5), extra]);
            let items = scan_rows(rows, "").unwrap();
            assert!(items[0].following_turn.is_none());
        }
        let mut rows = confirmed_message_rows();
        rows.push(start_at("b", 5));
        let items = scan_rows(rows, "").unwrap();
        assert_eq!(items[0].start_ordinal, Some(1));
        assert_eq!(items[0].following_turn, Some(("b".into(), 5)));
    }

    #[test]
    fn following_turn_requires_unique_source_start_ordinal() {
        let mut rows = confirmed_message_rows();
        rows[1].as_object_mut().unwrap().remove("ordinal");
        rows.push(start_at("b", 5));
        let items = scan_rows(rows, "").unwrap();
        assert!(items[0].start_ordinal.is_none());
        assert!(items[0].following_turn.is_none());
    }

    fn complete_boundary_rows() -> Vec<Value> {
        let mut rows = confirmed_message_rows();
        rows[1]["payload"]["started_at"] = json!(100);
        rows.push(json!({"type":"event_msg","ordinal":5,"payload":{
            "type":"task_complete","turn_id":"a","started_at":100,
            "completed_at":105,"duration_ms":5000,
        }}));
        rows
    }

    #[test]
    fn complete_turn_boundary_preserves_terminal_status_times_and_errors() {
        for (kind, error, status) in [
            ("task_complete", Value::Null, "completed"),
            (
                "task_complete",
                json!({"message":"failure","codex_error_info":"other"}),
                "failed",
            ),
            ("task_failed", Value::Null, "failed"),
            ("turn_aborted", Value::Null, "interrupted"),
        ] {
            let mut rows = complete_boundary_rows();
            rows[4]["payload"]["type"] = json!(kind);
            rows[4]["payload"]["error"] = error.clone();
            let items = scan_rows(rows, "").unwrap();
            let boundary = items[0].turn_boundary.as_ref().unwrap();
            assert_eq!((boundary.start_ordinal, boundary.end_ordinal), (1, 5));
            assert_eq!((boundary.started_at, boundary.completed_at), (100, 105));
            assert_eq!(boundary.status, status);
            assert_eq!(boundary.duration_ms, Some(5000));
            assert_eq!(
                boundary.error_json,
                (!error.is_null()).then(|| error.to_string())
            );
        }
    }

    #[test]
    fn complete_turn_boundary_requires_identity_ordinals_and_times() {
        for invalid in 0..11 {
            let mut rows = complete_boundary_rows();
            match invalid {
                0 => {
                    rows[1].as_object_mut().unwrap().remove("ordinal");
                }
                1 => {
                    rows[1]["payload"]
                        .as_object_mut()
                        .unwrap()
                        .remove("started_at");
                }
                2 => {
                    rows[4].as_object_mut().unwrap().remove("ordinal");
                }
                3 => {
                    rows[4]["payload"]
                        .as_object_mut()
                        .unwrap()
                        .remove("completed_at");
                }
                4 => rows[4]["payload"]["turn_id"] = json!("other"),
                5 => rows[4]["payload"]["started_at"] = json!(101),
                6 => rows[4]["payload"]["completed_at"] = json!(99),
                7 => rows[4]["ordinal"] = json!(2),
                8 => rows[1]["ordinal"] = json!(2),
                9 => rows[1]["ordinal"] = json!(-1),
                10 => rows[4]["payload"]["thread_id"] = json!("other"),
                _ => unreachable!(),
            }
            let items = scan_rows(rows, "").unwrap();
            assert_eq!(items.len(), 1, "invalid case {invalid}");
            assert!(items[0].turn_boundary.is_none(), "invalid case {invalid}");
        }
    }

    #[test]
    fn complete_turn_boundary_rejects_duplicates_interleaving_and_late_projection() {
        for invalid in 0..6 {
            let mut rows = complete_boundary_rows();
            match invalid {
                0 => rows.insert(2, rows[1].clone()),
                1 => rows.push(rows[4].clone()),
                2 => rows.insert(4, start_at("other", 4)),
                3 => rows.push(start_at("a", 8)),
                4 => rows.insert(1, json!({"type":"event_msg","ordinal":0,"payload":{"type":"task_complete","turn_id":"a","completed_at":99}})),
                5 => rows.push(json!({"type":"event_msg","ordinal":6,"payload":{"type":"item_completed","thread_id":"t","turn_id":"a","item":{"id":"late","type":"UserMessage","content":[{"type":"text","text":"原文"}]}}})),
                _ => unreachable!(),
            }
            let items = scan_rows(rows, "").unwrap();
            assert_eq!(items.len(), 1, "invalid case {invalid}");
            assert!(items[0].turn_boundary.is_none(), "invalid case {invalid}");
        }
    }

    #[test]
    fn complete_turn_boundary_can_use_timestamp_and_unambiguous_current_turn() {
        let mut rows = complete_boundary_rows();
        rows[1]["payload"]
            .as_object_mut()
            .unwrap()
            .remove("started_at");
        rows[1]["timestamp"] = json!("1970-01-01T00:01:40Z");
        rows[4]["payload"]
            .as_object_mut()
            .unwrap()
            .remove("completed_at");
        rows[4]["payload"]
            .as_object_mut()
            .unwrap()
            .remove("turn_id");
        rows[4]["timestamp"] = json!("1970-01-01T00:01:45Z");
        let items = scan_rows(rows, "").unwrap();
        let boundary = items[0].turn_boundary.as_ref().unwrap();
        assert_eq!((boundary.started_at, boundary.completed_at), (100, 105));
    }

    #[test]
    fn complete_turn_boundary_applies_to_goals_and_verified_projection() {
        let mut rows = complete_boundary_rows();
        rows[3] = json!({"type":"event_msg","ordinal":4,"payload":{"type":"item_completed","thread_id":"t","turn_id":"a","item":{"id":"native","type":"UserMessage","content":[{"type":"text","text":"原文"}]}}});
        let projected = scan_rows(rows, "").unwrap();
        assert_eq!(projected[0].projection, Some(("native".into(), 4)));
        assert!(projected[0].turn_boundary.is_some());

        let mut rows = complete_boundary_rows();
        rows.insert(1, goal("t"));
        rows[3] = message(
            "a",
            "<codex_internal_context><objective>\n原文\n</objective></codex_internal_context>",
            2,
        );
        rows.remove(4);
        rows.push(start_at("b", 6));
        let goals = scan_rows(rows, "").unwrap();
        assert_eq!(goals.len(), 1);
        assert!(goals[0].goal);
        assert!(goals[0].turn_boundary.is_some());
        assert_eq!(goals[0].following_turn, Some(("b".into(), 6)));
    }

    #[test]
    fn boundary_first_user_evidence_includes_attachments_before_text_steering() {
        let mut rows = complete_boundary_rows();
        rows[2] = message("a", "原文", 6);
        rows[3]["ordinal"] = json!(7);
        rows[4]["ordinal"] = json!(10);
        rows.insert(2, json!({"type":"response_item","ordinal":2,"payload":{"role":"user","content":[{"type":"input_image","image_url":"local"}],"internal_chat_message_metadata_passthrough":{"turn_id":"a"}}}));
        rows.insert(3, json!({"type":"event_msg","ordinal":3,"payload":{"type":"item_completed","thread_id":"t","turn_id":"a","item":{"id":"attachment","type":"UserMessage","content":[{"type":"image","image_url":"local"}]}}}));
        let items = scan_rows(rows, "").unwrap();
        assert_eq!(items.len(), 1);
        let boundary = items[0].turn_boundary.as_ref().unwrap();
        assert_eq!(
            boundary.first_user_projection,
            Some(("attachment".into(), 3))
        );
        assert_eq!(boundary.first_user_response_ordinal, Some(2));
        assert_eq!(items[0].ordinal, 6);
    }

    #[test]
    fn boundary_rejects_unidentified_user_projection_and_response() {
        for invalid in 0..6 {
            let mut rows = complete_boundary_rows();
            let mut item = json!({"type":"event_msg","ordinal":4,"payload":{"type":"item_completed","thread_id":"t","turn_id":"a","item":{"id":"native","type":"UserMessage","content":[{"type":"image","image_url":"local"}]}}});
            match invalid {
                0 => {
                    item["payload"]["item"]
                        .as_object_mut()
                        .unwrap()
                        .remove("id");
                }
                1 => {
                    item.as_object_mut().unwrap().remove("ordinal");
                }
                2 => {
                    item["payload"].as_object_mut().unwrap().remove("turn_id");
                }
                3 => {
                    item["payload"].as_object_mut().unwrap().remove("thread_id");
                }
                4 | 5 => {
                    item = message("a", "附件", 4);
                    item["payload"]["content"] =
                        json!([{"type":"input_image","image_url":"local"}]);
                    if invalid == 4 {
                        item.as_object_mut().unwrap().remove("ordinal");
                    } else {
                        item["payload"]["internal_chat_message_metadata_passthrough"]
                            .as_object_mut()
                            .unwrap()
                            .remove("turn_id");
                    }
                }
                _ => unreachable!(),
            }
            rows.insert(4, item);
            let items = scan_rows(rows, "").unwrap();
            assert_eq!(items.len(), 1);
            assert!(items[0].turn_boundary.is_none(), "invalid case {invalid}");
        }
    }

    #[test]
    fn boundary_first_user_response_excludes_internal_and_environment_messages() {
        let mut rows = complete_boundary_rows();
        rows[2]["ordinal"] = json!(6);
        rows[3]["ordinal"] = json!(7);
        rows[4]["ordinal"] = json!(8);
        rows.insert(
            2,
            message("a", "<environment_context>环境</environment_context>", 2),
        );
        rows.insert(
            3,
            message(
                "a",
                "<codex_internal_context>续执行</codex_internal_context>",
                3,
            ),
        );
        let items = scan_rows(rows, "").unwrap();
        let boundary = items[0].turn_boundary.as_ref().unwrap();
        assert_eq!(boundary.first_user_response_ordinal, Some(6));
    }

    #[test]
    fn goals_require_same_thread_and_first_matching_turn_and_deduplicate_continuations() {
        let body = "<codex_internal_context source=\"goal\"><objective>\n原文\n</objective></codex_internal_context>";
        let rows = vec![
            meta(),
            goal("wrong"),
            start("a"),
            message("a", body, 3),
            goal("t"),
            start("b"),
            message("wrong", body, 6),
            message("b", body, 7),
            start("c"),
            message("c", body, 9),
            goal("t"),
            start("d"),
            message("d", body, 12),
        ];
        let items = scan_rows(rows, "").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            (&items[0].turn, &items[0].text, items[0].created),
            (&"b".into(), &"原文".into(), 100000)
        );
    }

    #[test]
    fn repeated_goal_text_uses_goal_identity_instead_of_body() {
        let body = "<codex_internal_context source=\"goal\"><objective>\n原文\n</objective></codex_internal_context>";
        let rows = vec![
            meta(),
            goal_at("t", 100),
            start_at("a", 2),
            message("a", body, 3),
            start_at("continuation", 4),
            message("continuation", body, 5),
            goal_at("t", 200),
            start_at("b", 7),
            message("b", body, 8),
        ];
        let items = scan_rows(rows, "").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].goal_key.as_deref(), Some("created:100"));
        assert_eq!(items[1].goal_key.as_deref(), Some("created:200"));
        assert_eq!(items[0].text, items[1].text);
    }

    #[test]
    fn completed_item_and_user_message_evidence_are_consumed_once() {
        let completed = vec![
            meta(),
            start_at("a", 1),
            message("a", "重复", 2),
            message("a", "重复", 3),
            json!({"type":"event_msg","ordinal":4,"payload":{"type":"item_completed","thread_id":"t","turn_id":"a","item":{"id":"one","type":"UserMessage","content":[{"type":"text","text":"重复"}]}}}),
        ];
        let items = scan_rows(completed, "").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].ordinal, 3);
        assert_eq!(items[0].projection, Some(("one".into(), 4)));

        let event = vec![
            meta(),
            start_at("a", 1),
            message("a", "重复", 2),
            json!({"type":"event_msg","payload":{"type":"user_message","turn_id":"a","message":"重复"}}),
            message("a", "重复", 4),
        ];
        let result = scan_rows_with_issues(event, "").unwrap();
        assert!(result.candidates.is_empty());
        assert_eq!(result.issues.len(), 2);
        assert!(
            result
                .issues
                .iter()
                .all(|issue| issue.contains("重复文本用户事件数量与响应不一致"))
        );

        let balanced = vec![
            meta(),
            start_at("a", 1),
            json!({"type":"event_msg","payload":{"type":"user_message","turn_id":"a","message":"重复"}}),
            message("a", "重复", 3),
            message("a", "重复", 4),
            json!({"type":"event_msg","payload":{"type":"user_message","turn_id":"a","message":"重复"}}),
        ];
        let items = scan_rows(balanced, "").unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn multiple_completions_for_one_response_require_review() {
        let mut rows = confirmed_message_rows();
        for (id, ordinal) in [("native-a", 4), ("native-b", 5)] {
            rows.push(json!({"type":"event_msg","ordinal":ordinal,"payload":{"type":"item_completed","thread_id":"t","turn_id":"a","item":{"id":id,"type":"UserMessage","content":[{"type":"text","text":"原文"}]}}}));
        }
        let result = scan_rows_with_issues(rows, "").unwrap();
        assert!(result.candidates.is_empty());
        assert_eq!(result.issues.len(), 1);
        assert!(result.issues[0].contains("多个完成事件"));
    }

    #[test]
    fn first_user_evidence_survives_missing_terminal_boundary() {
        let rows = vec![
            meta(),
            start_at("a", 1),
            json!({"type":"response_item","ordinal":2,"payload":{"role":"user","content":[{"type":"input_image","image_url":"local"}],"internal_chat_message_metadata_passthrough":{"turn_id":"a"}}}),
            json!({"type":"event_msg","ordinal":3,"payload":{"type":"item_completed","thread_id":"t","turn_id":"a","item":{"id":"attachment","type":"UserMessage","content":[{"type":"image","image_url":"local"}]}}}),
            message("a", "后续文本", 4),
            json!({"type":"event_msg","ordinal":5,"payload":{"type":"user_message","turn_id":"a","message":"后续文本"}}),
        ];
        let items = scan_rows(rows, "").unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].turn_boundary.is_none());
        assert_eq!(
            items[0].first_user_projection,
            Some(("attachment".into(), 3))
        );
        assert_eq!(items[0].first_user_response_ordinal, Some(2));
        assert!(items[0].first_user_evidence_valid);
    }

    #[test]
    fn missing_first_user_identity_or_ordinal_marks_evidence_invalid() {
        for invalid in 0..2 {
            let mut first = json!({"type":"response_item","ordinal":2,"payload":{"role":"user","content":[{"type":"input_image","image_url":"local"}],"internal_chat_message_metadata_passthrough":{"turn_id":"a"}}});
            if invalid == 0 {
                first.as_object_mut().unwrap().remove("ordinal");
            } else {
                first["payload"]["internal_chat_message_metadata_passthrough"] = json!({});
            }
            let rows = vec![
                meta(),
                start_at("a", 1),
                first,
                message("a", "后续文本", 4),
                json!({"type":"event_msg","ordinal":5,"payload":{"type":"user_message","turn_id":"a","message":"后续文本"}}),
            ];
            let items = scan_rows(rows, "").unwrap();
            assert_eq!(items.len(), 1);
            assert!(
                !items[0].first_user_evidence_valid,
                "invalid case {invalid}"
            );
        }
    }

    #[test]
    fn ordinary_messages_need_corroboration_in_same_turn_and_exclude_environment() {
        let rows = vec![
            meta(),
            start("a"),
            message("a", "第一条", 2),
            json!({"type":"event_msg","ordinal":3,"payload":{"type":"item_completed","thread_id":"t","turn_id":"a","item":{"id":"first","type":"UserMessage","content":[{"type":"text","text":"第一条"}]}}}),
            start("b"),
            message("b", "第二条", 5),
            json!({"type":"event_msg","payload":{"type":"user_message","message":"第二条"}}),
            message("b", "无证据", 7),
            message("a", "第二条", 8),
            message("b", "<environment_context>环境</environment_context>", 9),
            json!({"type":"event_msg","payload":{"type":"user_message","message":"<environment_context>环境</environment_context>"}}),
        ];
        let items = scan_rows(rows, "").unwrap();
        assert_eq!(
            items.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            ["第一条", "第二条"]
        );
    }

    #[test]
    fn fork_scans_own_messages_without_inheriting_parent_turns() {
        let rows = vec![
            json!({"type":"session_meta","ordinal":0,"payload":{"id":"t","parent_thread_id":"parent","forked_from_id":"parent","subagent_history_start_ordinal":5}}),
            json!({"type":"session_meta","ordinal":1,"payload":{"id":"parent"}}),
            message("parent-turn", "父任务指令", 2),
            json!({"type":"event_msg","ordinal":3,"payload":{"type":"user_message","turn_id":"parent-turn","message":"父任务指令"}}),
            json!({"type":"event_msg","ordinal":4,"payload":{"type":"task_started","turn_id":"parent-turn"}}),
            message("child-turn", "子任务指令", 5),
            json!({"type":"event_msg","ordinal":6,"payload":{"type":"user_message","turn_id":"child-turn","message":"子任务指令"}}),
        ];
        let items = scan_rows(rows.clone(), "").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            (&items[0].thread[..], &items[0].turn[..], &items[0].text[..]),
            ("t", "child-turn", "子任务指令")
        );

        let mut foreign = rows.clone();
        foreign[1]["payload"]["id"] = json!("unrelated");
        assert!(scan_rows(foreign, "").is_err());
        let mut outside_boundary = rows.clone();
        outside_boundary[1]["ordinal"] = json!(5);
        assert!(scan_rows(outside_boundary, "").is_err());
        let mut no_boundary = rows.clone();
        no_boundary[0]["payload"]
            .as_object_mut()
            .unwrap()
            .remove("subagent_history_start_ordinal");
        assert!(scan_rows(no_boundary, "").is_err());
        let mut no_ordinal = rows;
        no_ordinal[2].as_object_mut().unwrap().remove("ordinal");
        assert!(scan_rows(no_ordinal, "").is_err());
    }

    #[test]
    fn rejects_mixed_sessions_and_nonterminal_corruption() {
        assert!(
            scan_rows(
                vec![
                    meta(),
                    json!({"type":"session_meta","payload":{"id":"other"}})
                ],
                ""
            )
            .is_err()
        );
        assert!(scan_rows(vec![meta()], "bad\n").is_err());
        let error = scan_rows(vec![meta()], "{\"type\":").unwrap_err();
        assert!(error.to_string().contains("会话记录尾行不完整"));
    }

    #[test]
    fn missing_identity_or_ordinal_and_partial_attachments_are_not_recovered() {
        let mut missing_ordinal = message("a", "原文", 2);
        missing_ordinal.as_object_mut().unwrap().remove("ordinal");
        let mut missing_turn = message("a", "原文", 3);
        missing_turn["payload"]["internal_chat_message_metadata_passthrough"] = json!({});
        let mut attachment = message("a", "原文", 4);
        attachment["payload"]["content"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"input_image","image_url":"file:///image.png"}));
        let rows = vec![
            meta(),
            start("a"),
            missing_ordinal,
            missing_turn,
            attachment,
            json!({"type":"event_msg","payload":{"type":"user_message","message":"原文"}}),
        ];
        assert!(scan_rows(rows, "").unwrap().is_empty());
    }
}
