# Codex++ Computer Use：原生直通方案

- 日期：2026-10-23
- 取代：[2026-10-23-codexpp-computer-use-bridge-design.md](./2026-10-23-codexpp-computer-use-bridge-design.md) 的 §2.5 路线（`cua_repl` + 薄适配层）
- 依据：[codexapp CU 与 Chrome 移植可行性分析](../reports/2026-10-23-codexapp-cu-and-chrome-porting-analysis.md)
- 状态：**M1 已实现**（`acbc1ef5`）。M2 审批以策略开关形式实现；M3 turn 边界经实测不做，§2.2 的 diff 行为现版本已不再出现，见 [2026-10-25 一致性核对](../reports/2026-10-25-cu-codex-parity-audit.md)

---

## 0. 一句话结论

现在这套是**二阶模拟**：`Codex++ → cua-repl(JS code-mode) → @oai/sky → SkyComputerUseService`，
然后在 Rust 里用 `build_sky_call` / `reshape_tool_result` / `decorate_state`
把这 10 个离散工具重新造了一遍。

而 Codex 给模型用的**不是** `cua-repl`，是另一个 server：`SkyComputerUseClient mcp`。
它对外就是一个标准 stdio MCP server，唯一门槛是**直接父进程必须是 OpenAI 签名的 node**。
垫一层签名 node 之后，工具面/信封/diff/截图全部原样透传即可——还原度不再靠对齐，而是结构性成立。

---

## 1. Codex 到底装了几个 CU server

`~/.codex/config.toml` 里有两条，是**两个不同的东西**：

| 注册项 | 命令 | 工具面 | 用途 |
|---|---|---|---|
| `[mcp_servers.computer-use]` | `~/.codex/computer-use/Codex Computer Use.app/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient mcp` | **10 个离散工具** | **模型实际调用** |
| `[mcp_servers.node_repl]` + 插件 `cua_repl` | `cua_node/bin/node_repl` + `@oai/cua-repl/bin/cua-repl.mjs` | `js / js_reset / turn_ended` | code-mode 外壳 + 浏览器桥 |

`SkyComputerUseClient mcp` 的完整工具面（实测 `tools/list`，逐字）：

```
list_apps, get_app_state, click, perform_secondary_action,
set_value, select_text, scroll, drag, press_key, type_text
```

每个工具都带 `annotations`（`readOnlyHint` / `destructiveHint` / `idempotentHint` / `openWorldHint`），
`list_apps` 与 `get_app_state` 是 `readOnlyHint:true`，其余都是 `false`。

`cua_repl` 插件的 `.mcp.json` 里写着 `"enabled_tools": ["js","js_reset","turn_ended"]`
与 `"omit_tools_from": ["code_mode","deferred"]`——它是 Codex 内部**另一条**路径，不是模型面。

**会话记录佐证**：`~/.codex/sessions/2026/10/23/rollout-…T20-11-43….jsonl` 里模型调用的是
`mcp__computer_use__click` / `get_app_state` / …，`turn_ended` 出现 **0 次**。

> 当前实现接的是第二个。这就是所有还原度问题的根源。

---

## 2. 决定性实测

### 2.1 谁能拉起原生 server

| 启动方式 | `initialize` | `tools/list` | `tools/call` |
|---|---|---|---|
| `zsh` 直接执行 | ✅ | ✅ | ❌ `-10000 Sender process is not authenticated` |
| `python3` 拉起 | ✅ | ✅ | ❌ 同上 |
| **OpenAI 签名的 node 直接拉起** | ✅ | ✅ | ✅ 真实数据 |
| node → `sh` → client | ✅ | ✅ | 未取得响应（见 §2.4） |

签名事实：

```
/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node
  Authority=Developer ID Application: OpenAI OpCo, LLC (2DC432GLL2)
```

服务端校验字段（从 `SkyComputerUseClient` 二进制提取）：
`_senderParentTeamID` / `_senderResponsibleTeamID` / `_senderResponsibleSigningID` /
`_senderResponsibleBundleID` / `_senderResponsibleExecutable`，失败码 `-10000`。

**结论：只需要一层 OpenAI 签名的父进程。** 不需要重签、不需要改 TCC。

### 2.2 原生返回长什么样

`get_app_state app=Finder`：

