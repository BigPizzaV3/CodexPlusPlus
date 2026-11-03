use std::io::Write;

use codex_plus_core::extension_discovery::{
    CommunityStore, CuratedFilter, Endpoints, PinnedPackage, SearchRepo, asset_name, community_entry, extract_package,
    parse_search_page, pick_latest_tag,
};
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SHA_V1: &str = "1111111111111111111111111111111111111111";
const SHA_HEAD: &str = "2222222222222222222222222222222222222222";

fn manifest(id: &str, version: &str) -> Value {
    json!({
        "id": id,
        "name": "字数统计",
        "version": version,
        "description": "统计输入框字数",
        "renderer": "main.js",
        "hooks": { "PreToolUse": [{ "matcher": "^Bash$", "script": "hooks/guard.mjs" }] },
        "permissions": { "routes": ["/settings/get"] },
        "icon": "icon.png",
    })
}

/// 打一个拓展包压缩包；`top` 非空时包在一层目录里（codeload 的格式）。
fn package_zip(top: &str, manifest: &Value) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        let prefix = if top.is_empty() { String::new() } else { format!("{top}/") };
        for (name, body) in [
            ("codexpp.json", serde_json::to_string(manifest).unwrap()),
            ("main.js", "codexPlus.extension(() => {});".to_string()),
            ("hooks/guard.mjs", "export default async () => ({});".to_string()),
        ] {
            writer.start_file(format!("{prefix}{name}"), options).unwrap();
            writer.write_all(body.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }
    buffer.into_inner()
}

fn search_item(repo: &str) -> Value {
    json!({
        "full_name": repo,
        "html_url": format!("https://github.com/{repo}"),
        "owner": { "login": repo.split('/').next().unwrap(), "avatar_url": "https://avatars.example/a.png" },
        "description": "仓库描述",
        "stargazers_count": 42,
        "license": { "spdx_id": "MIT" },
        "default_branch": "main",
        "pushed_at": "2026-10-28T09:00:00Z",
        "archived": false,
        "fork": false,
    })
}

fn endpoints(server: &MockServer) -> Endpoints {
    Endpoints { api: server.uri(), raw: format!("{}/raw", server.uri()), codeload: format!("{}/codeload", server.uri()), token: None }
}

fn manager(root: &std::path::Path) -> UserScriptManager {
    UserScriptManager::new(root.join("builtin"), root.join("user_scripts"), root.join("user_scripts.json"))
}

async fn mount_listing(server: &MockServer, expect_search: u64) {
    Mock::given(method("GET"))
        .and(path("/search/repositories"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 4,
            "items": [
                search_item("alice/codexpp-word-count"),
                search_item("bob/not-an-extension"),
                { "full_name": "carol/archived", "archived": true, "fork": false },
                { "full_name": "dave/forked", "archived": false, "fork": true },
            ],
        })))
        .expect(expect_search)
        .mount(server)
        .await;
    let mut listed_manifest = manifest("word-count", "1.1.0");
    listed_manifest["engines"] = json!({
        "codexpp": format!(">={}", codex_plus_core::extension_semver::codexpp_version()),
    });
    Mock::given(method("GET"))
        .and(path("/raw/alice/codexpp-word-count/main/codexpp.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(listed_manifest))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/raw/bob/not-an-extension/main/codexpp.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(server)
        .await;
}

#[test]
fn search_page_drops_archived_forks_and_unknown_licenses() {
    let mut unknown = search_item("erin/x");
    unknown["license"] = json!({ "spdx_id": "NOASSERTION" });
    let raw = json!({
        "total_count": 3,
        "items": [unknown, { "full_name": "a/b", "archived": true }, { "full_name": "c/d", "fork": true }],
    });
    let (repos, total) = parse_search_page(&raw).unwrap();
    assert_eq!(total, 3);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].repo, "erin/x");
    assert_eq!(repos[0].license, "");
    assert_eq!(repos[0].stars, 42);
}

