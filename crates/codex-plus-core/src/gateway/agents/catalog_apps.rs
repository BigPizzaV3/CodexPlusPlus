//! 通用 Agent 规格：桌面应用与 IDE：Claude Desktop、Zed、VS Code 系、JetBrains、Raycast 等。
//!
//! 写法参考 [`super::catalog_core`]。这一批的共同点：
//!
//! - 模型在应用自己的界面里选，配置文件里只能放「供应商 + 模型表」，个别有「默认模型」键的才一起设；
//! - 很多应用把供应商放在列表里（数组），框架按顶层键（或两层）整块还原，所以写入时用户的其它条目
//!   原样留在列表里，断开时整个列表回到接入前；
//! - 没有可写文件的应用（只能走数据库、运行中的本地 API、URL scheme 或进程环境变量的）不在这里。

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};

use crate::gateway::agents::catalog_core::{opencode_provider, output_limit};
use crate::gateway::agents::files::{parse_jsonc, strip_jsonc, yaml_block};
use crate::gateway::agents::{
    AgentEnv, AgentSpec, AgentWire, FileSet, Os, PROVIDER_ID, PROVIDER_NAME, Wiring, object_entry,
};

pub static SPECS: &[AgentSpec] = &[
    CLAUDE_DESKTOP,
    ZED,
    VSCODE,
    VSCODE_INSIDERS,
    VSCODIUM,
    ZOO_CODE,
    JETBRAINS_AIR,
    COPILOT_JETBRAINS,
    PENCIL,
    OPENPENCIL,
    RAYCAST,
    T3_CODE,
    ASTRBOT,
    OPENCLAW,
];

// ---------------------------------------------------------------- 公用小工具

/// 没写上下文窗口的模型按 128K 算（这些应用大多要求每个模型都有窗口）。
const DEFAULT_WINDOW: u64 = 128_000;

fn window_of(wiring: &Wiring, model: &str) -> u64 {
    wiring.window(model).unwrap_or(DEFAULT_WINDOW)
}

/// 网关的 Chat Completions 完整地址（要求填到 `/chat/completions` 的应用用）。
fn chat_url(wiring: &Wiring) -> String {
    format!("{}/chat/completions", wiring.v1())
}

/// `codexplus/<模型>`：按「供应商/模型」引用模型的应用用。
fn provider_ref(model: &str) -> String {
    format!("{PROVIDER_ID}/{model}")
}

/// 真实环境（读得到进程环境变量）。测试用的 [`AgentEnv::at`] 不读进程环境变量，
/// 这时也不去看系统级的应用目录、不查进程，结果只由临时目录决定。
fn is_live(env: &AgentEnv) -> bool {
    env.var("PATH").is_some()
}

/// macOS 上装了某个 `.app`（`~/Applications` 或 `/Applications`）。
fn mac_app(env: &AgentEnv, names: &[&str]) -> bool {
    if env.os != Os::Mac {
        return false;
    }
    let mut dirs = vec![env.home_path("Applications")];
    if is_live(env) {
        dirs.push(PathBuf::from("/Applications"));
    }
    dirs.iter()
        .any(|dir| names.iter().any(|name| dir.join(name).is_dir()))
}

