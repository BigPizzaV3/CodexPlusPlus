# Codex++ Computer Use 桥：把 Codex 的 CU 补给 Claude Code

- 日期：2026-10-23
- 依据：[codexapp CU 与 Chrome 移植可行性分析](../reports/2026-10-23-codexapp-cu-and-chrome-porting-analysis.md)
- 状态：**v2**（2026-10-23 实测修订：CU 改为「直接复用 Codex 的 `cua_repl` + 薄适配层」，自研引擎降级为可选降级方案）

> **实现状态（2026-10-23）**：M0′ + M1′ 已落地并实测通过。
> - 新增 `crates/codex-plus-core/src/computer_use.rs`（定位 / 注册 / 卸载 / 自检，7 个单元测试）
> - 新增 `apps/codex-plus-manager/src-tauri/src/computer_use_commands.rs`（5 条命令）+ `--codex-plus-cu-cli` 诊断入口
> - 新增管理器页 `apps/codex-plus-manager/src/ComputerUsePage.tsx`（路由 `computerUse`）
> - 实测：`--codex-plus-cu-cli check` → `ok:true`、`appCount:49`、2.2s；`claude mcp list` → `codex-plus-cu … ✔ Connected`；重复安装 `changed:false`；卸载后 `mcpServers` 恢复为 `{}` 并留下 `.codex-plus-backup-<ts>`
> - **关键坑（已修）**：只传 `SKY_CUA_SERVICE_PATH` 不够，缺 `CODEX_CLI_PATH` 会直接报 `-10005 codex app-server exited before returning a response`。CU 的审批链路需要一个 app-server 侧通道，必须把同包的 `codex-cli/CodexCLI.app/Contents/MacOS/codex` 一起传给子进程。
> - **另一个坑（已修）**：`cua-repl` 会再拉起 `node_repl`，只杀父进程会留下孙进程占着 stdout 管道，自检会永久挂住。现在子进程独立 process group，收尾用 `kill -9 -<pgid>`，并且不 join 读取线程。
> - **M3（Chrome）已被复用打通**：Codex 的浏览器桥只要求每次 `tools/call` 带 `_meta["x-codex-turn-metadata"]={session_id,turn_id}`，代理自动生成注入后，`cua.listBrowsers()` / `listTabs()` 与 browser 级 `management` capability（`tabs.group` / `tabGroups.update`）均可用——不需要自建 MV3 扩展。代理同时把「优先 element_index、避免坐标点击」这条约定追加进 MCP instructions。
> - **必须的中间层（2026-10-23 补）**：Claude Code 的 MCP 客户端**不声明 `elicitation`**，直连 `cua-repl` 会在真正接管窗口时抛 `nodeRepl.createElicitation is unavailable`——表现为「能读应用列表，一接管就失败」。现在配置指向 Codex++ 自己的 `--codex-plus-cu-mcp`：它作为 MCP 代理，对 `cua-repl` 声明 elicitation 并就地接受（留痕到 `~/.codex-plus/cu/elicitation.log`），工具面原样透传。自检也改成走这条代理路径，且刻意不声明 elicitation——代理一旦退化，自检立刻失败。
> - **设计修正**：Codex 桌面版自己已经把 `[mcp_servers.computer-use]`（`SkyComputerUseClient.app mcp`）与 `[mcp_servers.node_repl]` 写进 `~/.codex/config.toml`，而 CLI 与桌面会话共用这份文件——也就是说 Codex 侧**本来就有 CU**。因此产品默认只提供 `claudeCode` / `claudeDesktop` 两个接入目标；`codex` 目标保留为高级用法（仅用于没有该条目的环境），不做默认暴露，避免同一个 CU 服务挂两条连接。

---

## 1. 目标

在 Codex++ 内建一套 **Computer Use 引擎**，语义对齐 Codex app（AX 树 + element_index + 后台点击 + 虚拟光标），并通过 **MCP stdio** 同时供给：

