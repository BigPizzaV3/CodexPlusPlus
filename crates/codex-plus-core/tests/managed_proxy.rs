use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use codex_plus_core::local_account::{
    LocalAccountExport, LocalAuthSessionExport, LocalDeviceExport, LocalEntitlementExport,
    LocalUserExport,
};
use codex_plus_core::local_backend::{
    DEFAULT_BACKEND_TEAM_ID, IdentitySyncBody, LocalBackendStore,
};
use codex_plus_core::local_usage::{LocalUsageExport, LocalUsageSummary};
use codex_plus_core::managed_proxy::{ManagedProxyConfig, handle_managed_proxy_http_request};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;

const PAYMENT_OFFICIAL_TEST_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsjmCcy1bzjI5mjZ7ym9y
sBcur7A07ZkeUqiVBNQ1hw/cXJGdjH+GmihJupTxfMdWktjADxB12zFWoMMi15Vw
EuasR+X8pzzrVDxEoKOLTr0eZckZ861sEYxl8N3WQw771YP1DpVSi1D9cWclGFEr
F2NB7pUNzoa4r2AnwVbla/FIsSW5+bTC9tGxPIfv3PZwDpPbrktxpWKvXf9Ssg/0
QvTiswyD2uAKIkzd7NNRIq8ckGPtbxOGzXed/LEpJECPrBRKu9YXV/hcUelDY7cL
p6nBPhUl0d1EJheKCtLU/dPGyyPhzUb0q5fqk2o49gxXKKXOfsc3vhobDOrlDi+o
jwIDAQAB
-----END PUBLIC KEY-----"#;

#[tokio::test]
async fn managed_proxy_health_exposes_backend_db_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("backend.sqlite");
    let store = LocalBackendStore::new(db_path.clone());
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        user_read_api_key: "user-read-secret".to_string(),
        billing_api_key: "billing-secret".to_string(),
        access_api_key: "access-secret".to_string(),
        audit_api_key: "audit-secret".to_string(),
        ..ManagedProxyConfig::default()
    };

    let response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/health HTTP/1.1\r\nHost: jiyi\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("response");
    let body: Value = serde_json::from_slice(&response.body).expect("health json");

    assert_eq!(response.status_code, 200);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["backendDbPath"], db_path.to_string_lossy().as_ref());
    assert_eq!(body["userReadKeyConfigured"], true);
    assert_eq!(body["billingKeyConfigured"], true);
    assert_eq!(body["accessKeyConfigured"], true);
    assert_eq!(body["auditKeyConfigured"], true);
}

#[tokio::test]
async fn managed_proxy_rejects_missing_session_before_upstream() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        upstream_api_key: "upstream-secret".to_string(),
        upstream_base_url: "http://127.0.0.1:9/v1".to_string(),
        ..ManagedProxyConfig::default()
    };

    let response = handle_managed_proxy_http_request(
        b"GET /v1/models HTTP/1.1\r\nHost: jiyi\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("response");

    assert_eq!(response.status_code, 401);
    assert!(!String::from_utf8_lossy(&response.body).contains("upstream-secret"));
}

#[tokio::test]
async fn managed_proxy_forwards_models_with_upstream_key_only_after_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    let token = sync_sample_identity(&store);
    let (base_url, received) =
        start_fake_upstream("HTTP/1.1 200 OK", "application/json", r#"{"data":[]}"#);
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        upstream_api_key: "upstream-secret".to_string(),
        upstream_base_url: base_url,
        ..ManagedProxyConfig::default()
    };
    let request =
        format!("GET /v1/models HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer {token}\r\n\r\n");

    let response = handle_managed_proxy_http_request(request.as_bytes(), &config, &store, &client)
        .await
        .expect("response");
    let upstream_request = received
        .recv_timeout(Duration::from_secs(2))
        .expect("upstream request");

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body, br#"{"data":[]}"#);
    assert!(upstream_request.starts_with("GET /v1/models HTTP/1.1"));
    assert!(upstream_request.contains("authorization: Bearer upstream-secret"));
    assert!(!String::from_utf8_lossy(&response.body).contains("upstream-secret"));
}

#[tokio::test]
async fn managed_proxy_admin_block_revokes_session_before_upstream() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    let token = sync_sample_identity(&store);
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        upstream_api_key: "upstream-secret".to_string(),
        upstream_base_url: "http://127.0.0.1:9/v1".to_string(),
        admin_api_key: "admin-secret".to_string(),
        ..ManagedProxyConfig::default()
    };
    let block_body = r#"{"userId":"user-1","reason":"abuse review"}"#;
    let block_request = format!(
        "POST /jiyi/v1/admin/users/block HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer admin-secret\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        block_body.len(),
        block_body
    );

    let block_response =
        handle_managed_proxy_http_request(block_request.as_bytes(), &config, &store, &client)
            .await
            .expect("block response");
    let block_payload: Value = serde_json::from_slice(&block_response.body).expect("block json");
    let models_request =
        format!("GET /v1/models HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer {token}\r\n\r\n");
    let models_response =
        handle_managed_proxy_http_request(models_request.as_bytes(), &config, &store, &client)
            .await
            .expect("models response");
    let models_payload: Value = serde_json::from_slice(&models_response.body).expect("models json");
    let audit_request = "GET /jiyi/v1/admin/audit/events?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer admin-secret\r\n\r\n";
    let audit_response =
        handle_managed_proxy_http_request(audit_request.as_bytes(), &config, &store, &client)
            .await
            .expect("audit response");
    let audit_payload: Value = serde_json::from_slice(&audit_response.body).expect("audit json");
    let users_request = "GET /jiyi/v1/admin/users?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer admin-secret\r\n\r\n";
    let users_response =
        handle_managed_proxy_http_request(users_request.as_bytes(), &config, &store, &client)
            .await
            .expect("users response");
    let users_payload: Value = serde_json::from_slice(&users_response.body).expect("users json");

    assert_eq!(block_response.status_code, 200);
    assert_eq!(block_payload["userAccess"]["status"], "blocked");
    assert_eq!(block_payload["userAccess"]["sessionsRevoked"], 1);
    assert_eq!(models_response.status_code, 401);
    assert_eq!(models_payload["error"]["code"], "user_blocked");
    assert_eq!(audit_response.status_code, 200);
    assert!(
        audit_payload["auditEvents"]
            .as_array()
            .expect("audit events")
            .iter()
            .any(|event| event["eventType"] == "user_access_updated"
                && event["actorType"] == "managed_proxy_admin_api"
                && event["subjectUserId"] == "user-1")
    );
    assert_eq!(users_response.status_code, 200);
    assert_eq!(users_payload["status"], "ok");
    assert_eq!(users_payload["users"][0]["userId"], "user-1");
    assert_eq!(users_payload["users"][0]["phoneMasked"], "+86 138****5678");
    assert_eq!(users_payload["users"][0]["accessStatus"], "blocked");
    assert_eq!(users_payload["users"][0]["activeSessionCount"], 0);
    assert_eq!(users_payload["users"][0]["revokedSessionCount"], 1);
    assert_eq!(users_payload["users"][0]["todayUsedTokens"], 70);
    assert!(!String::from_utf8_lossy(&models_response.body).contains("upstream-secret"));
    assert!(!String::from_utf8_lossy(&users_response.body).contains("upstream-secret"));
}

