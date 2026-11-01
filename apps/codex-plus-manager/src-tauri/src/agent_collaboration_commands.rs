//! 管理器专用入口。协作配置不进入已有 settings 或第三方拓展路由。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use codex_plus_core::agent_collaboration::{
    CollaborationConfig, Store, TaskRequest, executor, integration, mcp, native_refresh,
};
use codex_plus_core::settings::SettingsStore;
use serde_json::{Value, json};

#[tauri::command]
pub async fn agent_collaboration_request(
    action: String,
    payload: Option<Value>,
) -> Result<Value, String> {
    let store = Store::default();
    if action == "mcp_config" {
        return mcp_configuration(
            &std::env::current_exe().map_err(|e| e.to_string())?,
            &codex_plus_core::paths::default_app_state_dir(),
        )
        .map_err(|e| e.to_string());
    }
    let refresh_requested = matches!(action.as_str(), "connect_codex" | "refresh_codex");
    let task_store = store.clone();
    let (mut snapshot, task_id, debug_ports) =
        tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
            let payload = payload.unwrap_or(Value::Null);
            let executable = std::env::current_exe().context("无法定位子 Agent MCP 程序")?;
            let home = codex_plus_core::codex_home::default_codex_home_dir();
            let state_dir = codex_plus_core::paths::default_app_state_dir();
            let task_id = match action.as_str() {
                "snapshot" => None,
                "refresh_codex" => {
                    require_owned_registration(&home, &executable, &state_dir)?;
                    None
                }
                "connect_codex" => {
                    connect_codex(
                        &integration::SPEC,
                        &SettingsStore::default(),
                        &home,
                        &executable,
                        &state_dir,
                    )?;
                    None
                }
                "save_config" => {
                    let config: CollaborationConfig =
                        serde_json::from_value(payload).context("协作配置格式无效")?;
                    store.save_config(config)?;
                    None
                }
                "submit" => {
                    let request: TaskRequest =
                        serde_json::from_value(payload).context("任务参数格式无效")?;
                    Some(store.submit(request)?.id)
                }
                "cancel" => {
                    let id = payload
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.trim().is_empty())
                        .context("缺少任务 ID")?;
                    store.cancel(id)?;
                    None
                }
                _ => bail!("不支持的协作操作"),
            };
            let mut snapshot = store.snapshot()?;
            snapshot["integration"] = registration_view(&home, &executable, &state_dir);
            let debug_ports = if refresh_requested {
                codex_refresh_ports()
            } else {
                Vec::new()
            };
            Ok((snapshot, task_id, debug_ports))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    if let Some(task_id) = task_id {
        executor::spawn_task(task_store, task_id);
    }
    if refresh_requested {
        snapshot["runtime"] =
            serde_json::to_value(native_refresh::refresh_in_codex(&debug_ports).await)
                .map_err(|_| "无法读取 Codex MCP 刷新结果".to_string())?;
    }
    Ok(snapshot)
}

/// 本实例状态、常规安装状态和标准端口；实际操作前仍严格验证 Codex 页面身份。
fn codex_refresh_ports() -> Vec<u16> {
    let mut ports = Vec::new();
    let mut push = |port: Option<u16>| {
        if let Some(port) = port.filter(|port| *port > 0) {
            if !ports.contains(&port) {
                ports.push(port);
            }
        }
    };
    push(
        codex_plus_core::status::StatusStore::default()
            .load_latest()
            .ok()
            .flatten()
            .and_then(|status| status.debug_port),
    );
    // 本地测试包隔离了自身状态，但接入的仍然是正常运行的 Codex App。
    if let Some(base) = directories::BaseDirs::new() {
        let path = base
            .home_dir()
            .join(".codex-session-delete/latest-status.json");
        push(
            codex_plus_core::status::StatusStore::new(path)
                .load_latest()
                .ok()
                .flatten()
                .and_then(|status| status.debug_port),
        );
    }
    push(Some(9229));
    ports
}

