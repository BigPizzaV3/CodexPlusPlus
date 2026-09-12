use std::sync::Arc;

use reqwest::StatusCode;
use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::net::TcpListener;
use xuan_plus_remote_bridge::{MobileRemote, RemoteBridgeState, router};

#[tokio::test(flavor = "current_thread")]
async fn bridge_requires_token_and_serves_status_tasks_and_validation_errors() {
    let directory = tempdir().unwrap();
    let home = directory.path().join("codex-home");
    std::fs::create_dir_all(home.join("sessions")).unwrap();
    let rollout = home.join("sessions").join("task.jsonl");
    std::fs::write(&rollout, "{}\n").unwrap();
    let index = Connection::open(home.join("state_5.sqlite")).unwrap();
    index
        .execute_batch(
            "CREATE TABLE threads(
               id TEXT, title TEXT, cwd TEXT, rollout_path TEXT,
               archived INTEGER, updated_at INTEGER
             );",
        )
        .unwrap();
    index
        .execute(
            "INSERT INTO threads VALUES (?1, ?2, ?3, ?4, 0, 1)",
            rusqlite::params![
                "official_task_0001",
                "Remote task",
                directory.path().to_string_lossy(),
                rollout.to_string_lossy(),
            ],
        )
        .unwrap();
    drop(index);

    let remote = Arc::new(MobileRemote::new(
        directory.path().join("mobile.sqlite"),
        home,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router(RemoteBridgeState::new(remote, Some("test-token".into()))),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::new();

    let unauthorized = client
        .get(format!("http://{address}/v1/mobile/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let status: Value = client
        .get(format!("http://{address}/v1/mobile/status"))
        .bearer_auth("test-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["enabled"], false);
    assert!(status.get("qrImage").is_some());

    for (path, payload) in [
        ("enable", json!({"enabled": false})),
        ("auto-sync", json!({"enabled": false})),
        ("select", json!({"selected": []})),
    ] {
        let response = client
            .post(format!("http://{address}/v1/mobile/{path}"))
            .bearer_auth("test-token")
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }

    let tasks: Value = client
        .post(format!("http://{address}/v1/mobile/tasks"))
        .bearer_auth("test-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tasks["tasks"][0]["id"], "official_task_0001");

    let invalid = client
        .post(format!("http://{address}/v1/mobile/send-input"))
        .bearer_auth("test-token")
        .json(&json!({
            "threadId": "../invalid",
            "clientRequestId": "request_0000000001",
            "text": "continue"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    server.abort();
}

#[cfg(windows)]
#[test]
fn state_initialization_preserves_legacy_table_contract_and_encrypts_identity() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("mobile-remote.sqlite");
    let remote = MobileRemote::new(database.clone(), directory.path().to_path_buf());
    remote.initialize_state().unwrap();
    let connection = Connection::open(database).unwrap();
    let tables: u32 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table' AND name IN ('mobile_identity', 'mobile_command_receipts')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 2);
    let protected_key: Vec<u8> = connection
        .query_row(
            "SELECT protected_key FROM mobile_identity WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!protected_key.is_empty());
    assert_ne!(protected_key.first().copied(), Some(0x30));
}
