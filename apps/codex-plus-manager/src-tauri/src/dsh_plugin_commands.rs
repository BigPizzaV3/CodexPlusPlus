//! 管理器「导入 dsh 插件」入口：导入、启停、配置、移除，以及把 dsh 插件 MCP 接入 Codex。
//! 插件的系统提示词与事件监听器经配套拓展包（网关中间件 + Codex 钩子）生效，每次变更后同步。

use std::path::Path;

use anyhow::{Context, bail};
use codex_plus_core::dsh_plugins::{self, DshPlugins, Toolchain};
use codex_plus_core::settings::SettingsStore;
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};

#[tauri::command]
pub async fn dsh_plugins_request(action: String, payload: Option<Value>) -> Result<Value, String> {
    // 浏览插件市场索引：只下载、解析，不改任何状态；选中后走普通的「导入」。
    if action == "browse_index" {
        let url = payload.as_ref().and_then(|payload| payload.get("url")).and_then(Value::as_str).unwrap_or_default().to_string();
        let entries = dsh_plugins::fetch_market_index(&url).await.map_err(|error| format!("{error:#}"))?;
        return Ok(json!({ "entries": entries }));
    }
    tauri::async_runtime::spawn_blocking(move || handle(&action, payload.unwrap_or(Value::Null)))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))
}

fn handle(action: &str, payload: Value) -> anyhow::Result<Value> {
    let state_dir = codex_plus_core::paths::default_app_state_dir();
    let plugins = DshPlugins::new(&state_dir);
    let executable = std::env::current_exe().context("无法定位 Codex++ 程序")?;
    let home = codex_plus_core::codex_home::default_codex_home_dir();
    let manager = crate::commands::default_user_script_manager();
    let mut message = String::new();
    // 需要同步配套拓展包的插件 id。
    let mut companion: Option<String> = None;
    match action {
        "snapshot" => {}
        "import" => {
            let source = dsh_plugins::parse_source(text(&payload, "source")?)?;
            let toolchain = Toolchain::detect();
            let record = plugins.import(&source, &toolchain)?;
            let skills = import_skills(&plugins, &record.id, &record.report);
            let tools = dsh_plugins::report_tool_count(&record.report);
            message = format!("已导入 {}：{tools} 个工具", record.name);
            match skills {
                Ok(ids) if !ids.is_empty() => {
                    message.push_str(&format!("，{} 个 skill 已导入并启用到 Codex", ids.len()));
                }
                Ok(_) => {}
                Err(error) => message.push_str(&format!("；skill 导入失败：{error:#}")),
            }
            if dsh_plugins::mcp_servers_json(&record.report).is_some() {
                message.push_str("；包里有 MCP 配置，可复制后在 MCP 设置里导入");
            }
            message.push('。');
            companion = Some(record.id);
        }
        "reinspect" => {
            let id = text(&payload, "id")?;
            plugins.reinspect(id, &Toolchain::detect())?;
            message = "已重新分析。".to_string();
            companion = Some(id.to_string());
        }
        "set_enabled" => {
            let id = text(&payload, "id")?;
            plugins.set_enabled(id, flag(&payload, "enabled")?)?;
            message = "已保存，新开 Codex 对话后生效。".to_string();
            companion = Some(id.to_string());
        }
        "set_row_enabled" => {
            let id = text(&payload, "id")?;
            plugins.set_row_enabled(id, text(&payload, "rowId")?, flag(&payload, "enabled")?)?;
            plugins.reinspect(id, &Toolchain::detect())?;
            message = "已保存，新开 Codex 对话后生效。".to_string();
            companion = Some(id.to_string());
        }
        "set_config" => {
            let id = text(&payload, "id")?;
            let config = payload.get("config").cloned();
            plugins.set_row_config(id, text(&payload, "rowId")?, config)?;
            plugins.reinspect(id, &Toolchain::detect())?;
            message = "配置已保存并重新分析。".to_string();
            companion = Some(id.to_string());
        }
        "import_skills" => {
            let id = text(&payload, "id")?;
            let registry = plugins.load()?;
            let record = registry
                .plugins
                .iter()
                .find(|plugin| plugin.id == id)
                .context("没有这个 dsh 插件")?;
            let ids = import_skills(&plugins, id, &record.report)?;
            message = format!("已导入 {} 个 skill 并启用到 Codex。", ids.len());
        }
        "remove" => {
            let record = plugins.remove(text(&payload, "id")?)?;
            message = format!("已移除 {}。", record.name);
            if !record.imported_skills.is_empty() {
                message.push_str("它导入的 skill 保留在 Skills 页，需要时在那里停用。");
            }
            match plugins.remove_companion(&manager, &record.id) {
                Ok(true) => {
                    sync_hooks(&manager, &mut message);
                }
                Ok(false) => {}
                Err(error) => message.push_str(&format!("配套拓展包未能移除：{error:#}")),
            }
        }
        "connect_codex" => {
            crate::agent_collaboration_commands::connect_codex(
                &dsh_plugins::SPEC,
                &SettingsStore::default(),
                &home,
                &executable,
                &state_dir,
            )?;
            message = dsh_plugins::status(&home, &executable, &state_dir)?.message;
        }
        _ => bail!("不支持的 dsh 插件操作：{action}"),
    }
    if let Some(id) = companion {
        match plugins.sync_companion(&manager, &id) {
            Ok(status) => {
                if action == "import" && !status.is_null() {
                    message.push_str(&companion_message(&status));
                }
                sync_hooks(&manager, &mut message);
            }
            Err(error) => message.push_str(&format!("配套拓展包未能更新：{error:#}")),
        }
    }
    snapshot(&plugins, &manager, &home, &executable, &state_dir, message)
}

