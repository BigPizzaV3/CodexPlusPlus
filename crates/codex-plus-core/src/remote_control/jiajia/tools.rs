//! 加加的工具：定义（给 MCP 的 `tools/list`）与执行（MCP 进程转回来的 `/api/jiajia/tool`）。
//!
//! 都是远程控制已有能力的包装：列会话、读会话、发消息 / 插话、新建、停止、改设置。
//! 输出是给模型读的紧凑中文文本；会话 ID 认完整 UUID 或前缀（列表里给前 8 位，省 token）。

use anyhow::{Context, bail};
use serde_json::{Value, json};

use super::Jiajia;
use super::events::{Phase, clip, project_name, request_summary, short_id};
use super::store::{self, LATER_LIMIT, MEMORY_LIMIT, now_ms};
use crate::remote_control::devices::Role;

/// MCP `tools/list` 的工具定义。
pub fn tool_definitions() -> Value {
    let thread = json!({ "type": "string", "description": "会话 ID（完整或前 8 位）" });
    json!([
        {
            "name": "list_threads",
            "description": "列出用户电脑上 Codex 的会话（最近更新的在前）：ID 前 8 位 | 标题 | 项目 | 状态 | 更新时间。",
            "inputSchema": { "type": "object", "properties": {
                "filter": { "type": "string", "enum": ["recent", "running", "waiting"], "description": "recent 最近的（默认）；running 运行中或等你处理的；waiting 只看等审批 / 提问的" },
                "project": { "type": "string", "description": "只看某个项目（目录名或路径的一部分）" },
                "limit": { "type": "integer", "description": "最多几条，默认 15" }
            } }
        },
        {
            "name": "read_thread",
            "description": "读一个会话最近的内容：用户说了什么、Codex 回了什么、跑了哪些命令、改了哪些文件、有没有出错或等审批。",
            "inputSchema": { "type": "object", "properties": { "thread": thread, "count": { "type": "integer", "description": "最近几条，默认 12" } }, "required": ["thread"] }
        },
        {
            "name": "send_to_thread",
            "description": "以用户的身份给会话发一条消息；会话正在跑就作为插话。发完会自动盯着这个会话。",
            "inputSchema": { "type": "object", "properties": { "thread": thread, "text": { "type": "string", "description": "要发的内容，写完整、具体" } }, "required": ["thread", "text"] }
        },
        {
            "name": "new_thread",
            "description": "在某个项目目录新开一个 Codex 会话并发出第一条消息（在后台跑，不打断桌面）。发完会自动盯着它。",
            "inputSchema": { "type": "object", "properties": {
                "project": { "type": "string", "description": "项目目录：绝对路径，或 list_projects 里的项目名" },
                "text": { "type": "string", "description": "第一条消息，写清楚要做什么" },
                "title": { "type": "string", "description": "会话标题，可省略" }
            }, "required": ["project", "text"] }
        },
        {
            "name": "stop_thread",
            "description": "停止会话正在跑的这一轮。",
            "inputSchema": { "type": "object", "properties": { "thread": thread }, "required": ["thread"] }
        },
        {
            "name": "list_projects",
            "description": "列出用户最近用过的项目目录（会话数、最近更新）。",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "list_models",
            "description": "列出可选的模型和推理强度。",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "set_thread_settings",
            "description": "改会话下一轮起用的模型或推理强度。需要用户的设备有「完全控制」权限。",
            "inputSchema": { "type": "object", "properties": {
                "thread": thread,
                "model": { "type": "string", "description": "模型 ID（list_models 里的）" },
                "effort": { "type": "string", "description": "推理强度，如 low / medium / high" }
            }, "required": ["thread"] }
        },
        {
            "name": "watch",
            "description": "盯住或放开一个会话：盯着的会话跑完、出错会告诉你。list 列出正在盯的。",
            "inputSchema": { "type": "object", "properties": {
                "action": { "type": "string", "enum": ["add", "remove", "list"] },
                "thread": thread,
                "reason": { "type": "string", "description": "为什么盯（自己备忘）" }
            }, "required": ["action"] }
        },
        {
            "name": "recent_events",
            "description": "最近 24 小时的会话动静（等审批、出错、跑完），包括没有告诉你的。",
            "inputSchema": { "type": "object", "properties": { "limit": { "type": "integer", "description": "默认 10" } } }
        },
        {
            "name": "later",
            "description": "约一个时间回来看看：到点你会收到 <回访>。最多同时 5 个。",
            "inputSchema": { "type": "object", "properties": {
                "action": { "type": "string", "enum": ["add", "cancel", "list"] },
                "minutes": { "type": "number", "description": "多少分钟后（add 用），1 到 10080" },
                "note": { "type": "string", "description": "到点要做什么（add 用）" },
                "id": { "type": "integer", "description": "回访编号（cancel 用）" }
            }, "required": ["action"] }
        },
        {
            "name": "remember",
            "description": "记下长期有用的事：about=user 记用户的偏好、习惯、身份；about=notes 记你自己的备忘（正在推进的事、约定）。append 追加一行；replace 整份重写（超长时用它整理精简）。",
            "inputSchema": { "type": "object", "properties": {
                "about": { "type": "string", "enum": ["user", "notes"] },
                "mode": { "type": "string", "enum": ["append", "replace"] },
                "text": { "type": "string" }
            }, "required": ["about", "text"] }
        }
    ])
}

