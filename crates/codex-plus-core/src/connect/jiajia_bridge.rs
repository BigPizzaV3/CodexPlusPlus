//! 微信里使用加加（#1809）。
//!
//! 每个微信联系人有两种模式：
//! - `direct`（直连）：原来的行为，消息直接交给这个联系人自己的 Codex 会话；
//! - `jiajia`（加加）：消息交给远程控制里的加加，和网页上是同一份聊天记录。
//!
//! 没自己选过的联系人按设置里的「微信默认模式」（默认加加）；加加不可用（远程控制没开、
//! 加加还没选模型）时自动退回直连，并在这次不可用期间第一次退回时说一句。
//! 只有扫码绑定的本人能用加加（按网页端「本人」的完全控制权限发给加加），其他联系人只能直连。
//!
//! 加加的回复是异步的：后台转发任务（[`JiajiaBridge::run_forwarder`]）盯着加加的聊天记录，
//! 把新的回复和审批卡片用本人最近一次的 `context_token` 发回微信；网页端用户自己发的话不回显。
//! 审批卡片转成文字（带编号 `#n`），回复数字或「同意 / 拒绝」即可审批，发出前先核对请求还挂着。
//! 「推送到微信」打开后，没在对话时加加的卡片和提醒也会推过来（直连模式也可以）；
//! 发送失败（token 失效等）就先存起来，本人下次发消息时补发「你不在时：…」。
//!
//! 微信连接和远程控制在同一个进程：经 [`crate::remote_control::hosted_service`] 直接调用，不走 HTTP。
//! 对加加的调用都隔着 [`JiajiaPort`]，测试里换成假的。
//!
//! 另有「专注模式」（[`focus`]）：本人钉住桌面上的一个会话，消息原样发给它、回复原样转回来，不经过加加。

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::WeixinCodexPath;
use super::weixin::{WeixinClient, WeixinMessage};
use crate::remote_control::devices::Role;
use crate::remote_control::jiajia::events;
use crate::remote_control::jiajia::store::ChatMessage;
use crate::remote_control::service::RemoteService;

/// 微信里发给加加时记的发送方（网页上显示「微信」），转发时也靠它认出「本人在微信里说的话」。
pub const SENDER_NAME: &str = crate::remote_control::jiajia::WEIXIN_SENDER;
/// 本人在微信里和加加说过话后，多长时间内算「正在对话」：这段时间里网页上和加加的对话、卡片也同步到微信。
const CONVERSATION_WINDOW_MS: i64 = 10 * 60 * 1000;
/// 发不出去的推送最多存多少条、每条最多多少字。
const PENDING_LIMIT: usize = 30;
const PENDING_TEXT_LIMIT: usize = 1200;
/// 「正在输入」最多挂多久（加加一直不回也要收起来）。
const TYPING_LIMIT_MS: i64 = 90_000;
/// 转发任务等加加新消息的最长间隔（也是加加不可用时重新检查的间隔）。
const FORWARD_POLL: Duration = Duration::from_millis(1500);

// ---------- 模式与设置 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PeerMode {
    Direct,
    Jiajia,
}

impl PeerMode {
    /// 只认 `direct`，其它（含空值、旧版本没有这个字段）都按默认的加加。
    pub fn parse(text: &str) -> Self {
        match text.trim() {
            "direct" => Self::Direct,
            _ => Self::Jiajia,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Jiajia => "jiajia",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Direct => "直连",
            Self::Jiajia => "加加",
        }
    }
}

/// 管理器里能改、改了即时生效的两项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeSettings {
    pub default_mode: PeerMode,
    pub push: bool,
}

/// 扫码绑定的本人：优先用扫码时记下的用户 ID；老版本没记的，白名单里只有一个人时就是他。
/// 认不出来时谁都不能用加加（会提示重新扫码）。
pub fn owner_peer(linked_user_id: &str, allow_from: &str) -> Option<String> {
    let linked = linked_user_id.trim();
    if !linked.is_empty() {
        return Some(linked.to_string());
    }
    let ids: Vec<&str> = allow_from.split(',').map(str::trim).filter(|id| !id.is_empty()).collect();
    match ids.as_slice() {
        [only] if *only != "*" => Some((*only).to_string()),
        _ => None,
    }
}

// ---------- 命令与审批回复 ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Jiajia,
    Direct,
    Status,
    /// 专注模式：列最近的会话。
    Sessions,
    /// 专注到列表里的第 N 个（或会话 ID 前几位）。参数可能为空（回用法）。
    Focus(String),
    /// 换到另一个会话（同 Focus）。
    Switch(String),
    /// 新开会话并专注：`[项目] 任务`。
    NewSession(String),
    /// 退出专注。
    Exit,
}

/// 命令字 → 是否带参数。中文命令带数字参数时可以不加空格（`/进入3`）。
const COMMANDS: &[(&str, bool)] = &[
    ("加加", false),
    ("jiajia", false),
    ("直连", false),
    ("direct", false),
    ("状态", false),
    ("status", false),
    ("会话", false),
    ("sessions", false),
    ("进入", true),
    ("focus", true),
    ("切换", true),
    ("switch", true),
    ("新会话", true),
    ("new", true),
    ("退出", false),
    ("exit", false),
];

