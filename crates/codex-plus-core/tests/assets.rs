#[test]
fn renderer_removes_orphaned_projected_rows_when_thread_is_missing() {
    let script = codex_plus_core::assets::renderer_script();

    assert!(script.contains("function isThreadMissingResult"));
    assert!(script.contains("Thread not found in local storage"));
    assert!(script.contains("function removeOrphanedProjectedRow"));
    assert!(script.contains("setProjectlessThreadIds(ref, \"remove\")"));
    assert!(script.contains("clearThreadWorkspaceHints(ref)"));
    assert!(script.contains("已移除本地列表中的失效会话"));
}