1. **Claude Code CLI**（`~/.claude.json` 用户级 `mcpServers`）
2. **Claude Desktop**（`claude_desktop_config.json`）
3. **Codex**（`config.toml` 的 `[mcp_servers.*]`，默认关闭——Codex 自带 CU）
4. 任意第三方 MCP 客户端（Cursor / Zed / 自研）

同一进程再挂第二组工具：**Chrome 控制 + 标签分组**（对齐 Codex 的 `management` capability）。

## 2. 非目标

- ❌ 不复制、不重签 OpenAI 的 `SkyComputerUseService`，也不连它的 `computeruse.sock`（服务端做 `LOCAL_PEERTOKEN` + teamID 校验）
- ❌ 不修改 Claude.app 本体（不进 asar、不重签）
- ❌ 不做锁屏 / SecurityAgent 授权插件（`CUALockScreenGuardian` 那条路）
- ❌ 默认不动用户真实光标与键盘焦点（只在显式 `foreground:true` 时）

> **注**：v1 的这条「不复制、不重签 `SkyComputerUseService`」仍然成立；v2 新增的是 **不复用它的二进制，但直接复用它的 MCP 入口**（见 §2.5）。

## 2.5 路线修订（2026-10-23 实测，覆盖第 3 节起的"自研引擎"前提）

**结论：不重写，直接复用 Codex 的 CU MCP server，加一层薄适配。**

三条实测证据（都在本机跑过）：

1. **它是标准 stdio MCP server，任意宿主可启动**
   `node /Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/cua-repl/bin/cua-repl.mjs`
   → `initialize` 返回 `{"name":"rmcp","version":"1.5.0"}`；`tools/list` = `js, js_add_node_module_dir, js_reset, turn_ended`。
2. **外部宿主能真的读到窗口**
   `sky.get_app_state({app:"com.apple.finder"})` → `APPS_OK target=mac len=3167`（真实 AX 文本）。期间服务端只发了一次 `elicitation/create`（`riskLevel:"low"`、`persist:["session","always"]`），接受后即放行。
3. **Claude Code 侧完全具备条件**
   CLI 2.1.245 二进制含 `elicitation/create`、`elicitation/complete`（175 处 `elicitation`）；账号级注册后 `claude mcp list` 显示 `codex-cu … ✔ Connected`。

→ 所以「给 Claude Code 补 Codex 的 CU」= **注册 + 适配 + 健康检查**，不涉及 AX/CGEvent 重写。

### 仍然不能直接拿的三样

| 项 | 原因 | 处置 |
|---|---|---|
| Chrome 控制 / 标签分组 | 扩展 ID 与 native host（`com.openai.codexextension`）双向绑定，host 还会校验 `no_matching_codex_install` | 自建扩展；`background.js` 是明文，`tabs.group` / `tabGroups.update` 的分组逻辑可照抄 |
| 锁屏 / SecurityAgent 授权插件 | 需 root，深水区 | 不做 |
| 自包含分发 | `cua_node` + `Codex Computer Use.app` 是 OpenAI 签名产物，搬进自己 bundle 会引入签名与更新问题 | 默认「引用 ChatGPT.app」；自包含只作离线场景选项 |

### 适配层必须解决的四个问题

1. **路径不能写死**
   版本目录 `unified-computer-use/<26.xxxx>` 会随更新变化 → 用 glob + 结构不变量校验（复用 `native_browser.rs` 的未知哈希降级范式）；ChatGPT.app 位置用 `mdfind kMDItemCFBundleIdentifier == "com.openai.chat"` 兜底。
2. **注册 / 卸载**
   写 `~/.claude.json` 的 `mcpServers["codex-plus-cu"]`，带 `_codexPlusManaged` 所有权标记 + 备份 + 文件锁（复用 `agent_collaboration/integration.rs` 的全套做法）。
