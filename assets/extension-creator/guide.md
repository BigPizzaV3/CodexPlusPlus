# Codex++ 拓展创作指南（给 agent 看）

你在帮用户做一个 Codex++ 拓展。用户可能说「插件」，但这里做的是 Codex++ 拓展，不是 Codex 自带的插件
（Codex 插件是 skills / MCP 包，由 Codex 管理，不归这些工具管）；回复用户时称它为「拓展」。
拓展是跑在 Codex 桌面 App 页面里的 JavaScript，能给 Codex 加页面、菜单、
会话行操作，或者改造页面行为。完成后拓展立刻热重载生效，用户不用重启。

## 同一对话持续修改

- **同一个对话后续默认继续修改原来的拓展，保留已有功能；只有用户明确说新建另一个，才创建新的。**
  “再加个功能”“顺便支持……”都属于继续修改，不能据此换 id 或另建拓展。
- 工具根据 Codex 在本次调用里传来的 `threadId` 保存关联，重连后仍记得；不同对话各自独立。
  `guide`、`list`、`read`、`write`、`configure` 返回的 `conversation.extensionId` 就是本对话当前目标，
  为 null 表示尚未关联。不要拿全局最近修改的拓展、页面当前显示的对话或其他对话的目标来推测。
- 首次创建时，`write` 提供未占用的 `id`、`name` 和 `files`，成功后自动关联。
  继续修改时先 `read({})` 读回原代码，再 `write({ files: [...] })`；也可以明确填写同一个 id。
  写入文件仍是完整内容，必须在原代码上添加功能，不能只写新增部分导致旧功能丢失。
- 更新时省略的名称、版本、配置、入口、权限路由、钩子等清单声明会保留；显式提供的字段会替换原值。
  要新增入口或列表项时先读原清单，再传包含旧项和新项的完整列表。没有提供的包内文件也会保留。
- 只有用户明确要求另建一个拓展时，才传 `newExtension: true`，同时给未占用的 `id`、`name` 和 `files`；
  成功后当前对话关联到新拓展，旧拓展保留。出错时先修复原因，不能擅自把 `newExtension` 改成 true。
- 用户明确指定编辑某个已有拓展（包括新开对话继续编辑）时，调用 `read({ id: "原拓展", select: true })`
  读回并选中它。普通 `read({ id: "其他拓展" })` 只读参考，不改变当前关联。
- 缺少可信 `threadId` 时仍可读取指定 id，但不能写入或建立关联；不要编造 ID 或绕过工具直接写拓展目录。
  告诉用户当前客户端未传递对话身份，需要使用支持此能力的 Codex 客户端重新连接拓展工具。

## 工作流程

1. 先弄清用户想要什么：出现在哪里（图标栏整页 / Codex++ 菜单开关或按钮 / 会话行「更多」菜单 / 直接改页面）、
   点了做什么、要不要记住设置。需求含糊时先问，别猜。
2. 要改 Codex 原生界面时（换图标、改颜色、隐藏或挪动按钮、在输入框旁加东西），先用
   `codexpp_extension_inspect` 查页面结构，不要凭印象猜选择器。见下文「改 Codex 原生界面」。
3. 用 `codexpp_extension_write` 写入：首次创建给一个简短的英文 `id`（字母数字 `-` `_`），中文 `name` 与 `description`；
   后续默认使用本对话的关联继续修改。代码放在 `files` 里（默认入口 `renderer.js`）。`codexpp.json` 由工具生成，不要自己写。
