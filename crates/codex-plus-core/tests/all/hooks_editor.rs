use codex_plus_core::hooks_editor::{
    config_defines_hooks, configured_projects, content_version, list_files, test_run, validate_hooks_document,
    write_file,
};
use serde_json::{Value, json};

// Hooks 编辑器后端：列文件、按版本写回、试运行。只碰临时目录。

fn files(value: &Value) -> &Vec<Value> {
    value["files"].as_array().unwrap()
}

#[test]
fn lists_user_file_and_configured_projects_with_trust() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    let trusted = temp.path().join("trusted-proj");
    let untrusted = temp.path().join("untrusted-proj");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(trusted.join(".codex")).unwrap();
    std::fs::create_dir_all(&untrusted).unwrap();
    std::fs::write(home.join("hooks.json"), r#"{"hooks":{}}"#).unwrap();
    std::fs::write(trusted.join(".codex/hooks.json"), r#"{"hooks":{"Stop":[]}}"#).unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            "[projects.{:?}]\ntrust_level = \"trusted\"\n\n[projects.{:?}]\ntrust_level = \"untrusted\"\n\n[hooks.state.\"k\"]\nenabled = false\n",
            trusted.to_string_lossy(),
            untrusted.to_string_lossy()
        ),
    )
    .unwrap();

    let result = list_files(&home, &json!({}));
    assert_eq!(result["status"], "ok");
    let list = files(&result);
    assert_eq!(list.len(), 3);
    assert_eq!(list[0]["scope"], "user");
    assert_eq!(list[0]["exists"], true);
    assert_eq!(list[0]["version"], content_version(Some(br#"{"hooks":{}}"#)));

    let by_root = |root: &std::path::Path| {
        list.iter()
            .find(|entry| entry["root"] == root.to_string_lossy().as_ref())
            .unwrap()
            .clone()
    };
    let t = by_root(&trusted);
    assert_eq!(t["trusted"], true);
    assert_eq!(t["exists"], true);
    assert_eq!(t["configured"], true);
    assert_eq!(t["text"], r#"{"hooks":{"Stop":[]}}"#);
    let u = by_root(&untrusted);
    assert_eq!(u["trusted"], false);
    assert_eq!(u["exists"], false);
    assert_eq!(u["version"], "");
    // 只有 [hooks.state] 不算在 config.toml 里定义了 hooks。
    assert_eq!(result["configDefinesHooks"], false);
}

#[test]
fn extra_roots_are_validated_and_deduplicated() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let extra = temp.path().join("extra");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&extra).unwrap();
    let extra_str = extra.to_string_lossy().into_owned();
    let result = list_files(
        &home,
        &json!({ "roots": [extra_str, extra_str, "relative/dir", temp.path().join("missing").to_string_lossy(), format!("{}/../x", extra_str)] }),
    );
    let list = files(&result);
    assert_eq!(list.len(), 2, "用户层 + 一个额外项目（重复的只算一次）");
    assert_eq!(list[1]["configured"], false);
    assert_eq!(list[1]["trusted"], false);
    assert_eq!(result["skipped"].as_array().unwrap().len(), 3);
}

#[test]
fn config_hooks_and_projects_parse() {
    let temp = tempfile::tempdir().unwrap();
    assert!(configured_projects(temp.path()).is_empty());
    assert!(!config_defines_hooks(temp.path()));
    std::fs::write(
        temp.path().join("config.toml"),
        "[[hooks.Stop]]\n[[hooks.Stop.hooks]]\ntype = \"command\"\ncommand = \"true\"\n\n[projects.\"/b\"]\ntrust_level = \"trusted\"\n[projects.\"/a\"]\n",
    )
    .unwrap();
    assert!(config_defines_hooks(temp.path()));
    assert_eq!(configured_projects(temp.path()), vec![("/a".to_string(), false), ("/b".to_string(), true)]);
}

#[test]
fn validates_hooks_document_shape() {
    assert!(validate_hooks_document(r#"{"hooks":{"Stop":[]},"extra":1}"#).is_ok());
    assert!(validate_hooks_document("{}").is_ok());
    assert!(validate_hooks_document("[]").is_err());
    assert!(validate_hooks_document(r#"{"hooks":[]}"#).is_err());
    assert!(validate_hooks_document(r#"{"hooks":{"Stop":{}}}"#).is_err());
    let error = validate_hooks_document("{oops").unwrap_err().to_string();
    assert!(error.contains("不是合法的 JSON"), "{error}");
}

#[test]
fn write_requires_matching_version_and_known_location() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("proj");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();

    // 新建：文件不存在时版本是空串。
    let text = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"true"}]}]}}"#;
    let written = write_file(&home, &json!({ "scope": "user", "text": text, "expectedVersion": "" })).unwrap();
    assert_eq!(std::fs::read_to_string(home.join("hooks.json")).unwrap(), text);
    let version = written["version"].as_str().unwrap().to_owned();
    assert_eq!(version, content_version(Some(text.as_bytes())));

    // 版本不对：拒绝，文件不变。
    let stale = write_file(&home, &json!({ "scope": "user", "text": "{}", "expectedVersion": "" })).unwrap_err();
    assert!(stale.to_string().contains("别处被修改"), "{stale}");
    assert_eq!(std::fs::read_to_string(home.join("hooks.json")).unwrap(), text);

    // 版本对：覆盖。
    write_file(&home, &json!({ "scope": "user", "text": "{}", "expectedVersion": version })).unwrap();
    assert_eq!(std::fs::read_to_string(home.join("hooks.json")).unwrap(), "{}");

    // 项目层：自动建 .codex 目录。
    write_file(
        &home,
        &json!({ "scope": "project", "root": project.to_string_lossy(), "text": "{}", "expectedVersion": "" }),
    )
    .unwrap();
    assert!(project.join(".codex/hooks.json").is_file());

    // 非法：内容、scope、项目目录。
    assert!(write_file(&home, &json!({ "scope": "user", "text": "[]", "expectedVersion": content_version(Some(b"{}")) })).is_err());
    assert!(write_file(&home, &json!({ "scope": "other", "text": "{}", "expectedVersion": "" })).is_err());
    assert!(write_file(&home, &json!({ "scope": "project", "root": "relative", "text": "{}", "expectedVersion": "" })).is_err());
    assert!(
        write_file(
            &home,
            &json!({ "scope": "project", "root": temp.path().join("missing").to_string_lossy(), "text": "{}", "expectedVersion": "" })
        )
        .is_err()
    );
    assert!(write_file(&home, &json!({ "scope": "user", "text": "{}" })).is_err(), "缺 expectedVersion 拒绝");
}

#[cfg(unix)]
#[tokio::test]
async fn test_run_feeds_stdin_and_reports_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let result = test_run(
        &home,
        &json!({
            "command": "cat; echo \"$CODEX_HOME\" >&2; exit 2",
            "cwd": temp.path().to_string_lossy(),
            "stdin": "{\"hook_event_name\":\"Stop\"}",
            "timeoutSec": 10,
        }),
    )
    .await
    .unwrap();
    assert_eq!(result["exitCode"], 2);
    assert_eq!(result["timedOut"], false);
    assert_eq!(result["stdout"], "{\"hook_event_name\":\"Stop\"}");
    assert_eq!(result["stderr"].as_str().unwrap().trim(), home.to_string_lossy());
}

#[cfg(unix)]
#[tokio::test]
async fn test_run_times_out_and_rejects_empty_command() {
    let temp = tempfile::tempdir().unwrap();
    let result = test_run(temp.path(), &json!({ "command": "sleep 5", "timeoutSec": 1 })).await.unwrap();
    assert_eq!(result["timedOut"], true);
    assert!(result["exitCode"].is_null());
    assert!(result["durationMs"].as_u64().unwrap() < 4000);

    assert!(test_run(temp.path(), &json!({ "command": "  " })).await.is_err());
}
