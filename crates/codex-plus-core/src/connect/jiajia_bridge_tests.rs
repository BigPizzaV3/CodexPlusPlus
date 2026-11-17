//! 微信里使用加加：全部用假的加加（[`MockJiajia`]）和 wiremock 假微信网关，不碰真实微信和远程控制。

use super::*;
use crate::connect::weixin::WeixinMessage;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

// ---------- 假的加加 ----------

#[derive(Default)]
struct MockState {
    unavailable: Option<String>,
    messages: Vec<ChatMessage>,
    rev: u64,
    next_id: u64,
    sent: Vec<(String, Option<String>, Role)>,
    /// 会话还挂着的请求（请求 id 的 JSON 文本）。
    live: HashMap<String, Vec<String>>,
    responded: Vec<(String, Value, Value)>,
    resolved: Vec<u64>,
    /// 加加的 focus_thread 留下的请求。
    focus_request: Option<crate::remote_control::jiajia::FocusRequest>,
}

struct MockJiajia(Mutex<MockState>);

impl MockJiajia {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(MockState { next_id: 1, ..Default::default() })))
    }

    fn lock(&self) -> MutexGuard<'_, MockState> {
        self.0.lock().unwrap()
    }

    fn set_unavailable(&self, reason: Option<&str>) {
        self.lock().unavailable = reason.map(str::to_string);
    }

    /// 像真的加加一样往聊天记录里追加一条。
    fn push(&self, role: &str, text: &str, by: Option<&str>, request: Option<Value>) -> u64 {
        let mut state = self.lock();
        let id = state.next_id;
        state.next_id += 1;
        state.rev += 1;
        let rev = state.rev;
        state.messages.push(ChatMessage {
            id,
            at: 0,
            role: role.into(),
            text: text.into(),
            threads: vec![],
            request,
            resolved: false,
            by: by.map(str::to_string),
            rev,
        });
        id
    }

    /// 贴一张审批卡片（结构同加加 `route()` 里贴的），并把请求登记成「还挂着」。
    fn push_card(&self, thread_id: &str, title: &str, request: Value) -> u64 {
        let key = request["id"].to_string();
        self.lock().live.entry(thread_id.to_string()).or_default().push(key);
        let text = events::request_summary(&request);
        self.push("notice", &text, None, Some(json!({ "threadId": thread_id, "title": title, "request": request })))
    }

    /// 卡片在网页上被处理了。
    fn resolve_elsewhere(&self, id: u64) {
        let mut state = self.lock();
        state.rev += 1;
        let rev = state.rev;
        if let Some(message) = state.messages.iter_mut().find(|message| message.id == id) {
            message.resolved = true;
            message.rev = rev;
        }
    }

    fn sent(&self) -> Vec<(String, Option<String>, Role)> {
        self.lock().sent.clone()
    }
}

#[async_trait]
impl JiajiaPort for MockJiajia {
    async fn unavailable_reason(&self) -> Option<String> {
        self.lock().unavailable.clone()
    }

    fn cursor(&self) -> Option<(usize, u64, u64)> {
        let state = self.lock();
        if state.unavailable.is_some() {
            return None;
        }
        Some((1, state.rev, state.messages.last().map(|message| message.id).unwrap_or(0)))
    }

    fn changed_since(&self, rev: u64) -> Vec<ChatMessage> {
        self.lock().messages.iter().filter(|message| message.rev > rev).cloned().collect()
    }

    async fn wait_changed(&self, timeout: Duration) {
        tokio::time::sleep(timeout.min(Duration::from_millis(20))).await;
    }

    async fn send(&self, text: &str, by: Option<String>, role: Role) -> anyhow::Result<()> {
        self.lock().sent.push((text.to_string(), by.clone(), role));
        self.push("user", text, by.as_deref(), None);
        Ok(())
    }

    async fn live_requests(&self, thread_id: &str) -> anyhow::Result<Vec<String>> {
        Ok(self.lock().live.get(thread_id).cloned().unwrap_or_default())
    }

    async fn respond(&self, thread_id: &str, request_id: &Value, body: &Value) -> anyhow::Result<()> {
        let mut state = self.lock();
        state.responded.push((thread_id.to_string(), request_id.clone(), body.clone()));
        let key = request_id.to_string();
        if let Some(live) = state.live.get_mut(thread_id) {
            live.retain(|existing| existing != &key);
        }
        Ok(())
    }

