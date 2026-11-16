//! 加加（远程控制里的聊天机器人）：事件推断、合并投递、回复格式、持久化、工具输出、MCP 转发、接口权限。

use codex_plus_core::remote_control::jiajia::events::{self, EventKind, Pacer, Phase};
use codex_plus_core::remote_control::jiajia::prompt::{self, parse_reply};
use codex_plus_core::remote_control::jiajia::store::{ChatMessage, DoneReports, Store, WATCH_LIMIT};
use serde_json::{Value, json};

fn view(runtime: &str, turns: Value, requests: Value) -> Value {
    json!({ "threadId": "t", "title": "修插件", "cwd": "/Users/me/CodexPlusPlus", "runtime": runtime, "turns": turns, "requests": requests })
}

fn approval(id: u64, command: &str) -> Value {
    json!({ "id": id, "method": "item/commandExecution/requestApproval", "params": { "command": command } })
}

#[test]
fn snapshot_reads_phase_requests_and_last_turn() {
    let running = events::snapshot(&view("active", json!([{ "turnId": "a", "status": "inProgress", "items": [] }]), json!([])));
    assert_eq!(running.phase, Phase::Running);
    assert!(running.last_end.is_none(), "还在跑的一轮不算结束");

    let waiting = events::snapshot(&view("active", json!([]), json!([approval(7, "npm test")])));
    assert_eq!(waiting.phase, Phase::Waiting, "有待处理请求就是等你处理");

    let done = events::snapshot(&view(
        "idle",
        json!([{ "turnId": "a", "status": "completed", "items": [
            { "type": "userMessage", "text": "跑测试" },
            { "type": "agentMessage", "text": "测试\n全部通过" }
        ] }]),
        json!([]),
    ));
    assert_eq!(done.phase, Phase::Idle);
    let end = done.last_end.unwrap();
    assert_eq!((end.turn_id.as_str(), end.status.as_str()), ("a", "completed"));
    assert_eq!(end.last_reply.as_deref(), Some("测试 全部通过"), "换行压平");

    // 离线历史用 id 而不是 turnId；带错误的 completed 当失败
    let failed = events::snapshot(&json!({ "runtime": "notLoaded", "turns": [{ "id": "b", "status": "completed", "error": { "message": "上游 502" }, "items": [] }] }));
    assert_eq!(failed.last_end.unwrap().status, "failed");
}

#[test]
fn transitions_report_new_requests_and_turn_ends_once() {
    let running = events::snapshot(&view("active", json!([{ "turnId": "a", "status": "inProgress", "items": [] }]), json!([])));
    let waiting = events::snapshot(&view("active", json!([{ "turnId": "a", "status": "inProgress", "items": [] }]), json!([approval(7, "rm -rf build")])));
    let found = events::transitions("t", Some(&running), &waiting, false);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, EventKind::Waiting);
    assert!(found[0].detail.contains("rm -rf build"));
    assert!(found[0].request.is_some(), "等审批带上请求，好贴卡片");
    assert!(events::transitions("t", Some(&waiting), &waiting, false).is_empty(), "同一个请求不重复报");

    let done = events::snapshot(&view("idle", json!([{ "turnId": "a", "status": "completed", "items": [{ "type": "agentMessage", "text": "好了" }] }]), json!([])));
    let found = events::transitions("t", Some(&waiting), &done, false);
    assert_eq!(found.iter().map(|event| event.kind).collect::<Vec<_>>(), vec![EventKind::Done]);
    assert!(found[0].detail.contains("好了"));
    assert_eq!(found[0].turn_id.as_deref(), Some("a"), "跑完的事件记下是哪一轮（加加读过这一轮就不再交给它）");
    assert_eq!(events::transitions("t", Some(&running), &waiting, false)[0].turn_id, None);
    assert!(events::transitions("t", Some(&done), &done, false).is_empty(), "同一轮不重复报");

    // 第一次看到：只报等审批；跑完的只有盯着的才报
    assert!(events::transitions("t", None, &done, false).is_empty());
    assert_eq!(events::transitions("t", None, &done, true).len(), 1);
    assert_eq!(events::transitions("t", None, &waiting, false).len(), 1);

    let interrupted = events::snapshot(&view("idle", json!([{ "turnId": "b", "status": "interrupted", "items": [] }]), json!([])));
    assert!(events::transitions("t", Some(&running), &interrupted, true).is_empty(), "用户自己停的不报");

    let failed = events::snapshot(&view("idle", json!([{ "turnId": "c", "status": "failed", "error": { "message": "额度用完了" }, "items": [] }]), json!([])));
    let found = events::transitions("t", Some(&done), &failed, false);
    assert_eq!(found[0].kind, EventKind::Failed);
    assert_eq!(found[0].detail, "额度用完了");
}

