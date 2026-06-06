use base64::Engine;
use crate::paths::default_settings_path;
use crate::settings::SettingsStore;
use serde_json::{Value, json};

const INJECT_I18N_SCRIPT: &str = include_str!("../../../assets/inject/i18n.js");
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
    let helper_url = format!("http://127.0.0.1:{helper_port}");
    let sponsor_images = sponsor_image_data_uris();
    let ui_language = embedded_ui_language();
    let ui_language_js = if ui_language.is_empty() {
        "null".to_string()
    } else {
        serde_json::to_string(&ui_language).expect("ui language should serialize")
    };
    format!(
        "window.__CODEX_SESSION_DELETE_HELPER__ = {};\nwindow.__CODEX_PLUS_SPONSOR_IMAGES__ = {};\nwindow.__CODEX_PLUS_VERSION__ = {};\nwindow.__CODEX_PLUS_BUILD__ = {};\nwindow.__CODEX_PLUS_UI_LANG__ = {};\n{}\n{}",
        serde_json::to_string(&helper_url).expect("helper URL should serialize"),
        serde_json::to_string(&sponsor_images).expect("sponsor images should serialize"),
        serde_json::to_string(crate::version::VERSION).expect("version should serialize"),
        serde_json::to_string(DIAGNOSTIC_BUILD_ID).expect("build id should serialize"),
        ui_language_js,
        INJECT_I18N_SCRIPT,
        renderer_script(),
    )
}

fn embedded_ui_language() -> String {
    let store = SettingsStore::new(default_settings_path());
    let Ok(settings) = store.load() else {
        return String::new();
    };
    let lang = settings.ui_language.trim();
    if lang == "en" || lang == "zh" {
        lang.to_string()
    } else {
        String::new()
    }
}

fn image_data_uri(mime_type: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}
