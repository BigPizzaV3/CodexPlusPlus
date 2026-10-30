# Codex++ 拓展创作指南（给 agent 看）

你在帮用户做一个 Codex++ 拓展。用户可能说「插件」，但这里做的是 Codex++ 拓展，不是 Codex 自带的插件
（Codex 插件是 skills / MCP 包，由 Codex 管理，不归这些工具管）；回复用户时称它为「拓展」。
拓展是跑在 Codex 桌面 App 页面里的 JavaScript，能给 Codex 加页面、菜单、
会话行操作，或者改造页面行为。完成后拓展立刻热重载生效，用户不用重启。

## 工作流程

1. 先弄清用户想要什么：出现在哪里（图标栏整页 / Codex++ 菜单开关或按钮 / 会话行「更多」菜单 / 直接改页面）、
   点了做什么、要不要记住设置。需求含糊时先问，别猜。
2. 要改 Codex 原生界面时（换图标、改颜色、隐藏或挪动按钮、在输入框旁加东西），先用
   `codexpp_extension_inspect` 查页面结构，不要凭印象猜选择器。见下文「改 Codex 原生界面」。
3. 用 `codexpp_extension_write` 写入：给一个简短的英文 `id`（字母数字 `-` `_`），中文 `name` 与 `description`，
   代码放在 `files` 里（默认入口 `renderer.js`）。`codexpp.json` 由工具生成，不要自己写。
4. 看返回结果里的 `status` / `error`：
   - `loaded` 且没有 `error`：改了界面的话用 `codexpp_extension_screenshot` 截图确认真的生效（选择器没命中
     时拓展不会报错，只是没效果），再告诉用户去哪里看（例如「左侧图标栏新出现的 X 入口」）。
   - `failed` 或有 `error`：读错误、改代码、再写一次。同一个 `id` 重复写入会覆盖。
   - `reload.reloaded = false`：按 `reload.reason` 告诉用户需要做什么（通常是在 Codex++ 管理器里点「热重载」）。
5. 改已有拓展前先 `codexpp_extension_read` 读回现有代码，在它的基础上改。
6. 不提供删除：用户不想要了就 `codexpp_extension_set_enabled` 停用，并告诉用户拓展目录在哪里、可以手动删除。

## 代码骨架

入口文件里调用一次 `codexPlus.extension`，**在顶层同步调用**（不要放进 `await` 或定时器之后）：

```js
codexPlus.extension((ctx) => {
  // 所有通过 ctx 做的注册，在拓展停用 / 热重载时都会自动撤销
});
```

`ctx` 能力：

