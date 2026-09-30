# 拓展 API 现状梳理与设计方案

日期：2026-09-30
范围：`crates/codex-plus-core/src/user_scripts.rs`、`bridge.rs`、`routes.rs`、`assets/inject/user-scripts-*.js`
状态：设计稿，未实施

---

## 一、现状梳理

### 1.1 拓展是怎么被执行的

拓展（用户脚本）不是配置项，也不是被 Codex 主动加载的插件，**是文件系统里的 `.js` 文件，由 Codex++ 拼成一个大字符串，通过 CDP 注入到 Codex 渲染进程主世界里执行**。

完整链路：

| 步骤 | 位置 | 说明 |
| --- | --- | --- |
| 1. 扫描 | `user_scripts.rs` `scan_script_files` | 扫 `builtin_dir` 与 `user_dir` 两个目录，只取 `.js`，按文件名小写排序 |
| 2. 定 key | `append_scripts` | `key = "{source}:{name}"`，即 `builtin:demo.js` / `user:demo.js` |
| 3. 读开关 | `load_config` | `user_scripts.json` 里的 `enabled`（全局）+ `scripts[key]`（单脚本） |
| 4. 打包 | `build_enabled_bundle` | 先放 `RUNTIME_SCRIPT`，再把每个**启用**的脚本用 `wrap_script` 包一层 IIFE |
| 5. 注入 | `apply_scripts_at` → `bridge::evaluate_script` | CDP `Runtime.evaluate`，一次性整包执行 |

目录来源（三个入口各自实现，逻辑重复）：

- 内置：`current_exe().parent()/user_scripts`
- 用户：`~/.config/Codex++/user_scripts`（Windows 为 `%APPDATA%\Codex++`）
- 配置：同目录 `user_scripts.json`

### 1.2 脚本运行环境

`wrap_script`（`user_scripts.rs:443`）干了四件事：

```js
(() => {
  if (!codexPlusIsNodeTestHarness && (子框架 || 无 electronBridge || 非 app://- 协议)) return;  // 环境守卫
  window.__codexPlusUserScripts.scripts[key] = { key, name, source, status: "loading", error: "" };
  window.__codexPlusUserScripts.currentKey = key;
  try { /* 用户脚本源码原样插入 */ status = "loaded" }
  catch (e) { status = "failed"; error = e.stack }
  finally { currentKey = null }
})();
```

关键性质：

- **无沙箱、无隔离**。源码直接被文本拼接进 IIFE，跑在和 Codex 自身 JS 完全同权的 `window` 上。改 DOM、猴补 `fetch`、读 `localStorage`、拿 `electronBridge` 都是自由的。
- **错误隔离粒度是「单脚本」**，不是「单调用」。一个脚本抛错只影响它自己，其余脚本继续执行。
- **只对顶层同步代码 try/catch**。`await` 之后的异步错误不会被捕获，状态会永远停在 `loaded`。
- **加载地址只允许主 frame + `app://-/`**。这条守卫在 `wrap_script` 和 `user-scripts-bootstrap.js` 里各写了一份。

### 1.3 事实上暴露给脚本的 API

虽然没有文档，但脚本能摸到两个全局对象 —— 这就是当前的 API 面：

#### (a) `window.__codexSessionDeleteBridge(path, payload) → Promise`

定义在 `bridge.rs:127`。名字是历史遗留（最初只用于会话删除），现在已经是**通用 RPC**。协议：

```
renderer: bridge(path, payload)
  → 分配自增 id，存进 __codexSessionDeleteCallbacks
  → window.{binding_name}(JSON.stringify({id, path, payload}))
main: handle_bridge_request(ctx, path, payload) → routes.rs:157 的 match
  → 回到 renderer: __codexSessionDeleteResolve(id, result)
```

返回值统一是 `{ status, session_id, message, ...业务字段 }`，`status` 取值 `ok` / `failed`。

**已知缺陷**：桥接重注入时会把所有 pending 的 resolver 用 `{status:"failed", message:"桥接已重新连接"}` 结掉（`bridge.rs:104`）。这是刻意设计，防止 Promise 永久 pending，但也意味着**长任务会被桥接重连打断**。

