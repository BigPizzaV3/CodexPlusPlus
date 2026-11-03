//! Codex++ 托管的本机 stdio MCP 条目：在 Codex `config.toml` 的 `[mcp_servers.<id>]` 下
//! 只管理自己那一条，不读取或改写认证文件。
//!
//! 每个托管服务由一份 [`ManagedMcp`] 描述（条目 id、启动参数首项、锁与备份文件名、文案）。
//! 归属靠 `args[0]` 识别：首个参数是本服务的启动标志才算「本工具的条目」，否则视为冲突、不覆盖。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use fs2::FileExt;
use serde::Serialize;
use toml_edit::{DocumentMut, Item, Table, Value};

const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

/// 一个托管 MCP 服务的固定描述。
#[derive(Debug, Clone, Copy)]
pub struct ManagedMcp {
    /// `[mcp_servers.<server_id>]`。
    pub server_id: &'static str,
    /// 启动参数首项，同时是归属标记。
    pub flag: &'static str,
    /// 用户可见的名称，例如「协作 MCP」。
    pub label: &'static str,
    /// 新建条目时写在上方的注释。
    pub comment: &'static str,
    /// Codex home 下的锁文件名。
    pub lock_file: &'static str,
    /// Codex home 下的备份目录名。
    pub backup_dir: &'static str,
    /// 本服务改名前用过的 `(条目 id, 启动标志)`。接入时把仍归本工具所有的旧条目移除，
    /// 免得 Codex 里同时挂着新旧两份；归属不符的同名条目不动。
    pub legacy: &'static [(&'static str, &'static str)],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationStatus {
    /// registered / missing / needs_update / conflict。
    pub status: String,
    pub message: String,
    pub home: String,
    pub config_path: String,
    pub registered: bool,
    pub owned: bool,
    /// 本工具的条目存在但被 `enabled = false` 停用（用户取消了接入）。
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReceipt {
    pub status: RegistrationStatus,
    pub changed: bool,
    pub backup_path: Option<String>,
}

/// 只读检查本条 MCP；存在配置不代表已启动的 Codex 会话已经重新加载工具。
pub fn status(
    spec: &ManagedMcp,
    home: &Path,
    executable: &Path,
    state_dir: &Path,
) -> anyhow::Result<RegistrationStatus> {
    let home = validate_paths(home, executable, state_dir)?;
    let path = home.join("config.toml");
    let config = read_config(&path)?;
    let document = parse_config(config.as_deref().unwrap_or(""))?;
    status_from_document(spec, &home, &document, executable, state_dir)
}

/// 原子安装/更新 live config。本条发生变化时先保存私有备份；重复安装不重写文件。
/// 管理器同时保存托管上下文；live 发布失败时只回滚本次上下文字段。
pub fn install(
    spec: &ManagedMcp,
    home: &Path,
    executable: &Path,
    state_dir: &Path,
) -> anyhow::Result<InstallReceipt> {
    commit_live(spec, home, executable, state_dir, |contents| {
        upsert_managed_context(spec, contents, executable, state_dir)
    })
}

/// 取消接入：把 live config 里本工具的条目置为 `enabled = false`，Codex 新会话就不再加载它的工具，
/// 省下工具清单占用的上下文。条目本身和用户加在上面的字段都保留，重新接入时原样启用。
pub fn disable(
    spec: &ManagedMcp,
    home: &Path,
    executable: &Path,
    state_dir: &Path,
) -> anyhow::Result<InstallReceipt> {
    commit_live(spec, home, executable, state_dir, |contents| {
        disable_managed_context(spec, contents)
    })
}

/// 在锁内读出 live config、做一次纯转换，内容有变化才备份并原子写回。
fn commit_live(
    spec: &ManagedMcp,
    home: &Path,
    executable: &Path,
    state_dir: &Path,
    transform: impl FnOnce(&str) -> anyhow::Result<String>,
) -> anyhow::Result<InstallReceipt> {
    let home = validate_paths(home, executable, state_dir)?;
    let lock_path = home.join(spec.lock_file);
    reject_symlink(&lock_path)?;
    let lock = open_private_new_or_existing(&lock_path)?;
    FileExt::lock_exclusive(&lock).with_context(|| format!("无法锁定{}接入操作", spec.label))?;
    let path = home.join("config.toml");
    let before = read_config(&path)?;
    let updated = transform(before.as_deref().unwrap_or(""))?;
    if before.as_deref().unwrap_or("") == updated {
        let document = parse_config(&updated)?;
        return Ok(InstallReceipt {
            status: status_from_document(spec, &home, &document, executable, state_dir)?,
            changed: false,
            backup_path: None,
        });
    }
    let backup_path = before
        .as_deref()
        .map(|contents| backup_config(spec, &home, contents))
        .transpose()?;
    // 其他程序不一定遵守本锁；发现读取后的变化就拒绝提交，不能覆盖它的新配置。
    if read_config(&path)? != before {
        bail!("Codex 配置在接入期间发生变化，未覆盖；请重新检查后再接入");
    }
    crate::settings::atomic_write_with(&path, |file| {
        // atomic_write_with 会在写完后恢复原权限；写入内容前先收紧临时文件权限。
        make_private(file)?;
        file.write_all(updated.as_bytes())
    })
    .with_context(|| format!("无法原子保存 Codex {}配置", spec.label))?;
    let document = parse_config(&updated)?;
    Ok(InstallReceipt {
        status: status_from_document(spec, &home, &document, executable, state_dir)?,
        changed: true,
        backup_path: backup_path.map(|path| path.to_string_lossy().into_owned()),
    })
}

/// 纯转换：用于 settings.relay_context_config_contents，保留其他 server 与未知选项。
/// 不调用既有“覆盖整条” helper，以便保留此条上用户设置的 approval_mode 等字段。
pub fn upsert_managed_context(
    spec: &ManagedMcp,
    contents: &str,
    executable: &Path,
    state_dir: &Path,
) -> anyhow::Result<String> {
    validate_launch_paths(executable, state_dir)?;
    let mut document = parse_config(contents)?;
    if let Some(server) = server_entry(spec, &document)? {
        if !is_owned(spec, server) {
            bail!("同名 {} 已被其他 MCP 配置占用，未覆盖", spec.server_id);
        }
        if has_transport_conflict(server) {
            bail!("同名{}包含 HTTP url，不能安全接管，未修改配置", spec.label);
        }
    }
    if document.get("mcp_servers").is_none() {
        let mut servers = Table::new();
        servers.set_implicit(true);
        document
            .as_table_mut()
            .insert("mcp_servers", Item::Table(servers));
    }
    let servers = document
        .get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
        .context("mcp_servers 必须是 TOML 表，未修改配置")?;
    for (legacy_id, legacy_flag) in spec.legacy {
        let owned = servers
            .get(legacy_id)
            .and_then(Item::as_table_like)
            .and_then(|server| server.get("args"))
            .and_then(Item::as_array)
            .and_then(|args| args.get(0))
            .and_then(Value::as_str)
            == Some(*legacy_flag);
        if owned {
            servers.remove(legacy_id);
        }
    }
    let new_entry = servers.get(spec.server_id).is_none();
    if new_entry {
        let mut server = Table::new();
        server
            .decor_mut()
            .set_prefix(format!("\n# {}\n", spec.comment));
        servers.insert(spec.server_id, Item::Table(server));
    }
    let server = servers
        .get_mut(spec.server_id)
        .and_then(Item::as_table_like_mut)
        .with_context(|| format!("{}条目必须是 TOML 表，未修改配置", spec.label))?;
    if server.get("command").and_then(Item::as_str) != executable.to_str() {
        set_value_preserving_decor(server, "command", Value::from(path_text(executable)?));
    }
    if !args_match(spec, server.get("args").and_then(Item::as_array), state_dir) {
        let mut args = toml_edit::Array::new();
        args.push(spec.flag);
        args.push("--state-dir");
        args.push(path_text(state_dir)?);
        set_value_preserving_decor(server, "args", Value::Array(args));
    }
    if new_entry
        || server
            .get("enabled")
            .is_some_and(|item| item.as_bool() != Some(true))
    {
        set_value_preserving_decor(server, "enabled", Value::from(true));
    }
    // 启动超时显式写长一点。
    //
    // 这些托管服务不是轻量握手：extension / dsh 宿主启动时要把整张注册表里的插件都加载
    // 一遍（实测 dsh 宿主冷启动能到十几秒），Codex 默认的 MCP 启动超时等不到 ready 就会
    // 判定失败，然后重连——而重连时旧连接的管道不一定被关掉，子进程只靠 stdin 收尾，
    // 于是每十几秒多一个挂着不动的孤儿（实测一天累积 45 个进程 / 2.2 GB）。
    // 写长超时让慢启动也能等到 ready，不再触发那条重连路径。用户自己改过就尊重原值。
    if server.get("startup_timeout_sec").is_none() {
        set_value_preserving_decor(server, "startup_timeout_sec", Value::from(120.0_f64));
    }
    Ok(document.to_string())
}

/// 纯转换：把本工具的条目（含仍归本工具所有的旧名条目）置为 `enabled = false`。
/// 没有条目或条目归属不符时原样返回，不碰别人的配置。
pub fn disable_managed_context(spec: &ManagedMcp, contents: &str) -> anyhow::Result<String> {
    let mut document = parse_config(contents)?;
    let Some(servers) = document.get_mut("mcp_servers") else {
        return Ok(contents.to_string());
    };
    let servers = servers
        .as_table_like_mut()
        .context("mcp_servers 必须是 TOML 表，未修改配置")?;
    let mut changed = false;
    let owned_ids = std::iter::once((spec.server_id, spec.flag)).chain(spec.legacy.iter().copied());
    for (id, flag) in owned_ids {
        let Some(server) = servers.get_mut(id).and_then(Item::as_table_like_mut) else {
            continue;
        };
        let owned = server
            .get("args")
            .and_then(Item::as_array)
            .and_then(|args| args.get(0))
            .and_then(Value::as_str)
            == Some(flag);
        if owned && server.get("enabled").and_then(Item::as_bool) != Some(false) {
            set_value_preserving_decor(server, "enabled", Value::from(false));
            changed = true;
        }
    }
    Ok(if changed { document.to_string() } else { contents.to_string() })
}

/// 托管上下文里本工具的条目是否被停用。切换供应商的 apply 流程会把停用条目整条移出 live 配置，
/// 所以「已取消接入」要以托管上下文为准。
pub fn disabled_in_context(spec: &ManagedMcp, contents: &str) -> bool {
    let Ok(document) = parse_config(contents) else {
        return false;
    };
    server_entry(spec, &document)
        .ok()
        .flatten()
        .filter(|server| is_owned(spec, server))
        .and_then(Item::as_table_like)
        .and_then(|server| server.get("enabled"))
        .and_then(Item::as_bool)
        == Some(false)
}

fn set_value_preserving_decor(
    table: &mut dyn toml_edit::TableLike,
    key: &str,
    mut replacement: Value,
) {
    if let Some(existing) = table.get_mut(key) {
        if let Some(value) = existing.as_value() {
            *replacement.decor_mut() = value.decor().clone();
        }
        // 原地替换值，保留键自身的前缀注释；insert 会连同 Key 一起重建。
        *existing = Item::Value(replacement);
    } else {
        table.insert(key, Item::Value(replacement));
    }
}

fn validate_paths(home: &Path, executable: &Path, state_dir: &Path) -> anyhow::Result<PathBuf> {
    validate_launch_paths(executable, state_dir)?;
    if !home.is_absolute() || !home.is_dir() {
        bail!("请选择已存在的绝对 Codex home 目录");
    }
    home.canonicalize().context("无法确认 Codex home 目录")
}

fn validate_launch_paths(executable: &Path, state_dir: &Path) -> anyhow::Result<()> {
    if !executable.is_absolute() || !executable.is_file() {
        bail!("MCP 可执行文件必须是已存在的绝对文件路径");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(executable)?.permissions().mode() & 0o111 == 0 {
            bail!("MCP 文件没有执行权限");
        }
    }
    if !state_dir.is_absolute() || (state_dir.exists() && !state_dir.is_dir()) {
        bail!("MCP 状态目录必须是明确的绝对目录路径");
    }
    path_text(executable)?;
    path_text(state_dir)?;
    Ok(())
}

fn path_text(path: &Path) -> anyhow::Result<&str> {
    let text = path.to_str().context("MCP 路径必须是有效 UTF-8")?;
    if text.chars().any(char::is_control) {
        bail!("MCP 路径不能包含控制字符");
    }
    Ok(text)
}

fn reject_symlink(path: &Path) -> anyhow::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("接入目标不能是符号链接，未修改配置")
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => bail!("无法检查 MCP 接入相关文件"),
    }
}

