use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

const OBSERVER: &str = include_str!("live_source.js");
const BINDING: &str = "xuanMobileReplyDelta";
const OBSERVER_KEY: &str = "__xuanMobileReplyObserver";
const MAX_LIVE_REPLY_EVENT_BYTES: usize = 1024 * 1024;
const OBSERVER_RENEW_INTERVAL: Duration = Duration::from_secs(15);
const OBSERVER_LEASE_MILLIS: u64 = 45_000;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReplyEvent {
    pub thread_id: String,
    pub item_id: String,
    pub sequence: u64,
    pub text: String,
    pub complete: bool,
}

#[derive(Default)]
pub(super) struct ReplyOverlay {
    item_id: String,
    sequence: u64,
    base: String,
    text: String,
    complete: bool,
    retired: Vec<String>,
}

impl ReplyOverlay {
    pub fn apply(&mut self, event: ReplyEvent, history: &str) -> bool {
        if self.retired.contains(&event.item_id)
            || (self.item_id == event.item_id && event.sequence <= self.sequence)
        {
            return false;
        }
        if self.item_id != event.item_id {
            let base = self.render(history);
            if !self.item_id.is_empty() {
                self.retired.push(self.item_id.clone());
                if self.retired.len() > 32 {
                    self.retired.remove(0);
                }
            }
            self.base = base;
            self.item_id = event.item_id;
        }
        self.sequence = event.sequence;
        self.text = event.text;
        self.complete = event.complete;
        true
    }

    pub fn render(&self, history: &str) -> String {
        if self.text.is_empty() {
            return history.to_owned();
        }
        let projected = if self.base.is_empty() {
            self.text.clone()
        } else {
            format!("{}\n\n---\n\n{}", self.base, self.text)
        };
        // 日志追平或超前时以持久记录为准，否则保留正在生成且尚未落盘的尾部。
        if history.starts_with(&projected) {
            history.to_owned()
        } else if projected.starts_with(history) {
            projected
        } else if self.complete {
            history.to_owned()
        } else {
            projected
        }
    }

    pub fn pending(&self, history: &str) -> bool {
        self.render(history) != history
    }

    pub fn persisted(&self, item_id: &str) -> bool {
        !item_id.is_empty() && self.item_id == item_id
    }
}

pub(super) async fn run(
    mut scope: watch::Receiver<BTreeSet<String>>,
    output: mpsc::Sender<ReplyEvent>,
) {
    loop {
        let empty = scope.borrow().is_empty();
        if empty && scope.changed().await.is_err() {
            return;
        }
        if scope.borrow().is_empty() {
            continue;
        }
        // 只复用轩++已启动的本机调试端口，不扫描其他进程或改变桌面启动参数。
        #[cfg(not(test))]
        let port = crate::status::StatusStore::default()
            .load_latest()
            .ok()
            .flatten()
            .and_then(|status| status.debug_port);
        // 自动化测试只能连接显式构造的本机夹具，不能误连正在使用的桌面。
        #[cfg(test)]
        let port: Option<u16> = None;
        if let Some(port) = port {
            let _ = session(port, &mut scope, &output).await;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            changed = scope.changed() => if changed.is_err() { return; }
        }
    }
}

pub(super) fn set_scope(sender: &watch::Sender<BTreeSet<String>>, selected: &BTreeSet<String>) {
    sender.send_if_modified(|current| {
        if current == selected {
            false
        } else {
            current.clone_from(selected);
            true
        }
    });
}