#[tokio::test]
async fn managed_proxy_admin_updates_entitlement_before_quota_check() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    let token = sync_sample_identity(&store);
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        upstream_api_key: "upstream-secret".to_string(),
        admin_api_key: "admin-secret".to_string(),
        ..ManagedProxyConfig::default()
    };
    let body = r#"{"userId":"user-1","planId":"jiyi_pro","planName":"极义 Pro","dailyTokenLimit":5000,"reason":"renewal paid"}"#;
    let entitlement_request = format!(
        "POST /jiyi/v1/admin/users/entitlement HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer admin-secret\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );

    let entitlement_response =
        handle_managed_proxy_http_request(entitlement_request.as_bytes(), &config, &store, &client)
            .await
            .expect("entitlement response");
    let entitlement_payload: Value =
        serde_json::from_slice(&entitlement_response.body).expect("entitlement json");
    let users_request = "GET /jiyi/v1/admin/users?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer admin-secret\r\n\r\n";
    let users_response =
        handle_managed_proxy_http_request(users_request.as_bytes(), &config, &store, &client)
            .await
            .expect("users response");
    let users_payload: Value = serde_json::from_slice(&users_response.body).expect("users json");
    let audit_request = "GET /jiyi/v1/admin/audit/events?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer admin-secret\r\n\r\n";
    let audit_response =
        handle_managed_proxy_http_request(audit_request.as_bytes(), &config, &store, &client)
            .await
            .expect("audit response");
    let audit_payload: Value = serde_json::from_slice(&audit_response.body).expect("audit json");
    let quota = store
        .quota_snapshot(&token)
        .expect("quota")
        .quota
        .expect("quota payload");

    assert_eq!(entitlement_response.status_code, 200);
    assert_eq!(entitlement_payload["status"], "ok");
    assert_eq!(entitlement_payload["entitlement"]["planId"], "jiyi_pro");
    assert_eq!(
        entitlement_payload["entitlement"]["previousPlanId"],
        "local_trial"
    );
    assert_eq!(users_response.status_code, 200);
    assert_eq!(users_payload["users"][0]["planId"], "jiyi_pro");
    assert_eq!(users_payload["users"][0]["planName"], "极义 Pro");
    assert_eq!(users_payload["users"][0]["dailyTokenLimit"], 5000);
    assert_eq!(users_payload["users"][0]["todayRemainingTokens"], 4930);
    assert_eq!(quota.plan_id.as_deref(), Some("jiyi_pro"));
    assert_eq!(quota.daily_token_limit, 5000);
    assert_eq!(quota.remaining_tokens, Some(4930));
    assert_eq!(audit_response.status_code, 200);
    assert!(
        audit_payload["auditEvents"]
            .as_array()
            .expect("audit events")
            .iter()
            .any(|event| event["eventType"] == "user_entitlement_updated"
                && event["subjectUserId"] == "user-1"
                && event["metadata"]["dailyTokenLimit"] == 5000)
    );
    assert!(!String::from_utf8_lossy(&entitlement_response.body).contains("upstream-secret"));
    assert!(!String::from_utf8_lossy(&users_response.body).contains("upstream-secret"));
}

#[tokio::test]
async fn managed_proxy_audit_key_is_read_only_and_supports_filters() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    sync_sample_identity(&store);
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        upstream_api_key: "upstream-secret".to_string(),
        admin_api_key: "admin-secret".to_string(),
        audit_api_key: "audit-secret".to_string(),
        ..ManagedProxyConfig::default()
    };
    let body = r#"{"userId":"user-1","planId":"jiyi_pro","planName":"极义 Pro","dailyTokenLimit":5000,"reason":"renewal paid"}"#;
    let entitlement_request = format!(
        "POST /jiyi/v1/admin/users/entitlement HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer admin-secret\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );

    let entitlement_response =
        handle_managed_proxy_http_request(entitlement_request.as_bytes(), &config, &store, &client)
            .await
            .expect("entitlement response");
    let audit_request = "GET /jiyi/v1/admin/audit/events?limit=10&eventType=user_entitlement_updated&actorType=managed_proxy_admin_api&subjectUserId=user-1 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer audit-secret\r\n\r\n";
    let audit_response =
        handle_managed_proxy_http_request(audit_request.as_bytes(), &config, &store, &client)
            .await
            .expect("audit response");
    let audit_payload: Value = serde_json::from_slice(&audit_response.body).expect("audit json");
    let denied_users_request = "GET /jiyi/v1/admin/users?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer audit-secret\r\n\r\n";
    let denied_users_response = handle_managed_proxy_http_request(
        denied_users_request.as_bytes(),
        &config,
        &store,
        &client,
    )
    .await
    .expect("denied users response");
    let denied_entitlement_request = format!(
        "POST /jiyi/v1/admin/users/entitlement HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer audit-secret\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let denied_entitlement_response = handle_managed_proxy_http_request(
        denied_entitlement_request.as_bytes(),
        &config,
        &store,
        &client,
    )
    .await
    .expect("denied entitlement response");
    let serialized_audit = String::from_utf8_lossy(&audit_response.body);

    assert_eq!(entitlement_response.status_code, 200);
    assert_eq!(audit_response.status_code, 200);
    let audit_events = audit_payload["auditEvents"]
        .as_array()
        .expect("audit events");
    assert_eq!(audit_events.len(), 1);
    assert_eq!(audit_events[0]["eventType"], "user_entitlement_updated");
    assert_eq!(audit_events[0]["actorType"], "managed_proxy_admin_api");
    assert_eq!(audit_events[0]["subjectUserId"], "user-1");
    assert_eq!(audit_events[0]["metadata"]["dailyTokenLimit"], 5000);
    assert_eq!(denied_users_response.status_code, 401);
    assert_eq!(denied_entitlement_response.status_code, 401);
    assert!(!serialized_audit.contains("upstream-secret"));
    assert!(!serialized_audit.contains("admin-secret"));
    assert!(!serialized_audit.contains("audit-secret"));
}

