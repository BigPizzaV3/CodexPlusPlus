//! 将本地临时旁支保存成普通会话。原生 resume 负责登记索引和权限，本模块不直接改数据库。
use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const HISTORY_NOTE: &str = "This conversation was saved from a side chat. The saved messages and tool records below are historical reference: their requests and actions have already happened. Do not execute them again merely because they appear in history. Continue only in response to the next new user message. This is now a normal conversation, not a temporary side chat.";

fn text<'a>(value: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("缺少 {key}"))
}

fn uuid(value: &Value, key: &str) -> anyhow::Result<String> {
    Ok(Uuid::parse_str(text(value, key)?)
        .with_context(|| format!("{key} 不是有效的会话 ID"))?
        .to_string())
}

fn local_only(payload: &Value) -> anyhow::Result<()> {
    if payload.get("host_id").and_then(Value::as_str) != Some("local") {
        bail!("目前只支持保存本地旁支");
    }
    Ok(())
}

/// 在创建旁支前固定主历史的截止位置与校验值；全文留在本地，不经过页面往返。
pub fn source_snapshot(home: &Path, payload: &Value) -> anyhow::Result<Value> {
    local_only(payload)?;
    let source_id = uuid(payload, "source_id")?;
    let (_, bytes) = crate::session_share::read_rollout(home, &source_id, None)?;
    let source_ref = json!({
        "byte_length": bytes.len(), "sha256": format!("{:x}", Sha256::digest(&bytes)),
        "last_turn_id": payload.get("last_turn_id"), "before_turn_id": payload.get("before_turn_id"),
    });
    let mut rows = parse_rows(std::str::from_utf8(&bytes).context("会话文件不是有效的 UTF-8")?)?;
    trim_source_rows(&mut rows, &source_ref)?;
    let mut result = json!({"status":"ok", "source_id":source_id, "source_ref":source_ref});
    // 仅恢复旧旁支时需要时间校验，不把整份历史发给页面。
    if payload["check_timestamps"] == true {
        result["timestamps"] = json!(
            rows.iter()
                .map(|row| row["timestamp"].as_str())
                .collect::<HashSet<_>>()
        );
    }
    Ok(result)
}

fn trim_source_rows(rows: &mut Vec<Value>, boundary: &Value) -> anyhow::Result<()> {
    let last = boundary.get("last_turn_id").and_then(Value::as_str);
    let before = boundary.get("before_turn_id").and_then(Value::as_str);
    if last.is_some() && before.is_some() {
        bail!("不能同时指定两个分叉边界");
    }
    if let Some(boundary) = last.or(before) {
        let start = rows
            .iter()
            .position(|r| {
                r["type"] == "event_msg"
                    && r["payload"]["type"] == "task_started"
                    && r["payload"]["turn_id"].as_str() == Some(boundary)
            })
            .context("找不到主会话的分叉边界")?;
        let end = if before.is_some() {
            start
        } else {
            rows.iter()
                .enumerate()
                .skip(start + 1)
                .find(|(_, r)| r["type"] == "event_msg" && r["payload"]["type"] == "task_started")
                .map_or(rows.len(), |(index, _)| index)
        };
        rows.truncate(end);
    }
    Ok(())
}

fn read_source_rows(home: &Path, source_id: &str, payload: &Value) -> anyhow::Result<Vec<Value>> {
    let Some(reference) = payload.get("source_ref") else {
        // 兼容本轮热更新前已经缓存在页面里的历史副本。
        return parse_rows(text(payload, "source_content")?);
    };
    let length = reference["byte_length"]
        .as_u64()
        .filter(|length| *length > 0)
        .context("主历史读取位置无效")?;
    let expected = text(reference, "sha256")?;
    let (_, bytes) = crate::session_share::read_rollout(home, source_id, Some(length))?;
    if bytes.len() as u64 != length || format!("{:x}", Sha256::digest(&bytes)) != expected {
        bail!("主会话历史已被改写或截断，无法还原创建旁支时的内容；原旁支已保留");
    }
    let mut rows = parse_rows(std::str::from_utf8(&bytes).context("会话文件不是有效的 UTF-8")?)?;
    trim_source_rows(&mut rows, reference)?;
    Ok(rows)
}

