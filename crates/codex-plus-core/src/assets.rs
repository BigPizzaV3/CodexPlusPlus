use base64::Engine;
use serde_json::{Value, json};
use std::path::Path;

use crate::settings::BackendSettings;

const RENDERER_SCRIPT: &str = include_str!("../../../assets/inject/renderer-inject.js");
const WHALE_DEFAULT_IMAGE: &[u8] =
    include_bytes!("../../../assets/inject/upstream/whale-widget/DSniang1.png");
const WHALE_RUNTIME: &str =
    include_str!("../../../assets/inject/upstream/whale-widget/full-widget-codex.js");
const WHALE_MEDIA: &[(&str, &str, &[u8])] = &[
    ("DSniang1.png", "image/png", WHALE_DEFAULT_IMAGE),
    (
        "Ya1.mp3",
        "audio/mpeg",
        include_bytes!("../../../assets/inject/upstream/whale-widget/Ya1.mp3"),
    ),
    (
        "Ya2.mp3",
        "audio/mpeg",
        include_bytes!("../../../assets/inject/upstream/whale-widget/Ya2.mp3"),
    ),
    (
        "D1.mp3",
        "audio/mpeg",
        include_bytes!("../../../assets/inject/upstream/whale-widget/D1.mp3"),
    ),
    (
        "D2.mp3",
        "audio/mpeg",
        include_bytes!("../../../assets/inject/upstream/whale-widget/D2.mp3"),
    ),
    (
        "minecraft-exp-orb.wav",
        "audio/wav",
        include_bytes!("../../../assets/inject/upstream/whale-widget/minecraft-exp-orb.wav"),
    ),
    (
        "task-end-a.wav",
        "audio/wav",
        include_bytes!("../../../assets/inject/upstream/whale-widget/task-end-a.wav"),
    ),
    (
        "bubble-petpet.gif",
        "image/gif",
        include_bytes!("../../../assets/inject/upstream/whale-widget/bubble-petpet.gif"),
    ),
    (
        "bubble-money1.gif",
        "image/gif",
        include_bytes!("../../../assets/inject/upstream/whale-widget/bubble-money1.gif"),
    ),
    (
        "rua.gif",
        "image/gif",
        include_bytes!("../../../assets/inject/upstream/whale-widget/rua.gif"),
    ),
];

fn whale_assets_json() -> String {
    let values: serde_json::Map<String, Value> = WHALE_MEDIA
        .iter()
        .map(|(name, mime, bytes)| ((*name).to_string(), json!(image_data_uri(mime, bytes))))
        .collect();
    Value::Object(values).to_string()
}

fn whale_runtime_script() -> String {
    // 直接注入函数声明；完整引擎使用词法宿主，不要求 renderer 开放 unsafe-eval。
    format!(
        "window.__CODEX_PLUS_WHALE_ASSETS__ = {};\nwindow.__CODEX_PLUS_WHALE_ENGINE__ = function(window,document,fetch,localStorage,setTimeout,clearTimeout,setInterval,clearInterval,requestAnimationFrame,cancelAnimationFrame,MutationObserver,ResizeObserver,AudioContext,webkitAudioContext,Image,Audio) {{\n{}\n}};\n",
        whale_assets_json(),
        WHALE_RUNTIME
    )
}

#[cfg(windows)]
const DREAM_TARGET_CSS: &str =
    include_str!("../../../assets/inject/upstream/dream-skin/windows/dream-skin.css");
#[cfg(not(windows))]
const DREAM_TARGET_CSS: &str =
    include_str!("../../../assets/inject/upstream/dream-skin/macos/dream-skin.css");
#[cfg(windows)]
const DREAM_TARGET_RENDERER: &str =
    include_str!("../../../assets/inject/upstream/dream-skin/windows/renderer-inject.js");
#[cfg(not(windows))]
const DREAM_TARGET_RENDERER: &str =
    include_str!("../../../assets/inject/upstream/dream-skin/macos/renderer-inject.js");