/// `/加加` `/直连` `/状态` `/会话` `/进入 N` `/切换 N` `/新会话 [项目] 任务` `/退出`
/// （也认英文 `/jiajia` `/direct` `/status` `/sessions` `/focus` `/switch` `/new` `/exit` 和全角斜杠）。命令不发给模型。
/// 不带参数的命令后面跟了别的字就不算命令（「/加加 帮我看看」照常当消息）。
pub fn parse_command(text: &str) -> Option<Command> {
    let text = text.trim();
    let rest = text.strip_prefix('/').or_else(|| text.strip_prefix('／'))?.trim();
    for (word, takes_arg) in COMMANDS {
        // 命令字按 ASCII 忽略大小写比较；参数保留原文。
        let Some(head) = rest.get(..word.len()).filter(|head| head.eq_ignore_ascii_case(word)) else { continue };
        let after = &rest[head.len()..];
        let arg = if after.is_empty() {
            ""
        } else if after.starts_with(char::is_whitespace) || (*takes_arg && !word.is_ascii() && after.starts_with(|c: char| c.is_ascii_digit())) {
            after.trim()
        } else {
            continue;
        };
        if !takes_arg && !arg.is_empty() {
            return None;
        }
        let arg = arg.to_string();
        return Some(match *word {
            "加加" | "jiajia" => Command::Jiajia,
            "直连" | "direct" => Command::Direct,
            "状态" | "status" => Command::Status,
            "会话" | "sessions" => Command::Sessions,
            "进入" | "focus" => Command::Focus(arg),
            "切换" | "switch" => Command::Switch(arg),
            "新会话" | "new" => Command::NewSession(arg),
            _ => Command::Exit,
        });
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// 卡片上列出的第几个选项（从 1 起）。
    Index(usize),
    Accept,
    AcceptForSession,
    Decline,
}

/// 对审批卡片的回复：选项 + 可选的卡片编号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalReply {
    pub choice: Choice,
    pub card: Option<u32>,
}

/// 认这几种写法：`1`、`1 #2`、`#2 1`、`1#2`、`同意`、`拒绝 #3`、`本会话都同意`。整条消息只能是这些，
/// 夹在句子里的数字不算（免得把正常聊天当成审批）。
pub fn parse_approval(text: &str) -> Option<ApprovalReply> {
    let text = text.trim().replace('＃', "#");
    let mut card = None;
    let mut rest = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '#' {
            rest.push(c);
            continue;
        }
        let mut digits = String::new();
        while let Some(digit) = chars.peek().copied().filter(char::is_ascii_digit) {
            digits.push(digit);
            chars.next();
        }
        if digits.is_empty() || card.is_some() {
            return None;
        }
        card = Some(digits.parse().ok()?);
    }
    let choice = match rest.trim() {
        "同意" | "批准" | "允许" => Choice::Accept,
        "本会话都同意" | "都同意" => Choice::AcceptForSession,
        "拒绝" | "不同意" => Choice::Decline,
        digits if (1..=2).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit()) => {
            Choice::Index(digits.parse().ok().filter(|index| *index > 0)?)
        }
        _ => return None,
    };
    Some(ApprovalReply { choice, card })
}

// ---------- 审批卡片 ----------

/// 卡片上的一个选项：显示的字和回给 Codex 的内容（和网页端点按钮时发的一样）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardOption {
    pub label: String,
    pub body: Value,
    /// 审批类请求的决定（accept / acceptForSession / decline / cancel），按字回复「同意」时用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
}

/// 已经发到微信、还没处理的一张卡片。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardRef {
    /// 微信里显示的编号 `#n`：取当前没被占用的最小正整数，处理完就空出来。
    pub no: u32,
    /// 加加聊天记录里这张卡片的 id（处理后在网页上标成已处理）。
    pub message_id: u64,
    pub thread_id: String,
    #[serde(default)]
    pub title: String,
    pub request_id: Value,
    pub options: Vec<CardOption>,
    /// 专注会话的卡片：微信里不显示编号（回个数字就是答它），也不在加加的聊天记录里（`message_id` 为 0）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focus: bool,
}

fn decision_label(decision: &str) -> &str {
    match decision {
        "accept" => "同意",
        "acceptForSession" => "本会话都同意",
        "decline" => "拒绝",
        "cancel" => "拒绝并停止",
        other => other,
    }
}