/// 命令的完整路径（PATH 与常见安装目录）；从访达启动的应用拿不到 shell 的 PATH，写进配置要用完整路径。
fn find_binary(env: &AgentEnv, binary: &str, extra: &[&str]) -> Option<PathBuf> {
    let windows = env.os == Os::Windows;
    let names: Vec<String> = if windows {
        ["", ".exe", ".cmd", ".bat"]
            .iter()
            .map(|ext| format!("{binary}{ext}"))
            .collect()
    } else {
        vec![binary.to_string()]
    };
    let separator = if windows { ';' } else { ':' };
    let mut dirs: Vec<PathBuf> = env
        .var("PATH")
        .map(|path| {
            path.split(separator)
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    for relative in [".local/bin", ".bun/bin", ".npm-global/bin", ".volta/bin"]
        .iter()
        .chain(extra)
    {
        dirs.push(env.home_path(relative));
    }
    if is_live(env) && !windows {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    dirs.iter().find_map(|dir| {
        names
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.metadata().is_ok_and(|meta| meta.is_file()))
    })
}

/// 进程在运行（按可执行文件名）。只在真实环境、且是本机系统时查。
fn process_running(env: &AgentEnv, name: &str) -> bool {
    if !is_live(env) || env.os != Os::current() {
        return false;
    }
    #[cfg(windows)]
    {
        let exe = format!("{name}.exe");
        crate::windows_enumerate_processes()
            .iter()
            .any(|process| process.exe_file.eq_ignore_ascii_case(&exe))
    }
    #[cfg(not(windows))]
    {
        // -f 按完整命令行匹配：Linux 的进程名最多 15 个字符，长名字用 -x 匹配不上。
        std::process::Command::new("pgrep")
            .args(["-f", &format!("(^|/){name}( |$)")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

/// 对象里按键名一层层取值。
fn lookup<'a>(map: &'a Map<String, Value>, path: &[&str]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    rest.iter()
        .try_fold(map.get(*first)?, |value, key| value.get(*key))
}

fn read_json_file(path: &Path) -> Option<Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| parse_jsonc(&text).ok())
}

/// 把对象列表里「我们的」条目换成 `want`：用户的条目原样留着；旧条目里不归我们管的键
/// （用户在应用里改过的开关、名字等，`owned` 以外的）按 `id_key` 对上后保留。
fn replace_ours(
    root: &mut Map<String, Value>,
    key: &str,
    ours: impl Fn(&Value) -> bool,
    want: Vec<Value>,
    id_key: &str,
    owned: &[&str],
) -> anyhow::Result<()> {
    let slot = root
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if slot.is_null() {
        *slot = Value::Array(Vec::new());
    }
    let Some(list) = slot.as_array_mut() else {
        bail!("配置里的 {key} 不是列表，为免改坏没有改动它");
    };
    let mut previous: Vec<Map<String, Value>> = Vec::new();
    list.retain(|item| {
        if ours(item) {
            if let Some(object) = item.as_object() {
                previous.push(object.clone());
            }
            false
        } else {
            true
        }
    });
    for mut item in want {
        if let Some(object) = item.as_object_mut() {
            let id = object.get(id_key).cloned();
            if let Some(old) = previous.iter().find(|old| old.get(id_key) == id.as_ref()) {
                for (name, value) in old {
                    if !owned.contains(&name.as_str()) {
                        object.insert(name.clone(), value.clone());
                    }
                }
            }
        }
        list.push(item);
    }
    Ok(())
}

// ---------------------------------------------------------------- Claude Desktop

/// Codex++ 在 Claude Desktop `configLibrary` 里的档案 id：与 Desktop 自己生成的 UUID 同形，
/// 最后一段是 "cdxpls" 的十六进制。
const DESKTOP_PROFILE_ID: &str = "00000000-0000-4000-8000-636478706c73";

/// Claude Desktop 的一组目录：`Claude`（放 `deploymentMode`）与 `Claude-3p`（第三方网关模式的全部数据）。
struct DesktopDirs {
    mode: PathBuf,
    three_p: PathBuf,
}

impl DesktopDirs {
    fn under(base: PathBuf) -> Self {
        Self {
            mode: base.join("Claude"),
            three_p: base.join("Claude-3p"),
        }
    }

    fn library(&self) -> PathBuf {
        self.three_p.join("configLibrary")
    }

    fn profile(&self) -> PathBuf {
        self.library().join(format!("{DESKTOP_PROFILE_ID}.json"))
    }
}

/// Windows 上 MSIX 安装的 Desktop：数据在 `%LOCALAPPDATA%\Packages\Claude_<发布者>\LocalCache`
/// （商店版是 `AnthropicPBC.Claude_<发布者>`）。
fn desktop_package(local: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(local.join("Packages"))
        .ok()?
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            (name.starts_with("Claude_") || name.starts_with("AnthropicPBC.Claude_"))
                && entry.path().is_dir()
        })
        .map(|entry| entry.path().join("LocalCache"))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Desktop 读的目录：macOS `~/Library/Application Support`，Windows `%LOCALAPPDATA%`
/// （MSIX 安装的再加上包里的一份，新旧版本读的位置不同，两份都写），Linux XDG 配置目录。
fn desktop_sets(env: &AgentEnv) -> Vec<DesktopDirs> {
    match env.os {
        Os::Mac => vec![DesktopDirs::under(env.app_config())],
        Os::Windows => {
            let local = env.local_app_data();
            let mut sets = vec![DesktopDirs::under(local.clone())];
            if let Some(package) = desktop_package(&local) {
                sets.push(DesktopDirs::under(package.join("Local")));
            }
            sets
        }
        Os::Linux => vec![DesktopDirs::under(env.xdg_config())],
    }
}

fn desktop_path(env: &AgentEnv) -> PathBuf {
    desktop_sets(env)
        .into_iter()
        .next()
        .map(|dirs| dirs.profile())
        .unwrap_or_default()
}

fn desktop_detect(env: &AgentEnv) -> bool {
    if desktop_sets(env)
        .iter()
        .any(|dirs| dirs.mode.is_dir() || dirs.three_p.is_dir())
    {
        return true;
    }
    if env.os == Os::Windows {
        return env.app_config().join("Claude").is_dir()
            || env
                .local_app_data()
                .join("AnthropicClaude")
                .join("claude.exe")
                .is_file();
    }
    mac_app(env, &["Claude.app"])
}

fn desktop_write_set(dirs: &DesktopDirs, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    // 网关档案：第三方网关模式下 Code 与 Cowork 都走它，不需要 Anthropic 登录。
    let profile = set.json(&dirs.profile())?;
    profile.insert("inferenceProvider".into(), json!("gateway"));
    profile.insert("inferenceGatewayBaseUrl".into(), json!(wiring.root));
    profile.insert("inferenceGatewayApiKey".into(), json!(wiring.token));
    profile.insert("inferenceGatewayAuthScheme".into(), json!("bearer"));
    profile
        .entry("disableDeploymentModeChooser")
        .or_insert(json!(true));
    profile
        .entry("coworkEgressAllowedHosts")
        .or_insert(json!(["*"]));

    // 档案登记进 _meta.json 并设为当前应用的那一份。
    let meta = set.json(&dirs.library().join("_meta.json"))?;
    let entries = meta
        .entry("entries")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(list) = entries.as_array_mut() else {
        bail!("Claude Desktop 的 configLibrary/_meta.json 里 entries 不是列表，为免改坏没有改动它");
    };
    if !list
        .iter()
        .any(|entry| entry.get("id").and_then(Value::as_str) == Some(DESKTOP_PROFILE_ID))
    {
        list.push(json!({ "id": DESKTOP_PROFILE_ID, "name": PROVIDER_NAME }));
    }
    meta.insert("appliedId".into(), json!(DESKTOP_PROFILE_ID));

    // 两边的 claude_desktop_config.json 都切到 3p 模式。
    let mode_config = set.json(&dirs.mode.join("claude_desktop_config.json"))?;
    mode_config.insert("deploymentMode".into(), json!("3p"));
    let own_servers = mode_config
        .get("mcpServers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let three_p_config = set.json(&dirs.three_p.join("claude_desktop_config.json"))?;
    three_p_config.insert("deploymentMode".into(), json!("3p"));
    // 3p 模式下 Desktop 只读 Claude-3p 里的 MCP 服务器：把原来配的、这边还没有的带过去。
    if !own_servers.is_empty() {
        let servers = three_p_config
            .entry("mcpServers")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(servers) = servers.as_object_mut() {
            for (name, server) in own_servers {
                servers.entry(name).or_insert(server);
            }
        }
    }
    Ok(())
}

/// Claude Code 认的模型能力声明：`<模型>=effort,xhigh_effort;<模型>=…`。
///
/// Desktop 的 Code 标签页跑的是 Claude Code（读 `~/.claude/settings.json`）。它不认识的模型
/// （包括我们的 mythos-gw- 别名）默认不发 effort，Desktop 菜单里选了也到不了网关。
const CLAUDE_CAPS_ENV: &str = "CLAUDE_CODE_MODEL_CAPABILITIES";

/// Claude Code 的设置目录：`$CLAUDE_CONFIG_DIR`，否则 `~/.claude`。
fn claude_code_settings_path(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["CLAUDE_CONFIG_DIR"], || env.home_path(".claude"))
        .join("settings.json")
}

/// 一个模型在这些档位下对 Claude Code 声明的能力：有档位就是 effort，
/// 档位达到 xhigh / max 再加 xhigh_effort / max_effort。网关发往上游时会按模型实有档位套，
/// 所以到 max 的模型也声明 xhigh（Claude Code 的 Ultracode 要 xhigh_effort）。
fn claude_caps(levels: &[String]) -> Option<String> {
    let has = |level: &str| levels.iter().any(|known| known == level);
    if levels.iter().all(|level| level == "none") {
        return None;
    }
    let mut caps = vec!["effort"];
    if has("xhigh") || has("max") {
        caps.push("xhigh_effort");
    }
    if has("max") {
        caps.push("max_effort");
    }
    Some(caps.join(","))
}

/// `CLAUDE_CODE_MODEL_CAPABILITIES` 的值：Desktop 交给 Claude Code 的每个有档位的别名。
/// Claude Code 把模型名去掉 `[1m]` 后整串比较，别名本来就是小写。没有一个带档位时为 None。
pub(crate) fn desktop_capabilities(wiring: &Wiring) -> Option<String> {
    let segments: Vec<String> = wiring
        .models
        .iter()
        .filter(|model| !model.id.to_ascii_lowercase().contains("claude"))
        .filter_map(|model| {
            let caps = claude_caps(&model.reasoning_levels)?;
            let alias = crate::gateway::route::claude_desktop_alias(&model.id, true);
            Some(format!("{alias}={caps}"))
        })
        .collect();
    (!segments.is_empty()).then(|| segments.join(";"))
}

fn desktop_write(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    for dirs in desktop_sets(env) {
        desktop_write_set(&dirs, wiring, set)?;
    }
    if let Some(caps) = desktop_capabilities(wiring) {
        let settings = set.json(&claude_code_settings_path(env))?;
        let env_map = object_entry(settings, "env");
        // 用户自己写了这个变量（不是我们的别名声明）就不覆盖。
        let ours = |value: &str| {
            value.split(';').all(|segment| {
                segment.trim().is_empty()
                    || segment
                        .trim()
                        .starts_with(crate::gateway::route::CLAUDE_EFFORT_ALIAS_PREFIX)
            })
        };
        let current = env_map.get(CLAUDE_CAPS_ENV).and_then(Value::as_str);
        if current.is_none_or(ours) {
            env_map.insert(CLAUDE_CAPS_ENV.to_string(), json!(caps));
        }
    }
    Ok(())
}

pub const CLAUDE_DESKTOP: AgentSpec = AgentSpec {
    id: "claude-desktop",
    name: "Claude Desktop",
    binary: "",
    wire: AgentWire::Anthropic,
    detect: desktop_detect,
    config_path: desktop_path,
    write: desktop_write,
    hint: "完全退出并重新打开 Claude Desktop 后生效（Code 与 Cowork 走网关，无需 Anthropic 登录）；非 Claude 模型在它的选择器里以真实名字显示（底层 id 是 claude-gw- / mythos-gw- 开头的别名），有思考档位的模型带思考等级菜单，选中的档位按模型实有档位套用",
};

fn desktop_local_path(env: &AgentEnv) -> PathBuf {
    // 仅用于展示；本地服务通过进程环境接入网关，不写 Claude 的部署配置。
    env.var(crate::paths::STATE_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| env.home_path(".codex-session-delete"))
        .join("claude-local")
        .join("conversations.json")
}

fn desktop_local_write(
    _env: &AgentEnv,
    _wiring: &Wiring,
    _set: &mut FileSet,
) -> anyhow::Result<()> {
    Ok(())
}

pub static CLAUDE_DESKTOP_LOCAL: AgentSpec = AgentSpec {
    id: "claude-desktop-local",
    name: "Claude Desktop 本地模式",
    binary: "",
    wire: AgentWire::Anthropic,
    detect: |env| env.os == Os::Mac && desktop_detect(env),
    config_path: desktop_local_path,
    write: desktop_local_write,
    hint: "从 Codex++ 启动原版 Claude Desktop 后生效；本地兼容服务通过进程环境连接网关，不修改官方应用或部署模式",
};

// ---------------------------------------------------------------- Zed

/// Zed 的配置目录：macOS 也是 `~/.config/zed`，Linux 跟 XDG，Windows `%APPDATA%\Zed`。
fn zed_dir(env: &AgentEnv) -> PathBuf {
    match env.os {
        Os::Mac => env.home_path(".config").join("zed"),
        Os::Windows => env.app_config().join("Zed"),
        Os::Linux => env.xdg_config().join("zed"),
    }
}

fn zed_path(env: &AgentEnv) -> PathBuf {
    zed_dir(env).join("settings.json")
}

fn zed_write(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|entry| {
            let window = window_of(wiring, &entry.id);
            let output = output_limit(window);
            json!({
                "name": entry.id,
                "display_name": entry.display_name,
                // Zed 的 max_tokens 是提示词与回复共用的窗口。
                "max_tokens": window + output,
                "max_output_tokens": output,
                "capabilities": {
                    "tools": true,
                    "images": false,
                    "parallel_tool_calls": false,
                    "prompt_cache_key": false,
                    "chat_completions": true,
                },
            })
        })
        .collect();
    let root = set.json(&zed_path(env))?;
    object_entry(object_entry(root, "language_models"), "openai_compatible").insert(
        PROVIDER_ID.to_string(),
        json!({ "api_url": wiring.v1(), "available_models": models }),
    );
    object_entry(root, "agent").insert(
        "default_model".to_string(),
        json!({ "provider": PROVIDER_ID, "model": wiring.default_model }),
    );
    Ok(())
}

pub const ZED: AgentSpec = AgentSpec {
    id: "zed",
    name: "Zed",
    binary: "zed",
    wire: AgentWire::Chat,
    detect: |env| {
        zed_dir(env).is_dir()
            || env.has_binary("zed")
            || mac_app(env, &["Zed.app", "Zed Preview.app"])
    },
    config_path: zed_path,
    write: zed_write,
    hint: "在 Zed 的 Agent 面板模型菜单里选 codexplus 下的模型；Zed 首次会要 API Key（存在系统钥匙串里），填这个 Agent 的网关 token",
};

// ---------------------------------------------------------------- VS Code 系

/// VS Code 内置 Chat 的自定义端点供应商（1.122+）。
const VSCODE_ENDPOINT: &str = "customendpoint";
/// 旧版 Copilot Chat 扩展（VSCodium 从扩展市场装的）的 OpenAI 兼容供应商。
const VSCODE_OAI: &str = "customoai";

struct VsCodeKind {
    /// User 目录所在的应用目录名：`Code`、`Code - Insiders`、`VSCodium`。
    folder: &'static str,
    /// 没有内置 Chat、要装 Copilot Chat 扩展的版本：扩展目录（相对用户目录）。
    extensions: Option<&'static str>,
}

const STABLE: VsCodeKind = VsCodeKind {
    folder: "Code",
    extensions: None,
};
const INSIDERS: VsCodeKind = VsCodeKind {
    folder: "Code - Insiders",
    extensions: None,
};
const CODIUM: VsCodeKind = VsCodeKind {
    folder: "VSCodium",
    extensions: Some(".vscode-oss/extensions"),
};

/// User 目录：macOS `~/Library/Application Support/<名>/User`，Windows `%APPDATA%`，Linux XDG。
fn vscode_user(env: &AgentEnv, kind: &VsCodeKind) -> PathBuf {
    env.app_config().join(kind.folder).join("User")
}

/// 一个 profile 自己的 settings.json / chatLanguageModels.json（沿用默认 profile 的那项为 None）。
#[derive(Debug, PartialEq, Eq)]
struct VsCodeProfile {
    settings: Option<PathBuf>,
    models: Option<PathBuf>,
}

/// 默认 profile 以外的 profile（`globalStorage/storage.json` 的 `userDataProfiles`）；
/// 目录已不在的跳过。
fn vscode_profiles(user: &Path) -> Vec<VsCodeProfile> {
    let Some(storage) = read_json_file(&user.join("globalStorage").join("storage.json")) else {
        return Vec::new();
    };
    let Some(profiles) = storage.get("userDataProfiles").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for profile in profiles {
        let location = profile.get("location");
        let dir = match location {
            Some(Value::String(id)) if !id.is_empty() => user.join("profiles").join(id),
            Some(Value::Object(uri)) => {
                let path = uri
                    .get("fsPath")
                    .and_then(Value::as_str)
                    .filter(|path| !path.is_empty())
                    .map(str::to_string)
                    .or_else(|| {
                        // URI 的 path：Windows 上是 `/c:/…`，去掉开头的斜杠。
                        uri.get("path").and_then(Value::as_str).map(|path| {
                            let bytes = path.as_bytes();
                            if bytes.len() > 2 && bytes[0] == b'/' && bytes[2] == b':' {
                                path[1..].to_string()
                            } else {
                                path.to_string()
                            }
                        })
                    });
                match path {
                    Some(path) if !path.is_empty() => PathBuf::from(path),
                    _ => continue,
                }
            }
            _ => continue,
        };
        if !dir.is_dir() {
            continue;
        }
        let uses_default = |key: &str| {
            profile
                .get("useDefaultFlags")
                .and_then(|flags| flags.get(key))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        out.push(VsCodeProfile {
            settings: (!uses_default("settings")).then(|| dir.join("settings.json")),
            models: (!uses_default("languageModels")).then(|| dir.join("chatLanguageModels.json")),
        });
    }
    out
}

/// 默认 profile 加上各 profile 自己的 settings.json。
fn vscode_settings_files(user: &Path) -> Vec<PathBuf> {
    std::iter::once(user.join("settings.json"))
        .chain(
            vscode_profiles(user)
                .into_iter()
                .filter_map(|profile| profile.settings),
        )
        .collect()
}

/// 扩展目录里装的 Copilot Chat 注册的是哪种供应商：有自定义端点用它，只有 OpenAI 兼容的用后者。
fn copilot_chat_vendor(extensions: &Path) -> &'static str {
    let Ok(entries) = std::fs::read_dir(extensions) else {
        return VSCODE_ENDPOINT;
    };
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let Some(rest) = name.strip_prefix("github.copilot-chat-") else {
            continue;
        };
        if !entry.path().is_dir() {
            continue;
        }
        // 平台版是 github.copilot-chat-<版本>-<平台>。
        let version: Vec<u64> = rest
            .split('-')
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect();
        if best.as_ref().is_none_or(|(known, _)| version > *known) {
            best = Some((version, entry.path()));
        }
    }
    let Some(package) = best.and_then(|(_, dir)| read_json_file(&dir.join("package.json"))) else {
        return VSCODE_ENDPOINT;
    };
    let vendors: Vec<&str> = package
        .pointer("/contributes/languageModelChatProviders")
        .and_then(Value::as_array)
        .map(|providers| {
            providers
                .iter()
                .filter_map(|provider| provider.get("vendor").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    if !vendors.contains(&VSCODE_ENDPOINT) && vendors.contains(&VSCODE_OAI) {
        VSCODE_OAI
    } else {
        VSCODE_ENDPOINT
    }
}

/// Codex++ 在 chatLanguageModels.json 里的分组。密钥 VS Code 只从密钥存储里读，
/// 所以 token 放在每个模型的请求头里（自定义端点用 Authorization，OpenAI 兼容的会丢掉它，改用 x-api-key）。
fn vscode_group(wiring: &Wiring, vendor: &str) -> Value {
    let url = chat_url(wiring);
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|entry| {
            let window = window_of(wiring, &entry.id);
            // VS Code 把输出预算从窗口里扣掉，留足提示词的空间。
            let output = (window / 4).clamp(1, 8_192);
            let mut model = json!({
                "id": entry.id,
                "name": entry.display_name,
                "url": url,
                "toolCalling": true,
                "vision": false,
                "maxOutputTokens": output,
            });
            if vendor == VSCODE_OAI {
                model["maxInputTokens"] = json!(window.saturating_sub(output));
                model["requestHeaders"] = json!({ "x-api-key": wiring.token });
            } else {
                model["contextWindow"] = json!(window);
                model["requestHeaders"] =
                    json!({ "Authorization": format!("Bearer {}", wiring.token) });
            }
            model
        })
        .collect();
    if vendor == VSCODE_OAI {
        json!({ "name": PROVIDER_ID, "vendor": VSCODE_OAI, "models": models })
    } else {
        json!({
            "name": PROVIDER_ID,
            "vendor": VSCODE_ENDPOINT,
            "apiType": "chat-completions",
            "models": models,
        })
    }
}

fn vscode_model_groups(text: Option<&str>) -> anyhow::Result<Vec<Value>> {
    match text {
        Some(current) if !strip_jsonc(current).trim().is_empty() => {
            match parse_jsonc(current)? {
                Value::Array(list) => Ok(list),
                _ => bail!("模型分组配置不是 JSON 数组，为免覆盖没有改动它"),
            }
        }
        _ => Ok(Vec::new()),
    }
}

fn vscode_group_is_ours(group: &Value) -> bool {
    group.get("name").and_then(Value::as_str) == Some(PROVIDER_ID)
        && matches!(
            group.get("vendor").and_then(Value::as_str),
            Some(VSCODE_ENDPOINT | VSCODE_OAI)
        )
}

/// 旧版按 Raw 保存的也是接入前原文；断开时只还原我们的分组，其它分组沿用当前值。
pub(super) fn restore_vscode_models(
    before: Option<&str>,
    current: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let Some(current) = current else {
        return Ok(None);
    };
    let original = vscode_model_groups(before)?;
    let mut groups = vscode_model_groups(Some(current))?;
    groups.retain(|group| !vscode_group_is_ours(group));
    for (index, group) in original.iter().enumerate() {
        if vscode_group_is_ours(group) {
            groups.insert(index.min(groups.len()), group.clone());
        }
    }
    if groups == original {
        return Ok(before.map(str::to_string));
    }
    let mut text = serde_json::to_string_pretty(&groups)?;
    text.push('\n');
    Ok(Some(text))
}

/// 改写 chatLanguageModels.json（JSONC 数组）：去掉旧的 Codex++ 分组，追加新的；
/// VS Code 自己写进分组的键（如模型的推理强度设置）保留。断开只还原 Codex++ 分组。
fn vscode_write_models(path: &Path, group: Value, set: &mut FileSet) -> anyhow::Result<()> {
    let text = set.raw(path)?;
    let mut list = vscode_model_groups(text.as_deref())
        .with_context(|| format!("无法解析 {}，为免覆盖没有改动它", path.display()))?;
    let vendor = group.get("vendor").cloned();
    let mut kept = Map::new();
    list.retain(|item| {
        let ours = vscode_group_is_ours(item);
        if ours && item.get("vendor") == vendor.as_ref() {
            kept = item.as_object().cloned().unwrap_or_default();
        }
        !ours
    });
    if let Value::Object(group) = group {
        for (key, value) in group {
            kept.insert(key, value);
        }
    }
    list.push(Value::Object(kept));
    let mut out = serde_json::to_string_pretty(&Value::Array(list))?;
    out.push('\n');
    *text = Some(out);
    Ok(())
}

fn vscode_write(
    env: &AgentEnv,
    kind: &VsCodeKind,
    wiring: &Wiring,
    set: &mut FileSet,
) -> anyhow::Result<()> {
    let user = vscode_user(env, kind);
    let vendor = match kind.extensions {
        Some(relative) => copilot_chat_vendor(&env.home_path(relative)),
        None => VSCODE_ENDPOINT,
    };
    let group = vscode_group(wiring, vendor);
    let profiles = vscode_profiles(&user);
    let model_files = std::iter::once(user.join("chatLanguageModels.json"))
        .chain(profiles.iter().filter_map(|profile| profile.models.clone()));
    for path in model_files {
        vscode_write_models(&path, group.clone(), set)?;
    }
    // 新开的对话默认用主模型。
    for path in vscode_settings_files(&user) {
        set.json(&path)?
            .insert("chat.defaultModel".to_string(), json!(wiring.default_model));
    }
    Ok(())
}

pub const VSCODE: AgentSpec = AgentSpec {
    id: "vscode",
    name: "VS Code Chat",
    binary: "code",
    wire: AgentWire::Chat,
    detect: |env| vscode_user(env, &STABLE).is_dir() || env.has_binary("code"),
    config_path: |env| vscode_user(env, &STABLE).join("chatLanguageModels.json"),
    write: |env, wiring, set| vscode_write(env, &STABLE, wiring, set),
    hint: "在 Chat 的模型选择器里选 codexplus 分组下的模型（需 VS Code 1.122+）；没出现时执行 Developer: Reload Window",
};

pub const VSCODE_INSIDERS: AgentSpec = AgentSpec {
    id: "vscode-insiders",
    name: "VS Code Insiders Chat",
    binary: "code-insiders",
    wire: AgentWire::Chat,
    detect: |env| vscode_user(env, &INSIDERS).is_dir() || env.has_binary("code-insiders"),
    config_path: |env| vscode_user(env, &INSIDERS).join("chatLanguageModels.json"),
    write: |env, wiring, set| vscode_write(env, &INSIDERS, wiring, set),
    hint: "在 Chat 的模型选择器里选 codexplus 分组下的模型；没出现时执行 Developer: Reload Window",
};

pub const VSCODIUM: AgentSpec = AgentSpec {
    id: "vscodium",
    name: "VSCodium Chat",
    binary: "codium",
    wire: AgentWire::Chat,
    detect: |env| vscode_user(env, &CODIUM).is_dir() || env.has_binary("codium"),
    config_path: |env| vscode_user(env, &CODIUM).join("chatLanguageModels.json"),
    write: |env, wiring, set| vscode_write(env, &CODIUM, wiring, set),
    hint: "需装 GitHub Copilot Chat 并开启 Chat 功能（chat.disableAIFeatures 设为 false）；Reload Window 后在 Chat 模型选择器选 codexplus 分组下的模型",
};

// ---------------------------------------------------------------- Zoo Code（VS Code 扩展）

/// Zoo Code 的扩展 id（安装目录是 `<id>-<版本>`）。
const ZOO_EXTENSION: &str = "zoocodeorganization.zoo-code";

/// 让 Zoo Code 启动时导入的设置文件，放在它自己的 globalStorage 目录里。
fn zoo_import_path(env: &AgentEnv) -> PathBuf {
    vscode_user(env, &STABLE)
        .join("globalStorage")
        .join(ZOO_EXTENSION)
        .join("settings")
        .join("codexplus-import.json")
}

fn zoo_installed(env: &AgentEnv) -> bool {
    std::fs::read_dir(env.home_path(".vscode/extensions")).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .starts_with(&format!("{ZOO_EXTENSION}-"))
                && entry.path().is_dir()
        })
    })
}