fn read_config(path: &Path) -> anyhow::Result<Option<String>> {
    reject_symlink(path)?;
    match std::fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES => {
            bail!("Codex 配置不是普通文件或超过 4 MiB 上限，未修改");
        }
        Ok(_) => std::fs::read_to_string(path)
            .map(Some)
            .map_err(|_| anyhow::anyhow!("无法读取 Codex 配置，未修改")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => bail!("无法读取 Codex 配置属性，未修改"),
    }
}

fn parse_config(contents: &str) -> anyhow::Result<DocumentMut> {
    // toml_edit 错误可能附带有凭据的源行，不能带到 UI 或日志。
    contents
        .parse::<DocumentMut>()
        .map_err(|_| anyhow::anyhow!("Codex/MCP 配置不是有效 TOML，未修改原文件"))
}

fn server_entry<'a>(
    spec: &ManagedMcp,
    document: &'a DocumentMut,
) -> anyhow::Result<Option<&'a Item>> {
    match document.get("mcp_servers") {
        None => Ok(None),
        Some(servers) => Ok(servers
            .as_table_like()
            .context("mcp_servers 必须是 TOML 表，未修改配置")?
            .get(spec.server_id)),
    }
}

fn is_owned(spec: &ManagedMcp, server: &Item) -> bool {
    server
        .as_table_like()
        .and_then(|server| server.get("args"))
        .and_then(Item::as_array)
        .and_then(|args| args.get(0))
        .and_then(Value::as_str)
        == Some(spec.flag)
}