async fn session(
    port: u16,
    scope: &mut watch::Receiver<BTreeSet<String>>,
    output: &mpsc::Sender<ReplyEvent>,
) -> anyhow::Result<()> {
    let targets = crate::cdp::list_targets(port).await?;
    let target = targets
        .iter()
        .find(|target| {
            crate::cdp::is_primary_codex_page_target(target) && target.url.starts_with("app://-/")
        })
        .context("桌面回复事件入口暂不可用")?;
    let url = target
        .web_socket_debugger_url
        .as_deref()
        .context("桌面回复连接暂不可用")?;
    crate::cdp::validate_cdp_websocket_url(url, port)?;
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(url),
    )
    .await??;
    let mut command_id = 1_u64;
    super::send(
        &mut socket,
        &json!({"id": command_id, "method": "Runtime.addBinding",
        "params": {"name": BINDING}}),
    )
    .await?;
    let mut lease = tokio::time::interval(OBSERVER_RENEW_INTERVAL);
    lease.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut observer_installed = false;
    let result = async {
        loop {
            tokio::select! {
                changed = scope.changed() => {
                    if changed.is_err() || scope.borrow().is_empty() { break; }
                    lease.reset_immediately();
                }
                _ = lease.tick() => {
                    let selected = serde_json::to_string(&*scope.borrow())?;
                    command_id += 1;
                    let expression = if observer_installed {
                        format!(
                            "window[{key}]?.renew({selected}, {OBSERVER_LEASE_MILLIS}) ?? false",
                            key = serde_json::to_string(OBSERVER_KEY)?,
                        )
                    } else {
                        observer_installed = true;
                        format!(
                            "({OBSERVER})({selected}, {binding}, {OBSERVER_LEASE_MILLIS})",
                            binding = serde_json::to_string(BINDING)?,
                        )
                    };
                    super::send(&mut socket, &json!({"id": command_id, "method": "Runtime.evaluate",
                        "params": {"expression": expression, "returnByValue": true}})).await?;
                }
                message = socket.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            let value: Value = serde_json::from_str(&text)?;
                            if value["error"].is_object() { bail!("桌面回复观察暂不可用"); }
                            if value["method"] != "Runtime.bindingCalled" || value["params"]["name"] != BINDING {
                                continue;
                            }
                            let event: ReplyEvent = serde_json::from_str(
                                value["params"]["payload"].as_str().context("回复事件格式无效")?
                            )?;
                            if !scope.borrow().contains(&event.thread_id) || event.item_id.is_empty()
                                || event.item_id.len() > 128 || event.sequence == 0
                                || event.text.len() > MAX_LIVE_REPLY_EVENT_BYTES
                            {
                                continue;
                            }
                            // 累计正文可覆盖队列中旧片段；队列拥堵时仍由最终日志补齐。
                            let _ = output.try_send(event);
                        }
                        Some(Ok(Message::Ping(bytes))) => socket.send(Message::Pong(bytes)).await?,
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Err(error)) => return Err(error.into()),
                        _ => {}
                    }
                }
            }
        }
        anyhow::Ok(())
    }.await;
    let cleanup = async {
        super::send(
            &mut socket,
            &json!({"id": command_id + 1, "method": "Runtime.evaluate",
            "params": {"expression": "window.__xuanMobileReplyObserver?.dispose()"}}),
        )
        .await?;
        super::send(
            &mut socket,
            &json!({"id": command_id + 2, "method": "Runtime.removeBinding",
            "params": {"name": BINDING}}),
        )
        .await?;
        socket.close(None).await?;
        anyhow::Ok(())
    };
    let _ = tokio::time::timeout(Duration::from_secs(2), cleanup).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(item: &str, sequence: u64, text: &str, complete: bool) -> ReplyEvent {
        ReplyEvent {
            thread_id: "fixture_task_0001".into(),
            item_id: item.into(),
            sequence,
            text: text.into(),
            complete,
        }
    }

    #[test]
    fn live_delta_is_visible_before_log_and_final_history_does_not_duplicate() {
        let mut overlay = ReplyOverlay::default();
        assert!(overlay.apply(event("item1", 1, "", false), "历史"));
        overlay.apply(event("item1", 2, "正在", false), "历史");
        assert_eq!(overlay.render("历史"), "历史\n\n---\n\n正在");
        assert!(!overlay.apply(event("item1", 1, "旧", false), "历史"));
        overlay.apply(event("item1", 3, "正在生成", true), "历史");
        let full = "历史\n\n---\n\n正在生成";
        assert_eq!(overlay.render("历史"), full);
        assert_eq!(overlay.render(full), full);
        assert!(!overlay.pending(full));
        overlay.apply(event("item2", 1, "正在生成", true), full);
        assert_eq!(overlay.render(full), format!("{full}\n\n---\n\n正在生成"));
        assert!(!overlay.apply(event("item1", 4, "旧消息", true), full));
        assert!(overlay.persisted("item2"));
    }

    #[tokio::test]
    async fn cdp_source_delivers_events_and_unsubscribes_without_executing_commands() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut http, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 4096];
            let read = http.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /json "));
            let body = json!([{"id": "fixture", "type": "page", "title": "Codex",
                "url": "app://-/index.html",
                "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}/devtools/page/fixture")}])
            .to_string();
            http.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            http.shutdown().await.unwrap();
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let mut delivered = false;
            let mut removed = false;
            while let Some(message) = socket.next().await {
                match message.unwrap() {
                    Message::Text(text) => {
                        let command: Value = serde_json::from_str(&text).unwrap();
                        assert!(matches!(
                            command["method"].as_str().unwrap(),
                            "Runtime.addBinding" | "Runtime.evaluate" | "Runtime.removeBinding"
                        ));
                        if command["method"] == "Runtime.evaluate" && !delivered {
                            assert!(
                                command["params"]["expression"]
                                    .as_str()
                                    .unwrap()
                                    .contains("fixture_task_0001")
                            );
                            let event = json!({"threadId":"fixture_task_0001","itemId":"item1",
                                "sequence":1,"text":"真实传输夹具","complete":false});
                            let notification = json!({"method":"Runtime.bindingCalled",
                                "params":{"name":BINDING,"payload":event.to_string()}});
                            socket
                                .send(Message::Text(notification.to_string().into()))
                                .await
                                .unwrap();
                            delivered = true;
                        }
                        if command["method"] == "Runtime.removeBinding" {
                            removed = true;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            assert!(delivered && removed);
        });
        let (scope_tx, mut scope) =
            watch::channel(BTreeSet::from(["fixture_task_0001".to_owned()]));
        let (output, mut events) = mpsc::channel(4);
        let client = tokio::spawn(async move { session(port, &mut scope, &output).await });
        let received = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.text, "真实传输夹具");
        scope_tx.send_replace(BTreeSet::new());
        tokio::time::timeout(Duration::from_secs(10), client)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .unwrap()
            .unwrap();
    }
}
