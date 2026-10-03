# CodexPlusPlus 最新 100 条 issue 审计

- 日期：2026-10-03
- 仓库：`/Users/mac/Desktop/CodexPlusPlus`
- 基线：`main` @ `2329b653`（审计开始于 `89615978`，期间 `c509c06d` #2313、`2329b653` 两笔已落 main）
- 范围：`gh issue list --limit 100 --state all`，即 **#2217 ~ #2379**，共 100 条
- 与上一轮的关系：上一轮报告 `docs/reports/2026-10-02-issue-audit-and-fix-plan.md` 覆盖 116 条，其中包括本批 100 条里的 70 条。**本报告只审这 70 条的增量状态变化 + 尚未覆盖的 30 条**，不重复上一轮已确认的结论。

---

## 1. 摘要

| 项 | 数量 |
|---|---|
| 本批 issue 总数 | 100 |
| 上一轮已覆盖 | 70（本报告只做增量核对） |
| **本轮新审** | **30** |
| 其中 OPEN | 12 |
| 其中 CLOSED | 18 |

**新审 30 条的结论分布：**

| 结论 | 条数 | issue |
|---|---|---|
| 已支持 / 可直接关闭 | 4 | #2284 #2355 #2221 #2263 |
| 已在 main 修复（含 v1.5.0 未发布者） | 7 | #2242 #2247 #2263 #2264 #2323 #2324 #2332 #2326 #2327 |
| not-our-bug | 6 | #2379 #2295 #2298 #2321 #2328 #2357 |
| confirmed-in-code（需动手修） | **4** | **#2329 #2339 #2376 #2322-后续** |
| 产品讨论 / 咨询（不动代码） | 3 | #2312 #2289 #2253 |
| Feature 待做 | 1 | #2228 |
| insufficient-info | 2 | #2246 #2254 |
| 静默关闭需复查 | 1 | #2248 |

**本轮最重要的四条结论：**

1. **`#2362`（P0 安装器竞态）修在 main、不在任何正式版里。** 修复提交 `8610d2f3` 是 main 的直系祖先，但不在 `v1.5.0`（v1.5.0 tag 早于它约 25 小时）。issue 已由 PR 合并消息**自动关闭且零条评论、零验证**。这是本批唯一一条「代码已修 / 效果未证实 / 且尚未发布」的条目 —— **下次发版必须包含它**，否则这条 P0 对全部存量用户持续开放。
2. **`#2322` 有一条被吞掉的回归信号。** 报障人 Reitzzz 在 2026-09-28T08:53:38Z 回帖「同一份 v1.4.0，升级上来的仍卡死、彻底卸载重装后正常」—— 这是一次干净的对照实验（唯一变量是残留状态）。该回帖发出后 **33 秒** issue 就被关闭，维护者没回应这一点。其指向的 `dispatcher` 模块发现缺兜底在 main 上**找不到对应修复**。
3. **`#2329`（强制中文失效）三个子根因里两个成立、一个需要更正，且全部未修。** 上报质量很高（附 CDP 探针与实测证据），值得按它给的方案动手：(a) 补丁只包了 Statsig 的 `getDynamicConfig`，没有 `getLayer`；(b) 用户脚本注入走 `DOMContentLoaded` + IPC + CDP eval，比 issue 描述的更晚；(c) 端口确实不重解析 —— 但「12 次重试」是误读，main 是 120 次 × 1s。
4. **上一轮报告的 §4.1 / §8 需要回填。** 报告当时要求「先合 PR #2337 再把改动搬进分片，否则 assemble 会抹掉」—— 这一步已经由 `5bb4f636`（直接改分片）落地。`PR #2337` 本体是 CLOSED 未合。凡是引用 §4.1 / §8 的执行清单，都应按「已落地」处理。**注意：该提交落于 2026-10-02，晚于 v1.5.0 的 tag（2026-10-01），因此不在任何正式版里，需等下一次发版。**

---

## 2. 需要动手修的（confirmed-in-code）

### 2.1 `#2329` —— 强制中文失效，三个独立根因（P1）

作者 `aliuzq` 给了 CDP 只读探针 + 日志的实测证据，是本批质量最高的报障。逐条核实如下。

#### (a) 补丁打错接口 —— **成立**

`assets/inject/renderer-inject/00-prelude.js:300-312`：

```js
const patchStatsigClient = (client) => {
  if (!client || typeof client !== "object") return;
  if (typeof client.getDynamicConfig !== "function") return;
  if (!client.__codexPlusForceChineseLocalePatched) {
    const originalGetDynamicConfig = client.getDynamicConfig.bind(client);
    client.getDynamicConfig = (name, options) => {
      const result = originalGetDynamicConfig(name, options);
      return name === "72216192" ? patchI18nConfig(result) : result;
```

`patchI18nConfig`（`:268-289`）改写的是 `dynamicConfig.value.enable_i18n` 与 `dynamicConfig.get`。全仓 grep `useLayer` / `_getLayerImpl` / `getLayer` / `NoValues` 在 `assets/` 与 `crates/` 下**零命中**，即注入层只包了 `getDynamicConfig`，Layer 通道完全没有补丁 —— 与 issue 描述一致。

值得警惕的是：`crates/codex-plus-core/tests/force_chinese_locale_settings.rs:84` 把 `script.contains("72216192")` 固化成断言，**这条断言锁的正是「补 DynamicConfig」这个错误接口**。改接口时必须同步改测试，否则会被自己的测试挡住。

