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
        if let Some(epoch) = payload.get("ownerEpoch") {
            anyhow::ensure!(epoch.as_str() == Some(&self.epoch), "重载 owner 已变化；没有提交旧操作");
        }
        let allow_page = payload.get("allowPage").and_then(Value::as_bool).unwrap_or(true);
        let source_document = match payload.get("sourceDocumentId") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_str().filter(|value| !value.is_empty() && value.len() <= 256)
                .context("重载来源文档标识无效")?.to_owned()),
        };
        let record = json!({ "operationId": id, "ownerEpoch": self.epoch, "allowPage": allow_page, "phase": "accepted", "active": true,
            "sourceDocumentId": source_document,
            "issuedAt": issued_at,
            "createdAt": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 });
        {
            let mut state = self.state.lock().unwrap();
            if let Some(previous) = state.get(&id) {
                anyhow::ensure!(previous["allowPage"] == allow_page
                    && previous["sourceDocumentId"] == record["sourceDocumentId"],
                    "重载操作编号与已有请求冲突");
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
            owner.observe(&id, Duration::from_secs(45), owner.run(&id, allow_page, source_document.as_deref(), &websocket, &manager)).await;
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

    #[cfg(test)]
    pub(super) fn finish_for_test(&self, id: &str, phase: &str, fields: Value) {
        self.update(id, phase, fields);
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
        if record["active"] == false { return; }
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

    async fn run(&self, id: &str, allow_page: bool, source_document: Option<&str>, websocket: &Arc<Mutex<Option<String>>>, manager: &UserScriptManager) {
        // Keep other reloads out even while a new document's bootstrap is allowed to run.
        let _operation_guard = self.operations.lock().await;
        // 同一操作不能在目标切换后静默转交给另一个 CDP target。
        let target = loop {
            let current = websocket.lock().unwrap().clone();
            if let Some(url) = current {
                break url;
            }
            self.update(id, "unknown", json!({ "reason": "页面尚未连接，操作结果待确认" }));
            tokio::time::sleep(Duration::from_millis(500)).await;
        };
        let pinned = Arc::new(Mutex::new(Some(target.clone())));
        self.run_until_target_changes(id, websocket, &target,
            self.run_on_target(id, allow_page, source_document, websocket, &target, &pinned, manager)).await;
    }

    async fn run_until_target_changes<F>(
        &self,
        id: &str,
        current: &Arc<Mutex<Option<String>>>,
        target: &str,
        operation: F,
    )
    where
        F: std::future::Future<Output = ()>,
    {
        tokio::pin!(operation);
        tokio::select! {
            _ = &mut operation => {}
            _ = wait_for_target_change(current, target) => {
                self.update(id, "unknown", json!({
                    "reason": "页面目标已切换；原操作仍在等待旧页面确认",
                    "active": true,
                }));
                operation.await;
            }
        }
    }

    async fn run_on_target(&self, id: &str, allow_page: bool, source_document: Option<&str>, current: &Arc<Mutex<Option<String>>>, target: &str, websocket: &Arc<Mutex<Option<String>>>, manager: &UserScriptManager) {
        self.run_on_target_with(id, allow_page, source_document, current, target, websocket, manager,
            |websocket, expression| {
                let websocket = websocket.clone();
                let expression = expression.to_owned();
                async move { evaluate(&websocket, &expression).await }
            }).await;
    }

    async fn run_on_target_with<F, Fut>(&self, id: &str, allow_page: bool, source_document: Option<&str>, current: &Arc<Mutex<Option<String>>>, target: &str, websocket: &Arc<Mutex<Option<String>>>, manager: &UserScriptManager, evaluate_page: F)
    where
        F: Fn(&Arc<Mutex<Option<String>>>, &str) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<Value>>,
    {
        let mut attempt = 0_u64;
        let mut prepare_sent = false;
        let mut mutation_confirmed = false;
        let mut applied_once = false;
        let mut stopped_source: Option<(String, u64)> = None;
        loop {
            attempt += 1;
            let operation_id = format!("{id}-{attempt:x}");
            let sequence = next_sequence();
            let (bundle, revision) = match snapshot(manager).await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    self.update(id, if stopped_source.is_some() || applied_once { "degraded" } else { "failed" },
                        json!({ "reason": format!("{error:#}") }));
                    return;
                }
            };
            self.update(id, "accepted", json!({ "revision": revision, "attempt": attempt }));
            // 探测本身不执行清理。旧目标不可达时不能一直占用初始化锁。
            loop {
                if target_changed(current, target) {
                    if self.finish_on_target_change(id, prepare_sent, mutation_confirmed) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                if evaluate_page(websocket, &build_probe_with_page_receipt(&operation_id)).await
                    .is_ok_and(|value| value["documentId"].is_string()) { break; }
                self.update(id, "unknown", json!({ "reason": "页面暂未响应，等待确认" }));
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let guard = self.execution.lock().await;
            if target_changed(current, target) {
                drop(guard);
                if self.finish_on_target_change(id, prepare_sent, mutation_confirmed) {
                    return;
                }
                continue;
            }
            // 获取锁后重新探测，防止等待锁期间 bootstrap 或导航改变文档。
            let before = match evaluate_page(websocket, &build_probe_with_page_receipt(&operation_id)).await {
                Ok(value) if value["documentId"].is_string() => value,
                _ => {
                    drop(guard);
                    self.update(id, "unknown", json!({ "reason": "页面暂未响应，等待确认" }));
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
            };
            if target_changed(current, target) {
                drop(guard);
                if self.finish_on_target_change(id, prepare_sent, mutation_confirmed) {
                    return;
                }
                continue;
            }
            let document = before["documentId"].as_str().unwrap_or_default().to_string();
            if document.is_empty() { continue; }
            if !prepare_sent && source_document.is_some_and(|source| source != document) {
                self.update(id, "rejected", json!({ "reason": "来源页面已切换；未把旧请求应用到新页面" }));
                return;
            }
            if stopped_source.as_ref().is_some_and(|(stopped_document, _)| stopped_document != &document) {
                drop(guard);
                if self.verify_navigation(id, websocket, &operation_id,
                    &stopped_source.as_ref().unwrap().0, "navigation", manager).await {
                    return;
                }
                stopped_source = None;
                continue;
            }
            let mut generation = codex_plus_core::extension_backend::backend_generation(manager);
            let source_revoked = stopped_source.is_some() || allow_page || before["cleanupSupported"] == true;
            if let Some((_, stopped_generation)) = stopped_source.as_ref() {
                if before["initialized"] != true || !before["backendSource"].is_null()
                    || generation != *stopped_generation
                {
                    self.update(id, "degraded", json!({ "reason": "页面清理后文档或后台代际已变化，未应用新快照" }));
                    return;
                }
                generation = *stopped_generation;
            } else if source_revoked {
                if before["initialized"] != true || !before["backendSource"].is_object() {
                    drop(guard);
                    self.update(id, "unknown", json!({ "reason": "等待页面初始化及后台来源确认" }));
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                match revoke_probed_backend_source(manager, &document, &before).await {
                    Some(next) => generation = next,
                    None => {
                        self.update(id, "degraded", json!({ "reason": "页面文档或后台来源已变化，未开始页面清理" }));
                        return;
                    }
                }
            }
            if target_changed(current, target) {
                if source_revoked && (!prepare_sent || mutation_confirmed) {
                    self.update(id, "degraded", json!({ "reason": "页面目标已切换，旧后台来源已撤销；请检查或重新加载页面" }));
                    return;
                }
                drop(guard);
                if self.finish_on_target_change(id, prepare_sent, mutation_confirmed) {
                    return;
                }
                continue;
            }
            self.update(id, "quiescing", json!({ "documentId": document }));
            let preparation = json!({
                "id": operation_id, "documentId": document, "revision": revision,
                "allowPage": allow_page, "sequence": sequence,
            });
            let prepare = build_prepare_with_page_receipt(&preparation);
            // A timeout does not cancel CDP execution. Never resend this prepare.
            prepare_sent = true;
            mutation_confirmed = false;
            let acknowledged = evaluate_page(&websocket, &prepare).await.ok();
            if let Some(response) = &acknowledged {
                if response["phase"] == "rejected" || response["phase"] == "failed" {
                    self.update(id, if source_revoked { "degraded" } else { "failed" },
                        json!({ "reason": if source_revoked { "页面准备未完成，旧后台来源已撤销；请检查当前页面" } else { response["reason"].as_str().unwrap_or("页面准备失败") } }));
                    return;
                }
            }
            let prepared = self.wait_preparation_with_probe(id, &preparation, acknowledged, || {
                self.wait_probe_with(id, &websocket, &operation_id, &evaluate_page)
            }).await;
            let Some(prepared) = prepared else {
                // Navigation can destroy the prepare response before it is read.
                // Let the new document's bootstrap acquire the same execution permit.
                drop(guard);
                if self.verify_navigation(id, &websocket, &operation_id, &document, "navigation", manager).await {
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
                    if self.verify_navigation(id, &websocket, &operation_id, &document, "page", manager).await {
                        return;
                    }
                    continue;
                }
                "stopped" => mutation_confirmed = true,
                _ => {
                    self.update(id, if source_revoked { "degraded" } else { "failed" },
                        json!({ "reason": "无法确认页面清理状态；请检查当前页面" }));
                    return;
                }
            }
            self.update(id, "stopped", json!({}));
            if target_changed(current, target) {
                self.finish_on_target_change(id, prepare_sent, mutation_confirmed);
                return;
            }
            match snapshot(manager).await {
                Ok((_, current_revision)) if current_revision == revision => {}
                Ok(_) => {
                    stopped_source = Some((document, generation));
                    continue;
                }
                Err(error) => {
                    self.update(id, "degraded", json!({ "reason": format!("页面已清理，但无法重新读取拓展快照；请修复后重新加载页面：{error:#}") }));
                    return;
                }
            }
            if target_changed(current, target) {
                self.finish_on_target_change(id, prepare_sent, mutation_confirmed);
                return;
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
                    self.update(id, "degraded", json!({ "reason": format!("页面已清理，无法固定后台快照；请修复后重新加载页面：{error:#}") }));
                    return;
                }
            };
            if target_changed(current, target) {
                self.finish_on_target_change(id, prepare_sent, mutation_confirmed);
                return;
            }
            let backend_source = match codex_plus_core::extension_backend::bind_backend_snapshot(
                manager, generation.wrapping_add(1), &document, staged,
            ).await {
                Ok(source) => source,
                Err(error) => {
                    self.update(id, "degraded", json!({ "reason": format!("页面已清理，无法绑定页面后台代际；请重新加载页面：{error:#}") }));
                    return;
                }
            };
            if target_changed(current, target) {
                self.finish_on_target_change(id, prepare_sent, mutation_confirmed);
                return;
            }
            stopped_source = None;
            self.update(id, "applying", json!({}));
            let apply = UserScriptManager::build_apply_operation(&operation_id, &document, &revision, &bundle, &backend_source);
            // As with prepare, a lost response is resolved by querying the same operation.
            mutation_confirmed = false;
            if let Ok(response) = evaluate_page(&websocket, &apply).await {
                if response["operation"]["phase"] == "rejected" || response["operation"]["phase"] == "failed" {
                    self.update(id, "failed", json!({ "reason": response["operation"]["reason"] }));
                    return;
                }
            }
            loop {
                let probe = self.wait_probe_with(id, &websocket, &operation_id, &evaluate_page).await;
                if probe["documentId"] != document { break; }
                if !preparation_identity_matches(&probe["operation"], &preparation) {
                    self.update(id, "unknown", json!({ "reason": "页面应用回执身份不匹配，操作结果待确认" }));
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                let phase = probe["operation"]["phase"].as_str().unwrap_or_default();
                if phase == "failed" || phase == "rejected" || phase == "superseded" {
                    self.update(id, "failed", json!({ "reason": probe["operation"]["reason"] }));
                    return;
                }
                if phase == "verifying" && probe["revision"] == revision && probe["ready"] == true {
                    applied_once = true;
                    mutation_confirmed = true;
                    if !codex_plus_core::extension_backend::backend_source_is_current(manager, &probe["backendSource"]) {
                        self.update(id, "degraded", json!({ "reason": "页面脚本已执行，但后台来源已失效" }));
                        return;
                    }
                    if target_changed(current, target) {
                        self.finish_on_target_change(id, prepare_sent, mutation_confirmed);
                        return;
                    }
                    match snapshot(manager).await {
                        Ok((_, current_revision)) if current_revision == revision => {}
                        Ok(_) => break,
                        Err(error) => {
                            self.update(id, "degraded", json!({
                                "reason": format!("页面脚本已执行，但无法确认当前拓展快照：{error:#}")
                            }));
                            return;
                        }
                    }
                    self.finish(id, "scripts", probe, manager).await;
                    return;
                }
                self.update(id, "unknown", json!({ "reason": "等待页面确认应用操作" }));
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }

    fn finish_on_target_change(&self, id: &str, prepare_sent: bool, mutation_confirmed: bool) -> bool {
        if !prepare_sent {
            self.update(id, "rejected", json!({ "reason": "页面目标已切换，未开始页面清理" }));
            return true;
        }
        if mutation_confirmed {
            self.update(id, "degraded", json!({ "reason": "页面目标已切换；此前操作已确认，但新页面尚未验证，请检查或重新加载页面" }));
            return true;
        }
        self.update(id, "unknown", json!({ "reason": "页面目标已切换，原操作仍待确认" }));
        false
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
            let operation = &observed["operation"];
            if preparation_identity_matches(operation, preparation) {
                let complete = match operation["phase"].as_str().unwrap_or_default() {
                    "stopped" => operation["mode"] == "scripts",
                    "pageRequested" => page_receipt_matches(operation, preparation),
                    "blocked" | "degraded" | "failed" | "rejected" => true,
                    _ => false,
                };
                if complete {
                    return Some(operation.clone());
                }
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
        self.wait_probe_with(id, websocket, operation_id, &|websocket, expression| {
            let websocket = websocket.clone();
            let expression = expression.to_owned();
            async move { evaluate(&websocket, &expression).await }
        }).await
    }

    async fn wait_probe_with<F, Fut>(&self, id: &str, websocket: &Arc<Mutex<Option<String>>>, operation_id: &str, evaluate_page: &F) -> Value
    where
        F: Fn(&Arc<Mutex<Option<String>>>, &str) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<Value>>,
    {
        loop {
            if let Ok(value) = evaluate_page(websocket, &build_probe_with_page_receipt(operation_id)).await {
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

async fn wait_for_target_change(current: &Arc<Mutex<Option<String>>>, target: &str) {
    loop {
        if target_changed(current, target) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn target_changed(current: &Arc<Mutex<Option<String>>>, target: &str) -> bool {
    current.lock().unwrap().as_deref().is_some_and(|url| url != target)
}

async fn revoke_probed_backend_source(manager: &UserScriptManager, document: &str, probe: &Value) -> Option<u64> {
    if probe.pointer("/backendSource/documentId").and_then(Value::as_str) != Some(document) {
        return None;
    }
    codex_plus_core::extension_backend::revoke_backend_source(manager, &probe["backendSource"]).await
}

fn preparation_identity_matches(receipt: &Value, preparation: &Value) -> bool {
    ["id", "documentId", "revision", "allowPage", "sequence"].iter()
        .all(|key| receipt.get(*key).is_some_and(|value| preparation.get(*key) == Some(value)))
}

fn page_receipt_matches(receipt: &Value, preparation: &Value) -> bool {
    receipt["phase"] == "pageRequested" && receipt["mode"] == "page" && receipt["allowPage"] == true
        && preparation_identity_matches(receipt, preparation)
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

    fn fixture_preparation(expression: &str) -> Value {
        let encoded = expression.lines()
            .find_map(|line| line.trim().strip_prefix("const expected = "))
            .and_then(|line| line.strip_suffix(';'))
            .expect("准备操作缺少预期身份");
        let mut operation: Value = serde_json::from_str(encoded).unwrap();
        operation["phase"] = json!("stopped");
        operation["mode"] = json!("scripts");
        operation
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
    async fn changed_snapshot_after_cleanup_is_prepared_again_on_the_same_document() {
        let dir = tempfile::tempdir().unwrap();
        let result = exercise_changed_snapshot(dir.path(), "change").await;
        assert_eq!(result.0["phase"], "applied");
        assert_eq!(result.1, 2);
        assert_eq!(result.2, 1);
    }

    #[tokio::test]
    async fn snapshot_error_after_cleanup_reports_degraded_without_reapplying() {
        let dir = tempfile::tempdir().unwrap();
        let result = exercise_changed_snapshot(dir.path(), "invalid").await;
        assert_eq!(result.0["phase"], "degraded");
        assert_eq!(result.1, 1);
        assert_eq!(result.2, 0);
    }

    #[tokio::test]
    async fn snapshot_error_after_apply_reports_the_completed_application() {
        let dir = tempfile::tempdir().unwrap();
        let result = exercise_changed_snapshot(dir.path(), "invalid_after_apply").await;
        assert_eq!(result.0["phase"], "degraded");
        assert!(result.0["reason"].as_str().unwrap().contains("页面脚本已执行"));
        assert_eq!(result.1, 1);
        assert_eq!(result.2, 1);
    }

    #[tokio::test]
    async fn lost_cleanup_response_is_confirmed_without_repeating_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let result = exercise_changed_snapshot(dir.path(), "lost_prepare").await;
        assert_eq!(result.0["phase"], "applied");
        assert_eq!(result.1, 1);
        assert_eq!(result.2, 1);
    }

    #[tokio::test]
    async fn timed_out_cleanup_can_finish_later_without_being_resent() {
        let dir = tempfile::tempdir().unwrap();
        let result = exercise_changed_snapshot(dir.path(), "deferred_prepare").await;
        assert_eq!(result.0["phase"], "applied");
        assert_eq!(result.1, 1);
        assert_eq!(result.2, 1);
    }

    #[tokio::test]
    async fn lost_apply_response_is_confirmed_without_repeating_application() {
        let dir = tempfile::tempdir().unwrap();
        let result = exercise_changed_snapshot(dir.path(), "lost_apply").await;
        assert_eq!(result.0["phase"], "applied");
        assert_eq!(result.1, 1);
        assert_eq!(result.2, 1);
    }

    #[tokio::test]
    async fn target_change_after_confirmed_apply_does_not_block_later_operations() {
        let dir = tempfile::tempdir().unwrap();
        let result = exercise_changed_snapshot(dir.path(), "target_after_apply").await;
        assert_eq!(result.0["phase"], "degraded");
        assert_eq!(result.0["active"], false);
        assert_eq!(result.1, 1);
        assert_eq!(result.2, 1);
    }

    #[tokio::test]
    async fn target_change_after_confirmed_cleanup_does_not_block_later_operations() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user");
        std::fs::create_dir(&user).unwrap();
        let script = user.join("fixture.js");
        std::fs::write(&script, "window.fixtureVersion = 'old';").unwrap();
        let manager = UserScriptManager::new(dir.path().join("builtin"), &user, dir.path().join("scripts.json"));
        let owner = ReloadOwner::default();
        let generation = codex_plus_core::extension_backend::backend_generation(&manager);
        let source = codex_plus_core::extension_backend::bind_backend_document(&manager, generation, "document").await.unwrap();
        let id = codex_plus_core::user_script_control::new_operation_id(owner.epoch());
        owner.state.lock().unwrap().insert(id.clone(), json!({ "active": true }));
        let current = Arc::new(Mutex::new(Some("old-target".to_string())));
        let pinned = Arc::new(Mutex::new(Some("old-target".to_string())));
        let changed_target = current.clone();
        let cleanups = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let applications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let operation = Arc::new(Mutex::new(None::<Value>));
        let cleanup_count = cleanups.clone();
        let application_count = applications.clone();
        let task = async {
            let _operation_guard = owner.operations.lock().await;
            owner.run_on_target_with(
                &id, true, Some("document"), &current, "old-target", &pinned, &manager,
                move |_, expression| {
                    let changed_target = changed_target.clone();
                    let cleanups = cleanup_count.clone();
                    let applications = application_count.clone();
                    let source = source.clone();
                    let script = script.clone();
                    let operation = operation.clone();
                    let expression = expression.to_owned();
                    async move {
                        if expression.contains("const encoded =") {
                            cleanups.fetch_add(1, Ordering::SeqCst);
                            std::fs::write(&script, "window.fixtureVersion = 'new';").unwrap();
                            let prepared = fixture_preparation(&expression);
                            *operation.lock().unwrap() = Some(prepared.clone());
                            Ok(prepared)
                        } else if expression.contains("const operation = runtime.applyOperation") {
                            applications.fetch_add(1, Ordering::SeqCst);
                            Ok(json!({ "operation": { "phase": "verifying" } }))
                        } else {
                            let stopped = cleanups.load(Ordering::SeqCst) > 0;
                            if stopped {
                                *changed_target.lock().unwrap() = Some("new-target".to_string());
                            }
                            Ok(json!({ "documentId": "document", "initialized": true, "ready": true,
                                "cleanupSupported": true, "backendSource": if stopped { Value::Null } else { source },
                                "operation": operation.lock().unwrap().clone() }))
                        }
                    }
                },
            ).await;
        };
        let completed = tokio::time::timeout(Duration::from_secs(2), task).await;
        let status = owner.status(&id).unwrap();
        let next_can_start = tokio::time::timeout(Duration::from_secs(1), owner.operations.lock()).await.is_ok();
        codex_plus_core::extension_backend::stop_all(&manager).await;
        assert!(completed.is_ok(), "已确认清理后的目标切换不能无限占用重载队列");
        assert_eq!(status["phase"], "degraded");
        assert_eq!(status["active"], false);
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
        assert_eq!(applications.load(Ordering::SeqCst), 0);
        assert!(next_can_start);
    }

    async fn exercise_changed_snapshot(root: &std::path::Path, after_prepare: &str) -> (Value, usize, usize) {
        let user = root.join("user");
        std::fs::create_dir(&user).unwrap();
        let script = user.join("fixture.js");
        std::fs::write(&script, "window.fixtureVersion = 'old';").unwrap();
        let config = root.join("scripts.json");
        let manager = UserScriptManager::new(root.join("builtin"), &user, &config);
        let owner = ReloadOwner::default();
        let generation = codex_plus_core::extension_backend::backend_generation(&manager);
        let source = codex_plus_core::extension_backend::bind_backend_document(&manager, generation, "document").await.unwrap();
        let id = codex_plus_core::user_script_control::new_operation_id(owner.epoch());
        owner.state.lock().unwrap().insert(id.clone(), json!({ "active": true }));
        let websocket = Arc::new(Mutex::new(Some("mock-target".to_string())));
        let old_evaluations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let new_evaluations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let old_cleanup = old_evaluations.clone();
        let new_apply = new_evaluations.clone();
        let probe_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let probes = probe_count.clone();
        let first_source = source.clone();
        let evaluator_manager = manager.clone();
        let operation = Arc::new(Mutex::new(None::<Value>));
        let invalid = after_prepare == "invalid";
        let invalid_after_apply = after_prepare == "invalid_after_apply";
        let change_after_apply = after_prepare == "target_after_apply";
        let lost_prepare = after_prepare == "lost_prepare";
        let deferred_prepare = after_prepare == "deferred_prepare";
        let lost_apply = after_prepare == "lost_apply";
        let current_target = websocket.clone();
        let task = async {
            let _operation_guard = owner.operations.lock().await;
            owner.run_on_target_with(
                &id, true, Some("document"), &websocket, "mock-target", &websocket, &manager,
                move |_, expression| {
                    let old_cleanup = old_cleanup.clone();
                    let new_apply = new_apply.clone();
                    let probes = probes.clone();
                    let first_source = first_source.clone();
                    let script = script.clone();
                    let config = config.clone();
                    let manager = evaluator_manager.clone();
                    let operation = operation.clone();
                    let current_target = current_target.clone();
                    let expression = expression.to_owned();
                    async move {
                        if expression.contains("const encoded =") {
                            if old_cleanup.fetch_add(1, Ordering::SeqCst) == 0 {
                                if invalid {
                                    std::fs::write(&config, "{invalid").unwrap();
                                } else if !invalid_after_apply && !change_after_apply && !lost_prepare && !deferred_prepare && !lost_apply {
                                    std::fs::write(&script, "window.fixtureVersion = 'new';").unwrap();
                                }
                            }
                            let prepared = fixture_preparation(&expression);
                            if deferred_prepare {
                                let pending = operation.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(Duration::from_millis(125)).await;
                                    *pending.lock().unwrap() = Some(prepared);
                                });
                                anyhow::bail!("模拟 CDP 超时后页面才完成清理");
                            }
                            *operation.lock().unwrap() = Some(prepared.clone());
                            if lost_prepare {
                                anyhow::bail!("模拟清理已执行但 CDP 响应超时");
                            }
                            Ok(prepared)
                        } else if expression.contains("const operation = runtime.applyOperation") {
                            assert!(expression.contains(if invalid_after_apply || change_after_apply || lost_prepare || deferred_prepare || lost_apply {
                                "window.fixtureVersion = 'old';"
                            } else {
                                "window.fixtureVersion = 'new';"
                            }));
                            new_apply.fetch_add(1, Ordering::SeqCst);
                            let mut applied = operation.lock().unwrap();
                            applied.as_mut().unwrap()["phase"] = json!("verifying");
                            if change_after_apply {
                                std::fs::write(&script, "window.fixtureVersion = 'new';").unwrap();
                            }
                            if invalid_after_apply {
                                std::fs::write(&config, "{invalid").unwrap();
                            }
                            if lost_apply {
                                anyhow::bail!("模拟应用已执行但 CDP 响应超时");
                            }
                            Ok(json!({ "operation": applied.clone() }))
                        } else {
                            let index = probes.fetch_add(1, Ordering::SeqCst);
                            let generation = codex_plus_core::extension_backend::backend_generation(&manager);
                            let operation = operation.lock().unwrap().clone();
                            if change_after_apply && new_apply.load(Ordering::SeqCst) > 0 {
                                *current_target.lock().unwrap() = Some("new-target".to_string());
                            }
                            let backend_source = if index < 2 {
                                first_source
                            } else if new_apply.load(Ordering::SeqCst) > 0 {
                                json!({ "ownerEpoch": first_source["ownerEpoch"], "generation": generation,
                                    "documentId": "document", "revision": operation.as_ref().unwrap()["revision"] })
                            } else {
                                Value::Null
                            };
                            let revision = if new_apply.load(Ordering::SeqCst) > 0 {
                                operation.as_ref().map(|value| value["revision"].clone())
                            } else {
                                manager.reload_snapshot().ok().map(|snapshot| json!(snapshot.1))
                            };
                            Ok(json!({ "documentId": "document", "initialized": true, "ready": true,
                                "cleanupSupported": true, "backendSource": backend_source,
                                "scripts": { "user:fixture.js": { "status": "loaded", "name": "fixture.js", "source": "user" } },
                                "revision": revision,
                                "operation": operation }))
                        }
                    }
                },
            ).await;
        };
        let completed = tokio::time::timeout(Duration::from_secs(3), task).await;
        let status = owner.status(&id).unwrap();
        let prepares = old_evaluations.load(Ordering::SeqCst);
        let applies = new_evaluations.load(Ordering::SeqCst);
        codex_plus_core::extension_backend::stop_all(&manager).await;
        completed.expect("版本变化后重载未完成");
        (status, prepares, applies)
    }

    #[tokio::test]
    async fn same_document_waits_for_a_matching_complete_preparation_receipt() {
        let owner = ReloadOwner::default();
        let preparation = page_preparation();
        let mut matching = preparation.clone();
        matching["phase"] = json!("stopped");
        matching["mode"] = json!("scripts");

        for (field, value) in [
            ("id", json!("other-attempt")),
            ("documentId", json!("other-document")),
            ("revision", json!("other-revision")),
            ("allowPage", json!(false)),
            ("sequence", json!(18)),
            ("mode", json!("page")),
        ] {
            let mut stale = matching.clone();
            stale[field] = value;
            let observed = json!({ "documentId": "old-document", "operation": stale });
            let result = tokio::time::timeout(Duration::from_millis(40),
                owner.wait_preparation_with_probe("reload", &preparation, None, || {
                    std::future::ready(observed.clone())
                })).await;
            assert!(result.is_err(), "不匹配的清理回执不能确认: {field}");
        }

        let mut incomplete = matching.clone();
        incomplete.as_object_mut().unwrap().remove("sequence");
        let observed = json!({ "documentId": "old-document", "operation": incomplete });
        assert!(tokio::time::timeout(Duration::from_millis(40),
            owner.wait_preparation_with_probe("reload", &preparation, None, || {
                std::future::ready(observed.clone())
            })).await.is_err(), "缺失字段的清理回执不能确认");

        let mut denied = preparation.clone();
        denied["allowPage"] = json!(false);
        let mut page_requested = denied.clone();
        page_requested["phase"] = json!("pageRequested");
        page_requested["mode"] = json!("page");
        let observed = json!({ "documentId": "old-document", "operation": page_requested });
        assert!(tokio::time::timeout(Duration::from_millis(40),
            owner.wait_preparation_with_probe("reload", &denied, None, || {
                std::future::ready(observed.clone())
            })).await.is_err(), "禁止刷新时不能接受页面刷新回执");

        let observed = json!({ "documentId": "old-document", "operation": matching.clone() });
        let prepared = owner.wait_preparation_with_probe("reload", &preparation, None, || {
            std::future::ready(observed.clone())
        }).await;
        assert_eq!(prepared, Some(matching));
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

    #[tokio::test]
    async fn changed_target_does_not_cancel_old_coordination_or_release_execution() {
        let owner = ReloadOwner::default();
        owner.state.lock().unwrap().insert("target-change".into(), json!({ "active": true }));
        let current = Arc::new(Mutex::new(Some("old-target".to_owned())));
        let task_owner = owner.clone();
        let task_current = current.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            task_owner.run_until_target_changes("target-change", &task_current, "old-target", async {
                let _guard = task_owner.lock_initial_load().await;
                let _ = started_tx.send(());
                let _ = finish_rx.await;
                task_owner.update("target-change", "applied", json!({}));
            }).await;
        });
        started_rx.await.unwrap();
        *current.lock().unwrap() = Some("new-target".to_owned());
        tokio::time::timeout(Duration::from_secs(2), async {
            while owner.status("target-change").unwrap()["phase"] != "unknown" {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        let status = owner.status("target-change").unwrap();
        assert_eq!(status["active"], true);
        assert!(!task.is_finished());
        assert!(tokio::time::timeout(Duration::from_millis(30), owner.lock_initial_load()).await.is_err());
        finish_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), task).await.unwrap().unwrap();
        assert_eq!(owner.status("target-change").unwrap()["phase"], "applied");
        let _guard = tokio::time::timeout(Duration::from_secs(1), owner.lock_initial_load()).await.unwrap();
    }

    #[tokio::test]
    async fn target_change_before_prepare_rejects_without_blocking_bootstrap() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user");
        std::fs::create_dir(&user).unwrap();
        let manager = UserScriptManager::new(dir.path().join("builtin"), user, dir.path().join("scripts.json"));
        let owner = ReloadOwner::default();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let old_target = format!("ws://127.0.0.1:{}/devtools/page/old", listener.local_addr().unwrap().port());
        let (connected_tx, connected_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let _ = connected_tx.send(());
            let _ = release_rx.await;
            drop(socket);
        });
        let current = Arc::new(Mutex::new(Some(old_target.clone())));
        let pinned = Arc::new(Mutex::new(Some(old_target.clone())));
        let id = codex_plus_core::user_script_control::new_operation_id(owner.epoch());
        owner.state.lock().unwrap().insert(id.clone(), json!({ "active": true }));
        let task_owner = owner.clone();
        let task_current = current.clone();
        let task_id = id.clone();
        let task = tokio::spawn(async move {
            task_owner.run_on_target(&task_id, true, Some("old-document"), &task_current, &old_target, &pinned, &manager).await;
        });
        tokio::time::timeout(Duration::from_secs(2), connected_rx).await.unwrap().unwrap();
        let guard = tokio::time::timeout(Duration::from_secs(1), owner.lock_initial_load()).await.unwrap();
        *current.lock().unwrap() = Some("ws://127.0.0.1:2/devtools/page/new".to_owned());
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while owner.status(&id).unwrap()["phase"] != "rejected" {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        assert_eq!(owner.status(&id).unwrap()["active"], false);
        task.await.unwrap();
        server.await.unwrap();
        drop(guard);
    }

    #[test]
    fn terminal_operation_cannot_be_made_active_again_by_a_late_observer() {
        let owner = ReloadOwner::default();
        owner.state.lock().unwrap().insert("finished".into(), json!({ "active": true }));
        owner.update("finished", "applied", json!({}));
        owner.update("finished", "unknown", json!({ "active": true }));
        let status = owner.status("finished").unwrap();
        assert_eq!(status["phase"], "applied");
        assert_eq!(status["active"], false);
    }

    #[tokio::test]
    async fn mismatched_probe_document_cannot_revoke_current_backend_source() {
        let dir = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(dir.path().join("builtin"), dir.path().join("user"), dir.path().join("scripts.json"));
        let generation = codex_plus_core::extension_backend::backend_generation(&manager);
        let source = codex_plus_core::extension_backend::bind_backend_document(&manager, generation, "new-document").await.unwrap();
        let probe = json!({ "backendSource": source });
        assert_eq!(revoke_probed_backend_source(&manager, "old-document", &probe).await, None);
        assert_eq!(codex_plus_core::extension_backend::backend_generation(&manager), generation);
        assert!(codex_plus_core::extension_backend::backend_source_is_current(&manager, &probe["backendSource"]));
        assert_eq!(revoke_probed_backend_source(&manager, "new-document", &probe).await, Some(generation.wrapping_add(1)));
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
        assert!(owner.submit(&json!({ "operationId": id, "allowPage": false, "sourceDocumentId": "other-document" }), websocket.clone(), manager.clone()).is_err());
        assert!(owner.submit(&json!({ "operationId": id, "allowPage": false, "ownerEpoch": "old-owner" }), websocket.clone(), manager.clone()).is_err());
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
