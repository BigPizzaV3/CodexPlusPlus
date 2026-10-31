//! Public-API regressions for Skill lifecycle data preservation.
use codex_plus_core::skills::SkillsManager;

#[test]
fn enabling_does_not_delete_a_directory_without_a_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let home = temp.path().join("home");
    let local = home.join("skills/alpha");
    std::fs::create_dir_all(source.join("alpha")).unwrap();
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(source.join("alpha/SKILL.md"), "managed").unwrap();
    std::fs::write(local.join("notes.txt"), "user notes").unwrap();
    let manager = SkillsManager::new(
        &source,
        temp.path().join("backups"),
        temp.path().join("skills.json"),
        &home,
    );
    assert!(manager.set_enabled("alpha", true).is_err());
    assert_eq!(
        std::fs::read_to_string(local.join("notes.txt")).unwrap(),
        "user notes"
    );
    assert!(!local.join("SKILL.md").exists());
}

#[test]
fn restore_link_failure_preserves_the_backup_and_state() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let backup = temp.path().join("backups/alpha-123");
    let source = temp.path().join("source");
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(backup.join("SKILL.md"), "original backup").unwrap();
    std::fs::write(home.join("skills"), "block directory creation").unwrap();
    let manager = SkillsManager::new(
        &source,
        temp.path().join("backups"),
        temp.path().join("skills.json"),
        &home,
    );
    let before = manager.load_state();
    assert!(manager.restore_backup("alpha-123").is_err());
    assert_eq!(manager.load_state(), before);
    assert_eq!(
        std::fs::read_to_string(backup.join("SKILL.md")).unwrap(),
        "original backup"
    );
    assert!(!source.join("alpha").exists());
}

#[test]
fn restore_does_not_replace_an_existing_local_skill() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let backup = temp.path().join("backups/alpha-123");
    let local = home.join("skills/alpha");
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(backup.join("SKILL.md"), "backup").unwrap();
    std::fs::write(local.join("SKILL.md"), "local edits").unwrap();
    let manager = SkillsManager::new(
        temp.path().join("source"),
        temp.path().join("backups"),
        temp.path().join("skills.json"),
        &home,
    );
    assert!(manager.restore_backup("alpha-123").is_err());
    assert_eq!(
        std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
        "local edits"
    );
    assert_eq!(
        std::fs::read_to_string(backup.join("SKILL.md")).unwrap(),
        "backup"
    );
}

#[test]
#[cfg(windows)]
fn failed_restore_preserves_the_original_backup() {
    let temp = tempfile::tempdir().unwrap();
    let backup = temp.path().join("backups/alpha-123");
    let state = temp.path().join("skills.json");
    let source = temp.path().join("source");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::write(backup.join("SKILL.md"), "original backup").unwrap();
    std::fs::write(&state, r#"{"repos":[],"installed":{}}"#).unwrap();
    let manager = SkillsManager::new(&source, temp.path().join("backups"), &state, &home);
    let before = manager.load_state();
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&state, permissions).unwrap();
    let result = manager.restore_backup("alpha-123");
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&state, permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(manager.load_state(), before);
    assert_eq!(
        std::fs::read_to_string(backup.join("SKILL.md")).unwrap(),
        "original backup"
    );
    assert!(!source.join("alpha").exists());
    assert!(!home.join("skills/alpha").exists());
    assert!(
        manager
            .list_backups()
            .iter()
            .any(|item| item.id == "alpha-123")
    );
}

#[test]
fn disabling_rejects_staging_inside_the_original_skill() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let local = home.join("skills/alpha");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("SKILL.md"), "original").unwrap();
    let source = local.join("managed");
    let manager = SkillsManager::new(
        &source,
        temp.path().join("backups"),
        temp.path().join("skills.json"),
        &home,
    );
    assert!(manager.set_enabled("alpha", false).is_err());
    assert_eq!(
        std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
        "original"
    );
    assert!(manager.list_backups().is_empty());
    assert_eq!(std::fs::read_dir(source).unwrap().count(), 0);
}

