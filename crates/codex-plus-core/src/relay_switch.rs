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
    switch_relay_profile_with_apply(
        store,
        home,
        next_settings,
        previous_active_relay_id,
        apply_selected_relay_profile,
    )
}

fn switch_relay_profile_with_apply(
    store: &SettingsStore,
    home: &Path,
    next_settings: BackendSettings,
    previous_active_relay_id: &str,
    apply: impl FnOnce(&Path, &BackendSettings) -> anyhow::Result<RelaySwitchResult>,
) -> anyhow::Result<RelaySwitchResult> {
    if !next_settings.relay_profiles_enabled {
        anyhow::bail!("供应商配置总开关已关闭，未写入 config.toml / auth.json。");
    }
    store.save_with_prepared_settings(
        &next_settings,
        |selected| {
            // CAS 已通过，且直到发布/回滚结束都不释放锁；不能在锁外捕获旧 live
            // 快照，否则早期失败可能把别人的成功切换恢复掉。
            let saved = store
                .load()
                .context("读取当前供应商设置失败，已中止切换以免覆盖用户配置")?;
            let live_snapshot =
                LiveFilesSnapshot::capture(home).context("读取当前 Codex 实时配置失败")?;
            // 子集请求可能连上一个供应商也没有，必须先补齐再回填。
            restore_missing_saved_profiles(selected, &saved);
            if !previous_active_relay_id.trim().is_empty()
                && previous_active_relay_id != selected.active_relay_id
            {
                backfill_profile_before_switch(home, selected, previous_active_relay_id)?;
            }
            Ok(live_snapshot)
        },
        |committed_revision, live_snapshot| {
            let selected_settings = store.load().context("读取供应商设置失败")?;
            anyhow::ensure!(
                selected_settings.settings_revision == committed_revision,
                "settings_revision_conflict：应用前设置已被外部修改，未覆盖 Codex 实时文件"
            );
            // prepare 之后的保存归一化也可能耗时；未开始 apply 时不应覆盖外部
            // 刚更新的 live 文件，更不能把这类早期失败当成需要恢复 live 的失败。
            live_snapshot.ensure_unchanged(home)?;
            crate::codex_app_state::capture_app_state_snapshot_nonfatal(home, "relay_switch.before");
            // 实时文件由底层原子写入函数回滚；这里重放准备快照会覆盖应用期间
            // CC Switch/Codex 的新写入。settings 原始字节仍由保存事务恢复。
            let result = apply(home, &selected_settings)?;
            crate::codex_app_state::sync_app_state_after_provider_switch_nonfatal(
                home,
                "relay_switch.after",
            );
            // 换供应商/聚合后，Codex 模型选择器里可能还留着上一个目录的模型 ID
            // （例如别的管理器写下的 `ccs-xxx/model`），按新目录清一遍。
            crate::codex_app_state::prune_stale_model_references_after_switch_nonfatal(
                home,
                &selected_settings,
                "relay_switch.after",
            );
            Ok(result)
        },
    )
    .map_err(|error| anyhow::anyhow!("{error:#}"))
}