#[cfg(windows)]
const CIDALA_TARGET_CSS: &str =
    include_str!("../../../assets/inject/upstream/cidala-tiger/windows/dream-skin.css");
#[cfg(not(windows))]
const CIDALA_TARGET_CSS: &str =
    include_str!("../../../assets/inject/upstream/cidala-tiger/macos/dream-skin.css");
#[cfg(windows)]
const CIDALA_TARGET_RENDERER: &str =
    include_str!("../../../assets/inject/upstream/cidala-tiger/windows/renderer-inject.js");
#[cfg(not(windows))]
const CIDALA_TARGET_RENDERER: &str =
    include_str!("../../../assets/inject/upstream/cidala-tiger/macos/renderer-inject.js");
const CODEX_SNOW_CSS: &str =
    include_str!("../../../assets/inject/upstream/snow-skin/dream-skin.css");
const CODEX_SNOW_RENDERER: &str =
    include_str!("../../../assets/inject/upstream/snow-skin/renderer-inject.js");
const GLASS_VISION_CSS: &str =
    include_str!("../../../assets/inject/upstream/glass-vision/glass-vision.css");
const GLASS_VISION_RENDERER: &str =
    include_str!("../../../assets/inject/upstream/glass-vision/renderer-inject.js");
#[cfg(windows)]
const DREAM_SKIN_DEFAULT_IMAGE: &[u8] =
    include_bytes!("../../../assets/inject/upstream/dream-skin/windows/dream-reference.jpg");
#[cfg(not(windows))]
const DREAM_SKIN_DEFAULT_IMAGE: &[u8] =
    include_bytes!("../../../assets/inject/upstream/dream-skin/macos/portal-hero.png");
const PET_REAL_MOUSE_SCRIPT: &str = include_str!("../../../assets/inject/pet-real-mouse-inject.js");
const STEPWISE_SCRIPT: &str = concat!(
    "(() => {\n",
    include_str!("../../../assets/inject/floating-panel/runtime/state.js"),
    include_str!("../../../assets/inject/floating-panel/core/appearance-runtime.js"),
    include_str!("../../../assets/inject/floating-panel/runtime/dom.js"),
    include_str!("../../../assets/inject/floating-panel/runtime/bridge-client.js"),
    include_str!("../../../assets/inject/floating-panel/runtime/answer-context.js"),
    include_str!("../../../assets/inject/floating-panel/stepwise/suggestions.js"),
    include_str!("../../../assets/inject/floating-panel/stepwise/generation.js"),
    include_str!("../../../assets/inject/floating-panel/runtime/lifecycle.js"),
    include_str!("../../../assets/inject/floating-panel/runtime/settings.js"),
    include_str!("../../../assets/inject/floating-panel/core/appearance.js"),
    include_str!("../../../assets/inject/floating-panel/core/host.js"),
    include_str!("../../../assets/inject/floating-panel/core/geometry.js"),
    include_str!("../../../assets/inject/floating-panel/core/interaction.js"),
    include_str!("../../../assets/inject/floating-panel/core/views.js"),
    include_str!("../../../assets/inject/floating-panel/outline/parser.js"),
    include_str!("../../../assets/inject/floating-panel/outline/navigation.js"),
    include_str!("../../../assets/inject/floating-panel/outline/feature.js"),
    include_str!("../../../assets/inject/floating-panel/outline/view.js"),
    include_str!("../../../assets/inject/floating-panel/core/scroll-state.js"),
    include_str!("../../../assets/inject/floating-panel-inject.js"),
    "\n})();\n",
);
pub const DIAGNOSTIC_BUILD_ID: &str = "diag-20260518-1";
const DREAM_SKIN_RENDERER_REVISION: &str = "24-home-composer-rounded";

pub fn renderer_script() -> &'static str {
    RENDERER_SCRIPT
}

