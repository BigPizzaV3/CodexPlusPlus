use codex_plus_core::skills::SkillsManager;
use std::{collections::BTreeMap, fs};

#[test]
fn reconciling_a_modified_agent_copy_preserves_previous_versions_and_allows_updates() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let manager = SkillsManager::new(root.join("source"), root.join("backups"), root.join("state.json"), root.join("home"))
        .with_agent_dirs(vec![("claude".into(), root.join("claude"))]);
    for (directory, text) in [("source", "old"), ("home/skills", "edit"), ("claude", "edit")] {
        let path = root.join(directory).join("reviewer"); fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), text).unwrap();
        if directory != "source" { fs::write(path.join(".codex-plus-managed"), "").unwrap(); }
    }
    fs::write(root.join("state.json"), r#"{"repos":[{"owner":"o","name":"r","branch":"main","subdir":"","enabled":true}],"installed":{"reviewer":{"id":"reviewer","repoKey":"o/r@main","repoPath":"reviewer","contentHash":"v1","installedAt":"1"}}}"#).unwrap();
    assert!(manager.try_local_inventory().unwrap().iter().any(|s| s.id == "reviewer" && s.conflict.is_none() && s.managed_source_status.as_deref() == Some("needs-sync")));
    let revisions = || manager.local_versions("reviewer").unwrap().into_iter().map(|v| (v.root,v.revision)).collect::<BTreeMap<_,_>>();
    let stale = revisions();
    fs::write(root.join("claude/reviewer/SKILL.md"), "newest").unwrap();
    assert!(manager.sync_installed_copies("reviewer", revisions()).is_err());
    assert!(manager.reconcile_managed_versions("reviewer", "claude", stale).is_err());
    manager.reconcile_managed_versions("reviewer", "claude", revisions()).unwrap();
    for directory in ["source", "home/skills", "claude"] { assert_eq!(fs::read_to_string(root.join(directory).join("reviewer/SKILL.md")).unwrap(), "newest"); }
    assert_eq!(manager.try_list_backups().unwrap().len(), 3);
    let guard = manager.capture_download_guard("reviewer", "o/r@main", false).unwrap();
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("r-main/reviewer/SKILL.md", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(b"---\nname: reviewer\ndescription: fixture\n---\nupstream").unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    manager.install_downloaded_zip(&bytes, &guard).unwrap();
    for directory in ["source", "home/skills", "claude"] { assert!(fs::read_to_string(root.join(directory).join("reviewer/SKILL.md")).unwrap().ends_with("upstream")); }
    manager.uninstall("reviewer").unwrap();
    assert!(!root.join("home/skills/reviewer").exists());
}

#[test]
fn matching_agent_installs_sync_without_picking_an_agent_and_reject_stale_state() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let manager = SkillsManager::new(root.join("source"), root.join("backups"), root.join("state.json"), root.join("home"))
        .with_agent_dirs(vec![("claude".into(), root.join("claude")), ("grok".into(), root.join("grok"))]);
    for (directory, text) in [("source", "old"), ("home/skills", "local"), ("claude", "local")] {
        let dir = root.join(directory).join("alpha"); fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), text).unwrap();
        if directory != "source" { fs::write(dir.join(".codex-plus-managed"), "").unwrap(); }
    }
    fs::write(root.join("state.json"), r#"{"repos":[],"installed":{"alpha":{"id":"alpha","repoKey":"a/b@main","repoPath":"alpha","contentHash":"v1"}}}"#).unwrap();
    let revisions = || manager.local_versions("alpha").unwrap().into_iter().map(|v| (v.root, v.revision)).collect();
    let stale = revisions();
    fs::write(root.join("source/alpha/SKILL.md"), "previous").unwrap();
    assert!(manager.sync_installed_copies("alpha", stale).is_err());
    manager.sync_installed_copies("alpha", revisions()).unwrap();
    manager.set_enabled_for("grok", "alpha", true).unwrap();
    for directory in ["source", "home/skills", "claude", "grok"] { assert_eq!(fs::read_to_string(root.join(directory).join("alpha/SKILL.md")).unwrap(), "local"); }
    assert!(manager.try_list_backups().unwrap().iter().any(|v| fs::read_to_string(std::path::Path::new(&v.backup_path).join("SKILL.md")).is_ok_and(|text| text == "previous")));
}

