//! 拓展一键发布：发布前检查、版本号、README 与打包，以及用假 gh + 本地裸仓库跑通的完整发布。

use codex_plus_core::extension_publish::{Publisher, PublishOptions, build_archive, bump_version, check, generate_readme};
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn manager(root: &Path) -> UserScriptManager {
    UserScriptManager::new(root.join("builtin"), root.join("user_scripts"), root.join("user_scripts.json"))
}

fn write_package(root: &Path, id: &str, manifest: Value, files: &[(&str, &str)]) {
    let dir = root.join("user_scripts").join(id);
    fs::create_dir_all(&dir).unwrap();
    let mut manifest = manifest;
    manifest["id"] = json!(id);
    fs::write(dir.join("codexpp.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    for (path, content) in files {
        let target = dir.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, content).unwrap();
    }
}

fn good_manifest() -> Value {
    json!({ "name": "字数统计", "description": "统计输入框字数", "version": "1.0.0", "renderer": "main.js",
        "hooks": { "PreToolUse": [{ "matcher": "^Bash$", "script": "hooks/guard.mjs" }] } })
}

#[test]
fn checks_manifest_secrets_and_package_shape() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let manager = manager(root);
    write_package(root, "clean", good_manifest(), &[("main.js", "codexPlus.extension(() => {});"), ("hooks/guard.mjs", "export default () => null;")]);
    let report = check(&manager, root, "clean").unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(!report.has_readme && !report.has_license);
    assert_eq!(report.suggested_repo_name, "codexpp-clean");

    let fake_key = format!("const KEY = \"sk-{}\";", "a".repeat(40));
    write_package(root, "leaky", json!({ "name": "漏密钥", "description": "x", "version": "1.0", "config": { "token": { "type": "string", "secret": true, "default": "abc" } } }), &[
        ("main.js", fake_key.as_str()),
        (".env", "TOKEN=1"),
        ("certs/server.pem", "x"),
    ]);
    let report = check(&manager, root, "leaky").unwrap();
    let errors = report.errors.join("\n");
    assert!(errors.contains("不是 x.y.z"), "{errors}");
    assert!(errors.contains("配置项 token 标了 secret"), "{errors}");
    assert!(errors.contains("疑似密钥"), "{errors}");
    let hits = report.secrets.iter().map(|hit| (hit.file.as_str(), hit.line)).collect::<Vec<_>>();
    assert!(hits.contains(&("main.js", 1)) && hits.contains(&(".env", 0)) && hits.contains(&("certs/server.pem", 0)), "{hits:?}");
    assert!(!serde_json::to_string(&report).unwrap().contains("aaaaaaaa"), "不回传命中内容");

    // dsh 配套包、单文件脚本不能发布。
    write_package(root, "dsh-companion", good_manifest(), &[("dsh-bridge.json", "{}")]);
    assert!(check(&manager, root, "dsh-companion").unwrap_err().to_string().contains("dsh"));
    fs::write(root.join("user_scripts/single.js"), "// 单文件").unwrap();
    assert!(check(&manager, root, "single").is_err());
}

#[test]
fn bumps_versions_and_builds_readme_and_archive() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let manager = manager(root);
    write_package(root, "word-count", good_manifest(), &[("main.js", "x"), ("hooks/guard.mjs", "y")]);
    assert_eq!(bump_version(&manager, "word-count", "patch").unwrap(), "1.0.1");
    assert_eq!(bump_version(&manager, "word-count", "minor").unwrap(), "1.1.0");
    let manifest: Value = serde_json::from_str(&fs::read_to_string(root.join("user_scripts/word-count/codexpp.json")).unwrap()).unwrap();
    assert_eq!(manifest["version"], "1.1.0");

    let readme = generate_readme(&manifest, "alice/codexpp-word-count");
    assert!(readme.starts_with("# 字数统计") && readme.contains("PreToolUse") && readme.contains("alice/codexpp-word-count/releases"));

    let archive = root.join("pkg.zip");
    build_archive(&root.join("user_scripts/word-count"), &archive).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
    let mut names = (0..zip.len()).map(|index| zip.by_index(index).unwrap().name().to_string()).collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["codexpp.json", "hooks/guard.mjs", "main.js"], "压缩包根目录就是包根目录");
}

