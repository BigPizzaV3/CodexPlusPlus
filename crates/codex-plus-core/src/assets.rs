use base64::Engine;
use serde_json::Map;
use serde_json::{Value, json};
use std::path::Path;

use crate::settings::BackendSettings;

const RENDERER_SCRIPT: &str = include_str!("../../../assets/inject/renderer-inject.js");
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
const STEPWISE_SCRIPT: &str = include_str!("../../../assets/inject/stepwise-inject.js");
const SPONSOR_ALIPAY: &[u8] = include_bytes!("../../../assets/images/sponsor-alipay.jpg");
const SPONSOR_WECHAT: &[u8] = include_bytes!("../../../assets/images/sponsor-wechat.jpg");
pub const DIAGNOSTIC_BUILD_ID: &str = "diag-20260518-1";
const DREAM_SKIN_RENDERER_REVISION: &str = "15";

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

    let (engine, renderer, css) = dream_skin_target_assets(settings);
    let theme = serde_json::to_string(&settings.codex_app_dream_skin_theme_config)
        .expect("dream skin target theme should serialize");
    let style_revision = dream_skin_content_signature(css.as_bytes());
    let payload_revision =
        dream_skin_target_payload_signature(settings, engine, &style_revision, &theme);
    let mut payload = renderer
        .replace("__DREAM_CSS_JSON__", &serde_json::to_string(css).unwrap())
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
            &serde_json::to_string(css).unwrap(),
        )
        .replace(
            "__GLASS_VISION_ART_JSON__",
            "window.__CODEX_PLUS_DREAM_SKIN_ART__",
        )
        .replace(
            "__DREAM_SKIN_CSS_JSON__",
            &serde_json::to_string(css).unwrap(),
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
    payload = format!(
        "(() => {{\nwindow.__CODEX_PLUS_EXTERNAL_DREAM_SKIN_RUNTIME__ = true;\nwindow.__CODEX_PLUS_CLEAR_DREAM_SKIN__?.();\n{}window.__CODEX_PLUS_DREAM_SKIN_ART_SIGNATURE__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_THEME__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_RUNTIME_REVISION__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_TARGET_ENGINE__ = {};\nconst result = {};\nconst state = window.__CODEX_DREAM_SKIN_STATE__ || window.__CODEX_GLASS_VISION_SKIN_STATE__;\nif (state) {{\n  state.version = `codex-plus:${{String(window.__CODEX_PLUS_DREAM_SKIN_PLATFORM__ || 'unknown')}}:${{window.__CODEX_PLUS_DREAM_SKIN_TARGET_ENGINE__}}:r${{window.__CODEX_PLUS_DREAM_SKIN_RUNTIME_REVISION__}}`;\n  state.observer?.disconnect?.();\n  if (state.timer) clearInterval(state.timer);\n  state.observer = null;\n  state.timer = null;\n}}\nwindow.__CODEX_PLUS_DREAM_SKIN_PAYLOAD_SIGNATURE__ = {};\nreturn result;\n}})()",
        art_assignment.unwrap_or_default(),
        serde_json::to_string(&dream_skin_art_content_signature(settings)).unwrap(),
        theme,
        serde_json::to_string(DREAM_SKIN_RENDERER_REVISION).unwrap(),
        serde_json::to_string(engine).unwrap(),
        payload,
        serde_json::to_string(&payload_revision).unwrap(),
    );
    payload
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

pub fn sponsor_image_data_uris() -> Value {
    json!({
        "alipay": image_data_uri("image/jpeg", SPONSOR_ALIPAY),
        "wechat": image_data_uri("image/jpeg", SPONSOR_WECHAT),
    })
}

pub fn injection_script(helper_port: u16) -> String {
    injection_script_with_settings(helper_port, &BackendSettings::default())
}

pub fn injection_script_with_settings(helper_port: u16, settings: &BackendSettings) -> String {
    let helper_url = format!("http://127.0.0.1:{helper_port}");
    let sponsor_images = sponsor_image_data_uris();
    let image_overlay = image_overlay_config(helper_port, settings);
    let dream_skin_art = dream_skin_art_data_uri(settings);
    let dream_skin_art_signature = dream_skin_art_content_signature(settings);
    let dream_skin_theme = &settings.codex_app_dream_skin_theme_config;
    let dream_skin_target_runtime = dream_skin_target_runtime_script(settings, false);
    let plugin_marketplaces = local_plugin_marketplaces();
    let paste_fix = paste_fix_enabled_config(settings);
    let force_chinese_locale = force_chinese_locale_config(settings);
    let fast_startup = fast_startup_config(settings);
    format!(
        "window.__CODEX_SESSION_DELETE_HELPER__ = {};\nwindow.__CODEX_PLUS_SPONSOR_IMAGES__ = {};\nwindow.__CODEX_PLUS_VERSION__ = {};\nwindow.__CODEX_PLUS_BUILD__ = {};\nwindow.__CODEX_PLUS_IMAGE_OVERLAY__ = {};\nwindow.__CODEX_PLUS_PLUGIN_MARKETPLACES__ = {};\nwindow.__CODEX_PLUS_EXTERNAL_DREAM_SKIN_RUNTIME__ = true;\nwindow.__CODEX_PLUS_DREAM_SKIN_PLATFORM__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_REVISION__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_ART__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_ART_SIGNATURE__ = {};\nwindow.__CODEX_PLUS_DREAM_SKIN_THEME__ = {};\nwindow.__CODEX_PLUS_PASTE_FIX__ = {};\nwindow.__CODEX_PLUS_FORCE_CHINESE_LOCALE__ = {};\nwindow.__CODEX_PLUS_FAST_STARTUP__ = {};\n{}\n{}\n{}",
        serde_json::to_string(&helper_url).expect("helper URL should serialize"),
        serde_json::to_string(&sponsor_images).expect("sponsor images should serialize"),
        serde_json::to_string(crate::version::VERSION).expect("version should serialize"),
        serde_json::to_string(DIAGNOSTIC_BUILD_ID).expect("build id should serialize"),
        serde_json::to_string(&image_overlay).expect("image overlay config should serialize"),
        serde_json::to_string(&plugin_marketplaces).expect("plugin marketplaces should serialize"),
        serde_json::to_string(dream_skin_platform()).expect("dream skin platform should serialize"),
        serde_json::to_string(DREAM_SKIN_RENDERER_REVISION)
            .expect("dream skin renderer revision should serialize"),
        serde_json::to_string(&dream_skin_art).expect("dream skin art should serialize"),
        serde_json::to_string(&dream_skin_art_signature)
            .expect("dream skin art signature should serialize"),
        serde_json::to_string(dream_skin_theme).expect("dream skin theme should serialize"),
        serde_json::to_string(&paste_fix).expect("paste fix config should serialize"),
        serde_json::to_string(&force_chinese_locale)
            .expect("force Chinese locale config should serialize"),
        serde_json::to_string(&fast_startup).expect("fast startup config should serialize"),
        renderer_script(),
        stepwise_script(),
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

fn local_plugin_marketplaces() -> Value {
    let home = crate::codex_home::default_codex_home_dir();
    local_plugin_marketplaces_from_home(&home)
}

fn local_plugin_marketplaces_from_home(home: &Path) -> Value {
    let installed_plugins = installed_plugins_from_config(&home);
    let marketplace_dir = home
        .join(".tmp")
        .join("plugins")
        .join(".agents")
        .join("plugins");
    let candidates = [
        marketplace_dir.join("marketplace.json"),
        marketplace_dir.join("api_marketplace.json"),
        home.join(".tmp")
            .join("plugins-remote")
            .join(".agents")
            .join("plugins")
            .join("marketplace.json"),
    ];
    let marketplaces = candidates
        .iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(path).ok()?;
            let mut marketplace: Value = serde_json::from_str(&text).ok()?;
            expand_local_plugin_marketplace(&mut marketplace, path, &home, &installed_plugins);
            if let Some(object) = marketplace.as_object_mut() {
                object
                    .entry("path")
                    .or_insert_with(|| Value::String(path.to_string_lossy().to_string()));
            }
            Some(marketplace)
        })
        .collect::<Vec<_>>();
    Value::Array(marketplaces)
}

fn expand_local_plugin_marketplace(
    marketplace: &mut Value,
    marketplace_path: &Path,
    home: &Path,
    installed_plugins: &std::collections::BTreeSet<String>,
) {
    let marketplace_name = marketplace
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let Some(plugins) = marketplace.get_mut("plugins").and_then(Value::as_array_mut) else {
        return;
    };
    let marketplace_root = marketplace_path
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".tmp").join("plugins"));
    for plugin in plugins {
        let Some(plugin_object) = plugin.as_object_mut() else {
            continue;
        };
        let plugin_name = plugin_object
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                plugin_object
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| id.split('@').next())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        if plugin_name.is_empty() {
            continue;
        }
        let manifest_path = marketplace_root
            .join("plugins")
            .join(&plugin_name)
            .join(".codex-plugin")
            .join("plugin.json");
        let plugin_root = marketplace_root.join("plugins").join(&plugin_name);
        if let Some(manifest) = plugin_manifest(&manifest_path) {
            merge_plugin_manifest(plugin_object, manifest);
        }
        absolutize_plugin_icon_paths(plugin_object, &plugin_root);
        plugin_object
            .entry("name".to_string())
            .or_insert_with(|| Value::String(plugin_name.clone()));
        plugin_object
            .entry("id".to_string())
            .or_insert_with(|| Value::String(format!("{plugin_name}@{marketplace_name}")));
        plugin_object
            .entry("marketplaceName".to_string())
            .or_insert_with(|| Value::String(marketplace_name.clone()));
        plugin_object
            .entry("marketplacePath".to_string())
            .or_insert_with(|| Value::String(marketplace_name.clone()));
        plugin_object
            .entry("keywords".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        plugin_object.insert(
            "installed".to_string(),
            Value::Bool(installed_plugins.contains(&format!("{plugin_name}@{marketplace_name}"))),
        );
    }
}

