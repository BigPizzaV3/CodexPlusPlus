//! Public-API regressions for Skill lifecycle data preservation.
use codex_plus_core::skills::SkillsManager;

fn version_fixture(temp: &tempfile::TempDir) -> SkillsManager {
    let source = temp.path().join("source");
    let local = temp.path().join("home/skills/alpha");
    for directory in [&source.join("alpha"), &local] {
        std::fs::create_dir_all(directory.join("scripts")).unwrap();
        std::fs::write(directory.join("SKILL.md"), "---\nname: alpha\n---\n").unwrap();
    }
    std::fs::write(source.join("alpha/scripts/run.txt"), "managed").unwrap();
    std::fs::write(local.join("scripts/run.txt"), "local edits").unwrap();
    let state = temp.path().join("skills.json");
    std::fs::write(&state, r#"{"repos":[],"installed":{"alpha":{"id":"alpha","repoKey":"acme/kit@main","repoPath":"alpha","contentHash":"original-remote"}}}"#).unwrap();
    SkillsManager::new(source, temp.path().join("backups"), state, temp.path().join("home"))
}

#[test]
fn inventory_detects_script_only_changes_and_both_resolution_choices_preserve_backups() {
    for keep_local in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let manager = version_fixture(&temp);
        assert!(manager.local_inventory()[0].local_difference);
        manager.resolve_local_version("alpha", keep_local).unwrap();
        let expected = if keep_local { "local edits" } else { "managed" };
        for directory in [temp.path().join("source/alpha"), temp.path().join("home/skills/alpha")] {
            assert_eq!(std::fs::read_to_string(directory.join("scripts/run.txt")).unwrap(), expected);
        }
        assert!(!manager.local_inventory()[0].local_difference);
        assert_eq!(manager.load_state().installed["alpha"].content_hash, "original-remote");
        let contents: Vec<_> = manager.list_backups().iter().map(|backup|
            std::fs::read_to_string(temp.path().join("backups").join(&backup.id).join("scripts/run.txt")).unwrap()
        ).collect();
        assert!(contents.iter().any(|text| text == "local edits"));
        assert!(contents.iter().any(|text| text == "managed"));
    }
}

#[test]
fn directory_comparison_catches_added_and_removed_files() {
    let temp = tempfile::tempdir().unwrap();
    let manager = version_fixture(&temp);
    std::fs::write(temp.path().join("source/alpha/scripts/run.txt"), "local edits").unwrap();
    assert!(!manager.local_difference("alpha").unwrap());
    std::fs::write(temp.path().join("home/skills/alpha/extra.txt"), "added").unwrap();
    assert!(manager.local_difference("alpha").unwrap());
    std::fs::write(temp.path().join("source/alpha/extra.txt"), "added").unwrap();
    assert!(!manager.local_difference("alpha").unwrap());
}

#[test]
fn codex_local_skills_are_discovered_without_import() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("home/skills/new-local");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("SKILL.md"), "---\nname: new-local\n---\n").unwrap();
    let manager = SkillsManager::new(temp.path().join("source"), temp.path().join("backups"), temp.path().join("skills.json"), temp.path().join("home"));
    let entries = manager.local_inventory();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].enabled && entries[0].installed);
    assert!(entries[0].repo_key.is_empty());
}

#[test]
fn verified_origin_association_retains_files_and_enables_update_detection() {
    use codex_plus_core::skills::{RemoteSkill, SkillRepo, skill_directory_hash};
    let temp = tempfile::tempdir().unwrap();
    let manager = version_fixture(&temp);
    std::fs::write(temp.path().join("skills.json"), r#"{"repos":[],"installed":{"alpha":{"id":"alpha","repoKey":"cc-switch"}}}"#).unwrap();
    let repo = SkillRepo { owner: "acme".into(), name: "kit".into(), branch: "main".into(), subdir: String::new(), enabled: true };
    let local_hash = skill_directory_hash(&temp.path().join("home/skills/alpha")).unwrap();
    let mut remote = RemoteSkill { id: "alpha--stable".into(), name: "alpha".into(), description: String::new(),
        repo_key: repo.key(), repo_path: "nested/alpha".into(), content_hash: local_hash };
    manager.associate_origin("alpha", &repo, &remote).unwrap();
    assert_eq!(manager.list_repos(), vec![repo]);
    assert_eq!(manager.load_state().installed["alpha"].repo_path, "nested/alpha");
    assert!(!manager.merge_entries(&[remote.clone()])[0].update_available);
    remote.content_hash = "new-upstream".into();
    assert!(manager.merge_entries(&[remote])[0].update_available);
    assert_eq!(std::fs::read_to_string(temp.path().join("home/skills/alpha/scripts/run.txt")).unwrap(), "local edits");
}

#[test]
fn ccs_origin_uses_explicit_repository_metadata_and_readme_path() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("cc.db");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("CREATE TABLE skills(directory TEXT,repo_owner TEXT,repo_name TEXT,repo_branch TEXT,readme_url TEXT);
        INSERT INTO skills VALUES('alpha','acme','kit','main','https://github.com/acme/kit/blob/main/nested/alpha/SKILL.md');
        INSERT INTO skills VALUES('local',NULL,NULL,NULL,NULL);").unwrap();
    let hints = codex_plus_core::skills::read_ccs_skill_origins(&db).unwrap();
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].repo_path.as_deref(), Some("nested/alpha"));
    assert_eq!(hints[0].repo.key(), "acme/kit@main");
}

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
    let manager = SkillsManager::new(&source, temp.path().join("backups"), temp.path().join("skills.json"), &home);
    assert!(manager.set_enabled("alpha", true).is_err());
    assert_eq!(std::fs::read_to_string(local.join("notes.txt")).unwrap(), "user notes");
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
    let manager = SkillsManager::new(&source, temp.path().join("backups"), temp.path().join("skills.json"), &home);
    let before = manager.load_state();
    assert!(manager.restore_backup("alpha-123").is_err());
    assert_eq!(manager.load_state(), before);
    assert_eq!(std::fs::read_to_string(backup.join("SKILL.md")).unwrap(), "original backup");
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
    let manager = SkillsManager::new(temp.path().join("source"), temp.path().join("backups"), temp.path().join("skills.json"), &home);
    assert!(manager.restore_backup("alpha-123").is_err());
    assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "local edits");
    assert_eq!(std::fs::read_to_string(backup.join("SKILL.md")).unwrap(), "backup");
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
    assert_eq!(std::fs::read_to_string(backup.join("SKILL.md")).unwrap(), "original backup");
    assert!(!source.join("alpha").exists());
    assert!(!home.join("skills/alpha").exists());
    assert_eq!(manager.list_backups().len(), 1);
}