/// 假 gh：记录参数；repo create 建本地裸仓库，release / topic 只记录，tag 查询读裸仓库。
fn fake_gh(dir: &Path, remotes: &Path, log: &Path) -> std::path::PathBuf {
    let script = format!(
        r#"#!/bin/bash
REMOTES='{remotes}'
echo "$*" >> '{log}'
case "$1 $2" in
  "auth status") exit 0;;
  "api user") echo tester; exit 0;;
  "repo view") [ -d "$REMOTES/$3.git" ] && echo '{{"name":"x"}}' && exit 0; exit 1;;
  "repo create") mkdir -p "$REMOTES/$3.git" && git init -q --bare --initial-branch=main "$REMOTES/$3.git" && exit 0;;
  "release create") [ -f "$4" ] && exit 0; echo "missing asset $4" >&2; exit 1;;
  "repo edit") exit 0;;
  "auth git-credential") exit 0;;
esac
if [ "$1" = api ]; then
  case "$2" in
    licenses/*) printf 'MIT License\n\nCopyright (c) [year] [fullname]\n'; exit 0;;
    repos/*/git/ref/tags/*)
      repo=$(echo "$2" | sed -E 's#repos/(.*)/git/ref/tags/.*#\1#'); tag=${{2##*/tags/}}
      git --git-dir="$REMOTES/$repo.git" rev-parse -q --verify "refs/tags/$tag" >/dev/null && exit 0; exit 1;;
  esac