```text
Computer Use state (CUA App Version: 1001540)
<app_state>
App=/System/Library/CoreServices/Finder.app/ (bundleID com.apple.finder, pid 479)
The following is a diff from the previous accessibility tree for Window: "a93ded4b1b47" with ~ and + representing changed and added elements, respectively. Removed elements are summarized by ID range.
~			54 文本 Value: 选择了1项（共1项），30.47 GB可用, ID: statusBarText
Selected:
	38 row (selected)

Note: Pay special attention to the content selected by the user. ...
</app_state>
```

紧接着第二次同 app 调用：

```text
Computer Use state (CUA App Version: 1001540)
<app_state>
App=/System/Library/CoreServices/Finder.app/ (bundleID com.apple.finder, pid 479)
There has been no change in the accessibility tree for Window: "a93ded4b1b47".
Selected:
	38 row (selected)
...
</app_state>
```

- 文本长度：首次 707 字符；无变化时 **527 字符**
- 截图：`image/jpeg`，**1840×928**（原分辨率，不缩放）
- 响应带 `_meta.codex/telemetry.span`

对照当前代理（`get_app_state app=QQ`）：

- 文本 **7574 字符**（全量树，因为写死 `disableDiff:true`）
- 截图 **1568×791**（`sips -Z 1568` 缩过）
- 响应无 `_meta`

### 2.3 审批由原生 server 自己发

未事先批准的 app，原生 server 会主动发起 server→client 请求：

```json
{"jsonrpc":"2.0","id":"<uuid>","method":"elicitation/create","params":{
  "message":"Allow ChatGPT to use \"Finder\"?",
  "_meta":{"persist":["always"]},
  "requestedSchema":{"properties":{},"type":"object"}}}
```

`@oai/sky` 的 `computer-use-policy.js` 里还有更完整的一版（供 code-mode 路径用）：
`Allow Computer Use to use "<displayName>"?`，附带 `codex_approval_kind:"mcp_tool_call"`、
`connector_id:"computer-use"`、`riskLevel`、`persist:["session"]` 或 `["session","always"]`。
答复语义：`accept` / `decline` / `cancel` → `accepted` / `declined` / `canceled`；
`accept` 且 `_meta.persist=="always"` 才持久化。

**实测：客户端即使没有声明 elicitation 能力，server 也照样发、照样阻塞**
（`get_app_state app=TextEdit` 15 秒超时，期间只收到这一条 elicitation）。

### 2.4 中间层不能有

`node → sh → SkyComputerUseClient` 的探针没有拿到 `tools/call` 响应。
尚未区分是「归属校验被中间 shell 破坏」还是探针本身的问题，
但既然 `node → client` 直挂已经够用，实现上**一律不插入 shell**。

---

## 3. 现有实现的差异清单

### A. 真 bug（会失败或产生错误结果）

| # | 问题 | 证据 | 后果 |
|---|---|---|---|
| A1 | **MCP stdout 被污染** | `shrink_screenshot` 用 `.status()`，`sips` 继承 stdout，打印输入/输出路径；两次实测 stdout 里各多出两条裸行 | 严格 MCP 客户端解析失败 |
| A2 | **截图与坐标空间不一致** | 原生 1840×928，代理 1568×791；schema 写明 `x/y` 是 screenshot pixel coordinates | 模型按缩小图读的坐标点下去偏约 1.17× |
| A3 | **丢 `frontmost` 标记** | 原生 `DeepSeek Harness — … [frontmost, running, …]`；`ListedApp::to_line` 不输出 | 模型无法判断前台应用 |
| A4 | **`click_count` 类型错** | 原生 `"type":"integer"`，代理写 `"type":"number"` | 严格 schema 校验告警 |
| A5 | **`annotations` 全丢** | 原生每个工具都有，代理 `tool_definitions()` 一个都没有 | 客户端无法区分读写，无法据此做权限策略 |

### B. 还原度偏差（不报错，但和 Codex 不一样）

