# Codex++ 插件体系设计

日期：2026-10-30
状态：P0 实施中，P1~P4 为规划
相关：`docs/specs/2026-09-30-extension-api-design.md`（拓展 API 现状）、`EXTENSIONS.md`（开发指南）

---

## 一、目标

让 Codex App 变成「能力都可以由插件贡献」的形态：一个插件包可以同时改界面、拦截协议、改模型请求、
挂 Codex 原生 hooks / MCP / skills，并且能干净地启用、停用和热重载。

参照对象是一个采用「万物皆插件」架构的 agent harness：它的 agent 循环、工具、会话存储、模型适配、UI
都是挂在同一棵插件树上的插件，没有特权内核。它靠四个机制撑起这个形态：

1. **注册即可逆副作用**：每次注册（事件监听、工具、UI 项）在插件卸载时自动撤销，热重载与启停因此干净。
2. **依赖注入**：插件声明依赖的服务，服务就绪前等待、消失后跟着卸载。
3. **waterfall 中间件**：在关键点（发请求前、工具执行前后、流式输出、系统提示词组装）用 `next()` 链式拦截。
4. **类型化 UI 插槽**：前端插件往具名 slot 里贡献组件，而不是直接改 DOM；配置 schema 自动生成设置表单。

## 二、约束：我们不拥有内核

Codex 的 agent 循环在 Rust app-server 里，渲染进程是闭源打包的 Electron 页面。我们做不到「内核也是插件」，
只能在 Codex++ 能控制的层面上提供插件挂载点。好在这些层面已经覆盖了参照对象的大部分能力缝：

| 层 | Codex++ 现状 | 对应能力缝 |
| --- | --- | --- |
| ① 界面层（CDP 注入 renderer） | `window.codexPlus` 已有 rowAction / navEntry / menuItem / page 四种注册 | UI slot |
| ② 协议层（renderer 内拦截 app-server JSON-RPC） | 插件市场已接管 `plugin/*`（`62-plugin-market-adapter.js`） | 会话 / 轮次事件、slash 命令 |
| ③ 模型网络层（本地网关、协议代理） | 路由、Key 轮换、本地压缩（`gateway/`、`protocol_proxy/`） | 请求改写、流改写、压缩、重试 |
| ④ Codex 原生 hooks | 仅在切换配置时保留 hooks 定义 | SessionStart / UserPromptSubmit / PreToolUse / PostToolUse / Stop |
| ⑤ Codex 原生插件 / MCP / skills | 插件市场能安装 `.codex-plugin` 包 | 工具注册（由 Codex++ 聚合 MCP 暴露） |

明确不做：替换 agent 循环、替换会话存储（Codex 自己写 rollout）、把执行环境整体迁到远端。

## 三、插件包

一个插件是 `user_scripts/` 下的一个目录，根部放 `codexpp.json`：

```
user_scripts/
  hello/
    codexpp.json
    renderer.js
```

```json
{
  "id": "hello",
  "name": "Hello",
  "version": "0.1.0",
  "description": "示例插件",
  "engines": { "codexpp": 1 },
  "renderer": ["renderer.js"],
  "permissions": { "routes": ["/diagnostics/log"] }
}
```

| 字段 | 说明 |
| --- | --- |
| `id` | 必填，`[A-Za-z0-9_-]`，与目录名无关；用于诊断与插件间引用 |
| `name` / `version` / `description` | 展示用 |
| `engines.codexpp` | 需要的拓展 API 版本（整数）。宿主版本低于它时不加载，在管理页显示原因 |
| `renderer` | 字符串或数组，按顺序拼进同一个 IIFE 执行。路径只能指向包内文件 |
| `permissions.routes` | 插件通过 `ctx.call` 能调的后端路由，必须是全局白名单的子集 |

单文件 `.js` 脚本继续支持，行为不变。P1 起清单会再增加 `protocol` / `gateway` / `hooks` / `mcp` /
`skills` / `configSchema` 等面，未知字段在 P0 一律忽略，保证前向兼容。

## 四、运行时内核（renderer）

入口 `codexPlus.plugin(definition)`，`definition` 是 `apply(ctx)` 函数，或 `{ id, apply }`：