3. **健康检查（失败要分类，不能只说"连不上"）**
   MCP 握手 → `js` 里跑 `sky.list_apps()` → 分四类输出：没装 ChatGPT.app / CU 未授权（TCC）/ 服务未就绪 / 版本布局变了。
4. **elicitation 兜底**
   Claude Code 支持 elicitation 就直接用它弹审批卡；若某版本不再声明该能力，再在 Codex++ 里加一层 MCP 代理（对外离散 `computer_*` 工具，对内驱动 `cua_repl` 并吞掉 elicitation，用 Codex++ 自己的原生确认框）。

### 里程碑修订

| 原计划 | 修订后 | 估时 |
|---|---|---|
| M0 引擎（AX/事件/截图） | **M0′ 适配层**：定位 + 注册 + 健康检查 + 诊断子命令 | 1–2 天 |
| M1 MCP + Claude 接入 | **M1′**：管理器 UI 开关 + elicitation 策略 + 端到端手测 | 1–2 天 |
| M2 审批 + 虚拟光标 | 降级：仅在 Claude 的 elicitation 体验不达标时才加代理层 | — |
| M3 Chrome 桥 | 不变（这条本来就得自建） | 1.5 周 |
| M4 Windows | 不变 | 1 周 |

**主路径从 6 人周降到 3–5 天**；第 5–9 节的自研引擎方案保留为「不依赖 ChatGPT.app」的可选降级路线（P2）。

## 3. 为什么放在 Codex++ 里最省力（仓库现状）

| 已有件 | 位置 | 对本功能的作用 |
|---|---|---|
| MCP stdio 进程入口 | `apps/codex-plus-manager/src-tauri/src/main.rs`：`--codex-plus-agent-mcp` 在 GUI/单实例锁之前分流 | **新增一个 flag 即可**：`--codex-plus-cu-mcp` |
| 幂等接入 + 备份 + 文件锁 | `crates/codex-plus-core/src/agent_collaboration/integration.rs`（`upsert_managed_context` / `.lock` / 备份目录） | 照抄成 JSON 版给 Claude 用 |
| Claude JSON ↔ Codex TOML 字段知识 | `crates/codex-plus-core/src/mcp_config.rs` | 已知 `type/headers/description` 在 Codex 侧会被丢弃等坑 |
| CDP 客户端 | `crates/codex-plus-core/src/cdp.rs` | Chrome 页面操作（`Input.*` / `Runtime.evaluate`） |
| Codex browser pipe 客户端 | `crates/codex-plus-core/src/native_browser_connection.rs`（`\\.\pipe\codex-browser-use`、4B 帧） | native messaging 的 framing / 鉴权范式 |
| 运行时版本/结构校验 | `crates/codex-plus-core/src/native_browser.rs`（对 `@oai/browser-desktop`、`@oai/cua-repl` 的 SHA + 结构不变量） | 判断「对面是不是 Codex 那一套」的既有范式 |
| 安装器 | `crates/codex-plus-core/src/install/{macos,windows}.rs` | helper / 扩展落盘与升级 |
| 构建签名 | `scripts/relaunch-local.mjs`（build → 换二进制 → codesign → open） | CU 权限是绑签名的，构建流程必须一起管 |
| 原生依赖 | `objc2` / `objc2-app-kit` / `core-graphics` 已在 `Cargo.lock` | Rust 直接调 AX / CGEvent，不必引入 Swift 工具链 |

**结论：不需要新语言、新进程模型、新安装器。** 缺的只是一个 CU 引擎 crate + 一个 MCP 工具面 + Claude 配置写入。

## 4. 架构