    fn resolve_card(&self, message_id: u64) {
        self.lock().resolved.push(message_id);
    }

    fn take_focus_request(&self) -> Option<crate::remote_control::jiajia::FocusRequest> {
        self.lock().focus_request.take()
    }
}

// ---------- 假的微信网关 ----------

struct Gateway {
    server: MockServer,
    /// 发送成功的文字（失败的不算）。
    texts: Arc<Mutex<Vec<String>>>,
    /// 发出去的 context_token。
    tokens: Arc<Mutex<Vec<String>>>,
    fail: Arc<AtomicBool>,
    typing: Arc<Mutex<Vec<i64>>>,
}

impl Gateway {
    async fn start() -> Self {
        let server = MockServer::start().await;
        let texts = Arc::new(Mutex::new(Vec::new()));
        let tokens = Arc::new(Mutex::new(Vec::new()));
        let fail = Arc::new(AtomicBool::new(false));
        let typing = Arc::new(Mutex::new(Vec::new()));
        let (sent, used, failing) = (texts.clone(), tokens.clone(), fail.clone());
        Mock::given(method("POST"))
            .and(path("/ilink/bot/sendmessage"))
            .respond_with(move |request: &Request| {
                if failing.load(Ordering::SeqCst) {
                    // token 失效时网关的样子：HTTP 200 但 ret 非 0。
                    return ResponseTemplate::new(200).set_body_json(json!({ "ret": -14, "errmsg": "session timeout" }));
                }
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                sent.lock().unwrap().push(body["msg"]["item_list"][0]["text_item"]["text"].as_str().unwrap_or_default().to_string());
                used.lock().unwrap().push(body["msg"]["context_token"].as_str().unwrap_or_default().to_string());
                ResponseTemplate::new(200).set_body_json(json!({ "ret": 0 }))
            })
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/ilink/bot/getconfig"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ret": 0, "typing_ticket": "ticket" })))
            .mount(&server)
            .await;
        let statuses = typing.clone();
        Mock::given(method("POST"))
            .and(path("/ilink/bot/sendtyping"))
            .respond_with(move |request: &Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                statuses.lock().unwrap().push(body["status"].as_i64().unwrap_or_default());
                ResponseTemplate::new(200).set_body_json(json!({ "ret": 0 }))
            })
            .mount(&server)
            .await;
        Self { server, texts, tokens, fail, typing }
    }

    fn client(&self) -> WeixinClient {
        // 附件也从假网关下载（专注模式转附件的用例）。
        WeixinClient::new(&self.server.uri(), "fixture-token", "").unwrap().with_media_base_url(&self.server.uri()).unwrap()
    }

    fn texts(&self) -> Vec<String> {
        self.texts.lock().unwrap().clone()
    }
}

type Logs = Arc<Mutex<Vec<(String, Value)>>>;

struct Fixture {
    gateway: Gateway,
    jiajia: Arc<MockJiajia>,
    sessions: Arc<focus_tests::MockSessions>,
    dir: tempfile::TempDir,
    logs: Logs,
    settings: WeixinCodexPath,
}