#[test]
fn request_summaries_say_what_is_needed() {
    assert_eq!(events::request_summary(&approval(1, "cargo test")), "要运行命令 `cargo test`");
    let array = json!({ "id": 1, "method": "item/commandExecution/requestApproval", "params": { "command": ["git", "push"] } });
    assert_eq!(events::request_summary(&array), "要运行命令 `git push`");
    let question = json!({ "id": 2, "method": "item/tool/requestUserInput", "params": { "questions": [{ "question": "用哪个数据库？" }] } });
    assert_eq!(events::request_summary(&question), "在提问：用哪个数据库？");
    assert!(events::request_summary(&json!({ "method": "item/fileChange/requestApproval", "params": {} })).starts_with("要改文件"));
}

fn event(kind: EventKind, thread: &str) -> events::Event {
    events::Event { kind, thread_id: thread.into(), title: format!("会话{thread}"), cwd: "/x/proj".into(), detail: String::new(), request: None, turn_id: None }
}

#[test]
fn pacer_gathers_dedupes_and_orders_events() {
    let mut pacer = Pacer::default();
    assert!(!pacer.is_due(i64::MAX));
    pacer.add(event(EventKind::Done, "a"), 0);
    assert!(!pacer.is_due(events::GATHER_DONE_MS - 1), "跑完的攒一分钟");
    assert!(pacer.is_due(events::GATHER_DONE_MS));
    pacer.add(event(EventKind::Failed, "a"), 10);
    pacer.add(event(EventKind::Waiting, "b"), 20);
    pacer.add(event(EventKind::Done, "b"), 30);
    assert_eq!(pacer.due_at(), Some(20 + events::GATHER_WAITING_MS), "等审批的催得紧");
    let batch = pacer.take(100_000);
    let kinds: Vec<(EventKind, &str)> = batch.iter().map(|event| (event.kind, event.thread_id.as_str())).collect();
    assert_eq!(kinds, vec![(EventKind::Waiting, "b"), (EventKind::Failed, "a"), (EventKind::Done, "b")], "同会话只留最新，但等审批不被跑完顶掉；按紧急程度排");
    assert!(pacer.is_empty());

    pacer.add(event(EventKind::Done, "old"), 0);
    assert!(pacer.take(events::STALE_DONE_MS + 1).is_empty(), "太久以前跑完的不报");

    pacer.add(event(EventKind::Waiting, "c"), 0);
    pacer.drop_waiting("c");
    assert!(pacer.is_empty(), "请求被处理了就撤掉");

    for index in 0..20 {
        pacer.add(event(EventKind::Done, &format!("t{index}")), 0);
    }
    assert_eq!(pacer.take(events::GATHER_DONE_MS).len(), events::BATCH_LIMIT);
}

#[test]
fn event_batches_are_marked_as_system_text() {
    let mut first = event(EventKind::Failed, "0123456789abcdef");
    first.detail = "上游 502".into();
    let text = events::format_batch(&[first.clone(), event(EventKind::Done, "fedcba9876543210")]);
    assert!(text.starts_with("<事件") && text.ends_with("</事件>"));
    assert!(text.contains("不是用户说的话"));
    assert!(text.contains("出错了：「会话0123456789abcdef」（会话 01234567，项目 proj） — 上游 502"));
    assert_eq!(events::notice_text(&first), "「会话0123456789abcdef」出错了：上游 502");
}

