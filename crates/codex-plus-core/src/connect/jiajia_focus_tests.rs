//! 微信专注模式：假的远程控制会话（[`MockSessions`]）+ 假的加加 + 假微信网关，不碰真实微信和侧车。

use super::*;
use crate::connect::jiajia_bridge::focus::{LIST_TTL_MS, SessionPort, ThreadSummary, pick_project, session_line, turn_report};
use crate::remote_control::uploads::{Attachment, AttachmentKind};

const T1: &str = "11111111-aaaa-4000-8000-000000000001";
const T2: &str = "22222222-bbbb-4000-8000-000000000002";

// ---------- 假的远程控制会话 ----------

#[derive(Default)]
struct SessionsState {
    off: bool,
    threads: Vec<ThreadSummary>,
    views: HashMap<String, Value>,
    /// (会话, 文字, 附件)
    sent: Vec<(String, String, Vec<Attachment>)>,
    created: Vec<(String, String)>,
}

pub(super) struct MockSessions(Mutex<SessionsState>);

impl MockSessions {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(SessionsState::default())))
    }

    fn lock(&self) -> MutexGuard<'_, SessionsState> {
        self.0.lock().unwrap()
    }

    /// 加一个会话（列表里排在最前面）和它的视图。
    fn add(&self, id: &str, title: &str, cwd: &str, state: &str, view: Value) {
        let mut sessions = self.lock();
        sessions.threads.insert(0, ThreadSummary { id: id.into(), title: title.into(), cwd: cwd.into(), state: state.into(), updated_at: now_ms() / 1000 - 180 });
        sessions.views.insert(id.into(), view);
    }

    fn set_view(&self, id: &str, view: Value) {
        self.lock().views.insert(id.into(), view);
    }

    /// 会话被删了。
    fn remove(&self, id: &str) {
        let mut sessions = self.lock();
        sessions.views.remove(id);
        sessions.threads.retain(|thread| thread.id != id);
    }

    fn sent(&self) -> Vec<(String, String, usize)> {
        self.lock().sent.iter().map(|(id, text, attachments)| (id.clone(), text.clone(), attachments.len())).collect()
    }
}

#[async_trait]
impl SessionPort for MockSessions {
    fn available(&self) -> bool {
        !self.lock().off
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadSummary>> {
        Ok(self.lock().threads.clone())
    }

    async fn thread_view(&self, thread_id: &str) -> anyhow::Result<Option<Value>> {
        Ok(self.lock().views.get(thread_id).cloned())
    }

    async fn send(&self, thread_id: &str, text: &str, attachments: &[Attachment]) -> anyhow::Result<bool> {
        let mut sessions = self.lock();
        if !sessions.views.contains_key(thread_id) {
            anyhow::bail!("找不到这个会话");
        }
        sessions.sent.push((thread_id.into(), text.into(), attachments.to_vec()));
        Ok(false)
    }

    async fn create_thread(&self, cwd: &str, title: &str) -> anyhow::Result<String> {
        let id = format!("c0ffee{:02x}-0000-4000-8000-000000000000", self.lock().created.len());
        self.lock().created.push((cwd.into(), title.into()));
        self.add(&id, title, cwd, "空闲", view(&id, title, cwd, "idle", json!([]), json!([])));
        Ok(id)
    }
}

fn view(id: &str, title: &str, cwd: &str, runtime: &str, turns: Value, requests: Value) -> Value {
    json!({ "threadId": id, "title": title, "cwd": cwd, "runtime": runtime, "turns": turns, "requests": requests })
}

fn turn(id: &str, status: &str, items: Value) -> Value {
    json!({ "turnId": id, "status": status, "items": items })
}

fn agent(text: &str) -> Value {
    json!({ "type": "agentMessage", "text": text })
}

impl Fixture {
    /// 一个叫「重构登录」的空闲会话（项目目录在临时目录里），上一轮回过「登录页改好了」。
    fn add_login_thread(&self) -> String {
        let cwd = self.dir.path().join("CodexPlusPlus");
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.display().to_string();
        let turns = json!([turn("t-old", "completed", json!([{ "type": "userMessage", "text": "重构登录" }, agent("登录页改好了")]))]);
        self.sessions.add(T1, "重构登录", &cwd, "空闲", view(T1, "重构登录", &cwd, "idle", turns, json!([])));
        cwd
    }