#[test]
fn conflicting_codex_edits_survive_restarts_and_repeated_toggles() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let backups = temp.path().join("backups");
    let home = temp.path().join("home");
    let state = temp.path().join("skills.json");
    let manager = SkillsManager::new(&source, &backups, &state, &home);
    let local = home.join("skills/alpha");
    std::fs::create_dir_all(source.join("alpha")).unwrap();
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(source.join("alpha/SKILL.md"), "original source").unwrap();
    std::fs::write(local.join("SKILL.md"), "user edits").unwrap();
    std::fs::write(&state, r#"{"repos":[],"installed":{"alpha":{"id":"alpha","repoKey":"acme/kit@main","repoPath":"alpha","contentHash":"old"}}}"#).unwrap();
    assert!(manager.set_enabled("alpha", false).is_err());
    assert_eq!(
        std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(),
        "original source"
    );
    assert_eq!(
        std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
        "user edits"
    );
    let manager = SkillsManager::new(&source, &backups, &state, &home);
    assert!(manager.codex_copy_conflict("alpha").unwrap());
    manager.resolve_codex_conflict("alpha", true).unwrap();
    assert_eq!(
        std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(),
        "user edits"
    );
    let installed = &manager.load_state().installed["alpha"];
    assert_eq!(installed.repo_key, "acme/kit@main");
    assert_eq!(installed.repo_path, "alpha");
    assert_eq!(installed.content_hash, "old");
    for _ in 0..4 {
        manager.set_enabled("alpha", false).unwrap();
        assert!(!local.exists());
        manager.set_enabled("alpha", true).unwrap();
        assert_eq!(
            std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
            "user edits"
        );
    }
    std::fs::write(local.join("SKILL.md"), "more edits").unwrap();
    assert!(manager.set_enabled("alpha", true).is_err());
    assert_eq!(
        std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
        "more edits"
    );
    manager.resolve_codex_conflict("alpha", false).unwrap();
    assert_eq!(
        std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
        "user edits"
    );
    assert!(manager.list_backups().iter().any(|backup| {
        std::fs::read_to_string(backups.join(&backup.id).join("SKILL.md"))
            .is_ok_and(|text| text == "more edits")
    }));
}

#[test]
fn agent_edits_do_not_change_library_and_are_backed_up_on_disable() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let backups = temp.path().join("backups");
    let home = temp.path().join("home");
    let agent = temp.path().join("claude");
    let external = temp.path().join("external");
    std::fs::create_dir_all(&external).unwrap();
    std::fs::write(external.join("SKILL.md"), "original").unwrap();
    let manager = SkillsManager::new(&source, &backups, temp.path().join("skills.json"), &home)
        .with_agent_dirs(vec![("claude".into(), agent.clone())]);
    manager.import_from_directory("alpha", &external).unwrap();
    manager.set_enabled_for("claude", "alpha", true).unwrap();
    std::fs::write(agent.join("alpha/SKILL.md"), "agent edit").unwrap();
    assert_eq!(
        std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(),
        "original"
    );
    manager.set_enabled_for("claude", "alpha", false).unwrap();
    assert!(manager.list_backups().iter().any(|backup| {
        std::fs::read_to_string(backups.join(&backup.id).join("SKILL.md"))
            .is_ok_and(|text| text == "agent edit")
    }));
    assert_eq!(
        std::fs::read_to_string(external.join("SKILL.md")).unwrap(),
        "original"
    );
}

#[test]
fn startup_adopts_existing_codex_skill_without_replacing_it() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let home = temp.path().join("home");
    let local = home.join("skills/custom");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("SKILL.md"), "user original").unwrap();
    let state = temp.path().join("skills.json");
    let manager = SkillsManager::new(&source, temp.path().join("backups"), &state, &home);
    manager.prepare_managed_skills().unwrap();
    assert!(manager.load_state().installed.contains_key("custom"));
    assert_eq!(
        std::fs::read_to_string(source.join("custom/SKILL.md")).unwrap(),
        "user original"
    );
    std::fs::write(local.join("SKILL.md"), "edited after adoption").unwrap();
    let reopened = SkillsManager::new(&source, temp.path().join("backups"), &state, &home);
    reopened.prepare_managed_skills().unwrap();
    assert!(reopened.codex_copy_conflict("custom").unwrap());
    assert_eq!(
        std::fs::read_to_string(source.join("custom/SKILL.md")).unwrap(),
        "user original"
    );
    assert_eq!(
        std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
        "edited after adoption"
    );
}

