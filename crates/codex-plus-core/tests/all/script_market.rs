use codex_plus_core::script_market::{download_script, fetch_market_manifest};
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
