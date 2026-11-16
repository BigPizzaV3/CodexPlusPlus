//! 加加的提示词与回复格式。
//!
//! 基础提示词整个替换掉 Codex 的「编码助手」提示词（`thread/start` 的 `baseInstructions`）；
//! 记忆、盯着的会话、最近聊天这些会变的东西放在 `developerInstructions`，每次重开会话时重新生成。

use super::events::{clip, short_id};
use super::store::{ChatMessage, Later, Watched};

pub const SILENT: &str = "[静默]";

pub const BASE_INSTRUCTIONS: &str = r#"你是「加加」，Codex++ 里的一个小助理。用户在手机上和你聊天；你替他照看这台电脑上 Codex 里的各个编码会话（Codex 是一个编码智能体，每个会话是它在某个项目里干的一件活）。

# 你能做的事（都用 jiajia 工具，不要用命令行或读写文件）
- 看会话：list_threads 列出会话及状态；read_thread 读某个会话最近在干什么；list_projects 列出项目目录。
- 管会话：send_to_thread 以用户的身份给会话发消息（会话正在跑就是插话）；new_thread 在某个项目新开会话并发出第一条消息；stop_thread 停掉正在跑的一轮；set_thread_settings 改会话的模型 / 推理强度（list_models 查可选模型）。
- 照看：watch 盯住或放开一个会话（盯着的会话跑完会告诉你）；recent_events 查最近的会话动静；later 约一个时间回来看看（比如「20 分钟后看看测试跑完没」）。
- 记事：remember 把用户的偏好、习惯、正在推进的事记下来，下次还记得。

# 规矩
- 只说做了的事。工具没成功就照实说，不编造结果。不确定会话在干什么就先 read_thread。
- 给会话发消息、新开会话：用户明确要你做，或者话里意思很清楚时才做。发出去的内容要完整、具体，Codex 看不到你和用户的聊天。
- 停止会话、改模型这类会打断别人干活的事，用户没明说就先问一句。
- 你不能替用户审批（运行命令、改文件、回答 Codex 的提问）。有会话等审批时，告诉用户是哪个会话、要做什么，请他在卡片或会话里自己点。
- 用户要的是「帮我盯着」时，用 watch 或 later，不要口头答应了却不安排。

# 说话方式
- 默认像微信聊天：简短、口语、中文，一般一两句话。列表最多五六行。不用标题，少用加粗。用户在设置里给你定了性格的话，语气按性格来。
- 提到某个会话时，在句末附上会话卡片标记 [[会话:会话ID前8位]]，用户点卡片就能进去。一条回复最多附三张卡片。
- 会话 ID 平时用前 8 位就行，工具都认。

# 会话动静
- 用户消息之外，你会收到 <事件> 包着的会话动静（等审批、出错、跑完），这是 Codex++ 自动发的，不是用户说的话。
- 值得告诉用户的就用一两句话转告（等审批、出错一般都要说；跑完的说结果要点）；不值得说的，整条回复只写 [静默]。
- <回访> 包着的是你之前用 later 约好的提醒，到点了，去看看再告诉用户。
- 深夜（23 点到 7 点）没有急事就 [静默]，等用户来找你再说。
"#;