#[test]
fn latest_tag_prefers_highest_stable_semver() {
    assert_eq!(pick_latest_tag(["v1.2.0", "v1.10.0", "v1.9.9", "latest"]).as_deref(), Some("v1.10.0"));
    assert_eq!(pick_latest_tag(["v2.0.0-beta.1", "v1.0.0"]).as_deref(), Some("v1.0.0"));
    assert_eq!(pick_latest_tag(["v2.0.0-beta.1", "v2.0.0-beta.2"]).as_deref(), Some("v2.0.0-beta.2"));
    assert_eq!(pick_latest_tag(["1.0.0", "release"]), None);
}

#[test]
fn community_entry_reads_manifest_and_flags_newer_engines() {
    let (repos, _) = parse_search_page(&json!({ "items": [search_item("alice/word-count")] })).unwrap();
    let repo: &SearchRepo = &repos[0];
    let entry = community_entry(repo, "https://raw.example", &manifest("word-count", "1.0.0")).unwrap();
    assert_eq!(entry.id, "word-count");
    assert_eq!(entry.description, "统计输入框字数");
    assert_eq!(entry.license, "MIT");
    assert!(entry.compatible);
    assert_eq!(entry.icon, "https://raw.example/alice/word-count/main/icon.png");
    assert_eq!(entry.permissions["hooks"][0]["event"], "PreToolUse");
    assert_eq!(entry.permissions["hooks"][0]["matcher"], "^Bash$");
    assert_eq!(entry.permissions["routes"][0], "/settings/get");
    assert_eq!(entry.permissions["renderer"][0], "main.js");

    let mut newer = manifest("word-count", "2.0.0");
    newer["engines"] = json!({ "codexpp": 99 });
    assert!(!community_entry(repo, "", &newer).unwrap().compatible);

    let mut bad = manifest("bad id", "1.0.0");
    assert!(community_entry(repo, "", &bad).is_err());
    bad = manifest("ok", "");
    assert!(community_entry(repo, "", &bad).is_err());
}

#[test]
fn community_entry_accepts_semver_ranges_and_preserves_engine_requirement() {
    let (repos, _) = parse_search_page(&json!({ "items": [search_item("alice/word-count")] })).unwrap();
    let current = codex_plus_core::extension_semver::codexpp_version();
    let mut package = manifest("word-count", "1.0.0");
    for (requirement, compatible) in [(format!(">={current}"), true), (format!("<{current}"), false)] {
        package["engines"] = json!({ "codexpp": requirement });
        let entry = community_entry(&repos[0], "", &package).unwrap();
        assert_eq!(entry.compatible, compatible);
        let saved = serde_json::to_value(&entry).unwrap();
        assert_eq!(saved["engine"], requirement);
        let restored: codex_plus_core::extension_discovery::CommunityEntry =
            serde_json::from_value(saved).unwrap();
        assert_eq!(restored, entry, "版本范围经过磁盘缓存后仍保留");
    }
    for invalid in [json!("not-a-version"), json!(true), json!({}), json!(-1)] {
        package["engines"] = json!({ "codexpp": invalid });
        assert!(community_entry(&repos[0], "", &package).is_err());
    }
}