另一条独立通道其实是真的在工作：`syncOfficialLocaleSetting()`（`00-prelude.js:199-240`）走 `vscode://codex/get-setting|set-setting` 把 `localeOverride` 写进官方设置并 `window.location.reload()`。所以「强制中文」目前实际靠的是**官方 localeOverride + navigator.language 伪装**，Statsig 那条路是冗余的 —— 这也解释了 issue 证据 1 的现象：探针看到 `layerValue.enable_i18n = true`（用户脚本自己补的）但界面仍英文。

**修法**：按 issue 附的片段把 patch 从 `getDynamicConfig` 扩到 `getLayer` / `_getLayerImpl`（往返回 layer 的 `__value` 里 `Object.assign`），保留现有 DynamicConfig 分支做双保险。

#### (b) 用户脚本注入竞态 —— **成立，且比 issue 描述的更早**

`assets/inject/user-scripts-bootstrap.js`（全文 15 行，由 `crates/codex-plus-core/src/user_scripts.rs:12` 用 `include_str!` 引入）：

```js
const load = () => {
    window.__codexSessionDeleteBridge("/user-scripts/load", {}).then((result) => {
      if (result?.status === "failed") console.warn("[Codex++] user scripts:", result.message);
    }).catch((error) => console.warn("[Codex++] user scripts:", error));
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", load, { once: true });
  else load();
```

服务端侧 `crates/codex-plus-core/src/routes.rs:194` → `runtime.load_user_scripts()` → `user_scripts.rs:23-35` → **`crate::bridge::evaluate_script(websocket, bundle)`**。

所以链路上有**三道**延迟：① `DOMContentLoaded`（不是 document-start）；② 一次 IPC 加一次磁盘扫描；③ 再走 CDP `Runtime.evaluate`。落在应用首读 Layer 之后是必然的。

**关键对照**：注入层自身的宿主脚本走的就是 document-start —— `crates/codex-plus-core/src/bridge.rs:486-495` 用 `Page.addScriptToEvaluateOnNewDocument` 注册 `build_bridge_script`，`:506-521` 对 `new_document_scripts` 逐条同时做 `addScriptToEvaluateOnNewDocument` + `Runtime.evaluate`。**同步通道在 `bridge.rs` 里已经存在，只是内置 force-locale 补丁没走它。**

**修法**：内置 force-locale/Layer 补丁改走 `add_script_to_new_documents`（`bridge.rs:262` 现成函数）；用户脚本的异步读盘通道保留给热重载，但首次注入不应走它。

#### (c) 自愈探测死端口 —— **部分成立，一处数据要更正**

端口确实不重解析：

- `launcher.rs:515` `let debug_port = hooks.select_debug_port(options.debug_port);` —— 在 `launch_and_inject_with_hooks` 开头算一次；
- `launcher.rs:826-827` → `crate::ports::select_packaged_codex_debug_port(requested)`，`ports.rs:85-97`：

```rust
if !is_windows || can_bind(requested) || is_existing_cdp(requested) {
    requested
} else {
    find_available()
}
```

- 这个值随后只被**传参**使用：`:628` `launch_codex(...)`、`:636` `ensure_injection(...)`、`:646` `start_bridge_watchdog(...)`，全程没有任何地方重新读进程命令行里的 `--remote-debugging-port`。

全仓 grep `remote-debugging-port` 只在两处出现：`launcher.rs:2672`（**拼启动参数**用）与 `watcher.rs:776`（仅 macOS 存活判定用，不回流）。所以「Windows 上 Codex 自重启换了端口，Codex++ 仍只认启动时那个」在代码上进得去 —— 62801 正是 `find_available()` 在 9229 被占时另选的端口。

**需要更正的一处**：issue 说「12 次重试全部 10061」。main 上是 `launcher.rs:278-294`：

```rust
for attempt in 1..=120 {
    let result = match self.bridge_context(debug_port, app_dir).await { ... };
    match result {
        Ok(()) => return true,
        Err(error) => { /* append_diagnostic_log */
            tokio::time::sleep(std::time::Duration::from_secs(1)).await; }
```

上限是 **120 次 / 1 秒间隔**；issue 抓到的 `attempt: 12` 只是日志里的中间一行。另外 inspector 端口也不是随机的：`launcher.rs:715-718` `select_native_menu_inspector_port` = `debug_port + 100`，62801→62901 这一对自洽，佐证启动端口确实是 62801。这一点不影响修复方向，只影响「自愈要等多久」的量级判断。

**修法**：在 `ensure_injection` 重试循环内（或 `bridge_context` 里）加一次「从进程命令行解析 `--remote-debugging-port` / 探测 `/json/version` 反查真实端口」的回退，命中后更新 `debug_port` 并同步给 `start_bridge_watchdog`。修 (c) 独立于 (a)(b)，是三者里成本最低的。

#### 顺带：PR #2337 的状态更正

`gh pr view 2337` → `state: CLOSED`、`mergedAt: null`，**不是合并进来的**。实际落地提交是 **`5bb4f636`** `fix(bridge): 窗口隐藏不判定桥接失效 + 退避持续健康才清零，根治每分钟整页重注入`，是 main 的直系祖先，但**不在 v1.5.0 内**（提交于 2026-10-02，晚于 v1.5.0 的 tag）——需等下一次发版。

上一轮报告担心的「只改产物、会被 assemble 抹掉」**已经不存在**：该提交直接改的是分片，`assets/inject/renderer-inject/20-menu.js:99-107` 现为