/// 按请求实际支持的选项生成卡片选项；微信里答不了的（多个问题、权限申请等）返回空。
pub fn card_options(request: &Value) -> Vec<CardOption> {
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            // 和网页端的审批卡片一致：只认字符串选项，没有拒绝就补一个，最多三个。
            let mut decisions: Vec<String> = match params.get("availableDecisions").and_then(Value::as_array) {
                Some(list) => list.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                None => vec!["accept".into(), "decline".into()],
            };
            if !decisions.iter().any(|decision| decision == "decline" || decision == "cancel") {
                decisions.push("decline".into());
            }
            decisions.truncate(3);
            decisions
                .into_iter()
                .map(|decision| CardOption {
                    label: decision_label(&decision).to_string(),
                    body: json!({ "decision": decision }),
                    decision: Some(decision),
                })
                .collect()
        }
        "item/tool/requestUserInput" => {
            // 只有一个带选项的问题时能在微信里答：回答格式同网页端 `{ answers: { 问题id: { answers: [选项] } } }`。
            let questions = params.get("questions").and_then(Value::as_array).cloned().unwrap_or_default();
            let [question] = questions.as_slice() else { return Vec::new() };
            let Some(id) = question.get("id").and_then(Value::as_str) else { return Vec::new() };
            question
                .get("options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| option.get("label").and_then(Value::as_str))
                        .map(|label| CardOption {
                            label: events::clip(label, 40),
                            body: json!({ "answers": { id: { "answers": [label] } } }),
                            decision: None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// 卡片转成微信里的一段文字。`others` 是同时还有几张待处理（大于 0 时提示带编号回复）。
pub fn card_text(card: &CardRef, request: &Value, others: usize) -> String {
    let title = if card.title.is_empty() { "未命名会话" } else { card.title.as_str() };
    let summary = events::request_summary(request);
    let is_question = request.get("method").and_then(Value::as_str) == Some("item/tool/requestUserInput");
    let tag = if card.focus { String::new() } else { format!("#{} ", card.no) };
    let mut text = if is_question {
        format!("❓ {tag}会话「{title}」{summary}")
    } else {
        format!("⚠️ {tag}会话「{title}」等你审批：{summary}")
    };
    if card.options.is_empty() {
        text.push_str("\n这类请求要在桌面或远程控制网页上处理。");
        return text;
    }
    text.push_str("\n回复 ");
    text.push_str(&options_line(&card.options));
    if others > 0 && !card.focus {
        text.push_str(&format!("\n同时还有 {others} 张待处理，回复时带上编号，如「1 #{}」", card.no));
    }
    text
}

fn options_line(options: &[CardOption]) -> String {
    options.iter().enumerate().map(|(index, option)| format!("{} {}", index + 1, option.label)).collect::<Vec<_>>().join(" · ")
}

fn next_card_no(cards: &[CardRef]) -> u32 {
    (1..).find(|no| !cards.iter().any(|card| card.no == *no)).unwrap_or(1)
}

// ---------- 持久化 ----------

/// 发不出去、等本人下次发消息时补发的一条推送。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPush {
    pub at: i64,
    pub text: String,
    /// 是审批卡片时记下它在加加聊天记录里的 id：补发前已经处理掉的卡片就不发了。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<u64>,
}

/// 存在微信连接状态旁边的 `<账号>-jiajia.json`，和原来的连接状态分开，直连模式的文件不受影响。
/// 不存 context_token（它在原来的连接状态里）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BridgeState {
    /// 联系人自己选过的模式（发过 /加加 或 /直连）。
    pub peer_modes: BTreeMap<String, PeerMode>,
    /// 这次加加不可用期间已经说明过「先按直连处理」的联系人；加加恢复可用后清掉，下次不可用再说一次。
    pub fallback_noticed: BTreeSet<String>,
    /// 发不出去、等本人下次发消息时补发的推送（最多 [`PENDING_LIMIT`] 条）。
    pub pending: VecDeque<PendingPush>,
    /// 已发到微信、还没处理的卡片。
    pub cards: Vec<CardRef>,
    /// 专注模式：联系人钉住的会话。
    pub focus: BTreeMap<String, focus::FocusState>,
    /// 专注模式：联系人最近一次 /会话 列出的编号（/进入 N 用）。
    pub session_lists: BTreeMap<String, focus::SessionList>,
    /// 专注模式：联系人上次专注 / 新开会话用的项目目录（/新会话 不写项目时用）。
    pub last_projects: BTreeMap<String, String>,
}

// ---------- 加加的接口 ----------

/// 微信这边要用到的加加能力。真实实现是 [`LiveJiajia`]，测试里换成假的。
#[async_trait]
pub trait JiajiaPort: Send + Sync {
    /// 现在能不能用：None 是能用，否则是给用户看的原因。
    async fn unavailable_reason(&self) -> Option<String>;
    /// 聊天记录当前位置：(实例标识, 变更序号, 最后一条 id)。不可用时 None。
    /// 实例标识在远程控制重启（换了一个加加）时会变。
    fn cursor(&self) -> Option<(usize, u64, u64)>;
    /// 变更序号比 `rev` 新的记录（新消息和被改动的旧消息）。
    fn changed_since(&self, rev: u64) -> Vec<ChatMessage>;
    /// 等聊天记录有变化，最多等 `timeout`。
    async fn wait_changed(&self, timeout: Duration);
    async fn send(&self, text: &str, by: Option<String>, role: Role) -> anyhow::Result<()>;
    /// 会话当前还挂着的请求（请求 id 的 JSON 文本）。
    async fn live_requests(&self, thread_id: &str) -> anyhow::Result<Vec<String>>;
    async fn respond(&self, thread_id: &str, request_id: &Value, body: &Value) -> anyhow::Result<()>;
    /// 把聊天记录里的卡片标成已处理。
    fn resolve_card(&self, message_id: u64);
    /// 加加（`focus_thread` 工具）要微信进入专注的请求，取一次就没了。
    fn take_focus_request(&self) -> Option<crate::remote_control::jiajia::FocusRequest> {
        None
    }
}

/// 同进程里正在跑的远程控制服务上的加加。
pub struct LiveJiajia;

impl LiveJiajia {
    fn jiajia() -> Option<Arc<crate::remote_control::jiajia::Jiajia>> {
        crate::remote_control::hosted_service().map(|(service, _)| service.jiajia())
    }

    /// 在远程控制自己的运行时上跑：加加会 `tokio::spawn` 后台任务，不能落在微信连接的运行时里。
    async fn on_service<T, F, Fut>(work: F) -> anyhow::Result<T>
    where
        F: FnOnce(Arc<RemoteService>) -> Fut,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let (service, runtime) = crate::remote_control::hosted_service().context("远程控制没开")?;
        runtime.spawn(work(service)).await.context("远程控制服务已停止")?
    }
}

#[async_trait]
impl JiajiaPort for LiveJiajia {
    async fn unavailable_reason(&self) -> Option<String> {
        let Some(jiajia) = Self::jiajia() else { return Some("远程控制没开".into()) };
        if !jiajia.is_configured() {
            return Some("加加还没选模型，先在远程控制网页里打开加加选一个".into());
        }
        None
    }

    fn cursor(&self) -> Option<(usize, u64, u64)> {
        let jiajia = Self::jiajia()?;
        let (rev, last) = jiajia.cursor();
        Some((Arc::as_ptr(&jiajia) as usize, rev, last))
    }

    fn changed_since(&self, rev: u64) -> Vec<ChatMessage> {
        Self::jiajia().map(|jiajia| jiajia.changed_since(rev)).unwrap_or_default()
    }

    async fn wait_changed(&self, timeout: Duration) {
        match Self::jiajia() {
            Some(jiajia) => {
                let mut updates = jiajia.subscribe();
                drop(jiajia);
                if let Ok(Err(_)) = tokio::time::timeout(timeout, updates.changed()).await {
                    // 加加已经释放（远程控制停了）：别空转。
                    tokio::time::sleep(timeout).await;
                }
            }
            None => tokio::time::sleep(timeout).await,
        }
    }

    async fn send(&self, text: &str, by: Option<String>, role: Role) -> anyhow::Result<()> {
        let text = text.to_string();
        Self::on_service(move |service| async move { service.jiajia().send(&text, by, role).await.map(|_| ()) }).await
    }

    async fn live_requests(&self, thread_id: &str) -> anyhow::Result<Vec<String>> {
        let thread_id = thread_id.to_string();
        Self::on_service(move |service| async move {
            let view = service.thread_view(&thread_id).await?;
            Ok(events::snapshot(&view).requests.into_iter().map(|(key, _)| key).collect())
        })
        .await
    }

    async fn respond(&self, thread_id: &str, request_id: &Value, body: &Value) -> anyhow::Result<()> {
        let (thread_id, request_id, body) = (thread_id.to_string(), request_id.clone(), body.clone());
        Self::on_service(move |service| async move { service.respond(&thread_id, &request_id, &body).await.map(|_| ()) }).await
    }

    fn resolve_card(&self, message_id: u64) {
        if let Some(jiajia) = Self::jiajia() {
            jiajia.resolve_card(message_id);
        }
    }

    fn take_focus_request(&self) -> Option<crate::remote_control::jiajia::FocusRequest> {
        Self::jiajia()?.take_focus_request()
    }
}

// ---------- 桥 ----------

/// 推送结果写诊断日志（不记 token 和正文）。测试里换成收集器。
pub type PushLog = Arc<dyn Fn(&str, Value) + Send + Sync>;

pub fn diagnostic_push_log() -> PushLog {
    Arc::new(|event, detail| {
        let _ = crate::diagnostic_log::append_diagnostic_log(event, detail);
    })
}

/// 一条微信消息交给桥之后的去向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inbound {
    /// 桥已经处理完（命令、审批、交给了加加），不再走直连。
    Handled,
    /// 照原来的直连流程处理。
    Direct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Reply,
    Card,
    Notice,
}