#[test]
fn corrupt_state_is_not_replaced_by_default_during_migration() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("skills.json");
    let local = temp.path().join("home/skills/custom");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("SKILL.md"), "user original").unwrap();
    std::fs::write(&state, "{broken").unwrap();
    let manager = SkillsManager::new(
        temp.path().join("source"),
        temp.path().join("backups"),
        &state,
        temp.path().join("home"),
    );
    assert!(manager.prepare_managed_skills().is_err());
    let repo = codex_plus_core::skills::parse_repo_key("example/skills@main").unwrap();
    assert!(manager.save_repo(repo.clone(), None).is_err());
    assert!(manager.delete_repo(&repo.key()).is_err());
    assert!(
        manager
            .replace_repo_if_unchanged(&repo, repo.clone())
            .is_err()
    );
    let mut disabled = repo;
    disabled.enabled = false;
    assert!(
        manager
            .update_repo_enabled_if_unchanged(&disabled, None)
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&state).unwrap(), "{broken");
    assert_eq!(
        std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
        "user original"
    );
}

#[test]
fn unreadable_legacy_library_is_retried_instead_of_marked_migrated() {
    let temp = tempfile::tempdir().unwrap();
    let legacy = temp.path().join("legacy");
    std::fs::write(&legacy, "temporarily not a directory").unwrap();
    let manager = SkillsManager::new(
        temp.path().join("library"),
        temp.path().join("backups"),
        temp.path().join("skills.json"),
        temp.path().join("home"),
    )
    .with_legacy_source_dir(&legacy);
    assert!(manager.prepare_managed_skills().is_err());
    assert!(!manager.load_state().independent_library_migrated);
    std::fs::remove_file(&legacy).unwrap();
    std::fs::create_dir_all(legacy.join("alpha")).unwrap();
    std::fs::write(legacy.join("alpha/SKILL.md"), "original").unwrap();
    manager.prepare_managed_skills().unwrap();
    assert_eq!(
        std::fs::read_to_string(manager.source_dir().join("alpha/SKILL.md")).unwrap(),
        "original"
    );
    assert!(manager.load_state().independent_library_migrated);
}

#[test]
fn startup_recovers_orphaned_managed_agent_copy_into_library() {
    let temp = tempfile::tempdir().unwrap();
    let agent = temp.path().join("claude/alpha");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::write(agent.join("SKILL.md"), "only copy").unwrap();
    std::fs::write(agent.join(".codex-plus-managed"), "").unwrap();
    let source = temp.path().join("source");
    let manager = SkillsManager::new(
        &source,
        temp.path().join("backups"),
        temp.path().join("skills.json"),
        temp.path().join("home"),
    )
    .with_agent_dirs(vec![("claude".into(), temp.path().join("claude"))]);
    manager.prepare_managed_skills().unwrap();
    assert!(manager.load_state().installed.contains_key("alpha"));
    assert_eq!(
        std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(),
        "only copy"
    );
    assert_eq!(
        std::fs::read_to_string(agent.join("SKILL.md")).unwrap(),
        "only copy"
    );
}

#[test]
fn old_library_is_copied_once_without_overwriting_codex_edits_or_resurrecting_uninstalled_skills() {
    let temp = tempfile::tempdir().unwrap();
    let legacy = temp.path().join("legacy/alpha");
    let source = temp.path().join("library");
    let codex = temp.path().join("home/skills/alpha");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::write(legacy.join("SKILL.md"), "old library").unwrap();
    std::fs::write(codex.join("SKILL.md"), "codex edits").unwrap();
    let state = temp.path().join("skills.json");
    std::fs::write(
        &state,
        r#"{"repos":[],"installed":{"alpha":{"id":"alpha"}}}"#,
    )
    .unwrap();
    let manager = SkillsManager::new(
        &source,
        temp.path().join("backups"),
        &state,
        temp.path().join("home"),
    )
    .with_legacy_source_dir(temp.path().join("legacy"));
    manager.prepare_managed_skills().unwrap();
    assert_eq!(
        std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(),
        "old library"
    );
    assert!(manager.codex_copy_conflict("alpha").unwrap());
    assert!(manager.load_state().independent_library_migrated);
    std::fs::write(legacy.join("SKILL.md"), "external change").unwrap();
    assert_eq!(
        std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(),
        "old library"
    );
    manager.resolve_codex_conflict("alpha", true).unwrap();
    manager.uninstall("alpha").unwrap();
    manager.prepare_managed_skills().unwrap();
    assert!(!source.join("alpha").exists());
    assert!(!manager.load_state().installed.contains_key("alpha"));
    assert_eq!(
        std::fs::read_to_string(legacy.join("SKILL.md")).unwrap(),
        "external change"
    );
}