```js
const codexAppModuleFailures = window.__codexPlusAppModuleFailures || (window.__codexPlusAppModuleFailures = new Map());
const codexAppAssetUrlLookups = window.__codexPlusAssetUrlLookups || (window.__codexPlusAssetUrlLookups = new Map());
```

`document.visibilityState === "hidden"` 早返回在 `bridge.rs:149`，位置正确（在 `:144` 的 bridge/health 存在性检查之后）。退避常量 `launcher.rs:33`：`BRIDGE_REINJECT_BACKOFF_RESET_AFTER_SECS = 60`。

**处置**：上一轮报告 §4.1 与 §7 第 1 批第 1 条应改为「已由 `5bb4f636` 落地，无需再搬分片」；但发版提醒要加一条——该提交不在 v1.5.0 内。

### 2.2 `#2339` —— DreamSkin 主题包 `skinApiVersion` 门限停在 1.5.12（P2）

`crates/codex-plus-core/src/dream_skin_package.rs:920-924`：

```rust
if compare_semver(&manifest.min_client_version, "1.5.12").is_gt() {
    bail!(
        "主题包需要更新版本的 Dream Skin 协议：{}",
        manifest.min_client_version
    );
}
```

常量硬编码，未参数化；`Cargo.toml:11` 当前 `version = "1.5.0"`。`：908` 另有一道 `skin_api_version != 1` 的整数门。issue 里描述的现象（客户端停在 1.5.12、装不了声明 1.5.18 的主题包）与代码现状逐字吻合。

**修法（二选一，建议前者）**：给市场清单侧加 `minClientVersion` 过滤，避免用户点完安装才知道不兼容；对齐上游 1.5.18 是更大的动作，需要同步 `assets/inject/upstream/dream-skin/` 整套资源与 `crates/codex-plus-core/tests/upstream_theme_assets.rs` 的 sha256 锁定。

### 2.3 `#2376` —— 每次安装/更新都无条件往桌面堆图标（P2，Feature）

`scripts/installer/windows/CodexPlusPlus.nsi:82-83`：

```nsis
CreateShortcut "$DESKTOP\Codex++.lnk" "$INSTDIR\codex-plus-plus.exe" "" "$INSTDIR\codex-plus-plus.exe"
CreateShortcut "$DESKTOP\Codex++ 管理工具.lnk" "$INSTDIR\codex-plus-plus-manager.exe" "" "$INSTDIR\codex-plus-plus-manager.exe"
```

同一条路径在 `crates/codex-plus-core/src/install/windows.rs:69-88` 也有一份（`install_shortcuts` → 两次 `create_entrypoint_shortcut`），**两边都是无条件创建**：没有开关，没有「桌面原本没有就不加」的检测。

**修法**（按用户给的三选一，建议第 2 条）：`IfFileExists` 判断目标 `.lnk` 是否已存在 —— 只在首次安装时创建，覆盖升级时若用户删过就不再补。注意 NSIS 段与 Rust `install_shortcuts` 要同步改，否则管理器触发的「安装/修复」路径会把图标加回来。

### 2.4 `#2322` 的后续（P1，建议新开 issue，不要重开原单）

见 §4.2，这里不重复。

---

## 3. 可为已支持直接关闭（4 条）

| issue | 结论 | 证据 | 建议回复 |
|---|---|---|---|
| **#2284** | 已支持，是 #2281 的原始 feature request | `assets/gpt6-sol-luna-model-metadata-compat.json` 已含 `gpt-6-sol` / `gpt-6-luna` 两条 slug（档位 low→max，Sol 另有 `ultra`，`context_window` 272000 / `max_context_window` 872000）；`model_suffix.rs:201` 声明常量、`:314` 挂进 `COMPATIBILITY_METADATA_SOURCES`；落地提交 `7e19e069`（2026-09-23） | 已在 #2281 补全并随 v1.4.0+ 生效，无需新代码。若你的模型列表里仍看不到，贴一份 `model_list` 原文。 |
| **#2355** | 已支持，由 `ea82bc74`（PR #2366，2026-10-02）覆盖 | `App.tsx:8620-8627` `DndContext` + `SortableContext items={modelWindowRows.map(...)}`，拖拽手柄 `.relay-model-drag`（`:8662-8671`），落盘 `handleModelRowsDragEnd`（`:8000-8015`）→ `reorderModelWindowRows`（`model-windows.ts:48-66`）→ `serializeModelWindowRows` 写回 `modelList` | 已在 v1.6 前的 main 上支持，请升级后复测；作用域是**模型行**（供应商列表另有 `App.tsx:7271-7288` 的一套）。 |
| **#2221** | already-fixed，`82fb0924`（2026-09-17） | 引擎实际在 `90-action-groups.js`（不是 95 分片）：调度器 `:959-971` 改为 rAF 收敛帧、删掉旧的 350ms `setInterval` 轮询；清理 `:973-988` `cancelAnimationFrame` + `mo.disconnect()` + `ro.disconnect()`；关闭开关传播走 `40-backend-settings.js:23-38` 的 5 秒心跳比对 | 已修复并随 v1.4.0 发布。若仍占 CPU，请给 `codexPlusSettings().conversationView` 的读回值与心跳日志。 |
| **#2263** | already-fixed，`0fd58cc3` + `994706f5`（2026-09-21） | `protocol_proxy.rs:4156-4177` `tool_search` 透传为 chat 的 function 工具；`:4153` namespace 变体；`:4038-4056` `CodexCustomToolKind::ToolSearch` 登记；`:3201-3224` 入向往还原；`:5089-5236` 出向经 `is_tool_search_proxy`（`:195`）分流；`:4700` `collect_tool_search_output_namespaces` | 已修复且回归测试齐备（`tests/protocol_proxy.rs:4766` 起 9 个用例）。若仍复现，重点查 `mcp_servers` 是否被 `relay_config.rs` 的表头处理二次打散。 |