| # | 问题 | 现状 | 原生 |
|---|---|---|---|
| B1 | **diff 被强行关掉** | `disableDiff:true` 写死；每个动作后再取一次全量状态 | 默认返回 diff / `no change`；QQ 场景 7574 vs 527 字符，**14× token 差** |
| B2 | **丢「用户动过」信号** | 无 | `The user changed 'QQ'. Re-query the latest state with get_app_state before sending more actions.` |
| B3 | **没有 turn 生命周期** | `turn_ended` 不在 tools/list，从不发送 | 服务端有 `ComputerUseIPCCodexTurnEndedRequest`，每轮结束重置 diff 基线与 app 会话 |
| B4 | **信封是手搓的** | `plutil` + `pgrep -x` + `lsappinfo` 反推 `App=… (bundleID …, pid …)` | service 直接给；多实例 / Electron / Helper 进程会猜错 |
| B5 | **`CUA App Version` 是猜的** | 取插件目录末段，兜底硬编码 `1001540` | service 自己的版本 |
| B6 | **审批被静默放行** | `elicitation/create` 一律回 `accept`，只写日志 | 见 §2.3：应弹卡、可拒绝、可持久化 |
| B7 | **instructions 自相矛盾** | `append_usage_notes` 让模型用 `cua.listBrowsers()` / `cua.getTab` | `js` 不在代理的 tools/list 里，模型永远调不到 |

---

## 4. 方案：原生直通

```text
Claude Code / Claude Desktop
        │  stdio MCP
        ▼
CodexPlusPlus --codex-plus-cu-mcp      （Rust：字节转发 + 审批拦截）
        │  stdio
        ▼
cua_node/bin/node  shim.mjs            （OpenAI 签名，唯一需要的一层）
        │  spawn(stdio: inherit)
        ▼
SkyComputerUseClient mcp               （原生 10 工具，零改动）
        │  unix socket（4B LE + JSON-RPC）
        ▼
SkyComputerUseService
```

### 4.1 shim（约 5 行，随二进制内嵌）

```js
// assets/cu/shim.mjs —— 由 OpenAI 签名的 node 执行，直接拉起原生 CU server。
// 中间不能再有 shell / 其它进程：服务端校验的是直接父进程的签名。
import { spawn } from "node:child_process";
const client = process.env.CODEX_PLUS_CU_CLIENT;
const child = spawn(client, ["mcp"], { stdio: "inherit" });
child.on("exit", (code, signal) => process.exit(code ?? (signal ? 1 : 0)));
for (const sig of ["SIGTERM", "SIGINT"]) process.on(sig, () => child.kill(sig));
```

用 `CODEX_PLUS_CU_CLIENT` 传路径，不拼命令行（避免空格路径被 shell 解析，也避免多插一层）。

### 4.2 Rust 侧职责

退化为三件事：

1. **转发**：`tools/list`、`tools/call`、结果、通知——**不解析、不改写、不合成**。
   只做 id 重映射（客户端 id ↔ 子进程 id）与会话生命周期管理。
2. **声明 elicitation**：`initialize` 时向子进程声明 `capabilities.elicitation`，
   因为 Claude Code 不声明，不声明的话原生 server 一样会阻塞（§2.3）。
3. **审批**：拦截子进程的 `elicitation/create`，弹真卡，按用户选择回 `accept/decline/cancel`。

不需要做的：信封、diff、截图、`App=` 行、`frontmost`、版本号、schema——全部由原生产出。

### 4.3 删除清单

`crates/codex-plus-core/src/computer_use_proxy.rs` 中以下内容作废（约 700 行）：

```
tool_definitions        build_sky_call          rewrite_tool_call
reshape_tool_result     extract_payload         decorate_state
split_app_instructions  AppProcess              format_list_apps
ListedApp               launch_services_bundle_path  guess_bundle_path
cua_app_version         installed_plugin_version
read_screenshot_file    shrink_screenshot       percent_decode / split_data_url
append_usage_notes      @@CU@@ 私有协议
```

`crates/codex-plus-core/src/computer_use.rs` 保留定位/注册/卸载/自检，改动：

- `locate_runtime` 新增 `SkyComputerUseClient` 与 `cua_node/bin/node` 两个绝对路径
- 不再需要 `SKY_CUA_SERVICE_PATH` / `CODEX_CLI_PATH` 那套 cua-repl 环境变量（仅降级路径保留）
- `PROBE_JS` 改为直接 `tools/list` + `tools/call list_apps`（不再依赖 `js` 工具）

### 4.4 浏览器面单独拆出