4. 看返回结果里的 `status` / `error`：
   - `reload.reloaded = null`：重载请求超时、执行结果尚未确认。先检查实际效果，不要自动重复重载。
   - `loaded` 且没有 `error`：改了界面的话用 `codexpp_extension_screenshot` 截图确认真的生效（选择器没命中
     时拓展不会报错，只是没效果），再告诉用户去哪里看（例如「左侧图标栏新出现的 X 入口」）。
   - `failed` 或有 `error`：读错误、改代码、在同一个关联拓展上再写一次，不要因报错另建一个。
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
| `ctx.registerRowAction({ label, icon, onActivate })` | 会话行「更多操作」里的一项，`onActivate({ row, session_id, title, archived, resolve, close })`。加 `placement: "inline"` 画成行上常驻的图标按钮（套原生按钮外观）；加 `surfaces: ["sidebar", "archived"]` 同时出现在归档页的行上（归档页的行未必有会话 id，用 `await resolve()` 取）。Codex++ 自带的「删除」「导出」就是这样注册的 |
| `ctx.registerComposerAction({ label, icon, onActivate })` | 输入区（发送按钮那一行）的图标按钮，`onActivate({ editor, insertText, close })`，`insertText(text)` 往输入框插字。比自己往输入区塞节点稳，Codex 改结构由 Codex++ 适配 |
| `ctx.registerMessageAction({ label, icon, roles, visible(message), onActivate({ message }) })` | 每条对话消息操作行（复制、编辑按钮旁）的图标按钮。`message = { role, id, turnId, conversationId, text, element }`，`roles` 取 `"user"` / `"assistant"` |
| `ctx.registerMessageDecoration({ roles, render({ message, container }) })` | 消息正文下方的一块内容，往 `container` 里画；流式输出时会清空重画。要给消息加标记、摘要、翻译用这个，不要自己往消息里塞节点 |
| `ctx.registerSlot({ slot, render({ container }) })` | 往固定位置放内容：`composer.above`（输入框上方）、`thread.top` / `thread.bottom`、`sidebar.top` / `sidebar.bottom`、`titlebar`（标题栏右侧）。Codex 重建 DOM 后会重新调 render；返回的句柄带 `refresh()` |
| `ctx.registerSidePanel({ title, icon, width, render({ container, close }) })` | 右侧面板，顶栏右侧有开关，打开时主区域自动让出宽度；返回值带 `open()` / `close()`，可配合斜杠命令打开 |
| `ctx.registerCommand({ name, description, run(args, { editor, insertText, setText, toast }) })` | 斜杠命令：用户在输入框输入 `/name 参数` 回车时调用，输入框随后清空；输入 `/` 时有提示、Tab 补全。只拦截已注册的拓展命令，Codex 自己的命令不受影响 |
| `ctx.on("codex/turn/completed", (params) => …)` | Codex 会话事件：`codex/thread/started`、`codex/turn/started`、`codex/turn/completed`、`codex/item/started`、`codex/item/completed`、`codex/item/agentMessage/delta`、`codex/error`，params 是 Codex 通知原样参数 |
| `ctx.intercept("codex/request", ({ method, params }, next) => next({ method, params: 新params }))` | 改写发往 Codex 的请求（例如 `turn/start` 时给用户输入加前缀）。必须同步返回，只能改 params；不改就原样 `next(value)` |
| `ctx.storage.get(key, 默认值) / set(key, value) / remove(key)` | 按拓展隔离的本地存储，值按 JSON 存取 |
| `ctx.registerNavEntry({ label, icon, onActivate })` | 只要一个图标栏按钮、不要整页时用 |
| `ctx.onScan(() => {…}, { phase })` | Codex 界面变了（切会话、侧栏或输入框重建）时调用，按帧合并、页面隐藏时不跑。要「Codex 重建后把自己的东西补回去」用它，**不用自己开 MutationObserver**。`phase: "light"` 每次变化都跑（回调要很便宜），默认 `"deferred"` 跟完整扫描。返回的撤销函数带 `run()` 立即跑一次 |
| `ctx.on("settings/loaded", ({ reason }) => …)` / `ctx.on("settings/changed", ({ key, value, persisted }) => …)` | 读到 Codex++ 设置（启动、每 5 秒心跳）/ 用户在 Codex 页面里改了一项设置时。要跟着 Codex++ 设置开关走的拓展订阅这两个 |
| `ctx.effect(() => cleanupFn)` | 任意副作用（插入的 DOM、patch 的函数），返回撤销函数 |
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
  目标可能晚出现或被重建（例如发送按钮只在输入框有文字时出现），用 `ctx.onScan` 在每次界面变化后检查、
  缺了就补（先查有没有自己的节点，有就不动），撤销时用 `ctx.effect` 返回的函数把节点移除。
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
- 手写的文本文件（代码、JSON、CSS）照旧用 `content`，单个最大 256 KiB。

