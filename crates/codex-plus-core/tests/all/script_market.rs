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

#[test]
fn curated_packages_blocked_list_and_official_owner_check() {
    let sha = "a".repeat(40);
    let manifest = parse_market_manifest(json!({
        "version": 2,
        "scripts": [
            { "id": "old-script", "name": "老脚本", "version": "1.0.0", "script_url": "https://example.com/a.js" },
            { "kind": "package", "id": "word-count", "name": "字数统计", "repo": "BigPizzaV3/codexpp-word-count", "tag": "v1.0.0", "sha": sha, "official": true },
            { "kind": "package", "id": "fake-official", "name": "假官方", "repo": "mallory/codexpp-fake", "sha": sha, "official": true },
            { "kind": "package", "id": "bad-sha", "name": "坏 sha", "repo": "alice/x", "sha": "main" },
        ],
        "packages": [{ "id": "notes", "name": "便签", "repo": "alice/codexpp-notes", "sha": sha.to_uppercase() }],
        "blocked": ["evil/codexpp-miner", "not-a-repo"],
    }))
    .unwrap();
    assert_eq!(manifest.scripts.len(), 1, "拓展包条目不混进单文件脚本");
    let ids = manifest.packages.iter().map(|package| package.id.as_str()).collect::<Vec<_>>();
    assert_eq!(ids, ["word-count", "fake-official", "notes"], "sha 无效的条目丢弃；packages 数组也认");
    let find = |id: &str| manifest.packages.iter().find(|package| package.id == id).unwrap();
    assert!(find("word-count").is_official(), "清单声明官方且仓库属于官方账号");
    assert_eq!(find("word-count").channel(), "official");
    assert!(!find("fake-official").is_official(), "清单声明官方但仓库不属于官方账号：只算精选");
    assert_eq!(find("fake-official").channel(), "curated");
    assert_eq!(find("notes").sha, sha, "sha 统一小写");
    assert_eq!(manifest.blocked, ["evil/codexpp-miner"]);

    let filter = codex_plus_core::script_market::curated_filter(&manifest);
    assert!(filter.is_blocked("Evil/CodexPP-Miner"), "下架名单不分大小写");
    assert_eq!(filter.curated_ids["word-count"], ("BigPizzaV3/codexpp-word-count".to_string(), true));
    let pinned = find("word-count").pinned();
    assert_eq!((pinned.repo.as_str(), pinned.sha.as_str(), pinned.channel.as_str()), ("BigPizzaV3/codexpp-word-count", sha.as_str(), "official"));
}