    fn last_text(&self) -> String {
        self.gateway.texts().pop().unwrap_or_default()
    }
}

// ---------- 解析与纯函数 ----------

#[test]
fn focus_commands_parse_with_args_aliases_and_fullwidth_slash() {
    assert_eq!(parse_command("/会话"), Some(Command::Sessions));
    assert_eq!(parse_command("/sessions"), Some(Command::Sessions));
    assert_eq!(parse_command("／会话 "), Some(Command::Sessions));
    assert_eq!(parse_command("/进入 3"), Some(Command::Focus("3".into())));
    assert_eq!(parse_command("／进入3"), Some(Command::Focus("3".into())), "中文命令带数字可以不加空格");
    assert_eq!(parse_command("/FOCUS 1a2b3c4d"), Some(Command::Focus("1a2b3c4d".into())));
    assert_eq!(parse_command("/进入"), Some(Command::Focus(String::new())));
    assert_eq!(parse_command("/切换 2"), Some(Command::Switch("2".into())));
    assert_eq!(parse_command("/switch 2"), Some(Command::Switch("2".into())));
    assert_eq!(parse_command("/新会话 CodexPlusPlus 修登录"), Some(Command::NewSession("CodexPlusPlus 修登录".into())));
    assert_eq!(parse_command("/new Fix Login"), Some(Command::NewSession("Fix Login".into())), "参数保留大小写");
    assert_eq!(parse_command("/退出"), Some(Command::Exit));
    assert_eq!(parse_command("/Exit"), Some(Command::Exit));
    for text in ["/会话 列表", "/会话列表", "/退出 了", "/newfoo", "/进入abc", "/exit now", "进入 1", "/sessions2"] {
        assert_eq!(parse_command(text), None, "{text}");
    }
}

#[test]
fn session_lines_turn_reports_and_project_picking() {
    let thread = ThreadSummary { id: T1.into(), title: "重构登录".into(), cwd: "/work/CodexPlusPlus".into(), state: "运行中".into(), updated_at: 1000 };
    assert_eq!(session_line(1, &thread, 1000 + 180, false), "1. 重构登录 · CodexPlusPlus · 运行中 · 3 分钟前");
    assert!(session_line(2, &thread, 1000, true).ends_with("（当前专注）"));

    let done = turn(
        "b",
        "completed",
        json!([
            { "type": "commandExecution", "command": ["cargo", "test"], "status": "completed" },
            { "type": "fileChange", "status": "completed", "changes": [{ "path": "/w/a.rs" }, { "path": "/w/b.rs" }] },
            { "type": "fileChange", "status": "completed", "changes": [{ "path": "/w/a.rs" }] },
            { "type": "fileChange", "status": "declined", "changes": [{ "path": "/w/c.rs" }] },
            { "type": "agentMessage", "phase": "commentary", "text": "先看看" },
            agent("改完了，测试都过了"),
            { "type": "agentMessage", "phase": "commentary", "text": "顺手记一下" }
        ]),
    );
    assert_eq!(turn_report(&done).unwrap(), "改完了，测试都过了\n\n（本轮执行了 1 条命令，改了 2 个文件）");
    let failed = json!({ "turnId": "c", "status": "failed", "error": { "message": "上游 502" }, "items": [] });
    assert_eq!(turn_report(&failed).unwrap(), "（这一轮出错了：上游 502）");
    assert_eq!(turn_report(&turn("d", "interrupted", json!([agent("做到一半")]))).unwrap(), "做到一半\n\n（这一轮被停止了）");
    assert_eq!(turn_report(&turn("e", "completed", json!([]))), None);

    let dir = tempfile::tempdir().unwrap();
    let threads = vec![ThreadSummary { cwd: "/work/CodexPlusPlus".into(), ..thread.clone() }];
    assert_eq!(pick_project(&threads, "codexplusplus 修一下登录页"), Some(("/work/CodexPlusPlus".into(), "修一下登录页".into())));
    assert_eq!(pick_project(&threads, "修一下 CodexPlusPlus 登录页"), None, "第一个词对不上项目：整句都是任务");
    assert_eq!(pick_project(&threads, "CodexPlusPlus"), Some(("/work/CodexPlusPlus".into(), String::new())));
    let absolute = format!("{} 跑测试", dir.path().display());
    assert_eq!(pick_project(&[], &absolute), Some((dir.path().display().to_string(), "跑测试".into())));
}

// ---------- 列表、进入、切换、退出 ----------

#[tokio::test]
async fn sessions_are_listed_with_numbers_that_expire() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    assert_eq!(say(&bridge, "me", "/进入 1").await, Inbound::Handled);
    assert!(fixture.last_text().contains("先发 /会话"), "还没列过就让先列");

