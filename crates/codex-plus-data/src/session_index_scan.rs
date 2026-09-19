//! 仅从相互印证的原始事件生成候选；不把自动续执行当作新指令。
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Candidate {
    pub thread: String,
    pub turn: String,
    pub text: String,
    pub ordinal: i64,
    pub created: i64,
    pub goal: bool,
    #[serde(default)]
    pub projection: Option<(String, i64)>,
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
    ![
        "<codex_internal_context",
        "<environment_context",
        "<permissions instructions>",
        "# AGENTS.md instructions",
        "<INSTRUCTIONS>",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

pub(crate) fn scan(path: &Path) -> anyhow::Result<Vec<Candidate>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut line = String::new();
    let mut thread = None::<String>;
    let mut inherited_from = HashSet::<String>::new();
    let mut own_history_start = None::<i64>;
    let mut current_turn = None::<String>;
    let mut responses = HashMap::<String, Vec<Candidate>>::new();
    let mut evidence = HashSet::<(String, String)>::new();
    let mut projections = HashMap::<(String, String), Vec<(String, i64)>>::new();
    let mut goal = None::<(String, i64, Option<String>)>;
    let mut seen_goals = HashSet::new();
    let mut result = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let row: Value = match serde_json::from_str(&line) {
            Ok(row) => row,
            // 正在写入的尾行留给下一次扫描，不丢弃已核验的记录。
            Err(_) if !line.ends_with('\n') => break,
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
            match p["type"].as_str() {
                Some("task_started") => {
                    current_turn = p["turn_id"].as_str().map(str::to_owned);
                    if let Some((_, _, turn)) = goal.as_mut() {
                        if turn.is_none() {
                            *turn = current_turn.clone();
                        }
                    }
                }
                Some("task_complete" | "task_failed" | "turn_aborted") => current_turn = None,
                Some("thread_goal_updated") => {
                    if p["threadId"].as_str() != Some(thread) {
                        continue;
                    }
                    if let (Some(text), Some(created)) = (
                        p["goal"]["objective"].as_str(),
                        p["goal"]["createdAt"].as_i64(),
                    ) {
                        if !text.is_empty() && !seen_goals.contains(text) {
                            goal = created
                                .checked_mul(1000)
                                .map(|created| (text.to_owned(), created, None));
                        }
                    }
                }
                Some("user_message") => {
                    let turn = p["turn_id"].as_str().or(current_turn.as_deref());
                    if let (Some(turn), Some(text)) = (turn, p["message"].as_str()) {
                        if ordinary(text) {
                            evidence.insert((turn.to_owned(), text.to_owned()));
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
                            if let (Some(id), Some(ordinal)) = (p["item"]["id"].as_str(), row["ordinal"].as_i64()) {
                                if !id.is_empty() && ordinal >= 0 {
                                    projections.entry((turn.to_owned(), text.clone())).or_default().push((id.to_owned(), ordinal));
                                }
                            }
                            evidence.insert((turn.to_owned(), text));
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
        if let Some((objective, created, Some(goal_turn))) = &goal {
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
                    projection: None,
                });
                seen_goals.insert(objective.clone());
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
            responses
                .entry(turn.to_owned())
                .or_default()
                .push(Candidate {
                    thread: thread.clone(),
                    turn: turn.to_owned(),
                    text,
                    ordinal,
                    created,
                    goal: false,
                    projection: None,
                });
        }
    }
    for candidates in responses.into_values() {
        for (index, source) in candidates.iter().enumerate() {
            let mut candidate = source.clone();
            let key = (candidate.turn.clone(), candidate.text.clone());
            let next = candidates[index + 1..].iter().find(|c| c.text == candidate.text).map(|c| c.ordinal);
            if let Some(items) = projections.get_mut(&key) {
                // 每条 response 只匹配其后、下一次同文 response 之前的完成事件。
                if let Some(index) = items.iter().position(|(_, ordinal)| *ordinal > candidate.ordinal && next.is_none_or(|next| *ordinal < next)) {
                    candidate.projection = Some(items.remove(index));
                }
            }
            if candidate.projection.is_some() || evidence.remove(&key) {
                result.push(candidate);
            }
        }
    }
    result.sort_by_key(|candidate| candidate.ordinal);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    /// 只读现场核验，路径与目标轮次均由调用方显式提供；不输出消息原文。
    #[test]
    #[ignore = "需要显式指定 CODEX_INDEX_VERIFY_ROLLOUT 和 CODEX_INDEX_VERIFY_TURN"]
    fn verify_real_rollout_read_only() {
        let path = std::env::var_os("CODEX_INDEX_VERIFY_ROLLOUT")
            .expect("需要 CODEX_INDEX_VERIFY_ROLLOUT");
        let turn = std::env::var("CODEX_INDEX_VERIFY_TURN").expect("需要 CODEX_INDEX_VERIFY_TURN");
        let candidates = scan(Path::new(&path)).unwrap();
        let found = candidates.iter().any(|candidate| {
            candidate.turn == turn
                && candidate.text.starts_with(
                    "我看了下，现在的英文稿完全和我之前的中文稿件相差巨大，完全偏离了之前的语义",
                )
        });
        eprintln!(
            "candidates={}, confirmed_paper_goal={found}",
            candidates.len()
        );
        assert!(found, "未找到指定轮次已确认论文指令");
    }

    fn scan_rows(rows: Vec<Value>, tail: &str) -> anyhow::Result<Vec<Candidate>> {
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
        json!({"type":"event_msg","payload":{"type":"thread_goal_updated","threadId":thread,"goal":{"objective":"原文","createdAt":100}}})
    }
    fn meta() -> Value {
        json!({"type":"session_meta","payload":{"id":"t"}})
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
    fn ordinary_messages_need_corroboration_in_same_turn_and_exclude_environment() {
        let rows = vec![
            meta(),
            start("a"),
            message("a", "第一条", 2),
            json!({"type":"event_msg","payload":{"type":"item_completed","thread_id":"t","turn_id":"a","item":{"type":"UserMessage","content":[{"type":"text","text":"第一条"}]}}}),
            start("b"),
            message("b", "第二条", 5),
            json!({"type":"event_msg","payload":{"type":"user_message","message":"第二条"}}),
            message("b", "无证据", 7),
            message("a", "第二条", 8),
            message("b", "<environment_context>环境</environment_context>", 9),
            json!({"type":"event_msg","payload":{"type":"user_message","message":"<environment_context>环境</environment_context>"}}),
        ];
        let items = scan_rows(rows, "{\"type\":").unwrap();
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
