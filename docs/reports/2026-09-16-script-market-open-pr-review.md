# 脚本市场开放 PR 审查（2026-09-16）

仓库：`BigPizzaV3/CodexPlusPlusScriptMarket`（`crates/codex-plus-core/src/script_market.rs:8` 里 `DEFAULT_MARKET_INDEX_URL` 指向它）
基线：`main` @ `90e8415`（"feat: update Codex Model Matrix to 1.0.0 (#52)"）
方法：克隆仓库、逐个 fetch PR head、核对 SHA-256 与 index.json、实跑 PR 自带测试、通读新脚本源码

**结论：9 个 PR 全部对当前 main 干净可合（`git merge-tree` 无冲突），SHA 校验 100% 通过，没有任何一个脚本发起本项目之外的外连请求。其中 4 个可以现在合，2 个要你拍产品判断，1 个建议先改。**

| PR | 类型 | 规模 | 测试 | 建议 |
|---|---|---|---|---|
| **#91** Prompt Optimize 1.0.4 | 更新 | +114/-38 | 14/14 通过 | ✅ 可合 |
| **#84** 每日 Token 1.4.15→1.4.18 | 更新 | +1431/-40 | 12/12 通过 | ✅ 可合 |
| **#85** relay-balance 0.3.3→0.3.7 | 更新 | +689/-73 | 无（仅语法检查） | ✅ 可合 |
| **#90** Model Matrix 索引元数据 | 元数据 | +16/-16 | 无 | ✅ 可合（见下方说明） |
| **#87** Window Model Isolation | 新脚本 | +440 | 无 | ⚠️ 需你判断 |
| **#88** DeepSeek Token Usage | 新脚本 | +3492 | 无 | ⚠️ 需你判断 |
| **#89** Windows 远程控制入口 | 新脚本 | +293 | 无（含模拟 Statsig 验证） | ⚠️ 需你判断 |
| **#86** Environment Injector v0.3.2 | 新脚本 | +17250 | 无 | ⚠️ 体量与可审计性 |

---

## 统一核对结果（全部通过）

- **SHA-256**：8 个 PR 里 index.json 声明的每个 `sha256` 都与对应脚本文件实际哈希逐字节一致。工作区 16→17 个条目里，唯一两个"文件缺失"是我脚本按同仓库 URL 推断路径的误报——`codex-zhcn-translate` 的 `script_url` 指向 `hL091015/CodexPlusPlusScriptMarket` 这个 fork，文件本就不在本仓库；`Codex%20Model%20Matrix.js` 是 URL 编码，解码后存在。
- **合并性**：8 个 PR head 全部 `merge-tree --write-tree` 干净，且都以当前 `origin/main` 为祖先。
- **解析兼容**：`parse_market_manifest` 对 `description`/`author`/`tags`/`homepage`/`sha256` 都是 `#[serde(default)]`，只强校验 `id`/`name`/`version`/`script_url`。各 PR 新增条目全部满足。
- **外连审计**：新脚本没有任何一个主动发起网络请求。#87、#89 零网络 API；#88 只包装 `fetch`/`XHR`/`WebSocket` 做**旁路观察**（读响应体解析 token 数），`isLikelyApiUrl` 过滤在 `deepseek`/loopback/`/responses`/`/chat/completions` 上；#86 的 `bridgeCall` 走宿主自己注入的 `window.__codexSessionDeleteBridge`，无硬编码远端。

---

## ✅ 可以现在合的四个

### #91 — Prompt Optimize 1.0.4

`index.json` 只改 `version` 和 `sha256` 两行。测试 **14/14 全过**（含"写回后 ProseMirror 重建节点时重新校验"这条新回归、以及 bearer token 脱敏用例）。改动集中在传输层：优先走 LLM Bridge，仅在 bridge 明确不支持时才回退到原生 fetch；超时放宽到 120 秒并加慢请求进度提示；旧的备份文件不再落进用户脚本加载目录。**这是 1.0.3 的正当后续**，作者在 PR 正文里说明了上游 #74 已合入 1.0.3 作为基线。

### #84 — Codex Daily Token Usage 1.4.15 → 1.4.18

作者按 #77 的关闭意见从最新 `origin/main` 重开了干净分支，没有堆 merge commit。内容：适配新版 renderer RPC（`localThreadCatalog` 取任务 + `workspaceFiles.read` 读本机 session JSONL）补齐近 5 日历史回填，旧 app-server dispatcher 保留为回退；修长任务跨日增量统计。测试 **12/12 通过**（新增 374 行测试）。index.json 只定点改这一个条目。

### #85 — Codex Relay Balance 0.3.3 → 0.3.7

顶栏兼容性修复：兼容新旧两版 Codex++ 顶栏结构，检测其他插件按钮占用区域后自动找空白位置，避免余额入口重叠；预留窗口控制按钮区域；顶栏变化/缩放/插件加载后自动重定位。无自动化测试，但 diff 只有顶栏定位逻辑，改动边界清楚。顺带把 `updated_at` 从 `2026-08-22` 更新到 `2026-08-31`——注意这个字段各 PR 各写各的，见下方"小瑕疵"。

