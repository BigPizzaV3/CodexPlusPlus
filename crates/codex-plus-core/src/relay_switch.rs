use std::io::ErrorKind;
use std::path::Path;

use anyhow::Context;

use crate::relay_config::{
    backfill_relay_profile_from_home_with_common, relay_config_status_from_home,
};
use crate::settings::{BackendSettings, RelayMode, SettingsStore};

#[derive(Debug, Clone, PartialEq)]
pub struct RelaySwitchResult {
    pub settings: BackendSettings,
    pub configured: bool,
    pub backup_path: Option<String>,
}

pub fn switch_relay_profile_in_home(
    store: &SettingsStore,
    home: &Path,
    next_settings: BackendSettings,
    previous_active_relay_id: &str,
) -> anyhow::Result<RelaySwitchResult> {
    let mut selected_settings = next_settings;
    if !selected_settings.relay_profiles_enabled {
        anyhow::bail!("供应商配置总开关已关闭，未写入 config.toml / auth.json。");
    }
    // 读不到当前设置时不要用默认值继续：回滚分支会把「仅剩默认供应商」写回磁盘，
    // 这正是供应商列表被清空的成因。读失败直接中止切换，保住原文件。
    store
        .load()
        .context("读取当前供应商设置失败，已中止切换以免覆盖用户配置")?;
    let original_settings_bytes = store
        .snapshot_raw_bytes()
        .context("读取当前供应商原始快照失败，已中止切换")?;
    let live_snapshot = LiveFilesSnapshot::capture(home).context("读取当前 Codex 实时配置失败")?;
    if !previous_active_relay_id.trim().is_empty()
        && previous_active_relay_id != selected_settings.active_relay_id
    {
        backfill_profile_before_switch(home, &mut selected_settings, previous_active_relay_id)?;
    }
    crate::codex_app_state::capture_app_state_snapshot_nonfatal(home, "relay_switch.before");

    store
        .save(&selected_settings)
        .context("保存供应商设置失败")?;
    let selected_settings = store.load().context("读取供应商设置失败")?;

    match apply_selected_relay_profile(home, &selected_settings) {
        Ok(result) => {
            crate::codex_app_state::sync_app_state_after_provider_switch_nonfatal(
                home,
                "relay_switch.after",
            );
            Ok(result)
        }
        Err(error) => {
            // 内部回滚恢复已验证原始字节，保留unknown字段并避免2→default1保护误拦。
            let settings_restore_error = store
                .restore_raw_snapshot(original_settings_bytes.as_deref())
                .err();
            let live_restore_error = live_snapshot.restore(home).err();
            if settings_restore_error.is_some() || live_restore_error.is_some() {
                anyhow::bail!(
                    "切换供应商失败：{error}；同时回滚配置失败：settings.json={}，Codex 实时文件={}",
                    settings_restore_error
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "ok".to_string()),
                    live_restore_error
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "ok".to_string())
                );
            }
            Err(error)
        }
    }
}

#[derive(Debug, Clone)]
struct LiveFilesSnapshot {
    config: Option<Vec<u8>>,
    auth: Option<Vec<u8>>,
}

impl LiveFilesSnapshot {
    fn capture(home: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            config: read_optional_bytes(&home.join("config.toml"))?,
            auth: read_optional_bytes(&home.join("auth.json"))?,
        })
    }

    fn restore(&self, home: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(home)?;
        restore_optional_file(&home.join("config.toml"), self.config.as_deref())
            .context("恢复 config.toml 失败")?;
        restore_optional_file(&home.join("auth.json"), self.auth.as_deref())
            .context("恢复 auth.json 失败")?;
        Ok(())
    }
}

