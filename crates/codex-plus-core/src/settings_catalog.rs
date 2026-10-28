//! 增强设置项清单。
//!
//! 管理器「Codex → 增强」页和注入 Codex 的「Codex++」设置页共用这一份清单
//! （`assets/enhancement-settings.json`）：标题、说明、分组、控件类型、平台限制、
//! 是否受增强总开关控制、在哪一侧显示都只在这里定义一次。
//! - 管理器在构建时直接 import 这份 JSON；
//! - 注入页通过只读路由 `/settings/catalog` 取得它，再按清单生成设置行，
//!   值仍走 `/settings/get`、`/settings/set`。

use serde_json::Value;

/// 清单原文。字段含义见 apps/codex-plus-manager/src/enhancement-catalog.ts 的类型说明。
pub const ENHANCEMENT_SETTINGS_CATALOG_JSON: &str =
    include_str!("../assets/enhancement-settings.json");

/// 解析后的清单，`/settings/catalog` 原样返回。
pub fn enhancement_settings_catalog() -> anyhow::Result<Value> {
    Ok(serde_json::from_str(ENHANCEMENT_SETTINGS_CATALOG_JSON)?)
}