#[test]
fn disabling_rejects_staging_inside_the_original_skill() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let local = home.join("skills/alpha");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("SKILL.md"), "original").unwrap();
    let source = local.join("managed");
    let manager = SkillsManager::new(&source, temp.path().join("backups"), temp.path().join("skills.json"), &home);
    assert!(manager.set_enabled("alpha", false).is_err());
    assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "original");
    assert!(manager.list_backups().is_empty());
    assert_eq!(std::fs::read_dir(source).unwrap().count(), 0);
}

#[test]
fn disabling_a_copied_installation_must_preserve_user_edits() {
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
    manager.set_enabled("alpha", false).unwrap();
    let mut preserved = std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap() == "user edits";
    for backup in manager.list_backups() {
        preserved |= std::fs::read_to_string(backups.join(backup.id).join("SKILL.md")).unwrap() == "user edits";
    }
    assert!(preserved, "disable succeeded but removed the only copy of user edits; source is unchanged and backups are empty");
    assert!(!local.exists());
    assert_eq!(std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(), "user edits");
    let installed = &manager.load_state().installed["alpha"];
    assert_eq!(installed.repo_key, "acme/kit@main");
    assert_eq!(installed.repo_path, "alpha");
    assert_eq!(installed.content_hash, "old");
    manager.set_enabled("alpha", true).unwrap();
    assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "user edits");
    std::fs::write(local.join("SKILL.md"), "more edits").unwrap();
    manager.set_enabled("alpha", true).unwrap();
    assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "more edits");
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
    std::fs::write(&state, r#"{"repos":[],"installed":{"alpha":{"id":"alpha"}}}"#).unwrap();
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&state, permissions).unwrap();
    let result = manager.uninstall("alpha");
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&state, permissions).unwrap();
    assert!(result.is_err(), "readonly state must reject persistence");
    assert!(manager.load_state().installed.contains_key("alpha"));
    assert!(local.is_dir(), "failed uninstall removed the installation but retained its installed record; original contents only remain in backup");
    assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "user original");
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
    assert!(manager.local_inventory().iter().any(|skill| skill.id == "alpha"));
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&state, permissions).unwrap();
    let result = manager.set_enabled("alpha", false);
    let mut permissions = std::fs::metadata(&state).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&state, permissions).unwrap();
    assert!(result.is_err(), "readonly state must reject persistence");
    assert!(manager.local_inventory().iter().any(|skill| skill.id == "alpha"), "failed disable moved the only local directory to source without registering it; inventory and backups no longer expose the skill");
    assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "user original");
    assert!(!source.join("alpha").exists());
    assert!(manager.list_backups().is_empty());
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
    assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "user edits");
    assert_eq!(std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(), "old source");
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
    std::fs::write(&state, r#"{"repos":[],"installed":{"alpha":{"id":"alpha"}}}"#).unwrap();
    let before = manager.load_state();
    // 不共享删除权限的目录句柄模拟 Windows 文件占用，阻止第二次移动。
    let occupied = std::fs::OpenOptions::new().read(true).share_mode(3)
        .custom_flags(0x02000000).open(source.join("alpha")).unwrap();
    let result = manager.uninstall("alpha");
    drop(occupied);
    assert!(result.is_err());
    assert_eq!(manager.load_state(), before);
    assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "local edits");
    assert_eq!(std::fs::read_to_string(source.join("alpha/SKILL.md")).unwrap(), "source content");
    assert!(manager.list_backups().is_empty());
}
