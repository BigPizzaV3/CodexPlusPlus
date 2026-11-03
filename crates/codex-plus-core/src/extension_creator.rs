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

mod context;

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

const INSTRUCTIONS: &str = "用户想给 Codex 加功能、改界面、做个小工具，或说「做个插件 / 做个拓展」时，用这些工具在对话里直接做出 Codex++ 拓展（注意：这不是 Codex 自带的插件，Codex 插件是 skills/MCP 包，不归这里管）：先调用 codexpp_extension_guide 读创作指南（只需一次）；要改 Codex 原生界面时先用 codexpp_extension_inspect 查清页面结构，不要猜选择器；再用 codexpp_extension_write 写入，写完用 codexpp_extension_screenshot 截图确认效果，根据返回的加载状态和报错迭代到能用为止，最后告诉用户在哪里使用。同一个对话默认继续修改已关联的拓展，保留已有功能；先 codexpp_extension_read 读取当前代码，read/write 省略 id 即使用本对话关联。只有用户明确说新建另一个拓展才传 newExtension: true；不要因新增需求或遇到错误而自行新建。用户明确要修改另一个已有拓展时，read 传 id 和 select: true 切换本对话目标。关联按工具调用携带的 threadId 隔离并持久保存，不得猜测对话 ID 或借用其他对话的目标。拓展在用户自己的 Codex 页面里运行，写入前确认需求，不要做用户没要求的事。跟用户交流时称它为「拓展」。";

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
            json!({"protocolVersion": version, "capabilities": {"tools": {"listChanged": true}, "prompts": {"listChanged": false}, "resources": {"listChanged": false}},
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
                    params.get("_meta").unwrap_or(&Value::Null),
                )
                .await,
            ),
            None => return Some(rpc_error(id, -32602, "tools/call 缺少工具名")),
        },
        "prompts/list" => json!({ "prompts": extension_prompts(&creator.manager).into_iter().map(|(public, prompt)| json!({
            "name": public, "description": prompt.description, "arguments": prompt.arguments,
        })).collect::<Vec<_>>() }),
        "prompts/get" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let Some((_, prompt)) = extension_prompts(&creator.manager).into_iter().find(|(public, _)| public == name) else {
                return Some(rpc_error(id, -32602, "没有这个提示词"));
            };
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            for required in prompt.arguments.iter().filter(|argument| argument["required"] == true) {
                if arguments.get(required["name"].as_str().unwrap_or_default()).is_none() {
                    return Some(rpc_error(id, -32602, &format!("缺少参数 {}", required["name"].as_str().unwrap_or_default())));
                }
            }
            json!({ "description": prompt.description, "messages": [{ "role": "user", "content": { "type": "text", "text": crate::extension_codex::render_prompt(&prompt.template, &arguments) } }] })
        }
        "resources/list" => json!({ "resources": extension_resources(&creator.manager).into_iter().map(|(uri, resource)| json!({
            "uri": uri, "name": resource.name, "description": resource.description, "mimeType": resource.mime_type,
        })).collect::<Vec<_>>() }),
        "resources/templates/list" => json!({ "resourceTemplates": [] }),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or_default();
            let Some((_, resource)) = extension_resources(&creator.manager).into_iter().find(|(candidate, _)| candidate == uri) else {
                return Some(rpc_error(id, -32602, "没有这个资源"));
            };
            match read_resource(uri, &resource) {
                Ok(content) => json!({ "contents": [content] }),
                Err(error) => return Some(rpc_error(id, -32603, &format!("{error:#}"))),
            }
        }
        _ => return Some(rpc_error(id, -32601, "未实现的 MCP 方法")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// 启用中拓展的提示词：(对外名 `<拓展>__<名字>`, 提示词)。
fn extension_prompts(manager: &user_scripts::UserScriptManager) -> Vec<(String, crate::extension_codex::ExtensionPrompt)> {
    manager
        .enabled_codex_assets()
        .unwrap_or_default()
        .into_iter()
        .flat_map(|item| {
            let id = item.id.clone();
            item.assets.prompts.into_iter().map(move |prompt| (user_scripts::extension_tool_public_name(&id, &prompt.name), prompt))
        })
        .collect()
}

/// 启用中拓展的资源：(uri, 资源)。
fn extension_resources(manager: &user_scripts::UserScriptManager) -> Vec<(String, crate::extension_codex::ExtensionResource)> {
    manager
        .enabled_codex_assets()
        .unwrap_or_default()
        .into_iter()
        .flat_map(|item| {
            let (id, root) = (item.id.clone(), item.root.clone());
            item.assets.resources.into_iter().map(move |resource| (crate::extension_codex::resource_uri(&id, &root, &resource.file), resource))
        })
        .collect()
}

fn read_resource(uri: &str, resource: &crate::extension_codex::ExtensionResource) -> anyhow::Result<Value> {
    use base64::Engine;
    let metadata = std::fs::metadata(&resource.file)?;
    if metadata.len() > crate::extension_codex::RESOURCE_MAX_BYTES {
        anyhow::bail!("资源超过 1 MiB");
    }
    let bytes = std::fs::read(&resource.file)?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) if !resource.mime_type.starts_with("image/") => json!({ "uri": uri, "mimeType": resource.mime_type, "text": text }),
        Ok(text) => json!({ "uri": uri, "mimeType": resource.mime_type, "blob": base64::engine::general_purpose::STANDARD.encode(text.as_bytes()) }),
        Err(error) => json!({ "uri": uri, "mimeType": resource.mime_type, "blob": base64::engine::general_purpose::STANDARD.encode(error.into_bytes()) }),
    })
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

