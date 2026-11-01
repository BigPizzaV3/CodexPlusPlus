//! 随 Codex++ 一起发布的官方拓展包（来源 `bundled`，脚本 key 为 `bundled:<目录名>`）。
//!
//! 包的文件嵌在程序里，第一次用到用户脚本管理器时写到配置目录的 `bundled_extensions/` 下（内容没变不重写），
//! 之后和别的拓展包一样被扫描、启停、热重载。用户不能删，可以停用。
//!
//! 目前有五个，都是原来的内置功能整体改成的拓展，页面逻辑只用公开拓展接口：
//! - 「Agent 桌宠」（`agent-pet`）：图片桌宠 + 大肥鱼；Codex++ 核心只留数据服务（图片 / 音效下载、大肥鱼的余额、用量与图库）。
//! - 「打字特效」（`typing-effects`）：纯页面功能，不用任何后端路由。
//! - 「背景图」（`image-overlay`）：核心只留读图路由 `/image-overlay/image`（只读本拓展配置里的那张本机图片）。
//! - 「用量状态栏」（`usage-statusline`）：取数走拓展白名单里的只读路由 `/thread-usage-live`。
//! - 「请求状态条」（`proxy-request-status`）：取数走拓展白名单里的只读路由 `/proxy-status`。
use std::path::Path;

use anyhow::Context;
use serde_json::{Map, Value, json};

/// 官方拓展包的来源名，也是脚本 key 的前缀。
pub const BUNDLED_SOURCE: &str = "bundled";
/// Agent 桌宠拓展包的目录名与脚本 key。
pub const AGENT_PET_DIR: &str = "agent-pet";
pub const AGENT_PET_KEY: &str = "bundled:agent-pet";
/// 打字特效拓展包。
pub const TYPING_EFFECTS_DIR: &str = "typing-effects";
pub const TYPING_EFFECTS_KEY: &str = "bundled:typing-effects";
/// 背景图拓展包。
pub const IMAGE_OVERLAY_DIR: &str = "image-overlay";
pub const IMAGE_OVERLAY_KEY: &str = "bundled:image-overlay";
/// 用量状态栏拓展包。
pub const USAGE_STATUSLINE_DIR: &str = "usage-statusline";
pub const USAGE_STATUSLINE_KEY: &str = "bundled:usage-statusline";
/// 请求状态条拓展包。
pub const PROXY_REQUEST_STATUS_DIR: &str = "proxy-request-status";
pub const PROXY_REQUEST_STATUS_KEY: &str = "bundled:proxy-request-status";

/// 改成官方拓展包的内置拓展：(原内置拓展 id, 接替它的官方拓展包 key)。第三方拓展包清单里
/// `replaces` 写原 id 仍然有效，它启用时对应的官方拓展包让位（见 `UserScriptManager` 的扫描）。
pub const RETIRED_BUILTINS: &[(&str, &str)] = &[
    ("usage-statusline", USAGE_STATUSLINE_KEY),
    ("proxy-request-status", PROXY_REQUEST_STATUS_KEY),
];

/// 原内置拓展 id 现在由哪个官方拓展包接替。
pub fn retired_builtin_key(id: &str) -> Option<&'static str> {
    RETIRED_BUILTINS.iter().find(|(retired, _)| *retired == id).map(|(_, key)| *key)
}

const TYPING_EFFECTS_MANIFEST: &str = include_str!("../../../assets/extensions/typing-effects/codexpp.json");
const TYPING_EFFECTS_ENGINE: &str = include_str!("../../../assets/extensions/typing-effects/effects.js");
const TYPING_EFFECTS_MAIN: &str = include_str!("../../../assets/extensions/typing-effects/main.js");
const IMAGE_OVERLAY_MANIFEST: &str = include_str!("../../../assets/extensions/image-overlay/codexpp.json");
const IMAGE_OVERLAY_ENGINE: &str = include_str!("../../../assets/extensions/image-overlay/overlay.js");
const IMAGE_OVERLAY_MAIN: &str = include_str!("../../../assets/extensions/image-overlay/main.js");
const USAGE_STATUSLINE_MANIFEST: &str = include_str!("../../../assets/extensions/usage-statusline/codexpp.json");
const USAGE_STATUSLINE_ENGINE: &str = include_str!("../../../assets/extensions/usage-statusline/statusline.js");
const USAGE_STATUSLINE_MAIN: &str = include_str!("../../../assets/extensions/usage-statusline/main.js");
const PROXY_REQUEST_STATUS_MANIFEST: &str = include_str!("../../../assets/extensions/proxy-request-status/codexpp.json");
const PROXY_REQUEST_STATUS_ENGINE: &str = include_str!("../../../assets/extensions/proxy-request-status/status.js");
const PROXY_REQUEST_STATUS_MAIN: &str = include_str!("../../../assets/extensions/proxy-request-status/main.js");