fi
exit 1
"#,
        remotes = remotes.display(),
        log = log.display(),
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
fn publishes_end_to_end_against_a_local_remote() {
    let Some(git) = codex_plus_core::extension_publish::locate_tool("git") else {
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let remotes = root.join("remotes");
    fs::create_dir_all(&remotes).unwrap();
    let log = root.join("gh.log");
    let gitconfig = root.join("gitconfig");
    fs::write(&gitconfig, "[user]\n\tname = Tester\n\temail = tester@example.com\n[init]\n\tdefaultBranch = main\n").unwrap();
    let manager = manager(root);
    write_package(root, "word-count", good_manifest(), &[("main.js", "codexPlus.extension(() => {});"), ("hooks/guard.mjs", "export default () => null;")]);
    let publisher = Publisher {
        gh: fake_gh(root, &remotes, &log),
        git,
        config_dir: root.to_path_buf(),
        remote_template: format!("{}/{{repo}}.git", remotes.display()),
        git_env: vec![("GIT_CONFIG_GLOBAL".into(), gitconfig.to_string_lossy().to_string()), ("GIT_CONFIG_NOSYSTEM".into(), "1".into())],
    };
    let options = PublishOptions { repo_name: String::new(), license: Some("mit".into()), generate_readme: true };

    let first = publisher.publish(&manager, "word-count", &options);
    assert_eq!(first["ok"], true, "{first:#}");
    assert_eq!(first["repo"], "tester/codexpp-word-count");
    let steps = first["steps"].as_array().unwrap().iter().map(|step| step["step"].as_str().unwrap()).collect::<Vec<_>>();
    assert!(steps.contains(&"创建仓库") && steps.contains(&"推送") && steps.contains(&"创建 release") && steps.contains(&"添加 topic"), "{steps:?}");
    let bare = remotes.join("tester/codexpp-word-count.git");
    let show = |spec: &str| {
        let output = std::process::Command::new("git").args(["--git-dir", &bare.to_string_lossy(), "show", spec]).output().unwrap();
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    assert!(show("v1.0.0:codexpp.json").contains("\"repository\": \"tester/codexpp-word-count\""), "推送了带 repository 的清单和 tag");
    assert!(show("v1.0.0:README.md").starts_with("# 字数统计"));
    assert!(show("v1.0.0:LICENSE").contains("Copyright (c) 20") && show("v1.0.0:LICENSE").contains("tester"));
    let gh_log = fs::read_to_string(&log).unwrap();
    assert!(gh_log.contains("release create v1.0.0") && gh_log.contains("word-count-v1.0.0.zip"), "{gh_log}");
    assert!(gh_log.contains("--add-topic codexpp-extension"));
    // 本地包也写回了 repository / README / LICENSE，下次检查认得出已发布的仓库。
    let report = check(&manager, root, "word-count").unwrap();
    assert_eq!(report.published_repo.as_deref(), Some("tester/codexpp-word-count"));
    assert!(report.has_readme && report.has_license);

    // 同一个版本再发：远端已有 tag，停在检查版本。
    let again = publisher.publish(&manager, "word-count", &options);
    assert_eq!(again["ok"], false);
    assert!(again["error"].as_str().unwrap().contains("已经有 v1.0.0"), "{again:#}");

    // 升版本后再发：走「取回仓库」，不再新建。
    bump_version(&manager, "word-count", "patch").unwrap();
    let second = publisher.publish(&manager, "word-count", &options);
    assert_eq!(second["ok"], true, "{second:#}");
    let steps = second["steps"].as_array().unwrap().iter().map(|step| step["step"].as_str().unwrap()).collect::<Vec<_>>();
    assert!(steps.contains(&"取回仓库") && !steps.contains(&"创建仓库"), "{steps:?}");
    assert!(show("v1.0.1:codexpp.json").contains("\"version\": \"1.0.1\""));
}

/// 真实发布：用本机已登录的 gh 发布到 GitHub，再按社区栏的流程从 GitHub 装回来。
/// 会建公开仓库，只手动跑：`cargo test -p codex-plus-core --test all extension_publish::live -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn live_publish_and_install_from_github() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let manager = manager(root);
    write_package(root, "publish-smoke-test", json!({
        "name": "发布流程测试",
        "description": "Codex++ 一键发布的实测包，可以删除",
        "version": "0.1.0",
        "author": "BigPizzaV3",
        "renderer": "main.js",
    }), &[("main.js", "codexPlus.extension((ctx) => { ctx.registerCommand({ name: \"publish-smoke\", description: \"发布流程测试\", run: (args, { toast }) => toast(\"发布流程测试\") }); });")]);
    let publisher = Publisher::new(root).expect("需要本机 gh 与 git");
    let options = PublishOptions { repo_name: String::new(), license: Some("mit".into()), generate_readme: true };
    let result = publisher.publish(&manager, "publish-smoke-test", &options);
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    assert_eq!(result["ok"], true);
    let repo = result["repo"].as_str().unwrap().to_string();

    // 社区栏安装流程：按 release → tag 解析版本，下载 release 附件并校验，装进另一个临时目录。
    let install_root = tempfile::tempdir().unwrap();
    let installer = UserScriptManager::new(install_root.path().join("builtin"), install_root.path().join("user_scripts"), install_root.path().join("user_scripts.json"));
    let store = codex_plus_core::extension_discovery::CommunityStore::new(install_root.path());
    let endpoints = codex_plus_core::extension_discovery::Endpoints::github();
    let prepared = store.prepare(&endpoints, &installer, &repo).await.unwrap();
    println!("prepared: tag={} sha={} fromAsset={}", prepared["tag"], prepared["sha"], prepared["fromAsset"]);
    assert_eq!(prepared["tag"], "v0.1.0");
    assert_eq!(prepared["fromAsset"], true, "从 release 附件安装（计入下载量）");
    store.install(&installer, prepared["token"].as_str().unwrap(), true, false).unwrap();
    let installed: Value = serde_json::from_str(&fs::read_to_string(install_root.path().join("user_scripts/publish-smoke-test/codexpp.json")).unwrap()).unwrap();
    assert_eq!(installed["repository"], repo.as_str());
    assert!(install_root.path().join("user_scripts/publish-smoke-test/README.md").is_file());
    assert!(install_root.path().join("user_scripts/publish-smoke-test/LICENSE").is_file());
}