#[test]
fn replies_can_stay_silent_and_carry_thread_cards() {
    assert_eq!(parse_reply("[静默]"), None);
    assert_eq!(parse_reply("  [静默]\n"), None);
    assert_eq!(parse_reply(""), None);
    let (text, threads) = parse_reply("「修插件」跑完了，测试全过 [[会话:01a1e5b5]]").unwrap();
    assert_eq!(text, "「修插件」跑完了，测试全过");
    assert_eq!(threads, vec!["01a1e5b5"]);
    let (_, threads) = parse_reply("a [[会话:1]] b [[会话:2]] [[会话:2]] [[会话:3]] [[会话:4]]").unwrap();
    assert_eq!(threads, vec!["1", "2", "3"], "去重，最多三张");
    let (text, threads) = parse_reply("没写完的 [[会话:abc").unwrap();
    assert_eq!(text, "没写完的 [[会话:abc");
    assert!(threads.is_empty());
}

#[test]
fn developer_instructions_carry_memory_watch_list_and_recent_chat() {
    use codex_plus_core::remote_control::jiajia::store::{Later, Watched};
    let watched = vec![Watched { thread_id: "0123456789".into(), title: "修插件".into(), reason: "加加新开的".into(), since_ms: 0 }];
    let later = vec![Later { id: 2, due_ms: 0, note: "看看测试".into() }];
    let recent = vec![
        ChatMessage { id: 1, at: 0, role: "user".into(), text: "帮我盯着".into(), threads: vec![], request: None, resolved: false, by: None, rev: 1 },
        ChatMessage { id: 2, at: 0, role: "event".into(), text: "不该出现".into(), threads: vec![], request: None, resolved: false, by: None, rev: 2 },
    ];
    let text = prompt::developer_instructions("2026-11-17 02:30 周二", "MacBook", "", "- 喜欢简短", "", &watched, &later, &recent);
    assert!(!text.contains("你的性格"), "没设性格就不写这一节");
    let styled = prompt::developer_instructions("now", "Mac", "  毒舌，但靠谱  ", "", "", &[], &[], &[]);
    assert!(styled.contains("## 你的性格（用户在设置里给你定的）\n毒舌，但靠谱\n"));
    assert!(styled.contains("不替用户审批"), "性格压不过规矩");
    assert!(text.contains("2026-11-17 02:30") && text.contains("MacBook"));
    assert!(text.contains("- 喜欢简短"));
    assert!(text.contains("（空）"), "空的笔记写明是空的");
    assert!(text.contains("01234567 修插件（加加新开的）"));
    assert!(text.contains("#2") && text.contains("看看测试"));
    assert!(text.contains("用户：帮我盯着"));
    assert!(!text.contains("不该出现"), "事件灰字不放进最近聊天");
    for rule in ["不能替用户审批", "[静默]", "[[会话:", "<事件>", "<回访>"] {
        assert!(prompt::BASE_INSTRUCTIONS.contains(rule), "提示词缺少：{rule}");
    }
}

fn chat(role: &str, text: &str) -> ChatMessage {
    ChatMessage { id: 0, at: 0, role: role.into(), text: text.into(), threads: vec![], request: None, resolved: false, by: None, rev: 0 }
}