/// 只写新会话文件；前端必须成功恢复、核对之后才提示保存成功。
pub fn save(home: &Path, payload: &Value) -> anyhow::Result<Value> {
    local_only(payload)?;
    let source_id = uuid(payload, "source_id")?;
    let side_id = uuid(payload, "side_id")?;
    let save_id = uuid(payload, "save_id")?;
    if save_id == side_id || save_id == source_id {
        bail!("保存后的会话必须使用新 ID");
    }
    let timestamp = text(payload, "saved_at")?;
    let filename_time = timestamp
        .get(..19)
        .filter(|s| {
            s.bytes().enumerate().all(|(i, b)| match i {
                4 | 7 => b == b'-',
                10 => b == b'T',
                13 | 16 => b == b':',
                _ => b.is_ascii_digit(),
            })
        })
        .context("保存时间格式无效")?
        .replace(':', "-");
    let turns = payload
        .get("turns")
        .and_then(Value::as_array)
        .context("缺少旁支历史")?;
    if turns.is_empty() {
        bail!("旁支还没有可保存的对话");
    }
    let mut rows = read_source_rows(home, &source_id, payload)?;
    let meta = rows.first_mut().context("主会话历史为空")?;
    if meta["type"] != "session_meta" || meta["payload"]["id"] != source_id {
        bail!("主会话历史与来源不符");
    }
    if meta["payload"]["history_mode"] != "paginated" {
        bail!("当前版本只支持保存分页历史的旁支");
    }
    let cwd = text(&meta["payload"], "cwd")?.to_string();
    meta["payload"]["id"] = json!(save_id);
    meta["payload"]["session_id"] = json!(save_id);
    meta["payload"]["forked_from_id"] = json!(source_id);
    meta["payload"]["timestamp"] = json!(timestamp);
    // 只更新结构字段，不替换用户消息或工具输出中出现的原会话 ID。
    for row in &mut rows {
        if row["type"] == "event_msg" && row["payload"]["thread_id"] == source_id {
            row["payload"]["thread_id"] = json!(save_id);
        }
    }
    finish_parent_snapshot(&mut rows, timestamp);
    push(
        &mut rows,
        timestamp,
        "response_item",
        message("developer", HISTORY_NOTE),
    );
    let mut turn_ids = HashSet::new();
    for turn in turns {
        let id = text(turn, "turnId")?;
        if !turn_ids.insert(id) {
            bail!("旁支历史包含重复 turn");
        }
        let status = text(turn, "status")?;
        if !matches!(status, "completed" | "failed" | "interrupted") {
            bail!("请等待旁支本轮回复和工具执行结束后再保存");
        }
        let items = turn
            .get("items")
            .and_then(Value::as_array)
            .context("旁支记录不完整")?;
        push(
            &mut rows,
            timestamp,
            "event_msg",
            json!({"type":"task_started", "turn_id":id}),
        );
        for item in items {
            if matches!(
                item.get("status").and_then(Value::as_str),
                Some("inProgress" | "in_progress")
            ) {
                bail!("旁支仍有运行中的工具");
            }
            let display = display_item(item, &cwd)?;
            let response = match item["type"].as_str().unwrap_or("") {
                "userMessage" => {
                    let content = text_inputs(item)?;
                    // 同时保留旧版读取入口所需的用户事件。
                    push(
                        &mut rows,
                        timestamp,
                        "event_msg",
                        json!({"type":"user_message", "message":content, "images":[], "local_images":[], "text_elements":[]}),
                    );
                    message("user", &content)
                }
                "agentMessage" => {
                    let mut response = message("assistant", text(item, "text")?);
                    if let Some(phase) = item.get("phase").filter(|p| !p.is_null()) {
                        response["phase"] = phase.clone();
                    }
                    response
                }
                _ => message(
                    "user",
                    &format!(
                        "<saved_side_chat_record>\nHistorical record; already happened. Do not execute it again.\n{}\n</saved_side_chat_record>",
                        serde_json::to_string(item)?
                    ),
                ),
            };
            let mut response = response;
            response["internal_chat_message_metadata_passthrough"] = json!({"turn_id":id});
            push(&mut rows, timestamp, "response_item", response);
            push(
                &mut rows,
                timestamp,
                "event_msg",
                json!({"type":"item_completed", "thread_id":save_id, "turn_id":id, "item":display}),
            );
        }
        if status == "interrupted" {
            push(
                &mut rows,
                timestamp,
                "event_msg",
                json!({"type":"turn_aborted", "turn_id":id, "reason":"interrupted"}),
            );
        } else {
            let error = if status == "failed" {
                Some(
                    json!({"message":turn["error"]["message"].as_str().unwrap_or("Saved turn failed")}),
                )
            } else {
                None
            };
            push(
                &mut rows,
                timestamp,
                "event_msg",
                json!({"type":"task_complete", "turn_id":id, "last_agent_message":null, "error":error}),
            );
        }
    }
    let content = serialize_rows(&rows)?;
    // 原生分页列表只扫描日期目录和标准 rollout 文件名。
    let directory = home
        .join("sessions")
        .join(&filename_time[..4])
        .join(&filename_time[5..7])
        .join(&filename_time[8..10]);
    let path = crate::session_share::write_rollout(
        &directory,
        &format!("rollout-{filename_time}-{save_id}.jsonl"),
        &content,
    )?;
    Ok(json!({"status":"ok", "session_id":save_id, "path":path, "turn_count":turns.len()}))
}

