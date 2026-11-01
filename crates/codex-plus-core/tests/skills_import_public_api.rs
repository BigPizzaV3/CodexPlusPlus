use codex_plus_core::skills::{SkillImportIncompleteTarget, SkillsManager};
use std::collections::BTreeMap;

#[test]
fn incomplete_target_is_available_to_external_crates() {
    let payload = serde_json::json!({
        "existing": [{"id": "existing", "name": "Existing", "error": "unreadable", "fingerprint": "snapshot"}]
    });
    let confirmations: BTreeMap<String, Vec<SkillImportIncompleteTarget>> =
        serde_json::from_value(payload).unwrap();
    assert_eq!(confirmations["existing"][0].id, "existing");
    let _method = SkillsManager::apply_skill_import_with_confirmations;
}
