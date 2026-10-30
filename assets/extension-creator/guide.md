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
| `ctx.on("codex/turn/completed", (params) => …)` | Codex 会话事件：`codex/thread/started`、`codex/turn/started`、`codex/turn/completed`、`codex/item/started`、`codex/item/completed`、`codex/item/agentMessage/delta`、`codex/error`，params 是 Codex 通知原样参数 |
| `ctx.intercept("codex/request", ({ method, params }, next) => next({ method, params: 新params }))` | 改写发往 Codex 的请求（例如 `turn/start` 时给用户输入加前缀）。必须同步返回，只能改 params；不改就原样 `next(value)` |
| `ctx.storage.get(key, 默认值) / set(key, value) / remove(key)` | 按拓展隔离的本地存储，值按 JSON 存取 |
| `ctx.registerNavEntry({ label, icon, onActivate })` | 只要一个图标栏按钮、不要整页时用 |
| `ctx.effect(() => cleanupFn)` | 任意副作用（MutationObserver、插入的 DOM、patch 的函数），返回撤销函数 |
| `ctx.listen(target, type, handler, options)` | 自动撤销的事件监听 |
| `ctx.setInterval / ctx.setTimeout` | 自动清理的定时器，返回值是撤销函数 |
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
- 查不到元素时可能是它还没出现：告诉用户需要的状态（例如「先在输入框里打几个字」），再查一次。

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
