//! 显式接入 Codex 的协作 MCP；只管理本条 stdio server，不读取或改写认证文件。
//!
//! 具体的读写、归属判断与备份由 [`crate::managed_mcp`] 统一实现，这里只固定协作服务的描述。

use std::path::Path;

use crate::managed_mcp::{self, ManagedMcp};

pub use crate::managed_mcp::{InstallReceipt, RegistrationStatus};

pub const SERVER_ID: &str = "codex_plus_collaboration";

pub const SPEC: ManagedMcp = ManagedMcp {
    server_id: SERVER_ID,
    flag: "--codex-plus-agent-mcp",
    label: "协作 MCP",
    comment: "Managed by Codex++: agent collaboration MCP",
    lock_file: ".codex-plus-agent-mcp.lock",
    backup_dir: ".codex-plus-agent-mcp-backups",
    legacy: &[],
};

/// 只读检查本条 MCP；存在配置不代表已启动的 Codex 会话已经重新加载工具。
pub fn status(
    home: &Path,
    executable: &Path,
    state_dir: &Path,
) -> anyhow::Result<RegistrationStatus> {
    managed_mcp::status(&SPEC, home, executable, state_dir)
}

/// 原子安装/更新 live config。本条发生变化时先保存私有备份；重复安装不重写文件。
/// 管理器同时保存托管上下文；live 发布失败时只回滚本次上下文字段。
pub fn install(home: &Path, executable: &Path, state_dir: &Path) -> anyhow::Result<InstallReceipt> {
    managed_mcp::install(&SPEC, home, executable, state_dir)
}

/// 纯转换：用于 settings.relay_context_config_contents，保留其他 server 与未知选项。
pub fn upsert_managed_context(
    contents: &str,
    executable: &Path,
    state_dir: &Path,
) -> anyhow::Result<String> {
    managed_mcp::upsert_managed_context(&SPEC, contents, executable, state_dir)
}