---

## 4. 已关闭 issue 的复查

### 4.1 汇总表

18 条已关闭 issue 全部逐条读了评论与关闭原因。

| issue | stateReason | 关单依据 | 分类 | 备注 |
|---|---|---|---|---|
| #2242 | COMPLETED | PR #2243（`32bda12a`）修 `provider_sync.rs` 资格判定，已随 v1.4.0 发布 | A / D | 0 条评论，PR 合并自动关；修复确已发布，可留 |
| #2247 | COMPLETED | PR #2251（`98140aab`）schema 收敛点递归剥除 `type:null`，已随 v1.4.0 发布 | A / D | 0 条评论，同时刻自动关；根因分析详实，无回归 |
| #2248 | COMPLETED | **无任何评论、无关联 PR** | **D** | **纯静默关闭**，2026-09-19。正文是一句 P1 症状描述（仍耗 token 无回复）。建议复查 |
| #2263 | COMPLETED | PR #2265（`7dc7eef2`），提交 `0fd58cc3`，已随 v1.4.0 发布 | A | 诊断精确、修复完整 |
| #2264 | COMPLETED | PR #2272（`d8dd03fe`）launcher/model 对齐，已随 v1.4.0 发布 | A / D | 「重启后目标模式被静默换模型烧额度」，值得单独确认修复覆盖到了目标模式路径 |
| #2295 | COMPLETED | 官方 5 小时额度耗尽，发帖人自行确认并接受 | B | 非本仓缺陷 |
| #2298 | COMPLETED | 发帖人自答「等待十几分钟后正常启动」后自关 | B | 「等十几分钟才起来」本身像冷启动缺陷（对照 #2244），但无代码证据 |
| #2306 | COMPLETED | AUMID 硬编码失效，v1.4.0 双防线修复（`2b4d8d68` + `6c11bf7f`），已发布 | A | 维护者自查自关；留下一条**未验证的差异**（显式 `--app-path` 可启、Manager 不可启），已请用户复测未见回报 |
| #2321 | COMPLETED | 用户指出「别用 codex++ 主题皮肤」，发帖人确认可用 | B | 皮肤/主题交互，有绕过办法 |
| #2322 | COMPLETED | v1.4.0 仍卡，彻底卸载重装后正常，判为升级遗留状态 | **C** | **见 §4.2** |
| #2323 | COMPLETED | 同 #2332 根因，`8baffa91`，已随 v1.5.0 发布 | A | 重复报告 |
| #2324 | COMPLETED | 同上（英文版） | A | 重复报告 |
| #2326 | COMPLETED | `0fd58cc3`，维护者回帖说明已随 v1.4.0 发布并关单 | A | 有真实评论往来，依据充分 |
| #2327 | COMPLETED | `bed5277d`（熔断 + 指数退避），已随 v1.4.0 发布 | A | 与 #2255/#2256/#2299 同根因 |
| #2328 | COMPLETED | 纯问答：告知 v1.4.0 已发布并给下载 | B | 非缺陷单 |
| #2332 | COMPLETED | `8baffa91`，维护者独立复核后关单 | A | 用户给出一行修复 + A/B 数据，质量很高；已随 v1.5.0 发布 |
| #2357 | COMPLETED | 发帖人自答「开科学上网后问题消失」后自关 | B | 与 #2329 的 (a)(b) 同根因的现象，但本次是网络环境 |
| #2362 | COMPLETED | PR #2368（`8610d2f3`），**尚未进入任何 release** | **A（未发布）** | **见 §4.3** |

分类统计：**A 10 条、B 5 条、C 1 条、D 2 条**（#2248、#2362 各自另有 A 属性）。

### 4.2 `#2322` —— 唯一的回归信号，且被 33 秒后关单吞掉

**信号**：报障人 `Reitzzz` 在 2026-09-28T08:53:38Z 回帖 —— **「升级到 v1.4.0 仍然卡在无限转圈，但连同配置文件一起卸载重装后，同样是 v1.4.0 就正常了」**。

**为什么这条必须处理**：

- 这是一次**干净的对照实验**：同一份 v1.4.0 二进制，唯一变量是重装清掉的持久化状态（供应商配置 / 残留注入缓存）。结论「升级流程未清理旧状态」由该用户自行验证。
- 该回帖发出后 **33 秒**（08:53:39Z）issue 就被关闭，**维护者对「原地升级仍中招」这一点没有作出任何回应**。
- 更关键：报告点名的另一处根因 —— `dispatcher` 补丁在 Codex 26.924 上**模块发现缺兜底**（维护者回帖已确认「确定存在、但很可能不是卡住直接原因」）—— **在 main 上找不到对应修复**。也就是说被确认存在的代码缺陷仍在，而「从 1.3.0 原地升上来的存量用户仍会中招」这条结论没有被任何提交覆盖。