/// Zoo Code 的供应商档案存在 VS Code 的加密密钥存储里，写不进去；它的
/// `zoo-code.autoImportSettingsPath` 设置指向的文件会在每次启动时导入，导入的档案成为当前档案。
fn zoo_write(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let model = &wiring.default_model;
    let window = window_of(wiring, model);
    let import = zoo_import_path(env);
    set.json(&import)?.insert(
        "providerProfiles".to_string(),
        json!({
            "currentApiConfigName": PROVIDER_NAME,
            "apiConfigs": {
                PROVIDER_NAME: {
                    "id": PROVIDER_ID,
                    "apiProvider": "openai",
                    "openAiBaseUrl": wiring.v1(),
                    "openAiApiKey": wiring.token,
                    "openAiModelId": model,
                    "openAiStreamingEnabled": true,
                    "openAiCustomModelInfo": {
                        "contextWindow": window,
                        "maxTokens": output_limit(window),
                        "supportsImages": false,
                        "supportsPromptCache": false,
                    },
                },
            },
        }),
    );
    let import_text = import.to_string_lossy().to_string();
    for path in vscode_settings_files(&vscode_user(env, &STABLE)) {
        set.json(&path)?.insert(
            "zoo-code.autoImportSettingsPath".to_string(),
            json!(import_text),
        );
    }
    Ok(())
}