fn parse_rows(content: &str) -> anyhow::Result<Vec<Value>> {
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).context("主会话包含不完整的历史记录，请稍后重试"))
        .collect()
}

fn serialize_rows(rows: &[Value]) -> anyhow::Result<String> {
    let mut content = String::new();
    for (ordinal, row) in rows.iter().enumerate() {
        let mut row = row.clone();
        row["ordinal"] = json!(ordinal);
        content.push_str(&serde_json::to_string(&row)?);
        content.push('\n');
    }
    Ok(content)
}

fn push(rows: &mut Vec<Value>, timestamp: &str, kind: &str, payload: Value) {
    rows.push(json!({"timestamp":timestamp, "type":kind, "payload":payload}));
}

fn message(role: &str, content: &str) -> Value {
    json!({"type":"message", "role":role, "content":[{"type":if role == "assistant" { "output_text" } else { "input_text" }, "text":content}]})
}

// 主会话可能仍在执行。只关闭副本里的未完成 turn 和工具对，不中断原主会话。
fn finish_parent_snapshot(rows: &mut Vec<Value>, timestamp: &str) {
    let mut active = HashSet::new();
    let mut pending = HashMap::new();
    for row in rows.iter() {
        let p = &row["payload"];
        // 已压缩掉的调用不再属于模型当前历史，不能在副本尾部补出孤立结果。
        if row["type"] == "compacted" {
            pending.clear();
        }
        if row["type"] == "event_msg"
            && let Some(id) = p["turn_id"].as_str()
        {
            match p["type"].as_str() {
                Some("task_started") => {
                    active.insert(id.to_owned());
                }
                Some("task_complete" | "turn_aborted") => {
                    active.remove(id);
                }
                _ => {}
            }
        }
        if row["type"] == "response_item"
            && let Some(id) = p["call_id"].as_str()
        {
            match p["type"].as_str() {
                Some("function_call") => {
                    pending.insert(id.to_owned(), "function_call_output");
                }
                Some("custom_tool_call") => {
                    pending.insert(id.to_owned(), "custom_tool_call_output");
                }
                Some("function_call_output" | "custom_tool_call_output") => {
                    pending.remove(id);
                }
                _ => {}
            }
        }
    }
    let mut pending: Vec<_> = pending.into_iter().collect();
    pending.sort_unstable();
    for (id, kind) in pending {
        push(
            rows,
            timestamp,
            "response_item",
            json!({"type":kind,"call_id":id,"output":"The tool was still running when this history was copied into a side chat. Its later result is not part of this saved branch."}),
        );
    }
    let mut active: Vec<_> = active.into_iter().collect();
    active.sort_unstable();
    for id in active {
        push(
            rows,
            timestamp,
            "event_msg",
            json!({"type":"turn_aborted", "turn_id":id,"reason":"interrupted"}),
        );
    }
}