**建议**：**新开一条 issue**（不要重开原单，尊重其关闭状态），标题聚焦「1.3.0 → 1.4.0+ 原地升级遗留状态导致启动卡死」，附上 Reitzzz 的对照结论，明确两项待办：

1. 升级流程清理旧注入缓存 / 供应商残留状态；
2. dispatcher 模块发现补 fallback 扫描（与 app-server 补丁的两级发现在结构上对齐）。

### 4.3 `#2362` —— P0 已修但未发布、未验证，被 PR 合并自动关单

**准确表述：根因已在 main 上被一处针对性修复覆盖，但修复未发布、未实机验证、未获报障人确认。当前状态是「代码已修 / 效果未证实」。**

三条依据：

1. **修复确实存在且根因分析扎实。** 提交 `8610d2f3`（2026-10-02 16:43:09 +0800），改 `scripts/installer/windows/CodexPlusPlus.nsi`，新增 `WaitForProcessExit` 宏：taskkill 后 500ms×20 轮轮询、过进程再补 500ms 锁释放缓冲，然后才 `File`/`Delete`；卸载段 `Delete` 静默失败改为弹窗提示。与上一轮报告 §4.4 的根因（`nsExec::ExecToLog 'taskkill /F'` 后零等待即覆盖写，镜像文件锁释放是异步的）完全吻合。

2. **修复不在任何已发布的版本里。** `git merge-base --is-ancestor 8610d2f3 v1.5.0` → 否。v1.5.0 发布于 2026-10-01T15:00:58Z，早于该提交约 25 小时。**当前所有正式版（含最新的 v1.5.0）仍然带这个安装器竞态** —— 用户从任何已发布版本升级/重装 Windows 包，理论上仍会撞到同一个「无法写入 codex-plus-plus-manager.exe」。

3. **关单时 issue 上没有任何验证证据。** `#2362` 的 comments 数组长度为 0：没有维护者说明、没有请报障人复测。它是被 PR #2368 的合并消息自动关掉的（合并时刻 2026-10-02T08:43:10Z 与 `closedAt` 08:43:11Z 只差 1 秒）。报障人 `zwyk6` 从头到尾没有回过一句「好了」。

4. **报告自己承认只修了一半。** §4.4 末尾写明「唯一提醒：它只修『进程正在退出』这一半；建议顺手在 `File` 前用 `IfFileExists` 循环重试并把失败明确提示用户」，并列了实机矩阵（V2 双进程运行中安装、V3 进程锁死目标文件、降级路径）—— 这些在仓库里没有任何落地记录。

**建议两件事**：

- (a) **下一次 release 必须包含 `8610d2f3`**，并在 release note 里注明安装器竞态修复；
- (b) 发版后在 #2362 上追加一条「请升级复测」的留言 —— 若报障人仍复现，说明命中的是「文件被外部进程锁死」而非「进程正在退出」的那另一半。

**不建议**仅凭合并就把这条静默保持关闭且不留任何说明。

### 4.4 `#2248` —— 唯一的纯静默关闭

0 条评论、无关联 PR、2026-09-19 关闭。正文只有一句症状：更新到 1.3.0 后「照常消耗 token 却没有任何回复」。

症状与 #2322 的第三方供应商路径在现象上相邻，建议**与 #2322 放在一起复查**，很可能指向同一类供应商/注入状态问题。

---

## 5. 不是我们的 bug

### 5.1 `#2379` —— `function_call_output requires call_id ... Responses WebSocket v2`（P1 表象）

**结论：上游发出，本仓零命中。**

全仓 grep `requires call_id` / `continuation via` / `Responses WebSocket` 在 `crates/`、`apps/`、`assets/`、`services/`、`docs/` 均无结果。本仓对这两个字段只有两处处理，且都是**透传/只读**：

- `protocol_proxy.rs:1841-1852` `conversation_id_from_responses_request` —— 把 `previous_response_id` 当会话标识**读取**（回退顺序 `conversation` → `conversation_id` → `previous_response_id`），用于日志/会话关联，不修改请求体；
- `protocol_proxy.rs:6022-6044` `copy_response_request_fields` —— 把字段从原始请求**复制到响应对象**，调用点 `:399`（非流式）、`:2632`（SSE `response.completed`）。方向是响应侧。

另注 `relay_config.rs:3901` / `:5131` 强制 `provider["wire_api"] = "responses"`（注释 `:3897-3900` 说明 Codex 26.901 起不再支持 `wire_api = "chat"`）—— 这只是把客户端钉在 Responses 线上，本身不产生该错误。

**建议关闭并指向上游。** 若要在本仓缓解，唯一可控点是 relay 侧：在 Responses 直通路径上剥掉 `previous_response_id` 并改为全量 `input` 回放 —— 但那是产品决策，不是缺陷修复。

### 5.2 其余 not-our-bug

| issue | 为什么不修 |
|---|---|
| #2295 | 官方 5 小时额度耗尽，发帖人已自认 |
| #2298 | 用户自答「等十几分钟正常」，无代码证据；若要与 #2244 冷启动合并考虑可另议 |
| #2321 | 官方主题皮肤与 Codex 新版容器的交互，有绕过办法 |
| #2328 | 纯问答，已答复 |
| #2357 | 发帖人自答开代理后消失，网络环境 |

### 5.3 信息不足（补问，不改代码）

