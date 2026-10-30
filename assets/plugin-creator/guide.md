# Codex++ 插件创作指南（给 agent 看）

你在帮用户做一个 Codex++ 插件。插件是跑在 Codex 桌面 App 页面里的 JavaScript，能给 Codex 加页面、菜单、
会话行操作，或者改造页面行为。完成后插件立刻热重载生效，用户不用重启。

## 工作流程

1. 先弄清用户想要什么：出现在哪里（图标栏整页 / Codex++ 菜单开关或按钮 / 会话行「更多」菜单 / 直接改页面）、
   点了做什么、要不要记住设置。需求含糊时先问，别猜。
2. 用 `codexpp_plugin_write` 写入：给一个简短的英文 `id`（字母数字 `-` `_`），中文 `name` 与 `description`，
   代码放在 `files` 里（默认入口 `renderer.js`）。`codexpp.json` 由工具生成，不要自己写。
3. 看返回结果里的 `status` / `error`：
   - `loaded` 且没有 `error`：告诉用户去哪里看（例如「左侧图标栏新出现的 X 入口」）。
   - `failed` 或有 `error`：读错误、改代码、再写一次。同一个 `id` 重复写入会覆盖。
   - `reload.reloaded = false`：按 `reload.reason` 告诉用户需要做什么（通常是在 Codex++ 管理器里点「热重载」）。
4. 改已有插件前先 `codexpp_plugin_read` 读回现有代码，在它的基础上改。
5. 不提供删除：用户不想要了就 `codexpp_plugin_set_enabled` 停用，并告诉用户插件目录在哪里、可以手动删除。

## 代码骨架

入口文件里调用一次 `codexPlus.plugin`，**在顶层同步调用**（不要放进 `await` 或定时器之后）：

```js
codexPlus.plugin((ctx) => {
  // 所有通过 ctx 做的注册，在插件停用 / 热重载时都会自动撤销
});
```

`ctx` 能力：

| 能力 | 说明 |
| --- | --- |
| `ctx.registerPage({ title, render }, { navLabel, icon })` | 整页视图并自动配图标栏入口。`render({ container, close })` **每次打开都会重新调用**，每次都从空白重建 `container` 的内容，不要缓存 DOM |
| `ctx.registerMenuItem(def)` | Codex++ 菜单「主页」面板的一行。开关形态：`{ label, description, toggleValue: () => bool, onChange: (next) => {} }`；按钮形态：`{ label, description, buttonLabel, onActivate: ({ close }) => {} }` |
| `ctx.registerRowAction({ label, icon, onActivate })` | 会话行「更多操作」里的一项，`onActivate({ row, session_id, close })` |
| `ctx.registerNavEntry({ label, icon, onActivate })` | 只要一个图标栏按钮、不要整页时用 |
| `ctx.effect(() => cleanupFn)` | 任意副作用（MutationObserver、插入的 DOM、patch 的函数），返回撤销函数 |
| `ctx.listen(target, type, handler, options)` | 自动撤销的事件监听 |
| `ctx.setInterval / ctx.setTimeout` | 自动清理的定时器 |
| `ctx.on(event, fn) / ctx.emit(event, ...args)` | 插件之间的事件 |
| `ctx.intercept(name, (value, next) => next(value), { order }) / ctx.waterfall(name, value)` | 中间件链 |
| `ctx.call(route, payload)` | 调 Codex++ 后端，只能调写进 `routes` 参数且在可用列表里的路由 |
| `ctx.toast(message, { type })` | 提示，type 为 info / success / warn / error |
| `ctx.fail(error)` | 上报 `await` 之后的异步错误，否则管理页看不到 |

`icon` 只接受单个字符（如 `"⏱"`）或 SVG 字符串。每个插件最多注册 16 项。

## 必须遵守

- **自己插入到 Codex 页面里的节点必须带 `data-codex-plus-ext` 属性**（值写插件 id），否则会触发 Codex++ 的
  扫描自喂循环、吃满 CPU。用 `ctx.effect` 插入，并在返回的撤销函数里移除。
- 自己加的 `<style>` 用唯一 id（如 `codex-plus-ext-<插件id>-style`），同样带 `data-codex-plus-ext`，撤销时移除。
- 颜色用 CSS 变量适配深浅色：`var(--codex-plus-text)`、`var(--codex-plus-bg-elevated)`、`var(--codex-plus-border)`。
  页面里的行可以复用 `<div class="codex-plus-row"><div><div class="codex-plus-row-title">标题</div>
  <div class="codex-plus-row-description">说明</div></div></div>`。
- 需要记住用户设置就用 `localStorage`，键名带插件 id 前缀。
- Codex 自己的 DOM 结构随版本变化，没有稳定性承诺。依赖它时要容错（找不到元素就静默跳过），并用
  `MutationObserver` 等待元素出现，而不是假设它已经在。
- 不要读取或外发用户的密钥、配置、会话内容，除非用户明确要求且清楚后果。不要请求任意网络地址，除非这就是用户要的功能。

## 示例：图标栏番茄钟

```js
codexPlus.plugin((ctx) => {
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
        // render 每次打开都会调用：定时器挂在页面元素上，元素被移除后停止
        const timer = ctx.setInterval(() => (container.isConnected ? paint() : timer()), 1000);
      },
    },
    { navLabel: "番茄钟", icon: "⏱" },
  );
});
```