## 用了第三方库的界面（打包产物）

要用 npm 上的库（图表、编辑器、React 组件等）时，不要把库的代码或打包结果贴进 `content`：先在本机临时目录里写好源码、
装依赖、用 esbuild 之类打包成**单个文件**，再用 `sourcePath` 把打包产物复制进包：

```bash
npx esbuild src/main.js --bundle --minify --format=iife --outfile=/tmp/my-ext/dist/renderer.js
```

```json
{ "path": "renderer.js", "sourcePath": "/tmp/my-ext/dist/renderer.js" }
```

- 扩展名不是图片 / 音频 / 字体的 `sourcePath` 按 UTF-8 文本复制，单个最大 4 MiB、合计最大 8 MiB；打包时开 `--minify`，
  只打进用到的部分。页面脚本要用 `--format=iife`（入口里照常调用 `codexPlus.extension`）。
- 打包产物会随 Codex 页面注入，越小越好。
- 之后改这个拓展：`codexpp_extension_read` 里大文件的 `content` 是 `null`（只给大小）；没有改动的包内文件会保留，
  不要用 null 或占位符覆盖。改源码时重新打包，再用 `sourcePath` 写回相同路径。

## 可配置项

用户以后可能想调的数值、开关、选项，不要写死在代码里，在 `codexpp_extension_write` 的 `config` 参数里声明：

```json
{ "minutes": { "type": "number", "title": "专注时长（分钟）", "default": 25 },
  "theme": { "type": "select", "title": "配色", "options": [{ "value": "red", "label": "番茄红" }, { "value": "green", "label": "薄荷绿" }] } }
```

- `type` 可选 `string`、`text`（多行）、`number`（可加 `min` / `max`）、`boolean`、`select`（必须给 `options`）、
  `array`（`items` 写元素类型，只能是简单类型）、`object`（`properties` 写子字段，最多 3 层）；`secret: true` 时界面按密码框显示。
- `title` / `description` 可以写 `{ "zh": "…", "en": "…" }`，管理器按界面语言显示。
- 页面代码读 `ctx.config.minutes`；钩子、网关中间件、工具脚本读第二个参数的 `context.config`。都是只读快照。
- 同一个拓展要按几组配置各跑一份（例如「工作」「休息」两个计时器）时，用 `instances` 参数：
  `[{ "id": "work", "name": "工作", "config": { "minutes": 50 } }, { "id": "rest", "name": "休息", "config": { "minutes": 10 } }]`。
  页面脚本按实例各调用一次，`ctx.config` 是叠上实例配置后的值，`ctx.instance` 是 `{ id, name }`，`ctx.storage` 与注册项按实例分开；
  实例最多 16 个，实例的 `config` 只能写 `config` 里声明过的键。钩子、网关、后台不分实例。
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
  `PostCompact`、`SubagentStart`、`SubagentStop`、`Stop`、`PreModelRequest`、`PostModelRequest`。
  后两个在每次请求模型前后触发（官方直连也触发），只能观察、不能拦截或改写，payload 有 `session_id`、`turn_id`、`model`，
  `PostModelRequest` 另有 `status`、`token_usage`；用户要「每一步都记录 / 计数 / 提醒」且不经代理时用它们。`matcher` 是正则，按工具名（PreToolUse / PostToolUse /
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
- 每走一步都会经过：第二个参数的 `step.index` 是本轮第几步（工具调用之后加一），`step.userText` 是本轮用户输入。
  返回 `{ reject: "原因" }` 可以不发这一步（例如限制一轮最多 20 步）。

