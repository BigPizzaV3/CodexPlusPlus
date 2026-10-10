// 压缩轮「前缀对齐」（「prompt 前缀稳定层」的组成部分，opt-in 同开关）。
//
// 背景（取证定案，见 /tmp/compact_prefix_align_heartbeat.log 与 COMPACT_PREFILL.md）：
// codex++ 客户端的自动压缩（auto compact）不走 /responses/compact，也不带
// compaction_trigger 控制项，而是把摘要指令原样追加在 input 末尾（user 角色）
// 并发送 `tools: []`。既有 compact 判定（路径 / compaction_trigger）对这一形态
// 双双落空：回填与改写全不触发，tools 由整块变空使 token 前缀在 chat 模板的
// tools 序列化区（第 0 块）分叉，压缩轮整段冷算（实测 216130 tokens @625tok/s
// ≈ 5.7 分钟），还把 tools=[] 毒化进 compaction_tools 的回填缓存。
//
// 本模块在 ChatCompletions 转换层入口对该请求体做「前缀对齐」：
//   1) 摘要指令必须唯一且落在消息序列末尾（user 角色）：system 区
//      （顶层 instructions / system|developer 输入项 / 首位 user 项）里的指令
//      实例搬到末尾，并携带客户端原始字节；
//   2) tools 缺失或为空时回填主会话最近一轮的 tools 字节（同 prompt_cache_key，
//      序列化逐字节一致，前缀纯追加）；
//   3) 除指令实例与 tools 外的一切字节不动；历史中段散落的指令实例（上一轮
//      压缩被中断的残留）属于既有前缀，绝不移动；
//   4) 压缩轮本体不得写 remember_tools 缓存（本模块检出即由调用方跳过记录）；
//   5) 响应封装不动：客户端把压缩摘要当普通回合收下并自行截断历史。
use crate::compaction_tools;
use serde_json::{json, Value};

/// 压缩摘要指令的首行签名：codex prompts/templates/compact/prompt.md 与 codex++
/// 客户端均以该句起头（客户端文本比 relay 的 COMPACTION_SUMMARY_INSTRUCTION 常
/// 量多一个行尾换行，故一律用签名前缀匹配并保留客户端原始字节，
/// 绝不做等值替换）。
pub const COMPACTION_INSTRUCTION_SIGNATURE: &str =
    "You are performing a CONTEXT CHECKPOINT COMPACTION";
/// 指令体的收尾句：用于在「指令混进大块文本」时圈定字节区域；找不到收尾句
/// 就不删（宁可保留分叉，也不腐蚀 system 正文）。
const COMPACTION_INSTRUCTION_TAIL: &str = "seamlessly continue the work.";
/// 补挂末尾指令时的兜底文本：仅当所有检出实例的原文都不可用时才会使用
//（正常路径都携带客户端原始字节）。与 protocol_proxy 的
/// COMPACTION_SUMMARY_INSTRUCTION 保持同文（openai/codex prompts/templates/compact/prompt.md）。
const FALLBACK_INSTRUCTION_TEXT: &str = "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.\n\nInclude:\n- Current progress and key decisions made\n- Important context, constraints, or user preferences\n- What remains to be done (clear next steps)\n- Any critical data, examples, or references needed to continue\n\nBe concise, structured, and focused on helping the next LLM seamlessly continue the work.";

/// 前缀对齐统计，供诊断事件 protocol_proxy.prefix_stabilizer_compaction_appended。
#[derive(Debug, Default, Clone, Copy)]
pub struct CompactionAlignReport {
    /// 检出的指令实例总数 = moved + 末尾原位保留（0/1）。
    pub instances: usize,
    /// 搬离原位的实例数（顶层 instructions / system|developer 项 / 首位 user 项）。
    pub moved: usize,
    /// 末尾补挂 user 指令条数（0/1）。
    pub appended: usize,
    /// 是否回填了主会话 tools 字节。
    pub tools_backfilled: bool,
    /// system 区混排文本里找到签名但圈不出完整区域：放弃删改（只回填 tools）。
    pub skipped_mixed: bool,
}