#[tokio::test]
async fn managed_proxy_role_keys_are_scoped_to_admin_routes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    sync_sample_identity(&store);
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        upstream_api_key: "upstream-secret".to_string(),
        user_read_api_key: "user-read-secret".to_string(),
        billing_api_key: "billing-secret".to_string(),
        access_api_key: "access-secret".to_string(),
        audit_api_key: "audit-secret".to_string(),
        ..ManagedProxyConfig::default()
    };
    let entitlement_body = r#"{"userId":"user-1","planId":"jiyi_pro","planName":"极义 Pro","dailyTokenLimit":5000,"reason":"renewal paid"}"#;
    let block_body = r#"{"userId":"user-1","reason":"risk review"}"#;

    let users_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/users?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer user-read-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("users response");
    let denied_read_entitlement = post_json_with_token(
        "/jiyi/v1/admin/users/entitlement",
        "user-read-secret",
        entitlement_body,
        &config,
        &store,
        &client,
    )
    .await;
    let entitlement_response = post_json_with_token(
        "/jiyi/v1/admin/users/entitlement",
        "billing-secret",
        entitlement_body,
        &config,
        &store,
        &client,
    )
    .await;
    let denied_billing_users = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/users?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer billing-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("denied billing users response");
    let block_response = post_json_with_token(
        "/jiyi/v1/admin/users/block",
        "access-secret",
        block_body,
        &config,
        &store,
        &client,
    )
    .await;
    let denied_access_entitlement = post_json_with_token(
        "/jiyi/v1/admin/users/entitlement",
        "access-secret",
        entitlement_body,
        &config,
        &store,
        &client,
    )
    .await;
    let audit_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/audit/events?limit=20 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer audit-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("audit response");
    let denied_audit_users = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/users?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer audit-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("denied audit users response");
    let audit_payload: Value = serde_json::from_slice(&audit_response.body).expect("audit json");
    let serialized_audit = String::from_utf8_lossy(&audit_response.body);

    assert_eq!(users_response.status_code, 200);
    assert_eq!(denied_read_entitlement.status_code, 401);
    assert_eq!(entitlement_response.status_code, 200);
    assert_eq!(denied_billing_users.status_code, 401);
    assert_eq!(block_response.status_code, 200);
    assert_eq!(denied_access_entitlement.status_code, 401);
    assert_eq!(audit_response.status_code, 200);
    assert_eq!(denied_audit_users.status_code, 401);
    let audit_events = audit_payload["auditEvents"]
        .as_array()
        .expect("audit events");
    assert!(audit_events.iter().any(|event| {
        event["eventType"] == "user_entitlement_updated" && event["actorId"] == "billing_api_key"
    }));
    assert!(audit_events.iter().any(|event| {
        event["eventType"] == "user_access_updated" && event["actorId"] == "access_api_key"
    }));
    assert!(!serialized_audit.contains("user-read-secret"));
    assert!(!serialized_audit.contains("billing-secret"));
    assert!(!serialized_audit.contains("access-secret"));
    assert!(!serialized_audit.contains("audit-secret"));
    assert!(!serialized_audit.contains("upstream-secret"));
}

