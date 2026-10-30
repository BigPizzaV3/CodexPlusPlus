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
- 需要记住用户设置就用 `localStorage`，键名带拓展 id 前缀。
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