const AGENT_PET_MANIFEST: &str = include_str!("../../../assets/extensions/agent-pet/codexpp.json");
const AGENT_PET_PET: &str = include_str!("../../../assets/extensions/agent-pet/pet.js");
const AGENT_PET_WHALE_COMPAT: &str = include_str!("../../../assets/extensions/agent-pet/whale-compat.js");
const AGENT_PET_WHALE: &str = include_str!("../../../assets/extensions/agent-pet/whale.js");
const AGENT_PET_MAIN: &str = include_str!("../../../assets/extensions/agent-pet/main.js");
/// 「拓展」详情页「细节」页签显示的说明。
const AGENT_PET_README: &str = include_str!("../../../assets/extensions/agent-pet/README.md");
/// 设置界面：Codex 里经 renderer 加载，管理器里内联进 settings.html（沙箱页面拿不到包里的其他文件）。
const AGENT_PET_SETTINGS: &str = include_str!("../../../assets/extensions/agent-pet/settings.js");
const AGENT_PET_SETTINGS_PAGE: &str = include_str!("../../../assets/extensions/agent-pet/settings-page.js");
const AGENT_PET_SETTINGS_HTML: &str = include_str!("../../../assets/extensions/agent-pet/settings.html");
const AGENT_PET_SETTINGS_PLACEHOLDER: &str = "/*__AGENT_PET_SETTINGS_SCRIPTS__*/";
/// 大肥鱼的上游引擎（原样保留，许可见 `assets/inject/upstream/whale-widget/PROVENANCE.md`）。
const WHALE_ENGINE: &str = include_str!("../../../assets/inject/upstream/whale-widget/full-widget-codex.js");
const AGENT_PET_ASSETS: &[(&str, &[u8])] = &[
    ("assets/icon.svg", include_bytes!("../../../assets/extensions/agent-pet/assets/icon.svg")),
    ("assets/sounds/Ya1.mp3", include_bytes!("../../../assets/inject/agent-pet/sounds/Ya1.mp3")),
    ("assets/sounds/Ya2.mp3", include_bytes!("../../../assets/inject/agent-pet/sounds/Ya2.mp3")),
    ("assets/sounds/D1.mp3", include_bytes!("../../../assets/inject/agent-pet/sounds/D1.mp3")),
    ("assets/sounds/D2.mp3", include_bytes!("../../../assets/inject/agent-pet/sounds/D2.mp3")),
];

/// 引擎包成一个函数声明：上游引擎用词法宿主（参数就是被代理的 window、document、定时器……），
/// 页面不必开放 unsafe-eval。和其余页面脚本拼在同一个拓展作用域里，由 whale.js 调用。
fn whale_engine_source() -> String {
    format!(
        "  function codexPlusWhaleEngine(window,document,fetch,localStorage,setTimeout,clearTimeout,setInterval,clearInterval,requestAnimationFrame,cancelAnimationFrame,MutationObserver,ResizeObserver,AudioContext,webkitAudioContext,Image,Audio) {{\n{WHALE_ENGINE}\n  }}\n"
    )
}