#[tokio::test]
async fn managed_proxy_admin_teams_are_scoped_to_user_read_and_billing_keys() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    sync_sample_identity(&store);
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        user_read_api_key: "user-read-secret".to_string(),
        billing_api_key: "billing-secret".to_string(),
        audit_api_key: "audit-secret".to_string(),
        ..ManagedProxyConfig::default()
    };
    let entitlement_body = format!(
        r#"{{"teamId":"{}","planId":"team_pro","planName":"团队 Pro","dailyTokenLimit":5000,"reason":"team renewal"}}"#,
        DEFAULT_BACKEND_TEAM_ID
    );

    let teams_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/teams?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer user-read-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("teams response");
    let denied_read_entitlement = post_json_with_token(
        "/jiyi/v1/admin/teams/entitlement",
        "user-read-secret",
        &entitlement_body,
        &config,
        &store,
        &client,
    )
    .await;
    let entitlement_response = post_json_with_token(
        "/jiyi/v1/admin/teams/entitlement",
        "billing-secret",
        &entitlement_body,
        &config,
        &store,
        &client,
    )
    .await;
    let denied_billing_teams = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/teams?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer billing-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("denied billing teams response");
    let teams_after_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/teams?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer user-read-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("teams after response");
    let audit_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/audit/events?limit=20&eventType=team_entitlement_updated HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer audit-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("audit response");
    let teams_payload: Value = serde_json::from_slice(&teams_response.body).expect("teams json");
    let entitlement_payload: Value =
        serde_json::from_slice(&entitlement_response.body).expect("entitlement json");
    let teams_after_payload: Value =
        serde_json::from_slice(&teams_after_response.body).expect("teams after json");
    let audit_payload: Value = serde_json::from_slice(&audit_response.body).expect("audit json");
    let serialized_teams = String::from_utf8_lossy(&teams_after_response.body);
    let serialized_audit = String::from_utf8_lossy(&audit_response.body);

    assert_eq!(teams_response.status_code, 200);
    assert_eq!(teams_payload["status"], "ok");
    assert_eq!(teams_payload["teams"][0]["teamId"], DEFAULT_BACKEND_TEAM_ID);
    assert_eq!(teams_payload["teams"][0]["memberCount"], 1);
    assert_eq!(teams_payload["teams"][0]["todayUsedTokens"], 70);
    assert_eq!(denied_read_entitlement.status_code, 401);
    assert_eq!(entitlement_response.status_code, 200);
    assert_eq!(
        entitlement_payload["teamEntitlement"]["teamId"],
        DEFAULT_BACKEND_TEAM_ID
    );
    assert_eq!(entitlement_payload["teamEntitlement"]["planId"], "team_pro");
    assert_eq!(denied_billing_teams.status_code, 401);
    assert_eq!(teams_after_response.status_code, 200);
    assert_eq!(teams_after_payload["teams"][0]["planId"], "team_pro");
    assert_eq!(teams_after_payload["teams"][0]["dailyTokenLimit"], 5000);
    assert_eq!(
        teams_after_payload["teams"][0]["todayRemainingTokens"],
        4930
    );
    assert_eq!(audit_response.status_code, 200);
    assert_eq!(
        audit_payload["auditEvents"][0]["eventType"],
        "team_entitlement_updated"
    );
    assert_eq!(
        audit_payload["auditEvents"][0]["metadata"]["teamId"],
        DEFAULT_BACKEND_TEAM_ID
    );
    assert_eq!(
        audit_payload["auditEvents"][0]["actorId"],
        "billing_api_key"
    );
    assert!(!serialized_teams.contains("hash-only"));
    assert!(!serialized_teams.contains("billing-secret"));
    assert!(!serialized_audit.contains("billing-secret"));
    assert!(!serialized_audit.contains("audit-secret"));
}

#[tokio::test]
async fn managed_proxy_billing_renewals_are_scoped_to_billing_key() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    sync_sample_identity(&store);
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        user_read_api_key: "user-read-secret".to_string(),
        billing_api_key: "billing-secret".to_string(),
        audit_api_key: "audit-secret".to_string(),
        ..ManagedProxyConfig::default()
    };
    let body = r#"{"subjectType":"user","subjectId":"user-1","planId":"jiyi_pro","planName":"极义 Pro","dailyTokenLimit":5000,"amountCents":19900,"currency":"cny","paymentChannel":"manual","externalOrderId":"order-001","reason":"manual renewal"}"#;

    let renewal_response = post_json_with_token(
        "/jiyi/v1/admin/billing/renewals",
        "billing-secret",
        body,
        &config,
        &store,
        &client,
    )
    .await;
    let denied_read_renewals = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/billing/renewals?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer user-read-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("denied read renewals response");
    let renewals_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/billing/renewals?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer billing-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("renewals response");
    let users_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/users?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer user-read-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("users response");
    let audit_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/audit/events?limit=10&eventType=billing_renewal_recorded HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer audit-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("audit response");
    let renewal_payload: Value =
        serde_json::from_slice(&renewal_response.body).expect("renewal json");
    let renewals_payload: Value =
        serde_json::from_slice(&renewals_response.body).expect("renewals json");
    let users_payload: Value = serde_json::from_slice(&users_response.body).expect("users json");
    let audit_payload: Value = serde_json::from_slice(&audit_response.body).expect("audit json");
    let serialized_renewals = String::from_utf8_lossy(&renewals_response.body);
    let serialized_audit = String::from_utf8_lossy(&audit_response.body);

    assert_eq!(renewal_response.status_code, 200);
    assert_eq!(renewal_payload["status"], "ok");
    assert_eq!(renewal_payload["renewal"]["subjectType"], "user");
    assert_eq!(renewal_payload["renewal"]["subjectId"], "user-1");
    assert_eq!(renewal_payload["renewal"]["planId"], "jiyi_pro");
    assert_eq!(renewal_payload["renewal"]["amountCents"], 19900);
    assert_eq!(renewal_payload["renewal"]["currency"], "CNY");
    assert_eq!(denied_read_renewals.status_code, 401);
    assert_eq!(renewals_response.status_code, 200);
    assert_eq!(
        renewals_payload["renewals"][0]["externalOrderId"],
        "order-001"
    );
    assert_eq!(users_response.status_code, 200);
    assert_eq!(users_payload["users"][0]["planId"], "jiyi_pro");
    assert_eq!(users_payload["users"][0]["todayRemainingTokens"], 4930);
    assert_eq!(audit_response.status_code, 200);
    assert_eq!(
        audit_payload["auditEvents"][0]["eventType"],
        "billing_renewal_recorded"
    );
    assert_eq!(
        audit_payload["auditEvents"][0]["actorId"],
        "billing_api_key"
    );
    assert_eq!(
        audit_payload["auditEvents"][0]["metadata"]["amountCents"],
        19900
    );
    assert!(!serialized_renewals.contains("billing-secret"));
    assert!(!serialized_renewals.contains("user-read-secret"));
    assert!(!serialized_audit.contains("billing-secret"));
    assert!(!serialized_audit.contains("audit-secret"));
}