impl Fixture {
    async fn new() -> Self {
        Self {
            gateway: Gateway::start().await,
            jiajia: MockJiajia::new(),
            sessions: focus_tests::MockSessions::new(),
            dir: tempfile::tempdir().unwrap(),
            logs: Arc::new(Mutex::new(Vec::new())),
            settings: WeixinCodexPath::new(""),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("state-jiajia.json")
    }

    fn bridge_with(&self, owner: Option<&str>, default_mode: PeerMode, push: bool) -> JiajiaBridge {
        let logs = self.logs.clone();
        JiajiaBridge::new(
            self.jiajia.clone(),
            self.gateway.client(),
            self.path(),
            owner.map(str::to_string),
            self.settings.clone(),
            BridgeSettings { default_mode, push },
            Arc::new(move |event, detail| logs.lock().unwrap().push((event.to_string(), detail))),
        )
        .with_focus(self.sessions.clone(), &self.dir.path().display().to_string())
    }

    fn bridge(&self) -> JiajiaBridge {
        self.bridge_with(Some("me"), PeerMode::Jiajia, false)
    }

    fn logs(&self) -> Vec<(String, Value)> {
        self.logs.lock().unwrap().clone()
    }
}

fn message(peer: &str, text: &str, token: &str) -> WeixinMessage {
    serde_json::from_value(json!({
        "from_user_id": peer,
        "context_token": token,
        "item_list": [{ "type": 1, "text_item": { "text": text } }]
    }))
    .unwrap()
}

async fn say(bridge: &JiajiaBridge, peer: &str, text: &str) -> Inbound {
    bridge.handle_inbound(&message(peer, text, &format!("ctx-{peer}")), text).await
}

fn command_request(id: Value, decisions: Value) -> Value {
    json!({
        "id": id,
        "method": "item/commandExecution/requestApproval",
        "params": { "command": ["cargo", "test"], "availableDecisions": decisions }
    })
}

// ---------- 解析 ----------

#[test]
fn commands_are_parsed_in_chinese_english_and_fullwidth_slash() {
    assert_eq!(parse_command("/加加"), Some(Command::Jiajia));
    assert_eq!(parse_command(" ／直连 "), Some(Command::Direct));
    assert_eq!(parse_command("/状态"), Some(Command::Status));
    assert_eq!(parse_command("/JiaJia"), Some(Command::Jiajia));
    assert_eq!(parse_command("/direct"), Some(Command::Direct));
    assert_eq!(parse_command("/status"), Some(Command::Status));
    assert_eq!(parse_command("加加"), None);
    assert_eq!(parse_command("/reset"), None);
    assert_eq!(parse_command("/加加 帮我看看"), None);
}

#[test]
fn approval_replies_accept_digits_keywords_and_card_numbers() {
    let reply = |choice, card| Some(ApprovalReply { choice, card });
    assert_eq!(parse_approval("1"), reply(Choice::Index(1), None));
    assert_eq!(parse_approval(" 2 #3 "), reply(Choice::Index(2), Some(3)));
    assert_eq!(parse_approval("#3 2"), reply(Choice::Index(2), Some(3)));
    assert_eq!(parse_approval("1#12"), reply(Choice::Index(1), Some(12)));
    assert_eq!(parse_approval("同意"), reply(Choice::Accept, None));
    assert_eq!(parse_approval("拒绝 ＃2"), reply(Choice::Decline, Some(2)));
    assert_eq!(parse_approval("本会话都同意"), reply(Choice::AcceptForSession, None));
    for text in ["0", "123", "1 2", "# 1", "1 #2 #3", "我同意", "1 个文件", "", "#2"] {
        assert_eq!(parse_approval(text), None, "{text}");
    }
}

#[test]
fn owner_is_linked_user_or_the_single_allowlisted_peer() {
    assert_eq!(owner_peer(" me ", "a,b"), Some("me".into()));
    assert_eq!(owner_peer("", "a@im.wechat"), Some("a@im.wechat".into()));
    assert_eq!(owner_peer("", " a , b "), None);
    assert_eq!(owner_peer("", "*"), None);
    assert_eq!(owner_peer("", ""), None);
    assert_eq!(PeerMode::parse(""), PeerMode::Jiajia);
    assert_eq!(PeerMode::parse("direct"), PeerMode::Direct);
    assert_eq!(PeerMode::parse("whatever"), PeerMode::Jiajia);
}

#[test]
fn card_text_lists_the_options_the_request_supports() {
    let request = command_request(json!(7), json!(["accept", "acceptForSession", "decline"]));
    let options = card_options(&request);
    let card = CardRef { no: 2, message_id: 9, thread_id: "t".into(), title: "修 bug".into(), request_id: json!(7), options, focus: false };
    let text = card_text(&card, &request, 0);
    assert!(text.starts_with("⚠️ #2 会话「修 bug」等你审批：要运行命令 `cargo test`"), "{text}");
    assert!(text.ends_with("回复 1 同意 · 2 本会话都同意 · 3 拒绝"), "{text}");
    assert!(card_text(&card, &request, 1).contains("如「1 #2」"));
    // 没给选项：默认同意 / 拒绝；只有 cancel 时不再补拒绝；对象形式的选项（网页也不显示）跳过。
    assert_eq!(card_options(&command_request(json!(1), Value::Null)).len(), 2);
    let labels: Vec<String> = card_options(&command_request(json!(1), json!(["accept", { "x": 1 }, "cancel"]))).into_iter().map(|option| option.label).collect();
    assert_eq!(labels, ["同意", "拒绝并停止"]);
    let question = json!({ "id": 3, "method": "item/tool/requestUserInput", "params": { "questions": [{ "id": "q", "question": "用哪个方案？", "options": [{ "label": "A" }, { "label": "B" }] }] } });
    let options = card_options(&question);
    assert_eq!(options[1].body, json!({ "answers": { "q": { "answers": ["B"] } } }));
    assert!(card_options(&json!({ "method": "item/permissions/requestApproval" })).is_empty());
}

// ---------- 模式 ----------

#[tokio::test]
async fn default_mode_uses_jiajia_when_available_and_falls_back_once_when_not() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    assert_eq!(say(&bridge, "me", "你好").await, Inbound::Handled);
    assert_eq!(fixture.jiajia.sent(), [("你好".to_string(), Some("微信".to_string()), Role::Full)]);
    assert_eq!(fixture.gateway.typing.lock().unwrap().as_slice(), [1], "交给加加后显示正在输入");