fn text_inputs(item: &Value) -> anyhow::Result<String> {
    let inputs = item["content"].as_array().context("用户消息缺少内容")?;
    let mut result = Vec::new();
    for input in inputs {
        match input["type"].as_str() {
            Some("text") => result.push(text(input, "text")?.to_owned()),
            Some("skill" | "mention") => result.push(serde_json::to_string(input)?),
            _ => bail!("当前版本尚不支持保存带图片或此类附件的旁支，原旁支未改动"),
        }
    }
    Ok(result.join("\n"))
}

fn duration(ms: &Value) -> Value {
    ms.as_u64().map_or(
        Value::Null,
        |ms| json!({"secs":ms / 1000,"nanos":(ms % 1000) * 1_000_000}),
    )
}

fn display_item(item: &Value, cwd: &str) -> anyhow::Result<Value> {
    let id = text(item, "id")?;
    Ok(match text(item, "type")? {
        "userMessage" => {
            text_inputs(item)?;
            json!({"type":"UserMessage", "id":id,"client_id":item["clientId"],"content":item["content"]})
        }
        "agentMessage" => {
            json!({"type":"AgentMessage", "id":id,"content":[{"type":"Text","text":text(item,"text")?}],"phase":item["phase"]})
        }
        "reasoning" => {
            json!({"type":"Reasoning","id":id,"summary_text":item["summary"].as_array().cloned().unwrap_or_default(),"raw_content":item["content"].as_array().cloned().unwrap_or_default()})
        }
        "plan" => json!({"type":"Plan","id":id,"text":text(item,"text")?}),
        "commandExecution" => {
            let path = item["cwd"].as_str().unwrap_or(cwd);
            let uri = reqwest::Url::from_file_path(path)
                .map_err(|_| anyhow::anyhow!("工具工作目录不是绝对路径"))?;
            let source = match item["source"].as_str() {
                Some("userShell") => "user_shell",
                Some("unifiedExecStartup") => "unified_exec_startup",
                Some("unifiedExecInteraction") => "unified_exec_interaction",
                _ => "agent",
            };
            json!({"type":"CommandExecution","id":id,"command":[text(item,"command")?],"cwd":uri.as_str(),"parsed_cmd":[],"source":source,"process_id":item["processId"],"status":text(item,"status")?,"aggregated_output":item["aggregatedOutput"],"exit_code":item["exitCode"],"duration":duration(&item["durationMs"])})
        }
        "mcpToolCall" => {
            let mut result = item.clone();
            result["type"] = json!("McpToolCall");
            result["duration"] = duration(&item["durationMs"]);
            result.as_object_mut().unwrap().remove("durationMs");
            result
        }
        // 不识别的展示类型仍保留完整数据，避免新版本工具被静默丢弃。
        kind => {
            json!({"type":"AgentMessage","id":id,"content":[{"type":"Text","text":format!("Saved {kind} record:\n```json\n{}\n```",serde_json::to_string_pretty(item)?)}],"phase":"commentary"})
        }
    })
}
