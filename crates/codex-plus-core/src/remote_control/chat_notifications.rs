use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::jiajia::events::{self, Event, EventKind, Snapshot};

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Provider { DingTalk, Feishu }

const PROVIDERS: [Provider; 2] = [Provider::DingTalk, Provider::Feishu];

#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum LinkNetwork { #[default] Lan, Tailscale, None }

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Robot {
    enabled: bool,
    webhook: String,
    secret: String,
    link_network: LinkNetwork,
    revision: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Settings { dingtalk: Robot, feishu: Robot }

impl Settings {
    fn robot(&self, provider: Provider) -> &Robot {
        match provider { Provider::DingTalk => &self.dingtalk, Provider::Feishu => &self.feishu }
    }
    fn robot_mut(&mut self, provider: Provider) -> &mut Robot {
        match provider { Provider::DingTalk => &mut self.dingtalk, Provider::Feishu => &mut self.feishu }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Update {
    enabled: Option<bool>,
    webhook: Option<String>,
    secret: Option<String>,
    clear_secret: bool,
    link_network: Option<LinkNetwork>,
}

fn endpoint(provider: Provider, value: &str) -> anyhow::Result<Url> {
    let invalid = "请输入该平台自定义机器人的完整 HTTPS Webhook 地址";
    ensure!(value.len() <= 2048, "{invalid}");
    let url = Url::parse(value).map_err(|_| anyhow::anyhow!(invalid))?;
    ensure!(url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
        && url.port().is_none() && url.fragment().is_none(), "{invalid}");
    match provider {
        Provider::DingTalk => {
            let mut query = url.query_pairs();
            let token = query.next().filter(|(name, _)| name == "access_token");
            ensure!(url.host_str() == Some("oapi.dingtalk.com") && url.path() == "/robot/send"
                && token.is_some_and(|(_, value)| !value.is_empty()) && query.next().is_none(), "{invalid}");
        }
        Provider::Feishu => {
            let key = url.path().strip_prefix("/open-apis/bot/v2/hook/").unwrap_or_default();
            ensure!(url.host_str() == Some("open.feishu.cn") && url.query().is_none() && !key.is_empty()
                && key.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'), "{invalid}");
        }
    }
    Ok(url)
}

fn updated(previous: &Robot, provider: Provider, patch: Update) -> anyhow::Result<Robot> {
    let mut next = previous.clone();
    if let Some(webhook) = patch.webhook {
        let url = endpoint(provider, webhook.trim())?;
        if next.webhook != url.as_str() {
            next.webhook = url.into();
            next.secret.clear();
        }
    }
    if patch.clear_secret { next.secret.clear(); }
    if let Some(secret) = patch.secret {
        let secret = secret.trim();
        ensure!(!secret.is_empty() && secret.len() <= 512 && !secret.chars().any(char::is_control), "签名密钥格式不正确");
        next.secret = secret.to_string();
    }
    if let Some(enabled) = patch.enabled { next.enabled = enabled; }
    if let Some(network) = patch.link_network { next.link_network = network; }
    ensure!(!next.enabled || !next.webhook.is_empty(), "请先保存机器人 Webhook");
    next.revision = Uuid::new_v4().to_string();
    Ok(next)
}

fn load(path: &Path) -> anyhow::Result<Settings> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("机器人配置文件格式不正确")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(_) => anyhow::bail!("无法读取机器人配置文件"),
    }
}

fn save(path: &Path, settings: &Settings) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = path.symlink_metadata() {
            ensure!(metadata.is_file(), "机器人配置路径不是普通文件");
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    let bytes = serde_json::to_vec(settings)?;
    crate::settings::atomic_write_with(path, |file| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&bytes)
    }).map_err(|_| anyhow::anyhow!("无法保存机器人配置文件"))
}

#[derive(Default)]
struct Store {
    path: Option<PathBuf>,
    settings: Settings,
    outcomes: HashMap<Provider, String>,
}

