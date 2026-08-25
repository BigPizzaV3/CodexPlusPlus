use std::io::{Cursor, Write};

use codex_plus_core::dream_skin_library::{
    load_stored_dream_skin_theme, prepare_dream_skin_activation, save_validated_dream_skin_package,
};
use codex_plus_core::dream_skin_package::{compile_safe_css, validate_and_read_package};
use serde_json::json;
use sha2::{Digest, Sha256};

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn package_bytes(platform: &str, css: &[u8], image_hash: Option<String>) -> Vec<u8> {
    package_bytes_with_prefix(platform, css, image_hash, "")
}

fn package_bytes_with_prefix(
    platform: &str,
    css: &[u8],
    image_hash: Option<String>,
    prefix: &str,
) -> Vec<u8> {
    let theme = serde_json::to_vec(&json!({
        "schemaVersion": 1,
        "id": "community.theme",
        "name": "Community Theme",
        "image": "background.png",
        "appearance": "dark",
        "art": { "focusX": 0.5, "focusY": 0.5, "safeArea": "none", "taskMode": "ambient" },
        "colors": {
            "background": "#111111", "panel": "#222222", "panelAlt": "#333333",
            "accent": "#44AA88", "accentAlt": "#55BB99", "secondary": "#667788",
            "highlight": "#77CCAA", "text": "#F5F5F5", "muted": "#AAAAAA", "line": "#444444"
        }
    }))
    .unwrap();
    let image = include_bytes!("../../../assets/inject/dream-skin-default.png");
    let manifest = serde_json::to_vec(&json!({
        "packageVersion": 1,
        "themeId": "community.theme",
        "version": "1.2.3",
        "skinApiVersion": 1,
        "minClientVersion": "0.0.0",
        "platforms": [platform],
        "capabilities": ["background", "tokens", "safe-css"],
        "publisher": { "id": "tester", "displayName": "Tester" },
        "license": "MIT",
        "provenance": { "aiGenerated": false, "summary": "Test fixture" },
        "files": [
            { "path": "theme.json", "mediaType": "application/json", "bytes": theme.len(), "sha256": sha256(&theme) },
            { "path": "background.png", "mediaType": "image/png", "bytes": image.len(), "sha256": image_hash.unwrap_or_else(|| sha256(image)) },
            { "path": "theme.css", "mediaType": "text/css", "bytes": css.len(), "sha256": sha256(css) }
        ],
        "createdAt": "2026-08-10T00:00:00Z"
    }))
    .unwrap();

    let mut archive = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut archive);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in [
            ("manifest.json", manifest.as_slice()),
            ("theme.json", theme.as_slice()),
            ("theme.css", css),
            ("background.png", image.as_slice()),
        ] {
            writer
                .start_file(format!("{prefix}{name}"), options)
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }
    archive.into_inner()
}

#[test]
fn validates_real_dream_skin_package_shape() {
    let css = br#"[data-ds-part="root"] { color: var(--ds-theme-color-text); }"#;
    let package = validate_and_read_package(&package_bytes("macos", css, None), "macos").unwrap();

    assert_eq!(package.manifest.theme_id, "community.theme");
    assert_eq!(package.image_name, "background.png");
    assert_eq!(package.css.as_bytes(), css);
}

#[test]
fn validates_package_inside_one_top_level_directory() {
    let css = br#"[data-ds-part="root"] { color: var(--ds-theme-color-text); }"#;
    let package = validate_and_read_package(
        &package_bytes_with_prefix("macos", css, None, "studio-theme/"),
        "macos",
    )
    .unwrap();

    assert_eq!(package.manifest.theme_id, "community.theme");
    assert_eq!(package.image_name, "background.png");
}

#[test]
fn rejects_wrong_platform_and_payload_hash() {
    let css = br#"[data-ds-part="root"] { color: #ffffff; }"#;
    assert!(
        validate_and_read_package(&package_bytes("windows", css, None), "macos")
            .unwrap_err()
            .to_string()
            .contains("不支持当前平台")
    );
    assert!(
        validate_and_read_package(&package_bytes("macos", css, Some("0".repeat(64))), "macos")
            .unwrap_err()
            .to_string()
            .contains("SHA-256")
    );
}

#[test]
fn compiles_safe_css_into_controlled_cascade() {
    let compiled = compile_safe_css(
        "[data-ds-part=\"sidebar\"] { background-color: #ffffff; border-radius: 12px; }",
    )
    .unwrap();

    assert!(compiled.starts_with("@layer dreamskin-community"));
    assert!(compiled.contains("background-color: #ffffff !important;"));
    assert!(compiled.contains("border-radius: 12px !important;"));
    assert!(compiled.contains("background-image: none !important;"));
}

#[test]
fn safe_css_matches_the_dreamskin_value_policy() {
    for css in [
        "[data-ds-part=\"main\"], [data-ds-part=\"sidebar\"] { color: #fff; }",
        "[data-ds-part=\"main\"] { color: var(--ds-theme-color-accent, red); }",
        "[data-ds-part=\"main\"] { opacity: 0; }",
        "[data-ds-part=\"main\"] { border-width: 99px; }",
        "[data-ds-part=\"main\"] { color: #fff; color: #000; }",
    ] {
        assert!(compile_safe_css(css).is_err(), "accepted unsafe CSS: {css}");
    }

    let root = compile_safe_css(
        "[data-ds-part=\"root\"] { color: var(--ds-theme-color-text); font-size: 14px; }",
    )
    .unwrap();
    assert!(root.contains("[data-ds-part=\"root\"] body"));

    let toolbar =
        compile_safe_css("[data-ds-part=\"composer-toolbar\"] { color: #ffffff; }").unwrap();
    assert!(toolbar.contains("button:not([class~=\"bg-token-foreground\"]"));
}

#[test]
fn installed_package_preserves_and_activates_safe_css() {
    let temp = tempfile::tempdir().unwrap();
    let css = br#"[data-ds-part="message"] { border-color: #44aa88; }"#;
    let package = validate_and_read_package(&package_bytes("macos", css, None), "macos").unwrap();

    save_validated_dream_skin_package(temp.path(), &package).unwrap();
    let stored = load_stored_dream_skin_theme(temp.path(), "community.theme").unwrap();
    prepare_dream_skin_activation(temp.path(), &stored).unwrap();

    assert_eq!(
        std::fs::read(temp.path().join("dream-skin/theme/current.css")).unwrap(),
        css
    );
}

#[test]
fn validates_external_dreamskin_fixture_when_provided() {
    let Ok(path) = std::env::var("DREAM_SKIN_REAL_PACKAGE") else {
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let package = validate_and_read_package(&bytes, "macos").unwrap();
    assert!(
        package
            .manifest
            .capabilities
            .iter()
            .any(|item| item == "safe-css")
    );
}
