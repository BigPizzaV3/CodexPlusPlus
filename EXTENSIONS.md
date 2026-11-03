# Codex++ 拓展开发指南

Codex++ 把注入到 Codex 页面里的 UI 能力开放给用户脚本（拓展）调用。入口只有一个全局对象：

```js
window.codexPlus
```

本文档描述**当前已经可用**的接口。接口版本：`apiVersion = 1`（适配层 `1.2.0`）。

- Claude Desktop 的 Code 本地会话使用独立的 `claudeDesktop` 声明接入按钮、后台工具和会话事件，见 [Claude 适配指南](docs/claude-desktop-extensions.md)。它不注入 `window.codexPlus`，不改变本文的 Codex 接口。
- 想了解设计取舍与已知限制的来龙去脉，见 [docs/specs/2026-09-30-ui-extension-api-design.md](docs/specs/2026-09-30-ui-extension-api-design.md)
- 类型声明：起 `types/codex-plus-extensions.d.ts`，在脚本开头加 `/// <reference types="./codex-plus-extensions" />` 即可获得编辑器补全

---

## 目录

- [快速上手](#快速上手)
- [脚本放在哪、怎么生效](#脚本放在哪怎么生效)
- [生命周期契约（最重要的一节）](#生命周期契约最重要的一节)
- [拓展实例与拓展包：codexPlus.extension](#拓展实例与拓展包codexplusextension)
- [在 Codex 对话里造拓展](#在-codex-对话里造拓展)
- [发布到社区拓展](#发布到社区拓展)
- [精选与官方拓展](#精选与官方拓展)
- [API 参考](#api-参考)
- [后端调用与路由白名单](#后端调用与路由白名单)
- [类名与属性契约](#类名与属性契约)
- [样式](#样式)
- [限制](#限制)
- [排查](#排查)
- [尚未开放的接口](#尚未开放的接口)

---

## 快速上手

```js
// 1. 会话行的「更多操作」菜单里加一项
window.codexPlus.registerRowAction({
  label: "导出 Markdown",
  icon: "⇩",
  onActivate: ({ session_id }) => {
    window.codexPlus.call("/export-markdown", { session_id })
      .then(() => window.codexPlus.toast("导出完成", { type: "success" }))
      .catch((error) => window.codexPlus.toast(`导出失败：${error.message}`, { type: "error" }));
  },
});

// 2. 图标栏加一个入口，点开是一个整页视图
window.codexPlus.registerPage(
  {
    title: "我的面板",
    render: ({ container }) => {
      // 不要缓存 container：每次打开页面都会重新调用 render
      container.innerHTML = `<div class="codex-plus-row">你好</div>`;
    },
  },
  { navLabel: "我的面板", icon: "P" },
);

// 3. Codex++ 菜单的「主页」面板里加一个开关
let enabled = false;
window.codexPlus.registerMenuItem({
  label: "我的开关",
  description: "开启后做某件事",
  toggleValue: () => enabled,
  onChange: (next) => {
    enabled = next;
    window.codexPlus.toast(`已${next ? "开启" : "关闭"}`, { type: "info" });
  },
});
```

---

## 脚本放在哪、怎么生效

| 平台 | 目录 |
| --- | --- |
| macOS / Linux | `~/.config/Codex++/user_scripts/` |
| Windows | `%APPDATA%\Codex++\user_scripts\` |

- 加载 `.js` 文件和带 `codexpp.json` 的子目录（拓展包，见下文），按名称小写排序执行
- 每个脚本 / 拓展包在独立 IIFE 里运行，互不污染
- 改完脚本后在管理页点「热重载拓展」即可生效，无需重启 Codex

脚本标识（`codexPlus.script.key`）形如 `user:my-script.js`，由来源目录与文件名拼成；拓展包是 `user:<目录名>`。

---

## 生命周期契约（最重要的一节）

**注册表持久，DOM 瞬态。**

Codex++ 的 UI 宿主会被反复重建：

| 宿主 | 何时重建 |
| --- | --- |
| 整页 overlay | 每次打开都先清空再重建 |
| 会话行按钮组 | 内部版本号变化时整组重建 |
| Codex++ 菜单 | 每次打开都重新构建 |
| 浮层面板 | 设置开关变化时 runtime 重启 |

所以要遵守三条：

1. **不要缓存 DOM 引用**。`render({ container })` 每次打开都重新调用，你应该每次都从零构建 `container` 的内容。
2. **不要假设你 `appendChild` 的节点还在**。宿主重建后它会被清掉，但**注册表里的数据仍在**，下一次渲染会自动重新带上它。你不需要为此做任何事。
3. **需要持有定时器 / 全局监听时**，用 `codexPlus.onCleanup` 注册清理，或在下次渲染时先把自己上一次的定时器清掉。

```js
let timer = null;
window.codexPlus.registerPage({
  title: "轮询面板",
  render: ({ container }) => {
    container.innerHTML = `<div data-output></div>`;
    const output = container.querySelector("[data-output]");
    clearInterval(timer);                        // 先停掉上一次的
    timer = setInterval(() => {
      output.textContent = new Date().toLocaleTimeString();
    }, 1000);
  },
});
window.codexPlus.onCleanup(() => clearInterval(timer));
```

**每个 `register*` 都返回一个 dispose 函数**，调用即注销该项并从所有宿主移除。注册的寿命不一定等于脚本的寿命——不需要时可以提前注销。

---

## 拓展实例与拓展包：codexPlus.extension

新写的拓展推荐用 `codexPlus.extension`。它创建一个拓展实例 `ctx`，**ctx 上的每次注册都随实例卸载按逆序自动撤销**，
不需要自己维护 dispose 列表，热重载时也不会残留旧实例。
早期版本叫 `codexPlus.plugin`，仍然可用，等同于 `codexPlus.extension`。

```js
codexPlus.extension((ctx) => {
  ctx.registerPage({ title: "时钟", render: ({ container }) => { container.textContent = new Date().toLocaleTimeString(); } });
  ctx.setInterval(() => ctx.emit("clock/tick", Date.now()), 1000); // 卸载时自动清理
  ctx.listen(window, "focus", () => ctx.toast("欢迎回来"));        // 卸载时自动 removeEventListener
  ctx.effect(() => {                                                // 任意副作用：返回撤销函数
    const observer = new MutationObserver(() => {});
    observer.observe(document.body, { childList: true });
    return () => observer.disconnect();
  });
});
```

要点：

- **在脚本同步初始化期间调用**。此时内核会替你登记热重载清理；`await` 之后创建的实例只能自己 `ctx.dispose()`。
- `apply` 同步抛错时，已做的注册先回滚再把错误抛出（脚本标为加载失败）；异步 `apply` reject 时同样回滚并上报。
- 回调、事件处理器、中间件抛错都记进本脚本的失败通道，不会影响其他拓展。

| ctx 成员 | 说明 |
| --- | --- |
| `effect(fn)` | 立即执行 `fn`，它返回的函数在卸载时调用；返回值可提前撤销。`fn` 是 async 时等它给出撤销函数，实例已卸载就拿到时立刻撤销 |
| `registerRowAction / registerNavEntry / registerMenuItem / registerPage` | 同 `codexPlus.register*`，归属与撤销自动处理 |
| `on(event, handler)` / `emit(event, ...args)` | 拓展间事件。内核广播 `extension/apply`（`apply` 含异步部分完成后）、`extension/dispose`（参数 `{ id, scriptKey }`）。`on("ready")` / `on("dispose")` 只属于本实例：`ready` 在 `apply` 完成后调用（已完成时立即调用），`dispose` 在卸载时调用 |
| `intercept(name, handler, { order })` / `waterfall(name, value)` | 中间件链：`handler(value, next)` 调 `next(v)` 交给下一个，不调即短路；出错的中间件被跳过；没有中间件时原样返回 |
| `listen(target, type, handler, options)` | 自动撤销的 `addEventListener` |
| `setTimeout / setInterval` | 自动清理的定时器 |
| `onScan(handler, { phase })` | 跟着 Codex++ 的页面扫描跑：Codex 界面变了（会话切换、侧栏 / 输入框重建……）时调用，按帧合并、页面隐藏时不跑。`phase` 为 `deferred`（默认，完整扫描）或 `light`（每次变化都跑，handler 要便宜）。适合「Codex 重建了某块界面，把自己的东西补回去」，不必自己开 MutationObserver。插进页面的节点必须带 `data-codex-plus-ext` 属性，否则会触发自喂扫描循环。返回的撤销函数带 `run()`：立即跑一次 |
| `call(route, payload, options)` | 拓展包只能调清单里声明的路由；单文件脚本沿用全局白名单 |
| `toast / fail / dispose` | 提示、上报失败、主动卸载 |
| `registerComposerAction(def)` | 输入区（发送按钮那一行）的图标按钮，`onActivate({ editor, insertText, close })` |
| `registerCommand({ name, description, run })` | 斜杠命令：输入框里 `/name 参数` 回车时调用 `run(args, { editor, insertText, setText, toast })`，输入框随后清空；输入 `/` 时在输入框上方提示匹配的拓展命令，Tab 补全。只拦截已注册的拓展命令，同名时先注册的生效 |
| `registerMessageAction({ label, icon, roles, visible, onActivate })` | 每条对话消息操作行（复制、编辑旁）的图标按钮，`onActivate({ message, close })`；`message` 见下 |
| `registerMessageDecoration({ roles, visible, render })` | 消息正文下方的一块内容：`render({ message, container })`，消息文字变化（流式输出）时清空重画，留空不占位 |
| `registerSidePanel({ title, icon, width, render })` | 右侧面板：顶栏右侧出现开关，打开时主区域让出宽度；`render({ container, close })` 每次打开调用，可返回清理函数；返回值带 `open() / close() / toggle()` |
| `asset(path)` | 包内 `assets/` 资源的 data URL（图片、音频、字体），找不到返回空串 |
| `provide(name, value) / get(name) / inject(deps, apply)` | 拓展间服务，见「拓展间服务」 |
| `locale.t / register / bind / resolve / current` | 多语言，见「多语言」 |
| `storage.get / set / remove / keys` | 按拓展 id 隔离的本地存储，值按 JSON 存取 |
| `config` | 生效配置的只读快照（清单 `config` 声明，见「拓展配置」） |
| `id / scriptKey / manifest / disposed` | 只读信息 |

### 界面插槽

```js
codexPlus.extension((ctx) => {
  ctx.registerSlot({ slot: "composer.above", render: ({ container }) => {
    container.textContent = "提示：今天的额度还剩 80%";
  } });
  const clock = ctx.registerSlot({ slot: "titlebar", render: ({ container }) => {
    const button = document.createElement("button");
    button.textContent = new Date().toLocaleTimeString();
    container.appendChild(button);
    return () => button.remove(); // 撤下或重画前调用
  } });
  ctx.setInterval(() => clock.refresh(), 60_000);
});
```

- 位置：`composer.above`（输入框上方）、`thread.top` / `thread.bottom`（对话区顶部 / 底部）、`sidebar.top` / `sidebar.bottom`
  （左侧栏列表顶部 / 底部）、`titlebar`（标题栏右侧按钮组）、`turn.after`（每一轮对话末尾，render 多收到 `turnKey`）、
  `sessionRow.prefix` / `sessionRow.suffix`（侧栏每个会话标题前 / 后，多收到 `sessionId`、`title`）、
  `overlay`（铺满窗口的浮层，默认不挡点击，放进去的元素可以点）、`activity.after`（对话里每个活动条目——命令、
  工具 / 子 agent 调用、出错——的末尾，多收到 `kind`（`command` / `toolCall` / `fileChange` / `reasoning` / `error` /
  `other`）、`itemIds`、`text`；条目收在 Codex 的「用时 Ns」里，用户展开后才出现；推理只在 Codex 的显示模式会显示推理时才有条目）、
  `composer.left` / `composer.right`（输入框底栏左侧按钮组末尾 / 右侧按钮组开头，和「添加内容」「发送」同一行）、
  `composer.below`（输入框下方）、`home.above`（新对话首页输入框上方）、`header.title`（顶栏会话标题后面）、
  `approval`（审批卡片底部，多收到 `kind`）、`sidebar.section`（侧栏「项目」「最近」等分组标题右侧，多收到 `section`）、
  `projectRow.suffix`（侧栏项目名后面，多收到 `projectId`、`label`）、`composer.model`（输入框底栏的模型 / 推理强度按钮后面）、
  `settings.nav`（Codex 设置页左侧导航末尾，多收到 `sampleClass`：原生导航项的类名，可借来对齐样式）。
  `activity.after` 的 `kind` 还可能是 `plan`（计划条目）。
  未知位置会报错。
- `render({ container, slot, ... })` 在容器新建时调用；Codex 重建 DOM（切换会话、侧栏刷新）后会重新调用，不要缓存节点。
  返回的函数在该项被撤下或重画前调用。`container` 留空则不占位。返回的句柄带 `refresh()` 强制重画。
- 同一位置多个拓展按 `order` 排列（越小越靠前）。拓展卸载、热重载时自动撤下。

### 消息插槽与侧边面板

```js
codexPlus.extension((ctx) => {
  // 助手消息上加「统计字数」按钮，并在正文下方显示字数
  ctx.registerMessageAction({ label: "统计字数", icon: "#", roles: ["assistant"],
    onActivate: ({ message }) => ctx.toast(`${message.text.length} 字`) });
  ctx.registerMessageDecoration({ roles: ["assistant"], render: ({ message, container }) => {
    if (message.text.length > 2000) container.textContent = `长回复：${message.text.length} 字`;
  } });
  // 右侧面板
  const panel = ctx.registerSidePanel({ title: "便签", icon: "✎", render: ({ container }) => {
    container.textContent = ctx.storage.get("note", "（空）");
  } });
  ctx.registerCommand({ name: "note", description: "打开便签", run: () => panel.open() });
});
```

- `message` 是 `{ role, id, turnId, conversationId, text, element }`：`role` 为 `user` / `assistant`；`text` 是正文纯文本，
  流式输出时是已输出的部分；`element` 只读用。消息节点会随滚动被 Codex 重建，Codex++ 负责每次补上按钮和装饰，
  拓展不要缓存这些节点。
- 只有存在消息插槽注册时，Codex++ 才观察对话区的变化（去抖 300ms）；没有注册时零开销。
- 侧边面板同一时间只开一个，挂在 Codex 的 React 树外面；拓展卸载、热重载时自动关闭。

### Codex 会话事件与请求中间件

拓展可以订阅 Codex app-server 推给页面的通知，也可以改写页面发往 app-server 的请求：

```js
codexPlus.extension((ctx) => {
  // 轮次结束时提示一下
  ctx.on("codex/turn/completed", (params) => ctx.toast(`本轮结束：${params?.turn?.status ?? ""}`));
  // 每次发起轮次时，在用户输入前加一段固定说明
  ctx.intercept("codex/request", (value, next) => {
    if (value.method !== "turn/start" || !Array.isArray(value.params?.input)) return next(value);
    const input = [{ type: "text", text: "（请用中文回答）" }, ...value.params.input];
    return next({ ...value, params: { ...value.params, input } });
  });
});
```

- 已转发的事件：`codex/thread/started`、`codex/turn/started`、`codex/turn/completed`、`codex/item/started`、
  `codex/item/completed`、`codex/item/agentMessage/delta`、`codex/error`，以及用量与等待用户的 `codex/thread/tokenUsage/updated`、
  `codex/item/tool/requestUserInput`、`codex/item/commandExecution/requestApproval`、`codex/item/fileChange/requestApproval`、
  `codex/item/permissions/requestApproval`、`codex/mcpServer/elicitation/request`、`codex/serverRequest/resolved`、
  `codex/item/tool/userInputAnswered`、`codex/item/approval/responded`；处理器参数是通知的原样 `params` 和 `{ method }`。
  **新版 Codex 的通知不再经过页面，这些事件收不到**；要跟踪会话进展请用下面「会话历史」里的 `session/event` / `agent/status`。
- `codex/request` 的值是 `{ method, params }`，只能改 `params`，改 `method` 无效。没有任何中间件时请求原样发出。
  新版 Codex 的请求经页面与宿主进程之间的 RPC 发出，Codex++ 在这一层拦截，中间件可以返回 Promise，
  超过 5 秒或出错时原样发出；旧版 Codex 的消息分发路径是同步的，那里返回 Promise 视为出错并原样放行。
- 一条用户消息会同时备好 `turn/start` 和 `turn/steer` 两份候选（会话空闲时发前者，正在回复时发后者），
  中间件会各收到一次，要改写用户输入就两种都处理。新会话的首条消息按 `turn/start` 交给中间件，
  它的 `params` 还带着建会话用的字段（`cwd`、`workspaceRoots` 等），没有 `threadId`。
- 要在请求**前后**都做事（拿到 app-server 的返回值）用 `codex/exchange`：值是 `{ method, params, hostId, send }`，
  中间件换掉 `send` 包住原请求，链尾调用最终的 `send()`。内置拓展「侧聊保存」就挂在这里（临时旁支分叉时记下主历史）：

  ```js
  ctx.intercept("codex/exchange", (exchange, next) => next({
    ...exchange,
    send: async () => {
      const result = await exchange.send();
      if (exchange.method === "thread/fork") ctx.toast(`已分叉：${result?.thread?.id ?? ""}`);
      return result;
    },
  }));
  ```

  它挂在 app-server 客户端的 sendRequest 补丁上，覆盖范围与该补丁一致；`send` 抛错会原样抛给 Codex，包装时自己兜住。
- 这几项都直接面向 Codex 的内部协议，字段随 Codex 版本可能变化，拓展要容错。

### 步骤准入与模型请求

`agent/pre-step` 和 `agent/request` 用 `ctx.on` 注册，处理器是 `(params, next)`：
先 `await next()` 拿到下游的决定，再返回改过的版本。

```js
codexPlus.extension((ctx) => {
  // 每条用户消息后面补一条上下文
  ctx.on("agent/pre-step", async ({ agent, messages, signal }, next) => {
    const decision = await next();
    if (decision.kind === "reject" || signal.aborted) return decision;
    const note = { role: "user", content: [{ type: "text", text: `[now: ${new Date().toISOString()}]` }] };
    return { ...decision, messages: [...decision.messages, note] };
  });
  // 换模型或推理强度
  ctx.on("agent/request", async (params, next) => {
    const config = await next();
    return { ...config, reasoningEffort: "high" };
  });
});
```

- `agent/pre-step`：`params` 是 `{ agent, turn, step, messages, signal, conversationId }`，`messages` 是本轮要进入的用户消息
  （`{ role: "user", id, source: { kind: "user" }, content: [{ type: "text", text }, …] }`，图片、提及等非文本条目原样在 `content` 里）。
  返回 `{ kind: "enter", messages }` 改写本轮输入（可以增删消息、改文字），或 `{ kind: "reject", reason? }` 不发这一轮，
  Codex 会显示发送失败，Codex++ 用 `reason` 弹提示。
- `agent/request`：`params` 是 `{ agent, turn, step, signal, conversationId }`，`next()` 返回
  `{ provider, model, reasoningEffort }`。`model` 是 Codex 里的完整模型 id（如 `jojocodemax/gpt-6.1-sol`），`provider` 是斜杠前的部分；
  改 `model` 或 `reasoningEffort` 会写回本轮的协作模式设置，只改 `provider` 时按「供应商/模型」拼回去。
- 限制：Codex 只在用户发消息时经过这里，一轮里的后续步骤（工具调用之后）不会再触发，`step` 恒为 1，`turn` 拿不到（`undefined`）；
  `agent` 只是 `{ id, session: { id } }` 描述，没有 `steer` / `inject` 等方法；`maxTokens`、`startsRequestSeries` 不生效。
  处理器抛错时跳过它、按 `next()` 的结果继续，不会挡住 Codex 发消息；全链超过 5 秒按原样发出，`signal` 随之中止。
- 同一条用户消息只跑一次这两条链，结果套用到它的 `turn/start` 与 `turn/steer` 两份候选上，之后才轮到 `codex/request`。

### 拓展配置

拓展包可以在清单里声明配置项，Codex++ 管理器（拓展列表里的「设置」）和 Codex「拓展」页面详情的「设置」页签都会自动生成设置表单：

```json
{
  "id": "pomodoro",
  "config": {
    "minutes": { "type": "number", "title": "专注时长（分钟）", "description": "每个番茄钟的长度", "default": 25 },
    "sound": { "type": "boolean", "title": "结束时提示音", "default": true },
    "theme": { "type": "select", "title": "配色", "options": [{ "value": "red", "label": "番茄红" }, "green"] },
    "token": { "type": "string", "title": "API 令牌", "secret": true }
  }
}
```

- 类型：`string`、`text`（多行）、`number`（可加 `min` / `max`）、`boolean`、`select`（`options` 为值或 `{ value, label }`）、
  `array`（`items` 为元素类型，只能是简单类型，最多 200 项）、`object`（`properties` 为子字段，最多嵌套 3 层）；每层最多
  32 项。没给 `default` 时取空串 / 0（有 `min` 取 `min`）/ false / 第一个选项 / 空数组；对象的默认值由子字段默认值组成，
  清单里的 `default` 只覆盖其中的键。`secret` 只影响界面（密码框），值仍以明文存在本机 `user_scripts.json` 的 `configs` 里。
- `title` / `description` 可以是字符串，也可以是 `{ "zh": "…", "en": "…" }`，管理器按界面语言显示。
- 本机文件路径：`string` 加 `"format": "file"`，管理器表单在输入框旁给「选择文件」按钮（系统对话框）；`"extensions": ["png", "jpg"]`
  限定可选的扩展名（不带点，最多 20 个，只能和 `format: "file"` 一起用）。值仍是普通字符串，页面拿不到本机文件，要读内容
  得经 Codex++ 的路由或常驻后台（背景图拓展就是这样，见 `assets/extensions/image-overlay/`）。
- 嵌套示例：`"proxy": { "type": "object", "properties": { "host": { "type": "string" }, "port": { "type": "number", "min": 1, "max": 65535 } } }`，
  `"words": { "type": "array", "items": { "type": "string" } }`。保存时对象可以只给部分键（其余取默认），未声明的键会被拒绝。
- 读取：页面代码用 `ctx.config`；钩子、网关中间件、工具脚本用第二个参数的 `context.config`。都是生效配置
  （默认值叠加已保存且类型正确的值）的只读快照。
- 修改：管理器表单保存后自动热重载；对话造拓展可用 `codexpp_extension_configure` 改值。只接受声明过的键，类型不符即报错。
- 自己画设置界面：表单不够用时（图库、试听、预览……），清单写 `"settings": { "page": "settings.html" }`，管理器里「设置」
  打开这个页面（和[管理器面板](#管理器面板)同一套沙箱与 `codexPlusManager` 接口，拓展停用时也能打开）；Codex 里
  「拓展」页面选中这个拓展时，详情的「设置」页签显示 `ctx.registerSettingsPanel({ render({ container, config, saveConfig }) })` 画的面板
  （拓展停用、面板还没登记时退回按清单生成的通用表单），
  `saveConfig(values)` 只存本拓展声明过的项、回生效配置，可返回清理函数。两边想用同一份界面，就把界面写成一个函数，
  页面脚本里调一次、设置页里内联一份（官方的 Agent 桌宠拓展就是这样，见 `assets/extensions/agent-pet/settings.js`）。
- 不重载地接住新配置：`ctx.onConfigUpdate((next, prev) => { 应用 next; return true; })`。管理器保存后先问页面，这个拓展的
  每个实例都返回 `true` 时 `ctx.config` 原地换成新值，不再热重载；有一个实例没接（没注册或返回别的）就照常整体热重载。
- 多实例：清单写 `"instances": [{ "id": "work", "name": "工作", "config": { "label": "工作区" } }, { "id": "home" }]`，
  页面脚本按实例各应用一次，`ctx.config` 是生效配置叠上该实例的 `config`，`ctx.instance` 为 `{ id, name }`，
  `ctx.id` 变成 `<拓展 id>#<实例 id>`，注册项的 `id` 与 `ctx.storage` 也按实例分开。实例最多 16 个，`config` 只能写清单 `config`
  里声明过的键。钩子、网关、后台不分实例。

### 给模型加工具

拓展包可以声明给 Codex 模型调用的工具，由已接入的 Codex++ 拓展 MCP（`codex_plus_extensions`）对外提供：

```json
{
  "id": "text-tools",
  "name": "文本工具",
  "tools": [{
    "name": "word_count",
    "description": "统计一段文字的字数",
    "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] },
    "script": "tools/word-count.mjs",
    "timeout": 30
  }]
}
```

```js
// tools/word-count.mjs：默认导出 (args, { extension, tool }) => 结果
export default ({ text }) => `共 ${[...text].length} 个字符`;
```

- 对外工具名为 `<拓展 id>__<工具名>`（合计不超过 64 个字符）；`inputSchema` 必须是 `type: "object"` 的 JSON Schema，省略时为空对象。
- 返回字符串作为文本；返回 `{ content: [...] }` 按 MCP 内容块原样返回；其它值转为 JSON 文本；抛错时以错误返回给模型。
  `timeout` 1~300 秒，默认 30。每次调用起一个 Node 进程。
- 前提是在「拓展」页的「对话造拓展」里接入过 Codex。拓展写入、启停、重载后服务会通知 Codex 刷新工具列表；
  已经开着的对话可能仍用旧列表，新开对话一定生效。

### Codex 钩子

拓展包可以在清单里声明 Codex 钩子，在工具执行前后、提交提示词时、本轮结束前插手 Codex 的 agent 循环：

```json
{
  "id": "rm-guard",
  "name": "危险命令拦截",
  "hooks": {
    "PreToolUse": [{ "matcher": "^Bash$", "script": "hooks/guard.mjs", "timeout": 30 }]
  }
}
```

```js
// hooks/guard.mjs：默认导出 (payload, { event, extension }) => Codex 钩子输出 | null
export default (payload) => /rm\s+-rf\s+\//.test(payload.tool_input?.command ?? "")
  ? { hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "deny", permissionDecisionReason: "禁止 rm -rf /" } }
  : null;
```

- 可用事件：`SessionStart`、`UserPromptSubmit`、`PreToolUse`、`PermissionRequest`、`PostToolUse`、`PreCompact`、
  `PostCompact`、`SubagentStart`、`SubagentStop`、`Stop`、`PreModelRequest`、`PostModelRequest`
  （`SessionEnd` / `Interrupt` 超时只有 1~3 秒，不开放）。
- `PreModelRequest` / `PostModelRequest`：agent 每次请求模型前 / 后各触发一次，**官方直连也触发**，适合按步观察
  （计步、计时、记用量、按步提醒）。payload 有 `session_id`、`turn_id`、`model`、`transcript_path`、`cwd`；
  `PostModelRequest` 另有 `status`、`error`、`token_usage`。不带消息内容，要看内容用 `context.sessionQuery`
  按 `session_id` 读。**只能观察**：Codex 不采用它们的输出，拦不下这一步，也加不了上下文；要改请求请用网关中间件
  （需经 Codex++ 代理）。`matcher` 按模型名匹配。它们挡在每一步前面，脚本要快。
- `payload` 与输出都按 Codex 钩子协议；多个拓展的输出由 Codex++ 合并：任一 `deny` 即拒绝、`allow` 只采用第一个
  `updatedInput`、`decision: "block"` 与 `continue: false` 任一即生效、`additionalContext` 与 `systemMessage` 按拓展顺序拼接。
- 运行方式：Codex++ 只往 `~/.codex/config.toml` 写**固定的分发条目**（每个用到的事件一条，命令
  `CodexPlusPlus --codex-plus-hook <事件>`，超时 120 秒，状态文字「Codex++ 拓展钩子」）。Codex 要求钩子经用户
  信任后才运行，所以第一次用到某个事件时，需要在 Codex 的 Hooks 设置里信任这条条目一次；之后拓展增删、启停都不用
  重新信任，由 Codex++ 的拓展开关决定哪些钩子在跑。拓展启停、删除、热重载以及对话造拓展写入后都会自动同步条目，
  没有拓展再用到的事件会移除。
- 每次触发只起一个 Node 进程（优先用 Codex App 自带的 Node，或 `CODEX_PLUS_NODE` 指定），按 `matcher` 过滤后并发
  运行命中的脚本；单个脚本出错或超时只记诊断日志，不影响其它拓展，也不挡 Codex。
- 钩子脚本以你的用户权限运行、没有沙箱。信任分发条目即信任了 Codex++ 拓展开关里启用的全部钩子。

### 网关请求中间件

拓展包可以在 Codex 的模型请求发往上游前改写它：

```json
{ "id": "zh-reply", "name": "中文回复", "gateway": { "request": [{ "script": "gateway/request.mjs", "timeout": 5 }] } }
```

```js
// gateway/request.mjs：默认导出 (body, { path, extension }) => 新 body | null
export default (body) => ({ ...body, instructions: `${body.instructions ?? ""}\n\n请始终用简体中文回答。` });
```

- `body` 是 OpenAI Responses 请求体的副本；返回新对象即替换，返回 `null` 不改。多个拓展按拓展 key 顺序串联，
  前一个的输出是后一个的输入。
- 按步触发：agent 每走一步（每次工具调用之后）都会向模型发一次请求，都经过这里。第二个参数里的 `step` 是
  `{ index, toolResults, userText, firstStep }`（本轮第几步、已有几个工具结果、本轮用户输入、是否第一步）。
  返回 `{ reject: "原因" }` 即不发这一步，Codex 收到带原因的错误，后面的中间件不再跑（与页面侧 `agent/pre-step` 的 reject 相同）。`timeout` 1~10 秒（默认 5），整条链最多 15 秒。
- 加请求头：往第二个参数的 `headers` 对象里写（`context.headers["x-trace"] = "1"`），发往上游时附上。
  鉴权、Cookie、Host、内容长度 / 类型这类头由 Codex++ 管，写了也会被忽略。失败重试时沿用同一组头。
- 只对经 Codex++ 本地代理 / 网关发出的请求生效（中转、聚合、网关模式）；官方直连不经过 Codex++，不受影响。
- 只有存在启用中的网关中间件时才会为每个请求起一个 Node 进程；任何中间件出错或超时都会保留原请求并记诊断日志。
- 中间件能看到完整的请求内容（含对话），以你的用户权限运行。只启用你信任的拓展。

#### 段、顺序与关掉内置环节

Codex++ 自己在发请求前也会做几件事（本地压缩、第三方推理折叠、exec 工具适配、按供应商路由与协议转换）。
拓展的中间件可以排在这些内置环节之前或之后，也可以关掉其中可替换的环节自己实现：

```json
{
  "gateway": {
    "request": [
      { "script": "gateway/early.mjs", "order": -10 },
      { "script": "gateway/upstream.mjs", "stage": "upstream" }
    ],
    "replaces": ["builtin:reasoning-collapse"]
  }
}
```

- `stage`：`request`（默认）在内置环节之前改原始 Responses 请求；`upstream` 在内置环节之后改真正发给上游的请求体，
  第二个参数多给 `stage: "upstream"`、`wireApi`（`responses` / `chat` / `anthropic`…）、`relay: { id, name }`、`model`、
  `attempt`（换候选重发时每次都跑，请求体按候选不同）。`upstream` 段同样可以 `{ reject }`、加请求头。
- `stage: "route"`：中转轮换按规则选出候选供应商之后调用，收到 `{ model, conversationId, candidates: [{ id, name, protocol }] }`，
  返回重排 / 删减后的同形对象或 `{ order: ["供应商 id", …] }`，按这个顺序尝试（认不得的 id 忽略，一个都不认得时照原顺序）。
  网关模式的候选（供应商 + Key）同样经过这里，`candidates` 每项是 `{ id: "<供应商>#<序号>", provider, name, model }`，
  上下文多给 `agent`。只有一个候选时不调用；聚合模型、显式模型路由不经过这里。
- `order`：同一段里越小越先，相同时按拓展 key 顺序。
- `replaces`：关掉的内置环节，可选 `builtin:local-compaction`（本地压缩）、`builtin:reasoning-collapse`（第三方推理折叠）、
  `builtin:exec-adapt`（exec 工具适配）、`builtin:relay-rotation`（轮换失败后自动换下一个供应商；关掉后只用选中的那一个，
  配合 route 段自己决定顺序）、`builtin:gateway-failover`（网关模式里一个候选失败后换下一个；关掉后只试排第一的那个）。
  本地代理（中转与网关两种模式都经过）里还能关：
  - `builtin:auto-retry`：「请求避震」。关掉后上游错误原样交回，不再退避重发；要不要重试交给 `gateway.error`。
  - `builtin:foreign-reasoning`：别家签发的加密推理内容的剥离与拒收后重发。关掉后加密项原样发出，被拒也不重发。
  - `builtin:unlisted-model-mapping`：目录外模型名映射到默认模型。关掉后模型名原样转发，映射在 request 段自己做。

  网关模式里还能关：
  - `builtin:key-cooldown`：429 / 额度耗尽后让 Key 冷却。关掉后不再冷却，已在冷却的 Key 也照常当候选（排在可用的后面）。
  - `builtin:key-breaker`：供应商连续 5xx / 连接失败后熔断。关掉后不再计入熔断，已熔断的供应商也照常当候选。
  - `builtin:local-rate-limit`：Key 的本地 RPM / 并发限速。关掉后不再按名额排队或跳过。
  - `builtin:title-model`：起名请求改发到绑定里设的起名模型。关掉后起名请求照原模型路由。

  关掉冷却 / 熔断 / 限速后，换不换下一个候选仍按失败分类定（配合 `builtin:gateway-failover` 一起关就只试第一个）；
  要避开哪些 Key、供应商由拓展在 route 段自己筛。拓展启用时生效，停用后恢复。

  两种模式都还能关：
  - `builtin:request-conversion`：把 Responses 请求转成 Chat Completions / Anthropic Messages。关掉后 upstream 段拿到的请求体
    仍是 Responses 形状（上下文 `requestConversion: "extension"`，`wireApi` / `relay.protocol` 是上游协议，`endpoint` 是按上游协议选好的地址），
    由拓展转成上游要的格式。响应方向仍由 Codex++ 按上游协议转回 Responses，响应流中间件拿到的照旧是 Responses 事件。
  - `builtin:auth-headers`：按供应商注入 `Authorization` / `x-api-key` / `anthropic-version`。关掉后 Codex++ 只带供应商的自定义头，
    upstream 段里**只有声明了这一项的拓展**拿到 `context.credential = { apiKey, protocol }`（这个候选的 Key，网关模式里是轮换选中的那个），
    它往 `context.headers` 里写的鉴权头也才被采用；别的拓展看不到 Key，写的鉴权头照旧被丢掉。官方登录成员的 OAuth 凭据、
    转给 Codex++ 本机网关那一跳的令牌仍由 Codex++ 带；网关模式的 Anthropic 原生直通改走协议代理链路（直通不经过 upstream 段）。
  - `builtin:official-shaping`：官方登录 / ChatGPT 后端的请求整理（请求体改成 Codex CLI 的样子、UA 与版本头、ChatGPT API 账号的
    请求整理、官方请求自动补 web_search 声明）。关掉后由拓展在 upstream 段整理（上下文 `official: true`）；会话头与官方凭据仍由 Codex++ 处理。
  - `builtin:request-log`：请求日志与用量统计。关掉后经代理的请求不再写请求日志、不计入用量，「请求状态条」也看不到进行中的请求；
    要记就在响应流中间件里自己记（`response.completed` 事件带 usage）。

  仍由 Codex++ 做、不开放的只有：响应方向的协议转换（响应流中间件排在它之后，拿到的是转好的 Responses 事件，要改回复在那里改），
  以及官方登录凭据本身（OAuth token 不交给拓展）。

### 网关响应流中间件

拓展包可以改写模型回复流回 Codex 的事件，写法是「包住整条流」的中间件：

```json
{ "id": "redactor", "name": "回复脱敏", "gateway": { "response": [{ "script": "gateway/response.mjs", "timeout": 5 }] } }
```

```js
// gateway/response.mjs：默认导出 (context, next) => 异步可迭代对象
export default async function* (context, next) {
  for await (const event of next()) {
    if (event.type === "response.output_text.delta") yield { ...event, delta: event.delta.replaceAll("secret", "***") };
    else yield event;
  }
}
```

- 事件是 Codex 实际收到的 Responses SSE 事件对象（带 `type`）。可以改写、丢弃、插入事件，也可以不调用
  `next()` 自己给出整条流；`next()` 只能调用一次。改了文字增量时，`response.output_item.done` /
  `response.completed` 里的完整文本要一并改。
- `context`：`{ path, extension, config, request }`，`request` 是请求摘要（`model`、`instructions`、`reasoning`、
  `tools` 工具名、`inputItems`、`lastUserText`、`sessionId` 会话 id），不含完整对话。
- 多个拓展按拓展 key 顺序嵌套，先声明的在最外层。所有响应中间件跑在一个常驻 Node 工作进程里，每个事件一次往返。
- 任一中间件出错、超时（`timeout` 1~10 秒，按单个事件计）或工作进程退出，这次回复剩下的部分原样放行。
- 只对经 Codex++ 本地代理 / 网关的流式回复生效；官方直连、远程压缩回复不经过这里。

### 网关失败重试

```json
{ "gateway": { "error": [{ "script": "gateway/error.mjs", "timeout": 5 }] } }
```

```js
// gateway/error.mjs：默认导出 (error, { path, extension, config }) => { retry: true, delayMs } | { kind: "retry" } | null
export default (error) => (error.status === 429 && error.attempt < 2 ? { retry: true, delayMs: error.json?.retryAfterMs ?? 2000 } : null);
```

- 返回 `{ kind: "fallback", model: "其它模型" }` 即换这个模型重发（降级），同样计入 3 次上限。
- 被拓展请求中间件拒绝的那一步不会进入这里。
- 上游返回错误状态时调用，`error` 为 `{ status, body, json, path, attempt }`（`body` 最多 64 KiB）。多个拓展按顺序问，
  第一个要求重试的生效；同一请求最多重发 3 次，间隔最多 10 秒。不重试时 Codex 照常收到原错误。
- 只对经 Codex++ 代理 / 网关的请求生效；没有拓展声明时不读响应体，零开销。流式回复中途断开不在此列。

### 工具管道

对应「工具执行前 / 执行 / 执行后」三段：

```json
{
  "toolPipeline": {
    "preExecute": [{ "matcher": "^Bash$", "script": "tools/pre.mjs", "timeout": 30 }],
    "execute": [{ "matcher": "^Bash$", "script": "tools/exec.mjs", "timeout": 60 }],
    "postExecute": [{ "matcher": "^exec_command$", "script": "tools/post.mjs", "timeout": 5 }]
  }
}
```

```js
// tools/pre.mjs：改参数或拦截
export default (input, { name, callId }) => input.command.includes("rm -rf") ? { deny: "不允许" } : null;
// tools/exec.mjs：返回结果即代为执行，原工具不跑；返回 undefined 交还原工具
export default (input) => (input.command === "date" ? new Date().toISOString() : undefined);
// tools/post.mjs：改写模型看到的工具结果
export default (output) => (typeof output === "string" ? output.replace(/sk-\w+/g, "sk-***") : undefined);
```

- `preExecute` / `execute` 在 Codex 的 PreToolUse 钩子里运行（`matcher` 匹配钩子里的工具名，如 `Bash`），上下文
  `{ name, callId, cwd, payload, config, sessionQuery, service }`；用到时 Codex 配置里会写入「Codex++ 拓展钩子」分发条目，
  需要在 Codex 的 Hooks 设置里信任一次。
- `execute` 返回结果后，原工具调用以「拒绝 + 结果」结束；经 Codex++ 代理时，下一次请求里网关把这条工具输出整个换成
  纯结果，模型看到的就是执行结果。不经代理时模型从拒绝理由里读到结果。
- `postExecute` 在网关请求侧改写 `function_call_output` / `custom_tool_call_output`（`matcher` 匹配模型侧工具名，如
  `exec_command`、`apply_patch`、`mcp__服务__工具`），上下文 `{ name, input, callId, config }`。按「拓展 + 脚本 + 调用 +
  输出」缓存，同一条结果只改一次。只对经 Codex++ 代理的请求生效。
- Codex 界面里显示的仍是原工具结果；改写只影响模型看到的内容。

### 会话历史（只读）

内置服务 `sessionQuery` 只读查询 Codex 的会话，页面、常驻后台、模型工具三处都能用：

```js
codexPlus.extension({ inject: ["sessionQuery"], async apply(ctx) {
  const [latest] = await ctx.sessionQuery.listSessions({ limit: 1 });
  const turns = await ctx.sessionQuery.listEvents(latest.id, { types: ["user/message"], fromEnd: true, limit: 5 });
  const page = await ctx.sessionQuery.searchSessions({ query: "bug", limit: 20 }); // page.nextCursor
} });
// 常驻后台：export default ({ sessionQuery }) => ({ ... })；模型工具：export default (args, { sessionQuery }) => ...
```

| 方法 | 返回 |
| --- | --- |
| `listSessions({ archived?, limit?, offset? })` | 会话数组，新到旧 |
| `filterSessions({ id?, cwd?, archived?, createdAfter?, createdBefore?, limit? })` | 会话数组 |
| `readSession(id)` | `{ session, events }` 完整事件日志 |
| `readTitle(id)` | 标题 |
| `listEvents(id, { types?, offset?, limit?, fromEnd? })` | 轻量事件（长文本截到 200 字） |
| `filterEvents(id, { types?, text?, limit? })` | 匹配的事件（文本不分大小写） |
| `readEvent({ sessionId, seq, before?, after? })` | 以 seq 为中心的事件窗口 |
| `searchSessions({ query, limit?, cursor? })` | `{ items, nextCursor }`，按标题、首条消息、目录匹配 |
| `searchEvents({ query, sessionId?, types?, archived?, limit?, cursor? })` | `{ items: [{ sessionId, event }], nextCursor }`；不给 sessionId 时按新到旧扫全部会话，找够一页即停 |
| `readEventsSince({ sessionId, seq, limit? })` | seq 及之后的完整事件（增量读） |
| `project(id, { init, apply })` | 把事件日志折叠成状态：`apply(state, event)` 返回新状态 |
| `observeSession(id, handler, { intervalMs?, fromSeq? })` | 订阅新事件（默认从当前末尾开始，每 2 秒增量读），返回撤销函数；页面与后台可用 |

- 会话：`{ id, title, cwd, model, createdAt, updatedAt, archived, preview, gitBranch }`。
- 事件：`{ type, seq, time, data }`。`type` 有 `turn/start`、`user/message`、`assistant/message`（`data.phase`）、
  `assistant/reasoning`（只有摘要）、`tool/call`（`name`、`callId`、`input`）、`tool/result`（`callId`、`output`）、
  `turn/complete`（`durationMs`、`lastMessage`）、`turn/aborted`（`reason`）、`token/usage`（`total`、`last`、`contextWindow`）、
  `session/compacted`。
  Codex 注入的环境上下文、开发者消息、加密推理不返回；单条文本超过 2 万字截断。
- 全程只读：数据库只读打开，rollout 文件只读。

#### 全部会话的事件流

```js
codexPlus.extension((ctx) => {
  ctx.on("session/event", (event, { sessionId }) => { if (event.type === "tool/call") console.log(sessionId, event.data.name); });
  ctx.on("agent/status", ({ sessionId, status, turn }) => { /* status: running | idle | aborted */ });
});
// 常驻后台：sessionQuery.observeAll((event, { sessionId }) => …, { onStatus })
```

- 有人监听时才开始轮询（页面可见时 1.5 秒一次，后台 2 秒），只看最近更新的 20 个会话；监听前已有的事件不重放，
  之后新建的会话从头发。监听全部撤下后自动停。页面里也可以用 `sessionQuery.observeAll(handler)`。

### 会话写入（分叉、追加、发起一轮）

内置服务 `sessions` 经 Codex 自己的 app-server 连接写会话：

```js
codexPlus.extension({ inject: ["sessions", "sessionQuery"], async apply(ctx) {
  const [latest] = await ctx.sessionQuery.listSessions({ limit: 1 });
  const copy = await ctx.sessions.fork(latest.id);                        // 分叉出一个新会话
  await ctx.sessions.append(copy.id, "（补充背景：本项目使用 pnpm）");       // 追加一条用户消息
  await ctx.sessions.append(copy.id, [{ role: "assistant", text: "收到" }]);
  const turn = await ctx.sessions.startTurn(copy.id, "请总结一下上面的对话"); // 让 agent 跑一轮
  // await ctx.sessions.interrupt(copy.id, turn.id);
} });
```

| 方法 | 说明 |
| --- | --- |
| `list({ limit?, cursor?, archived? })` | Codex 的会话列表（`thread/list` 的原样结果） |
| `create(input?, options?)` | 新建会话；给了 `input` 时紧接着发起第一轮，返回 `{ thread, turn }` |
| `fork(id, options?)` | 分叉，返回新会话（含 `id`、`forkedFromId`） |
| `rename(id, name)` | 改会话标题 |
| `append(id, items)` | 追加条目：字符串当作用户消息，`{ role, text }` 按角色转成消息，带 `type` 的对象按 Responses 条目原样写入 |
| `startTurn(id, input, options?)` | 以 `input`（文字或 `[{ type: "text", text }]`）发起一轮，返回 `{ id, status, … }` |
| `steer(id, input, turnId?)` | 会话正在跑时往当前这一轮插话 |
| `interrupt(id, turnId)` | 中断一轮 |
| `compact(id)` | 压缩会话上下文 |

- 只开放上面几个方法（没有删除）。页面里 `inject: ["sessions"]`；常驻后台、模型工具、钩子的上下文里也有 `sessions`，
  由 Codex++ 在 Codex 页面里执行，所以 Codex 需要开着。
- `startTurn` 会真的让模型回复（消耗额度），只在用户要求时用。读会话用 `sessionQuery`。

### 模型供应商

拓展可以声明模型供应商，并入 Codex++ 网关的供应商池（网关模式下 Codex 的模型列表里出现 `<id>/<模型>`）：

```json
{
  "id": "my-relay",
  "config": { "apiKey": { "type": "string", "title": "API Key", "secret": true } },
  "providers": [{
    "id": "myrelay", "name": "我的中转", "protocol": "chat",
    "baseUrl": "https://relay.example.com/v1", "apiKey": { "config": "apiKey" },
    "models": ["gpt-x", "gpt-y"]
  }]
}
```

- `protocol`：`responses`（默认）/ `chat` / `anthropic`。`apiKey` 写字面值，或 `{ "config": "键" }` 引用拓展的 string 配置项
  （用户在拓展设置里填，Key 不写进代码）。`models` 至少一个；每个拓展最多 8 个供应商。
- `"discoverModels": true`：从上游 `/models` 拉模型列表（10 分钟刷新一次），与 `models` 合并；这时 `models` 可以为空。
- 只在构建 Agent 模型资料与网关路由时并入，不写进 `gateway.json`；与用户已有的同 id 供应商以用户的为准，
  写了 `"override": true` 时顶替用户的那个（位置不变）。拓展停用即消失、被顶替的恢复。
- 要用代码实现一个供应商（用代码写供应商适配器）：在常驻后台里起一个本机 HTTP 服务实现 Chat / Responses 接口，
  `baseUrl` 指向 `http://127.0.0.1:<端口>`。

### 拓展调模型

页面里 `ctx.llm`，常驻后台、模型工具、钩子的上下文里 `llm`，经 Codex++ 协议代理用当前的供应商与模型路由发一次请求：

```js
const { text, model, usage } = await ctx.llm.complete({
  input: "把下面这段话改成一句话摘要：……",   // 字符串，或 [{ role: "user" | "assistant" | "developer", content }]
  instructions: "只输出摘要",                 // 可选
  model: "gpt-5-mini",                        // 可选，默认 Codex 配置里的 model
  reasoningEffort: "low",                     // 可选；还有 maxOutputTokens、temperature
});
const { models, default: current } = await ctx.llm.listModels(); // Codex 当前模型目录里的模型
```

- 这次请求不经过任何拓展的网关中间件（免得自己改写自己）；上游是 Responses 还是 Chat Completions、流式还是整段都会整理成 `text`。
- 会消耗额度，单次最长 120 秒。

### 技能、子 agent、MCP 服务器与全局指令

这几类是 Codex 自己的能力，拓展启用时由 Codex++ 写进 Codex 的配置目录，停用时撤掉（只动自己写过的，不覆盖你已有的同名条目）：

```json
{
  "id": "review-kit",
  "agents": [{ "name": "reviewer", "description": "代码审查，改完代码后派它", "instructions": "prompts/reviewer.md", "model": "gpt-5", "reasoningEffort": "high" }],
  "mcpServers": {
    "docs": { "command": "node", "args": ["${extensionDir}/mcp/server.mjs"], "env": { "API_KEY": "${config.apiKey}" } },
    "search": { "url": "https://example.com/mcp" }
  },
  "instructions": "rules.md",
  "config": { "apiKey": { "type": "string", "secret": true } }
}
```

- `skills`：包内含 `SKILL.md` 的目录列表；省略时自动收 `skills/*/SKILL.md`。装进 `~/.codex/skills/<拓展 id>-<目录名>`。
- `agents`：写成 Codex 的 `[agents.<拓展 id>-<名字>]` 角色；`instructions`（文字或包内 .md）、`model`、`reasoningEffort`
  放进 Codex++ 生成的角色配置文件。`nicknames` 可给角色起几个昵称。
- `mcpServers`：本地进程（`command` / `args` / `env` / `cwd`）或 HTTP（`url` / `headers`），二选一；
  写成 `[mcp_servers.<拓展 id>-<名字>]`。字符串里的 `${extensionDir}` 换成包目录，`${config.键}` 换成拓展配置值。
- `instructions`：文字或包内 .md，合并进 `~/.codex/AGENTS.md` 里 Codex++ 托管的一段——官方直连也生效，适合改系统提示词。
- 改动在新开的 Codex 对话里生效。

### 提示词与资源

```json
{
  "prompts": [{ "name": "review", "description": "审查一个文件", "arguments": [{ "name": "file", "required": true }], "template": "prompts/review.md" }],
  "resources": [{ "name": "团队规范", "file": "docs/style.md" }]
}
```

- 经 Codex++ 拓展 MCP（`codex_plus_extensions`）提供：提示词对外名 `<拓展 id>__<名字>`，模板里的 `{{参数}}` 换成调用时的值；
  资源地址 `codexpp://<拓展 id>/<包内路径>`，单个文件最多 1 MiB。

### HTTP 路由与定时任务

```json
{
  "backend": { "script": "backend/main.mjs", "http": true },
  "schedules": [{ "method": "digest", "at": "09:00" }, { "method": "poll", "everyMinutes": 30 }]
}
```

```js
// backend/main.mjs
export default ({ sessions }) => ({
  // POST http://127.0.0.1:<Codex++ 本地代理端口>/ext/<拓展 id>/hook
  http(request) {          // { method, path, query, headers, body, json? }
    if (request.path === "/hook") return { status: 200, json: { ok: true } };
    return { status: 404, body: "not found" };
  },
  async digest() { /* 每天 9 点 */ },
  async poll() { /* 每 30 分钟 */ },
});
```

- `backend.http: true` 才开放 `/ext/<拓展 id>/…`；返回字符串当纯文本，返回 `{ status?, headers?, body?, json? }` 按字段组装。
  带 `Origin` 的请求（网页发来的）一律拒绝。
- `schedules`：`everyMinutes`（1~10080）或 `at: "HH:MM"`（本机时区）二选一，最多 8 条，需要同时声明 `backend`。
  Codex++ 每分钟检查一次，到点调用后台方法（参数 `{ scheduledAt, schedule }`）；上次运行时间记在拓展数据目录，重启不补跑也不重复。
  Codex++ 没在运行时不会触发。

### 输入触发、快捷键、主题与提问

```js
codexPlus.extension((ctx) => {
  // 输入框里打 # 时弹候选，选中后把「#查询词」换成 insert（@ $ / 是 Codex 自己的，不能用）
  ctx.registerInputTrigger({ trigger: "#", items: async (query) => [{ label: "#123 登录失败", description: "issue", insert: "#123 " }] });
  // 快捷键：Mod 在 macOS 是 ⌘，其余是 Ctrl；必须带修饰键（或 F1~F12）
  ctx.registerShortcut({ key: "Mod+Shift+K", run: () => ctx.toast("按下了") });
  // 主题：同一时间只生效一个，用户的选择记在本机
  ctx.registerTheme({ id: "sepia", name: "复古", variables: { "--color-token-main-surface-primary": "#f4ecd8" }, css: "body{letter-spacing:.01em}" });
  ctx.themes.apply("sepia");     // 也可以 inject: ["themes"] 后 list() / current / apply(null)
  // 问用户：返回 { value, text }，取消为 null
  const answer = await ctx.ask({ title: "要继续吗？", options: [{ value: "no", label: "算了" }, { value: "yes", label: "继续" }] });
  const named = await ctx.ask({ title: "起个名字", input: { placeholder: "名字" } });
});
// 常驻后台、模型工具、钩子：ui.ask({ … }) / ui.toast("文字")，在 Codex 页面里弹（最长等 10 分钟）
```

- `@`（提及）、`$`（技能）、`/`（命令）是 Codex 自己的触发符，拓展注册会报错。
- 输入触发在光标前是「触发符 + 查询词」（触发符前是开头或空白、查询词里没有空白）时弹出，↑↓ 选、回车 / Tab 确认、Esc 关。
- 主题切换时发 `theme/change` 事件；拓展装上 / 卸下后合并发一次 `extensions/changed`（`{ extensions: [{ id, scriptKey, version }] }`）。
- 每次从 Codex++ 读到设置（启动时、之后每次心跳）发 `settings/loaded`（`{ reason: "startup" | "heartbeat", changedKeys? }`）。
  心跳时 `changedKeys` 是这次值变了的设置字段名（只有名字没有值，空数组表示没变），管理器里改的设置靠它识别；启动时不带。
- 当前供应商换了发 `relayProfile/switched`（`{ previous, profile }`）；决定模型目录的字段变了（模型列表、每模型窗口 / 自动压缩 /
  元数据、聚合成员、换供应商）发 `modelCatalog/changed`（`{ profile }`），新目录用 `/model-catalog/public` 取。
  `profile` 形状同 `/settings/public` 的 `activeProfile`。页面里改的立即发，管理器里改的经心跳（约 5 秒）发；第一次读到设置时不发。
- 在 Codex 页面里改了一项 Codex++ 设置后发 `settings/changed`（`{ key, value, persisted }`）：`key` 是改动的设置字段
  （存在 Codex++ 后端的用后端字段名，如 `codexAppCapacityAutoRetry`），先按新值发一次 `persisted: false`（这时读设置已是新值），
  写进后端后再发一次 `persisted: true`。管理器里改的设置经心跳的 `settings/loaded` 过来。
  跟着设置走的内置拓展都靠订阅这两个事件跟上设置变化，Codex++ 读写设置的地方不点名任何功能。

### 内置拓展与顶替

Codex++ 自带的功能全部是内置拓展：管理器「Codex → 内置拓展」页与 Codex 里的 Codex++ 设置页按同一份清单
（`crates/codex-plus-core/assets/enhancement-settings.json`）列出它们，每项一个开关。页面里跑的内置功能各是一个
内置拓展实例（`scriptKey` 为 `builtin:<id>`），和第三方拓展走同一套生命周期，在 `extensions/changed` 里也能看到。

- **每项只有一个开关**。有对应设置项的（会话删除、粘贴修复、梦皮、语音听写……），开关就是那个设置项，默认值也跟着它；
  没有设置项的（模型分组、多开身份提示、会话标题中间件）启停记在用户脚本配置的 `scripts["builtin:<id>"]` 里。
  子选项（删除不再确认、建议直接发送、重试间隔、对话宽度……）作为所属内置拓展的参数，开关打开后在它下面显示。
- 「启用内置拓展」总开关关闭时暂停全部内置拓展；「独立模式」里的补丁不受它控制。
- 页面里跑的内置拓展每个都只是一个 `apply(ctx)`，只用公开的 ctx 接口：多开身份提示挂在 `overlay` 插槽给的容器里，
  侧聊保存挂在 `codex/exchange` 中间件上，
  粘贴修复的监听经 `ctx.listen` 挂上；Codex 重建界面后要补回去的（语音听写、轮次回退、梦皮、会话 ID 角标、
  滚动位置恢复、对话宽度、已删会话的行过滤……）经 `ctx.onScan` 登记扫描步骤；跟着设置走的订阅 `settings/loaded`、`settings/changed`。
  它们的共同写法是「装上时对齐一次、卸下时再对齐一次（这时功能读到的已是停用，自己撤下）」，第三方顶替时照抄即可。
  Codex++ 的扫描循环与读写设置的地方不再点名功能，内置拓展卸下时它的监听、中间件、扫描步骤、定时器都跟着撤掉；
  同页重复注入时先卸下上一份注入装上的实例。会话删除、Markdown 导出也是经 `registerRowAction` 注册的会话行操作
  （删除是 `placement: "inline"`，导出带 `surfaces: ["sidebar", "archived"]`），内核只是按注册项画按钮组的宿主；
  语音听写的快捷键监听在内置拓展装上时才开始，没装着时什么也不跑。
  「改 Codex 原生界面」的能力（原生菜单、原生重试按钮、原生会话管理器）刻意不开放给拓展（见「尚未开放的接口」）。
- 拓展包在清单里声明 `replaces` 即可顶替内置功能，它启用时内置的让位、停用时内置的回来：

```json
{ "id": "my-groups", "renderer": "renderer.js", "replaces": ["model-picker-groups"] }
```

  已经改成官方拓展包的原内置拓展（`usage-statusline`、`proxy-request-status`）仍可写进 `replaces`：
  你的拓展启用时对应的官方拓展包让位（管理器里显示停用、`replacedBy` 是你的拓展），停用时它回来。

  在 Codex++ 后台生效的内置拓展（原生 Fast 强制解锁、原生输入预测、模型白名单解锁、强制中文界面、第三方推理折叠、
  纯 API 禁用 Work 视图、原生浏览器请求标识兼容、历史轮次补全、桌宠跟随鼠标）同样能顶替：被顶替时 Codex++ 后台把它们的设置项
  按「关」处理（多数要重启 Codex 才完全生效），顶替它们的拓展用下面这些接口写出等价功能：

  | 内置拓展 | 等价写法 |
  | --- | --- |
  | 原生 Fast 解锁、原生输入预测、强制中文、模型白名单解锁 | `preload`（在 Codex 自己的脚本之前执行，配 `reloadOnLaunch: true`），见「页面最早期脚本与启动前后」 |
  | 纯 API 禁用 Work 视图、原生浏览器请求标识兼容 | `launch.before` / `launch.after`（启动 Codex 之前 / 退出之后跑 Node 脚本）、`launch.args` |
  | 第三方推理折叠 | 网关响应流中间件 |
  | 历史轮次补全 | 订阅公开事件 `codexpp/session-index-repaired`（顶替后，会话索引修复结束时发，`{ aborted, checkedAtMs }`） |
  | 桌宠跟随鼠标 | 路由 `/system/cursor-position`（系统鼠标位置）+ `/cdp/evaluate` 往 `avatar-overlay` 页面推坐标 |

  只有「官方额度解锁」不能顶替：它只在关闭增强时生效，那时拓展根本不加载。
- **官方拓展包**：随 Codex++ 一起发布、来源为 `bundled` 的拓展包（脚本 key `bundled:<目录名>`），嵌在程序里，第一次用到时写到
  `<配置目录>/bundled_extensions/` 下，之后和别的拓展包一样扫描、启停、热重载，可以停用、不能删除。目前有「Agent 桌宠」
  （`bundled:agent-pet`，源码在 `assets/extensions/agent-pet/`）：原来的内置桌宠与大肥鱼整体改成了它，页面代码只用公开接口，
  数据服务是白名单路由 `/agent-pet/catalog`、`/agent-pet/image`、`/agent-pet/sound`、`/agent-pet/import-sound`、`/whale/media`、
  `/whale/full`、`/whale/history`、`/whale/session`——这些路由只在有启用中的拓展在清单里声明了它们时才干活，供应商 Key 只在 Codex++ 里用。
  它的设置界面也在包里（`settings.js`），Codex「拓展」详情页和管理器「设置」用的是同一份。可以照着它写自己的桌宠。
  另有「打字特效」（`bundled:typing-effects`）与「背景图」（`bundled:image-overlay`），源码在 `assets/extensions/` 下：
  打字特效是纯页面功能，不用任何路由；背景图的图片经白名单路由 `/image-overlay/image` 读，这条路由只读背景图拓展自己配置里的
  那张本机图片、不接受页面传入的路径，同样只在有启用中的拓展声明了它时才干活。原来的打字特效、背景图设置在第一次用到时换算成
  它们的配置，启停照旧。
  「用量状态栏」（`bundled:usage-statusline`）与「请求状态条」（`bundled:proxy-request-status`）也在 `assets/extensions/` 下：
  分别经白名单路由 `/thread-usage-live`、`/proxy-status` 取数，挂在 `composer.below`、`composer.above` 插槽里，并提供
  `codexpp.usage`、`codexpp.proxyStatus` 服务。原来的「用量状态栏」「请求状态条」开关在第一次用到时换算成它们的启停（默认都开）。
- `codexPlus.builtins()` 返回 `[{ id, name, active, disabled, replacedBy }]`：`active` 表示内置拓展实例在用（没停用、没被顶替）；
  有设置项的内置拓展，功能本身是否打开还看那个设置项。「多开身份提示」完全用公开接口（`overlay` 插槽）写成，
  可以当作写法参考；「会话标题中间件」是 `ctx.intercept("session/title", (value, next) => …)` 链：会话首轮结束后以
  `{ sessionId, title, firstMessage, lastMessage }` 跑一遍，返回值的 `title` 与原标题不同就改名。

### 页面最早期脚本与启动前后

有些功能必须赶在 Codex 自己的脚本之前、或在 Codex 启动之前做（改写 Codex 的模块、改启动参数、改 Codex 的本地状态文件）：

```json
{
  "id": "my-fast",
  "preload": { "scripts": ["preload/fast.js"], "reloadOnLaunch": true },
  "launch": { "args": ["--lang=zh-CN"], "before": "launch/before.mjs", "after": "launch/after.mjs", "timeout": 10 },
  "replaces": ["native-fast-unlock"]
}
```

- `preload`：经 CDP 的「新文档脚本」登记，Codex 页面每次新建文档时在 Codex 自己的脚本之前执行（同 Codex++ 的原生模块补丁）。
  脚本里能读 `codexPlusPreload`（`{ id, key, config }`），这时 `codexPlus` 与拓展内核还不在，只能用浏览器原生接口；
  各拓展分别 try/catch，抛错记进 `window.__codexPlusPreloadErrors`。`reloadOnLaunch: true` 时启动 Codex 后整页重载一次，
  脚本才赶得上第一份文档。改了 preload 要重启 Codex 才生效。单个脚本最大 512 KiB。
- `launch.args`：追加到 Codex 的启动参数（用户在管理器里填的同名参数优先）。只能是 `--名字` 或 `--名字=值`，最多 16 个；
  调试端口、用户数据目录、远程调试来源由 Codex++ 管，不能设。
- `launch.before` / `launch.after`：启动 Codex 之前 / Codex 退出之后各跑一次（Node，默认导出函数），拿到
  `{ event, extension, codexHome, stateDir, appDir, platform, config }`，不含任何凭据。超时 `timeout` 秒（1~30，默认 10）；
  出错、超时只记诊断日志，不挡 Codex 启动。
- 都只在用户脚本总开关与增强都开着时生效；「下次启动跳过拓展」开着时也跳过。

### 悬浮球页签、工具调用视图、设置页分区

```js
codexPlus.extension((ctx) => {
  // 右下角悬浮球面板里加一个页签（悬浮球的「下一步建议」或「回答大纲」至少开着一个时可见）
  ctx.registerFloatingTab({ title: "待办", icon: "✓", render: ({ container, close }) => {
    container.textContent = "今天还有 3 件事";
  } });
  // 对话里的工具调用 / 命令条目末尾挂一块内容；按工具名（字符串、正则或正则字符串）或条目类型过滤
  ctx.registerToolView({ tools: ["exec_command", /^mcp__/], render: ({ container, call }) => {
    // call：{ kind, itemIds, text, sessionId, callId, name, input, output }，工具名、参数、结果来自会话日志
    container.textContent = `${call.name} 输出 ${String(call.output ?? "").length} 字`;
  } });
  ctx.registerToolView({ kinds: ["plan"], render: ({ container }) => { container.textContent = "计划条目"; } });
  // Codex 设置页左侧导航末尾加一项，点开是自己画的页面
  ctx.registerSettingsSection({ title: "我的拓展", icon: "⚙", render: ({ container }) => {
    container.textContent = "设置内容";
  } });
});
```

- 三者都随拓展实例卸载撤下。工具调用条目收在 Codex 的「用时 Ns」里，用户展开后才出现；只按 `kinds` 过滤时不读会话日志。

### 按事件找写法

同一类能力在 Codex++ 里按层落在不同地方：页面里的 `ctx.on` 只能观察和改写页面经手的东西，工具执行、模型请求要在 Node 侧
（清单的 `toolPipeline` / `hooks` / `gateway`）插手。

| 想做的事 | 写法 |
| --- | --- |
| 改写或拒绝本轮用户输入、换模型 / 推理强度 | 页面 `ctx.on("agent/pre-step" / "agent/request", (params, next) => …)`（见上一节） |
| 观察全部会话的事件、轮次开始 / 结束 | 页面 `ctx.on("session/event" / "agent/status", …)` |
| 工具结果写进会话、新会话出现、一轮被中止时得到通知 | 页面 `ctx.on("tools/result" / "agent/created" / "agent/error", …)`：`tools/result` 收到 `({ callId, name, arguments, agent }, { isError, value, content })`，`agent` 只是 `{ id, session: { id } }` 描述 |
| 工具执行前改参数 / 拦截、代为执行、执行后改结果 | 清单 `toolPipeline` 的 preExecute / execute / postExecute |
| 本轮要停下时要求继续、会话开始时补上下文、子 agent 起止 | 清单 `hooks`：Stop / SessionStart / SubagentStart / SubagentStop |
| 改系统提示词、改写模型回复流 | 清单 `gateway.request` / `gateway.response`；只改提示词也可以用 `instructions` |
| 上游出错时重试或降级 | 清单 `gateway.error` |

已经按 dsh 插件写法写好的插件（`apply(ctx)` 里 `ctx.on("tools/pre-execute", …)`、`ctx.tools.register(…)`、`ctx.commands.register(…)`
等）不用改写：在管理器「拓展 → dsh 插件」用本地目录导入，或者放进拓展包里、在清单里用 `dsh` 指向它：

```json
{ "id": "my-kit", "renderer": "renderer.js", "dsh": "dsh-plugin" }
```

`dsh-plugin/` 是包内一个带 `package.json` 的目录（单个插件或 bundle 都行）。对话造拓展的写入工具同样认 `dsh` 与 `instances`。拓展启用时 Codex++ 把它作为 dsh 插件导入
（id 为 `ext-<拓展 id>`）并生成配套拓展包，拓展停用时一起停用、删除时一起移除；插件目录或拓展版本变了会重新分析。
包里的 `Service` 子类插件会实例化并提供给同包插件 `inject`（支持行上的 `isolate` 与 `ctx.isolate`）；插件之间的自定义事件
（`ctx.emit` / `serial` / `waterfall` …）跨包也能收到；包导出的 `./typert` 会自动登记，前端半边用 `ctx.remote.<namespace>.<method>()` 调；
`ctx.webServer.registerUpgrade` 注册的 WebSocket 经 `/ext/dsh-<插件>/<路径>` 可用（Host 必须是本机回环地址，网页只接受同源）。
支持情况见 `docs/plans/2026-10-30-dsh-plugin-import-design.md`（第 14 节是最新一期）。

### 拓展间服务

```js
// 提供方
codexPlus.extension((ctx) => {
  ctx.provide("metrics", { record: (name) => console.log("metric", name) });
});
// 使用方：声明 inject，依赖齐全才应用
codexPlus.extension({ inject: ["metrics"], apply(ctx) { ctx.metrics.record("opened"); } });
```

- `ctx.provide(name, value)`：服务名是全局扁平的命名空间，重名抛错；随提供方卸载撤销。
- `inject`：写在 `codexPlus.extension({ inject, apply })` 里（或第二个参数 `{ inject }`）。依赖没到时挂起、不报错；
  提供方卸载或被替换时依赖方先卸载、回到挂起，提供方回来后自动重新应用。此时 `codexPlus.extension` 返回的是
  生命周期句柄 `{ id, state, ctx, dispose() }`，`state` 为 `pending` / `active` / `failed` / `disposed`。
- 可选依赖：`inject` 用对象写法 `{ metrics: { required: false } }`，缺了照样应用，`ctx.metrics` 为 `undefined`；
  提供方出现、消失或被替换时同样重新应用。也可以不写进 `inject`，用到时 `ctx.get(name)`（不存在为 `undefined`）。
- `ctx.inject(deps, apply)`：子作用域，只让依赖某服务的那部分挂起 / 重建。
- 内置服务：`sessionQuery`、`locale`；官方拓展包的数据也是服务：`codexpp.usage`（用量状态栏提供，`cached(threadId?)` 取它算好的速度、缓存命中、上下文占用）、
  `codexpp.proxyStatus`（请求状态条提供，`requests()` 最近的代理请求、`fetch()` 立即问一次）。这两个服务随提供它的拓展包启停，
  要自己画界面又不想依赖它们，可以直接调白名单路由 `/thread-usage-live`、`/proxy-status`。`codexPlus.services()` 列出当前已提供的服务与提供方。

#### 后台提供的服务（跨进程）

常驻后台也能提供服务，页面、其它后台、模型工具、钩子都按名字调用，调用都落到同一个后台进程：

```json
{ "backend": { "script": "backend/main.mjs", "provides": ["counter"] } }
```

```js
// backend/main.mjs
export default ({ provide }) => {
  let total = 0;
  provide("counter", { add: (n) => (total += n), total: () => total });
  return {};
};
// 页面：codexPlus.extension({ inject: ["counter"], async apply(ctx) { await ctx.counter.add(1); } })
// 后台 / 模型工具 / 钩子：await service("counter").add(1)   （service 在默认导出的参数或第二个参数里）
```

- 服务名要在 `backend.provides` 里声明（字母开头，字母、数字、`_ . -`），后台代码里用 `provide(name, impl)` 实现；
  同名时按拓展 key 顺序先声明的生效。调用参数和返回值要能 JSON 序列化，方法都是异步的。
- 页面：注入包把后台服务清单交给页面，登记成可 `inject` 的服务；页面里已有同名服务时以页面的为准。
- 拓展 MCP（模型工具）与钩子进程不托管后台，它们的调用经 Codex 页面桥接转给托管后台的 Codex++ 进程，
  所以 Codex 需要开着。

### 多语言

```
my-ext/
├── codexpp.json
├── locale/en.json   { "meta": { "title": "Word Count", "description": "..." }, "count": "{n} words" }
└── locale/zh.json   { "meta": { "title": "字数统计", "description": "..." }, "count": "{n} 字" }
```

```js
codexPlus.extension((ctx) => {
  ctx.toast(ctx.locale.t("count", { n: 42 }));               // 包内词条，自动登记在拓展 id 命名空间
  ctx.locale.register("menu", { zh: { open: "打开" }, en: { open: "Open" } });
  const t = ctx.locale.bind("menu");
  ctx.on("locale/change", ({ locale }) => ctx.toast(`${locale}: ${t("open")}`));
});
```

- 语言文件名就是语言 id（`en`、`zh`、`zh-TW`），每个最大 256 KiB；`meta.title` / `meta.description` 是本地化的
  名称与描述，管理器按界面语言显示，缺了就用清单里的 `name` / `description`。
- 查找顺序：当前语言（如 `zh-CN`）→ 主语言（`zh`）→ `en`，先在指定命名空间、再在 `common`，都没有返回 key；
  占位符写 `{name}`，键可以用点号取嵌套词条（`meta.title`）。
- `ctx.locale.current` 是 Codex 页面的语言；`ctx.locale.resolve({ en, zh })` 解析多语言文本对象。
- 也可以 `inject: ["locale"]` 用全局的 `locale` 服务（`register` 返回撤销函数，需要自己撤销）。

### 常驻后台

页面脚本拿不到本地文件、进程和长时间运行的计时器。拓展包可以声明一个常驻后台，由页面经 `ctx.backend` 调用：

```json
{ "id": "notes", "name": "便签", "renderer": "main.js", "backend": { "script": "backend/main.mjs" } }
```

```js
// backend/main.mjs：默认导出 (ctx) => 方法表；ctx = { extension, config, dataDir, emit, log, sessionQuery, provide, service }
import fs from "node:fs";
export default ({ emit }) => ({
  read: () => (fs.existsSync("note.txt") ? fs.readFileSync("note.txt", "utf8") : ""),
  save: (text) => { fs.writeFileSync("note.txt", String(text)); emit("saved", { length: String(text).length }); return true; },
});
```

```js
// main.js（页面）
codexPlus.extension((ctx) => {
  ctx.backend.on("saved", ({ length }) => ctx.toast(`已保存 ${length} 字`));
  ctx.registerCommand({ name: "note", description: "读便签", run: async () => ctx.toast(await ctx.backend.call("read")) });
});
```

- 第一次 `call` 时用 Node 启动（优先 Codex App 自带的 Node，或 `CODEX_PLUS_NODE`），之后常驻在 Codex++ 启动器里；
  页面加载、拓展热重载 / 启停时全部停止，下次调用重新启动。内存状态不持久，要持久就写文件。
- 工作目录是 `~/.config/Codex++/extension-data/<拓展 id>/`（即 `ctx.dataDir`）。
- 与后台之间是行式 JSON：方法只收一个 JSON 参数、返回可序列化的值，抛错即 reject。默认超时 20 秒，最多 24 秒。
- `emit(event, payload)` 进有界队列（每个后台保留最近 200 条），页面 `ctx.backend.on` 以长轮询取走；`on` 不会启动后台。
- 后台以你的用户权限运行、没有沙箱。只启用你信任的拓展。

### 管理器面板

拓展可以在 Codex++ 管理器里有自己的一块界面（统计、批量操作、比设置表单更复杂的配置）。清单里声明 `manager`：

```json
{ "id": "notes", "name": "便签", "backend": { "script": "backend/main.mjs" }, "manager": { "page": "manager.html", "title": "便签统计" } }
```

管理器「本地拓展」里这一行的「…」菜单会出现「打开面板」，面板就展开在这一行下面。页面里有 `window.codexPlusManager`：

```html
<!-- manager.html：脚本、样式都内联 -->
<p>共 <b id="count">…</b> 条</p>
<button id="clear">清空</button>
<script>
  const api = window.codexPlusManager;
  api.backend.call("stats").then(({ count }) => { document.getElementById("count").textContent = count; });
  document.getElementById("clear").onclick = async () => { await api.backend.call("clear"); api.toast("已清空"); };
  api.onTheme((theme) => document.body.classList.toggle("dark", theme === "dark"));
</script>
```

| `codexPlusManager` 成员 | 说明 |
| --- | --- |
| `config.get()` / `config.set(values)` / `config.onChange(fn)` | 读写本拓展的配置（只能写清单 `config` 里声明过的项，类型要对）；保存后和设置表单一样热更新到 Codex 页面 |
| `backend.call(method, params)` | 调本拓展的常驻后台（`backend.script` 导出的方法），超时 30 秒 |
| `asset(path)` | 本拓展 `assets/` 里资源的 data URL |
| `toast(text)` | 在管理器里弹一条提示 |
| `theme()` / `onTheme(fn)` | 管理器当前是 `dark` 还是 `light`；`<html data-theme>` 也会跟着设好 |
| `data(name, params)` | 读清单 `manager.data` 里声明过的只读管理器数据（见下） |
| `navigate(target)` | 跳到管理器的另一个页面 |
| `call(route, payload)` | 调本拓展清单 `permissions.routes` 声明过、且开放给拓展页面的 Codex++ 数据路由（目前是 `/agent-pet/*`） |

- 页面放在**沙箱 iframe** 里，允许脚本和表单交互：拿不到管理器页面与它的接口，不能弹窗、不能跳转顶层页面。
- CSP 限制外部资源请求和表单外发（`default-src 'none'`、`form-action 'none'`）：脚本、样式写在页面里，图片等资源用 `asset()` 取，
  要访问网络或本机文件就放进常驻后台，经 `backend.call` 调。CSP 不阻止 iframe 自身导航，不能当作完全断网的安全边界；只安装信任作者的拓展。
- `page` 必须是包内的 .html 文件，最大 1 MiB；`title` 不填用拓展名。拓展停用时面板打不开。
- 也可以只声明 `manager`（不带页面脚本）：纯管理器工具类的拓展。

管理器本身也「万物皆拓展」：拓展页面可以进侧栏，也可以顶替管理器的内置页面。

```json
{ "manager": { "page": "page.html", "title": "用量看板", "nav": "management", "icon": "bar-chart",
  "replaces": ["gatewayUsage"], "data": ["requestLogs", "gatewayUsage"] } }
```

- `nav`：侧栏入口所在分组 `gateway` / `management`（`true` 同它）/ `tools`，点开是整页的拓展页面，也进 ⌘K 面板。
  `icon` 可选 `app-window`、`bar-chart`、`list`、`server`、`store`、`palette`、`network`、`book`、`waypoints`、`plug`、`split`、
  `messages`、`paw`、`git-branch`、`wrench`、`settings`、`code`、`layers`，其余用默认图标。
- `replaces`：顶替的管理器页面（路由 id：`hub`、`gatewayProviders`、`gateway`、`gatewayRouting`、`gatewayUsage`、`gatewaySettings`、
  `requestLog`、`overview`、`relay`、`multiLaunch`、`relayEnvironment`、`claude`、`sessions`、`context`、`skills`、`weixin`、`dreamSkin`、
  `pluginMarket`、`recommendations`、`agentCache`、`agentCollaboration`、`computerUse`）。打开这些页面时显示拓展页面，
  上方有「查看内置页面」随时切回。「万物皆拓展」「内置拓展」「设置」三页是停用拓展的入口，不能顶替。
- `data`：页面经 `codexPlusManager.data(name, params)` 能读的只读数据，要先在这里声明：`requestFlow`（最近的请求，`{ limit }`）、
  `requestLogs`（请求日志，`{ limit, sinceMs, agent, relayId, model }`）、`gatewayUsage`（用量统计，`{ range }`）、
  `localSessions`（本机会话，`{ offset, limit }`）。都不含 Key。
- `codexPlusManager.navigate(target)`：跳到管理器别的页面（路由 id，或 `ext:<拓展 key>` 打开另一个拓展页面）。

#### 公共只读数据

除上述四类外，`manager.data` 还可以声明以下七类。调用方式相同；新增数据返回下表对象（没有 `status` / `message` 包装），读取失败时 Promise reject，旧四类的参数与返回结构不变。

| 数据名 | 返回与范围 |
| --- | --- |
| `gatewayProviders` | `{ providers }`：网关供应商的 `id/name/enabled/protocol/models/modelWindows/contextWindow/keyRpm/keyConcurrency/price/modelPrices`；不是直连供应商列表 |
| `gatewayRouting` | `{ groups, autoGroups }`：组的 `id/name/mode/sticky/members[{model,weight}]/hideMembers/pick`；自动组额外给 `hidden/customized` |
| `gatewayHealth` | `{ scope: "managerProcess", providers, speeds, agents }`：供应商熔断状态、剩余冷却时间、连续错误数；模型首字耗时样本；Agent 请求／改道／失败计数及进行中的供应商 id。不返回 Key id、错误原文、完整请求轨迹 |
| `gatewayQuota` | `{ scope: "managerProcess", cachedOnly: true, providers: [{ id, name, quota }] }`：已有额度缓存，`quota` 为 `{ windows: [{ windowMinutes, usedPercent, resetsAtMs }], updatedAtMs }`；无缓存时为 `null`，不是零额度 |
| `installedSkills` | `{ skills: [{ id, name, enabled, bundled, managed, agents, contentHash }] }`：本地已装技能；`enabled` 对应 Codex，其他 Agent 看 `agents`；不读远端目录，版本以内容哈希表示 |
| `installedPlugins` | `{ plugins: [{ id, name, version, source, enabled }] }`：仅 Codex++ 插件市场托管的本地安装记录，不代表全部 Codex 插件；`source` 仅 `public/full`，`id` 沿用原生 `name@market` 标识 |
| `agentCollaboration` | `{ enabled, agents, tasks, total }`：Agent 的 `id/name/enabled/model/maxConcurrentTasks`；任务的 `id/agentId/status/createdAtMs/updatedAtMs/cancelRequested`。参数 `{ limit }` 默认 50、范围 1–200，按更新时间倒序、同时间按 id 升序 |

```json
{ "manager": { "page": "panel.html", "data": ["gatewayProviders", "gatewayHealth", "gatewayQuota"] } }
```

```js
const { providers } = await codexPlusManager.data("gatewayProviders");
const { providers: quotas } = await codexPlusManager.data("gatewayQuota");
// 额度是缓存：quota === null 表示暂未采集，不能显示成余额为零。
```

- 新增数据在宿主侧重新检查拓展总开关、当前启停状态、有效清单及 `manager.data` 声明；停用后已打开的页面也不能继续读取。
- 数据按明确字段构造，不透传内部配置、供应商地址、代理地址、Key（包括打码 Key）、磁盘路径、任务提示词／答案／进度／错误。
- `gatewayHealth`、`gatewayQuota` 只读**当前管理器进程**已有缓存；网关在别的进程处理请求时可能为空或不是最新，不保证跨进程汇总。
- 不主动探测／刷新上游、不安装下载、不启动执行器。账户余额当前只有主动联网刷新入口，暂不开放；`gatewayQuota` 是额度窗口，不是账户余额。
- `agentCollaboration` 只读主数据库已落盘快照，不刷新租约、不创建数据库／schema／WAL sidecar，也不读取尚未 checkpoint 的 WAL；状态可能落后于执行中的任务。
- 这些是 manager iframe 的数据能力，不扩展 renderer 的 `codexPlus.call` 白名单，也不开放设置写入、删除、安装或任意 Tauri 命令。类型定义见 `types/codex-plus-extensions.d.ts`。

### 资源文件

拓展包里的图片、音频、字体放在 `assets/` 下，页面代码用 `ctx.asset(path)` 取 data URL：

```js
codexPlus.extension((ctx) => {
  const audio = new Audio(ctx.asset("assets/ding.mp3"));
  ctx.registerComposerAction({ label: "叮", icon: "🔔", onActivate: () => audio.play() });
});
```

- 支持 png / jpg / gif / webp / svg / ico / avif / mp3 / wav / ogg / m4a / woff / woff2 / ttf / otf。单个最大 1 MiB，
  一个包合计最大 4 MiB，超出的不下发（管理器里能看到 `skippedAssets`）。资源随拓展下发给页面，越小越好。
- 页面里不能用 `file://` 加载本机文件。对话造拓展时，agent 用写入工具 `files[].sourcePath` 把本机文件按字节复制进包。

### 拓展包

一个拓展可以是一个目录：放在 `user_scripts/` 下，根部有 `codexpp.json`。

```
user_scripts/
  clock/
    codexpp.json
    renderer.js
```

```json
{
  "id": "clock",
  "name": "时钟",
  "version": "0.1.0",
  "description": "图标栏里的时钟页面",
  "engines": { "codexpp": 1 },
  "renderer": ["renderer.js"],
  "permissions": { "routes": ["/diagnostics/log"] }
}
```

| 字段 | 说明 |
| --- | --- |
| `id` | 必填，只能包含字母、数字、`-`、`_` |
| `name` / `version` / `description` | 管理页展示用 |
| `icon` | 图标：包内 `assets/` 下的图片（png / jpg / gif / webp / svg / ico / avif，如 `"assets/icon.svg"`），或 https 地址。显示在 Codex「拓展」页面的侧栏、列表和详情头部；找不到或不是图片时用默认图标 |
| `engines.codexpp` | 整数：需要的 `apiVersion`；字符串：对 Codex++ 版本的 semver 范围（如 `"^2.1"`、`">=2.1.0 <3"`、`"2.x"`、`"1.0 - 2.5"`、`"^1 \|\| ^2"`）。不满足时不加载并在管理页显示原因 |
| `renderer` | 字符串或数组，必须是包内相对路径；按顺序拼进同一个作用域执行。只声明 `hooks` 的拓展可以省略 |
| `hooks` | Codex 钩子声明，见上文「Codex 钩子」 |
| `gateway` | 网关请求中间件声明，见上文「网关请求中间件」 |
| `tools` | 给模型的工具声明，见上文「给模型加工具」 |
| `config` | 配置项声明，见上文「拓展配置」 |
| `permissions.routes` | `ctx.call` 能调的路由，需在[路由白名单](#后端调用与路由白名单)内。这是声明约束而不是安全边界 |

包根目录放一个 `README.md`（也认 `README.zh-CN.md` / `README.zh.md`，中文版优先，不分大小写），Codex「拓展」页面的详情
就会在「细节」页签里显示它，形态对齐 VS Code 的扩展详情：「细节」是 README，「设置」是设置面板或通用表单，「功能」列出清单里
接管的能力（工具、钩子、网关、后台、Skills……），右侧栏是标识符、版本、来源与链接。README 支持常用 Markdown（标题、列表、
任务列表、引用、代码块、表格、链接、图片），原始 HTML 当文字显示；链接只放行 http(s) / mailto / 页内锚点，图片只放行 https
与包内 `assets/` 下的文件（写相对路径，如 `![截图](assets/shot.png)`）。最多读前 512 KB。

清单不合法（JSON 错误、入口越出包目录、版本不兼容等）时包不注入，管理页显示「加载失败」和原因。
未知字段会被忽略，后续版本会在清单里增加更多能力面。拓展包是文件夹，管理页不提供删除按钮，请在目录里手动移除。
整体设计见 [docs/plans/2026-10-30-codexpp-extension-system-design.md](docs/plans/2026-10-30-codexpp-extension-system-design.md)。

### 在 Codex 对话里造拓展

不想自己写代码时，可以让 Codex 帮你写：在管理器「拓展」页的「对话造拓展」一栏点「接入 Codex」，然后新开一个 Codex 对话，
直接说需求，例如「帮我做个拓展：图标栏加一个番茄钟」。说「插件」也行，agent 知道这里指的是 Codex++ 拓展。

接入后 Codex 多了一个本机 MCP 服务 `codex_plus_extensions`（由 Codex++ 主程序以 `--codex-plus-extension-mcp` 运行），提供：

| 工具 | 作用 |
| --- | --- |
| `codexpp_extension_guide` | 创作指南（`assets/extension-creator/guide.md`）、API 版本与可用路由 |
| `codexpp_extension_list` / `codexpp_extension_read` | 列出拓展与加载状态、读回某个拓展的源码 |
| `codexpp_extension_inspect` | 只读查看正在运行的 Codex 页面：关键区域概览，或按文字 / 选择器查元素，返回稳定属性与建议选择器 |
| `codexpp_extension_screenshot` | 截取 Codex 页面的某个元素或整个窗口，写完拓展后确认效果 |
| `codexpp_extension_write` | 写入 `user_scripts/<id>/`（清单由参数生成），立刻热重载并返回加载状态与报错 |
| `codexpp_extension_set_enabled` / `codexpp_extension_reload` | 启停、手动热重载 |

边界：

- 只写用户目录下的拓展包，同名目录必须是同 id 的拓展包才会覆盖；路径只能在包内。
- 大小：agent 逐字写的文本（`content`）单个 256 KiB、合计 1 MiB——这是模型一次工具调用能可靠写出的量，不是安全边界；
  本机已有的文件用 `sourcePath` 复制：图片、音频、字体单个 1 MiB、合计 4 MiB（以 data URL 随页面下发），
  打包产物等文本单个 4 MiB、合计 8 MiB（随页面注入）。用了第三方库的界面让 agent 先用 esbuild 打包成单文件再复制进包。
- 不提供删除，不要的拓展由 agent 停用，文件由你手动删除。
- 只做页内热重载。已加载的旧脚本里有没登记清理函数的，热重载会刷新整个 Codex 页面、可能打断当前对话，
  这时工具不自动重载，会让你在管理器里手动点「热重载」。
- 写入属于 MCP 工具调用，是否每次确认由 Codex 自己的工具审批设置决定。

### 发布到社区拓展

管理器「拓展」页的「社区拓展」分栏会列出 GitHub 上打了 `codexpp-extension` topic 的公开仓库，所有 Codex++ 用户都能看到并安装。

**一键发布**：在「本地拓展」里点拓展包那一行的「发布」。需要本机装了 GitHub CLI 并登录（`gh auth login`），git 配好
`user.name` / `user.email`；Codex++ 不读、不存 token，推送时只在那一条命令上借用 `gh auth git-credential`。
发布前会检查清单（名称、描述、`x.y.z` 版本号）、密钥（`.env`、`*.pem`、`auth.json` 等文件名，`sk-…`、`ghp_…`、私钥等内容，
`secret` 配置项的默认值）、软链与体积，有问题时不能发布；版本号已在远端存在时可以点「补丁号 +1」。发布会：
建公开仓库 `codexpp-<id>`（可改名）或取回已有仓库 → 用拓展包整体替换仓库内容并写入 `repository` → 没有 README 时按清单生成、
按选择加 LICENSE → 提交并打 `v<version>` → 推送 → 建 release 并上传 `<id>-v<version>.zip` → 加 topic。每一步的输出都显示在
发布框里，失败时停在那一步，可以改好后重试。

也可以手动发布：

1. 仓库**根目录**放拓展包（`codexpp.json` 和它引用的文件），推到公开仓库。一个仓库只放一个拓展包。
2. 仓库设置 → Topics 加上 `codexpp-extension`。最长 10 分钟后出现在社区栏（列表缓存周期），用户点「刷新」可立即看到。
3. 发布版本：打 `v<version>` 形式的 tag（如 `v1.2.0`）并建 release，再把包根目录打成 `<id>-v<version>.zip`
   （压缩包根目录就是包根目录）作为附件上传到这个 release。

安装时按「最新 release → 最新 `v*` tag → 默认分支」选版本，并锁定到对应的提交：

- release 里有 `<id>-<tag>.zip` 时下载这个附件，并按 GitHub 记录的 sha256 校验；
- 否则按提交从 GitHub 下载仓库压缩包；
- 一个 tag 都没有时，用户装的是默认分支当时的提交，界面会提示「作者未发布版本」。

清单里可以加几个只用于展示的字段，加载器会忽略它们：`author`、`homepage`、`icon`（仓库内相对路径，或 GitHub 上的 https 地址）、
`license`（GitHub 认不出 `LICENSE` 文件时用）、`keywords`。

社区栏标着「未审核」：用户安装前会看到这个版本声明的全部能力（后台进程、钩子、网关中间件、模型工具、路由、页面脚本），
安装后默认不启用。解包限制为 2000 个文件、10 MiB，拒绝软链和越界路径；清单的 `id` 必须与默认分支上的一致。
本机已有同 id 的拓展时需要用户确认替换，旧目录移到 `extension_backups/`。

### 精选与官方拓展

管理器「拓展 → 精选」分栏来自 Codex++ 维护的精选清单（`BigPizzaV3/CodexPlusPlusScriptMarket` 的 `index.json`），
每一条都经过审核。清单里除了老的单文件脚本，还可以放拓展包：

```json
{
  "scripts": [
    { "kind": "package", "id": "word-count", "name": "字数统计", "description": "…", "author": "alice",
      "repo": "alice/codexpp-word-count", "tag": "v1.2.0", "sha": "<40 位提交 sha>", "official": false }
  ],
  "blocked": ["mallory/codexpp-miner"]
}
```

- **锁定提交**：安装只按 `sha` 从 GitHub 下载这一个提交，不看作者后来发的 release 或 tag。作者更新后要重新审核、改清单里的
  `sha`，用户那边才会看到「更新」。`tag` 只用于展示。拓展包条目也可以单独放在顶层的 `packages` 数组里。
- **官方**：条目写了 `"official": true`，**并且**仓库属于 Codex++ 官方账号（代码里的 `OFFICIAL_OWNERS`，目前是
  `BigPizzaV3`）才显示「官方」徽章并排在最前；只满足其一的按精选显示。拓展自己的清单里写什么都不算数。
- **下架**：`blocked` 里的仓库在社区栏不显示、不能安装；本机已经装了的会在精选栏顶部提示停用（不会自动删除）。
- **与社区栏的关系**：进了精选清单的仓库不再出现在社区栏。社区栏里和精选 / 官方拓展同 id 的条目、以及非官方仓库自称「官方」
  的条目会标「可疑」并写明原因；本机装着官方或精选版本时，社区里同 id 的拓展不能替换它。
- 想申请精选：向市场仓库提 PR 加一条 `kind: "package"` 条目，写明要审核的 tag 与对应 sha；也可以直接联系维护者。
- **维护者在管理器里审核**：本机 gh 账号对市场仓库有写权限时，社区卡片上有「审核」，精选卡片在作者发了新版本时有
  「审核更新」。审核框下载并暂存仓库当前的最新版本（就是将要锁定的那个提交），列出文件、能力、密钥扫描，更新时附上与
  当前锁定提交的差异；点「通过」后管理器用 gh 往 `index.json` 的 `packages` 段写一条（同仓库则替换），作为市场仓库的
  一个提交，提交说明写明仓库与 sha。只改 `packages` 段和 `updated_at`，其余内容原样保留；疑似密钥或软链没处理前不能收录。

---

## API 参考

### `codexPlus.version` / `codexPlus.apiVersion`

- `version`：适配层版本，字符串
- `apiVersion`：接口契约版本，数字，当前为 `1`。契约发生破坏性变化时会增加

```js
if (window.codexPlus.apiVersion < 1) {
  window.codexPlus.toast("请升级 Codex++ 以使用本拓展", { type: "warn" });
}
```

### `codexPlus.script`

`{ key: string, manifest }`，当前脚本的标识与清单（拓展包才有，单文件脚本为 `null`）。用于诊断与日志。是 getter，脚本初始化结束后再读也能拿到。

### `codexPlus.toast(message, options?)`

右下角提示。

| 选项 | 说明 |
| --- | --- |
| `type` | `info` / `success` / `warn` / `error`，决定边框配色；`info` 与不传等价，用默认外观 |

- 最多同时显示 **3 条**，超出时挤掉最旧的一条（先入先出），剩余几条会自动重排位置
- 单条默认 10 秒后自动消失
- 返回值：`{ ok: boolean, value?: () => void }`。`value` 是「立即关闭这一条」的函数

### `codexPlus.call(route, payload?, options?)`

调用 Codex++ 后端，返回 Promise，**失败时 reject**。

```js
try {
  const result = await window.codexPlus.call("/diagnostics/log", { event: "hello" });
} catch (error) {
  console.error(error.message);   // 可读的失败原因
}
```

| 选项 | 说明 |
| --- | --- |
| `timeout` | 毫秒，默认 26000 |

错误归一化说明：底层桥接有两种错误风格（路由层返回 `{status:"failed"}`，面板层返回 `{error}`），这里统一转成 Promise reject，所以你只需 `try/catch`。

**超时只放弃等待，不会取消服务端任务**——桥接协议没有取消通道。另外页面刷新或桥接重连会中断在途请求。

### `codexPlus.registerRowAction(definition, options?)`

在会话行上加一项操作：默认进「更多操作」菜单，也可以画成行上常驻的图标按钮，或者同时出现在归档页的行上。
内置的「删除」（行内按钮）与「导出」（菜单项 + 归档页）就是这样注册的。

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `label` | 是 | 菜单项文字；行内按钮的可访问标签与悬停提示；归档页按钮的文字 |
| `onActivate` | 二选一 | `(context) => void`，见下 |
| `render` | 二选一 | `({ container, row, session_id, title, close }) => void \| cleanup`：自己往菜单里画一项（如 React 组件），菜单拆掉时调用返回的清理函数（只用于菜单项） |
| `icon` | 否 | 单个字形或 SVG 字符串；不传用 `◇` |
| `placement` | 否 | `"menu"`（默认）或 `"inline"`：画成行上常驻的图标按钮，套 Codex 原生「置顶 / 归档」按钮的外观 |
| `surfaces` | 否 | 出现在哪：`["sidebar"]`（默认，侧栏会话行）、加上 `"archived"` 也画在归档页行的「取消归档」旁（文字按钮） |
| `className` | 否 | 给行内按钮 / 归档页按钮加的类名（样式与识别用） |
| `onMount` | 否 | `({ button, row, session_id }) => void`：行内按钮建好、套原生外观之前调用，可设属性。按钮会随行重建，不要在这里挂监听 |

`onActivate` 收到 `{ row, session_id, title, archived, resolve, close, button?, event? }`：`row` 是该行的 DOM 元素，
`archived` 表示是不是归档页的行；归档页的行未必读得到会话 id，`await resolve()` 会按标题问 Codex++ 找到 `{ session_id, title }`。
`close()` 关闭菜单。行内按钮被 React 摘掉时整组按指纹重建。

### `codexPlus.currentSessionId()`

当前打开的 Codex 会话 id；首页、新对话还没有 id 时为空串。拓展实例上同名 `ctx.currentSessionId()`；斜杠命令的
`run(args, context)` 里也有 `context.sessionId`。

### `codexPlus.currentSession()`

当前打开的会话 `{ id, title, hostId, remote }`：`hostId` 是会话所在主机（本机为 `"local"`），`remote` 为真时是远程 / SSH 会话
（本机会话日志里没有它的用量）。

### `codexPlus.registerNavEntry(definition, options?)`

在 Codex 图标栏加一个入口。

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `label` | 是 | 可访问标签（也是 `aria-label`） |
| `onActivate` | 是 | 点击时调用 |
| `icon` | 否 | 单个字形或 SVG 字符串 |

> 一般不用直接用它——`registerPage` 会自动配一个入口。手写 `registerNavEntry` 适合「入口只做一件事、不需要独立页面」的场景。

### `codexPlus.registerPage(definition, options?)`

注册一个整页视图，**并自动配一个图标栏入口**。

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `title` | 是 | 页面标题 |
| `render` | 是 | `({ container, close, script }) => void`，**每次打开都调用** |
| `icon` | 否 | 入口图标 |

| 选项 | 说明 |
| --- | --- |
| `navLabel` | 图标栏入口的标签，默认取 `title` |
| `order` | 多个入口之间的排序 |

页面外壳复用内置页面的骨架，所以自动获得：缩放跟随、深浅色主题、图标栏选中态同步、原生选中态压制、窗口 resize 跟随。

> 为什么自动配入口：内置的三个页面（Codex++ / 拓展 / 推荐内容）都是「图标栏入口 + 整页」的形态。第三方沿用同一种形态，用户才不会去弹窗里找入口。

### `codexPlus.registerMenuItem(definition, options?)`

在 Codex++ 菜单的「主页」面板里加一行。根据你提供的字段决定形态：

**开关形态**：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `label` | 是 | 标题 |
| `onChange` | 是 | `(next: boolean) => void` |
| `description` | 否 | 副标题说明 |
| `toggleValue` | 否 | `() => boolean`，提供当前值 |

**按钮形态**：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `label` | 是 | 标题 |
| `onActivate` | 是 | `({ close }) => void` |
| `description` | 否 | 副标题说明 |
| `buttonLabel` | 否 | 按钮文字，默认「打开」 |

行为说明：

- 注册时如果菜单已经打开，会**立刻补上**
- 菜单重新打开时，开关状态从 `toggleValue()` 重新读取（所以你应当把状态存在自己那边，而不是指望 Codex++ 替你保存）
- Codex++ 没有把拓展的设置项写进后端配置——需要持久化就自己用 `localStorage` 或调后端

**注册选项**（`register*` 通用的第二个参数）：

| 选项 | 说明 |
| --- | --- |
| `id` | 自定义 id 片段；不传自动生成。同一类别内 id 不能重复 |
| `order` | 排序；内置项永远排在拓展项之前，此值只影响拓展项之间的相对顺序 |
| `scriptKey` | 显式指定归属脚本；默认取当前正在初始化的脚本，一般不用传 |

### `codexPlus.onCleanup(fn)`

注册清理函数。热重载、禁用、删除脚本时，按「脚本逆序 + 注册逆序」执行。

> **注意**：只有当**所有**已加载脚本都注册了清理函数时，热重载才走原地清理。任何一个脚本没注册（或清理抛错、返回 Promise），Codex++ 会回退到**整页刷新**。这是为了保证不残留旧实例——也意味着**你注册清理函数能让所有人的重载体验更好**。

### `codexPlus.fail(error)`

主动上报失败。

脚本**初始化期间的同步错误**会被 Codex++ 自动捕获并显示在管理页；但 `await` 之后的**异步错误不会**——这类错误要用这个方法上报，否则脚本在管理页会一直显示正常，实际已经死了。

```js
window.codexPlus.call("/thread-usage-history", { session_id })
  .catch((error) => window.codexPlus.fail(error));
```

---

## 后端调用与路由白名单

`codexPlus.call` 只允许调用白名单内的路由。未列出的路由**即使后端支持也会被拒绝**（这是刻意的：新增路由默认不开放，需要显式决定）。

| 路由 | 用途 |
| --- | --- |
| `/diagnostics/log` | 写一条诊断日志 |
| `/thread-usage-history` | 读会话用量历史 |
| `/archived-thread` | 读归档会话 |
| `/export-markdown` | 导出 Markdown |
| `/user-scripts/list` | 读拓展清单 |
| `/cdp/targets` | 列出可注入的 Codex 页面（`main` / `avatar-overlay` / `quick-chat`） |
| `/cdp/evaluate` | 在匹配的 Codex 页面执行 JS 表达式 |
| `/system/cursor-position` | 系统鼠标位置 `{ x, y }`（与 `window.screenX` 同一坐标系，只读；Windows、macOS） |
| `/agent-pet/catalog`、`/agent-pet/image`、`/agent-pet/sound`、`/agent-pet/import-sound` | Agent 桌宠的图库、图片、音效与导入自定义音效（拓展要在清单 `permissions.routes` 里声明，且启用中才干活） |
| `/request-log/recent` | 最近已完成的模型请求 `{ entries }`，按开始时间倒序；`limit` 默认 20、最多 100。每条有 `id`、`model` / `upstreamModel`、`relayName`、`durationMs`、`success`、`statusCode`、`usage.inputTokens` / `outputTokens`、`timestampMs` 等（只读，与托盘「实时请求」同源） |
| `/proxy-status` | 进行中的模型请求 `{ requests }`，每条有 `id`、`model`、`threadId`、`relayName`、`startedAtMs`、`phase`、`firstTokenMs`、`attempts`、`autoRetries`、`retryReasons`、`estimatedOutputTokens`（只读，与官方拓展包「请求状态条」同源） |
| `/thread-usage-live` | 会话最近的 token 事件 `{ session_id, events }`，参数同 `/thread-usage-history`。只读 rollout 尾部，可以秒级轮询（只读，与官方拓展包「用量状态栏」同源） |
| `/backend/status` | Codex++ 版本与连接状态 `{ status, message, version }`（只读） |
| `/settings/public` | 设置的公开子集：`codexPlusVersion`、`codexAppVersion`、`enhancementsEnabled`、`relayProfilesEnabled`、`modelCatalogFingerprint`，以及 `activeProfile { id, name, relayMode, protocol, model, contextWindow, autoCompactLimit, aggregate }`。不含地址、Key 与 config / auth 内容（只读） |
| `/model-catalog/public` | 当前供应商的模型目录：`models`、`modelMetadata`（每模型的 `contextWindow` 等）、`model`、`default_model`、`provider_name`、`model_provider`、`service_tier`。去掉了来源地址与本机路径（只读） |

**写能力没有开放**：`/settings/set`、`/delete`、`/undo` 等都不在名单内。如果你确实需要某个路由，去 Codex++ 仓库提 issue 说明用途，不要在脚本里绕过白名单直接调底层桥接（见下方「限制」第 1 条）。

`/cdp/evaluate` 是显式开放的底层注入能力，用来把脚本打进宠物 overlay 这类独立窗口。它**不是**任意 CDP 协议：只能 `Runtime.evaluate`，只能打到 `app://-/index.html` 的 Codex 页面，返回值里也不带 WebSocket URL。调用时必须提供 `kind` 或 `target_id` 其一：

```js
const { targets } = await window.codexPlus.call("/cdp/targets");
await window.codexPlus.call("/cdp/evaluate", {
  kind: "avatar-overlay",
  expression: "window.__myOverlayHook = true",
});
```

---

## 类名与属性契约

`codexPlus.constants` 暴露稳定的类名与属性。**这些值一旦发布不再更名**。

| 键 | 值 | 说明 |
| --- | --- | --- |
| `toastClass` | `codex-delete-toast` | 提示元素 |
| `pageClass` | `codex-plus-page-overlay` | 整页 overlay 根节点 |
| `actionGroupClass` | `codex-session-actions` | 会话行按钮组 |
| `moreMenuClass` | `codex-session-more-menu` | 会话行「更多操作」菜单 |
| `railSelector` | `nav[data-app-navigation-rail]` | Codex 图标栏 |
| `railDestinationSelector` | `[data-sidebar-destination]` | 图标栏原生按钮 |
| `pageNavAttribute` | `data-codex-plus-page-nav` | 页面左导航项属性 |
| `extensionAttribute` | `data-codex-plus-ext` | 拓展节点标记，见下方说明 |

**关于 `extensionAttribute`**：你自己 `appendChild` 进 Codex 容器的节点，应当带上 `data-codex-plus-ext` 属性。Codex++ 的扫描调度靠它把你的写入排除在自激循环之外——不带的话，你的每次写入都可能触发一轮全量扫描，严重时会吃满一个 CPU 核（这是真实踩过的 bug，见 issue #1960）。

通过 `register*` 接口创建的节点，Codex++ 会自动加上这个属性，你不用管。只有你自己直接往页面插节点时才需要手动加。

---

## 样式

配色走 CSS 变量，直接复用即可自动适配深浅色主题：

```css
.my-ext-panel {
  color: var(--codex-plus-text);
  background: var(--codex-plus-bg-elevated);
  border: 1px solid var(--codex-plus-border);
}
```

自己注入 `<style>` 时，请给节点带唯一 id 前缀（例如 `codex-plus-ext-<你的脚本名>`），避免与内置样式互相覆盖。

复用内置的行样式可以让拓展页面看起来与 Codex++ 一致：

```html
<div class="codex-plus-row">
  <div>
    <div class="codex-plus-row-title">标题</div>
    <div class="codex-plus-row-description">说明文字</div>
  </div>
</div>
```

---

## 限制

理解这些能省下大量排查时间：

1. **这不是安全边界。** 脚本跑在 Codex 页面的主世界，和 Codex 自身的 JS 同权。你可以直接调 `window.__codexSessionDeleteBridge` 绕过路由白名单，也可以改任何 DOM。白名单和配额的设计目的是**防误用，不是防恶意**——既然你已经在写脚本，请遵守约定。
2. **每个脚本最多注册 16 项，全局最多 64 项**，超出时 `register*` 会抛错。
3. **`render` 里抛错不会白屏**。Codex++ 会捕获并把错误写进该脚本的状态（管理页可见），页面上显示一块错误占位。
4. **回调抛错不会冒泡到点击处理**。所有你提供的回调都经过失败隔离包装，抛错会被记录并上报，但不会影响 Codex++ 自身 UI 的可用性。
5. **图标仅支持单个字形或 SVG 字符串**，不接受任意 HTML（注入任意 HTML 会让拓展有机会破坏内置 UI 结构）。

---

## 排查

| 手段 | 用途 |
| --- | --- |
| 管理页「拓展」页 | 每个脚本的加载状态与错误 |
| `window.__codexPlusExtensionFailures` | 最近 100 条拓展失败记录（含脚本归属与调用栈） |
| `window.__codexPlusRegistryLog` | 注册 / 注销流水，最近 200 条 |
| `window.codexPlus.script.key` | 确认自己在哪个脚本上下文里 |

常见现象与原因：

- **脚本显示 `loaded` 但行为异常** → 多半是异步错误被吞了，补上 `codexPlus.fail`
- **`register*` 抛「最多注册 16 项」** → 该脚本注册数量超限，先用 dispose 注销不再需要的项
- **`call` 报「未开放的路由」** → 该路由不在白名单，见[后端调用](#后端调用与路由白名单)
- **`call` 报「Codex 页面尚未连接」** → 桥接还没建立，稍后重试或提示用户刷新页面
- **注册的 UI 不见了** → 宿主被重建了。注册表仍在，下次渲染会自动带上；如果你自己 `appendChild` 的节点，需要自己重挂

---

## 尚未开放的接口

以下能力**当前不可用**，写在文档里是为了避免你去找一个不存在的接口：

| 能力 | 状态 |
| --- | --- |
| Codex 原生 DOM 的操作接口 | 不打算开放。那是脚本自己打补丁的事，Codex++ 不对其稳定性作任何承诺 |
| 写类后端路由（`/settings/set`、`/delete` 等） | 未开放，需逐个评估后再决定 |

---

## 附：完整可运行示例

```js
/// <reference types="./codex-plus-extensions" />

// 会话行：把当前会话导出为 Markdown
window.codexPlus.registerRowAction({
  label: "导出 Markdown",
  icon: "⇩",
  onActivate: async ({ session_id, close }) => {
    close();
    try {
      const result = await window.codexPlus.call("/export-markdown", { session_id });
      window.codexPlus.toast(`已导出：${result.path || "完成"}`, { type: "success" });
    } catch (error) {
      window.codexPlus.toast(`导出失败：${error.message}`, { type: "error" });
      window.codexPlus.fail(error);
    }
  },
});

// 整页视图：列出所有已安装拓展
window.codexPlus.registerPage(
  {
    title: "拓展清单",
    render: async ({ container }) => {
      container.innerHTML = `<div class="codex-plus-row">正在加载…</div>`;
      try {
        const result = await window.codexPlus.call("/user-scripts/list");
        const scripts = result.scripts || [];
        container.innerHTML = scripts.length
          ? scripts.map((s) => `
              <div class="codex-plus-row">
                <div>
                  <div class="codex-plus-row-title">${s.name}</div>
                  <div class="codex-plus-row-description">${s.key} · ${s.enabled ? "已启用" : "已停用"}</div>
                </div>
              </div>`).join("")
          : `<div class="codex-plus-row">没有已安装的拓展。</div>`;
      } catch (error) {
        container.innerHTML = `<div class="codex-plus-row">加载失败：${error.message}</div>`;
      }
    },
  },
  { navLabel: "拓展清单", icon: "≡" },
);

// 菜单项：一个开关
let autoExport = false;
window.codexPlus.registerMenuItem({
  label: "自动导出",
  description: "开启后每次回答结束自动导出",
  toggleValue: () => autoExport,
  onChange: (next) => { autoExport = next; },
});
```