/// write 工具里 gateway 字段的说明。可关的内置环节直接从 `BUILTIN_GATEWAY_SEGMENTS` 生成，新开放环节时不会漏写。
fn gateway_schema_description() -> String {
    format!(
        "网关中间件声明，例如 {{\"request\": [{{\"script\": \"gateway/request.mjs\"}}], \"response\": [{{\"script\": \"gateway/response.mjs\"}}]}}；\
         request 改写发给模型的请求（stage 为 route 时在选好候选供应商后重排 / 筛选，stage 为 upstream 时改 Codex++ 转换后真正发给上游的请求体，order 决定先后），\
         response 改写模型回复的事件流，error 决定失败后是否重试，replaces 关掉内置环节（{}）。\
         写法见指南「网关请求中间件」「网关响应流中间件」",
        user_scripts::BUILTIN_GATEWAY_SEGMENTS.join(" / ")
    )
}

fn tool_definitions() -> Value {
    let read_only = json!({"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false});
    let writes = json!({"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false});
    json!([
        {
            "name": "codexpp_extension_guide",
            "description": "读取 Codex++ 拓展创作指南与本对话关联的拓展：ctx API、约定、示例和可用路由。写拓展前先调用一次；后续需求默认继续修改 conversation.extensionId。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": read_only,
        },
        {
            "name": "codexpp_extension_list",
            "description": "列出用户已有的 Codex++ 拓展与脚本，含启用状态、加载状态、报错和本对话的拓展关联；不切换当前目标。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": read_only,
        },
        {
            "name": "codexpp_extension_read",
            "description": "读回拓展清单与源码，修改前使用。省略 id 读取本对话当前拓展；指定 id 只读参考，不切换目标。用户明确要编辑某个已有拓展时传 id 和 select: true，关联到本对话。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "拓展 id；省略使用本对话关联"},
                    "select": {"type": "boolean", "description": "仅在用户明确要求编辑这个已有拓展时设为 true，将 id 关联到本对话；默认 false"}
                },
                "additionalProperties": false,
            },
            "annotations": writes,
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
            "description": "创建或更新本对话关联的 Codex++ 拓展并热重载。后续需求默认继续更新：省略 id/name/清单字段会沿用原值，先 read 保留原功能。只有用户明确说新建另一个拓展才传 newExtension: true 和未占用的 id。不同对话分别保存目标，不能默认覆盖别的拓展。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "拓展 id：字母、数字、- 和 _，最长 64；首次创建必填，后续省略使用本对话关联"},
                    "name": {"type": "string", "description": "给用户看的名字；首次创建必填，更新时省略沿用"},
                    "newExtension": {"type": "boolean", "description": "仅当用户明确要求新建另一个拓展时为 true，必须提供未占用的 id；成功后本对话改为关联新拓展。默认 false"},
                    "description": {"type": "string", "description": "一句话说明拓展做什么"},
                    "version": {"type": "string", "description": "版本号，默认 0.1.0"},
                    "routes": {"type": "array", "items": {"type": "string"}, "description": "拓展需要通过 ctx.call 调用的后端路由，只能从指南列出的可用路由里选；不需要就省略"},
                    "renderer": {"type": ["array", "string"], "items": {"type": "string"}, "description": "按顺序执行的页面入口文件；省略时若 files 里有 renderer.js 就用它，只声明 hooks 的拓展可以没有页面入口"},
                    "hooks": {"type": "object", "description": "Codex 钩子声明，例如 {\"PreToolUse\": [{\"matcher\": \"^Bash$\", \"script\": \"hooks/guard.mjs\", \"timeout\": 30}]}；写法见指南「Codex 钩子」"},
                    "toolPipeline": {"type": "object", "description": "工具管道声明，例如 {\"preExecute\": [{\"matcher\": \"^Bash$\", \"script\": \"tools/pre.mjs\"}], \"execute\": [...], \"postExecute\": [...]}：preExecute 改参数或拦截，execute 代为执行，postExecute 改写工具结果。写法见指南「工具管道」"},
                    "providers": {"type": "array", "description": "模型供应商，例如 [{\"id\": \"myrelay\", \"protocol\": \"chat\", \"baseUrl\": \"https://…/v1\", \"apiKey\": {\"config\": \"apiKey\"}, \"models\": [\"m1\"]}]：并入 Codex++ 网关供应商池，模型名是 <id>/<模型>。写法见指南「模型供应商」"},
                    "skills": {"type": "array", "items": {"type": "string"}, "description": "技能目录（包内含 SKILL.md 的目录），例如 [\"skills/review\"]；省略时自动收 skills/*/SKILL.md。启用后装进 ~/.codex/skills，新开对话可用"},
                    "agents": {"type": "array", "description": "子 agent 预设，例如 [{\"name\": \"reviewer\", \"description\": \"代码审查\", \"instructions\": \"prompts/reviewer.md\", \"model\": \"gpt-5\", \"reasoningEffort\": \"high\"}]：写成 Codex 的 agents 角色"},
                    "mcpServers": {"type": "object", "description": "外部 MCP 服务器，例如 {\"weather\": {\"command\": \"node\", \"args\": [\"${extensionDir}/mcp/server.mjs\"], \"env\": {\"KEY\": \"${config.apiKey}\"}}} 或 {\"docs\": {\"url\": \"https://…/mcp\"}}：写进 Codex 配置"},
                    "schedules": {"type": "array", "description": "定时任务，例如 [{\"method\": \"digest\", \"at\": \"09:00\"}, {\"method\": \"poll\", \"everyMinutes\": 30}]：到点调用后台导出的方法，需要同时声明 backend"},
                    "prompts": {"type": "array", "description": "提示词模板，例如 [{\"name\": \"review\", \"description\": \"审查\", \"arguments\": [{\"name\": \"focus\"}], \"template\": \"prompts/review.md\"}]：模板里写 {{focus}}，经 Codex++ 拓展 MCP 提供"},
                    "resources": {"type": "array", "description": "只读资源，例如 [{\"name\": \"规范\", \"file\": \"docs/style.md\"}]：经 Codex++ 拓展 MCP 提供给模型读取"},
                    "instructions": {"type": "string", "description": "全局指令：包内 .md 路径或文字，合并进 ~/.codex/AGENTS.md 里 Codex++ 托管的一段（官方直连也生效）"},
                    "instances": {"type": "array", "items": {"type": "object"}, "description": "同一个拓展按多组配置各跑一份，例如 [{\"id\": \"work\", \"name\": \"工作\", \"config\": {\"minutes\": 50}}, {\"id\": \"rest\", \"name\": \"休息\", \"config\": {\"minutes\": 10}}]：页面脚本对每个实例各调用一次 apply，ctx.config 是实例自己的配置（与 config 声明合并）"},
                    "dsh": {"type": "string", "description": "包内按 dsh 插件写法写的插件目录（如 \"dsh\"，目录里有 package.json 与插件入口）：拓展启用时由 Codex++ 的 dsh 兼容宿主导入运行。写法见指南「在拓展包里带 dsh 插件」"},
                    "replaces": {"type": "array", "items": {"type": "string"}, "description": "顶替的内置拓展 id，例如 [\"model-picker-groups\"]：本拓展启用时这些内置拓展让位、停用时回来（写已改成官方拓展包的 usage-statusline / proxy-request-status 时让位的是对应的官方拓展包）。写法见指南「顶替 Codex++ 内置功能」"},
                    "manager": {"type": "object", "description": "管理器面板，例如 {\"page\": \"manager.html\", \"title\": \"统计\"}：管理器「本地拓展」里这一行的菜单出现「打开面板」；nav 放进侧栏分组（gateway / management / tools），replaces 顶替管理器的内置页面，data 声明能经 codexPlusManager.data() 读的只读数据。页面放在沙箱 iframe 里，经 window.codexPlusManager 读写本拓展配置、调本拓展后台（backend.call）、取 assets 资源；页面不能联网，资源要内联或用 asset()。写法见指南「管理器面板」"},
                    "preload": {"type": ["object", "array", "string"], "description": "页面最早期脚本，例如 {\"scripts\": [\"preload/patch.js\"], \"reloadOnLaunch\": true}：在 Codex 自己的脚本之前执行（拿不到 codexPlus，只读 codexPlusPreload），改了要重启 Codex。写法见指南「赶在 Codex 之前（preload）与启动前后（launch）」"},
                    "launch": {"type": "object", "description": "启动前后，例如 {\"args\": [\"--lang=zh-CN\"], \"before\": \"launch/before.mjs\", \"after\": \"launch/after.mjs\"}：args 追加到 Codex 启动参数，before / after 在启动 Codex 之前、退出之后跑 Node 脚本（默认导出函数，拿到 codexHome、stateDir、config）"},
                    "backend": {"type": "object", "description": "常驻后台声明，例如 {\"script\": \"backend/main.mjs\"}；脚本默认导出 async (ctx) => ({ 方法 })，页面用 ctx.backend.call / on 调用。写法见指南「常驻后台」"},
                    "config": {"type": "object", "description": "配置项声明，例如 {\"minutes\": {\"type\": \"number\", \"title\": \"时长（分钟）\", \"default\": 25}}；type 可选 string / number / boolean / select（配 options）。用户可在管理器里改，脚本里读 ctx.config / context.config"},
                    "tools": {"type": "array", "description": "给模型用的工具声明，例如 [{\"name\": \"lookup\", \"description\": \"…\", \"inputSchema\": {\"type\": \"object\", \"properties\": {…}}, \"script\": \"tools/lookup.mjs\", \"timeout\": 30}]；写法见指南「给模型加工具」", "items": {"type": "object"}},
                    "gateway": {"type": "object", "description": gateway_schema_description()},
                    "files": {
                        "type": "array",
                        "description": "要写入的文件，不要包含 codexpp.json。手写的文本给 content（单个最大 256 KiB）；本机已有的文件给 sourcePath（绝对路径）：图片、音频、字体放在 assets/ 下按字节复制（页面里用 ctx.asset(\"assets/xx.png\") 取地址，单个最大 1 MiB），打包产物等文本按 UTF-8 复制（单个最大 4 MiB）。用了第三方库的界面先用 esbuild 等打包成单个文件写到本机，再用 sourcePath 复制进包，不要把打包结果贴进 content。不要把 base64 写进代码",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string", "description": "包内相对路径"},
                                "content": {"type": "string", "description": "文本文件的完整内容"},
                                "sourcePath": {"type": "string", "description": "本机文件绝对路径：png / jpg / gif / webp / svg / mp3 / wav / ogg / woff2 等按资源复制（单个最大 1 MiB），其余按 UTF-8 文本复制（打包产物等，单个最大 4 MiB）"},
                            },
                            "required": ["path"],
                            "additionalProperties": false,
                        },
                    },
                },
                "required": ["files"],
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
            "description": "修改本对话拓展的配置值（只能改清单 config 里声明过的项），省略 id 使用当前关联，只覆盖给出的键，其余保留；保存后热重载。用户说「把 X 改成 Y」且 X 是拓展配置时用它，不必重写代码。secret 项（如 API Key）在清单里显示为 ******，不要让用户把 Key 发进对话，请他们在管理器里填。",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "string", "description": "省略使用本对话关联；已有绑定时不能改其他拓展"}, "values": {"type": "object", "description": "要设置的配置项与值"}},
                "required": ["values"],
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
    #[serde(default)]
    providers: Option<Value>,
    #[serde(default)]
    skills: Option<Value>,
    #[serde(default)]
    agents: Option<Value>,
    #[serde(default, rename = "mcpServers")]
    mcp_servers: Option<Value>,
    #[serde(default)]
    instructions: Option<Value>,
    #[serde(default)]
    schedules: Option<Value>,
    #[serde(default)]
    prompts: Option<Value>,
    #[serde(default)]
    resources: Option<Value>,
    #[serde(default)]
    manager: Option<Value>,
    #[serde(default)]
    preload: Option<Value>,
    #[serde(default)]
    launch: Option<Value>,
    #[serde(default)]
    replaces: Option<Value>,
    #[serde(default)]
    instances: Option<Value>,
    #[serde(default)]
    dsh: Option<Value>,
    #[serde(deserialize_with = "file_list")]
    files: Vec<WriteFile>,
}

