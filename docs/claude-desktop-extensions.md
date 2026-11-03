# Claude Desktop 拓展适配

Claude Desktop 的 **Code 本地会话**通过官方 Claude Code Mods 接入 Codex++ 拓展。第一版支持输入框上方的按钮、模型可调用的后台工具，以及会话和回合事件。Chat、Cowork、远程会话和 DOM 注入不在本次适配范围内。

在「万物皆拓展」顶部切换到 **Claude**，开启「启用 Claude 拓展」，再重新启动增强版 Claude。页面只列出声明 `claudeDesktop` 的本地拓展；Claude 的总开关和每项开关与 Codex 独立，切回 Codex 会保留原来的页签。Claude 总开关默认关闭，统一在此管理。现有市场还没有宿主兼容声明，因此只在 Codex 下展示。

插件通过本次增强版启动进程的 `CLAUDE_CODE_PLUGIN_DIRS` 加载，不写全局 Claude 设置，也不改 Codex 的拓展接口。管理页不会将 Codex 的加载状态当作 Claude 的运行状态；Claude 已启用的条目标记为「重启后应用」。通用 DOM 脚本运行时尚未接入。

Claude Code 需支持 Mods 及 `$.mcp.connect`；本次已在 Claude Code 2.1.314 的官方插件校验和测试引擎中验证。请使用带有相应能力的 Claude Desktop，较旧版本不保证兼容。

## 清单

在现有 `codexpp.json` 中新增 `claudeDesktop`，方法名对应同一份 `backend.script` 返回对象中的方法：

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