## 请求失败时重试（gateway.error）

上游返回错误状态（429、5xx 等）时让拓展决定要不要重发，在 `gateway` 参数里声明 `error`：

```js
// gateway/error.mjs：默认导出 (error, { path, extension, config }) => { retry: true, delayMs } | null
export default (error) => (error.status === 429 && error.attempt < 2 ? { retry: true, delayMs: 2000 } : null);
```

- `error` 是 `{ status, body（文本，最多 64 KiB）, json（能解析时）, path, attempt（已重试次数）}`。
- 返回 `{ retry: true, delayMs }`（或 `{ kind: "retry" }`）即重发同一请求；返回 `{ kind: "fallback", model: "备用模型" }` 即换模型重发；
  最多 3 次，间隔最多 10 秒。
- 网关请求中间件想加请求头时写第二个参数的 `headers`（`context.headers["x-trace"] = "1"`），鉴权类头会被忽略。

## 关掉 Codex++ 网关的内置环节（gateway.replaces）

拓展要自己接管某一步时，在 `gateway` 参数里写 `"replaces": ["builtin:…"]`，拓展启用时关掉那个内置环节、停用后恢复：

- 两种模式都经过：`builtin:auto-retry`（请求避震，关掉后上游错误原样交回，重试交给 `gateway.error`）、
  `builtin:foreign-reasoning`（别家加密推理内容的处理）、`builtin:unlisted-model-mapping`（目录外模型名映射）、
  `builtin:local-compaction`（本地压缩）、`builtin:reasoning-collapse`、`builtin:exec-adapt`、`builtin:relay-rotation`（失败换下一个供应商）。
- 网关模式：`builtin:gateway-failover`（候选失败换下一个）、`builtin:key-cooldown`（429 后 Key 冷却）、
  `builtin:key-breaker`（供应商熔断）、`builtin:local-rate-limit`（Key 的 RPM / 并发限速）、`builtin:title-model`（起名请求改发起名模型）。
- 关掉冷却 / 熔断后，在冷却的 Key 照常当候选（排在可用的后面），要避开谁在 `route` 段的中间件里自己筛。
- 也能关请求转换 `builtin:request-conversion`（upstream 段拿到 Responses 形状的请求体，自己转成上游协议；响应仍由 Codex++ 转回）、鉴权头 `builtin:auth-headers`（只有声明它的拓展在 upstream 段拿到 `context.credential.apiKey`，自己往 `context.headers` 写鉴权头）、官方整理 `builtin:official-shaping`、请求日志与用量 `builtin:request-log`（关掉后不记日志、不计用量）。
- 只在用户明确要接管这一步时才关，并告诉用户关掉了什么；关掉鉴权头意味着拓展脚本会拿到 Key，要提醒用户。

## 工具管道（改工具参数、代为执行、改工具结果）

在 `codexpp_extension_write` 的 `toolPipeline` 参数里声明，每项 `{ matcher, script, timeout }`：

```json
{ "preExecute": [{ "matcher": "^Bash$", "script": "tools/pre.mjs" }],
  "execute": [{ "matcher": "^Bash$", "script": "tools/exec.mjs" }],
  "postExecute": [{ "matcher": "^exec_command$", "script": "tools/post.mjs" }] }
```

- `preExecute`：`(input, { name, callId, cwd, payload, config }) => 新 input | { deny: "原因" } | null`。改参数或拦截，
  在 Codex 的 PreToolUse 钩子里跑，`matcher` 匹配钩子里的工具名（如 `Bash`）。
- `execute`：同样的参数，返回结果（字符串或 JSON）就由拓展代为执行，原工具不跑；返回 `undefined` 交还原工具。
  经 Codex++ 代理时模型看到的就是这个结果，不经代理时模型从「拒绝理由」里读到它。