#[test]
fn store_persists_chat_settings_and_tracks_unread() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path());
    assert!(!store.settings.configured);
    assert!(store.settings.proactive, "主动消息默认开");
    assert_eq!(store.settings.done_reports, DoneReports::Watched, "默认只报盯着的");

    let first = store.push(chat("user", "你好"));
    let second = store.push(chat("bot", "在呢"));
    store.push(chat("notice", "「修插件」跑完了"));
    store.push(chat("event", "灰字不算未读"));
    assert_eq!((first, second), (1, 2));
    assert_eq!(store.unread(), 2, "只有加加说的话、提醒算未读");
    store.mark_read(second);
    assert_eq!(store.unread(), 1);
    store.mark_read(1);
    assert_eq!(store.read_upto, 2, "已读位置不倒退");
    store.mark_read(999);
    assert_eq!(store.read_upto, 4, "不超过最新一条");

    let rev = store.rev();
    assert!(store.update(second, |message| message.resolved = true));
    let changed = store.changed_since(rev);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].id, second, "改动的记录也会推送");
    assert_eq!(store.page_before(Some(3), 1)[0].id, 2);

    store.settings.configured = true;
    store.settings.model = Some("deepseek-flash".into());
    store.save_settings();
    store.watch("thread-a", "修插件", "加加新开的");
    store.watch("thread-a", "修插件（改名）", "");
    assert_eq!(store.watched.len(), 1, "同一会话不重复盯");

    let reopened = Store::open(dir.path());
    assert_eq!(reopened.messages.len(), 4);
    assert!(reopened.messages[1].resolved, "改动写回了文件");
    assert_eq!(reopened.settings.model.as_deref(), Some("deepseek-flash"));
    assert_eq!(reopened.watched[0].title, "修插件（改名）");
    assert_eq!(reopened.read_upto, 4);

    let mut reopened = reopened;
    assert_eq!(reopened.push(chat("user", "接着聊")), 5, "重开后编号接着往下");
    reopened.clear_chat();
    assert!(reopened.messages.is_empty());
    assert_eq!(reopened.unread(), 0);
    assert_eq!(Store::open(dir.path()).messages.len(), 0);
    assert_eq!(Store::open(dir.path()).watched.len(), 1, "清空聊天不动盯着的会话");

    for index in 0..WATCH_LIMIT + 5 {
        reopened.watch(&format!("t{index}"), "", "");
    }
    assert_eq!(reopened.watched.len(), WATCH_LIMIT, "满了挤掉最早的");
    assert!(reopened.unwatch("t44"));
    assert!(!reopened.unwatch("nope"));
}

#[test]
fn persona_and_avatar_settings_are_validated() {
    use codex_plus_core::remote_control::jiajia::Jiajia;
    use codex_plus_core::remote_control::jiajia::store::{Avatar, PERSONA_LIMIT, Settings};
    let base = Settings::default();
    assert_eq!(base.persona, "");
    assert_eq!(base.avatar, Avatar::default());

    let set = Jiajia::apply_settings(base.clone(), &json!({ "persona": "  傲娇  " })).unwrap();
    assert_eq!(set.persona, "傲娇", "去掉首尾空白");
    assert_eq!(Jiajia::apply_settings(set.clone(), &json!({ "persona": "" })).unwrap().persona, "", "清空就是恢复默认");
    assert!(Jiajia::apply_settings(base.clone(), &json!({ "persona": "字".repeat(PERSONA_LIMIT + 1) })).is_err());
    assert!(Jiajia::apply_settings(base.clone(), &json!({ "persona": 1 })).is_err());

    let with_image = Settings { avatar: Avatar { image: Some("avatar.jpg".into()), version: 3, ..Default::default() }, ..base.clone() };
    let text = Jiajia::apply_settings(with_image.clone(), &json!({ "avatar": { "text": "🐱", "color": "#07c160" } })).unwrap();
    assert_eq!(text.avatar.text.as_deref(), Some("🐱"));
    assert_eq!(text.avatar.color.as_deref(), Some("#07c160"));
    assert_eq!(text.avatar.image, None, "改用文字头像就不用图片了");
    assert!(Jiajia::apply_settings(base.clone(), &json!({ "avatar": { "text": "五个字太长" } })).is_err());
    assert!(Jiajia::apply_settings(base.clone(), &json!({ "avatar": { "color": "red" } })).is_err());
    assert!(Jiajia::apply_settings(base.clone(), &json!({ "avatar": { "color": "#12345g" } })).is_err());
    let reset = Jiajia::apply_settings(text, &json!({ "avatar": { "reset": true } })).unwrap();
    assert_eq!((reset.avatar.text, reset.avatar.color, reset.avatar.image), (None, None, None));
    assert!(Jiajia::apply_settings(with_image.clone(), &json!({ "model": "x" })).unwrap().avatar.image.is_some(), "不碰头像就保持原样");
}