#[test]
fn extract_rejects_unsafe_or_mismatched_packages() {
    let root = tempdir().unwrap();
    // codeload 格式：唯一顶层目录被剥掉。
    let dest = root.path().join("ok");
    extract_package(&package_zip("repo-sha", &manifest("word-count", "1.0.0")), "word-count", &dest).unwrap();
    assert!(dest.join("codexpp.json").is_file());
    assert!(dest.join("hooks/guard.mjs").is_file());

    // 在 Finder 里压缩的附件：包根在一层目录里，旁边多一个 __MACOSX。
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("word-count/codexpp.json", options).unwrap();
        writer.write_all(serde_json::to_string(&manifest("word-count", "1.0.0")).unwrap().as_bytes()).unwrap();
        writer.start_file("__MACOSX/word-count/._codexpp.json", options).unwrap();
        writer.finish().unwrap();
    }
    let finder = root.path().join("finder");
    extract_package(&buffer.into_inner(), "word-count", &finder).unwrap();
    assert!(finder.join("codexpp.json").is_file());
    assert!(!finder.join("__MACOSX").exists());

    let mismatch = extract_package(&package_zip("", &manifest("other", "1.0.0")), "word-count", &root.path().join("m"));
    assert!(format!("{:#}", mismatch.unwrap_err()).contains("不一致"));

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("codexpp.json", options).unwrap();
        writer.write_all(serde_json::to_string(&manifest("word-count", "1.0.0")).unwrap().as_bytes()).unwrap();
        writer.add_symlink("main.js", "/etc/passwd", options).unwrap();
        writer.finish().unwrap();
    }
    let symlink = extract_package(&buffer.into_inner(), "word-count", &root.path().join("s"));
    assert!(format!("{:#}", symlink.unwrap_err()).contains("软链"));

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        writer.start_file("README.md", zip::write::SimpleFileOptions::default()).unwrap();
        writer.finish().unwrap();
    }
    let missing = extract_package(&buffer.into_inner(), "word-count", &root.path().join("n"));
    assert!(format!("{:#}", missing.unwrap_err()).contains("codexpp.json"));
}