/// 管理器里的设置页：模板里的占位换成设置界面与管理器适配脚本。
pub fn agent_pet_settings_html() -> String {
    AGENT_PET_SETTINGS_HTML.replace(AGENT_PET_SETTINGS_PLACEHOLDER, &format!("{AGENT_PET_SETTINGS}\n{AGENT_PET_SETTINGS_PAGE}"))
}
/// Agent 桌宠拓展包的全部文件：(包内相对路径, 内容)。
pub fn agent_pet_files() -> Vec<(&'static str, Vec<u8>)> {
    let mut files = vec![
        ("codexpp.json", AGENT_PET_MANIFEST.as_bytes().to_vec()),
        ("pet.js", AGENT_PET_PET.as_bytes().to_vec()),
        ("whale-compat.js", AGENT_PET_WHALE_COMPAT.as_bytes().to_vec()),
        ("whale-engine.js", whale_engine_source().into_bytes()),
        ("whale.js", AGENT_PET_WHALE.as_bytes().to_vec()),
        ("settings.js", AGENT_PET_SETTINGS.as_bytes().to_vec()),
        ("settings.html", agent_pet_settings_html().into_bytes()),
        ("main.js", AGENT_PET_MAIN.as_bytes().to_vec()),
        ("README.md", AGENT_PET_README.as_bytes().to_vec()),
    ];
    files.extend(AGENT_PET_ASSETS.iter().map(|(path, bytes)| (*path, bytes.to_vec())));
    files
}

/// 打字特效拓展包的全部文件。
pub fn typing_effects_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("codexpp.json", TYPING_EFFECTS_MANIFEST.as_bytes().to_vec()),
        ("effects.js", TYPING_EFFECTS_ENGINE.as_bytes().to_vec()),
        ("main.js", TYPING_EFFECTS_MAIN.as_bytes().to_vec()),
    ]
}

/// 背景图拓展包的全部文件。
pub fn image_overlay_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("codexpp.json", IMAGE_OVERLAY_MANIFEST.as_bytes().to_vec()),
        ("overlay.js", IMAGE_OVERLAY_ENGINE.as_bytes().to_vec()),
        ("main.js", IMAGE_OVERLAY_MAIN.as_bytes().to_vec()),
    ]
}

/// 用量状态栏拓展包的全部文件。
pub fn usage_statusline_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("codexpp.json", USAGE_STATUSLINE_MANIFEST.as_bytes().to_vec()),
        ("statusline.js", USAGE_STATUSLINE_ENGINE.as_bytes().to_vec()),
        ("main.js", USAGE_STATUSLINE_MAIN.as_bytes().to_vec()),
    ]
}

/// 请求状态条拓展包的全部文件。
pub fn proxy_request_status_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("codexpp.json", PROXY_REQUEST_STATUS_MANIFEST.as_bytes().to_vec()),
        ("status.js", PROXY_REQUEST_STATUS_ENGINE.as_bytes().to_vec()),
        ("main.js", PROXY_REQUEST_STATUS_MAIN.as_bytes().to_vec()),
    ]
}

/// 全部官方拓展包：(目录名, 包内文件)。
pub fn packages() -> Vec<(&'static str, Vec<(&'static str, Vec<u8>)>)> {
    vec![
        (AGENT_PET_DIR, agent_pet_files()),
        (TYPING_EFFECTS_DIR, typing_effects_files()),
        (IMAGE_OVERLAY_DIR, image_overlay_files()),
        (USAGE_STATUSLINE_DIR, usage_statusline_files()),
        (PROXY_REQUEST_STATUS_DIR, proxy_request_status_files()),
    ]
}

/// 把官方拓展包写到 `directory`（`<配置目录>/bundled_extensions`）下；内容没变的文件不重写。
pub fn materialize(directory: &Path) -> anyhow::Result<()> {
    for (package, files) in packages() {
        let root = directory.join(package);
        for (relative, content) in files {
            let path = root.join(relative);
            if std::fs::read(&path).is_ok_and(|existing| existing == content) {
                continue;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).with_context(|| format!("无法创建 {}", parent.display()))?;
            }
            crate::settings::atomic_write(&path, &content).with_context(|| format!("无法写入 {}", path.display()))?;
        }
    }
    Ok(())
}