原生 CU server **没有**浏览器工具。当前靠 `cua-repl` 的 `js` 里的 `cua.*` 兼职，
这条路在直通方案下不存在，因此：

- **CU = 原生直通**（`codex-plus-cu`）
- **浏览器 = 独立 server**（`codex-plus-browser`，继续走 `cua-repl` 或自建 MV3 + NMH）

两者不混在一个工具面里。这一点是刻意的：Codex 自己也是 `computer-use` 与 `node_repl`
两个 server 分开注册的。

---

## 5. 审批模型

### 5.1 三种答复

| 用户选择 | 回给子进程 | 原生行为 |
|---|---|---|
| 允许一次 | `{"action":"accept","content":{}}` | 放行本次会话 |
| 始终允许 | `accept` + 记入持久化 | 下次仍会发 elicitation，但可直接命中持久化记录 |
| 拒绝 | `{"action":"decline"}` | 抛 `Computer Use was not approved to use <app>`，`isError:true` |
| 超时/取消 | `{"action":"cancel"}` | 同 decline 的可见后果 |

### 5.2 实现要点

1. **必须阻塞**：收到 elicitation 后在答复前不转发任何后续请求，否则会打乱服务端状态。
2. **超时兜底**：默认 120s，超时回 `cancel`，绝不让 Claude 会话永久挂住。
3. **持久化**：`~/.codex-plus/cu/approvals.json`，键为 bundle id，值为
   `{"tier":"read|click|full","decidedAt":<unix>,"mode":"always|session"}`。
   `session` 只在本进程生命周期内有效。
4. **审计**：`~/.codex-plus/cu/audit.jsonl` 追加记录（时间、bundle id、工具名、结果），
   **不记录输入文本**。现有的 `elicitation.log` 并入此处。
5. **UI 承载**：MCP 进程是独立进程，不能直接弹 Tauri 窗口。走
   「本地 socket → 管理器 GUI」或 macOS 原生 `NSAlert`（经 `osascript` 之外的方式）。
   前者便于复用现有页面，后者不依赖管理器是否在运行——推荐前者 + 后者兜底。

---

## 6. 文件级改动清单

```
assets/cu/shim.mjs                                    新增（内嵌资源）
crates/codex-plus-core/src/computer_use.rs            定位：+client/node 路径；自检改写
crates/codex-plus-core/src/computer_use_proxy.rs      重写为字节转发 + 审批拦截（约 -700 行）
crates/codex-plus-core/src/computer_use_approval.rs   新增：审批存储 / 审计 / 进程间通道
apps/codex-plus-manager/src-tauri/src/computer_use_commands.rs
                                                      新增 approvals_list / approvals_revoke /
                                                      approval_respond
apps/codex-plus-manager/src/ComputerUsePage.tsx       新增「已授权应用」区块
scripts/cu-parity.mjs                                 新增：golden A/B 对照脚本（见 §7.2）
```

不改：`Cargo.toml`（无新依赖）、`Info.plist`（TCC 身份沿用 Codex++ 主二进制）。

> 注：TCC 授权（辅助功能 / 屏幕录制）仍然绑在 `SkyComputerUseService` 自己的 bundle id 上，
> 由 ChatGPT.app 那套装好时完成，本次改动不涉及。

---

## 7. 里程碑与验收

### 7.1 里程碑

#### M1 直通（1–2 天）

- shim + 字节转发；删除 §4.3 全部逻辑
- `tools/list` 与原生**逐字节相等**（去掉 `_meta` 与版本字段后 diff 为空）
- 连续两次 `get_app_state`：第二次必须是 `There has been no change in the accessibility tree`
- 截图 1840×928（原分辨率）
- `list_apps` 含 `frontmost`
- stdout 中不存在任何非 JSON 行

#### M2 审批（2–3 天）

- 首次接管弹卡；拒绝后返回 `isError:true` 且文案为原生那句
- 「始终允许」后不再弹（但 server 仍会发 elicitation，代理直接命中持久化）
- 超时回 `cancel`，会话不挂
- 管理器页面能看到并撤销已授权应用

#### M3 turn 边界（可选，1 天）

- 进程生命周期 ≈ 会话：Claude 会话结束 → 代理退出 → 子进程退出 → service 释放 app
- 若需要「按轮」而非「按进程」重置 diff 基线，用空闲超时（默认 90s）补一次 turn 语义

