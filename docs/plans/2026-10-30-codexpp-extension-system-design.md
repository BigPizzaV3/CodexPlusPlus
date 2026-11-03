# Codex++ 拓展体系设计

日期：2026-10-30
状态：P0~P3 已实施（P1、P3 部分），P4 与持续项为规划
相关：`docs/specs/2026-09-30-extension-api-design.md`（拓展 API 现状）、`EXTENSIONS.md`（开发指南）

---

## 命名

- **拓展**：Codex++ 自己注入 Codex 页面的能力，包括单文件脚本拓展与带 `codexpp.json` 的拓展包，
  接口是 `codexPlus.extension(ctx => …)`（早期名 `codexPlus.plugin` 保留为别名）。
- **插件**：只指 Codex 自带的插件（skills / MCP / apps 包），Codex++「插件市场」安装的就是这种。
- **dsh 插件**：dsh 自己的插件，导入后按层落到 Codex++，见 `2026-10-30-dsh-plugin-import-design.md`。

## 一、目标

让 Codex App 变成「能力都可以由拓展贡献」的形态：一个拓展包可以同时改界面、拦截协议、改模型请求、
挂 Codex 原生 hooks / MCP / skills，并且能干净地启用、停用和热重载。

参照对象是一个采用「万物皆插件」架构的 agent harness：它的 agent 循环、工具、会话存储、模型适配、UI
都是挂在同一棵插件树上的插件，没有特权内核。它靠四个机制撑起这个形态：

1. **注册即可逆副作用**：每次注册（事件监听、工具、UI 项）在插件卸载时自动撤销，热重载与启停因此干净。
2. **依赖注入**：插件声明依赖的服务，服务就绪前等待、消失后跟着卸载。
3. **waterfall 中间件**：在关键点（发请求前、工具执行前后、流式输出、系统提示词组装）用 `next()` 链式拦截。
4. **类型化 UI 插槽**：前端插件往具名 slot 里贡献组件，而不是直接改 DOM；配置 schema 自动生成设置表单。

## 二、约束：我们不拥有内核

Codex 的 agent 循环在 Rust app-server 里，渲染进程是闭源打包的 Electron 页面。我们做不到「内核也是拓展」，
只能在 Codex++ 能控制的层面上提供拓展挂载点。好在这些层面已经覆盖了参照对象的大部分能力缝：

| 层 | Codex++ 现状 | 对应能力缝 |
| --- | --- | --- |
| ① 界面层（CDP 注入 renderer） | `window.codexPlus` 已有 rowAction / navEntry / menuItem / page 四种注册 | UI slot |
| ② 协议层（renderer 内拦截 app-server JSON-RPC） | 插件市场已接管 `plugin/*`（`62-plugin-market-adapter.js`） | 会话 / 轮次事件、slash 命令 |
| ③ 模型网络层（本地网关、协议代理） | 路由、Key 轮换、本地压缩（`gateway/`、`protocol_proxy/`） | 请求改写、流改写、压缩、重试 |
| ④ Codex 原生 hooks | 仅在切换配置时保留 hooks 定义 | SessionStart / UserPromptSubmit / PreToolUse / PostToolUse / Stop |
| ⑤ Codex 原生插件 / MCP / skills | 插件市场能安装 `.codex-plugin` 包 | 工具注册（由 Codex++ 聚合 MCP 暴露） |

明确不做：替换 agent 循环、替换会话存储（Codex 自己写 rollout）、把执行环境整体迁到远端。

## 三、拓展包