    fixture.jiajia.set_unavailable(Some("远程控制没开"));
    assert_eq!(say(&bridge, "me", "第二条").await, Inbound::Direct);
    assert_eq!(say(&bridge, "me", "第三条").await, Inbound::Direct);
    let texts = fixture.gateway.texts();
    assert_eq!(texts.len(), 1, "只说明一次：{texts:?}");
    assert!(texts[0].contains("远程控制没开") && texts[0].contains("先直接交给 Codex"), "{texts:?}");
    assert!(fixture.bridge().state().fallback_noticed.contains("me"), "说明过的记录落盘，重启后不再重复");

    // 恢复可用后照常走加加；下次再不可用会重新说明一次。
    fixture.jiajia.set_unavailable(None);
    assert_eq!(say(&bridge, "me", "第四条").await, Inbound::Handled);
    assert!(bridge.state().fallback_noticed.is_empty());
    assert_eq!(fixture.jiajia.sent().len(), 2);
}

#[tokio::test]
async fn explicit_choice_is_remembered_and_overrides_the_default() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    assert_eq!(say(&bridge, "me", "/直连").await, Inbound::Handled);
    assert!(fixture.gateway.texts()[0].contains("已切到直连"));
    assert_eq!(say(&bridge, "me", "跑一下测试").await, Inbound::Direct);
    assert!(fixture.jiajia.sent().is_empty(), "命令和直连消息都不发给加加");

    // 选择持久化：重启后还是直连。
    let restarted = fixture.bridge();
    assert_eq!(say(&restarted, "me", "还是直连").await, Inbound::Direct);
    assert_eq!(say(&restarted, "me", "/jiajia").await, Inbound::Handled);
    assert_eq!(say(&restarted, "me", "现在给加加").await, Inbound::Handled);
    assert_eq!(fixture.jiajia.sent().len(), 1);

    // 默认模式是直连时，没选过的本人走直连；管理器保存的设置即时覆盖启动时的值。
    let other = Fixture::new().await;
    let direct = other.bridge_with(Some("me"), PeerMode::Direct, false);
    assert_eq!(say(&direct, "me", "hi").await, Inbound::Direct);
    other.settings.set_defaults(crate::connect::SessionDefaults {
        model: String::new(),
        reasoning_effort: String::new(),
        project_id: String::new(),
        work_dir: String::new(),
        sandbox: "read-only".into(),
        default_mode: "jiajia".into(),
        push_to_weixin: false,
    });
    assert_eq!(say(&direct, "me", "hi").await, Inbound::Handled);
}

#[tokio::test]
async fn reset_commands_still_go_to_the_direct_session_in_jiajia_mode() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    assert_eq!(say(&bridge, "me", "/reset").await, Inbound::Direct);
    assert!(fixture.jiajia.sent().is_empty());
}

#[tokio::test]
async fn non_owner_contacts_cannot_use_jiajia() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    assert_eq!(say(&bridge, "friend", "你好").await, Inbound::Direct);
    assert_eq!(say(&bridge, "friend", "/加加").await, Inbound::Handled);
    assert!(fixture.gateway.texts()[0].contains("只对扫码绑定的本人开放"));
    assert!(!bridge.state().peer_modes.contains_key("friend"));
    assert_eq!(say(&bridge, "friend", "再试").await, Inbound::Direct);
    assert!(fixture.jiajia.sent().is_empty());

    // 认不出本人时谁都不能用，并提示重新扫码。
    let unknown = fixture.bridge_with(None, PeerMode::Jiajia, false);
    assert_eq!(say(&unknown, "me", "hi").await, Inbound::Direct);
    assert_eq!(say(&unknown, "me", "/加加").await, Inbound::Handled);
    assert!(fixture.gateway.texts().last().unwrap().contains("重新扫码"));
    assert!(fixture.jiajia.sent().is_empty());
}