pub fn dream_skin_default_image() -> (&'static str, &'static [u8]) {
    #[cfg(windows)]
    return ("image/jpeg", DREAM_SKIN_DEFAULT_IMAGE);
    #[cfg(not(windows))]
    return ("image/png", DREAM_SKIN_DEFAULT_IMAGE);
}

pub fn dream_skin_art_data_uri(settings: &BackendSettings) -> String {
    if !settings.codex_app_dream_skin_enabled {
        return String::new();
    }
    let custom_path = settings.codex_app_dream_skin_image_path.trim();
    if !custom_path.is_empty()
        && crate::dream_skin::is_managed_dream_skin_image(
            Path::new(custom_path),
            &crate::paths::default_app_state_dir(),
        )
        && let Some(data_uri) = image_file_data_uri(Path::new(custom_path))
    {
        return data_uri;
    }
    let (content_type, image) = dream_skin_default_image();
    image_data_uri(content_type, image)
}

fn dream_skin_platform() -> &'static str {
    if cfg!(windows) { "windows" } else { "macos" }
}

fn uses_cidala_target_engine(style_preset: &str) -> bool {
    matches!(
        style_preset,
        "midnight-aurora" | "amber-dusk" | "forest-mist" | "cyber-neon" | "sakura-dawn"
    )
}

fn dream_skin_target_assets(
    settings: &BackendSettings,
) -> (&'static str, &'static str, &'static str) {
    let theme = &settings.codex_app_dream_skin_theme_config;
    let style_preset =
        crate::settings::resolve_dream_skin_style_preset(&theme.id, &theme.style_preset);
    match style_preset.as_str() {
        "codex-snow" => ("snow", CODEX_SNOW_RENDERER, CODEX_SNOW_CSS),
        "glass-vision" => ("glass-vision", GLASS_VISION_RENDERER, GLASS_VISION_CSS),
        value if uses_cidala_target_engine(value) => {
            ("cidala-tiger", CIDALA_TARGET_RENDERER, CIDALA_TARGET_CSS)
        }
        _ => ("dream-skin", DREAM_TARGET_RENDERER, DREAM_TARGET_CSS),
    }
}