#### (b) `window.__codexPlusUserScripts.registerCleanup(fn)`

定义在 `assets/inject/user-scripts-runtime.js`。热重载时的清理协议：

- 只在脚本初始化期间可注册，`currentKey` 为 null 时抛 `Register cleanup during script initialization`
- 重载时按「脚本逆序 + 注册逆序」执行
- 任一脚本没注册清理函数 / 清理抛错 / 返回 Promise → **整页刷新回退**（`prepareReload` 返回 `"page"`）

这里有个值得注意的取舍：为了保证不残留旧实例，一个不守规矩的脚本会让**所有**脚本都走整页刷新。

### 1.4 桥接路由全清单（38 条）

按功能分组，`*` 标记的是脚本作者真正会关心的：

**拓展自身**：`*/user-scripts/list`、`*/user-scripts/set-enabled`、`*/user-scripts/set-script-enabled`、`*/user-scripts/delete`、`*/user-scripts/load`、`*/user-scripts/reload`

**市场**：`*/script-market/list`、`*/script-market/install`

**会话**：`*/delete`、`*/undo`、`*/export-markdown`、`*/thread-usage-history`、`*/archived-thread`、`*/session/export`、`*/session/import`、`*/remote-control-session/recover`

**配置**：`/settings/get`、`/settings/set`、`/backend/status`、`/codex-model-catalog`、`/ads`

**模型/供应商**：`/llm-proxy`、`/codex-config-model`

**其他**：`/devtools/open`、`/manager/open`、`/manager/open-transient`、`/share/create`、`/diagnostics/log`、`/upstream-worktree/*`（4 条）、`/stepwise/*`（4 条）、`/zed-remote/*`（7 条）

### 1.5 市场清单格式

`script_market.rs` 的 `MarketScript`，来自远程 `index.json`：

```
id, name, description, version, author, tags[], homepage,
script_url, requirements[], limitations[], icon
```

`requirements` / `limitations` 是**纯展示文本**（详情页给人看的），不参与任何校验或权限判定。

原本还有一个 `sha256` 字段，但 `install_market_script_content` 从来没做过比对（CHANGELOG 记录「移除脚本安装时的 checksum 阻断」是有意为之），属于历史遗留的死字段。**已于 2026-09-30 从结构体、解析器、payload 与前端类型中整体移除。**

### 1.6 缺口清单

| # | 缺口 | 后果 |
| --- | --- | --- |
| 1 | 无 API 版本号，无握手 | Rust 侧改路由名/改返回结构，第三方脚本静默失效 |
| 2 | 路由表是内部实现细节 | 作者只能读 `conversation-canvas` 源码猜接口 |
| 3 | 全局名 `__codexSessionDeleteBridge` 语义错位 | 新作者根本猜不到这是通用 RPC |
| 4 | 无 manifest 声明 API 需求 | `requirements` 只是给人看的字符串 |
| 5 | 无权限模型 | 脚本能调 `/settings/set`、`/session/export`、`/llm-proxy`，全凭自觉 |
| 6 | 无类型定义 | 无 `.d.ts`、无编辑器补全 |
| 7 | 异步错误不上报 | 状态停在 `loaded`，管理页显示正常但脚本已死 |
| 8 | 无调用超时 | 脚本调桥接，服务端慢则 Promise 长时间 pending |
| 9 | 无文档 | 全仓库只有 `docs/user-script-reload.md` 讲热重载 |
| 10 | 目录解析逻辑三处重复 | launcher / tauri commands 各写一份，易漂移 |

---

## 二、设计目标

**要做的**

1. 给脚本一个**稳定、有版本、有类型**的调用面，路由重构不再打断第三方脚本
2. 让权限**可声明、可校验、可见**（用户在管理页能看到脚本要什么、实际用了什么）
3. 保持**完全向后兼容**：现有脚本（至少 `conversation-canvas`）一行不改继续工作
4. 补齐生命周期：异步错误上报、调用超时、清理契约可观测

**不做的**

- 不做沙箱 / iframe 隔离。脚本价值就在于改页面，隔离等于砍掉核心能力，且 Codex 页面本身无法被我们隔离控制
- 不做脚本签名与强制审核。市场是自建的，信任模型不变
- 不改 `evaluate_script` 注入机制本身

