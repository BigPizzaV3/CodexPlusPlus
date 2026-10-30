//! 「对话造拓展」：给 Codex 用的本机 stdio MCP 服务。
//!
//! 用户在 Codex 对话里描述想要的功能，agent 通过这里的工具写出 Codex++ 拓展包、立刻热重载、
//! 读回加载状态与报错再迭代。走 MCP 而不是让 agent 自己写文件、跑命令，是因为 Codex 的沙箱通常
//! 不能写工作区以外的目录，也连不上本机调试端口；MCP 服务在沙箱外运行，写入还会经过 Codex 的工具审批。
//!
//! 只管理 user 目录下的拓展包（`user_scripts/<id>/`），不删除文件，不碰单文件脚本与内置脚本。

use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::managed_mcp::ManagedMcp;
use crate::user_scripts::{self, ExtensionWrite, UserScriptManager};

pub const SERVER_ID: &str = "codex_plus_extensions";
pub const MCP_FLAG: &str = "--codex-plus-extension-mcp";
/// 早期接入时写下的条目与启动参数。重新接入时替换掉本工具的旧条目，主程序也继续认旧参数。
pub const LEGACY_SERVER_ID: &str = "codex_plus_plugins";
pub const LEGACY_MCP_FLAG: &str = "--codex-plus-plugin-mcp";

pub const SPEC: ManagedMcp = ManagedMcp {
    server_id: SERVER_ID,
    flag: MCP_FLAG,
    label: "拓展创作 MCP",
    comment: "Managed by Codex++: extension creator MCP",
    lock_file: ".codex-plus-extension-mcp.lock",
    backup_dir: ".codex-plus-extension-mcp-backups",
    legacy: &[(LEGACY_SERVER_ID, LEGACY_MCP_FLAG)],
};

/// 写入请求里带着完整的文件内容，上限比单文件上限宽一些。
const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
/// 热重载后再等一会儿读一次运行状态，捕捉拓展异步阶段报的错。
const SETTLE_DELAY: Duration = Duration::from_millis(800);

const GUIDE: &str = include_str!("../../../assets/extension-creator/guide.md");
/// 只读页面查看脚本：以 `(脚本)(参数)` 求值，agent 只能传参数，不能传代码。
const INSPECT_SCRIPT: &str = include_str!("../../../assets/extension-creator/inspect.js");
/// 截图宽度上限（CSS 像素），超出时按比例缩小，免得图片撑爆对话。
const SCREENSHOT_MAX_WIDTH: f64 = 1400.0;
const EXTENSION_API_SOURCE: &str =
    include_str!("../../../assets/inject/renderer-inject/91-extension-api.js");

const INSTRUCTIONS: &str = "用户想给 Codex 加功能、改界面、做个小工具，或说「做个插件 / 做个拓展」时，用这些工具在对话里直接做出 Codex++ 拓展（注意：这不是 Codex 自带的插件，Codex 插件是 skills/MCP 包，不归这里管）：先调用 codexpp_extension_guide 读创作指南（只需一次）；要改 Codex 原生界面时先用 codexpp_extension_inspect 查清页面结构，不要猜选择器；再用 codexpp_extension_write 写入，写完用 codexpp_extension_screenshot 截图确认效果，根据返回的加载状态和报错迭代到能用为止，最后告诉用户在哪里使用。改已有拓展前先 codexpp_extension_read。拓展在用户自己的 Codex 页面里运行，写入前确认需求，不要做用户没要求的事。跟用户交流时称它为「拓展」。";

/// 服务依赖：拓展目录与 Codex 页面调试端口。端口为空时只写文件、不热重载。
pub struct ExtensionCreator {
    pub manager: UserScriptManager,
    pub debug_ports: Vec<u16>,
}

pub async fn serve(creator: ExtensionCreator) -> anyhow::Result<()> {
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<Option<Vec<u8>>>(4);
    // 读 stdin 是阻塞的，放到独立线程；超长行用 None 表示，由主循环回错误。
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        loop {
            match read_bounded_line(&mut reader) {
                Ok(Some(line)) => {
                    if sender.blocking_send(line).is_err() {
                        break;
                    }
                }
                Ok(None) | Err(_) => break,
            }
        }
    });
    while let Some(line) = receiver.recv().await {
        let response = match line {
            None => Some(rpc_error(Value::Null, -32600, "请求行超过 2 MiB 上限")),
            Some(line) if line.iter().all(u8::is_ascii_whitespace) => None,
            Some(line) => match serde_json::from_slice::<Value>(&line) {
                Ok(request) => handle_request(&creator, request).await,
                Err(_) => Some(rpc_error(Value::Null, -32700, "JSON 解析失败")),
            },
        };
        if let Some(response) = response {
            let changes_tools = response_changes_tools(&response);
            let stdout = std::io::stdout();
            let mut output = stdout.lock();
            serde_json::to_writer(&mut output, &response).context("无法写入 MCP 响应")?;
            output.write_all(b"\n")?;
            // 拓展写入、启停、重载后，它声明的工具可能变了：通知 Codex 重新拉工具列表。
            if changes_tools {
                output.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n")?;
            }
            output.flush()?;
        }
    }
    Ok(())
}

