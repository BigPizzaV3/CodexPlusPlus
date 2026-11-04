//! 安装时只读取目标 Skill 子树；目录发现不解压任何文件正文。

use std::io::Cursor;
use std::path::Path;

use anyhow::Context;
use sha2::{Digest, Sha256};

use super::{
    parse_skill_frontmatter, parse_skills_from_tree, read_bounded_zip_entry, safe_relative_path,
    validate_skill_id, validate_zip_entries, zip_entry_relative_path, RemoteSkill, SkillRepo,
    REPO_ZIP_UNPACKED_LIMIT_BYTES, SKILL_MANIFEST_FILE,
};

/// 调用方提供已创建的空暂存目录，并负责失败后的暂存目录处理。
/// 已安装项按精确路径定位且保留旧 ID；新安装项使用清单树生成的稳定 ID。
pub(super) fn stage_downloaded_zip(
    repo: &SkillRepo,
    zip: &[u8],
    id: &str,
    installed_path: Option<&str>,
    staging: &Path,
) -> anyhow::Result<RemoteSkill> {
    validate_skill_id(id)?;
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).context("skill 仓库压缩包无法解析")?;

    // 稳定 ID 只依赖仓库键与路径，发现阶段不需要 blob 内容或真实 sha。
    // 使用 raw 条目只查看 headers，无关 Skill 的清单也不读取。
    let mut manifests = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index_raw(index)?;
        if file.is_dir() {
            continue;
        }
        let Some(relative) = archive_relative_path(file.name()) else {
            continue;
        };
        if relative == SKILL_MANIFEST_FILE || relative.ends_with("/SKILL.md") {
            manifests.push(serde_json::json!({ "type": "blob", "path": relative }));
        }
    }
    let mut skill = parse_skills_from_tree(repo, &serde_json::json!({ "tree": manifests }))
        .into_iter()
        .find(|skill| match installed_path {
            Some(path) => skill.repo_path == path,
            None => skill.id == id,
        })
        .with_context(|| match installed_path {
            Some(path) => format!("仓库压缩包中未找到 Skill 路径：{path}"),
            None => format!("仓库压缩包中未找到 Skill：{id}"),
        })?;
    skill.id = id.to_string();

    // 写入前统一校验根、冲突路径和目标子树的文件名别名。
    validate_zip_entries(&mut archive, Some(&skill.repo_path))?;
    let prefix = if skill.repo_path == "." {
        String::new()
    } else {
        format!("{}/", skill.repo_path)
    };
    let mut files = Vec::new();
    let mut total_size = 0;
    let mut wrote_manifest = false;
    for index in 0..archive.len() {
        let file = archive.by_index_raw(index)?;
        let Some(relative) = archive_relative_path(file.name()) else {
            continue;
        };
        if file.is_symlink()
            && (skill.repo_path == "."
                || relative == skill.repo_path
                || relative.starts_with(&prefix))
        {
            anyhow::bail!("skill 中不允许包含符号链接：{}", file.name());
        }
        let Some(inner) = relative.strip_prefix(prefix.as_str()) else {
            continue;
        };
        if inner.is_empty() {
            continue;
        }
        let output_path = staging.join(safe_relative_path(inner)?);
        if file.is_dir() {
            std::fs::create_dir_all(&output_path)
                .with_context(|| format!("创建目录失败：{}", output_path.display()))?;
            continue;
        }
        drop(file);

        let mut file = archive
            .by_index(index)
            .with_context(|| format!("读取压缩包条目 {index} 失败"))?;
        let declared_size = file.size();
        let contents = read_bounded_zip_entry(
            &mut file,
            declared_size,
            &mut total_size,
            REPO_ZIP_UNPACKED_LIMIT_BYTES,
        )
        .with_context(|| format!("读取压缩包条目 {} 失败", file.name()))?;
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败：{}", parent.display()))?;
        }
        std::fs::write(&output_path, &contents)
            .with_context(|| format!("写入 {} 失败", output_path.display()))?;

        // 与 Git trees API / parse_skills_from_zip 使用相同的 blob ID。
        let mut blob = sha1::Sha1::new();
        blob.update(format!("blob {}\0", contents.len()).as_bytes());
        blob.update(&contents);
        files.push((inner.to_string(), format!("{:x}", blob.finalize())));
        if inner == SKILL_MANIFEST_FILE {
            (skill.name, skill.description) =
                parse_skill_frontmatter(&String::from_utf8_lossy(&contents), &skill.id);
            wrote_manifest = true;
        }
    }
    if !wrote_manifest {
        anyhow::bail!("{} 下没有 SKILL.md，不是一个有效的 skill", skill.repo_path);
    }

    // 与 parse_skills_from_tree 一致：相对路径排序，每个字段以 NUL 分隔。
    files.sort();
    let mut hasher = Sha256::new();
    for (path, sha) in files {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(sha.as_bytes());
        hasher.update([0]);
    }
    skill.content_hash = format!("{:x}", hasher.finalize());
    Ok(skill)
}