---

## 三、方案设计

### 3.1 总体：加一层适配层，不动底层

```
用户脚本
   │  只认这层
   ▼
window.codexPlus  ←── 新增的适配层（runtime script 里实现）
   │  内部转译
   ▼
window.__codexSessionDeleteBridge  ←── 现有桥接，保持不变
   │
   ▼
routes.rs 路由表  ←── 加版本前缀与新路由，旧路由保留
```

**关键决策：适配层放在 renderer 侧（JS），不是 Rust 侧。**

理由：桥接注入可能发生在任何时候、可能被重新注入，而适配层打包在 `RUNTIME_SCRIPT` 里随每次 bundle 一起下发，天然跟随版本。放 Rust 侧反而要处理「适配层版本 vs 页面已加载版本」的不一致。

### 3.2 命名空间形状

```js
window.codexPlus = {
  version: "1.0.0",          // 适配层版本
  apiVersion: 1,             // API 契约版本，脚本据此分支
  bridgeVersion: 3,          // 桥接能力位，用于探测

  // 声明式调用：脚本写这个
  call(route, payload, options) -> Promise<Result>,
  // 便捷封装
  settings: { get(), set(patch) },
  session:  { export(id), import(data), delete(id) },
  ui:       { toast(msg), panel(...) },
  log(event, detail),

  // 生命周期（现有能力，规范化）
  onCleanup(fn),
  on(event, handler),         // 新增事件总线，见 3.5
  script: { key, name, source, manifest },

  capabilities: [...],        // 本脚本被授予的权限
}
```

`codexPlus.call()` 做的事：

1. 查本地路由白名单（编译进适配层），未声明 → 直接 reject，**不发请求**
2. 校验 payload 是不是可序列化对象（避免脚本传函数/DOM 节点把桥接搞崩）
3. 挂默认超时（30s），超时 reject 并记一条诊断
4. 转成 `__codexSessionDeleteBridge("/v1/" + route, payload)`
5. 归一化返回：把 `{status:"failed", message}` 转成 reject，`status:"ok"` 转成 resolve，让脚本用 `try/catch` 而不是 `if (res.status)`

### 3.3 权限声明

脚本文件头部加一段可选注释块（保持 `.js` 单一文件，不引入额外 manifest 文件）：

```js
// ==CodexPlus==
// id: conversation-canvas
// name: 会话画布
// apiVersion: 1
// capabilities: [session.read, diagnostics.log]
// ==/CodexPlus==
```

解析放在 Rust 侧 `scan_script_files` 阶段（正则扫前 N 行），结果进 inventory。规则：

- **无声明块 = 旧脚本 = 全量权限 + 标记 `legacy`**，保证兼容
- **有声明块 = 白名单模式**，适配层按 `capabilities` 拒绝越权调用

映射表（route → capability）编译进适配层，与 Rust 侧共享同一份来源。建议放 `assets/inject/extension-api/capabilities.json`，Rust 用 `include_str!` 读，JS 侧用构建脚本注入，避免两边漂移。

初版 capability 集合：

| capability | 覆盖路由 |
| --- | --- |
| `settings.read` | `/settings/get`、`/backend/status`、`/codex-model-catalog` |
| `settings.write` | `/settings/set` |
| `session.read` | `/session/export`、`/thread-usage-history`、`/archived-thread` |
| `session.write` | `/session/import`、`/delete`、`/undo` |
| `session.export` | `/export-markdown` |
| `network.proxy` | `/llm-proxy`（高危，给用户显著提示） |
| `ui.manager` | `/manager/open`、`/devtools/open` |
| `diagnostics.log` | `/diagnostics/log` |

**必须诚实说明的局限**：权限模型是**防误用，不是防恶意**。脚本跑在主世界，一行 `window.__codexSessionDeleteBridge("/settings/set", ...)` 就绕过了适配层。真正的强制只能靠 CDP 侧的注入隔离，而那样会砍掉脚本能力。所以定位是：给守规矩的作者一个好接口，给用户一个可见的知情权。这一点要在文档里对作者和用户都讲清楚，不能假装是安全边界。