fn registration_view(home: &Path, executable: &Path, state_dir: &Path) -> Value {
    match integration::status(home, executable, state_dir) {
        Ok(registration) => json!({
            "status": if registration.registered { "connected" } else if registration.status == "conflict" { "conflict" } else { "not_connected" },
            "message": registration.message,
            "home": registration.home,
            "configPath": registration.config_path,
        }),
        Err(error) => json!({"status": "not_connected", "message": error.to_string()}),
    }
}

/// 刷新只能用于已明确接入的本工具配置；旧测试包路径不一致仍可重读。
fn require_owned_registration(
    home: &Path,
    executable: &Path,
    state_dir: &Path,
) -> anyhow::Result<()> {
    let registration = integration::status(home, executable, state_dir)?;
    if !registration.owned || registration.status == "conflict" {
        bail!("请先一键接入 Codex，再刷新工具；当前协作 MCP 未登记或归属不符");
    }
    Ok(())
}

/// 只更新指定托管 MCP 这一条。使用既有局部更新 API，保留其他设置与供应商数据。
/// 协作 MCP 与插件创作 MCP 共用这套「托管上下文 + live 配置」两段提交与回滚。
pub(crate) fn connect_codex(
    spec: &codex_plus_core::managed_mcp::ManagedMcp,
    settings_store: &SettingsStore,
    home: &Path,
    executable: &Path,
    state_dir: &Path,
) -> anyhow::Result<()> {
    let registration = codex_plus_core::managed_mcp::status(spec, home, executable, state_dir)?;
    if registration.status == "conflict" {
        bail!("同名{}属于其他配置，未覆盖", spec.label);
    }
    commit_managed_change(
        spec,
        settings_store,
        "接入",
        |contents| {
            codex_plus_core::managed_mcp::upsert_managed_context(spec, contents, executable, state_dir)
        },
        || codex_plus_core::managed_mcp::install(spec, home, executable, state_dir).map(|_| ()),
    )
}

/// 取消接入：托管上下文与 live 配置里本工具的条目都置为 `enabled = false`（切换供应商重写配置时
/// 也不会被重新启用）。条目归属不符时什么都不改。
pub(crate) fn disconnect_codex(
    spec: &codex_plus_core::managed_mcp::ManagedMcp,
    settings_store: &SettingsStore,
    home: &Path,
    executable: &Path,
    state_dir: &Path,
) -> anyhow::Result<()> {
    commit_managed_change(
        spec,
        settings_store,
        "取消接入",
        |contents| codex_plus_core::managed_mcp::disable_managed_context(spec, contents),
        || codex_plus_core::managed_mcp::disable(spec, home, executable, state_dir).map(|_| ()),
    )
}

/// 两段提交：先写 Codex++ 托管上下文，再写 Codex live 配置；live 失败时只回滚本次写入的上下文。
fn commit_managed_change(
    spec: &codex_plus_core::managed_mcp::ManagedMcp,
    settings_store: &SettingsStore,
    verb: &str,
    transform: impl FnOnce(&str) -> anyhow::Result<String>,
    commit_live: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let settings = settings_store
        .load()
        .with_context(|| format!("无法读取 Codex++ 托管配置，未{verb}"))?;
    let before = settings.relay_context_config_contents.clone();
    let updated = transform(&before)?;
    let changed = updated != before;
    let mut committed_context = updated.clone();
    if changed {
        let committed = settings_store
            .update(context_patch(&settings, &updated)?)
            .with_context(|| format!("无法保存{}托管配置，未修改 Codex live 配置", spec.label))?;
        // 设置存储会归一化 TOML，以实际提交结果作为回滚的比较基准。
        committed_context = committed.relay_context_config_contents;
    }
    if let Err(error) = commit_live() {
        if changed {
            // 重新读取，恢复的仅是本次写入的上下文字段，不能覆盖期间的其他设置变化。
            let rollback = (|| -> anyhow::Result<()> {
                let latest = settings_store.load()?;
                if latest.relay_context_config_contents != committed_context {
                    bail!("托管 MCP 已被其他操作修改，未覆盖新数据");
                }
                settings_store
                    .update(context_patch(&latest, &before)?)
                    .map(|_| ())
            })();
            if rollback.is_err() {
                bail!("{verb} Codex 失败：{error}；托管配置未能恢复，请刷新 MCP 设置后检查");
            }
        }
        return Err(error).with_context(|| format!("{}{verb}失败，托管配置已保持原状", spec.label));
    }
    Ok(())
}