#[test]
#[cfg(windows)]
#[ignore = "需要 Windows 开发者模式或符号链接权限"]
fn legacy_windows_directory_symlinks_are_detached_from_library() {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        CreateSymbolicLinkW, SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
        SYMBOLIC_LINK_FLAG_DIRECTORY,
    };
    use windows::core::PCWSTR;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source/alpha");
    let home = temp.path().join("home");
    let codex = home.join("skills/alpha");
    let claude_root = temp.path().join("claude");
    let claude = claude_root.join("alpha");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&claude_root).unwrap();
    std::fs::write(source.join("SKILL.md"), "old").unwrap();
    for target in [&codex, &claude] {
        let name: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let created = unsafe {
            CreateSymbolicLinkW(
                PCWSTR(name.as_ptr()),
                PCWSTR(destination.as_ptr()),
                SYMBOLIC_LINK_FLAG_DIRECTORY | SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
            )
        };
        assert!(created.as_bool(), "当前 Windows 不允许创建测试目录软链");
        assert!(
            std::fs::symlink_metadata(target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
    let state = temp.path().join("skills.json");
    std::fs::write(
        &state,
        r#"{"repos":[],"installed":{"alpha":{"id":"alpha"}}}"#,
    )
    .unwrap();
    let manager = SkillsManager::new(
        temp.path().join("source"),
        temp.path().join("backups"),
        &state,
        &home,
    )
    .with_agent_dirs(vec![("claude".into(), claude_root)]);
    manager.prepare_managed_skills().unwrap();
    assert!(
        !std::fs::symlink_metadata(&codex)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        !std::fs::symlink_metadata(&claude)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    std::fs::write(codex.join("SKILL.md"), "codex edit").unwrap();
    std::fs::write(claude.join("SKILL.md"), "claude edit").unwrap();
    assert_eq!(
        std::fs::read_to_string(source.join("SKILL.md")).unwrap(),
        "old"
    );
    assert!(manager.codex_copy_conflict("alpha").unwrap());
}

#[test]
#[cfg(unix)]
fn legacy_managed_links_are_migrated_to_independent_copies() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let home = temp.path().join("home");
    let codex = home.join("skills/alpha");
    let claude = temp.path().join("claude/alpha");
    std::fs::create_dir_all(source.join("alpha")).unwrap();
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::create_dir_all(claude.parent().unwrap()).unwrap();
    std::fs::write(source.join("alpha/SKILL.md"), "old").unwrap();
    symlink(source.join("alpha"), &codex).unwrap();
    symlink(source.join("alpha"), &claude).unwrap();
    let state = temp.path().join("skills.json");
    std::fs::write(
        &state,
        r#"{"repos":[],"installed":{"alpha":{"id":"alpha"}}}"#,
    )
    .unwrap();
    let manager =
        SkillsManager::new(&source, temp.path().join("backups"), &state, &home).with_agent_dirs(
            vec![("claude".into(), claude.parent().unwrap().to_path_buf())],
        );
    manager.prepare_managed_skills().unwrap();
    assert!(
        !std::fs::symlink_metadata(&codex)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        !std::fs::symlink_metadata(&claude)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    std::fs::write(codex.join("SKILL.md"), "codex edit").unwrap();
    std::fs::write(claude.join("SKILL.md"), "claude edit").unwrap();
    assert_eq!(
        std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(),
        "old"
    );
    assert!(manager.codex_copy_conflict("alpha").unwrap());
}

#[test]
#[cfg(windows)]
fn failed_uninstall_must_not_leave_stale_installed_state() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let backups = temp.path().join("backups");
    let home = temp.path().join("home");
    let state = temp.path().join("skills.json");
    let manager = SkillsManager::new(&source, &backups, &state, &home);
    let local = home.join("skills/alpha");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("SKILL.md"), "user original").unwrap();
    std::fs::write(
        &state,
        r#"{"repos":[],"installed":{"alpha":{"id":"alpha"}}}"#,
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&state, permissions).unwrap();
    let result = manager.uninstall("alpha");
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&state, permissions).unwrap();
    assert!(result.is_err(), "readonly state must reject persistence");
    assert!(manager.load_state().installed.contains_key("alpha"));
    assert!(
        local.is_dir(),
        "failed uninstall removed the installation but retained its installed record; original contents only remain in backup"
    );
    assert_eq!(
        std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
        "user original"
    );
    assert!(manager.list_backups().is_empty());
}