/// 响应是否来自会改变拓展工具列表的调用（写入、启停、重载），据此发 list_changed 通知。
fn response_changes_tools(response: &Value) -> bool {
    response
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|value| value.get("reload").is_some())
}

/// 读一行；超过上限时丢弃整行余下部分并返回 `Some(None)`，EOF 返回 `None`。
fn read_bounded_line(reader: &mut impl BufRead) -> std::io::Result<Option<Option<Vec<u8>>>> {
    let mut line = Vec::new();
    let read = reader
        .by_ref()
        .take(MAX_REQUEST_BYTES as u64 + 1)
        .read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > MAX_REQUEST_BYTES && line.last() != Some(&b'\n') {
        let mut rest = Vec::new();
        reader.read_until(b'\n', &mut rest)?;
        return Ok(Some(None));
    }
    Ok(Some(Some(line)))
}

pub async fn handle_request(creator: &ExtensionCreator, request: Value) -> Option<Value> {
    let Some(object) = request.as_object() else {
        return Some(rpc_error(Value::Null, -32600, "请求必须是 JSON-RPC 对象"));
    };
    // 通知没有响应。
    let id = object.get("id").cloned()?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(rpc_error(Value::Null, -32600, "JSON-RPC 请求格式无效"));
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(rpc_error(id, -32600, "缺少请求方法"));
    };
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2024-11-05");
            let version = if matches!(requested, "2024-11-05" | "2025-03-26" | "2025-06-18") {
                requested
            } else {
                "2025-06-18"
            };
            json!({"protocolVersion": version, "capabilities": {"tools": {"listChanged": true}},
                "serverInfo": {"name": "codex-plus-extension-creator", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS})
        }
        "ping" => json!({}),
        "tools/list" => {
            let mut tools = tool_definitions().as_array().cloned().unwrap_or_default();
            tools.extend(extension_tool_definitions(&creator.manager));
            json!({ "tools": tools })
        }
        "tools/call" => match params.get("name").and_then(Value::as_str) {
            Some(name) => tool_result(
                call_tool(
                    creator,
                    name,
                    params.get("arguments").cloned().unwrap_or_else(|| json!({})),
                )
                .await,
            ),
            None => return Some(rpc_error(id, -32602, "tools/call 缺少工具名")),
        },
        _ => return Some(rpc_error(id, -32601, "未实现的 MCP 方法")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// 工具结果里带这个键时，直接作为 MCP content 返回（截图要返回图片块）。
const MCP_CONTENT_KEY: &str = "__mcpContent";

fn tool_result(result: anyhow::Result<Value>) -> Value {
    match result {
        Ok(Value::Object(mut object)) if object.contains_key(MCP_CONTENT_KEY) => {
            json!({"content": object.remove(MCP_CONTENT_KEY).unwrap_or(Value::Null), "isError": false})
        }
        Ok(value) => json!({"content": [{"type": "text", "text": value.to_string()}], "isError": false}),
        Err(error) => json!({"content": [{"type": "text", "text": format!("{error:#}")}], "isError": true}),
    }
}

fn tool_definitions() -> Value {
    let read_only = json!({"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false});
    let writes = json!({"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false});
    json!([
        {
            "name": "codexpp_extension_guide",
            "description": "读取 Codex++ 拓展创作指南：拓展能做什么、ctx API、必须遵守的约定、示例，以及可用的后端路由。写拓展前先调用一次。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": read_only,
        },
        {
            "name": "codexpp_extension_list",
            "description": "列出用户已有的 Codex++ 拓展与脚本，含启用状态、加载状态和报错。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": read_only,
        },
        {
            "name": "codexpp_extension_read",
            "description": "读回一个拓展包的清单与源码，修改已有拓展前使用。",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "string", "description": "拓展 id"}},
                "required": ["id"],
                "additionalProperties": false,
            },
            "annotations": read_only,
        },
        {
            "name": "codexpp_extension_inspect",
            "description": "只读查看用户正在运行的 Codex 页面结构，写改动 Codex 原生界面的拓展前先用它认路。不给条件时返回关键区域概览（输入框、输入区按钮、图标栏、顶栏）；给 query 按 aria-label / title / 文字查找，给 selector 按 CSS 选择器查找。每个元素返回稳定属性、建议选择器（只用稳定属性拼成）与带构建哈希、不可依赖的类名。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "按 aria-label、title、placeholder 或文字查找，例如「发送」"},
                    "selector": {"type": "string", "description": "CSS 选择器"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": "最多返回几个元素，默认 20"},
                    "visibleOnly": {"type": "boolean", "description": "只返回可见元素，默认 true"},
                    "includeCodexPlus": {"type": "boolean", "description": "是否包含 Codex++ 与拓展自己插入的节点，默认 false"}
                },
                "additionalProperties": false,
            },
            "annotations": read_only,
        },
        {
            "name": "codexpp_extension_screenshot",
            "description": "截取用户正在运行的 Codex 页面：给 selector 截那个元素（可加 padding 留边），不给截整个窗口。写完拓展后用它确认效果。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": {"type": "string", "description": "要截取的元素的 CSS 选择器；省略则截整个窗口"},
                    "padding": {"type": "integer", "minimum": 0, "maximum": 400, "description": "元素四周留出的像素，默认 16"}
                },
                "additionalProperties": false,
            },
            "annotations": read_only,
        },
        {
            "name": "codexpp_extension_write",
            "description": "创建或覆盖一个 Codex++ 拓展包并立刻热重载，返回加载状态与报错。codexpp.json 由本工具根据参数生成；同一 id 重复写入即覆盖，用于迭代修改。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "拓展 id：字母、数字、- 和 _，最长 64"},
                    "name": {"type": "string", "description": "给用户看的名字"},
                    "description": {"type": "string", "description": "一句话说明拓展做什么"},
                    "version": {"type": "string", "description": "版本号，默认 0.1.0"},
                    "routes": {"type": "array", "items": {"type": "string"}, "description": "拓展需要通过 ctx.call 调用的后端路由，只能从指南列出的可用路由里选；不需要就省略"},
                    "renderer": {"type": ["array", "string"], "items": {"type": "string"}, "description": "按顺序执行的页面入口文件；省略时若 files 里有 renderer.js 就用它，只声明 hooks 的拓展可以没有页面入口"},
                    "hooks": {"type": "object", "description": "Codex 钩子声明，例如 {\"PreToolUse\": [{\"matcher\": \"^Bash$\", \"script\": \"hooks/guard.mjs\", \"timeout\": 30}]}；写法见指南「Codex 钩子」"},
                    "toolPipeline": {"type": "object", "description": "工具管道声明，例如 {\"preExecute\": [{\"matcher\": \"^Bash$\", \"script\": \"tools/pre.mjs\"}], \"execute\": [...], \"postExecute\": [...]}：preExecute 改参数或拦截，execute 代为执行，postExecute 改写工具结果。写法见指南「工具管道」"},
                    "backend": {"type": "object", "description": "常驻后台声明，例如 {\"script\": \"backend/main.mjs\"}；脚本默认导出 async (ctx) => ({ 方法 })，页面用 ctx.backend.call / on 调用。写法见指南「常驻后台」"},
                    "config": {"type": "object", "description": "配置项声明，例如 {\"minutes\": {\"type\": \"number\", \"title\": \"时长（分钟）\", \"default\": 25}}；type 可选 string / number / boolean / select（配 options）。用户可在管理器里改，脚本里读 ctx.config / context.config"},
                    "tools": {"type": "array", "description": "给模型用的工具声明，例如 [{\"name\": \"lookup\", \"description\": \"…\", \"inputSchema\": {\"type\": \"object\", \"properties\": {…}}, \"script\": \"tools/lookup.mjs\", \"timeout\": 30}]；写法见指南「给模型加工具」", "items": {"type": "object"}},
                    "gateway": {"type": "object", "description": "网关中间件声明，例如 {\"request\": [{\"script\": \"gateway/request.mjs\"}], \"response\": [{\"script\": \"gateway/response.mjs\"}]}；request 改写发给模型的请求，response 改写模型回复的事件流。写法见指南「网关请求中间件」「网关响应流中间件」"},
                    "files": {
                        "type": "array",
                        "description": "要写入的文件，不要包含 codexpp.json。文本给 content；图片、音频、字体等资源给 sourcePath（本机文件的绝对路径，按字节复制，放在 assets/ 下，页面里用 ctx.asset(\"assets/xx.png\") 取地址）。不要把 base64 写进代码",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string", "description": "包内相对路径"},
                                "content": {"type": "string", "description": "文本文件的完整内容"},
                                "sourcePath": {"type": "string", "description": "二进制资源：本机文件绝对路径（png / jpg / gif / webp / svg / mp3 / wav / ogg / woff2 等，单个最大 1 MiB）"},
                            },
                            "required": ["path"],
                            "additionalProperties": false,
                        },
                    },
                },
                "required": ["id", "name", "files"],
                "additionalProperties": false,
            },
            "annotations": writes,
        },
        {
            "name": "codexpp_extension_set_enabled",
            "description": "启用或停用一个拓展包并热重载。用户不想要某个拓展时用它停用（本工具不删除文件）。",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "string"}, "enabled": {"type": "boolean"}},
                "required": ["id", "enabled"],
                "additionalProperties": false,
            },
            "annotations": writes,
        },
        {
            "name": "codexpp_extension_configure",
            "description": "修改一个拓展包的配置值（只能改清单 config 里声明过的项），保存后热重载。用户说「把 X 改成 Y」且 X 是拓展配置时用它，不必重写代码。",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "string"}, "values": {"type": "object", "description": "要设置的配置项与值"}},
                "required": ["id", "values"],
                "additionalProperties": false,
            },
            "annotations": writes,
        },
        {
            "name": "codexpp_extension_reload",
            "description": "热重载所有拓展并返回加载状态，用于用户手动改过文件之后。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": writes,
        },
    ])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteArguments {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    routes: Vec<String>,
    #[serde(default, deserialize_with = "string_or_list")]
    renderer: Vec<String>,
    #[serde(default)]
    hooks: Option<Value>,
    #[serde(default)]
    gateway: Option<Value>,
    #[serde(default)]
    tools: Option<Value>,
    #[serde(default)]
    config: Option<Value>,
    #[serde(default)]
    backend: Option<Value>,
    #[serde(default, rename = "toolPipeline")]
    tool_pipeline: Option<Value>,
    #[serde(deserialize_with = "file_list")]
    files: Vec<WriteFile>,
}

