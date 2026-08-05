use codex_plus_core::user_catalog::{apply_before_launch, merge_catalogs};
use serde_json::json;

// ---------------------------------------------------------------------------
// merge_catalogs 单元测试
// ---------------------------------------------------------------------------

#[test]
fn merge_replaces_whole_object_for_matching_slug() {
    // issue #1772: 同 slug 时用户整个对象替换基础对象，不是字段级合并
    let base = json!({
        "models": [
            {
                "slug": "deepseek-v4-flash",
                "display_name": "DeepSeek V4 Flash",
                "context_window": 272000,
                "base_instructions": "You are Codex..."
            }
        ]
    });
    let user = json!({
        "models": [
            {
                "slug": "deepseek-v4-flash",
                "context_window": 1048576,
                "max_context_window": 1048576,
                "default_reasoning_effort": "high",
                "base_instructions": "You are DeepSeek..."
            }
        ]
    });

    let merged: serde_json::Value =
        serde_json::from_str(&merge_catalogs(&base.to_string(), &user.to_string()).unwrap())
            .unwrap();
    let model = &merged["models"][0];

    // 用户字段全部生效
    assert_eq!(model["context_window"], 1048576);
    assert_eq!(model["max_context_window"], 1048576);
    assert_eq!(model["default_reasoning_effort"], "high");
    assert_eq!(model["base_instructions"], "You are DeepSeek...");
    // 基础对象中用户未提供的字段不存在（整个对象替换，不是合并）
    assert!(model.get("display_name").is_none());
}

#[test]
fn merge_appends_new_slug_from_user() {
    let base = json!({
        "models": [
            {"slug": "gpt-5.5", "context_window": 272000}
        ]
    });
    let user = json!({
        "models": [
            {
                "slug": "deepseek-v4-flash",
                "context_window": 1048576,
                "base_instructions": "You are DeepSeek"
            }
        ]
    });

    let merged: serde_json::Value =
        serde_json::from_str(&merge_catalogs(&base.to_string(), &user.to_string()).unwrap())
            .unwrap();
    let models = merged["models"].as_array().unwrap();

    assert_eq!(models.len(), 2);
    // 基础模型不受影响
    assert_eq!(models[0]["slug"], "gpt-5.5");
    assert_eq!(models[0]["context_window"], 272000);
    // 用户模型追加
    assert_eq!(models[1]["slug"], "deepseek-v4-flash");
    assert_eq!(models[1]["context_window"], 1048576);
}

#[test]
fn merge_preserves_base_only_models() {
    let base = json!({
        "models": [
            {"slug": "gpt-5.5", "context_window": 272000},
            {"slug": "gpt-5.6-sol", "context_window": 272000}
        ]
    });
    let user = json!({
        "models": [
            {"slug": "gpt-5.6-sol", "context_window": 1000000}
        ]
    });

    let merged: serde_json::Value =
        serde_json::from_str(&merge_catalogs(&base.to_string(), &user.to_string()).unwrap())
            .unwrap();
    let models = merged["models"].as_array().unwrap();

    // gpt-5.5 保持原样
    let gpt55 = models.iter().find(|m| m["slug"] == "gpt-5.5").unwrap();
    assert_eq!(gpt55["context_window"], 272000);
    // gpt-5.6-sol 被用户替换
    let sol = models.iter().find(|m| m["slug"] == "gpt-5.6-sol").unwrap();
    assert_eq!(sol["context_window"], 1000000);
}

#[test]
fn merge_rejects_duplicate_slug_in_user_file() {
    let base = json!({"models": [{"slug": "gpt-5.5"}]});
    let user = json!({
        "models": [
            {"slug": "deepseek-v4-flash", "context_window": 1000000},
            {"slug": "deepseek-v4-flash", "context_window": 2000000}
        ]
    });

    let result = merge_catalogs(&base.to_string(), &user.to_string());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("重复 slug"));
}

#[test]
fn merge_rejects_user_entry_without_slug() {
    let base = json!({"models": []});
    let user = json!({"models": [{"context_window": 1000000}]});

    let result = merge_catalogs(&base.to_string(), &user.to_string());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("非空 slug"));
}

#[test]
fn merge_rejects_empty_slug() {
    let base = json!({"models": []});
    let user = json!({"models": [{"slug": "  "}]});

    let result = merge_catalogs(&base.to_string(), &user.to_string());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("非空 slug"));
}

#[test]
fn merge_rejects_invalid_user_json() {
    let result = merge_catalogs(r#"{"models":[]}"#, "not json");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("用户目录 JSON 解析失败")
    );
}

#[test]
fn merge_rejects_missing_models_array() {
    let result = merge_catalogs(r#"{"models":[]}"#, r#"{"not_models":[]}"#);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("用户目录必须包含 models 数组")
    );
}