#[test]
#[cfg(windows)]
fn failed_local_disable_must_not_hide_the_skill() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let backups = temp.path().join("backups");
    let home = temp.path().join("home");
    let state = temp.path().join("skills.json");
    let manager = SkillsManager::new(&source, &backups, &state, &home);
    let local = home.join("skills/alpha");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("SKILL.md"), "user original").unwrap();
    std::fs::write(&state, r#"{"repos":[],"installed":{}}"#).unwrap();
    assert!(
        manager
            .local_inventory()
            .iter()
            .any(|skill| skill.id == "alpha")
    );
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&state, permissions).unwrap();
    let result = manager.set_enabled("alpha", false);
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&state, permissions).unwrap();
    assert!(result.is_err(), "readonly state must reject persistence");
    assert!(
        manager
            .local_inventory()
            .iter()
            .any(|skill| skill.id == "alpha"),
        "failed disable moved the only local directory to source without registering it; inventory and backups no longer expose the skill"
    );
    assert_eq!(
        std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
        "user original"
    );
    assert!(!source.join("alpha").exists());
    assert!(manager.list_backups().iter().all(|item| {
        std::fs::read_to_string(backups.join(&item.id).join("SKILL.md"))
            .is_ok_and(|text| text == "user original")
    }));
}

#[test]
#[cfg(windows)]
fn failed_copy_disable_restores_both_directories_and_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let backups = temp.path().join("backups");
    let home = temp.path().join("home");
    let state = temp.path().join("skills.json");
    let manager = SkillsManager::new(&source, &backups, &state, &home);
    let local = home.join("skills/alpha");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(source.join("alpha")).unwrap();
    std::fs::write(local.join("SKILL.md"), "user edits").unwrap();
    std::fs::write(source.join("alpha/SKILL.md"), "old source").unwrap();
    std::fs::write(&state, r#"{"repos":[],"installed":{"alpha":{"id":"alpha","repoKey":"acme/kit@main","repoPath":"alpha"}}}"#).unwrap();
    let before = manager.load_state();
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&state, permissions).unwrap();
    let result = manager.set_enabled("alpha", false);
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&state, permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(manager.load_state(), before);
    assert_eq!(
        std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
        "user edits"
    );
    assert_eq!(
        std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(),
        "old source"
    );
    assert!(manager.list_backups().is_empty());
}

#[test]
#[cfg(windows)]
fn failed_second_backup_restores_the_first_uninstall_move() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let backups = temp.path().join("backups");
    let home = temp.path().join("home");
    let state = temp.path().join("skills.json");
    let manager = SkillsManager::new(&source, &backups, &state, &home);
    let local = home.join("skills/alpha");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(source.join("alpha")).unwrap();
    std::fs::write(local.join("SKILL.md"), "local edits").unwrap();
    std::fs::write(source.join("alpha/SKILL.md"), "source content").unwrap();
    std::fs::write(
        &state,
        r#"{"repos":[],"installed":{"alpha":{"id":"alpha"}}}"#,
    )
    .unwrap();
    let before = manager.load_state();
    // 不共享删除权限的目录句柄模拟 Windows 文件占用，阻止第二次移动。
    let occupied = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(0x02000000)
        .open(source.join("alpha"))
        .unwrap();
    let result = manager.uninstall("alpha");
    drop(occupied);
    assert!(result.is_err());
    assert_eq!(manager.load_state(), before);
    assert_eq!(
        std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
        "local edits"
    );
    assert_eq!(
        std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(),
        "source content"
    );
    assert!(manager.list_backups().is_empty());
}
