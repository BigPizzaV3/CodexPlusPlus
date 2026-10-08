# 18 个 open PR 逐个审查 —— 合并前必读（2026-09-15）

审查基线：`main` @ `14edc14c`（v1.3.0）
方法：逐个读 diff + 逐 PR 隔离跑测试 + 全量顺序合并验证 + 前端 `npm test`

---

## 结论速览

| PR | 作者 | 结论 | 关键理由 |
|---|---|---|---|
| [#2186](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2186) | dongyu23 | ✅ **可合** | 26.908 RPC stub 冻结的完整适配，CI 全绿 |
| [#2187](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2187) | dongyu23 | ✅ **可合** | 皮肤重扫性能修复 + 顺带修了 Windows 基线，CI 全绿 |
| [#2196](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2196) | dongyu23 | ✅ **可合** | 端口问题修复质量最高，CI 全绿 |
| [#2197](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2197) | dongyu23 | ✅ **可合** | 窗口字段适配，Windows CI 红是 main 既有问题 |
| [#2195](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2195) | dongyu23 | ✅ **可合** | CI 全绿，测试扎实 |
| [#2192](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2192) | 130040167 | ✅ **可合** | CI 全绿，微信连接修复 |
| [#2155](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2155) | dongyu23 | ✅ **可合** | CI 全绿，0x80270254 兜底 |
| [#2183](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2183) | 2277533612 | ✅ **可合** | 已 rebase，逻辑自洽 |
| [#2172](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2172) | 0xTotoroX | ✅ **可合** | 前端断言同步更新完好 |
| [#2084](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2084) | liulinboyi | 🟡 **可合但需注意** | 你已 APPROVED，但 `atomic_write_with` 会与 #2176 冲突 |
| [#2137](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2137) | zzr767299 | 🟡 **需先 rebase** | 基于 26 commit 前的 main，且与 #2197 同域 |
| [#2202](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2202) | czw2591686773-ui | ❌ **需修改** | 2 个新测试在 macOS 上必挂；含硬编码本机路径 |
| [#2168](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2168) | 0xTotoroX | ❌ **需补一处** | 版本号升到 2.0.8 但断言仍写 2.0.7 |
| [#2176](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2176) | 130040167 | ⚠️ **有共享文件冲突** | 功能没问题，但与 #2187/#2174/#2192 改同一组测试 |
| [#2174](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2174) | dongyu23 | ⚠️ **有共享文件冲突** | 同上；且与 #2178 功能完全重复 |
| [#2178](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2178) | dongyu23 | ❌ **建议关闭重开** | 7 个 commit 含 2 次 Revert；重复 #2174；**偷偷缩小了 CI 测试范围** |
| [#2159](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2159) | yytw21 | ✅ **可合** | 6 行改动，但需补交叉检查 |
| [#1770](https://github.com/BigPizzaV3/CodexPlusPlus/pull/1770) | Story19240 | 🟡 **需 rebase** | 基于 21 commit 前，缺 `tools` 字段无法编译 |

---

## 一、必须先解决的两个全局问题

### 问题 1：main 分支自己在 Windows 上是红的 🔴

**这是理解这批 PR 的钥匙。**

`pr-build.yml` 的 `windows-artifacts` job 跑 `cargo test --workspace`，而 main 上稳定失败 4 个：

```
test codex_home::tests::removal_guard_rejects_filesystem_root ... FAILED
test settings::tests::settings_store_load_bad_json_returns_default ... FAILED
test settings::tests::settings_store_load_missing_file_returns_default ... FAILED
test settings::tests::settings_store_save_load_roundtrip_uses_custom_path ... FAILED
```

我确认过：`main` @ `14edc14c` 的 `pr-build.yml` 运行确实是 `failure`（9/13、9/14 连续红）。

两个根因：

1. **`ensure_safe_recursive_removal` 的根路径判定在 Windows 上失效**。当前是 `target == Path::new("/")`，但 Windows 上 `Path::new("/")` 不是绝对路径，normalize 后会变成当前盘符根（如 `C:\`），相等比较拦不住。**即：Windows 上递归删除盘符根可以绕过守卫。**
2. **三个 settings 测试是过期断言**。tool-shards 功能引入后 `load()`/`save()` 都会把扁平字段镜像进 `tools.codex`，期望值仍是裸 `BackendSettings::default()`。

**后果**：**5 个不同的 PR（#2187 #2192 #2176 #2174 #2178）各自独立地修了这同一组测试**。它们的改法互不兼容，全都会改 `codex_home.rs` 和 `settings.rs` 的同一批行。

**建议**：先把这两个问题在 main 上单独修掉（一并解决能关掉 #2146 的硬校验诉求），之后这批 PR 里重复的测试修改全部不再需要，冲突面直接归零。

### 问题 2：#2178 把 CI 测试范围从全量缩到了单文件 🔴

```diff
  - name: Rust tests
-   run: cargo test --workspace
+   run: cargo test -p codex-plus-core --test relay_config
```

这是 `.github/workflows/pr-build.yml` 的改动，混在一个名叫「修复 Windows 测试的路径与设置断言」的 commit 里。**合并后 99% 的 CI 覆盖会静默消失。**

我核实过：#2178 的最终 head（`1eda1e27`）确实带着这一行。这是本次审查里唯一一个我认为必须拦下的改动。

---

## 二、合并冲突地图（实测）

我做了真实的顺序合并验证。按「小改动优先」的顺序合：

```
OK       pr2183      OK       pr2197      OK       pr2155
OK       pr2159      OK       pr2196      OK       pr2137
OK       pr2172      CONFLICT pr2174      OK       pr1770
OK       pr2168      OK       pr2195      OK       pr2202
OK       pr2187      CONFLICT pr2178      CONFLICT pr2084
CONFLICT pr2192      CONFLICT pr2176      OK       pr2186
```

**冲突全部集中在 3 个文件**：

```
crates/codex-plus-core/src/codex_home.rs     ← #2187 #2192 #2176 #2174 #2178 #2186
crates/codex-plus-core/src/settings.rs       ← 同上（13/18 个 PR 都碰这个文件）
crates/codex-plus-core/src/relay_config.rs   ← #2174 #2178
```

**注意 `settings.rs` 被 13 个 PR 触碰**——这是全仓库最热的文件，`--theirs` 式的粗暴解决会静默丢掉功能代码。我实测中 `git checkout --theirs` 就丢掉了 #1770 的 `standard_openai_protocol` 字段和 #2084 的 `atomic_write_with` 函数，两者都只在编译期才暴露。

**冲突内容是机械的**（注释差异、`||` 条件的冗余项、相同的测试期望值），不是设计分歧——前提是问题 1 先在 main 上解决。

### 建议的合并顺序

1. **先修 main 的两个全局问题**（根路径守卫 + settings 断言）
2. 合无冲突的：#2186 #2187 #2196 #2195 #2197 #2192 #2155 #2183 #2172 #2159
3. 合需 rebase 的：#1770 → #2137
4. 处理共享文件冲突：#2176 #2174（#2178 关闭）
5. 最后合 #2084（改动最大，2035 行）

---

## 三、逐个 PR 详审

### ✅ #2186 —— 26.908 RPC stub 冻结适配（+518/-13, 5 文件）

**这是这批里技术含量最高的一个。** Codex 26.908 把消息 dispatcher 和 `AppServerRequestClient` 都藏进了不可写的 RPC stub / 模块闭包，旧的两条注入路径同时失效。

作者的两段式方案：

1. 渲染层加 `codexServiceTierDispatcherPatchable()` 探测，遇到不可写 stub 优雅跳过——**先消除崩溃**；
2. 新增 CDP Debugger 条件断点捕获：渲染层用纯文本定位算出 `sendRequest` 的断点坐标（UTF-16 计数，与 V8 语义一致）上报 bridge，bridge 用独立 CDP 会话下条件断点 `!window.__codexPlusAppServerClientClass`，命中时 `evaluateOnCallFrame` 把类构造器挂到 window。

**我的检查**：

- 断点定位用 `lastIndexOf(anchor, markerIdx)` + 220 字符距离上限，锚点漂移时会 `return null` 并记 `app_server_client_capture_locate_failed` 诊断 —— **降级路径完整**，不会影响宿主
- 条件断点保证页面重载后自动重新捕获，正常路径零暂停
- 捕获循环有 `bridge_generation_is_current` 检查和 8 小时 watch 上限，不会泄漏
- 设置门控：`codex_app_service_tier_controls` 或 `codex_app_model_whitelist_unlock` 关闭时直接 return
- 顺带移除了已证实无效的 `codex-message-from-view` 事件改写补丁
- CI：Windows/macOS×2 全绿

**唯一要注意的**：同时存在实例补丁（旧版路径）和原型补丁（新版路径）两套代码，`patchAppServerModelRequestClient` 里的逻辑现在被复制了一份到 `installCodexAppServerClientPrototypePatch`。两份实现需要保持同步——这是可接受的取舍（旧版兼容），但值得加一条注释说明。

**能关**：#2177、#2167、#2180（后两条我在 issue 里已引用此 PR）

---

### ✅ #2187 —— 皮肤长会话卡顿（+80/-6, 4 文件）

根因定位准确：`dream_skin_skin_api_bootstrap_script` 的 `MutationObserver(() => mark())` 对全文档 childList 无过滤监听，每个流式批次都触发 12 组选择器全量 `querySelectorAll` + 无条件 `setAttribute`。

修复三层：打标幂等（值不变不写）、纯文本节点批次直接跳过、结构性变更 250ms 防抖。

**我的检查**：

- 回归测试用 IIFE 边界提取 bootstrap 片段做断言，方法可靠
- 保留了 `window.__CODEX_PLUS_DREAM_SKIN_API_OBSERVER__` 契约
- **第二个 commit 顺带修了 Windows 基线**（就是问题 1 的那两个文件）——这也是它冲突的来源
- CI 全绿

**能关**：#2181

---

### ✅ #2196 —— 端口绑定失败分类 + 环境变量挪端口（+347/-44, 10 文件）

**这批里工程质量最高的一个。**

根因：Windows 上 Hyper-V/WSL 开机会把动态端口范围（49152-65535）里的段划进排除区间，57321 落在里面时 bind 报 os error 10013。这不是 `AddrInUse`，过去既不重试也不解释，而且配置持久化在协议代理模式，之后每次启动都撞同一堵墙。

三处改动都很扎实：

1. **错误分类**：`error_is_bind_forbidden()` 区分「被占用 / 被系统保留 / 其他」，被保留时立即失败（不烧 6 秒重试预算）并给出 `netsh` 检查、重启、换端口三条出路；无关错误原样冒泡**不误贴标签**
2. **`CODEX_PLUS_PROTOCOL_PROXY_PORT` 环境变量**：沿用 `CODEX_PLUS_GUARD_PORT` 的既有模式，写入 base_url 与检测配置的**全部 14 处**调用点统一走 `protocol_proxy_port()`，管理端同步替换。我 grep 确认没有遗漏
3. **测试**：新增独立测试文件 `protocol_proxy_port_env.rs`，验证环境变量覆盖后写入侧与检测侧一致；新增两个 launcher 测试验证分类报错与错误冒泡

**我的检查**：

- 新增的 `a_windows_reserved_protocol_proxy_port_fails_fast_with_actionable_advice` 在 **macOS 上会挂**——因为 `describe_helper_bind_failure` 里用 `cfg!(windows)` 做了运行时分支，macOS 走的是通用文案，测试却断言 Windows 专属文案。这个测试应该加 `#[cfg(windows)]`，或者把断言放宽到跨平台共有的部分（`"绑定被系统拒绝"`）
- 除此之外逻辑干净
- CI 全绿

**建议**：合并前请作者修一下这条测试的 cfg 门，或者合并时顺手改（一行）。

**能关**：#2189、#2165、#2076

---

### ✅ #2197 —— `max_context_window` 适配（+133/-18, 8 文件）

**核心洞察**：`max_context_window` 是 codex 运行时的 clamp 权威（openai/codex#19185 官方确认），取 `context_window` 会把模型真实能力写低。官方 gpt-5.6 / gpt-6 目录是 `272000/872000`，而内置模板此前两个字段都是 272000。

改动分四块，都克制的：

1. 粘贴导入：`max_context_window` 优先于 `context_window`，且窗口字段**不再混入 metadata map**（避免残留值反向覆盖界面配置）
2. 生成器：未显式配置窗口时保留官方模板的 max 上限；显式配置时两字段同值——**产品语义不变**
3. 内置模板对齐官方：`gpt-5.6-sol/terra/luna`、`gpt-6-astra` 的 max 272000→872000
4. UI 侧 `synchronizeModelMetadataDocumentContextWindow` 同步维护两个字段

**我的检查**：

- 显式配置时两字段同值的语义有测试守护（`overridden["max_context_window"] == 200_000`）
- `apply_model_metadata_overrides` 把 `max_context_window` 加进保护字段列表，防止历史残留值覆盖——这一条很关键
- CI 的 Windows job 红是 main 既有问题（问题 1），非本 PR 引入
- 前端 194 测试全过

**能关**：#2191

---

### ✅ #2195 —— 获取模型报真实失败原因 + 请求超时（+497/-27, 4 文件）

根因：智谱 `/api/v1` 在 key 缺失或无效时返回 **HTTP 200 + 业务错误信封**，而 `fetch_models_from_source` 只认 HTTP 状态码，真因被静默吞掉。作者实测复现了这个行为。

修复：识别业务错误信封、报出真实原因、加请求超时。测试新增 142 行覆盖这类响应形态。

CI 全绿。**能关**：#2190

---

### ✅ #2192 —— 微信连接 CLI 路径（+116/-19, 5 文件）

两个独立问题：修 1）修改 CLI 路径后运行中的连接仍用旧路径；修 2）CLI 不存在导致失败后状态卡在 error，界面显示「启动」但再启动被「连接已在运行」拦截。

方案：`WeixinCodexPath`（`Arc<Mutex<String>>`）在消息边界检测路径变化，变化时关闭旧 app-server 再应用；`save_settings` 成功后同步更新 runtime；失败后状态从 `error` 改为 `retrying`，允许下一条消息重试。

**我的检查**：`apply()` 的 trim 语义有测试覆盖（`" C:/new/codex.exe "` → `"C:/new/codex.exe"`，空白 → `""`）；App.tsx 侧选择路径后自动保存。

**注意**：它也带了 Windows 基线修复（commit 2），所以冲突。

---

### ✅ #2159 —— 官方模式清理 relay 上下文限制（+6/-0, 2 文件）

6 行改动，把 `model_context_window` 和 `model_auto_compact_token_limit` 加进 `clear_relay_config_to_home_with_auth` 的移除列表。测试同步加了断言。逻辑正确。

**需要你也确认**：从 256K 中转切回官方后，这两个键被删掉是恢复官方默认——但如果用户自己手动设过这两个值（不是中转写的），切回官方时会一并丢失。这个取舍我认为合理（官方模式就该用官方配置），但值得在心里过一遍。

---

### ✅ #2155 —— 激活失败回退直接执行（+39/-16, 5 文件）

在已合并的 #2154 之上追加：`activate_packaged_app` 失败时不再 `?` 直接抛错，而是记一条 `launcher.packaged_activation_fallback` 诊断日志后落到 `Process` 分支直接启动 exe；`LaunchStatus` 新增 `aumid` 字段，`latest-status.json` 直接可定位真实 AUMID。

**我的检查**：`match` 改造后 `Ok` 分支的 `return` 与 `Err` 分支的 fall-through 语义正确——`Err` 会继续执行到函数末尾的通用 `Process` 启动路径。所有构造点（launcher、commands.rs）都补了 `aumid: None`。CI 全绿。

---

### ✅ #2183 —— Kimi K3 adaptive thinking（+24/-3, 2 文件）

`kimi-k3` 拒绝 `thinking.type = "enabled"`（只接受 `adaptive`/`disabled`）。新增 `kimi_thinking_enabled_type()`，并补了 `is_kimi_coding_model` 对 `kimi-k3` 的识别。

**我的检查**：测试断言从 `"enabled"` 改为 `"adaptive"` 覆盖了 k3 路径，同时补了 `kimi-k3` 显式的用例。已 rebase 到最新 main。

---

### ✅ #2172 —— stepwise 强制中文（+16/-7, 1 文件）

把语言策略从「推断输入语言」改成「强制简体中文」，并明确保护英文专有名词、代码、路径、命令。新增的 `assert!(system.is_ascii(), "Model instructions must be written in English")` 是个好约束。

**注意**：这是**产品行为变更**，不是纯 bug 修复——以前会跟随对话语言输出。如果这是你要的（看图里 #1462 的反馈，是），就合；如果还想保留多语言能力，需要改成开关。

---

### 🟡 #2084 —— 历史会话流式修复（+2035/-291, 13 文件）

你已在 9/09 APPROVED，现在 `MERGEABLE/UNSTABLE`，base 落后 4 个 commit。

**改动本质**：把批量 provider sync 改成两阶段流式——逐行扫描只保留轻量 rewrite plan + 元数据 + mtime + SHA-256；仅对确实需要改的文件逐文件流式改写。新增 `atomic_write_with` 流式原子写入 helper、真实进度上报 API（`scanning`/`planning`/`backing_up`/`rewriting`/`updating_indexes`/`rolling_back`/`complete`），前端移除模拟 `setInterval` 改为监听后端计数。

**我的检查**：

- 写入前重新校验源文件 SHA-256，文件被外部修改时跳过——**不会覆盖新内容**，这个设计对
- 回滚改为基于原始 `session_meta` 行的数量/顺序/hash 校验，不再缓存整份 rollout，同时恢复原始 mtime 和权限
- `vars_os()` 容错处理避免 macOS 非 UTF-8 环境变量导致 manager 崩溃
- 隔离测试 `cargo test -p codex-plus-core --test launcher` 通过

**合并时要注意**：它给 `settings.rs` 加了 `atomic_write_with`，与 #2176 的 `atomic_write` 调用点在同一个区域，会冲突。#2176 的 `ensure_windows_sandbox_usable_for_current_user` 需要改成走 `atomic_write`（这个函数在新版里仍然存在，只是内部改调 `atomic_write_with`），所以是机械冲突。

**能关**（合完让用户复测）：#1424、#1366、#1465、#2080

---

### 🟡 #2137 —— 自定义 Responses provider catalog（+3/-0, 1 文件）

改动只有 3 行：给 `apply_model_catalog_to_config` 的「跳过生成」条件加一个例外 `&& !(custom_responses && profile.has_model_routes())`。

**问题**：

1.  **base 落后 26 个 commit**（`be6a4585`）。`custom_responses` 的定义在 #2136 合入时已经换掉了，直接合会冲突
2.  **与 #2197 修改同一个函数域**——#2197 也在动 catalog 生成逻辑
3.  **PR 自身没有新增测试**（审查记录里已提过）

**建议**：让它 rebase 到最新 main，并补一条 catalog 生成的回归测试。顺序上放在 #2197 之后。

---

### ❌ #2202 —— Linux 原生可执行文件 + .deb 打包（+425/-106, 3 文件）

**代码方向没问题**，但有三处必须先修：

**1. 两个新测试在 macOS 上必挂**（我实测确认）：

```
test app_paths_linux_detects_chatgpt_executable_and_builds_it ... FAILED
test app_paths_linux_finds_codex_app_from_search_roots_avoiding_empty_opt ... FAILED
```

根因：这两个测试**没加 `#[cfg(target_os = "linux")]`**，但依赖 Linux 专属的实现分支：

- `is_supported_app_executable_name()` 的 Linux 分支（无扩展名的 `Codex`/`ChatGPT`）被 `#[cfg(target_os = "linux")]` 包住
- `executable_in_dir()` 用 `LINUX_CODEX_EXECUTABLES` 也是 Linux 专属

所以在 macOS 上 `normalize_codex_app_path` 返回 `None`，断言 `Some(...)` 必然失败。

**注意**：CI 只在 `windows-latest` 跑 Rust 测试，所以这个失败**在 PR 的 CI 里看不见**——但会在任何 macOS/Linux 开发者本地炸掉，也会在将来 CI 加 macOS Rust 测试时炸。

**2. `.deb` 打包脚本里有硬编码的本机路径**：

```bash
NODE_BIN="${NODE_BIN:-/home/czw/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node}"
PNPM_BIN="${PNPM_BIN:-/home/czw/.cache/codex-runtimes/.../pnpm}"
```

这个路径是作者的机器。虽然用了 `${VAR:-default}` 形式可以覆盖，但默认值应该改成探测 `command -v node`。

**3. `normalize_codex_app_path` 删掉了 `is_supported_app_executable_name` 快路径**，改成 `if !path.exists() { return None }` + 只对 `is_file()` 递归。逻辑上对 macOS 的 `Codex.app` 无影响，但这是个行为变更，值得在合并前确认没有依赖「传可执行文件路径进去也能解析」的调用点。

---

### ❌ #2168 —— 悬浮窗内容显隐（+30/-7, 3 文件）

修复本身对：让内容默认 `opacity: 0; visibility: hidden`，只在外壳完全展开（`[data-open="true"][data-morphing="false"]`）后显示，并禁用面板自身 CSS 过渡。新增的回归测试用正则解析 CSS 规则做断言，方法可靠。

**但它把 `SCRIPT_VERSION` 从 `2.0.7` 升到 `2.0.8`，却没更新 `cdp_bridge.rs:161` 的断言**：

```rust
assert!(script.contains("const SCRIPT_VERSION = \"2.0.7\";"));
```

我实测确认这是它 Windows CI 失败的原因（`test stepwise_runtime_bumps_version_when_reinjection_contract_changes ... FAILED`）。

**修法**：在 `cdp_bridge.rs` 里把 `2.0.7` 改成 `2.0.8`。一行。

---

### ⚠️ #2176 —— Windows 沙盒设置保留（+87/-7, 5 文件）

**功能上没问题**，两个改动：

1. `preserve_live_app_settings` 的保留键列表加入 `"windows"`——避免切换 relay 模板时覆盖用户已完成的沙盒配置
2. 新增 `ensure_windows_sandbox_usable_for_current_user()`，普通权限启动时把 `elevated` 降级为 `unelevated`；提权进程保持用户设置

**但它的 commit 3 也去修了 Windows 基线**（`codex_home.rs` + `settings.rs`），所以和 #2187 #2192 #2174 #2178 #2186 全部冲突。

**一个语义要留意**：`ensure_windows_sandbox_usable_for_current_user` 会**静默改写用户的 config.toml**（把 `elevated` 改成 `unelevated`）。这在普通权限下是合理的（否则启动不了），但建议加一条诊断日志，让用户知道配置被改了。

---

### ⚠️ #2174 —— 服务模式一直加载（+99/-11, 8 文件）

**功能上有两个真修复**：

1. **桥接重注入丢失未完成回调**——旧实现直接替换 `window.__codexSessionDeleteCallbacks`，正在等待的 Promise 永远 pending，界面永久停在「正在读取」。修复是重注入时先把旧 resolver 全部 reject，并保留请求序号避免编号冲突。**这个根因分析很准。**
2. **超时兜底**——为 Codex 应用设置、状态回退、config.toml 的 `service_tier` 读取各加 5 秒超时

**但它同时包含了 #2178 的全部功能代码**（`multi_agent_version` + `preserve_missing_table_keys`），而两者对同一函数的语义**断言相反**：

| | #2174 | #2178 |
|---|---|---|
| 测试 | `apply_relay_profile_preserves_live_multi_agent_features` | 同名 |
| `config_contents` | `"model = \"gpt-5.6-sol\"\n"`（模板里没有 features） | `"...\n[features]\nmulti_agent_v2 = false\n"`（模板里写 false） |
| live 里 `multi_agent_v2 = true` | 断言结果 `Some(true)` | 断言结果 `Some(false)` |

**#2174 的断言是对的**：`preserve_missing_table_keys` 只在 target 缺失该键时才插入 live 值——#2178 的测试构造了模板已含 `multi_agent_v2` 的场景，此时应该保留模板值，但它却断言 live 值获胜，与实现矛盾。

**结论**：合 #2174，不要合 #2178。这也是我建议关闭 #2178 的技术理由之一（另一个是 CI 缩小问题）。

**注意**：`settings.rs` 的改动（超时相关）与 #2187/#2192/#2176 冲突。

---

### ❌ #2178 —— 建议关闭重开（+74/-5，但 7 个 commit）

**这个 PR 应该重开一个干净的。**

问题清单：

1. **7 个 commit 里包含 2 组 Revert**：
   - `chore: bump version to 1.3.0` → `Revert "chore: bump version to 1.3.0"`
   - `修复 Windows 测试的路径与设置断言` → `Revert "修复 Windows 测试的路径与设置断言"`

   这个 commit 历史合并进 main 会永久污染 `git log`。

2. **功能代码与 #2174 完全重复**（我实测过两个 PR 的 `model_suffix.rs` 和 `relay_config.rs` diff，`multi_agent_version` 和 `preserve_missing_table_keys` 两处一模一样）。

3. **测试断言与实现矛盾**（见上表，#2174 的版本才对）。

4. **🔴 缩小了 CI 测试范围**（见问题 2），这是必须拦下的。

5. **顺带改了 `scripts/installer/macos/package-dmg.sh`** 的 hdiutil 重试逻辑——这个改动本身是好的（DMG 转换重试从 5 次/递增退避改成 12 次/固定 5 秒），但它属于另一个 PR 的主题。

**建议**：关闭 #2178，把 `package-dmg.sh` 的 DMG 重试改动单独提一个 PR（那个改动是有价值的），其余功能由 #2174 覆盖。

---

### 🟡 #1770 —— 纯标准协议开关（+193/-8, 9 文件）

**功能设计质量不错**：

- 纯 opt-in：`#[serde(rename = "standardOpenaiProtocol", default, skip_serializing_if = "is_false")]`，关闭时不写 JSON，导出 round-trip 字节一致
- `ChatReasoningStyle::Default` 只停发厂商私有方言（`reasoning_split`/`thinking`/`enable_thinking`/OpenRouter `reasoning`），**保留标准 `reasoning_effort`**
- 新增测试 `responses_request_standard_protocol_strips_vendor_reasoning_dialects` 覆盖开/关两条路径
- UI 开关文案清楚

**但 base 落后 21 个 commit**（`e3baac32`），我实测**无法直接合并编译**：

```
error[E0609]: no field `tools` on type `&mut BackendSettings`
error[E0609]: no field `standard_openai_protocol` on type `&RelayProfile`
error[E0560]: struct `RelayProfile` has no field named `standard_openai_protocol`
```

它的 `settings.rs` 版本还没有 `tools: BTreeMap<ToolId, ToolConfig>` 字段（tool-shards 功能之后才加的）。**必须 rebase。**

**另外**（上次审查已提过）：`ccs_import.rs` / `provider_import.rs` 里只各补了一行 `standard_openai_protocol: false` 字面量——**导入路径不会让用户打开这个开关**，只能在管理器 UI 里手动勾。要么补上导入路径的支持，要么在 PR 描述里说清这个限制。

**能关**：#1407（合完让用户复测）

---

## 四、可关闭的 issue（合并后）

合完这批 PR，这些 issue 可以直接关：

| issue | 由哪个 PR 关闭 |
|---|---|
| #2177, #2167, #2180 | #2186 |
| #2181 | #2187 |
| #2189, #2165, #2076 | #2196 |
| #2191 | #2197 |
| #2190 | #2195 |
| #1407 | #1770 |
| #1424, #1366, #1465, #2080 | #2084（让用户复测后再关） |

---

## 五、建议的执行顺序

1. **先修 main 的两个全局问题** —— Windows 根路径守卫 + 3 个 settings 过期断言。修完之后这批 PR 的冲突面直接归零
2. **合 CI 全绿且无冲突的 10 个**：#2186 #2187 #2196 #2195 #2197 #2192 #2155 #2183 #2172 #2159
3. **让 #1770 rebase** 到最新 main 后合（顺带要求补 UI 导入路径说明）
4. **让 #2137 rebase**到含 #2197 的 main 后合，要求补回归测试
5. **关闭 #2178**，把 `package-dmg.sh` 的 DMG 重试改动单独提一个 PR
6. **处理 #2176 #2174** —— 功能都保留，只是共享文件要手工解冲突（#2174 的超时修复优先）
7. **修 #2168 的一行断言**（`cdp_bridge.rs` 的 `2.0.7` → `2.0.8`）后合
8. **修 #2202** —— 给两个 Linux 测试加 `#[cfg(target_os = "linux")]`，去掉硬编码的 `/home/czw/...` 路径
9. **最后合 #2084** —— 你已经 APPROVED，改动最大，放最后避免反复解冲突

要我按这个顺序开始做吗？合并前我会停下来问你（按你之前的规矩）。
