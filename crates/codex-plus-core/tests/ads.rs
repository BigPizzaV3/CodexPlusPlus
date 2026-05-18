use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use codex_plus_core::ads::{
    DEFAULT_AD_LIST_URLS, cache_busted_ad_url, fetch_ad_list_from_urls, normalize_ad_payload,
};
use serde_json::json;

#[test]
fn default_ad_urls_match_legacy_helper_sources() {
    assert_eq!(
        DEFAULT_AD_LIST_URLS,
        [
            "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json",
            "https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json",
        ]
    );
}

#[test]
fn cache_busted_ad_url_appends_version_query_to_plain_url() {
    assert_eq!(
        cache_busted_ad_url("https://example.test/ads.json", 1779035222758),
        "https://example.test/ads.json?v=1779035222758"
    );
}

#[test]
fn cache_busted_ad_url_preserves_existing_query() {
    assert_eq!(
        cache_busted_ad_url("https://example.test/ads.json?source=cdn", 1779035222758),
        "https://example.test/ads.json?source=cdn&v=1779035222758"
    );
}

#[test]
fn normalizes_remote_ads_for_plugin_and_manager_rendering() {
    let payload = normalize_ad_payload(json!({
        "version": 1,
        "ads": [
            {
                "id": "sponsor",
                "type": "sponsor",
                "title": "赞助商",
                "description": "推荐内容",
                "url": "https://example.test",
                "highlights": ["稳定"]
            },
            {
                "id": "normal",
                "type": "normal",
                "title": "普通推荐",
                "description": "推荐内容",
                "url": "https://example.org"
            },
            {
                "id": "broken",
                "type": "normal",
                "title": "",
                "description": "missing title",
                "url": "https://example.invalid"
            }
        ]
    }));

    assert_eq!(payload["version"], json!(1));
    assert_eq!(payload["ads"].as_array().unwrap().len(), 2);
    assert_eq!(payload["ads"][0]["type"], json!("sponsor"));
    assert_eq!(payload["ads"][1]["type"], json!("normal"));
}

#[tokio::test]
async fn fetch_ad_list_tries_backup_url_when_primary_fails() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 1024];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            if request.starts_with("GET /primary.json?") {
                stream
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            } else {
                assert!(request.starts_with("GET /backup.json?"), "{request}");
                let body = json!({
                    "version": 1,
                    "ads": [{
                        "id": "backup-ad",
                        "type": "normal",
                        "title": "Backup",
                        "description": "Loaded from backup",
                        "url": "https://example.test",
                        "highlights": []
                    }]
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        }
    });

    let payload = fetch_ad_list_from_urls(&[
        format!("http://127.0.0.1:{port}/primary.json"),
        format!("http://127.0.0.1:{port}/backup.json"),
    ])
    .await
    .unwrap();
    thread.join().unwrap();

    assert_eq!(payload["ads"][0]["id"], json!("backup-ad"));
}