#[test]
fn retaining_same_name_versions_leaves_files_independent_and_identifies_local_sources() {
    let temp = tempfile::tempdir().unwrap(); let root = temp.path();
    let manager = SkillsManager::new(root.join("source"), root.join("backups"), root.join("state.json"), root.join("home"))
        .with_agent_dirs(vec![("claude".into(), root.join("claude"))]);
    for (directory, text) in [("home/skills", "a"), ("claude", "b")] { let p = root.join(directory).join("x"); fs::create_dir_all(&p).unwrap(); fs::write(p.join("SKILL.md"), text).unwrap(); fs::write(p.join(".codex-plus-managed"), "").unwrap(); }
    let revisions = || manager.local_versions("x").unwrap().into_iter().map(|v| (v.root,v.revision)).collect::<BTreeMap<_,_>>();
    manager.keep_local_versions("x", revisions()).unwrap();
    assert!(!root.join("claude/x/.codex-plus-managed").exists());
    let inventory = manager.try_local_inventory().unwrap();
    assert_eq!(inventory.iter().filter(|s| s.id == "x" && s.repo_key == "local").count(), 2);
    manager.reconcile_managed_versions("x", "claude", revisions()).unwrap();
    assert_eq!(fs::read_to_string(root.join("claude/x/SKILL.md")).unwrap(), "b");
    assert!(!root.join("home/skills/x").exists());
    let backup = &manager.try_list_backups().unwrap()[0]; manager.restore_backup(&backup.id).unwrap();
    assert_eq!(fs::read_to_string(root.join("home/skills/x/SKILL.md")).unwrap(), "a");
}

#[test]
fn selected_version_archive_restore_export_and_enable_never_overwrite_another_agent() {
    let temp = tempfile::tempdir().unwrap(); let root = temp.path();
    let manager = SkillsManager::new(root.join("source"), root.join("backups"), root.join("state.json"), root.join("home"))
        .with_agent_dirs(vec![("claude".into(), root.join("claude")), ("grok".into(), root.join("grok"))]);
    let path = root.join("claude/alpha"); fs::create_dir_all(&path).unwrap(); fs::write(path.join("SKILL.md"), "local").unwrap();
    let revision = &manager.local_versions("alpha").unwrap()[0].revision;
    manager.enable_local_version("alpha", "claude", revision, "grok").unwrap();
    assert!(manager.enable_local_version("alpha", "claude", revision, "grok").is_err());
    manager.archive_local_version("alpha", "claude", revision).unwrap();
    assert!(!path.exists());
    let backup = &manager.try_list_backups().unwrap()[0];
    manager.export_skill_backup(&backup.id, root).unwrap();
    let codex_dir = root.join("home/skills");
    assert!(!codex_dir.exists());
    manager.export_skill_backup(&backup.id, &codex_dir).unwrap();
    assert!(codex_dir.join("alpha/SKILL.md").is_file());
    assert!(manager.export_skill_backup(&backup.id, root).is_err());
    manager.restore_backup(&backup.id).unwrap();
    for dir in ["claude/alpha", "grok/alpha", "alpha"] { assert_eq!(fs::read_to_string(root.join(dir).join("SKILL.md")).unwrap(), "local"); }
}

#[test]
fn a_local_version_backup_restores_only_to_its_configured_empty_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let source = root.join("source");
    let backups = root.join("backups");
    let state = root.join("state.json");
    let home = root.join("home");
    let claude = root.join("claude");
    let manager = SkillsManager::new(&source, &backups, &state, &home)
        .with_agent_dirs(vec![("claude".into(), claude.clone())]);
    fs::create_dir_all(claude.join("alpha")).unwrap();
    fs::write(claude.join("alpha/SKILL.md"), "original").unwrap();
    let revision = manager.local_versions("alpha").unwrap()[0].revision.clone();
    manager.archive_local_version("alpha", "claude", &revision).unwrap();
    let backup = manager.try_list_backups().unwrap().remove(0);

    let moved_root = SkillsManager::new(&source, &backups, &state, &home)
        .with_agent_dirs(vec![("claude".into(), root.join("other-claude"))]);
    assert!(moved_root.restore_backup(&backup.id).is_err());
    assert!(!claude.join("alpha").exists());

    fs::create_dir(claude.join("alpha")).unwrap();
    fs::write(claude.join("alpha/SKILL.md"), "replacement").unwrap();
    assert!(manager.restore_backup(&backup.id).is_err());
    assert_eq!(fs::read_to_string(claude.join("alpha/SKILL.md")).unwrap(), "replacement");
    assert!(std::path::Path::new(&backup.backup_path).join("SKILL.md").is_file());
}
