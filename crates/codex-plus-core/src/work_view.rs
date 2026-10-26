//! 纯 API Key 供应商下强制退出 Work 视图。
//!
//! 官方 26.1016.x 的 `conversationDetailMode = "STEPS_PROSE"`（Work 视图）在
//! 存在 `mcp_servers.node_repl` 时会要求会话带 ChatGPT 账号身份：
//! `getCachedAuthenticatedPrincipal()` 为空就直接抛
//! `Work browser access requires an authenticated account`，整条消息提交失败。
//! 纯 API Key 的 `auth.json` 只有 `OPENAI_API_KEY`，永远拿不到这个身份。
//!
//! 这里只在启动 Codex 之前把 `[desktop] conversationDetailMode` 写回
//! `STEPS_COMMANDS`，不碰 `config.toml` 的其它内容，也不干预已登录账号的
//! Work 视图（只对 `RelayMode::PureApi` 生效）。

use std::path::Path;

use anyhow::Context;

use crate::settings::{BackendSettings, RelayMode};

/// Codex 桌面端 `[desktop]` 里的详情模式键。
const DETAIL_MODE_KEY: &str = "conversationDetailMode";
/// 官方取值：Codex（命令式）视图。
const CODEX_DETAIL_MODE: &str = "STEPS_COMMANDS";

/// 开关是否应当介入：供应商配置、增强总开关、本开关都打开，且当前供应商是纯 API 模式。
pub fn should_apply(settings: &BackendSettings) -> bool {
    settings.relay_profiles_enabled
        && settings.enhancements_enabled
        && settings.codex_app_disable_work_view
        && settings.active_relay_profile().relay_mode == RelayMode::PureApi
}

/// 启动 Codex 前调用。返回被替换掉的旧值；没有改动时返回 `None`。
pub fn apply_detail_mode_preference(
    home: &Path,
    settings: &BackendSettings,
) -> anyhow::Result<Option<String>> {
    if !should_apply(settings) {
        return Ok(None);
    }
    force_codex_detail_mode(home)
}

/// 只改 `[desktop] conversationDetailMode`，保留文件其余内容与注释。
pub fn force_codex_detail_mode(home: &Path) -> anyhow::Result<Option<String>> {
    let path = home.join("config.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        // 新装环境还没有 config.toml 时不主动创建：没有 desktop 段就没有 Work 视图。
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("读取 {} 失败", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("解析 {} 失败", path.display()))?;
    let current = doc
        .get("desktop")
        .and_then(|desktop| desktop.get(DETAIL_MODE_KEY))
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    if current.as_deref() == Some(CODEX_DETAIL_MODE) {
        return Ok(None);
    }
    if doc.get("desktop").is_none() {
        doc["desktop"] = toml_edit::table();
    }
    let desktop = doc["desktop"]
        .as_table_mut()
        .context("config.toml 的 [desktop] 不是表，跳过详情模式改写")?;
    desktop[DETAIL_MODE_KEY] = toml_edit::value(CODEX_DETAIL_MODE);
    crate::settings::atomic_write(&path, doc.to_string().as_bytes())
        .with_context(|| format!("写入 {} 失败", path.display()))?;
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(home: &Path, desktop: &str) {
        std::fs::write(
            home.join("config.toml"),
            format!("model = \"gpt-5\"\n\n[desktop]\n{desktop}"),
        )
        .unwrap();
    }

    #[test]
    fn forces_codex_detail_mode_when_work_view_is_open() {
        let temp = tempfile::tempdir().unwrap();
        write_config(
            temp.path(),
            "conversationDetailMode = \"STEPS_PROSE\"\nshow-context-window-usage = true\n",
        );
        let replaced = force_codex_detail_mode(temp.path()).unwrap();
        assert_eq!(replaced.as_deref(), Some("STEPS_PROSE"));
        let config = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
        assert!(config.contains("conversationDetailMode = \"STEPS_COMMANDS\""));
        assert!(config.contains("show-context-window-usage = true"));
        assert!(config.contains("model = \"gpt-5\""));
    }

    #[test]
    fn second_run_is_a_no_op() {
        let temp = tempfile::tempdir().unwrap();
        write_config(temp.path(), "conversationDetailMode = \"STEPS_COMMANDS\"\n");
        assert_eq!(force_codex_detail_mode(temp.path()).unwrap(), None);
    }

    #[test]
    fn creates_desktop_section_when_missing() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("config.toml"), "model = \"gpt-5\"\n").unwrap();
        assert_eq!(force_codex_detail_mode(temp.path()).unwrap(), None);
        let config = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
        assert!(config.contains("[desktop]"));
        assert!(config.contains("conversationDetailMode = \"STEPS_COMMANDS\""));
    }

    #[test]
    fn missing_config_is_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(force_codex_detail_mode(temp.path()).unwrap(), None);
        assert!(!temp.path().join("config.toml").exists());
    }

    #[test]
    fn only_pure_api_profiles_are_touched() {
        let mut settings = BackendSettings::default();
        settings.codex_app_disable_work_view = true;
        assert!(!should_apply(&settings), "默认供应商不是纯 API");

        let mut profile = settings.active_relay_profile();
        profile.relay_mode = RelayMode::PureApi;
        settings.relay_profiles = vec![profile];
        settings.active_relay_id = settings.relay_profiles[0].id.clone();
        assert!(should_apply(&settings));

        settings.codex_app_disable_work_view = false;
        assert!(!should_apply(&settings));

        settings.codex_app_disable_work_view = true;
        settings.enhancements_enabled = false;
        assert!(!should_apply(&settings));

        settings.enhancements_enabled = true;
        settings.relay_profiles_enabled = false;
        assert!(!should_apply(&settings));
    }
}