/// 一个要写入的文件：文本给 `content`，图片 / 音频 / 字体等二进制资源给 `sourcePath`（本机文件，按字节复制）。
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WriteFile {
    path: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    source_path: Option<String>,
}

/// `renderer` 既收数组也收单个字符串（清单本身两种写法都认，agent 常写成字符串）。
fn string_or_list<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Either::deserialize(deserializer)? {
        Either::One(value) => vec![value],
        Either::Many(values) => values,
    })
}

/// `files` 既收 `[{ path, content | sourcePath }]`，也收 `{ "路径": "内容" }`。
fn file_list<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<WriteFile>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        List(Vec<WriteFile>),
        Map(BTreeMap<String, String>),
    }
    Ok(match Either::deserialize(deserializer)? {
        Either::List(files) => files,
        Either::Map(files) => files
            .into_iter()
            .map(|(path, content)| WriteFile { path, content: Some(content), source_path: None })
            .collect(),
    })
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct InspectArguments {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    visible_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    include_codex_plus: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScreenshotArguments {
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    padding: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdArguments {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigureArguments {
    id: String,
    values: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnabledArguments {
    id: String,
    enabled: bool,
}

async fn call_tool(creator: &ExtensionCreator, name: &str, arguments: Value) -> anyhow::Result<Value> {
    match name {
        "codexpp_extension_guide" => Ok(json!({
            "guide": GUIDE,
            "apiVersion": user_scripts::EXTENSION_API_VERSION,
            "availableRoutes": extension_routes(),
            "extensionsDirectory": creator.manager.user_dir().to_string_lossy(),
        })),
        "codexpp_extension_list" => {
            let live = live_status(&creator.debug_ports).await.map(|(_, status)| status);
            let inventory = creator.manager.inventory_with_runtime_status(live.as_ref())?;
            Ok(json!({
                "extensionsDirectory": creator.manager.user_dir().to_string_lossy(),
                "codexConnected": live.is_some(),
                "items": summarize_items(&inventory, None),
            }))
        }
        "codexpp_extension_read" => {
            let arguments: IdArguments =
                serde_json::from_value(arguments).context("codexpp_extension_read 参数无效")?;
            creator.manager.read_extension_package(&arguments.id)
        }
        "codexpp_extension_write" => {
            let arguments: WriteArguments =
                serde_json::from_value(arguments).context("codexpp_extension_write 参数无效")?;
            let mut files = BTreeMap::new();
            let mut assets = BTreeMap::new();
            for file in arguments.files {
                let duplicate = match (file.content, file.source_path) {
                    (Some(content), None) => files.insert(file.path.clone(), content).is_some(),
                    (None, Some(source)) => assets.insert(file.path.clone(), std::path::PathBuf::from(source)).is_some(),
                    _ => anyhow::bail!("文件 {} 要给 content（文本）或 sourcePath（本机文件）其中之一", file.path),
                };
                if duplicate || (files.contains_key(&file.path) && assets.contains_key(&file.path)) {
                    anyhow::bail!("文件 {} 重复出现", file.path);
                }
            }
            let written = creator.manager.write_extension_package(&ExtensionWrite {
                id: arguments.id.clone(),
                name: arguments.name,
                version: arguments.version,
                description: arguments.description,
                routes: arguments.routes,
                renderer: arguments.renderer,
                hooks: arguments.hooks,
                gateway: arguments.gateway,
                tools: arguments.tools,
                config: arguments.config,
                backend: arguments.backend,
                tool_pipeline: arguments.tool_pipeline,
                files,
                assets,
            })?;
            let key = user_scripts::extension_package_key(&arguments.id);
            let mut result = reload_and_report(creator, &key).await?;
            result["written"] = written;
            Ok(result)
        }
        "codexpp_extension_set_enabled" => {
            let arguments: EnabledArguments =
                serde_json::from_value(arguments).context("codexpp_extension_set_enabled 参数无效")?;
            if !creator.manager.has_extension_package(&arguments.id) {
                anyhow::bail!("没有 id 为 {} 的用户拓展包", arguments.id);
            }
            let key = user_scripts::extension_package_key(&arguments.id);
            creator.manager.set_script_enabled(&key, arguments.enabled)?;
            reload_and_report(creator, &key).await
        }
        "codexpp_extension_reload" => reload_and_report(creator, "").await,
        "codexpp_extension_configure" => {
            let arguments: ConfigureArguments =
                serde_json::from_value(arguments).context("codexpp_extension_configure 参数无效")?;
            let key = user_scripts::extension_package_key(&arguments.id);
            let config = creator.manager.set_extension_config(&key, &arguments.values)?;
            let mut result = reload_and_report(creator, &key).await?;
            result["config"] = config;
            Ok(result)
        }
        name if name.contains("__") => call_extension_tool(creator, name, arguments).await,
        "codexpp_extension_inspect" => {
            let arguments: InspectArguments =
                serde_json::from_value(arguments).context("codexpp_extension_inspect 参数无效")?;
            let websocket = codex_page_websocket(&creator.debug_ports).await?;
            evaluate_value(&websocket, &format!("({})({})", INSPECT_SCRIPT.trim(), serde_json::to_string(&arguments)?)).await
        }
        "codexpp_extension_screenshot" => {
            let arguments: ScreenshotArguments =
                serde_json::from_value(arguments).context("codexpp_extension_screenshot 参数无效")?;
            screenshot(&creator.debug_ports, arguments).await
        }
        _ => anyhow::bail!("未知工具 {name}"),
    }
}

/// 热重载并汇报。目标 key 非空时把它的状态单独提出来，方便 agent 判断要不要继续修。
async fn reload_and_report(creator: &ExtensionCreator, key: &str) -> anyhow::Result<Value> {
    let reload = reload(creator).await;
    let mut live = None;
    if reload["reloaded"] == true {
        tokio::time::sleep(SETTLE_DELAY).await;
        live = live_status(&creator.debug_ports).await.map(|(_, status)| status);
    }
    let inventory = creator.manager.inventory_with_runtime_status(live.as_ref())?;
    let items = summarize_items(&inventory, None);
    let target = (!key.is_empty())
        .then(|| summarize_items(&inventory, Some(key)).into_iter().next())
        .flatten();
    // 拓展声明的 Codex 钩子：让 Codex 配置里的分发条目与之一致。新增条目时提醒用户去信任。
    let mut hooks = crate::extension_hooks::sync_default(&creator.manager);
    if hooks["added"].as_array().is_some_and(|added| !added.is_empty()) {
        hooks["note"] = json!("Codex 配置里新增了 Codex++ 拓展钩子的分发条目：Codex 会在用户信任后才运行它们（每个事件信任一次），请提醒用户在 Codex 的 Hooks 设置里信任「Codex++ 拓展钩子」，并新开对话生效。");
    }
    // 写入 / 启停 / 配置单个拓展时只回这一个，不带全部列表（几十个拓展的摘要会白白占满 agent 的上下文）。
    let mut result = json!({ "reload": reload, "extension": target, "hooks": hooks });
    if key.is_empty() {
        result["items"] = json!(items);
    }
    Ok(result)
}

/// 只有页内重载才自动执行。某个已加载脚本没登记清理函数时，重载会整页刷新 Codex，
/// 可能打断用户正在进行的对话，这种情况交给用户在管理器里手动点。
async fn reload(creator: &ExtensionCreator) -> Value {
    let mut last_error = String::from("没有找到已连接的 Codex 页面；拓展文件已写好，Codex 下次启动或在 Codex++ 管理器点「热重载」后生效");
    for port in &creator.debug_ports {
        let Ok(status) = user_scripts::live_runtime_status(*port).await else {
            continue;
        };
        let blockers = status
            .as_object()
            .map(|scripts| {
                scripts
                    .iter()
                    .filter(|(_, record)| {
                        record
                            .get("cleanups")
                            .and_then(Value::as_array)
                            .is_none_or(Vec::is_empty)
                    })
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !blockers.is_empty() {
            return json!({
                "reloaded": false,
                "reason": "有旧脚本不支持页内热重载，自动重载会刷新整个 Codex 页面、可能打断当前对话，所以没有自动执行。拓展文件已写好，请让用户在 Codex++ 管理器的「本地拓展」里点「热重载」。",
                "blockingScripts": blockers,
            });
        }
        match user_scripts::reload_live_scripts(*port, &creator.manager).await {
            Ok(inventory) => {
                return json!({ "reloaded": true, "mode": inventory["reload_mode"] });
            }
            Err(error) => last_error = format!("热重载失败：{error:#}"),
        }
    }
    json!({ "reloaded": false, "reason": last_error })
}

/// 启用中拓展声明的工具，以 MCP 工具定义返回。名字是 `<拓展 id>__<工具名>`。
fn extension_tool_definitions(manager: &UserScriptManager) -> Vec<Value> {
    manager
        .enabled_extension_tools()
        .unwrap_or_default()
        .into_iter()
        .map(|(id, tool)| {
            json!({
                "name": user_scripts::extension_tool_public_name(&id, &tool.name),
                "description": format!("[Codex++ 拓展 {id}] {}", tool.description),
                "inputSchema": tool.input_schema,
            })
        })
        .collect()
}

/// stdin：`{ script, args, extension, tool, timeoutMs }`；调用默认导出，stdout：`{ result }` 或 `{ error }`。
const TOOL_RUNNER: &str = r#"
import { pathToFileURL } from "node:url";
import readline from "node:readline";
const write = (value) => process.stdout.write(JSON.stringify(value) + "\n");
const lines = readline.createInterface({ input: process.stdin });
const hostPending = new Map();
let hostSeq = 0;
const host = (service, method, params) => new Promise((resolve, reject) => {
  const id = ++hostSeq;
  hostPending.set(id, { resolve, reject });
  write({ host: id, service, method, params: params ?? null });
});
const withId = (method) => (id, options) => host("sessionQuery", method, { id, options: options ?? {} });
const readSession = (id) => host("sessionQuery", "readSession", { id });
const readEventsSince = (query) => host("sessionQuery", "readEventsSince", query ?? {});
// 把会话事件日志折叠成状态；apply(state, event) 返回 undefined 视为不变。
const project = async (id, { init, apply } = {}) => {
  if (typeof apply !== "function") throw new Error("project 需要 apply(state, event)");
  const { events } = await readSession(id);
  let state = typeof init === "function" ? init() : init;
  for (const event of events) { const next = apply(state, event); if (next !== undefined) state = next; }
  return state;
};
// 只读会话查询，和页面、后台里的 sessionQuery 同一套方法。
const sessionQuery = Object.freeze({
  listSessions: (options) => host("sessionQuery", "listSessions", options ?? {}),
  filterSessions: (filters) => host("sessionQuery", "filterSessions", filters ?? {}),
  readSession,
  readTitle: (id) => host("sessionQuery", "readTitle", { id }),
  listEvents: withId("listEvents"),
  filterEvents: withId("filterEvents"),
  readEvent: (query) => host("sessionQuery", "readEvent", query ?? {}),
  readEventsSince,
  project,
  searchSessions: (query) => host("sessionQuery", "searchSessions", query ?? {}),
  searchEvents: (query) => host("sessionQuery", "searchEvents", query ?? {}),
});
let input = null;
lines.on("line", async (line) => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if (message.hostReply !== undefined) {
    const waiter = hostPending.get(message.hostReply);
    hostPending.delete(message.hostReply);
    if (message.error !== undefined) waiter?.reject(new Error(message.error)); else waiter?.resolve(message.result);
    return;
  }
  if (input) return;
  input = message;
  let timer;
  try {
    const module = await import(pathToFileURL(input.script).href);
    const fn = typeof module.default === "function" ? module.default : module[input.tool];
    if (typeof fn !== "function") throw new Error(`${input.script} 没有默认导出函数`);
    const timeout = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`超时（${input.timeoutMs / 1000} 秒）`)), input.timeoutMs); });
    const result = await Promise.race([Promise.resolve(fn(input.args ?? {}, { extension: input.extension, tool: input.tool, config: input.config ?? {}, sessionQuery })), timeout]);
    write({ final: { result: result === undefined ? null : result } });
  } catch (error) {
    write({ final: { error: String(error?.stack || error?.message || error) } });
  } finally {
    clearTimeout(timer);
    process.exit(0);
  }
});
"#;

