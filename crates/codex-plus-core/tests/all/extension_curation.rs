//! 精选维护：拼条目、只改 packages 段地写清单、差异截断，以及用假 gh 跑通的读写市场仓库。
use codex_plus_core::extension_curation::{CuratedEntry, Curator, can_be_official, curated_entry, trim_compare, upsert_package, utc_iso};
use codex_plus_core::extension_discovery::CommunityEntry;
use codex_plus_core::extension_publish::Publisher;
use codex_plus_core::script_market::parse_market_manifest;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

/// 和线上清单一样的排版：两空格缩进，脚本条目的键不是字母序。
const INDEX: &str = r#"{
  "version": 1,
  "updated_at": "2026-10-26T12:51:20.000Z",
  "scripts": [
    {
      "id": "relay-balance",
      "name": "Relay \"Balance\" {x}",
      "version": "0.3.7",
      "script_url": "https://example.com/a.js"
    }
  ],
  "blocked": []
}
"#;

fn entry(repo: &str, id: &str, sha: &str) -> CuratedEntry {
    let meta = json!({ "repo": repo, "id": id, "version": "1.0.0", "tag": "v1.0.0", "sha": sha });
    curated_entry(&meta, &json!({ "name": "字数统计", "description": "统计字数", "keywords": ["text"] }), None, "2026-10-30T00:00:00.000Z", false).unwrap()
}

#[test]
fn builds_entries_from_staged_meta() {
    let community = CommunityEntry {
        repo: "BigPizzaV3/codexpp-word-count".into(),
        url: String::new(),
        owner: "BigPizzaV3".into(),
        owner_avatar: String::new(),
        description: "仓库描述".into(),
        stars: 0,
        license: String::new(),
        default_branch: "main".into(),
        pushed_at: String::new(),
        id: "word-count".into(),
        name: "旧名字".into(),
        version: "0.9.0".into(),
        author: String::new(),
        homepage: String::new(),
        icon: "https://raw.githubusercontent.com/x/icon.png".into(),
        keywords: vec![],
        engine: json!(1),
        compatible: true,
        permissions: Value::Null,
    };
    let meta = json!({ "repo": "BigPizzaV3/codexpp-word-count", "id": "word-count", "version": "1.0.0", "tag": "v1.0.0", "sha": SHA_B });
    // 审核通过只算精选：官方账号的仓库也要明确勾选才标官方。
    let curated = curated_entry(&meta, &json!({ "name": "字数统计" }), Some(&community), "2026-10-30T00:00:00.000Z", false).unwrap();
    assert!(!curated.official);
    let official = curated_entry(&meta, &json!({ "name": "字数统计" }), Some(&community), "2026-10-30T00:00:00.000Z", true).unwrap();
    assert!(official.official && can_be_official("bigpizzav3/x"));
    assert_eq!(official.sha, SHA_B.to_lowercase());
    assert_eq!(official.name, "字数统计", "清单优先");
    assert_eq!(official.description, "仓库描述", "清单没写时用社区条目");
    assert_eq!(official.icon, "https://raw.githubusercontent.com/x/icon.png");
    assert_eq!(official.author, "BigPizzaV3");
    assert_eq!(official.homepage, "https://github.com/BigPizzaV3/codexpp-word-count");

    let other = curated_entry(&json!({ "repo": "alice/codexpp-word-count", "id": "word-count", "sha": SHA_A }), &json!({ "keywords": ["text"] }), None, "", true).unwrap();
    assert!(!other.official, "非官方账号勾了也不算");
    assert_eq!(other.tags, ["text"]);

    let bad = json!({ "repo": "alice/x", "id": "x", "sha": "123" });
    assert!(curated_entry(&bad, &json!({}), None, "", false).is_err());
}

