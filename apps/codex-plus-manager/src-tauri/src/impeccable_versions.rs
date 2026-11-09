use std::io::Read;
use std::path::Path;

use anyhow::Context;
use codex_plus_core::skills::SkillsManager;
use serde::Serialize;

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpeccableVersions {
    pub skill_version: Option<String>,
    pub engine_version: Option<String>,
    pub required_engine_version: Option<String>,
}

fn version_scalar(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value.strip_prefix('"').and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')))
        .unwrap_or(value);
    (!value.is_empty() && value.len() <= 128
        && value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+' | '_')))
        .then(|| value.to_string())
}

fn metadata_version(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut in_metadata = false;
    let mut saw_metadata = false;
    let mut saw_version = false;
    let mut child_indent = None;
    let mut version = None;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            return version;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent == 0 {
            in_metadata = trimmed == "metadata:";
            if in_metadata {
                if saw_metadata { return None; }
                saw_metadata = true;
            }
            continue;
        }
        if !in_metadata {
            continue;
        }
        let direct_indent = *child_indent.get_or_insert(indent);
        if indent == direct_indent {
            if let Some(value) = trimmed.strip_prefix("version:") {
                if saw_version { return None; }
                saw_version = true;
                version = version_scalar(value);
            }
        }
    }
    None
}

fn read_within(root: &Path, path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("无法读取 {}", path.display())),
    }
    let resolved = path.canonicalize().with_context(|| format!("无法解析 {}", path.display()))?;
    anyhow::ensure!(resolved.starts_with(root), "版本文件位于 Skill 目录之外：{}", path.display());
    let mut bytes = Vec::new();
    std::fs::File::open(&resolved)?.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 1024 * 1024, "版本文件过大：{}", path.display());
    Ok(Some(String::from_utf8(bytes).with_context(|| format!("版本文件不是 UTF-8：{}", path.display()))?))
}

/// 只读取已发现的 impeccable 实例，不执行启动器或其他程序。
pub fn inspect_impeccable_versions(manager: &SkillsManager, instance_key: &str) -> anyhow::Result<ImpeccableVersions> {
    let skill = manager.try_local_inventory()?.into_iter()
        .find(|skill| skill.id == "impeccable" && skill.instance_key == instance_key)
        .context("未找到当前 impeccable 实例，请刷新技能列表")?;
    let location = skill.locations.first().context("未找到 impeccable 的实际位置")?;
    let manifest = Path::new(&location.manifest_path);
    let root = manifest.parent().context("Skill 路径没有父目录")?.canonicalize()?;
    let skill_version = read_within(&root, manifest)?.and_then(|text| metadata_version(&text));
    let required_engine_version = read_within(&root, &root.join("scripts").join("VERSION"))?
        .and_then(|text| version_scalar(&text));
    Ok(ImpeccableVersions { skill_version, engine_version: None, required_engine_version })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, SkillsManager, String) {
        let temp = tempfile::tempdir().unwrap();
        let shared = temp.path().join("shared");
        let root = shared.join("impeccable");
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::write(root.join("SKILL.md"), "---\nname: impeccable\nmetadata:\n  version: 4.8.0\n---\n").unwrap();
        std::fs::write(root.join("scripts/VERSION"), "0.1.18\n").unwrap();
        let manager = SkillsManager::new(
            temp.path().join("library"), temp.path().join("backups"),
            temp.path().join("state.json"), temp.path().join("codex"),
        ).with_shared_skills_dir(shared);
        let instance_key = manager.try_local_inventory().unwrap().into_iter()
            .find(|skill| skill.id == "impeccable").unwrap().instance_key;
        (temp, manager, instance_key)
    }

    #[test]
    fn versions_keep_engine_requirement_distinct_from_actual_engine() {
        let (_temp, manager, key) = fixture();
        let versions = inspect_impeccable_versions(&manager, &key).unwrap();
        assert_eq!(versions.skill_version.as_deref(), Some("4.8.0"));
        assert_eq!(versions.required_engine_version.as_deref(), Some("0.1.18"));
        assert!(versions.engine_version.is_none());
        assert!(inspect_impeccable_versions(&manager, "other").is_err());
    }

    #[test]
    fn unknown_or_nested_metadata_is_not_mistaken_for_a_version() {
        assert_eq!(metadata_version("---\nmetadata:\n  other:\n    version: 1.0\n---\n"), None);
        assert_eq!(metadata_version("---\nmetadata:\n  version: '4.8.0'\n---\n"), Some("4.8.0".into()));
        assert_eq!(metadata_version("---\nmetadata:\n  version: 4.8.0\n"), None);
        assert_eq!(metadata_version("---\nmetadata:\n  version: 4.8.0\n  version: 5.0\n---\n"), None);
    }

    #[test]
    fn version_reader_rejects_files_outside_the_skill_root() {
        let (temp, manager, key) = fixture();
        let root = temp.path().join("shared/impeccable").canonicalize().unwrap();
        let outside = temp.path().join("outside-version");
        std::fs::write(&outside, "9.9.9").unwrap();
        assert!(read_within(&root, &outside).is_err());
        let versions = inspect_impeccable_versions(&manager, &key).unwrap();
        assert_eq!(versions.required_engine_version.as_deref(), Some("0.1.18"));
    }

    #[cfg(unix)]
    #[test]
    fn version_reader_rejects_symlinks_outside_the_skill_root() {
        let (temp, manager, key) = fixture();
        let scripts = temp.path().join("shared/impeccable/scripts");
        let outside = temp.path().join("outside-version");
        std::fs::write(&outside, "9.9.9").unwrap();
        std::fs::remove_file(scripts.join("VERSION")).unwrap();
        std::os::unix::fs::symlink(&outside, scripts.join("VERSION")).unwrap();
        assert!(inspect_impeccable_versions(&manager, &key).is_err());
    }
}