fn dream_skin_target_runtime_script(settings: &BackendSettings, include_art: bool) -> String {
    if !settings.codex_app_dream_skin_enabled || settings.codex_app_dream_skin_paused {
        return String::new();
    }

    let (engine, renderer, base_css) = dream_skin_target_assets(settings);
    let managed_css = managed_dream_skin_css(settings);
    let css = format!("{base_css}\n{managed_css}");
    let theme = serde_json::to_string(&settings.codex_app_dream_skin_theme_config)
        .expect("dream skin target theme should serialize");
    let style_revision = dream_skin_content_signature(css.as_bytes());
    let payload_revision =
        dream_skin_target_payload_signature(settings, engine, &style_revision, &theme);
    let mut payload = renderer
        .replace("__DREAM_CSS_JSON__", &serde_json::to_string(&css).unwrap())
        .replace("__DREAM_ART_JSON__", "window.__CODEX_PLUS_DREAM_SKIN_ART__")
        .replace(
            "__DREAM_THEME_JSON__",
            "window.__CODEX_PLUS_DREAM_SKIN_THEME__",
        )
        .replace(
            "__DREAM_VERSION_JSON__",
            &serde_json::to_string("2.1.0-snow.1").unwrap(),
        )
        .replace(
            "__GLASS_VISION_CSS_JSON__",
            &serde_json::to_string(&css).unwrap(),
        )
        .replace(
            "__GLASS_VISION_ART_JSON__",
            "window.__CODEX_PLUS_DREAM_SKIN_ART__",
        )
        .replace(
            "__DREAM_SKIN_CSS_JSON__",
            &serde_json::to_string(&css).unwrap(),
        )
        .replace(
            "__DREAM_SKIN_ART_JSON__",
            "window.__CODEX_PLUS_DREAM_SKIN_ART__",
        )
        .replace(
            "__DREAM_SKIN_THEME_JSON__",
            "window.__CODEX_PLUS_DREAM_SKIN_THEME__",
        )
        .replace(
            "__DREAM_SKIN_VERSION_JSON__",
            &serde_json::to_string("1.2.0").unwrap(),
        )
        .replace(
            "__DREAM_SKIN_STYLE_REVISION_JSON__",
            &serde_json::to_string(&style_revision).unwrap(),
        )
        .replace(
            "__DREAM_SKIN_PAYLOAD_REVISION_JSON__",
            &serde_json::to_string(&payload_revision).unwrap(),
        );
    if payload.contains("__DREAM_") || payload.contains("__GLASS_VISION_") {
        panic!("dream skin target renderer contains unresolved placeholders");
    }

    let art_assignment = include_art.then(|| {
        format!(
            "window.__CODEX_PLUS_DREAM_SKIN_ART__ = {};\n",
            serde_json::to_string(&dream_skin_art_data_uri(settings))
                .expect("dream skin target art should serialize")
        )
    });
    let skin_api_bootstrap = dream_skin_skin_api_bootstrap_script(&theme);
    payload = format!(
        "(() => {{\nwindow.__CODEX_PLUS_EXTERNAL_DREAM_SKIN_RUNTIME__ = true;\nwindow.__CODEX_PLUS_CLEAR_DREAM_SKIN__?.();\n{}window.__CODEX_PLUS_DREAM_SKIN_ART_SIGNATURE__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_THEME__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_RUNTIME_REVISION__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_TARGET_ENGINE__ = {};\n{}const result = {};\nconst state = window.__CODEX_DREAM_SKIN_STATE__ || window.__CODEX_GLASS_VISION_SKIN_STATE__;\nif (state) {{\n  state.version = `codex-plus:${{String(window.__CODEX_PLUS_DREAM_SKIN_PLATFORM__ || 'unknown')}}:${{window.__CODEX_PLUS_DREAM_SKIN_TARGET_ENGINE__}}:r${{window.__CODEX_PLUS_DREAM_SKIN_RUNTIME_REVISION__}}`;\n}}\nwindow.__CODEX_PLUS_DREAM_SKIN_PAYLOAD_SIGNATURE__ = {};\nreturn result;\n}})()",
        art_assignment.unwrap_or_default(),
        serde_json::to_string(&dream_skin_art_content_signature(settings)).unwrap(),
        theme,
        serde_json::to_string(DREAM_SKIN_RENDERER_REVISION).unwrap(),
        serde_json::to_string(engine).unwrap(),
        skin_api_bootstrap,
        payload,
        serde_json::to_string(&payload_revision).unwrap(),
    );
    payload
}

fn managed_dream_skin_css(settings: &BackendSettings) -> String {
    let image_path = settings.codex_app_dream_skin_image_path.trim();
    if image_path.is_empty()
        || !crate::dream_skin::is_managed_dream_skin_image(
            Path::new(image_path),
            &crate::paths::default_app_state_dir(),
        )
    {
        return String::new();
    }
    let css_path = Path::new(image_path)
        .parent()
        .map(|parent| parent.join("current.css"));
    let Some(css_path) = css_path else {
        return String::new();
    };
    let Ok(css) = std::fs::read_to_string(css_path) else {
        return String::new();
    };
    crate::dream_skin_package::compile_safe_css(&css).unwrap_or_default()
}