#[tokio::test]
async fn status_reports_mode_availability_and_push() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    say(&bridge, "me", "/状态").await;
    let text = fixture.gateway.texts().pop().unwrap();
    assert!(text.contains("当前模式：加加（默认）"), "{text}");
    assert!(text.contains("加加：可用"), "{text}");
    assert!(text.contains("推送到微信：关"), "{text}");
    fixture.jiajia.set_unavailable(Some("远程控制没开"));
    say(&bridge, "me", "/status").await;
    let text = fixture.gateway.texts().pop().unwrap();
    assert!(text.contains("先按直连处理") && text.contains("不可用（远程控制没开）"), "{text}");
    say(&bridge, "friend", "/状态").await;
    assert!(fixture.gateway.texts().pop().unwrap().contains("加加只对扫码绑定的本人开放"));
}

#[tokio::test]
async fn attachments_sent_to_jiajia_get_a_text_only_note() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    let image: WeixinMessage = serde_json::from_value(json!({
        "from_user_id": "me", "context_token": "ctx",
        "item_list": [{ "type": 2, "image_item": { "url": "http://127.0.0.1:1/x" } }]
    }))
    .unwrap();
    assert_eq!(bridge.handle_inbound(&image, "").await, Inbound::Handled);
    assert!(fixture.gateway.texts()[0].contains("加加暂时只看文字，切到 /直连 可以发图片"));
    assert!(fixture.jiajia.sent().is_empty());
}

// ---------- 转发 ----------

#[tokio::test]
async fn forwarder_sends_jiajia_replies_and_cards_but_not_user_messages() {
    let fixture = Fixture::new().await;
    fixture.jiajia.push("bot", "很早以前的回复", None, None);
    let bridge = fixture.bridge();
    bridge.forward_once().await;
    assert!(fixture.gateway.texts().is_empty(), "第一次只记位置，不重放旧记录");

    say(&bridge, "me", "帮我看看会话").await;
    fixture.jiajia.push("bot", "都在跑，没什么事", None, None);
    fixture.jiajia.push("user", "网页上说的话", Some("本机浏览器"), None);
    fixture.jiajia.push("bot", "回网页的话", None, None);
    fixture.jiajia.push("event", "「部署」跑完了", None, None);
    fixture.jiajia.push_card("thread-1", "部署", command_request(json!(7), json!(["accept", "decline"])));
    bridge.forward_once().await;
    let texts = fixture.gateway.texts();
    assert_eq!(texts.len(), 3, "{texts:?}");
    assert_eq!(texts[0], "都在跑，没什么事");
    assert_eq!(texts[1], "回网页的话", "正在对话时网页上的对话也同步（共用聊天记录）");
    assert!(texts[2].starts_with("⚠️ #1 会话「部署」等你审批") && texts[2].contains("回复 1 同意 · 2 拒绝"), "{texts:?}");
    assert!(!texts.iter().any(|text| text.contains("网页上说的话") || text.contains("帮我看看会话") || text.contains("「部署」跑完了")));
    assert!(fixture.gateway.tokens.lock().unwrap().iter().all(|token| token == "ctx-me"), "用本人最近一次的 token");
    assert_eq!(fixture.gateway.typing.lock().unwrap().as_slice(), [1, 2], "回复送到后收起正在输入");
    let logs = fixture.logs();
    assert!(logs.iter().all(|(event, detail)| event == "weixin.jiajia_push" && detail["ok"] == true), "{logs:?}");
    assert_eq!(bridge.state().cards.len(), 1);
}

