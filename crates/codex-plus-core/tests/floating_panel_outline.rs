#[test]
fn outline_fragments_expose_the_expected_contract() {
    let parser = include_str!("../../../assets/inject/floating-panel/outline/parser.js")
        .replace("\r\n", "\n");
    let navigation = include_str!("../../../assets/inject/floating-panel/outline/navigation.js")
        .replace("\r\n", "\n");
    let feature = include_str!("../../../assets/inject/floating-panel/outline/feature.js")
        .replace("\r\n", "\n");
    let view = include_str!("../../../assets/inject/floating-panel/outline/view.js")
        .replace("\r\n", "\n");

    assert!(parser.starts_with("/* Answer Outline parser:"));
    assert!(parser.contains("function outlineCollectHeadingElements"));
    assert!(parser.contains("function outlineDedupeItems"));
    assert!(parser.contains("function outlineNormalizeDisplayLevels"));
    assert!(navigation.starts_with("/* Answer Outline navigation:"));
    assert!(navigation.contains("function outlineJumpTo"));
    assert!(navigation.contains("function outlineJumpToAnchor"));
    assert!(feature.starts_with("/* Answer Outline lifecycle:"));
    assert!(feature.contains("function refreshOutline"));
    assert!(feature.contains("contextMatches(requestContext)"));
    assert!(feature.contains("if (!isCurrentRuntime() || !outlineEnabled()) return;"));
    assert!(view.starts_with("/* Answer Outline view:"));
    assert!(view.contains("function outlineHtml"));
    assert!(view.contains("function attachOutlineEvents"));

    for source in [parser.as_str(), navigation.as_str(), feature.as_str(), view.as_str()] {
        assert!(!source.contains("import "));
        assert!(!source.contains("export "));
        assert!(!source.contains("fetch("));
        assert!(!source.contains("new WebSocket"));
        assert!(!source.contains("new MutationObserver"));
        assert!(!source.contains("/stepwise/generate"));
    }
}