fn context_patch(
    settings: &codex_plus_core::settings::BackendSettings,
    contents: &str,
) -> anyhow::Result<Value> {
    let mut patch = json!({"relayContextConfigContents": contents});
    // 较新的设置存储支持条件更新；通过序列化元数据兼容已有公开 API。
    if let Some(revision) = serde_json::to_value(settings)?
        .get("revision")
        .filter(|value| value.is_string())
    {
        patch["revision"] = revision.clone();
    }
    Ok(patch)
}

fn mcp_configuration(executable: &Path, state_dir: &Path) -> anyhow::Result<Value> {
    if !executable.is_absolute() || !state_dir.is_absolute() {
        bail!("MCP 可执行文件与状态目录必须使用绝对路径");
    }
    let command = executable
        .to_str()
        .context("MCP 可执行路径不是有效 UTF-8")?;
    let state_dir = state_dir.to_str().context("MCP 状态目录不是有效 UTF-8")?;
    let args = ["--codex-plus-agent-mcp", "--state-dir", state_dir];
    let mut document = toml_edit::DocumentMut::new();
    document["mcp_servers"]["codex_plus_collaboration"]["command"] = toml_edit::value(command);
    let mut arguments = toml_edit::Array::new();
    for argument in args {
        arguments.push(argument);
    }
    document["mcp_servers"]["codex_plus_collaboration"]["args"] = toml_edit::value(arguments);
    let claude =
        json!({"mcpServers": {"codex_plus_collaboration": {"command": command, "args": args}}});
    Ok(json!({
        "command": command,
        "args": args,
        "codexToml": document.to_string(),
        "claudeJson": serde_json::to_string_pretty(&claude)?,
    }))
}

fn mcp_store_from_args(args: &[OsString]) -> anyhow::Result<Store> {
    match args {
        [] => Ok(Store::default()),
        [flag, directory] if flag == "--state-dir" => {
            let directory = PathBuf::from(directory);
            if !directory.is_absolute() {
                bail!("--state-dir 必须是绝对目录");
            }
            Ok(Store::new(directory.join("agent-collaboration.sqlite3")))
        }
        _ => bail!("MCP 模式仅接受可选的 --state-dir <绝对目录>"),
    }
}

