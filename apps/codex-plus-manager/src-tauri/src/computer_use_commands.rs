//! Computer Use 接入命令。
//!
//! 只暴露给管理器前端；不注册到用户脚本路由，也不经过 HTTP 层。
//! 具体行为都在 `codex_plus_core::computer_use` 里，这里只做参数解析与提示文案。

use serde_json::{Value, json};

use crate::commands::{CommandResult, failed, ok};

async fn run_blocking<T, F>(run: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(run)
        .await
        .map_err(|error| format!("任务执行失败：{error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn computer_use_status() -> CommandResult<Value> {
    match run_blocking(codex_plus_core::computer_use::status).await {
        Ok(report) => {
            let message = if report.runtime_found {
                "已找到 Codex 运行时。"
            } else {
                "未找到 ChatGPT.app，Computer Use 暂时不可接入。"
            };
            ok(message, serde_json::to_value(report).unwrap_or_else(|_| json!({})))
        }
        Err(error) => failed(&error, json!({})),
    }
}

/// 前端传来的 target：内置目标的 id，或自定义目标的 `custom-…` id。
enum TargetRef {
    Builtin(codex_plus_core::computer_use::Target),
    Custom(codex_plus_core::computer_use::CustomTarget),
}

impl TargetRef {
    fn resolve(target: &str) -> anyhow::Result<Self> {
        match codex_plus_core::computer_use::Target::parse(target) {
            Ok(parsed) => Ok(Self::Builtin(parsed)),
            Err(error) => codex_plus_core::computer_use::custom_target(target)
                .map(Self::Custom)
                .map_err(|_| error),
        }
    }

    /// Claude Code 回合钩子要写的 settings.json；不是 Claude Code 时为 `None`。
    fn hooks_path(&self) -> Option<std::path::PathBuf> {
        match self {
            Self::Builtin(codex_plus_core::computer_use::Target::ClaudeCode) => {
                Some(codex_plus_core::claude_hooks::claude_settings_path())
            }
            Self::Builtin(_) => None,
            Self::Custom(custom) => custom.claude_settings_path(),
        }
    }
}

#[tauri::command]
pub async fn computer_use_install(target: String) -> CommandResult<Value> {
    let parsed = match TargetRef::resolve(&target) {
        Ok(target) => target,
        Err(error) => return failed(&error.to_string(), json!({})),
    };
    let hooks_path = parsed.hooks_path();
    let outcome = match parsed {
        TargetRef::Builtin(builtin) => {
            run_blocking(move || codex_plus_core::computer_use::install(builtin)).await
        }
        TargetRef::Custom(custom) => {
            run_blocking(move || codex_plus_core::computer_use::install_custom(&custom.id)).await
        }
    };
    match outcome {
        Ok(receipt) => {
            // 只有 Claude Code 有 hooks。装完 MCP 顺手挂上回合结束钩子，
            // 否则浏览器标签不会在回合结束时清理（见 claude_hooks 模块注释）。
            let hooks = hooks_path
                .and_then(|path| codex_plus_core::claude_hooks::install_at(&path).ok());
            let message = if receipt.changed {
                "已接入；重启对应客户端后生效。"
            } else {
                "已是接入状态，无需改动。"
            };
            // 前端的 `CuMutation` 直接读 payload 顶层的 `changed` / `backupPath`，
            // 所以钩子信息是追加字段，不能把 receipt 再包一层。
            let mut payload = serde_json::to_value(receipt).unwrap_or_else(|_| json!({}));
            if let Some(object) = payload.as_object_mut() {
                object.insert("hooks".into(), json!(hooks));
            }
            ok(message, payload)
        }
        Err(error) => failed(&error, json!({})),
    }
}

#[tauri::command]
pub async fn computer_use_uninstall(target: String) -> CommandResult<Value> {
    let parsed = match TargetRef::resolve(&target) {
        Ok(target) => target,
        Err(error) => return failed(&error.to_string(), json!({})),
    };
    let hooks_path = parsed.hooks_path();
    let outcome = match parsed {
        TargetRef::Builtin(builtin) => {
            run_blocking(move || codex_plus_core::computer_use::uninstall(builtin)).await
        }
        TargetRef::Custom(custom) => {
            run_blocking(move || codex_plus_core::computer_use::uninstall_custom(&custom.id)).await
        }
    };
    match outcome {
        Ok(receipt) => {
            // 撤掉接入时同步摘掉回合结束钩子，别在用户不用的客户端里留一条空命令。
            let hooks = hooks_path
                .and_then(|path| codex_plus_core::claude_hooks::uninstall_at(&path).ok());
            let message = if receipt.changed {
                "已移除接入；原配置已备份。"
            } else {
                "本来就没有接入，未改动配置。"
            };
            let mut payload = serde_json::to_value(receipt).unwrap_or_else(|_| json!({}));
            if let Some(object) = payload.as_object_mut() {
                object.insert("hooks".into(), json!(hooks));
            }
            ok(message, payload)
        }
        Err(error) => failed(&error, json!({})),
    }
}

/// 新增自定义接入目标（只登记，不写客户端配置；接入仍由用户点「接入」触发）。
#[tauri::command]
pub async fn computer_use_custom_add(
    kind: String,
    path: String,
    name: Option<String>,
) -> CommandResult<Value> {
    let kind = match codex_plus_core::computer_use::Target::parse(&kind) {
        Ok(kind) => kind,
        Err(error) => return failed(&error.to_string(), json!({})),
    };
    let name = name.unwrap_or_default();
    let outcome = run_blocking(move || {
        codex_plus_core::computer_use::add_custom_target(kind, &path, &name)?;
        codex_plus_core::computer_use::status()
    })
    .await;
    match outcome {
        Ok(report) => ok(
            "已添加自定义目标；点「接入」写入配置。",
            serde_json::to_value(report).unwrap_or_else(|_| json!({})),
        ),
        Err(error) => failed(&error, json!({})),
    }
}

/// 给自定义目标改名：只改界面显示名，不动客户端配置。`name` 为空恢复默认显示。
#[tauri::command]
pub async fn computer_use_custom_rename(id: String, name: String) -> CommandResult<Value> {
    let outcome =
        run_blocking(move || codex_plus_core::computer_use::rename_custom_target(&id, &name)).await;
    match outcome {
        Ok(report) => ok(
            "已更新名称。",
            serde_json::to_value(report).unwrap_or_else(|_| json!({})),
        ),
        Err(error) => failed(&error, json!({})),
    }
}

/// 删除自定义目标：先撤掉我们写入的条目与回合钩子，再从列表里去掉。
#[tauri::command]
pub async fn computer_use_custom_remove(id: String) -> CommandResult<Value> {
    let outcome = run_blocking(move || {
        let custom = codex_plus_core::computer_use::custom_target(&id)?;
        if let Some(path) = custom.claude_settings_path() {
            codex_plus_core::claude_hooks::uninstall_at(&path)?;
        }
        codex_plus_core::computer_use::remove_custom_target(&id)
    })
    .await;
    match outcome {
        Ok(report) => ok(
            "已删除自定义目标，并移除了写入它配置里的条目。",
            serde_json::to_value(report).unwrap_or_else(|_| json!({})),
        ),
        Err(error) => failed(&error, json!({})),
    }
}

#[tauri::command]
pub async fn computer_use_check() -> CommandResult<Value> {
    match run_blocking(|| Ok(codex_plus_core::computer_use::check())).await {
        Ok(report) => {
            let message = if report.ok {
                format!(
                    "自检通过：识别到 {} 个应用，用时 {} ms。",
                    report.app_count.unwrap_or(0),
                    report.duration_ms
                )
            } else {
                report
                    .failure_detail
                    .clone()
                    .unwrap_or_else(|| "自检失败。".to_string())
            };
            ok(
                &message,
                serde_json::to_value(report).unwrap_or_else(|_| json!({})),
            )
        }
        Err(error) => failed(&error, json!({})),
    }
}

/// 打开 ChatGPT.app。运行时缺失时给用户一个明确的下一步动作。
#[tauri::command]
pub async fn computer_use_open_app() -> CommandResult<Value> {
    let result = run_blocking(|| -> anyhow::Result<String> {
        let runtime = codex_plus_core::computer_use::locate_runtime()?
            .ok_or_else(|| anyhow::anyhow!("未找到 ChatGPT.app"))?;
        Ok(runtime.app_path)
    })
    .await;
    match result {
        Ok(app_path) => {
            let status = std::process::Command::new("open")
                .arg(&app_path)
                .status();
            match status {
                Ok(status) if status.success() => ok(
                    "已打开 ChatGPT；请确认 Computer Use 的辅助功能与屏幕录制权限已授予。",
                    json!({ "appPath": app_path }),
                ),
                Ok(status) => failed(
                    &format!("打开 ChatGPT 失败（退出码 {:?}）", status.code()),
                    json!({ "appPath": app_path }),
                ),
                Err(error) => failed(&format!("打开 ChatGPT 失败：{error}"), json!({})),
            }
        }
        Err(error) => failed(&error, json!({})),
    }
}

/// 检测 Codex 浏览器扩展是否已在 Chrome 里安装。
///
/// 判断依据：native messaging host 配置文件是否存在，以及 Chrome 扩展目录是否有
/// 对应扩展 ID 的目录。只检测状态，不做任何写入。
#[tauri::command]
pub async fn computer_use_browser_extension_status() -> CommandResult<Value> {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    // Codex 用的两个扩展 ID（生产版 + 另一发布渠道）。
    const EXT_IDS: &[&str] = &[
        "hehggadaopoacecdllhhajmbjkdcmajg",
        "odlomjlbamekndcpllcnffbgeohgkmjh",
    ];
    // native messaging host 文件：有它说明 Codex 已经注册过了。
    let host_json = home
        .join("Library/Application Support/Google/Chrome/NativeMessagingHosts")
        .join("com.openai.codexextension.json");
    let host_registered = host_json.is_file();
    // Chrome 扩展目录：有任意一个 ID 目录就视为已安装。
    let ext_base = home.join("Library/Application Support/Google/Chrome/Default/Extensions");
    let ext_installed = EXT_IDS
        .iter()
        .any(|id| ext_base.join(id).is_dir());
    let installed = host_registered || ext_installed;
    ok(
        if installed { "Chrome 浏览器扩展已安装。" } else { "Chrome 浏览器扩展未安装。" },
        json!({
            "installed": installed,
            "hostRegistered": host_registered,
            "extensionInstalled": ext_installed,
            "storeUrl": "https://chromewebstore.google.com/detail/chatgpt/hehggadaopoacecdllhhajmbjkdcmajg",
        }),
    )
}

/// 打开 Chrome Web Store 里的 ChatGPT 扩展页，让用户点「添加到 Chrome」。
#[tauri::command]
pub async fn computer_use_open_extension_store() -> CommandResult<Value> {
    let url = "https://chromewebstore.google.com/detail/chatgpt/hehggadaopoacecdllhhajmbjkdcmajg";
    #[cfg(windows)]
    let result = codex_plus_core::windows_open_url(url).map_err(|e| e.to_string());
    #[cfg(not(windows))]
    let result = std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string());
    match result {
        Ok(()) => ok("已打开 Chrome Web Store 扩展页。", json!({ "url": url })),
        Err(error) => failed(&format!("打开扩展页失败：{error}"), json!({ "url": url })),
    }
}

// ---------------------------------------------------------------------------
// 审批
// ---------------------------------------------------------------------------

/// 当前审批策略与已记忆的应用。
///
/// `auto` = 全部放行（只留痕）；`claude` = 弹卡确认（转给当前连接的 MCP 客户端，同 Codex）。
#[tauri::command]
pub async fn computer_use_approval_get() -> CommandResult<Value> {
    let mode = codex_plus_core::computer_use_approval::ApprovalMode::current();
    ok(
        "已读取审批策略。",
        json!({
            "mode": mode.id(),
            // 操作方式：tools（离散工具）/ js（cua_repl）。
            "cuMode": codex_plus_core::computer_use_proxy::CuMode::current().id(),
            "pip": codex_plus_core::settings::SettingsStore::default()
                .load()
                .map(|settings| settings.computer_use_pip)
                .unwrap_or(true),
            "approvals": codex_plus_core::computer_use_approval::ApprovalStore::read_all(),
            "lastClient": codex_plus_core::computer_use_approval::last_client(),
            // 浏览器标签的清理靠 Claude Code 的 Stop 钩子；装上没有要在页面上说清楚。
            "turnHooks": {
                "installed": codex_plus_core::claude_hooks::is_installed(),
                "events": codex_plus_core::claude_hooks::installed_events(),
                "settings": codex_plus_core::claude_hooks::claude_settings_path().display().to_string(),
            },
        }),
    )
}

/// 切换审批策略。写进 `settings.json` 的 `computerUseApproval`。
///
/// 只改这一个键：设置里还有几十个互不相关的字段，整份回写会把并发改动冲掉。
#[tauri::command]
pub async fn computer_use_approval_set(mode: String) -> CommandResult<Value> {
    if !matches!(mode.as_str(), "auto" | "claude") {
        return failed("审批策略只能是 auto 或 claude", json!({}));
    }
    match run_blocking(move || -> anyhow::Result<()> {
        let store = codex_plus_core::settings::SettingsStore::default();
        let mut settings = store.load()?;
        settings.computer_use_approval = mode;
        store.save(&settings)?;
        Ok(())
    })
    .await
    {
        Ok(()) => ok("审批策略已保存；对新开的客户端会话生效。", json!({})),
        Err(error) => failed(&error, json!({})),
    }
}

/// 切换操作方式：`tools`（离散工具）或 `js`（cua_repl，和 Codex 一致）。
/// 写进 `settings.json` 的 `computerUseMode`，只改这一个键；MCP 进程启动时读取。
#[tauri::command]
pub async fn computer_use_mode_set(mode: String) -> CommandResult<Value> {
    if !matches!(mode.as_str(), "tools" | "js") {
        return failed("操作方式只能是 tools 或 js", json!({}));
    }
    match run_blocking(move || -> anyhow::Result<()> {
        let store = codex_plus_core::settings::SettingsStore::default();
        let mut settings = store.load()?;
        settings.computer_use_mode = mode;
        store.save(&settings)?;
        Ok(())
    })
    .await
    {
        Ok(()) => ok("操作方式已保存；对新开的客户端会话生效。", json!({})),
        Err(error) => failed(&error, json!({})),
    }
}

/// 开关「操作时显示悬浮预览」。写进 `settings.json` 的 `computerUsePip`，只改这一个键。
#[tauri::command]
pub async fn computer_use_pip_set(enabled: bool) -> CommandResult<Value> {
    match run_blocking(move || -> anyhow::Result<()> {
        let store = codex_plus_core::settings::SettingsStore::default();
        let mut settings = store.load()?;
        settings.computer_use_pip = enabled;
        store.save(&settings)?;
        Ok(())
    })
    .await
    {
        Ok(()) => ok(
            if enabled { "已开启悬浮预览。" } else { "已关闭悬浮预览。" },
            json!({}),
        ),
        Err(error) => failed(&error, json!({})),
    }
}

/// 撤销某个应用的「始终允许」：Computer Use 自己的记录（与 Codex 共用）和旧版文件都删。
#[tauri::command]
pub async fn computer_use_approval_forget(app: String) -> CommandResult<Value> {
    match run_blocking(move || codex_plus_core::computer_use_approval::ApprovalStore::forget(&app))
        .await
    {
        Ok(true) => ok("已撤销该应用的审批记忆。", json!({})),
        Ok(false) => ok("没有找到该应用的审批记录。", json!({})),
        Err(error) => failed(&error, json!({})),
    }
}

/// 清空全部审批记忆。
#[tauri::command]
pub async fn computer_use_approval_clear() -> CommandResult<Value> {
    match run_blocking(codex_plus_core::computer_use_approval::ApprovalStore::clear).await {
        Ok(()) => ok("已清空全部审批记忆。", json!({})),
        Err(error) => failed(&error, json!({})),
    }
}

/// 预览帧事件名；前端 `listen` 它来更新画面。
pub const PREVIEW_FRAME_EVENT: &str = "cu-preview-frame";
/// 预览结束事件名（窗口关闭、取流失败、主动停止）。
pub const PREVIEW_END_EVENT: &str = "cu-preview-end";

/// 当前预览会话。同一时刻只允许一个。
static PREVIEW: std::sync::Mutex<Option<codex_plus_core::window_capture::PreviewHandle>> =
    std::sync::Mutex::new(None);

/// 申请「屏幕录制」权限（没授权时弹系统对话框）。
#[tauri::command]
pub async fn computer_use_preview_request_permission() -> CommandResult<Value> {
    match run_blocking(move || -> anyhow::Result<bool> {
        Ok(codex_plus_core::window_capture::request_permission())
    })
    .await
    {
        Ok(true) => ok("已获得屏幕录制权限。", json!({ "granted": true })),
        Ok(false) => failed(
            "还没有屏幕录制权限：在 系统设置 → 隐私与安全性 → 屏幕录制 里勾选 Codex++，然后重启应用。",
            json!({ "granted": false }),
        ),
        Err(error) => failed(&error, json!({ "granted": false })),
    }
}

/// 列出可预览的窗口。
#[tauri::command]
pub async fn computer_use_preview_windows() -> CommandResult<Value> {
    let permitted = codex_plus_core::window_capture::has_permission();
    match run_blocking(codex_plus_core::window_capture::list_windows).await {
        Ok(windows) => {
            let message = if !permitted {
                "还没有屏幕录制权限，先去系统设置里授权（点「申请权限」可直接跳转）。"
            } else if windows.is_empty() {
                "没有可预览的窗口。"
            } else {
                "已列出可预览的窗口。"
            };
            // CommandResult 用 #[serde(flatten)] 展开载荷，只能是对象不能是数组
            ok(
                message,
                json!({
                    "windows": serde_json::to_value(windows).unwrap_or_else(|_| json!([])),
                    "permitted": permitted,
                }),
            )
        }
        Err(error) => failed(&error, json!({ "windows": [], "permitted": permitted })),
    }
}

/// 开始预览某个窗口。帧以 base64 data URL 通过事件推给前端。
#[tauri::command]
pub async fn computer_use_preview_start(
    app: tauri::AppHandle,
    window_id: u32,
    scale: Option<f64>,
    max_fps: Option<f64>,
    mode: Option<String>,
) -> CommandResult<Value> {
    use base64::Engine as _;
    use tauri::Emitter as _;

    stop_preview();
    let scale = scale.unwrap_or(0.5);
    let max_fps = max_fps.unwrap_or(12.0);
    let capture_mode = match mode.as_deref() {
        Some("stream") => codex_plus_core::window_capture::CaptureMode::Stream,
        _ => codex_plus_core::window_capture::CaptureMode::Screenshot,
    };
    let frame_app = app.clone();
    let seq = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    let handle = match codex_plus_core::window_capture::start_preview(
        window_id,
        scale,
        max_fps,
        capture_mode,
        move |jpeg| {
            let n = seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let encoded = base64::engine::general_purpose::STANDARD.encode(&jpeg);
            let _ = frame_app.emit(
                PREVIEW_FRAME_EVENT,
                json!({ "seq": n, "data": format!("data:image/jpeg;base64,{encoded}") }),
            );
        },
    ) {
        Ok(handle) => handle,
        Err(error) => return failed(&error.to_string(), json!({})),
    };

    // 抓取线程因窗口关闭等原因自行退出时，把原因转达给前端
    let watch_app = app.clone();
    let watch_error = handle.error_slot();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let reason = watch_error.lock().ok().and_then(|e| e.clone());
            if let Some(reason) = reason {
                let _ = watch_app.emit(PREVIEW_END_EVENT, json!({ "reason": reason }));
                return;
            }
            // 预览已被换掉或主动停掉时，错误槽不会再有变化，别常驻
            if PREVIEW.lock().map(|slot| slot.is_none()).unwrap_or(true) {
                return;
            }
        }
    });

    let mut slot = PREVIEW.lock().unwrap_or_else(|e| e.into_inner());
    *slot = Some(handle);
    ok(
        "预览已开始。",
        json!({ "event": PREVIEW_FRAME_EVENT, "scale": scale, "maxFps": max_fps, "mode": format!("{:?}", capture_mode).to_lowercase() }),
    )
}

/// 停止预览。
#[tauri::command]
pub async fn computer_use_preview_stop() -> CommandResult<Value> {
    stop_preview();
    ok("预览已停止。", json!({}))
}

fn stop_preview() {
    let handle = PREVIEW
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(handle) = handle {
        handle.stop();
    }
}
