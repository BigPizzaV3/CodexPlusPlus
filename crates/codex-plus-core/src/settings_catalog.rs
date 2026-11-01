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

/// 一个内置拓展的定义。清单里带 `builtin` 的设置项各对应一个（名称、说明、分组取自设置项），
/// 没有设置项的放在清单顶层 `builtins` 里。
#[derive(Debug, Clone)]
pub struct BuiltinExtensionDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub group: String,
    /// 启停读写的设置字段（BackendSettings 的 JSON key，嵌套用点号，如 `agentPet.enabled`）。
    /// 没有时启停记在用户脚本配置 `scripts["builtin:<id>"]` 里。
    pub setting: Option<String>,
    /// 打开 / 关闭时写入设置字段的值；开关型是 true / false，下拉型（打字特效）由清单给出。
    pub on_value: Value,
    pub off_value: Value,
    /// 改了要重启 Codex 才生效。
    pub restart: bool,
    pub platforms: Vec<String>,
    /// 受增强总开关控制（独立模式补丁不受）。
    pub master_gated: bool,
    /// 能否被拓展包的 `replaces` 顶替。Rust 侧（启动、注入、本地代理）生效的功能经 `builtin_takeover` 按顶替状态让位；
    /// 只有拓展根本不加载时才生效的功能（独立模式的官方额度解锁）标 false。
    pub replaceable: bool,
    /// 归在这个拓展下的子选项（清单里 `parent` 指向它的设置项 key）。
    pub params: Vec<String>,
}

impl BuiltinExtensionDef {
    /// 设置里的值是否算「开着」。
    pub fn is_on(&self, value: Option<&Value>) -> bool {
        match value {
            None | Some(Value::Null) | Some(Value::Bool(false)) => false,
            Some(value) => *value != self.off_value,
        }
    }
}

/// 全部内置拓展，按清单顺序：先设置项里的，再顶层 `builtins`。
pub fn builtin_extension_defs() -> &'static [BuiltinExtensionDef] {
    static DEFS: std::sync::OnceLock<Vec<BuiltinExtensionDef>> = std::sync::OnceLock::new();
    DEFS.get_or_init(|| {
        let catalog = enhancement_settings_catalog().expect("enhancement-settings.json 解析失败");
        let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let items = catalog["items"].as_array().cloned().unwrap_or_default();
        let params_of = |id: &str| {
            items
                .iter()
                .filter(|item| item.get("parent").and_then(Value::as_str) == Some(id))
                .map(|item| text(item, "key"))
                .collect::<Vec<_>>()
        };
        let mut defs = Vec::new();
        for item in &items {
            let Some(id) = item.get("builtin").and_then(Value::as_str) else {
                continue;
            };
            defs.push(BuiltinExtensionDef {
                id: id.to_string(),
                name: text(item, "title"),
                description: text(item, "description"),
                group: text(item, "group"),
                setting: Some(text(item, "key")),
                on_value: item.get("builtinOn").cloned().unwrap_or(Value::Bool(true)),
                off_value: item.get("builtinOff").cloned().unwrap_or(Value::Bool(false)),
                restart: item.get("restart").and_then(Value::as_bool).unwrap_or(false),
                platforms: item
                    .get("platforms")
                    .and_then(Value::as_array)
                    .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default(),
                master_gated: item.get("masterGated").and_then(Value::as_bool).unwrap_or(true),
                replaceable: item.get("replaceable").and_then(Value::as_bool).unwrap_or(true),
                params: params_of(id),
            });
        }
        for entry in catalog["builtins"].as_array().cloned().unwrap_or_default() {
            let id = text(&entry, "id");
            defs.push(BuiltinExtensionDef {
                params: params_of(&id),
                id,
                name: text(&entry, "title"),
                description: text(&entry, "description"),
                group: text(&entry, "group"),
                setting: entry.get("setting").and_then(Value::as_str).map(str::to_string),
                on_value: Value::Bool(true),
                off_value: Value::Bool(false),
                restart: entry.get("restart").and_then(Value::as_bool).unwrap_or(false),
                platforms: Vec::new(),
                master_gated: true,
                replaceable: entry.get("replaceable").and_then(Value::as_bool).unwrap_or(true),
            });
        }
        defs
    })
}

/// 按 id 找内置拓展定义。
pub fn builtin_extension_def(id: &str) -> Option<&'static BuiltinExtensionDef> {
    builtin_extension_defs().iter().find(|def| def.id == id)
}
