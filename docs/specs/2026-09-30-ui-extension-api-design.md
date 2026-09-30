# UI 能力开放给拓展（用户脚本）——现状梳理与设计方案

日期：2026-09-30
目标：把 Codex++ 注入的 UI 能力（菜单项、导航入口、会话行按钮、浮层面板 tab、toast、整页视图）暴露给第三方用户脚本调用
相关文件：`assets/inject/renderer-inject/`、`assets/inject/floating-panel/`、`crates/codex-plus-core/src/user_scripts.rs`
状态：设计稿，未实施

---

## 〇、一句话结论

**当前不存在任何 UI 扩展点，一个都没有。** 六类 UI 能力全部是硬编码模板字符串 + 函数内局部变量，锁在 IIFE 闭包里，第三方脚本拿不到任何一项。

所以这件事的真实工作量不是「把函数挂到 window 上」，而是**先把散落的 UI 装配代码重构成注册中心，再开放稳定接口**。挂函数是一天的活，重构是主要的活。

---

## 一、现状梳理

### 1.1 两套注入体系，互不相通

| | renderer-inject | floating-panel |
|---|---|---|
| 入口 | `assets/inject/renderer-inject.js` | `assets/inject/floating-panel-inject.js` |
| 拼装方式 | Node 脚本切分/回收（`scripts/split-renderer-inject.mjs` + `assemble-renderer-inject.mjs`），产物落盘，测试守漂移 | Rust `concat!` 编译期拼接（`crates/codex-plus-core/src/assets.rs:48-71`），无中间产物 |
| 分片清单 | `manifest.json`（**已与内容漂移，见 1.6**） | 硬编码在 `concat!` 参数顺序里 |
| 作用域 | 单个大 IIFE（`00-prelude.js:1` → `99-tail.js`） | 单个大 IIFE |
| 注入时机 | 总是注入 | **按需注入**：`stepwise_enabled \|\| answer_outline_enabled` 才拼进去（`assets.rs:447-451`） |
| 执行顺序 | 先 | 后 |

两者是拼成**同一个字符串一次性注入**的，但各自独立 IIFE，作用域互不污染。

用户脚本是**第三套**，由 `wrap_script`（`user_scripts.rs:443`）注入，时间点晚于前两者，每个脚本独立 IIFE。

### 1.2 全局命名空间现状：42 个 `__codexPlus*`，能力 API 一个没有

全量清点后，`window.__codexPlus*` 共 42 个名字，性质分布：

- **约 60%** 是幂等哨兵：`__codexPlusModelPatchFailures`（出现 30 次）、`__codexPlusAppServerClientPrototypePatchInstalled`、`__codexPlusModelMessagePatchInstalled`……
- **约 30%** 是定时器句柄 / 清理槽位：`__codexPlusPageNavigationCloseTimer`、`__codexPlusConversationViewCleanup`、`__codexPlusOfficialUsageWindowCleanup`
- **约 10%** 是测试钩子：`__codexPlusServiceTierTest`、`__codexPlusRateLimitUnlockTest`、`__codexPlusPluginMarketplaceTest`

**42 个里只有 1 个有清理路径**（`pet-real-mouse-inject.js:298` 的 `delete`）。

唯一称得上「对外接口」的是梦皮（dream skin）：

```js
window.__CODEX_PLUS_APPLY_DREAM_SKIN__   // 10-style.js:1879
window.__CODEX_PLUS_CLEAR_DREAM_SKIN__   // 10-style.js:2132
```

**这是本项目已有的先例**——launcher 侧通过它们调用渲染层能力。说明「大 IIFE 往外挂能力对象」这个模式不用从零发明，照这个形状做即可。

### 1.3 六类 UI 能力的可开放性评级