pub const ZOO_CODE: AgentSpec = AgentSpec {
    id: "zoo-code",
    name: "Zoo Code",
    binary: "",
    wire: AgentWire::Chat,
    detect: zoo_installed,
    config_path: zoo_import_path,
    write: zoo_write,
    hint: "在 VS Code 里执行 Developer: Reload Window，Zoo Code 启动时导入并切到 Codex++ 档案；断开后该档案仍留在 Zoo Code 里，需手动切走",
};

// ---------------------------------------------------------------- JetBrains Air

/// Air 的配置目录：macOS `~/Library/Application Support/JetBrains/Air`，Windows `%APPDATA%`，Linux XDG。
fn air_dir(env: &AgentEnv) -> PathBuf {
    env.app_config().join("JetBrains").join("Air")
}

fn air_opencode_config(env: &AgentEnv) -> PathBuf {
    air_dir(env).join("codexplus-opencode.json")
}

/// Air 自己的 Agent 没有可改的地址，但它接受任何 ACP Agent（`acp.json` 的 `agent_servers`）。
/// 这里加一个跑 `opencode acp` 的「Codex++」Agent，用 `OPENCODE_CONFIG` 指向一份只开 Codex++
/// 供应商的 OpenCode 配置，Air 的模型菜单里就是网关的模型。
fn air_write(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let config = air_opencode_config(env);
    let root = set.json(&config)?;
    root.insert(
        "$schema".to_string(),
        json!("https://opencode.ai/config.json"),
    );
    object_entry(root, "provider").insert(PROVIDER_ID.to_string(), opencode_provider(wiring));
    root.insert("enabled_providers".to_string(), json!([PROVIDER_ID]));
    root.insert(
        "model".to_string(),
        json!(provider_ref(&wiring.default_model)),
    );
    root.insert(
        "small_model".to_string(),
        json!(provider_ref(&wiring.small_model)),
    );
    let opencode = find_binary(env, "opencode", &[".opencode/bin"])
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "opencode".to_string());
    let acp = set.json(&air_dir(env).join("acp.json"))?;
    object_entry(acp, "agent_servers").insert(
        PROVIDER_NAME.to_string(),
        json!({
            "command": opencode,
            "args": ["acp"],
            "env": { "OPENCODE_CONFIG": config.to_string_lossy() },
        }),
    );
    Ok(())
}

pub const JETBRAINS_AIR: AgentSpec = AgentSpec {
    id: "jetbrains-air",
    name: "JetBrains Air",
    binary: "",
    wire: AgentWire::Chat,
    detect: |env| air_dir(env).is_dir(),
    config_path: |env| air_dir(env).join("acp.json"),
    write: air_write,
    hint: "在 Air 新建任务时从 Agent 菜单选 Codex++，模型菜单里是网关的模型；需要本机装有 OpenCode",
};

// ---------------------------------------------------------------- Copilot (JetBrains)

/// Copilot 语言服务器的 BYOK 配置：`$XDG_CONFIG_HOME/github-copilot/byok.json`，
/// Windows `%USERPROFILE%\AppData\Local\github-copilot`，其余 `~/.config/github-copilot`。
fn copilot_byok_path(env: &AgentEnv) -> PathBuf {
    let dir = match env
        .dir_var("XDG_CONFIG_HOME")
        .filter(|dir| dir.is_absolute())
    {
        Some(dir) => dir,
        None if env.os == Os::Windows => env.home_path("AppData").join("Local"),
        None => env.home_path(".config"),
    };
    dir.join("github-copilot").join("byok.json")
}

/// 某个 JetBrains IDE（或 Android Studio）装了 Copilot 插件。
fn copilot_plugin_installed(env: &AgentEnv) -> bool {
    let (base, nested) = match env.os {
        Os::Linux => (env.xdg_data(), false),
        _ => (env.app_config(), true),
    };
    ["JetBrains", "Google"].iter().any(|vendor| {
        std::fs::read_dir(base.join(vendor)).is_ok_and(|ides| {
            ides.flatten().any(|ide| {
                let plugins = if nested {
                    ide.path().join("plugins")
                } else {
                    ide.path()
                };
                plugins.join("github-copilot-intellij").is_dir()
            })
        })
    })
}

fn copilot_write(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let root = set.json(&copilot_byok_path(env))?;
    let models_key = format!("{PROVIDER_ID}-models-config");
    // 用户在 Manage Models 里隐藏的模型保持隐藏。
    let hidden: Vec<String> = root
        .get(&models_key)
        .and_then(Value::as_object)
        .map(|models| {
            models
                .iter()
                .filter(|(_, model)| {
                    model.get("isRegistered").and_then(Value::as_bool) == Some(false)
                })
                .map(|(id, _)| id.clone())
                .collect()
        })
        .unwrap_or_default();
    let url = chat_url(wiring);
    let mut models = Map::new();
    for entry in &wiring.models {
        let id = &entry.id;
        let window = window_of(wiring, id);
        models.insert(
            id.to_string(),
            json!({
                "deploymentUrl": url,
                "isRegistered": !hidden.iter().any(|hidden| hidden == id),
                "isCustomModel": true,
                "modelCapabilities": {
                    "name": entry.display_name,
                    "maxInputTokens": window,
                    "maxOutputTokens": (window / 4).clamp(1, 16_000),
                    "toolCalling": true,
                    "vision": false,
                },
            }),
        );
    }
    root.insert(
        format!("{PROVIDER_ID}-provider-config"),
        json!({ "groupName": PROVIDER_ID, "apiType": "chatCompletions" }),
    );
    root.insert(format!("{PROVIDER_ID}-api-key"), json!(wiring.token));
    root.insert(models_key, Value::Object(models));
    Ok(())
}

pub const COPILOT_JETBRAINS: AgentSpec = AgentSpec {
    id: "copilot-jetbrains",
    name: "Copilot (JetBrains)",
    binary: "",
    wire: AgentWire::Chat,
    detect: |env| copilot_plugin_installed(env) || copilot_byok_path(env).is_file(),
    config_path: copilot_byok_path,
    write: copilot_write,
    hint: "重启 IDE 后在 Copilot Chat 的模型选择器里选 codexplus 下的模型；需要 Copilot 账号支持自带密钥（BYOK）",
};

// ---------------------------------------------------------------- Pencil

fn pencil_path(env: &AgentEnv) -> PathBuf {
    env.home_path(".pencil").join("models.json")
}

fn pencil_write(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|entry| {
            let window = window_of(wiring, &entry.id);
            json!({
                "id": entry.id,
                "name": entry.display_name,
                "reasoning": false,
                "input": ["text"],
                "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0 },
                "contextWindow": window,
                "maxTokens": (window / 4).clamp(1, 16_384),
                "compat": {},
            })
        })
        .collect();
    let root = set.json(&pencil_path(env))?;
    object_entry(root, "providers").insert(
        PROVIDER_ID.to_string(),
        json!({
            "name": PROVIDER_NAME,
            "baseUrl": wiring.v1(),
            "api": "openai-completions",
            "apiKey": wiring.token,
            "models": models,
        }),
    );
    Ok(())
}

pub const PENCIL: AgentSpec = AgentSpec {
    id: "pencil",
    name: "Pencil",
    binary: "",
    wire: AgentWire::Chat,
    detect: |env| env.home_path(".pencil").is_dir() || mac_app(env, &["Pencil.app", "pen.dev.app"]),
    config_path: pencil_path,
    write: pencil_write,
    hint: "重启 Pencil 后在 Agent 输入框的模型选择器里选 Codex++ 下的模型",
};

// ---------------------------------------------------------------- OpenPencil

/// OpenPencil 一个条目最多保留的模型数，多的它下次保存时会丢掉。
const OPENPENCIL_MAX_MODELS: usize = 64;

fn openpencil_path(env: &AgentEnv) -> PathBuf {
    env.app_config().join("openpencil").join("settings.json")
}

