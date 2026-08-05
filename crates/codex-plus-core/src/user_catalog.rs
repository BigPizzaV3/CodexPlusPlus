//! 用户自定义模型目录合并（#1772）。
//!
//! 全局设置，独立于 Relay Profile 和供应商管理。在 Codex 启动前，将用户选择的
//! `models.json` 与基础目录按 slug 合并，生成有效目录并更新 config.toml 的
//! `model_catalog_json` 指针。禁用时恢复原始指针。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{Value, json};
use toml_edit::DocumentMut;

/// Codex++ 生成的有效目录在 ~/.codex/ 下的相对路径。
const EFFECTIVE_CATALOG_RELATIVE: &str = "model-catalogs/codexplusplus-effective.json";

/// 状态文件路径，记录原始 `model_catalog_json` 值以便恢复。
const STATE_FILE_RELATIVE: &str = "model-catalogs/.codexplusplus-catalog-state.json";

/// 内置基础模型目录（assets/codex-models.json）。
const BUNDLED_CATALOG_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/codex-models.json"
));

/// 在启动 Codex 前执行模型目录合并。
///
/// - `enabled` 且 `user_catalog_path` 非空：读取用户目录，合并基础目录，生成
///   有效目录，保存原始指针，更新 config.toml。
/// - `enabled` 为 false 或路径为空：恢复原始 `model_catalog_json`（如果之前被修改过）。
///
/// 任何错误都应阻止 Codex 启动，不得静默回退。
pub fn apply_before_launch(
    home: &Path,
    enabled: bool,
    user_catalog_path: &str,
) -> anyhow::Result<()> {
    let state_path = home.join(STATE_FILE_RELATIVE);

    if !enabled || user_catalog_path.trim().is_empty() {
        restore_original_catalog(home, &state_path)?;
        return Ok(());
    }

    // 1. 读取用户目录
    let user_path = resolve_path(home, user_catalog_path.trim());
    let user_json = std::fs::read_to_string(&user_path)
        .with_context(|| format!("读取用户模型目录失败：{}", user_path.display()))?;

    // 2. 读取当前 config.toml
    let config_path = home.join("config.toml");
    let config_contents = std::fs::read_to_string(&config_path).unwrap_or_default();

    // 3. 确定基础目录和原始指针
    let (base_json, original_pointer) = read_base_catalog(home, &config_contents, &state_path);

    // 4. 合并
    let effective_json = merge_catalogs(&base_json, &user_json)?;

    // 5. 原子写入有效目录
    let effective_path = home.join(EFFECTIVE_CATALOG_RELATIVE);
    crate::settings::atomic_write(&effective_path, effective_json.as_bytes())?;

    // 6. 保存原始指针到状态文件（仅在指针不是我们自己的有效目录时更新）
    save_state(&state_path, original_pointer.as_deref())?;

    // 7. 更新 config.toml 指向有效目录
    update_config_catalog_pointer(home, &config_contents, EFFECTIVE_CATALOG_RELATIVE)?;

    Ok(())
}

/// 合并基础目录和用户目录（#1772 合并规则）。
///
/// - 仅基础有：保留基础完整对象
/// - 仅用户有：加入有效目录
/// - 两边相同 slug：**用户整个对象替换基础对象**（非字段级合并）
/// - 用户文件内重复 slug：报错
///
/// 保留用户条目中的未知字段，以兼容未来 catalog 新增字段。
pub fn merge_catalogs(base_json: &str, user_json: &str) -> anyhow::Result<String> {
    let mut base: Value = serde_json::from_str(base_json)
        .map_err(|e| anyhow::anyhow!("基础目录 JSON 解析失败：{e}"))?;
    let user: Value = serde_json::from_str(user_json)
        .map_err(|e| anyhow::anyhow!("用户目录 JSON 解析失败：{e}"))?;

    let user_models = user
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("用户目录必须包含 models 数组"))?;

    // 校验：每个条目必须有非空 slug，且不能有重复 slug
    let mut seen_slugs = HashSet::new();
    for model in user_models {
        let slug = model
            .get("slug")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("用户目录中每个模型必须包含非空 slug 字段"))?;
        if !seen_slugs.insert(slug.to_string()) {
            anyhow::bail!("用户目录中存在重复 slug：{slug}");
        }
    }

    let base_models = base
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("基础目录必须包含 models 数组"))?;

    // 合并：同 slug 整个对象替换，新 slug 追加
    for user_model in user_models {
        let slug = user_model.get("slug").and_then(Value::as_str).unwrap();
        if let Some(base_model) = base_models
            .iter_mut()
            .find(|m| m.get("slug").and_then(Value::as_str) == Some(slug))
        {
            *base_model = user_model.clone();
        } else {
            base_models.push(user_model.clone());
        }
    }

    Ok(serde_json::to_string_pretty(&base)?)
}