#[tokio::test]
async fn managed_proxy_payment_webhook_reconciles_with_dedicated_key() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    sync_sample_identity(&store);
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        user_read_api_key: "user-read-secret".to_string(),
        billing_api_key: "billing-secret".to_string(),
        payment_webhook_api_key: "webhook-secret".to_string(),
        audit_api_key: "audit-secret".to_string(),
        ..ManagedProxyConfig::default()
    };
    let body = r#"{"provider":"mockpay","gatewayEventId":"evt_001","externalOrderId":"pay-order-001","status":"trade_success","subjectType":"user","subjectId":"user-1","planId":"jiyi_pro","planName":"极义 Pro","dailyTokenLimit":5000,"amountCents":19900,"currency":"cny","reason":"gateway callback","rawPayload":{"secret":"payment-secret","payerPhone":"13800000000"}}"#;

    let health_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/health HTTP/1.1\r\nHost: jiyi\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("health response");
    let denied_read_webhook = post_json_with_token(
        "/jiyi/v1/billing/payment-webhook",
        "user-read-secret",
        body,
        &config,
        &store,
        &client,
    )
    .await;
    let webhook_response = post_json_with_token(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        body,
        &config,
        &store,
        &client,
    )
    .await;
    let duplicate_response = post_json_with_token(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        body,
        &config,
        &store,
        &client,
    )
    .await;
    let reconcile_response = post_json_with_token(
        "/jiyi/v1/admin/billing/reconcile?limit=10",
        "billing-secret",
        "{}",
        &config,
        &store,
        &client,
    )
    .await;
    let denied_read_reconcile = post_json_with_token(
        "/jiyi/v1/admin/billing/reconcile?limit=10",
        "user-read-secret",
        "{}",
        &config,
        &store,
        &client,
    )
    .await;
    let users_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/users?limit=10 HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer user-read-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("users response");
    let audit_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/admin/audit/events?limit=20&eventType=billing_payment_webhook_received HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer audit-secret\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("audit response");

    let health_payload: Value = serde_json::from_slice(&health_response.body).expect("health json");
    let webhook_payload: Value =
        serde_json::from_slice(&webhook_response.body).expect("webhook json");
    let duplicate_payload: Value =
        serde_json::from_slice(&duplicate_response.body).expect("duplicate json");
    let reconcile_payload: Value =
        serde_json::from_slice(&reconcile_response.body).expect("reconcile json");
    let users_payload: Value = serde_json::from_slice(&users_response.body).expect("users json");
    let audit_payload: Value = serde_json::from_slice(&audit_response.body).expect("audit json");
    let serialized_webhook = String::from_utf8_lossy(&webhook_response.body);
    let serialized_audit = String::from_utf8_lossy(&audit_response.body);

    assert_eq!(health_response.status_code, 200);
    assert_eq!(health_payload["paymentWebhookKeyConfigured"], true);
    assert_eq!(denied_read_webhook.status_code, 401);
    assert_eq!(webhook_response.status_code, 200);
    assert_eq!(webhook_payload["status"], "ok");
    assert_eq!(webhook_payload["payment"]["duplicate"], false);
    assert_eq!(
        webhook_payload["payment"]["event"]["processingStatus"],
        "applied"
    );
    assert_eq!(
        webhook_payload["payment"]["renewal"]["externalOrderId"],
        "pay-order-001"
    );
    assert_eq!(duplicate_response.status_code, 200);
    assert_eq!(duplicate_payload["payment"]["duplicate"], true);
    assert_eq!(reconcile_response.status_code, 200);
    assert_eq!(reconcile_payload["reconciliation"]["attempted"], 0);
    assert_eq!(denied_read_reconcile.status_code, 401);
    assert_eq!(users_response.status_code, 200);
    assert_eq!(users_payload["users"][0]["planId"], "jiyi_pro");
    assert_eq!(users_payload["users"][0]["todayRemainingTokens"], 4930);
    assert_eq!(audit_response.status_code, 200);
    assert_eq!(
        audit_payload["auditEvents"][0]["eventType"],
        "billing_payment_webhook_received"
    );
    assert_eq!(
        audit_payload["auditEvents"][0]["actorId"],
        "payment_webhook_api_key"
    );
    assert!(!serialized_webhook.contains("rawPayload"));
    assert!(!serialized_webhook.contains("payment-secret"));
    assert!(!serialized_webhook.contains("13800000000"));
    assert!(!serialized_webhook.contains("webhook-secret"));
    assert!(!serialized_audit.contains("payment-secret"));
    assert!(!serialized_audit.contains("13800000000"));
    assert!(!serialized_audit.contains("webhook-secret"));
    assert!(!serialized_audit.contains("audit-secret"));
}

#[tokio::test]
async fn managed_proxy_payment_webhook_requires_hmac_signature_when_configured() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    sync_sample_identity(&store);
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        payment_webhook_api_key: "webhook-secret".to_string(),
        payment_webhook_signature_secret: "signing-secret".to_string(),
        ..ManagedProxyConfig::default()
    };
    let body = r#"{"provider":"mockpay","gatewayEventId":"evt_signed_001","externalOrderId":"signed-order-001","status":"paid","subjectType":"user","subjectId":"user-1","planId":"jiyi_pro","planName":"极义 Pro","dailyTokenLimit":5000,"amountCents":19900,"currency":"cny","reason":"signed gateway callback"}"#;

    let missing_signature = post_json_with_token(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        body,
        &config,
        &store,
        &client,
    )
    .await;
    let timestamp = OffsetDateTime::now_utc().unix_timestamp().to_string();
    let invalid_signature = post_json_with_headers(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        body,
        &[
            ("X-Jiyi-Payment-Timestamp", timestamp.as_str()),
            (
                "X-Jiyi-Payment-Signature",
                "sha256=0000000000000000000000000000000000000000000000000000000000000000",
            ),
        ],
        &config,
        &store,
        &client,
    )
    .await;
    let stale_timestamp = "1000";
    let stale_signature_value = format!(
        "sha256={}",
        payment_webhook_signature("signing-secret", stale_timestamp, body)
    );
    let stale_signature = post_json_with_headers(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        body,
        &[
            ("X-Jiyi-Payment-Timestamp", stale_timestamp),
            ("X-Jiyi-Payment-Signature", stale_signature_value.as_str()),
        ],
        &config,
        &store,
        &client,
    )
    .await;
    let valid_signature_value = format!(
        "sha256={}",
        payment_webhook_signature("signing-secret", &timestamp, body)
    );
    let valid_signature = post_json_with_headers(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        body,
        &[
            ("X-Jiyi-Payment-Timestamp", timestamp.as_str()),
            ("X-Jiyi-Payment-Signature", valid_signature_value.as_str()),
        ],
        &config,
        &store,
        &client,
    )
    .await;
    let health_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/health HTTP/1.1\r\nHost: jiyi\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("health response");

    let missing_payload: Value =
        serde_json::from_slice(&missing_signature.body).expect("missing json");
    let invalid_payload: Value =
        serde_json::from_slice(&invalid_signature.body).expect("invalid json");
    let stale_payload: Value = serde_json::from_slice(&stale_signature.body).expect("stale json");
    let valid_payload: Value = serde_json::from_slice(&valid_signature.body).expect("valid json");
    let health_payload: Value = serde_json::from_slice(&health_response.body).expect("health json");
    let serialized_valid = String::from_utf8_lossy(&valid_signature.body);

    assert_eq!(missing_signature.status_code, 401);
    assert_eq!(
        missing_payload["error"]["code"],
        "missing_payment_signature"
    );
    assert_eq!(invalid_signature.status_code, 401);
    assert_eq!(
        invalid_payload["error"]["code"],
        "invalid_payment_signature"
    );
    assert_eq!(stale_signature.status_code, 401);
    assert_eq!(stale_payload["error"]["code"], "stale_payment_signature");
    assert_eq!(valid_signature.status_code, 200);
    assert_eq!(
        valid_payload["payment"]["event"]["processingStatus"],
        "applied"
    );
    assert_eq!(health_payload["paymentWebhookSignatureConfigured"], true);
    assert!(!serialized_valid.contains("signing-secret"));
    assert!(!serialized_valid.contains("webhook-secret"));
}

