use base64::Engine;
use serde_json::{Value, json};

const RENDERER_SCRIPT: &str = include_str!("../../../assets/inject/renderer-inject.js");
const SPONSOR_ALIPAY: &[u8] = include_bytes!("../../../assets/images/sponsor-alipay.jpg");
const SPONSOR_WECHAT: &[u8] = include_bytes!("../../../assets/images/sponsor-wechat.jpg");

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
    format!(
        "window.__CODEX_SESSION_DELETE_HELPER__ = {};\nwindow.__CODEX_PLUS_SPONSOR_IMAGES__ = {};\nwindow.__CODEX_PLUS_VERSION__ = {};\n{}",
        serde_json::to_string(&helper_url).expect("helper URL should serialize"),
        serde_json::to_string(&sponsor_images).expect("sponsor images should serialize"),
        serde_json::to_string(crate::version::VERSION).expect("version should serialize"),
        renderer_script(),
    )
}

fn image_data_uri(mime_type: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}