- `postExecute`：`(output, { name, input, callId, config }) => 新 output | undefined`。改写工具结果，`matcher` 匹配
  模型侧的工具名（如 `exec_command`、`apply_patch`、`mcp__服务__工具`）。只对经 Codex++ 代理的请求生效，
  同一条结果只改一次。
- 前两段用到 PreToolUse 钩子：第一次用时要告诉用户在 Codex 的 Hooks 设置里信任「Codex++ 拓展钩子」一次。

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

- 另有 `readEventsSince({ sessionId, seq })`（增量读）、`project(id, { init, apply })`（把事件折叠成状态）、
  `observeSession(id, handler, { intervalMs })`（订阅新事件，返回撤销函数；页面与后台可用）。
- 会话：`{ id, title, cwd, model, createdAt, updatedAt, archived, preview, gitBranch }`。
- 事件：`{ type, seq, time, data }`，`type` 为 `turn/start`、`user/message`、`assistant/message`、`assistant/reasoning`、
  `tool/call`、`tool/result`、`turn/complete`、`turn/aborted`、`token/usage`、`session/compacted`；`data` 里有 `turn`、`text` / `name` / `input` / `output` 等。
- 跟踪所有会话：页面 `ctx.on("session/event", (event, { sessionId }) => …)`、`ctx.on("agent/status", ({ sessionId, status }) => …)`
  （status 为 running / idle / aborted）；后台 `sessionQuery.observeAll(handler, { onStatus })`。`codex/turn/completed` 这类旧事件在新版 Codex 里收不到，不要用。
- 当前页面所在会话的 id：消息插槽回调里的 `message.conversationId`。钩子脚本的第二个参数里也有 `sessionQuery`。

## 写会话（分叉、追加条目、让 agent 跑一轮）

`inject: ["sessions"]` 后（后台、工具、钩子的上下文里也有 `sessions`）：`create("文字")` 新建会话并跑第一轮、`fork(id)` 分叉、
`rename(id, "标题")`、`append(id, "文字" | [{ role, text }])` 追加条目、`startTurn(id, "文字")` 发起一轮、`steer(id, "文字")`
往正在跑的一轮插话（这三个会真的调用模型，只在用户要求时用）、`interrupt(id, turnId)`、`compact(id)`、`list()`。

## 让拓展自己调模型

页面 `ctx.llm`，后台 / 工具 / 钩子上下文里的 `llm`：`await llm.complete({ input: "文字或消息数组", instructions?, model?, reasoningEffort? })`
→ `{ text, model, usage }`；`llm.listModels()` → `{ models, default }`。用当前供应商，会消耗额度，不经过拓展网关中间件。

## 技能、子 agent、MCP 服务器、全局指令、提示词与资源

`codexpp_extension_write` 的参数：
- `skills`：包内含 `SKILL.md` 的目录（省略时自动收 `skills/*/SKILL.md`），装进 `~/.codex/skills`。
- `agents`：`[{ name, description, instructions: "prompts/x.md" 或文字, model?, reasoningEffort? }]`，写成 Codex 的子 agent 角色。
- `mcpServers`：`{ 名字: { command, args, env } | { url } }`，写进 Codex 配置；`${extensionDir}`、`${config.键}` 会被替换。
- `instructions`：文字或包内 .md，合并进全局 AGENTS.md（官方直连也生效）——用户要「改系统提示词」「每次都遵守某规则」时用它。
- `prompts`：`[{ name, description, arguments: [{ name, required }], template: "文字或 .md，含 {{参数}}" }]`；
  `resources`：`[{ name, file: "docs/x.md" }]`。两者经 Codex++ 拓展 MCP 提供。
- 前四类在新开的 Codex 对话里生效，提醒用户新开对话。

## Webhook 与定时任务