#[tokio::test]
async fn managed_proxy_payment_webhook_verifies_alipay_rsa2_when_configured() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    sync_sample_identity(&store);
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        payment_webhook_api_key: "webhook-secret".to_string(),
        payment_webhook_alipay_public_key: PAYMENT_OFFICIAL_TEST_PUBLIC_KEY.to_string(),
        ..ManagedProxyConfig::default()
    };
    let missing_body = r#"{"provider":"alipay","gatewayEventId":"alipay_evt_missing","externalOrderId":"alipay-order-missing","status":"trade_success","subjectType":"user","subjectId":"user-1","planId":"jiyi_pro","planName":"Jiyi Pro","dailyTokenLimit":5000,"amountCents":19900,"currency":"CNY","paymentChannel":"alipay","reason":"alipay callback","rawPayload":{"app_id":"app-1","out_trade_no":"alipay-order-001","total_amount":"199.00","trade_status":"TRADE_SUCCESS","sign_type":"RSA2"}}"#;
    let invalid_body = r#"{"provider":"alipay","gatewayEventId":"alipay_evt_invalid","externalOrderId":"alipay-order-invalid","status":"trade_success","subjectType":"user","subjectId":"user-1","planId":"jiyi_pro","planName":"Jiyi Pro","dailyTokenLimit":5000,"amountCents":19900,"currency":"CNY","paymentChannel":"alipay","reason":"alipay callback","rawPayload":{"app_id":"app-1","out_trade_no":"alipay-order-001","total_amount":"199.00","trade_status":"TRADE_SUCCESS","sign_type":"RSA2","sign":"invalid-sign"}}"#;
    let valid_body = r#"{"provider":"alipay","gatewayEventId":"alipay_evt_001","externalOrderId":"alipay-order-001","status":"trade_success","subjectType":"user","subjectId":"user-1","planId":"jiyi_pro","planName":"Jiyi Pro","dailyTokenLimit":5000,"amountCents":19900,"currency":"CNY","paymentChannel":"alipay","reason":"alipay callback","rawPayload":{"app_id":"app-1","out_trade_no":"alipay-order-001","total_amount":"199.00","trade_status":"TRADE_SUCCESS","sign_type":"RSA2","sign":"bH2htyuwz1BgrhpLp+A178Iimp/OT9IcVHePlq0b2W1M51kUl29Rp/P85jfMYJr6V+XgBE+vdb+IkRJa+0tMh0P54ybKonQQ8VKrC9DNtLXVLqfuIbRORWcF3CCyaJhzFSwb7qzrhhwS8v5LgvHH8XLKZjVNv8jxce2U7IAdN9QwBLWZRRYsUfRRFrnuv+CgGJaZPnsX6q6iRyzidSzX/if/FtDqGr8rr0FTbTlmhF7GwFxpk6B8tD+PIsXi7nrLytC/JxGcrYnIo82RzRo/RXmpJPG3qGY4nYAjR8b9g6Vp0RgvC/9KZhjRoIXeJqR/OXYtyJcQcsMFvPbToUzGGw=="}}"#;

    let missing_signature = post_json_with_token(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        missing_body,
        &config,
        &store,
        &client,
    )
    .await;
    let invalid_signature = post_json_with_token(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        invalid_body,
        &config,
        &store,
        &client,
    )
    .await;
    let valid_signature = post_json_with_token(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        valid_body,
        &config,
        &store,
        &client,
    )
    .await;
    let health_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/health HTTP/1.1\r\nHost: jiyi\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("health response");

    let missing_payload: Value =
        serde_json::from_slice(&missing_signature.body).expect("missing json");
    let invalid_payload: Value =
        serde_json::from_slice(&invalid_signature.body).expect("invalid json");
    let valid_payload: Value = serde_json::from_slice(&valid_signature.body).expect("valid json");
    let health_payload: Value = serde_json::from_slice(&health_response.body).expect("health json");
    let serialized_valid = String::from_utf8_lossy(&valid_signature.body);
    let serialized_health = String::from_utf8_lossy(&health_response.body);

    assert_eq!(missing_signature.status_code, 401);
    assert_eq!(
        missing_payload["error"]["code"],
        "missing_official_payment_signature"
    );
    assert_eq!(invalid_signature.status_code, 401);
    assert_eq!(
        invalid_payload["error"]["code"],
        "invalid_official_payment_signature"
    );
    assert_eq!(valid_signature.status_code, 200);
    assert_eq!(
        valid_payload["payment"]["event"]["processingStatus"],
        "applied"
    );
    assert_eq!(
        valid_payload["payment"]["renewal"]["externalOrderId"],
        "alipay-order-001"
    );
    assert_eq!(
        health_payload["paymentWebhookAlipaySignatureConfigured"],
        true
    );
    assert_eq!(
        health_payload["paymentWebhookWechatpaySignatureConfigured"],
        false
    );
    assert!(!serialized_valid.contains("webhook-secret"));
    assert!(!serialized_health.contains("webhook-secret"));
    assert!(!serialized_health.contains("BEGIN PUBLIC KEY"));
}

