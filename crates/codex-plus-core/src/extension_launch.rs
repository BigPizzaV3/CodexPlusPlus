//! 拓展在 Codex 页面之外的两个时机：
//!
//! - **页面最早期脚本 `preload`**：经 CDP 的 `Page.addScriptToEvaluateOnNewDocument` 登记，每次 Codex 页面
//!   新建文档时在 Codex 自己的脚本之前执行（与 Codex++ 的原生模块补丁、强制中文同一时机）。
//!   `reloadOnLaunch: true` 时启动 Codex 后整页重载一次，让脚本赶在第一份文档的 Codex 脚本之前。
//! - **启动前后 `launch`**：`args` 追加到 Codex 的启动参数；`before` 在启动 Codex 之前、`after` 在 Codex
//!   退出之后各跑一次（Node，默认导出函数，拿到 `{ event, codexHome, stateDir, appDir, config }`）。
//!
//! 内置的「原生 Fast 解锁」「强制中文」「纯 API 禁用 Work 视图」「原生浏览器请求标识兼容」等就是在这两个时机
//! 生效的；拓展用 `replaces` 顶替它们后，用这里的接口写出等价功能。
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};

/// 页面最早期脚本（codexpp.json 的 `preload`）。
#[derive(Debug, Clone, Default)]
pub struct ExtensionPreload {
    pub scripts: Vec<PathBuf>,
    pub reload_on_launch: bool,
}

/// 启动前后（codexpp.json 的 `launch`）。
#[derive(Debug, Clone, Default)]
pub struct ExtensionLaunch {
    pub args: Vec<String>,
    pub before: Option<PathBuf>,
    pub after: Option<PathBuf>,
    pub timeout_secs: u64,
}

impl ExtensionLaunch {
    pub fn is_empty(&self) -> bool {
        self.args.is_empty() && self.before.is_none() && self.after.is_none()
    }
}

/// 单个 preload 脚本的大小上限：每份新文档都要执行，不能太大。
const PRELOAD_MAX_BYTES: u64 = 512 * 1024;
/// 启动参数最多几个、每个多长。
const LAUNCH_MAX_ARGS: usize = 16;
const LAUNCH_ARG_MAX_LEN: usize = 256;
/// 启动前后脚本的超时（秒）：挡在 Codex 启动之前，不能太久。
pub const LAUNCH_MAX_TIMEOUT_SECS: u64 = 30;
const LAUNCH_DEFAULT_TIMEOUT_SECS: u64 = 10;
/// 这些参数由 Codex++ 自己管（调试端口、用户数据目录、远程调试来源），拓展不能改。
const BLOCKED_ARG_PREFIXES: &[&str] = &[
    "--remote-debugging-port",
    "--remote-debugging-pipe",
    "--remote-debugging-address",
    "--remote-allow-origins",
    "--user-data-dir",
    "--inspect",
];

/// 解析 `preload`：字符串、字符串数组，或 `{ scripts: [...], reloadOnLaunch }`。
pub fn parse_preload(root: &Path, value: Option<&Value>) -> anyhow::Result<ExtensionPreload> {
    let Some(value) = value else {
        return Ok(ExtensionPreload::default());
    };
    let (entries, reload_on_launch) = match value {
        Value::String(entry) => (vec![entry.clone()], false),
        Value::Array(items) => (string_list(items).context("preload 只能是包内脚本路径的字符串数组")?, false),
        Value::Object(map) => {
            let entries = match map.get("scripts") {
                Some(Value::String(entry)) => vec![entry.clone()],
                Some(Value::Array(items)) => string_list(items).context("preload.scripts 只能是字符串数组")?,
                _ => anyhow::bail!("preload.scripts 必填：包内脚本路径"),
            };
            let reload = match map.get("reloadOnLaunch") {
                None => false,
                Some(Value::Bool(flag)) => *flag,
                Some(_) => anyhow::bail!("preload.reloadOnLaunch 只能是 true / false"),
            };
            (entries, reload)
        }
        _ => anyhow::bail!("preload 只能是脚本路径、路径数组或 {{ \"scripts\": [...], \"reloadOnLaunch\": true }}"),
    };
    let mut scripts = Vec::new();
    for entry in entries {
        let path = crate::user_scripts::resolve_package_path(root, &entry)?;
        if !path.is_file() {
            anyhow::bail!("preload 脚本 {entry} 必须是文件");
        }
        if std::fs::metadata(&path)?.len() > PRELOAD_MAX_BYTES {
            anyhow::bail!("preload 脚本 {entry} 超过 512 KiB");
        }
        scripts.push(path);
    }
    Ok(ExtensionPreload { scripts, reload_on_launch })
}