#[test]
fn avatar_images_are_checked_stored_and_replaced() {
    use codex_plus_core::remote_control::jiajia::Jiajia;
    use codex_plus_core::remote_control::jiajia::store::avatar_extension;
    assert_eq!(avatar_extension(b"\xff\xd8\xff\xe0rest"), Some("jpg"));
    assert_eq!(avatar_extension(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
    assert_eq!(avatar_extension(b"GIF89a..."), Some("gif"));
    assert_eq!(avatar_extension(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
    assert_eq!(avatar_extension(b"<svg onload=alert(1)>"), None, "只认位图，不认 SVG / HTML");

    let dir = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let jiajia = Jiajia::new(dir.path().to_path_buf(), std::sync::Weak::new());
        assert!(jiajia.avatar_image().is_none());
        assert!(jiajia.set_avatar_image(b"not an image").is_err());
        assert!(jiajia.set_avatar_image(&vec![0xff; 2 * 1024 * 1024]).is_err(), "太大的拒收");

        let summary = jiajia.set_avatar_image(b"\x89PNG\r\n\x1a\npng-bytes").unwrap();
        assert_eq!(summary["settings"]["avatar"]["image"], "avatar.png");
        assert_eq!(summary["settings"]["avatar"]["version"], 1);
        assert_eq!(jiajia.avatar_image().unwrap().1, "image/png");
        // 换成 JPEG：旧的 PNG 删掉
        jiajia.set_avatar_image(b"\xff\xd8\xff\xe0jpeg").unwrap();
        assert!(!dir.path().join("avatar.png").exists());
        assert_eq!(jiajia.avatar_image().unwrap().1, "image/jpeg");

        // 改用文字头像：图片文件删掉，版本号变，网页才会重画
        let summary = jiajia.update_settings(&json!({ "avatar": { "text": "猫" } })).await.unwrap();
        assert!(!dir.path().join("avatar.jpg").exists());
        assert_eq!(summary["settings"]["avatar"]["text"], "猫");
        assert_eq!(summary["settings"]["avatar"]["version"], 3);
        assert!(jiajia.avatar_image().is_none());

        jiajia.update_settings(&json!({ "persona": "元气满满" })).await.unwrap();
        assert_eq!(Store::open(dir.path()).settings.persona, "元气满满", "写回了文件");
    });
}

#[test]
fn thread_views_are_summarized_for_the_model() {
    use codex_plus_core::remote_control::jiajia::tools::{relative_time, summarize_view};
    let view = json!({
        "title": "修插件", "cwd": "/Users/me/CodexPlusPlus", "runtime": "active", "controls": { "model": "deepseek-flash" },
        "turns": [
            { "turnId": "a", "status": "completed", "items": [
                { "type": "userMessage", "text": "修一下透明度" },
                { "type": "commandExecution", "command": ["npm", "test"], "status": "failed" },
                { "type": "fileChange", "changes": [{ "path": "/Users/me/CodexPlusPlus/src/a.ts" }, { "path": "/Users/me/CodexPlusPlus/src/b.ts" }] },
                { "type": "agentMessage", "text": "改好了" }
            ] },
            { "turnId": "b", "status": "failed", "error": { "message": "上游 502" }, "items": [] }
        ],
        "requests": [approval(3, "git push")]
    });
    let text = summarize_view(&view, 12);
    for expected in ["「修插件」", "等你处理", "模型 deepseek-flash", "用户：修一下透明度", "[命令] `npm test` 失败", "[改文件] a.ts, b.ts", "Codex：改好了", "[这一轮失败] 上游 502", "[等你处理] 要运行命令 `git push`"] {
        assert!(text.contains(expected), "缺少 {expected}：\n{text}");
    }
    let short = summarize_view(&view, 2);
    assert!(short.contains("（更早的 3 条省略）"));
    assert!(!short.contains("修一下透明度"));
    // 被拒绝的改动要写明（实测加加把 declined 的改动说成「改了」）
    let declined = json!({ "turns": [{ "turnId": "a", "status": "interrupted", "items": [{ "type": "fileChange", "status": "declined", "changes": [{ "path": "/tmp/a.txt" }] }] }] });
    let text = summarize_view(&declined, 12);
    assert!(text.contains("[改文件] a.txt（被拒绝，没改）") && text.contains("[这一轮被停止]"), "{text}");

    assert_eq!(relative_time(1000, 1030), "刚刚");
    assert_eq!(relative_time(1000, 1000 + 5 * 60), "5 分钟前");
    assert_eq!(relative_time(1000, 1000 + 3 * 3600), "3 小时前");
    assert_eq!(relative_time(1000, 1000 + 2 * 86_400), "2 天前");
}

#[test]
fn mcp_proxy_lists_tools_and_forwards_calls() {
    use codex_plus_core::remote_control::jiajia::mcp::handle_request;
    use codex_plus_core::remote_control::jiajia::tools::{activity_label, tool_definitions};
    let never = |_: &str, _: &Value| -> anyhow::Result<String> { unreachable!() };
    let init = handle_request(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } }), never).unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert!(handle_request(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }), never).is_none(), "通知不回");

    let listed = handle_request(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }), never).unwrap();
    let tools = listed["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
    for name in ["list_threads", "read_thread", "send_to_thread", "new_thread", "stop_thread", "set_thread_settings", "watch", "later", "remember", "recent_events"] {
        assert!(names.contains(&name), "缺少工具 {name}");
    }
    let mut unique = names.clone();
    unique.dedup();
    assert_eq!(unique.len(), names.len());
    for tool in tool_definitions().as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
        assert!(tool["description"].as_str().is_some_and(|text| !text.is_empty()));
    }
    assert_eq!(activity_label("read_thread"), "正在看会话内容");

    let call = |name: &str, arguments: &Value| -> anyhow::Result<String> {
        if name == "boom" {
            anyhow::bail!("会话不存在");
        }
        Ok(format!("{name}:{}", arguments["thread"].as_str().unwrap_or_default()))
    };
    let ok = handle_request(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "read_thread", "arguments": { "thread": "01a1" } } }), call).unwrap();
    assert_eq!(ok["result"]["content"][0]["text"], "read_thread:01a1");
    assert_eq!(ok["result"]["isError"], false);
    let failed = handle_request(&json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "boom" } }), call).unwrap();
    assert_eq!(failed["result"]["isError"], true, "工具失败交给模型看，不当协议错误");
    assert_eq!(failed["result"]["content"][0]["text"], "会话不存在");
    let unknown = handle_request(&json!({ "jsonrpc": "2.0", "id": 5, "method": "sampling/createMessage" }), call).unwrap();
    assert_eq!(unknown["error"]["code"], -32601);
}