#[tokio::test]
async fn managed_proxy_payment_webhook_verifies_wechatpay_rsa2_when_configured() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    sync_sample_identity(&store);
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        payment_webhook_api_key: "webhook-secret".to_string(),
        payment_webhook_wechatpay_public_key: PAYMENT_OFFICIAL_TEST_PUBLIC_KEY.to_string(),
        ..ManagedProxyConfig::default()
    };
    let body = r#"{"provider":"wechatpay","gatewayEventId":"wechat_evt_001","externalOrderId":"wechat-order-001","status":"SUCCESS","subjectType":"user","subjectId":"user-1","planId":"jiyi_pro","planName":"Jiyi Pro","dailyTokenLimit":5000,"amountCents":19900,"currency":"CNY","paymentChannel":"wechatpay","reason":"wechat callback"}"#;

    let missing_signature = post_json_with_token(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        body,
        &config,
        &store,
        &client,
    )
    .await;
    let invalid_signature = post_json_with_headers(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        body,
        &[
            ("Wechatpay-Timestamp", "4102444800"),
            ("Wechatpay-Nonce", "nonce-test"),
            ("Wechatpay-Signature", "invalid-signature"),
        ],
        &config,
        &store,
        &client,
    )
    .await;
    let valid_signature = post_json_with_headers(
        "/jiyi/v1/billing/payment-webhook",
        "webhook-secret",
        body,
        &[
            ("Wechatpay-Timestamp", "4102444800"),
            ("Wechatpay-Nonce", "nonce-test"),
            (
                "Wechatpay-Signature",
                "CSSPoQKMFLp2bYM5fCf2MNE4zpwHXvYjpt62r6Sbf5VxwFcfomBlVGi7E+cRDVQ/ldVpO/ABbnzHh1PP/q2XtdxUwYdVoV0MWW+/AGXnqSQ9sJ8dQ5ct/c2O4d0uPppaOCg+4ok7mvsSXXCXEmgbseQwCA71bDFSHyi97mthZEe6VUEaP1EHxK6HXxF4mznswWDYuHR2fSgfB/WinmCHFUV2+szZZpK/wsG/1qAcbiwO+MNJtvoupPGf/bTVorVJRvj3AHOIdDnR7yCEQVjk3zQEZChY2Hh7u9xgXtxalwOkGD17xtxLncEtPgKJA8yEmgbBNpokmPN8YlniVwxZXA==",
            ),
        ],
        &config,
        &store,
        &client,
    )
    .await;
    let health_response = handle_managed_proxy_http_request(
        b"GET /jiyi/v1/health HTTP/1.1\r\nHost: jiyi\r\n\r\n",
        &config,
        &store,
        &client,
    )
    .await
    .expect("health response");

    let missing_payload: Value =
        serde_json::from_slice(&missing_signature.body).expect("missing json");
    let invalid_payload: Value =
        serde_json::from_slice(&invalid_signature.body).expect("invalid json");
    let valid_payload: Value = serde_json::from_slice(&valid_signature.body).expect("valid json");
    let health_payload: Value = serde_json::from_slice(&health_response.body).expect("health json");
    let serialized_valid = String::from_utf8_lossy(&valid_signature.body);
    let serialized_health = String::from_utf8_lossy(&health_response.body);

    assert_eq!(missing_signature.status_code, 401);
    assert_eq!(
        missing_payload["error"]["code"],
        "missing_official_payment_signature"
    );
    assert_eq!(invalid_signature.status_code, 401);
    assert_eq!(
        invalid_payload["error"]["code"],
        "invalid_official_payment_signature"
    );
    assert_eq!(valid_signature.status_code, 200);
    assert_eq!(
        valid_payload["payment"]["event"]["processingStatus"],
        "applied"
    );
    assert_eq!(
        valid_payload["payment"]["renewal"]["externalOrderId"],
        "wechat-order-001"
    );
    assert_eq!(
        health_payload["paymentWebhookAlipaySignatureConfigured"],
        false
    );
    assert_eq!(
        health_payload["paymentWebhookWechatpaySignatureConfigured"],
        true
    );
    assert!(!serialized_valid.contains("webhook-secret"));
    assert!(!serialized_health.contains("webhook-secret"));
    assert!(!serialized_health.contains("BEGIN PUBLIC KEY"));
}

#[tokio::test]
async fn managed_proxy_records_usage_after_responses_forward() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    let token = sync_sample_identity(&store);
    let (base_url, received) = start_fake_upstream(
        "HTTP/1.1 200 OK",
        "application/json",
        r#"{"id":"resp_test","object":"response","usage":{"input_tokens":5,"output_tokens":7,"total_tokens":12},"output":[]}"#,
    );
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        upstream_api_key: "upstream-secret".to_string(),
        upstream_base_url: base_url,
        ..ManagedProxyConfig::default()
    };
    let body = r#"{"model":"gpt-5.5","input":"hello"}"#;
    let request = format!(
        "POST /v1/responses HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );

    let response = handle_managed_proxy_http_request(request.as_bytes(), &config, &store, &client)
        .await
        .expect("response");
    let upstream_request = received
        .recv_timeout(Duration::from_secs(2))
        .expect("upstream request");
    let quota = store
        .quota_snapshot(&token)
        .expect("quota")
        .quota
        .expect("quota payload");

    assert_eq!(response.status_code, 200);
    assert!(upstream_request.starts_with("POST /v1/responses HTTP/1.1"));
    assert!(upstream_request.contains("authorization: Bearer upstream-secret"));
    assert!(upstream_request.contains(body));
    assert_eq!(quota.used_tokens, 82);
    assert_eq!(quota.request_count, 3);
    assert_eq!(quota.remaining_tokens, Some(918));
}