fn archive_relative_path(name: &str) -> Option<String> {
    // 与 extract_skill_subtree 一样，剥去仓库根后统一用 '/'，不依赖宿主分隔符。
    Some(
        zip_entry_relative_path(name)?
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Write;

    use super::super::{parse_repo_key, parse_skills_from_zip};
    use super::*;

    fn repo_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for &(name, contents) in entries {
            if name.ends_with('/') {
                writer.add_directory(name, options).unwrap();
            } else {
                writer.start_file(name, options).unwrap();
                writer.write_all(contents).unwrap();
            }
        }
        writer.finish().unwrap().into_inner()
    }

    fn skill_id(repo: &SkillRepo, manifest: &str) -> String {
        parse_skills_from_tree(
            repo,
            &serde_json::json!({ "tree": [{ "type": "blob", "path": manifest }] }),
        )
        .remove(0)
        .id
    }

    fn set_declared_size(zip: &mut [u8], name: &str, size: u32) {
        // 只修改中央目录的展开大小，不分配数百 MiB。若实现读取这个条目，
        // 大小上限或实际/声明大小不符必然报错，成功便证明它完全未被解压。
        let central = (0..zip.len().saturating_sub(46))
            .find(|&offset| {
                zip[offset..offset + 4] == *b"PK\x01\x02"
                    && u16::from_le_bytes([zip[offset + 28], zip[offset + 29]]) as usize
                        == name.len()
                    && zip.get(offset + 46..offset + 46 + name.len()) == Some(name.as_bytes())
            })
            .expect("fixture entry in central directory");
        zip[central + 24..central + 28].copy_from_slice(&size.to_le_bytes());
    }

    #[test]
    fn stages_small_skill_without_reading_oversized_unrelated_manifest() {
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let mut zip = repo_zip(&[
            ("kit-main/alpha-other/SKILL.md", b"unread"),
            ("kit-main/alpha/SKILL.md", b"---\nname: Alpha\n---\n"),
            ("kit-main/alpha/scripts/run.py", b"print('ok')"),
        ]);
        set_declared_size(
            &mut zip,
            "kit-main/alpha-other/SKILL.md",
            REPO_ZIP_UNPACKED_LIMIT_BYTES as u32 + 1,
        );
        let error = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap_err();
        assert!(format!("{error:#}").contains("256 MiB"));

        let staging = tempfile::tempdir().unwrap();
        let id = skill_id(&repo, "alpha/SKILL.md");
        let skill = stage_downloaded_zip(&repo, &zip, &id, None, staging.path()).unwrap();
        assert_eq!(skill.id, id);
        assert_eq!(skill.name, "Alpha");
        assert_eq!(skill.repo_path, "alpha");
        assert_eq!(
            std::fs::read(staging.path().join("scripts/run.py")).unwrap(),
            b"print('ok')"
        );
        assert!(!staging.path().join("alpha-other").exists());
    }

    #[test]
    fn rejects_oversized_target_and_cumulative_target_size() {
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let id = skill_id(&repo, "alpha/SKILL.md");
        for declared_size in [
            REPO_ZIP_UNPACKED_LIMIT_BYTES,
            REPO_ZIP_UNPACKED_LIMIT_BYTES + 1,
        ] {
            let mut zip = repo_zip(&[
                ("kit-main/alpha/SKILL.md", b"manifest"),
                ("kit-main/alpha/large.bin", b"not actually large"),
            ]);
            set_declared_size(&mut zip, "kit-main/alpha/large.bin", declared_size as u32);
            let staging = tempfile::tempdir().unwrap();
            let error = stage_downloaded_zip(&repo, &zip, &id, None, staging.path()).unwrap_err();
            assert!(format!("{error:#}").contains("256 MiB"));
            assert!(!staging.path().join("large.bin").exists());
        }
    }

    #[test]
    fn metadata_and_hash_match_full_zip_for_root_nested_and_scoped_skills() {
        // 顺序故意打乱；父 Skill 的 hash 必须包含嵌套 Skill 和二进制文件。
        let entries: &[(&str, &[u8])] = &[
            ("kit-main/plugins/alpha/z.bin", b"\0\xff\x01"),
            ("kit-main/plugins/beta/SKILL.md", b"beta"),
            ("kit-main/plugins/alpha/nested/SKILL.md", b"nested"),
            ("kit-main/SKILL.md", b"---\nname: Root\n---\n"),
            ("kit-main/plugins/alpha/empty/", b""),
            (
                "kit-main/plugins/alpha/SKILL.md",
                b"---\nname: Alpha\ndescription: >\n  First\n  Second\n---\n",
            ),
            ("kit-main/plugins/alpha/a.txt", b"hello\n"),
            ("kit-main/plugins/alpha-other/SKILL.md", b"other"),
        ];
        let zip = repo_zip(entries);
        for key in [
            "acme/kit@main",
            "acme/kit@main:plugins",
            "acme/kit@main:plugins/alpha",
        ] {
            let repo = parse_repo_key(key).unwrap();
            let expected = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap();
            assert!(!expected.is_empty());
            for expected in expected {
                let staging = tempfile::tempdir().unwrap();
                let actual =
                    stage_downloaded_zip(&repo, &zip, &expected.id, None, staging.path()).unwrap();
                assert_eq!(actual, expected);
                assert!(staging.path().join("SKILL.md").is_file());
                assert!(!staging.path().join("kit-main").exists());
                let prefix = if expected.repo_path == "." {
                    "kit-main/".to_string()
                } else {
                    format!("kit-main/{}/", expected.repo_path)
                };
                for &(name, contents) in entries {
                    if let Some(inner) = name.strip_prefix(&prefix) {
                        let output = staging.path().join(inner);
                        if name.ends_with('/') {
                            assert!(output.is_dir());
                        } else {
                            assert_eq!(std::fs::read(output).unwrap(), contents);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn legacy_installed_path_selects_exact_skill_and_preserves_id() {
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let zip = repo_zip(&[
            ("kit-main/a/alpha/SKILL.md", b"wrong"),
            (
                "kit-main/z/alpha/SKILL.md",
                b"---\ndescription: Legacy\n---\n",
            ),
        ]);
        let staging = tempfile::tempdir().unwrap();
        let skill =
            stage_downloaded_zip(&repo, &zip, "alpha", Some("z/alpha"), staging.path()).unwrap();
        assert_eq!(skill.id, "alpha");
        assert_eq!(skill.name, "alpha");
        assert_eq!(skill.description, "Legacy");
        assert_eq!(skill.repo_path, "z/alpha");
        let expected = parse_skills_from_zip(&repo, &zip, &BTreeMap::new())
            .unwrap()
            .into_iter()
            .find(|skill| skill.repo_path == "z/alpha")
            .unwrap();
        assert_eq!(skill.content_hash, expected.content_hash);
        assert_eq!(skill.repo_key, expected.repo_key);
        assert_eq!(
            std::fs::read(staging.path().join("SKILL.md")).unwrap(),
            b"---\ndescription: Legacy\n---\n"
        );

        // 路径缺失或非精确匹配时，不能退回按 ID 安装另一个同名 Skill。
        for path in ["", "missing", "z/alpha/", "./z/alpha"] {
            let staging = tempfile::tempdir().unwrap();
            assert!(
                stage_downloaded_zip(&repo, &zip, &expected.id, Some(path), staging.path())
                    .is_err()
            );
            assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn unknown_id_does_not_write_any_files() {
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let zip = repo_zip(&[("kit-main/alpha/SKILL.md", b"alpha")]);
        for id in ["unknown", "alpha"] {
            let staging = tempfile::tempdir().unwrap();
            let error = stage_downloaded_zip(&repo, &zip, id, None, staging.path()).unwrap_err();
            assert!(error.to_string().contains("未找到 Skill"));
            assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn rejects_only_selected_subtree_symlinks() {
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for name in ["kit-main/alpha/SKILL.md", "kit-main/beta/SKILL.md"] {
            writer.start_file(name, options).unwrap();
            writer.write_all(b"manifest").unwrap();
        }
        writer
            .add_symlink("kit-main/beta/link", "../../outside", options)
            .unwrap();
        writer
            .add_symlink("kit-main/alpha-other", "../../outside", options)
            .unwrap();
        let zip = writer.finish().unwrap().into_inner();
        let staging = tempfile::tempdir().unwrap();
        stage_downloaded_zip(
            &repo,
            &zip,
            &skill_id(&repo, "alpha/SKILL.md"),
            None,
            staging.path(),
        )
        .unwrap();
        let staging = tempfile::tempdir().unwrap();
        let error = stage_downloaded_zip(
            &repo,
            &zip,
            &skill_id(&repo, "beta/SKILL.md"),
            None,
            staging.path(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("符号链接"));
        assert!(!staging.path().join("link").exists());
    }

    #[test]
    fn rejects_invalid_archive_paths_before_writing() {
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let id = skill_id(&repo, "alpha/SKILL.md");
        for invalid in [
            "evil-main/alpha/other.txt",
            "kit-main/alpha/../escape.txt",
            "kit-main/alpha/./other.txt",
        ] {
            let zip = repo_zip(&[("kit-main/alpha/SKILL.md", b"alpha"), (invalid, b"invalid")]);
            let staging = tempfile::tempdir().unwrap();
            assert!(stage_downloaded_zip(&repo, &zip, &id, None, staging.path()).is_err());
            assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 0);
        }
    }
}