fn dream_skin_skin_api_bootstrap_script(theme: &str) -> String {
    format!(
        r#"(() => {{
  const theme = {};
  const root = document.documentElement;
  const colors = theme && typeof theme.colors === "object" ? theme.colors : {{}};
  const variables = {{
    "--ds-theme-color-background": colors.background,
    "--ds-theme-color-panel": colors.panel,
    "--ds-theme-color-panel-alt": colors.panelAlt,
    "--ds-theme-color-accent": colors.accent,
    "--ds-theme-color-accent-alt": colors.accentAlt,
    "--ds-theme-color-secondary": colors.secondary,
    "--ds-theme-color-highlight": colors.highlight,
    "--ds-theme-color-text": colors.text,
    "--ds-theme-color-muted": colors.muted,
    "--ds-theme-color-line": colors.line,
    "--ds-theme-image-focus-x": String(theme?.art?.focusX ?? 0.5),
    "--ds-theme-image-focus-y": String(theme?.art?.focusY ?? 0.5),
  }};
  for (const [name, value] of Object.entries(variables)) if (typeof value === "string" && value) root.style.setProperty(name, value);
  const map = {{
    root: "html", sidebar: "aside.app-shell-left-panel", main: "main.main-surface",
    header: "header.app-header-tint", home: ".dream-skin-home, [data-feature='game-source']",
    "home-hero": ".dream-skin-home > div:first-child, [data-feature='game-source']",
    "project-list": "[data-feature='game-source']", thread: "main.main-surface [role='main']",
    message: "main.main-surface article", composer: ".composer-surface-chrome",
    "composer-toolbar": ".composer-surface-chrome [role='toolbar']", dialog: "[role='dialog']",
  }};
  const mark = () => {{
    for (const [part, selector] of Object.entries(map)) for (const node of document.querySelectorAll(selector)) {{
      // data-ds-part 是皮肤 API 的挂载点标记，值不变时绝不重写，避免长会话里对每条消息重复置属性
      if (node.getAttribute("data-ds-part") !== part) node.setAttribute("data-ds-part", part);
    }}
  }};
  mark();
  window.__CODEX_PLUS_DREAM_SKIN_API_OBSERVER__?.disconnect?.();
  let markTimer = null;
  const scheduleMark = () => {{
    if (markTimer !== null) return;
    markTimer = setTimeout(() => {{
      markTimer = null;
      mark();
    }}, 250);
  }};
  const observer = new MutationObserver((records) => {{
    // 流式输出只产生纯文本节点增删，不会增减皮肤挂载点；这类批次直接跳过（issue #2181）
    if (records.every((record) =>
      [...record.addedNodes].every((node) => node.nodeType === 3)
      && [...record.removedNodes].every((node) => node.nodeType === 3))) return;
    scheduleMark();
  }});
  observer.observe(document.documentElement, {{ childList: true, subtree: true }});
  window.__CODEX_PLUS_DREAM_SKIN_API_OBSERVER__ = observer;
}})();"#,
        theme,
    )
}

fn dream_skin_target_payload_signature(
    settings: &BackendSettings,
    engine: &str,
    style_revision: &str,
    theme: &str,
) -> String {
    dream_skin_content_signature(
        format!(
            "{engine}:{style_revision}:{}:{theme}",
            dream_skin_art_content_signature(settings)
        )
        .as_bytes(),
    )
}

pub fn stepwise_script() -> &'static str {
    STEPWISE_SCRIPT
}

pub fn pet_real_mouse_script() -> &'static str {
    PET_REAL_MOUSE_SCRIPT
}