/// 一个要写入的文件：文本给 `content`；本机已有的文件给 `sourcePath`——图片 / 音频 / 字体按字节复制成资源，
/// 其余（打包产物、HTML、CSS 等文本）按 UTF-8 读进来，单个上限 4 MiB。
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WriteFile {
    path: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    source_path: Option<String>,
}

/// `sourcePath` 指向的本机文本文件（打包产物、HTML、CSS 等）：绝对路径、普通文件、UTF-8、不超过复制上限。
fn read_copied_text(path: &str, source: &str) -> anyhow::Result<String> {
    let source = std::path::Path::new(source);
    if !source.is_absolute() {
        anyhow::bail!("{path} 的 sourcePath 必须是绝对路径");
    }
    let metadata = std::fs::metadata(source).with_context(|| format!("读不到 {}", source.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("{} 不是普通文件", source.display());
    }
    let limit = user_scripts::EXTENSION_COPIED_TEXT_MAX_BYTES;
    if metadata.len() > limit as u64 {
        anyhow::bail!("{path} 超过复制文件上限 {} MiB（打包时开 minify、拆掉用不到的依赖）", limit / 1024 / 1024);
    }
    let bytes = std::fs::read(source).with_context(|| format!("读不到 {}", source.display()))?;
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{path} 不是 UTF-8 文本；图片、音频、字体请放在 assets/ 下（按扩展名识别为资源）"))
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
struct ReadArguments {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    select: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigureArguments {
    #[serde(default)]
    id: Option<String>,
    values: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnabledArguments {
    id: String,
    enabled: bool,
}

fn required_thread_id(meta: &Value) -> anyhow::Result<String> {
    context::thread_id(meta)?.context(
        "客户端没有提供可信的对话 threadId，无法确认本次修改属于哪个对话，未写入。请使用支持传递 threadId 的 Codex 客户端重新连接拓展工具；不要猜测 ID 或使用当前页面焦点代替。",
    )
}

fn conversation_view(thread_id: &str, extension_id: Option<&str>) -> Value {
    json!({ "threadId": thread_id, "extensionId": extension_id })
}

fn conversation_status(manager: &UserScriptManager, meta: &Value) -> anyhow::Result<Value> {
    let Some(thread_id) = context::thread_id(meta)? else {
        return Ok(Value::Null);
    };
    let bindings = context::Bindings::lock(manager)?;
    Ok(conversation_view(&thread_id, bindings.get(&thread_id)))
}

fn read_for_conversation(creator: &ExtensionCreator, arguments: Value, meta: &Value) -> anyhow::Result<Value> {
    let arguments: ReadArguments =
        serde_json::from_value(arguments).context("codexpp_extension_read 参数无效")?;
    let thread_id = context::thread_id(meta)?;
    if arguments.select && thread_id.is_none() {
        required_thread_id(meta)?;
    }
    if arguments.select && arguments.id.is_none() {
        anyhow::bail!("选择已有拓展时必须提供 id");
    }
    // 没有对话身份时仍允许按明确 id 只读参考，但不能建立或借用任何关联。
    let mut bindings = thread_id.as_ref().map(|_| context::Bindings::lock(&creator.manager)).transpose()?;
    let current = thread_id.as_deref().and_then(|thread| bindings.as_ref()?.get(thread));
    let id = arguments.id.as_deref().or(current).context("本对话尚未关联拓展，请提供要读取的 id")?;
    let mut result = creator.manager.read_extension_package(id)?;
    let id = id.to_owned();
    if arguments.select {
        if result.pointer("/manifest/id").and_then(Value::as_str) != Some(&id) {
            anyhow::bail!("拓展清单 id 与目录不一致，未建立关联");
        }
        bindings.as_mut().unwrap().bind(thread_id.as_deref().unwrap(), &id)?;
    }
    result["conversation"] = match thread_id {
        Some(thread) => conversation_view(&thread, bindings.as_ref().and_then(|bindings| bindings.get(&thread))),
        None => Value::Null,
    };
    Ok(result)
}

fn prepare_conversation_write(
    manager: &UserScriptManager,
    bindings: &context::Bindings,
    thread_id: &str,
    arguments: Value,
) -> anyhow::Result<WriteArguments> {
    #[derive(Deserialize)]
    struct Selection {
        #[serde(default)]
        id: Option<String>,
        #[serde(default, rename = "newExtension")]
        new_extension: bool,
    }
    let selection: Selection = serde_json::from_value(arguments.clone()).context("codexpp_extension_write 参数无效")?;
    let mut arguments = arguments.as_object().cloned().context("codexpp_extension_write 参数必须是对象")?;
    arguments.remove("newExtension");
    let current = bindings.get(thread_id);
    let updating = current.is_some() && !selection.new_extension;
    let id = if updating {
        let current = current.unwrap();
        if selection.id.as_deref().is_some_and(|id| id != current) {
            anyhow::bail!("本对话已关联拓展 {current}，默认只能继续修改它。仅当用户明确要求新建另一个拓展时传 newExtension: true；明确要编辑已有拓展时先 read(id, select: true)。");
        }
        if !manager.has_extension_package(current) {
            anyhow::bail!("本对话关联的拓展 {current} 已不存在，未自动重新创建。请恢复该拓展，或按用户明确要求新建/选择拓展。");
        }
        current.to_owned()
    } else {
        let id = selection.id.context("首次创建或明确新建拓展时必须提供 id 和 name")?;
        user_scripts::validate_extension_id(&id)?;
        match std::fs::symlink_metadata(manager.user_dir().join(&id)) {
            Ok(_) => anyhow::bail!("拓展 id {id} 已被占用，未覆盖。用户明确要修改它时先 read(id, select: true)；新建请使用未占用的 id。"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("无法检查拓展目录"),
        }
        id
    };
    if updating {
        let existing = manager.read_extension_package(&id)?;
        let manifest = existing["manifest"].as_object().context("当前拓展清单损坏，未覆盖")?;
        if manifest.get("id").and_then(Value::as_str) != Some(&id) {
            anyhow::bail!("当前拓展清单 id 与关联不一致，未覆盖");
        }
        // 更新只继承省略的声明；显式传值仍可替换。源码由 agent 读回后在原功能上修改。
        for key in [
            "name", "description", "version", "renderer", "hooks", "gateway", "tools", "config",
            "backend", "toolPipeline", "providers", "skills", "agents", "mcpServers", "instructions",
            "schedules", "prompts", "resources", "manager", "preload", "launch", "replaces", "instances", "dsh",
        ] {
            if !arguments.contains_key(key) {
                if let Some(value) = manifest.get(key) {
                    arguments.insert(key.to_owned(), value.clone());
                }
            }
        }
        if !arguments.contains_key("routes") {
            if let Some(routes) = existing.pointer("/manifest/permissions/routes") {
                arguments.insert("routes".into(), routes.clone());
            }
        }
    }
    arguments.insert("id".into(), json!(id));
    serde_json::from_value(Value::Object(arguments)).context("codexpp_extension_write 参数无效")
}

fn write_for_conversation(creator: &ExtensionCreator, arguments: Value, meta: &Value) -> anyhow::Result<(String, Value, Value)> {
    let thread_id = required_thread_id(meta)?;
    // 目标解析、写包和保存关联在同一把跨进程锁内；异步热重载前释放。
    let mut bindings = context::Bindings::lock(&creator.manager)?;
    let arguments = prepare_conversation_write(&creator.manager, &bindings, &thread_id, arguments)?;
    let mut files = BTreeMap::new();
    let mut assets = BTreeMap::new();
    let mut copied_text = std::collections::BTreeSet::new();
    for file in arguments.files {
        let duplicate = match (file.content, file.source_path) {
            (Some(content), None) => files.insert(file.path.clone(), content).is_some(),
            (None, Some(source)) if user_scripts::extension_asset_mime(&file.path).is_some() => {
                assets.insert(file.path.clone(), std::path::PathBuf::from(source)).is_some()
            }
            (None, Some(source)) => {
                let text = read_copied_text(&file.path, &source)?;
                copied_text.insert(file.path.clone());
                files.insert(file.path.clone(), text).is_some()
            }
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
        providers: arguments.providers,
        extra: [
            ("skills", arguments.skills),
            ("agents", arguments.agents),
            ("mcpServers", arguments.mcp_servers),
            ("instructions", arguments.instructions),
            ("schedules", arguments.schedules),
            ("prompts", arguments.prompts),
            ("resources", arguments.resources),
            ("manager", arguments.manager),
            ("preload", arguments.preload),
            ("launch", arguments.launch),
            ("replaces", arguments.replaces),
            ("instances", arguments.instances),
            ("dsh", arguments.dsh),
        ]
            .into_iter()
            .filter_map(|(key, item)| item.map(|item| (key.to_string(), item)))
            .collect(),
        files,
        assets,
        copied_text,
        max_file_bytes: None,
    })?;
    bindings.bind(&thread_id, &arguments.id)?;
    let conversation = conversation_view(&thread_id, Some(&arguments.id));
    Ok((arguments.id, written, conversation))
}

fn configure_for_conversation(creator: &ExtensionCreator, arguments: Value, meta: &Value) -> anyhow::Result<(String, Value, Value)> {
    let arguments: ConfigureArguments =
        serde_json::from_value(arguments).context("codexpp_extension_configure 参数无效")?;
    let thread_id = required_thread_id(meta)?;
    let mut bindings = context::Bindings::lock(&creator.manager)?;
    let current = bindings.get(&thread_id);
    if let (Some(current), Some(requested)) = (current, arguments.id.as_deref()) {
        if current != requested {
            anyhow::bail!("本对话已关联拓展 {current}，未修改 {requested} 的配置。用户明确要切换目标时先 read(id, select: true)。");
        }
    }
    let id = arguments.id.as_deref().or(current).context("本对话尚未关联拓展，请先选择要配置的拓展")?.to_owned();
    if !creator.manager.has_extension_package(&id) {
        anyhow::bail!("没有 id 为 {id} 的用户拓展包");
    }
    let key = user_scripts::extension_package_key(&id);
    let config = creator.manager.update_extension_config(&key, &arguments.values)?;
    bindings.bind(&thread_id, &id)?;
    Ok((key, config, conversation_view(&thread_id, Some(&id))))
}

async fn call_tool(creator: &ExtensionCreator, name: &str, arguments: Value, meta: &Value) -> anyhow::Result<Value> {
    match name {
        "codexpp_extension_guide" => Ok(json!({
            "guide": GUIDE,
            "apiVersion": user_scripts::EXTENSION_API_VERSION,
            "availableRoutes": extension_routes(),
            "extensionsDirectory": creator.manager.user_dir().to_string_lossy(),
            "conversation": conversation_status(&creator.manager, meta)?,
        })),
        "codexpp_extension_list" => {
            let live = live_status(&creator.debug_ports).await.map(|(_, status)| status);
            let inventory = creator.manager.inventory_with_runtime_status(live.as_ref())?;
            Ok(json!({
                "extensionsDirectory": creator.manager.user_dir().to_string_lossy(),
                "codexConnected": live.is_some(),
                "items": summarize_items(&inventory, None),
                "conversation": conversation_status(&creator.manager, meta)?,
            }))
        }
        "codexpp_extension_read" => read_for_conversation(creator, arguments, meta),
        "codexpp_extension_write" => {
            let (id, written, conversation) = write_for_conversation(creator, arguments, meta)?;
            let key = user_scripts::extension_package_key(&id);
            let mut result = reload_and_report(creator, &key).await?;
            result["written"] = written;
            result["conversation"] = conversation;
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
            let (key, config, conversation) = configure_for_conversation(creator, arguments, meta)?;
            let mut result = reload_and_report(creator, &key).await?;
            let schema = result.pointer("/extension/package/configSchema").cloned().unwrap_or(Value::Null);
            result["config"] = user_scripts::mask_secret_config(&schema, &config);
            result["conversation"] = conversation;
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
const lines = readline.createInterface({ input: process.stdin });
/*HOST_CLIENT*/
let input = null;
lines.on("line", async (line) => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if (handleHostReply(message)) return;
  if (input) return;
  input = message;
  let timer;
  try {
    const module = await import(pathToFileURL(input.script).href);
    const fn = typeof module.default === "function" ? module.default : module[input.tool];
    if (typeof fn !== "function") throw new Error(`${input.script} 没有默认导出函数`);
    const timeout = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`超时（${input.timeoutMs / 1000} 秒）`)), input.timeoutMs); });
    const result = await Promise.race([Promise.resolve(fn(input.args ?? {}, { extension: input.extension, tool: input.tool, config: input.config ?? {}, sessionQuery, sessions, service, llm, ui })), timeout]);
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
        &crate::extension_node::with_host_client(TOOL_RUNNER),
        &input,
        Duration::from_secs(tool.timeout_secs + 5),
        &creator.manager,
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
                        "package": mask_package_secrets(&item["package"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 清单里的拓展配置交给模型前把 secret 项打码。
fn mask_package_secrets(package: &Value) -> Value {
    let mut package = package.clone();
    if package.get("config").is_some() {
        package["config"] = user_scripts::mask_secret_config(&package["configSchema"], &package["config"]);
    }
    package
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