### #90 — Codex Model Matrix 索引元数据修正

**这个 PR 修的是一个真实的索引/文件错配**，而且方向和它标题写的不完全一样：

- 仓库里 `scripts/Codex Model Matrix.js` 在 main 上**已经是 1.0.0**（`author: Xiazhixuan119748`，本地 55146 字节，sha `01e05ef...`）。
- 但 `index.json` 里的条目还停在旧状态：`name: "Codex Native Matrix Selector"`、`version: 0.8.0`、`author: "Codex"`、`homepage: ""`、`sha256: 0a25838...`（**与文件实际哈希不符**）。

也就是说 **main 当前把 1.0.0 的脚本配着 0.8.0 的元数据和一个对不上的哈希发出去**。这个 PR 把条目改成 `Codex Model Matrix / 1.0.0 / lx / github.com/Xiazhixuan119748/Codex-Model-Matrix` + 正确 SHA，并把条目从数组靠后位置挪到了第 2 位。

**顺带指出的一个既有问题（不是这个 PR 引入的）**：`crates/codex-plus-core/src/script_market.rs` 的 `install_market_script_content` 并不校验 SHA，而 CHANGELOG 第 61 行记录了"移除脚本安装时的 checksum 阻断"是有意为之。所以错配的 `sha256` 目前不会阻断安装，危害是**市场上显示的版本/作者/名称与用户实际拿到的脚本不一致**，以及任何依赖哈希做完整性判断的环节失效。值得单独开个 issue 跟踪"要么恢复校验，要么把 sha256 降级成纯展示字段并在 UI 上说明"。

**唯一可讨论的点**：它把 `id: codex-native-matrix-selector` 的展示名从自己的 "Codex Native Matrix Selector" 换成了第三方作者的名字。同 id 意味着老条目被覆盖。从"文件已经是人家的 1.0.0"看这是纠正而非顶替，但**确认新选择器确实替代了原来的原生选择器**这个事实判断该你来做。

---

## ⚠️ 需要你拍判断的三个

### #87 — Window Model Isolation（代码干净，动机也正当）

**解决什么**：Codex 某个版本之后模型配置快照从窗口级变成了全局，导致在逆向任务的窗口里一个不注意就用掉自己的官方 key。脚本按 conversation 维护模型/推理强度快照，发送前发现不匹配就拦截并弹提示。

**代码质量**：424 行，零网络、零 eval、零 `innerHTML` 注入（只有一段静态 `<style>`）。有完整的 `destroy()` 拆卸（observer、interval、所有 timer、监听器、注入的 style/toast 全部清理）。所有事件处理都先 `event.isTrusted` 判断，忽略合成事件。`REACT_KEYS` 只是常量声明，未见破解 fiber 内部状态。

**一点权力让渡要说清楚**：它调用 `event.preventDefault()` + `stopImmediatePropagation()` **拦截你的"发送"点击和 Enter 键**，然后用合成鼠标/键盘事件（`activate()` 派发 pointerdown/mousedown/pointerup/mouseup/click）去**自动改你的模型选择**。这是不可见的 UI 自动化——脚本在替你操作选择器。作者已用 `userSelectionUntil` 时间窗和 `document.hasFocus()`/`visibilityState` 检查降低干扰，拦截时也会弹明确的红框提示，但这确实比"只读监控"类脚本的信任成本高一档。**装不装是用户自己的选择**，市场里应该让描述讲清楚这一点。

### #88 — DeepSeek Token Usage（功能好，但要接受它读你的全量响应体）

**解决什么**：DeepSeek 模型在 Codex 里的 token 用量与费用统计，内置官方 CNY 费率表（区分缓存命中/未命中/输出，按北京时间峰谷计价），按天/月查看，图表悬浮明细，面板可拖拽缩放并记忆位置，可收成 mini 状态条。

**代码质量**：3456 行。设计上比较克制——**只观察不篡改**：包装 `window.fetch`、`XMLHttpRequest.prototype.open/send`、`window.WebSocket`，在响应到达后读 `responseText`/`event.data` 解析 token 数，然后原样返回/放行，不改请求也不改响应。包装函数都打了 `__deepseekUsageWrapped = VERSION` 标记避免重复包装，也保留了原函数引用。

**存储与隐私**：源码注释明确写了"面板自己的 localStorage 不保存任何密钥"，用户填的 key 只存在于页面内存。作者的 market 快照是对着空临时 Codex home 生成的。

**要接受的代价**：它为了取 token 数，会**读取所有命中 `isLikelyApiUrl` 的响应体**——包括任意 `deepseek` 字样 URL、loopback、`/responses`、`/chat/completions`、`/completions`。对本地统计工具来说这不可避免，但意味着**任何走这些路径的响应内容都经过它的解析代码**。PR 正文声明 `version: 1.13.0`，实际 index.json 里是 `1.16.1`——作者后来更新了版本但没改正文，**正文陈旧，代码是新的**。