| issue | 要问什么 |
|---|---|
| **#2246** | `failed to launch Codex executable C:\Program Files\WindowsApps\OpenAI.Codex_26.915.4065.0_x64__...\app\Codex.exe`（Codex++ 1.2.12）。要 `codex-plus.log` 全文 + 当前 Codex++ 版本 —— 1.2.12 早于 AUMID 激活改造（`2a41afb` / `2b4d8d68`），**很可能只是版本太老**，先让用户升级到最新再复测，不必先动代码 |
| **#2254** | 0x80270254，与 #2306 同根因（AUMID 硬编码失效），但用户是 1.3.0、且 `settings_path` 落在 `C:\Users\ASUS\.codex-session-delete\settings.json`（异常路径，疑似用旧版/绿色版）。要「显式 `--app-path` 能否启动」的对照结果（同 #2306 的未验证差异） |
| **#2377** | 启动问题，正文 33KB 主要是 bridge 心跳日志（每 5 秒 `backend/status` + `settings/get`，全部 `resolve_ok`，**看不出异常**）。关键线索是用户自述「开了 VPN 时启动失败，关掉代理后成功」—— 需确认 `launcher.rs` 的 AUMID 激活或 `watcher` 探测是否在 `HTTPS_PROXY` 环境下走网络路径。**建议先要 `latest-status.json` 与图 2 的错误串**；另注意 `c509c06d`（#2313，2026-10-03 落 main）重写了启动/重启生命周期，很可能已覆盖，应先让用户升级复测 |
| **#2253** | 见 §6.3 |

---

## 6. 需求类与产品讨论

### 6.1 `#2228` —— 通用模板缺 `max` + 逐模型自定义思考强度

**拆成两半：**

**(a)「通用模板缺 max」不是缺陷。** 模板基座 `assets/codex-models.json` 的内置条目档位上限本就是 `xhigh`（`gpt-5.5 low..xhigh`、`gpt-5.4 low..xhigh`、`gpt-5.3-codex low..xhigh`）。`max` 由**注入层**补：`70-model-catalog.js:171-183`：

```js
const hasMax = efforts.some((e) => e.reasoningEffort === "max");
const hasUltra = efforts.some((e) => e.reasoningEffort === "ultra");
if (!hasMax) efforts.push({ reasoningEffort: "max", ... });
...
return ["low","medium","high","xhigh","max","ultra"].map(...);
```

所以渲染侧并不止于 `xhigh`，用户看到的上限取决于**该模型解析到的 metadata**。`35ede662` 只是新增 `assets/gpt61-sol-model-metadata-compat.json` 并挂进查找链（`model_suffix.rs:205` / `:525`），**没有改动任何「通用模板档位列表」** —— 因此 #2358 的修法 ≠ #2228 被修。#2358 是「gpt-6.1 Sol 这个模型的 max 被漏了」（已解决），#2228 问的是「任意第三方模型的通用模板」。

**(b)「逐个模型自定义思考强度」已有能力，只是没暴露成专用 UI。** `RelayProfile.model_metadata`（`settings.rs:83-88`，前端字段 `modelMetadata`）按 slug 覆盖任意字段，Rust 侧 `relay_config.rs:2812` `apply_model_metadata_overrides`：

```rust
for (key, value) in user_override {
    if matches!(key.as_str(),
        "slug" | "context_window" | "max_context_window" | "auto_compact_token_limit") {
        continue;
    }
    model_object.insert(key.clone(), value.clone());
}
```

`supported_reasoning_levels` **不在**排除名单里，故可逐模型写。UI 入口是模型行的「模型配置」导入面板（`App.tsx:8629` 起、`commitModelMetadata` `App.tsx:7938`；校验 `model-metadata.ts:787-799`）。issue body 说「现在只能手动改模型声明 json 文件」，与代码现状完全吻合。

**处置：Feature 待做。** 实现上只需在导入面板加一个 effort 勾选器（写回同一个 `modelMetadata` map），**后端零改动**。不要反过来在 `codex-models.json` 基座上逐个加 `max` —— 那会产生「所有第三方模型都支持 max」的错误宣称。

### 6.2 `#2355` / `#2284` —— 见 §3

### 6.3 `#2253` —— 输入框圆圈显示不正确（insufficient-info，但有一个确定的可疑点）

这是本 fork 的目标 feature（#1171 / #931）。**后端链路已完整落地**：

- `apply_context_limits_to_config` 在 `relay_config.rs:2324-2336`，写**顶层** `model_context_window`；调用点 `:531` / `:549` / `:710` / `:759`；
- catalog 生成字段名在 `model_suffix.rs:572-573`：`model["context_window"]`、`model["max_context_window"]`；
- 写盘在 `relay_config.rs:2528-2538`。

**但圆圈是 Codex 客户端按它自己读到的窗口渲染的**，本仓只能通过 `config.toml` + `model_catalog_json` 间接影响。「显示不正确」的三个候选成因（按可复现性排序）：

1. **命中外部 catalog 降级路径**：外部 `model_catalog_json` 指针存在且未被认作 Codex++ 托管时走 `apply_external_catalog_fallback`（`relay_config.rs:2339-2362`），退化成**全模型共用一个顶层 `model_context_window`** —— 这正是「圆圈显示不对」的稳定表现形态，也是三个候选里唯一能可靠复现的；
2. 该模型未进 `model_list`（只在窗口 map 里有 key）→ 不生成 catalog 条目；
3. 圆圈口径含输出上限，与配置值不是同一量。

