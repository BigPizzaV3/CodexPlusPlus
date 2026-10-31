use codex_plus_core::script_market::{
    download_script, fetch_market_manifest, market_scripts_payload, parse_market_manifest,
};
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// 脚本市场改走统一代理客户端后（#1329 / #1339），回环地址仍需直连，
// 即便进程里设置了 HTTPS_PROXY 也不能把本地请求送进代理。
#[tokio::test]
async fn fetch_market_manifest_reads_index_through_shared_client() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/index.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": 2,
            "updated_at": "2026-10-25",
            "scripts": [{
                "id": "demo",
                "name": "Demo",
                "version": "1.0.0",
                "script_url": format!("{}/demo.js", server.uri()),
            }],
        })))
        .mount(&server)
        .await;

    let manifest = fetch_market_manifest(&format!("{}/index.json", server.uri()))
        .await
        .unwrap();

    assert_eq!(manifest.version, 2);
    assert_eq!(manifest.scripts.len(), 1);
    assert_eq!(manifest.scripts[0].id, "demo");
}

#[tokio::test]
async fn fetch_market_manifest_error_keeps_status_context() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/index.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let error = fetch_market_manifest(&format!("{}/index.json", server.uri()))
        .await
        .unwrap_err();
    let message = format!("{error:#}");

    assert!(message.contains("script market index returned an error status"));
    assert!(message.contains("404"));
}

#[tokio::test]
async fn download_script_returns_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/demo.js"))
        .respond_with(ResponseTemplate::new(200).set_body_string("console.log('ok');"))
        .mount(&server)
        .await;

    let body = download_script(&format!("{}/demo.js", server.uri()))
        .await
        .unwrap();

    assert_eq!(body, b"console.log('ok');");
}

// 市场清单逐条脚本的更新时间（#2781）：兼容 updated_at / updatedAt，缺省为空，
// 并原样透传到给前端的 payload，供卡片显示和按更新时间排序。
#[test]
fn market_script_updated_at_is_parsed_and_forwarded() {
    let manifest = parse_market_manifest(json!({
        "version": 1,
        "updated_at": "2026-10-26T12:51:20.000Z",
        "scripts": [
            {
                "id": "snake",
                "name": "Snake",
                "version": "1.0.0",
                "script_url": "https://example.com/snake.js",
                "updated_at": " 2026-10-20T08:00:00Z ",
            },
            {
                "id": "camel",
                "name": "Camel",
                "version": "1.0.0",
                "script_url": "https://example.com/camel.js",
                "updatedAt": "2026-09-01",
            },
            {
                "id": "legacy",
                "name": "Legacy",
                "version": "1.0.0",
                "script_url": "https://example.com/legacy.js",
            },
        ],
    }))
    .unwrap();

    let updated: Vec<&str> = manifest
        .scripts
        .iter()
        .map(|script| script.updated_at.as_str())
        .collect();
    assert_eq!(updated, vec!["2026-10-20T08:00:00Z", "2026-09-01", ""]);

    let temp = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        temp.path().join("user"),
        temp.path().join("user_scripts.json"),
    );
    let payload = market_scripts_payload(&manager, &manifest, "ok", "");
    let scripts = payload["scripts"].as_array().unwrap();
    assert_eq!(scripts[0]["updatedAt"], "2026-10-20T08:00:00Z");
    assert_eq!(scripts[1]["updatedAt"], "2026-09-01");
    assert_eq!(scripts[2]["updatedAt"], "");
}