一个拓展是 `user_scripts/` 下的一个目录，根部放 `codexpp.json`：

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
  "description": "示例拓展",
  "engines": { "codexpp": 1 },
  "renderer": ["renderer.js"],
  "permissions": { "routes": ["/diagnostics/log"] }
}
```

| 字段 | 说明 |
| --- | --- |
| `id` | 必填，`[A-Za-z0-9_-]`，与目录名无关；用于诊断与拓展间引用 |
| `name` / `version` / `description` | 展示用 |
| `engines.codexpp` | 需要的拓展 API 版本（整数）。宿主版本低于它时不加载，在管理页显示原因 |
| `renderer` | 字符串或数组，按顺序拼进同一个 IIFE 执行。路径只能指向包内文件 |
| `permissions.routes` | 拓展通过 `ctx.call` 能调的后端路由，必须是全局白名单的子集 |

单文件 `.js` 脚本继续支持，行为不变。P1 起清单会再增加 `protocol` / `gateway` / `hooks` / `mcp` /
`skills` / `configSchema` 等面，未知字段在 P0 一律忽略，保证前向兼容。

## 四、运行时内核（renderer）

入口 `codexPlus.extension(definition)`，`definition` 是 `apply(ctx)` 函数，或 `{ id, apply }`：

```js
codexPlus.extension((ctx) => {
  ctx.registerPage({ title: "Hello", render: ({ container }) => { container.textContent = "hi"; } });
  ctx.setInterval(tick, 1000);
  ctx.listen(window, "focus", onFocus);
  ctx.on("other-extension/ready", onReady);
  ctx.intercept("hello/greeting", (value, next) => next(value + "!"));
});
```

`ctx` 上的每个注册都绑定到这个拓展实例的寿命上：

| 能力 | 说明 |
| --- | --- |
| `ctx.effect(fn)` | 立即执行 `fn`，它返回的函数在卸载时调用；返回提前撤销用的函数 |
| `ctx.registerRowAction / NavEntry / MenuItem / Page` | 同 `codexPlus.register*`，归属自动绑定，卸载时自动注销 |
| `ctx.on(event, handler)` / `ctx.emit(event, ...args)` | 拓展间事件总线，监听随卸载撤销 |
| `ctx.intercept(name, handler, { order })` / `ctx.waterfall(name, value)` | 中间件链。`handler(value, next)` 调 `next(v)` 交给下一个，不调即短路；没有任何中间件时原样返回 |
| `ctx.listen(target, type, handler, options)` | `addEventListener` 的自动撤销版 |
| `ctx.setTimeout / setInterval` | 定时器自动清理 |
| `ctx.call(route, payload)` | 拓展包只能调清单 `permissions.routes` 里声明的路由 |
| `ctx.toast / fail` | 同 `codexPlus` 上的同名方法，归属自动绑定 |
| `ctx.dispose()` / `ctx.disposed` | 主动卸载 / 是否已卸载 |

卸载顺序：所有副作用按注册的逆序撤销，单个撤销抛错只记进该拓展的失败通道，不影响其余撤销。

**与热重载的衔接**：拓展在脚本同步初始化期间创建时，内核自动向用户脚本运行时登记一个清理函数
（`registerCleanup`），重载时整体卸载。只用 `codexPlus.extension` 写的脚本因此天然支持页内热重载，
不再因「没登记清理」而触发整页刷新。

**错误隔离**：`apply`、事件处理器、中间件都经 `runCodexPlusExtensionCallback` 包一层，抛错记进该脚本
的状态通道。中间件抛错时跳过它，把原值交给下一个。

不做的事：依赖注入（P0 拓展之间只有事件与 waterfall 两种耦合）、配置 schema 与设置表单（P1）。

## 五、分阶段

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| **P0** ✅ | 内核（ctx / effect / 事件 / waterfall / 自动清理）；`codexpp.json` 包加载、版本校验、路由权限；管理页显示包信息 | 一个拓展包能注册页面、热重载不刷新页面、越权路由被拒 |
| **P1** ✅（部分） | 已做：会话事件 `codex/*`、`codex/request` 请求中间件、输入区按钮插槽、斜杠命令、`ctx.storage`、清单 `config` 配置项与管理器自动生成的设置表单、对话造拓展的页面查看与截图、消息操作按钮与消息装饰插槽、右侧面板插槽。`codex/request` 依赖 app-server 客户端类捕获，该捕获在新版 Codex 上失效，另案修复 | 拓展订阅轮次结束、往输入区加按钮 |
| **P2** ✅ | Codex hooks：拓展声明 `hooks`，Codex++ 只写固定分发条目（用户信任一次），`--codex-plus-hook` 每次起一个 Node 跑完命中的钩子并按协议合并。模型工具：拓展声明 `tools`，由 `codex_plus_extensions` MCP 对外提供 | 拓展可阻塞危险命令、给提示词补上下文 |
| **P3** ✅ | 网关请求中间件 `gateway.request`：本地代理解析请求体后按顺序改写；网关响应流中间件 `gateway.response`：`(context, next) => 异步可迭代对象` 包住整条事件流，常驻 Node 工作进程逐事件一问一答，出错超时原样放行 | 拓展可往系统提示词追加段落 |
| **后台** ✅ | 常驻后台 `backend.script`：启动器首次调用时用 Node 拉起，行式 JSON 收发；页面 `ctx.backend.call / on`（事件长轮询）。实例卸载时按实例标识停止，调用前核对脚本 / 配置 / 启用状态，过期重起 | 页面能读写本地文件、后台推事件到页面 |
| **对齐补完** ✅ | `engines.codexpp` 支持 semver 范围；会话查询加 `project` / `observeSession` / `readEventsSince` 与全量搜索缓存；工具管道 `toolPipeline`（preExecute 改参数 / 拦截、execute 代为执行、postExecute 改写结果）；网关失败重试 `gateway.error`；后台服务 `backend.provides` + `provide` / `service()`，页面 / 后台 / 工具 / 钩子跨进程调用；配置项支持 text / array / object / min-max / 多语言说明。`codex/request` 改挂 createRequest；新版 Codex 的请求改在页面与宿主进程之间的 capnweb RPC 帧上拦截（`91-extension-rpc.js`），并补上 `agent/pre-step`（改写 / 拒绝本轮用户输入）与 `agent/request`（改模型、推理强度） | — |
| **再补齐** ✅ | 会话写入 `sessions`（thread/fork、thread/inject_items、turn/start、turn/interrupt，经页面里 Codex 自己的 AppServerManager）；拓展声明模型供应商 `providers` 并入网关供应商池；网关请求中间件按步触发（`context.step`、`{ reject }`）；通用界面插槽；RPC 层的 `agent/pre-step` / `agent/request`（另一会话） | — |
| P4 | 拓展市场支持拓展包；安装时校验 `engines`，展示权限。展开为发布、收录与统计，见 `2026-10-30-extension-publish-discovery-design.md` | 市场能装拓展包 |
| 不做 | 文件监听自动热重载：`docs/user-script-reload.md` 明确「不增加后台轮询或文件监听器」，对话写入后自动重载、手动改文件用「热重载」 | — |
| **服务与数据** ✅ | 只读会话查询服务 `sessionQuery`（页面 / 后台 / 工具三处可用，事件日志形状 `{ type, seq, time, data }`）；拓展间服务 `ctx.provide` / `inject` / `ctx.inject` / `ctx.get`，依赖缺失挂起、提供方消失卸载、回来重新应用；多语言 `locale/<语言>.json` 与 `ctx.locale` | 拓展能搜历史会话、互相调用、随界面语言切换 |
| 持续 | 把内置功能（宠物、听写、轮次回退、用量状态栏）逐个拆成内置拓展 | API 足够表达内置功能 |
| **内置功能改用公开接口**（2026-11-01） | 公开事件 `settings/loaded`：读设置的地方不再点名功能，桌宠、打字特效、容量重试、梦皮、对话宽度、消息导航条、微信会话同步各自订阅；公开中间件 `codex/exchange`（包住一次 app-server 请求）：侧聊保存挂在上面；粘贴修复从启动期全局监听改为 `ctx.listen`，改设置不必重启；微信会话同步成为内置拓展；同页重复注入先卸下旧内核上的内置拓展。仍是开关壳的：历史轮次补全（由 Rust 经 CDP 驱动）、滚动位置恢复、悬浮球两项（独立的悬浮球运行时） | 内置功能卸下即撤销 |
| **内置拓展只剩 apply**（2026-11-01） | 公开 `ctx.onScan(handler, { phase })`（扫描步骤登记在拓展名下，卸下即撤）与公开事件 `settings/changed`（页面里改设置后先按新值、写进后端后各发一次）。原来只靠包装层 sync / scan / settings 钩子的 15 个内置拓展（语音听写、轮次回退、容量重试、会话分享、模型分组、梦皮、插件市场解锁、会话删除、Markdown 导出、会话 ID 角标、滚动位置恢复、消息导航条、微信会话同步、下一步建议、回答大纲）改成各自一个 `apply(ctx)`，包装层不再有私有钩子；扫描循环不再点名已删会话过滤、对话宽度、滚动位置恢复，`setCodexPlusSetting` / `refreshCodexPlusBackendToggles` 不再点名任何功能；模型分组的轮询不再自启。随后：`registerRowAction` 加 `placement: "inline"`（行上常驻图标按钮）、`surfaces: ["archived"]`（归档页行）、`className` / `onMount`，内置拓展的注册项可用 0~999 排序；会话删除、Markdown 导出改为注册会话行操作，内核只是按注册项画按钮组的宿主；用量状态栏、请求状态条的轮询、听写快捷键、消息导航条的启动轮询改由内置拓展装上时开始。功能侧仍读停用集合判断自己装着没有（行为与闭包内标记等价）；Rust 侧驱动的 11 项只登记（启动期补丁刻意不开放） | 第三方拓展用同样的接口能写出这些内置功能 |
| **网关环节可关**（2026-11-01） | `gateway.replaces` 再开放 7 个内置环节：本地代理两种模式都经过的 `builtin:auto-retry`（请求避震）、`builtin:foreign-reasoning`（别家加密推理处理）、`builtin:unlisted-model-mapping`（目录外模型映射），网关模式的 `builtin:key-cooldown`、`builtin:key-breaker`、`builtin:local-rate-limit`、`builtin:title-model`。关掉冷却 / 熔断后在冷却的 Key 照常当候选（排在可用的后面），由拓展在 route 段筛。仍不开放：协议转换、凭据与鉴权头、官方登录 / ChatGPT 后端整理、请求日志与用量（关掉后请求发不出或计费对不上） | 拓展能整体接管网关的调度与重试 |
| **管理器面板**（2026-11-01） | 清单 `manager: { page, title }`：管理器「本地拓展」行菜单「打开面板」，页面放进沙箱 iframe（允许脚本和表单交互，CSP 限制外部资源与表单外发；iframe 自导航仍须信任作者），经 postMessage 的 `window.codexPlusManager` 只能读写本拓展配置、调本拓展常驻后台、取本拓展 assets、弹提示、跟随主题。只声明 `manager` 的包也合法；对话造拓展的写入工具认 `manager` | 拓展能在管理器里有自己的界面，且碰不到管理器的其它能力 |
| **补齐剩余差距** ✅（2026-11-01） | ① Rust 侧生效的 10 个内置拓展（原生 Fast、原生输入预测、模型白名单解锁、强制中文、背景图、第三方推理折叠、Work 视图、原生浏览器兼容、历史轮次补全、桌宠跟随鼠标）都能被 `replaces` 顶替：`builtin_takeover` 把被顶替的设置项按「关」处理，启动、注入、桥接、本地代理、启动器驱动读到的都是处理后的设置；只有独立模式的官方额度解锁不能顶替（那时拓展不加载）。顶替用的新接口：清单 `preload`（新文档脚本，Codex 自己的脚本之前执行，可要求启动后重载一次）、`launch`（`args` 追加启动参数，`before` / `after` 在启动前 / 退出后跑 Node 脚本）、公开事件 `codexpp/session-index-repaired`、只读路由 `/system/cursor-position`。② `gateway.replaces` 再开放 4 个环节：`builtin:request-conversion`（请求方向的协议转换）、`builtin:auth-headers`（鉴权头；Key 只交给声明它的拓展的 upstream 中间件）、`builtin:official-shaping`（官方登录 / ChatGPT 后端整理）、`builtin:request-log`（请求日志与用量）。仍由 Codex++ 做的只剩响应方向的协议转换与官方 OAuth 凭据。③ 管理器的页面也可由拓展贡献：`manager.nav` 进侧栏与 ⌘K、`manager.replaces` 顶替 23 个内置页面（「万物皆拓展」「内置拓展」「设置」除外，被顶替页上方可切回）、`manager.data` 声明可读的只读数据（请求流、请求日志、用量、本机会话），面板多了 `data` / `navigate` | 第三方拓展能写出所有内置功能的等价物 |
| **对齐 dsh** ✅（2026-11-01） | 拓展内核：可选依赖（`inject` 对象写法 `required: false`）、实例级 `ready` / `dispose`、异步 effect、清单 `instances` 多实例；网关：`stage: upstream` 排在内置环节之后、`order`、`replaces` 关掉内置环节、供应商 `override`；内置拓展：扫描步骤与全局监听归拓展所有，`codexpp.usage` / `codexpp.proxyStatus` 数据服务。dsh 插件侧的覆盖见 `2026-10-30-dsh-plugin-import-design.md` 第 9~11 节 | — |

## 六、对话造拓展

用户在 Codex 对话里描述需求，agent 直接写出拓展包并热重载，读回加载状态与报错后继续迭代。

- **为什么走 MCP**：Codex 的沙箱通常不能写工作区以外的目录，也连不上本机调试端口。MCP 服务由 Codex 拉起、
  在沙箱外运行，写入还会经过 Codex 的工具审批。
- **服务**：`codex_plus_extensions`，即 Codex++ 主程序 `--codex-plus-extension-mcp --state-dir <状态目录>`
  （`crates/codex-plus-core/src/extension_creator.rs`）。工具：guide / list / read / write / set_enabled / reload，
  没有删除。
- **接入**：与协作 MCP 共用 `crates/codex-plus-core/src/managed_mcp.rs`：先写 Codex++ 托管上下文（切换供应商不丢），
  再原子写 Codex live 配置，冲突不覆盖、失败只回滚本条。管理器「拓展」页的「对话造拓展」一栏触发。
- **热重载安全**：重载前读一次页面运行状态，有不支持页内重载的旧脚本就不自动执行（否则会整页刷新、打断对话），
  交给用户在管理器手动点。重载后等 0.8 秒再读一次状态，捕捉拓展异步阶段报的错。
- **写入约束**：只写 user 目录、包内相对路径、拒绝软链逃逸、同名目录必须是同 id 的拓展包、单文件 256 KiB / 总量 1 MiB；
  清单由工具生成，`engines.codexpp` 固定为当前版本。

## 七、风险

- **Codex 升级**：DOM 与协议随版本变。拓展只依赖我们承诺的 slot / 事件契约，由 Codex++ 适配层吸收变化，
  契约测试守住。
- **安全**：拓展与 Codex 页面同权，P3 起还会跑子进程。不做沙箱（参照对象同样不做），靠权限声明、
  管理页展示与市场审核。
- **扫描性能**：拓展插入的节点必须带 `data-codex-plus-ext`，内核的 `register*` 已经自动处理（issue #1960）。
