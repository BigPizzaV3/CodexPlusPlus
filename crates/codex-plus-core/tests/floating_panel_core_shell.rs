#[test]
fn core_shell_fragments_expose_the_expected_contract() {
    let host = include_str!("../../../assets/inject/floating-panel/core/host.js");
    let geometry = include_str!("../../../assets/inject/floating-panel/core/geometry.js");
    let interaction = include_str!("../../../assets/inject/floating-panel/core/interaction.js");
    let scroll = include_str!("../../../assets/inject/floating-panel/core/scroll-state.js");
    let views = include_str!("../../../assets/inject/floating-panel/core/views.js");

    assert!(host.starts_with("/* Floating-panel host:"));
    assert!(host.contains("function resolveFabExpression"));
    assert!(geometry.starts_with("/* Floating-panel geometry:"));
    assert!(geometry.contains("function setOpen"));
    assert!(interaction.contains("function installPanelDrag"));
    assert!(interaction.contains("setPointerCapture"));
    assert!(scroll.contains("function captureViewScroll"));
    assert!(scroll.contains("function restoreViewScroll"));
    assert!(views.contains("function renderFloat"));
    assert!(views.contains("function attachNextEvents"));
    for source in [host, geometry, interaction, scroll, views] {
        assert!(!source.contains("import "));
        assert!(!source.contains("export "));
    }
}