```
Claude Code CLI ─┐
Claude Desktop   ─┼── MCP stdio ──► Codex++（/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus
Codex            ─┤                  --codex-plus-cu-mcp）
Cursor 等        ─┘                        │
                                           ├── cu-engine（in-process，Rust）
                                           │     ├── macos: AXUIElement + CGEventPostToPid + 截图 + 虚拟光标 overlay
                                           │     └── windows: UIA + SendInput + WGC/DXGI（M4）
                                           ├── approval store（~/.codex-plus/cu/approvals.json）
                                           ├── session store（每次 Claude 会话一个 turn/窗口白名单）
                                           └── browser bridge（UDS + token）
                                                 └── native host（--codex-plus-cu-host）
                                                       └── Chrome MV3 扩展（tabs / tabGroups / debugger）
```

**关键决策：MVP 不做独立 helper.app**，CU 引擎跑在 Codex++ 主二进制里。理由：

1. TCC 身份只有一份（用户给 Codex++ 授权一次；MCP 进程与 GUI 同 bundle、同签名，共享授权）
2. 不引入 XPC / 双进程调试成本
3. 已有 `--codex-plus-agent-mcp` 证明「同一二进制跑 stdio MCP」可行

> 代价：MCP 进程会加载 Tauri 依赖链（~十几 MB RSS）。M0 实测后可再拆 `crates/codex-plus-cu-helper`（纯 Rust、无 Tauri）作为降级方案，接口不变。

## 5. 引擎设计

### 5.1 抽象

```rust
pub trait Backend: Send + Sync {
    fn list_apps(&self) -> Result<Vec<AppInfo>>;
    fn get_state(&self, app: &str, opts: StateOptions) -> Result<AppState>;   // ax_text + tree + screenshot?
    fn click(&self, req: ClickReq) -> Result<()>;                             // element_index | (x,y)
    fn type_text(&self, app: &str, element: Option<u64>, text: &str) -> Result<()>;
    fn press_key(&self, app: &str, key: &str) -> Result<()>;
    fn scroll(&self, req: ScrollReq) -> Result<()>;
    fn drag(&self, req: DragReq) -> Result<()>;
    fn set_value(&self, app: &str, element: u64, value: &str) -> Result<()>;
    fn select_text(&self, req: SelectTextReq) -> Result<()>;
    fn secondary_action(&self, app: &str, element: u64, action: &str) -> Result<()>;
    fn screenshot(&self, target: ShotTarget) -> Result<Shot>;
    fn permissions(&self) -> Result<PermissionState>;
}
```

### 5.2 macOS 实现（对齐 Codex 的三条主干）

| 动作 | 实现 | 备注 |
|---|---|---|
| 枚举/定位窗口 | `NSWorkspace.runningApplications` + `CGWindowListCopyWindowInfo` + `AXUIElementCreateApplication(pid)` | 只暴露有窗口、非隐藏的 app |
| AX 树 | `AXUIElementCopyAttributeValue` 递归；`AXUIElementCopyActionNames` 取可用动作 | 元素 ID 用「路径指纹」稳定化（不走内存地址） |
| 元素点击 | `AXUIElementPerformAction(el, "AXPress")` | 零鼠标事件，后台可用 |
| 坐标点击 | `AXUIElementCopyElementAtPosition` 先反查元素；命中走 AXPress，否则 `CGEventCreateMouseEvent` + **`CGEventPostToPid`** | 定向投递，真光标不动 |
| 键盘 | `CGEventCreateKeyboardEvent` + `CGEventPostToPid`（用 `TISCopyCurrentKeyboardLayoutInputSource` 做字符→键码） | Codex 用私有 `SAIVirtualKeyPress`；我们降级到公开 API |
| 文本 | `AXUIElementSetAttributeValue(kAXValueAttribute / kAXSelectedTextAttribute)`；中文/emoji 走剪贴板 paste + 还原 | 避免 unicode 直塞失效 |
| 滚动 | `AXUIElementPerformAction` 的 `AXScrollUpByPage` 等；兜底 `CGEventCreateScrollWheelEvent` + postToPid | |
| 拖拽 | `CGEventCreateMouseEvent(.leftMouseDown/.leftMouseDragged/.leftMouseUp)` 序列 + postToPid | |
| 截图 | MVP：`screencapture -x -l<windowid> -t png <tmp>`（系统自带，需屏幕录制权限）；M2 换 `SCScreenshotManager` + `SCContentFilter`（排除自己窗口） | |
| 虚拟光标 | 无边框 `NSPanel`（`ignoresMouseEvents`、`level=.screenSaver`、`collectionBehavior=.canJoinAllSpaces`）画 32×32 光标图，点击时脉冲 | Codex 的 `VirtualCursor` 同思路；MVP 可先不做 |