| 能力 | 说明 |
| --- | --- |
| `ctx.registerPage({ title, render }, { navLabel, icon })` | 整页视图并自动配图标栏入口。`render({ container, close })` **每次打开都会重新调用**，每次都从空白重建 `container` 的内容，不要缓存 DOM |
| `ctx.registerMenuItem(def)` | Codex++ 菜单「主页」面板的一行。开关形态：`{ label, description, toggleValue: () => bool, onChange: (next) => {} }`；按钮形态：`{ label, description, buttonLabel, onActivate: ({ close }) => {} }` |
| `ctx.registerRowAction({ label, icon, onActivate })` | 会话行「更多操作」里的一项，`onActivate({ row, session_id, close })` |
| `ctx.registerComposerAction({ label, icon, onActivate })` | 输入区（发送按钮那一行）的图标按钮，`onActivate({ editor, insertText, close })`，`insertText(text)` 往输入框插字。比自己往输入区塞节点稳，Codex 改结构由 Codex++ 适配 |
| `ctx.registerMessageAction({ label, icon, roles, visible(message), onActivate({ message }) })` | 每条对话消息操作行（复制、编辑按钮旁）的图标按钮。`message = { role, id, turnId, conversationId, text, element }`，`roles` 取 `"user"` / `"assistant"` |
| `ctx.registerMessageDecoration({ roles, render({ message, container }) })` | 消息正文下方的一块内容，往 `container` 里画；流式输出时会清空重画。要给消息加标记、摘要、翻译用这个，不要自己往消息里塞节点 |
| `ctx.registerSidePanel({ title, icon, width, render({ container, close }) })` | 右侧面板，顶栏右侧有开关，打开时主区域自动让出宽度；返回值带 `open()` / `close()`，可配合斜杠命令打开 |
| `ctx.registerCommand({ name, description, run(args, { editor, insertText, setText, toast }) })` | 斜杠命令：用户在输入框输入 `/name 参数` 回车时调用，输入框随后清空；输入 `/` 时有提示、Tab 补全。只拦截已注册的拓展命令，Codex 自己的命令不受影响 |
| `ctx.on("codex/turn/completed", (params) => …)` | Codex 会话事件：`codex/thread/started`、`codex/turn/started`、`codex/turn/completed`、`codex/item/started`、`codex/item/completed`、`codex/item/agentMessage/delta`、`codex/error`，params 是 Codex 通知原样参数 |
| `ctx.intercept("codex/request", ({ method, params }, next) => next({ method, params: 新params }))` | 改写发往 Codex 的请求（例如 `turn/start` 时给用户输入加前缀）。必须同步返回，只能改 params；不改就原样 `next(value)` |
| `ctx.storage.get(key, 默认值) / set(key, value) / remove(key)` | 按拓展隔离的本地存储，值按 JSON 存取 |
| `ctx.registerNavEntry({ label, icon, onActivate })` | 只要一个图标栏按钮、不要整页时用 |
| `ctx.effect(() => cleanupFn)` | 任意副作用（MutationObserver、插入的 DOM、patch 的函数），返回撤销函数 |
| `ctx.listen(target, type, handler, options)` | 自动撤销的事件监听 |
| `ctx.setInterval / ctx.setTimeout` | 自动清理的定时器。**返回值是撤销函数（调用它即取消），不是定时器 id**，不能交给 `clearTimeout` |
| `ctx.asset("assets/pet.png")` | 包内资源（图片、音频、字体）的 data URL，直接给 `img.src` / CSS `url()`；写法见「图片、音频等资源」 |
| `ctx.on(event, fn) / ctx.emit(event, ...args)` | 拓展之间的事件 |
| `ctx.intercept(name, (value, next) => next(value), { order }) / ctx.waterfall(name, value)` | 中间件链 |
| `ctx.call(route, payload)` | 调 Codex++ 后端，只能调写进 `routes` 参数且在可用列表里的路由 |
| `ctx.toast(message, { type })` | 提示，type 为 info / success / warn / error |
| `ctx.fail(error)` | 上报 `await` 之后的异步错误，否则管理页看不到 |

`icon` 只接受单个字符（如 `"⏱"`）或 SVG 字符串。每个拓展最多注册 16 项。

## 改 Codex 原生界面

拓展和 Codex 页面同权，原生界面的任何部分都能改，但 Codex 的页面结构没有稳定性承诺，按下面做才不容易失效：

- **先查再写**：`codexpp_extension_inspect` 不带参数给出输入框、输入区按钮、图标栏、顶栏的概览；`query`
  按 aria-label / 文字找（例如「发送」「语音」），`selector` 验证你写的选择器能命中几个元素。
- **只用稳定选择器**：用返回结果里的 `selector`（由 aria-label、role、data-* 拼成）。`hashedClasses` 里
  `_Xxx_1abcd_2` 这种类名带构建哈希，Codex 一发版就变，绝不要用。
- **外观改动优先纯 CSS**：注入一个 `<style>` 隐藏原图标、用 `::before` / `::after` 放新内容、改颜色尺寸。
  不动 Codex 的节点，React 重绘也不会冲掉，停用拓展时移除 `<style>` 即恢复原样。
- **要加节点时**：插在目标旁边而不是替换它（替换会断掉 Codex 绑在节点上的事件），节点带 `data-codex-plus-ext`；
  目标可能晚出现或被重建（例如发送按钮只在输入框有文字时出现），用 `ctx.effect` 里的 `MutationObserver`
  监听并在撤销函数里 `disconnect()`。