fn with_store<T>(run: impl FnOnce(&mut Store, &Path) -> anyhow::Result<T>) -> anyhow::Result<T> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    let path = super::state_path().with_file_name("chat-notifications.json");
    let mut store = STORE.get_or_init(Default::default).lock().unwrap_or_else(|error| error.into_inner());
    if store.path.as_deref() != Some(&path) {
        *store = Store { settings: load(&path)?, path: Some(path.clone()), outcomes: HashMap::new() };
    }
    run(&mut store, &path)
}

impl Store {
    fn public(&self) -> Value {
        let status = |provider| {
            let robot = self.settings.robot(provider);
            json!({ "enabled": robot.enabled, "hasWebhook": !robot.webhook.is_empty(),
                "hasSecret": !robot.secret.is_empty(), "linkNetwork": robot.link_network,
                "lastResult": self.outcomes.get(&provider) })
        };
        json!({ "dingtalk": status(Provider::DingTalk), "feishu": status(Provider::Feishu) })
    }
}

pub fn status() -> anyhow::Result<Value> { with_store(|store, _| Ok(store.public())) }

pub fn configure(provider: Provider, update: Option<Update>) -> anyhow::Result<Value> {
    with_store(|store, path| {
        let mut next = store.settings.clone();
        *next.robot_mut(provider) = match update {
            Some(update) => updated(store.settings.robot(provider), provider, update)?,
            None => Robot::default(),
        };
        save(path, &next)?;
        store.settings = next;
        store.outcomes.remove(&provider);
        Ok(store.public())
    })
}

fn remember(provider: Provider, revision: &str, result: &anyhow::Result<()>) {
    let _ = with_store(|store, _| {
        if store.settings.robot(provider).revision == revision {
            store.outcomes.insert(provider, result.as_ref().map(|_| "发送成功".into()).unwrap_or_else(|error| error.to_string()));
        }
        Ok(())
    });
}

fn client() -> anyhow::Result<Client> {
    Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(8))
        .build().map_err(|_| anyhow::anyhow!("无法初始化机器人连接"))
}

fn sign(key: &[u8], message: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(message);
    STANDARD.encode(mac.finalize().into_bytes())
}

fn request(provider: Provider, robot: &Robot, title: &str, text: &str, link: Option<(&str, &str)>, now_ms: u64) -> anyhow::Result<(Url, Value)> {
    let mut url = endpoint(provider, &robot.webhook)?;
    let body = match provider {
        Provider::DingTalk => {
            if !robot.secret.is_empty() {
                let signing = format!("{now_ms}\n{}", robot.secret);
                url.query_pairs_mut().append_pair("timestamp", &now_ms.to_string())
                    .append_pair("sign", &sign(robot.secret.as_bytes(), signing.as_bytes()));
            }
            let link = link.map(|(label, href)| format!("\n\n[{label}]({href})")).unwrap_or_default();
            json!({ "msgtype": "markdown", "markdown": { "title": title, "text": format!("### {title}\n\n{text}{link}") } })
        }
        Provider::Feishu => {
            let mut lines = vec![json!([{ "tag": "text", "text": text }])];
            if let Some((label, href)) = link { lines.push(json!([{ "tag": "a", "text": label, "href": href }])); }
            let mut body = json!({ "msg_type": "post", "content": { "post": { "zh_cn": { "title": title, "content": lines } } } });
            if !robot.secret.is_empty() {
                let seconds = (now_ms / 1000).to_string();
                body["sign"] = json!(sign(format!("{seconds}\n{}", robot.secret).as_bytes(), b""));
                body["timestamp"] = json!(seconds);
            }
            body
        }
    };
    Ok((url, body))
}

fn accepted(provider: Provider, response: &Value) -> anyhow::Result<()> {
    let code = match provider {
        Provider::DingTalk => response.get("errcode"),
        Provider::Feishu => response.get("code").or_else(|| response.get("StatusCode")),
    }.and_then(Value::as_i64).context("机器人返回了无法识别的响应")?;
    ensure!(code == 0, "机器人拒绝了消息（错误码 {code}）");
    Ok(())
}