#[tokio::test]
async fn listing_is_cached_and_falls_back_when_github_fails() {
    let root = tempdir().unwrap();
    let server = MockServer::start().await;
    mount_listing(&server, 1).await;
    let store = CommunityStore::new(root.path());
    let manager = manager(root.path());

    let first = store.list(&endpoints(&server), &manager, false).await;
    let entries = first["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{first}");
    assert_eq!(entries[0]["repo"], "alice/codexpp-word-count");
    assert_eq!(entries[0]["installed"], Value::Null);
    // 缓存未过期，第二次不再搜索（expect(1) 在 server 释放时校验）。
    let second = store.list(&endpoints(&server), &manager, false).await;
    assert_eq!(second["entries"], first["entries"]);
    assert_eq!(second["stale"], false);
    // 本机就是源码（一键发布写回了 repository）：没有安装记录，也算已有，渠道是 local。
    let source = manager.user_dir().join("word-count");
    std::fs::create_dir_all(&source).unwrap();
    let mut local = manifest("word-count", "1.2.0");
    local["repository"] = json!("https://github.com/Alice/codexpp-word-count.git");
    std::fs::write(source.join("codexpp.json"), serde_json::to_string(&local).unwrap()).unwrap();
    let own = store.list(&endpoints(&server), &manager, false).await;
    assert_eq!(own["entries"][0]["installed"]["channel"], "local", "{own}");
    assert_eq!(own["entries"][0]["installed"]["version"], "1.2.0");
    // repository 指向别的仓库就不算。
    local["repository"] = json!("bob/codexpp-word-count");
    std::fs::write(source.join("codexpp.json"), serde_json::to_string(&local).unwrap()).unwrap();
    let other = store.list(&endpoints(&server), &manager, false).await;
    assert_eq!(other["entries"][0]["installed"], Value::Null);
    std::fs::remove_file(source.join("codexpp.json")).unwrap();

    // 强制刷新时 GitHub 失败：退回上次的列表并标注。
    let down = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search/repositories"))
        .respond_with(ResponseTemplate::new(403).insert_header("x-ratelimit-remaining", "0"))
        .mount(&down)
        .await;
    let stale = store.list(&endpoints(&down), &manager, true).await;
    assert_eq!(stale["stale"], true);
    assert_eq!(stale["entries"], first["entries"]);
    assert!(stale["message"].as_str().unwrap().contains("限流"), "{stale}");

    // 作者下架 / 删除后从缓存里拿掉（大小写不敏感），不用等缓存过期。
    store.forget_repo("Alice/codexpp-word-count").unwrap();
    let forgotten = store.list(&endpoints(&down), &manager, false).await;
    assert!(forgotten["entries"].as_array().unwrap().is_empty(), "{forgotten}");

    store.set_enabled(false).unwrap();
    let off = store.list(&endpoints(&down), &manager, true).await;
    assert_eq!(off["enabled"], false);
    assert!(off["entries"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn installs_release_asset_pinned_to_tag_commit() {
    let root = tempdir().unwrap();
    let server = MockServer::start().await;
    mount_listing(&server, 1).await;
    let mut release_manifest = manifest("word-count", "1.0.0");
    release_manifest["engines"] = json!({
        "codexpp": format!(">={}", codex_plus_core::extension_semver::codexpp_version()),
    });
    let asset = package_zip("", &release_manifest);
    let digest = format!("sha256:{:x}", Sha256::digest(&asset));
    Mock::given(method("GET"))
        .and(path("/repos/alice/codexpp-word-count/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tag_name": "v1.0.0",
            "assets": [
                { "name": "other.zip", "browser_download_url": format!("{}/assets/other.zip", server.uri()) },
                { "name": asset_name("word-count", "v1.0.0"),
                  "browser_download_url": format!("{}/assets/word-count.zip", server.uri()), "digest": digest },
            ],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/alice/codexpp-word-count/commits/v1.0.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "sha": SHA_V1 })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/assets/word-count.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(asset))
        .expect(1)
        .mount(&server)
        .await;
    let store = CommunityStore::new(root.path());
    let manager = manager(root.path());
    let endpoints = endpoints(&server);
    let listing = store.list(&endpoints, &manager, false).await;
    assert_eq!(listing["entries"][0]["engine"], release_manifest["engines"]["codexpp"]);
    assert_eq!(listing["entries"][0]["compatible"], true);

    // 列表里是默认分支的 1.1.0，实际装的是 release 的 1.0.0：权限按实际版本给。
    let prepared = store.prepare(&endpoints, &manager, "alice/codexpp-word-count").await.unwrap();
    assert_eq!(prepared["version"], "1.0.0");
    assert_eq!(prepared["tag"], "v1.0.0");
    assert_eq!(prepared["sha"], SHA_V1);
    assert_eq!(prepared["fromAsset"], true);
    assert_eq!(prepared["fromDefaultBranch"], false);
    assert_eq!(prepared["conflict"], Value::Null);
    assert_eq!(prepared["permissions"]["hooks"][0]["event"], "PreToolUse");
    // 暂存期间还没进用户目录。
    assert!(!root.path().join("user_scripts/word-count").exists());

    let installed = store.install(&manager, prepared["token"].as_str().unwrap(), false, false).unwrap();
    assert_eq!(installed["hooks"], true);
    assert!(root.path().join("user_scripts/word-count/codexpp.json").is_file());
    let config = manager.load_config();
    assert_eq!(config.scripts.get("user:word-count"), Some(&false), "默认不启用");
    let state = store.load_state();
    let record = &state.installs["word-count"];
    assert_eq!(record.repo, "alice/codexpp-word-count");
    assert_eq!(record.sha, SHA_V1);
    assert!(record.from_asset);
    // 暂存令牌用过即失效。
    assert!(store.install(&manager, prepared["token"].as_str().unwrap(), false, false).is_err());

    let listed = store.list(&endpoints, &manager, false).await;
    assert_eq!(listed["entries"][0]["installed"]["version"], "1.0.0");
    assert_eq!(listed["entries"][0]["installed"]["sha"], SHA_V1);
}

#[tokio::test]
async fn falls_back_to_default_branch_and_guards_local_conflicts() {
    let root = tempdir().unwrap();
    let server = MockServer::start().await;
    mount_listing(&server, 1).await;
    Mock::given(method("GET"))
        .and(path("/repos/alice/codexpp-word-count/releases/latest"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/alice/codexpp-word-count/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "name": "nightly" }])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/alice/codexpp-word-count/commits/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "sha": SHA_HEAD })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/codeload/alice/codexpp-word-count/zip/{SHA_HEAD}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(package_zip("codexpp-word-count-2222", &manifest("word-count", "1.1.0"))))
        .mount(&server)
        .await;
    let store = CommunityStore::new(root.path());
    let manager = manager(root.path());
    let endpoints = endpoints(&server);
    store.list(&endpoints, &manager, false).await;

    // 本机已有一个手写的同 id 拓展。
    let local = root.path().join("user_scripts/word-count");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("codexpp.json"), serde_json::to_string(&manifest("word-count", "0.1.0")).unwrap()).unwrap();
    std::fs::write(local.join("main.js"), "// 手写版本").unwrap();

    let prepared = store.prepare(&endpoints, &manager, "alice/codexpp-word-count").await.unwrap();
    assert_eq!(prepared["fromDefaultBranch"], true);
    assert_eq!(prepared["tag"], Value::Null);
    assert_eq!(prepared["sha"], SHA_HEAD);
    assert_eq!(prepared["fromAsset"], false);
    assert_eq!(prepared["conflict"]["source"], "local");
    let token = prepared["token"].as_str().unwrap();

    let refused = store.install(&manager, token, true, false).unwrap_err();
    assert!(format!("{refused:#}").contains("确认替换"));
    assert_eq!(std::fs::read_to_string(local.join("main.js")).unwrap(), "// 手写版本");

    let installed = store.install(&manager, token, true, true).unwrap();
    let backup = std::path::PathBuf::from(installed["backup"].as_str().unwrap());
    assert_eq!(std::fs::read_to_string(backup.join("main.js")).unwrap(), "// 手写版本");
    assert_eq!(std::fs::read_to_string(local.join("main.js")).unwrap(), "codexPlus.extension(() => {});");
    assert_eq!(manager.load_config().scripts.get("user:word-count"), Some(&true));
    assert!(store.load_state().installs["word-count"].from_default_branch);
}