- **找不到目标就什么都不做**，不要抛错卡住；改完用 `codexpp_extension_screenshot` 截图确认。
- 查不到元素时可能是它还没出现：告诉用户需要的状态（例如「先打开设置页」），再查一次。
- **输入区右下角的主按钮是一个会变的槽位**：发送（有字、空闲）/ 排队（有字、运行中）/ 停止（无字、运行中）/
  开启语音聊天（无字、空闲），`aria-label` 跟着变。你调用工具时 Codex 正在运行这一轮，**永远看不到空闲时的
  发送按钮**，不要让用户「打几个字再告诉我」。用 `inspect` 概览里的 `composerPrimaryAction.selector`
  （`button.bg-composer-primary`）选这个槽位，再按需要用 `:not([aria-label="停止"])` 之类排除状态；
  界面是英文时 `aria-label` 是英文。
- 不要用 Computer Use / 截屏控制去操作 Codex 自己的窗口（会被拒绝，也会打断用户），看页面一律用
  `codexpp_extension_inspect` 和 `codexpp_extension_screenshot`。

示例：把语音按钮的图标换成狗头（选择器以 inspect 实际查到的为准）：

```js
codexPlus.extension((ctx) => {
  ctx.effect(() => {
    const style = document.createElement("style");
    style.id = "codex-plus-ext-doge-voice-style";
    style.setAttribute("data-codex-plus-ext", "doge-voice");
    style.textContent = `
      button[aria-label="开始新的语音聊天"] svg { display: none; }
      button[aria-label="开始新的语音聊天"]::after { content: "🐶"; font-size: 18px; line-height: 1; }
    `;
    document.head.append(style);
    return () => style.remove();
  });
});
```

## 图片、音频等资源

桌宠图片、提示音、字体这类二进制文件放在包的 `assets/` 下，用 `codexpp_extension_write` 的 `files` 里写
`sourcePath`（本机文件的绝对路径，按字节复制），不要把 base64 粘进代码：

```json
{ "path": "assets/pet.png", "sourcePath": "/Users/me/Downloads/pet-transparent.png" }
```

```js
const img = document.createElement("img");
img.src = ctx.asset("assets/pet.png");   // data URL，页面里直接可用
```

- 支持 png / jpg / gif / webp / svg / ico / avif / mp3 / wav / ogg / m4a / woff / woff2 / ttf / otf；单个最大 1 MiB，
  一个包合计最大 4 MiB（资源会随拓展下发给页面，越小越好，图片先缩到实际显示尺寸的 2 倍以内）。
- 页面里不能用 `file://` 路径加载本机文件；也不要在终端里把图片 base64 打印出来再复制进代码——对话里的长
  base64 会被替换成占位符，写进代码就坏了。
- 文本文件（代码、JSON、CSS）照旧用 `content`。

## 可配置项

用户以后可能想调的数值、开关、选项，不要写死在代码里，在 `codexpp_extension_write` 的 `config` 参数里声明：

```json
{ "minutes": { "type": "number", "title": "专注时长（分钟）", "default": 25 },
  "theme": { "type": "select", "title": "配色", "options": [{ "value": "red", "label": "番茄红" }, { "value": "green", "label": "薄荷绿" }] } }
```

- `type` 可选 `string`、`number`、`boolean`、`select`（必须给 `options`）；`secret: true` 时界面按密码框显示。
- 页面代码读 `ctx.config.minutes`；钩子、网关中间件、工具脚本读第二个参数的 `context.config`。都是只读快照。
- 用户在 Codex++ 管理器的拓展列表里点「设置」修改；对话里用户说「改成 50 分钟」时，用
  `codexpp_extension_configure` 改值即可，不必重写代码。改完会自动热重载。

## 给模型加工具

要让 Codex 的模型能**调用**一个新能力（查资料、算东西、调本地服务），在 `codexpp_extension_write` 的 `tools`
参数里声明，脚本放在 `files` 里：

```json
[{ "name": "word_count", "description": "统计一段文字的字数", "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] }, "script": "tools/word-count.mjs", "timeout": 30 }]
```

- 脚本默认导出 `(args, { extension, tool }) => 结果`，可以 async。返回字符串作为文本；返回 `{ content: [...] }`
  按 MCP 内容块原样返回；其它值转成 JSON 文本；抛错时模型会看到错误。`timeout` 1~300 秒，默认 30。
- 工具名只能是字母、数字、`_`、`-`；对外名字是 `<拓展 id>__<工具名>`，由已接入的 Codex++ 拓展 MCP 提供。
  写入后告诉用户：**新开一个对话**工具才会出现。
- 工具脚本以用户权限运行，只做描述里写的事。