const PET_V2_SPRITE_DETECTION_SCRIPT: &str = r#"
  const isV2Sprite = async (mascot) => {
    if (!mascot) return false;
    if (Array.from(mascot.querySelectorAll("img")).some((image) =>
      image.naturalWidth === 1536 && image.naturalHeight === 2288
    )) return true;
    for (const element of [mascot, ...mascot.querySelectorAll("*")]) {
      const background = getComputedStyle(element).backgroundImage || "";
      const match = background.match(/url\(["']?([^"')]+)/i);
      if (!match) continue;
      const source = match[1];
      const cacheKey = "__codexPlusPetV2SpriteProbe";
      let probe = window[cacheKey];
      if (!probe || probe.source !== source) {
        probe = { source, valid: false, pending: true };
        probe.promise = (async () => {
          try {
            const image = new Image();
            image.src = source;
            await image.decode();
            return image.naturalWidth === 1536 && image.naturalHeight === 2288;
          } catch {
            return false;
          }
        })().then((valid) => {
          probe.valid = valid;
          probe.pending = false;
          return valid;
        });
        window[cacheKey] = probe;
      }
      const wasPending = probe.pending;
      const valid = wasPending ? await probe.promise : probe.valid;
      if (wasPending) {
        const currentBackground = getComputedStyle(element).backgroundImage || "";
        const currentMatch = currentBackground.match(/url\(["']?([^"')]+)/i);
        if (currentMatch?.[1] !== source) continue;
      }
      if (window[cacheKey] === probe && valid) return true;
    }
    return false;
  };
"#;

pub fn pet_real_mouse_capability_probe_script() -> String {
    let mut script = String::from(
        r#"
(async () => {
  const mascot = document.querySelector('[data-avatar-mascot="true"]');
"#,
    );
    script.push_str(PET_V2_SPRITE_DETECTION_SCRIPT);
    script.push_str(
        r#"
  if (!await isV2Sprite(mascot)) return false;
  const urls = [
    ...Array.from(document.scripts || []).map((script) => script.src),
    ...Array.from(document.querySelectorAll("link[href]") || []).map((link) => link.href),
    ...performance.getEntriesByType("resource").map((entry) => entry.name),
  ].filter((url) => url && url.includes("/assets/") && url.split("?")[0].endsWith(".js"));
  let dispatcherUrl = urls.find((url) => url.includes("vscode-api-"));
  if (!dispatcherUrl) {
    for (const url of urls) {
      try {
        const source = await fetch(url).then((response) => response.ok ? response.text() : "");
        const match = source.match(/["'](\.\/(?:assets\/)?vscode-api-[^"']+\.js)["']/);
        if (match) {
          dispatcherUrl = new URL(match[1], url).href;
          break;
        }
      } catch {
      }
    }
  }
  if (!dispatcherUrl) return false;
  try {
    const module = await import(dispatcherUrl);
    return Object.values(module || {}).some((value) => value
      && typeof value.dispatchHostMessage === "function"
      && typeof value.subscribe === "function");
  } catch {
    return false;
  }
})()
"#,
    );
    script
}

pub fn pet_real_mouse_update_script(x: i32, y: i32) -> String {
    let mut script = String::from(
        r#"(async () => {
  const mascot = document.querySelector('[data-avatar-mascot="true"]');
"#,
    );
    script.push_str(PET_V2_SPRITE_DETECTION_SCRIPT);
    script.push_str(&format!(
        r#"
  return await isV2Sprite(mascot)
    && window.__codexPlusPetRealMouseLook?.updateScreenPoint?.({{ x: {x}, y: {y} }}) === true;
}})()"#
    ));
    script
}

pub fn pet_real_mouse_stop_script() -> &'static str {
    "window.__codexPlusPetRealMouseLook?.stop?.();"
}

pub fn injection_script(helper_port: u16) -> String {
    injection_script_with_settings(helper_port, &BackendSettings::default())
}

pub fn hide_official_usage_alert_config(settings: &BackendSettings) -> bool {
    let profile = settings.active_relay_profile();
    profile.relay_mode == crate::settings::RelayMode::Official && profile.official_mix_api_key
}

pub fn injection_script_with_settings(helper_port: u16, settings: &BackendSettings) -> String {
    let helper_url = format!("http://127.0.0.1:{helper_port}");
    let image_overlay = image_overlay_config(helper_port, settings);
    let dream_skin_art = dream_skin_art_data_uri(settings);
    let dream_skin_art_signature = dream_skin_art_content_signature(settings);
    let dream_skin_theme = &settings.codex_app_dream_skin_theme_config;
    let dream_skin_target_runtime = dream_skin_target_runtime_script(settings, false);
    let paste_fix = paste_fix_enabled_config(settings);
    let hide_official_usage_alert = hide_official_usage_alert_config(settings);
    let stepwise_runtime =
        if settings.codex_app_stepwise_enabled || settings.codex_app_answer_outline_enabled {
            stepwise_script()
        } else {
            ""
        };
    format!(
        "window.__CODEX_SESSION_DELETE_HELPER__ = {};\nwindow.__CODEX_PLUS_VERSION__ = {};\nwindow.__CODEX_PLUS_BUILD__ = {};\nwindow.__CODEX_PLUS_IMAGE_OVERLAY__ = {};\nwindow.__CODEX_PLUS_EXTERNAL_DREAM_SKIN_RUNTIME__ = true;\nwindow.__CODEX_PLUS_DREAM_SKIN_PLATFORM__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_REVISION__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_ART__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_ART_SIGNATURE__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_THEME__ = {};\nwindow.__CODEX_PLUS_PASTE_FIX__ = {};\nwindow.__CODEX_PLUS_HIDE_OFFICIAL_USAGE_ALERT__ = {};\n{}\n{}\n{}",
        serde_json::to_string(&helper_url).expect("helper URL should serialize"),
        serde_json::to_string(crate::version::VERSION).expect("version should serialize"),
        serde_json::to_string(DIAGNOSTIC_BUILD_ID).expect("build id should serialize"),
        serde_json::to_string(&image_overlay).expect("image overlay config should serialize"),
        serde_json::to_string(dream_skin_platform()).expect("dream skin platform should serialize"),
        serde_json::to_string(DREAM_SKIN_RENDERER_REVISION)
            .expect("dream skin renderer revision should serialize"),
        serde_json::to_string(&dream_skin_art).expect("dream skin art should serialize"),
        serde_json::to_string(&dream_skin_art_signature)
            .expect("dream skin art signature should serialize"),
        serde_json::to_string(dream_skin_theme).expect("dream skin theme should serialize"),
        serde_json::to_string(&paste_fix).expect("paste fix config should serialize"),
        serde_json::to_string(&hide_official_usage_alert)
            .expect("usage alert config should serialize"),
        format!(
            "window.__CODEX_PLUS_WHALE_IMAGE__ = {};\n{}\n{}\n{}",
            serde_json::to_string(&image_data_uri("image/png", WHALE_DEFAULT_IMAGE))
                .expect("whale image should serialize"),
            whale_runtime_script(),
            include_str!("../../../assets/inject/composer-readiness.js"),
            renderer_script()
        ),
        stepwise_runtime,
        dream_skin_target_runtime,
    )
}

pub fn dream_skin_live_update_probe_script() -> String {
    format!(
        "(() => {{ const state = window.__CODEX_DREAM_SKIN_STATE__ || window.__CODEX_GLASS_VISION_SKIN_STATE__; if (window.__CODEX_PLUS_DREAM_SKIN_RUNTIME_REVISION__ !== {} || !state) return null; state.ensure?.(); return JSON.stringify({{ artSignature: String(window.__CODEX_PLUS_DREAM_SKIN_ART_SIGNATURE__ || ''), payloadSignature: String(window.__CODEX_PLUS_DREAM_SKIN_PAYLOAD_SIGNATURE__ || '') }}); }})()",
        serde_json::to_string(DREAM_SKIN_RENDERER_REVISION)
            .expect("dream skin renderer revision should serialize")
    )
}

pub fn dream_skin_live_update_script(settings: &BackendSettings, include_art: bool) -> String {
    dream_skin_target_runtime_script(settings, include_art)
}

pub fn dream_skin_art_content_signature(settings: &BackendSettings) -> String {
    let custom_path = settings.codex_app_dream_skin_image_path.trim();
    if !custom_path.is_empty()
        && crate::dream_skin::is_managed_dream_skin_image(
            Path::new(custom_path),
            &crate::paths::default_app_state_dir(),
        )
        && let Ok(bytes) = std::fs::read(custom_path)
    {
        return dream_skin_content_signature(&bytes);
    }
    dream_skin_content_signature(DREAM_SKIN_DEFAULT_IMAGE)
}

pub fn dream_skin_runtime_content_signature(settings: &BackendSettings) -> String {
    let (engine, _, css) = dream_skin_target_assets(settings);
    let theme = serde_json::to_string(&settings.codex_app_dream_skin_theme_config)
        .expect("dream skin target theme should serialize");
    let style_revision = dream_skin_content_signature(css.as_bytes());
    dream_skin_target_payload_signature(settings, engine, &style_revision, &theme)
}

fn dream_skin_content_signature(value: &[u8]) -> String {
    let hash = value.iter().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(16_777_619)
    });
    format!("{}-{hash:x}", value.len())
}