#[tokio::test]
async fn managed_proxy_identity_sync_requires_sync_api_key() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        identity_sync_api_key: "sync-secret".to_string(),
        ..ManagedProxyConfig::default()
    };
    let body = serde_json::to_string(&serde_json::json!({ "body": sample_body() })).expect("body");
    let missing_auth_request = format!(
        "POST /jiyi/v1/identity/sync HTTP/1.1\r\nHost: jiyi\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );

    let response = handle_managed_proxy_http_request(
        missing_auth_request.as_bytes(),
        &config,
        &store,
        &client,
    )
    .await
    .expect("response");

    assert_eq!(response.status_code, 401);
}

#[tokio::test]
async fn managed_proxy_identity_sync_accepts_wrapped_body_and_returns_session_token() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
    let client = reqwest::Client::new();
    let config = ManagedProxyConfig {
        identity_sync_api_key: "sync-secret".to_string(),
        ..ManagedProxyConfig::default()
    };
    let body = serde_json::to_string(&serde_json::json!({
        "generatedAtMs": current_ms(),
        "schemaVersion": 1,
        "endpoint": "http://127.0.0.1:57421/jiyi/v1/identity/sync",
        "method": "POST",
        "headers": {
            "authorization": "Bearer <redacted>",
            "content-type": "application/json"
        },
        "piiPolicy": "masked",
        "body": sample_body()
    }))
    .expect("body");
    let request = format!(
        "POST /jiyi/v1/identity/sync HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer sync-secret\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );

    let response = handle_managed_proxy_http_request(request.as_bytes(), &config, &store, &client)
        .await
        .expect("response");
    let payload: Value = serde_json::from_slice(&response.body).expect("json response");
    let token = payload
        .pointer("/activeSession/accessToken")
        .and_then(Value::as_str)
        .expect("active session token");
    let verified = store.verify_session_token(token).expect("verify token");

    assert_eq!(response.status_code, 200);
    assert!(token.starts_with("jiyi-local-"));
    assert!(verified.authenticated);
    assert_eq!(
        payload
            .pointer("/receipt/usersUpserted")
            .and_then(Value::as_u64),
        Some(1)
    );
}

async fn post_json_with_token(
    path: &str,
    token: &str,
    body: &str,
    config: &ManagedProxyConfig,
    store: &LocalBackendStore,
    client: &reqwest::Client,
) -> codex_plus_core::managed_proxy::ManagedProxyHttpResponse {
    post_json_with_headers(path, token, body, &[], config, store, client).await
}

async fn post_json_with_headers(
    path: &str,
    token: &str,
    body: &str,
    headers: &[(&str, &str)],
    config: &ManagedProxyConfig,
    store: &LocalBackendStore,
    client: &reqwest::Client,
) -> codex_plus_core::managed_proxy::ManagedProxyHttpResponse {
    let extra_headers = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect::<String>();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: jiyi\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\n{extra_headers}Content-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    handle_managed_proxy_http_request(request.as_bytes(), config, store, client)
        .await
        .expect("post response")
}

fn payment_webhook_signature(secret: &str, timestamp: &str, body: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sync_sample_identity(store: &LocalBackendStore) -> String {
    store
        .apply_identity_sync(&sample_body())
        .expect("sync")
        .active_session
        .expect("active session")
        .access_token
}

fn start_fake_upstream(
    status_line: &'static str,
    content_type: &'static str,
    body: &'static str,
) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind upstream");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let request = read_http_request(&mut stream);
        tx.send(request).expect("send request");
        let response = format!(
            "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).expect("write");
    });
    (format!("http://{addr}/v1"), rx)
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut header_end = None;
    let mut content_length = 0_usize;
    loop {
        let read = stream.read(&mut chunk).expect("read");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if header_end.is_none() {
            header_end = find_header_end(&buffer);
            if let Some(end) = header_end {
                content_length = content_length_from_headers(&buffer[..end]).unwrap_or(0);
            }
        }
        if let Some(end) = header_end {
            if buffer.len() >= end + 4 + content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length_from_headers(headers: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(headers);
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn sample_body() -> IdentitySyncBody {
    let now = current_ms();
    IdentitySyncBody {
        generated_at_ms: now,
        schema_version: 1,
        pii_policy: "masked".to_string(),
        account: LocalAccountExport {
            generated_at_ms: now,
            db_path: "/tmp/local.sqlite".to_string(),
            active_session: Some(LocalAuthSessionExport {
                user_id: "user-1".to_string(),
                phone_masked: "+86 138****5678".to_string(),
                login_at_ms: now,
                expires_at_ms: now + 60 * 60 * 1000,
                device_id: "device-1".to_string(),
                session_expired: false,
            }),
            users: vec![LocalUserExport {
                user_id: "user-1".to_string(),
                phone_masked: "+86 138****5678".to_string(),
                phone_hash: "hash-only".to_string(),
                created_at_ms: now - 1000,
                last_login_at_ms: now,
            }],
            devices: vec![LocalDeviceExport {
                user_id: "user-1".to_string(),
                device_id: "device-1".to_string(),
                first_seen_at_ms: now - 1000,
                last_seen_at_ms: now,
            }],
            entitlements: vec![LocalEntitlementExport {
                user_id: "user-1".to_string(),
                plan_id: "local_trial".to_string(),
                plan_name: "本地试用".to_string(),
                daily_token_limit: 1000,
                updated_at_ms: now,
            }],
        },
        usage: LocalUsageExport {
            generated_at_ms: now,
            db_path: "/tmp/usage.sqlite".to_string(),
            summaries: vec![LocalUsageSummary {
                day: today_key(),
                subject_id: "user-1".to_string(),
                plan_id: Some("local_trial".to_string()),
                request_count: 2,
                request_bytes: 100,
                response_bytes: 200,
                estimated_tokens: 75,
                reported_total_tokens: 70,
                effective_total_tokens: 70,
                first_seen_at_ms: now - 1000,
                last_seen_at_ms: now,
            }],
        },
    }
}

fn current_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_millis() as i64
}

fn today_key() -> String {
    let now = time::OffsetDateTime::from_unix_timestamp(current_ms() / 1000).expect("date");
    let date = now.date();
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        date.month() as u8,
        date.day()
    )
}