### 7.2 golden 对照脚本

`scripts/cu-parity.mjs` 用固定 JSON-RPC 脚本分别打「原生直连（经 shim）」与「经代理」，
归一化（去时间戳 / telemetry / 绝对临时路径）后逐项 diff：

```
list_apps · get_app_state ×2（diff 语义）· get_app_state（新 app，触发 elicitation）
click(element_index) · click(x,y) · type_text · press_key · scroll · select_text · set_value
```

任一项不一致即失败。这条脚本把「还原度」从人工比对变成可回归的断言。

每个里程碑完成后按仓库规范 `node scripts/relaunch-local.mjs` 构建启动实测。

---

## 8. 风险与回退

| 风险 | 影响 | 处置 |
|---|---|---|
| OpenAI 收紧校验（查祖父进程 / 加 nonce / 校验 bundle） | 直通失效 | 保留现有 `cua-repl` 路径作为**降级开关**（它本就是官方设计的「任意宿主可拉起」入口） |
| 依赖 `ChatGPT.app` 在位 | 与现状一致（当前也依赖） | 自检里明确分类，不再笼统报「服务未就绪」 |
| elicitation 阻塞 | 会话挂死 | 强制超时 + `cancel` 兜底（§5.2） |
| Codex 版本更新改变工具面 | 模型用法漂移 | 直通方案下无需改代码；golden 脚本会先报出来 |
| 与 Codex 自己的 CU 同时操作同一 app | 抢窗口 / diff 基线互相污染 | UI 提示「二者择一」；代理侧不做跨进程协调 |

---

## 9. 附录

### 9.1 关键路径

```
# 原生 CU server（本次直通目标）
~/.codex/computer-use/Codex Computer Use.app/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient

# 唯一可用的父进程（OpenAI 签名）
/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node

# 现有 cua-repl 路径（降级用）
/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/cua-repl/bin/cua-repl.mjs

# 会话信封样本
~/.codex/sessions/2026/10/23/rollout-*.jsonl
```

### 9.2 复现探针

```bash
# 直连（会失败，用于确认 -10000）
printf '%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"p","version":"1"}}}' \
 '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
 '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_apps","arguments":{}}}' \
| ~/.codex/computer-use/"Codex Computer Use.app"/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient mcp

# 经 OpenAI 签名 node 直连（会成功）
/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node -e '
const {spawn}=require("child_process");
const c=spawn(process.env.CLIENT,["mcp"],{stdio:["pipe","pipe","inherit"]});
for (const m of [
 {jsonrpc:"2.0",id:1,method:"initialize",params:{protocolVersion:"2025-06-18",capabilities:{elicitation:{}},clientInfo:{name:"p",version:"1"}}},
 {jsonrpc:"2.0",method:"notifications/initialized"},
 {jsonrpc:"2.0",id:2,method:"tools/call",params:{name:"list_apps",arguments:{}}}])
  c.stdin.write(JSON.stringify(m)+"\n");
c.stdout.pipe(process.stdout);
setTimeout(()=>c.kill("SIGKILL"),15000);'
```

### 9.3 关键字符串（定位用）

```
# 认证
Sender process is not authenticated / _senderParentTeamID / _senderResponsibleSigningID

# 信封
Computer Use state (CUA App Version: / <app_state> / <app_specific_instructions>
The following is a diff from the previous accessibility tree
There has been no change in the accessibility tree
The user changed '<App>'. Re-query the latest state

# 审批
elicitation/create / Allow ChatGPT to use / computer-use-persisted-state
codex_approval_kind / persist

# turn
ComputerUseIPCCodexTurnEndedRequest / turnEnded
```

### 9.4 与既有文档的关系

- 本文取代 `2026-10-23-codexpp-computer-use-bridge-design.md` 的 §2.5「复用 cua_repl」路线。
- 该文档 §2.5 的三条实测结论（cua-repl 是标准 stdio MCP、外部宿主能读到窗口、
  Claude Code 缺 elicitation 会断）**依然成立且仍然是降级路径的基石**，
  只是不再是主路径。
- 自研引擎（该文档第 5 节）继续作为 P2 备选，优先级不变。