fn absolutize_plugin_icon_paths(plugin: &mut Map<String, Value>, plugin_root: &Path) {
    for key in ["composerIconPath", "logoPath"] {
        absolutize_string_field(plugin, key, plugin_root);
    }
    let Some(interface) = plugin.get_mut("interface").and_then(Value::as_object_mut) else {
        return;
    };
    for key in ["composerIcon", "composerIconUrl", "logo", "logoUrl"] {
        absolutize_string_field(interface, key, plugin_root);
    }
}

fn absolutize_string_field(object: &mut Map<String, Value>, key: &str, root: &Path) {
    let Some(value) = object.get(key).and_then(Value::as_str).map(str::to_string) else {
        return;
    };
    let Some(path) = absolutize_plugin_asset_path(&value, root) else {
        return;
    };
    object.insert(key.to_string(), Value::String(path));
}

fn absolutize_plugin_asset_path(value: &str, root: &Path) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("data:")
        || trimmed.starts_with("http:")
        || trimmed.starts_with("https:")
        || trimmed.starts_with("file:")
        || Path::new(trimmed).is_absolute()
    {
        return None;
    }
    let relative = trimmed.strip_prefix("./").unwrap_or(trimmed);
    Some(root.join(relative).to_string_lossy().to_string())
}

fn plugin_manifest(path: &Path) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&text)
        .ok()?
        .as_object()
        .cloned()
}