/// 原来的内置桌宠设置（`agentPet` 与 `codexAppWhaleBalance*`）换算成桌宠拓展的配置。
pub fn agent_pet_config_from_settings(settings: &crate::settings::BackendSettings) -> Map<String, Value> {
    let pet = &settings.agent_pet;
    let value = json!({
        "kind": if pet.is_whale_widget() { "whaleWidget" } else { "image" },
        "image": pet.image,
        "customImageUrl": pet.custom_image_url,
        "size": pet.size,
        "soundEnabled": pet.sound_enabled,
        "soundSet": pet.sound_set,
        "customSoundPress": pet.custom_sound_press,
        "customSoundRelease": pet.custom_sound_release,
        "volume": pet.volume,
        "codexAppWhaleBalanceProtocol": settings.codex_app_whale_balance_protocol,
        "codexAppWhaleBalancePath": settings.codex_app_whale_balance_path,
        "codexAppWhaleBalanceField": settings.codex_app_whale_balance_field,
        "codexAppWhaleBalanceCurrency": settings.codex_app_whale_balance_currency,
        "codexAppWhaleBalanceScale": settings.codex_app_whale_balance_scale,
    });
    value.as_object().cloned().unwrap_or_default()
}

/// 原来的打字特效设置（`codexAppTypingEffect`）换算成打字特效拓展的 (配置, 是否启用)：
/// 选过特效就启用并沿用那个特效；原来是关闭（默认）就保持停用，配置用默认的彩虹。
pub fn typing_effects_migration(settings: &crate::settings::BackendSettings) -> (Map<String, Value>, bool) {
    let effect = settings.codex_app_typing_effect.as_str();
    let enabled = matches!(effect, "rainbow" | "fireworks" | "stars");
    let mut config = Map::new();
    config.insert("effect".into(), json!(if enabled { effect } else { "rainbow" }));
    (config, enabled)
}

/// 原来的背景图设置（`codexAppImageOverlay*`）换算成背景图拓展的 (配置, 是否启用)：开着且选了图片才启用。
pub fn image_overlay_migration(settings: &crate::settings::BackendSettings) -> (Map<String, Value>, bool) {
    let path = settings.codex_app_image_overlay_path.trim();
    let enabled = settings.codex_app_image_overlay_enabled && !path.is_empty();
    let value = json!({
        "imagePath": path,
        "opacity": settings.codex_app_image_overlay_opacity.clamp(1, 100),
        "fitMode": settings.codex_app_image_overlay_fit_mode,
    });
    (value.as_object().cloned().unwrap_or_default(), enabled)
}

/// 原来的用量状态栏开关（`codexAppUsageStatusLine`，默认开）换算成用量状态栏拓展的 (配置, 是否启用)。没有配置项。
pub fn usage_statusline_migration(settings: &crate::settings::BackendSettings) -> (Map<String, Value>, bool) {
    (Map::new(), settings.codex_app_usage_status_line)
}

/// 原来的请求状态条开关（`codexAppProxyRequestStatus`，默认开）换算成请求状态条拓展的 (配置, 是否启用)。没有配置项。
pub fn proxy_request_status_migration(settings: &crate::settings::BackendSettings) -> (Map<String, Value>, bool) {
    (Map::new(), settings.codex_app_proxy_request_status)
}

/// 背景图最大读多大：图片整张以 data URI 经桥接交给页面，过大的图会拖慢 Codex。
const IMAGE_OVERLAY_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// `/image-overlay/image`：读背景图拓展配置里的那张本机图片，返回 `{ status, dataUrl }`。
/// 路径只来自本拓展已保存的配置，不接受页面传入，页面借这条路由读不到别的文件。
pub fn image_overlay_image(manager: &crate::user_scripts::UserScriptManager) -> Value {
    let config = match manager.effective_extension_config(IMAGE_OVERLAY_KEY) {
        Ok(config) => config,
        Err(error) => return json!({ "status": "failed", "message": format!("{error:#}") }),
    };
    let path = config["imagePath"].as_str().unwrap_or_default().trim().to_string();
    if path.is_empty() {
        return json!({ "status": "empty", "dataUrl": "" });
    }
    let path = Path::new(&path);
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() <= IMAGE_OVERLAY_MAX_BYTES => {}
        Ok(meta) if meta.is_file() => return json!({ "status": "failed", "message": "图片超过 64 MiB" }),
        _ => return json!({ "status": "failed", "message": "找不到图片文件" }),
    }
    match crate::assets::image_file_data_uri(path) {
        Some(data_url) => json!({ "status": "ok", "dataUrl": data_url }),
        None => json!({ "status": "failed", "message": "只支持 png / jpg / webp / gif / bmp 图片" }),
    }
}