#[test]
fn merge_preserves_unknown_fields_in_user_entries() {
    // 用户条目中的未知字段应保留（兼容未来 catalog 新增字段）
    let base = json!({"models": [{"slug": "gpt-5.5", "context_window": 272000}]});
    let user = json!({
        "models": [
            {
                "slug": "gpt-5.5",
                "context_window": 1000000,
                "future_capability": "something_new",
                "experimental_flag": true
            }
        ]
    });

    let merged: serde_json::Value =
        serde_json::from_str(&merge_catalogs(&base.to_string(), &user.to_string()).unwrap())
            .unwrap();
    let model = &merged["models"][0];

    assert_eq!(model["future_capability"], "something_new");
    assert_eq!(model["experimental_flag"], true);
}

#[test]
fn merge_handles_empty_user_models() {
    let base = json!({"models": [{"slug": "gpt-5.5"}]});
    let user = json!({"models": []});

    let merged: serde_json::Value =
        serde_json::from_str(&merge_catalogs(&base.to_string(), &user.to_string()).unwrap())
            .unwrap();
    assert_eq!(merged["models"].as_array().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// apply_before_launch 集成测试
// ---------------------------------------------------------------------------

#[test]
fn apply_before_launch_generates_effective_catalog_and_updates_config() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();

    // 创建用户 models.json
    let user_catalog = home.join("my-models.json");
    std::fs::write(
        &user_catalog,
        json!({
            "models": [
                {
                    "slug": "deepseek-v4-flash",
                    "context_window": 1048576,
                    "max_context_window": 1048576,
                    "base_instructions": "You are DeepSeek."
                }
            ]
        })
        .to_string(),
    )
    .unwrap();

    // 创建初始 config.toml（无 model_catalog_json）
    std::fs::write(home.join("config.toml"), "model = \"gpt-5.5\"\n").unwrap();

    apply_before_launch(home, true, user_catalog.to_str().unwrap()).unwrap();

    // config.toml 应指向有效目录
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(config.contains("model-catalogs/codexplusplus-effective.json"));

    // 有效目录应存在且包含合并结果
    let effective: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join("model-catalogs/codexplusplus-effective.json")).unwrap(),
    )
    .unwrap();
    let models = effective["models"].as_array().unwrap();

    // 基础模型（bundled）仍存在
    assert!(models.iter().any(|m| m["slug"] == "gpt-5.5"));

    // 用户模型存在且字段完整
    let ds = models
        .iter()
        .find(|m| m["slug"] == "deepseek-v4-flash")
        .unwrap();
    assert_eq!(ds["context_window"], 1048576);
    assert_eq!(ds["base_instructions"], "You are DeepSeek.");

    // 状态文件应存在，记录原始指针为 null（原本没有）
    let state: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join("model-catalogs/.codexplusplus-catalog-state.json"))
            .unwrap(),
    )
    .unwrap();
    assert!(state["original_model_catalog_json"].is_null());
}

#[test]
fn apply_before_launch_disabled_restores_original_pointer() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();

    // 模拟之前启用时的状态：config 指向有效目录，状态文件记录原始值
    std::fs::write(
        home.join("config.toml"),
        "model = \"gpt-5.5\"\nmodel_catalog_json = \"model-catalogs/codexplusplus-effective.json\"\n",
    )
    .unwrap();

    let state_path = home.join("model-catalogs/.codexplusplus-catalog-state.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    std::fs::write(
        &state_path,
        json!({"original_model_catalog_json": "custom/old-catalog.json"}).to_string(),
    )
    .unwrap();

    // 禁用时恢复
    apply_before_launch(home, false, "").unwrap();

    // config.toml 应恢复原始指针
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(config.contains("model_catalog_json = \"custom/old-catalog.json\""));
    assert!(!config.contains("codexplusplus-effective.json"));

    // 状态文件应被删除
    assert!(!state_path.exists());
}

#[test]
fn apply_before_launch_disabled_restores_null_when_original_was_absent() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();

    std::fs::write(
        home.join("config.toml"),
        "model = \"gpt-5.5\"\nmodel_catalog_json = \"model-catalogs/codexplusplus-effective.json\"\n",
    )
    .unwrap();

    let state_path = home.join("model-catalogs/.codexplusplus-catalog-state.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    std::fs::write(
        &state_path,
        json!({"original_model_catalog_json": null}).to_string(),
    )
    .unwrap();

    apply_before_launch(home, false, "").unwrap();

    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(!config.contains("model_catalog_json"));
    assert!(!state_path.exists());
}

#[test]
fn apply_before_launch_disabled_without_state_is_noop() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();

    std::fs::write(home.join("config.toml"), "model = \"gpt-5.5\"\n").unwrap();

    apply_before_launch(home, false, "").unwrap();

    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert_eq!(config, "model = \"gpt-5.5\"\n");
}