**明确不做的**：私有 SAI、`AXUIElementPostKeyboardEvent`、`SLSEventPostToPid`、锁屏 overlay。

### 5.3 安全闸门（在引擎里硬编码，不靠提示词）

1. 拒绝操作自己的进程（`pid == std::process::id()`），截图排除自身窗口
2. `IsSecureEventInputEnabled()` 为真时拒绝 `type/key`（密码框场景）
3. `AXSecureTextField` / `AXRole=="AXTextField" && subrole=="AXSecureTextField"` 拒绝 `set_value/type`
4. 每个 app 首次操作需审批（三级：`read` / `click` / `full`）
5. 白名单外的 app 一律 `approval_required`，不静默降级
6. 所有动作写审计（`~/.codex-plus/cu/audit.jsonl`，含 app/动作/元素/时间，不含文本内容）

## 6. MCP 工具面

### 6.1 设计取舍

Codex 用「code-mode」（一个 `js` 工具跑 JS）。**我们不用 code-mode 做默认**：Claude Code 的模型对离散工具的工具调用成功率更高、审计更容易；但保留一个可选 `cu_run_js`（`cu_js` 开关）供高级用法。

### 6.2 工具清单（desktop）

| 工具 | 参数 | 返回 |
|---|---|---|
| `computer_permissions` | — | `{accessibility, screenRecording, backend, hint}` |
| `computer_list_apps` | `{running_only?}` | `[{id,bundleId,name,pid,isRunning,windows[]}]` |
| `computer_get_state` | `{app, include_screenshot?, query?, disable_diff?}` | `{text, tree, screenshot?, window}` |
| `computer_click` | `{app, element_index?, x?, y?, mouse_button?, click_count?, key?, duration_ms?}` | `{ok, mode:"ax"|"pid"}` |
| `computer_type` | `{app, element_index?, text}` | `{ok}` |
| `computer_key` | `{app, key}`（`cmd+shift+t`） | `{ok}` |
| `computer_scroll` | `{app, element_index?, x?, y?, direction, pages?}` | `{ok}` |
| `computer_drag` | `{app, from:{x,y}, to:{x,y}}` | `{ok}` |
| `computer_set_value` | `{app, element_index, value}` | `{ok}` |
| `computer_select_text` | `{app, element_index, text, prefix?, suffix?, selection_type?}` | `{ok}` |
| `computer_secondary_action` | `{app, element_index, action}` | `{ok}` |
| `computer_paste` | `{app, text, format:"text"\|"md"\|"html"}` | `{ok}` |
| `computer_screenshot` | `{app?, window_id?}` | `{data_url, width, height}` |
| `computer_batch` | `{actions:[…]}` | `[{ok}]` |

### 6.3 工具清单（browser，M3）

`browser_list` / `browser_tabs` / `browser_tab_open` / `browser_tab_activate` / `browser_tab_close` /
`browser_group_create` / `browser_group_update` / `browser_group_list` / `browser_snapshot` /
`browser_click` / `browser_type` / `browser_eval` / `browser_screenshot`

`browser_group_*` 严格映射 `chrome.tabs.group` + `chrome.tabGroups.update({title,color,collapsed})`，
只暴露 `collapsed/color/title` 三个可写字段（对齐 Codex 的属性白名单）。