/// 切换请求必须携带完整 settings。供应商列表页曾按分页过滤后直接把子集提交给切换
/// （前端缺陷）：模型聚合会因成员不在请求里直接失败（聚合成员供应商不存在），
/// 一旦成功保存，其它分页的供应商还会从 settings.json 里消失。
/// 这里按 id 把磁盘上已有、请求里缺失的供应商与聚合配置补回；请求里的同 id 版本优先，
/// 只兜底缺失项，保证任何调用方都不会把用户配置越切越少。
fn restore_missing_saved_profiles(selected: &mut BackendSettings, saved: &BackendSettings) {
    let mut restored_ids: Vec<String> = Vec::new();
    for profile in &saved.relay_profiles {
        if !selected
            .relay_profiles
            .iter()
            .any(|item| item.id == profile.id)
        {
            selected.relay_profiles.push(profile.clone());
            restored_ids.push(profile.id.clone());
        }
    }
    for aggregate in &saved.aggregate_relay_profiles {
        if !selected
            .aggregate_relay_profiles
            .iter()
            .any(|item| item.id == aggregate.id)
        {
            selected.aggregate_relay_profiles.push(aggregate.clone());
            restored_ids.push(format!("aggregate:{}", aggregate.id));
        }
    }
    if !restored_ids.is_empty() {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "relay_switch.restored_missing_profiles",
            serde_json::json!({
                "restoredIds": restored_ids,
                "selectedProfileCount": selected.relay_profiles.len(),
                "savedProfileCount": saved.relay_profiles.len(),
            }),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

    fn ensure_unchanged(&self, home: &Path) -> anyhow::Result<()> {
        let current = Self::capture(home).context("应用前检查 Codex 实时配置失败")?;
        anyhow::ensure!(
            &current == self,
            "实时配置在切换准备期间发生变化，已停止切换并保留外部修改"
        );
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
        crate::relay_config::apply_relay_profile_for_switch(
            home,
            &relay,
            &common_config,
            crate::protocol_proxy::protocol_proxy_port(),
            settings.relay_preserve_nonprovider_settings,
        )?
    } else {
        validate_switch_profile_files(&relay)?;
        crate::relay_config::apply_relay_profile_for_switch(
            home,
            &relay,
            &common_config,
            crate::protocol_proxy::protocol_proxy_port(),
            settings.relay_preserve_nonprovider_settings,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::RelayProfile;

    #[test]
    fn failed_apply_preserves_external_live_files() {
        for after_apply in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().join("home");
            std::fs::create_dir(&home).unwrap();
            let store_path = temp.path().join("settings.json");
            let store = SettingsStore::new(store_path.clone());
            let profile = |id: &str| RelayProfile {
                id: id.to_string(),
                name: id.to_string(),
                relay_mode: RelayMode::PureApi,
                config_contents: format!(
                    "model_provider = \"custom\"\n[model_providers.custom]\nname = \"custom\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"https://{id}.example/v1\"\n"
                ),
                auth_contents: format!(r#"{{"OPENAI_API_KEY":"fixture-{id}"}}"#),
                ..RelayProfile::default()
            };
            store
                .save(&BackendSettings {
                    active_relay_id: "old".to_string(),
                    relay_profiles: vec![profile("old"), profile("next")],
                    ..BackendSettings::default()
                })
                .unwrap();
            let original = store.load().unwrap();
            apply_selected_relay_profile(&home, &original).unwrap();
            let original_settings = std::fs::read(&store_path).unwrap();
            let mut next = original;
            next.active_relay_id = "next".to_string();
            let external = profile("external");
            let error = switch_relay_profile_with_apply(
                &store,
                &home,
                next,
                "old",
                |home, selected| {
                    if after_apply {
                        apply_selected_relay_profile(home, selected)?;
                    }
                    // 模拟 CC Switch 在准备阶段或提交后替换两份实时文件。
                    std::fs::write(home.join("config.toml"), &external.config_contents)?;
                    std::fs::write(home.join("auth.json"), &external.auth_contents)?;
                    anyhow::bail!("injected apply failure");
                },
            )
            .unwrap_err();
            assert!(format!("{error:#}").contains("injected apply failure"));
            assert_eq!(std::fs::read(&store_path).unwrap(), original_settings);
            assert_eq!(
                std::fs::read_to_string(home.join("config.toml")).unwrap(),
                external.config_contents,
                "外部 config 不能被切换回滚覆盖，after_apply={after_apply}"
            );
            assert_eq!(
                std::fs::read_to_string(home.join("auth.json")).unwrap(),
                external.auth_contents,
                "外部 auth 不能被切换回滚覆盖，after_apply={after_apply}"
            );
        }
    }
}