#[test]
fn apply_before_launch_reuses_saved_original_on_relaunch() {
    // 二次启动：config 已指向有效目录，应从状态文件读取原始指针作为 base
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();

    // 用户 catalog
    let user_catalog = home.join("my-models.json");
    std::fs::write(
        &user_catalog,
        json!({
            "models": [
                {"slug": "deepseek-v4-flash", "context_window": 1048576}
            ]
        })
        .to_string(),
    )
    .unwrap();

    // 模拟之前已启用：config 指向有效目录
    std::fs::write(
        home.join("config.toml"),
        "model = \"gpt-5.5\"\nmodel_catalog_json = \"model-catalogs/codexplusplus-effective.json\"\n",
    )
    .unwrap();

    // 状态文件记录原始值为 null
    let state_path = home.join("model-catalogs/.codexplusplus-catalog-state.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    std::fs::write(
        &state_path,
        json!({"original_model_catalog_json": null}).to_string(),
    )
    .unwrap();

    // 再次启动
    apply_before_launch(home, true, user_catalog.to_str().unwrap()).unwrap();

    // 状态文件仍应记录 null（不应被有效目录路径覆盖）
    let state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    assert!(state["original_model_catalog_json"].is_null());

    // 有效目录应包含合并结果
    let effective: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join("model-catalogs/codexplusplus-effective.json")).unwrap(),
    )
    .unwrap();
    assert!(
        effective["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["slug"] == "deepseek-v4-flash")
    );
}

#[test]
fn apply_before_launch_errors_on_missing_user_file() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    std::fs::write(home.join("config.toml"), "model = \"gpt-5.5\"\n").unwrap();

    let result = apply_before_launch(home, true, "/nonexistent/path.json");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("读取用户模型目录失败")
    );
}

#[test]
fn apply_before_launch_errors_on_invalid_user_json() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    std::fs::write(home.join("config.toml"), "model = \"gpt-5.5\"\n").unwrap();
    std::fs::write(home.join("bad.json"), "not json").unwrap();

    let result = apply_before_launch(home, true, home.join("bad.json").to_str().unwrap());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("JSON 解析失败"));
}

#[test]
fn apply_before_launch_errors_on_duplicate_slug() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    std::fs::write(home.join("config.toml"), "model = \"gpt-5.5\"\n").unwrap();
    std::fs::write(
        home.join("dup.json"),
        json!({
            "models": [
                {"slug": "deepseek-v4-flash", "context_window": 1000000},
                {"slug": "deepseek-v4-flash", "context_window": 2000000}
            ]
        })
        .to_string(),
    )
    .unwrap();

    let result = apply_before_launch(home, true, home.join("dup.json").to_str().unwrap());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("重复 slug"));
}

#[test]
fn apply_before_launch_preserves_existing_external_catalog_as_base() {
    // 如果 config.toml 已有外部 model_catalog_json，应以其为基础合并
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();

    // 创建外部基础目录
    let base_catalog = home.join("base-catalog.json");
    std::fs::write(
        &base_catalog,
        json!({
            "models": [
                {"slug": "custom-model", "context_window": 128000},
                {"slug": "deepseek-v4-flash", "context_window": 128000, "display_name": "Old Name"}
            ]
        })
        .to_string(),
    )
    .unwrap();

    // config.toml 指向外部目录
    std::fs::write(
        home.join("config.toml"),
        format!(
            "model = \"gpt-5.5\"\nmodel_catalog_json = \"{}\"\n",
            base_catalog.to_str().unwrap().replace('\\', "/")
        ),
    )
    .unwrap();

    // 用户目录覆盖 deepseek-v4-flash
    let user_catalog = home.join("user.json");
    std::fs::write(
        &user_catalog,
        json!({
            "models": [
                {"slug": "deepseek-v4-flash", "context_window": 1048576, "max_context_window": 1048576}
            ]
        })
        .to_string(),
    )
    .unwrap();

    apply_before_launch(home, true, user_catalog.to_str().unwrap()).unwrap();

    // 状态文件应记录原始外部目录路径
    let state: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join("model-catalogs/.codexplusplus-catalog-state.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        state["original_model_catalog_json"]
            .as_str()
            .unwrap()
            .replace('\\', "/"),
        base_catalog.to_str().unwrap().replace('\\', "/")
    );

    // 有效目录应以外部目录为基础
    let effective: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join("model-catalogs/codexplusplus-effective.json")).unwrap(),
    )
    .unwrap();
    let models = effective["models"].as_array().unwrap();

    // 外部目录中的 custom-model 保留
    assert!(models.iter().any(|m| m["slug"] == "custom-model"));

    // deepseek-v4-flash 被用户对象替换（display_name 不再存在）
    let ds = models
        .iter()
        .find(|m| m["slug"] == "deepseek-v4-flash")
        .unwrap();
    assert_eq!(ds["context_window"], 1048576);
    assert!(ds.get("display_name").is_none());
}
