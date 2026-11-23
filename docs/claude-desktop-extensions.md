# Claude Desktop 拓展适配

一个 `codexpp.json` 拓展包在 Claude Desktop 里有两部分，都写在清单的 `claudeDesktop` 块里，可以只用其中一部分：

- **页面部分**（`renderer`、`styles`、`methods`）：脚本和样式注入 Claude 的页面，Chat、Cowork、Code 页面都生效。脚本里用的是和 Codex 同名的 `window.codexPlus` 接口，可以读配置、存数据、调自己的后台。见下文「页面部分」。
- **Code 会话部分**（`buttons`、`events`、`tools`）：通过官方 Claude Code Mods 接入 Code 本地会话，支持输入框上方的按钮、模型可调用的后台工具，以及会话和回合事件。远程会话不在范围内。

在「万物皆拓展」顶部切换到 **Claude**，开启「启用 Claude 拓展」。「已安装」里列出两类：装在页面扩展目录里的（页面扩展和拓展包，格式与扩展市场和 ClaudeMom 相同），和 Codex++ 拓展目录里声明了 `claudeDesktop` 的拓展包。Claude 的总开关和每项开关与 Codex 独立；Claude 总开关默认关闭，只管 Codex++ 拓展目录里的拓展包。

两部分的生效时机不同：

- 页面部分：开关、配置、装卸之后刷新 Claude 页面（⌘R / Ctrl+R）就按新的来，完整功能模式和补丁版都一样。
- Code 会话部分：插件通过启动进程的 `CLAUDE_CODE_PLUGIN_DIRS` 加载，不写全局 Claude 设置；增删拓展或修改清单后要重新启动 Claude。

Claude Code 需支持 Mods 及 `$.mcp.connect`；Code 会话部分已在 Claude Code 2.1.314 的官方插件校验和测试引擎中验证。请使用带有相应能力的 Claude Desktop，较旧版本不保证兼容。

**兼容提醒**：`renderer`、`styles`、`methods` 是 2.9.0 新增的字段。更早的 Codex++ 不认识它们，会把整个包判成清单有误（Codex 那边也不加载）。要兼顾旧版用户时，在清单里写 `"engines": { "codexpp": ">=2.9.0" }`，旧版会给出「版本不满足」而不是「清单有误」。

## 页面部分

```json
{
  "id": "my-extension",
  "name": "My Extension",
  "permissions": { "routes": ["/backend/status"] },
  "backend": { "script": "backend.mjs" },
  "config": { "greeting": { "type": "string", "default": "hi" } },
  "claudeDesktop": {
    "renderer": ["renderer.js"],
    "styles": ["style.css"],
    "methods": ["status"]
  }
}
```

| 字段 | 说明 |
| --- | --- |
| `claudeDesktop.renderer` | 字符串或数组，包内的 `.js`；按顺序拼进同一个作用域执行。顶层的 `renderer` 是给 Codex 页面的，不会进 Claude |
| `claudeDesktop.styles` | 数组，包内的 `.css`。只想换样式的包可以只写这一项 |
| `claudeDesktop.methods` | 页面可以调用的后台方法（`ctx.backend.call`）。写了它就要同时声明 `backend.script`；只有脚本和样式的包不需要后台 |

文件路径要用包内的相对路径，不能有隐藏文件或 `..`，每类最多 32 个。这三个字段的规则在 ClaudeMom 的 `claude-local` 里，单独使用 ClaudeMom 时也认同样的清单。

### 页面里能用什么

脚本里的全局对象仍是 `window.codexPlus`，成员名、`ctx` 的成员名和 `codexPlus.constants` 的键与 Codex 里一致，所以一份代码两边都能跑。用下面两个成员区分宿主：

```js
const host = codexPlus.host ?? "codex";            // Claude 里是 "claudeDesktop"
if (codexPlus.supports?.("registerComposerAction") ?? true) { /* 有输入框按钮这个位置 */ }
```

目前在 Claude 里已经接上的：

- 拓展实例与生命周期：`codexPlus.extension`、`ctx.effect`、`ctx.on` / `emit`、`ctx.intercept` / `waterfall`、`ctx.provide` / `get` / `inject`、`ctx.listen`、`ctx.setTimeout` / `setInterval`、`ctx.onScan`、`ctx.dispose`。
- 界面：`toast`、`ask`、`registerPage`（整页覆盖层）、`registerNavEntry`、`registerFloatingTab`、`registerShortcut`、`registerTheme`。页面入口和悬浮页签在右下角的悬浮按钮里。
- 数据：`ctx.config`、`ctx.storage`、`ctx.locale`、`ctx.backend.call` / `on`、`ctx.call`（只开放 `/diagnostics/log`、`/backend/status`，同样要在 `permissions.routes` 里声明）。

和 Codex 不一样的地方：