    for index in 0..12 {
        let id = format!("{index:08x}-0000-4000-8000-000000000000");
        fixture.sessions.add(&id, &format!("活 {index}"), "/work/proj", "空闲", view(&id, &format!("活 {index}"), "/work/proj", "idle", json!([]), json!([])));
    }
    fixture.add_login_thread();
    fixture.sessions.lock().threads[0].state = "运行中".into();
    assert_eq!(say(&bridge, "me", "/会话").await, Inbound::Handled);
    let text = fixture.last_text();
    let lines: Vec<&str> = text.lines().filter(|line| line.chars().next().is_some_and(|c| c.is_ascii_digit())).collect();
    assert_eq!(lines.len(), 10, "最多 10 条：{text}");
    assert_eq!(lines[0], "1. 重构登录 · CodexPlusPlus · 运行中 · 3 分钟前");
    assert!(text.ends_with("回复 /进入 编号"), "{text}");
    assert!(fixture.jiajia.sent().is_empty(), "命令不发给加加");
    assert_eq!(fixture.bridge().state().session_lists["me"].threads[1].title, "活 11", "编号对应的列表落盘");

    // 列表外的编号、过期的列表都给明确提示。
    say(&bridge, "me", "/进入 11").await;
    assert!(fixture.last_text().contains("列表里没有第 11 个（共 10 个）"));
    bridge.inner().state.session_lists.get_mut("me").unwrap().at -= LIST_TTL_MS + 1;
    say(&bridge, "me", "/进入 1").await;
    assert!(fixture.last_text().contains("会话列表已经过期了，重新发 /会话"));
    assert!(bridge.state().focus.is_empty());

    // 列出来之后会话被删了：进入时说清楚。
    say(&bridge, "me", "/会话").await;
    fixture.sessions.remove(T1);
    say(&bridge, "me", "/进入 1").await;
    assert!(fixture.last_text().contains("这个会话已经不在了"));
    assert!(bridge.state().focus.is_empty());
}