### 6.4 AX 文本格式（对齐 Codex，模型无需重新学）

```
[0] window "Notion" actions=[raise]
  [1] button "Search" actions=[AXPress]
  [2] text_field "" value="" states=[editable,focused]
  [3] list
    [4] row "Q4 计划" states=[selected]
```

- 默认返回 **与上次的 diff**（新增/删除/变化），`disable_diff:true` 返回全量
- `query` 做大小写无关过滤（保留祖先链）
- 单次输出超 25k tokens 时按层截断并在尾部标注 `[truncated: N subtree]`

## 7. Claude Code 接入

### 7.1 写哪里

| 目标 | 文件 | 键 |
|---|---|---|
| Claude Code CLI（用户级，默认） | `~/.claude.json` | `mcpServers["codex-plus-cu"]` |
| Claude Code CLI（项目级，可选） | `<project>/.mcp.json` | 同名 |
| Claude Desktop | `~/Library/Application Support/Claude/claude_desktop_config.json` | `mcpServers["codex-plus-cu"]` |
| Codex（可选，默认关） | `~/.codex/config.toml` | `[mcp_servers.codex_plus_cu]` |

写入内容：

```json
{
  "mcpServers": {
    "codex-plus-cu": {
      "type": "stdio",
      "command": "/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus",
      "args": ["--codex-plus-cu-mcp"],
      "env": {}
    }
  }
}
```

### 7.2 幂等与安全（照抄 `agent_collaboration/integration.rs` 的做法）

- 文件锁：`~/.codex-plus-cu.lock`（`fs2`），防管理器与 CLI 同时写
- 备份：首次修改前存 `<file>.codex-plus-backup-<ts>`，只保留最近 5 份
- 所有权标记：写入的条目带 `"_codexPlusManaged": true`，卸载时只删自己那条，不碰用户手写条目
- 未知 schema：解析失败 → 不改文件，返回 `needs_manual`，UI 给出可复制的 JSON 片段
- CLI 兜底：若本机有 `claude` 可执行文件，提供「用 CLI 注册」按钮（`claude mcp add -s user codex-plus-cu -- <cmd>`），写入失败时用它

### 7.3 生效方式

Claude Code 在**会话启动时**读配置，不热加载。注册后 UI 明确提示「重启 Claude Code 会话生效」。
MCP 进程生命周期由 Claude 管理（每个会话一个）；进程退出即释放所有审批锁。

## 8. 权限与审批模型

| 层 | 内容 | 存储 |
|---|---|---|
| TCC | 辅助功能（必须）、屏幕录制（截图才要） | 系统 TCC，按 bundle + 签名 |
| App 授权 | 每个 bundle id 的 tier：`read`（只看）/ `click`（可点可键）/ `full`（含 set_value/paste） | `~/.codex-plus/cu/approvals.json` |
| 会话锁 | 一次 Claude 会话只允许操作授权过的 app；`computer_release_lock` 主动释放 | 内存 + 落盘兜底 |
| 动作闸门 | secure input、密码框、自身窗口、系统对话框 | 引擎内硬编码 |

UI 流程：`computer_get_state` 首次遇到未授权 app → 返回：

```json
{"error":"approval_required","app":{"bundleId":"com.google.Chrome","name":"Google Chrome"},
 "how":"请打开 Codex++ → Computer Use → 批准该应用（或运行 computer_request_access）"}
```

配套工具 `computer_request_access {app, tier}` 触发管理器弹原生确认框（复用 Tauri dialog）。

## 9. 文件级改动清单

### 9.1 新 crate `crates/codex-plus-cu/`