- `backend: { "script": "backend/main.mjs", "http": true }` 后，`http://127.0.0.1:<Codex++ 本地代理端口>/ext/<拓展 id>/<路径>`
  交给后台导出的 `http({ method, path, query, headers, body, json })`，返回字符串或 `{ status, json | body }`。
- `schedules: [{ method: "digest", at: "09:00" }, { method: "poll", everyMinutes: 30 }]`：到点调用后台方法，需要声明 `backend`。

## 输入触发、快捷键、主题、问用户

- `ctx.registerInputTrigger({ trigger: "#", items: async (query) => [{ label, description, insert }] })`：输入框里打 # 时弹候选。
  `@`、`$`、`/` 是 Codex 自己的提及与命令，不能用。
- `ctx.registerShortcut({ key: "Mod+Shift+K", run })`：Mod 在 macOS 是 ⌘；必须带修饰键。
- `ctx.registerTheme({ id, name, variables: { "--变量": "值" }, css })`，`ctx.themes.apply(id)` 切换，`apply(null)` 恢复。
- `await ctx.ask({ title, message, options: [{ value, label }], input: { placeholder } })` → `{ value, text }` 或 null；
  后台 / 工具 / 钩子里用 `ui.ask(...)`、`ui.toast("文字")`。
- 新插槽位置：`turn.after`（每轮末尾，render 收到 turnKey）、`sessionRow.prefix` / `sessionRow.suffix`（侧栏会话标题前后，收到 sessionId）、`overlay`（全窗口浮层）、`activity.after`（每个命令 / 工具调用 / 出错条目末尾，收到 kind、itemIds、text）。

## 提供模型供应商

在 `codexpp_extension_write` 的 `providers` 参数里声明 `[{ id, name, protocol: "responses" | "chat" | "anthropic", baseUrl,
apiKey: { "config": "apiKey" }, models: [...] }]`，同时在 `config` 里声明 `apiKey`（string、secret）。并入 Codex++ 网关后，
模型名是 `<id>/<模型>`，要在网关模式下使用。Key 让用户在拓展设置里填，不要写进代码，也不要让用户把 Key 发进对话。
上游有 `/models` 接口时可以写 `discoverModels: true` 自动拉模型列表。

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
- 内置服务：`sessionQuery`（只读会话查询）、`sessions`（会话写入）、`locale`（多语言）、`themes`（主题）。
- 后台也能提供服务，给页面、别的后台、模型工具、钩子共用：在 `backend` 里写 `"provides": ["counter"]`，后台默认导出里
  `provide("counter", { add(n) { ... } })`；别处用 `service("counter").add(1)`（后台、工具、钩子），页面直接
  `inject: ["counter"]`。所有调用都落到同一个后台进程，状态是共享的。

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
// backend/main.mjs：默认导出 (ctx) => 方法表，可以 async；ctx = { extension, config, dataDir, emit, log, sessionQuery, provide, service }
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

## 管理器面板（拓展在 Codex++ 管理器里的界面）

要做统计、批量操作、比设置表单更复杂的配置界面时，在 `codexpp_extension_write` 的 `manager` 参数里写
`{ "page": "manager.html", "title": "面板标题" }`，页面放在 `files` 里。用户在 Codex++ 管理器「本地拓展」里
这一行的「…」菜单点「打开面板」，面板展开在这一行下面。页面里用 `window.codexPlusManager`：

```html
<p>共 <b id="count">…</b> 条</p>
<script>
  const api = window.codexPlusManager;
  api.backend.call("stats").then(({ count }) => { document.getElementById("count").textContent = count; });
  api.onTheme((theme) => document.body.classList.toggle("dark", theme === "dark"));
</script>
```