| UI 能力 | 实现位置 | 现状 | 开放难度 |
| --- | --- | --- | --- |
| **toast** | `80-session-share.js:47` `showToast(msg, undoToken)` | 闭包私有；**单例语义**（每次先删旧的）；**无样式变体** | ★ 低 |
| **会话行按钮** | `90-action-groups.js:103` `createSessionMoreMenuItem(label, icon, onActivate)` | **已有干净的扩展点形状**，但调用方写死在 `:211-228` | ★ 低 |
| **导航入口** | `40-backend-settings.js:1548` `spec = {id, label, iconMarkup, withStatus, onActivate}` | **最接近注册点的东西**，但是函数内局部数组 | ★★ 中 |
| **整页视图** | `40-backend-settings.js:1004` `openCodexPlusModal({page:true, ...})` | 能力成熟（缩放/主题/选中态全都处理了），但只认写死的三个 tab；未知 tab **兜底成 `"home"`** | ★★ 中 |
| **菜单项** | `40-backend-settings.js:1015-1152` | **一整块 `innerHTML` 模板字符串**，无数组无 schema 无 registry | ★★★ 高 |
| **浮层面板 tab** | `floating-panel/core/views.js:233` | **三元链分派**，且被 `DEFAULT_VIEW_ORDER` 硬闸门过滤；FAB 状态机、`stopRuntime()` 清理清单、CSS 规则三处都要同步改 | ★★★★ 高 |

### 1.4 三个「无声回收」陷阱（必须先解决）

**(a) `isExtensionUiNode` 白名单硬编码** — `98-scan-schedule.js:2`