// ---------------------------------------------------------------------------
// 内部实现
// ---------------------------------------------------------------------------

/// 读取基础目录和原始指针。
///
/// 如果当前 `model_catalog_json` 已经指向我们的有效目录（之前启动时设置的），
/// 则从状态文件中读取真正的原始指针。
fn read_base_catalog(
    home: &Path,
    config_contents: &str,
    state_path: &Path,
) -> (String, Option<String>) {
    let current_pointer =
        crate::relay_config::root_key_string(config_contents, "model_catalog_json");

    let original_pointer = if current_pointer.as_deref() == Some(EFFECTIVE_CATALOG_RELATIVE) {
        // 当前指针是我们的有效目录——从状态文件读取真正的原始值
        read_state_original(state_path)
    } else {
        // 当前指针不是我们的——它就是原始值
        current_pointer
    };

    let base_json = match &original_pointer {
        Some(path) if !path.is_empty() => {
            let full_path = resolve_path(home, path);
            std::fs::read_to_string(&full_path).unwrap_or_else(|_| BUNDLED_CATALOG_JSON.to_string())
        }
        _ => BUNDLED_CATALOG_JSON.to_string(),
    };

    (base_json, original_pointer)
}

/// 恢复原始 `model_catalog_json`（如果之前被 Codex++ 修改过）。
fn restore_original_catalog(home: &Path, state_path: &Path) -> anyhow::Result<()> {
    if !state_path.exists() {
        return Ok(());
    }

    let state_json = std::fs::read_to_string(state_path)
        .with_context(|| format!("读取状态文件失败：{}", state_path.display()))?;
    let state: Value = serde_json::from_str(&state_json)
        .map_err(|e| anyhow::anyhow!("状态文件 JSON 解析失败：{e}"))?;
    let original = state
        .get("original_model_catalog_json")
        .and_then(Value::as_str);

    let config_path = home.join("config.toml");
    let config_contents = std::fs::read_to_string(&config_path).unwrap_or_default();

    match original {
        Some(path) if !path.is_empty() => {
            update_config_catalog_pointer(home, &config_contents, path)?;
        }
        _ => {
            remove_config_catalog_pointer(home, &config_contents)?;
        }
    }

    // 删除状态文件
    let _ = std::fs::remove_file(state_path);

    Ok(())
}

fn read_state_original(state_path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(state_path).ok()?;
    let value: Value = serde_json::from_str(&contents).ok()?;
    value
        .get("original_model_catalog_json")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

fn save_state(state_path: &Path, original: Option<&str>) -> anyhow::Result<()> {
    if let Some(parent) = state_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let state = json!({
        "original_model_catalog_json": original
    });
    crate::settings::atomic_write(state_path, state.to_string().as_bytes())?;
    Ok(())
}

fn update_config_catalog_pointer(
    home: &Path,
    config_contents: &str,
    pointer: &str,
) -> anyhow::Result<()> {
    let mut doc = parse_toml(config_contents)?;
    doc["model_catalog_json"] = toml_edit::value(pointer);
    let updated = ensure_trailing_newline(doc.to_string());
    crate::settings::atomic_write(&home.join("config.toml"), updated.as_bytes())?;
    Ok(())
}

fn remove_config_catalog_pointer(home: &Path, config_contents: &str) -> anyhow::Result<()> {
    let mut doc = parse_toml(config_contents)?;
    doc.as_table_mut().remove("model_catalog_json");
    let updated = ensure_trailing_newline(doc.to_string());
    crate::settings::atomic_write(&home.join("config.toml"), updated.as_bytes())?;
    Ok(())
}

fn parse_toml(contents: &str) -> anyhow::Result<DocumentMut> {
    let contents = contents.trim_start_matches('\u{feff}');
    if contents.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        contents
            .parse::<DocumentMut>()
            .map_err(|e| anyhow::anyhow!("config.toml 解析失败：{e}"))
    }
}

fn resolve_path(home: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        home.join(path)
    }
}

fn ensure_trailing_newline(mut s: String) -> String {
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}