/// 执行一个拓展工具。结果约定：字符串 → 文本；`{ content: [...] }` → 原样作为 MCP 内容块；
/// 其它值 → JSON 文本。脚本抛错 → isError。
async fn call_extension_tool(creator: &ExtensionCreator, name: &str, arguments: Value) -> anyhow::Result<Value> {
    let (id, tool) = creator
        .manager
        .enabled_extension_tools()?
        .into_iter()
        .find(|(id, tool)| user_scripts::extension_tool_public_name(id, &tool.name) == name)
        .with_context(|| format!("未知工具 {name}（对应的拓展可能已停用或删除）"))?;
    let node = crate::extension_node::require_node()?;
    let input = json!({
        "script": tool.script,
        "args": arguments,
        "extension": id,
        "tool": tool.name,
        "timeoutMs": tool.timeout_secs * 1000,
        "config": tool.config,
    });
    let output = crate::extension_node::run_runner_with_host(
        &node,
        TOOL_RUNNER,
        &input,
        Duration::from_secs(tool.timeout_secs + 5),
    )
    .await?;
    if let Some(error) = output.get("error").and_then(Value::as_str) {
        anyhow::bail!("拓展 {id} 的工具 {} 出错：{error}", tool.name);
    }
    let result = output.get("result").cloned().unwrap_or(Value::Null);
    let content = match &result {
        Value::String(text) => json!([{ "type": "text", "text": text }]),
        Value::Object(object) if object.get("content").is_some_and(Value::is_array) => object["content"].clone(),
        other => json!([{ "type": "text", "text": other.to_string() }]),
    };
    Ok(json!({ MCP_CONTENT_KEY: content }))
}

