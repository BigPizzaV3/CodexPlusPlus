use codex_plus_core::dream_skin_market::{
    DreamSkinMarketTheme, install_market_theme, install_market_theme_from_base, load_market,
};
use codex_plus_core::settings::DreamSkinThemeConfig;
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn market_theme(config: &[u8], image: &[u8]) -> DreamSkinMarketTheme {
    DreamSkinMarketTheme {
        id: "market-demo".to_string(),
        name: "Market Demo".to_string(),
        version: "1.0.0".to_string(),
        author: "Codex++".to_string(),
        description: "Test theme".to_string(),
        license: "MIT".to_string(),
        source_url: "https://github.com/BigPizzaV3/CodexPlusPlus-Themes".to_string(),
        tags: vec!["test".to_string()],
        theme: "themes/market-demo/theme.json".to_string(),
        image: "themes/market-demo/image.png".to_string(),
        preview: "themes/market-demo/preview.jpg".to_string(),
        theme_sha256: sha256(config),
        image_sha256: sha256(image),
        preview_url: String::new(),
        installed: false,
        installed_version: String::new(),
        update_available: false,
    }
}

async fn mount_theme(server: &MockServer, config: &[u8], image: &[u8]) {
    Mock::given(method("GET"))
        .and(path("/themes/market-demo/theme.json"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(config.to_vec()))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/themes/market-demo/image.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(image.to_vec()))
        .mount(server)
        .await;
}

#[tokio::test]
async fn installs_verified_market_theme_into_local_library() {
    let state = tempdir().unwrap();
    let server = MockServer::start().await;
    let mut config = DreamSkinThemeConfig::default();
    config.id = "market-demo".to_string();
    config.name = "Market Demo".to_string();
    config.style_preset = "cyber-neon".to_string();
    let config = serde_json::to_vec(&config).unwrap();
    let image = b"\x89PNG\r\n\x1a\nmarket-image";
    mount_theme(&server, &config, image).await;

    let installed = install_market_theme_from_base(
        state.path(),
        &market_theme(&config, image),
        &format!("{}/", server.uri()),
    )
    .await
    .unwrap();

    assert_eq!(installed.id, "market-demo");
    let stored = codex_plus_core::dream_skin_library::load_stored_dream_skin_theme(
        state.path(),
        "market-demo",
    )
    .unwrap();
    assert_eq!(stored.config.style_preset, "cyber-neon");
    assert!(
        state
            .path()
            .join("dream-skin/themes/market-demo/theme.json")
            .is_file()
    );
    assert!(
        state
            .path()
            .join("dream-skin/themes/market-demo/image.png")
            .is_file()
    );
    let records =
        std::fs::read_to_string(state.path().join("dream-skin/market/installed.json")).unwrap();
    assert!(records.contains("\"market-demo\": \"1.0.0\""));
}

#[tokio::test]
async fn rejects_hash_mismatch_before_installing_theme() {
    let state = tempdir().unwrap();
    let server = MockServer::start().await;
    let mut config = DreamSkinThemeConfig::default();
    config.id = "market-demo".to_string();
    config.name = "Market Demo".to_string();
    let config = serde_json::to_vec(&config).unwrap();
    let image = b"\x89PNG\r\n\x1a\nmarket-image";
    mount_theme(&server, &config, image).await;
    let mut theme = market_theme(&config, image);
    theme.image_sha256 = "0".repeat(64);

    let error = install_market_theme_from_base(state.path(), &theme, &format!("{}/", server.uri()))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("SHA-256"));
    assert!(!state.path().join("dream-skin/themes/market-demo").exists());
}

#[tokio::test]
#[ignore = "requires the public CodexPlusPlus-Themes repository"]
async fn live_market_theme_can_be_fetched_and_installed() {
    let state = tempdir().unwrap();
    let market = load_market(state.path()).await.unwrap();
    assert!(!market.cached);
    let theme = market
        .manifest
        .themes
        .iter()
        .find(|theme| theme.id == "dream-skin-original")
        .unwrap();

    let installed = install_market_theme(state.path(), theme).await.unwrap();

    assert_eq!(installed.id, "dream-skin-original");
    assert!(
        state
            .path()
            .join("dream-skin/themes/dream-skin-original/theme.json")
            .is_file()
    );
}