```js
function isExtensionUiNode(node) {
  return !!node?.closest?.(`.codex-delete-toast, .codex-delete-confirm-overlay, .codex-plus-modal-overlay, .${codexPlusPageClass}, #${codexPlusSidebarNavId}, ... #codex-plus-menu`);
}
```

配合 `shouldScheduleScan` 里的自喂循环防护（`98-scan-schedule.js:60`）：

> 一次变更如果只动了我们自己的 UI，就不该再排一次 scan。

**后果是二选一**：第三方插的节点不在白名单里 → 要么触发扫描风暴（代码注释明确记录了 issue #1960：空闲时每秒全量扫描五次，macOS 上吃满一个核，实测 301 请求/秒），要么自己的 UI 被过滤掉后不再刷新。

**这条是硬阻塞，且必须用「可注册选择器」解决，不能继续往字符串里加名字。**

**(b) `codexDeleteStyleVersion` 强杀样式** — `00-prelude.js:415`（当前 `"23"`）

```js
function installStyle() {
  const existingStyle = document.getElementById(styleId);
  if (existingStyle?.dataset.codexDeleteStyleVersion === codexDeleteStyleVersion) return;
  ...
}
```

第三方注入的 CSS 独立于这个版本号，但如果不走同一套注入机制，就会**永远不加载**（或与内置样式打架）。

**(c) `codexActionGroupVersion` 强杀按钮** — `00-prelude.js:435`（当前 `"6"`）

会话行按钮靠 `group.dataset.codexActionGroupVersion === codexActionGroupVersion` 判幂等，不匹配**整组删掉重建**。第三方塞进 group 的按钮会被静默清掉。

同理还有 `codexDeleteVersion`（`"7"`）、`codexExportVersion`（`"1"`）、`codexArchiveRowActionsVersion`（`"1"`）。

### 1.5 其它已知缺口

- **`codexPlusSettings()` 是扁平 key**（`10-style.js:1353`），第三方 key 直接撞内置项
- **overlay 每次打开都重建**：`openCodexPlusModal` 先 `querySelectorAll(...).forEach(remove)` 再 `document.body.appendChild`，**第三方注入的节点会随之消失**
- **`stopRuntime()` 清理清单写死**（`floating-panel/runtime/lifecycle.js:171`）：逐项列举 7 个固定清理句柄，第三方注册的定时器不会被清理
- **`bridgeCall` 超时不 reject，而是 resolve `{error: "page bridge timed out"}`**（`floating-panel/runtime/bridge-client.js:10`，26000ms）——这个约定第三方必须遵守，否则错误处理不一致
- **异步错误不上报**：`wrap_script` 只 catch 顶层同步代码，`await` 之后的错误会让状态永远停在 `loaded`

### 1.6 附带发现：分片名与内容已漂移

`manifest.json` 声明的分片名和文件实际内容对不上，原因是 `split-renderer-inject.mjs` 按顶层声明边界切分，而 `assemble-renderer-inject.mjs` 只按 manifest 顺序拼回，每个文件开头都带着上一块的尾巴：

- `20-menu.js` 的 fragment 名是「菜单」，但 `renderCodexPlusMenu()`（`:1`）只是同步已有按钮状态；真正的菜单 HTML 在 `40-backend-settings.js:1015-1152`
- `50-navigation.js` 的 `installCodexPlusNavigationEntries()`（`:1`）只有 8 行，是个分发壳；真正的 rail 安装在 `40-backend-settings.js:1374-1580`

**这不阻塞本设计，但会让后续维护者找错文件，建议顺手修正 manifest 描述。**

---

## 二、设计目标

**要做的**

1. 建一个**注册中心**，让内置项与第三方项走同一条渲染路径（这是所有能力开放的前提）
2. 六类 UI 能力分批开放，每批独立可发布
3. 第三方 UI 失败**不能拖垮宿主**：一个坏插件不该让菜单白屏或触发扫描风暴
4. 保持**完全向后兼容**：现有脚本（`conversation-canvas`）一行不改继续工作

**不做的**

- 不做沙箱隔离（脚本跑主世界是既有事实，隔离等于砍掉核心能力）
- 不开放「修改 Codex 原生 DOM」的接口——那是脚本自己猴补丁的事，不该由我们许诺
- 不承诺 UI 的像素级稳定性，只承诺**类名/属性契约**

---

## 三、架构设计

### 3.1 核心：注册中心

在 renderer-inject 里新增一个 fragment（建议 `85-registry.js`，插在 80 与 90 之间——此时依赖的原语都已声明，UI 消费方尚未执行），定义：

```js
const codexPlusRegistry = {
  menus:      new Map(),   // id -> { id, label, description, icon, order, panel, onActivate, visible }
  navEntries: new Map(),   // id -> { id, label, iconMarkup, withStatus, order, onActivate }
  rowActions: new Map(),   // id -> { id, label, icon, order, onActivate }
  pages:      new Map(),   // id -> { id, title, icon, render, destroy }
  views:      new Map(),   // id -> { id, title, icon, order, render, attach, enabled }
  toasts:     /* 见 3.4，不是注册表 */
};
```

**三条设计约束**：

1. **注册中心不依赖任何具体 UI 结构**。它只存数据和回调，渲染由各消费方自己读。
2. **内置项也走注册中心**（`register` 时带 `internal: true`）。这样「第三方能不能插进来」和「内置项怎么写」是同一个问题，不会出现两条代码路径。这是本次重构最大的价值，也是最大的风险（要动所有内建 UI）。
3. **注册顺序决定渲染顺序，`order` 字段可覆盖**。内置项用保留段位：`order < 1000` 为内置，第三方默认从 `1000` 起，避免插到内置项前面破坏既有布局。

### 3.2 对外接口层：单一挂载点

沿用梦皮的先例，**只挂一个全局对象**，不再散落：

```js
window.codexPlus = {
  version: "1.0.0",
  apiVersion: 1,

  // UI 注册（返回 dispose 函数）
  ui: {
    registerMenuItem(def)  -> dispose,
    registerNavEntry(def)  -> dispose,
    registerRowAction(def) -> dispose,
    registerPage(def)      -> dispose,
    registerView(def)      -> dispose,
    toast(message, options),
    constants: { /* 类名常量，见 3.6 */ },
  },

  // 回调桥接（见 3.4）
  call(route, payload, options) -> Promise,

  // 生命周期
  onCleanup(fn),
  on(event, handler),
  ready(),
  fail(error),

  script: { key, name, source },
  capabilities: [...],
}
```

**挂载时机**：在 `99-tail.js` 收尾之前挂。此时所有 fragment 都已执行，闭包变量全部可用。

**`dispose` 语义**：每个 `register*` 返回一个函数，调用即注销并清理该扩展渲染的 DOM。这是 `registerCleanup` 之外的第二层，因为**注册的寿命 ≠ 脚本的寿命**（见 3.5 的 overlay 重建问题）。

### 3.3 关键改造点（按能力）

#### (a) `isExtensionUiNode` —— 必须最先做

把硬编码字符串改成可注册的：

```js
const extensionUiSelectors = new Set([/* 现有 13 个内置选择器 */]);
function registerExtensionUiSelector(selector) { extensionUiSelectors.add(selector); }

