//! 网关接入与断开都只改有归属的 JSONC 值，保留用户文件的其它原文。

use codex_plus_core::gateway::agents::files::{
    AgentStash, FileChange, FileSet, parse_jsonc, restored_text,
};
use serde_json::json;

fn edit(
    before: &str,
    update: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) -> FileChange {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.jsonc");
    std::fs::write(&path, before).unwrap();
    let mut files = FileSet::new();
    update(files.json(&path).unwrap());
    let changes = files.changes().unwrap();
    assert_eq!(changes.len(), 1);
    changes.into_iter().next().unwrap()
}

#[test]
fn changes_keep_comments_order_indent_and_trailing_commas() {
    let before = "{\n    // 用户设置\n    \"z\": \"http://example.test/a//b,}\",\n    \"model\": \"before\", // 跟着模型的说明\n    \"a\": [\"a,]\", \"escaped\\\\\\\"//\",],\n}\n";
    let change = edit(before, |root| {
        root.insert("model".into(), json!("after"));
    });
    assert_eq!(
        change.after.as_deref().unwrap(),
        before.replace("\"before\"", "\"after\""),
    );
}

#[test]
fn unchanged_values_do_not_rewrite_a_jsonc_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.jsonc");
    let before = "{\n\t// 原文\n\t\"z\": 1,\n\t\"a\": 2,\n}\n";
    std::fs::write(&path, before).unwrap();
    let mut files = FileSet::new();
    files.json(&path).unwrap().insert("a".into(), json!(2));
    assert!(files.changes().unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(path).unwrap(), before);
}

#[test]
fn nested_additions_and_removals_keep_unrelated_members() {
    let before = "{\n  \"provider\": {\n    // 用户项\n    \"mine\": {\"baseURL\":\"http://mine\", \"keep\": true,},\n    \"remove\": \"old\",\n  },\n  \"untouched\": 42, // 保留\n}\n";
    let change = edit(before, |root| {
        let provider = root["provider"].as_object_mut().unwrap();
        provider.remove("remove");
        provider.insert(
            "codexplus".into(),
            json!({"baseURL":"http://gateway", "apiKey":"local-test"}),
        );
    });
    let after = change.after.as_deref().unwrap();
    assert!(
        after.contains("// 用户项\n    \"mine\": {\"baseURL\":\"http://mine\", \"keep\": true,},")
    );
    assert!(after.contains("  \"untouched\": 42, // 保留\n"));
    // 新对象里字段的先后取决于 serde_json 是否开了 preserve_order（整个工作区一起编译时会被别的 crate 打开），
    // 这里只断言缩进和内容，不断言 apiKey / baseURL 的先后。
    let block = after
        .split("    \"codexplus\": {\n")
        .nth(1)
        .and_then(|rest| rest.split("\n    },\n").next())
        .expect("codexplus 块按四格缩进写出");
    let mut fields: Vec<&str> = block.lines().map(|line| line.trim_end_matches(',')).collect();
    fields.sort_unstable();
    assert_eq!(fields, ["      \"apiKey\": \"local-test\"", "      \"baseURL\": \"http://gateway\""]);
    assert!(!after.contains("\"remove\""));
    assert_eq!(
        parse_jsonc(after).unwrap()["provider"]["mine"]["keep"],
        true
    );
}

#[test]
fn crlf_bom_and_tabs_survive_add_edit_and_restore_merge() {
    let before = "\u{feff}{\r\n\t// Windows 配置\r\n\t\"model\": \"own\",\r\n\t\"user\": {\"theme\": \"dark\",},\r\n}\r\n";
    let change = edit(before, |root| {
        root.insert("model".into(), json!("gateway"));
        root.insert("provider".into(), json!({"baseURL":"http://gateway"}));
    });
    let after = change.after.as_deref().unwrap();
    assert!(after.starts_with('\u{feff}'));
    assert_eq!(after.matches('\u{feff}').count(), 1);
    assert!(after.contains("\r\n\t\"provider\": {\r\n\t\t\"baseURL\": \"http://gateway\"\r\n\t},"));
    assert!(!after.replace("\r\n", "").contains('\n'));
    let current = after.replace("\"dark\"", "\"light\"");
    let mut stash = AgentStash::default();
    stash.record(&change).unwrap();
    let file = stash.files.values().next().unwrap();
    let restored = restored_text(file, Some(&current)).unwrap().unwrap();
    assert_eq!(restored, before.replace("\"dark\"", "\"light\""));
}

