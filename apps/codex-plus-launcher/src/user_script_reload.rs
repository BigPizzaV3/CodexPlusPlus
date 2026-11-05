use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};

const PAGE_REQUEST_RECEIPT_KEY: &str = "__codexPlusUserScriptsPageRequest";

static NEXT_SEQUENCE: std::sync::LazyLock<AtomicU64> = std::sync::LazyLock::new(|| {
    let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    AtomicU64::new(time.saturating_mul(1024))
});

pub(super) fn next_sequence() -> u64 {
    NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone)]
pub(super) struct ReloadOwner {
    state: Arc<Mutex<HashMap<String, Value>>>,
    epoch: String,
    retired_before: Arc<AtomicU64>,
    operations: Arc<tokio::sync::Mutex<()>>,
    execution: Arc<tokio::sync::Mutex<()>>,
    desired_probes: Arc<Mutex<DesiredProbes>>,
}

#[derive(Default)]
struct DesiredProbes {
    running: bool,
    completed: HashMap<String, (u64, Option<String>)>,
    sequence: u64,
}

impl Default for ReloadOwner {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            epoch: codex_plus_core::user_script_control::new_owner_epoch(),
            retired_before: Arc::new(AtomicU64::new(0)),
            operations: Arc::new(tokio::sync::Mutex::new(())),
            execution: Arc::new(tokio::sync::Mutex::new(())),
            desired_probes: Arc::new(Mutex::new(DesiredProbes::default())),
        }
    }
}

impl ReloadOwner {
    pub(super) fn epoch(&self) -> &str { &self.epoch }

    pub(super) fn control_response(&self, operation: Value, verification: Option<&str>, manager: &UserScriptManager) -> Value {
        let (desired, pending) = if operation["phase"] == "applied" {
            let manager = manager.clone();
            self.desired_revision(&operation, verification, move || manager.reload_snapshot().ok().map(|snapshot| snapshot.1))
        } else { (None, false) };
        json!({ "reloadOperation": operation, "desiredRevision": desired, "desiredRevisionPending": pending })
    }

