//! Skill 迁移包（#2852）：把选中的 Skill 连同分组、各 Agent 的启用状态打成一个 zip，换机器后一次导入。
//!
//! 包里是 `codex-plus-skills.json`（清单）和 `skills/<id>/…`（每个 Skill 的文件）。
//! 导入走「从本地目录导入」同一套流程；本机已有同名 Skill 时不覆盖文件，只恢复分组和启用状态。
//! 分组本身存在管理窗口里，清单只带分组键，由界面在导入后写回。

use std::io::Write;

use super::*;

const SKILL_BUNDLE_MANIFEST_FILE: &str = "codex-plus-skills.json";
const SKILL_BUNDLE_FORMAT: &str = "codex-plus-skill-bundle";
const SKILL_BUNDLE_VERSION: u32 = 1;
const SKILL_BUNDLE_SKILLS_DIR: &str = "skills";
const SKILL_BUNDLE_MANIFEST_LIMIT_BYTES: u64 = 4 * 1024 * 1024;

/// 迁移包里的一个 Skill。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillBundleSkill {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// 分组键：`custom:<名称>` 或 `repo:<owner/name>`；空串表示按来源。
    #[serde(default)]
    pub group: String,
    /// 导出时启用到了哪些 Agent。
    #[serde(default)]
    pub agents: Vec<String>,
    /// 导出时的来源，只作记录；导入后的来源是「本地导入」。
    #[serde(default)]
    pub repo_key: String,
    #[serde(default)]
    pub repo_path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillBundleManifest {
    format: String,
    version: u32,
    #[serde(default)]
    exported_at: String,
    /// 分组键，按导出时的显示顺序。
    #[serde(default)]
    groups: Vec<String>,
    #[serde(default)]
    skills: Vec<SkillBundleSkill>,
}