#[test]
fn restore_merge_preserves_comments_added_while_connected() {
    let before = "{\n  // 自己的模型\n  \"model\": \"mine\",\n  \"preferences\": {\"color\": \"black\"}\n}\n";
    let change = edit(before, |root| {
        root.insert("model".into(), json!("gateway"));
        root.insert("codexplus".into(), json!({"key":"local-test"}));
    });
    let current = change.after.as_ref().unwrap().replace(
        "\"preferences\": {\"color\": \"black\"}",
        "\"preferences\": {\"color\": \"white\", /* 新注释 */ \"size\": 12}",
    );
    let mut stash = AgentStash::default();
    stash.record(&change).unwrap();
    let restored = restored_text(stash.files.values().next().unwrap(), Some(&current))
        .unwrap()
        .unwrap();
    assert_eq!(
        restored,
        before.replace(
            "\"preferences\": {\"color\": \"black\"}",
            "\"preferences\": {\"color\": \"white\", /* 新注释 */ \"size\": 12}"
        ),
    );
}

#[test]
fn restore_preserves_comment_only_and_format_only_edits() {
    let before = "{\n  // 原说明\n  \"model\": \"mine\",\n  \"untouched\": true\n}\n";
    let change = edit(before, |root| {
        root.insert("model".into(), json!("gateway"));
        root.insert("codexplus".into(), json!({"key":"local-test"}));
    });
    let mut stash = AgentStash::default();
    stash.record(&change).unwrap();
    let record = stash.files.values().next().unwrap();
    assert_eq!(
        restored_text(record, change.after.as_deref())
            .unwrap()
            .as_deref(),
        Some(before),
    );
    for current in [
        change
            .after
            .as_ref()
            .unwrap()
            .replace("// 原说明", "// 用户的新说明"),
        change
            .after
            .as_ref()
            .unwrap()
            .replace("  \"untouched\"", "    \"untouched\""),
    ] {
        let restored = restored_text(record, Some(&current)).unwrap().unwrap();
        assert_eq!(
            parse_jsonc(&restored).unwrap(),
            parse_jsonc(before).unwrap()
        );
        if current.contains("用户的新说明") {
            assert!(restored.contains("// 用户的新说明"));
            assert!(!restored.contains("// 原说明"));
        } else {
            assert!(restored.contains("    \"untouched\""));
        }
    }
}

#[test]
fn restore_keeps_new_comments_when_gateway_created_the_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("new.jsonc");
    let mut files = FileSet::new();
    files
        .json(&path)
        .unwrap()
        .insert("model".into(), json!("gateway"));
    let change = files.changes().unwrap().pop().unwrap();
    let mut stash = AgentStash::default();
    stash.record(&change).unwrap();
    let record = stash.files.values().next().unwrap();
    assert_eq!(
        restored_text(record, change.after.as_deref()).unwrap(),
        None
    );
    let current = change.after.unwrap().replace('{', "{\n  // 后来加的说明");
    let restored = restored_text(record, Some(&current)).unwrap().unwrap();
    assert!(restored.contains("// 后来加的说明"));
    assert_eq!(parse_jsonc(&restored).unwrap(), json!({}));
}

#[test]
fn later_gateway_refresh_does_not_claim_user_comments_as_original_format() {
    let before = "{\n  // 原说明\n  \"model\": \"mine\",\n  \"keep\": true\n}\n";
    let first = edit(before, |root| {
        root.insert("model".into(), json!("gateway-one"));
    });
    let current = first.after.as_ref().unwrap().replace("// 原说明", "// 新增用户说明");
    let second = edit(&current, |root| {
        root.insert("model".into(), json!("gateway-two"));
    });
    let mut stash = AgentStash::default();
    stash.record(&first).unwrap();
    // 两次 FileChange 模拟同一个配置路径。
    let second = FileChange { path: first.path.clone(), ..second };
    stash.record(&second).unwrap();
    let record = stash.files.values().next().unwrap();
    let restored = restored_text(record, second.after.as_deref()).unwrap().unwrap();
    assert!(restored.contains("// 新增用户说明"));
    assert!(!restored.contains("// 原说明"));
    assert_eq!(parse_jsonc(&restored).unwrap(), parse_jsonc(before).unwrap());
}