    fn desired_revision(
        &self,
        operation: &Value,
        verification: Option<&str>,
        read: impl FnOnce() -> Option<String> + Send + 'static,
    ) -> (Option<String>, bool) {
        let Some(verification) = verification.filter(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())) else {
            return (None, false);
        };
        let key = format!("{}\0{verification}", operation["operationId"].as_str().unwrap_or_default());
        let mut probes = self.desired_probes.lock().unwrap();
        if let Some((_, result)) = probes.completed.get(&key) {
            return (result.clone(), false);
        }
        if probes.running { return (None, true); }
        probes.running = true;
        let state = self.desired_probes.clone();
        // 全 owner 同时只有一次文件探测。等待文件锁的任务不随客户端超时反复创建。
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(read).await.ok().flatten();
            let mut probes = state.lock().unwrap();
            probes.running = false;
            probes.sequence = probes.sequence.wrapping_add(1);
            let sequence = probes.sequence;
            if probes.completed.len() >= 256 {
                if let Some(oldest) = probes.completed.iter().min_by_key(|(_, value)| value.0).map(|(key, _)| key.clone()) {
                    probes.completed.remove(&oldest);
                }
            }
            probes.completed.insert(key, (sequence, result));
        });
        (None, true)
    }

    pub(super) fn submit(
        &self,
        payload: &Value,
        websocket: Arc<Mutex<Option<String>>>,
        manager: UserScriptManager,
    ) -> anyhow::Result<Value> {
        use codex_plus_core::user_script_control::{new_operation_id, now_ms, operation_issued_at};
        let id = payload.get("operationId").and_then(Value::as_str)
            .filter(|id| !id.is_empty()).map(str::to_owned).unwrap_or_else(|| new_operation_id(&self.epoch));
        let issued_at = operation_issued_at(&self.epoch, &id).context("重载操作编号无效或属于旧 owner")?;
        anyhow::ensure!(issued_at <= now_ms().saturating_add(30_000), "重载操作编号时间无效");
        let allow_page = payload.get("allowPage").and_then(Value::as_bool).unwrap_or(true);
        let record = json!({ "operationId": id, "ownerEpoch": self.epoch, "allowPage": allow_page, "phase": "accepted", "active": true,
            "issuedAt": issued_at,
            "createdAt": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 });
        {
            let mut state = self.state.lock().unwrap();
            if let Some(previous) = state.get(&id) {
                anyhow::ensure!(previous["allowPage"] == allow_page, "重载操作编号与已有请求冲突");
                return Ok(previous.clone());
            }
            anyhow::ensure!(issued_at > self.retired_before.load(Ordering::Relaxed),
                "重载操作编号已过期；未重新执行");
            anyhow::ensure!(state.values().filter(|value| value["active"] == true).count() < 32,
                "等待确认的重载操作过多，请稍后再试");
            if state.len() >= 4096 {
                let oldest = state.iter().filter(|(_, value)| value["active"] != true)
                    .min_by_key(|(_, value)| value["issuedAt"].as_u64().unwrap_or_default())
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    if let Some(retired) = state.remove(&oldest) {
                        // 高水位墓碑永久保留在本 owner 内；淘汰详情不能让旧编号重新执行。
                        self.retired_before.fetch_max(retired["issuedAt"].as_u64().unwrap_or_default(), Ordering::Relaxed);
                    }
                }
            }
            anyhow::ensure!(issued_at > self.retired_before.load(Ordering::Relaxed),
                "重载操作编号已过期；未重新执行");
            anyhow::ensure!(state.len() < 4096, "重载操作过多，请稍后再试");
            state.insert(id.clone(), record.clone());
        }
        let owner = self.clone();
        tokio::spawn(async move {
            owner.observe(&id, Duration::from_secs(45), owner.run(&id, allow_page, &websocket, &manager)).await;
        });
        Ok(record)
    }

    async fn observe(&self, id: &str, warning_after: Duration, operation: impl std::future::Future<Output = ()>) {
        tokio::pin!(operation);
        tokio::select! {
            _ = &mut operation => {}
            _ = tokio::time::sleep(warning_after) => {
                self.update(id, "unknown", json!({ "reason": "等待确认超时；原操作仍在协调中", "active": true }));
                operation.await;
            }
        }
    }

    pub(super) fn status(&self, id: &str) -> Option<Value> {
        self.state.lock().unwrap().get(id).cloned()
    }

    pub(super) fn control_status(&self, id: &str) -> Value {
        if let Some(record) = self.status(id) { return record; }
        let retired = codex_plus_core::user_script_control::operation_issued_at(&self.epoch, id)
            .is_none_or(|issued_at| issued_at <= self.retired_before.load(Ordering::Relaxed));
        json!({
            "operationId": id, "ownerEpoch": self.epoch, "active": false,
            "phase": if retired { "rejected" } else { "not_found" },
            "reason": if retired { "此操作编号已过期或属于旧 owner；未重新执行" } else { "此操作尚未登记" },
        })
    }

    pub(super) async fn lock_initial_load(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.execution.lock().await
    }

    fn update(&self, id: &str, phase: &str, fields: Value) {
        let mut state = self.state.lock().unwrap();
        let Some(record) = state.get_mut(id) else { return };
        record["phase"] = json!(phase);
        if matches!(phase, "applied" | "failed" | "blocked" | "degraded" | "rejected") {
            record["active"] = json!(false);
        }
        if let Some(fields) = fields.as_object() {
            for (key, value) in fields {
                record[key] = value.clone();
            }
        }
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "user_scripts.reload_stage", json!({ "operationId": id, "phase": phase, "details": fields }),
        );
    }

    async fn run(&self, id: &str, allow_page: bool, websocket: &Arc<Mutex<Option<String>>>, manager: &UserScriptManager) {
        // Keep other reloads out even while a new document's bootstrap is allowed to run.
        let _operation_guard = self.operations.lock().await;
        let mut attempt = 0_u64;
        loop {
            let guard = self.execution.lock().await;
            attempt += 1;
            let operation_id = format!("{id}-{attempt:x}");
            let sequence = next_sequence();
            let (bundle, revision) = match snapshot(manager).await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    self.update(id, "failed", json!({ "reason": format!("{error:#}") }));
                    return;
                }
            };
            self.update(id, "accepted", json!({ "revision": revision, "attempt": attempt }));
            let before = self.wait_probe(id, websocket, &operation_id).await;
            let document = before["documentId"].as_str().unwrap_or_default().to_string();
            if document.is_empty() { continue; }
            let mut generation = codex_plus_core::extension_backend::backend_generation(manager);
            let source_revoked = allow_page || before["cleanupSupported"] == true;
            if source_revoked {
                match codex_plus_core::extension_backend::revoke_backend_document(manager, generation).await {
                    Some(next) => generation = next,
                    None => {
                        self.update(id, "degraded", json!({ "reason": "后台代际已变化，未开始页面清理" }));
                        return;
                    }
                }
            }
            self.update(id, "quiescing", json!({ "documentId": document }));
            let preparation = json!({
                "id": operation_id, "documentId": document, "revision": revision,
                "allowPage": allow_page, "sequence": sequence,
            });
            let prepare = build_prepare_with_page_receipt(&preparation);
            // A timeout does not cancel CDP execution. Never resend this prepare.
            let acknowledged = evaluate(websocket, &prepare).await.ok();
            if let Some(response) = &acknowledged {
                if response["phase"] == "rejected" || response["phase"] == "failed" {
                    self.update(id, "failed", json!({ "reason": response["reason"] }));
                    return;
                }
            }
            let prepared = self.wait_preparation_with_probe(id, &preparation, acknowledged, || {
                self.wait_probe(id, websocket, &operation_id)
            }).await;
            let Some(prepared) = prepared else {
                // Navigation can destroy the prepare response before it is read.
                // Let the new document's bootstrap acquire the same execution permit.
                drop(guard);
                if self.verify_navigation(id, websocket, &operation_id, &document, "navigation", manager).await {
                    return;
                }
                continue;
            };
            match prepared["phase"].as_str().unwrap_or_default() {
                "blocked" if source_revoked => {
                    self.update(id, "degraded", json!({ "reason": "页面清理能力发生变化，旧后台来源已撤销；需要重新加载页面" }));
                    return;
                }
                "blocked" | "degraded" | "failed" | "rejected" => {
                    self.update(id, prepared["phase"].as_str().unwrap_or("failed"), json!({ "reason": prepared["reason"] }));
                    return;
                }
                "pageRequested" => {
                    self.update(id, "pageRequested", json!({ "mode": "page", "reason": prepared["reason"] }));
                    drop(guard);
                    if self.verify_navigation(id, websocket, &operation_id, &document, "page", manager).await {
                        return;
                    }
                    continue;
                }
                "stopped" => {}
                _ => {
                    self.update(id, "failed", json!({ "reason": "无法确认页面清理状态" }));
                    return;
                }
            }
            self.update(id, "stopped", json!({}));
            if snapshot(manager).await.ok().map(|snapshot| snapshot.1) != Some(revision.clone()) {
                continue;
            }
            if !codex_plus_core::extension_backend::stop_all_if_generation(manager, generation).await {
                self.update(id, "degraded", json!({ "reason": "后台代际已变化，页面已清理但未重新加载" }));
                return;
            }
            let (staging_manager, staging_revision) = (manager.clone(), revision.clone());
            let staged = match tokio::task::spawn_blocking(move || staging_manager.stage_backend_snapshot(&staging_revision))
                .await.context("后台快照准备任务失败").and_then(|result| result)
            {
                Ok(staged) => staged,
                Err(error) => {
                    self.update(id, "degraded", json!({ "reason": format!("无法固定后台快照：{error:#}") }));
                    return;
                }
            };
            let backend_source = match codex_plus_core::extension_backend::bind_backend_snapshot(
                manager, generation.wrapping_add(1), &document, staged,
            ).await {
                Ok(source) => source,
                Err(error) => {
                    self.update(id, "degraded", json!({ "reason": format!("无法绑定页面后台代际：{error:#}") }));
                    return;
                }
            };
            self.update(id, "applying", json!({}));
            let apply = UserScriptManager::build_apply_operation(&operation_id, &document, &revision, &bundle, &backend_source);
            // As with prepare, a lost response is resolved by querying the same operation.
            if let Ok(response) = evaluate(websocket, &apply).await {
                if response["operation"]["phase"] == "rejected" || response["operation"]["phase"] == "failed" {
                    self.update(id, "failed", json!({ "reason": response["operation"]["reason"] }));
                    return;
                }
            }
            loop {
                let probe = self.wait_probe(id, websocket, &operation_id).await;
                if probe["documentId"] != document { break; }
                let phase = probe["operation"]["phase"].as_str().unwrap_or_default();
                if phase == "failed" || phase == "rejected" || phase == "superseded" {
                    self.update(id, "failed", json!({ "reason": probe["operation"]["reason"] }));
                    return;
                }
                if phase == "verifying" && probe["revision"] == revision && probe["ready"] == true {
                    if !codex_plus_core::extension_backend::backend_source_is_current(manager, &probe["backendSource"]) {
                        self.update(id, "degraded", json!({ "reason": "页面脚本已执行，但后台来源已失效" }));
                        return;
                    }
                    if snapshot(manager).await.ok().map(|snapshot| snapshot.1) != Some(revision.clone()) {
                        break;
                    }
                    self.finish(id, "scripts", probe, manager).await;
                    return;
                }
                self.update(id, "unknown", json!({ "reason": "等待页面确认应用操作" }));
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }

    async fn wait_preparation_with_probe<F, Fut>(
        &self,
        id: &str,
        preparation: &Value,
        acknowledged: Option<Value>,
        probe: F,
    ) -> Option<Value>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Value>,
    {
        // 已确认的页面刷新不能被下一次探测的新文档覆盖。
        if let Some(operation) = acknowledged.filter(|value| page_receipt_matches(value, preparation)) {
            return Some(operation);
        }
        loop {
            let observed = probe().await;
            let receipt = &observed["pageRequestReceipt"];
            if page_receipt_matches(receipt, preparation) {
                return Some(receipt.clone());
            }
            if observed["documentId"] != preparation["documentId"] {
                return None;
            }
            if observed["operation"].is_object() {
                return Some(observed["operation"].clone());
            }
            self.update(id, "unknown", json!({ "reason": "等待页面确认清理操作" }));
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn verify_navigation(
        &self,
        id: &str,
        websocket: &Arc<Mutex<Option<String>>>,
        operation_id: &str,
        previous_document: &str,
        mode: &str,
        manager: &UserScriptManager,
    ) -> bool {
        self.verify_navigation_with_probe(id, previous_document, mode, manager, || {
            self.wait_probe(id, websocket, operation_id)
        }).await
    }

    async fn verify_navigation_with_probe<F, Fut>(
        &self,
        id: &str,
        previous_document: &str,
        mode: &str,
        manager: &UserScriptManager,
        probe: F,
    ) -> bool
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Value>,
    {
        loop {
            let observed = probe().await;
            if observed["documentId"] != previous_document && observed["initialized"] != true && observed["initialError"].is_string() {
                self.update(id, "blocked", json!({
                    "mode": mode, "reason": "新页面初始化未完成；请检查拓展状态或重新设置下次启动跳过选项",
                }));
                return true;
            }
            if observed["documentId"] != previous_document && observed["initialized"] == true && observed["ready"] == true {
                let _guard = self.execution.lock().await;
                let confirmed = probe().await;
                if confirmed["documentId"] != observed["documentId"]
                    || confirmed["initialized"] != true || confirmed["ready"] != true
                {
                    continue;
                }
                if confirmed["initialMode"] == "skipped" {
                    self.update(id, "blocked", json!({ "mode": mode, "reason": "新页面按设置跳过了本次拓展加载" }));
                    return true;
                }
                if !codex_plus_core::extension_backend::backend_source_is_current(manager, &confirmed["backendSource"]) {
                    self.update(id, "degraded", json!({ "mode": mode, "reason": "新页面后台来源与当前会话不一致" }));
                    return true;
                }
                let current = match snapshot(manager).await {
                    Ok((_, revision)) => revision,
                    Err(error) => {
                        self.update(id, "degraded", json!({ "reason": format!("无法确认当前拓展快照：{error:#}") }));
                        return true;
                    }
                };
                if confirmed["revision"].as_str() == Some(current.as_str()) {
                    self.finish(id, mode, confirmed, manager).await;
                    return true;
                }
                return false;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn wait_probe(&self, id: &str, websocket: &Arc<Mutex<Option<String>>>, operation_id: &str) -> Value {
        loop {
            if let Ok(value) = evaluate(websocket, &build_probe_with_page_receipt(operation_id)).await {
                if value["documentId"].is_string() { return value; }
            }
            self.update(id, "unknown", json!({ "reason": "页面暂未响应，操作结果待确认" }));
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn finish(&self, id: &str, mode: &str, probe: Value, manager: &UserScriptManager) {
        let (inventory_manager, runtime) = (manager.clone(), probe.get("scripts").cloned());
        let inventory = match tokio::task::spawn_blocking(move || inventory_manager.inventory_with_runtime_status(runtime.as_ref()))
            .await.context("拓展清单确认任务失败").and_then(|result| result)
        {
            Ok(inventory) => inventory,
            Err(error) => {
                self.update(id, "degraded", json!({ "reason": format!("无法确认拓展清单：{error:#}") }));
                return;
            }
        };
        let (phase, reason) = inventory_verdict(&inventory, probe.get("scripts"));
        self.update(id, phase, json!({
            "mode": mode, "documentId": probe["documentId"], "revision": probe["revision"], "inventory": inventory,
            "reason": reason,
        }));
    }
}

fn page_receipt_matches(receipt: &Value, preparation: &Value) -> bool {
    receipt["phase"] == "pageRequested" && receipt["allowPage"] == true
        && ["id", "documentId", "revision", "allowPage", "sequence"].iter()
            .all(|key| receipt.get(*key).is_some_and(|value| preparation.get(*key) == Some(value)))
}

fn build_prepare_with_page_receipt(preparation: &Value) -> String {
    let prepare = UserScriptManager::build_prepare_operation(
        preparation["id"].as_str().unwrap(),
        preparation["documentId"].as_str().unwrap(),
        preparation["revision"].as_str().unwrap(),
        preparation["allowPage"].as_bool().unwrap(),
        preparation["sequence"].as_u64().unwrap(),
    );
    format!(r#"(() => {{
  const encoded = {prepare};
  const operation = JSON.parse(encoded);
  const expected = {preparation};
  if (operation.phase === "pageRequested" && operation.allowPage === true &&
      ["id", "documentId", "revision", "allowPage", "sequence"].every(key => operation[key] === expected[key])) {{
    // 单条收据随同一标签页刷新保留；旧 attempt 不得证明新操作触发了刷新。
    try {{ window.sessionStorage.setItem({key}, JSON.stringify(operation)); }} catch {{}}
  }}
  return encoded;
}})()"#, key = json!(PAGE_REQUEST_RECEIPT_KEY))
}

fn build_probe_with_page_receipt(operation_id: &str) -> String {
    let probe = UserScriptManager::build_operation_probe(operation_id);
    format!(r#"(() => {{
  const probe = JSON.parse({probe});
  try {{
    const receipt = window.sessionStorage.getItem({key});
    if (receipt && receipt.length <= 2048) probe.pageRequestReceipt = JSON.parse(receipt);
  }} catch {{}}
  return JSON.stringify(probe);
}})()"#, key = json!(PAGE_REQUEST_RECEIPT_KEY))
}

async fn snapshot(manager: &UserScriptManager) -> anyhow::Result<(String, String)> {
    let manager = manager.clone();
    tokio::task::spawn_blocking(move || manager.reload_snapshot()).await.context("读取拓展快照任务失败")?
}

fn inventory_verdict(inventory: &Value, runtime: Option<&Value>) -> (&'static str, &'static str) {
    if runtime.and_then(Value::as_object).is_none() {
        return ("degraded", "无法确认页面中的拓展状态");
    }
    let Some(scripts) = inventory["scripts"].as_array() else {
        return ("degraded", "无法确认拓展清单");
    };
    for script in scripts {
        let configured = script["configuredEnabled"].as_bool();
        let state = script["runtimeStatus"].as_str();
        if state == Some("failed") && configured == Some(true) {
            return ("failed", "页面中的拓展加载失败");
        }
        if !matches!((configured, state), (Some(true), Some("loaded")) | (Some(false), Some("not_loaded"))) {
            return ("degraded", "页面拓展状态与配置不一致");
        }
    }
    ("applied", "")
}

async fn evaluate(websocket: &Arc<Mutex<Option<String>>>, expression: &str) -> anyhow::Result<Value> {
    let url = websocket.lock().unwrap().clone().context("Codex 页面尚未连接")?;
    let response = codex_plus_core::bridge::evaluate_script(&url, expression).await?;
    let encoded = response.pointer("/result/result/value").and_then(Value::as_str).context("页面没有返回操作状态")?;
    Ok(serde_json::from_str(encoded)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_preparation() -> Value {
        json!({
            "id": "reload-attempt-1", "documentId": "old-document",
            "revision": "current-revision", "allowPage": true, "sequence": 17,
        })
    }

    fn probe_after_requested_page_reload(preparation: &Value, storage_available: bool) -> Value {
        let prepare = build_prepare_with_page_receipt(preparation);
        let probe = build_probe_with_page_receipt(preparation["id"].as_str().unwrap());
        let script = format!(r#"
const assert = require("node:assert/strict");
const vm = require("node:vm");
const storageAvailable = {storage_available};
const entries = new Map();
const storage = {{
  getItem(key) {{ return entries.get(key) || null; }},
  setItem(key, value) {{ entries.set(key, value); }},
}};
const timers = [];
const context = vm.createContext({{
  document: {{ readyState: "complete", getElementById() {{ return null; }} }},
  console,
}});
function page(documentId, legacy) {{
  return {{
    __codexPlusUserScripts: {{ documentId, scripts: legacy ? {{ legacy: {{ status: "loaded" }} }} : {{}} }},
    __codexPlusUserScriptsBootstrap: true,
    sessionStorage: storageAvailable ? storage : null,
    setTimeout(callback) {{ timers.push(callback); }},
    location: {{ reload() {{
      assert.equal(entries.size, storageAvailable ? 1 : 0);
      context.window = page("new-document", false);
    }} }},
  }};
}}
context.window = page("old-document", true);
assert.equal(JSON.parse(vm.runInContext({prepare}, context)).phase, "pageRequested");
assert.equal(timers.length, 1);
timers.shift()();
const observed = JSON.parse(vm.runInContext({probe}, context));
assert.equal(observed.documentId, "new-document");
process.stdout.write(JSON.stringify(observed));
"#, prepare = json!(prepare), probe = json!(probe));
        let output = std::process::Command::new("node").args(["-e", &script]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice(&output.stdout).unwrap()
    }

    #[tokio::test]
    async fn acknowledged_page_request_survives_the_next_probe_being_a_new_document() {
        let owner = ReloadOwner::default();
        let preparation = page_preparation();
        let mut acknowledged = preparation.clone();
        acknowledged["phase"] = json!("pageRequested");
        acknowledged["mode"] = json!("page");
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let prepared = owner.wait_preparation_with_probe("reload", &preparation, Some(acknowledged.clone()), || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(json!({ "documentId": "new-document", "operation": null }))
        }).await;
        assert_eq!(prepared, Some(acknowledged));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn lost_prepare_ack_recovers_page_request_from_the_new_documents_receipt() {
        let owner = ReloadOwner::default();
        let preparation = page_preparation();
        let observed = probe_after_requested_page_reload(&preparation, true);
        assert!(observed["operation"].is_null());
        let prepared = owner.wait_preparation_with_probe("reload", &preparation, None, || {
            std::future::ready(observed.clone())
        }).await.unwrap();
        assert_eq!(prepared["phase"], "pageRequested");
        assert_eq!(prepared["mode"], "page");
        assert_eq!(prepared["reason"], "cleanup_missing");
    }

    #[tokio::test]
    async fn new_document_never_uses_a_receipt_from_another_attempt_or_policy() {
        let owner = ReloadOwner::default();
        let preparation = page_preparation();
        let observed = probe_after_requested_page_reload(&preparation, true);
        for (field, stale_value) in [
            ("id", json!("reload-attempt-0")),
            ("documentId", json!("earlier-document")),
            ("revision", json!("old-revision")),
            ("sequence", json!(16)),
            ("allowPage", json!(false)),
            ("phase", json!("stopped")),
        ] {
            let mut stale = observed.clone();
            stale["pageRequestReceipt"][field] = stale_value;
            assert!(owner.wait_preparation_with_probe("reload", &preparation, None, || {
                std::future::ready(stale.clone())
            }).await.is_none(), "stale receipt field: {field}");
        }
    }

    #[tokio::test]
    async fn unavailable_page_receipt_keeps_an_unacknowledged_new_document_as_navigation() {
        let owner = ReloadOwner::default();
        let preparation = page_preparation();
        let observed = probe_after_requested_page_reload(&preparation, false);
        assert!(observed.get("pageRequestReceipt").is_none());
        assert!(owner.wait_preparation_with_probe("reload", &preparation, None, || {
            std::future::ready(observed.clone())
        }).await.is_none());
    }

    #[tokio::test]
    async fn observation_timeout_does_not_cancel_the_operation() {
        let owner = ReloadOwner::default();
        owner.state.lock().unwrap().insert("test".into(), json!({ "active": true }));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let cloned = owner.clone();
        let task = tokio::spawn(async move {
            cloned.observe("test", Duration::from_millis(10), async {
                let _ = started_tx.send(());
                let _ = finish_rx.await;
            }).await;
        });
        started_rx.await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while owner.status("test").unwrap()["phase"] != "unknown" {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        assert_eq!(owner.status("test").unwrap()["active"], true);
        assert!(!task.is_finished());
        finish_tx.send(()).unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn bootstrap_can_run_without_allowing_another_reload_to_overtake() {
        let owner = ReloadOwner::default();
        let current = owner.operations.lock().await;
        let execution = owner.execution.lock().await;
        let next_owner = owner.clone();
        let (next_tx, mut next_rx) = tokio::sync::oneshot::channel();
        let next = tokio::spawn(async move {
            let _guard = next_owner.operations.lock().await;
            let _ = next_tx.send(());
        });
        let bootstrap_owner = owner.clone();
        let (bootstrap_tx, bootstrap_rx) = tokio::sync::oneshot::channel();
        let bootstrap = tokio::spawn(async move {
            let _guard = bootstrap_owner.lock_initial_load().await;
            let _ = bootstrap_tx.send(());
        });
        drop(execution);
        tokio::time::timeout(Duration::from_secs(1), bootstrap_rx).await.unwrap().unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut next_rx).await.is_err());
        drop(current);
        tokio::time::timeout(Duration::from_secs(1), next_rx).await.unwrap().unwrap();
        bootstrap.await.unwrap();
        next.await.unwrap();
    }

    #[test]
    fn inventory_verdict_requires_matching_terminal_states() {
        let runtime = json!({});
        assert_eq!(inventory_verdict(&json!({"scripts": []}), None).0, "degraded");
        assert_eq!(inventory_verdict(&json!({}), Some(&runtime)).0, "degraded");
        assert_eq!(inventory_verdict(&json!({"scripts": [
            {"configuredEnabled": true, "runtimeStatus": "not_loaded"}
        ]}), Some(&runtime)).0, "degraded");
        assert_eq!(inventory_verdict(&json!({"scripts": [
            {"configuredEnabled": true, "runtimeStatus": "loading"}
        ]}), Some(&runtime)).0, "degraded");
        assert_eq!(inventory_verdict(&json!({"scripts": [
            {"configuredEnabled": false, "runtimeStatus": "loaded"}
        ]}), Some(&runtime)).0, "degraded");
        assert_eq!(inventory_verdict(&json!({"scripts": [
            {"configuredEnabled": true, "runtimeStatus": "failed"}
        ]}), Some(&runtime)).0, "failed");
        assert_eq!(inventory_verdict(&json!({"scripts": [
            {"configuredEnabled": true, "runtimeStatus": "loaded"},
            {"configuredEnabled": false, "runtimeStatus": "not_loaded"}
        ]}), Some(&runtime)).0, "applied");
    }

    #[tokio::test]
    async fn duplicate_native_submission_and_retired_ids_never_execute_again() {
        let owner = ReloadOwner::default();
        let dir = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(dir.path().join("builtin"), dir.path().join("user"), dir.path().join("scripts.json"));
        let websocket = Arc::new(Mutex::new(None));
        let id = codex_plus_core::user_script_control::new_operation_id(owner.epoch());
        let payload = json!({ "operationId": id, "allowPage": false });
        let first = owner.submit(&payload, websocket.clone(), manager.clone()).unwrap();
        let duplicate = owner.submit(&payload, websocket.clone(), manager.clone()).unwrap();
        assert_eq!(first, duplicate);
        assert_eq!(owner.state.lock().unwrap().len(), 1);
        assert!(owner.submit(&json!({ "operationId": id, "allowPage": true }), websocket.clone(), manager.clone()).is_err());
        let issued_at = first["issuedAt"].as_u64().unwrap();
        owner.state.lock().unwrap().remove(&id);
        owner.retired_before.store(issued_at, Ordering::Relaxed);
        assert_eq!(owner.control_status(&id)["phase"], "rejected");
        assert!(owner.submit(&payload, websocket, manager).is_err());
    }

    #[tokio::test]
    async fn native_deadline_does_not_drop_an_accepted_owner_operation() {
        use codex_plus_core::user_script_control::{self as control, ControlAction, ControlIdentity};
        let dir = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(dir.path().join("builtin"), dir.path().join("user"), dir.path().join("scripts.json"));
        let owner = ReloadOwner::default();
        let identity = ControlIdentity::new(dir.path(), 9336, &manager).unwrap();
        let id = control::new_operation_id(owner.epoch());
        let delayed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let handler_owner = owner.clone();
        let websocket = Arc::new(Mutex::new(None));
        let handler = Arc::new(move |action| match action {
            ControlAction::Submit(payload) => {
                let operation = handler_owner.submit(&payload, websocket.clone(), manager.clone())?;
                if !delayed.swap(true, Ordering::SeqCst) {
                    // 真实 owner 已接收，但 ACK 晚于 native 通道的三秒截止时间。
                    std::thread::sleep(Duration::from_millis(3200));
                }
                Ok(json!({ "reloadOperation": operation }))
            }
            ControlAction::Status(id) | ControlAction::StatusWithVerification(id, _) =>
                Ok(json!({ "reloadOperation": handler_owner.control_status(&id) })),
        });
        let _server = control::start_owner(identity.clone(), owner.epoch().to_owned(), handler).unwrap();
        let submit = json!({ "action": "submit", "ownerEpoch": owner.epoch(), "operationId": id, "allowPage": false });
        assert!(control::request(&identity, submit.clone()).await.is_err());
        let status = control::request(&identity, json!({
            "action": "status", "ownerEpoch": owner.epoch(), "operationId": id,
        })).await.unwrap();
        assert_eq!(status["result"]["reloadOperation"]["operationId"], id);
        assert_eq!(status["result"]["reloadOperation"]["active"], true);
        assert_eq!(owner.state.lock().unwrap().len(), 1);
        control::request(&identity, submit).await.unwrap();
        assert_eq!(owner.state.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn failed_new_document_initialization_ends_navigation_and_releases_operation() {
        let dir = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(dir.path().join("builtin"), dir.path().join("user"), dir.path().join("scripts.json"));
        let owner = ReloadOwner::default();
        owner.state.lock().unwrap().insert("navigation".to_owned(), json!({ "active": true }));
        let task_owner = owner.clone();
        tokio::spawn(async move {
            let _operation = task_owner.operations.lock().await;
            assert!(task_owner.verify_navigation_with_probe("navigation", "old-document", "page", &manager, || {
                std::future::ready(json!({
                    "documentId": "new-document", "initialized": false, "ready": true,
                    "initialError": "skip_pending_other_doc",
                }))
            }).await);
        }).await.unwrap();
        assert_eq!(owner.status("navigation").unwrap()["phase"], "blocked");
        assert_eq!(owner.status("navigation").unwrap()["active"], false);
        let _guard = tokio::time::timeout(Duration::from_secs(1), owner.operations.lock()).await.unwrap();
    }

    #[tokio::test]
    async fn desired_revision_retries_do_not_accumulate_behind_a_held_config_lock() {
        let dir = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(dir.path().join("builtin"), dir.path().join("user"), dir.path().join("scripts.json"));
        manager.save_config(&Default::default()).unwrap();
        let lock = std::fs::OpenOptions::new().read(true).write(true)
            .open(dir.path().join("scripts.json.lock")).unwrap();
        lock.lock().unwrap();
        let owner = ReloadOwner::default();
        let operation = json!({ "operationId": "applied-operation", "phase": "applied" });
        let verification = "a".repeat(32);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (first_manager, first_calls) = (manager.clone(), calls.clone());
        assert_eq!(owner.desired_revision(&operation, Some(&verification), move || {
            first_calls.fetch_add(1, Ordering::SeqCst);
            let _ = started_tx.send(());
            first_manager.reload_snapshot().ok().map(|snapshot| snapshot.1)
        }), (None, true));
        started_rx.await.unwrap();
        for index in 0..64 {
            let verification = format!("{index:032x}");
            let calls = counted.clone();
            assert_eq!(owner.desired_revision(&operation, Some(&verification), move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Some("unexpected duplicate read".to_owned())
            }), (None, true));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        lock.unlock().unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let (revision, pending) = owner.desired_revision(&operation, Some(&verification), || {
                    panic!("同一验证编号必须复用已完成结果")
                });
                if !pending {
                    assert!(revision.is_some());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let calls = counted.clone();
        let next_verification = "b".repeat(32);
        assert_eq!(owner.desired_revision(&operation, Some(&next_verification), move || {
            calls.fetch_add(1, Ordering::SeqCst);
            Some("new intent reads a fresh snapshot".to_owned())
        }), (None, true));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let (revision, pending) = owner.desired_revision(&operation, Some(&next_verification), || unreachable!());
                if !pending {
                    assert_eq!(revision.as_deref(), Some("new intent reads a fresh snapshot"));
                    break;
                }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert_eq!(counted.load(Ordering::SeqCst), 2);
    }
}