#[tokio::test]
async fn rejects_asset_with_wrong_digest_and_invalid_packages() {
    let root = tempdir().unwrap();
    let server = MockServer::start().await;
    mount_listing(&server, 1).await;
    Mock::given(method("GET"))
        .and(path("/repos/alice/codexpp-word-count/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tag_name": "v1.0.0",
            "assets": [{ "name": asset_name("word-count", "v1.0.0"),
                "browser_download_url": format!("{}/assets/word-count.zip", server.uri()),
                "digest": format!("sha256:{}", "0".repeat(64)) }],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/alice/codexpp-word-count/commits/v1.0.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "sha": SHA_V1 })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/assets/word-count.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(package_zip("", &manifest("word-count", "1.0.0"))))
        .mount(&server)
        .await;
    let store = CommunityStore::new(root.path());
    let manager = manager(root.path());
    let endpoints = endpoints(&server);
    store.list(&endpoints, &manager, false).await;
    let error = store.prepare(&endpoints, &manager, "alice/codexpp-word-count").await.unwrap_err();
    assert!(format!("{error:#}").contains("sha256"), "{error:#}");
    assert!(store.prepare(&endpoints, &manager, "../etc").await.is_err());
    assert!(!root.path().join("user_scripts/word-count").exists());
    // 失败的暂存会被清掉。
    let staging = root.path().join(".extension_staging");
    assert!(!staging.exists() || std::fs::read_dir(&staging).unwrap().next().is_none());
}

/// 联网核对真实 GitHub 的搜索与版本解析接口：`cargo test -p codex-plus-core --test all extension_discovery::live -- --ignored`
#[tokio::test]
#[ignore]
async fn live_github_search_and_version_resolution() {
    let endpoints = Endpoints::github();
    let (entries, problems) = codex_plus_core::extension_discovery::search(&endpoints).await.unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    eprintln!("社区拓展 {} 个", entries.len());
    let client = reqwest::Client::builder().user_agent("Codex++ test").build().unwrap();
    let resolved = codex_plus_core::extension_discovery::resolve_version(
        &client,
        &endpoints,
        "BigPizzaV3/CodexPlusPlus",
        "codexplusplus",
        "main",
    )
    .await
    .unwrap();
    eprintln!("{resolved:?}");
    assert_eq!(resolved.sha.len(), 40);
    assert!(resolved.tag.is_some() || resolved.from_default_branch);
}

