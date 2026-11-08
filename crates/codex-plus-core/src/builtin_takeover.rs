//! 在 Rust 侧（启动、注入、本地代理、启动器驱动）生效的内置拓展，被拓展包用 `replaces` 顶替时让位。
//!
//! 页面侧的内置拓展由页面运行时按下发的 `replacedBy` 卸下；这里管页面之外的那些：
//! 读设置决定是否生效的地方先过一遍 [`mask_settings`]，被顶替的内置拓展对应的设置项按「关」处理，
//! 功能就和用户在管理器里关掉它一样不生效，交给顶替它的拓展（页面最早期脚本 `preload`、启动前后脚本
//! `launch`、页面脚本、网关中间件……）自己实现。
//!
//! 拓展只在增强总开关开着时加载，所以只有增强开着时才按顶替处理；关着时内置功能照旧（例如只在独立模式
//! 生效的官方额度解锁不可顶替，清单里标 `replaceable: false`）。
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::settings::BackendSettings;

/// 顶替状态的缓存：启动器、本地代理每次读都扫拓展目录太重，3 秒足够让启停拓展后很快生效。
const CACHE_TTL: Duration = Duration::from_secs(3);
static CACHE: Mutex<Option<(Instant, BTreeSet<String>)>> = Mutex::new(None);

/// 被启用中的拓展包顶替的内置拓展 id。
pub fn replaced_builtins() -> BTreeSet<String> {
    let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((at, set)) = cache.as_ref()
        && at.elapsed() < CACHE_TTL
    {
        return set.clone();
    }
    let set: BTreeSet<String> = crate::user_scripts::default_user_script_manager()
        .builtin_extension_state()
        .ok()
        .and_then(|state| state.get("replacedBy").and_then(|value| value.as_object()).map(|map| map.keys().cloned().collect()))
        .unwrap_or_default();
    *cache = Some((Instant::now(), set.clone()));
    set
}

/// 这个内置拓展是否被顶替（3 秒缓存）。
pub fn replaced(id: &str) -> bool {
    replaced_builtins().contains(id)
}

/// 拓展启停后立刻按新状态判断（管理器保存后调用）。
pub fn invalidate() {
    *CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// 把被顶替的内置拓展对应的设置项按「关」处理后的设置。没有顶替、或增强总开关关着时原样返回。
pub fn mask_settings(settings: BackendSettings) -> BackendSettings {
    if !settings.enhancements_enabled {
        return settings;
    }
    mask_settings_with(settings, &replaced_builtins())
}

/// 同 [`mask_settings`]，顶替集合由调用方给（测试用）。
pub fn mask_settings_with(mut settings: BackendSettings, replaced: &BTreeSet<String>) -> BackendSettings {
    if replaced.is_empty() || !settings.enhancements_enabled {
        return settings;
    }
    for id in replaced {
        switch_off(&mut settings, id);
    }
    settings
}

/// Rust 侧生效的内置拓展：id → 对应的设置项。页面侧的内置拓展由页面运行时卸下，不在这里。
pub const RUST_SIDE_BUILTINS: &[&str] = &[
    "native-browser-identification",
    "model-whitelist-unlock",
    "native-fast-unlock",
    "native-predictions",
    "reported-usage-compaction",
    "reasoning-collapse",
    "disable-work-view",
    "force-chinese-locale",
    "history-turn-recovery",
    "pet-real-mouse",
];

fn switch_off(settings: &mut BackendSettings, id: &str) {
    match id {
        "native-browser-identification" => settings.codex_app_native_browser_require_identification = false,
        "model-whitelist-unlock" => settings.codex_app_model_whitelist_unlock = false,
        "native-fast-unlock" => settings.codex_app_native_fast_unlock = false,
        "native-predictions" => settings.codex_app_native_predictions = false,
        "reported-usage-compaction" => settings.codex_app_reported_usage_compaction = false,
        "reasoning-collapse" => settings.codex_app_third_party_reasoning_collapse = false,
        "disable-work-view" => settings.codex_app_disable_work_view = false,
        "force-chinese-locale" => settings.codex_app_force_chinese_locale = false,
        "history-turn-recovery" => settings.codex_app_history_turn_recovery = false,
        "pet-real-mouse" => settings.codex_app_pet_real_mouse_look = false,
        _ => {}
    }
}

/// 读设置文件并按顶替状态处理（注入、桥接等直接读设置的地方用）。
pub fn load_effective_settings() -> anyhow::Result<BackendSettings> {
    crate::settings::SettingsStore::default().load().map(mask_settings)
}