#[tokio::test]
async fn enter_switch_and_exit_move_messages_between_focus_and_the_old_mode() {
    let fixture = Fixture::new().await;
    fixture.add_login_thread();
    fixture.sessions.add(T2, "写文档", "/work/docs", "空闲", view(T2, "写文档", "/work/docs", "idle", json!([]), json!([])));
    let bridge = fixture.bridge();

    // 会话 ID 前 8 位也能进入；附上它最近一条回复。
    assert_eq!(say(&bridge, "me", "/进入 11111111").await, Inbound::Handled);
    let text = fixture.last_text();
    assert!(text.starts_with("已进入「重构登录」，之后的消息直接发给它；/退出 回到加加"), "{text}");
    assert!(text.contains("上次 Codex 说：登录页改好了"), "{text}");
    let focus = fixture.bridge().state().focus["me"].clone();
    assert_eq!((focus.thread_id.as_str(), focus.seen_turns.clone()), (T1, vec!["t-old".to_string()]), "专注状态落盘，进入前的轮次记为已看过");

    // 专注中：消息直接发给会话，不给加加也不走直连；/reset 仍交给直连。
    assert_eq!(say(&bridge, "me", "再加个测试").await, Inbound::Handled);
    assert_eq!(fixture.sessions.sent(), [(T1.to_string(), "再加个测试".to_string(), 0)]);
    assert!(fixture.jiajia.sent().is_empty());
    assert_eq!(say(&bridge, "me", "/reset").await, Inbound::Direct);

    // 重启后还在专注。
    let restarted = fixture.bridge();
    assert_eq!(say(&restarted, "me", "接着来").await, Inbound::Handled);
    assert_eq!(fixture.sessions.sent().last().unwrap().0, T1);

    say(&restarted, "me", "/会话").await;
    assert!(fixture.last_text().contains("1. 写文档"), "新加的会话排最前");
    assert!(fixture.last_text().contains("重构登录 · CodexPlusPlus · 空闲 · 3 分钟前（当前专注）"));
    say(&restarted, "me", "/切换 1").await;
    assert!(fixture.last_text().starts_with("已切换到「写文档」"), "{}", fixture.last_text());
    say(&restarted, "me", "改一下 README").await;
    assert_eq!(fixture.sessions.sent().last().unwrap(), &(T2.to_string(), "改一下 README".to_string(), 0));

    say(&restarted, "me", "/退出").await;
    assert_eq!(fixture.last_text(), "已退出「写文档」，回到加加。");
    assert!(restarted.state().focus.is_empty());
    assert_eq!(say(&restarted, "me", "现在给加加").await, Inbound::Handled);
    assert_eq!(fixture.jiajia.sent().len(), 1);
    say(&restarted, "me", "/exit").await;
    assert!(fixture.last_text().contains("现在没有专注的会话"));

    // 原来是直连的：退出后回到直连。
    let direct = Fixture::new().await;
    direct.add_login_thread();
    let bridge = direct.bridge_with(Some("me"), PeerMode::Direct, false);
    say(&bridge, "me", "/focus 11111111").await;
    assert!(direct.last_text().contains("/退出 回到直连"));
    assert_eq!(say(&bridge, "me", "跑测试").await, Inbound::Handled);
    say(&bridge, "me", "/退出").await;
    assert_eq!(direct.last_text(), "已退出「重构登录」，回到直连。");
    assert_eq!(say(&bridge, "me", "跑测试").await, Inbound::Direct);
}

#[tokio::test]
async fn new_session_picks_the_project_and_enters_focus() {
    let fixture = Fixture::new().await;
    let cwd = fixture.add_login_thread();
    let bridge = fixture.bridge();

    say(&bridge, "me", "/新会话").await;
    assert!(fixture.last_text().starts_with("用法：/新会话"));
    say(&bridge, "me", "/新会话 CodexPlusPlus").await;
    assert!(fixture.last_text().starts_with("用法：/新会话"), "只写了项目没写任务");

    say(&bridge, "me", "/新会话 codexplusplus 修一下登录页").await;
    let text = fixture.last_text();
    assert!(text.starts_with("已在 CodexPlusPlus 新开「修一下登录页」并发出任务，之后的消息直接发给它"), "{text}");
    let created = fixture.sessions.lock().created.clone();
    assert_eq!(created, [(cwd.clone(), "修一下登录页".to_string())]);
    let new_id = bridge.state().focus["me"].thread_id.clone();
    assert_eq!(fixture.sessions.sent(), [(new_id.clone(), "修一下登录页".to_string(), 0)]);
    assert_eq!(bridge.state().last_projects["me"], cwd);

    // 不写项目：用上次的项目。
    say(&bridge, "me", "/new 再写个测试").await;
    assert_eq!(fixture.sessions.lock().created.last().unwrap(), &(cwd.clone(), "再写个测试".to_string()));

    // 没用过项目：退回微信连接配置的工作目录。
    let fresh = Fixture::new().await;
    let bridge = fresh.bridge();
    say(&bridge, "me", "/新会话 随便干点啥").await;
    assert_eq!(fresh.sessions.lock().created, [(fresh.dir.path().display().to_string(), "随便干点啥".to_string())]);
    assert!(fresh.last_text().contains("新开「随便干点啥」"));
}

#[tokio::test]
async fn status_reports_the_current_focus() {
    let fixture = Fixture::new().await;
    fixture.add_login_thread();
    let bridge = fixture.bridge();
    say(&bridge, "me", "/状态").await;
    assert!(fixture.last_text().contains("\n未专注\n"), "{}", fixture.last_text());
    say(&bridge, "me", "/进入 11111111").await;
    say(&bridge, "me", "/状态").await;
    let text = fixture.last_text();
    assert!(text.contains("当前专注：重构登录（空闲）"), "{text}");
    assert!(text.contains("/会话"), "命令列表带上专注命令：{text}");
    say(&bridge, "friend", "/状态").await;
    assert!(!fixture.last_text().contains("专注"), "非本人不显示专注");
}