#[test]
fn upsert_only_touches_packages_and_updated_at() {
    let (first, replaced) = upsert_package(INDEX, &entry("alice/codexpp-word-count", "word-count", SHA_A), "2026-10-30T08:00:00.000Z").unwrap();
    assert!(!replaced);
    // 脚本段原文不变（键序、转义都保留）。
    let scripts_before = &INDEX[INDEX.find("\"scripts\"").unwrap()..INDEX.find("\"blocked\"").unwrap()];
    assert!(first.contains(scripts_before), "{first}");
    assert!(first.contains("\"updated_at\": \"2026-10-30T08:00:00.000Z\""));
    assert!(first.contains("\n  \"packages\": [\n    {\n      \"kind\": \"package\",\n      \"id\": \"word-count\""), "{first}");
    let parsed = parse_market_manifest(serde_json::from_str(&first).unwrap()).unwrap();
    assert_eq!(parsed.scripts.len(), 1);
    assert_eq!(parsed.packages.len(), 1);
    assert_eq!(parsed.packages[0].sha, SHA_A);
    assert_eq!(parsed.packages[0].tag.as_deref(), Some("v1.0.0"));

    // 同仓库再收录：替换，保留手加的字段；不同仓库追加。
    let with_note = first.replace("\"official\": false", "\"official\": false,\n      \"note\": \"人工备注\"");
    let mut newer = entry("Alice/codexpp-word-count", "word-count", SHA_B);
    newer.version = "1.1.0".into();
    let (second, replaced) = upsert_package(&with_note, &newer, "2026-10-31T00:00:00.000Z").unwrap();
    assert!(replaced);
    let value: Value = serde_json::from_str(&second).unwrap();
    assert_eq!(value["packages"].as_array().unwrap().len(), 1);
    assert_eq!(value["packages"][0]["sha"], SHA_B.to_lowercase());
    assert_eq!(value["packages"][0]["version"], "1.1.0");
    assert_eq!(value["packages"][0]["note"], "人工备注");
    let (third, _) = upsert_package(&second, &entry("bob/codexpp-timer", "timer", SHA_A), "x").unwrap();
    assert_eq!(serde_json::from_str::<Value>(&third).unwrap()["packages"].as_array().unwrap().len(), 2);

    // 同 id 被别的仓库占了：拒绝。
    let taken = upsert_package(&third, &entry("mallory/codexpp-word-count", "word-count", SHA_A), "x").unwrap_err();
    assert!(taken.to_string().contains("已有 id 为 word-count"), "{taken}");
    // 拓展包混在 scripts 里的老写法：不自动改。
    let mixed = INDEX.replace("\"blocked\": []", "\"blocked\": [],\n  \"packages\": []").replace(
        "\"script_url\": \"https://example.com/a.js\"",
        "\"script_url\": \"https://example.com/a.js\"\n    },\n    { \"kind\": \"package\", \"id\": \"word-count\", \"repo\": \"alice/codexpp-word-count\", \"sha\": \"x\"",
    );
    assert!(upsert_package(&mixed, &entry("alice/codexpp-word-count", "word-count", SHA_A), "x").unwrap_err().to_string().contains("scripts"));
    // 没有 updated_at、空对象也能写。
    let (empty, _) = upsert_package("{}", &entry("alice/a", "a", SHA_A), "x").unwrap();
    assert_eq!(serde_json::from_str::<Value>(&empty).unwrap()["packages"][0]["id"], "a");
}