fn has_transport_conflict(server: &Item) -> bool {
    server
        .as_table_like()
        .is_some_and(|table| table.get("url").is_some())
}

fn status_from_document(
    spec: &ManagedMcp,
    home: &Path,
    document: &DocumentMut,
    executable: &Path,
    state_dir: &Path,
) -> anyhow::Result<RegistrationStatus> {
    let entry = server_entry(spec, document)?;
    let owned = entry.is_some_and(|server| is_owned(spec, server));
    if entry.is_some_and(has_transport_conflict) {
        return Ok(RegistrationStatus {
            status: "conflict".to_string(),
            message: format!("同名{}包含 HTTP url，未修改配置", spec.label),
            home: home.to_string_lossy().into_owned(),
            config_path: home.join("config.toml").to_string_lossy().into_owned(),
            registered: false,
            owned,
            disabled: false,
        });
    }
    let current = entry
        .filter(|_| owned)
        .and_then(Item::as_table_like)
        .is_some_and(|server| {
            server.get("command").and_then(Item::as_str) == executable.to_str()
                && server.get("enabled").and_then(Item::as_bool) != Some(false)
                && args_match(spec, server.get("args").and_then(Item::as_array), state_dir)
        });
    let disabled = entry
        .filter(|_| owned)
        .and_then(Item::as_table_like)
        .and_then(|server| server.get("enabled"))
        .and_then(Item::as_bool)
        == Some(false);
    let (status, message) = match (entry.is_some(), owned, current) {
        (_, true, false) if disabled => ("needs_update", "已取消接入，Codex 新对话不会加载这些工具"),
        (false, _, _) => ("missing", "尚未接入 Codex"),
        (true, false, _) => ("conflict", "同名 MCP 属于其他配置，未接管"),
        (_, true, false) => ("needs_update", "已找到本工具的 MCP，需要更新路径或启用"),
        (_, true, true) => ("registered", ""),
    };
    let message = if status == "registered" {
        format!("已注册{}；新开 Codex 对话或重新加载 MCP 后可使用", spec.label)
    } else {
        message.to_string()
    };
    Ok(RegistrationStatus {
        status: status.to_string(),
        message,
        home: home.to_string_lossy().into_owned(),
        config_path: home.join("config.toml").to_string_lossy().into_owned(),
        registered: current,
        owned,
        disabled,
    })
}

fn args_match(spec: &ManagedMcp, args: Option<&toml_edit::Array>, state_dir: &Path) -> bool {
    args.is_some_and(|args| {
        args.len() == 3
            && args.get(0).and_then(Value::as_str) == Some(spec.flag)
            && args.get(1).and_then(Value::as_str) == Some("--state-dir")
            && args.get(2).and_then(Value::as_str) == state_dir.to_str()
    })
}

fn open_private_new_or_existing(path: &Path) -> anyhow::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).context("无法打开 MCP 接入锁")
}

fn make_private(file: &File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

fn backup_config(spec: &ManagedMcp, home: &Path, contents: &str) -> anyhow::Result<PathBuf> {
    let directory = home.join(spec.backup_dir);
    reject_symlink(&directory)?;
    if !directory.exists() {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&directory)
            .context("无法创建 MCP 配置备份目录")?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    }
    let path = directory.join(format!(
        "config-{}-{}.toml.bak",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_millis())
            .unwrap_or_default(),
        uuid::Uuid::new_v4()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).context("无法创建 MCP 配置备份")?;
    file.write_all(contents.as_bytes())
        .context("无法保存 MCP 配置备份")?;
    file.sync_all()?;
    Ok(path)
}