// ---------- 转发回复与审批 ----------

#[tokio::test]
async fn only_replies_that_finish_after_entering_are_forwarded_once() {
    let fixture = Fixture::new().await;
    let cwd = fixture.add_login_thread();
    let bridge = fixture.bridge();
    let running = json!([
        turn("t-old", "completed", json!([agent("登录页改好了")])),
        turn("t-run", "inProgress", json!([agent("正在改")]))
    ]);
    fixture.sessions.set_view(T1, view(T1, "重构登录", &cwd, "active", running, json!([])));
    say(&bridge, "me", "/进入 11111111").await;
    assert!(fixture.last_text().contains("它正在跑"), "{}", fixture.last_text());
    let before = fixture.gateway.texts().len();
    bridge.poll_focus(true).await;
    assert_eq!(fixture.gateway.texts().len(), before, "旧回复不重发，跑着的这一轮还没结束");

    let finished = json!([
        turn("t-old", "completed", json!([agent("登录页改好了")])),
        turn("t-run", "completed", json!([
            { "type": "commandExecution", "command": "cargo test", "status": "completed" },
            { "type": "fileChange", "status": "completed", "changes": [{ "path": "a.rs" }, { "path": "b.rs" }] },
            agent("改完了")
        ]))
    ]);
    fixture.sessions.set_view(T1, view(T1, "重构登录", &cwd, "idle", finished.clone(), json!([])));
    bridge.poll_focus(true).await;
    assert_eq!(fixture.last_text(), "改完了\n\n（本轮执行了 1 条命令，改了 2 个文件）");
    assert_eq!(fixture.gateway.tokens.lock().unwrap().last().unwrap(), "ctx-me", "用本人最近的 token");
    let count = fixture.gateway.texts().len();
    bridge.poll_focus(true).await;
    fixture.bridge().poll_focus(true).await;
    assert_eq!(fixture.gateway.texts().len(), count, "同一轮不重复转发，重启也不重发");

    // 在桌面上开的新一轮（出错了）也转过来。
    let mut turns = finished.as_array().unwrap().clone();
    turns.push(json!({ "turnId": "t-3", "status": "failed", "error": { "message": "上游 502" }, "items": [] }));
    fixture.sessions.set_view(T1, view(T1, "重构登录", &cwd, "idle", json!(turns), json!([])));
    bridge.poll_focus(true).await;
    assert_eq!(fixture.last_text(), "（这一轮出错了：上游 502）");

    // 退出后不再转。
    say(&bridge, "me", "/退出").await;
    turns.push(turn("t-4", "completed", json!([agent("又一条")])));
    fixture.sessions.set_view(T1, view(T1, "重构登录", &cwd, "idle", json!(turns), json!([])));
    bridge.poll_focus(true).await;
    assert!(!fixture.gateway.texts().iter().any(|text| text == "又一条"));
}