```
src/lib.rs            # 对外 API：CuEngine::new()、ToolRouter::call(name, args)
src/protocol.rs       # 请求/响应类型（serde），与 MCP schema 一一对应
src/ax.rs             # AX 树遍历、元素指纹、diff 算法、query 过滤
src/approval.rs       # 三级 tier、审批存储、审计日志
src/mcp.rs            # tools/list + tools/call 路由；instructions 文本
src/claude_config.rs  # ~/.claude.json / .mcp.json / claude_desktop_config.json 读写
src/codex_config.rs   # [mcp_servers.codex_plus_cu] 读写（复用 toml_edit）
src/macos/mod.rs
src/macos/ax.rs       # AXUIElement FFI 封装（extern "C" + objc2）
src/macos/event.rs    # CGEventPostToPid / 键码映射 / TIS
src/macos/capture.rs  # screencapture -l（M0）→ SCScreenshotManager（M2）
src/macos/overlay.rs  # 虚拟光标 NSPanel（M2）
src/windows/mod.rs    # M4：UIA + SendInput
src/browser/mod.rs    # M3：UDS 客户端 + 扩展协议
tests/ax_tree.rs
tests/claude_config.rs
tests/approval.rs
```

依赖新增（仅 macOS target）：`objc2`、`objc2-app-kit`、`objc2-foundation`、`objc2-core-foundation`、`core-graphics`、`core-foundation`（都已在 `Cargo.lock` 里，只是提升为直接依赖）。

### 9.2 管理器侧

```
apps/codex-plus-manager/src-tauri/src/main.rs                 # + "--codex-plus-cu-mcp" / "--codex-plus-cu-host" 分流
apps/codex-plus-manager/src-tauri/src/commands/cu.rs          # status/install/uninstall/approvals/permissions
apps/codex-plus-manager/src-tauri/src/lib.rs                  # 注册命令 + run_cu_mcp()
apps/codex-plus-manager/src/ComputerUsePage.tsx               # 新页面
apps/codex-plus-manager/src/ComputerUsePage.css
apps/codex-plus-manager/src/App.tsx                           # 路由/导航项（复刻 AgentCollaborationPage 的接法）
```

### 9.3 资源与脚本

```
assets/cu-extension/            # M3：MV3 扩展（manifest.json, background.js, popup/）
assets/cu-extension/README.md
scripts/build-cu-extension.mjs  # zip 打包 + 版本号注入
```

`scripts/relaunch-local.mjs` **不需要改**（CU 在主二进制内）；但要在 `Info.plist` 补：

```xml
<key>NSAppleEventsUsageDescription</key><string>……</string>
```

（`paste` 走剪贴板时不走 Apple Events，只有 `computer_paste` 的 md/html 形态可能需要，可选。）

## 10. 里程碑与验收

| 里程碑 | 内容 | 验收标准 | 估时 |
|---|---|---|---|
| **M0 引擎** | `crates/codex-plus-cu` + macOS AX/点击/键盘/截图 + `cu-cli` 调试子命令 | `cargo test`；手工：对 Notion 搜索框 `type+click` 不抢焦点成功；对 Finder 列表 `element_index` 点击成功；截图非空 | 3–5 天 |
| **M1 MCP + Claude 接入** | `--codex-plus-cu-mcp`、工具面、`~/.claude.json` 写入、卸载 | `claude` 里问「打开 Notion 搜索 Q4」能跑通；卸载后 `~/.claude.json` 恢复原状（比对备份） | 2–3 天 |
| **M2 审批 + 权限 + 虚拟光标** | approvals.json、三级 tier、TCC 状态页、overlay | 未授权 app 返回 `approval_required`；批准后同会话不再问；overlay 可开关 | 3 天 |
| **M3 Chrome 桥** | MV3 扩展 + native host + UDS + `browser_*` 工具 | 能列出真实 Chrome 标签、建带标题和颜色的分组、后台点页面不切标签 | 1.5 周 |
| **M4 Windows** | UIA + SendInput + 截图 | Windows 上同一套工具跑通 Notepad + Chrome | 1 周 |

