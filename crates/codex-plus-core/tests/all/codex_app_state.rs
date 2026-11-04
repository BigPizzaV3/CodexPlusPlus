use codex_plus_core::codex_app_state::{
    capture_app_state_snapshot, sync_app_state_after_provider_switch,
};
use serde_json::{Value, json};

#[test]
fn app_state_sync_restores_safe_state_and_ignores_sensitive_snapshot_keys() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let state_path = home.join(".codex-global-state.json");
    std::fs::write(
        &state_path,
        json!({
            "electron-saved-workspace-roots": ["C:/work/app", "C:\\work\\app\\"],
            "project-order": ["C:/work/app"],
            "active-workspace-roots": "C:/work/app",
            "electron-workspace-root-labels": {
                "C:/work/app/": "App"
            },
            "electron-avatar-overlay-bounds": {
                "x": 20,
                "y": 30,
                "width": 320,
                "height": 240
            },
            "electron-avatar-overlay-open": true,
            "electron-main-window-bounds": {
                "x": 10,
                "y": 10,
                "width": 1280,
                "height": 800
            },
            "thread-workspace-root-hints": {
                "thread-1": "C:/work/app",
                "local:thread-2": {
                    "workspaceRoot": "D:/work/other"
                }
            },
            "thread-projectless-output-directories": {
                "thread-1": "C:/work/app/out"
            },
            "thread-writable-roots": {
                "thread-1": ["C:/work/app"]
            },
            "projectless-thread-ids": ["thread-1", "thread-1"],
            "electron-persisted-atom-state": {
                "app-shell:right-panel-width:v2:/": 420,
                "avatar-overlay-mascot-width-px": 160,
                "composer-auto-context-enabled": false,
                "default-service-tier": "priority",
                "diff-filter": "all",
                "enter-behavior": "cmdAlways",
                "first-awake-pet-notification-avatar-ids": ["otter"],
                "has-seen-multi-agent-composer-banner": true,
                "electron:onboarding-workspace-autolaunch-applied": true,
                "sidebar-collapsed-sections-v1": ["cloud"],
                "sidebar-project-expanded-v1-codex:C:/work/app": true,
                "sidebar-width": 296,
                "thread-summary-panel-section-expanded-progress": false,
                "thread-client-id-v1:thread-1": "do-not-copy",
                "heartbeat-thread-permissions-by-id": {
                    "thread-1": "do-not-copy"
                },
                "prompt-history": ["secret"],
                "OPENAI_API_KEY": "do-not-copy",
                "provider-token-cache": "do-not-copy"
            },
            "prompt-history": ["secret"],
            "provider-token-cache": "secret"
        })
        .to_string(),
    )
    .unwrap();

    let snapshot_path = capture_app_state_snapshot(home)
        .unwrap()
        .expect("snapshot should be created");
    assert!(snapshot_path.is_file());

    std::fs::write(
        &state_path,
        json!({
            "electron-saved-workspace-roots": ["D:/fresh/app"],
            "active-workspace-roots": "D:/fresh/app",
            "thread-workspace-root-hints": {
                "thread-3": "D:/fresh/app"
            },
            "electron-persisted-atom-state": {
                "service-tier-default": "standard"
            }
        })
        .to_string(),
    )
    .unwrap();

    let result = sync_app_state_after_provider_switch(home).unwrap();
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();

    assert!(result.changed);
    assert!(result.backup_path.as_deref().unwrap().is_dir());
    assert!(result.snapshot_path.as_deref().unwrap().is_file());
    assert_eq!(
        state["electron-saved-workspace-roots"],
        json!(["D:\\fresh\\app", "C:\\work\\app"])
    );
    assert_eq!(
        state["active-workspace-roots"],
        json!(["D:\\fresh\\app", "C:\\work\\app"])
    );
    assert_eq!(
        state["electron-workspace-root-labels"],
        json!({"C:\\work\\app": "App"})
    );
    assert_eq!(state["electron-avatar-overlay-open"], true);
    assert_eq!(state["electron-avatar-overlay-bounds"]["width"], 320);
    assert_eq!(state["electron-main-window-bounds"]["height"], 800);
    assert_eq!(
        state["thread-workspace-root-hints"]["thread-1"],
        "C:/work/app"
    );
    assert_eq!(
        state["thread-workspace-root-hints"]["thread-3"],
        "D:/fresh/app"
    );
    assert_eq!(
        state["thread-projectless-output-directories"]["thread-1"],
        "C:/work/app/out"
    );
    assert_eq!(
        state["thread-writable-roots"]["thread-1"],
        json!(["C:/work/app"])
    );
    assert_eq!(state["projectless-thread-ids"], json!(["thread-1"]));
    assert_eq!(
        state["electron-persisted-atom-state"]["default-service-tier"],
        "priority"
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["composer-auto-context-enabled"],
        false
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["enter-behavior"],
        "cmdAlways"
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["avatar-overlay-mascot-width-px"],
        160
    );
    assert_eq!(state["electron-persisted-atom-state"]["sidebar-width"], 296);
    assert_eq!(
        state["electron-persisted-atom-state"]["app-shell:right-panel-width:v2:/"],
        420
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["sidebar-project-expanded-v1-codex:C:/work/app"],
        true
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["has-seen-multi-agent-composer-banner"],
        true
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["electron:onboarding-workspace-autolaunch-applied"],
        true
    );
    assert!(state.get("prompt-history").is_none());
    assert!(state.get("provider-token-cache").is_none());
    assert!(
        state["electron-persisted-atom-state"]
            .get("prompt-history")
            .is_none()
    );
    assert!(
        state["electron-persisted-atom-state"]
            .get("thread-client-id-v1:thread-1")
            .is_none()
    );
    assert!(
        state["electron-persisted-atom-state"]
            .get("heartbeat-thread-permissions-by-id")
            .is_none()
    );
    assert!(
        state["electron-persisted-atom-state"]
            .get("OPENAI_API_KEY")
            .is_none()
    );
    assert!(
        state["electron-persisted-atom-state"]
            .get("provider-token-cache")
            .is_none()
    );
}