#[tokio::test]
async fn focus_cards_have_no_number_and_bare_digits_approve_them() {
    let fixture = Fixture::new().await;
    let cwd = fixture.add_login_thread();
    let bridge = fixture.bridge_with(Some("me"), PeerMode::Jiajia, true);
    bridge.forward_once().await;
    say(&bridge, "me", "/状态").await;

    // 别的会话的卡片照常（推送开着）：带编号 #1。
    fixture.jiajia.push_card(T2, "部署", command_request(json!(3), json!(["accept", "decline"])));
    bridge.forward_once().await;
    assert!(fixture.last_text().starts_with("⚠️ #1 会话「部署」"));

    // 专注的会话等审批：专注这边发卡片，不带编号；加加那边贴的同一张不重复发。
    let request = command_request(json!(7), json!(["accept", "decline"]));
    fixture.jiajia.lock().live.insert(T1.to_string(), vec!["7".to_string()]);
    fixture.sessions.set_view(T1, view(T1, "重构登录", &cwd, "active", json!([]), json!([request.clone()])));
    say(&bridge, "me", "/进入 11111111").await;
    assert!(fixture.last_text().contains("它在等你处理"));
    fixture.jiajia.push_card(T1, "重构登录", request.clone());
    let before = fixture.gateway.texts().len();
    bridge.forward_once().await;
    assert_eq!(fixture.gateway.texts().len(), before, "加加贴的专注会话卡片不发");
    bridge.poll_focus(true).await;
    let card = fixture.last_text();
    assert!(card.starts_with("⚠️ 会话「重构登录」等你审批：要运行命令 `cargo test`"), "{card}");
    assert!(!card.contains('#'), "专注会话的卡片不带编号：{card}");
    bridge.poll_focus(true).await;
    assert_eq!(fixture.gateway.texts().len(), before + 1, "同一个请求只发一次");

    // 还有别的卡片，光秃秃的数字也是答专注会话的。
    assert_eq!(say(&bridge, "me", "1").await, Inbound::Handled);
    assert_eq!(fixture.jiajia.lock().responded, [(T1.to_string(), json!(7), json!({ "decision": "accept" }))]);
    assert_eq!(fixture.last_text(), "已回复「同意」：会话「重构登录」。");
    let cards = bridge.state().cards;
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].thread_id, T2, "别的会话的卡片还在");
    assert!(fixture.sessions.sent().is_empty(), "审批不当消息发给会话");

    // 在桌面上处理掉的请求：微信里的卡片跟着拿掉。
    let request = command_request(json!(8), json!(["accept", "decline"]));
    fixture.sessions.set_view(T1, view(T1, "重构登录", &cwd, "active", json!([]), json!([request])));
    bridge.poll_focus(true).await;
    assert_eq!(bridge.state().cards.len(), 2);
    fixture.sessions.set_view(T1, view(T1, "重构登录", &cwd, "active", json!([]), json!([])));
    bridge.poll_focus(true).await;
    assert_eq!(bridge.state().cards.len(), 1);
}

// ---------- 权限、远程控制没开、会话没了 ----------

#[tokio::test]
async fn non_owners_and_remote_control_off_get_an_explanation() {
    let fixture = Fixture::new().await;
    fixture.add_login_thread();
    let bridge = fixture.bridge();
    for command in ["/会话", "/进入 1", "/新会话 修 bug", "/退出"] {
        assert_eq!(say(&bridge, "friend", command).await, Inbound::Handled);
        assert!(fixture.last_text().contains("专注模式只对扫码绑定的本人开放"), "{command}");
    }
    assert!(bridge.state().session_lists.is_empty());
    let unknown = fixture.bridge_with(None, PeerMode::Jiajia, false);
    say(&unknown, "me", "/会话").await;
    assert!(fixture.last_text().contains("重新扫码"));

    say(&bridge, "me", "/进入 11111111").await;
    fixture.sessions.lock().off = true;
    for command in ["/会话", "/切换 1", "/新会话 修 bug"] {
        say(&bridge, "me", command).await;
        assert!(fixture.last_text().contains("请先在 Codex++ 打开远程控制"), "{command}");
    }
    // 专注中远程控制关了：说明没发出去，不悄悄交给别人。
    assert_eq!(say(&bridge, "me", "继续").await, Inbound::Handled);
    assert!(fixture.last_text().contains("这条没发给「重构登录」") && fixture.last_text().contains("请先在 Codex++ 打开远程控制"));
    assert!(fixture.sessions.sent().is_empty() && fixture.jiajia.sent().is_empty());
    say(&bridge, "me", "/状态").await;
    assert!(fixture.last_text().contains("当前专注：重构登录（远程控制没开）"));
    bridge.poll_focus(true).await;
    assert!(bridge.state().focus.contains_key("me"), "远程控制没开不算会话没了");
    say(&bridge, "me", "/退出").await;
    assert!(fixture.last_text().starts_with("已退出「重构登录」"), "退出不需要远程控制");
}

