# 拓展 UI 接口

Codex++ 把注入到 Codex 页面里的 UI 能力开放给用户脚本（拓展）调用。入口只有一个：

```js
window.codexPlus
```

本文写给拓展作者。想了解为什么这样设计、有哪些已知限制，见
[设计文档](specs/2026-09-30-ui-extension-api-design.md)。

---

## 快速上手

脚本放在 `~/.config/Codex++/user_scripts/`（Windows 为 `%APPDATA%\Codex++\user_scripts\`），
文件名以 `.js` 结尾。改完在管理页点「热重载拓展」生效。

```js
// 1. 会话行的「更多操作」里加一项
window.codexPlus.registerRowAction({
  label: "导出为 CSV",
  icon: "⇩",
  onActivate: ({ session_id }) => {
    window.codexPlus.call("/session/export", { session_id })
      .then((result) => window.codexPlus.toast("导出完成", { type: "success" }))
      .catch((error) => window.codexPlus.toast(`导出失败：${error.message}`, { type: "error" }));
  },
});

// 2. 图标栏加一个入口，点开是一个整页视图
window.codexPlus.registerPage(
  {
    title: "我的面板",
    render: ({ container, close }) => {
      // 不要缓存 container：每次打开页面都会重新调用 render，
      // 上一次的 DOM 已经被销毁了。
      container.innerHTML = `<div class="codex-plus-row">你好</div>`;
    },
  },
  { navLabel: "我的面板", icon: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8"/></svg>' },
);
```

---

## 生命周期契约（最重要的一节）

**注册表持久，DOM 瞬态。**

Codex++ 的 UI 宿主会被反复重建：

| 宿主 | 何时重建 |
| --- | --- |
| 整页 overlay | 每次打开都清空重建 |
| 会话行按钮组 | `codexActionGroupVersion` 变化时整组重建 |
| 浮层面板 | 设置开关变化时 runtime 重启 |

所以：

- **不要缓存 DOM 引用**。`render({ container })` 每次打开都重新调用，你应该每次都从零构建 `container` 的内容。
- **不要假设你 `appendChild` 的节点还在**。宿主重建后它会被清掉，但注册表里的数据仍在，下一次渲染会重新带上它。
- 需要持有定时器 / 全局监听时，用 `onCleanup` 注册清理函数，或把它挂在 `container` 上并在下次渲染时重建。

```js
let timer = null;
window.codexPlus.registerPage({
  title: "轮询面板",
  render: ({ container }) => {
    container.innerHTML = `<div data-output></div>`;
    const output = container.querySelector("[data-output]");
    clearInterval(timer);                       // 上次的定时器先停掉
    timer = setInterval(() => { output.textContent = new Date().toLocaleTimeString(); }, 1000);
  },
});
window.codexPlus.onCleanup(() => clearInterval(timer));
```

---

## API 参考

### `codexPlus.apiVersion`

数字，当前为 `1`。后端契约变化时会增加。脚本可以据此提示用户升级：

```js
if (window.codexPlus.apiVersion < 1) {
  window.codexPlus.toast("请升级 Codex++ 以使用本拓展", { type: "warn" });
}
```

### `codexPlus.script`

`{ key }`，当前脚本的标识（形如 `user:my-script.js`）。用于诊断。

### `codexPlus.toast(message, options?)`

右下角提示。

| 选项 | 说明 |
| --- | --- |
| `type` | `info` / `success` / `warn` / `error`，决定边框配色；不传用默认外观 |

最多同时显示 3 条，超出时最旧的被挤掉。返回一个函数，调用即提前关闭。

### `codexPlus.call(route, payload?, options?)`

调用 Codex++ 后端，返回 Promise，失败时 reject。

```js
try {
  const result = await window.codexPlus.call("/diagnostics/log", { event: "hello" });
} catch (error) {
  // error.message 是可读的失败原因
}
```

**路由白名单**（未列出的路由不可调用）：

| 路由 | 用途 |
| --- | --- |
| `/diagnostics/log` | 写一条诊断日志 |
| `/session/export` | 导出会话内容 |
| `/thread-usage-history` | 读会话用量历史 |
| `/archived-thread` | 读归档会话 |
| `/export-markdown` | 导出 Markdown |
| `/user-scripts/list` | 读拓展清单 |

`options.timeout` 可覆盖默认的 26 秒超时。注意桥接协议没有取消通道，超时只放弃等待，服务端任务仍会跑完；如果此时页面刷新或桥接重连，请求会被中断。

### `codexPlus.registerRowAction(definition, options?)`

在会话行的「更多操作」菜单里加一项。

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `label` | 是 | 菜单项文字 |
| `onActivate` | 是 | `({ row, session_id, close }) => void` |
| `icon` | 否 | 单个字形或 SVG 字符串 |

### `codexPlus.registerPage(definition, options?)`

注册一个整页视图，并自动配一个图标栏入口。

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `title` | 是 | 页面标题 |
| `render` | 是 | `({ container, close, script }) => void`，每次打开都调用 |
| `icon` | 否 | 入口图标，单个字形或 SVG 字符串 |

| 选项 | 说明 |
| --- | --- |
| `navLabel` | 图标栏入口的可访问标签，默认取 `title` |
| `order` | 多个拓展入口之间的排序，默认按注册顺序 |

### `codexPlus.onCleanup(fn)`

注册清理函数。热重载、禁用、删除脚本时，按「脚本逆序 + 注册逆序」执行。

**注意**：只有当**所有**已加载脚本都注册了清理函数时，热重载才走原地清理；
任何一个脚本没注册（或清理抛错、返回 Promise），Codex++ 会回退到整页刷新。
这是为了保证不残留旧实例，但也意味着**你注册清理函数能让所有人的重载体验更好**。

### `codexPlus.fail(error)`

主动上报失败。`wrap_script` 只捕获脚本初始化期间的同步错误，`await` 之后的
异步错误不会被自动捕获——用这个方法上报，管理页才能显示出来。

```js
window.codexPlus.call("/session/export", { session_id })
  .catch((error) => window.codexPlus.fail(error));
```

### `codexPlus.constants`

稳定的类名与属性契约。**这些值一旦发布不再更名**。

| 键 | 值 |
| --- | --- |
| `toastClass` | `codex-delete-toast` |
| `pageClass` | `codex-plus-page-overlay` |
| `actionGroupClass` | `codex-session-actions` |
| `moreMenuClass` | `codex-session-more-menu` |
| `railSelector` | `nav[data-app-navigation-rail]` |
| `pageNavAttribute` | `data-codex-plus-page-nav` |
| `extensionAttribute` | `data-codex-plus-ext` |

---

## 样式

Codex++ 的 UI 配色走 CSS 变量，拓展直接复用即可自动适配深浅色主题：

```css
.my-ext-panel {
  color: var(--codex-plus-text);
  background: var(--codex-plus-bg-elevated);
  border: 1px solid var(--codex-plus-border);
}
```

自己注入 `<style>` 时，请给节点带唯一的 id 前缀，避免和内置样式互相覆盖。

---

## 限制

理解这些限制能省下大量排查时间：

1. **不是安全边界**。脚本跑在 Codex 页面的主世界，和 Codex 自身 JS 同权。你可以
   直接调 `window.__codexSessionDeleteBridge` 绕过路由白名单，也可以改任何 DOM。
   白名单和配额是**防误用、不是防恶意**——既然你已经在写脚本，请遵守约定。
2. **每个脚本最多注册 16 项**，全局最多 64 项，超出会抛错。
3. **`render` 里抛错不会白屏**。Codex++ 会捕获并把错误写进该脚本的状态，
   管理页可见；页面上显示一块错误占位。
4. **不要注册选择器**。插入节点时带上 `data-codex-plus-ext` 属性即可
   （`registerPage` 等接口会自动加），扫描调度靠它把你的写入排除在自激循环之外。
5. **浮层面板 tab 尚未开放**。目前只有会话行、图标栏入口、整页视图、toast 四类。

---

## 排查

- 管理页「拓展」页能看到每个脚本的加载状态与错误。
- `window.__codexPlusExtensionFailures` 是最近 100 条拓展失败记录。
- `window.__codexPlusRegistryLog` 是注册/注销的流水。
- 脚本卡在 `loaded` 却行为异常，多半是异步错误被吞了——补上 `codexPlus.fail`。