fn openpencil_write(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    if process_running(env, "openpencil-desktop") {
        bail!("OpenPencil 退出时会把设置整份写回、盖掉这次写入：请先退出 OpenPencil 再接入");
    }
    let root = set.json(&openpencil_path(env))?;
    // version 是必填项，不是 1 的整份文件都会被 OpenPencil 忽略。
    match root.get("version") {
        None => {
            root.insert("version".to_string(), json!(1));
        }
        Some(version) if version == &json!(1) => {}
        Some(version) => bail!(
            "OpenPencil 的 settings.json 写的 version 是 {version}，不是它认的 1，为免改坏没有改动它"
        ),
    }
    let mut models: Vec<&str> = Vec::new();
    for id in wiring.model_ids() {
        if !id.is_empty() && !models.contains(&id) && models.len() < OPENPENCIL_MAX_MODELS {
            models.push(id);
        }
    }
    let mut entry = json!({
        "id": PROVIDER_ID,
        "preset": "custom",
        "display_name": PROVIDER_NAME,
        "kind": "openai-compat",
        "api_key": wiring.token,
        "model": models.first().copied().unwrap_or_default(),
        "base_url": wiring.v1(),
        "enabled": true,
    });
    if models.len() > 1 {
        entry["models"] = json!(models);
    }
    // 用户在 OpenPencil 里给这个条目改的名字和开关保留。
    replace_ours(
        root,
        "builtin_agents",
        |item| item.get("id").and_then(Value::as_str) == Some(PROVIDER_ID),
        vec![entry],
        "id",
        &[
            "id", "preset", "kind", "api_key", "model", "models", "base_url",
        ],
    )
}

pub const OPENPENCIL: AgentSpec = AgentSpec {
    id: "openpencil",
    name: "OpenPencil",
    binary: "",
    wire: AgentWire::Chat,
    detect: |env| {
        openpencil_path(env).parent().is_some_and(Path::is_dir)
            || env.home_path(".openpencil").is_dir()
            || mac_app(env, &["OpenPencil.app"])
    },
    config_path: openpencil_path,
    write: openpencil_write,
    hint: "接入和断开前先退出 OpenPencil（它退出时会写回设置）；打开后在对话的模型选择器里选 Codex++ 下的模型",
};

// ---------------------------------------------------------------- Raycast

/// Raycast AI 的自定义供应商：Mac 与 Windows 都是用户目录下的 `~/.config/raycast/ai/providers.yaml`。
fn raycast_path(env: &AgentEnv) -> PathBuf {
    env.home_path(".config")
        .join("raycast")
        .join("ai")
        .join("providers.yaml")
}

fn raycast_entry(wiring: &Wiring) -> Value {
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|entry| {
            let mut model = json!({
                "id": entry.id,
                "name": entry.display_name,
                "abilities": {
                    "temperature": { "supported": true },
                    "vision": { "supported": false },
                    "system_message": { "supported": true },
                    "tools": { "supported": true },
                    "reasoning_effort": { "supported": false },
                },
            });
            if let Some(window) = entry.context_window {
                model["context"] = json!(window);
            }
            model
        })
        .collect();
    json!({
        "id": PROVIDER_ID,
        "name": PROVIDER_NAME,
        "base_url": wiring.v1(),
        "api_keys": { PROVIDER_ID: wiring.token },
        "models": models,
    })
}

fn yaml_line_value(line: &str) -> &str {
    let value = line.trim();
    if value.starts_with('"') || value.starts_with('\'') {
        return value;
    }
    match value.find(" #") {
        Some(index) => value[..index].trim(),
        None => value,
    }
}

fn yaml_unquote(value: &str) -> &str {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        return &value[1..value.len() - 1];
    }
    value
}

/// 列表里的一项是不是 Codex++ 的（这一项自己那一层的 `id: codexplus`，不看模型的 id）。
fn raycast_item_is_ours(item: &str) -> bool {
    let mut lines = item.lines();
    let Some(first) = lines.next() else {
        return false;
    };
    let dash = first.len() - first.trim_start().len();
    let after_dash = first.trim_start().trim_start_matches('-');
    let key_column = first.len() - after_dash.trim_start().len();
    let is_id = |text: &str| {
        text.split_once(':').is_some_and(|(key, value)| {
            key.trim() == "id" && yaml_unquote(yaml_line_value(value)) == PROVIDER_ID
        })
    };
    if is_id(after_dash.trim_start()) {
        return true;
    }
    lines.any(|line| {
        let content = line.trim_start();
        let column = line.len() - content.len();
        column == key_column && column > dash && is_id(content)
    })
}

/// 结尾的空行和注释（`YamlDoc::set_block` 会把原块尾部的这些接回去，这里先摘掉免得重复）。
fn split_trailing_comments(lines: &[&str]) -> usize {
    let mut end = lines.len();
    while end > 0 {
        let trimmed = lines[end - 1].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            end -= 1;
        } else {
            break;
        }
    }
    end
}

/// 新的 `providers` 块：用户的条目（连同注释）原样保留，换掉或追加 Codex++ 那一项，缩进跟用户的列表一致。
fn raycast_providers_block(existing: Option<&str>, entry: &Value) -> anyhow::Result<String> {
    let generated = yaml_block("providers", &json!([entry]));
    let Some(block) = existing else {
        return Ok(generated);
    };
    let lines: Vec<&str> = block.split_inclusive('\n').collect();
    let Some((header, rest)) = lines.split_first() else {
        return Ok(generated);
    };
    let rest = &rest[..split_trailing_comments(rest)];
    let inline = header
        .split_once(':')
        .map(|(_, value)| yaml_line_value(value))
        .unwrap_or_default();
    if !inline.is_empty() {
        // `providers: []` 之类：空列表才整块换掉，行内写了条目的不碰。
        if matches!(inline, "[]" | "~" | "null") && rest.is_empty() {
            return Ok(generated);
        }
        bail!("Raycast 的 providers.yaml 里 providers 不是缩进写法的列表，为免改坏没有改动它");
    }
    let indent = rest
        .iter()
        .find_map(|line| {
            let content = line.trim_start_matches(' ');
            content.starts_with('-').then(|| line.len() - content.len())
        })
        .unwrap_or(2);
    let mut head = String::new();
    let mut items: Vec<String> = Vec::new();
    for line in rest {
        let content = line.trim_start_matches(' ');
        if line.len() - content.len() == indent && content.starts_with('-') {
            items.push((*line).to_string());
            continue;
        }
        match items.last_mut() {
            Some(item) => item.push_str(line),
            None => head.push_str(line),
        }
    }
    items.retain(|item| !raycast_item_is_ours(item));
    let mut out = String::from(header.trim_end_matches('\n'));
    out.push('\n');
    out.push_str(&head);
    for item in items {
        out.push_str(&item);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    // 生成的列表项缩进 2 格，换成用户列表的缩进。
    let pad = " ".repeat(indent);
    for line in generated.split_inclusive('\n').skip(1) {
        out.push_str(&pad);
        out.push_str(line.strip_prefix("  ").unwrap_or(line));
    }
    Ok(out)
}

fn raycast_write(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let doc = set.yaml(&raycast_path(env))?;
    let block = raycast_providers_block(doc.block("providers"), &raycast_entry(wiring))?;
    doc.set_block("providers", block);
    Ok(())
}

pub const RAYCAST: AgentSpec = AgentSpec {
    id: "raycast",
    name: "Raycast",
    binary: "",
    wire: AgentWire::Chat,
    detect: |env| {
        mac_app(env, &["Raycast.app"]) || env.home_path(".config").join("raycast").is_dir()
    },
    config_path: raycast_path,
    write: raycast_write,
    hint: "Raycast 会自动读取改动，在 AI Chat / Quick AI 的模型选择器里选 Codex++ 下的模型（需 Raycast Pro）",
};

// ---------------------------------------------------------------- T3 Code

fn t3_path(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["T3CODE_HOME"], || env.home_path(".t3"))
        .join("userdata")
        .join("settings.json")
}

/// 写在 Codex++ 实例上、归网关管的环境变量。
const T3_ENV: &[&str] = &["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"];

/// T3 Code 的 Codex++ 供应商实例：用 Claude Code 驱动（claudeAgent），环境变量指向网关，
/// 网关的模型作为它的自定义模型。用户给这个实例改的名字、开关、额外环境变量和其它键保留。
fn t3_write(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let root = set.json(&t3_path(env))?;
    // 用户给 T3 自带的 Claude 指定的可执行文件与 Claude 目录，沿用到这个实例上。
    let mut config = Map::new();
    config.insert(
        "customModels".to_string(),
        Value::Array(
            wiring
                .models
                .iter()
                .map(|entry| json!({ "slug": entry.id, "name": entry.display_name }))
                .collect(),
        ),
    );
    for key in ["binaryPath", "homePath"] {
        let own = lookup(root, &["providerInstances", "claudeAgent", "config", key])
            .or_else(|| lookup(root, &["providers", "claudeAgent", key]))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(value) = own {
            config.insert(key.to_string(), json!(value));
        }
    }
    let instances = object_entry(root, "providerInstances");
    let previous = instances
        .get(PROVIDER_ID)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut environment = vec![
        json!({ "name": "ANTHROPIC_BASE_URL", "value": wiring.root, "sensitive": false }),
        json!({ "name": "ANTHROPIC_AUTH_TOKEN", "value": wiring.token, "sensitive": false }),
    ];
    if let Some(Value::Array(own)) = previous.get("environment") {
        environment.extend(
            own.iter()
                .filter(|item| {
                    item.get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| !T3_ENV.contains(&name))
                })
                .cloned(),
        );
    }
    let mut instance = previous;
    instance.insert("driver".to_string(), json!("claudeAgent"));
    instance
        .entry("displayName")
        .or_insert(json!(PROVIDER_NAME));
    instance.entry("enabled").or_insert(json!(true));
    instance.insert("environment".to_string(), Value::Array(environment));
    let merged_config = match instance.remove("config") {
        Some(Value::Object(mut own)) => {
            own.extend(config);
            own
        }
        _ => config,
    };
    instance.insert("config".to_string(), Value::Object(merged_config));
    instances.insert(PROVIDER_ID.to_string(), Value::Object(instance));
    Ok(())
}