async fn send(client: &Client, provider: Provider, robot: &Robot, title: &str, text: &str, link: Option<(&str, &str)>) -> anyhow::Result<()> {
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let (url, body) = request(provider, robot, title, text, link, now_ms)?;
    post(client, provider, url, &body).await
}

async fn post(client: &Client, provider: Provider, url: Url, body: &Value) -> anyhow::Result<()> {
    let mut response = client.post(url).json(body).send().await.map_err(|_| anyhow::anyhow!("无法连接机器人，请检查网络"))?;
    ensure!(response.status().is_success(), "机器人返回 HTTP {}", response.status().as_u16());
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| anyhow::anyhow!("未能读取机器人响应"))? {
        ensure!(bytes.len() + chunk.len() <= 16 * 1024, "机器人响应过大");
        bytes.extend_from_slice(&chunk);
    }
    accepted(provider, &serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("机器人返回了无法识别的响应"))?)
}

pub async fn test(provider: Provider) -> anyhow::Result<Value> {
    let robot = with_store(|store, _| Ok(store.settings.robot(provider).clone()))?;
    ensure!(!robot.webhook.is_empty(), "请先保存机器人 Webhook");
    let result = send(&client()?, provider, &robot, "Codex++ 测试通知", "机器人连接测试", None).await;
    remember(provider, &robot.revision, &result);
    result?;
    status()
}

#[derive(Clone)]
struct Notice { revision: String, kind: EventKind, thread: Uuid }

fn link(network: LinkNetwork, thread: Uuid, lan_port: Option<u16>) -> Option<String> {
    if network == LinkNetwork::None { return None; }
    let port = lan_port?;
    let address = super::tls::local_addresses().into_iter().find(|ip| super::tls::is_tailscale(ip) == (network == LinkNetwork::Tailscale))?;
    Some(format!("https://{}/#thread={thread}", std::net::SocketAddr::new(address, port)))
}

async fn deliver(client: &Client, provider: Provider, notice: Notice, lan_port: Option<u16>) {
    let Ok(robot) = with_store(|store, _| Ok(store.settings.robot(provider).clone())) else { return };
    if !robot.enabled || robot.revision != notice.revision { return; }
    let title = match notice.kind {
        EventKind::Waiting => "Codex++ 等待处理", EventKind::Done => "Codex++ 任务完成", EventKind::Failed => "Codex++ 任务出错",
    };
    let href = link(robot.link_network, notice.thread, lan_port);
    let label = if notice.kind == EventKind::Waiting { "查看并处理请求" } else { "查看会话" };
    let message = format!("会话 {}", &notice.thread.to_string()[..8]);
    let result = send(client, provider, &robot, title, &message, href.as_deref().map(|href| (label, href))).await;
    remember(provider, &robot.revision, &result);
}

pub(super) struct Worker {
    senders: Vec<(Provider, mpsc::Sender<Notice>)>,
    seen: VecDeque<(Provider, String, String)>,
}

pub(super) type Routes = Vec<(Provider, String)>;

pub(super) fn changes(id: &str, previous: Option<&Snapshot>, current: &Snapshot, just_enabled: bool, report_new_end: bool) -> Vec<Event> {
    if just_enabled { events::transitions(id, None, current, false) }
    else { events::transitions(id, previous, current, report_new_end) }
}

pub(super) fn routes() -> Routes {
    with_store(|store, _| Ok(PROVIDERS.into_iter().filter_map(|provider| {
        let robot = store.settings.robot(provider);
        robot.enabled.then(|| (provider, robot.revision.clone()))
    }).collect())).unwrap_or_default()
}

impl Worker {
    pub(super) fn start(lan_port: Option<u16>) -> Self {
        let senders = PROVIDERS.into_iter().map(|provider| {
            let (sender, mut receiver) = mpsc::channel::<Notice>(64);
            tokio::spawn(async move {
                let client = client();
                while let Some(notice) = receiver.recv().await {
                    match &client {
                        Ok(client) => deliver(client, provider, notice, lan_port).await,
                        Err(_) => remember(provider, &notice.revision, &Err(anyhow::anyhow!("无法初始化机器人连接"))),
                    }
                }
            });
            (provider, sender)
        }).collect();
        Self { senders, seen: VecDeque::new() }
    }

