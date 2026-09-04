#[test]
fn stepwise_fragments_expose_the_expected_contract() {
    let suggestions = include_str!("../../../assets/inject/floating-panel/stepwise/suggestions.js")
        .replace("\r\n", "\n");
    let generation = include_str!("../../../assets/inject/floating-panel/stepwise/generation.js")
        .replace("\r\n", "\n");

    assert!(suggestions.starts_with("/* Stepwise suggestions:"));
    assert!(suggestions.contains("function uniquePrompts"));
    assert!(suggestions.contains("function parseStepwiseJson"));
    assert!(suggestions.contains("function extractStepwisePayload"));
    assert!(generation.starts_with("/* Stepwise generation:"));
    assert!(generation.contains("function requestBridgeStepwise"));
    assert!(generation.contains("function forceRefreshStepwise"));
    assert!(generation.contains("function fillComposer"));
    assert!(generation.contains("bridgeCall(\n      \"/stepwise/generate\""));
    assert!(generation.contains("requestEpoch === state.stepwiseEpoch"));
    assert!(generation.contains("contextMatches(requestContext)"));
    assert!(generation.contains("if (!isCurrentRuntime() || !stepwiseEnabled()) return;"));
    assert!(generation.contains("state.activeContext.assistantMessageId"));

    for source in [suggestions.as_str(), generation.as_str()] {
        assert!(!source.contains("import "));
        assert!(!source.contains("export "));
        assert!(!source.contains("fetch("));
        assert!(!source.contains("new WebSocket"));
        assert!(!source.contains("outline"));
    }
}