## Codex 钩子（拦截工具调用、给提示词补上下文）

页面拓展管不到 Codex 的 agent 循环；要在**工具执行前后、用户提交提示词时、本轮结束前**插手，用钩子。
在 `codexpp_extension_write` 的 `hooks` 参数里声明，脚本放在 `files` 里：

```json
{ "PreToolUse": [{ "matcher": "^Bash$", "script": "hooks/guard.mjs", "timeout": 30 }] }
```

- 可用事件：`SessionStart`、`UserPromptSubmit`、`PreToolUse`、`PermissionRequest`、`PostToolUse`、`PreCompact`、
  `PostCompact`、`SubagentStart`、`SubagentStop`、`Stop`。`matcher` 是正则，按工具名（PreToolUse / PostToolUse /
  PermissionRequest）、`source`（SessionStart）等匹配，省略表示全部。`timeout` 1~100 秒，默认 30。
- 脚本是 Node ESM 模块，默认导出 `(payload, { event, extension }) => 输出`，可以是 async。`payload` 是 Codex 传来的
  原样数据（`tool_name`、`tool_input`、`prompt`、`cwd` 等，下划线命名）。返回 `null` 表示没意见。
- 输出用 Codex 的钩子格式：
  - 拦截工具：`{ hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "deny", permissionDecisionReason: "原因" } }`
  - 改写工具参数：`permissionDecision: "allow"` 加 `updatedInput`（Bash 必须带字符串 `command`）
  - 给模型补上下文：`hookSpecificOutput.additionalContext: "…"`（UserPromptSubmit、SessionStart、PostToolUse 等）
  - 让本轮别停：Stop 事件返回 `{ decision: "block", reason: "还要做什么" }`
- 多个拓展的结果由 Codex++ 合并：任一拒绝即拒绝，上下文按拓展顺序拼接。
- 首次用到某个事件时，Codex++ 会往 Codex 配置写一条「Codex++ 拓展钩子」分发条目，**用户要在 Codex 的 Hooks
  设置里信任它一次**（写入结果的 `hooks.note` 会提示），之后新开对话生效。务必把这一步告诉用户。
- 钩子脚本在用户电脑上以用户权限运行，只做用户要求的事。

## 网关请求中间件（改写发给模型的请求）

要在**每次请求模型前**改请求体（往 `instructions` 追加要求、过滤或改写 `input`、调 `reasoning`），在
`codexpp_extension_write` 的 `gateway` 参数里声明：

```json
{ "request": [{ "script": "gateway/request.mjs", "timeout": 5 }] }
```

- 脚本默认导出 `(body, { path, extension }) => 新 body | null`，可以 async。`body` 是 OpenAI Responses 请求体
  （`model`、`instructions`、`input`、`tools`、`reasoning` 等）的副本，返回新对象即替换，返回 `null` 不改。
  `timeout` 1~10 秒，默认 5。多个拓展按顺序串联。
- 只对经 Codex++ 本地代理 / 网关发出的请求生效（中转、聚合、网关模式）；官方直连不经过这里，要告诉用户。
- 它挡在每一次模型请求前面：保持快、只做用户要求的事、不要外发请求内容。出错或超时会保留原请求。

## 网关响应流中间件（改写模型的回复）

要在模型的回复**流回 Codex 的途中**改写（替换敏感词、给回复加尾注、拦掉某类事件），在 `gateway` 参数里声明
`response`：

```json
{ "response": [{ "script": "gateway/response.mjs", "timeout": 5 }] }
```

```js
// gateway/response.mjs：默认导出 (context, next) => 异步可迭代对象
export default async function* (context, next) {
  for await (const event of next()) {
    if (event.type === "response.output_text.delta") yield { ...event, delta: event.delta.replaceAll("机密", "**") };
    else yield event;
  }
}
```

- `next()` 给出上游的事件流（Codex 实际收到的 Responses SSE 事件，带 `type`：`response.created`、
  `response.output_text.delta`、`response.output_item.done`、`response.completed` 等）；返回的流就是交给 Codex 的。
  可以改写、丢弃、插入事件，也可以不调用 `next()` 自己给出整条流；`next()` 只能调用一次。
- 改了文字增量（`delta`）时，`response.output_item.done` / `response.completed` 里带的完整文本要一并改，
  否则 Codex 最终显示的仍是原文。
