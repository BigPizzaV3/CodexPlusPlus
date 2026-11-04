use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use codex_plus_core::models::{DeleteResult, ExportResult, SessionRef};
use codex_plus_core::routes::{
    BridgeContext, BridgeDataService, BridgeRuntimeService, BridgeSettingsService,
    handle_bridge_request,
};
use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::{Value, json};

struct MockSettings(Option<bool>);

#[async_trait]
impl BridgeSettingsService for MockSettings {
    async fn get_settings(&self) -> anyhow::Result<BackendSettings> {
        let enabled = self
            .0
            .ok_or_else(|| anyhow::anyhow!("synthetic settings failure"))?;
        Ok(BackendSettings {
            usage_history_enabled: enabled,
            ..BackendSettings::default()
        })
    }
    async fn set_settings(&self, _: Value) -> anyhow::Result<BackendSettings> {
        panic!("read-only route must not write settings")
    }
}

struct UnusedRuntime;

#[async_trait]
impl BridgeRuntimeService for UnusedRuntime {
    async fn user_script_inventory(&self) -> anyhow::Result<Value> {
        panic!("runtime called")
    }
    async fn set_user_scripts_enabled(&self, _: bool) -> anyhow::Result<Value> {
        panic!("runtime called")
    }
    async fn set_user_script_enabled(&self, _: String, _: bool) -> anyhow::Result<Value> {
        panic!("runtime called")
    }
    async fn delete_user_script(&self, _: String) -> anyhow::Result<Value> {
        panic!("runtime called")
    }
    async fn reload_user_scripts(&self) -> anyhow::Result<Value> {
        panic!("runtime called")
    }
    async fn open_devtools(&self) -> anyhow::Result<Value> {
        panic!("runtime called")
    }
    async fn open_manager(&self, _: Value) -> anyhow::Result<Value> {
        panic!("runtime called")
    }
    async fn backend_status(&self) -> anyhow::Result<Value> {
        panic!("runtime called")
    }
    async fn codex_model_catalog(&self) -> anyhow::Result<Value> {
        panic!("runtime called")
    }
    async fn ads(&self) -> anyhow::Result<Value> {
        panic!("runtime called")
    }
}

#[derive(Default)]
struct MockData {
    queries: Mutex<Vec<Value>>,
    clears: AtomicUsize,
}

#[async_trait]
impl BridgeDataService for MockData {
    async fn usage_history(&self, payload: Value) -> anyhow::Result<Value> {
        self.queries.lock().unwrap().push(payload.clone());
        Ok(json!({"status":"ok","complete":true,"echo":payload}))
    }
    async fn clear_usage_history(&self) {
        self.clears.fetch_add(1, Ordering::SeqCst);
    }
    async fn delete(&self, _: SessionRef) -> anyhow::Result<DeleteResult> {
        panic!("unexpected data route")
    }
    async fn undo(&self, _: String) -> anyhow::Result<DeleteResult> {
        panic!("unexpected data route")
    }
    async fn export_markdown(&self, _: SessionRef) -> anyhow::Result<ExportResult> {
        panic!("unexpected data route")
    }
    async fn thread_usage_history(&self, _: SessionRef) -> anyhow::Result<Value> {
        panic!("unexpected data route")
    }
    async fn find_archived_thread_by_title(&self, _: String) -> anyhow::Result<Option<SessionRef>> {
        panic!("unexpected data route")
    }
}

fn context(enabled: Option<bool>) -> (BridgeContext, Arc<MockData>) {
    let data = Arc::new(MockData::default());
    let ctx = BridgeContext::new(
        Arc::new(MockSettings(enabled)),
        Arc::new(UnusedRuntime),
        data.clone(),
    );
    (ctx, data)
}

async fn assert_rejected(enabled: Option<bool>) {
    let _state = crate::common::IsolatedTestState::new();
    let (ctx, data) = context(enabled);
    let result = handle_bridge_request(ctx, "/usage/history", json!({"period":"all"})).await;
    assert_eq!(result["status"], "unavailable");
    assert_eq!(result["enabled"], false);
    assert_eq!(result["complete"], false);
    assert_eq!(result["records"], json!([]));
    assert_eq!(data.clears.load(Ordering::SeqCst), 1);
    assert!(data.queries.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn disabled_usage_history_rejects_and_clears_without_querying() {
    assert!(!BackendSettings::default().usage_history_enabled);
    assert_rejected(Some(false)).await;
}

#[tokio::test(flavor = "current_thread")]
async fn settings_failure_rejects_and_clears_without_querying() {
    assert_rejected(None).await;
}

#[tokio::test(flavor = "current_thread")]
async fn enabled_usage_history_forwards_original_payload_without_clearing() {
    let _state = crate::common::IsolatedTestState::new();
    let (ctx, data) = context(Some(true));
    let payload = json!({"period":"all","page":3,"pageSize":7,
    "includeSessionCursors":true,"search":"synthetic","prices":[{
        "model":"synthetic-route-model","currency":"USD",
        "input":0.123,"cachedInput":0.045,"output":0.678
    }]});
    let result = handle_bridge_request(ctx, "/usage/history", payload.clone()).await;
    assert_eq!(
        result,
        json!({"status":"ok","complete":true,"echo":payload})
    );
    assert_eq!(*data.queries.lock().unwrap(), vec![payload]);
    assert_eq!(data.clears.load(Ordering::SeqCst), 0);
}

#[test]
fn usage_history_and_proxy_status_settings_are_independent() {
    let settings: BackendSettings = serde_json::from_value(json!({
        "enhancementsEnabled": true,
    }))
    .unwrap();
    assert!(!settings.usage_history_enabled);
    assert!(settings.codex_app_proxy_request_status);

    let directory = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(directory.path().join("settings.json"));
    store.save(&settings).unwrap();
    let updated = store
        .update(json!({
            "usageHistoryEnabled": true,
            "codexAppProxyRequestStatus": false,
        }))
        .unwrap();
    assert!(updated.usage_history_enabled);
    assert!(!updated.codex_app_proxy_request_status);

    let updated = store
        .update(json!({ "usageHistoryEnabled": false }))
        .unwrap();
    assert!(!updated.usage_history_enabled);
    assert!(!updated.codex_app_proxy_request_status);

    let updated = store
        .update(json!({ "codexAppProxyRequestStatus": true }))
        .unwrap();
    assert!(!updated.usage_history_enabled);
    assert!(updated.codex_app_proxy_request_status);
    let persisted = store.load().unwrap();
    assert!(!persisted.usage_history_enabled);
    assert!(persisted.codex_app_proxy_request_status);
}
