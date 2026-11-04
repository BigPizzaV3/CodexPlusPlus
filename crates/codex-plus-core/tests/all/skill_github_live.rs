//! Opt-in public GitHub repository smoke test.
use codex_plus_core::skills::{
    SkillRepo, download_repo_zip, extract_skill_subtree, resolve_repo_skills,
};
use std::collections::BTreeMap;

#[tokio::test]
#[ignore = "requires network access to a public GitHub repository"]
async fn planning_with_files_catalog_and_zip_can_be_read() {
    let repo = SkillRepo {
        owner: "OthmanAdi".into(),
        name: "planning-with-files".into(),
        branch: "master".into(),
        subdir: "skills/planning-with-files".into(),
        enabled: true,
    };
    let (repo, skills) = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        resolve_repo_skills(repo, &BTreeMap::new()),
    )
    .await
    .expect("repository scan timed out")
    .unwrap();
    let skill = skills
        .iter()
        .find(|skill| skill.repo_path == "skills/planning-with-files")
        .expect("repository catalog should contain planning-with-files");
    let archive =
        tokio::time::timeout(std::time::Duration::from_secs(90), download_repo_zip(&repo))
            .await
            .expect("repository download timed out")
            .unwrap();
    let temp = tempfile::tempdir().unwrap();
    extract_skill_subtree(&archive, &skill.repo_path, temp.path()).unwrap();
    assert!(temp.path().join("SKILL.md").is_file());
}