- `manager` 里再写 `"nav": "management"`（或 `gateway` / `tools`）就进管理器侧栏、整页打开；`"replaces": ["gatewayUsage"]` 顶替管理器的内置页面（上方有「查看内置页面」可切回；「万物皆拓展」「内置拓展」「设置」不能顶替）；`"data": ["requestLogs"]` 后能用 `codexPlusManager.data("requestLogs", { limit })` 读只读数据（`requestFlow` / `requestLogs` / `gatewayUsage` / `localSessions`），`codexPlusManager.navigate("hub")` 跳到别的页面。
- 能用的只有：`config.get()` / `config.set(values)` / `config.onChange(fn)`（本拓展配置，只能写声明过的项）、
  `backend.call(method, params)`（本拓展的常驻后台，要同时声明 `backend`）、`asset(path)`（本拓展 assets/ 的 data URL）、
  `toast(text)`、`theme()` / `onTheme(fn)`。都返回 Promise。
- 页面在沙箱 iframe 里：**不能联网、不能加载外部或包内其它文件**，脚本和样式写在页面里；图片用 `asset()` 取；
  要联网或读写本机文件就放进常驻后台，经 `backend.call` 调。页面最大 1 MiB。
- 没法用 `codexpp_extension_screenshot` 看管理器（它只截 Codex 页面），写好后让用户打开面板看看。

## 顶替 Codex++ 内置功能

Codex++ 自带的功能都是内置拓展（会话删除、Markdown 导出、语音听写、模型分组……）。用户想换掉某个内置功能的
做法时，在 `codexpp_extension_write` 的 `replaces` 参数里写内置拓展 id（如 `["model-picker-groups"]`）：你的拓展启用时
那个内置拓展让位，停用时它回来。内置拓展能做的你都能用同样的 ctx 接口做（`onScan`、`registerSlot`、
`registerRowAction`、`settings/changed`……）。
用量状态栏、请求状态条已改成官方拓展包，`replaces` 里写 `usage-statusline` / `proxy-request-status` 时让位的是对应的官方拓展包；
它们的数据作为服务提供（`inject: ["codexpp.usage"]`、`["codexpp.proxyStatus"]`），也可以直接调白名单路由
`/thread-usage-live`、`/proxy-status`，自己画界面即可。

## 赶在 Codex 之前（preload）与启动前后（launch）

要在 Codex 自己的脚本之前改页面（改写 Codex 模块、强制语言）时，用写入工具的 `preload` 参数：`{ "scripts": ["preload/patch.js"], "reloadOnLaunch": true }`。脚本在每份新文档最早执行，拿不到 `codexPlus`，只能读 `codexPlusPreload`（`{ id, key, config }`）；改了要重启 Codex。
要在启动 Codex 之前 / 退出之后做事（改 Codex 的本地状态文件、恢复补丁）或追加启动参数时，用 `launch` 参数：`{ "args": ["--lang=zh-CN"], "before": "launch/before.mjs", "after": "launch/after.mjs" }`，脚本默认导出 `(context) => …`，拿到 `codexHome`、`stateDir`、`appDir`、`config`。顶替 Rust 侧的内置功能（原生 Fast 解锁、强制中文、Work 视图……）就用这两个。

## 在拓展包里带 dsh 插件

用户手上已经有按 dsh（deepseek-harness）插件写法写的插件（`apply(ctx)` 里 `ctx.tools.register`、`ctx.on("tools/pre-execute", …)`
等），不用改写成拓展：把插件目录（带 `package.json`）作为 `files` 写进包里的一个子目录，`dsh` 参数写这个目录名（如 `"dsh-plugin"`）。
拓展启用时 Codex++ 把它作为 dsh 插件导入运行（工具经 MCP、事件经钩子与网关），停用、删除时一起停用、移除。
写好后用 `codexpp_extension_list` 看加载状态；dsh 插件的逐行兼容性报告在管理器「拓展 → dsh 插件」里。
新写的功能优先直接写成拓展，`dsh` 只用于搬用户已有的 dsh 插件。

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
  `ctx.onScan` 在界面变化后再找，而不是假设它已经在。
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
