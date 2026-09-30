# Codex++ 拓展开发指南

Codex++ 把注入到 Codex 页面里的 UI 能力开放给用户脚本（拓展）调用。入口只有一个全局对象：

```js
window.codexPlus
```

本文档描述**当前已经可用**的接口。接口版本：`apiVersion = 1`（适配层 `1.0.0`）。

- 想了解设计取舍与已知限制的来龙去脉，见 [docs/specs/2026-09-30-ui-extension-api-design.md](docs/specs/2026-09-30-ui-extension-api-design.md)
- 类型声明：起 `types/codex-plus-extensions.d.ts`，在脚本开头加 `/// <reference types="./codex-plus-extensions" />` 即可获得编辑器补全

---

## 目录

- [快速上手](#快速上手)
- [脚本放在哪、怎么生效](#脚本放在哪怎么生效)
- [生命周期契约（最重要的一节）](#生命周期契约最重要的一节)
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
  label: "导出为 CSV",
  icon: "⇩",
  onActivate: ({ session_id }) => {
    window.codexPlus.call("/session/export", { session_id })
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

- 只加载 `.js` 文件，按文件名小写排序执行
- 每个脚本在独立 IIFE 里运行，互不污染
- 改完脚本后在管理页点「热重载拓展」即可生效，无需重启 Codex

脚本标识（`codexPlus.script.key`）形如 `user:my-script.js`，由来源目录与文件名拼成。

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

`{ key: string }`，当前脚本的标识。用于诊断与日志。是 getter，脚本初始化结束后再读也能拿到。

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

在会话行的「更多操作」菜单里加一项。

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `label` | 是 | 菜单项文字 |
| `onActivate` | 是 | `({ row, session_id, close }) => void` |
| `icon` | 否 | 单个字形或 SVG 字符串；不传用 `◇` |

`onActivate` 收到的 `row` 是该会话行的 DOM 元素，`session_id` 已解析好，`close()` 关闭菜单。

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
window.codexPlus.call("/session/export", { session_id })
  .catch((error) => window.codexPlus.fail(error));
```

---

## 后端调用与路由白名单

`codexPlus.call` 只允许调用白名单内的路由。未列出的路由**即使后端支持也会被拒绝**（这是刻意的：新增路由默认不开放，需要显式决定）。

| 路由 | 用途 |
| --- | --- |
| `/diagnostics/log` | 写一条诊断日志 |
| `/session/export` | 导出会话内容 |
| `/thread-usage-history` | 读会话用量历史 |
| `/archived-thread` | 读归档会话 |
| `/export-markdown` | 导出 Markdown |
| `/user-scripts/list` | 读拓展清单 |

**写能力没有开放**：`/settings/set`、`/delete`、`/undo`、`/share/create` 等都不在名单内。如果你确实需要某个路由，去 Codex++ 仓库提 issue 说明用途，不要在脚本里绕过白名单直接调底层桥接（见下方「限制」第 1 条）。

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
6. **浮层面板（右下角悬浮球）的 tab 尚未开放**，见下节。

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
| 浮层面板 tab（右下角悬浮球里加视图） | 未开放。该系统是编译期拼接的闭合结构，改造面较大，见设计文档的 Step 4 |
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