function isExtensionUiNode(node) {
  for (const selector of extensionUiSelectors) {
    if (node?.closest?.(selector)) return true;
  }
  return false;
}
```

**性能注意**：`shouldScheduleScan` 在每次 mutation 上都跑，选择器数量增长要控制。当前 13 个，第三方注册上限建议设 32 个并做校验（选择器语法合法性）。另外 `closest()` 逐个调用比一次传逗号串慢，可缓存成逗号拼接串，只在 Set 变化时重建。

**第三方注册时必须提供选择器**，`registerMenuItem` 等接口从定义里自动推导（约定：所有扩展节点带 `data-codex-plus-ext="<scriptKey>"`），这样只需注册一条 `[data-codex-plus-ext="<key>"]`。

#### (b) 样式注入 —— 版本号问题

不要把第三方 CSS 塞进 `codexDeleteStyleVersion` 那套（那是内置样式的版本控制）。新增独立注入：

```js
// 每个拓展一个 <style id="codex-plus-ext-style-<key>">
function injectExtensionStyle(key, css) { ... }
```

**但必须处理与内置 `<style>` 的层叠顺序**：内置样式用 `--codex-plus-*` 变量，第三方应该优先复用变量而不是覆盖规则。文档里要写清楚。

#### (c) `codexActionGroupVersion` —— 给第三方留位置

改法：`attachButton(row)` 重建 group 后，遍历 `codexPlusRegistry.rowActions` 重新挂。**关键是重建时必须重新挂**，因为版本号变化会整组删掉。

同时 `positionSessionMoreMenu` 的高度估算（`90-action-groups.js:89` 写死 `Math.max(80, …)`）要改成按实际项数算，否则第三方加项后菜单会溢出屏幕。

#### (d) 浮层面板 tab —— 最贵的一块

按「先固化约定，再开放」两步：

**第一步**：把三元链改成元数据表。

```js
// core/views.js
const VIEWS = {
  next:     { title: "下一步建议", icon: "next",     enabled: stepwiseEnabled, render: nextHtml,     attach: attachNextEvents },
  outline:  { title: "回答大纲",   icon: "outline",  enabled: outlineEnabled,  render: outlineHtml,  attach: attachOutlineEvents },
  settings: { title: "设置",       icon: "settings", enabled: () => true,      render: settingsHtml, attach: attachSettingsEvents },
};
```

改动点（六处，全部是「去三元化」）：
1. `viewTabHtml()`（`core/views.js:26`）从表取 title/icon
2. 内容分派（`core/views.js:233`）改查表
3. 事件分派（`core/views.js:260`）改查表
4. `normalizeActiveTab()`（`runtime/state.js:446`）改查表
5. `normalizeViewOrder()`（`runtime/state.js:246`）的过滤集合从 `DEFAULT_VIEW_ORDER` 改为「表的 key + 已注册的第三方 tab」
6. `enabledViewOrder()`（`runtime/state.js:427`）改查表的 `enabled()`

**第二步**：暴露 `registerView`，并必须同时处理三件事，否则会漏：
- **FAB 表达式求值器**（`core/host.js:63` `resolveFabExpression`）：第三方 tab 激活时，FAB 会显示 next/outline 的状态。需要给表加可选的 `expression()`，第三方不提供则回退默认
- **`stopRuntime()` 清理清单**（`runtime/lifecycle.js:171`）：加一个 `state.extensionCleanups` 数组纳入清理
- **CSS**：`core/appearance.js` 里 `.csw-icon[data-view="next"] svg`（`:983`）、`[data-view="settings"]`（`:991`）、`.csw-body[data-view-body="next"]`（`:1042`）是按视图名硬选的，第三方视图得自己提供样式，文档要给模板

**一个坑**：浮层面板是**按需注入**的（`assets.rs:447`）。如果两个开关都关，整个面板不存在，`registerView` 会失败。接口必须**容忍宿主不存在**——返回一个 no-op dispose，并记一条诊断，而不是抛错。

#### (e) 整页视图 —— 放开 tab 白名单

`codexPlusModalTab()`（`40-backend-settings.js:998`）会把未知 tab 兜底成 `"home"`，这是第三方页面拿不到控制权的直接原因。改为：先查 `codexPlusRegistry.pages`，命中则调用其 `render()`，未命中才兜底。

同时 rail specs（`40-backend-settings.js:1570`）和 `renderCodexPlusPageNavItems()`（`40-backend-settings.js:463`）都要从注册中心读。

**特别注意**：新增 rail 入口必须一并注册进 `syncCodexPlusRailNativeSelection()`（`40-backend-settings.js:812`）的数组，否则 Codex 原生 selected 态压不下来，rail 上会同时亮两个。

#### (f) toast —— 先改语义再开放

现状有两个问题必须先解决：

1. **单例语义**：`showToast` 每次先删旧的。第三方和内置会互相踩。改为**队列 + 最多同时 3 条**，超出则挤掉最旧的
2. **无样式变体**：加 `options.type = "info" | "success" | "warn" | "error"`，对应 CSS class。当前只有一个外观（`10-style.js:260-275`）

改完再暴露 `codexPlus.ui.toast(message, options)`。

### 3.4 后端调用：沿用现有桥，加包装

第三方调后端不需要新机制——`window.__codexSessionDeleteBridge` 已经是全局可达的（浮层面板和 `conversation-canvas` 都在用）。但直接暴露它有三个问题：名字语义错位、无超时、无错误归一。

所以 `codexPlus.call(route, payload, options)` 做薄包装：

1. 查路由白名单（编译进接口层），未声明直接 reject，**不发请求**
2. 校验 payload 可序列化
3. 默认超时 26s（与浮层面板一致）
4. **错误归一化**：现有实现有两种风格——路由层返回 `{status:"failed"}`，浮层面板返回 `{error: "..."}`。统一转成 reject + 结构化 error 对象

**注意**：`bridge.rs:104` 在桥接重注入时会把所有 pending resolver 用 `{status:"failed", message:"桥接已重新连接"}` 结掉。这是刻意的（防 Promise 永久 pending），但意味着**长任务会被桥接重连打断**，接口层应把这个情况识别为「可重试」而非「失败」。

### 3.5 生命周期：必须解决 overlay 重建

这是最容易设计错的地方。三个层次的生命周期，现在只有一层：

| 层次 | 触发 | 现有机制 | 需要补的 |
| --- | --- | --- | --- |
| 脚本卸载/热重载 | 用户点热重载 | `registerCleanup` ✅ | — |
| **UI 宿主重建** | overlay 重新打开、会话行重排、面板 runtime 重启 | **无** ❌ | 注册中心重挂 |
| 页面刷新 | 整页 reload 回退 | 天然解决 | — |

**UE 宿主重建**的三种情况和应对：

1. **overlay 重建**（`openCodexPlusModal` 每次清空重来）：注册中心的 `pages` / `menus` 数据是持久的，渲染函数每次重建时重新读并重建 DOM。**第三方只提供 `render()`，不持有 DOM 引用**——这是接口契约的核心，必须写进文档。
2. **会话行按钮重建**（`codexActionGroupVersion` 变化）：同上，`rowActions` 注册表重挂。
3. **浮层面板 runtime 重启**（`stopRuntime()` → `activateRuntime()`）：`state.extensionCleanups` 纳入清理，重启时第三方 tab 仍在注册表里，`renderFloat()` 会自动重建。

**统一原则**：**注册表持久，DOM 瞬态**。第三方永远不应该 `appendChild` 之后指望它还在；所有渲染都应该是「从注册表读数据 → 每次都全量重建」。

### 3.6 契约固化：类名与常量

CSS 类名常量现在全在闭包里（`00-prelude.js:405,418,419,423,424`），第三方只能硬编码字面量。开放接口时必须一并暴露：

```js
window.codexPlus.ui.constants = {
  pageClass: "codex-plus-page-overlay",
  railSelector: "nav[data-app-navigation-rail]",
  extensionAttr: "data-codex-plus-ext",
  // ...
};
```

同时定契约：**这些类名一旦发布不再更名，新增用新名字**。`codexDeleteStyleVersion` 那种「版本号变了整个重建」的机制是内部实现，不该泄漏到第三方。

### 3.7 失败隔离

第三方代码可能抛错、可能死循环、可能插一个无限增长的 DOM。三条防线：

1. **注册时校验**：`register*` 对 def 做 schema 校验（必填字段、类型、order 范围、选择器语法），不合法直接 reject 并记诊断
2. **渲染时 try/catch**：每个第三方扩展的 `render()` 单独包 try/catch，失败时**渲染一个错误占位**（而不是让整个菜单白屏），并把错误写进该脚本的 `status`/`error` 字段（复用 `wrap_script` 已有的上报通道）
3. **数量上限**：每个脚本最多注册 16 项、全局最多 64 项，超出拒绝并诊断。防止劣质插件把扫描拖慢

---

## 四、实施顺序

按「阻塞点优先 + 独立可发布」排：

### Step 1 — 地基（无对外接口，纯重构）
- 新增 `85-registry.js` 注册中心
- `isExtensionUiNode` 改为可注册选择器（**这是硬阻塞，必须第一步**）
- 内置项迁到注册中心（菜单 / 导航 / 会话行三条线）
- 测试：注册中心单测 + 现有 UI 测试全绿（确保重构无行为变化）

**验收标准**：功能零变化，纯内部结构改动。这一步不做任何对外暴露。

### Step 2 — 开放第一批（低风险）
- 挂 `window.codexPlus`（版本号、`ui.constants`、`onCleanup`）
- `registerRowAction` + `registerNavEntry`（内部形状已经很好，改造成本最低）
- `toast` 语义改造（队列 + type）并暴露
- 扩展样式注入机制
- 迁移 `conversation-canvas` 试用

### Step 3 — 开放整页与菜单
- `registerPage`，放开 `codexPlusModalTab` 白名单
- `registerMenuItem`，拆菜单 `innerHTML` 模板为「内置项数组 + 渲染函数」
- 设置 key 命名空间（第三方 key 加 `ext:<scriptKey>:` 前缀）
- rail 选中态同步纳入新入口

### Step 4 — 开放浮层面板 tab（最贵，可延后）
- 三元链改 `VIEWS` 元数据表
- `registerView` + FAB 表达式分支 + `stopRuntime` 清理
- 第三方视图 CSS 模板与文档

### Step 5 — 文档与类型
- `types/codex-plus-extensions.d.ts`（从注册中心 schema 生成）
- `EXTENSIONS.md`（仓库根目录）：快速上手、完整 API、类名契约、生命周期契约、失败排查
- AGENTS.md 补「已发布的类名/接口不再更名」约定

---

## 五、风险与开放问题

**风险**

1. **Step 1 的重构面很大**。「内置项也走注册中心」意味着要动菜单、导航、会话行三处核心 UI，任何行为偏差都会影响所有用户。缓解：这一步不开放任何接口，纯重构，靠现有测试兜底；如果风险不可接受，退一步允许内置项暂时双轨（新代码走注册中心，旧代码保留），但**这会留下长期技术债**。
2. **扫描性能**。`isExtensionUiNode` 加选择器后每次 mutation 都要多跑几次 `closest()`。当前已有 issue #1960 的前科，必须做选择器数量上限 + 合并成单串缓存。
3. **浮层面板按需注入**。视图接口在宿主不存在时必须有优雅降级，否则用户关掉两个开关后第三方脚本会报错。

**开放问题**

1. **Step 1 是否接受「内置项双轨」过渡**？还是必须一次性全迁？这决定风险与工期的取舍。
2. **`registerView` 是否值得做**？浮层面板是闭合系统，改造面最大（FAB 状态机 + 清理清单 + CSS 三处），而它的用户群（用 stepwise/outline 的人）和写插件的开发者重合度可能很低。**建议 Step 4 待前三个 Step 上线、有真实第三方需求后再启动。**
3. **是否需要「插件市场」联动**？市场清单里 `requirements` / `limitations` 目前只是展示文本。如果开放 UI 接口，可以考虑在市场详情页展示「该拓展需要哪些 UI 能力」，但这是 Step 5 之后的事。
4. **失败隔离的错误占位**：渲染错误占位块会让用户看到「这个拓展坏了」，但也会污染界面。是否改为「静默移除 + 管理页报错」？倾向后者，但需要你确认。

**未验证项**

- `codexDeleteStyleVersion` 之外的版本号（`codexDeleteVersion` / `codexExportVersion` 等）是否也有类似的「版本变化即重建」逻辑影响第三方——需要逐个确认
