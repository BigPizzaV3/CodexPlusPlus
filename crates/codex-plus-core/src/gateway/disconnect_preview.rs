//! 断开前的只读预览。通用 Agent 直接计算还原结果，原生 Agent 只在私有副本里预演。
//! 返回给界面的 diff 是配置原文的逐行差异（含配置值与注释），由用户确认后才执行。

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::agents::{self, files::AgentStash};
use super::apply::{self, ApplyContext};
use super::restore;
use super::store::{AgentBinding, GatewaySettings, GatewayStore};
use crate::settings::{BackendSettings, SettingsStore};
use crate::tools::ToolId;
use crate::tools::claude::ClaudeContext;

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
/// 参与逐行 diff 的单侧最大行数；超过则不做 LCS，按整体删除 + 新增展示。
const MAX_DIFF_SOURCE_LINES: usize = 800;
/// diff 输出的最大行数；超出截断。
const MAX_DIFF_OUTPUT_LINES: usize = 800;
/// 每处变更前后保留的上下文行数。
const DIFF_CONTEXT_LINES: usize = 2;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisconnectPreview {
    pub agent: String,
    pub preview_token: String,
    pub files: Vec<DisconnectFile>,
    pub warnings: Vec<String>,
    pub can_disconnect: bool,
    /// Codex 没法逐项预览（接入前没有实时存档，或要切回的是聚合供应商）时：断开等于把 Codex
    /// 完整切换到这个直连供应商（和在供应商列表里切换一样，切换前备份当前配置、收回借出的登录）。
    /// 这时 `can_disconnect` 仍为 false（普通断开不放行），由用户确认后走「完整切换并断开」。
    pub full_switch: Option<FullSwitchTarget>,
    #[serde(skip)]
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FullSwitchTarget {
    pub relay_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisconnectFile {
    pub path: String,
    pub kind: DisconnectFileKind,
    pub diff: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DisconnectFileKind {
    Modify,
    Remove,
    Create,
}

#[derive(Default)]
struct Sources {
    files: BTreeMap<PathBuf, Option<Vec<u8>>>,
    locations: BTreeMap<PathBuf, String>,
}

impl Sources {
    fn read(&mut self, path: &Path) -> anyhow::Result<Option<String>> {
        if !self.files.contains_key(path) {
            let bytes = read_source(path)?;
            self.locations
                .insert(path.to_path_buf(), source_location(path)?);
            self.files.insert(path.to_path_buf(), bytes);
        }
        self.files[path]
            .as_deref()
            .map(|bytes| {
                std::str::from_utf8(bytes)
                    .map(str::to_string)
                    .map_err(|_| anyhow::anyhow!("配置不是 UTF-8 文本，无法安全预览"))
            })
            .transpose()
    }

    fn fingerprint(
        &self,
        ctx: &ApplyContext,
        agent: &ToolId,
        settings: &BackendSettings,
        settings_bytes: Option<&[u8]>,
    ) -> anyhow::Result<String> {
        let mut hash = Sha256::new();
        hash_part(&mut hash, agent.as_str().as_bytes());
        hash_part(&mut hash, &ctx.port.to_le_bytes());
        // SettingsStore 的 Debug 仅包含私有 path 字段；路径只参与指纹，不返回界面。
        hash_part(&mut hash, format!("{:?}", ctx.settings).as_bytes());
        hash_part(&mut hash, &serde_json::to_vec(settings)?);
        hash_optional(&mut hash, settings_bytes);
        for (path, bytes) in &self.files {
            hash_part(&mut hash, path.to_string_lossy().as_bytes());
            hash_part(&mut hash, self.locations[path].as_bytes());
            hash_optional(&mut hash, bytes.as_deref());
        }
        Ok(format!("{:x}", hash.finalize()))
    }

    fn unchanged(&self) -> anyhow::Result<bool> {
        for (path, before) in &self.files {
            if read_source(path)? != *before || source_location(path)? != self.locations[path] {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn source_location(path: &Path) -> anyhow::Result<String> {
    let mut hash = Sha256::new();
    for ancestor in std::path::absolute(path)?.ancestors() {
        hash_part(&mut hash, ancestor.to_string_lossy().as_bytes());
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                let kind = metadata.file_type();
                hash.update([1, u8::from(kind.is_dir()), u8::from(kind.is_symlink())]);
                if kind.is_symlink() {
                    hash_part(
                        &mut hash,
                        fs::read_link(ancestor)?.to_string_lossy().as_bytes(),
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => hash.update([0]),
            Err(_) => anyhow::bail!("无法核验配置文件路径，未生成断开预览"),
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn read_source(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => anyhow::bail!("无法读取待还原文件，未生成断开预览"),
    };
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_SOURCE_BYTES {
        anyhow::bail!("待还原文件不是普通文件或超过 16 MiB，无法安全预览");
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        anyhow::bail!("待还原文件超过 16 MiB，无法安全预览");
    }
    Ok(Some(bytes))
}

fn hash_part(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn hash_optional(hash: &mut Sha256, bytes: Option<&[u8]>) {
    hash.update([u8::from(bytes.is_some())]);
    if let Some(bytes) = bytes {
        hash_part(hash, bytes);
    }
}

pub fn preview(
    ctx: &ApplyContext,
    settings: &BackendSettings,
    agent: &ToolId,
) -> anyhow::Result<DisconnectPreview> {
    let mut result = DisconnectPreview {
        agent: agent.as_str().to_string(),
        preview_token: String::new(),
        files: Vec::new(),
        warnings: vec![
            "预览展示配置文件原文差异，密钥等敏感值也一并可见。".into(),
            "断开还会更新 Codex++ 的绑定、供应商记录与恢复存档，这些内部状态不在配置 diff 中。"
                .into(),
        ],
        can_disconnect: true,
        full_switch: None,
        fingerprint: String::new(),
    };
    let mut sources = Sources::default();
    let settings_bytes = ctx
        .settings
        .snapshot_raw_bytes()
        .map_err(|_| anyhow::anyhow!("无法安全读取供应商设置，未生成断开预览"))?;
    sources.read(&ctx.gateway.path)?;
    let gateway = ctx.gateway.load().ok();
    let binding = gateway
        .as_ref()
        .and_then(|gateway| gateway.binding(agent))
        .cloned();
    if gateway.is_some() && binding.is_none() && !apply::agent_points_at_gateway(ctx, agent) {
        collect_agent_sources(ctx, agent, &mut sources)?;
    } else if let Some(spec) = agents::spec(agent.as_str()) {
        generic_preview(ctx, spec, &mut sources, &mut result)?;
    } else if matches!(agent, ToolId::Claude | ToolId::Grok | ToolId::Codex) {
        native_preview(
            ctx,
            settings,
            agent,
            binding.as_ref().unwrap_or(&AgentBinding::default()),
            gateway.as_ref(),
            settings_bytes.as_deref(),
            &mut sources,
            &mut result,
        )?;
    } else {
        result.can_disconnect = false;
        result
            .warnings
            .push("该 Agent 没有可用的断开适配器。".into());
    }
    if !sources.unchanged()?
        || settings_bytes
            != ctx
                .settings
                .snapshot_raw_bytes()
                .map_err(|_| anyhow::anyhow!("预览期间供应商设置发生变化，请重新预览"))?
    {
        anyhow::bail!("预览期间配置发生变化，请重新预览");
    }
    result.fingerprint = sources.fingerprint(ctx, agent, settings, settings_bytes.as_deref())?;
    Ok(result)
}

fn collect_agent_sources(
    ctx: &ApplyContext,
    agent: &ToolId,
    sources: &mut Sources,
) -> anyhow::Result<()> {
    match agent {
        ToolId::Claude => {
            sources.read(&ctx.claude.store_path)?;
            sources.read(&ctx.claude.claude_home.join("settings.json"))?;
            sources.read(&ctx.backup_root.join("claude-before-gateway.json"))?;
        }
        ToolId::Grok => {
            sources.read(&ctx.grok_home.join("config.toml"))?;
            sources.read(&ctx.backup_root.join("grok-before-gateway.json"))?;
        }
        ToolId::Codex => {
            sources.read(&ctx.codex_home.join("config.toml"))?;
            sources.read(&ctx.codex_home.join("auth.json"))?;
            sources.read(&ctx.backup_root.join("codex-before-gateway.json"))?;
        }
        other => {
            if let Some(spec) = agents::spec(other.as_str()) {
                let path = agents::connect::stash_path(ctx, spec.id);
                let saved = sources.read(&path)?;
                sources.read(&(spec.config_path)(&ctx.agent_env))?;
                if let Some(saved) = saved
                    .as_deref()
                    .and_then(|text| serde_json::from_str::<AgentStash>(text).ok())
                {
                    for path in saved.files.keys() {
                        sources.read(path)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn generic_preview(
    ctx: &ApplyContext,
    spec: &agents::AgentSpec,
    sources: &mut Sources,
    result: &mut DisconnectPreview,
) -> anyhow::Result<()> {
    collect_agent_sources(ctx, &ToolId::Unknown(spec.id.into()), sources)?;
    let stash_path = agents::connect::stash_path(ctx, spec.id);
    let saved = sources.read(&stash_path)?;
    let Some(saved) = saved
        .as_deref()
        .and_then(|text| serde_json::from_str::<AgentStash>(text).ok())
    else {
        if agents::connect::points_at_gateway(ctx, spec) {
            result.can_disconnect = false;
            result
                .warnings
                .push("配置仍指向网关，但恢复存档缺失或损坏；未提供不完整的断开预览。".into());
        }
        return Ok(());
    };
    if !agents::connect::points_at_gateway(ctx, spec) {
        result
            .warnings
            .push("配置已由其他工具改走；断开只清理网关记录，不覆盖当前配置。".into());
        return Ok(());
    }
    for (path, record) in &saved.files {
        let before = sources.read(path)?;
        let after = match agents::files::restored_text(record, before.as_deref()) {
            Ok(after) => after,
            Err(_) => {
                result.can_disconnect = false;
                result.files.clear();
                result
                    .warnings
                    .push("恢复存档无法与当前配置安全合并；请先检查配置语法后重新预览。".into());
                return Ok(());
            }
        };
        add_file(result, path, before.as_deref(), after.as_deref());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn native_preview(
    ctx: &ApplyContext,
    settings: &BackendSettings,
    agent: &ToolId,
    binding: &AgentBinding,
    gateway: Option<&GatewaySettings>,
    settings_bytes: Option<&[u8]>,
    sources: &mut Sources,
    result: &mut DisconnectPreview,
) -> anyhow::Result<()> {
    collect_agent_sources(ctx, agent, sources)?;
    if gateway.is_none() {
        result.can_disconnect = false;
        result
            .warnings
            .push("网关状态无法读取；无法在隔离环境中精确预演此次还原。".into());
        return Ok(());
    }
    if *agent != ToolId::Codex && super::drift::check(ctx, agent).is_some() {
        let recorded = match agent {
            ToolId::Claude => apply::agent_points_at_gateway(ctx, agent),
            ToolId::Grok => ctx.backup_root.join("grok-before-gateway.json").exists(),
            _ => false,
        };
        if recorded {
            result
                .warnings
                .push("配置已由其他工具改走；断开只清理网关记录，不覆盖当前配置。".into());
            return Ok(());
        }
    }
    if *agent == ToolId::Grok
        && apply::agent_points_at_gateway(ctx, agent)
        && sources
            .files
            .get(&ctx.backup_root.join("grok-before-gateway.json"))
            .and_then(Option::as_deref)
            .and_then(|bytes| serde_json::from_slice::<restore::FileStash>(bytes).ok())
            .is_none()
        && settings
            .tools
            .get(&ToolId::Grok)
            .and_then(crate::tools::grok::active_grok_profile)
            .is_none_or(|profile| {
                crate::tools::grok::parse_grok_model_list(&profile.model_list).is_empty()
            })
    {
        result.can_disconnect = false;
        result.warnings.push(
            "Grok 仍连接网关，但没有可用的恢复存档，当前供应商也没有模型；请在 Grok 供应商页添加并应用一个直连供应商后再断开。".into(),
        );
        return Ok(());
    }
    let sandbox = Sandbox::new()?;
    let copied = sandbox.context(ctx)?;
    for (source, target) in [
        (&ctx.gateway.path, copied.gateway.path.clone()),
        (&ctx.claude.store_path, copied.claude.store_path.clone()),
    ] {
        if let Some(bytes) = sources.files.get(source).and_then(Option::as_deref) {
            sandbox.write(&target, bytes)?;
        }
    }
    if let Some(bytes) = settings_bytes {
        sandbox.write(&sandbox.root.join("settings.json"), bytes)?;
    }
    let live_files = match agent {
        ToolId::Claude => vec![(
            ctx.claude.claude_home.join("settings.json"),
            copied.claude.claude_home.join("settings.json"),
        )],
        ToolId::Grok => vec![(
            ctx.grok_home.join("config.toml"),
            copied.grok_home.join("config.toml"),
        )],
        ToolId::Codex => vec![
            (
                ctx.codex_home.join("config.toml"),
                copied.codex_home.join("config.toml"),
            ),
            (
                ctx.codex_home.join("auth.json"),
                copied.codex_home.join("auth.json"),
            ),
        ],
        _ => Vec::new(),
    };
    for (source, target) in &live_files {
        if let Some(bytes) = sources.files.get(source).and_then(Option::as_deref) {
            sandbox.write(target, bytes)?;
        }
    }
    let stash_name = format!("{}-before-gateway.json", agent.as_str());
    if let Some(bytes) = sources
        .files
        .get(&ctx.backup_root.join(&stash_name))
        .and_then(Option::as_deref)
    {
        sandbox.write(&copied.backup_root.join(&stash_name), bytes)?;
    }
    let projected = if *agent == ToolId::Codex {
        apply::return_lent_login(&copied.gateway, &copied.codex_home)
            .and_then(|_| apply::preview_codex_stashed_restore(&copied, binding))
    } else {
        apply::disable_binding(&copied, settings, agent)
            .map(|_| {
                Some(
                    live_files
                        .iter()
                        .map(|(_, path)| {
                            restore::read_optional(path).map(|text| (path.clone(), text))
                        })
                        .collect::<anyhow::Result<Vec<_>>>(),
                )
            })
            .and_then(|files| files.transpose())
    };
    let projected = match projected {
        Ok(Some(files)) => files,
        projected => {
            result.can_disconnect = false;
            // Codex 的存档投影给不出结果（没存档、要切回聚合供应商、存档损坏）时，真实断开
            // 走的是完整切换到直连供应商：在隔离副本里跑同一套切换，把结果文件读回来做 diff。
            let full_switch = (*agent == ToolId::Codex)
                .then(|| codex_full_switch_target(settings, binding))
                .flatten();
            match full_switch {
                Some(target) => {
                    result.full_switch = Some(target);
                    match apply::preview_codex_disconnect_apply(&copied, binding) {
                        Ok(files) => {
                            result.warnings.push(
                                "接入前没有留下实时存档（或要切回的是聚合供应商）：以下是在隔离环境里按「完整切换到直连供应商」预演出的改动，断开前会备份当前配置。"
                                    .into(),
                            );
                            for (copied_path, after) in files {
                                // 预演在隔离副本里生成的新绝对路径（如 model_catalog_json
                                // 指针）换回真实 home，再与实时文件对比。
                                let after = after.map(|text| {
                                    text.replace(
                                        copied.codex_home.to_string_lossy().as_ref(),
                                        ctx.codex_home.to_string_lossy().as_ref(),
                                    )
                                });
                                if let Some((path, _)) =
                                    live_files.iter().find(|(_, dest)| *dest == copied_path)
                                {
                                    let before = sources.read(path)?;
                                    add_file(
                                        result,
                                        path,
                                        before.as_deref(),
                                        after.as_deref(),
                                    );
                                }
                            }
                        }
                        Err(_) => result.warnings.push(
                            "接入前没有留下实时存档（或要切回的是聚合供应商），也没能在隔离环境里预演切换；断开会把 Codex 完整切换到下面这个直连供应商，切换前会备份当前配置。"
                                .into(),
                        ),
                    }
                }
                None if *agent == ToolId::Codex && projected.is_ok() => result.warnings.push(
                    "Codex 的直连模式里没有别的供应商可切回：先在「连接与供应商」里添加一个直连供应商。"
                        .into(),
                ),
                None => result.warnings.push(
                    "隔离预演未能完成还原；原配置未被修改，请检查可切回的供应商和恢复存档。"
                        .into(),
                ),
            }
            return Ok(());
        }
    };
    for (copied_path, after) in projected {
        if let Some((path, _)) = live_files.iter().find(|(_, target)| *target == copied_path) {
            let before = sources.read(path)?;
            add_file(result, path, before.as_deref(), after.as_deref());
        }
    }
    if *agent == ToolId::Codex {
        result.warnings.push("Codex 配置按实时恢复存档计算；正式断开仍需成功切换直连供应商，并更新模型目录、应用状态和备份。供应商校验或写入失败时会中止断开。".into());
    }
    Ok(())
}

/// 完整切换时切到哪个直连供应商：与真正断开（`restore_codex`）同一个选法——接入前那一个还在就用它，
/// 不在了就用列表里第一个普通供应商。
fn codex_full_switch_target(settings: &BackendSettings, binding: &AgentBinding) -> Option<FullSwitchTarget> {
    let managed = apply::CODEX_GATEWAY_RELAY_ID;
    let previous = binding.restore.codex_previous_relay_id.trim();
    let named = |id: &str| -> Option<String> {
        if id.is_empty() || id == managed {
            return None;
        }
        settings
            .aggregate_relay_profiles
            .iter()
            .find(|profile| profile.id == id)
            .map(|profile| profile.name.clone())
            .or_else(|| settings.relay_profiles.iter().find(|profile| profile.id == id).map(|profile| profile.name.clone()))
    };
    if let Some(name) = named(previous) {
        return Some(FullSwitchTarget { relay_id: previous.to_string(), name });
    }
    settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id != managed)
        .map(|profile| FullSwitchTarget { relay_id: profile.id.clone(), name: profile.name.clone() })
}

fn add_file(
    result: &mut DisconnectPreview,
    path: &Path,
    before: Option<&str>,
    after: Option<&str>,
) {
    if before == after {
        return;
    }
    result.files.push(DisconnectFile {
        path: path.to_string_lossy().to_string(),
        kind: match (before, after) {
            (None, Some(_)) => DisconnectFileKind::Create,
            (Some(_), None) => DisconnectFileKind::Remove,
            _ => DisconnectFileKind::Modify,
        },
        diff: text_diff(before, after),
    });
}

/// 原文逐行 diff：有差异的行按原文展示（含配置值），每处变更保留前后
/// `DIFF_CONTEXT_LINES` 行上下文，不相邻的变更块之间用 `@@` 分隔。
fn text_diff(before: Option<&str>, after: Option<&str>) -> String {
    let mut diff = format!(
        "--- {}\n+++ {}\n",
        if before.is_some() {
            "current"
        } else {
            "/dev/null"
        },
        if after.is_some() {
            "restored"
        } else {
            "/dev/null"
        },
    );
    let old: Vec<&str> = before.map(|text| text.lines().collect()).unwrap_or_default();
    let new: Vec<&str> = after.map(|text| text.lines().collect()).unwrap_or_default();
    let ops = diff_ops(&old, &new);
    if ops.iter().all(|op| matches!(op, DiffOp::Keep(_))) {
        // 文本不同但行序列相同：只差在末尾换行。
        diff.push_str("(仅末尾换行不同)\n");
        return diff;
    }
    diff.push_str(&render_diff(&ops));
    diff
}

enum DiffOp<'a> {
    Keep(&'a str),
    Remove(&'a str),
    Add(&'a str),
}

/// LCS 逐行对齐；任一侧超 `MAX_DIFF_SOURCE_LINES` 时退化为整体替换，避免平方级内存。
fn diff_ops<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<DiffOp<'a>> {
    if old.len() > MAX_DIFF_SOURCE_LINES || new.len() > MAX_DIFF_SOURCE_LINES {
        return old
            .iter()
            .map(|line| DiffOp::Remove(line))
            .chain(new.iter().map(|line| DiffOp::Add(line)))
            .collect();
    }
    let width = new.len() + 1;
    let mut table = vec![0u32; (old.len() + 1) * width];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[i * width + j] = if old[i] == new[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let mut ops = Vec::with_capacity(old.len() + new.len());
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            ops.push(DiffOp::Keep(old[i]));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            ops.push(DiffOp::Remove(old[i]));
            i += 1;
        } else {
            ops.push(DiffOp::Add(new[j]));
            j += 1;
        }
    }
    ops.extend(old[i..].iter().map(|line| DiffOp::Remove(line)));
    ops.extend(new[j..].iter().map(|line| DiffOp::Add(line)));
    ops
}

fn render_diff(ops: &[DiffOp]) -> String {
    // 标出要展示的行：每个变更行加上前后上下文行。
    let mut shown = vec![false; ops.len()];
    for (index, op) in ops.iter().enumerate() {
        if matches!(op, DiffOp::Keep(_)) {
            continue;
        }
        let start = index.saturating_sub(DIFF_CONTEXT_LINES);
        let end = (index + DIFF_CONTEXT_LINES).min(ops.len() - 1);
        for mark in &mut shown[start..=end] {
            *mark = true;
        }
    }
    let mut out = String::new();
    let mut emitted = 0usize;
    let mut hidden = false;
    for (index, op) in ops.iter().enumerate() {
        if !shown[index] {
            hidden = true;
            continue;
        }
        if hidden {
            out.push_str("@@\n");
            emitted += 1;
            hidden = false;
        }
        let (prefix, line) = match op {
            DiffOp::Keep(line) => (" ", *line),
            DiffOp::Remove(line) => ("-", *line),
            DiffOp::Add(line) => ("+", *line),
        };
        out.push_str(prefix);
        out.push_str(line);
        out.push('\n');
        emitted += 1;
        if emitted >= MAX_DIFF_OUTPUT_LINES {
            out.push_str("… 差异过长，已截断\n");
            break;
        }
    }
    out
}

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new() -> anyhow::Result<Self> {
        let root = std::env::temp_dir().join(format!(
            "codexplus-disconnect-preview-{}",
            uuid::Uuid::new_v4()
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&root).context("无法创建私有断开预演目录")?;
        Ok(Self { root })
    }

    fn context(&self, source: &ApplyContext) -> anyhow::Result<ApplyContext> {
        let path = |name| self.root.join(name);
        for name in ["claude", "codex", "grok", "backups", "home"] {
            fs::create_dir(path(name))?;
        }
        Ok(ApplyContext {
            gateway: GatewayStore::at(path("gateway.json")),
            claude: ClaudeContext {
                store_path: path("claude-providers.json"),
                claude_home: path("claude"),
                backup_root: path("backups"),
            },
            codex_home: path("codex"),
            grok_home: path("grok"),
            settings: SettingsStore::new(path("settings.json")),
            backup_root: path("backups"),
            port: source.port,
            agent_env: agents::AgentEnv::at(path("home")),
        })
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
        anyhow::ensure!(path.starts_with(&self.root), "预演路径不在隔离目录内");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(path)?.write_all(bytes)?;
        Ok(())
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // 只清理由本次预演创建的随机目录，逐个删除文件，不跟随符号链接。
        fn cleanup(path: &Path) {
            let Ok(entries) = fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() && !kind.is_symlink() => cleanup(&path),
                    Ok(_) => {
                        let _ = fs::remove_file(&path);
                    }
                    Err(_) => {}
                }
            }
            let _ = fs::remove_dir(path);
        }
        cleanup(&self.root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::agents::files::{AgentStash, FileSet};
    use crate::gateway::apply::{ProviderInput, create_provider, enable_binding};
    use crate::settings::{RelayMode, RelayProtocol};
    use serde_json::{Value, json};

    struct Fixture {
        temp: tempfile::TempDir,
        ctx: ApplyContext,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let ctx = ApplyContext {
            gateway: GatewayStore::at(state.join("gateway.json")),
            claude: ClaudeContext {
                store_path: state.join("claude-providers.json"),
                claude_home: temp.path().join("claude"),
                backup_root: state.join("backups"),
            },
            codex_home: temp.path().join("codex"),
            grok_home: temp.path().join("grok"),
            settings: SettingsStore::new(state.join("settings.json")),
            backup_root: state.join("backups"),
            port: 57491,
            agent_env: agents::AgentEnv::at(temp.path().join("home")),
        };
        for home in [&ctx.claude.claude_home, &ctx.codex_home, &ctx.grok_home] {
            fs::create_dir_all(home).unwrap();
        }
        create_provider(
            &ctx.gateway,
            &ProviderInput {
                name: "Test".into(),
                base_url: "https://api.example.test/v1".into(),
                protocol: RelayProtocol::Responses,
                keys: vec!["fake-provider-secret".into()],
                model: "test-model".into(),
                model_list: "test-model".into(),
                price: None,
                model_prices: None,
                proxy: None,
                strip_image_generation_tool: None,
                subscription_policy: None,
                strip_responses_image_detail: None,
                disable_tool_search: None,
            },
            "custom",
            None,
        )
        .unwrap();
        Fixture { temp, ctx }
    }

    fn binding() -> AgentBinding {
        AgentBinding {
            enabled: true,
            model: "test-model".into(),
            ..AgentBinding::default()
        }
    }

    fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(root).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    visit(&entry.path(), files);
                } else {
                    files.insert(entry.path(), fs::read(entry.path()).unwrap());
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(root, &mut files);
        files
    }

    fn generic(fixture: &Fixture, before: Option<&str>) -> PathBuf {
        let ctx = &fixture.ctx;
        let spec = agents::spec("opencode").unwrap();
        let path = (spec.config_path)(&ctx.agent_env);
        if let Some(before) = before {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, before).unwrap();
        }
        let mut files = FileSet::new();
        let root = files.json(&path).unwrap();
        root.insert("model".into(), json!("gateway"));
        root.insert(
            "provider".into(),
            json!({
                "codexplus": {
                    "baseURL": "http://127.0.0.1:57491/gw/opencode/v1",
                    "apiKey": "fake-client-secret"
                }
            }),
        );
        let mut saved = AgentStash::default();
        for change in files.changes().unwrap() {
            saved.record(&change).unwrap();
            agents::files::write_file(&change.path, change.after.as_deref()).unwrap();
        }
        restore::save_stash(&agents::connect::stash_path(ctx, spec.id), &saved).unwrap();
        ctx.gateway
            .update(|gateway| {
                gateway.bindings.insert("opencode".into(), binding());
                Ok(())
            })
            .unwrap();
        path
    }

    #[test]
    fn generic_preview_matches_restore_without_writing_and_shows_all_values() {
        let fixture = fixture();
        let before = "{\n // fake-comment-secret\n \"model\": \"mine\",\n \"apiKey\": \"fake-original-secret\",\n \"baseURL\": \"https://user:fake-url-secret@example.test/v1?token=fake-query-secret\",\n \"fake-field-secret\": \"fake-unknown-secret\"\n}\n";
        let path = generic(&fixture, Some(before));
        let snapshot_before = snapshot(fixture.temp.path());
        let result = preview(
            &fixture.ctx,
            &BackendSettings::default(),
            &ToolId::Unknown("opencode".into()),
        )
        .unwrap();
        assert!(result.can_disconnect);
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].kind, DisconnectFileKind::Modify);
        assert_eq!(snapshot(fixture.temp.path()), snapshot_before);
        let encoded = serde_json::to_string(&result).unwrap();
        // 文件原文差异：当前值与还原值（含注释）都展示。
        for shown in [
            "fake-comment-secret",
            "fake-original-secret",
            "fake-url-secret",
            "fake-query-secret",
            "fake-field-secret",
            "fake-unknown-secret",
            "fake-client-secret",
        ] {
            assert!(encoded.contains(shown), "diff 应展示 {shown}");
        }
        // 网关内部的供应商密钥不写入 Agent 配置，不属于文件 diff。
        assert!(!encoded.contains("fake-provider-secret"));
        assert!(!encoded.contains("fingerprint"));
        assert!(!encoded.contains(&result.fingerprint));
        let current = fs::read_to_string(&path).unwrap();
        apply::disable_binding(
            &fixture.ctx,
            &BackendSettings::default(),
            &ToolId::Unknown("opencode".into()),
        )
        .unwrap();
        let actual = fs::read_to_string(&path).unwrap();
        assert_eq!(actual, before);
        assert_eq!(
            result.files[0].diff,
            text_diff(Some(&current), Some(&actual))
        );
    }

    #[test]
    fn created_config_is_previewed_as_removal_and_fingerprint_tracks_unchanged_fields() {
        let fixture = fixture();
        let path = generic(&fixture, None);
        let agent = ToolId::Unknown("opencode".into());
        let first = preview(&fixture.ctx, &BackendSettings::default(), &agent).unwrap();
        assert_eq!(first.files[0].kind, DisconnectFileKind::Remove);
        let mut current = fs::read_to_string(&path).unwrap();
        current = current.replace('{', "{ /* fake-new-comment */");
        fs::write(&path, current).unwrap();
        let second = preview(&fixture.ctx, &BackendSettings::default(), &agent).unwrap();
        assert_ne!(first.fingerprint, second.fingerprint);
        assert_eq!(second.files[0].kind, DisconnectFileKind::Modify);
        assert!(second.files[0].diff.contains("fake-new-comment"));
    }

    #[test]
    fn generic_drift_preserves_the_new_configuration() {
        let fixture = fixture();
        let path = generic(&fixture, Some("{\"model\":\"mine\"}"));
        fs::write(&path, "{\"baseURL\":\"https://another.test/v1\"}").unwrap();
        let before = snapshot(fixture.temp.path());
        let result = preview(
            &fixture.ctx,
            &BackendSettings::default(),
            &ToolId::Unknown("opencode".into()),
        )
        .unwrap();
        assert!(result.can_disconnect);
        assert!(result.files.is_empty());
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("其他工具"))
        );
        assert_eq!(snapshot(fixture.temp.path()), before);
    }

    #[test]
    fn malformed_config_cannot_produce_a_partial_or_secret_bearing_preview() {
        let fixture = fixture();
        let path = generic(&fixture, None);
        fs::write(&path, "not-json fake-parse-secret /gw/opencode").unwrap();
        let result = preview(
            &fixture.ctx,
            &BackendSettings::default(),
            &ToolId::Unknown("opencode".into()),
        )
        .unwrap();
        assert!(!result.can_disconnect);
        assert!(result.files.is_empty());
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("fake-parse-secret")
        );
    }

    #[test]
    fn claude_and_grok_preview_match_real_disconnect_without_touching_live_files() {
        for agent in [ToolId::Claude, ToolId::Grok] {
            let fixture = fixture();
            let path = match agent {
                ToolId::Claude => {
                    let path = fixture.ctx.claude.claude_home.join("settings.json");
                    fs::write(
                        &path,
                        "{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://direct.test\",\"ANTHROPIC_AUTH_TOKEN\":\"fake-native-secret\"},\"permissions\":{\"deny\":[\"Bash(rm)\"]}}",
                    )
                    .unwrap();
                    path
                }
                ToolId::Grok => {
                    let path = fixture.ctx.grok_home.join("config.toml");
                    fs::write(
                        &path,
                        "[ui]\nyolo = false\n[models]\ndefault = \"own\"\n[model.own]\nmodel = \"own\"\nbase_url = \"https://direct.test/v1\"\napi_key = \"fake-native-secret\"\napi_backend = \"responses\"\n",
                    )
                    .unwrap();
                    path
                }
                _ => unreachable!(),
            };
            enable_binding(&fixture.ctx, &agent, binding()).unwrap();
            let current = fs::read_to_string(&path).unwrap();
            let before = snapshot(fixture.temp.path());
            let result = preview(&fixture.ctx, &BackendSettings::default(), &agent).unwrap();
            assert!(result.can_disconnect, "{:?}", result.warnings);
            assert_eq!(snapshot(fixture.temp.path()), before);
            assert!(
                serde_json::to_string(&result)
                    .unwrap()
                    .contains("fake-native-secret")
            );
            apply::disable_binding(&fixture.ctx, &BackendSettings::default(), &agent).unwrap();
            let actual = restore::read_optional(&path).unwrap();
            assert_eq!(result.files.len(), 1);
            assert_eq!(
                result.files[0].diff,
                text_diff(Some(&current), actual.as_deref())
            );
        }
    }

    #[test]
    fn grok_without_stash_or_direct_models_explains_the_missing_target_without_writing() {
        for saved in [None, Some("not-json fake-stash-secret")] {
            let fixture = fixture();
            let path = fixture.ctx.grok_home.join("config.toml");
            fs::write(
                &path,
                "[model.gateway]\nbase_url = \"http://127.0.0.1:57491/gw/grok/v1\"\napi_key = \"fake-live-secret\"\n",
            )
            .unwrap();
            fixture
                .ctx
                .gateway
                .update(|gateway| {
                    gateway.bindings.insert("grok".into(), binding());
                    Ok(())
                })
                .unwrap();
            if let Some(saved) = saved {
                fs::create_dir_all(&fixture.ctx.backup_root).unwrap();
                fs::write(
                    fixture.ctx.backup_root.join("grok-before-gateway.json"),
                    saved,
                )
                .unwrap();
            }
            let mut settings = BackendSettings::default();
            settings.tools.insert(
                ToolId::Grok,
                crate::tools::ToolConfig {
                    relay_profiles: vec![crate::settings::RelayProfile {
                        id: "empty-models".into(),
                        model_list: String::new(),
                        api_key: "fake-profile-secret".into(),
                        ..crate::settings::RelayProfile::default()
                    }],
                    active_relay_id: "empty-models".into(),
                    ..crate::tools::ToolConfig::default()
                },
            );
            fixture.ctx.settings.save(&settings).unwrap();
            let before = snapshot(fixture.temp.path());
            let result = preview(&fixture.ctx, &settings, &ToolId::Grok).unwrap();
            assert!(!result.can_disconnect);
            assert!(result.files.is_empty());
            assert!(result.warnings.iter().any(|warning| {
                warning.contains("Grok 供应商页添加并应用一个直连供应商")
            }));
            assert!(
                !result
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("隔离预演未能完成"))
            );
            let encoded = serde_json::to_string(&result).unwrap();
            for secret in [
                "fake-stash-secret",
                "fake-live-secret",
                "fake-profile-secret",
            ] {
                assert!(!encoded.contains(secret));
            }
            assert_eq!(snapshot(fixture.temp.path()), before);

            fs::write(
                &path,
                "[model.direct]\nbase_url = \"https://direct.test/v1\"\napi_key = \"fake-live-secret\"\n",
            )
            .unwrap();
            let before = snapshot(fixture.temp.path());
            let direct = preview(&fixture.ctx, &settings, &ToolId::Grok).unwrap();
            assert!(direct.can_disconnect);
            assert!(direct.files.is_empty());
            assert!(
                !direct
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("Grok 供应商页添加并应用一个直连供应商"))
            );
            assert_eq!(snapshot(fixture.temp.path()), before);
        }
    }

    fn codex_direct(ctx: &ApplyContext) {
        let mut direct = crate::provider_import::pool_relay_profile(
            "Direct",
            "https://direct.test/v1",
            "fake-direct-secret",
            RelayProtocol::Responses,
            "test-model",
            "test-model",
            &[],
        )
        .unwrap();
        direct.id = "direct".into();
        direct.relay_mode = RelayMode::PureApi;
        crate::relay_config::normalize_relay_profile_for_storage(&mut direct).unwrap();
        let mut settings = BackendSettings::default();
        settings.relay_profiles_enabled = true;
        settings.relay_profiles = vec![direct];
        settings.active_relay_id = "direct".into();
        crate::relay_switch::switch_relay_profile_in_home(
            &ctx.settings,
            &ctx.codex_home,
            settings,
            "",
        )
        .unwrap();
    }

    #[test]
    fn codex_stashed_config_and_auth_projection_exactly_match_real_restore() {
        let fixture = fixture();
        let ctx = &fixture.ctx;
        codex_direct(ctx);
        let path = ctx.codex_home.join("config.toml");
        let auth_path = ctx.codex_home.join("auth.json");
        let other = fs::read_to_string(&path)
            .unwrap()
            .replace("https://direct.test/v1", "http://127.0.0.1:3425/other");
        fs::write(&path, other).unwrap();
        fs::write(
            &auth_path,
            r#"{"auth_mode":"chatgpt","tokens":{"id_token":"x","access_token":"fake-before-token","refresh_token":"fake-refresh","account_id":"account"}}"#,
        )
        .unwrap();
        enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
        let mut current = fs::read_to_string(&path).unwrap();
        current.push_str("\n[mcp_servers.user]\ncommand = \"node\"\n");
        fs::write(&path, current).unwrap();
        let mut auth: Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        auth["tokens"]["access_token"] = json!("fake-refreshed-token");
        fs::write(&auth_path, serde_json::to_string(&auth).unwrap()).unwrap();
        let settings = ctx.settings.load().unwrap();
        let binding = ctx
            .gateway
            .load()
            .unwrap()
            .binding(&ToolId::Codex)
            .unwrap()
            .clone();
        let projected = apply::preview_codex_stashed_restore(ctx, &binding)
            .unwrap()
            .unwrap();
        let before = snapshot(fixture.temp.path());
        let result = preview(ctx, &settings, &ToolId::Codex).unwrap();
        assert!(result.can_disconnect, "{:?}", result.warnings);
        assert_eq!(snapshot(fixture.temp.path()), before);
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(encoded.contains("fake-refreshed-token"));
        assert!(encoded.contains("fake-direct-secret"));
        apply::disable_binding(ctx, &settings, &ToolId::Codex).unwrap();
        for (path, expected) in projected {
            assert_eq!(restore::read_optional(&path).unwrap(), expected);
        }
        let actual: Value = serde_json::from_str(&fs::read_to_string(auth_path).unwrap()).unwrap();
        assert_eq!(actual["tokens"]["access_token"], "fake-refreshed-token");
    }

    #[test]
    fn codex_without_stash_previews_the_real_full_switch() {
        let fixture = fixture();
        codex_direct(&fixture.ctx);
        enable_binding(&fixture.ctx, &ToolId::Codex, binding()).unwrap();
        // 没有实时存档（旧版接入、或存档丢了）：在隔离副本里跑真实的完整切换，按结果出差异。
        std::fs::remove_file(fixture.ctx.backup_root.join("codex-before-gateway.json")).ok();
        let settings = fixture.ctx.settings.load().unwrap();
        let live_before: Vec<(String, Option<String>)> = ["config.toml", "auth.json"]
            .iter()
            .map(|name| {
                let path = fixture.ctx.codex_home.join(name);
                (name.to_string(), restore::read_optional(&path).unwrap())
            })
            .collect();
        let before = snapshot(fixture.temp.path());
        let result = preview(&fixture.ctx, &settings, &ToolId::Codex).unwrap();
        assert!(!result.can_disconnect);
        assert_eq!(
            result.full_switch,
            Some(FullSwitchTarget { relay_id: "direct".into(), name: "Direct".into() })
        );
        assert_eq!(snapshot(fixture.temp.path()), before);
        // 预演给出的 diff 与真实断开写出的内容一致，且不泄露隔离副本的临时路径。
        assert!(!result.files.is_empty(), "完整切换的预演应给出文件差异");
        for file in &result.files {
            assert!(
                !file.diff.contains("codexplus-disconnect-preview"),
                "{} 的 diff 不应带隔离目录路径",
                file.path
            );
        }
        apply::disable_binding(&fixture.ctx, &settings, &ToolId::Codex).unwrap();
        for file in &result.files {
            let name = Path::new(&file.path)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            let live_before = live_before
                .iter()
                .find(|(n, _)| *n == name)
                .and_then(|(_, text)| text.clone());
            let after = restore::read_optional(Path::new(&file.path)).unwrap();
            assert_eq!(
                file.diff,
                text_diff(live_before.as_deref(), after.as_deref()),
                "{name} 的预演 diff 应与真实断开一致"
            );
        }
    }

    #[test]
    fn borrowed_codex_login_is_returned_only_in_the_private_preview_copy() {
        use base64::Engine;

        let fixture = fixture();
        let ctx = &fixture.ctx;
        let claims = json!({"iss": "https://auth.openai.com", "sub": "loan-user"});
        let id_token = format!(
            "header.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        codex_direct(ctx);
        let config_path = ctx.codex_home.join("config.toml");
        let auth_path = ctx.codex_home.join("auth.json");
        let other = fs::read_to_string(&config_path)
            .unwrap()
            .replace("https://direct.test/v1", "http://127.0.0.1:3425/other");
        fs::write(&config_path, other).unwrap();
        ctx.gateway
            .update(|gateway| {
                let mut provider = gateway.providers[0].clone();
                provider.relay.id = "loan".into();
                provider.relay.relay_mode = RelayMode::Official;
                provider.relay.api_key.clear();
                provider.relay.api_keys.clear();
                provider.relay.base_url.clear();
                provider.relay.auth_contents = json!({
                    "auth_mode": "chatgpt",
                    "codex_plus_gateway": true,
                    "tokens": {
                        "id_token": id_token,
                        "access_token": "fake-loaned-token",
                        "refresh_token": "fake-loaned-refresh",
                        "account_id": "loan-account"
                    }
                })
                .to_string();
                gateway.providers.push(provider);
                Ok(())
            })
            .unwrap();
        enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
        let mut auth: Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        auth["tokens"]["access_token"] = json!("fake-new-loaned-token");
        fs::write(&auth_path, auth.to_string()).unwrap();
        let settings = ctx.settings.load().unwrap();
        let binding = ctx
            .gateway
            .load()
            .unwrap()
            .binding(&ToolId::Codex)
            .unwrap()
            .clone();
        let before = snapshot(fixture.temp.path());
        let result = preview(ctx, &settings, &ToolId::Codex).unwrap();
        assert!(result.can_disconnect, "{:?}", result.warnings);
        assert_eq!(snapshot(fixture.temp.path()), before);
        assert!(
            serde_json::to_string(&result)
                .unwrap()
                .contains("fake-new-loaned-token")
        );

        let sandbox = Sandbox::new().unwrap();
        let copied = sandbox.context(ctx).unwrap();
        for (source, target) in [
            (ctx.gateway.path.clone(), copied.gateway.path.clone()),
            (config_path, copied.codex_home.join("config.toml")),
            (auth_path.clone(), copied.codex_home.join("auth.json")),
            (
                ctx.backup_root.join("codex-before-gateway.json"),
                copied.backup_root.join("codex-before-gateway.json"),
            ),
        ] {
            sandbox.write(&target, &fs::read(source).unwrap()).unwrap();
        }
        sandbox
            .write(
                &sandbox.root.join("settings.json"),
                &ctx.settings.snapshot_raw_bytes().unwrap().unwrap(),
            )
            .unwrap();
        apply::return_lent_login(&copied.gateway, &copied.codex_home).unwrap();
        let projected = apply::preview_codex_stashed_restore(&copied, &binding)
            .unwrap()
            .unwrap();
        apply::disable_binding(ctx, &settings, &ToolId::Codex).unwrap();
        for (copied_path, expected) in projected {
            let path = ctx.codex_home.join(copied_path.file_name().unwrap());
            assert_eq!(restore::read_optional(&path).unwrap(), expected);
        }
        let actual: Value = serde_json::from_str(&fs::read_to_string(auth_path).unwrap()).unwrap();
        assert!(actual.get("tokens").is_none());
        let gateway = ctx.gateway.load().unwrap();
        let returned: Value =
            serde_json::from_str(&gateway.provider("loan").unwrap().relay.auth_contents).unwrap();
        assert_eq!(returned["tokens"]["access_token"], "fake-new-loaned-token");
    }

    #[cfg(unix)]
    #[test]
    fn fingerprint_rejects_same_content_when_a_config_symlink_changes_target() {
        use std::os::unix::fs::symlink;
        let fixture = fixture();
        let path = generic(&fixture, Some("{\"model\":\"mine\"}"));
        let content = fs::read(&path).unwrap();
        let first = fixture.temp.path().join("first.json");
        let second = fixture.temp.path().join("second.json");
        fs::write(&first, &content).unwrap();
        fs::write(&second, &content).unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&first, &path).unwrap();
        let agent = ToolId::Unknown("opencode".into());
        let original = preview(&fixture.ctx, &BackendSettings::default(), &agent).unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&second, &path).unwrap();
        let changed = preview(&fixture.ctx, &BackendSettings::default(), &agent).unwrap();
        assert_ne!(original.fingerprint, changed.fingerprint);
    }

    #[test]
    fn text_diff_shows_full_lines_including_values() {
        let diff = text_diff(
            Some(
                "model = \"old\"\napi_key = \"fake-old-key\"\nkeep = 1\n",
            ),
            Some(
                "model = \"new\"\napi_key = \"fake-new-key\"\nkeep = 1\n",
            ),
        );
        assert!(diff.contains("--- current\n+++ restored\n"));
        assert!(diff.contains("-model = \"old\""));
        assert!(diff.contains("+model = \"new\""));
        assert!(diff.contains("-api_key = \"fake-old-key\""));
        assert!(diff.contains("+api_key = \"fake-new-key\""));
        assert!(diff.contains(" keep = 1"));
        // 相距超过上下文的变更分块展示，中间用 @@ 分隔。
        let spaced = text_diff(
            Some("a = \"old\"\nb = 1\nc = 1\nd = 1\ne = 1\nf = 1\ng = 1\nz = \"old\"\n"),
            Some("a = \"new\"\nb = 1\nc = 1\nd = 1\ne = 1\nf = 1\ng = 1\nz = \"new\"\n"),
        );
        assert!(spaced.contains("@@\n"));
        assert!(!spaced.contains(" d = 1"));
        // 整文件新增 / 删除按 /dev/null 头展示。
        assert_eq!(
            text_diff(None, Some("a = 1\n")),
            "--- /dev/null\n+++ restored\n+a = 1\n"
        );
        assert_eq!(
            text_diff(Some("a = 1\n"), None),
            "--- current\n+++ /dev/null\n-a = 1\n"
        );
        // 只差末尾换行时给出说明而不是空 diff。
        assert!(text_diff(Some("a = 1"), Some("a = 1\n")).contains("仅末尾换行"));
    }

    #[test]
    fn sandbox_is_private_and_removes_only_its_own_files() {
        let root;
        {
            let sandbox = Sandbox::new().unwrap();
            root = sandbox.root.clone();
            let path = root.join("auth.json");
            sandbox.write(&path, b"fake-test-secret").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                    0o700
                );
                assert_eq!(
                    fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
        assert!(!root.exists());
    }
}
