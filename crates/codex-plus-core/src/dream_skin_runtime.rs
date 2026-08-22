use std::path::Path;

use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::Value;

use crate::settings::SettingsStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamSkinState {
    Pass,
    Warning,
    Fail,
    NotRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamSkinCheckLevel {
    Pass,
    Warning,
    Fail,
}

impl DreamSkinCheckLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warning => "warning",
            Self::Fail => "fail",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinCheck {
    pub id: String,
    pub label: String,
    pub level: DreamSkinCheckLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinRuntimeStatus {
    pub state: DreamSkinState,
    pub enabled: bool,
    pub paused: bool,
    pub live_applied: bool,
    pub checks: Vec<DreamSkinCheck>,
}

impl DreamSkinRuntimeStatus {
    pub fn not_running(enabled: bool, paused: bool) -> Self {
        Self {
            state: DreamSkinState::NotRunning,
            enabled,
            paused,
            live_applied: false,
            checks: vec![check(
                "runtime",
                "Codex 运行状态",
                DreamSkinCheckLevel::Warning,
                "未检测到可验证的 Codex CDP renderer。",
            )],
        }
    }

    pub fn pending_restart(enabled: bool, paused: bool) -> Self {
        Self {
            state: DreamSkinState::Warning,
            enabled,
            paused,
            live_applied: false,
            checks: vec![check(
                "runtime",
                "Codex 运行状态",
                DreamSkinCheckLevel::Warning,
                "主题已保存，需要重启 Codex 后完整切换。",
            )],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinVerification {
    pub state: DreamSkinState,
    pub pass: bool,
    pub version: Option<String>,
    pub checks: Vec<DreamSkinCheck>,
    pub screenshot_path: Option<String>,
    pub raw: Value,
}

pub fn macos_arch_name(rust_arch: &str) -> &str {
    match rust_arch {
        "aarch64" => "arm64",
        other => other,
    }
}

pub fn parse_renderer_verification(raw: Value) -> anyhow::Result<DreamSkinVerification> {
    let installed = bool_at(&raw, "/installed");
    let version = raw
        .get("version")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let style = bool_at(&raw, "/stylePresent");
    let chrome = bool_at(&raw, "/chromePresent")
        && raw.get("chromePointerEvents").and_then(Value::as_str) == Some("none");
    let sidebar = bool_at(&raw, "/sidebar/visible");
    let composer = bool_at(&raw, "/composer/visible");
    let no_horizontal_overflow = !bool_at(&raw, "/documentOverflow/x");
    let no_vertical_overflow = !bool_at(&raw, "/documentOverflow/y");
    let home_route = bool_at(&raw, "/homeRoute");
    let home_pass = !home_route
        || (bool_at(&raw, "/homePresent")
            && bool_at(&raw, "/hero/visible")
            && raw
                .get("visibleCardCount")
                .and_then(Value::as_u64)
                .is_some_and(|count| (1..=6).contains(&count)));
    let version_pass = version
        .as_deref()
        .is_some_and(|value| value.starts_with("codex-plus:"));

    let mut checks = vec![
        bool_check(
            "installed",
            "皮肤标记",
            installed,
            "皮肤根标记已安装。",
            "未找到皮肤根标记。",
        ),
        bool_check(
            "version",
            "注入版本",
            version_pass,
            "Codex++ 皮肤版本有效。",
            "注入版本不是 Codex++ Dream Skin。",
        ),
        bool_check(
            "style",
            "皮肤样式",
            style,
            "目标项目样式已安装。",
            "目标项目样式缺失。",
        ),
        bool_check(
            "chrome",
            "装饰层",
            chrome,
            "装饰层存在且不拦截点击。",
            "装饰层缺失或会拦截点击。",
        ),
        bool_check(
            "sidebar",
            "原生侧栏",
            sidebar,
            "原生侧栏可见。",
            "原生侧栏不可见。",
        ),
        bool_check(
            "composer",
            "原生输入框",
            composer,
            "原生输入框可见。",
            "原生输入框不可见。",
        ),
        bool_check(
            "home",
            "首页内容",
            home_pass,
            "首页横幅和建议卡正常。",
            "首页横幅或建议卡不符合目标项目要求。",
        ),
        bool_check(
            "overflow",
            "横向溢出",
            no_horizontal_overflow,
            "页面无横向溢出。",
            "页面存在横向溢出。",
        ),
    ];
    checks.push(check(
        "verticalOverflow",
        "纵向溢出",
        if no_vertical_overflow {
            DreamSkinCheckLevel::Pass
        } else {
            DreamSkinCheckLevel::Warning
        },
        if no_vertical_overflow {
            "页面无纵向文档溢出。"
        } else {
            "页面存在纵向文档滚动，请检查当前路由内容。"
        },
    ));
    let pass = checks
        .iter()
        .all(|item| item.level != DreamSkinCheckLevel::Fail);

    Ok(DreamSkinVerification {
        state: if pass {
            DreamSkinState::Pass
        } else {
            DreamSkinState::Fail
        },
        pass,
        version,
        checks,
        screenshot_path: None,
        raw,
    })
}

pub async fn dream_skin_status(debug_port: u16) -> DreamSkinRuntimeStatus {
    let settings = SettingsStore::default().load().unwrap_or_default();
    if !settings.enhancements_enabled || !settings.codex_app_dream_skin_enabled {
        return DreamSkinRuntimeStatus {
            state: DreamSkinState::Warning,
            enabled: false,
            paused: settings.codex_app_dream_skin_paused,
            live_applied: false,
            checks: vec![check(
                "settings",
                "皮肤设置",
                DreamSkinCheckLevel::Warning,
                "Dream Skin 当前未启用。",
            )],
        };
    }
    if settings.codex_app_dream_skin_paused {
        return DreamSkinRuntimeStatus {
            state: DreamSkinState::Warning,
            enabled: true,
            paused: true,
            live_applied: false,
            checks: vec![check(
                "settings",
                "皮肤设置",
                DreamSkinCheckLevel::Warning,
                "Dream Skin 已暂停，主题配置仍保留。",
            )],
        };
    }

    let mut identity = platform_identity_check(&settings.codex_app_path);
    if identity.level == DreamSkinCheckLevel::Fail {
        return DreamSkinRuntimeStatus {
            state: DreamSkinState::Fail,
            enabled: true,
            paused: false,
            live_applied: false,
            checks: vec![identity],
        };
    }
    let verification = match verify_dream_skin(debug_port, None).await {
        Ok(result) => result,
        Err(_) => return DreamSkinRuntimeStatus::not_running(true, false),
    };
    let state = verification.state;
    let live_applied = verification.pass;
    identity.level = DreamSkinCheckLevel::Pass;
    let mut checks = vec![identity];
    checks.extend(verification.checks);
    DreamSkinRuntimeStatus {
        state,
        enabled: true,
        paused: false,
        live_applied,
        checks,
    }
}

pub async fn apply_dream_skin_live(
    debug_port: u16,
    helper_port: u16,
) -> anyhow::Result<DreamSkinRuntimeStatus> {
    let settings = SettingsStore::default().load()?;
    if !settings.enhancements_enabled || !settings.codex_app_dream_skin_enabled {
        bail!("Dream Skin is not enabled");
    }
    if settings.codex_app_dream_skin_paused {
        bail!("Dream Skin is paused");
    }
    let identity = platform_identity_check(&settings.codex_app_path);
    if identity.level == DreamSkinCheckLevel::Fail {
        bail!("{}", identity.message);
    }
    let target = primary_target(debug_port).await?;
    let websocket = target
        .web_socket_debugger_url
        .as_deref()
        .context("Codex renderer has no WebSocket URL")?;
    let probe = crate::assets::dream_skin_live_update_probe_script();
    let live_probe = crate::bridge::evaluate_script(websocket, &probe)
        .await
        .ok()
        .and_then(|response| {
            response
                .pointer("/result/result/value")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let live_signatures = live_probe.as_deref().and_then(|value| {
        serde_json::from_str::<Value>(value).ok().map(|parsed| {
            (
                parsed
                    .get("artSignature")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                parsed
                    .get("payloadSignature")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        })
    });
    if live_signatures.as_ref().is_some_and(|(_, payload)| {
        payload == &crate::assets::dream_skin_runtime_content_signature(&settings)
    }) {
        return Ok(dream_skin_status(debug_port).await);
    }
    let script = if let Some((live_art_signature, _)) = live_signatures {
        let include_art =
            live_art_signature != crate::assets::dream_skin_art_content_signature(&settings);
        crate::assets::dream_skin_live_update_script(&settings, include_art)
    } else {
        crate::assets::injection_script_with_settings(helper_port, &settings)
    };
    crate::bridge::evaluate_script(websocket, &script).await?;
    Ok(dream_skin_status(debug_port).await)
}

pub async fn pause_dream_skin_live(debug_port: u16) -> anyhow::Result<()> {
    let target = primary_target(debug_port).await?;
    let websocket = target
        .web_socket_debugger_url
        .as_deref()
        .context("Codex renderer has no WebSocket URL")?;
    crate::bridge::evaluate_script(websocket, cleanup_script()).await?;
    Ok(())
}

pub async fn reload_dream_skin_live(debug_port: u16) -> anyhow::Result<()> {
    let target = primary_target(debug_port).await?;
    let websocket = target
        .web_socket_debugger_url
        .as_deref()
        .context("Codex renderer has no WebSocket URL")?;
    crate::bridge::evaluate_script(websocket, reload_script()).await?;
    Ok(())
}

pub async fn restore_dream_skin(debug_port: u16) -> anyhow::Result<()> {
    let live_result = pause_dream_skin_live(debug_port).await;
    crate::dream_skin::sync_default_dream_skin_base_theme(
        false,
        &crate::settings::DreamSkinThemeConfig::default(),
    )?;
    live_result?;
    reload_dream_skin_live(debug_port).await
}

pub async fn verify_dream_skin(
    debug_port: u16,
    screenshot_path: Option<&Path>,
) -> anyhow::Result<DreamSkinVerification> {
    let identity = crate::cdp::browser_identity(debug_port).await?;
    identity.browser_id()?;
    let target = primary_target(debug_port).await?;
    let websocket = target
        .web_socket_debugger_url
        .as_deref()
        .context("Codex renderer has no WebSocket URL")?;
    let response =
        crate::bridge::evaluate_script(websocket, renderer_verification_script()).await?;
    let encoded = response
        .pointer("/result/result/value")
        .and_then(Value::as_str)
        .context("Dream Skin verifier returned no value")?;
    let raw: Value = serde_json::from_str(encoded).context("invalid Dream Skin verifier JSON")?;
    let mut verification = parse_renderer_verification(raw)?;
    if let Some(path) = screenshot_path {
        crate::bridge::capture_page_screenshot(websocket, path).await?;
        verification.screenshot_path = Some(path.to_string_lossy().to_string());
    }
    Ok(verification)
}

pub fn renderer_verification_script() -> &'static str {
    r#"(() => {
  const box = (node) => {
    if (!node) return null;
    const rect = node.getBoundingClientRect();
    const style = getComputedStyle(node);
    return {
      x: Math.round(rect.x), y: Math.round(rect.y),
      width: Math.round(rect.width), height: Math.round(rect.height),
      visible: rect.width > 0 && rect.height > 0 && style.display !== "none" && style.visibility !== "hidden",
    };
  };
  const homeSignal = document.querySelector('[data-testid="home-icon"]') ||
    document.querySelector('[data-feature="game-source"]') ||
    document.querySelector('.group\\/home-suggestions');
  const homeRoute = homeSignal?.closest('[role="main"]') || null;
  const home = document.querySelector('[role="main"].dream-home, [role="main"].dream-skin-home, [role="main"].glass-vision-home');
  const suggestions = home?.querySelector('.group\\/home-suggestions') || null;
  const cards = suggestions ? [...suggestions.querySelectorAll('button')].map(box) : [];
  const chrome = document.getElementById('codex-dream-skin-chrome') ||
    document.getElementById('codex-glass-vision-skin-chrome');
  return JSON.stringify({
    installed: document.documentElement.classList.contains('codex-dream-skin') ||
      document.documentElement.classList.contains('codex-glass-vision-skin'),
    version: window.__CODEX_DREAM_SKIN_STATE__?.version ||
      window.__CODEX_GLASS_VISION_SKIN_STATE__?.version || null,
    stylePresent: Boolean(document.getElementById('codex-dream-skin-style') ||
      document.getElementById('codex-glass-vision-skin-style')),
    chromePresent: Boolean(chrome),
    chromePointerEvents: getComputedStyle(chrome || document.body).pointerEvents,
    homeRoute: Boolean(homeRoute),
    homePresent: Boolean(home),
    hero: box(home?.firstElementChild?.firstElementChild?.firstElementChild),
    visibleCardCount: cards.filter((item) => item?.visible).length,
    projectButton: box(home?.querySelector('.group\\/project-selector > button')),
    composer: box(document.querySelector('.composer-surface-chrome')),
    sidebar: box(document.querySelector('aside.app-shell-left-panel')),
    documentOverflow: {
      x: document.documentElement.scrollWidth > document.documentElement.clientWidth,
      y: document.documentElement.scrollHeight > document.documentElement.clientHeight,
    },
  });
})()"#
}

fn cleanup_script() -> &'static str {
    r#"(() => {
  if (typeof window.__CODEX_PLUS_CLEAR_DREAM_SKIN__ === 'function') {
    window.__CODEX_PLUS_CLEAR_DREAM_SKIN__();
    return true;
  }
  window.__CODEX_DREAM_SKIN_DISABLED__ = true;
  window.__CODEX_GLASS_VISION_SKIN_DISABLED__ = true;
  if (window.__CODEX_DREAM_SKIN_STATE__?.cleanup) {
    window.__CODEX_DREAM_SKIN_STATE__.cleanup();
    return true;
  }
  const root = document.documentElement;
  for (const className of [...(root?.classList || [])]) {
    if (className === 'codex-dream-skin' || className === 'codex-glass-vision-skin' || className.startsWith('codex-theme-')) {
      root?.classList.remove(className);
    }
  }
  document.querySelectorAll('[class]').forEach((node) => {
    for (const className of [...node.classList]) {
      if (/^theme-[a-z0-9-]+-(?:home|home-shell|task|task-shell)$/.test(className) || /^glass-vision-(?:home|home-shell|task|task-shell)$/.test(className)) {
        node.classList.remove(className);
      }
    }
  });
  document.getElementById('codex-dream-skin-style')?.remove();
  document.getElementById('codex-glass-vision-skin-style')?.remove();
  document.getElementById('codex-plus-dream-skin-style')?.remove();
  document.getElementById('codex-dream-skin-chrome')?.remove();
  document.getElementById('codex-glass-vision-skin-chrome')?.remove();
  document.getElementById('codex-theme-chrome')?.remove();
  return true;
})()"#
}

fn reload_script() -> &'static str {
    r#"(() => {
  window.setTimeout(() => window.location.reload(), 350);
  return true;
})()"#
}