pub const T3_CODE: AgentSpec = AgentSpec {
    id: "t3code",
    name: "T3 Code",
    binary: "",
    wire: AgentWire::Anthropic,
    detect: |env| {
        t3_path(env)
            .parent()
            .and_then(Path::parent)
            .is_some_and(Path::is_dir)
            || mac_app(env, &["T3 Code (Alpha).app", "T3 Code.app"])
    },
    config_path: t3_path,
    write: t3_write,
    hint: "在 T3 Code 的模型选择器里选 Codex++ 供应商下的模型（由本机的 Claude Code 运行）",
};

// ---------------------------------------------------------------- AstrBot

/// AstrBot 的根目录：`$ASTRBOT_ROOT`，否则桌面版的 `~/.astrbot`；
/// 也认 `astrbot init` 常放的 `~/astrbot`、`~/AstrBot`（里面已有配置或 `.astrbot` 标记时）。
fn astrbot_root(env: &AgentEnv) -> PathBuf {
    if let Some(root) = env.dir_var("ASTRBOT_ROOT") {
        return root;
    }
    let desktop = env.home_path(".astrbot");
    if desktop.join("data").join("cmd_config.json").is_file() {
        return desktop;
    }
    for name in ["astrbot", "AstrBot"] {
        let root = env.home_path(name);
        if root.join("data").join("cmd_config.json").is_file() || root.join(".astrbot").is_file() {
            return root;
        }
    }
    desktop
}

fn astrbot_path(env: &AgentEnv) -> PathBuf {
    astrbot_root(env).join("data").join("cmd_config.json")
}

/// 4.10 起 AstrBot 把聊天供应商拆成「来源」（地址与 Key）和「模型」两张表。
/// 用户在 WebUI 里改过的、不归我们管的键（超时、代理、开关、额外请求体）保留。
fn astrbot_write(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let root = set.json(&astrbot_path(env))?;
    let fresh = root.is_empty();
    let source = json!({
        "id": PROVIDER_ID,
        "provider": "openai",
        "type": "openai_chat_completion",
        "provider_type": "chat_completion",
        "enable": true,
        "key": [wiring.token],
        "api_base": wiring.v1(),
        "timeout": 120,
        "proxy": "",
        "custom_headers": {},
    });
    replace_ours(
        root,
        "provider_sources",
        |item| item.get("id").and_then(Value::as_str) == Some(PROVIDER_ID),
        vec![source],
        "id",
        &["id", "provider", "type", "provider_type", "key", "api_base"],
    )?;
    let models: Vec<Value> = wiring
        .model_ids()
        .map(|id| {
            json!({
                "id": provider_ref(id),
                "enable": true,
                "provider_source_id": PROVIDER_ID,
                "model": id,
                "modalities": ["text", "tool_use"],
                "custom_extra_body": {},
                // 0：由 AstrBot 自己查窗口。
                "max_context_tokens": wiring.window(id).unwrap_or(0),
            })
        })
        .collect();
    replace_ours(
        root,
        "provider",
        |item| item.get("provider_source_id").and_then(Value::as_str) == Some(PROVIDER_ID),
        models,
        "id",
        &[
            "id",
            "provider_source_id",
            "model",
            "modalities",
            "max_context_tokens",
        ],
    )?;
    // 默认聊天模型：4.28 起在 agent_runner 里，之前在 provider_settings 里。
    let pick = json!(provider_ref(&wiring.default_model));
    if fresh || root.contains_key("agent_runner") {
        let model = object_entry(
            object_entry(object_entry(root, "agent_runner"), "config"),
            "model",
        );
        model.insert("provider_id".to_string(), pick);
    } else {
        object_entry(root, "provider_settings").insert("default_provider_id".to_string(), pick);
    }
    Ok(())
}

pub const ASTRBOT: AgentSpec = AgentSpec {
    id: "astrbot",
    name: "AstrBot",
    binary: "astrbot",
    wire: AgentWire::Chat,
    detect: |env| {
        astrbot_path(env).is_file() || astrbot_root(env).is_dir() || env.has_binary("astrbot")
    },
    config_path: astrbot_path,
    write: astrbot_write,
    hint: "重启 AstrBot 后生效，默认聊天模型切到 Codex++ 的主模型；重启前别在 WebUI 里保存设置（会整份写回）",
};

// ---------------------------------------------------------------- OpenClaw

/// OpenClaw 的配置：`$OPENCLAW_CONFIG_PATH`，否则状态目录（`$OPENCLAW_STATE_DIR`，
/// 有 `$OPENCLAW_PROFILE` 时 `~/.openclaw-<profile>`，否则 `~/.openclaw`）里的 openclaw.json；
/// 其中的 `~` 是 `$OPENCLAW_HOME`（设置了时）。
fn openclaw_path(env: &AgentEnv) -> PathBuf {
    let home = env
        .var("OPENCLAW_HOME")
        .filter(|home| home != "undefined" && home != "null")
        .map(|home| env.expand(&home))
        .unwrap_or_else(|| env.home.clone());
    let expand = |value: String| -> PathBuf {
        if value == "~" {
            return home.clone();
        }
        match value
            .strip_prefix("~/")
            .or_else(|| value.strip_prefix("~\\"))
        {
            Some(rest) => home.join(rest),
            None => PathBuf::from(value),
        }
    };
    if let Some(path) = env.var("OPENCLAW_CONFIG_PATH") {
        return expand(path);
    }
    let profile = env.var("OPENCLAW_PROFILE").filter(|profile| {
        !profile.eq_ignore_ascii_case("default")
            && profile.len() <= 64
            && profile
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            && !profile.starts_with(['_', '-'])
    });
    let dir = match (env.var("OPENCLAW_STATE_DIR"), profile) {
        (Some(dir), _) => expand(dir),
        (None, Some(profile)) => home.join(format!(".openclaw-{profile}")),
        (None, None) => home.join(".openclaw"),
    };
    dir.join("openclaw.json")
}

fn openclaw_write(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let root = set.json(&openclaw_path(env))?;
    let provider = object_entry(
        object_entry(object_entry(root, "models"), "providers"),
        PROVIDER_ID,
    );
    provider.insert("baseUrl".to_string(), json!(wiring.v1()));
    provider.insert("apiKey".to_string(), json!(wiring.token));
    provider.insert("api".to_string(), json!("openai-completions"));
    // OpenClaw 的配置校验很严：只写它认识的键；用户在条目里加的键（headers、maxTokens 等）保留。
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|entry| {
            json!({
                "id": entry.id,
                "name": entry.display_name,
                "reasoning": false,
                "input": ["text"],
                "contextWindow": window_of(wiring, &entry.id),
            })
        })
        .collect();
    replace_ours(
        provider,
        "models",
        |_| true,
        models,
        "id",
        &["id", "name", "reasoning", "input", "contextWindow"],
    )?;

    let reference = provider_ref(&wiring.default_model);
    let defaults = object_entry(object_entry(root, "agents"), "defaults");
    match defaults.get_mut("model") {
        // {primary, fallbacks}：只换 primary，备用模型留着。
        Some(Value::Object(model)) => {
            model.insert("primary".to_string(), json!(reference));
        }
        _ => {
            defaults.insert("model".to_string(), json!(reference));
        }
    }
    // 像 `openclaw models set` 一样把它加进 agents.defaults.models（旧版本当作允许的模型表）。
    if let Some(Value::Object(models)) = defaults.get_mut("models") {
        let prefix = format!("{PROVIDER_ID}/");
        models.retain(|key, value| {
            !(key.starts_with(&prefix)
                && key != &reference
                && value.as_object().is_some_and(Map::is_empty))
        });
        models.entry(reference.clone()).or_insert(json!({}));
    }
    // 非空的 modelPolicy.allow 是白名单：没放行 Codex++ 的模型时加上 codexplus/*。
    let all = format!("{PROVIDER_ID}/*");
    if let Some(Value::Object(policy)) = defaults.get_mut("modelPolicy")
        && let Some(Value::Array(allow)) = policy.get_mut("allow")
        && !allow.is_empty()
        && !allow.iter().any(|item| {
            item.as_str()
                .is_some_and(|item| item == "*" || item == reference || item == all)
        })
    {
        allow.push(json!(all));
    }
    Ok(())
}