/// 解析 `launch: { args, before, after, timeout }`。
pub fn parse_launch(root: &Path, value: Option<&Value>) -> anyhow::Result<ExtensionLaunch> {
    let Some(value) = value else {
        return Ok(ExtensionLaunch::default());
    };
    let Value::Object(map) = value else {
        anyhow::bail!("launch 必须是对象：{{ \"args\": [\"--flag\"], \"before\": \"launch/before.mjs\", \"after\": \"launch/after.mjs\" }}");
    };
    let args = match map.get("args") {
        None => Vec::new(),
        Some(Value::Array(items)) => string_list(items).context("launch.args 只能是字符串数组")?,
        Some(_) => anyhow::bail!("launch.args 只能是字符串数组"),
    };
    if args.len() > LAUNCH_MAX_ARGS {
        anyhow::bail!("launch.args 最多 {LAUNCH_MAX_ARGS} 个");
    }
    for arg in &args {
        validate_launch_arg(arg)?;
    }
    let script = |key: &str| -> anyhow::Result<Option<PathBuf>> {
        match map.get(key) {
            None => Ok(None),
            Some(Value::String(entry)) => {
                let path = crate::user_scripts::resolve_package_path(root, entry)?;
                if !path.is_file() {
                    anyhow::bail!("launch.{key} 必须是包内的脚本文件");
                }
                Ok(Some(path))
            }
            Some(_) => anyhow::bail!("launch.{key} 只能是包内脚本路径"),
        }
    };
    let timeout_secs = match map.get("timeout") {
        None => LAUNCH_DEFAULT_TIMEOUT_SECS,
        Some(value) => value
            .as_u64()
            .filter(|secs| (1..=LAUNCH_MAX_TIMEOUT_SECS).contains(secs))
            .with_context(|| format!("launch.timeout 是 1~{LAUNCH_MAX_TIMEOUT_SECS} 的整数秒"))?,
    };
    Ok(ExtensionLaunch { args, before: script("before")?, after: script("after")?, timeout_secs })
}

/// 启动参数只能是 `--名字` 或 `--名字=值`，不能带换行，不能改 Codex++ 自己管的参数。
fn validate_launch_arg(arg: &str) -> anyhow::Result<()> {
    let name = arg.split('=').next().unwrap_or_default();
    let valid_name = name.len() > 2
        && name.starts_with("--")
        && name[2..].chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if !valid_name || arg.len() > LAUNCH_ARG_MAX_LEN || arg.chars().any(char::is_control) {
        anyhow::bail!("launch.args 里的 {arg} 不合法：只能是 --名字 或 --名字=值，不超过 {LAUNCH_ARG_MAX_LEN} 个字符");
    }
    if BLOCKED_ARG_PREFIXES.iter().any(|blocked| name.eq_ignore_ascii_case(blocked)) {
        anyhow::bail!("launch.args 不能设置 {name}：调试端口与用户数据目录由 Codex++ 管理");
    }
    Ok(())
}

fn string_list(items: &[Value]) -> Option<Vec<String>> {
    items.iter().map(|item| item.as_str().map(str::to_string)).collect()
}

