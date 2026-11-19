use codex_plus_core::custom_prompts::{list_custom_prompts, parse_custom_prompt};
use codex_plus_core::settings::BackendSettings;

// 解析规则与 Codex 应用（读 <CODEX_HOME>/prompts/*.md 生成 /prompts:<名字>）保持一致。

#[test]
fn missing_prompts_dir_yields_empty_list() {
    let temp = tempfile::tempdir().unwrap();
    assert!(list_custom_prompts(temp.path()).is_empty());
}

#[test]
fn lists_only_top_level_markdown_files_sorted_by_id() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("prompts");
    std::fs::create_dir_all(dir.join("nested")).unwrap();
    std::fs::write(dir.join("review.md"), "Review $1").unwrap();
    std::fs::write(dir.join("alpha.md"), "Alpha").unwrap();
    std::fs::write(dir.join("notes.txt"), "not a prompt").unwrap();
    std::fs::write(dir.join("nested").join("inner.md"), "nested").unwrap();
    std::fs::create_dir_all(dir.join("folder.md")).unwrap();

    let prompts = list_custom_prompts(temp.path());
    let ids: Vec<_> = prompts.iter().map(|prompt| prompt.id.as_str()).collect();
    assert_eq!(ids, ["alpha", "review"]);
    assert_eq!(prompts[1].content, "Review $1");
    assert_eq!(prompts[1].description, None);
}

#[test]
fn skips_oversized_files() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("prompts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("huge.md"), "x".repeat(300 * 1024)).unwrap();
    std::fs::write(dir.join("small.md"), "ok").unwrap();
    let ids: Vec<_> = list_custom_prompts(temp.path()).into_iter().map(|prompt| prompt.id).collect();
    assert_eq!(ids, ["small"]);
}

#[test]
fn frontmatter_supplies_description_and_argument_hint() {
    let prompt = parse_custom_prompt(
        "review".into(),
        "---\ndescription: \"Review a branch\"\nargument-hint: [BRANCH=<name>]\nother: ignored\n---\n\nReview $BRANCH carefully.\n",
    );
    assert_eq!(prompt.description.as_deref(), Some("Review a branch"));
    assert_eq!(prompt.argument_hint.as_deref(), Some("[BRANCH=<name>]"));
    assert_eq!(prompt.content, "Review $BRANCH carefully.");
}

#[test]
fn without_frontmatter_the_whole_file_is_content() {
    let prompt = parse_custom_prompt("plain".into(), "  just text: with colon\n");
    assert_eq!(prompt.content, "just text: with colon");
    assert_eq!(prompt.description, None);
    assert_eq!(prompt.argument_hint, None);

    // 只有开头的 --- 没有结尾：不算 frontmatter，原样当正文。
    let unclosed = parse_custom_prompt("open".into(), "---\ndescription: x\nbody");
    assert_eq!(unclosed.description, None);
    assert!(unclosed.content.starts_with("---"));
}

#[test]
fn serializes_with_camel_case_keys() {
    let prompt = parse_custom_prompt("x".into(), "---\nargument-hint: FILE\n---\nbody");
    let value = serde_json::to_value(&prompt).unwrap();
    assert_eq!(value["argumentHint"], "FILE");
    assert_eq!(value["content"], "body");
    assert!(value["description"].is_null());
}

#[test]
fn prompt_args_setting_defaults_on_including_old_settings() {
    assert!(BackendSettings::default().codex_app_prompt_args);
    let old: BackendSettings = serde_json::from_value(serde_json::json!({ "enhancementsEnabled": true })).unwrap();
    assert!(old.codex_app_prompt_args);
    let off: BackendSettings = serde_json::from_value(serde_json::json!({ "codexAppPromptArgs": false })).unwrap();
    assert!(!off.codex_app_prompt_args);
    assert_eq!(serde_json::to_value(&off).unwrap()["codexAppPromptArgs"], false);
}