impl Kind {
    fn id(self) -> &'static str {
        match self {
            Kind::Reply => "reply",
            Kind::Card => "card",
            Kind::Notice => "notice",
        }
    }
}

/// 加加最近一次收到的输入是谁给的：决定它接下来的回复要不要发到微信。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastInput {
    None,
    Weixin,
    Web,
    Event,
}

struct TokenInfo {
    token: String,
    /// 收到这个 token 的时间（启动时从连接状态带过来的不知道时间）。
    at: Option<i64>,
    /// 用它推送失败过：等本人发来新消息换了 token 再试，期间的推送直接存起来。
    failed: bool,
}

#[derive(Clone, Copy)]
struct Cursor {
    instance: usize,
    rev: u64,
    max_id: u64,
}

struct Inner {
    state: BridgeState,
    tokens: HashMap<String, TokenInfo>,
    /// 本人最近一次在微信里发给加加的时间（判断「正在对话」）。
    last_weixin_at: Option<i64>,
    last_input: LastInput,
    cursor: Option<Cursor>,
    /// 正在显示的「正在输入」：(联系人, typing_ticket, 开始时间)。
    typing: Option<(String, String, i64)>,
    /// 微信里最近显示的是审批卡片（之后本人还没说别的）：这时回个光秃秃的数字才算审批。
    /// 加加回了话、本人发了别的消息之后，数字可能是在回答别的问题，要带 `#编号` 才算。
    card_prompt: bool,
    /// 专注会话上次查看的时间、上次本人发消息的时间、上次看到时它是不是在忙（决定多久看一次）。
    focus_polled_at: i64,
    focus_sent_at: i64,
    focus_busy: bool,
}

pub struct JiajiaBridge {
    port: Arc<dyn JiajiaPort>,
    /// 专注模式用的会话操作（远程控制的会话），测试里换成假的。
    sessions: Arc<dyn focus::SessionPort>,
    /// 微信连接配置的工作目录（/新会话 没写项目、也没用过项目时用）。
    work_dir: String,
    client: WeixinClient,
    path: PathBuf,
    owner: Option<String>,
    settings: WeixinCodexPath,
    fallback: BridgeSettings,
    log: PushLog,
    inner: Mutex<Inner>,
}

impl JiajiaBridge {
    /// `settings` 里保存过的设置优先（管理器改了即时生效），没有时用启动时的 `fallback`。
    pub fn new(
        port: Arc<dyn JiajiaPort>,
        client: WeixinClient,
        path: PathBuf,
        owner: Option<String>,
        settings: WeixinCodexPath,
        fallback: BridgeSettings,
        log: PushLog,
    ) -> Self {
        let state = std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        Self {
            port,
            sessions: Arc::new(focus::LiveSessions),
            work_dir: String::new(),
            client,
            path,
            owner,
            settings,
            fallback,
            log,
            inner: Mutex::new(Inner {
                state,
                tokens: HashMap::new(),
                last_weixin_at: None,
                last_input: LastInput::None,
                cursor: None,
                typing: None,
                card_prompt: false,
                focus_polled_at: 0,
                focus_sent_at: 0,
                focus_busy: false,
            }),
        }
    }