    pub(super) fn push(&mut self, (provider, revision): &(Provider, String), event: &Event) {
        let Ok(thread) = Uuid::parse_str(&event.thread_id) else { return };
        let id = if event.kind == EventKind::Waiting {
            event.request.as_ref().and_then(|request| request.get("id")).map(Value::to_string)
        } else { event.turn_id.clone() };
        let Some(id) = id else { return };
        let identity = (*provider, revision.clone(), format!("{thread}/{}/{id}", event.kind.id()));
        if self.seen.contains(&identity) { return; }
        let Some((_, sender)) = self.senders.iter().find(|(kind, _)| kind == provider) else { return };
        if sender.try_send(Notice { revision: revision.clone(), kind: event.kind, thread }).is_err() {
            remember(*provider, revision, &Err(anyhow::anyhow!("通知队列已满，本条消息未发送")));
            return;
        }
        if self.seen.len() == 1024 { self.seen.pop_front(); }
        self.seen.push_back(identity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{body_json, method, path}};

    const DING: &str = "https://oapi.dingtalk.com/robot/send?access_token=fixture-token";
    const FEISHU: &str = "https://open.feishu.cn/open-apis/bot/v2/hook/fixture-hook";
    const SECRET: &str = "fixture-signing-secret";
    const TIMESTAMP: u64 = 1_700_000_000_123;
    const THREAD: &str = "01992000-0000-7000-8000-000000000001";

    fn robot(webhook: &str) -> Robot {
        Robot { enabled: true, webhook: webhook.into(), secret: SECRET.into(), ..Robot::default() }
    }

    #[test]
    fn signing_follows_each_platforms_timestamp_unit_and_input_order() {
        let href = format!("https://192.168.1.2:57361/#thread={THREAD}");
        let link = Some(("查看并处理请求", href.as_str()));
        let (ding_url, ding_body) = request(Provider::DingTalk, &robot(DING), "Codex++ 测试通知", "消息", link, TIMESTAMP).unwrap();
        let query: HashMap<_, _> = ding_url.query_pairs().collect();
        assert_eq!(query["timestamp"], "1700000000123");
        assert_eq!(query["sign"], "Ls6YXL72VGUqOIUTMz5OooisS8VeWrP0x+rN30juI8Q=");
        assert_eq!(ding_body["msgtype"], "markdown");
        assert!(ding_body["markdown"]["text"].as_str().unwrap().contains(&format!("[查看并处理请求]({href})")));
        let (fs_url, fs_body) = request(Provider::Feishu, &robot(FEISHU), "Codex++ 测试通知", "消息", link, TIMESTAMP).unwrap();
        assert_eq!(fs_url.as_str(), FEISHU);
        assert_eq!(fs_body["timestamp"], "1700000000");
        assert_eq!(fs_body["sign"], "p7jQQAwRH+waZB1I+OqmysAwmjZVKfbEC1Fpr/9O/Ns=");
        assert_eq!(fs_body.pointer("/content/post/zh_cn/content/1/0/href"), Some(&json!(href)));
        let unsigned = Robot { webhook: FEISHU.into(), ..Default::default() };
        let (_, unsigned_body) = request(Provider::Feishu, &unsigned, "Codex++", "消息", None, TIMESTAMP).unwrap();
        assert!(unsigned_body.get("sign").is_none());
        assert!(unsigned_body.get("timestamp").is_none());
    }

    #[test]
    fn settings_keep_credentials_private_across_save_edit_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("robots.json");
        let empty = load(&path).unwrap();
        assert!(!empty.dingtalk.enabled && !empty.feishu.enabled);
        let settings = Settings { dingtalk: robot(DING), feishu: robot(FEISHU) };
        save(&path, &settings).unwrap();
        let stored = load(&path).unwrap();
        let public = Store { settings: stored.clone(), ..Default::default() }.public();
        assert_eq!(public["dingtalk"]["hasWebhook"], true);
        assert_eq!(public["feishu"]["hasSecret"], true);
        assert!(!public.to_string().contains("fixture"));
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let paused = updated(&stored.dingtalk, Provider::DingTalk, Update { enabled: Some(false), ..Default::default() }).unwrap();
        assert_eq!(paused.secret, SECRET);
        assert_eq!(paused.webhook, DING);
        let replacement = DING.replace("fixture-token", "new-fixture");
        let changed = updated(&paused, Provider::DingTalk, Update { webhook: Some(replacement), ..Default::default() }).unwrap();
        assert!(changed.secret.is_empty());
        assert_ne!(changed.revision, paused.revision);
        assert!(updated(&Robot::default(), Provider::DingTalk, Update { enabled: Some(true), ..Default::default() }).is_err());
        assert!(endpoint(Provider::Feishu, DING).is_err());
        assert!(endpoint(Provider::DingTalk, FEISHU).is_err());
        assert!(endpoint(Provider::DingTalk, "https://oapi.dingtalk.com/robot/send").is_err());
    }

