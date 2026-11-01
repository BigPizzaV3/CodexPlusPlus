//! 随 Codex++ 一起发布的官方拓展包（来源 `bundled`，脚本 key 为 `bundled:<目录名>`）。
//!
//! 包的文件嵌在程序里，第一次用到用户脚本管理器时写到配置目录的 `bundled_extensions/` 下（内容没变不重写），
//! 之后和别的拓展包一样被扫描、启停、热重载。用户不能删，可以停用。
//!
//! 目前只有「Agent 桌宠」（`agent-pet`）：原来的内置桌宠（图片桌宠 + 大肥鱼）整体改成了这个拓展，
//! 页面逻辑只用公开拓展接口；Codex++ 核心只留数据服务（图片 / 音效下载、大肥鱼的余额、用量与图库）。
use std::path::Path;

use anyhow::Context;
use serde_json::{Map, Value, json};

/// 官方拓展包的来源名，也是脚本 key 的前缀。
pub const BUNDLED_SOURCE: &str = "bundled";
/// Agent 桌宠拓展包的目录名与脚本 key。
pub const AGENT_PET_DIR: &str = "agent-pet";
pub const AGENT_PET_KEY: &str = "bundled:agent-pet";

const AGENT_PET_MANIFEST: &str = include_str!("../../../assets/extensions/agent-pet/codexpp.json");
const AGENT_PET_PET: &str = include_str!("../../../assets/extensions/agent-pet/pet.js");
const AGENT_PET_WHALE_COMPAT: &str = include_str!("../../../assets/extensions/agent-pet/whale-compat.js");
const AGENT_PET_WHALE: &str = include_str!("../../../assets/extensions/agent-pet/whale.js");
const AGENT_PET_MAIN: &str = include_str!("../../../assets/extensions/agent-pet/main.js");
/// 大肥鱼的上游引擎（原样保留，许可见 `assets/inject/upstream/whale-widget/PROVENANCE.md`）。
const WHALE_ENGINE: &str = include_str!("../../../assets/inject/upstream/whale-widget/full-widget-codex.js");
const AGENT_PET_SOUNDS: &[(&str, &[u8])] = &[
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

/// Agent 桌宠拓展包的全部文件：(包内相对路径, 内容)。
pub fn agent_pet_files() -> Vec<(&'static str, Vec<u8>)> {
    let mut files = vec![
        ("codexpp.json", AGENT_PET_MANIFEST.as_bytes().to_vec()),
        ("pet.js", AGENT_PET_PET.as_bytes().to_vec()),
        ("whale-compat.js", AGENT_PET_WHALE_COMPAT.as_bytes().to_vec()),
        ("whale-engine.js", whale_engine_source().into_bytes()),
        ("whale.js", AGENT_PET_WHALE.as_bytes().to_vec()),
        ("main.js", AGENT_PET_MAIN.as_bytes().to_vec()),
    ];
    files.extend(AGENT_PET_SOUNDS.iter().map(|(path, bytes)| (*path, bytes.to_vec())));
    files
}

/// 把官方拓展包写到 `directory`（`<配置目录>/bundled_extensions`）下；内容没变的文件不重写。
pub fn materialize(directory: &Path) -> anyhow::Result<()> {
    let root = directory.join(AGENT_PET_DIR);
    for (relative, content) in agent_pet_files() {
        let path = root.join(relative);
        if std::fs::read(&path).is_ok_and(|existing| existing == content) {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("无法创建 {}", parent.display()))?;
        }
        crate::settings::atomic_write(&path, &content).with_context(|| format!("无法写入 {}", path.display()))?;
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