**顺带发现一个确定的可疑点**（不在任何 issue 里）：`build_model_catalog_json_with_capabilities`（`model_suffix.rs:533-610`）**从不写 `max_output_tokens`**，而 `assets/deepseek-model-metadata.json` 为每个模型都带了 `max_output_tokens: 393216`。若 Codex 的圆圈/百分比口径涉及输出上限，这里会落空。

**处置**：先索要复现材料（`model_list`、`model_windows`、生成的 `~/.codex/model-catalogs/<id>.json` 与 `config.toml`），确认是否命中第 1 条。

### 6.4 `#2289` / `#2312` —— 不动代码

- **#2289**（无法自由切换模型）：5 条评论走向是 `Yuimi-chaya` 复现了「纯 API 下把 deepseek 官方作为独立供应商，经单模型路由接入主供应商」可行，并追问用户是否已分别配好两个供应商、是否设了单模型路由 —— **对话停在这里，用户未回复**。代码侧 `RelayModelRoute` / `has_model_routes()` 都在（`settings.rs:117` / `:289`），`should_write_managed_model_catalog` 也会因 `has_model_routes` 触发 catalog 生成（`relay_config.rs:2402` / `:2511`）。**属操作门槛而非能力缺失**，建议保持 OPEN 按支持工单跟进。
- **#2312**（统一工作流提案）：body 自述「不是功能已实现声明，也不要求一次性重写现有系统」，0 条评论。是产品方向讨论，**不需要对代码动手**，建议转 Discussion 或加 `roadmap` 标签封存。

---

## 7. 对上一轮报告的订正

上一轮报告 `2026-10-02-issue-audit-and-fix-plan.md` 有三处需要回填或更正：

| 位置 | 原表述 | 实际状态 |
|---|---|---|
| §4.1 / §7 第 1 批 1 / §8 | 「PR #2337 …… 必须先把改动从产物搬到分片，否则下次 assemble 会静默抹掉」 | **已完成**。PR #2337 本体 CLOSED 未合；实际由 `5bb4f636` 直接改分片落地，**已在 v1.5.0 内**。§4.1 与 §7 第 1 批第 1 条应改为「已落地，无需再搬」 |
| §4.5 | 「全仓 grep `image_resize_notice` 零命中」 | 写作时点正确，但该结论已被 `a8dcd29f` **反转**为「命中即修复说明」（测试里有该字面量）。需回填：**该 P0 已由 `a8dcd29f` 修复，含双保险** |
| §4.4（#2362） | 「P0 confirmed，合入 #2368 后关闭」 | 已按此执行，但**修复未发布、未验证**，见本报告 §4.3 |

另：上一轮 §9 表里判为「要求修改」的 PR 中，**#2313 已于 2026-10-03 合入**（`c509c06d`，`fix: make launch and restart lifecycle deterministic`），新增 `launch-status.ts` 与 `native_browser.rs` 守卫、`watcher.rs` 改动共 6 文件 +361/-212。该提交落在 v1.5.0 之后，未发布。

---

## 8. 建议的执行顺序

### 立即（发版阻断项）

1. **确认 `8610d2f3` 进入下一次 release**，release note 注明安装器竞态修复；发版后在 #2362 追加「请升级复测」。这是本批唯一的发布阻断项。
2. **新开 issue 承接 #2322 的回归信号**（原文见 §4.2），两项待办：升级流程清理残留状态、dispatcher 模块发现补 fallback。

### 第 1 批（注入层，本批唯一的 confirmed P1 代码缺陷）

3. **#2329**：(a) 补丁扩到 `getLayer` / `_getLayerImpl`（记住同步改 `force_chinese_locale_settings.rs:84` 那条会锁住错误接口的断言）；(b) 内置 force-locale 补丁改走 `bridge.rs:262` 的 `add_script_to_new_documents`；(c) `ensure_injection` 重试循环内加端口反查回退。三条互相独立，可分开提交、分开验证。

### 第 2 批（小改动，确定性高）

4. **#2339** 给市场清单加 `minClientVersion` 过滤（一行判断，避免用户装到一半才失败）。
5. **#2376** NSIS 段 + `install/windows.rs` 同步加 `IfFileExists` 检测。

### 第 3 批（可关闭，回复即可）

6. 关 #2284（指向 `7e19e069`）、#2355（指向 `ea82bc74`）、#2221（指向 `82fb0924`）、#2263（指向 `0fd58cc3`）、#2379（指向上游）。
7. #2248 复查：与 #2322 归并考虑。
8. #2246 / #2254 回帖索要版本与对照结果（很可能只是版本太老）。

### 第 4 批（Feature / 讨论）

9. **#2228(b)**：模型配置导入面板加 effort 勾选器，后端零改动。
10. **#2253**：索要复现材料，重点查外部 catalog 降级路径；顺带评估 `max_output_tokens` 缺失是否需要补。
11. **#2312** 转 Discussion / 加 `roadmap`；**#2289** 按支持工单继续跟进。

### 纪律

- 注入层任何改动必须走完 **assemble + npm test + `--check`** 三步（AGENTS.md 已有约定）；
- **`10-style.js` 的改动要先 `codexDeleteStyleVersion` +1**，否则 `installStyle` 因版本号相同静默不重建；
- 改 `assets/inject/upstream/dream-skin/` 任何字节都要同步改 `crates/codex-plus-core/tests/upstream_theme_assets.rs` 的 sha256 锁定，否则测试会红；
- **不要在 `codex-models.json` 基座上逐个加 `max`**（§6.1）。

---

## 附：本轮核查方式