/// 每次开加加会话时的近况：时间、记忆、盯着的会话、回访、最近聊天。
pub fn developer_instructions(
    now_text: &str,
    host: &str,
    persona: &str,
    user_memory: &str,
    notes: &str,
    watched: &[Watched],
    later: &[Later],
    recent: &[ChatMessage],
) -> String {
    let mut text = format!("现在是 {now_text}。用户的电脑叫「{host}」。\n");
    let section = |title: &str, body: &str| -> String {
        let body = body.trim();
        if body.is_empty() { format!("\n## {title}\n（空）\n") } else { format!("\n## {title}\n{body}\n") }
    };
    if !persona.trim().is_empty() {
        text.push_str(&format!(
            "\n## 你的性格（用户在设置里给你定的）\n{}\n照这个性格说话，而且要放开演：每条回复的语气、用词、口头禅都带出这个性格，让用户一眼认出是你，\
             不要退回平淡的汇报腔。它比默认的说话方式优先。\
             但「规矩」那几条不变：内容照实说、不编造，不替用户审批，打断别人干活前先问。\n",
            persona.trim()
        ));
    }
    text.push_str(&section("关于用户（你记下的，USER）", user_memory));
    text.push_str(&section("你的笔记（NOTES）", notes));
    let watched_lines: Vec<String> = watched
        .iter()
        .map(|watched| {
            let reason = if watched.reason.is_empty() { String::new() } else { format!("（{}）", watched.reason) };
            format!("- {} {}{reason}", short_id(&watched.thread_id), if watched.title.is_empty() { "未命名" } else { &watched.title })
        })
        .collect();
    text.push_str(&section("你盯着的会话", &watched_lines.join("\n")));
    let later_lines: Vec<String> = later.iter().map(|item| format!("- #{} {} {}", item.id, format_clock(item.due_ms), item.note)).collect();
    text.push_str(&section("你约好的回访", &later_lines.join("\n")));
    let chat_lines: Vec<String> = recent
        .iter()
        .filter(|message| matches!(message.role.as_str(), "user" | "bot" | "notice"))
        .map(|message| {
            let who = match message.role.as_str() {
                "user" => "用户",
                "notice" => "提醒",
                _ => "你",
            };
            format!("[{}] {who}：{}", format_clock(message.at), clip(&message.text, 200))
        })
        .collect();
    text.push_str(&section("最近的聊天（你这个会话是新开的，之前聊过这些）", &chat_lines.join("\n")));
    text
}

/// 加加的一条回复：去掉卡片标记后的文字 + 提到的会话（原样，可能是前缀）。`None` 表示这次不说话。
pub fn parse_reply(text: &str) -> Option<(String, Vec<String>)> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == SILENT || (trimmed.starts_with(SILENT) && trimmed[SILENT.len()..].trim().is_empty()) {
        return None;
    }
    let mut threads = Vec::new();
    let mut out = String::new();
    let mut rest = trimmed;
    while let Some(start) = rest.find("[[会话:") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "[[会话:".len()..];
        match after.find("]]") {
            Some(end) => {
                let id = after[..end].trim().to_string();
                if !id.is_empty() && !threads.contains(&id) && threads.len() < 3 {
                    threads.push(id);
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    let text = out.replace(SILENT, "").trim().to_string();
    if text.is_empty() && threads.is_empty() {
        return None;
    }
    Some((text, threads))
}

/// 本地时间格式化：借 SQLite 的 `localtime`（core 没有时区库，仓库里别处也是这么取本地日期的）。
fn local_time(ms: i64, format: &str) -> String {
    rusqlite::Connection::open_in_memory()
        .and_then(|db| db.query_row("SELECT strftime(?1, ?2 / 1000, 'unixepoch', 'localtime')", rusqlite::params![format, ms], |row| row.get::<_, String>(0)))
        .unwrap_or_default()
}

/// 毫秒时间戳 → 本地「11-17 02:30」。
pub fn format_clock(ms: i64) -> String {
    local_time(ms, "%m-%d %H:%M")
}

/// 毫秒时间戳 → 「2026-11-17 02:30 周二」。
pub fn format_now(ms: i64) -> String {
    let weekday = local_time(ms, "%w").parse::<usize>().ok().and_then(|day| ["日", "一", "二", "三", "四", "五", "六"].get(day).copied()).unwrap_or("");
    format!("{} 周{weekday}", local_time(ms, "%Y-%m-%d %H:%M"))
}

/// 本地小时（0–23），判断深夜用。
pub fn local_hour(ms: i64) -> u32 {
    local_time(ms, "%H").parse().unwrap_or(12)
}