fn merge_plugin_manifest(plugin: &mut Map<String, Value>, manifest: Map<String, Value>) {
    for (key, value) in manifest {
        plugin.entry(key).or_insert(value);
    }
}

fn installed_plugins_from_config(home: &Path) -> std::collections::BTreeSet<String> {
    let text = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    let doc = text.parse::<toml_edit::DocumentMut>().ok();
    let Some(plugins) = doc
        .as_ref()
        .and_then(|doc| doc.get("plugins"))
        .and_then(toml_edit::Item::as_table)
    else {
        return std::collections::BTreeSet::new();
    };
    plugins
        .iter()
        .filter_map(|(id, item)| {
            let enabled = item
                .get("enabled")
                .and_then(toml_edit::Item::as_bool)
                .unwrap_or(false);
            enabled.then(|| id.to_string())
        })
        .collect()
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

pub fn force_chinese_locale_config(settings: &BackendSettings) -> Value {
    json!({ "enabled": settings.codex_app_force_chinese_locale, "locale": "zh-CN" })
}

pub fn fast_startup_config(settings: &BackendSettings) -> Value {
    json!({ "enabled": settings.codex_app_fast_startup, "statsigTimeoutMs": 800 })
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
    fn image_overlay_config_includes_fit_mode() {
        let settings = BackendSettings {
            codex_app_image_overlay_fit_mode: "fill".to_string(),
            ..BackendSettings::default()
        };
        let config = image_overlay_config(57321, &settings);

        assert_eq!(config["fitMode"].as_str(), Some("fill"));
    }

    #[test]
    fn local_plugin_marketplaces_includes_api_marketplace_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let marketplace_dir = home
            .join(".tmp")
            .join("plugins")
            .join(".agents")
            .join("plugins");
        let api_plugin_dir = home
            .join(".tmp")
            .join("plugins")
            .join("plugins")
            .join("build-web-apps");
        let remote_marketplace_dir = home
            .join(".tmp")
            .join("plugins-remote")
            .join(".agents")
            .join("plugins");
        let remote_plugin_dir = home
            .join(".tmp")
            .join("plugins-remote")
            .join("plugins")
            .join("product-design");
        std::fs::create_dir_all(&marketplace_dir).unwrap();
        std::fs::create_dir_all(&remote_marketplace_dir).unwrap();
        std::fs::create_dir_all(api_plugin_dir.join(".codex-plugin")).unwrap();
        std::fs::create_dir_all(remote_plugin_dir.join(".codex-plugin")).unwrap();
        std::fs::write(
            marketplace_dir.join("marketplace.json"),
            r#"{"name":"openai-curated","plugins":[{"name":"gmail"}]}"#,
        )
        .unwrap();
        std::fs::write(
            marketplace_dir.join("api_marketplace.json"),
            r#"{"name":"openai-api-curated","plugins":[{"name":"build-web-apps"}]}"#,
        )
        .unwrap();
        std::fs::write(
            remote_marketplace_dir.join("marketplace.json"),
            r#"{"name":"openai-curated-remote","plugins":[{"name":"product-design"}]}"#,
        )
        .unwrap();
        std::fs::write(
            api_plugin_dir.join(".codex-plugin").join("plugin.json"),
            r#"{"interface":{"displayName":"Build Web Apps"}}"#,
        )
        .unwrap();
        std::fs::write(
            remote_plugin_dir.join(".codex-plugin").join("plugin.json"),
            r#"{"interface":{"displayName":"Product Design"}}"#,
        )
        .unwrap();

        let marketplaces = local_plugin_marketplaces_from_home(home);
        let array = marketplaces.as_array().unwrap();

        assert_eq!(array.len(), 3);
        assert_eq!(array[0]["name"].as_str(), Some("openai-curated"));
        assert_eq!(array[1]["name"].as_str(), Some("openai-api-curated"));
        assert_eq!(array[2]["name"].as_str(), Some("openai-curated-remote"));
        assert_eq!(
            array[1]["plugins"][0]["interface"]["displayName"].as_str(),
            Some("Build Web Apps")
        );
        assert_eq!(
            array[2]["plugins"][0]["interface"]["displayName"].as_str(),
            Some("Product Design")
        );
        assert_eq!(
            array[2]["plugins"][0]["marketplaceName"].as_str(),
            Some("openai-curated-remote")
        );
        assert_eq!(
            array[2]["plugins"][0]["marketplacePath"].as_str(),
            Some("openai-curated-remote")
        );
    }
}