#[test]
fn formats_utc_and_trims_large_diffs() {
    assert_eq!(utc_iso(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(utc_iso(1_793_404_800 + 3_661), "2026-10-31T01:01:01.000Z");
    assert_eq!(utc_iso(951_782_400), "2000-02-29T00:00:00.000Z");

    let huge = "+中".repeat(20_000);
    let raw = json!({
        "total_commits": 3,
        "html_url": "https://github.com/a/b/compare/x...y",
        "files": [
            { "filename": "main.js", "status": "modified", "additions": 20000, "deletions": 0, "patch": huge },
            { "filename": "logo.png", "status": "added", "additions": 0, "deletions": 0, "changes": 0 },
        ],
    });
    let trimmed = trim_compare(&raw);
    assert_eq!(trimmed["commits"], 3);
    assert_eq!(trimmed["files"][0]["truncated"], true);
    assert!(trimmed["files"][0]["patch"].as_str().unwrap().len() <= 24 * 1024);
    assert_eq!(trimmed["files"][1]["truncated"], false);
}

/// 假 gh：账号 tester；市场仓库的写权限看 PUSH 文件；contents 接口读写本地文件。
fn fake_gh(dir: &Path) -> std::path::PathBuf {
    let script = format!(
        r#"#!/bin/bash
DIR='{dir}'
echo "$*" >> "$DIR/gh.log"
case "$1 $2" in
  "auth status") exit 0;;
  "auth token") echo fake-token; exit 0;;
  "api user") echo tester; exit 0;;
esac
if [ "$1" = api ] && [ "$2" = "repos/BigPizzaV3/CodexPlusPlusScriptMarket" ]; then cat "$DIR/PUSH"; exit 0; fi
if [ "$1" = api ] && [ "$2" = "repos/BigPizzaV3/CodexPlusPlusScriptMarket/contents/index.json?ref=main" ]; then
  printf '{{"sha":"%s","content":"%s"}}' "$(cat "$DIR/index.sha")" "$(base64 < "$DIR/index.json" | tr -d '\n')"; exit 0
fi
if [ "$1" = api ] && [ "$2" = -X ] && [ "$3" = PUT ]; then
  shift 4
  while [ $# -gt 0 ]; do
    case "$2" in
      content=*) printf '%s' "${{2#content=}}" | base64 -d > "$DIR/index.json";;
      sha=*) [ "${{2#sha=}}" = "$(cat "$DIR/index.sha")" ] || {{ echo "sha mismatch" >&2; exit 1; }};;
      message=*) printf '%s\n' "${{2#message=}}" >> "$DIR/messages";;
    esac
    shift 2
  done
  echo next > "$DIR/index.sha"
  echo '{{"commit":{{"html_url":"https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket/commit/abc"}}}}'; exit 0
fi
exit 1
"#,
        dir = dir.display(),
    );
    let path = dir.join("gh");
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