    #[test]
    fn new_robot_reports_pending_requests_without_replaying_finished_tasks() {
        let previous = events::snapshot(&json!({"turns":[{"turnId":"one","status":"inProgress"}]}));
        let current = events::snapshot(&json!({
            "turns":[{"turnId":"one","status":"completed"}],
            "requests":[{"id":7,"method":"item/commandExecution/requestApproval"}]
        }));
        let finished = events::snapshot(&json!({"turns":[{"turnId":"two","status":"failed"}]}));
        let kinds = |items: Vec<Event>| items.into_iter().map(|item| item.kind).collect::<Vec<_>>();
        assert_eq!(kinds(changes(THREAD, Some(&previous), &current, true, true)), [EventKind::Waiting]);
        assert_eq!(kinds(changes(THREAD, Some(&previous), &current, false, true)), [EventKind::Waiting, EventKind::Done]);
        assert!(changes(THREAD, Some(&current), &current, false, true).is_empty());
        assert_eq!(kinds(changes(THREAD, None, &finished, false, true)), [EventKind::Failed]);
        assert!(changes(THREAD, None, &finished, false, false).is_empty());
        let thread = Uuid::parse_str(THREAD).unwrap();
        assert!(link(LinkNetwork::None, thread, Some(57361)).is_none());
        assert!(link(LinkNetwork::Lan, thread, None).is_none());
    }

    #[tokio::test]
    async fn delivers_json_and_reports_application_errors_even_when_http_succeeds() {
        let mock = MockServer::start().await;
        for (provider, webhook, route, success) in [(Provider::DingTalk, DING, "ding", json!({"errcode":0})),
            (Provider::Feishu, FEISHU, "feishu", json!({"code":0}))] {
            let (_, body) = request(provider, &robot(webhook), "Codex++", "测试", None, TIMESTAMP).unwrap();
            Mock::given(method("POST")).and(path(format!("/{route}"))).and(body_json(&body))
                .respond_with(ResponseTemplate::new(200).set_body_json(success)).expect(1).mount(&mock).await;
            post(&client().unwrap(), provider, format!("{}/{route}", mock.uri()).parse().unwrap(), &body).await.unwrap();
        }
        let rejected = json!({"code": 19021, "msg": SECRET});
        Mock::given(method("POST")).and(path("/rejected"))
            .respond_with(ResponseTemplate::new(200).set_body_json(rejected)).expect(1).mount(&mock).await;
        let result = post(&client().unwrap(), Provider::Feishu, format!("{}/rejected", mock.uri()).parse().unwrap(), &json!({})).await;
        let error = result.unwrap_err().to_string();
        assert!(error.contains("19021"));
        assert!(!error.contains(SECRET));
        assert!(accepted(Provider::DingTalk, &json!({"other":0})).is_err());
    }
}
