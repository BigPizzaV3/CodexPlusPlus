use base64::Engine;
use serde_json::{Value, json};
use std::path::Path;

use crate::settings::BackendSettings;

const RENDERER_SCRIPT: &str = include_str!("../../../assets/inject/renderer-inject.js");
const SPONSOR_ALIPAY: &[u8] = include_bytes!("../../../assets/images/sponsor-alipay.jpg");
const SPONSOR_WECHAT: &[u8] = include_bytes!("../../../assets/images/sponsor-wechat.jpg");
pub const DIAGNOSTIC_BUILD_ID: &str = "diag-20260518-1";

pub fn renderer_script() -> &'static str {
    RENDERER_SCRIPT
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
    format!(
        "window.__CODEX_SESSION_DELETE_HELPER__ = {};\nwindow.__CODEX_PLUS_SPONSOR_IMAGES__ = {};\nwindow.__CODEX_PLUS_VERSION__ = {};\nwindow.__CODEX_PLUS_BUILD__ = {};\nwindow.__CODEX_PLUS_IMAGE_OVERLAY__ = {};\n{}",
        serde_json::to_string(&helper_url).expect("helper URL should serialize"),
        serde_json::to_string(&sponsor_images).expect("sponsor images should serialize"),
        serde_json::to_string(crate::version::VERSION).expect("version should serialize"),
        serde_json::to_string(DIAGNOSTIC_BUILD_ID).expect("build id should serialize"),
        serde_json::to_string(&image_overlay).expect("image overlay config should serialize"),
        renderer_script(),
    )
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
        "dataUrl": data_url,
        "imageUrl": if enabled {
            format!("http://127.0.0.1:{helper_port}/overlay/image")
        } else {
            String::new()
        },
    })
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