- `context` 带 `path`、`extension`、`config` 和请求摘要 `request`（`model`、`instructions`、`tools` 工具名、
  `lastUserText` 用户最后一句话等）。
- 多个拓展按顺序嵌套，先声明的在最外层。出错或超时（`timeout` 1~10 秒，按单个事件计）时，这次回复剩下的部分
  原样放行。和请求中间件一样，只对经 Codex++ 本地代理 / 网关的请求生效。

## 读会话历史（只读）

页面、常驻后台、模型工具都能只读查询 Codex 的会话：

- 页面：`codexPlus.extension({ inject: ["sessionQuery"], apply(ctx) { ctx.sessionQuery.listSessions() } })`
- 后台：默认导出的参数里有 `sessionQuery`；工具：第二个参数里有 `sessionQuery`

方法（都返回 Promise）：`listSessions({ archived, limit, offset })`、`filterSessions({ id, cwd, createdAfter, createdBefore })`、
`readSession(id)` → `{ session, events }`、`readTitle(id)`、`listEvents(id, { types, offset, limit, fromEnd })`、
`filterEvents(id, { types, text })`、`readEvent({ sessionId, seq, before, after })`、
`searchSessions({ query, limit, cursor })` → `{ items, nextCursor }`、`searchEvents({ query, sessionId, types, limit, cursor })`。

- 会话：`{ id, title, cwd, model, createdAt, updatedAt, archived, preview, gitBranch }`。
- 事件：`{ type, seq, time, data }`，`type` 为 `turn/start`、`user/message`、`assistant/message`、`assistant/reasoning`、
  `tool/call`、`tool/result`；`data` 里有 `turn`、`text` / `name` / `input` / `output` 等。
- 当前页面所在会话的 id：消息插槽回调里的 `message.conversationId`。

## 拓展之间共享能力（服务）

一个拓展用 `ctx.provide("名字", 对象)` 提供服务，别的拓展声明 `inject` 后使用：

```js
// 提供方
codexPlus.extension((ctx) => { ctx.provide("wordCounter", { count: (text) => text.length }); });
// 使用方：依赖齐全才应用；提供方卸载时跟着卸载，回来后自动重新应用
codexPlus.extension({ inject: ["wordCounter"], apply(ctx) { ctx.toast(String(ctx.wordCounter.count("你好"))); } });
```

- 服务名是全局扁平的，重名会报错；可选依赖用 `ctx.get("名字")`（不存在时为 undefined）。
- `ctx.inject(["名字"], (sub) => { ... })` 开一个子作用域，只让依赖那个服务的部分挂起。
- 内置服务：`sessionQuery`（只读会话查询）、`locale`（多语言）。

## 多语言

- 包内放 `locale/en.json`、`locale/zh.json`……：`{ "meta": { "title": "名称", "description": "描述" }, "hi": "你好 {name}" }`。
  管理器按界面语言显示 `meta` 里的名称与描述；其余键是拓展自己的词条。
- 页面里 `ctx.locale.t("hi", { name })` 按当前语言取词，回退顺序：当前语言 → 主语言 → en → key 本身。
  也可以 `ctx.locale.register("命名空间", { zh: {...}, en: {...} })` 再 `ctx.locale.bind("命名空间")`。
- `ctx.locale.current` 是当前语言（Codex 页面的语言），切换时会收到 `ctx.on("locale/change", ...)`。

## 常驻后台（页面调本地能力、保持状态、推事件）

页面脚本跑在 Codex 页面里，不能读写本地文件、起进程、长时间计时。要这些能力就声明一个常驻后台，
在 `codexpp_extension_write` 的 `backend` 参数里写 `{ "script": "backend/main.mjs" }`，脚本放在 `files` 里：

```js
// backend/main.mjs：默认导出 (ctx) => 方法表，可以 async；ctx = { extension, config, dataDir, emit, log, sessionQuery }
import fs from "node:fs";
export default ({ emit }) => {
  let count = 0;
  setInterval(() => emit("tick", { count: ++count }), 60_000);
  return {
    async readNote() { return fs.existsSync("note.txt") ? fs.readFileSync("note.txt", "utf8") : ""; },
    async saveNote(text) { fs.writeFileSync("note.txt", String(text)); return true; },
  };
};
```