pub fn image_overlay_config(helper_port: u16, settings: &BackendSettings) -> Value {
    let has_path = !settings.codex_app_image_overlay_path.trim().is_empty();
    let enabled = settings.codex_app_image_overlay_enabled && has_path;
    let data_url = if enabled {
        image_file_data_uri(Path::new(settings.codex_app_image_overlay_path.trim()))
            .unwrap_or_default()
    } else {
        String::new()
    };
    json!({
        "enabled": enabled && !data_url.is_empty(),
        "opacity": f64::from(settings.codex_app_image_overlay_opacity.clamp(1, 100)) / 100.0,
        "fitMode": settings.codex_app_image_overlay_fit_mode.as_str(),
        "dataUrl": data_url,
        "imageUrl": if enabled {
            format!("http://127.0.0.1:{helper_port}/overlay/image")
        } else {
            String::new()
        },
    })
}

pub fn paste_fix_enabled_config(settings: &BackendSettings) -> Value {
    json!({ "enabled": settings.codex_app_paste_fix })
}

fn image_data_uri(mime_type: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn image_file_data_uri(path: &Path) -> Option<String> {
    let mime_type = image_content_type(path)?;
    let bytes = std::fs::read(path).ok()?;
    Some(image_data_uri(mime_type, &bytes))
}

fn image_content_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_does_not_project_retired_plugin_cache() {
        let script = injection_script_with_settings(57321, &BackendSettings::default());
        assert!(!script.contains("window.__CODEX_PLUS_PLUGIN_MARKETPLACES__ ="));
    }

    #[test]
    fn whale_character_is_embedded_for_offline_and_live_enable() {
        let script = injection_script(57321);
        let prefix = "window.__CODEX_PLUS_WHALE_IMAGE__ = ";
        let value = script.split_once(prefix).unwrap().1.lines().next().unwrap();
        let uri: String = serde_json::from_str(value.trim_end_matches(';')).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(uri.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        assert_eq!(bytes, WHALE_DEFAULT_IMAGE);
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn whale_media_preserves_original_audio_and_animation_bytes() {
        let values: Value = serde_json::from_str(&whale_assets_json()).unwrap();
        assert_eq!(values.as_object().unwrap().len(), 10);
        for (name, mime, expected) in WHALE_MEDIA {
            let uri = values[*name].as_str().unwrap();
            let prefix = format!("data:{mime};base64,");
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(uri.strip_prefix(&prefix).unwrap())
                .unwrap();
            assert_eq!(&decoded, expected);
        }
        let script = whale_runtime_script();
        assert!(script.contains("window.__CODEX_PLUS_WHALE_ENGINE__ = function("));
        assert!(!script.contains("new Function("));
    }

    #[test]
    fn image_overlay_config_includes_fit_mode() {
        let settings = BackendSettings {
            codex_app_image_overlay_fit_mode: "fill".to_string(),
            ..BackendSettings::default()
        };
        let config = image_overlay_config(57321, &settings);

        assert_eq!(config["fitMode"].as_str(), Some("fill"));
    }
}