#[tokio::test]
async fn card_digits_map_to_respond_and_handled_requests_are_rejected() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    bridge.forward_once().await;
    say(&bridge, "me", "开始").await;
    let first = fixture.jiajia.push_card("thread-1", "修 bug", command_request(json!(7), json!(["accept", "acceptForSession", "decline"])));
    bridge.forward_once().await;
    assert_eq!(say(&bridge, "me", "2").await, Inbound::Handled);
    assert_eq!(fixture.jiajia.lock().responded, [("thread-1".to_string(), json!(7), json!({ "decision": "acceptForSession" }))]);
    assert_eq!(fixture.jiajia.lock().resolved, [first]);
    assert!(fixture.gateway.texts().last().unwrap().contains("已回复「本会话都同意」：#1 会话「修 bug」"));
    assert!(bridge.state().cards.is_empty());

    // 新卡片复用空出来的编号；请求已经不挂着了（在桌面处理过）：不发，回「已经处理过了」。
    let second = fixture.jiajia.push_card("thread-2", "部署", command_request(json!("req-9"), json!(["accept", "decline"])));
    fixture.jiajia.lock().live.clear();
    bridge.forward_once().await;
    assert!(fixture.gateway.texts().last().unwrap().starts_with("⚠️ #1 会话「部署」"));
    assert_eq!(say(&bridge, "me", "同意").await, Inbound::Handled);
    assert_eq!(fixture.gateway.texts().last().unwrap(), "这个请求已经处理过了。");
    assert_eq!(fixture.jiajia.lock().responded.len(), 1);
    assert_eq!(fixture.jiajia.lock().resolved, [first, second]);

    // 没有卡片时数字就是普通消息，照常交给加加。
    assert_eq!(say(&bridge, "me", "1").await, Inbound::Handled);
    assert_eq!(fixture.jiajia.sent().last().unwrap().0, "1");
}

#[tokio::test]
async fn several_pending_cards_are_told_apart_by_number() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    bridge.forward_once().await;
    say(&bridge, "me", "开始").await;
    fixture.jiajia.push_card("thread-1", "一号", command_request(json!(1), json!(["accept", "decline"])));
    fixture.jiajia.push_card("thread-2", "二号", command_request(json!(2), json!(["accept", "cancel"])));
    bridge.forward_once().await;
    assert!(fixture.gateway.texts().last().unwrap().contains("同时还有 1 张待处理，回复时带上编号，如「1 #2」"));

    say(&bridge, "me", "1").await;
    assert!(fixture.gateway.texts().last().unwrap().contains("现在有 2 张待处理的卡片（#1 #2）"));
    assert!(fixture.jiajia.lock().responded.is_empty());
    say(&bridge, "me", "1 #5").await;
    assert!(fixture.gateway.texts().last().unwrap().contains("没有编号 #5"));
    say(&bridge, "me", "3 #2").await;
    assert!(fixture.gateway.texts().last().unwrap().contains("卡片 #2 可以回复：1 同意 · 2 拒绝并停止"));

    say(&bridge, "me", "拒绝 #2").await;
    say(&bridge, "me", "1").await;
    let responded = fixture.jiajia.lock().responded.clone();
    assert_eq!(responded[0], ("thread-2".to_string(), json!(2), json!({ "decision": "cancel" })), "只有 cancel 时「拒绝」就是它");
    assert_eq!(responded[1], ("thread-1".to_string(), json!(1), json!({ "decision": "accept" })), "只剩一张时不用带编号");
}

/// 卡片之后加加又说了话（比如问「选 1 还是 2」）：光秃秃的数字是在回答加加，要带编号才算审批。
#[tokio::test]
async fn bare_digits_only_approve_right_after_a_card() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    bridge.forward_once().await;
    say(&bridge, "me", "开始").await;
    fixture.jiajia.push_card("thread-1", "一号", command_request(json!(1), json!(["accept", "decline"])));
    fixture.jiajia.push("bot", "另外，你想先修 1 还是 2？", None, None);
    bridge.forward_once().await;
    assert_eq!(say(&bridge, "me", "2").await, Inbound::Handled);
    assert_eq!(fixture.jiajia.sent().last().unwrap().0, "2", "数字交给加加");
    assert!(fixture.jiajia.lock().responded.is_empty());
    assert_eq!(say(&bridge, "me", "1 #1").await, Inbound::Handled);
    assert_eq!(fixture.jiajia.lock().responded, [("thread-1".to_string(), json!(1), json!({ "decision": "accept" }))]);

    // 本人说了别的之后也一样。
    fixture.jiajia.push_card("thread-2", "二号", command_request(json!(2), json!(["accept", "decline"])));
    bridge.forward_once().await;
    say(&bridge, "me", "先别管它").await;
    say(&bridge, "me", "1").await;
    assert_eq!(fixture.jiajia.lock().responded.len(), 1);
    assert_eq!(fixture.jiajia.sent().last().unwrap().0, "1");
}