fn starts_signature(text: &str) -> bool {
    text.trim_start().starts_with(COMPACTION_INSTRUCTION_SIGNATURE)
}

/// 文本里是否存在「行首即签名」的行：这是指令实例的锚点特征。
/// 行中提及（如日志引用）不算，避免把普通轮误判成压缩轮。
fn has_line_anchored_signature(text: &str) -> bool {
    text.lines().any(|line| line.trim_start().starts_with(COMPACTION_INSTRUCTION_SIGNATURE))
}

fn find_newline(haystack: &[u8], from: usize) -> Option<usize> {
    haystack[from..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| offset + from)
}

/// 在一段文本里圈定指令的字节区域：从签名所在行的行首起，逐行向下扫到包含
/// 收尾句的那一行为止，并顺带吞掉紧随其后的一个换行，避免给 system 留空行。
/// 找不到签名或收尾句返回 None（放弃删改）。
fn find_instruction_span(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut line_start = 0_usize;
    loop {
        let line_end = find_newline(bytes, line_start).unwrap_or(bytes.len());
        if text[line_start..line_end]
            .trim_start()
            .starts_with(COMPACTION_INSTRUCTION_SIGNATURE)
        {
            let mut end = line_end;
            loop {
                if text[line_start..end].contains(COMPACTION_INSTRUCTION_TAIL) {
                    let end = if bytes.get(end) == Some(&b'\n') { end + 1 } else { end };
                    return Some((line_start, end));
                }
                if end >= bytes.len() {
                    return None;
                }
                end = find_newline(bytes, end + 1).unwrap_or(bytes.len());
            }
        }
        if line_end >= bytes.len() {
            return None;
        }
        line_start = line_end + 1;
    }
}

/// 对单个字符串执行指令切除。返回 (摘出的指令原文, 切除后的剩余文本, 是否
/// 「见签名但圈不出区域」)。剩余文本为 None 表示整段应删除。
fn excise_text(text: &str) -> (Option<String>, Option<String>, bool) {
    match find_instruction_span(text) {
        Some((start, end)) => {
            let removed = text[start..end].to_string();
            let rest = format!("{}{}", &text[..start], &text[end..]);
            (
                Some(removed),
                if rest.trim().is_empty() {
                    None
                } else {
                    Some(rest)
                },
                false,
            )
        }
        None => (
            None,
            Some(text.to_string()),
            has_line_anchored_signature(text),
        ),
    }
}

/// 对 content（字符串或分片数组）执行切除，返回 (切除后新值, 首个摘出文本,
/// 是否遇到无法圈定的混排签名, 是否有切除)。新值为 None 表示整段应删除。
fn excise_content(content: &Value) -> (Option<Value>, Option<String>, bool, bool) {
    let mut removed_first: Option<String> = None;
    let mut skipped = false;
    match content {
        Value::String(text) => {
            let (removed, rest, without_tail) = excise_text(text);
            if without_tail {
                skipped = true;
            }
            let cut = removed.is_some();
            (rest.map(|text| json!(text)), removed, skipped, cut)
        }
        Value::Array(parts) => {
            let mut kept: Vec<Value> = Vec::with_capacity(parts.len());
            for part in parts {
                let Some(text) = part.get("text").and_then(Value::as_str) else {
                    kept.push(part.clone());
                    continue;
                };
                let (removed, rest, without_tail) = excise_text(text);
                if without_tail {
                    skipped = true;
                    kept.push(part.clone());
                    continue;
                }
                let Some(removed) = removed else {
                    kept.push(part.clone());
                    continue;
                };
                removed_first.get_or_insert(removed);
                if let Some(rest) = rest {
                    let mut residue = part.clone();
                    residue["text"] = json!(rest);
                    kept.push(residue);
                }
            }
            let cut = removed_first.is_some();
            if kept.is_empty() {
                (None, removed_first, skipped, cut)
            } else {
                (Some(json!(kept)), removed_first, skipped, cut)
            }
        }
        _ => (Some(content.clone()), None, false, false),
    }
}