/// 用户正在运行的 Codex 主页面的 CDP 地址。只认 Codex 主页面，不碰管理器或其他窗口。
async fn codex_page_websocket(ports: &[u16]) -> anyhow::Result<String> {
    for port in ports {
        let Ok(targets) = crate::cdp::list_targets(*port).await else {
            continue;
        };
        if let Some(websocket) = crate::cdp::pick_injectable_codex_page_target(&targets)
            .ok()
            .and_then(|target| target.web_socket_debugger_url)
        {
            return Ok(websocket);
        }
    }
    anyhow::bail!("没有找到已连接的 Codex 页面：请确认 Codex 由 Codex++ 启动并且正在运行")
}

async fn evaluate_value(websocket: &str, expression: &str) -> anyhow::Result<Value> {
    let response = crate::bridge::evaluate_script_return_by_value(websocket, expression).await?;
    if let Some(exception) = response.pointer("/result/exceptionDetails") {
        anyhow::bail!(
            "页面脚本出错：{}",
            exception
                .pointer("/exception/description")
                .and_then(Value::as_str)
                .unwrap_or("renderer exception")
        );
    }
    Ok(response
        .pointer("/result/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}

async fn screenshot(ports: &[u16], arguments: ScreenshotArguments) -> anyhow::Result<Value> {
    let websocket = codex_page_websocket(ports).await?;
    let padding = f64::from(arguments.padding.unwrap_or(16));
    // 只读：取元素或视口的位置，不改页面。选择器作为 JSON 字符串传入，不拼进代码。
    let probe = format!(
        r#"(() => {{
  const selector = {selector};
  const viewport = {{ x: 0, y: 0, width: innerWidth, height: innerHeight }};
  if (!selector) return {{ found: true, rect: viewport, viewport }};
  let el = null;
  try {{ el = document.querySelector(selector); }} catch (error) {{ return {{ found: false, error: "选择器无效：" + error.message }}; }}
  if (!el) return {{ found: false, error: "页面上没有匹配的元素" }};
  const r = el.getBoundingClientRect();
  return {{ found: r.width > 0 && r.height > 0, error: r.width > 0 && r.height > 0 ? "" : "元素不可见（宽或高为 0）", rect: {{ x: r.x, y: r.y, width: r.width, height: r.height }}, viewport }};
}})()"#,
        selector = serde_json::to_string(&arguments.selector.clone().unwrap_or_default())?
    );
    let located = evaluate_value(&websocket, &probe).await?;
    if located["found"] != true {
        anyhow::bail!("{}", located["error"].as_str().unwrap_or("无法定位截图区域"));
    }
    let number = |value: &Value| value.as_f64().unwrap_or(0.0);
    let (view_width, view_height) = (number(&located["viewport"]["width"]), number(&located["viewport"]["height"]));
    let rect = &located["rect"];
    let x = (number(&rect["x"]) - padding).max(0.0);
    let y = (number(&rect["y"]) - padding).max(0.0);
    let width = (number(&rect["x"]) + number(&rect["width"]) + padding).min(view_width) - x;
    let height = (number(&rect["y"]) + number(&rect["height"]) + padding).min(view_height) - y;
    if width <= 0.0 || height <= 0.0 {
        anyhow::bail!("截图区域在窗口之外");
    }
    let scale = (SCREENSHOT_MAX_WIDTH / width).min(1.0);
    let response = crate::bridge::send_cdp_command(
        &websocket,
        "Page.captureScreenshot",
        json!({
            "format": "png",
            "captureBeyondViewport": false,
            "clip": { "x": x, "y": y, "width": width, "height": height, "scale": scale },
        }),
    )
    .await?;
    let data = response
        .pointer("/result/data")
        .and_then(Value::as_str)
        .filter(|data| !data.is_empty())
        .context("Codex 页面没有返回截图")?;
    Ok(json!({
        MCP_CONTENT_KEY: [
            { "type": "image", "data": data, "mimeType": "image/png" },
            { "type": "text", "text": json!({ "selector": arguments.selector, "clip": { "x": x.round(), "y": y.round(), "width": width.round(), "height": height.round() }, "scale": scale }).to_string() },
        ]
    }))
}