/// 联网装一次测试仓库 BigPizzaV3/codexpp-community-hello（release v0.1.0 带附件）：
/// `cargo test -p codex-plus-core --test all extension_discovery::live_install -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn live_install_from_real_github() {
    let root = tempdir().unwrap();
    let store = CommunityStore::new(root.path());
    let manager = manager(root.path());
    let endpoints = Endpoints::github();
    let listed = store.list(&endpoints, &manager, true).await;
    eprintln!("列表：{}", serde_json::to_string_pretty(&listed["entries"]).unwrap());
    let repo = "BigPizzaV3/codexpp-community-hello";
    assert!(listed["entries"].as_array().unwrap().iter().any(|entry| entry["repo"] == repo), "{listed}");
    let prepared = store.prepare(&endpoints, &manager, repo).await.unwrap();
    eprintln!("准备：{prepared:#}");
    assert_eq!(prepared["tag"], "v0.1.0");
    assert_eq!(prepared["fromAsset"], true);
    assert_eq!(prepared["permissions"]["hooks"][0]["event"], "PreToolUse");
    let installed = store.install(&manager, prepared["token"].as_str().unwrap(), false, false).unwrap();
    eprintln!("安装：{installed:#}");
    assert!(root.path().join("user_scripts/community-hello/renderer.js").is_file());
    let inventory = manager.inventory().unwrap();
    let item = inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == "user:community-hello").unwrap().clone();
    eprintln!("加载器：{item:#}");
    assert_eq!(item["status"], "disabled");
    let listed = store.list(&endpoints, &manager, false).await;
    assert_eq!(listed["entries"][0]["installed"]["tag"], "v0.1.0");
}