/// 导入时说明映射到了哪里。
fn companion_message(status: &Value) -> String {
    let mut parts = Vec::new();
    if status["instructions"] == true {
        parts.push("系统提示词经网关追加到 instructions".to_string());
    }
    let hooks: Vec<&str> = status["hooks"]
        .as_array()
        .map(|hooks| hooks.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !hooks.is_empty() {
        parts.push(format!("事件转成 Codex 钩子 {}", hooks.join("、")));
    }
    format!("已生成配套拓展「{}」：{}。", status["id"].as_str().unwrap_or_default(), parts.join("；"))
}

/// 让 Codex 配置里的拓展钩子分发条目与启用中的拓展一致；新增条目时提醒用户去信任。
fn sync_hooks(manager: &UserScriptManager, message: &mut String) {
    let hooks = codex_plus_core::extension_hooks::sync_default(manager);
    if let Some(error) = hooks.get("error").and_then(Value::as_str) {
        message.push_str(&format!("Codex 钩子配置未能同步：{error}"));
    } else if hooks["added"].as_array().is_some_and(|added| !added.is_empty()) {
        message.push_str("Codex 配置里新增了「Codex++ 拓展钩子」，请在 Codex 的 Hooks 设置里信任一次，新开对话生效。");
    }
}

fn text<'a>(payload: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("缺少参数 {key}"))
}

fn flag(payload: &Value, key: &str) -> anyhow::Result<bool> {
    payload
        .get(key)
        .and_then(Value::as_bool)
        .with_context(|| format!("缺少参数 {key}"))
}

/// 把报告里的 skill 目录导入 Codex++ Skills（同名的旧目录会进 Skills 备份），并启用到 Codex。
fn import_skills(plugins: &DshPlugins, id: &str, report: &Value) -> anyhow::Result<Vec<String>> {
    let manager = crate::commands::default_skills_manager();
    let mut imported = Vec::new();
    let mut errors = Vec::new();
    for directory in dsh_plugins::report_skill_dirs(report) {
        let candidates = match codex_plus_core::skills::local_skill_import_candidates(&directory) {
            Ok(candidates) => candidates,
            Err(error) => {
                errors.push(format!("{}：{error}", directory.display()));
                continue;
            }
        };
        for (skill_id, path) in candidates {
            let result = manager
                .import_from_local_directory(&skill_id, &path)
                .and_then(|_| manager.set_enabled(&skill_id, true));
            match result {
                Ok(()) => imported.push(skill_id),
                Err(error) => errors.push(format!("{skill_id}：{error}")),
            }
        }
    }
    if !imported.is_empty() {
        plugins.set_imported_skills(id, imported.clone())?;
    }
    if !errors.is_empty() {
        bail!(errors.join("；"));
    }
    Ok(imported)
}

fn snapshot(
    plugins: &DshPlugins,
    manager: &UserScriptManager,
    home: &Path,
    executable: &Path,
    state_dir: &Path,
    message: String,
) -> anyhow::Result<Value> {
    let registry = plugins.load()?;
    let toolchain = Toolchain::detect();
    let items: Vec<Value> = registry
        .plugins
        .iter()
        .map(|record| {
            let mut item = serde_json::to_value(record).unwrap_or(Value::Null);
            item["toolCount"] = json!(dsh_plugins::report_tool_count(&record.report));
            item["mcpJson"] = dsh_plugins::mcp_servers_json(&record.report)
                .map(|value| json!(serde_json::to_string_pretty(&value).unwrap_or_default()))
                .unwrap_or(Value::Null);
            item["companion"] = plugins.companion_status(manager, record);
            item
        })
        .collect();
    let integration = match dsh_plugins::status(home, executable, state_dir) {
        Ok(status) => json!({
            "status": status.status,
            "registered": status.registered,
            "message": status.message,
            "configPath": status.config_path,
        }),
        Err(error) => json!({"status": "error", "registered": false, "message": error.to_string()}),
    };
    Ok(json!({
        "message": message,
        "plugins": items,
        "integration": integration,
        "toolchain": toolchain,
        "pluginDir": plugins.root().to_string_lossy(),
    }))
}