/// content 是否以指令签名起头（整条 content 就是指令实例的形态）。
fn content_starts_signature(content: Option<&Value>) -> bool {
    match content {
        Some(Value::String(text)) => starts_signature(text),
        Some(Value::Array(parts)) => parts
            .first()
            .and_then(|part| part.get("text").and_then(Value::as_str))
            .is_some_and(starts_signature),
        _ => false,
    }
}

fn content_may_contain_instruction(content: Option<&Value>) -> bool {
    match content {
        Some(Value::String(text)) => has_line_anchored_signature(text),
        Some(Value::Array(parts)) => parts.iter().any(|part| {
            part.get("text")
                .and_then(Value::as_str)
                .is_some_and(has_line_anchored_signature)
        }),
        _ => false,
    }
}

fn is_message_item(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("message")
}

fn item_role(item: &Value) -> &str {
    item.get("role").and_then(Value::as_str).unwrap_or("")
}

fn first_text_of(parts: &[Value]) -> String {
    parts
        .first()
        .and_then(|part| part.get("text").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

/// 整项即指令的消息项：返回指令原文（字符串 content 或分片数组单片且整片即指令）。
/// 分片数组要求恰有一片文本，混排项走 excise 通道，不在此列。
fn whole_item_instruction_text(item: &Value) -> Option<String> {
    match item.get("content") {
        Some(Value::String(text)) if starts_signature(text.trim()) => Some(text.clone()),
        Some(Value::Array(parts)) if parts.len() == 1 => {
            let text = parts[0].get("text").and_then(Value::as_str)?;
            starts_signature(text).then(|| text.to_string())
        }
        _ => None,
    }
}

fn tail_instruction_item(text: &str) -> Value {
    json!({
        "type": "message",
        "role": "user",
        "content": [{ "type": "input_text", "text": text }]
    })
}

/// 末尾项是否恰为一条 user 角色、整项即指令的消息。
fn tail_is_user_instruction(items: &[Value]) -> bool {
    items.last().is_some_and(|item| {
        is_message_item(item) && item_role(item) == "user"
            && whole_item_instruction_text(item).is_some()
    })
}

/// 压缩轮「前缀对齐」入口。未检出任何指令实例时返回 None（请求体原样放行，
/// 调用方照常走 remember_tools）。检出实例后：
/// - system 区（顶层 instructions / system|developer 输入项）与首位 user 项里的
///   指令实例搬离原位，历史其余项与字节一律不动；
/// - 保证末尾恰有一条 user 指令（缺则补挂，携带客户端原始字节）；
/// - tools 缺失或空数组时回填主会话最近一轮 tools 字节（逐字节一致）。
pub fn align_compaction_request(request: &Value) -> Option<(Value, CompactionAlignReport)> {
    let mut report = CompactionAlignReport::default();
    let mut aligned = request.clone();
    let mut moved_text: Option<String> = None;

    // 1) 顶层 instructions（字符串或分片数组）里混入的指令实例。
    if let Some(instructions) = aligned.get("instructions").cloned() {
        let (rebuilt, removed, skipped, cut) = excise_content(&instructions);
        if cut {
            let replacement = rebuilt.unwrap_or_else(|| {
                if instructions.is_string() {
                    json!("")
                } else {
                    json!([])
                }
            });
            aligned["instructions"] = replacement;
            if let Some(text) = removed {
                report.moved += 1;
                moved_text.get_or_insert(text);
            }
        }
        if skipped {
            report.skipped_mixed = true;
        }
    }

    // 2) input 项。
    if let Some(items) = aligned.get_mut("input").and_then(Value::as_array_mut) {
        if !items.is_empty() {
            // 2a) system|developer 输入项（含末尾）：整项即指令则整项摘除；
            //     混排文本按字节区域切除、其余字节原样保留。
            let mut cursor = 0_usize;
            while cursor < items.len() {
                let item = &items[cursor];
                if is_message_item(item)
                    && matches!(item_role(item), "system" | "developer")
                    && content_may_contain_instruction(item.get("content"))
                {
                    let content = item.get("content").cloned().unwrap_or(Value::Null);
                    let (rebuilt, removed, skipped, cut) = excise_content(&content);
                    if skipped {
                        report.skipped_mixed = true;
                    }
                    if cut {
                        report.moved += 1;
                        if let Some(text) = removed {
                            moved_text.get_or_insert(text);
                        }
                        match rebuilt {
                            Some(value) => items[cursor]["content"] = value,
                            None => {
                                items.remove(cursor);
                                continue;
                            }
                        }
                    }
                }
                cursor += 1;
            }

            // 2b) 首位 user 指令实例（「指令进消息头部」形态）：整项摘除，
            //     改由末尾补挂。历史中段（下标 ≥1）的实例属于既有前缀，不动。
            if items.len() >= 2 {
                let head_text = items
                    .first()
                    .filter(|item| is_message_item(item) && item_role(item) == "user")
                    .and_then(whole_item_instruction_text);
                if let Some(text) = head_text {
                    items.remove(0);
                    moved_text.get_or_insert(text);
                    report.moved += 1;
                }
            }

            // 2c) 末尾保证恰有一条 user 指令：原位已有则字节不动；有搬移且末尾
            //     缺失时补挂一条（优先客户端原始字节，其次 relay 兜底文本）。
            let tail_present = tail_is_user_instruction(items);
            if report.moved > 0 && !tail_present {
                let text = moved_text
                    .clone()
                    .unwrap_or_else(|| FALLBACK_INSTRUCTION_TEXT.to_string());
                items.push(tail_instruction_item(&text));
                report.appended = 1;
            }
            if tail_is_user_instruction(items) {
                report.instances += 1;
            }
        }
    }

    if report.moved == 0 && report.instances == 0 && !report.skipped_mixed {
        return None;
    }
    report.instances += report.moved;

    // 3) tools 缺失或空数组：回填主会话最近一轮 tools 字节（逐字节一致）。
    let tools_empty = match aligned.get("tools") {
        None => true,
        Some(Value::Array(tools)) => tools.is_empty(),
        Some(_) => false,
    };
    if tools_empty {
        if let Some(tools) = compaction_tools::preserved_tools(request) {
            aligned["tools"] = tools;
            report.tools_backfilled = true;
        }
    }

    Some((aligned, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction_text() -> String {
        // 与 codex++ 客户端语料（/tmp/compact_forensics seq=3）一致：含行尾换行。
        format!("{FALLBACK_INSTRUCTION_TEXT}\n")
    }

    fn base_tools() -> Value {
        json!([{"type": "function", "name": "exec_command", "parameters": {"type": "object"}}])
    }

    fn message(role: &str, text: &str) -> Value {
        json!({ "type": "message", "role": role, "content": [{ "type": "input_text", "text": text }] })
    }

    fn remember(key: &str) {
        compaction_tools::remember_tools(
            &json!({"prompt_cache_key": key, "tools": base_tools()}),
        );
    }

    #[test]
    fn tail_user_instruction_backfills_tools_without_touching_history() {
        // 现网 514-msg 形态：指令在末尾 user，tools 为空数组。
        let key = "align-thread-1";
        remember(key);
        let mut compaction = json!({
            "prompt_cache_key": key,
            "tools": base_tools(),
            "input": [message("developer", "base system"), message("user", "hi")],
        });
        compaction["tools"] = json!([]);
        compaction["input"]
            .as_array_mut()
            .unwrap()
            .push(message("user", &instruction_text()));
        let history_before = compaction["input"].clone();
        let (aligned, report) = align_compaction_request(&compaction).expect("必须检出");
        assert_eq!(report.moved, 0, "末尾原位不搬运");
        assert_eq!(report.appended, 0);
        assert_eq!(report.instances, 1);
        assert!(report.tools_backfilled);
        // 历史逐项逐字节一致，仅 tools 被回填。
        assert_eq!(aligned["input"], history_before);
        assert_eq!(
            serde_json::to_string(&aligned["tools"]).unwrap(),
            serde_json::to_string(&base_tools()).unwrap()
        );
    }

    #[test]
    fn instruction_in_system_head_is_moved_to_tail() {
        // 防御形态：指令整条塞在头部 system 项，指令以外字节必须逐字节保留。
        let key = "align-thread-2";
        remember(key);
        let normal = json!({
            "prompt_cache_key": key,
            "tools": base_tools(),
            "input": [message("system", "主会话系统块"), message("user", "历史消息")],
        });
        let mut compaction = normal.clone();
        compaction["tools"] = json!([]);
        compaction["input"]
            .as_array_mut()
            .unwrap()
            .insert(0, message("system", &instruction_text()));
        let (aligned, report) = align_compaction_request(&compaction).expect("必须检出");
        assert_eq!(report.moved, 1);
        assert_eq!(report.appended, 1);
        assert!(report.tools_backfilled);
        let items = aligned["input"].as_array().unwrap();
        assert_eq!(items.len(), 3, "摘除头部指令项，末尾补挂一条");
        assert_eq!(items[0], normal["input"].as_array().unwrap()[0]);
        assert_eq!(items[1], normal["input"].as_array().unwrap()[1]);
        assert_eq!(item_role(&items[2]), "user");
        assert_eq!(
            whole_item_instruction_text(&items[2]).unwrap(),
            instruction_text(),
            "补挂指令必须携带客户端原始字节"
        );
    }

    #[test]
    fn instruction_mixed_into_developer_block_is_cut_byte_exact() {
        // 指令混排进 developer 大块文本：切除区域外字节必须原样。
        let key = "align-thread-3";
        let block = format!("前段固定文本\n{}后段固定文本\n", instruction_text());
        remember(key);
        let request = json!({
            "prompt_cache_key": key,
            "tools": [],
            "input": [message("developer", &block), message("user", "hi")],
        });
        let (aligned, report) = align_compaction_request(&request).expect("必须检出");
        assert_eq!(report.moved, 1);
        assert_eq!(report.appended, 1);
        let cut = first_text_of(aligned["input"][0]["content"].as_array().unwrap());
        assert_eq!(cut, "前段固定文本\n后段固定文本\n", "切除后 system 正文必须逐字节还原");
        assert_eq!(item_role(&aligned["input"][1]), "user");
        assert_eq!(
            whole_item_instruction_text(aligned["input"].as_array().unwrap().last().unwrap())
                .unwrap(),
            instruction_text()
        );
        assert!(report.tools_backfilled);
    }

    #[test]
    fn instruction_without_tail_sentence_is_conservatively_skipped() {
        // 混排文本只见签名不见收尾句：不动 system 正文，但 tools 回填照做。
        let key = "align-thread-4";
        let block = "You are performing a CONTEXT CHECKPOINT COMPACTION 的残缺引用\n其余正文";
        remember(key);
        let request = json!({
            "prompt_cache_key": key,
            "tools": [],
            "input": [message("developer", block), message("user", "hi")],
        });
        let (aligned, report) = align_compaction_request(&request).expect("必须检出");
        assert!(report.skipped_mixed);
        assert_eq!(report.moved, 0, "无法圈定区域时不删");
        assert!(report.tools_backfilled);
        assert_eq!(aligned["input"][0]["content"], request["input"][0]["content"]);
    }

    #[test]
    fn mid_history_instruction_is_untouched_and_not_detected() {
        // 上一轮压缩中断后残留的历史中段实例（early-rewrite 形态）：绝不移动，
        // 该轮按普通轮放行。
        let request = json!({
            "prompt_cache_key": "align-thread-5",
            "tools": base_tools(),
            "input": [
                message("developer", "系统块"),
                message("user", &instruction_text()),
                message("assistant", "被打断的回答"),
                message("user", "新的普通消息"),
            ],
        });
        assert!(align_compaction_request(&request).is_none());
    }

    #[test]
    fn system_role_tail_instruction_is_moved_to_user_tail() {
        // 指令以 system 角色收尾：整项摘除，末尾补挂 user 项，指令字节原样。
        let key = "align-thread-6";
        remember(key);
        let request = json!({
            "prompt_cache_key": key,
            "tools": [],
            "input": [message("user", "历史"), message("system", &instruction_text())],
        });
        let (aligned, report) = align_compaction_request(&request).expect("必须检出");
        assert_eq!(report.moved, 1);
        assert_eq!(report.appended, 1);
        let items = aligned["input"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], request["input"].as_array().unwrap()[0]);
        assert_eq!(item_role(&items[1]), "user");
        assert_eq!(
            whole_item_instruction_text(&items[1]).unwrap(),
            instruction_text()
        );
        assert!(report.tools_backfilled);
    }

    #[test]
    fn head_user_instruction_instance_is_moved_to_tail() {
        // 「指令在消息序列头部」形态（首位 user 整项即指令）：搬到末尾。
        let key = "align-thread-9";
        remember(key);
        let request = json!({
            "prompt_cache_key": key,
            "tools": [],
            "input": [message("user", &instruction_text()), message("user", "历史消息")],
        });
        let (aligned, report) = align_compaction_request(&request).expect("必须检出");
        assert_eq!(report.moved, 1);
        assert_eq!(report.appended, 1);
        let items = aligned["input"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], request["input"].as_array().unwrap()[1]);
        assert_eq!(
            whole_item_instruction_text(&items[1]).unwrap(),
            instruction_text()
        );
        assert!(report.tools_backfilled);
    }

    #[test]
    fn compaction_round_detection_guards_remember_tools() {
        // 压缩轮本体（检出 Some）与普通轮（None）的分界：调用方据此决定
        // 跳过/执行 remember_tools，杜绝 [] 毒化回填缓存。
        let compaction = json!({
            "prompt_cache_key": "align-thread-7",
            "tools": [],
            "input": [message("user", "历史"), message("user", &instruction_text())],
        });
        assert!(align_compaction_request(&compaction).is_some());
        let normal = json!({
            "prompt_cache_key": "align-thread-7",
            "tools": base_tools(),
            "input": [message("user", "普通轮")],
        });
        assert!(align_compaction_request(&normal).is_none());
    }

    #[test]
    fn no_cached_tools_keeps_legacy_empty_tools_behavior() {
        let request = json!({
            "prompt_cache_key": "align-thread-never-seen",
            "tools": [],
            "input": [message("user", "历史"), message("user", &instruction_text())],
        });
        let (aligned, report) = align_compaction_request(&request).expect("必须检出");
        assert!(!report.tools_backfilled);
        assert_eq!(aligned["tools"], json!([]));
        assert_eq!(report.instances, 1);
    }

    #[test]
    fn top_level_instructions_containment_is_cut() {
        // 指令混进顶层 instructions：切除后其余字节保留，指令补到消息末尾。
        let key = "align-thread-8";
        remember(key);
        let request = json!({
            "prompt_cache_key": key,
            "tools": [],
            "instructions": format!("基础指令头\n{}", instruction_text()),
            "input": [message("user", "hi")],
        });
        let (aligned, report) = align_compaction_request(&request).expect("必须检出");
        assert_eq!(report.moved, 1);
        assert_eq!(report.appended, 1);
        assert_eq!(aligned["instructions"], json!("基础指令头\n"));
        let last = aligned["input"].as_array().unwrap().last().unwrap();
        assert_eq!(item_role(last), "user");
        assert_eq!(
            whole_item_instruction_text(last).unwrap(),
            instruction_text()
        );
        assert!(report.tools_backfilled);
    }
}