/// 界面选中的一个 Skill 和它所在的分组键。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillBundleSelection {
    pub id: String,
    #[serde(default)]
    pub group: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillBundleExport {
    pub exported: Vec<String>,
    /// 没能导出的 Skill 及原因。
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillBundleImport {
    pub groups: Vec<String>,
    /// 导入后本机有的 Skill（新导入的和原来就有的），界面据此恢复分组。
    pub skills: Vec<SkillBundleSkill>,
    pub imported: Vec<String>,
    /// 本机原来就有、没有覆盖文件的 Skill。
    pub existing: Vec<String>,
    pub errors: Vec<String>,
}

impl SkillsManager {
    /// 把选中的 Skill 写成迁移包。单个 Skill 导不出来（未安装、有冲突、含符号链接）时跳过并记下原因；
    /// 一个都导不出来才报错。包先写到同目录的临时文件，写完再改名，不会留下半个包。
    pub fn export_skill_bundle(
        &self,
        selections: &[SkillBundleSelection],
        groups: &[String],
        destination: &Path,
    ) -> anyhow::Result<SkillBundleExport> {
        anyhow::ensure!(!selections.is_empty(), "没有选择要导出的 Skill");
        let inventory = self.try_local_inventory()?;
        let mut outcome = SkillBundleExport::default();
        let mut skills = Vec::new();
        let mut sources = Vec::new();
        let mut seen = BTreeSet::new();
        for selection in selections {
            let id = selection.id.trim();
            if !seen.insert(id.to_string()) {
                continue;
            }
            match self.bundle_skill(&inventory, id, &selection.group) {
                Ok((skill, source)) => {
                    skills.push(skill);
                    sources.push(source);
                }
                Err(error) => outcome.errors.push(format!("{id}：{error:#}")),
            }
        }
        anyhow::ensure!(
            !skills.is_empty(),
            "没有可导出的 Skill：{}",
            outcome.errors.join("；")
        );
        let used: BTreeSet<&str> = skills.iter().map(|skill| skill.group.as_str()).collect();
        let manifest = SkillBundleManifest {
            format: SKILL_BUNDLE_FORMAT.to_string(),
            version: SKILL_BUNDLE_VERSION,
            exported_at: current_unix_timestamp_string(),
            groups: groups
                .iter()
                .filter(|group| !group.is_empty() && used.contains(group.as_str()))
                .cloned()
                .collect(),
            skills,
        };

        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .context("迁移包的保存位置无效")?;
        let staging = parent.join(format!(".codex-plus-skill-bundle-{}", uuid::Uuid::new_v4()));
        let written = (|| -> anyhow::Result<()> {
            let file = std::fs::File::create_new(&staging)
                .with_context(|| format!("创建 {} 失败", staging.display()))?;
            let mut writer = zip::ZipWriter::new(std::io::BufWriter::new(file));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file(SKILL_BUNDLE_MANIFEST_FILE, options)?;
            writer.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
            for (skill, source) in manifest.skills.iter().zip(&sources) {
                let prefix = format!("{SKILL_BUNDLE_SKILLS_DIR}/{}", skill.id);
                write_bundle_dir(&mut writer, source, &prefix, true)
                    .with_context(|| format!("打包 Skill {} 失败", skill.id))?;
            }
            writer.finish()?.flush()?;
            Ok(())
        })();
        if let Err(error) = written {
            let _ = std::fs::remove_file(&staging);
            return Err(error);
        }
        if let Err(error) = std::fs::rename(&staging, destination) {
            let _ = std::fs::remove_file(&staging);
            return Err(error).with_context(|| format!("写入 {} 失败", destination.display()));
        }
        outcome.exported = manifest.skills.into_iter().map(|skill| skill.id).collect();
        Ok(outcome)
    }

    /// 一个待导出的 Skill：清单条目和它的文件所在目录。
    fn bundle_skill(
        &self,
        inventory: &[SkillEntry],
        id: &str,
        group: &str,
    ) -> anyhow::Result<(SkillBundleSkill, PathBuf)> {
        validate_skill_id(id)?;
        let entries: Vec<&SkillEntry> = inventory
            .iter()
            .filter(|entry| entry.id == id && entry.installed && !entry.bundled)
            .collect();
        anyhow::ensure!(!entries.is_empty(), "未安装");
        anyhow::ensure!(
            entries.iter().all(|entry| entry.conflict.is_none()
                || entry.conflict_kind.as_deref() == Some("equal-independent")),
            "各安装位置的内容不同，请先解决冲突"
        );
        let mut roots = vec![self.source_dir.clone(), self.linked_dir()];
        roots.extend(self.shared_skills_dir().map(Path::to_path_buf));
        roots.extend(self.agent_dirs.iter().map(|(_, dir)| dir.clone()));
        let source = roots
            .into_iter()
            .map(|root| root.join(id))
            .find(|dir| dir.join(SKILL_MANIFEST_FILE).is_file())
            .context("找不到 Skill 的文件")?
            .canonicalize()?;
        // 按固定顺序列出：Codex 在前，其余按 Agent 目录的登记顺序。
        let agents = std::iter::once("codex")
            .chain(self.agent_dirs.iter().map(|(agent, _)| agent.as_str()))
            .filter(|agent| entries.iter().any(|entry| entry.agents.iter().any(|item| item == agent)))
            .map(str::to_string)
            .collect();
        let first = entries[0];
        Ok((
            SkillBundleSkill {
                id: id.to_string(),
                name: first.name.clone(),
                group: group.trim().to_string(),
                agents,
                repo_key: first.repo_key.clone(),
                repo_path: first.repo_path.clone(),
            },
            source,
        ))
    }

    /// 导入迁移包：解到临时目录，逐个按「从本地目录导入」装进受控源，再按清单启用到各 Agent。
    /// 单个 Skill 失败不影响其余的；返回的清单交给界面恢复分组。
    pub fn import_skill_bundle(&self, archive_path: &Path) -> anyhow::Result<SkillBundleImport> {
        let file = std::fs::File::open(archive_path)
            .with_context(|| format!("打不开迁移包：{}", archive_path.display()))?;
        let mut archive = zip::ZipArchive::new(BufReader::new(file))
            .context("迁移包不是有效的 zip 文件")?;
        let manifest = read_bundle_manifest(&mut archive)?;
        let staging = std::env::temp_dir().join(format!("codex-plus-skill-bundle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&staging)
            .with_context(|| format!("创建临时目录失败：{}", staging.display()))?;
        let result = (|| -> anyhow::Result<SkillBundleImport> {
            extract_bundle_skills(&mut archive, &manifest, &staging)?;
            let installed: BTreeSet<String> = self
                .try_local_inventory()?
                .into_iter()
                .filter(|entry| entry.installed)
                .map(|entry| entry.id)
                .collect();
            let mut outcome = SkillBundleImport {
                groups: manifest.groups.clone(),
                ..Default::default()
            };
            for skill in &manifest.skills {
                let directory = staging.join(&skill.id);
                if installed.contains(&skill.id) {
                    outcome.existing.push(skill.id.clone());
                } else if !directory.join(SKILL_MANIFEST_FILE).is_file() {
                    outcome.errors.push(format!("{}：迁移包里没有它的 SKILL.md", skill.id));
                    continue;
                } else if let Err(error) = self.import_from_local_directory(&skill.id, &directory) {
                    outcome.errors.push(format!("{}：{error:#}", skill.id));
                    continue;
                } else {
                    outcome.imported.push(skill.id.clone());
                }
                for agent in &skill.agents {
                    // 这台机器上没有的 Agent 直接跳过。
                    if agent != "codex" && !self.agent_dirs.iter().any(|(known, _)| known == agent) {
                        continue;
                    }
                    if let Err(error) = self.set_enabled_for(agent, &skill.id, true) {
                        outcome.errors.push(format!("{}（启用到 {agent}）：{error:#}", skill.id));
                    }
                }
                outcome.skills.push(skill.clone());
            }
            Ok(outcome)
        })();
        let _ = std::fs::remove_dir_all(&staging);
        result
    }
}

/// 不进迁移包的文件和目录（#2852）：本机的密钥文件、版本库、依赖和缓存目录、日志。
/// 迁移包会被拷到别的机器、发给别人，这些要么敏感，要么换台机器就没用。
/// `.env.example` 这类给人照着填的模板照常打包。
fn is_left_out_of_bundles(name: &str) -> bool {
    const DIRECTORIES: &[&str] = &[".git", "node_modules", "__pycache__", ".venv", "venv", ".pytest_cache", ".mypy_cache"];
    const TEMPLATE_SUFFIXES: &[&str] = &[".example", ".sample", ".template"];
    let lower = name.to_ascii_lowercase();
    if DIRECTORIES.contains(&lower.as_str()) || lower == ".ds_store" || lower.ends_with(".log") {
        return true;
    }
    (lower == ".env" || lower.starts_with(".env."))
        && !TEMPLATE_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix))
}

/// 把一个 Skill 目录写进 zip 的 `prefix/` 下。符号链接不打包（导入端也不接受），遇到就报错。
fn write_bundle_dir<W: Write + Seek>(
    writer: &mut zip::ZipWriter<W>,
    directory: &Path,
    prefix: &str,
    root: bool,
) -> anyhow::Result<()> {
    let mut entries = std::fs::read_dir(directory)
        .with_context(|| format!("读取目录失败：{}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let directory_options = zip::write::SimpleFileOptions::default();
    writer.add_directory(format!("{prefix}/"), directory_options)?;
    for entry in entries {
        let name = entry.file_name();
        // 副本标记只属于那一份副本，不跟着走。
        if root && name == MANAGED_COPY_MARKER {
            continue;
        }
        let name = name
            .to_str()
            .with_context(|| format!("文件名不是有效的 UTF-8：{}", entry.path().display()))?;
        if is_left_out_of_bundles(name) {
            continue;
        }
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        anyhow::ensure!(!metadata_is_link(&metadata), "包含符号链接：{}", path.display());
        let archived = format!("{prefix}/{name}");
        if metadata.is_dir() {
            write_bundle_dir(writer, &path, &archived, false)?;
            continue;
        }
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        #[cfg(unix)]
        let options = {
            use std::os::unix::fs::PermissionsExt;
            options.unix_permissions(metadata.permissions().mode() & 0o777)
        };
        writer.start_file(archived, options)?;
        let mut file = std::fs::File::open(&path)
            .with_context(|| format!("读取 {} 失败", path.display()))?;
        std::io::copy(&mut file, writer)?;
    }
    Ok(())
}

fn read_bundle_manifest<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> anyhow::Result<SkillBundleManifest> {
    let mut entry = archive
        .by_name(SKILL_BUNDLE_MANIFEST_FILE)
        .context("这不是 Codex++ 的 Skill 迁移包（缺少清单）")?;
    let declared = entry.size();
    anyhow::ensure!(declared <= SKILL_BUNDLE_MANIFEST_LIMIT_BYTES, "迁移包的清单过大");
    let mut total = 0;
    let bytes = read_bounded_zip_entry(&mut entry, declared, &mut total, SKILL_BUNDLE_MANIFEST_LIMIT_BYTES)?;
    let manifest: SkillBundleManifest =
        serde_json::from_slice(&bytes).context("迁移包的清单无法解析")?;
    anyhow::ensure!(manifest.format == SKILL_BUNDLE_FORMAT, "这不是 Codex++ 的 Skill 迁移包");
    anyhow::ensure!(
        manifest.version <= SKILL_BUNDLE_VERSION,
        "迁移包来自更新版本的 Codex++，请升级后再导入"
    );
    anyhow::ensure!(!manifest.skills.is_empty(), "迁移包里没有 Skill");
    let mut seen = BTreeSet::new();
    for skill in &manifest.skills {
        validate_skill_id(&skill.id)?;
        anyhow::ensure!(!skill.id.starts_with('.'), "非法的 skill id：{}", skill.id);
        anyhow::ensure!(seen.insert(skill.id.as_str()), "迁移包里有重复的 Skill：{}", skill.id);
    }
    Ok(manifest)
}

/// 把清单里列出的 Skill 解到 `staging/<id>/`。清单之外的条目不解；
/// 路径越界、符号链接、解压后超过上限都拒绝。
fn extract_bundle_skills<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    manifest: &SkillBundleManifest,
    staging: &Path,
) -> anyhow::Result<()> {
    let ids: BTreeSet<&str> = manifest.skills.iter().map(|skill| skill.id.as_str()).collect();
    let prefix = format!("{SKILL_BUNDLE_SKILLS_DIR}/");
    let mut total_size = 0;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .with_context(|| format!("读取迁移包条目 {index} 失败"))?;
        let name = file.name().to_string();
        let Some(inner) = name.strip_prefix(&prefix) else {
            continue;
        };
        anyhow::ensure!(!name.contains('\\'), "迁移包包含不安全路径：{name}");
        let (id, rest) = inner.split_once('/').unwrap_or((inner, ""));
        if !ids.contains(id) {
            continue;
        }
        anyhow::ensure!(!file.is_symlink(), "Skill 中不允许包含符号链接：{name}");
        let rest = rest.trim_end_matches('/');
        let output = if rest.is_empty() {
            staging.join(id)
        } else {
            anyhow::ensure!(
                rest.split('/').all(|part| !part.is_empty() && part != "." && part != ".."),
                "迁移包包含不安全路径：{name}"
            );
            #[cfg(windows)]
            for part in rest.split('/') {
                validate_windows_name(part)?;
            }
            staging.join(id).join(safe_relative_path(rest)?)
        };
        if file.is_dir() {
            std::fs::create_dir_all(&output)
                .with_context(|| format!("创建目录失败：{}", output.display()))?;
            continue;
        }
        anyhow::ensure!(!rest.is_empty(), "迁移包包含不安全路径：{name}");
        let declared = file.size();
        let contents = read_bounded_zip_entry(&mut file, declared, &mut total_size, REPO_ZIP_UNPACKED_LIMIT_BYTES)
            .with_context(|| format!("读取迁移包条目 {name} 失败"))?;
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败：{}", parent.display()))?;
        }
        std::fs::write(&output, &contents)
            .with_context(|| format!("写入 {} 失败", output.display()))?;
        #[cfg(unix)]
        if let Some(mode) = file.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&output, std::fs::Permissions::from_mode(mode & 0o777))
                .with_context(|| format!("设置文件权限失败：{}", output.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager(root: &Path) -> SkillsManager {
        SkillsManager::new(
            root.join("skills"),
            root.join("skill-backups"),
            root.join("skills.json"),
            root.join("codex-home"),
        )
        .with_agent_dirs(vec![
            ("claude".into(), root.join("claude-skills")),
            ("grok".into(), root.join("grok-skills")),
        ])
    }

    fn write_skill(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("SKILL.md"), format!("---\nname: {name}\n---\nbody of {name}")).unwrap();
        std::fs::write(dir.join("scripts").join("run.sh"), "#!/bin/sh\n").unwrap();
    }

    fn selection(id: &str, group: &str) -> SkillBundleSelection {
        SkillBundleSelection { id: id.into(), group: group.into() }
    }

    fn entry<'a>(entries: &'a [SkillEntry], id: &str) -> &'a SkillEntry {
        entries.iter().find(|item| item.id == id && item.installed).unwrap()
    }

    fn bundle_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for &(name, contents) in entries {
            writer.start_file(name, options).unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn manifest_json(ids: &[&str]) -> Vec<u8> {
        serde_json::to_vec(&SkillBundleManifest {
            format: SKILL_BUNDLE_FORMAT.into(),
            version: SKILL_BUNDLE_VERSION,
            skills: ids.iter().map(|id| SkillBundleSkill { id: id.to_string(), ..Default::default() }).collect(),
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn bundle_round_trip_restores_files_groups_and_agent_state() {
        let old = tempfile::tempdir().unwrap();
        let source = manager(old.path());
        let external = old.path().join("external");
        write_skill(&external.join("alpha"), "Alpha");
        write_skill(&external.join("beta"), "Beta");
        source.import_from_local_directory("alpha", &external.join("alpha")).unwrap();
        source.import_from_local_directory("beta", &external.join("beta")).unwrap();
        source.set_enabled_for("codex", "alpha", true).unwrap();
        source.set_enabled_for("grok", "alpha", true).unwrap();
        source.set_enabled_for("claude", "beta", true).unwrap();

        let package = old.path().join("skills.zip");
        let exported = source
            .export_skill_bundle(
                &[selection("alpha", "custom:Workflow"), selection("beta", ""), selection("missing", "")],
                &["custom:Unused".into(), "custom:Workflow".into()],
                &package,
            )
            .unwrap();
        assert_eq!(exported.exported, vec!["alpha", "beta"]);
        assert_eq!(exported.errors.len(), 1, "没安装的跳过并记下原因");
        assert!(exported.errors[0].starts_with("missing："));
        // 没留下临时文件。
        let leftovers: Vec<_> = std::fs::read_dir(old.path()).unwrap().flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".codex-plus-skill-bundle-"))
            .collect();
        assert!(leftovers.is_empty());

        let new = tempfile::tempdir().unwrap();
        let target = manager(new.path());
        let imported = target.import_skill_bundle(&package).unwrap();
        assert_eq!(imported.imported, vec!["alpha", "beta"]);
        assert!(imported.existing.is_empty());
        assert!(imported.errors.is_empty(), "{:?}", imported.errors);
        assert_eq!(imported.groups, vec!["custom:Workflow"], "只带用到的分组");
        assert_eq!(imported.skills[0].group, "custom:Workflow");
        assert_eq!(imported.skills[0].agents, vec!["codex", "grok"]);
        assert_eq!(imported.skills[1].agents, vec!["claude"]);

        let inventory = target.try_local_inventory().unwrap();
        let alpha = entry(&inventory, "alpha");
        assert_eq!(alpha.name, "Alpha");
        assert_eq!(alpha.repo_key, LOCAL_IMPORT_REPO_KEY);
        assert!(alpha.agents.contains(&"codex".to_string()) && alpha.agents.contains(&"grok".to_string()));
        assert!(!alpha.agents.contains(&"claude".to_string()));
        assert_eq!(entry(&inventory, "beta").agents, vec!["claude"]);
        assert_eq!(
            std::fs::read_to_string(target.source_dir().join("alpha").join("scripts").join("run.sh")).unwrap(),
            "#!/bin/sh\n"
        );
        assert!(!target.source_dir().join("alpha").join(MANAGED_COPY_MARKER).exists());
    }

    #[test]
    fn importing_keeps_existing_skill_files_and_only_restores_state() {
        let old = tempfile::tempdir().unwrap();
        let source = manager(old.path());
        write_skill(&old.path().join("external").join("alpha"), "Packaged");
        source.import_from_local_directory("alpha", &old.path().join("external").join("alpha")).unwrap();
        source.set_enabled_for("claude", "alpha", true).unwrap();
        let package = old.path().join("skills.zip");
        source.export_skill_bundle(&[selection("alpha", "custom:Kept")], &["custom:Kept".into()], &package).unwrap();

        let new = tempfile::tempdir().unwrap();
        let target = manager(new.path());
        write_skill(&new.path().join("external").join("alpha"), "Local");
        target.import_from_local_directory("alpha", &new.path().join("external").join("alpha")).unwrap();

        let imported = target.import_skill_bundle(&package).unwrap();
        assert!(imported.imported.is_empty());
        assert_eq!(imported.existing, vec!["alpha"]);
        assert!(imported.errors.is_empty(), "{:?}", imported.errors);
        assert_eq!(imported.skills[0].group, "custom:Kept");
        let manifest = std::fs::read_to_string(target.source_dir().join("alpha").join("SKILL.md")).unwrap();
        assert!(manifest.contains("name: Local"), "本机已有的文件不被迁移包覆盖");
        let inventory = target.try_local_inventory().unwrap();
        assert!(entry(&inventory, "alpha").agents.contains(&"claude".to_string()));
    }

    /// #2852：密钥文件、版本库、依赖和缓存目录、日志不进迁移包。
    #[test]
    fn export_leaves_out_secrets_caches_and_logs() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(temp.path());
        let external = temp.path().join("external").join("alpha");
        write_skill(&external, "Alpha");
        manager.import_from_local_directory("alpha", &external).unwrap();
        let installed = manager.source_dir().join("alpha");
        for (path, contents) in [
            (".env", "API_KEY=sk-live-secret"),
            (".env.local", "TOKEN=secret"),
            (".env.example", "API_KEY="),
            (".DS_Store", ""),
            ("debug.log", "log"),
            (".git/config", "[core]"),
            ("node_modules/pkg/index.js", "module.exports = 1"),
            ("scripts/__pycache__/run.cpython-312.pyc", "pyc"),
            ("scripts/.env", "NESTED=secret"),
            ("scripts/helper.py", "print(1)"),
        ] {
            let path = installed.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        // 依赖目录里常有符号链接（node_modules/.bin）：整个目录不打包，也就不该因此导出失败。
        #[cfg(unix)]
        {
            std::fs::create_dir_all(installed.join("node_modules").join(".bin")).unwrap();
            std::os::unix::fs::symlink("../pkg/index.js", installed.join("node_modules").join(".bin").join("pkg")).unwrap();
        }

        let package = temp.path().join("skills.zip");
        manager.export_skill_bundle(&[selection("alpha", "")], &[], &package).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&package).unwrap()).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_string())
            .filter(|name| name.starts_with("skills/alpha/") && !name.ends_with('/'))
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "skills/alpha/.env.example",
                "skills/alpha/SKILL.md",
                "skills/alpha/scripts/helper.py",
                "skills/alpha/scripts/run.sh",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn export_skips_skills_with_symlinks_and_fails_when_nothing_is_left() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(temp.path());
        write_skill(&temp.path().join("external").join("alpha"), "Alpha");
        manager.import_from_local_directory("alpha", &temp.path().join("external").join("alpha")).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", manager.source_dir().join("alpha").join("link")).unwrap();
        let package = temp.path().join("skills.zip");
        let error = manager.export_skill_bundle(&[selection("alpha", "")], &[], &package).unwrap_err();
        assert!(format!("{error:#}").contains("符号链接"), "{error:#}");
        assert!(!package.exists(), "失败时不留下半个包");
        assert!(manager.export_skill_bundle(&[], &[], &package).is_err());
    }

    #[test]
    fn import_rejects_foreign_archives_unsafe_paths_and_newer_versions() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(temp.path());
        let write = |name: &str, bytes: Vec<u8>| {
            let path = temp.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };

        let not_zip = write("plain.zip", b"not a zip".to_vec());
        assert!(manager.import_skill_bundle(&not_zip).is_err());

        let foreign = write("foreign.zip", bundle_zip(&[("skills/alpha/SKILL.md", b"alpha")]));
        let error = manager.import_skill_bundle(&foreign).unwrap_err();
        assert!(format!("{error:#}").contains("缺少清单"), "{error:#}");

        let newer = serde_json::json!({ "format": SKILL_BUNDLE_FORMAT, "version": SKILL_BUNDLE_VERSION + 1, "skills": [{ "id": "alpha" }] });
        let newer = write("newer.zip", bundle_zip(&[(SKILL_BUNDLE_MANIFEST_FILE, newer.to_string().as_bytes())]));
        assert!(format!("{:#}", manager.import_skill_bundle(&newer).unwrap_err()).contains("升级"));

        for id in ["../escape", ".hidden", "a/b"] {
            let bad_id = write("bad-id.zip", bundle_zip(&[(SKILL_BUNDLE_MANIFEST_FILE, &manifest_json(&[id]))]));
            assert!(manager.import_skill_bundle(&bad_id).is_err(), "{id}");
        }

        let escape = write("escape.zip", bundle_zip(&[
            (SKILL_BUNDLE_MANIFEST_FILE, &manifest_json(&["alpha"])),
            ("skills/alpha/SKILL.md", b"alpha"),
            ("skills/alpha/../../escape.txt", b"escape"),
        ]));
        let error = manager.import_skill_bundle(&escape).unwrap_err();
        assert!(format!("{error:#}").contains("不安全路径"), "{error:#}");
        assert!(manager.try_local_inventory().unwrap().iter().all(|item| item.id != "alpha"), "整包拒绝，不装任何 Skill");

        // 清单之外的目录不解；清单里有、包里没文件的记为失败，其余照常导入。
        let partial = write("partial.zip", bundle_zip(&[
            (SKILL_BUNDLE_MANIFEST_FILE, &manifest_json(&["alpha", "ghost"])),
            ("skills/alpha/SKILL.md", b"---\nname: Alpha\n---\n"),
            ("skills/unlisted/SKILL.md", b"unlisted"),
        ]));
        let imported = manager.import_skill_bundle(&partial).unwrap();
        assert_eq!(imported.imported, vec!["alpha"]);
        assert_eq!(imported.errors.len(), 1);
        assert!(imported.errors[0].starts_with("ghost："));
        assert!(!manager.source_dir().join("unlisted").exists());
    }
}