每个里程碑完成后按仓库规范 `node scripts/relaunch-local.mjs` 构建启动实测。

## 11. 风险与坑

| 风险 | 影响 | 处置 |
|---|---|---|
| **TCC 绑 cdhash** | ad-hoc 重签后每次构建都重新弹窗 | 本地开发用固定自签证书（`security find-identity` 建一张），CI/发版用 Developer ID；文档写明「换签名 = 重新授权」 |
| Claude Code 配置 schema 漂移 | 写入失败或条目被忽略 | 先读后写 + 结构校验（`mcpServers` 必须 object）+ 备份 + CLI 兜底 + 失败不改文件 |
| MCP 进程太重 | 每个 Claude 会话多十几 MB | M0 实测；超 50MB 就抽 `cu-helper` 纯 Rust 二进制（接口不变，Tauri 不参与） |
| AX 树巨大（Electron/VSCode） | token 爆炸、超时 | `query` 过滤 + 深度上限 8 + 单节点文本 200 字符 + diff + 25k token 截断 |
| 后台点击的边界 | 菜单栏、系统对话框、Secure Input 点不动 | 引擎显式识别并返回结构化错误（`secure_input_active` / `system_dialog`），不静默失败 |
| 键盘 unicode | 中文/emoji 走 CGEvent 会丢 | 走 `AXUIElementSetAttributeValue` 或剪贴板 paste + 还原 |
| Chrome 扩展审核 | `debugger` + `<all_urls>` 商店难上 | M3 先做「未打包加载 + 企业策略」两路，商店作为长期项 |
| 与 Codex 原生 CU 冲突 | 用户同时开两套，重复操作 | 默认不给 Codex 注册；若用户手动加了，UI 提示二者择一 |
| 法律/合规边界 | 复用 OpenAI 二进制 | 只参考架构，不复用其二进制/包；协议字段是通用 JSON-RPC，不构成复制 |

## 12. 与 Codex 原版的能力差异（诚实对照）

| 能力 | Codex app | 本方案 M3 后 |
|---|---|---|
| AX 树 + element_index + diff | ✅ | ✅ |
| 元素点击（零鼠标事件） | ✅ AXPress | ✅ AXPress |
| 坐标点击（后台、不动光标） | ✅ CGEventPostToPid | ✅ CGEventPostToPid |
| 虚拟光标 overlay | ✅ VirtualCursor + 45 帧动画 | ✅ NSPanel 简化版 |
| 键盘（后台施键） | ✅ 私有 `SAIVirtualKeyPress` | ⚠️ 公开 `CGEventPostToPid`（成功率略低，菜单快捷键场景够用） |
| 截图 | ✅ ScreenCaptureKit + 排除自身 | M0 `screencapture`，M2 换 SCK |
| per-app 审批 / 三级权限 | ✅ | ✅ |
| 锁屏/授权插件 | ✅（深水区） | ❌ 不做 |
| 录制回放 / 记忆（Skysight） | ✅ | ❌ 不在本期 |
| Chrome 标签分组 | ✅ | ✅ M3 |

---

## 13. 开工顺序（第一周）

1. `cargo new --lib crates/codex-plus-cu`，加 workspace member
2. `src/macos/ax.rs`：App 枚举 + AX 树导出 + 元素指纹（先不做 diff）
3. `src/macos/event.rs`：`CGEventPostToPid` 点击 + 键盘
4. `src/protocol.rs` + `src/lib.rs`：`Backend` trait + `CuEngine`
5. `tests/ax_tree.rs`：对 mock 树跑指纹/query/截断
6. 调试入口：`CodexPlusPlus --codex-plus-cu-cli get-state --app Finder`（临时子命令，M1 后保留为诊断用）
7. 手工验收：Finder / Notion / Chrome 三个目标各跑通「读树 → 点元素 → 输入 → 截图」

M0 跑通后再接 MCP，避免「协议还没稳就接 Claude」。
