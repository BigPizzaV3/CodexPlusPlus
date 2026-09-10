#[test]
fn runtime_fragments_expose_the_expected_contract() {
    let state = include_str!("../../../assets/inject/floating-panel/runtime/state.js");
    let dom = include_str!("../../../assets/inject/floating-panel/runtime/dom.js");
    let bridge = include_str!("../../../assets/inject/floating-panel/runtime/bridge-client.js");
    let context = include_str!("../../../assets/inject/floating-panel/runtime/answer-context.js");
    let lifecycle = include_str!("../../../assets/inject/floating-panel/runtime/lifecycle.js");
    let settings = include_str!("../../../assets/inject/floating-panel/runtime/settings.js");

    let state = state.replace("\r\n", "\n");
    assert!(state.starts_with("/*\n * Floating-panel runtime state."));
    assert!(state.contains("function isCurrentRuntime"));
    assert!(state.contains("runtimeGeneration: 0"));
    assert!(dom.contains("function stripOwnUi"));
    assert!(dom.contains("function elementText"));
    assert!(bridge.contains("function bridgeCall"));
    assert_eq!(bridge.matches("window[PAGE_BRIDGE]").count(), 2);
    assert!(context.starts_with("/* Shared answer-context"));
    assert!(context.contains("function contextSnapshot"));
    assert!(context.contains("function findLatestAssistantMessage"));
    assert!(context.contains("function installContextTracking"));
    assert!(context.contains("function removeContextTracking"));
    assert!(lifecycle.starts_with("/* Floating-panel runtime lifecycle"));
    assert!(lifecycle.contains("function stopRuntime"));
    assert!(lifecycle.contains("state.runtimeGeneration += 1"));
    assert!(settings.contains("function syncSettings"));
    assert!(settings.contains("function destroy"));
    assert!(settings.contains("async function start"));

    for source in [state.as_str(), dom, bridge, context, lifecycle, settings] {
        assert!(!source.contains("import "));
        assert!(!source.contains("export "));
        assert!(!source.contains("new WebSocket"));
        assert!(!source.contains("Runtime.addBinding"));
    }
}