    /// 专注模式用的会话操作和默认工作目录。
    pub fn with_focus(mut self, sessions: Arc<dyn focus::SessionPort>, work_dir: &str) -> Self {
        self.sessions = sessions;
        self.work_dir = work_dir.trim().to_string();
        self
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn settings(&self) -> BridgeSettings {
        self.settings.bridge_settings().unwrap_or(self.fallback)
    }

    fn is_owner(&self, peer: &str) -> bool {
        self.owner.as_deref() == Some(peer)
    }

    fn save(&self, state: &BridgeState) {
        if let Ok(bytes) = serde_json::to_vec_pretty(state) {
            let _ = crate::settings::atomic_write(&self.path, &bytes);
        }
    }

    /// 当前持久状态的副本（测试和 /状态 用）。
    pub fn state(&self) -> BridgeState {
        self.inner().state.clone()
    }

    /// 启动时把连接状态里存着的 token 带过来：推送用本人最近一次的 token。
    pub fn seed_tokens(&self, tokens: &BTreeMap<String, String>) {
        let mut inner = self.inner();
        for (peer, token) in tokens {
            inner.tokens.entry(peer.clone()).or_insert(TokenInfo { token: token.clone(), at: None, failed: false });
        }
    }

    fn note_token(&self, peer: &str, token: &str) {
        if token.trim().is_empty() {
            return;
        }
        self.inner().tokens.insert(peer.to_string(), TokenInfo { token: token.to_string(), at: Some(now_ms()), failed: false });
    }

    fn mode_of(&self, inner: &Inner, peer: &str, settings: &BridgeSettings) -> PeerMode {
        inner.state.peer_modes.get(peer).copied().unwrap_or(settings.default_mode)
    }

    async fn reply(&self, peer: &str, context_token: &str, text: &str) {
        let _ = self.client.send_text_chunks(peer, text, context_token).await;
    }

    // ---------- 收到微信消息 ----------

    /// 每条通过白名单、带 context_token 的微信消息先经过这里。
    pub async fn handle_inbound(&self, message: &WeixinMessage, text: &str) -> Inbound {
        let peer = message.from_user_id.as_str();
        let token = message.context_token.as_str();
        self.note_token(peer, token);
        let is_owner = self.is_owner(peer);
        let approval = (is_owner && !message.has_attachments()).then(|| parse_approval(text)).flatten();
        if approval.is_none() && parse_command(text).is_none() {
            // 本人说了别的：之后光秃秃的数字不再默认指卡片。
            self.inner().card_prompt = false;
        }
        if is_owner {
            self.flush_pending(peer).await;
        }
        if !message.has_attachments()
            && let Some(command) = parse_command(text)
        {
            let reply = self.run_command(peer, command).await;
            self.reply(peer, token, &reply).await;
            return Inbound::Handled;
        }
        // 直连会话自己的命令（/reset 等）不管什么模式都照旧交给直连处理。
        if text.trim_start().starts_with("/reset") {
            return Inbound::Direct;
        }
        // 有卡片等着时，本人回的数字 / 「同意」就是审批（直连模式开了推送也一样）：
        // 带 `#编号` 的总算；不带的只在微信里最近显示的就是卡片时才算。
        let (has_cards, card_prompt) = {
            let inner = self.inner();
            (!inner.state.cards.is_empty(), inner.card_prompt)
        };
        if has_cards
            && let Some(reply) = approval
            && (reply.card.is_some() || card_prompt)
        {
            let answer = self.approve(reply).await;
            self.reply(peer, token, &answer).await;
            return Inbound::Handled;
        }
        // 专注中：本人的消息直接交给钉住的会话（不管原来是加加还是直连）。
        if is_owner && let Some(focus) = self.focus_of(peer) {
            return self.send_to_focus(peer, token, message, text, &focus).await;
        }
        let settings = self.settings();
        let mode = {
            let inner = self.inner();
            self.mode_of(&inner, peer, &settings)
        };
        if !is_owner || mode == PeerMode::Direct {
            return Inbound::Direct;
        }
        if let Some(reason) = self.port.unavailable_reason().await {
            let first = {
                let mut inner = self.inner();
                let first = inner.state.fallback_noticed.insert(peer.to_string());
                if first {
                    self.save(&inner.state);
                }
                first
            };
            if first {
                let notice = format!(
                    "加加现在用不了（{reason}），这条先直接交给 Codex 处理。\n加加需要先在 Codex++ 里打开远程控制；发 /直连 固定用直连、不再提示，发 /状态 查看当前模式。"
                );
                self.reply(peer, token, &notice).await;
            }
            return Inbound::Direct;
        }
        {
            let mut inner = self.inner();
            if inner.state.fallback_noticed.remove(peer) {
                self.save(&inner.state);
            }
        }
        if message.has_attachments() {
            self.reply(peer, token, "加加暂时只看文字，切到 /直连 可以发图片和文件。").await;
            if text.trim().is_empty() {
                return Inbound::Handled;
            }
        }
        match self.port.send(text, Some(SENDER_NAME.to_string()), Role::Full).await {
            Ok(()) => {
                self.inner().last_weixin_at = Some(now_ms());
                self.start_typing(peer, token).await;
            }
            Err(error) => {
                self.reply(peer, token, &format!("加加没收到这条消息：{}", events::clip(&format!("{error:#}"), 160))).await;
            }
        }
        Inbound::Handled
    }

    async fn run_command(&self, peer: &str, command: Command) -> String {
        match command {
            Command::Jiajia => {
                if !self.is_owner(peer) {
                    return if self.owner.is_none() {
                        "还没认出扫码绑定的本人，暂时不能用加加：请在 Codex++ 管理器里重新扫码登录微信后再试。".into()
                    } else {
                        "加加只对扫码绑定的本人开放；你的消息会继续直接交给 Codex。".into()
                    };
                }
                self.set_mode(peer, PeerMode::Jiajia);
                let mut text = String::from("已切到加加：消息交给加加，它替你照看 Codex 里的会话。发 /直连 切回直连。");
                if let Some(reason) = self.port.unavailable_reason().await {
                    text.push_str(&format!("\n不过加加现在用不了（{reason}），可用之前消息先直接交给 Codex。"));
                }
                text
            }
            Command::Direct => {
                self.set_mode(peer, PeerMode::Direct);
                if self.is_owner(peer) {
                    "已切到直连：消息直接交给你在微信里的 Codex 会话。发 /加加 切回加加。".into()
                } else {
                    "已切到直连：消息直接交给你在微信里的 Codex 会话。".into()
                }
            }
            Command::Status => self.status_text(peer).await,
            command => self.run_focus_command(peer, command).await,
        }
    }

    fn set_mode(&self, peer: &str, mode: PeerMode) {
        let mut inner = self.inner();
        inner.state.peer_modes.insert(peer.to_string(), mode);
        // 自己选过了就不用再提示「先按直连处理」。
        inner.state.fallback_noticed.remove(peer);
        self.save(&inner.state);
    }

    async fn status_text(&self, peer: &str) -> String {
        let settings = self.settings();
        let unavailable = self.port.unavailable_reason().await;
        let focus_line = if self.is_owner(peer) { Some(self.focus_status_line(peer).await) } else { None };
        let inner = self.inner();
        let explicit = inner.state.peer_modes.get(peer).copied();
        let mode = explicit.unwrap_or(settings.default_mode);
        let source = if explicit.is_some() { "你选的" } else { "默认" };
        let mut lines = Vec::new();
        if !self.is_owner(peer) {
            lines.push("当前模式：直连（加加只对扫码绑定的本人开放）".to_string());
        } else if mode == PeerMode::Jiajia && unavailable.is_some() {
            lines.push(format!("当前模式：加加（{source}），加加现在用不了，先按直连处理"));
        } else {
            lines.push(format!("当前模式：{}（{source}）", mode.label()));
        }
        lines.push(match &unavailable {
            None => "加加：可用".to_string(),
            Some(reason) => format!("加加：不可用（{reason}）"),
        });
        lines.push(format!("推送到微信：{}", if settings.push { "开" } else { "关（在 Codex++ 管理器的微信设置里打开）" }));
        lines.extend(focus_line);
        if self.is_owner(peer) {
            let cards = &inner.state.cards;
            if !cards.is_empty() {
                let list = cards.iter().map(|card| format!("#{}「{}」", card.no, if card.title.is_empty() { "未命名会话" } else { &card.title })).collect::<Vec<_>>().join("、");
                lines.push(format!("待处理：{list}（回复「1 #编号」这样带编号处理）"));
            }
            if !inner.state.pending.is_empty() {
                lines.push(format!("还有 {} 条推送没送达，你发消息时会补发", inner.state.pending.len()));
            }
        }
        lines.push(if self.is_owner(peer) { "命令：/加加 /直连 /状态 /会话 /进入 编号 /新会话 任务 /退出".to_string() } else { "命令：/加加 /直连 /状态".to_string() });
        lines.join("\n")
    }

    // ---------- 审批 ----------

    async fn approve(&self, reply: ApprovalReply) -> String {
        let card = {
            let inner = self.inner();
            let cards = &inner.state.cards;
            let numbers = || cards.iter().map(|card| format!("#{}", card.no)).collect::<Vec<_>>().join(" ");
            let focused = self.focused_thread(&inner);
            let focus_cards: Vec<&CardRef> = cards.iter().filter(|card| Some(&card.thread_id) == focused.as_ref()).collect();
            match reply.card {
                Some(no) => match cards.iter().find(|card| card.no == no) {
                    Some(card) => card.clone(),
                    None => return format!("没有编号 #{no} 的待处理卡片。现在待处理的是：{}", numbers()),
                },
                None if cards.len() == 1 => cards[0].clone(),
                // 专注会话的卡片不带编号：它只有一张时，光秃秃的数字就是答它。
                None if focus_cards.len() == 1 => focus_cards[0].clone(),
                None => {
                    let last = cards.last().map(|card| card.no).unwrap_or(1);
                    return format!("现在有 {} 张待处理的卡片（{}），回复时带上编号，例如「1 #{last}」。", cards.len(), numbers());
                }
            }
        };
        let wanted = |decisions: &[&str]| card.options.iter().find(|option| option.decision.as_deref().is_some_and(|decision| decisions.contains(&decision)));
        let option = match reply.choice {
            Choice::Index(index) => card.options.get(index - 1),
            Choice::Accept => wanted(&["accept"]),
            Choice::AcceptForSession => wanted(&["acceptForSession"]),
            Choice::Decline => wanted(&["decline"]).or_else(|| wanted(&["cancel"])),
        };
        let Some(option) = option.cloned() else {
            return format!("卡片 #{} 可以回复：{}", card.no, options_line(&card.options));
        };
        // 和网页端一样，先核对这张卡片的请求还是当前待处理的请求：已处理 / 过期的不再发。
        match self.port.live_requests(&card.thread_id).await {
            Ok(live) if !live.contains(&card.request_id.to_string()) => {
                self.finish_card(&card);
                return "这个请求已经处理过了。".into();
            }
            Ok(_) => {}
            Err(error) => return format!("没能确认这个请求的状态：{}", events::clip(&format!("{error:#}"), 120)),
        }
        match self.port.respond(&card.thread_id, &card.request_id, &option.body).await {
            Ok(()) => {
                self.finish_card(&card);
                let title = if card.title.is_empty() { "未命名会话" } else { card.title.as_str() };
                if card.focus {
                    format!("已回复「{}」：会话「{title}」。", option.label)
                } else {
                    format!("已回复「{}」：#{} 会话「{title}」。", option.label, card.no)
                }
            }
            Err(error) if format!("{error:#}").contains("已经处理过") => {
                self.finish_card(&card);
                "这个请求已经处理过了。".into()
            }
            Err(error) => format!("没发出去：{}", events::clip(&format!("{error:#}"), 120)),
        }
    }

    /// 卡片处理完：从待处理里拿掉，聊天记录里标成已处理（专注会话的卡片不在聊天记录里）。
    fn finish_card(&self, card: &CardRef) {
        {
            let mut inner = self.inner();
            inner.state.cards.retain(|existing| existing.no != card.no);
            self.save(&inner.state);
        }
        if card.message_id != 0 {
            self.port.resolve_card(card.message_id);
        }
    }

    // ---------- 转发加加的消息 ----------

    /// 后台转发：加加有新消息就看要不要发到微信。`stop` 置位后退出。
    pub async fn run_forwarder(self: Arc<Self>, stop: Arc<AtomicBool>) {
        while !stop.load(Ordering::SeqCst) {
            self.forward_once().await;
            self.take_focus_request().await;
            self.poll_focus(false).await;
            self.expire_typing().await;
            self.port.wait_changed(FORWARD_POLL).await;
        }
        self.stop_typing().await;
    }

    /// 处理一遍上次之后的新记录。第一次看到某个加加（刚启动、远程控制重启）只记位置，不重放旧记录。
    pub async fn forward_once(&self) {
        let Some((instance, rev, max_id)) = self.port.cursor() else {
            self.inner().cursor = None;
            return;
        };
        let from = {
            let mut inner = self.inner();
            match inner.cursor {
                Some(cursor) if cursor.instance == instance && cursor.rev <= rev => cursor,
                _ => {
                    inner.cursor = Some(Cursor { instance, rev, max_id });
                    return;
                }
            }
        };
        if from.rev == rev {
            return;
        }
        let mut messages = self.port.changed_since(from.rev);
        messages.sort_by_key(|message| message.rev);
        let settings = self.settings();
        let now = now_ms();
        let mut outgoing: Vec<(Kind, String, Option<u64>)> = Vec::new();
        {
            let mut inner = self.inner();
            let mut cursor = from;
            let mut dirty = false;
            for message in messages {
                cursor.rev = cursor.rev.max(message.rev);
                if message.id <= cursor.max_id {
                    // 旧记录有改动：只关心卡片在别处（网页、桌面）被处理了。
                    if message.resolved && inner.state.cards.iter().any(|card| card.message_id == message.id) {
                        inner.state.cards.retain(|card| card.message_id != message.id);
                        dirty = true;
                    }
                    continue;
                }
                cursor.max_id = message.id;
                match message.role.as_str() {
                    // 用户发的话不回显：只记下是谁说的（决定加加接下来的回复发不发到微信）。
                    "user" => {
                        inner.last_input = if message.by.as_deref() == Some(SENDER_NAME) { LastInput::Weixin } else { LastInput::Web };
                    }
                    "event" => inner.last_input = LastInput::Event,
                    "bot" | "notice" => {
                        let kind = if message.request.is_some() {
                            Kind::Card
                        } else if message.role == "bot" {
                            Kind::Reply
                        } else {
                            Kind::Notice
                        };
                        if (kind == Kind::Card && message.resolved) || !self.should_deliver(&inner, kind, &settings, now) {
                            continue;
                        }
                        // 专注会话的审批由专注那边直接发（不带编号），这里不重复发。
                        let card_thread = message.request.as_ref().and_then(|info| info.get("threadId")).and_then(Value::as_str);
                        if kind == Kind::Card && card_thread.is_some() && card_thread.map(str::to_string) == self.focused_thread(&inner) {
                            continue;
                        }
                        match kind {
                            Kind::Card => {
                                if let Some((text, card)) = register_card(&mut inner.state, &message) {
                                    dirty |= card.is_some();
                                    outgoing.push((kind, text, card));
                                }
                            }
                            Kind::Reply => outgoing.push((kind, message.text.trim().to_string(), None)),
                            Kind::Notice => outgoing.push((kind, format!("🔔 {}", message.text.trim()), None)),
                        }
                    }
                    _ => {}
                }
            }
            inner.cursor = Some(cursor);
            if dirty {
                self.save(&inner.state);
            }
        }
        for (kind, text, card) in outgoing {
            if text.is_empty() {
                continue;
            }
            if kind == Kind::Reply {
                self.stop_typing().await;
            }
            self.deliver(kind, text, card).await;
        }
    }

    /// 这条加加的消息要不要发到本人的微信。
    fn should_deliver(&self, inner: &Inner, kind: Kind, settings: &BridgeSettings, now: i64) -> bool {
        let Some(owner) = self.owner.as_deref() else { return false };
        // 回本人在微信里说的话：总是发回去。
        if kind == Kind::Reply && inner.last_input == LastInput::Weixin {
            return true;
        }
        // 加加模式下正在对话：网页上和加加的对话、卡片、提醒都同步到微信（共用一份聊天记录）。
        // 专注中不算在和加加对话（消息没经过加加）。
        let conversing = self.mode_of(inner, owner, settings) == PeerMode::Jiajia
            && !inner.state.focus.contains_key(owner)
            && inner.last_weixin_at.is_some_and(|at| now - at < CONVERSATION_WINDOW_MS);
        if conversing {
            return true;
        }
        // 没在对话：只有打开了推送才发，且只发会话的动静（卡片、提醒、加加因为事件主动说的话）。
        settings.push
            && match kind {
                Kind::Card | Kind::Notice => true,
                Kind::Reply => inner.last_input == LastInput::Event,
            }
    }

    /// 用本人最近的 token 发出去；没有 token、token 刚失败过或这次发送失败，就存起来等补发。
    async fn deliver(&self, kind: Kind, text: String, card: Option<u64>) {
        let Some(owner) = self.owner.clone() else { return };
        let (token, has_failed_token) = {
            let inner = self.inner();
            let info = inner.tokens.get(&owner);
            (info.filter(|info| !info.failed).map(|info| (info.token.clone(), info.at)), info.is_some_and(|info| info.failed))
        };
        let Some((token, received_at)) = token else {
            let queued = self.enqueue(&text, card);
            let reason = if has_failed_token { "tokenFailed" } else { "noToken" };
            (self.log)("weixin.jiajia_push", json!({ "kind": kind.id(), "ok": false, "reason": reason, "queued": queued }));
            return;
        };
        let token_age_ms = received_at.map(|at| now_ms() - at);
        match self.client.send_text_chunks(&owner, &text, &token).await {
            Ok(()) => {
                self.inner().card_prompt = kind == Kind::Card;
                (self.log)("weixin.jiajia_push", json!({ "kind": kind.id(), "ok": true, "tokenAgeMs": token_age_ms }));
            }
            Err(error) => {
                if let Some(info) = self.inner().tokens.get_mut(&owner).filter(|info| info.token == token) {
                    info.failed = true;
                }
                let queued = self.enqueue(&text, card);
                (self.log)(
                    "weixin.jiajia_push",
                    json!({ "kind": kind.id(), "ok": false, "reason": "sendFailed", "tokenAgeMs": token_age_ms, "queued": queued, "error": events::clip(&format!("{error:#}"), 160) }),
                );
            }
        }
    }

    /// 存一条待补发的推送，返回现在存了几条。
    fn enqueue(&self, text: &str, card: Option<u64>) -> usize {
        let mut inner = self.inner();
        let text: String = text.chars().take(PENDING_TEXT_LIMIT).collect();
        inner.state.pending.push_back(PendingPush { at: now_ms(), text, card });
        while inner.state.pending.len() > PENDING_LIMIT {
            inner.state.pending.pop_front();
        }
        self.save(&inner.state);
        inner.state.pending.len()
    }

    /// 本人发来消息时先补发存着的推送（已经处理掉的卡片跳过）。
    async fn flush_pending(&self, peer: &str) {
        let (entries, taken, token, has_card) = {
            let mut inner = self.inner();
            if inner.state.pending.is_empty() {
                return;
            }
            let Some(token) = inner.tokens.get(peer).map(|info| info.token.clone()) else { return };
            let cards: Vec<u64> = inner.state.cards.iter().map(|card| card.message_id).collect();
            let kept: Vec<&PendingPush> = inner.state.pending.iter().filter(|entry| entry.card.is_none_or(|id| cards.contains(&id))).collect();
            let has_card = kept.iter().any(|entry| entry.card.is_some());
            let entries: Vec<String> = kept.iter().map(|entry| entry.text.clone()).collect();
            let taken = inner.state.pending.len();
            if entries.is_empty() {
                inner.state.pending.clear();
                self.save(&inner.state);
                return;
            }
            (entries, taken, token, has_card)
        };
        let text = format!("你不在时：\n\n{}", entries.join("\n\n"));
        match self.client.send_text_chunks(peer, &text, &token).await {
            Ok(()) => {
                let mut inner = self.inner();
                let taken = taken.min(inner.state.pending.len());
                inner.state.pending.drain(..taken);
                // 补发里有卡片：接下来回的数字就是审批。
                inner.card_prompt = has_card;
                self.save(&inner.state);
                (self.log)("weixin.jiajia_backlog", json!({ "ok": true, "count": entries.len() }));
            }
            Err(error) => {
                (self.log)("weixin.jiajia_backlog", json!({ "ok": false, "count": entries.len(), "error": events::clip(&format!("{error:#}"), 160) }));
            }
        }
    }

    // ---------- 正在输入 ----------

    async fn start_typing(&self, peer: &str, token: &str) {
        self.stop_typing().await;
        let ticket = tokio::time::timeout(Duration::from_secs(2), self.client.start_typing(peer, token)).await.ok().and_then(Result::ok);
        if let Some(ticket) = ticket {
            self.inner().typing = Some((peer.to_string(), ticket, now_ms()));
        }
    }

    async fn stop_typing(&self) {
        let typing = self.inner().typing.take();
        if let Some((peer, ticket, _)) = typing {
            let _ = tokio::time::timeout(Duration::from_secs(2), self.client.stop_typing(&peer, &ticket)).await;
        }
    }

    async fn expire_typing(&self) {
        let expired = self.inner().typing.as_ref().is_some_and(|(_, _, at)| now_ms() - at > TYPING_LIMIT_MS);
        if expired {
            self.stop_typing().await;
        }
    }
}

/// 把聊天记录里的卡片登记成微信里的 `#n`，返回要发的文字和（能在微信里处理时）卡片 id。
fn register_card(state: &mut BridgeState, message: &ChatMessage) -> Option<(String, Option<u64>)> {
    let info = message.request.as_ref()?;
    let request = info.get("request").cloned().unwrap_or(Value::Null);
    let thread_id = info.get("threadId").and_then(Value::as_str).unwrap_or_default().to_string();
    let title = info.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
    let options = card_options(&request);
    let card = CardRef {
        no: next_card_no(&state.cards),
        message_id: message.id,
        thread_id,
        title,
        request_id: request.get("id").cloned().unwrap_or(Value::Null),
        options,
        focus: false,
    };
    let text = card_text(&card, &request, state.cards.len());
    if card.options.is_empty() || card.thread_id.is_empty() || card.request_id.is_null() {
        // 微信里答不了的只发文字提醒，不占编号。
        let text = text.replacen(&format!("#{} ", card.no), "", 1);
        return Some((text, None));
    }
    let id = card.message_id;
    state.cards.push(card);
    Some((text, Some(id)))
}

fn now_ms() -> i64 {
    crate::remote_control::jiajia::store::now_ms()
}

#[path = "jiajia_focus.rs"]
pub mod focus;

pub use focus::{LiveSessions, SessionPort, ThreadSummary};

#[cfg(test)]
#[path = "jiajia_bridge_tests.rs"]
mod tests;