pub const OPENCLAW: AgentSpec = AgentSpec {
    id: "openclaw",
    name: "OpenClaw",
    binary: "openclaw",
    wire: AgentWire::Chat,
    detect: |env| {
        openclaw_path(env).parent().is_some_and(Path::is_dir) || env.has_binary("openclaw")
    },
    config_path: openclaw_path,
    write: openclaw_write,
    hint: "OpenClaw 会自动重新加载；已经选过模型的会话发 /model default 切到 Codex++ 的主模型",
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::agents::ModelEntry;

    fn wiring(id: &str) -> Wiring {
        Wiring {
            agent: id.to_string(),
            root: format!("http://127.0.0.1:57321/gw/{id}"),
            token: format!("cpx-gw-{id}-1"),
            default_model: "ds/deepseek-chat".to_string(),
            small_model: "ds/deepseek-chat".to_string(),
            models: vec![
                ModelEntry {
                    id: "ds/deepseek-chat".to_string(),
                    display_name: "ds/deepseek-chat".to_string(),
                    context_window: Some(128_000),
                    reasoning_levels: Vec::new(),
                },
                ModelEntry {
                    id: "group/daily".to_string(),
                    display_name: "group/daily".to_string(),
                    context_window: None,
                    reasoning_levels: Vec::new(),
                },
            ],
        }
    }

    fn write_file(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// 跑一遍写入，返回改到的文件（路径 → 写入后的全文）。
    fn connect(
        spec: &AgentSpec,
        env: &AgentEnv,
        id: &str,
    ) -> std::collections::BTreeMap<PathBuf, String> {
        let mut set = FileSet::new();
        (spec.write)(env, &wiring(id), &mut set).unwrap();
        set.changes()
            .unwrap()
            .into_iter()
            .map(|change| (change.path, change.after.unwrap_or_default()))
            .collect()
    }

    fn json_of(text: &str) -> Value {
        parse_jsonc(text).unwrap()
    }

    #[test]
    fn claude_desktop_local_metadata_follows_state_directory() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        assert_eq!(
            desktop_local_path(&env),
            temp.path()
                .join(".codex-session-delete/claude-local/conversations.json")
        );
        let state = temp.path().join("local instance state");
        let env = env.with_var(crate::paths::STATE_DIR_ENV, state.to_str().unwrap());
        assert_eq!(
            desktop_local_path(&env),
            state.join("claude-local/conversations.json")
        );
    }

    #[test]
    fn claude_desktop_local_detection_requires_supported_platform() {
        for os in [Os::Mac, Os::Windows, Os::Linux] {
            let temp = tempfile::tempdir().unwrap();
            let env = AgentEnv::at(temp.path()).with_os(os);
            let config_dir = desktop_sets(&env).remove(0).mode;
            std::fs::create_dir_all(config_dir).unwrap();
            assert!((CLAUDE_DESKTOP.detect)(&env));
            assert_eq!((CLAUDE_DESKTOP_LOCAL.detect)(&env), os == Os::Mac);
        }
    }

    #[test]
    fn claude_desktop_keeps_user_profiles_and_carries_mcp() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Mac);
        let base = temp.path().join("Library/Application Support");
        write_file(
            &base.join("Claude/claude_desktop_config.json"),
            r#"{"mcpServers":{"fs":{"command":"npx"}},"deploymentMode":"1p"}"#,
        );
        write_file(
            &base.join("Claude-3p/configLibrary/_meta.json"),
            r#"{"appliedId":"mine","entries":[{"id":"mine","name":"Mine"}]}"#,
        );
        let files = connect(&CLAUDE_DESKTOP, &env, "claude-desktop");
        let meta = json_of(&files[&base.join("Claude-3p/configLibrary/_meta.json")]);
        assert_eq!(meta["appliedId"], DESKTOP_PROFILE_ID);
        assert_eq!(meta["entries"][0]["id"], "mine");
        assert_eq!(meta["entries"][1]["id"], DESKTOP_PROFILE_ID);
        let profile = json_of(
            &files[&base
                .join("Claude-3p/configLibrary")
                .join(format!("{DESKTOP_PROFILE_ID}.json"))],
        );
        assert_eq!(
            profile["inferenceGatewayBaseUrl"],
            "http://127.0.0.1:57321/gw/claude-desktop"
        );
        assert_eq!(profile["inferenceProvider"], "gateway");
        let mode = json_of(&files[&base.join("Claude/claude_desktop_config.json")]);
        assert_eq!(mode["deploymentMode"], "3p");
        assert_eq!(mode["mcpServers"]["fs"]["command"], "npx");
        let three_p = json_of(&files[&base.join("Claude-3p/claude_desktop_config.json")]);
        assert_eq!(three_p["deploymentMode"], "3p");
        assert_eq!(three_p["mcpServers"]["fs"]["command"], "npx");
    }

    #[test]
    fn claude_desktop_declares_effort_caps_only_for_aliases_with_levels() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Mac);
        let mut wired = wiring("claude-desktop");
        wired.models[0].reasoning_levels = ["minimal", "low", "medium", "high", "xhigh"]
            .iter()
            .map(|level| level.to_string())
            .collect();
        let mut set = FileSet::new();
        (CLAUDE_DESKTOP.write)(&env, &wired, &mut set).unwrap();
        let files: std::collections::BTreeMap<PathBuf, String> = set
            .changes()
            .unwrap()
            .into_iter()
            .map(|change| (change.path, change.after.unwrap_or_default()))
            .collect();
        let settings = json_of(&files[&temp.path().join(".claude/settings.json")]);
        let alias = crate::gateway::route::claude_desktop_alias("ds/deepseek-chat", true);
        assert!(alias.starts_with("mythos-gw-"));
        // 没有档位资料的 group/daily 不声明；xhigh 为顶的模型声明 effort 与 xhigh_effort。
        assert_eq!(
            settings["env"]["CLAUDE_CODE_MODEL_CAPABILITIES"],
            format!("{alias}=effort,xhigh_effort")
        );

        // 用户自己写的声明不覆盖。
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Mac);
        write_file(
            &temp.path().join(".claude/settings.json"),
            r#"{"env":{"CLAUDE_CODE_MODEL_CAPABILITIES":"my-model=effort"}}"#,
        );
        let mut set = FileSet::new();
        (CLAUDE_DESKTOP.write)(&env, &wired, &mut set).unwrap();
        let kept = set
            .changes()
            .unwrap()
            .into_iter()
            .find(|change| change.path == temp.path().join(".claude/settings.json"))
            .and_then(|change| change.after);
        if let Some(text) = kept {
            assert_eq!(
                json_of(&text)["env"]["CLAUDE_CODE_MODEL_CAPABILITIES"],
                "my-model=effort"
            );
        }
    }

    #[test]
    fn vscode_writes_every_profile_and_keeps_user_groups() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        let user = temp.path().join(".config/Code/User");
        write_file(
            &user.join("settings.json"),
            "{\n  // 我的设置\n  \"editor.fontSize\": 14,\n}\n",
        );
        write_file(
            &user.join("chatLanguageModels.json"),
            "[\n  // 自己的\n  {\"name\": \"mine\", \"vendor\": \"customendpoint\", \"models\": []},\n  {\"name\": \"codexplus\", \"vendor\": \"customendpoint\", \"settings\": {\"x\": 1}, \"models\": []},\n]\n",
        );
        write_file(
            &user.join("globalStorage/storage.json"),
            r#"{"userDataProfiles":[{"location":"p1","name":"Work"},{"location":"p2","name":"Shared","useDefaultFlags":{"settings":true,"languageModels":true}},{"location":"gone","name":"Gone"}]}"#,
        );
        std::fs::create_dir_all(user.join("profiles/p1")).unwrap();
        std::fs::create_dir_all(user.join("profiles/p2")).unwrap();

        let files = connect(&VSCODE, &env, "vscode");
        let models = json_of(&files[&user.join("chatLanguageModels.json")]);
        let list = models.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["name"], "mine");
        assert_eq!(list[1]["name"], "codexplus");
        // VS Code 自己写进分组的设置保留。
        assert_eq!(list[1]["settings"]["x"], 1);
        assert_eq!(
            list[1]["models"][0]["url"],
            "http://127.0.0.1:57321/gw/vscode/v1/chat/completions"
        );
        assert_eq!(
            list[1]["models"][0]["requestHeaders"]["Authorization"],
            "Bearer cpx-gw-vscode-1"
        );
        let settings = json_of(&files[&user.join("settings.json")]);
        assert_eq!(settings["editor.fontSize"], 14);
        assert_eq!(settings["chat.defaultModel"], "ds/deepseek-chat");
        // 有自己设置的 profile 也写；沿用默认的、目录已不在的不写。
        assert!(files.contains_key(&user.join("profiles/p1/settings.json")));
        assert!(files.contains_key(&user.join("profiles/p1/chatLanguageModels.json")));
        assert!(!files.contains_key(&user.join("profiles/p2/settings.json")));
        assert!(
            !files
                .keys()
                .any(|path| path.starts_with(user.join("profiles/gone")))
        );
    }

    #[test]
    fn vscodium_uses_openai_compatible_group_for_old_copilot_chat() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        write_file(
            &temp
                .path()
                .join(".vscode-oss/extensions/github.copilot-chat-0.48.1/package.json"),
            r#"{"contributes":{"languageModelChatProviders":[{"vendor":"copilot"},{"vendor":"customoai"}]}}"#,
        );
        let files = connect(&VSCODIUM, &env, "vscodium");
        let models = json_of(
            &files[&temp
                .path()
                .join(".config/VSCodium/User/chatLanguageModels.json")],
        );
        let group = &models[0];
        assert_eq!(group["vendor"], "customoai");
        assert_eq!(
            group["models"][0]["requestHeaders"]["x-api-key"],
            "cpx-gw-vscodium-1"
        );
        assert!(group["models"][0].get("contextWindow").is_none());
    }

    #[test]
    fn vscode_models_file_must_be_an_array() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        write_file(
            &temp
                .path()
                .join(".config/Code/User/chatLanguageModels.json"),
            "{\"oops\": true}",
        );
        let mut set = FileSet::new();
        assert!((VSCODE.write)(&env, &wiring("vscode"), &mut set).is_err());
    }

    #[test]
    fn raycast_keeps_user_providers_and_comments() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let path = raycast_path(&env);
        let original = "# 我的供应商\nproviders:\n- id: mine # 自己的\n  name: Mine\n  base_url: http://x/v1\n  models:\n  - id: codexplus\n    name: trick\n- id: codexplus\n  name: old\n\n# 工具\ntools:\n  enabled: true\n";
        write_file(&path, original);
        let files = connect(&RAYCAST, &env, "raycast");
        let text = &files[&path];
        assert!(text.starts_with("# 我的供应商\nproviders:\n- id: mine # 自己的\n"));
        // 用户条目里叫 codexplus 的模型不算我们的条目。
        assert!(text.contains("  - id: codexplus\n    name: trick\n"));
        assert!(!text.contains("name: old"));
        assert!(text.contains("- api_keys:") || text.contains("- id: \"codexplus\""));
        assert!(text.contains("http://127.0.0.1:57321/gw/raycast/v1"));
        assert!(text.ends_with("\n# 工具\ntools:\n  enabled: true\n"));
        // 列表项缩进跟用户的一致（0 格）。
        assert!(!text.contains("\n  - api_keys:") && !text.contains("\n  - id: \"codexplus\""));

        // 再写一次结果不变。
        std::fs::write(&path, text).unwrap();
        let mut set = FileSet::new();
        (RAYCAST.write)(&env, &wiring("raycast"), &mut set).unwrap();
        assert!(set.changes().unwrap().is_empty());
    }

    #[test]
    fn raycast_refuses_inline_lists() {
        assert!(raycast_providers_block(Some("providers: [{id: a}]\n"), &json!({})).is_err());
        assert!(raycast_providers_block(Some("providers: []\n"), &json!({ "id": "x" })).is_ok());
    }

    #[test]
    fn openpencil_keeps_user_agents_name_and_switch() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        let path = openpencil_path(&env);
        write_file(
            &path,
            r#"{"version":1,"theme":"dark","builtin_agents":[{"id":"builtin-1","kind":"anthropic"},{"id":"codexplus","display_name":"我的网关","enabled":false,"model":"old"}]}"#,
        );
        let files = connect(&OPENPENCIL, &env, "openpencil");
        let value = json_of(&files[&path]);
        assert_eq!(value["theme"], "dark");
        let agents = value["builtin_agents"].as_array().unwrap();
        assert_eq!(agents[0]["id"], "builtin-1");
        assert_eq!(agents[1]["display_name"], "我的网关");
        assert_eq!(agents[1]["enabled"], false);
        assert_eq!(agents[1]["model"], "ds/deepseek-chat");
        assert_eq!(agents[1]["models"][1], "group/daily");
        assert_eq!(
            agents[1]["base_url"],
            "http://127.0.0.1:57321/gw/openpencil/v1"
        );

        write_file(&path, r#"{"version":2}"#);
        let mut set = FileSet::new();
        assert!((OPENPENCIL.write)(&env, &wiring("openpencil"), &mut set).is_err());
    }

    #[test]
    fn astrbot_keeps_user_sources_and_webui_edits() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let path = astrbot_path(&env);
        write_file(
            &path,
            "\u{feff}{\"provider_sources\":[{\"id\":\"ds\",\"key\":[\"sk\"]},{\"id\":\"codexplus\",\"timeout\":300,\"key\":[\"old\"]}],\"provider\":[{\"id\":\"ds/chat\",\"provider_source_id\":\"ds\"},{\"id\":\"codexplus/ds/deepseek-chat\",\"provider_source_id\":\"codexplus\",\"enable\":false}],\"provider_settings\":{\"default_provider_id\":\"ds/chat\",\"wake\":\"/\"}}",
        );
        let files = connect(&ASTRBOT, &env, "astrbot");
        let value = json_of(&files[&path]);
        let sources = value["provider_sources"].as_array().unwrap();
        assert_eq!(sources[0]["id"], "ds");
        assert_eq!(sources[1]["timeout"], 300);
        assert_eq!(sources[1]["key"][0], "cpx-gw-astrbot-1");
        assert_eq!(
            sources[1]["api_base"],
            "http://127.0.0.1:57321/gw/astrbot/v1"
        );
        let models = value["provider"].as_array().unwrap();
        assert_eq!(models[0]["id"], "ds/chat");
        assert_eq!(models[1]["enable"], false);
        assert_eq!(models.len(), 3);
        // 没有 agent_runner 的旧版本：默认模型写在 provider_settings 里，其余设置不动。
        assert_eq!(
            value["provider_settings"]["default_provider_id"],
            "codexplus/ds/deepseek-chat"
        );
        assert_eq!(value["provider_settings"]["wake"], "/");
        assert!(value.get("agent_runner").is_none());
    }

    #[test]
    fn openclaw_switches_primary_and_keeps_fallbacks() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let path = openclaw_path(&env);
        write_file(
            &path,
            r#"{
  // 我的配置
  "models": {"providers": {"mine": {"baseUrl": "http://x"}, "codexplus": {"headers": {"x-a": "1"}, "models": [{"id": "ds/deepseek-chat", "maxTokens": 9}]}}},
  "agents": {"defaults": {"model": {"primary": "mine/a", "fallbacks": ["mine/b"]}, "models": {"mine/a": {}, "codexplus/old": {}}, "modelPolicy": {"allow": ["mine/*"]}}},
}"#,
        );
        let files = connect(&OPENCLAW, &env, "openclaw");
        let value = json_of(&files[&path]);
        let provider = &value["models"]["providers"]["codexplus"];
        assert_eq!(value["models"]["providers"]["mine"]["baseUrl"], "http://x");
        assert_eq!(provider["headers"]["x-a"], "1");
        assert_eq!(provider["models"][0]["maxTokens"], 9);
        assert_eq!(provider["models"][0]["contextWindow"], 128_000);
        assert_eq!(provider["baseUrl"], "http://127.0.0.1:57321/gw/openclaw/v1");
        let defaults = &value["agents"]["defaults"];
        assert_eq!(defaults["model"]["primary"], "codexplus/ds/deepseek-chat");
        assert_eq!(defaults["model"]["fallbacks"][0], "mine/b");
        assert!(defaults["models"].get("codexplus/old").is_none());
        assert!(
            defaults["models"]
                .get("codexplus/ds/deepseek-chat")
                .is_some()
        );
        assert_eq!(defaults["modelPolicy"]["allow"][1], "codexplus/*");
    }

    #[test]
    fn copilot_keeps_hidden_models_and_other_providers() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        let path = copilot_byok_path(&env);
        assert_eq!(path, temp.path().join(".config/github-copilot/byok.json"));
        write_file(
            &path,
            r#"{"mine-api-key":"k","codexplus-models-config":{"group/daily":{"isRegistered":false}}}"#,
        );
        let files = connect(&COPILOT_JETBRAINS, &env, "copilot-jetbrains");
        let value = json_of(&files[&path]);
        assert_eq!(value["mine-api-key"], "k");
        let models = &value["codexplus-models-config"];
        assert_eq!(models["group/daily"]["isRegistered"], false);
        assert_eq!(models["ds/deepseek-chat"]["isRegistered"], true);
        assert_eq!(
            models["ds/deepseek-chat"]["deploymentUrl"],
            "http://127.0.0.1:57321/gw/copilot-jetbrains/v1/chat/completions"
        );
    }

    #[test]
    fn t3_instance_keeps_user_settings() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let path = t3_path(&env);
        write_file(
            &path,
            r#"{"providerInstances":{"claudeAgent":{"driver":"claudeAgent","config":{"binaryPath":"/opt/claude"}},"codexplus":{"displayName":"网关","environment":[{"name":"HTTPS_PROXY","value":"http://p"},{"name":"ANTHROPIC_BASE_URL","value":"old"}],"config":{"autoCompactWindow":5}}}}"#,
        );
        let files = connect(&T3_CODE, &env, "t3code");
        let value = json_of(&files[&path]);
        let instance = &value["providerInstances"]["codexplus"];
        assert_eq!(
            value["providerInstances"]["claudeAgent"]["driver"],
            "claudeAgent"
        );
        assert_eq!(instance["displayName"], "网关");
        assert_eq!(instance["driver"], "claudeAgent");
        assert_eq!(instance["config"]["binaryPath"], "/opt/claude");
        assert_eq!(instance["config"]["autoCompactWindow"], 5);
        assert_eq!(
            instance["config"]["customModels"][0]["slug"],
            "ds/deepseek-chat"
        );
        let environment = instance["environment"].as_array().unwrap();
        assert_eq!(environment[0]["value"], "http://127.0.0.1:57321/gw/t3code");
        assert_eq!(environment.len(), 3);
        assert_eq!(environment[2]["name"], "HTTPS_PROXY");
    }

    #[test]
    fn air_runs_opencode_on_gateway_config() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        // 逐段拼：Windows 上 join(".config/JetBrains/Air") 会留下混用的分隔符，和实现拼出的字符串对不上。
        let dir = temp.path().join(".config").join("JetBrains").join("Air");
        write_file(
            &dir.join("acp.json"),
            r#"{"agent_servers":{"Mine":{"command":"x"}}}"#,
        );
        let files = connect(&JETBRAINS_AIR, &env, "jetbrains-air");
        let acp = json_of(&files[&dir.join("acp.json")]);
        assert_eq!(acp["agent_servers"]["Mine"]["command"], "x");
        let ours = &acp["agent_servers"]["Codex++"];
        assert_eq!(ours["args"][0], "acp");
        assert_eq!(
            ours["env"]["OPENCODE_CONFIG"],
            dir.join("codexplus-opencode.json")
                .to_string_lossy()
                .as_ref()
        );
        let config = json_of(&files[&dir.join("codexplus-opencode.json")]);
        assert_eq!(config["enabled_providers"][0], "codexplus");
        assert_eq!(config["model"], "codexplus/ds/deepseek-chat");
    }

    /// 已有用户配置时接入再断开：每个改过的文件都逐字节回到接入前（带注释的原文也一样）。
    #[test]
    fn existing_user_configs_restore_exactly() {
        use crate::gateway::agents::files::{AgentStash, restored_text};

        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        let user = temp.path().join(".config/Code/User");
        write_file(
            &user.join("settings.json"),
            "{\n  // 我的设置\n  \"editor.fontSize\": 14,\n}\n",
        );
        write_file(
            &user.join("chatLanguageModels.json"),
            "[\n  // 自己的\n  {\"name\": \"mine\", \"vendor\": \"customendpoint\"},\n]\n",
        );
        write_file(
            &raycast_path(&env),
            "# 头\nproviders:\n  - id: mine\n    name: Mine\n\n# 尾\ntools: {}\n",
        );
        write_file(
            &temp
                .path()
                .join(".config/Claude-3p/configLibrary/_meta.json"),
            "{\"appliedId\":\"mine\",\"entries\":[{\"id\":\"mine\"}]}",
        );
        write_file(
            &temp.path().join(".config/zed/settings.json"),
            "// Zed\n{\n  \"theme\": \"One Dark\",\n}\n",
        );
        for spec in [&VSCODE, &RAYCAST, &CLAUDE_DESKTOP, &ZED, &ZOO_CODE] {
            let mut set = FileSet::new();
            (spec.write)(&env, &wiring(spec.id), &mut set).unwrap();
            let changes = set.changes().unwrap();
            assert!(!changes.is_empty(), "{} 没有写任何文件", spec.id);
            let mut stash = AgentStash::default();
            for change in &changes {
                stash.record(change).unwrap();
            }
            for change in &changes {
                let file = &stash.files[&change.path];
                assert_eq!(
                    restored_text(file, change.after.as_deref()).unwrap(),
                    change.before,
                    "{} 的 {} 没有还原",
                    spec.id,
                    change.path.display()
                );
            }
        }
    }
}