async fn primary_target(debug_port: u16) -> anyhow::Result<crate::cdp::CdpTarget> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    crate::cdp::pick_injectable_codex_page_target(&targets)
}

fn platform_identity_check(configured_path: &str) -> DreamSkinCheck {
    let configured = if configured_path.trim().is_empty() {
        None
    } else {
        Some(Path::new(configured_path))
    };
    let Some(app_dir) = crate::app_paths::resolve_codex_app_dir(configured) else {
        return check(
            "appIdentity",
            "官方应用身份",
            DreamSkinCheckLevel::Fail,
            "没有找到可验证的官方 Codex 应用。",
        );
    };
    platform_identity_check_for_dir(&app_dir)
}

#[cfg(windows)]
fn platform_identity_check_for_dir(app_dir: &Path) -> DreamSkinCheck {
    let matched = crate::app_paths::registered_windows_packages()
        .unwrap_or_default()
        .iter()
        .any(|package| {
            windows_app_path_matches_registered_root(app_dir, &package.install_location)
        });
    if matched {
        check(
            "appIdentity",
            "官方应用身份",
            DreamSkinCheckLevel::Pass,
            "Windows 注册包身份与启动路径一致。",
        )
    } else {
        check(
            "appIdentity",
            "官方应用身份",
            DreamSkinCheckLevel::Fail,
            "当前路径不是可验证的官方 Windows 注册包，已拒绝皮肤注入。",
        )
    }
}