/// 工具 → 网页上「加加正在…」。
pub fn activity_label(tool: &str) -> &'static str {
    match tool {
        "list_threads" | "list_projects" => "正在看会话列表",
        "read_thread" => "正在看会话内容",
        "send_to_thread" => "正在给会话发消息",
        "new_thread" => "正在新开会话",
        "stop_thread" => "正在停止会话",
        "set_thread_settings" | "list_models" => "正在改会话设置",
        "remember" => "正在记下来",
        "later" => "正在安排回访",
        "watch" => "正在安排盯着",
        "recent_events" => "正在翻最近的动静",
        _ => "正在处理",
    }
}

/// 「3 分钟前」。
pub fn relative_time(seconds: i64, now_secs: i64) -> String {
    let delta = (now_secs - seconds).max(0);
    match delta {
        0..=59 => "刚刚".into(),
        60..=3599 => format!("{} 分钟前", delta / 60),
        3600..=86_399 => format!("{} 小时前", delta / 3600),
        _ => format!("{} 天前", delta / 86_400),
    }
}

/// 会话视图压成给模型读的几行。
pub fn summarize_view(view: &Value, count: usize) -> String {
    let title = view.get("title").and_then(Value::as_str).unwrap_or("未命名");
    let cwd = view.get("cwd").and_then(Value::as_str).unwrap_or_default();
    let model = view.pointer("/controls/model").or_else(|| view.get("model")).and_then(Value::as_str).unwrap_or_default();
    let runtime = view.get("runtime").and_then(Value::as_str).unwrap_or_default();
    let requests = view.get("requests").and_then(Value::as_array).cloned().unwrap_or_default();
    let state = if !requests.is_empty() {
        "等你处理"
    } else if runtime == "active" {
        "运行中"
    } else {
        "空闲"
    };
    let mut lines = vec![format!("「{title}」 项目 {} | {state}{}", if cwd.is_empty() { "?" } else { cwd }, if model.is_empty() { String::new() } else { format!(" | 模型 {model}") })];
    let mut entries: Vec<String> = Vec::new();
    for turn in view.get("turns").and_then(Value::as_array).into_iter().flatten() {
        for item in turn.get("items").and_then(Value::as_array).into_iter().flatten() {
            let text = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or_default();
            let line = match item.get("type").and_then(Value::as_str).unwrap_or_default() {
                "userMessage" => format!("用户：{}", clip(text("text"), 300)),
                "steeringUserMessage" => format!("用户插话：{}", clip(text("text"), 300)),
                "agentMessage" if !text("text").is_empty() => format!("Codex：{}", clip(text("text"), 400)),
                "commandExecution" => {
                    let command = match item.get("command") {
                        Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
                        Some(Value::String(command)) => command.clone(),
                        _ => String::new(),
                    };
                    format!("[命令] `{}` {}", clip(&command, 120), status_word(text("status")))
                }
                "fileChange" => {
                    let files: Vec<String> = item
                        .get("changes")
                        .and_then(Value::as_array)
                        .map(|changes| changes.iter().filter_map(|change| change.get("path").and_then(Value::as_str)).map(|path| project_name(path)).collect())
                        .unwrap_or_default();
                    // 带上状态：被拒绝的改动不能读成「改了」（实测加加把 declined 的改动说成改好了）。
                    let status = match text("status") {
                        "declined" => "（被拒绝，没改）",
                        "failed" => "（失败）",
                        "inProgress" => "（进行中）",
                        _ => "",
                    };
                    format!("[改文件] {}{status}", clip(&files.join(", "), 160))
                }
                "webSearch" => format!("[搜索] {}", clip(text("query"), 80)),
                "mcpToolCall" => format!("[工具] {}", text("tool")),
                _ => continue,
            };
            entries.push(line);
        }
        if let Some(message) = turn.pointer("/error/message").and_then(Value::as_str) {
            entries.push(format!("[这一轮失败] {}", clip(message, 200)));
        } else if turn.get("status").and_then(Value::as_str) == Some("interrupted") {
            entries.push("[这一轮被停止]".into());
        }
    }
    let skip = entries.len().saturating_sub(count.max(1));
    if skip > 0 {
        lines.push(format!("（更早的 {skip} 条省略）"));
    }
    lines.extend(entries.into_iter().skip(skip));
    for request in &requests {
        lines.push(format!("[等你处理] {}", request_summary(request)));
    }
    lines.join("\n")
}

