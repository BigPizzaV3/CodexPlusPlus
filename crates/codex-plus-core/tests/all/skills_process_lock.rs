use codex_plus_core::skills::SkillsManager;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct TestChild(Child);

impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn manager(root: &Path, state: &str) -> SkillsManager {
    SkillsManager::new(
        root.join("library"),
        root.join("backups"),
        root.join(state),
        root.join("codex"),
    )
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn child(root: &Path, mode: &str) -> TestChild {
    TestChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "skills_process_lock::process_lock_and_recovery",
                "--nocapture",
            ])
            .env("SKILL_LOCK_TEST_ROOT", root)
            .env("SKILL_LOCK_TEST_MODE", mode)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

#[test]
fn process_lock_and_recovery() {
    if let Some(root) = std::env::var_os("SKILL_LOCK_TEST_ROOT") {
        let root = Path::new(&root);
        match std::env::var("SKILL_LOCK_TEST_MODE").unwrap().as_str() {
            "hold" => {
                let manager = manager(root, "one.json");
                let _guard = manager.lock_recovered_transactions().unwrap();
                std::fs::write(root.join("held"), b"held").unwrap();
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
            "recover" => {
                std::fs::write(root.join("started"), b"started").unwrap();
                assert!(manager(root, "two.json").prepare_managed_skills().is_err());
                std::fs::write(root.join("done"), b"done").unwrap();
            }
            _ => unreachable!(),
        }
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut holder = child(root, "hold");
    wait_for(&root.join("held"));
    let journal = root.join(".skill-transactions").join("active.json");
    std::fs::write(&journal, b"unfinished").unwrap();
    let mut recoverer = child(root, "recover");
    wait_for(&root.join("started"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!root.join("done").exists());
    assert!(journal.exists());
    holder.0.kill().unwrap();
    holder.0.wait().unwrap();
    wait_for(&root.join("done"));
    assert!(recoverer.0.wait().unwrap().success());
    assert!(journal.exists());
    assert!(
        root.join(".skill-transactions")
            .join("operation.lock")
            .exists()
    );
}

#[test]
fn delete_backup_does_not_prepare_skills_or_recover_journals() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let backup = root.join("backups").join("alpha-123");
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::write(backup.join("SKILL.md"), b"backup").unwrap();
    let unmanaged = root.join("codex/skills/unmanaged");
    std::fs::create_dir_all(&unmanaged).unwrap();
    std::fs::write(unmanaged.join("SKILL.md"), b"---\nname: unmanaged\n---\n").unwrap();
    let journal_dir = root.join(".skill-transactions");
    std::fs::create_dir_all(&journal_dir).unwrap();
    std::fs::write(journal_dir.join("broken.json"), b"broken").unwrap();

    manager(root, "state.json")
        .delete_backup("alpha-123")
        .unwrap();
    assert!(!backup.exists());
    assert!(unmanaged.join("SKILL.md").exists());
    assert!(!root.join("library/unmanaged").exists());
    assert!(!root.join("state.json").exists());
    assert!(journal_dir.join("broken.json").exists());
}

#[test]
fn invalid_backup_ids_are_rejected_before_lock_or_preparation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let manager = manager(root, "state.json");
    assert!(manager.delete_backup("../invalid").is_err());
    assert!(manager.restore_backup("../invalid").is_err());
    assert!(manager.restore_backup("-123").is_err());
    assert!(!root.join(".skill-transactions").exists());
    assert!(!root.join("library").exists());
    assert!(!root.join("state.json").exists());
}