#[test]
fn app_state_sync_normalizes_current_state_and_writes_backup_before_change() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let state_path = home.join(".codex-global-state.json");
    std::fs::write(
        &state_path,
        json!({
            "electron-saved-workspace-roots": ["C:/work/app", "C:\\work\\app\\"],
            "active-workspace-roots": "C:/work/app/",
            "projectless-thread-ids": ["thread-1", "thread-1"]
        })
        .to_string(),
    )
    .unwrap();

    let result = sync_app_state_after_provider_switch(home).unwrap();
    let backup_path = result
        .backup_path
        .expect("normalization should create backup");
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    let backup: Value = serde_json::from_str(
        &std::fs::read_to_string(backup_path.join(".codex-global-state.json")).unwrap(),
    )
    .unwrap();

    assert!(result.changed);
    assert_eq!(
        state["electron-saved-workspace-roots"],
        json!(["C:\\work\\app"])
    );
    assert_eq!(state["active-workspace-roots"], json!("C:\\work\\app"));
    assert_eq!(state["projectless-thread-ids"], json!(["thread-1"]));
    assert_eq!(
        backup["electron-saved-workspace-roots"],
        json!(["C:/work/app", "C:\\work\\app\\"])
    );
}

/// 切换目录后「最近使用」里上个目录的模型 ID 要被清掉，合法条目和其他字段原样保留。
#[test]
fn prune_stale_recent_models_drops_unavailable_entries_and_keeps_the_rest() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let state_path = home.join(".codex-global-state.json");
    std::fs::write(
        &state_path,
        json!({
            "electron-saved-workspace-roots": ["C:/work/app"],
            "electron-persisted-atom-state": {
                "composer-recent-model-configurations-v1": [
                    {"model": "cpx-relay-mv5bil9a/deepseek-v4-flash", "reasoningEffort": "max"},
                    {"model": "ccs-deepseek/deepseek-flash", "reasoningEffort": "max"},
                    {"model": "gpt-6.1-sol", "reasoningEffort": "max"},
                    {"reasoningEffort": "max"}
                ],
                "sidebar-width": 280
            }
        })
        .to_string(),
    )
    .unwrap();

    let mut valid = std::collections::HashSet::new();
    valid.insert("cpx-relay-mv5bil9a/deepseek-v4-flash".to_string());
    let removed = codex_plus_core::codex_app_state::prune_stale_recent_model_configurations(
        home, &valid,
    )
    .unwrap();

    assert_eq!(removed, 2);
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    let entries = state["electron-persisted-atom-state"]
        ["composer-recent-model-configurations-v1"]
        .as_array()
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["model"], "cpx-relay-mv5bil9a/deepseek-v4-flash");
    // 没有 model 字段的条目结构不认识，原样保留。
    assert!(entries[1].get("model").is_none());
    assert_eq!(state["electron-saved-workspace-roots"], json!(["C:/work/app"]));
    assert_eq!(
        state["electron-persisted-atom-state"]["sidebar-width"],
        json!(280)
    );
    // 改前必须留备份。
    let backups = std::fs::read_dir(home.join("backups_state/app-state-sync"))
        .unwrap()
        .count();
    assert!(backups >= 1);
}