#[test]
fn empty_and_comment_only_objects_accept_new_members() {
    for before in [
        "{}",
        "{ }",
        "{\n}",
        "{\n  // 用户注释\n}",
        "{ /* 用户注释 */ }",
        "{\r\n\t// 用户注释\r\n}",
        "// 文件说明\n",
        "\u{feff}",
    ] {
        let change = edit(before, |root| {
            root.insert("gateway".into(), json!({"model":"a"}));
        });
        let after = change.after.as_deref().unwrap();
        assert_eq!(
            parse_jsonc(after).unwrap(),
            json!({"gateway":{"model":"a"}}),
            "{before:?}"
        );
        if before.contains("用户注释") {
            assert!(after.contains("用户注释"));
        }
        if before.starts_with('\u{feff}') {
            assert!(after.starts_with('\u{feff}'));
        }
    }
}

#[test]
fn removing_first_middle_last_and_only_members_stays_valid() {
    for before in [
        "{\"a\":1,\"b\":2,\"c\":3}",
        "{\"a\":1,\"b\":2,\"c\":3,}",
        "{\n  // 前面\n  \"a\":1,\n  \"b\":2, // 中间\n  \"c\":3\n}",
        "{ /* 不归属键的注释 */ \"a\":1}",
    ] {
        let parsed = parse_jsonc(before).unwrap();
        for key in parsed.as_object().unwrap().keys() {
            let change = edit(before, |root| {
                root.remove(key);
            });
            let mut expected = parsed.clone();
            expected.as_object_mut().unwrap().remove(key);
            let after = change.after.as_deref().unwrap();
            assert_eq!(parse_jsonc(after).unwrap(), expected, "{before:?}, {key}");
            if before.contains("不归属键") {
                assert!(after.contains("不归属键"));
            }
        }
    }
}

#[test]
fn arrays_patch_changed_elements_and_keep_unchanged_comments() {
    let before = "{\n  \"items\": [\n    // 头\n    {\"name\":\"a\", \"own\": true,},\n    {\"name\":\"b\"}, // 尾\n  ],\n  \"other\": [\"keep\",],\n}\n";
    for desired in [
        json!([{"name":"a", "own":true}, {"name":"x"}]),
        json!([{"name":"a", "own":true}]),
        json!([{"name":"a", "own":true}, {"name":"x"}, {"name":"b"}]),
        json!([{"name":"new"}, {"name":"a", "own":true}, {"name":"b"}]),
        json!([]),
    ] {
        let change = edit(before, |root| {
            root.insert("items".into(), desired.clone());
        });
        let after = change.after.as_deref().unwrap();
        assert_eq!(parse_jsonc(after).unwrap()["items"], desired);
        assert!(after.contains("  \"other\": [\"keep\",],\n"));
        assert!(after.contains("// 头"));
        if desired
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "a")
        {
            assert!(after.contains("{\"name\":\"a\", \"own\": true,}"));
        }
    }
}

#[test]
fn appending_after_inline_comments_keeps_the_file_valid() {
    for before in [
        "{\"a\": 1 // 末尾\n}",
        "{\"a\": 1 /* 末尾 */}",
        "{\"a\": 1, /* 末尾 */}",
        "{\n  \"a\": 1, // 末尾\n}",
        "{\n  \"a\": 1 /* 末尾 */}",
    ] {
        let change = edit(before, |root| {
            root.insert("b".into(), json!(2));
        });
        let after = change.after.as_deref().unwrap();
        assert_eq!(
            parse_jsonc(after).unwrap(),
            json!({"a":1,"b":2}),
            "{before:?}"
        );
        assert!(after.contains("末尾"));
    }
}