pub fn windows_app_path_matches_registered_root(app_dir: &Path, install_location: &Path) -> bool {
    let app_path = normalized_windows_path(app_dir);
    let root = normalized_windows_path(install_location);
    app_path == root || app_path.starts_with(&format!("{root}\\"))
}

fn normalized_windows_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

#[cfg(target_os = "macos")]
fn platform_identity_check_for_dir(app_dir: &Path) -> DreamSkinCheck {
    let verify = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app_dir)
        .output();
    let details = std::process::Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=4"])
        .arg(app_dir)
        .output();
    let architectures = std::process::Command::new("/usr/bin/lipo")
        .args(["-archs"])
        .arg(crate::app_paths::build_codex_executable(app_dir))
        .output();
    let current_arch = macos_arch_name(std::env::consts::ARCH);
    let valid = verify.is_ok_and(|output| output.status.success())
        && details.is_ok_and(|output| {
            String::from_utf8_lossy(&output.stderr).contains("TeamIdentifier=2DC432GLL2")
        })
        && architectures.is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .split_whitespace()
                    .any(|arch| arch == current_arch)
        });
    bool_check(
        "appIdentity",
        "官方应用身份",
        valid,
        "macOS 应用签名和 OpenAI Team ID 有效。",
        "macOS 应用签名或 OpenAI Team ID 无法验证。",
    )
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_identity_check_for_dir(_app_dir: &Path) -> DreamSkinCheck {
    check(
        "appIdentity",
        "官方应用身份",
        DreamSkinCheckLevel::Warning,
        "当前平台没有 Dream Skin 官方应用身份规则。",
    )
}

fn bool_at(value: &Value, pointer: &str) -> bool {
    value.pointer(pointer).and_then(Value::as_bool) == Some(true)
}

fn bool_check(
    id: &str,
    label: &str,
    passed: bool,
    passed_message: &str,
    failed_message: &str,
) -> DreamSkinCheck {
    check(
        id,
        label,
        if passed {
            DreamSkinCheckLevel::Pass
        } else {
            DreamSkinCheckLevel::Fail
        },
        if passed {
            passed_message
        } else {
            failed_message
        },
    )
}

fn check(id: &str, label: &str, level: DreamSkinCheckLevel, message: &str) -> DreamSkinCheck {
    DreamSkinCheck {
        id: id.to_string(),
        label: label.to_string(),
        level,
        message: message.to_string(),
    }
}