#[tokio::test]
async fn cards_handled_on_the_web_are_dropped_from_weixin() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    bridge.forward_once().await;
    say(&bridge, "me", "开始").await;
    let card = fixture.jiajia.push_card("thread-1", "一号", command_request(json!(1), json!(["accept", "decline"])));
    bridge.forward_once().await;
    assert_eq!(bridge.state().cards.len(), 1);
    fixture.jiajia.resolve_elsewhere(card);
    bridge.forward_once().await;
    assert!(bridge.state().cards.is_empty());
    let sent_before = fixture.gateway.texts().len();
    assert_eq!(say(&bridge, "me", "1").await, Inbound::Handled);
    assert_eq!(fixture.jiajia.sent().last().unwrap().0, "1", "卡片没了，数字照常发给加加");
    assert_eq!(fixture.gateway.texts().len(), sent_before, "网页处理卡片不在微信刷屏");
}

#[tokio::test]
async fn nothing_is_pushed_outside_a_conversation_when_push_is_off() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge();
    bridge.seed_tokens(&BTreeMap::from([("me".to_string(), "saved".to_string())]));
    bridge.forward_once().await;
    fixture.jiajia.push("event", "「部署」出错了", None, None);
    fixture.jiajia.push("bot", "部署出错了，原因是…", None, None);
    fixture.jiajia.push("notice", "「部署」出错了：网络断了", None, None);
    fixture.jiajia.push_card("thread-1", "部署", command_request(json!(1), json!(["accept", "decline"])));
    bridge.forward_once().await;
    assert!(fixture.gateway.texts().is_empty());
    assert!(bridge.state().cards.is_empty());
}

#[tokio::test]
async fn push_in_direct_mode_queues_failures_and_backfills_on_the_next_message() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge_with(Some("me"), PeerMode::Direct, true);
    bridge.seed_tokens(&BTreeMap::from([("me".to_string(), "saved-token".to_string())]));
    bridge.forward_once().await;

    // 推送开着：直连模式也收卡片和提醒；网页上的对话不推，事件引出的加加回复推。
    fixture.jiajia.push("user", "网页问的", Some("本机浏览器"), None);
    fixture.jiajia.push("bot", "网页的回答", None, None);
    fixture.jiajia.push("event", "「部署」跑完了", None, None);
    fixture.jiajia.push("bot", "部署跑完了，一切正常", None, None);
    bridge.forward_once().await;
    assert_eq!(fixture.gateway.texts(), ["部署跑完了，一切正常"]);
    assert_eq!(fixture.gateway.tokens.lock().unwrap().as_slice(), ["saved-token"]);

    // token 失效：发送失败存起来，之后的推送不再硬试。
    fixture.gateway.fail.store(true, Ordering::SeqCst);
    let card = fixture.jiajia.push_card("thread-1", "部署", command_request(json!(5), json!(["accept", "decline"])));
    fixture.jiajia.push("notice", "「测试」出错了：断言失败", None, None);
    bridge.forward_once().await;
    let state = fixture.bridge().state();
    assert_eq!(state.pending.len(), 2, "待补发的推送落盘");
    assert_eq!(state.pending[0].card, Some(card));
    let attempts = fixture.gateway.server.received_requests().await.unwrap().iter().filter(|request| request.url.path() == "/ilink/bot/sendmessage").count();
    assert_eq!(attempts, 2, "失败一次后不再用同一个 token 硬试");
    let logs = fixture.logs();
    let failures: Vec<&Value> = logs.iter().filter(|(_, detail)| detail["ok"] == false).map(|(_, detail)| detail).collect();
    assert_eq!(failures[0]["reason"], "sendFailed");
    assert_eq!(failures[1]["reason"], "tokenFailed");
    let logged = serde_json::to_string(&logs).unwrap();
    assert!(!logged.contains("saved-token") && !logged.contains("断言失败") && !logged.contains("cargo test"), "日志不记 token 和正文：{logged}");

    // 本人下次发消息：先补发「你不在时」，再照常直连处理这条消息；补发里的卡片照样能审批。
    fixture.gateway.fail.store(false, Ordering::SeqCst);
    assert_eq!(say(&bridge, "me", "我回来了").await, Inbound::Direct);
    let backfill = fixture.gateway.texts().pop().unwrap();
    assert!(backfill.starts_with("你不在时：") && backfill.contains("⚠️ #1 会话「部署」") && backfill.contains("🔔 「测试」出错了"), "{backfill}");
    assert_eq!(fixture.gateway.tokens.lock().unwrap().last().unwrap(), "ctx-me", "补发用新 token");
    assert!(bridge.state().pending.is_empty());
    assert_eq!(say(&bridge, "me", "1").await, Inbound::Handled);
    assert_eq!(fixture.jiajia.lock().responded, [("thread-1".to_string(), json!(5), json!({ "decision": "accept" }))]);
    assert!(fixture.logs().iter().any(|(event, detail)| event == "weixin.jiajia_backlog" && detail["ok"] == true));
}