#[tokio::test]
async fn curated_packages_install_pinned_and_lock_out_community_replacements() {
    let root = tempdir().unwrap();
    let server = MockServer::start().await;
    mount_listing(&server, 1).await;
    // 精选版本：只按清单里的提交下载，不查 release / tag。
    Mock::given(method("GET"))
        .and(path(format!("/codeload/BigPizzaV3/codexpp-word-count/zip/{SHA_V1}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(package_zip("codexpp-word-count-1111", &manifest("word-count", "1.0.0"))))
        .expect(1)
        .mount(&server)
        .await;
    let store = CommunityStore::new(root.path());
    let manager = manager(root.path());
    let endpoints = endpoints(&server);
    let pinned = PinnedPackage {
        repo: "BigPizzaV3/codexpp-word-count".into(),
        id: "word-count".into(),
        tag: Some("v1.0.0".into()),
        sha: SHA_V1.to_uppercase(),
        channel: "official".into(),
    };
    let prepared = store.prepare_pinned(&endpoints, &manager, &pinned).await.unwrap();
    assert_eq!(prepared["sha"], SHA_V1);
    assert_eq!(prepared["channel"], "official");
    assert_eq!(prepared["fromAsset"], false);
    store.install(&manager, prepared["token"].as_str().unwrap(), true, false).unwrap();
    let record = &store.load_state().installs["word-count"];
    assert_eq!((record.channel.as_str(), record.sha.as_str()), ("official", SHA_V1));

    // 社区里另一个仓库的同 id 拓展：即使用户点了替换，也不能顶掉官方版本。
    Mock::given(method("GET")).and(path("/repos/alice/codexpp-word-count/releases/latest")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    Mock::given(method("GET")).and(path("/repos/alice/codexpp-word-count/tags")).respond_with(ResponseTemplate::new(200).set_body_json(json!([]))).mount(&server).await;
    Mock::given(method("GET")).and(path("/repos/alice/codexpp-word-count/commits/main")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "sha": SHA_HEAD }))).mount(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/codeload/alice/codexpp-word-count/zip/{SHA_HEAD}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(package_zip("x", &manifest("word-count", "9.9.9"))))
        .mount(&server)
        .await;
    store.list(&endpoints, &manager, false).await;
    let community = store.prepare(&endpoints, &manager, "alice/codexpp-word-count").await.unwrap();
    assert_eq!(community["conflict"]["channel"], "official");
    let refused = store.install(&manager, community["token"].as_str().unwrap(), true, true).unwrap_err();
    assert!(format!("{refused:#}").contains("官方版本"), "{refused:#}");
    assert_eq!(store.load_state().installs["word-count"].repo, "BigPizzaV3/codexpp-word-count");

    // 无效 sha 直接拒绝，不发请求。
    let bad = PinnedPackage { sha: "main".into(), ..pinned.clone() };
    assert!(store.prepare_pinned(&endpoints, &manager, &bad).await.unwrap_err().to_string().contains("sha 无效"));
}

#[tokio::test]
async fn community_listing_honours_blocked_and_curated_repos_and_flags_impostors() {
    let root = tempdir().unwrap();
    let server = MockServer::start().await;
    mount_listing(&server, 1).await;
    let store = CommunityStore::new(root.path());
    let manager = manager(root.path());
    let endpoints = endpoints(&server);
    let ids = |listing: &Value| listing["entries"].as_array().unwrap().iter().map(|entry| entry["repo"].as_str().unwrap().to_string()).collect::<Vec<_>>();

    let plain = store.list(&endpoints, &manager, false).await;
    assert_eq!(ids(&plain), ["alice/codexpp-word-count"]);
    assert_eq!(plain["entries"][0]["warnings"], json!([]));

    // 官方拓展用了同一个 id：社区条目带仿冒警示。
    let mut curated_ids = std::collections::BTreeMap::new();
    curated_ids.insert("word-count".to_string(), ("BigPizzaV3/codexpp-word-count".to_string(), true));
    let impostor = CuratedFilter { curated_ids, official_owners: vec!["bigpizzav3".into()], ..CuratedFilter::default() };
    let flagged = store.list_filtered(&endpoints, &manager, false, &impostor).await;
    assert!(flagged["entries"][0]["warnings"][0].as_str().unwrap().contains("官方拓展同 id"));

    // 下架或已进精选的仓库不显示。
    let blocked = CuratedFilter { blocked: vec!["ALICE/codexpp-word-count".into()], ..CuratedFilter::default() };
    assert!(ids(&store.list_filtered(&endpoints, &manager, false, &blocked).await).is_empty());
    let curated = CuratedFilter { curated_repos: vec!["alice/codexpp-word-count".into()], ..CuratedFilter::default() };
    assert!(ids(&store.list_filtered(&endpoints, &manager, false, &curated).await).is_empty());
}

#[test]
fn self_proclaimed_official_entries_are_flagged() {
    let repo = SearchRepo {
        repo: "mallory/codexpp-tools".into(),
        url: String::new(),
        owner: "mallory".into(),
        owner_avatar: String::new(),
        description: "Codex++ 官方出品的工具集".into(),
        stars: 1,
        license: "MIT".into(),
        default_branch: "main".into(),
        pushed_at: String::new(),
    };
    let mut claimed = manifest("tools", "1.0.0");
    claimed["name"] = json!("Codex++ 官方工具集");
    let entry = community_entry(&repo, "https://raw.example", &claimed).unwrap();
    let filter = CuratedFilter { official_owners: vec!["BigPizzaV3".into()], ..CuratedFilter::default() };
    assert!(filter.warnings(&entry).iter().any(|warning| warning.contains("自称官方")));
    let official = community_entry(&SearchRepo { owner: "bigpizzav3".into(), repo: "BigPizzaV3/codexpp-tools".into(), ..repo }, "https://raw.example", &claimed).unwrap();
    assert!(filter.warnings(&official).is_empty(), "官方账号自己的仓库不算自称");
}