#[test]
fn jiajia_routes_require_roles() {
    use codex_plus_core::remote_control::devices::Role;
    use codex_plus_core::remote_control::server::required_role;
    assert_eq!(required_role("GET", &["jiajia"]), Role::View, "只读设备也能看加加");
    assert_eq!(required_role("GET", &["jiajia", "stream"]), Role::View);
    assert_eq!(required_role("POST", &["jiajia", "read"]), Role::View);
    assert_eq!(required_role("GET", &["jiajia", "avatar"]), Role::View);
    for action in ["messages", "interrupt", "settings", "clear", "memory", "avatar"] {
        assert_eq!(required_role("POST", &["jiajia", action]), Role::Message, "{action}");
    }
}

/// 加加的会话：临时（不落盘、桌面看不到）、只读沙箱、从不审批；只挂加加自己的 MCP，工具直接给模型、调用不弹审批；
/// 工具入口只认本机和加加专用令牌，在设备认证之前单独处理。
#[test]
fn bot_thread_is_ephemeral_sandboxed_and_tools_are_loopback_only() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/remote_control/jiajia/mod.rs")).unwrap();
    for expected in [
        r#""ephemeral": true"#,
        r#""sandbox": "read-only""#,
        r#""approvalPolicy": "never""#,
        r#""baseInstructions": prompt::BASE_INSTRUCTIONS"#,
        r#"config.insert("features.tool_search".into(), json!(false));"#,
        r#"config.insert(server("default_tools_approval_mode"), json!("approve"));"#,
        r#"config.insert(format!("mcp_servers.{id}.enabled"), json!(false));"#,
        r#"item.get("phase").and_then(Value::as_str) == Some("commentary")"#,
        // 加加自己读过的那一轮，跑完 / 出错不再唤醒它（实测：派完活马上读到结果，一分钟后又被事件唤醒一次）
        "if self.already_seen(event) {",
        "batch.into_iter().filter(|event| !self.already_seen(event))",
        // 审批卡片拿当前请求核对（服务重启后没有上次的状态可比，卡片也不能一直挂着）；有未处理卡片的会话也要看
        "settled.push((id.clone(), next.requests.iter().map(|(key, _)| key.clone()).collect()));",
        "carded.iter().any(|carded| carded == id)",
    ] {
        assert!(source.contains(expected), "加加会话缺少：{expected}");
    }
    let server = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/remote_control/server.rs")).unwrap();
    assert!(server.contains("context.scope == Scope::Loopback && provided_token(request).is_some_and(|token| token_matches(jiajia.tool_token(), &token))"));
    let tool = server.find(r#"request.path == "/api/jiajia/tool""#).unwrap();
    let auth = server.find("let Some(principal) = authenticate(").unwrap();
    assert!(tool < auth, "工具入口不认设备令牌，单独校验");
    let main = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/codex-plus-manager/src-tauri/src/main.rs")).unwrap();
    assert!(main.contains("codex_plus_core::remote_control::jiajia::MCP_FLAG"), "主程序认加加的 MCP 参数");
    assert!(main.find("jiajia::MCP_FLAG").unwrap() < main.find("--codex-plus-multi-worker").unwrap(), "在启动 GUI 之前处理");
}