```js
// 页面侧
const text = await ctx.backend.call("readNote");
ctx.backend.on("tick", ({ count }) => ctx.toast(`第 ${count} 分钟`));
```

- 第一次 `call` 时 Codex++ 用 Node 启动后台，之后常驻；页面刷新、拓展热重载 / 启停时全部停掉，下次调用再起。
  所以后台里的内存状态会丢，要持久就写文件。
- 工作目录是拓展专属的数据目录（`ctx.dataDir`），相对路径都落在这里。
- 方法只收一个参数（JSON），返回值要能 JSON 序列化；抛错时页面的 `call` 会 reject。默认 20 秒超时，
  `call(method, params, { timeout })` 最多 24 秒，耗时更久的活儿先返回、做完再 `emit`。
- `emit(event, payload)` 推给页面的 `ctx.backend.on(event, handler)`（`"*"` 收全部）；`on` 不会启动后台。
- 后台以用户权限运行、没有沙箱，只做用户要求的事。

## 工具连接断开时

`codexpp_extension_*` 工具报 `Transport closed` 说明拓展服务断开了（通常是 Codex++ 刚更新或重启过），这个对话
里不会自动恢复：告诉用户**新开一个对话**继续，拓展文件已经写好的不会丢。不要自己在终端里拉起 Codex++ 进程模拟
工具调用。

## 必须遵守

- **自己插入到 Codex 页面里的节点必须带 `data-codex-plus-ext` 属性**（值写拓展 id），否则会触发 Codex++ 的
  扫描自喂循环、吃满 CPU。用 `ctx.effect` 插入，并在返回的撤销函数里移除。
- 自己加的 `<style>` 用唯一 id（如 `codex-plus-ext-<拓展id>-style`），同样带 `data-codex-plus-ext`，撤销时移除。
- 颜色用 CSS 变量适配深浅色：`var(--codex-plus-text)`、`var(--codex-plus-bg-elevated)`、`var(--codex-plus-border)`。
  页面里的行可以复用 `<div class="codex-plus-row"><div><div class="codex-plus-row-title">标题</div>
  <div class="codex-plus-row-description">说明</div></div></div>`。
- **`render` 里创建的定时器、监听要随页面关闭撤销**：`render` 每打开一次页面就调用一次，`ctx.setInterval`
  只在拓展停用时才清理。所以在 `render` 里用它时，回调里先判断 `container.isConnected`，不在页面上就调用它
  返回的撤销函数（见下面示例），否则每开一次页面就多一个空转的定时器。
- 需要记住用户设置就用 `ctx.storage`（自动按拓展隔离）。
- Codex 自己的 DOM 结构随版本变化，没有稳定性承诺。依赖它时要容错（找不到元素就静默跳过），并用
  `MutationObserver` 等待元素出现，而不是假设它已经在。
- 不要读取或外发用户的密钥、配置、会话内容，除非用户明确要求且清楚后果。不要请求任意网络地址，除非这就是用户要的功能。

## 示例：图标栏番茄钟

```js
codexPlus.extension((ctx) => {
  const key = "codexpp-pomodoro:endsAt";
  const left = () => Math.max(0, Number(localStorage.getItem(key) || 0) - Date.now());
  ctx.registerPage(
    {
      title: "番茄钟",
      render: ({ container }) => {
        container.innerHTML = "";
        const row = document.createElement("div");
        row.className = "codex-plus-row";
        const text = document.createElement("div");
        text.className = "codex-plus-row-title";
        const button = document.createElement("button");
        button.textContent = "开始 25 分钟";
        button.onclick = () => {
          localStorage.setItem(key, String(Date.now() + 25 * 60 * 1000));
          ctx.toast("番茄钟开始", { type: "success" });
        };
        row.append(text, button);
        container.append(row);
        const paint = () => {
          const seconds = Math.ceil(left() / 1000);
          text.textContent = seconds ? `剩余 ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}` : "未开始";
        };
        paint();
        // render 每次打开都会调用：页面关掉（container 不在文档里）就撤销这个定时器
        const stop = ctx.setInterval(() => (container.isConnected ? paint() : stop()), 1000);
      },
    },
    { navLabel: "番茄钟", icon: "⏱" },
  );
});
```