### #89 — Windows 远程控制入口（技术上干净，但你之前发过同类拒绝）

**解决什么**：部分 Windows 版 Codex 已内置远程控制功能，但"设置 → 连接 → 控制其他设备"入口没显示。脚本包装 Statsig 的 `checkGate()`/`getFeatureGate()`，把 gate `782640499` 强制成 `false`、`2055603567` 强制成 `true`，把隐藏入口显示出来。

**代码质量**：207 行，零网络、零 eval。实现相当规矩：`Map` 驱动覆盖表、非目标 gate 一律 `Reflect.apply` 原方法、包装时保存 `ownDescriptor` 以便 `stop()` 精确还原（有 own property 就 defineProperty 回原 descriptor，没有就 delete）、`Object.freeze` 暴露的 API、重复加载先调上一个实例的 `stop()`。作者还附了中文使用说明和模拟 Statsig 的验证。

**需要你判断的**：这本质上是**用脚本覆盖上游的功能开关**。你之前审 #2208 时就遇到过同构的问题——"不使用服务端 rollout 决策"是否算策略绕过。这里更直接：它是在客户端把厂商灰度关掉的入口打开。技术上它不伪造登录、不碰配对/设备密钥/签名（这些仍由 Codex 自己处理），但它确实绕过了厂商的灰度判断。**这是产品判断不是代码质量问题**，而且这个脚本是给"客户端已有功能只是入口被隐藏"的场景用的，作者也如实标注了"Codex 内部开关变化时可能需要适配"。

### #86 — Codex Environment Injector v0.3.2（体量本身是问题）

**解决什么**：per-conversation 绑定模型、Provider、prompt、权限和 Memory 策略；带当前线程绑定与脱敏的 `acknowledged` 证明；Memories 和 AGENTS 审阅 UI；headless prewarm/direct-RPC 注入器。

**代码质量**：**17220 行、841 KB 的打包产物**，内含 React 19 + jsx runtime。这是本次审的最大障碍——不是发现它有问题，而是**它大到没法真正审**。少量 `innerHTML` 都是 React 内部的（`<script>` 占位、key 赋值），不是注入 sink。

**隐私声明做得好**：作者明确写了提交的 market 产物是对着**空临时 Codex home** 生成的——`profiles: []`、无 AGENTS 内容、无 Memories 内容，脚本里内嵌的 `ENVIRONMENT_BUNDLE` 确实就是这个空快照（我核对了第 18 行，`"profiles":[]`、所有 content 为空字符串）。这个习惯值得肯定，应该写进 CONTRIBUTING 当投稿要求。

**需要你判断的**：它通过 `window.__codexSessionDeleteBridge` 调宿主 bridge 读 AGENTS/Memories，并且**会 patch dispatcher 的 `dispatchMessage`**（`__codexServiceTierOriginalDispatchMessage` 标记）来注入 per-conversation 环境。功能正当、声明诚实，但一个 17k 行的黑盒在每次对话开始时改写分发路径，**维护面和安全面都不小**。如果合，建议要求作者同时提供源码仓库链接和可复现的构建流程（PR 正文给了 `github.com/FlyCatdev/codex-environment-injector`），至少让下一次改动有 diff 可看。

---

## 小瑕疵（不阻塞合并）

1. **`updated_at` 各 PR 各写各的**：#84/#87/#90/#91 保持 `2026-08-22T15:50:28Z` 不变，#85 写 `2026-08-31`，#86 写 `2026-09-04`，#89 写 `2026-09-11`，#88 写 `2026-09-15`。多 PR 并行时这个字段必然 last-write-wins，**合并后统一刷成合并当天**即可。
2. **#88 的 PR 正文与产物版本不一致**：正文写 `1.13.0`，index.json 是 `1.16.1`。合并前请作者更新正文，或在合并说明里注明。
3. **#90 挪动了条目位置**：把 Model Matrix 从第 15 位挪到第 2 位，改变了市场列表顺序。如果顺序对你有意义（比如按受欢迎度排练），注意这一处。
4. **`codex-zhcn-translate` 是跨仓库引用**：`script_url` 指向 `hL091015/CodexPlusPlusScriptMarket` 的 fork。本仓库无法校验它的 SHA 或做变更审查——**这是市场当前的一个供应链盲点**，值得单独讨论（要么收编到本仓库，要么在 UI 上标注"第三方托管"）。

---

## 建议的合并顺序

先合 4 个低风险的更新：**#91 → #84 → #85 → #90**（都改 index.json 同一批行，逐个合比一次合冲突少）。

再处理 3 个新脚本：#87、#88、#89 都是自包含条目，合并无冲突，**取决于你对上面各自动机/权限的判断**。

#86 建议单独决定，并优先要求源码与构建可复现。

合并后跑一次统一收尾：把 `updated_at` 刷成合并当天，重算一遍全仓库 SHA 一致性。