pub fn run_mcp(args: &[OsString]) -> anyhow::Result<()> {
    let store = mcp_store_from_args(args)?;
    tauri::async_runtime::block_on(mcp::serve(store))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_configuration_round_trips_spaces_quotes_and_shared_state() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir
            .path()
            .join("Codex++ \"local\".app")
            .join("CodexPlusPlus");
        let state_dir = dir.path().join("collaboration state");
        let config = mcp_configuration(&executable, &state_dir).unwrap();
        let codex: toml_edit::DocumentMut = config["codexToml"].as_str().unwrap().parse().unwrap();
        let server = &codex["mcp_servers"]["codex_plus_collaboration"];
        assert_eq!(server["command"].as_str(), executable.to_str());
        assert_eq!(
            server["args"].as_array().unwrap().get(2).unwrap().as_str(),
            state_dir.to_str()
        );
        let claude: Value = serde_json::from_str(config["claudeJson"].as_str().unwrap()).unwrap();
        assert_eq!(
            claude["mcpServers"]["codex_plus_collaboration"]["args"],
            config["args"]
        );
    }

    #[test]
    fn mcp_mode_rejects_unknown_or_relative_arguments_before_starting() {
        for args in [
            vec!["--state-dir"],
            vec!["--state-dir", "relative"],
            vec!["--execute", "anything"],
        ] {
            let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
            assert!(mcp_store_from_args(&args).is_err());
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(
            mcp_store_from_args(&[
                OsString::from("--state-dir"),
                dir.path().as_os_str().to_owned()
            ])
            .is_ok()
        );
    }

    #[test]
    fn connect_codex_persists_the_same_server_in_managed_and_live_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("codex-home");
        std::fs::create_dir(&home).unwrap();
        let executable = std::env::current_exe().unwrap();
        let state_dir = dir.path().join("state");
        let settings = SettingsStore::new(dir.path().join("settings.json"));
        let mut raw =
            serde_json::to_value(codex_plus_core::settings::BackendSettings::default()).unwrap();
        raw["futurePreference"] = json!({"keep": true});
        std::fs::write(
            dir.path().join("settings.json"),
            serde_json::to_vec(&raw).unwrap(),
        )
        .unwrap();
        connect_codex(&integration::SPEC, &settings, &home, &executable, &state_dir).unwrap();
        let managed = settings.load().unwrap().relay_context_config_contents;
        assert!(managed.contains("codex_plus_collaboration"));
        assert!(managed.contains("--codex-plus-agent-mcp"));
        assert_eq!(
            registration_view(&home, &executable, &state_dir)["status"],
            "connected"
        );
        connect_codex(&integration::SPEC, &settings, &home, &executable, &state_dir).unwrap();
        assert_eq!(
            settings.load().unwrap().relay_context_config_contents,
            managed
        );
        let persisted: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(persisted["futurePreference"], json!({"keep": true}));
    }

    #[test]
    fn refresh_rejects_missing_unowned_and_conflicting_registration_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let state_dir = dir.path().join("state");
        assert!(require_owned_registration(dir.path(), &executable, &state_dir).is_err());
        assert!(!dir.path().join("config.toml").exists());
        for contents in [
            "[mcp_servers.codex_plus_collaboration]\ncommand = 'other'\n",
            "[mcp_servers.codex_plus_collaboration]\ncommand = 'other'\nargs = ['--codex-plus-agent-mcp']\nurl = 'https://example.invalid/mcp'\n",
        ] {
            std::fs::write(dir.path().join("config.toml"), contents).unwrap();
            assert!(require_owned_registration(dir.path(), &executable, &state_dir).is_err());
            assert_eq!(
                std::fs::read_to_string(dir.path().join("config.toml")).unwrap(),
                contents
            );
        }
    }

    #[test]
    fn refresh_accepts_owned_registration_from_a_previous_test_package() {
        let dir = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let previous_state = dir.path().join("previous-state");
        let current_state = dir.path().join("current-state");
        integration::install(dir.path(), &executable, &previous_state).unwrap();
        let contents = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        let status = integration::status(dir.path(), &executable, &current_state).unwrap();
        assert_eq!(status.status, "needs_update");
        assert!(status.owned);
        assert!(!status.registered);
        require_owned_registration(dir.path(), &executable, &current_state).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("config.toml")).unwrap(),
            contents
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_live_publish_restores_only_the_managed_context() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("codex-home");
        std::fs::create_dir(&home).unwrap();
        let settings = SettingsStore::new(dir.path().join("settings.json"));
        let mut before = codex_plus_core::settings::BackendSettings::default();
        before.relay_context_config_contents =
            "[mcp_servers.existing]\ncommand = \"existing-tool\"\n".to_string();
        settings.save(&before).unwrap();
        let original = settings.load().unwrap().relay_context_config_contents;
        let target = dir.path().join("not-a-lock");
        std::fs::write(&target, "leave intact").unwrap();
        std::os::unix::fs::symlink(&target, home.join(".codex-plus-agent-mcp.lock")).unwrap();
        assert!(
            connect_codex(
                &integration::SPEC,
                &settings,
                &home,
                &std::env::current_exe().unwrap(),
                &dir.path().join("state")
            )
            .is_err()
        );
        assert_eq!(
            settings.load().unwrap().relay_context_config_contents,
            original
        );
        assert!(!home.join("config.toml").exists());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "leave intact");
    }
}