/// issue #2876 / #2707：新版 Codex 的项目键（local-projects / thread-project-* 等）
/// 必须进快照并在 merge 时带回，否则读到半截 state 再 merge 写回会把项目归属截掉。
#[test]
fn app_state_sync_passes_through_new_codex_project_keys() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let state_path = home.join(".codex-global-state.json");
    std::fs::write(
        &state_path,
        json!({
            "local-projects": {"proj-1": {"name": "App", "root": "C:/work/app"}},
            "selected-project": {"id": "proj-1"},
            "app-server-projects-migration-by-host": {"local": {"proj-1": "srv-1"}},
            "thread-project-membership-host-ids": {"thread-1": "local"},
            "thread-project-assignments": {"thread-1": "proj-1"},
            "sidebar-project-thread-orders": {"proj-1": ["thread-1"]},
            "project-appearances": {"proj-1": {"icon": "🚀"}},
            "thread-workspace-root-hints": {"thread-1": "C:/work/app"}
        })
        .to_string(),
    )
    .unwrap();

    let snapshot_path = capture_app_state_snapshot(home).unwrap().expect("snapshot");
    assert!(snapshot_path.is_file());
    let snapshot: Value = serde_json::from_str(&std::fs::read_to_string(&snapshot_path).unwrap()).unwrap();
    // 快照必须原样带出新版项目键，否则 merge 时无从恢复。
    for key in [
        "local-projects",
        "selected-project",
        "app-server-projects-migration-by-host",
        "thread-project-membership-host-ids",
        "thread-project-assignments",
        "sidebar-project-thread-orders",
        "project-appearances",
    ] {
        assert!(
            snapshot["state"].get(key).is_some(),
            "snapshot is missing {key}"
        );
    }

    // 模拟读到一份半截 state（项目键缺了），merge 后应该被快照恢复。
    std::fs::write(
        &state_path,
        json!({
            "thread-workspace-root-hints": {"thread-1": "C:/work/app"}
        })
        .to_string(),
    )
    .unwrap();
    let result = sync_app_state_after_provider_switch(home).unwrap();
    assert!(result.changed);
    let state: Value = serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    assert_eq!(state["local-projects"]["proj-1"]["name"], "App");
    assert_eq!(state["thread-project-membership-host-ids"]["thread-1"], "local");
    assert_eq!(state["project-appearances"]["proj-1"]["icon"], "🚀");
}

/// issue #2876 / #2707：读到「有 thread-project-* 数据但所有顶层项目键都缺」的
/// 半截 state 时拒绝 merge——继续写回会把项目归属永久截掉。
#[test]
fn app_state_sync_refuses_to_merge_truncated_state() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let state_path = home.join(".codex-global-state.json");
    std::fs::write(
        &state_path,
        json!({
            "thread-project-membership-host-ids": {"thread-1": "local"},
            "thread-project-assignments": {"thread-1": "proj-1"},
            "thread-workspace-root-hints": {"thread-1": "C:/work/app"}
        })
        .to_string(),
    )
    .unwrap();
    let error = sync_app_state_after_provider_switch(home).unwrap_err();
    assert!(error.to_string().contains("partially-written"), "{error}");
}

/// 目录算不出来（空集合）时绝不改文件：宁可留着残留，也不能误删用户的选择。
#[test]
fn prune_stale_recent_models_skips_when_no_published_models() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let state_path = home.join(".codex-global-state.json");
    let original = json!({
        "electron-persisted-atom-state": {
            "composer-recent-model-configurations-v1": [
                {"model": "ccs-deepseek/deepseek-flash"}
            ]
        }
    })
    .to_string();
    std::fs::write(&state_path, &original).unwrap();

    let removed = codex_plus_core::codex_app_state::prune_stale_recent_model_configurations(
        home,
        &std::collections::HashSet::new(),
    )
    .unwrap();

    assert_eq!(removed, 0);
    assert_eq!(std::fs::read_to_string(&state_path).unwrap(), original);
}