async fn live_status(ports: &[u16]) -> Option<(u16, Value)> {
    for port in ports {
        if let Ok(status) = user_scripts::live_runtime_status(*port).await {
            return Some((*port, status));
        }
    }
    None
}

/// 清单只保留 agent 判断需要的字段；`only` 非空时只返回那一项。
fn summarize_items(inventory: &Value, only: Option<&str>) -> Vec<Value> {
    inventory["scripts"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|item| only.is_none_or(|key| item["key"] == key))
                .map(|item| {
                    json!({
                        "key": item["key"],
                        "name": item["name"],
                        "kind": item["kind"],
                        "source": item["source"],
                        "enabled": item["enabled"],
                        "status": item["status"],
                        "error": item["error"],
                        "package": item["package"],
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 从拓展接口层源码里读路由白名单，免得两处维护漂移。
fn extension_routes() -> Vec<String> {
    let Some(start) = EXTENSION_API_SOURCE.find("const codexPlusExtensionRoutes = new Set([") else {
        return Vec::new();
    };
    let Some(end) = EXTENSION_API_SOURCE[start..].find("]);") else {
        return Vec::new();
    };
    EXTENSION_API_SOURCE[start..start + end]
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('"'))
        .filter_map(|line| line.trim_end_matches(',').trim_matches('"').split('"').next())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_whitelist_is_read_from_extension_api() {
        let routes = extension_routes();
        assert!(routes.contains(&"/diagnostics/log".to_string()));
        assert!(!routes.iter().any(|route| route == "/settings/set"));
    }

    #[test]
    fn bounded_reader_rejects_oversized_lines_and_keeps_going() {
        let mut input = vec![b'x'; MAX_REQUEST_BYTES + 10];
        input.push(b'\n');
        input.extend_from_slice(b"{\"ok\":1}\n");
        let mut reader = std::io::BufReader::new(&input[..]);
        assert!(matches!(read_bounded_line(&mut reader).unwrap(), Some(None)));
        assert_eq!(read_bounded_line(&mut reader).unwrap(), Some(Some(b"{\"ok\":1}\n".to_vec())));
        assert_eq!(read_bounded_line(&mut reader).unwrap(), None);
    }
}