- **其余挂载点还没有接上**：`registerComposerAction`、`registerCommand`、`registerInputTrigger`、`registerMessageAction`、`registerMessageDecoration`、`registerToolView`、`registerSlot`、`registerSidePanel`、`registerRowAction`、`registerSettingsSection`、`registerSettingsPanel`、`registerMenuItem`。调用它们不会报错，只是不出现；「万物皆拓展」的那一行会列出这个拓展用了哪些 Claude 里还没有的接口。
- **取数据的接口会拒绝**：`ctx.llm.*`、没开放的路由、没声明的后台方法，返回的 Promise 以 `CodexPlusUnsupportedError` 拒绝（`error.code === "unsupported"`）。
- **密钥类配置不进页面**：`config` 里 `secret: true` 的项在页面的 `ctx.config` 里是 `null`，只有后台拿得到。页面里 Claude 自己的脚本和拓展脚本同权，放进页面就等于交给了页面里的所有代码。
- **`ctx.storage` 跟着页面的源走**：登录账号的界面（`claude.ai`）和本机版界面（`app://localhost`）各存各的。要长期保存的数据放后台的 `ctx.dataDir`。
- `ctx.asset()` 目前返回空串；`currentSessionId()` 取地址里的会话 id；`builtins()` 为空。

### 页面调后台

`ctx.backend.call(方法, 参数)` 只能调 `claudeDesktop.methods` 里列的方法，每次调用都会重新核对总开关、拓展开关和声明。后台脚本和 Code 会话部分用的是同一个 `backend.script`；Codex++ 开着时两边调到的是同一个后台实例，状态是通的。Codex++ 没开时：

- 完整功能模式、补丁版的页面调用都会失败，并说明「拓展后台由 Codex++ 托管，但它没有在运行」。
- Code 会话里的按钮、工具照旧可用（退回各自会话进程里托管）。

一个拓展的脚本有语法错误或运行出错，只影响它自己：别的拓展和 Claude 页面照常工作，原因显示在「万物皆拓展」的那一行。

## Code 会话部分的清单

Code 会话部分同样写在 `claudeDesktop` 里，方法名对应同一份 `backend.script` 返回对象中的方法：

```json
{
  "id": "my-extension",
  "name": "My Extension",
  "backend": { "script": "backend.mjs" },
  "claudeDesktop": {
    "buttons": [
      { "id": "status", "label": "Status", "method": "status" }
    ],
    "events": {
      "session.start": "onSession",
      "turn.start": "onTurn",
      "turn.complete": "onTurn",
      "session.end": "onSession"
    },
    "tools": [
      {
        "name": "status",
        "description": "Read extension status",
        "method": "status",
        "inputSchema": {
          "type": "object",
          "properties": {},
          "additionalProperties": false
        }
      }
    ]
  }
}
```

工具输入必须声明 `properties`，只传这些字段；不支持开放的 `additionalProperties`。`tool`、`agentId`、`tool_use_id`、`toolUseId` 是 Claude 事件保留字段，不能作为顶层工具参数。工具在 Claude 中命名为 `mcp__codexpp-extension-host__cpx_<序号>_<name>`，序号由本次加载顺序决定。

## 后台与事件

后台仍使用现有 `export default function create(ctx)` 形式。Claude 的 `ctx.host` 为 `"claudeDesktop"`。按钮方法收到 `{ host, source: "button", buttonId, sessionId }`；工具方法收到清单声明的输入字段。返回字符串或 `{ text: "..." }` 时，按钮会显示通知；模型工具可以返回任意 JSON 值。

事件方法收到 `{ host, event, sessionId }`，按事件附带 `turnId`、`agentId`、`isAborted`、`reason` 或 `source`。不会自动发送提示词、回答或会话记录。清空、恢复和分支会话通过官方 `classic.SessionStart` 转成新的 `session.start` 通知；Mods 热重载也会产生 `session.start`，消费者应允许重复初始化。

会话事件是观测通知，返回值不改变 Claude 的行为。每次普通事件后台调用最多等待 3 秒，`session.end` 最多等待 750 毫秒。多个拓展依次接收事件；`session.end` 同时受 Claude 的退出时限限制，为尽力投递，宿主快速退出时不保证完成。

Claude 使用独立的后台实例和数据目录，避免与 Codex 运行状态混用。第一版不开放 Codex 专属的会话读写、UI、模型调用和跨拓展服务给 Claude 后台；这些宿主 RPC 会明确报错。Node 后台自身的文件、网络和进程能力保持原有语义。

后台经受管插件自己的 stdio MCP 调用，只允许执行 `claudeDesktop` 中列明的方法。每次调用重新检查总开关、拓展启用状态和方法声明。停用后旧按钮可能仍显示到下次重启，但不能继续执行后台。增删拓展或修改清单后，请重新启动增强版 Claude。

## 验证

可按说明安装 [连通测试示例](../tools/claude-extension-demo/README.md)，依次验证按钮、真实回合事件和模型工具共用同一个后台。

源码包含 Mods 行为测试、原生 Claude 插件静态校验及离线测试引擎、清单契约测试、后台隔离测试及 Claude 启动环境测试。原生测试不发送模型请求，也不改用户 Claude 配置；未安装支持 Mods 测试的 Claude Code 时跳过这一项。

官方接口参考：[Mods](https://code.claude.com/docs/en/plugins/mods/overview)、[API](https://code.claude.com/docs/en/plugins/mods/api)、[UI](https://code.claude.com/docs/en/plugins/mods/interface)。