#[test]
fn user_mcp_servers_are_listed_for_disabling() {
    use codex_plus_core::remote_control::jiajia::configured_mcp_servers;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    assert!(configured_mcp_servers(&path).is_empty(), "没有文件不报错");
    std::fs::write(&path, "model = \"x\"\n[mcp_servers.computer_use]\ncommand = \"a\"\n[mcp_servers.codex_plus_extensions]\ncommand = \"b\"\n").unwrap();
    let mut ids = configured_mcp_servers(&path);
    ids.sort();
    assert_eq!(ids, vec!["codex_plus_extensions", "computer_use"]);
    std::fs::write(&path, "not toml [[[").unwrap();
    assert!(configured_mcp_servers(&path).is_empty());
}

#[test]
fn web_page_pins_jiajia_and_reuses_request_cards() {
    let page = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/remote-control/index.html")).unwrap();
    for expected in [
        r#"<section class="m-screen m-chat" id="mBot" hidden>"#,
        "/api/jiajia/stream?token=",
        "const pinned = jj.summary && !ui.query.trim() && ui.filter === \"all\" ? [jjRow()] : [];",
        "requestCard(info.request || {}, info.threadId,",
        "function requestCard(request, threadId = state.current, onDone = null)",
        "await api(\"POST\", `/threads/${threadId}/requests/",
        "card.onclick = () => openChat(id, \"bot\");",
        "if (ui.from === \"bot\") {",
        "m.startJiajia();",
        "function jjSetupSheet()",
        "api(\"POST\", \"/jiajia/read\"",
        // 性格与头像：··· 里两项；头像图片裁成 256 的正方形再传；列表和气泡都用同一个画头像的函数
        "function jjPersonaSheet()",
        "function jjAvatarSheet()",
        "canvas.width = canvas.height = 256;",
        "fetch(\"/api/jiajia/avatar\", { method: \"POST\"",
        "/api/jiajia/avatar?v=${avatar.version || 0}&token=",
        "jjFace(avatar.querySelector(\".face\"));",
        "jjFace(node.querySelector(\".who\"));",
    ] {
        assert!(page.contains(expected), "网页缺少：{expected}");
    }
    assert!(page.contains("$(\"mBot\").hidden = screen !== \"bot\";"), "加加和会话对话占同一个位置");
}
