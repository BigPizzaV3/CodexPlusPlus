#[test]
fn appearance_fragments_expose_the_expected_contract() {
    let runtime = include_str!("../../../assets/inject/floating-panel/core/appearance-runtime.js");
    let stylesheet = include_str!("../../../assets/inject/floating-panel/core/appearance.js");

    assert!(runtime.starts_with("/* Floating-panel appearance runtime:"));
    assert!(stylesheet.starts_with("/* Floating-panel appearance:"));
    assert!(runtime.contains("function installTypographyObserver"));
    assert!(runtime.contains("function applyMaterial"));
    assert!(stylesheet.contains("prefers-reduced-motion"));
    assert!(!runtime.contains("import "));
    assert!(!runtime.contains("export "));
    assert!(!stylesheet.contains("import "));
    assert!(!stylesheet.contains("export "));
}