fn status_word(status: &str) -> &'static str {
    match status {
        "completed" => "完成",
        "failed" => "失败",
        "declined" => "被拒绝",
        "inProgress" => "运行中",
        _ => "",
    }
}

fn text_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty())
}

impl Jiajia {
    /// 执行一个工具，返回给模型读的文本。
    pub async fn call_tool(&self, name: &str, args: &Value) -> anyhow::Result<String> {
        let service = self.service()?;
        match name {
            "list_threads" => self.tool_list_threads(args).await,
            "read_thread" => {
                let (id, _) = self.resolve_thread(text_arg(args, "thread").context("缺少 thread")?).await?;
                let count = args.get("count").and_then(Value::as_u64).unwrap_or(12).clamp(1, 60) as usize;
                let view = service.thread_view(&id).await?;
                self.note_seen(&id, &view);
                Ok(format!("会话 {}\n{}", short_id(&id), summarize_view(&view, count)))
            }
            "send_to_thread" => {
                self.require_role(Role::Message)?;
                let (id, title) = self.resolve_thread(text_arg(args, "thread").context("缺少 thread")?).await?;
                let text = text_arg(args, "text").context("缺少 text")?;
                let result = service.send(&id, text).await?;
                self.store().watch(&id, &title, "加加发过消息");
                let how = if result.get("kind").and_then(Value::as_str) == Some("steer") { "已插话" } else { "已发出，会话开始跑了" };
                Ok(format!("{how}（会话 {}「{title}」）。已自动盯着，跑完会告诉你。", short_id(&id)))
            }
            "new_thread" => {
                self.require_role(Role::Message)?;
                let project = text_arg(args, "project").context("缺少 project")?;
                let text = text_arg(args, "text").context("缺少 text")?;
                let cwd = self.resolve_project(project).await?;
                let title = text_arg(args, "title").map(str::to_string).unwrap_or_else(|| clip(text, 20));
                let id = service.create_thread(&cwd, &title).await?;
                service.send(&id, text).await?;
                self.store().watch(&id, &title, "加加新开的");
                self.touch();
                Ok(format!("已在 {} 新开会话 {}「{title}」并发出第一条消息，正在后台跑。已自动盯着。", project_name(&cwd), short_id(&id)))
            }
            "stop_thread" => {
                self.require_role(Role::Message)?;
                let (id, title) = self.resolve_thread(text_arg(args, "thread").context("缺少 thread")?).await?;
                service.interrupt(&id).await?;
                Ok(format!("已停止会话 {}「{title}」这一轮。", short_id(&id)))
            }
            "list_projects" => self.tool_list_projects().await,
            "list_models" => {
                let models = service.models().await?;
                let lines: Vec<String> = models
                    .iter()
                    .map(|model| {
                        let efforts = model.get("efforts").and_then(Value::as_array).map(|efforts| efforts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("/")).unwrap_or_default();
                        let default = model.get("defaultEffort").and_then(Value::as_str).map(|effort| format!("，默认 {effort}")).unwrap_or_default();
                        format!(
                            "- {} — {}{}",
                            model.get("id").and_then(Value::as_str).unwrap_or_default(),
                            model.get("label").and_then(Value::as_str).unwrap_or_default(),
                            if efforts.is_empty() { String::new() } else { format!("（推理强度 {efforts}{default}）") }
                        )
                    })
                    .collect();
                Ok(if lines.is_empty() { "没有读到可选模型".into() } else { lines.join("\n") })
            }
            "set_thread_settings" => {
                self.require_role(Role::Full)?;
                let (id, title) = self.resolve_thread(text_arg(args, "thread").context("缺少 thread")?).await?;
                let mut change = json!({});
                if let Some(model) = text_arg(args, "model") {
                    change["model"] = json!(model);
                }
                if let Some(effort) = text_arg(args, "effort") {
                    change["effort"] = json!(effort);
                }
                if change.as_object().is_some_and(|change| change.is_empty()) {
                    bail!("没有要改的：给 model 或 effort");
                }
                service.update_settings(&id, &change).await?;
                Ok(format!("已改会话 {}「{title}」的设置，下一轮起生效。", short_id(&id)))
            }
            "watch" => self.tool_watch(args).await,
            "recent_events" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(10).clamp(1, 50) as usize;
                let store = self.store();
                let events: Vec<String> = store
                    .events
                    .iter()
                    .rev()
                    .take(limit)
                    .map(|event| {
                        let kind = match event.kind.as_str() {
                            "waiting" => "等你处理",
                            "failed" => "出错",
                            _ => "跑完",
                        };
                        format!("[{}] {kind}「{}」({}) {}", super::prompt::format_clock(event.at), event.title, short_id(&event.thread_id), event.detail)
                    })
                    .collect();
                Ok(if events.is_empty() { "最近 24 小时没有动静".into() } else { events.join("\n") })
            }
            "later" => self.tool_later(args),
            "remember" => self.tool_remember(args),
            other => bail!("没有这个工具：{other}"),
        }
    }

    fn require_role(&self, role: Role) -> anyhow::Result<()> {
        if self.sender_role() < role {
            match role {
                Role::Full => bail!("和你说话的这台设备没有「完全控制」权限，不能做这件事；请用户在 Codex++ 的设备列表里调高权限，或在电脑上操作"),
                _ => bail!("和你说话的这台设备只能查看，不能做这件事"),
            }
        }
        Ok(())
    }

    async fn tool_list_threads(&self, args: &Value) -> anyhow::Result<String> {
        let service = self.service()?;
        let list = service.list_threads(None).await?;
        let filter = text_arg(args, "filter").unwrap_or("recent");
        let project = text_arg(args, "project").map(str::to_lowercase);
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(15).clamp(1, 50) as usize;
        let now_secs = now_ms() / 1000;
        let mut lines = Vec::new();
        for thread in list.get("threads").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = thread.get("id").and_then(Value::as_str) else { continue };
            let cwd = thread.get("cwd").and_then(Value::as_str).unwrap_or_default();
            if project.as_ref().is_some_and(|project| !cwd.to_lowercase().contains(project.as_str())) {
                continue;
            }
            let known = self.known_phase(id).await;
            let listed = thread.get("status").and_then(Value::as_str).unwrap_or_default();
            let (state, busy, waiting) = match known {
                Some((Phase::Waiting, _)) => ("等你处理", true, true),
                Some((Phase::Running, _)) => ("运行中", true, false),
                Some((Phase::Idle, Some(end))) if end == "failed" => ("上一轮出错", false, false),
                _ if listed == "active" => ("运行中", true, false),
                _ => ("空闲", false, false),
            };
            if (filter == "running" && !busy) || (filter == "waiting" && !waiting) {
                continue;
            }
            let title = thread.get("title").and_then(Value::as_str).unwrap_or("未命名");
            let updated = thread.get("updatedAt").and_then(Value::as_i64).unwrap_or(0);
            let watched = if self.store().is_watched(id) { " | 盯着" } else { "" };
            lines.push(format!("{} | {} | {} | {state} | {}{watched}", short_id(id), clip(title, 40), project_name(cwd), relative_time(updated, now_secs)));
            if lines.len() >= limit {
                break;
            }
        }
        Ok(if lines.is_empty() { "没有符合条件的会话".into() } else { lines.join("\n") })
    }

    async fn tool_list_projects(&self) -> anyhow::Result<String> {
        let projects = self.projects().await?;
        let now_secs = now_ms() / 1000;
        let lines: Vec<String> = projects.iter().take(20).map(|(cwd, count, updated)| format!("- {} | {cwd} | {count} 个会话 | {}", project_name(cwd), relative_time(*updated, now_secs))).collect();
        Ok(if lines.is_empty() { "还没有项目".into() } else { lines.join("\n") })
    }

    /// 最近会话里出现过的项目目录：(路径, 会话数, 最近更新)，最近的在前。
    async fn projects(&self) -> anyhow::Result<Vec<(String, usize, i64)>> {
        let list = self.service()?.list_threads(None).await?;
        let mut projects: Vec<(String, usize, i64)> = Vec::new();
        for thread in list.get("threads").and_then(Value::as_array).into_iter().flatten() {
            let Some(cwd) = thread.get("cwd").and_then(Value::as_str).filter(|cwd| !cwd.is_empty()) else { continue };
            let updated = thread.get("updatedAt").and_then(Value::as_i64).unwrap_or(0);
            match projects.iter_mut().find(|(existing, _, _)| existing == cwd) {
                Some(project) => {
                    project.1 += 1;
                    project.2 = project.2.max(updated);
                }
                None => projects.push((cwd.to_string(), 1, updated)),
            }
        }
        projects.sort_by_key(|(_, _, updated)| std::cmp::Reverse(*updated));
        Ok(projects)
    }

    /// 项目名或路径 → 本机存在的绝对路径。
    async fn resolve_project(&self, project: &str) -> anyhow::Result<String> {
        let path = std::path::Path::new(project);
        if path.is_absolute() {
            if path.is_dir() {
                return Ok(project.to_string());
            }
            bail!("目录不存在：{project}");
        }
        let projects = self.projects().await?;
        let wanted = project.to_lowercase();
        let matches: Vec<&String> = projects.iter().map(|(cwd, _, _)| cwd).filter(|cwd| project_name(cwd).to_lowercase() == wanted).collect();
        let matches = if matches.is_empty() { projects.iter().map(|(cwd, _, _)| cwd).filter(|cwd| cwd.to_lowercase().contains(&wanted)).collect() } else { matches };
        match matches.as_slice() {
            [one] => Ok((*one).clone()),
            [] => bail!("找不到项目「{project}」，先用 list_projects 看看，或者给绝对路径"),
            many => bail!("「{project}」对上了好几个项目：{}，给完整路径", many.iter().map(|cwd| cwd.as_str()).collect::<Vec<_>>().join("、")),
        }
    }

    /// 会话 ID（完整或前缀）→ (完整 ID, 标题)。
    pub(super) async fn resolve_thread(&self, raw: &str) -> anyhow::Result<(String, String)> {
        let raw = raw.trim().trim_start_matches('#');
        let list = self.service()?.list_threads(None).await?;
        let threads: Vec<(String, String)> = list
            .get("threads")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|thread| Some((thread.get("id")?.as_str()?.to_string(), thread.get("title").and_then(Value::as_str).unwrap_or("未命名").to_string())))
            .collect();
        if let Ok(uuid) = uuid::Uuid::parse_str(raw) {
            let id = uuid.to_string();
            let title = threads.iter().find(|(known, _)| known == &id).map(|(_, title)| title.clone()).unwrap_or_default();
            return Ok((id, title));
        }
        let prefix = raw.to_lowercase();
        if prefix.len() < 4 || !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            bail!("会话 ID 不对：{raw}（用 list_threads 里的 ID）");
        }
        let mut candidates: Vec<(String, String)> = threads.into_iter().filter(|(id, _)| id.starts_with(&prefix)).collect();
        if candidates.is_empty() {
            let store = self.store();
            candidates = store.watched.iter().filter(|watched| watched.thread_id.starts_with(&prefix)).map(|watched| (watched.thread_id.clone(), watched.title.clone())).collect();
        }
        match candidates.len() {
            1 => Ok(candidates.remove(0)),
            0 => bail!("找不到会话 {raw}，先用 list_threads 看看"),
            _ => bail!("{raw} 对上了好几个会话，多给几位"),
        }
    }

    async fn tool_watch(&self, args: &Value) -> anyhow::Result<String> {
        match text_arg(args, "action").unwrap_or("list") {
            "add" => {
                let (id, title) = self.resolve_thread(text_arg(args, "thread").context("缺少 thread")?).await?;
                self.store().watch(&id, &title, text_arg(args, "reason").unwrap_or_default());
                self.touch();
                Ok(format!("盯着 {}「{title}」了，跑完、出错会告诉你。", short_id(&id)))
            }
            "remove" => {
                let (id, _) = self.resolve_thread(text_arg(args, "thread").context("缺少 thread")?).await?;
                let removed = self.store().unwatch(&id);
                self.touch();
                Ok(if removed { format!("不再盯 {} 了。", short_id(&id)) } else { "本来就没盯着它。".into() })
            }
            _ => {
                let store = self.store();
                let lines: Vec<String> = store.watched.iter().map(|watched| format!("- {}「{}」{}", short_id(&watched.thread_id), watched.title, watched.reason)).collect();
                Ok(if lines.is_empty() { "没有在盯的会话".into() } else { lines.join("\n") })
            }
        }
    }

    fn tool_later(&self, args: &Value) -> anyhow::Result<String> {
        let mut store = self.store();
        let result = match text_arg(args, "action").unwrap_or("list") {
            "add" => {
                let minutes = args.get("minutes").and_then(Value::as_f64).context("缺少 minutes")?;
                if !(1.0..=10_080.0).contains(&minutes) {
                    bail!("minutes 要在 1 到 10080（一周）之间");
                }
                let note = text_arg(args, "note").context("缺少 note")?;
                if store.later.len() >= LATER_LIMIT {
                    bail!("已经约了 {LATER_LIMIT} 个回访，先 cancel 一个");
                }
                let id = store.later.iter().map(|item| item.id).max().unwrap_or(0) + 1;
                let due_ms = now_ms() + (minutes * 60_000.0) as i64;
                store.later.push(store::Later { id, due_ms, note: clip(note, 300) });
                store.save_later();
                format!("约好了：#{id} 在 {} 回来看看。", super::prompt::format_clock(due_ms))
            }
            "cancel" => {
                let id = args.get("id").and_then(Value::as_u64).context("缺少 id")? as u32;
                let before = store.later.len();
                store.later.retain(|item| item.id != id);
                if store.later.len() == before {
                    bail!("没有 #{id} 这个回访");
                }
                store.save_later();
                format!("#{id} 取消了。")
            }
            _ => {
                let lines: Vec<String> = store.later.iter().map(|item| format!("- #{} {} {}", item.id, super::prompt::format_clock(item.due_ms), item.note)).collect();
                if lines.is_empty() { "没有约好的回访".into() } else { lines.join("\n") }
            }
        };
        drop(store);
        self.touch();
        Ok(result)
    }

    fn tool_remember(&self, args: &Value) -> anyhow::Result<String> {
        let about = text_arg(args, "about").context("缺少 about")?;
        let text = text_arg(args, "text").context("缺少 text")?;
        let replace = text_arg(args, "mode") == Some("replace");
        let mut store = self.store();
        let current = match about {
            "user" => store.user_memory.clone(),
            "notes" => store.notes.clone(),
            other => bail!("about 只能是 user 或 notes，不是 {other}"),
        };
        let next = if replace {
            text.to_string()
        } else if current.trim().is_empty() {
            format!("- {text}")
        } else {
            format!("{}\n- {text}", current.trim_end())
        };
        if next.chars().count() > MEMORY_LIMIT {
            bail!("记满了（上限 {MEMORY_LIMIT} 字）：用 mode=replace 把这份整理精简后重写");
        }
        match about {
            "user" => store.user_memory = next,
            _ => store.notes = next,
        }
        store.save_memory();
        Ok("记下了。".into())
    }
}