/// 把启用中拓展的 preload 脚本包成一段新文档脚本：每个拓展各自 try/catch，抛错记进
/// `window.__codexPlusPreloadErrors`，不影响别的拓展与 Codex 自己的脚本。脚本里能读 `codexPlusPreload`
/// （`{ id, key, config }`）。没有时返回 None。
pub fn preload_document_script(entries: &[(String, String, Value, Vec<PathBuf>)]) -> Option<String> {
    let mut blocks = Vec::new();
    for (key, id, config, scripts) in entries {
        for path in scripts {
            let Ok(source) = std::fs::read_to_string(path) else {
                continue;
            };
            let meta = json!({ "key": key, "id": id, "config": config });
            blocks.push(format!(
                "(() => {{ const codexPlusPreload = Object.freeze({meta}); try {{\n{source}\n}} catch (error) {{ (window.__codexPlusPreloadErrors ||= []).push({{ key: codexPlusPreload.key, error: String(error?.stack || error?.message || error) }}); }} }})();"
            ));
        }
    }
    (!blocks.is_empty()).then(|| blocks.join("\n"))
}

/// 启动前后脚本的运行器：import 拓展脚本，调用默认导出（或 `before` / `after` 具名导出），结果写回 stdout。
const LAUNCH_RUNNER: &str = r#"
import { pathToFileURL } from "node:url";
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const input = JSON.parse(Buffer.concat(chunks).toString("utf8"));
let result = { ok: true };
try {
  const module = await import(pathToFileURL(input.script).href);
  const fn = typeof module.default === "function" ? module.default : module[input.context.event];
  if (typeof fn !== "function") throw new Error(`${input.script} 没有默认导出函数`);
  const value = await fn(input.context);
  result = { ok: true, value: value === undefined ? null : value };
} catch (error) {
  result = { ok: false, error: String(error?.stack || error?.message || error) };
}
process.stdout.write(JSON.stringify(result));
"#;

/// 启动前后的上下文（不含任何凭据）。
#[derive(Debug, Clone)]
pub struct LaunchContext {
    pub codex_home: PathBuf,
    pub state_dir: PathBuf,
    pub app_dir: PathBuf,
}

/// 跑启用中拓展的 `launch.before` / `launch.after`。逐个执行，出错与超时只记诊断日志，不挡 Codex 启动 / 退出。
pub async fn run_scripts(event: &str, context: &LaunchContext, entries: &[(String, Value, ExtensionLaunch)]) {
    let scripts: Vec<_> = entries
        .iter()
        .filter_map(|(id, config, launch)| {
            let script = if event == "before" { launch.before.as_ref() } else { launch.after.as_ref() };
            script.map(|script| (id, config, script, launch.timeout_secs))
        })
        .collect();
    if scripts.is_empty() {
        return;
    }
    let Ok(node) = crate::extension_node::require_node() else {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "extension_launch.node_missing",
            json!({ "event": event }),
        );
        return;
    };
    for (id, config, script, timeout) in scripts {
        let input = json!({
            "script": script,
            "context": {
                "event": event,
                "extension": id,
                "codexHome": context.codex_home,
                "stateDir": context.state_dir,
                "appDir": context.app_dir,
                "platform": std::env::consts::OS,
                "config": config,
            },
        });
        let outcome = crate::extension_node::run_runner(&node, LAUNCH_RUNNER, &input, Duration::from_secs(timeout)).await;
        let detail = match outcome {
            Ok(reply) if reply["ok"] == true => json!({ "extension": id, "event": event, "status": "ok" }),
            Ok(reply) => json!({ "extension": id, "event": event, "status": "failed", "error": reply["error"] }),
            Err(error) => json!({ "extension": id, "event": event, "status": "failed", "error": format!("{error:#}") }),
        };
        let _ = crate::diagnostic_log::append_diagnostic_log("extension_launch.script", detail);
    }
}

/// 拓展追加的启动参数接在用户自己的参数后面；同名参数用户的优先（用户写过的名字拓展不再追加）。
pub fn merged_launch_args(user_args: &[String], entries: &[(String, Value, ExtensionLaunch)]) -> Vec<String> {
    let name = |arg: &str| arg.split('=').next().unwrap_or_default().to_ascii_lowercase();
    let mut merged = user_args.to_vec();
    for (_, _, launch) in entries {
        for arg in &launch.args {
            if !merged.iter().any(|existing| name(existing) == name(arg)) {
                merged.push(arg.clone());
            }
        }
    }
    merged
}