#[cfg(unix)]
#[test]
fn maintainers_read_and_write_the_market_index() {
    let Some(git) = codex_plus_core::extension_publish::locate_tool("git") else {
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    fs::write(dir.join("index.json"), INDEX).unwrap();
    fs::write(dir.join("index.sha"), "first").unwrap();
    fs::write(dir.join("PUSH"), "false").unwrap();
    let curator = Curator::new(Publisher {
        gh: fake_gh(dir),
        git,
        config_dir: dir.to_path_buf(),
        remote_template: String::new(),
        git_env: Vec::new(),
    });

    let status = curator.status().unwrap();
    assert_eq!(status["account"], "tester");
    assert_eq!(status["maintainer"], false);
    let refused = curator.publish_entry(&entry("alice/codexpp-word-count", "word-count", SHA_A), "x").unwrap_err();
    assert!(refused.to_string().contains("没有写权限"), "{refused}");
    assert_eq!(fs::read_to_string(dir.join("index.json")).unwrap(), INDEX, "没权限不写");

    fs::write(dir.join("PUSH"), "true").unwrap();
    assert_eq!(curator.token().as_deref(), Some("fake-token"));
    let (text, sha) = curator.read_index().unwrap();
    assert_eq!(text, INDEX);
    assert_eq!(sha, "first");

    let result = curator.publish_entry(&entry("alice/codexpp-word-count", "word-count", SHA_A), "2026-10-30T08:00:00.000Z").unwrap();
    assert_eq!(result["replaced"], false);
    assert_eq!(result["commitUrl"], "https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket/commit/abc");
    let written = fs::read_to_string(dir.join("index.json")).unwrap();
    assert_eq!(result["index"], written.as_str());
    assert_eq!(parse_market_manifest(serde_json::from_str(&written).unwrap()).unwrap().packages[0].repo, "alice/codexpp-word-count");
    let messages = fs::read_to_string(dir.join("messages")).unwrap();
    assert!(messages.contains("curate: 收录 word-count v1.0.0（alice/codexpp-word-count@aaaaaaa）"), "{messages}");

    // 再收录同仓库：基于新的文件 sha 写入，提交说明是「更新」。
    let updated = curator.publish_entry(&entry("alice/codexpp-word-count", "word-count", SHA_B), "x").unwrap();
    assert_eq!(updated["replaced"], true);
    assert!(fs::read_to_string(dir.join("messages")).unwrap().contains("curate: 更新 word-count"));
}

/// 真实只读：本机 gh 是否是市场仓库的维护者、线上清单读得出、写入算法在真实清单上只动 packages 段。
/// 不写市场仓库。手动跑：`cargo test -p codex-plus-core --test all extension_curation::live -- --ignored --nocapture`
#[test]
#[ignore]
fn live_reads_the_real_market_index_without_writing() {
    let curator = Curator::new(Publisher::new(tempfile::tempdir().unwrap().path()).expect("需要本机 gh 与 git"));
    let status = curator.status().unwrap();
    eprintln!("维护者状态：{status}");
    let (text, sha) = curator.read_index().unwrap();
    eprintln!("线上清单 {} 字节，文件 sha {sha}", text.len());
    let before = parse_market_manifest(serde_json::from_str(&text).unwrap()).unwrap();
    let (next, _) = upsert_package(&text, &entry("alice/codexpp-live-check", "live-check", SHA_A), "2026-10-30T00:00:00.000Z").unwrap();
    let after = parse_market_manifest(serde_json::from_str(&next).unwrap()).unwrap();
    assert_eq!(after.scripts, before.scripts, "脚本条目不变");
    assert_eq!(after.blocked, before.blocked);
    assert_eq!(after.packages.len(), before.packages.len() + 1);
    // 除了 packages 段和 updated_at，原文逐字保留。
    let strip = |text: &str| text.lines().filter(|line| !line.contains("\"updated_at\"")).collect::<Vec<_>>().join("\n");
    let head = next.find("\n  \"packages\"").unwrap_or(next.len());
    assert!(strip(&text).trim_end().trim_end_matches('}').trim_end().starts_with(strip(&next[..head]).trim_end().trim_end_matches(',')), "packages 之前的原文不变");
}

/// 真实只读：按审核框的流程下载并暂存一个真实仓库的最新版本，列文件、扫密钥、拼条目，再取一次真实差异。
#[tokio::test]
#[ignore]
async fn live_reviews_a_real_repository_without_writing() {
    use codex_plus_core::extension_curation::{inspect_staged, staged_manifest};
    use codex_plus_core::extension_discovery::{CommunityStore, Endpoints};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let curator = Curator::new(Publisher::new(root).expect("需要本机 gh 与 git"));
    let endpoints = Endpoints::github().with_token(curator.token());
    let store = CommunityStore::new(root);
    let manager = codex_plus_core::user_scripts::UserScriptManager::new(root.join("builtin"), root.join("user_scripts"), root.join("user_scripts.json"));
    let repo = "BigPizzaV3/codexpp-ice-tea-voice-button";
    let prepared = store.prepare(&endpoints, &manager, repo).await.unwrap();
    let (dir, meta) = store.staged(prepared["token"].as_str().unwrap()).unwrap();
    let inspection = inspect_staged(&dir);
    let entry = curated_entry(&meta, &staged_manifest(&dir).unwrap(), store.cached_entry(repo).as_ref(), "2026-10-30T00:00:00.000Z", false).unwrap();
    eprintln!("暂存：{meta}\n检查：{inspection}\n条目：{}", serde_json::to_string_pretty(&entry).unwrap());
    assert!(!entry.official && entry.sha.len() == 40);
    assert!(inspection["files"].as_array().unwrap().iter().any(|file| file["path"] == "codexpp.json"));
    let history = curator.publisher.clone();
    let first_commit = std::process::Command::new(&history.gh).args(["api", &format!("repos/{repo}/commits?per_page=5"), "--jq", ".[-1].sha"]).output().unwrap();
    let base = String::from_utf8_lossy(&first_commit.stdout).trim().to_string();
    if base.len() == 40 && base != entry.sha {
        let diff = curator.compare(repo, &base, &entry.sha).unwrap();
        eprintln!("差异：{} 个提交，{} 个文件", diff["commits"], diff["files"].as_array().unwrap().len());
    }
    store.discard(prepared["token"].as_str().unwrap());
}