- 5 组并行只读子代理（协议层 / 注入与本地化 / 模型目录 / 已关闭 issue 复查 / Windows 启动与打包），全部结论基于 `main` 当下的实际代码，每一条都给了 `file:line` 与关键代码片段；
- 所有「已修复」判定均用 `git merge-base --is-ancestor <sha> v1.5.0` 独立校验过是否进入正式发布，**不采信提交信息自述**；
- 未修改仓库任何文件；未跑 `cargo build` / `cargo test`（仅读）。
- Windows 启动与打包那组子代理中途因机器休眠中断，该部分（#2376 / #2377 / #2246 / #2254 / #2362 / AUMID 回退链）由主会话直接读码补完，结论见 §5.3 与 §4.3。

---

## 10. 处置执行结果（2026-10-03）

按「能回复的回复、能关闭的关闭、能修复的修复」逐条执行完毕。本批 100 条内所有条目均已处置。

### 10.1 代码修复（3 处，已推 main）

| 提交 | issue | 内容 |
|---|---|---|
| `be62bc16` | #2329 #2339 #2376 | ① 强制中文：补丁从 `getDynamicConfig` 扩到 `getLayer` / `_getLayerImpl`，并往 layer 的 `__value` 写入 `enable_i18n` / `locale_source`；② DreamSkin 版本门限由硬编码 `1.5.12` 改为跟随 crate 自身版本；③ 桌面快捷方式改为「目标不存在才创建」，NSIS 段与 `install/windows.rs` 同步 |
| `b6e992b3` | #2302 | 上下文大小输入框展开 K/M 单位：`1M` → `1000000`，不再被 `replace(/[^\d]/g,"")` 静默剥成 `1`（该 bug 会让 `model_context_window` 变成 1，Codex 反复重跑任务） |

验证：`cargo check -p codex-plus-core` 通过；`dream_skin_package` 7 passed、`force_chinese_locale_settings` 5 passed、`cdp_bridge` 161 passed；`npm test` 338 passed / 1 skipped；`assemble-renderer-inject --check` 一致。

**新发现（本轮首次报出）**：#2302 是此前 116 条审计里完全没有的缺陷——`parse_optional_positive_u64` 只拒 0，`1` 一路合法写进 config，属静默数据损坏。

### 10.2 关闭清单（41 条）

**已修复可关（29 条）**：#2217 #2218 #2255 #2256 #2257 #2258 #2263 #2266 #2267 #2275 #2294 #2304 #2330 #2338 #2341 #2345 #2351 #2358 #2359 #2360 #2363 #2367 #2203 #2173 #2182 #2216 #2221 #2339 #2376

**误报 / 上游 / 范围外（9 条）**：#2235 #2238 #2268 #2287 #2344 #2369 #2373 #2223 #2379

**已支持可直接关（2 条）**：#2284 #2355

**另**：#2302（本轮新修）。

### 10.3 需发版提醒（跨批次，务必确认）

| issue / 提交 | 状态 |
|---|---|
| **#2362**（`8610d2f3`） | 安装器竞态修复，**不在 v1.5.0**。下一次 release 必须包含，发版后在原单追加复测请求 |
| **#2330 / #2266 / #2267 / #2249 / #2304**（`5bb4f636` / `e5cd4de7`） | 重注入根治 + macOS Dock reopen，**均不在 v1.5.0** |
| **#2275 / #2257 / #2367 / #2258 / #2359 / #2345 / #2294 / #2173 / #2203** | 协议层与注入层修复，**均不在 v1.5.0** |
| **#2313**（`c509c06d`） | 启动/重启生命周期重写，**不在 v1.5.0** |
| 本轮修复（`be62bc16` / `b6e992b3`） | 自然也不在 v1.5.0 |

### 10.4 未关闭的 41 条：全部为「等用户回帖」或「已答复待复测」

本批仍开着的 41 条**没有一条是「需要开发者单方面动作」的**，逐类如下：

- **等用户补证据**（已在帖内列出所需材料）：#2220 #2226 #2233 #2237 #2244 #2246 #2253 #2254 #2269 #2270 #2273 #2297 #2300 #2314 #2315 #2325 #2331 #2340 #2343 #2349 #2372 #2377 #2380 #2259 #2260 #2261 #2262(#2228) #2312 #2219 #2374 #2375
- **已答复、待复测**：#2236（待确认要透传哪些头）#2227 #2364 #2252（三条均为「已修未发版」，已说明等待下一次发版）

### 10.5 遗留待办（本轮确认但未动手）

1. **#2322 的回归信号**——建议新开 issue 承接（原地升级遗留状态 + dispatcher fallback），不要重开原单；
2. **#2362 的发版后回帖**——release 后在原单追加复测请求；
3. **#2244 的启动阶段埋点**——「全程无日志」是确定的可修项，且是 #2343 / #2314 / #2377 这类报障的共同基础设施缺位；
4. **`max_output_tokens` 不写入**——`build_model_catalog_json_with_capabilities`（`model_suffix.rs:533-610`）从不写该字段，而 DeepSeek 元数据里每个模型都有；与 #2253 的圆圈口径可能相关；
5. **#2228 / #2374 的档位勾选器**——`modelMetadata` 已可覆盖 `supported_reasoning_levels`，缺的只是 UI；
6. **#2240 的窗口列可见性**——能力已存在，缺的是入口显眼度（属体验改进，需先决定是否改变「未配置 = 272K」的语义）。