若要真强制，唯一可行路径是 Rust 侧在 `handle_bridge_request` 里做**调用栈来源判定** —— 但 CDP 注入的代码与页面代码同源，无法区分。**结论：做不到，明确放弃这条**。

### 3.4 版本协商

`/user-scripts/list` 返回里加：

```json
{
  "api": { "adapter": "1.0.0", "api_version": 1, "min_api_version": 1, "capabilities": ["session.read", ...] }
}
```

脚本可据此分支：

```js
if (window.codexPlus.apiVersion < 1) { /* 提示用户升级 Codex++ */ }
```

Rust 侧改路由时的约定（写进 AGENTS.md）：

- 只增不改：新增路由用新路径，旧路径至少保留两个 minor 版本
- 破坏性变更必须 bump `api_version`，并在适配层保留一版转译

### 3.5 生命周期补强

**(a) 异步错误上报**

`wrap_script` 现在只 catch 同步段。补：适配层提供 `codexPlus.ready()`，脚本在异步初始化完成后调用；同时 runtime 注册全局兜底：

```js
window.addEventListener("unhandledrejection", (e) => {
  const key = window.__codexPlusUserScripts.currentKey;  // 仅在初始化窗口内归属
  if (key) markFailed(key, e.reason);
});
```

注意：只在 `currentKey` 非空（即脚本正在初始化）时归属，否则会把页面自身的 rejection 算到脚本头上。异步阶段的错误由脚本自己 `codexPlus.fail(err)` 上报。

**(b) 调用超时**

`codexPlus.call` 默认 30s 超时，脚本可覆盖。超时只 reject，不取消服务端任务（桥接协议没有 cancel 通道，加 cancel 要动 Rust 侧，留到 apiVersion 2）。

**(c) 清理契约可观测**

管理页 inventory 里给每个脚本加 `hasCleanup: bool`。没有清理函数的脚本，用户点热重载前能看到「该脚本不支持热重载，将刷新页面」的提示，而不是事后才发现整页闪一下。

### 3.6 类型定义与文档

新增 `types/codex-plus-extensions.d.ts`，随仓库分发，作者 `/// <reference types="..." />` 即可获得补全。内容从 `capabilities.json` 的路由表生成，保证与实现同步。

文档新增 `docs/extension-api.md`，包含：快速上手模板、完整 API 参考、capability 表、热重载契约、迁移指南。

---

## 四、实施顺序

分四步，每步独立可发布、可回滚。

**Step 1 — 适配层与版本号（无破坏）**
- 新增 `assets/inject/extension-api/adapter.js`，作为 `RUNTIME_SCRIPT` 的一部分下发
- 挂 `window.codexPlus`，实现 `call` / `log` / `onCleanup` / `fail`
- `/user-scripts/list` 返回 `api` 字段
- **旧全局名保持不变**，旧脚本零影响
- 测试：`crates/codex-plus-core/tests/extension_api.rs`，沿用 `user_scripts_reload.rs` 的 Node `vm` harness 风格

**Step 2 — capability 声明与白名单**
- Rust 解析头部注释块，进 inventory
- 适配层按 capability 拦截
- 管理页展示每个脚本的 capability 徽章 + `legacy` 标记
- 迁移 `conversation-canvas` 加声明块作为示范

**Step 3 — 生命周期补强**
- 异步错误归属、调用超时、`hasCleanup` 上报
- 管理页热重载前提示不支持清理的脚本

**Step 4 — 类型与文档**
- 生成 `.d.ts`，写 `docs/extension-api.md`
- AGENTS.md 补「路由只增不改」约定

---

## 五、需要确认的开放问题

1. ~~**`sha256` 到底校验没有？**~~ **已解决**：确认从未校验，且 CHANGELOG 记录移除 checksum 阻断是有意为之，故 2026-09-30 将该死字段整体移除。
2. **`/llm-proxy` 是否应该对脚本开放？** 它是高价值能力也是高危能力，倾向默认不授予、需用户显式勾选。
3. **三处目录解析重复**是否本次一并收敛到 `app_paths.rs`？属于顺手清理，不动行为。
4. **`apiVersion` 起点用 1 还是直接标 0.x 表示不稳定？** 倾向 `1`，因为契约一旦发出就不该再随意破坏。