#[tokio::test]
async fn a_vanished_focus_session_exits_automatically_and_says_so_once() {
    let fixture = Fixture::new().await;
    fixture.add_login_thread();
    let bridge = fixture.bridge();
    say(&bridge, "me", "/进入 11111111").await;
    fixture.sessions.remove(T1);
    // 重启后第一次看就发现没了。
    let restarted = fixture.bridge();
    restarted.seed_tokens(&BTreeMap::from([("me".to_string(), "saved".to_string())]));
    restarted.poll_focus(true).await;
    assert_eq!(fixture.last_text(), "专注的会话「重构登录」已经不在了，已退出专注，回到加加。");
    assert!(restarted.state().focus.is_empty());
    let count = fixture.gateway.texts().len();
    restarted.poll_focus(true).await;
    assert_eq!(fixture.gateway.texts().len(), count, "只说一次");

    // 发消息时才发现没了：退出并说明这条没发出去。
    fixture.add_login_thread();
    say(&restarted, "me", "/进入 11111111").await;
    fixture.sessions.remove(T1);
    assert_eq!(say(&restarted, "me", "还在吗").await, Inbound::Handled);
    assert!(fixture.last_text().contains("「重构登录」已经不在了，已退出专注，回到加加。这条没发出去"), "{}", fixture.last_text());
    assert!(restarted.state().focus.is_empty());
}

// ---------- 附件 ----------

#[tokio::test]
async fn focus_forwards_weixin_images_and_reports_failed_downloads() {
    let fixture = Fixture::new().await;
    fixture.add_login_thread();
    Mock::given(method("GET")).and(path("/image")).respond_with(ResponseTemplate::new(200).set_body_bytes(b"\x89PNG\r\n\x1a\nfixture".to_vec())).mount(&fixture.gateway.server).await;
    let bridge = fixture.bridge();
    say(&bridge, "me", "/进入 11111111").await;

    let image = |url: String, text: Option<&str>| -> WeixinMessage {
        let mut items = vec![json!({ "type": 2, "image_item": { "url": url } })];
        if let Some(text) = text {
            items.insert(0, json!({ "type": 1, "text_item": { "text": text } }));
        }
        serde_json::from_value(json!({ "from_user_id": "me", "context_token": "ctx-me", "item_list": items })).unwrap()
    };
    let ok = image(format!("{}/image", fixture.gateway.server.uri()), Some("看看这个报错"));
    assert_eq!(bridge.handle_inbound(&ok, "看看这个报错").await, Inbound::Handled);
    let sent = fixture.sessions.lock().sent.clone();
    assert_eq!(sent.len(), 1);
    let (thread, text, attachments) = &sent[0];
    assert_eq!((thread.as_str(), text.as_str()), (T1, "看看这个报错"));
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].kind, AttachmentKind::Image);
    assert_eq!(std::fs::read(&attachments[0].path).unwrap(), b"\x89PNG\r\n\x1a\nfixture", "图片下载到会话目录交给会话");
    assert!(attachments[0].path.starts_with(fixture.dir.path().join("CodexPlusPlus").canonicalize().unwrap()));

    // 下载失败：不悄悄丢掉，说一声。
    let broken = image("http://127.0.0.1:1/x".into(), None);
    assert_eq!(bridge.handle_inbound(&broken, "").await, Inbound::Handled);
    assert!(fixture.last_text().starts_with("附件没收到，没发给「重构登录」"), "{}", fixture.last_text());
    assert_eq!(fixture.sessions.sent().len(), 1);
}

// ---------- 加加切进专注 ----------

#[tokio::test]
async fn jiajia_focus_request_switches_weixin_into_focus() {
    let fixture = Fixture::new().await;
    fixture.add_login_thread();
    let bridge = fixture.bridge();
    say(&bridge, "me", "帮我进入重构登录那个会话").await;
    fixture.jiajia.lock().focus_request = Some(crate::remote_control::jiajia::FocusRequest { thread_id: T1.into(), title: "重构登录".into(), at: now_ms() });
    bridge.take_focus_request().await;
    assert!(fixture.last_text().starts_with("已进入「重构登录」，之后的消息直接发给它；/退出 回到加加"), "{}", fixture.last_text());
    assert_eq!(bridge.state().focus["me"].thread_id, T1);
    assert_eq!(say(&bridge, "me", "继续改").await, Inbound::Handled);
    assert_eq!(fixture.sessions.sent(), [(T1.to_string(), "继续改".to_string(), 0)]);
    assert_eq!(fixture.jiajia.sent().len(), 1, "专注后不再经过加加");
    // 请求只取一次。
    bridge.take_focus_request().await;
    assert!(fixture.jiajia.lock().focus_request.is_none());
}