```js
codexPlus.plugin((ctx) => {
  ctx.registerPage({ title: "Hello", render: ({ container }) => { container.textContent = "hi"; } });
  ctx.setInterval(tick, 1000);
  ctx.listen(window, "focus", onFocus);
  ctx.on("other-plugin/ready", onReady);
  ctx.intercept("hello/greeting", (value, next) => next(value + "!"));
});
```

`ctx` 上的每个注册都绑定到这个插件实例的寿命上：

| 能力 | 说明 |
| --- | --- |
| `ctx.effect(fn)` | 立即执行 `fn`，它返回的函数在卸载时调用；返回提前撤销用的函数 |
| `ctx.registerRowAction / NavEntry / MenuItem / Page` | 同 `codexPlus.register*`，归属自动绑定，卸载时自动注销 |
| `ctx.on(event, handler)` / `ctx.emit(event, ...args)` | 插件间事件总线，监听随卸载撤销 |
| `ctx.intercept(name, handler, { order })` / `ctx.waterfall(name, value)` | 中间件链。`handler(value, next)` 调 `next(v)` 交给下一个，不调即短路；没有任何中间件时原样返回 |
| `ctx.listen(target, type, handler, options)` | `addEventListener` 的自动撤销版 |
| `ctx.setTimeout / setInterval` | 定时器自动清理 |
| `ctx.call(route, payload)` | 包插件只能调清单 `permissions.routes` 里声明的路由 |
| `ctx.toast / fail` | 同 `codexPlus` 上的同名方法，归属自动绑定 |
| `ctx.dispose()` / `ctx.disposed` | 主动卸载 / 是否已卸载 |

卸载顺序：所有副作用按注册的逆序撤销，单个撤销抛错只记进该插件的失败通道，不影响其余撤销。

**与热重载的衔接**：插件在脚本同步初始化期间创建时，内核自动向用户脚本运行时登记一个清理函数
（`registerCleanup`），重载时整体卸载。只用 `codexPlus.plugin` 写的脚本因此天然支持页内热重载，
不再因「没登记清理」而触发整页刷新。

**错误隔离**：`apply`、事件处理器、中间件都经 `runCodexPlusExtensionCallback` 包一层，抛错记进该脚本
的状态通道。中间件抛错时跳过它，把原值交给下一个。

不做的事：依赖注入（P0 插件之间只有事件与 waterfall 两种耦合）、配置 schema 与设置表单（P1）。

## 五、分阶段

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| **P0** | 内核（ctx / effect / 事件 / waterfall / 自动清理）；`codexpp.json` 包加载、版本校验、路由权限；管理页显示包信息 | 一个包插件能注册页面、热重载不刷新页面、越权路由被拒 |
| P1 | 协议层类型化事件（turn 开始 / 结束、item 完成、thread 切换）与中间件；扩充 UI slot（输入框附近、消息节点装饰、右侧面板、设置项）；`configSchema` 自动生成设置表单 | 用插件实现一个「轮次结束提示」不依赖 Codex DOM |
| P2 | Codex hooks 代理：Codex++ 作为统一 hook 命令写入 config，按事件分发给插件声明的 hook；聚合 MCP：插件声明工具，由 Codex++ 的 MCP 服务暴露给 Codex | 插件可阻塞危险命令、注册一个工具被模型调用 |
| P3 | 网关中间件：请求 / 响应走子进程 JSON 协议（与 Codex hooks 同一套 stdin/stdout 约定），不嵌入 JS 引擎 | 插件可往系统提示词追加段落 |
| P4 | 脚本市场与插件市场合并；安装时校验 `engines`，展示权限 | 市场能装包插件 |
| 持续 | 把内置功能（宠物、听写、轮次回退、用量状态栏）逐个拆成内置插件 | API 足够表达内置功能 |

## 六、风险

- **Codex 升级**：DOM 与协议随版本变。插件只依赖我们承诺的 slot / 事件契约，由 Codex++ 适配层吸收变化，
  契约测试守住。
- **安全**：插件与 Codex 页面同权，P3 起还会跑子进程。不做沙箱（参照对象同样不做），靠权限声明、
  管理页展示与市场审核。
- **扫描性能**：拓展插入的节点必须带 `data-codex-plus-ext`，内核的 `register*` 已经自动处理（issue #1960）。