#[tokio::test]
async fn backfill_skips_cards_that_were_handled_meanwhile() {
    let fixture = Fixture::new().await;
    let bridge = fixture.bridge_with(Some("me"), PeerMode::Direct, true);
    bridge.forward_once().await;
    // 还没收到过本人的消息（没有 token）：直接存起来。
    let card = fixture.jiajia.push_card("thread-1", "部署", command_request(json!(5), json!(["accept", "decline"])));
    bridge.forward_once().await;
    assert_eq!(fixture.logs()[0].1["reason"], "noToken");
    fixture.jiajia.resolve_elsewhere(card);
    bridge.forward_once().await;
    assert_eq!(say(&bridge, "me", "在吗").await, Inbound::Direct);
    assert!(fixture.gateway.texts().is_empty(), "卡片已在别处处理，不再补发");
    assert!(bridge.state().pending.is_empty());
}

// ---------- 接进真实的消息循环 ----------

/// 用假微信网关驱动真实的连接循环：本人的消息交给加加（不起 Codex），加加的回复由后台任务发回微信。
#[tokio::test]
async fn connect_loop_routes_owner_messages_to_jiajia_and_forwards_replies() {
    let fixture = Fixture::new().await;
    let now = super::super::now_ms();
    let updates = json!({
        "ret": 0,
        "msgs": [{
            "seq": 1, "message_id": 42, "from_user_id": "me", "client_id": "loop-test",
            "create_time_ms": now as i64, "message_type": 1, "message_state": 2,
            "item_list": [{ "type": 1, "text_item": { "text": "帮我看看会话" } }],
            "context_token": "loop-ctx"
        }],
        "get_updates_buf": "YnVm",
        "longpolling_timeout_ms": 1000
    });
    Mock::given(method("POST"))
        .and(path("/ilink/bot/getupdates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(updates).set_delay(Duration::from_millis(200)))
        .mount(&fixture.gateway.server)
        .await;
    let config = crate::connect::WeixinConnectConfig {
        base_url: fixture.gateway.server.uri(),
        token: "fixture-token".into(),
        account_id: "loop-test".into(),
        linked_user_id: "me".into(),
        work_dir: fixture.dir.path().display().to_string(),
        ..Default::default()
    };
    let status: crate::connect::SharedWeixinConnectStatus = Arc::new(Mutex::new(Default::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let logs = fixture.logs.clone();
    let task = tokio::spawn(crate::connect::run_weixin_connect_with_jiajia(
        config,
        stop.clone(),
        status.clone(),
        WeixinCodexPath::new(""),
        crate::connect::outbound::OutboundPolicy::new(false, "", ""),
        crate::connect::session_store::ConnectSessionStore::new(fixture.dir.path().join("connect-state.json")),
        fixture.jiajia.clone(),
        Arc::new(move |event, detail| logs.lock().unwrap().push((event.to_string(), detail))),
    ));
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while fixture.jiajia.sent().is_empty() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(fixture.jiajia.sent(), [("帮我看看会话".to_string(), Some("微信".to_string()), Role::Full)], "同一条消息只处理一次");
    fixture.jiajia.push("bot", "两个会话都在跑", None, None);
    while !fixture.gateway.texts().iter().any(|text| text == "两个会话都在跑") && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    stop.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap().unwrap();
    assert_eq!(fixture.gateway.texts(), ["两个会话都在跑"]);
    assert_eq!(fixture.gateway.tokens.lock().unwrap().as_slice(), ["loop-ctx"]);
    assert_eq!(status.lock().unwrap().processed_messages, 1);
}

#[path = "jiajia_focus_tests.rs"]
mod focus_tests;