fn read_optional_bytes(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn restore_optional_file(path: &Path, contents: Option<&[u8]>) -> anyhow::Result<()> {
    match contents {
        Some(contents) => crate::settings::atomic_write(path, contents).map_err(Into::into),
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

fn backfill_profile_before_switch(
    home: &Path,
    settings: &mut BackendSettings,
    previous_active_relay_id: &str,
) -> anyhow::Result<()> {
    // 确认属于另一个已保存供应商的 live 配置，不回填到旧 profile（共享 Key 也适用）。
    if crate::relay_config::live_config_matches_other_profile_in_home(
        home,
        settings,
        previous_active_relay_id,
    )? {
        return Ok(());
    }
    // 找不到上一个供应商是用户能自助处理的一类原因（它已被删除或 id 变了），
    // 单独给一条能直接照做的话；下面回填失败则属于另一类，见那里的写法。
    let profile = settings
        .relay_profiles
        .iter_mut()
        .find(|profile| profile.id == previous_active_relay_id)
        .with_context(|| {
            format!(
                "当前供应商（{previous_active_relay_id}）已不在配置列表中，已停止切换以避免覆盖用户改动。"
            )
        })?;
    // issue #1888：管理器把本函数返回的错误拼成「回填当前供应商配置失败：{error}」，
    // 而 `{error}` 只打印最外层 context，底层原因（读 home 失败等）会被丢掉，
    // 用户看到的就是一句没有信息量的报错。这里把 context 链完整拼进消息里。
    backfill_relay_profile_from_home_with_common(
        home,
        profile,
        &mut settings.relay_context_config_contents,
    )
    .with_context(|| format!("回填当前供应商配置失败（供应商：{previous_active_relay_id}）"))
    .map_err(|error| anyhow::anyhow!("{error:#}"))
}

fn apply_selected_relay_profile(
    home: &Path,
    settings: &BackendSettings,
) -> anyhow::Result<RelaySwitchResult> {
    let relay = settings.active_relay_profile();
    let common_config = relay_combined_common_config(settings);
    let result = if relay.relay_mode == RelayMode::Official && !relay.official_mix_api_key {
        crate::relay_config::apply_official_profile_to_home(home, &relay, &common_config)?
    } else {
        validate_switch_profile_files(&relay)?;
        crate::relay_config::apply_relay_profile_to_home_with_switch_rules(
            home,
            &relay,
            &common_config,
        )?
    };
    let status = relay_config_status_from_home(home);
    if relay.relay_mode == RelayMode::PureApi && !status.configured {
        anyhow::bail!(
            "纯 API 配置写入后未检测到完整 custom provider，请检查 config.toml 和供应商 API Key。"
        );
    }
    Ok(RelaySwitchResult {
        settings: settings.clone(),
        configured: status.configured,
        backup_path: result.backup_path,
    })
}

fn validate_switch_profile_files(profile: &crate::settings::RelayProfile) -> anyhow::Result<()> {
    if profile.relay_mode != RelayMode::Aggregate && profile.config_contents.trim().is_empty() {
        anyhow::bail!(
            "供应商「{}」缺少独立 config.toml，已停止切换，避免继续显示上一套配置文件。",
            if profile.name.trim().is_empty() {
                profile.id.as_str()
            } else {
                profile.name.as_str()
            }
        );
    }
    if profile.relay_mode == RelayMode::Official
        && serde_json::from_str::<serde_json::Value>(&profile.auth_contents)
            .ok()
            .and_then(|value| {
                value
                    .get("OPENAI_API_KEY")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .map(str::is_empty)
            })
            == Some(false)
    {
        anyhow::bail!(
            "官方混合 API 不应在 auth.json 中保存 OPENAI_API_KEY。请清理此供应商的 auth.json 后再切换。"
        );
    }
    Ok(())
}

fn relay_combined_common_config(settings: &BackendSettings) -> String {
    let sections = [
        settings.relay_common_config_contents.trim(),
        settings.relay_context_config_contents.trim(),
    ]
    .into_iter()
    .filter(|section| !section.is_empty())
    .collect::<Vec<_>>();
    if sections.is_empty() {
        String::new()
    } else {
        crate::relay_config::normalize_config_text(&format!("{}\n", sections.join("\n\n")))
    }
}
