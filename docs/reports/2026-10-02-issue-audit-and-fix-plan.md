# CodexPlusPlus 开放 issue 审计与修复方案

- 日期：2026-10-02
- 仓库：`/Users/mac/Desktop/CodexPlusPlus`
- 基线：`main` @ `2ce62815`（fix(inject): 插件解锁改用结构正则识别 marketplace 过滤器）
- 数据来源：8 组并行审计代理对近 141 条开放 issue 的只读核查结论
- 去重后规模：**116 条 issue + 1 条 PR（#2303）**；`#2299` 为审计批次中的编号占位条目（未取到正文，非真实 issue），不计入

---

## 1. 摘要

本轮审计覆盖 GitHub 上近期开放的 141 条 issue，去掉跨主题重复提报与批次占位条目后，实际给出结论 **116 条**（另有 1 条 PR #2303 在 §9 处理）。结论分布：

| 结论 | 条数 | 含义 |
|---|---|---|
| `confirmed-in-code` | 31 | 已在代码里定位到确定缺陷，可动手修 |
| `likely` | 37 | 有强指向但缺最后一环现场证据（多为采样/日志不足） |
| `already-fixed` | 19 | main 上已修（含部分仅进 v1.5.0，v1.4.0 用户未拿到），只需升级复测后关单 |
| `not-our-bug` | 17 | 上游 Codex 客户端 / 第三方供应商 / 杀软策略 / 产品范围外 |
| `insufficient-info` | 12 | 正文与评论都不足以定位，需回帖补证据 |
| **合计** | **116** | 少量 issue 同时出现在两个主题下（#2180 / #2188 / #2315），已在正文标注归属 |

**最重要的三条结论：**

1. **注入层的「周期性强占主线程」是本轮最大的单一根因**，一条链路同时解释 #2330（60s 重复解析）、#2169（68% 空转）、#2267/#2266（输入法与滚动卡顿）、#2201（listener/observer 不回收）：`launcher.rs` 的桥接看门狗把「窗口隐藏」误判为「桥接失效」，而重注入退避又因为「刚注入完必然健康」被永久清零（`crates/codex-plus-core/src/launcher.rs:2879`）。修复已在 PR #2337，但**必须把改动从产物搬到分片**（`assets/inject/renderer-inject/20-menu.js:99`、`:128`），否则下次 `assemble` 会把修复静默抹掉。
2. **协议翻译层有三个独立的状态机缺陷**（#2275/#2257 同根因、#2210、#2367），全部是「单条 item 转换正确、item 之间的顺序与 pending 交接出错」。这是 107 条 `protocol_proxy` 测试的盲区，三条修复都不大，收益最广。
3. **纯 API 模式下的能力缺失有三个根因**（#2294 / #2173 / #2209）：一个可修（CUA runtime 白名单过窄，改一行修一片）、一个不可修（原生 app-server 要求 `auth_method == "chatgpt"`，必须对外直说）、一个必须立刻修（`relay_config.rs:721-722` 纯 API 分支整体覆盖 `auth.json`，会静默摧毁唯一可用的 workaround）。

**P0 清单（9 条）**：#2330、#2363、#2350、#2362、#2275、#2257、#2210、#2294、#2173。

---

## 2. 已修复可关闭

以下 issue 在 main 上已有落地提交，**不要重复投入**。给出证据与建议回复，可直接关单。

| issue | 主题 | 落地提交 / 位置 | 建议关闭回复（一句话） |
|---|---|---|---|
| #2255 | 26.915 asset 重扫 254 fetch/s | `bed5277d`（2026-09-21，`git tag --contains` = v1.4.0 v1.5.0） | 已在 v1.4.0 修复（熔断不再被 provider 重试绕过 + 重试改指数退避），请升级到 v1.5.0 后按 CDP 抓 fetch 计数复测；若仍复现请贴新的调用栈。 |
| #2256 | 26.911 空转高 CPU | 同上 `bed5277d`；代码见 `assets/inject/renderer-inject/70-model-catalog.js:782-813`（统一计数、8 次熔断）、`:770-781`（指数退避 250ms→30s）、`20-menu.js:200-222`（fallback 名单按 `codexAppAssetCandidateUrls()` 动态筛选） | 已在 v1.4.0 修复（你定位的两个缺口都已闭合），升级后空闲 10s 窗口内 fetch 应远低于 1651 次，diagnostic 里能看到 `model_app_server_request_patch_skipped`。 |
| #2167 | 26.909 右上角版本徽章不渲染 | PR #2186；降级路径见 `70-model-catalog.js:630-673`、`:675-700` | 已修复（走 CDP 断点捕获降级路径），升级后徽章应恢复，诊断里可见 `app_server_client_capture_located`。 |
| #2182 | AUMID 硬编码导致无法从管理工具打开 ChatGPT | `2a41afb`（#2154，2026-09-10）+ `2b4d8d68`（#2202 后续加固） | 已修复，v1.5.0 之后的包会带上；请确认当前包构建 SHA 晚于 `2a41afb`。 |
| #2216 | 切回 official 后残留中转站 provider 段导致 401 | `1b214349`（2026-09-22，`git tag --contains` = v1.4.0 v1.5.0）；`crates/codex-plus-core/src/relay_config.rs:1027-1055`、`:4108` | 主因已修，升 v1.5.0 复测；正文里附带的 `runtimes\cua_node\<hex>` 模板路径陈旧问题属实且未修，已另开跟踪。 |
| #2207 | 切换后 config.toml 报 invalid transport | `bc3669e2`（语义合并重写）+ `6b02b74a` / `7daabac8`（写盘前用 live 补齐 mcp_servers）；`relay_config.rs:1734`、`:1310`、`:1337` | 已修（v1.4.0/v1.5.0）；注意**磁盘上已损坏的 config.toml 不会自愈**，需按 §4 的体检路径处理。 |
| #2281 | 补全 GPT-6 Sol / Luna 元数据 | PR #2281 已合并；`assets/gpt6-sol-luna-model-metadata-compat.json` | 已合并，升 v1.5.0 即生效。 |
| #2282 | 官登额度提示与功能锁 | PR #2282，随 v1.4.0 发布；`assets/inject/renderer-inject/95-conversation-view.js:483` `syncOfficialUsagePolicy` | 已合并，无需处理。 |
| #2286 | 修混合 API 被官方额度阻止发送 | PR #2286（squash `a9af207`），随 v1.4.0 发布 | 已合并；注意其依赖「按源码结构定位」，后续 Codex 改压缩标识符时需沿用结构判定。 |
| #2109 | GPT-6 不能选 ultra / 快速模式 | `assets/gpt6-sol-luna-model-metadata-compat.json` + `assets/gpt61-sol-model-metadata-compat.json`；`3d6d360d` 补 fast 档位 | 已修复，升 v1.5.0 复测。 |
| #2121 | 缺少 GPT-6 Max / ultra | `2d667570`（PR #2112）；gpt-6-astra / gpt-6-sol 已进 `codex-models.json` | 本 issue 已修；兜底机制本身见 #2374 的统一修复。 |
| #2358 | gpt-6.1 Sol 最高只有 xhigh | `35ede662` / merge `11813fad`（2026-09-30）；`model_suffix.rs:314` | 已修，v1.5.0 已包含。 |
| #2141 | 生成的 model-catalogs 与官方 models_cache.json 差异大 | `f652cc9f`（2026-09-18）+ `7e7d44a4`（#2280）；`crates/codex-plus-core/src/model_suffix.rs:403-435`、`:634-676` | 已修；「按时间取较新一份」是有意不实现（兼容层承载 Fast/Ultra 特性字段），可在 issue 里说明。 |
| #2338 | v1.4.0 无法压缩 | `549551d3` 等（仅进 v1.5.0）；`crates/codex-plus-core/src/protocol_proxy.rs:717-760`、`:513`（`cmp_` 前缀） | v1.4.0 独有的回归，已修，直接升 v1.5.0。 |
| #2341 | remote compaction v2 缺 `response.output_item.done` | 同上，`protocol_proxy.rs:717-760` 已按 issue「期望响应」逐字实现 | 已修，关单并指向 `549551d3`。 |
| #2360 | v1.4.0 远程压缩失败，回退 1.2.56 正常 | 同上 | 已修，升 v1.5.0；你的技术判断正确。 |
| #2217 | deepseek-flash 压缩报 `got 0 from 2 output items` | CHANGELOG 1.4.0 有条目；`549551d3` / `4b71d6f9` / `94151316` / `7577eeca` | 已在 v1.4.0 修复，升 v1.5.0 复测。 |
| #2218 | 每次重启都「ChatGPT 遇到了问题」，重试正常 | PR #2186 + PR #2174；`7d2a43a2` | 已由 #2186/#2174 覆盖，升 v1.5.0 验证。 |
| #2129 | 快捷方式偶发启动失败 + 残留进程清理不安全 | `watcher.rs:118-139` `filter_killable_launcher_processes`、`:502-548`；`settings.rs:1282-1287` 护栏 | 五条诉求中四条 main 已覆盖，剩余「偶发无反应」并入 #2314 的可观测性修复。 |
| #2188 | 纯 API 下 Web 能力依赖分析（讨论记录） | 其中浏览器请求标识路径已由 `a5794a4d` / `0d1c3dbd` 解决（见 `docs/native-browser-identification.md:69`） | 无需修代码；建议加时效标注并转成常驻依赖表（见 #2223 的 fix）。 |

> 注：#2188 在两组审计里分别判为 `already-fixed` 与 `not-our-bug`，两者一致——**那段代码链路已被后续提交超越，但原生 app-server 认证的一半结论仍成立**。归入 `not-our-bug` 不重复计数，处置见 §3。

---

## 3. 不是我们的 bug

以下问题根因在上游 Codex 客户端、第三方供应商、Windows 安全策略或产品范围之外，**本仓库不应改代码**。给出证据与建议回复。

### 3.1 上游遥测/客户端行为

| issue | 证据 | 为什么不修 | 建议回复要点 |
|---|---|---|---|
| #2107 | 整仓 grep `cua_node` / `runtimes` / `.staging-` 零引用；评论区把根因定位到 `%LOCALAPPDATA%\OpenAI\Codex\runtimes\cua_node` 的 `.staging-*` 部署过程 | 是 Codex 官方 CUA Runtime 的自部署行为，Codex++ 不写该目录 | 转成上游 SOP：看到 `.staging-*` 不要删，等它自己完成；「最小化失效」另开 issue 并附 MainWindowHandle 与 `latest-status.json`。 |
| #2169（部分） | 全仓 grep `heapProfiler` / `HeapProfiler.start` 零命中；`cdp.rs` 只用 Runtime/Page/DOM 域 | V8 heap profiler 由 Codex 自家 renderer 启停，Codex++ 没有开启调用 | (a) 重注入循环按 §4.2 修；(b) 让用户用 `sample` 对比「只启动 ChatGPT」与「经 Codex++ 启动」以分离责任。 |
| #2260 | `model_provider="custom"` + `model_catalog_json` 指针正是本仓 `relay_config.rs` 的合法写入形态（对应主 feature #1171/#931） | 用户的「被第三方改写」推断站不住，且正文无可操作信息 | 说明该指针是本工具配中转站的正常产物；要 `codexAppVersion`、实际错误文案、「App 清理配套组件」的完整路径（很可能对照 #2107 的 cua_node SOP）。 |
| #2126 | 报错 URL 是 `chatgpt.com/backend-api/codex/responses` 且带 cf-ray | 请求直发官方后端，根本没走 Codex++ 本地代理；403 是 Cloudflare WAF，404 是账号无 gpt-5.5 权限 | 确认供应商是否「使用中」；403 换网络重试，404 需在官方侧解决订阅；若本意是走第三方却打到官方域名才需要我们跟进。 |
| #2171 | `ws://127.0.0.1:12326` 全仓零命中；不是 helper 端口（后者 57321 量级） | 该端口由扩展自身起，属 Codex Desktop 原生 sidepanel ↔ app-server | 但**不能直接判 not-our-bug**：注入层确实订阅了 browser 相关消息，需先证伪（见 §4.25）。 |
| #2188 / #2373 / #2369 / #2287 | `Codex auth token is unavailable` / `unsupported Codex auth method: apikey` 均出自 CUA runtime 二进制，本仓 grep 零命中 | 原生 `getAuthStatus` 硬性要求 `auth_method == "chatgpt"`，检查发生在 HTTP 之前，代理/注入/元数据三处都够不到 | **不许做「把 apikey 标签改写成 chatgpt」的绕过**——那只改标签不转凭据，原生侧会把原 token 放进 `Authorization: Bearer`，服务端不接受，属既无效又带伪造身份风险。可行路径只有「官方登录 + 第三方模型」。 |

### 3.2 第三方供应商 / 缓存 / 额度语义

| issue | 证据 | 为什么不修 | 建议回复要点 |
|---|---|---|---|
| #2252 | 报错原文 `chatgpt authentication required for remote plugin catalog` 全仓零命中；本仓只在 `60-plugin-marketplace.js:132/137/189-194` 做本地兜底降级 | 远端插件目录要求 ChatGPT 登录态，API key 鉴权在协议层不成立 | 不是把插件藏了；可做体验补救（顶部说明条），**不要**把远端失败包装成成功。 |
| #2325 | 本仓不参与 prompt caching 的构造或改写；`protocol_proxy.rs` 只做协议转换与转发 | 缓存命中由上游中转站实现决定 | 转讨论或关闭；可补文档：确认 base_url 是否 Responses 原生端点，且不要配错上下文窗口。 |
| #2144 | `relay_config.rs:3411-3417`：混合模式下 API Key 写进 `experimental_bearer_token`，官方身份只保留登录；`relay_rotation.rs:185/215` 的 failover 只在聚合成员间轮转 | 「官方额度优先、耗尽兜底 API、恢复切回」是需要新增的 feature，不是缺陷 | 关单并在设置页文案澄清；替代方案是聚合供应商的 failover/轮转，或手动切回官方登录。 |
| #2100 | 用户提供的 200 条 bridge 日志里没有优化请求 | Prompt Optimize 是脚本市场的第三方脚本，请求没到 Codex++ helper | 转脚本仓库追踪；让脚本作者在点击处加 try/catch 打 console。 |

### 3.3 Windows 安全策略 / 杀软 / 产品范围

| issue | 证据 | 为什么不修 | 建议回复要点 |
|---|---|---|---|
| #2184 | 维护者评论区已定性；注入机制（写脚本文件 + 回环调试端口）是启发式检测高命中项 | 杀软误报，代码开源无混淆；「反杀软」是军备竞赛且有安全代价 | README/Release notes 固定一段 Defender 加白步骤；启动期自检把 os error 2 升级为「可能被隔离」提示。 |
| #2269 | os error 4551 = WDAC/AppLocker/Smart App Control 阻断；`install/mod.rs:305` 把 spawn 失败统一包成裸错误 | 企业应用控制策略，重装无用 | 只改可读性：识别 `raw_os_error() == Some(4551)` 给出「请联系管理员放行 / 检查智能应用控制」文案；**不要**做成自动关闭 Smart App Control。 |
| #2238 | 注入层与 Codex 页面结构强绑定（`00-prelude.js` 第 4 行的宿主守卫） | 换宿主等于重写，不是适配 | 转 Discussion；若只是想要「按模型配窗口/供应商切换」，`crates/codex-plus-core` 那部分与宿主解耦，可单独提需求。 |
| #2268 | 与本仓范围无关（workbuddy 是另一产品） | 评论已有人质疑 | close as not-planned；若本意是「在 Codex++ 里管理多个 Codex 账号」则另开 issue。 |
| #2344 | 会话沙箱由 `~/.codex/config.toml` 的 `sandbox_mode` / `approval_policy` 决定 | 是 Codex 本体能力，不是 Codex++ 限制；标题已标「已解决」 | 直接关闭；给出 sandbox_mode 取值指引。 |
| #2139 | 注入层未找到针对宿主模式选择器的改写（grep 无命中） | 更可能是上游 Codex 自身形态变化 | 要 DOM 片段（outerHTML）+ 官方直启对照截图 + `codexAppVersion`，再二分。 |

### 3.4 已修但仍需发版提醒

- **#2182**：main 已有 `2a41afb` + `2b4d8d68`，v1.3.0 的包构建早于修复约 10 小时，故正式包没带上。需在下一个 release 关闭，并核验构建 SHA。

---

## 4. P0 / P1 缺陷详表

每条独立成小节：现象 → 根因（file:line）→ 修复方案（具体到函数与改法）→ 影响面 → 需要补的测试。所有代码引用相对仓库根。

### 4.1 #2330 —— 注入模块每 60s 重复解析 + 全量 rescan（P0，confirmed）

**现象**：空闲状态每分钟一次注入模块重新解析，叠加一轮含约 286 个 `app://` asset 的全量 rescan；renderer 单次 burst 顶到 55.6% 单核。

**根因**：60 秒周期不是 Codex++ 的定时器，是桥接看门狗把「隐藏窗口」误判成「桥接失效」后的整份脚本重注入。
- `crates/codex-plus-core/src/launcher.rs:1137`：看门狗 `tokio::time::interval(Duration::from_secs(5))`；窗口最小化后 Chromium 把后台定时器钳到约 1 次/分钟，`assets/inject/renderer-inject/40-backend-settings.js:194` 的 5s heartbeat 随之停摆。
- `crates/codex-plus-core/src/bridge.rs:137-154`：`bridge_health_check_script` 只看 `lastInjectionAt(≤5s) / lastSuccessAt(≤15s) / lastAttemptAt(≤15s)`，**没有 visibilityState 判据**，后台必然返回 false。
- `launcher.rs:26` `BRIDGE_HEALTH_FAILURE_THRESHOLD=2` → `launcher.rs:2811` `should_reinject_after_health_result` 放行 → `launcher.rs:2832` `check_and_reinject_bridge_inner` → `launcher.rs:2669` `try_inject` → `assets::injection_script_with_settings` 重发整份 616KB。
- 退避挡不住：`launcher.rs:2879-2883` `Some(true) | None => backoff.reset()`，刚注入完的第一次健康检查必然通过（`lastInjectionAt` 在 5s 内），`consecutive_attempts` 立刻归零，`BridgeReinjectBackoff`（`launcher.rs:33-64`）永远停在第一档。
- 与 rescan 严格 1:1：每次新实例 `codexPlusModelWhitelistRefreshUntil` 从 0 起算，`70-model-catalog.js:890-903` 的 120ms tick 重新跑满 2.5s。

**修复方案**（PR #2337 思路正确，但有三个必改落地点）：
1. **改动必须落到分片，不能只改产物**。PR #2337（`54b61fa6`）只改了 `assets/inject/renderer-inject.js`，reviewer 已指出 `assemble --check` 报「产物与分片不一致」（269 pass / 1 fail）。把以下改动搬进分片后再跑 `node scripts/assemble-renderer-inject.mjs`：
   - `assets/inject/renderer-inject/20-menu.js:99` 的 `codexAppModuleFailures` → 改挂 `window` 跨重注入保留；
   - `assets/inject/renderer-inject/20-menu.js:128` 的 `codexAppAssetUrlFromScriptText` → 同上。
   否则下一次组装会把修复静默抹掉。
2. **`bridge.rs:137` 的 `bridge_health_check_script` 加 hidden 早返回**：`if (document.visibilityState === "hidden") return true;`。**必须放在 bridge/health 存在性检查之后**，否则页面重载后真丢桥时不再修复。
3. **`launcher.rs` 给 `BridgeReinjectBackoff` 加 `healthy_since: Option<Instant>`**，把 `launcher.rs:2879` 改成分派：
   - `Some(true) => backoff.observe_healthy(now)`（持续健康满 `BRIDGE_REINJECT_BACKOFF_RESET_AFTER_SECS=60` 才清零）；
   - `None => {}`（探测不确定不改变退避）；
   - `Some(false) => backoff.observe_unhealthy()`。
   **`launcher.rs:2870` 的 `browser_identity_changed` reset 必须保留**，否则应用换实例后新页面要白等退避。

**影响面**：仅改 launcher/bridge 与注入分片，不触及 per-profile 数据模型；`settings.rs` 无变化；不碰 `window.codexPlus` 的已发布类名与 `constants`；20-menu.js 两处改挂 `window` 属内部实现，第三方脚本无依赖。

**需要补的测试**：
- Rust：仿 `launcher.rs:3634` 新增 `reinject_backoff_resets_only_after_sustained_health`，断言「注入后立刻健康不清零 / 中途不健康重新计时 / 持续健康满阈值才清零」。
- Rust：`cdp_bridge.rs` 加 hidden / hidden 且无 bridge / visible 三种健康检查用例（PR #2337 已有 7 行）。
- 前端：`renderer-inject.test.ts` 断言产物含 `window.__codexPlusAppModuleFailures`（跨重注入保留）。
- 实机：CDP 连 9229 最小化窗口跑 3 分钟，`Debugger.enable` 数 `scriptParsed` 中同一 hash 重复次数应为 0。

### 4.2 #2169 —— 调试端口持续连接导致 68% CPU 空转（P1，likely）

**现象**：macOS 上 Codex++ 进程 68% CPU 空转。

**根因**：三层，只有第一层是我们的。
- (a) 桥接重注入循环：`e06c9fb5` 加了指数退避，但退避在 health 恢复时清零、健康判定含 `lastInjectionAt ≤5s`（`bridge.rs:149`），刚注入完必然判健康 → 退避实际不生效。**与 §4.1 是同一入口**（`launcher.rs:2879` 的 reset）。
- (b) `V8HeapProfilerConnection::Start()`：Codex 自家 renderer 的诊断能力，全仓 grep `heapProfiler` 零命中，非我们引入，但被高频重注入反复触发。
- (c) 每秒 80 次 `sysctl`：`launcher.rs:1129` 只在 Windows 上 spawn `run_pet_real_mouse_cursor_driver`，macOS 不跑，故该轮询来自 ChatGPT 自身。

**修复方案**：只需修 (a)，与 §4.1 完全同一改法（合入 PR #2337 的三条改动 + 分片同步）。(b) 在 issue 里回复：heap profiler 由 Codex 客户端自身启停，Codex++ 只做 CDP 求值，请用户用 `sample` 对比两次启动。(c) 判为上游。

**影响面**：同 §4.1。

**测试**：修 (a) 后 macOS 最小化再打开，`codex-plus.log` 不应出现每分钟一次的 `bridge.reinject_start`，CPU 应从 68% 回落到个位数。

### 4.3 #2363 + #2350 —— 会话展示与输入框丢失（P0，confirmed）

**现象**：更换皮肤后「会话展示及输入框丢失」/「聊天栏消失」。`8baffa91` 只修了一半。

**根因**：两条叠加的皮肤侧缺陷，落在 vendored 皮肤产物里（`crates/codex-plus-core/src/assets.rs:11-20` 用 `include_str!` 内嵌 `assets/inject/upstream/dream-skin/windows/dream-skin.css`，`10-style.js:1937` 整段塞进 `<style id="codex-dream-skin-style">`）。
1. `dream-skin.css:1419-1423` 的 `[class*="_MainContentTopFade_"] { display: none !important }` 是**子串匹配**，在 26.924+ 上同时命中挂在整条对话内容容器上的状态类与真正的遮罩层；规则带 `display: none !important`，命中容器即隐藏对话流 + 输入框整棵子树。`8baffa91` 只改了裸属性那一路（`dream-skin.css:1414-1417`），`[class*=` 这路仍是同一形态。该 CSS 由 `data-dream-art-wide="true"` 门控（`renderer-inject.js:422` 写该属性），与「更换皮肤后才复现」吻合。
2. 输入框周围白区：`dream-skin.css:907-916` 与 `1148-1149` 对 `:is(.composer-surface-chrome, [class*="_ComposerLayoutRoot_"], ...)` 设 `background: var(--ds-immersive-composer-solid) !important`，而 `1148` 那条（art-wide 分支）没有排除外层 `main` 的二级不透明层；`1409-1412` 的 `[data-app-shell-focus-area] [data-sticky]:not(:has([data-codex-composer-root]))::before` 只清了 backdrop 没清 background。历史会话比首页多两层背景，故只在历史会话暴露。

**修复方案**（改 vendor 产物 + 回归测试，别动主分片）：
- (a) 把 `dream-skin.css:1420` 的子串选择器收紧为属性精确匹配或排除容器，更稳的做法是照 `8baffa91` 的思路改用宿主 CSS 自己用的 `[data-app-shell-main-content-top-fade=visible] ._MainContentTopFade_gs442_2` 形态（只命中遮罩后代）。
- (b) **给 `display: none !important` 加范围守卫**：在 `1420` 规则上加 `:not(:has([data-testid="conversation-turn"], [data-message-author-role]))`，任何含对话内容的节点一律不隐藏。**这条比修选择器更值钱，是兜底**。
- (c) 白区：在 `1148-1156` 的 art-wide composer 规则里补 `:not(:has(> [data-codex-composer-root]))`，或把 `--ds-immersive-composer` 从「不透明面板色」降级为半透明（`dream-skin.css:56/106`）。
- 牵连：`assets.rs` 是 `include_str!`，改 vendor 文件即改二进制产物；`dream-skin.test.ts:121/133/151` 会按路径读这两份产物，改选择器必须同步改测试里的 `SELECTOR_CONTRACT` 断言；改完仍需跑 `node scripts/assemble-renderer-inject.mjs`（若 `10-style.js` 被牵连）。

**影响面**：只动 vendor 皮肤产物与测试；`codexAppThreadIdBadge` 等 opt-in 开关不受影响；不碰 per-profile 行为。

**需要补的测试**：`dream-skin.test.ts` 加「危险规则必须带 `:not(:has([data-message-author-role]))` 守卫」的断言（对 `dream-skin.css` 全文跑正则，覆盖所有 `display: none !important`）；加一条「不得存在未加内容守卫的裸 display:none」的静态断言。真机：独立 user-data-dir 起 CDP，统计 `main :is(.app-shell-main-content-top-fade, [class*="_MainContentTopFade_"])` 命中数与每个命中的 `clientHeight`，容器高度 > 200px 即回归。

> 归属注：#2350 / #2363 在另一组审计里被归到「会话数据/磁盘」主题，实为 dream-skin 样式误伤，与 8baffa91 / 2334 同族，两处结论一致。

### 4.4 #2362 —— 升级/重装报「无法写入 codex-plus-plus-manager.exe」（P0，confirmed）

**现象**：Win11 升级或重装时安装器报无法写入，重装也失败。

**根因**：`scripts/installer/windows/CodexPlusPlus.nsi:31-37`（安装段）与 `:60-75`（卸载段）在 `nsExec::ExecToLog 'taskkill /IM … /F'` 之后**零等待**立刻 `File` 覆盖写。taskkill 只发终止指令、不同步；镜像文件锁的释放（进程退出清理、WebView2 子进程树、杀软扫描）是异步的。卸载段 `Delete` 对占用文件静默失败，于是「卸载后重装」也继续失败。

**修复方案**：**采纳 PR #2368**（纯新增 +50 行）。`WaitForProcessExit` 宏在 taskkill 后用 `$SYSDIR\tasklist.exe` 输出 + `StrFunc.nsh` 的 `StrStr` 子串判断进程名（无管道、无 PATH 依赖、语言无关）轮询 500ms×20，等到过进程再补 500ms 锁释放缓冲，然后才 `File`/`Delete`；卸载段 `Delete` 后加残留检测弹窗。评估：三个已知坑（PATH 里 Git 的 Unix `find.exe` 劫持管道、Win11 tasklist 无匹配仍返回 0、语言相关输出）都规避了；等待宏无 `Abort` 分支、异常收敛到「立即放行」，tasklist 缺失时自动退化原行为。唯一提醒：它只修「进程正在退出」这一半；建议顺手在 `File` 前用 `IfFileExists` 循环重试并把失败明确提示用户。

**影响面**：只改 NSIS 脚本，不碰 Rust/注入层。CI `pr-build.yml` 已会 makensis 编译。

**需要补的测试**：实机矩阵——V2 双进程运行中安装、V3 进程锁死目标文件（应 20 轮后超时继续且不破坏被锁文件）、降级路径（tasklist 故意缺失）。

### 4.5 #2275 + #2257 —— `<image_resize_notice>` 插在 tool 结果之间导致 400（P0，confirmed）

**现象**：#2275 单点 400「No tool output found for tool call b」；#2257 是并行 `view_image` 的多点形态（每两条 tool 消息之间夹一条 developer，三个 tool_call 全部被判 orphaned）。协议切到 responses 直连上游同样失败。

**根因**：`crates/codex-plus-core/src/protocol_proxy.rs:3377` `enforce_tool_call_pairing` 用「连续 tool 消息」判定配对——`:3378-3390` 用 `take_while` 只收集紧跟在 assistant 之后、role 连续为 tool 的消息，中间插了 developer 就停止计数。夹心结构 `assistant(tool_calls=[a,b]) → tool(a) → developer → tool(b)` 里 followers 只数到 1 条，b 被判 orphaned 从 `tool_calls` 摘掉降级成文本——但 b 的 tool 消息还在原位置，这正是「role 'tool' 无前置 tool_calls」的来源。`relocate_tool_output_images`（`:3458`）只在 tool 区尾部插 user 消息；`append_responses_item` 的 `_` 兜底（`:3330-3350`）对 developer 只调 `flush_tool_calls`。全仓 grep `image_resize_notice` 零命中。

**修复方案**：在 `responses_to_chat_completions_with_options` 的消息后处理链（`protocol_proxy.rs:250-256`）插入重排步骤 `relocate_interleaved_non_tool_messages`，**位置必须在 `enforce_tool_call_pairing` 之前**（否则配对已被错误摘除，无法恢复）：
- 扫描每个 `assistant(tool_calls=[...])`，收集「本条 tool_call 集合尚未全部应答」的区间；
- 区间内遇到 role 为 system/user（developer 已映射成 system，见 `responses_role_to_chat_role` `:3699`）的非 tool 消息时暂存到缓冲区，继续吃 tool 消息，直到全部 `tool_call_id` 配齐；
- 把缓冲区消息整体移到该连续 tool 区之后。
- 注意 `collapse_system_messages_to_head`（`:3690`）会把所有 system 消息抽到头部，与「搬移到 tool 区后」冲突：**推荐给这条通知保留 role=user**（不参与 system 归并，语义上也更准），而不是在 collapse 里加豁免标记。
- 补一条防线：`enforce_tool_call_pairing` 摘 orphaned tool_call 时，把对应的 `role:'tool'` 消息一并移除或降级成 user（当前 `:3416-3440` 只处理 assistant 侧）。

**影响面**：改协议转换公共出口，responses 直通路径同样受益（该重排应在协议分流之前）；不影响 per-profile；不碰已发布字段名。

**需要补的测试**：直接复用 issue 的两条最小复现：
1. `user + function_call(a) + function_call(b) + function_call_output(a) + developer + function_call_output(b)`，断言每个 tool 消息前都能找到恰好一条含该 `tool_call_id` 的 `assistant.tool_calls`，且 developer 内容出现在 tool 区之后；
2. 三工具、每条 output 后跟一条 developer 通知的用例（复刻 #2257 ordinal 977-989 形状），断言不存在没有前置 `tool_calls` 的 tool 消息。
现有 `tests/protocol_proxy.rs` 的 4324 行 image_resize 零覆盖。

### 4.6 #2210 —— DeepSeek thinking 模式 `reasoning_content` 丢失（P0，confirmed）

**现象**：带工具调用的轮次报 reasoning_content 错误；**同一份历史重放能过、实机必炸**。

**根因**：`protocol_proxy.rs:3638` 起 `flush_tool_calls` 的 merge 分支丢 `pending_reasoning`——`:3658-3663` 当 messages 最后一条已是 assistant 时，直接 `merge_tool_calls_into_message` 然后 `return`，`pending_reasoning` 既没附加也没被 `mem::take`，随函数返回被丢弃；只有 `:3665-3677` 的新建 assistant 路径才写 `reasoning_content`。触发时序：reasoning item 后先出现不带 tool_calls 的 assistant 文本消息（`append_responses_item` 的 `_` 分支 `:3330-3350`，`pending_tool_calls.is_empty()` 时把 reasoning 附加到文本消息并 take），随后才来 function_call——此时最后一条是 assistant 且已有内容，走 merge 提前 return。`ensure_tool_call_reasoning_content`（`:3553`）只能补 content 与 reasoning_content 同时为空的占位，这里 content 非空、占位补不上、真实 reasoning 又被丢。

**修复方案**：改 `flush_tool_calls` 的 merge 分支——在 `merge_tool_calls_into_message` 之后，把尚未消费的 `pending_reasoning` 通过 `append_reasoning_to_assistant_message`（`:3704`）追加到同一条 assistant 上，再 return。即把 `:3658-3663` 改成先 merge，再 `if !pending_reasoning.is_empty() { append_reasoning_to_assistant_message(last, &std::mem::take(pending_reasoning).join("\n")); }`。该函数已有「不覆盖已有真实 reasoning、只在 content 缺失时补空串」的语义，直接复用。

**影响面**：改 `protocol_proxy.rs` 单点；不影响 per-profile；不碰发布字段。

**需要补的测试**：补 timing 用例 `reasoning → assistant 文本消息 → function_call → function_call_output`，断言带 tool_calls 的 assistant 消息同时有真实 content 和真实 reasoning_content。**注意现有 `responses_request_merges_reasoning_text_and_tool_calls_like_ccx`（`tests/protocol_proxy.rs:1234`）恰好绕开了 merge 路径**，所以一直没抓到——新用例必须真正进入 merge 分支。

### 4.7 #2294 + #2209 —— CUA runtime 白名单挡住浏览器控制（P0，confirmed）

**现象**：被判定为 API 接入后浏览器控制无法接入，报 `unsupported Codex auth method: apikey`。1.4.0 用户还看到 `bin/node_repl.exe` 报 `Unsupported native runtime component`，1.5.0 的 0.0.27 倒在更早的 manifest 判据上——**同一个「白名单太窄」根因的两个落点，别当成两个 bug**。

**根因**：`crates/codex-plus-core/src/native_browser.rs:20-25` 把 CUA runtime 的 SHA-256 与 manifest 哈希硬编码，只认 0.0.11 / 0.0.24（`for_manifest` `:89-99`）。链路：`reconcile_locked`（`:639-672`）先 `restore_all`，`:662-670` 读 manifest 调 `for_manifest`，0.0.27 两个哈希都不匹配 → `Err` → `reconcile_contract` 的 `if result.is_err()`（`:634-639`）把 `control.json` 写成 `requireIdentification:false`（fail-closed）→ `require-identification.mjs` 里 `control?.requireIdentification !== true` 直接 `return fallback()` 回落云端策略 → 云端对 API 接入判定不可用。另运行时的锚点也从 `new eh(...je...,sv)` 漂到 `new uh(...We...,wv)`，即使放开 manifest 白名单，`transform_binding`（`native_browser.rs:313-320`）的 `text.matches(anchor).count() == 1` 仍会失败。

**修复方案**（**不要再往 `RuntimeContract` 里堆第三个常量**——0.0.11→0.0.24 已证明一次版本即作废整张表）：
1. **认结构不认哈希**：`for_manifest` 改为解析 manifest JSON，校验结构不变量（required 字段、`bin/node_modules/@oai/browser-desktop/scripts/browser-service.mjs` 存在、`bin/node_modules/@oai/cua-repl/bin/cua-repl.mjs` 存在、版本号 ≥ 0.0.11），保留 `CURRENT_/pinned` 作为「已知良好」快路径；未知哈希从 `Err` 降级为「结构通过即接受，标记 `runtime_adapted_unknown`」。
2. **认锚点结构不认压缩名**：`transform_binding` 现在是 `text.matches(ANCHOR).count() == 1` + `!text.contains("cppNativeIdentificationReader")` 两条裸串判据。改成结构正则匹配 ``new \w+\(r,this\.clientApi,\(\)=>\w+\(this\.runtime\),this\.turnEndedTracker,\w+\)`` 的**唯一**命中，并把第三个捕获组（policy 回调名）带入替换串，这样 `sv` 与 `wv` 都能吃下，且仍拒绝多命中。补一层运行时自检：替换后要求函数名出现次数与替换前一致。
3. `crates/codex-plus-core/src/native_browser_connection.rs:76-80` 的扩展白名单加「同 family 但未登记 ID」的降级展示，让用户在 UI 上区分「扩展没连上」与「扩展连上了但版本不认识」。

**影响面**：`RuntimeContract` 是 `assets/native-browser/require-identification.mjs` 之外唯一改动点；`codex_app_native_browser_require_identification`（`settings.rs:497`）与 launcher 门（`apps/codex-plus-launcher/src/main.rs:437`）都不用动；`docs/native-browser-identification.md:54-66` 的哈希表必须同步更新，并删掉「未来版本不保证」那句与实现不符的免责。

**需要补的测试**：在 `native_browser.rs` 现有 `current_binding_uses_current_metadata_and_policy_callback`（`:1260` 起）旁加「0.0.27 形状」合成 service + 合成 manifest，断言 `transform_binding` 成功且产出串含 `cppNativeIdentificationReader(this.runtime,wv,We,`；回归：挖掉 `cua-repl.mjs` 字段后 `for_manifest` 必须 Err。真机按 `docs/native-browser-identification.md` 的 Validation 一节，注意文档已写明「已开启过标识的 Edge profile 不能证明首次启用」，必须用全新 user-data-dir。

### 4.8 #2173 —— 纯 API 下 Chrome 插件失败，三个根因必须拆开（P0/P1，confirmed）

**现象**：`unsupported Codex auth method: apikey`；`cua.getState()` 聚合报错；EDGE native host 报错。

**根因**：三个独立根因被混着讨论：
- **(a) 浏览器请求标识**：与 §4.7 完全同一根因（白名单过窄 → `requireIdentification` 写 false → 回落云端策略）。已在代码里被 PR #2208 / #2335 覆盖，只是版本表跟不上。
- **(b) app-server 认证**：`require-identification.mjs` 完全够不到。错误来自 CUA runtime 另起的 `codex app-server` 调 `getAuthStatus(includeToken=true)`，原生分支硬性要求 `auth_method == "chatgpt"`（#2188 第 2 节反汇编定位：VA 0x14032C9F9 读 authToken、0x14032CAF4 读 authMethod）。**检查发生在 HTTP 发出之前**，本地代理/注入改头/改元数据三处都兜不住。dongyu23 的「OpenAI 会话身份」能生效，正是因为它改的是 `model_provider` 让 `getAuthStatus` 改读 auth.json。
- **(c) `auth.json` 整体覆盖**：`crates/codex-plus-core/src/relay_config.rs:721-722` 的 PureApi 分支直接 `apply_relay_files_to_home(home, &compatible_config, &profile.auth_contents)`，把 profile 快照整体写进 auth.json；而 Aggregate（`:726-731`）与 Official（`:733-735`）都先经 `auth_contents_with_proxy_key`（`:3529-3559`）合并 live auth.json。**纯 API 供应商一切换就可能把 `tokens` 抹掉，从而静默摧毁 (b) 的唯一 workaround**——用户表现为「本来能用的插件某天开始报 apikey」，且没有任何配置报错。

**修复方案**：分三块，别混。
- **(a)** 见 §4.7，同一处改动同时解决。
- **(b) 如实承认修不了，别承诺**。能做的是：(i) 把「会话身份」入口从「供应商编辑页里一个被 disabled 条件的下拉」提到显眼位置——`apps/codex-plus-manager/src/App.tsx:8451-8468` 的 hint 现在只说「选择 OpenAI 后…中转仍使用 custom 表」，没告诉用户这是让 CUA 插件可用的开关；补前置条件检查（`relay_config.rs:3989-4003` 的 `auth_contents_looks_like_chatgpt_auth` 可复用），无 `tokens` 时明确提示「此选项无法解决插件问题」；(ii) 注入层加针对 `unsupported Codex auth method` 的识别与本地提示（照 `30-service-tier.js:1007-1055` `installDictationSupportPatch` 那套范式），**必须一开始就写成结构匹配**（见 2ce62815 的教训）并配真实反例测试；(iii) 把 #2188 第 2 节的链路结论固化进 `docs/`。
- **(c) 与 Aggregate/Official 对齐**：`relay_config.rs:721-722` 的 PureApi 分支在写 auth.json 前先读 live、保留其 `tokens` / `auth_mode`，只覆盖本 profile 真正需要覆盖的字段。

**影响面**：(b)(i) 只动前端表单，不动契约；(ii) 属「加载模块 + 源正则」类，是最易随 Codex 压缩失灵的写法，必须结构匹配；(c) 改 `relay_config.rs` 写入对称性，需同步核对该函数所有分支。

**需要补的测试**：
- 纯 API profile 切换后断言 `auth.json` 的 `tokens` 字段未被清除（这条直接锁住 (c)，是本次最有价值的一条）；
- 前端表单校验：`auth.json` 无 `tokens` 时选 openai 会话身份给出警告；
- (b)(ii) 若实现错误识别，照 `codexPluginBuildFlavorFilterSourcePattern` 的做法在契约测试里放两个换过压缩名的真实反例。

### 4.9 #2367 —— Anthropic 上游工具 schema 顶层 `oneOf` 导致整轮 400（P1，confirmed）

**现象**：Anthropic（Claude）上游工具 schema 顶层 `oneOf`，整轮 400。

**根因**：`protocol_proxy.rs:4202` `normalize_schema_object` 的循环（`:4247-4255`）对每个 key 递归 `normalize_schema_value` 后原样保留；唯一结构性改写是 `:4230-4245` 的 `len > 1 且带 $ref` 分支。zod-to-json-schema 生成的 `{type:"object", properties:{}, oneOf:[{$ref:"#/$defs/__schema0"},...], $defs:{...}}` 原封不动发给上游。全仓 grep `oneOf` 零命中（含 `assets/` 与测试）；`settings.rs:306` 的 `RelayProtocol` 只有 Responses / ChatCompletions 两档，无 Anthropic 分支。

**修复方案**：在 `normalize_chat_tool_parameters`（`protocol_proxy.rs:4201`）链路上新增 `flatten_top_level_combinators`，置于 `inline_ref_siblings` 之后（必须先内联，否则合并分支时 `$defs` 无法摊平）：
1. 顶层存在 `oneOf/allOf/anyOf` 时，先递归对每个分支做 normalize；
2. 摊平——`properties` 取各分支并集（同名属性若都是 object 则递归合并 `properties/required`，冲突时保留严格度更低的一方，`required` 取交集），`$defs` 合并；
3. 剥掉顶层组合器键，把 `type:"object"`/`properties`/`required` 补回；
4. 无法摊平（分支含非对象类型、循环 $ref、合并后 properties 为空）时降级：把各分支塞进一个 properties 下的私有字段并加 description；
5. 仍不合法则丢弃该工具并打 diagnostic log，**绝不让整轮 400**。
做成 profile 级开关（默认开）。**注意 `transfer_voice_call` 是顶层 anyOf、`complete_conversational_onboarding_task` 是顶层 oneOf**，三者走同一条路径，不能只处理 oneOf。

**影响面**：对 DeepSeek / Qwen 等不校验的上游无副作用（合法化后语义等价）；per-profile 隔离；不碰发布字段。

**需要补的测试**：`tests/protocol_proxy.rs` 加用例——输入 issue 里 `automation_update` 的原始 parameters（顶层 oneOf + $defs + 三个分支 $ref），断言转换后顶层不含组合器、含非空 properties、每个分支贡献字段都在 properties 里；再补一个合并失败的降级用例（分支为字符串类型），断言工具不消失而是带说明。现有 107 条测试里 `oneOf|anyOf|allOf` 零命中。

### 4.10 #2244 —— 冷启动 80~110 秒、约 9 GB 全量读取、全程无日志（P1，likely）

**现象**：冷启动 80~110 秒，两进程对 1.89 GiB 应用包约 9 GB 全量读取，全程无日志（用户采样 87.5/80.3/102.6/104.1/109.6 秒）。

**根因**：先纠正 issue 的推论——9.0 GB ÷ 1.89 GiB ≈ 4.4 倍，且启动链路里**没有任何代码读应用包内容**（`app_paths` 只用 `read_dir` 与 `AppxManifest.xml` 的 `read_to_string`）。所以「读的就是应用包」大概率是采样归属误判（Win32_Process 的读写字节是进程级累计，杀软与 WebView2 子进程树的 IO 可能被记到父进程头上）。真正可疑的是两处同步、无上限、无缓存的全量扫描：
- `crates/codex-plus-core/src/codex_sqlite.rs:294-339` `sanitize_logs_model_suffixes` 每次启动都对 `logs_2.sqlite` 跑 `SELECT rowid, feedback_log_body FROM logs WHERE feedback_log_body LIKE '%[%'`，把命中行的整段 body 全读进内存（`:324-327` 的 `rows`），再逐行正则替换。`feedback_log_body` 是完整请求/响应文本，命中行总量可达 GB 级，**没有「上次已清理」的短路**。
- `crates/codex-plus-core/src/launcher.rs:540-559` `sanitize_historical_model_suffixes` 在启动早期同步执行。

另外半数是启动顺序：`launcher.rs:583-615` 先等 helper 端口再 `launch_codex`，之后才 `ensure_injection`；`launcher.rs:257-280` 的 `ensure_injection` 是 `for attempt in 1..=120` + 每次失败 sleep 1s，**每次循环都在 `try_inject`（`:2669-2690`）里重建约 616KB 注入脚本文本**。

缺日志：`status.rs:7-16` 的 `LaunchStatus` 只有 `status/message/started_at_ms/debug_port/helper_port/codex_app/aumid`，没有阶段字段；`ensure_injection` 的重试只写 `diagnostic_log` 不写 latest-status。

**修复方案**（三档，从确定有收益的开始）：
1. **给 `sanitize_logs_model_suffixes`（`codex_sqlite.rs:294-339`）加短路与分批**：先 `SELECT COUNT(*) … WHERE feedback_log_body LIKE '%[%'` 做便宜预检，命中 0 直接返回（干净机器常态）；命中不为 0 时改用 rowid 分页逐批读、逐批写，避免全量 collect。
2. **给启动链路加阶段埋点**：`launcher.rs:497-660` 每个前置步骤（provider sync、ensure_windows_sandbox、sanitize_historical_model_suffixes、helper bind、launch_codex）前后各写一条带耗时的 diagnostic。**这是 issue 最痛的体验问题，也是本轮无法定到 confirmed 的直接原因**。
3. **`ensure_injection` 改写**：注入脚本循环外构建一次（`injection_script_with_settings` 返回值在循环内不变），120 次循环改指数退避（1s→2s→4s，封顶 5s），每 10 次下调日志级别。
4. **`status.rs` 给 `LaunchStatus` 加 `phase: Option<String>` 与 `progress: Option<u8>`**，在关键节点调 `save_latest`；manager 侧对 `status==starting` 且超阈值时禁用「重启」按钮。

**影响面**：新字段 `Option + serde default`，不改数据模型兼容性；`codex_sqlite` 只改清理时机与内存占用，不改 replace 语义；**不要把清理挪到异步后台**——历史 model 名里的 `[1M]` 后缀必须在 Codex 读取 rollout 之前清掉。

**需要补的测试**：
- Rust：给 `ensure_injection` 抽纯函数注入点（probe 闭包），断言重试间隔序列符合退避表、且 `injection_script` 只构造一次；
- `codex_sqlite.rs`：无命中时只跑预检不打开事务；命中超一批时结果与不分批一致；
- `status.rs`：序列化测试断言新字段默认 None 且旧 JSON 能反序列化。
- 实机：Windows 冷启动抓 Win32_Process 每秒字节采样，注入脚本构造次数与磁盘读量应显著下降。

### 4.11 #2267 + #2266 —— 界面卡顿 / Windows 输入法逐字蹦出（P1，likely）

**现象**：#2267 打开一段时间后输入卡、侧栏滚动卡；#2266 Windows 原生输入法「打完一句话才逐字蹦出」。

**根因**：主因与 §4.1 同源（每分钟一次的整份脚本重注入 + 全量 asset rescan 抢占 renderer 主线程数秒）。次要放大因素：`assets/inject/renderer-inject/10-style.js:2062-2068` 的梦皮 `MutationObserver` 监听 `document.documentElement` 且 `attributes:true`（有 `attributeFilter` 收窄，但长会话流式输出时 class 高频变化仍会反复 `scheduleEnsure`），配合 `:2069` 的 `setInterval(ensure, 4000)` 常驻。
**#2266 的独立候选**：`98-scan-schedule.js:59-82` 的 `shouldScheduleScan` 判据里**没有排除 contenteditable / composition 节点**，输入法上屏插入的 composition 节点可能被判为 scan-relevant，于是每个字触发一次全量 scan（含 `refreshCodexModelWhitelistFromScan`，`95-conversation-view.js:1456`）。这与「一个字一个字蹦出来」高度吻合。

**修复方案**：
1. 先合入 PR #2337 的三条改动（见 §4.1），这是卡顿主因。
2. `10-style.js` 梦皮 `ensure` 加可见性门控：`document.hidden` 时直接 return；长会话流式输出期间降频（`MutationObserver` 回调的 180ms debounce 提到 500ms，或连续 N 次无实际样式变化时跳 `setInterval` 兜底）。**注意 `10-style.js:1861-1867` 已有 `observer.disconnect()/clearInterval/removeEventListener` 清理路径，新增门控不要破坏它**。
3. **#2266 专项**：在 `98-scan-schedule.js` 的 `shouldScheduleScan` / `isChatContentMutation` 补「输入法组合阶段的变更不排 scan」——`mutation.target` 命中 `[contenteditable="true"], textarea, input[type="text"], .composer-*` 且 `mutation.type` 为 `characterData` 或 `addedNodes` 全为文本节点时返回 false；同时监听 `compositionstart/compositionend`，组合期间挂起 `scheduleScan`，结束后补一次。**不要放过真正的用户消息上屏**——那条走 `[data-message-author-role]` / `[data-testid="conversation-turn"]`，与 composer 内组合节点可区分。
4. 对第三方脚本引入可观测性：把「单脚本 CPU 采样超阈值」写进诊断日志。
改完必须跑 `node scripts/assemble-renderer-inject.mjs`。

**影响面**：全部在注入分片，`98-scan-schedule.js` 的调度是全链路共用，任何「滚动期间少扫」的改动要确认会话行按钮/皮肤在滚动后能恢复——**建议滚动结束时无条件补一次 `scan()`**；不碰 Rust 与 per-profile。

**需要补的测试**：契约测试断言产物里存在 `compositionstart/compositionend` 门控与 composer 选择器；梦皮门控写前端单测（模拟 `document.hidden = true` 时 ensure 不被调用，`dream-skin.test.ts` 可扩展）。实机（Windows）：原生输入法连续输入 30 个中文字，埋点统计 scan 调用次数应从「接近字数」降到 ≤2 次。

### 4.12 #2201 —— 重启 Codex 疯狂占内存（P1，likely）

**现象**：重启后内存持续增长、反应慢。

**根因**：issue 正文只有一句话；作者评论里「皮肤 MutationObserver 无过滤导致全量重扫」这条**已不成立**——`10-style.js:2062-2068` 已收窄，且 `01-registry.js` 的 `data-codex-plus-ext` 自喂防护已在位。剩下的可复现路径是 §4.1 的 60 秒重注入：每次都重建整套 DOM 监听与 observer，旧实例 listener 不会全部解绑（仓库里大量 handler 挂在 `window` 上但只有部分 `removeEventListener`，例如 `98-scan-schedule.js:131` 与 `92-extension-host.js:78` 只在特定路径解绑），长期运行累积。

**修复方案**：
1. 主修仍是合入 PR #2337（停掉 60 秒重注入，从源头止住 listener 累积）。
2. 补一层自愈：在 `40-backend-settings.js:89-90` 已有的 `codexPlusBackendGeneration` 世代号机制上扩展——每次注入把本世代的定时器/observer 句柄登记到 `window` 上的表，新世代启动时遍历上一代并 `clearInterval/clearTimeout/disconnect/removeEventListener`。**世代号机制已是现成的隔离点，不要引入第二套**。（PR #2337 作者实测 window 上留着 65 个 keydown。）

**影响面**：注入层，遵守「注册表持久、DOM 瞬态」；不碰 Rust / per-profile。

**需要补的测试**：前端单测模拟两次注入，断言第一次注册的定时器与 observer 在第二次启动后被清理（对登记表长度断言）。实机：连续重注入 N 次后统计 window 上 `addEventListener` 净增数应接近 0。

### 4.13 #2157 + #2304 —— macOS Dock 点击无法唤回窗口（P1，confirmed）

**现象**：#2157 最小化到 Dock 后点击图标无法打开；#2304 点右上角关闭（落 Dock）后点击不弹出。**同一根因的两个入口**。

**根因**：macOS 上 Dock 点击产生 `RunEvent::Reopen`，而 `apps/codex-plus-manager/src-tauri/src/lib.rs:207-216` 的 `app.run` 只匹配 `RunEvent::Opened`（且只用于 dreamskin:// 与会话分享 URL），**没有 `Reopen` 分支**。窗口被 `lib.rs:342-359` 的 `CloseRequested` 隐藏后（非 transient 时 `api.prevent_close() + window.hide()`），无任何回调把它 show 回来——唯一能 show 的 `show_main_window`（`lib.rs:464-470`）只被托盘（`lib.rs:302-321`）与 URL 路径调用。另 `lib.rs:475-495` 的 `focus_existing_manager_window` 只有 `#[cfg(windows)]` 实现，macOS 上二次启动走空路径（`lib.rs:27` 的单实例守卫失败后调它，什么也不做）。

**修复方案**：在 `lib.rs:207` 的 `app.run` 闭包里、与 `RunEvent::Opened` 并列加 macOS 分支：`if let tauri::RunEvent::Reopen { .. } = event { show_main_window(app_handle); }`。若想同时覆盖「从 Finder 再次双击 .app」，需给 `lib.rs:475` 的 `focus_existing_manager_window` 补 macOS 实现（例如 `NSApp activate` + 通过 loopback guard 端口通知已有实例 show，或改用 Tauri 单实例插件），**注意不要动 `lib.rs:519-575` 那段端口守卫逻辑**（它同时承担「已运行就不再启一个实例」的职责，改动风险高）；也**不要影响 Windows 那条 `manager.already_running` 分支依赖的 `startup_is_background()`（`lib.rs:364-372`）**。
**这条已在 PR #2365 实现，建议直接审并合并**（见 §9）。

**影响面**：只动 manager 的 Tauri 事件循环；不碰注入层、不碰 per-profile。

**需要补的测试**：实机（macOS）三层验证——最小化 → 30 分钟 → 点 Dock；点右上角关闭 → 点 Dock；Finder 双击 .app 与 Raycast 再次唤起。回归：`--transient` 模式（`lib.rs:352-357`）应仍直接退出而非隐藏；Windows `focus_existing_manager_window` 行为不变（`lib.rs:377` 的 `manager_launch_mode_tests` 已覆盖 `--background`）。

### 4.14 #2364 + #2227 —— 插件市场不显示 / 安装失败（P1，likely）

**现象**：#2364 打开增强也看不到插件；#2227 能显示市场但装不上。

**根因**：三条叠加。
1. **门控把「未加载」当「relay 模式」**：`60-plugin-marketplace.js:544` `pluginPatchDisabledInRelayMode() { return !codexPlusBackendSettingsLoaded || codexPlusBackendSettings.launchMode === "relay"; }`——后端设置没加载出来时**整个插件补丁链路 early return**（`:1:20 / 398 / 428 / 501` 四处守卫），而 `clearPluginPatchArtifacts()`（`:547-548`）是**空函数**。设置加载失败 → 补丁全不装 → 市场按官方策略过滤 → 用户看到「打开了增强但一片空白」，且无任何可观测降级信号。
2. **版本门控硬编码**：`20-menu.js:16-18` 三条版本线（`26.601.2237` / `26.616.0` / `26.803.0`），`:43-48/:62-69/:71-75` 拿 `codexPlusBackendSettings.codexAppVersion` 去比。`codexPluginMarketplaceRequestPatchStrategy()` 返回 `"unknown"` 时会去跑三分支全装（`:1437-1446` 的 else），代价是重复装补丁。
3. **安装侧改写有损**：`60-plugin-marketplace.js:57-77` 对 `install-plugin` 在 `marketplacePath` 以 `remote:` 开头时**删掉 marketplacePath 并改成 remoteMarketplaceName**（`:70-74`），再在 `:170-172` 用改写后的 params 发请求。若上游只认其中一种，这次改写就改出服务端不认的形状。且 `plugin_install_request_debug`（`:173-184`）在 #2227 的日志里一条都没有——说明**安装请求根本没走到打过补丁的通道**。

**修复方案**：
- (a) **门控解耦**（`60-plugin-marketplace.js:544`）：改成只认 relay——`return codexPlusBackendSettingsLoaded && codexPlusBackendSettings.launchMode === "relay";`；加载完成前走保守分支但要有诊断。同时把空实现的 `clearPluginPatchArtifacts()` 补上实际清理，或在 relay 模式下明确上报一次 `plugin_patch_skipped_relay_mode`。
- (b) **版本门控兜底**（`20-menu.js:43-48`）：`codexPluginUnlockStrategy()` 返回 `"unknown"` 时调用方已按 modern 处理（保留）；把 `codexPluginMarketplaceRequestPatchStrategy()` 的 `"unknown"` 显式归一到一个策略，不要「全上」；三条版本线加注释说明每次 Codex 大版本要复核。
- (c) **featured 过滤器判据补强**（`50-navigation.js:256-265`）：`isCodexPluginFeaturedFilter` 在 `filtered.length >= sample.length` 之外补一条「sample 里至少有一个 id 能过官方前缀/白名单判定」的弱证据，拿不到映射时返回 false（宁可不解锁精选区也不要误判）。
- (d) **安装改写非破坏性**：保留 `marketplacePath` 原值，只在缺 `remoteMarketplaceName` 时**补**一个字段，不要 `delete next.marketplacePath`（`:72`）。删字段不可逆，补字段对不认它的服务端无害。
- (e) **install-plugin 失败降级**：现在 `:195-203` 只上报然后 `throw`；识别出确是改写导致的失败时，用原始 params 重试一次。
- 牵连：改 `50-navigation.js` / `60-plugin-marketplace.js` 后必须 `node scripts/assemble-renderer-inject.mjs` + `--check`；`renderer-inject.test.ts:815-855` 三条正则契约测试要同步；`codexPlusExtensionRoutes` 白名单不受影响。

**影响面**：全部隔离在插件链路内，不动 per-profile；不碰 `window.codexPlus` 已发布类名。

**需要补的测试**：渲染层单测——构造 `codexPlusBackendSettingsLoaded = false` + `launchMode = "patch"`，断言 `pluginPatchDisabledInRelayMode()` 为 false（当前是 true，这就是回归点）；再补 `launchMode = "relay"` 为 true。安装改写单测——输入 `{marketplacePath:"remote:openai-curated", pluginName:"x"}`，断言输出**同时**保留 marketplacePath 与 remoteMarketplaceName（当前会删前者）。真机：抓 `/diagnostics/log` 确认 `plugin_install_request_debug` 成对出现。

### 4.15 #2258 + #2085 —— 对话居中宽度失效 / 不够宽（P1/P2，confirmed）

**现象**：#2258 Codex 新版改容器类名后宽度规则失效；#2085 宽度固定 900 不自适应。**两个独立缺陷**——即使 #2258 修好，900 这个常量仍不会自适应，别把其中一个当另一个的修复。

**根因**：
- #2258：`90-action-groups.js:386-395` 的 `conversationViewContentClasses` 是按旧版硬编码的类名数组（含 `max-w-(--thread-content-max-width)`、`pb-8`），`:420-433` 的 `conversationViewHasAllClasses` 用 `classes.every(...)` **全等匹配**，缺一个即 false；`conversationViewFindByClasses` 无任何长度候选或降级路径。用户实测新类名 `max-w-(--thread-body-max-width)`、`pb-8` 不再单独存在，各自单独就足以让匹配归零。归零后 `:703` `conversationViewApplyNativeWidth` 拿到 null，`:752-779` `conversationViewAlignNow` 在 `targets.length === 0` 时直接 return——**完全静默，无诊断上报**。对比 `:299` 同一文件找标题节点用的是多候选逗号选择器，明显更抗变更。
- #2085：`90-action-groups.js:700-708` 的 `conversationViewApplyNativeWidth` 直接 `const maxWidth = conversationViewWidth() + "px"` 写死，从不参考容器可用宽度。

**修复方案**：
- (a) 目标查找改多候选（`90-action-groups.js:420-433`）：保留旧清单作为候选之一，并列补 `max-w-(--thread-body-max-width)` 与结构性写法 `div[class*="max-w-(--thread-"][class*="mx-auto"]`；`conversationViewHasAllClasses` 改成「候选之一命中即可」，或直接改成 `document.querySelectorAll` 逗号选择器。
- (b) 去掉对非结构性样式类的硬依赖：把 `"pb-8"` 从 `conversationViewContentClasses` 删掉或降级为可选。
- (c) 兜底：两处类名都没命中时，用 CSS 变量反查宿主节点（`getComputedStyle(el).getPropertyValue("--thread-body-max-width")` 非空即是）。
- (d) **在 `conversationViewAlignNow` 的 `if (!targets.length) return;`（`:760`）处加一次性 `sendCodexPlusDiagnostic("conversation_view_target_not_found", ...)`**——静默是这次最贵的地方。
- (e) #2085 自适应：保持设置值为**上限**，实际写 `min(conversationViewWidth(), 可用宽度)`；可用宽度取 `conversationViewSessionRectFor(el)`（`:710-712` 已有）减去两侧留白（约 2×16）。计算放进现有两阶段批量（`:752-779` 已是先统一写 style 再统一读几何），**不要引入新的读-写交替**，否则退回 82fb0924 修掉的强制重排问题。建议加 `conversationViewEffectiveWidth(containerWidth)` 集中边界逻辑。
- 牵连：改了分片要重组产物；`renderer-inject.test.ts` 相关断言会动；`conversationViewMinWidth/MaxAllowedWidth/DefaultWidth`（`00-prelude.js:390-392`）的 320/4000/900 边界不变。

**影响面**：前端 localStorage + 后端设置，与供应商配置无关，per-profile 单值行为不受影响。

**需要补的测试**：喂两种夹具（旧版类名含 `pb-8` / 新版含 `max-w-(--thread-body-max-width)` 无 `pb-8`），断言 `conversationViewFindContentEl()` 在新夹具上也非 null（当前返回 null，即回归点）。#2085 单测：设置值 900 + 容器 1400 → 写 900；容器 600 → 写 600（当前两端都写 900）。

### 4.16 #2180 —— threadIdBadge reparent 导致新建对话后崩溃（P1，likely）

**现象**：新建对话几分钟后出现「ChatGPT 遇到了问题」错误页。owner 已把 issue 往 #2186（dispatcher 冻结）方向关闭，**这是误判风险**——#2186 修的是 26.908 的 service-tier 注入链路，与 reparent 无关。

**根因**：`60-plugin-marketplace.js:696-706` 的 `wrapThreadTitleForBadge`：`parent.insertBefore(wrapper, titleNode); wrapper.appendChild(titleNode);`——把官方 React 管理的标题节点移到新父节点下。调用点 `:735` → `:764` `refreshThreadIdBadges`（每次 scan 都调，`95-conversation-view.js:742`）。reporter 的隔离复现（React 19 + jsdom 26）结论决定性：改标题节点 key 替换节点时移除链路抛 `NotFoundError: The node to be removed is not a child of this node.`，不包装则正常。副作用时机「新建对话几分钟后」正对应标题从临时名转正式名/侧栏重排，与复现表的「替换节点」「插入兄弟节点」吻合。

**修复方案**（消除 reparent，改成不碰 React 节点的呈现方式）：
- (a) 首选 CSS 伪元素：给 row（`[data-app-action-sidebar-thread-id]`，`00-prelude.js:601`）加属性如 `data-codex-thread-id-label="[abc1234 09-30]"`，在 `10-style.js:88` 现成的 `.${threadIdBadgeClass}` 样式块里改成 `[data-codex-thread-id-label]::after { content: attr(data-codex-thread-id-label) }`。标题节点完全不动。
- (b) 次选独立覆盖层：把徽标作为行的绝对定位兄弟，不包 `titleNode`。
- (c) 迁移清理：`removeThreadIdBadges`（`:707-720`）目前做反向 reparent，改成只清属性；存量 wrapper 需一次显式清理（删掉所有 `[data-codex-thread-id-badge-wrap="true"]` 并把子节点还原，只跑一次）。
- (d) 在 issue 里回一句「#2186 修的是另一条路径，threadIdBadge 的 reparent 仍在 HEAD 上」。
- 牵连：`codexAppThreadIdBadge` 是 opt-in 且默认 false（`10-style.js:1320`），只影响开了开关的用户，可单独发补丁版；遵守「注册表持久、DOM 瞬态」，别缓存 `titleNode`；改后跑 assemble + npm test + `--check`。

**影响面**：opt-in 默认关，风险面小；不碰 per-profile。

**需要补的测试**：把 reporter 的最小复现（React 19 + jsdom 26）落成单测——挂载 `data-thread-title` 节点 → 执行改后标记逻辑 → `flushSync` 改 key 重渲染 → 断言不抛 `NotFoundError`（当前会抛）。另测标题临时名转正式名、侧栏插入/重排、开关关闭后的清理三个场景。

### 4.17 #2203 —— 供应商切换失败：外部 model_catalog_json（P1，confirmed）

**现象**：切换供应商报「当前 Codex 配置使用外部 model_catalog_json」。

**根因**：两处 `bail` 都是设计上的冲突拒绝，触发条件不同：
- `relay_config.rs:2411-2413`：config.toml 已有 `model_catalog_json` 指针且不是本 profile 的，且 `is_codex_plus_managed_model_catalog`（`:3040`）与 `is_cc_switch_model_catalog`（`:3032`）都不成立、指针不含未展开变量（`:3090`）时进 else。
- `relay_config.rs:2442-2444`：`live_external_model_catalog`（`:3022`）直接读 live 的 `model_catalog_json`。
两处都在 `has_per_model_overrides` 为真时 bail（用户配了带 `[窗口]` 后缀的模型、或设了 `model_metadata`、或设了每模型 auto_compact 百分比，`:2364-2373`）。
另：issue 评论里「PR #2197 / #2178 与本问题无关」的结论正确——#2197 是 max_context_window 适配（issue #2191），#2178 修 #2161 且未合并，**维护者之前的回复指错了**。

**修复方案**：改 `apply_model_catalog_to_config`（`relay_config.rs:2332`）——正确取舍是**接管而不是拒绝**：
1. 把 `is_codex_plus_managed_model_catalog` 从「路径形状」升级为「路径形状 + 文件内容是本程序生成的」（文件里有 `models` 数组且每条带 `priority` 即可判定）；
2. 对确认是外部用户手写的指针，**不要 bail**，改为「保留外部指针 + 在 config.toml 顶层写 `model_context_window` / `model_auto_compact_token_limit` 兜底」，并在返回值带 `warning` 字段，前端展示「当前使用外部 catalog，每模型窗口未生效」——失败让用户完全切不了供应商，代价远大于特性降级；
3. `warning` 字段加进 `RelayApplyResult` 与管理器 UI。
**注意这会影响 `preserves_user_model_catalog_json` 测试的既有契约，需同步改期望值。**

**影响面**：改 `relay_config.rs` 主链路与返回结构；不碰注入层。

**需要补的测试**：三条——外部指针 + 配了 `deepseek-v4-pro[1M]` 时断言切换成功且指针未被改写；外部指针 + 无 per-model 覆盖时断言仍走 `copy_standard_responses_catalog`；返回结果带 warning 且前端能渲染。另补反向测试：用户手写且内容非本程序生成的指针不被覆盖。

### 4.18 #2233 —— 选择聚合供应商时增强全部失效（P1，likely）

**现象**：选聚合供应商后 Codex 增强（汉化/插件/脚本）全部失效。

**根因**：注入链里**没有**「按 relay_mode 关掉注入」的代码路径（注入由 launcher 的 launchMode/桥接决定，与供应商 profile 无关；`assets/inject/renderer-inject/*.js` grep 不到 Aggregate 分支）。更可能是聚合模式的注入后置条件没满足：`relay_config.rs:3786-3788` 对 Aggregate 强制写 `requires_openai_auth = false`，`:3809-3820` 在 `has_model_routes / uses_no_auth` 时把 base_url 指向本地协议代理，`:3559-3569` 走 `auth_contents_with_proxy_key` 生成含代理 token 的 auth.json。若协议代理端口未就绪，或桥接在 Codex 连不上目标 base_url 时进入降级/重注入退避，用户看到的就是「增强全没了」。

**修复方案**：
1. 先取证据：让用户在聚合模式下导出 `codex-plus.log`，过滤 bridge/inject 条目，确认是「注入没跑」还是「注入跑了但页面被重载冲掉」；
2. 若是协议代理未就绪：在 `ensure_active_protocol_proxy_config_in_home`（`relay_config.rs:291`）之外补一条「协议代理端口不可达时不把 base_url 指向代理，而是保留直连并记录降级原因」；
3. 若确认是桥接重注入把注入产物冲掉，按 `e06c9fb5` 的既有模式把聚合场景纳入降级可见化范围。
**不要把注入开关做成 per-profile**——增强是否生效不应随供应商变化。

**影响面**：改 `relay_config.rs` 的代理就绪判定与降级可见化；不碰注入开关语义。

**需要补的测试**：契约测试——Aggregate profile 应用后生成 config 的 base_url 必须指向已绑定的本地代理端口（或明确回落直连），不允许出现指向未监听端口的地址。

### 4.19 #2081 —— 渠道模型更新后不刷新列表、旧对话不换模型（P1，confirmed）

**现象**：供应商渠道更新模型后 Codex 不刷新列表；旧对话不应用新选中模型。

**根因**：
- 根因 1（列表不刷新）：门槛在 `should_write_managed_model_catalog`（`relay_config.rs:2618-2633`）——只有 `metadata_overrides` 为真、或条目带 `suffix_window / auto_compact_percent`、或 `requires_bundled_metadata_catalog(slug)`、或官方 deepseek-v4、或 `standard_responses && has_model_routes` 时才写 catalog。普通无后缀的 relay 模型（glm-5.2、kimi-k3 等）全部落在门外，函数在 `:2464-2476` 直接返回。另一条独立不刷新路径在 `:2398-2423`（外部指针且无 per-model 覆盖时 `return Ok(config_text)`，`:2422`）。
- 根因 2（旧对话不换模型）：确认。`crates/codex-plus-core/src/codex_sqlite.rs:250-288` 的 `sanitize_thread_model_suffixes_in_db` 只把 `threads.model` 里带合法后缀的值剥成裸 slug（SQL `WHERE model LIKE '%[%'`，`:271`），**没有「把旧默认模型回填为新选中模型」的逻辑**；`crates/codex-plus-data/src/provider_sync.rs:4325` 只同步 `threads.model_provider`，不动 `model` 列。
- **与 #2374 直接对立**：`relay_config.rs:2394` 的注释与 `preserves_user_model_catalog_json` 测试把「用户手写指针不覆盖」当契约，而 #2374 抱怨的正是「每次切换都覆盖我的手动配置」。修一边必须同时处理另一边。

**修复方案**：
1. 放宽 `should_write_managed_model_catalog`（`:2618`）：对「本 profile 自己管理的 catalog（路径形如 `model-catalogs/<sanitized-id>.json`）」一律返回 true；把「不生成」契约收窄为「不创建 `model-catalogs` 目录下的新文件」。同时确认 `:2470-2475` 的「未重写则删指针」在新分支下仍可达。
2. 新增 `backfill_thread_models_to_default(home, previous_model, new_model)`，在 `codex_sqlite.rs` 里按 `UPDATE threads SET model = ?1 WHERE model = ?2` 只改「仍等于上一次默认模型」的行，不动用户手改过的会话；调用点放在切换成功后、且必须先探测 Codex 已退出（sqlite 无 WAL 写锁）——复用 `306d14ee / 26cedd25` 那套锁探测。**回填做成设置项而非默认开启**。

**影响面**：`relay_config.rs` 契约测试需同步；回填只影响 opt-in；不碰 per-profile 单值行为。

**需要补的测试**：relay_config 集成测试——profile 的 model_list 为纯无后缀自定义模型，先写旧 catalog 再切换，断言 `model-catalogs/<id>.json` 内容已更新（当前会失败）；codex_sqlite 测试——三行 model 分别为旧默认/用户手改/新模型，调回填后断言只有第一行被改。

### 4.20 #2146 —— %USERPROFILE%\.codex 被永久删除（P1，likely）

**现象**：使用过程中整个 `%USERPROFILE%\.codex` 被永久删除。

**根因**：审计全部 `remove_dir_all` 调用点，**没有一条能删到 `.codex` 的既定路径**——会话删除走 `crates/codex-plus-data/src/storage.rs`，`delete_local`（`:184`）按 schema 分派，rollout 文件逐个 `fs::remove_file`（`:615`）并带备份（`:1325`）与不可读时的显式报错（`:608-611`），无递归删除。其余 `remove_dir_all` 只作用于 staging/backup/lock 目录（`skills.rs:364/384/410/550`、`plugin_marketplace.rs:332/393/592`、`provider_sync.rs:1685/1708/1753/1802`）。兜底 `crates/codex-plus-core/src/codex_home.rs:26` 的 `ensure_safe_recursive_removal` 拒绝删除 CODEX_HOME 本身/祖先/文件系统根，规范化后判定，覆盖 Windows 盘符根与 UNC（`2edabfe6` / `dcaefeb6` / `c5b1db26`）。**该守卫目前只挂在 `install/mod.rs:128` 一处。**

**修复方案**：
1. 把 `ensure_safe_recursive_removal` 提升为强制约束：在 `codex_home.rs` 提供 `wrap_remove_dir_all(target, home)`，逐一替换那 11 处裸 `remove_dir_all`（skills 4、plugin_marketplace 4、provider_sync 4、install 1 已用）。
2. 给破坏性操作加审计日志：`diagnostic_log` 补 `destructive_op` 事件（操作名、目标路径、文件数、结果），至少覆盖会话删除、技能卸载、provider sync 的 quarantine 清理。
3. 备份位置独立于 `.codex`：确认 rollout 备份落在 `.codex-session-delete` 下而非 `.codex` 内部。
4. **待决策**：`codexAppSessionDelete` 默认值现在是 true（`settings.rs:467-468` `default_true`），涉及不可逆操作，建议评估改 default false 并在开启时提示——**这条改变存量用户行为，需你拍板**。

**影响面**：替换调用点不改变各目标目录的语义（当前都是固定安全目录），只在值被写坏时提供下限保护；审计日志新增不碰现有行为。

**需要补的测试**：逐处替换后对每个调用点补「传入指向 CODEX_HOME 或其祖先的路径，断言返回 Err 且内容未变」（`codex_home.rs:137-275` 已有同类断言可复用）；provider_sync 把 lock 路径伪造成 home 本身，断言隔离/释放流程不会递归删 home；审计日志断言删除后日志里有 `destructive_op` 事件。

### 4.21 #2343 + #2314 —— 找不到 Codex App 目录 / 手动选择无反应（P1，likely）

**现象**：#2343 26.924 后找不到 `codex.exe`；#2314 管理工具读不到 Codex 应用、手动选择也无反应。三条 issue（含 #2204）全部卡在同一句日志上。

**根因**：`crates/codex-plus-core/src/app_paths.rs:509-556` 的 `normalize_codex_app_path` 要求路径存在、且 `executable_in_dir(path).is_some()`（`:531`）或 `path.join("app")` 下能枚举到 exe 且包名像 `OpenAI.Codex_*`（`:540-544`）。WindowsApps 的 ACL 第三方进程常枚举不了 exe，于是 `:536` 的 `is_codex_store_package_dir` 兜底成了唯一出路——而它的前提是最后一段能过 `codex_package_parts`（`:953-974`）。判据失败时 `normalize` 返回 None，落到 `resolve_codex_app_dir(None)` 自动探测，探测也失败时 `launcher.rs:792-802` 直接 `anyhow!("Codex App directory not found")`。**这是 #2204 / #2314 / #2343 日志里那条 `launcher.failed` 文案**，四个平台四种失败原因共用一句。#2314 的「手动选择也无反应」是多一层：`commands.rs:6333-6347` 的 `load_overview_payload` 里 `normalize_codex_app_path` 返回 None 时 UI 静默显示未检测，调用方不区分「路径无效」与「不是 Codex 目录」。

**修复方案**（**先补可观测性再修判据，不能盲改**）：
1. **第一步（可立即做）**：在 `DefaultLaunchHooks::resolve_app_dir`（`launcher.rs:792-802`）的报错里带上探测轨迹——试过的每个候选根、`normalize_codex_app_path` 的拒绝理由（不存在/是 Codex++ 目录/无 exe/包名不匹配）写进 `diagnostic_log` 与错误消息。
2. **第二步（拿到日志后再定）**：若确认卡在包名判据，把 `:540-544` 的兜底从「包名像 `OpenAI.Codex_*`」放宽为「父链上任意一段像，或同目录存在 `AppxManifest.xml`」。**牵连**：`packaged_app_user_model_id`（`:693-712`）依赖 `package_name_from_app_dir` 反推 AUMID，放宽判据后必须保证 AUMID 仍能从 manifest 真读到，否则把 #2308/#2310 重新打开。
3. **把拒绝理由传到前端**：`normalize_codex_app_path` 拆出返回 `anyhow::Result<PathBuf>` 的内核（保留现有 Option 薄壳），在 `commands.rs` 的选择路径上走内核并把错误文案回给前端。

**影响面**：纯可观测性 + 判据放宽；不动判据语义时可零风险先合第一步；不碰 per-profile 与注入层。

**需要补的测试**：`app_paths.rs` 补「多根下第二个根命中」；「所有根都空时返回 None 且错误文案含已搜索根列表」；「包名不匹配但存在 `AppxManifest.xml` 的目录应被接受」；「Codex++ 安装目录仍必须被拒」。normalize 内核分别对四种输入返回可区分的错误/成功。

### 4.22 #2351 —— 提权运行时 AUMID 激活失败被静默降级（P1，confirmed）

**现象**：提权运行时报「该进程没有程序包标识符」，且状态误报 running。

**根因**：两层叠加。第一层 `launcher.rs:1004-1031`：`ActivateApplication` 失败后只写一条 `launcher.packaged_activation_fallback` 诊断，就落下去走 `build_codex_command` 按路径直接执行 `WindowsApps\…\ChatGPT.exe`。这个回退本意是兜底 #2308/#2310，但只有「激活失败」一个信号，无法区分「门牌号变了（按路径可用）」与「提权（按路径必死）」——提权进程激活 MSIX 被 Windows 拒绝，按路径拉起的进程没有包身份，`GetPackageFamilyName` 返回 15700 / 0x80073D54，应用自检失败、9229 永不 LISTENING。第二层 `launcher.rs:605-666`：spawn 本身「成功」，状态写成 running；注入失败只降级 `running_degraded`，**这条链路上永远不会落到 failed**。

**修复方案**：**采纳 PR #2371**（+59 行，`launcher.rs` 与 `windows_integration.rs`）：
- `windows_integration.rs` 新增 `current_process_is_elevated()`（`IsUserAnAdmin`，非 Windows 恒 false）。**必须新写而不是复用 `relay_config.rs:2185` 的私有 `windows_process_is_elevated`**——后者在非 Windows 上刻意返回 true 以保留 sandbox 设置语义，语义不同。
- `launcher.rs` 把回退决策抽成纯函数 `packaged_activation_fallback_block_reason(is_elevated)`：提权 → `bail!` 带「取消 属性→兼容性→以管理员身份运行 勾选」指引，错误沿既有 `Err` 分支如实写 `latest-status.json` `status=failed` 并清理已启动进程；非提权 → 保留原回退，零行为变化。
- 对「非提权代理激活」的取舍（PR 作者判断正确）：explorer 代启动会丢 `--remote-debugging-port`，受限令牌降权 spawn 基建复杂且易被杀软拦；Codex++ 全家（NSIS `RequestExecutionLevel user`、manager、launcher）都是用户级设计，与 Codex 应用的通信走本地 TCP，无任何功能需要管理员身份。

**影响面**：非提权路径零行为变化；把「提权」与「#2308/#2310 AUMID 解析失败」在日志上分开（两者文案相同，过去排查成本极高）。

**需要补的测试**：`cargo test -p codex-plus-core --lib launcher::tests`（PR 报 27 passed，含新增 3 项）：提权→回退被禁止且错误含「管理员」；非提权→返回 None 保留回退；CI/开发 shell 下 `current_process_is_elevated()` 为 false。真机：勾选后冷启动应得 failed + 指引；取消后恢复 running 且 `hasBridge: true`。

### 4.23 #2354 —— Windows 安装器未注册 URL 协议（P1，confirmed）

**现象**：`codexplusplus://` 网页深链导入在 Windows 全部失效。

**根因**：`apps/codex-plus-manager/src-tauri/tauri.conf.json` 无 deep-link 插件配置、`capabilities/default.json` 权限只有 `core:default + dialog:default`、`Cargo.toml` 无 deep-link/single-instance 依赖——确实没有 Tauri 侧深链通道。但 `apps/codex-plus-manager/src-tauri/src/main.rs:7-23` 走的是 argv 解析 + `focus_existing_manager_window`（Win32 唤起已运行实例），而 `lib.rs:208` 的 `RunEvent::Opened` 带 `#[cfg(target_os = "macos")]`。**argv 那套在 Windows 上本来就够用，缺的只是注册表**：`scripts/installer/windows/CodexPlusPlus.nsi` 全文没有任何 `Software\Classes` 写入，而运行时的 `install/windows.rs:172-200` 的 `register_url_protocol` 覆盖 `codexplusplus` 与 `dreamskin` 两套键，却只被 `install_entrypoints`（`commands.rs:4143`）这个用户手动点的按钮调用，**manager 启动时不自检**。macOS 走 `Info.plist` 的 `CFBundleURLTypes`，落位即生效。

**修复方案**：**采纳 PR #2370** 的安装段改动（写 `codexplusplus` + `dreamskin` 两个协议键，值格式 `"<INSTDIR>\codex-plus-plus-manager.exe" "%1"`，键名与文案与 `install/windows.rs` 的 `register_url_protocol_key` 逐字一致；卸载段 `DeleteRegKey` 对应清理）。**强烈建议一并做「启动时幂等自检」**：校验 `HKCU\Software\Classes\codexplusplus\shell\open\command` 是否等于 `current_exe` 的 `"%1"` 形式，不符则重写——安装器只覆盖新装用户，存量用户不会重装却会开软件，exe 路径一旦变（换目录、绿色版解压）深链就静默指向旧路径。判据注意：别把「读不到键」误判成「需要注册」而在用户没同意时改注册表。

**影响面**：纯注册表写入，不碰 CLI 参数、不碰 per-profile、不碰注入层。

**需要补的测试**：`install/windows.rs` 契约测试——`register_url_protocol_key` 生成的 command 必须是 `\"<path>\" \"%1\"`；再从 `.nsi` 文本断言同款字面量（防两侧漂移）。实机：`rundll32 url.dll,FileProtocolHandler codexplusplus://…` 观察进程是否被调起。

### 4.24 #2339 —— 卸载项残留：注册表键名与运行时不一致（P1，confirmed）

**现象**：升级到 1.4.0 后卸载项残留（issue 顺带反馈，容易被修主诉时漏掉）。

**根因**：两侧键名硬编码不一致。NSIS `scripts/installer/windows/CodexPlusPlus.nsi:50-57` 写 `Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex++`，卸载段 `:78` 也只删这个键；运行时 `crates/codex-plus-core/src/install/windows.rs:8-10` 却把 `CodexPlusPlus` 当正式键、`Codex++` 当 legacy。于是经安装器装过又被管理工具碰过的机器留两条；反之运行时装过、之后走 NSIS 卸载的机器，`CodexPlusPlus` 那条永远删不掉。同源问题：`InstallLocation` 也不同步（NSIS 写 `$INSTDIR`，运行时写 `manager_path` 的父目录）。

**修复方案**：统一到 `CodexPlusPlus`。改 `CodexPlusPlus.nsi`：安装段写 `Uninstall\CodexPlusPlus` 并在写之前 `DeleteRegKey` 掉 `Uninstall\Codex++`（`:54-57` 的 6 条 `WriteRegStr` 改键名，`:78` 的 `DeleteRegKey` 加一条 legacy 清理）。**别顺手删掉 legacy 清理分支**——存量用户靠它收尸。

**影响面**：纯注册表；不碰 CLI 参数、per-profile、注入层。

**需要补的测试**：`crates/codex-plus-core/src/install` 补断言 `build_windows_entrypoint_plan` 产出的 `legacy_uninstall_key == "Codex++"` 且 `uninstall_key == "CodexPlusPlus"`；再写一个读 `.nsi` 文本断言含 `Uninstall\CodexPlusPlus` 的测试（沿用仓库「断言读文本」的风格）。

### 4.25 #2160 + #2222 —— provider sync 前置读取失败打断启动 / 误认 ChatGPT Classic（P1）

**#2160 根因**：`crates/codex-plus-data/src/provider_sync.rs:1136` 的 `format!("Provider sync skipped: {err}")` 把底层 serde_json 原始解析错误拼进用户文案，而 err 来自前置状态读取（`%USERPROFILE%\.codex\.codex-global-state.json`）。两条报错都是 serde_json 原文：`column 1` 是空/截断文件；hex escape 那条（threweird0 已查明）是文件里两处孤立的 UTF-16 高代理项 `\ud8ce`——.NET 与 Python 都认为可解析，只有 serde_json 严格模式拒绝。放大点在 `apps/codex-plus-launcher/src/main.rs:715`：provider sync 未完成时 `launcher_main` 直接 `anyhow::bail!`，整次启动被前置状态读取失败连带打断。

**#2222 根因**：`crates/codex-plus-core/src/watcher.rs:809-824` 的 `is_macos_codex_desktop_main` 只从参数抠出 `.app/Contents/MacOS/` 后的可执行名，匹配 `"Codex" | "Codex Dev" | "ChatGPT" | "ChatGPT Dev"`。ChatGPT Classic.app 的主可执行文件恰好叫 `ChatGPT`，于是命中，`provider_sync.rs:896-905` 误判 Codex 仍在运行。这也解释「之前版本没有、近期才出现」——同名产物是后来才纳入匹配列表的。

**修复方案**：
- #2160 三步：(1) 把 provider sync 的前置状态读取失败与「同步本身失败」区分开——`codex_app_state` 的 snapshot/load 失败降级为「跳过 app-state 同步、继续启动并写诊断」，参考 `codex_app_state.rs:84-94` 的 `capture_app_state_snapshot_nonfatal` 模式；(2) **对孤立代理项做容错**：解析前把 `\uD800-\uDFFF` 区间里不成对的转义序列替换为 U+FFFD（在原始文本上做，不依赖完整解析）；(3) 用户可见文案去掉 serde_json 原始错误，给「全局状态文件 X 存在无法解析的字符，已跳过并在 <备份路径> 保留原件」。**注意 provider sync 的「必须停 Codex 应用」语义不能被降级绕过**，那是安全前提。
- #2222 升级判据：`watcher.rs:809-824` 先从 args 切出 `.app` 路径，再用该 `.app` 的 `Contents/Info.plist` 读 `CFBundleIdentifier`，**只认 Codex 桌面应用的 bundle id**；复用既有的 `macos_app_plist_value`（`app_paths.rs`，`build_codex_executable` 在 `:587` 用它读 `CFBundleExecutable`），不引入新依赖。保留可执行名匹配作为二级判据但把 `"ChatGPT"` 降级（仅 bundle id 缺失时才按名字命中）。**注意 `find_codex_processes` 是无端口版本，不能要求命令行含 `--remote-debugging-port`**（那条只适用于 `:772-790` 的 for-debug-port 变体）。方向必须是「更准」而不是「更松」——这是 provider sync 的前置守卫，放宽会导致真 Codex 在跑时也放行 → 锁竞争。

**影响面**：改数据层与启动降级策略 + watcher 判据；不碰 per-profile 与注入层。

**需要补的测试**：
- #2160：构造含孤立高代理项的 `.codex-global-state.json`，断言解析不失败、原件被保留/备份、launcher 不因前置读取失败而 bail；
- #2222：`watcher.rs` 已有 `macos_codex_process_ids` 单元测试（`:290-304`，第 300 行 `/usr/bin/open -W -a /Applications/ChatGPT.app` 正是帮凶），补两个反例——ChatGPT Classic.app（bundle id 非 Codex）不得命中；Codex.app 的 helpers 路径不得命中。

### 4.26 #2359 —— Windows 白色主题下黑底黑字（P1，confirmed）

**现象**：Windows 白色主题下 Codex++ 页面黑底黑字完全看不清，切深色主题反而正常。

**根因**：`assets/inject/renderer-inject/10-style.js:1123-1140` 定义面板语义变量，`:1128 --codex-plus-bg-primary: var(--color-token-bg-primary, var(--token-bg-primary, #fff))`、`:1132 --codex-plus-text: var(--color-token-text-primary, var(--token-text-primary, #171717))`；页面容器硬规则在 `:1238-1242` 用 `!important` 覆盖成 `var(--codex-plus-bg-primary)`。问题在三级回退链的中间那级：Codex 提供 `--color-token-*` 还是 `--token-*` 会随版本与主题切换变化，两层回退同时落空时前景背景同明度。
**同类隐患（独立条目）**：`10-style.js` 里散布写死的深色系色值——`:969/:985`（`#3f3f46` + `#f3f4f6`）、`:1018`、`:1103/1108/1115`（`#f3f4f6/#f8fafc/#a1a1aa/#71717a/#9ca3af` + `rgba(255,255,255,.05/.16)`）、`:705/:803-805`——深色下正常，浅色下无对比度。这不是「黑底黑字」的直接原因，但属同类不同侧面。

**修复方案**：
1. **先探真值再改**：用 CDP 连独立 user-data-dir 起调试端口，dump `document.documentElement / body` 的 computed style，把 `--color-token-*` 与 `--token-*` 两套里实际有值的变量名与值列出来（浅色/深色各一次）。仓库 `scripts/probe-extensions-cdp.mjs` 可作模板；**不要猜 token 名，去问真机**。
2. 拿到真值后二选一：两套都稳定 → 回退链收敛成「只从确实存在的那套取，前景背景取自同一套」+ 显式浅/深分支；同名 token 会漂移 → 改成分主题显式取值（`[data-theme=light] / [data-theme=dark]` 或 `prefers-color-scheme` 分支），不靠三级 var 回退碰运气。
3. 删掉 `:1128` 那类会静默失效的回退名（如 `--color-token-bg-elevated-secondary` 若不在 Codex 提供集合里）。
4. **硬编码色值**：只改「文字与承载它的背景成对出现且两者都写死」的位置（`:1103-1115` 的 ad 系列优先），改成成对的语义变量；装饰性边框与 rgba 覆盖可以留着。**这是独立验收项**，避免主修复被美术改动淹没而难回滚。
5. **任何 `10-style.js` 改动都要先把 `codexDeleteStyleVersion` +1**（见 `00-prelude.js:413`），否则 `installStyle`（`:1-3`）会因版本号相同而静默不重建 style 标签，改了等于没改。改完跑 `node scripts/assemble-renderer-inject.mjs` 与 `--check`，并同步 `renderer-inject.test.ts` / `dream-skin.test.ts`。

**影响面**：注入层样式；不碰 per-profile 与数据模型。

**需要补的测试**：先补探针（CDP dump 两套 token 真值），再固化成契约测试——断言容器背景与前景文字取自同一套 token 命名空间，且不引用已知不存在的变量名。

### 4.27 #2140 —— 启动报「找不到包 0x80073CF1」（P1，confirmed）

**现象**：Codex 装在 `D:\aitool\OpenAI.Codex_26.901.6511.0_x64__2p2nqsd0c76g0\app`（非标准 WindowsApps 路径），启动报 0x80073CF1。

**根因**：`launcher.rs:988-993` 在 `cfg!(windows)` 且 `build_packaged_activation` 返回 Some 时**无条件走 AUMID 激活**；`build_packaged_activation`（`:2728-2738`）拿 AUMID 靠 `app_paths.rs:693-712` 的 `packaged_app_user_model_id`——它先用 `package_name_from_app_dir`（`:752-760`）从路径末段反推包名，再 `codex_package_parts`（`:953-974`）拆 publisher id，最后优先读 `AppxManifest.xml` 的 Application Id、失败才退回硬编码 `"App"`。在非 WindowsApps 的复制/搬移目录上，路径命名恰好符合包形状，于是反推出 AUMID `OpenAI.Codex_2p2nqsd0c76g0!App`，但该包在此目录并未被系统注册，`ActivateApplication` 报 0x80073CF1。**判据「目录名像 MSIX 包」被当成了「这个包已注册」的证据。**

**修复方案**：把「包名像」与「包已注册」分开判定。改 `app_paths.rs`：`packaged_app_user_model_id`（`:693-712`）在返回 AUMID 前，先用已存在的 `registered_windows_packages`（`:134-146`，走 `GetPackagesByPackageFamily/GetPackagePathByFullName`）确认该包族在当前用户下注册；未注册返回 None。返回 None 后 `launcher.rs:983-988` 的 `if let Some(activation)` 自然落空，走 `:1095` 的 `build_codex_command` 按路径启动——对「非标准路径下的合法副本」正确。
**与 #2351 协同**：两者判据正交（提权 vs 包注册状态）且互不冲突，但都在同一个 `if let Some(activation)` 分支上，改时保证两个条件都按预期生效。**牵连**：`registered_windows_packages` 不得跨启动周期缓存（`:130-133` 已有注释说明 AppX 更新会改 full name 与安装目录）。

**影响面**：改 `app_paths.rs` 判据；非 Windows 无影响；不碰 per-profile。

**需要补的测试**：目录名像 MSIX 包但系统未注册时 `packaged_app_user_model_id` 返回 None（探测函数需可注入）；已注册时仍返回 manifest 里的 Application Id（守住 #2308/#2310 的修复）。

### 4.28 #2345 —— 自定义供应商无法为模型单独配置图片处理（P1，confirmed）

**现象**：为模型单独配置 strip/vlm 后保存，全部变回「原样发送」。

**根因**：两处叠加。
1. **后缀与 map key 不匹配**：UI 行名允许带窗口后缀（如 `deepseek-v4-pro[1M]`），而服务端按**原始 model 字符串**查 map：`crates/codex-plus-core/src/vision.rs:122` 的 `image_handling_mode` 做 `map.get(model)` 全字匹配，**不剥后缀**（对比 `model_suffix.rs:385` 的 `normalized_model_slug` 是剥后缀的）。用户带后缀保存的 key 与请求里不带后缀的 model 对不上，查不到就回落 `SendAsIs`（`vision.rs:130`）。
2. **保存前读回覆盖**：`apps/codex-plus-manager/src/App.tsx:7518` 的 effect 依赖含 `profile.modelVlm`，保存成功后 draft 变化触发 effect，重新执行 `modelWindowRowsFromProfile(...)`，该函数（`model-windows.ts:104-148`）只认 `'vlm'/'strip'`，其余一律给 `'send-as-is'`（`:147`）。任何 key 对不上都会在这里静默重置。
（序列化侧 `model-windows.ts:159-183` 本身是对的。）

**修复方案**：统一 key 归一化，两处都改。
- 前端：`modelWindowRowsFromProfile` 与 `serializeModelWindowRows` 里对 model 做 `modelSlugFromRowName`（`model-metadata.ts` 已有）归一后再读写 `modelVlm` map；`vlmMap` 的未知值不再静默转 `send-as-is`，保留原值让校验报错。
- 后端：`vision.rs:122` 的 `image_handling_mode` 改用 `crate::model_suffix::parse_model_suffix(model).0` 归一后再查，并补一次「去后缀 key」的二次查找作兼容（历史数据里带后缀的 key 也能命中）。唯一调用方是 `protocol_proxy.rs:1734`。
- **不动已发布的 `modelVlm` 字段名与值域**（第三方脚本按字面量依赖）。

**影响面**：per-model 配置正确性问题的公共根；不改字段契约。

**需要补的测试**：`vision.rs` 补 `image_handling_mode` 用例——map 用带后缀 key、查询用不带后缀 model（及反向），断言两条都命中。前端 `model-windows.test.ts` 补「带 `[1M]` 后缀的行选 vlm，序列化再解析回来仍是 vlm」的往返用例（当前缺失的最直接回归）。

### 4.29 #2116 —— 微信链接不可用：sandbox variant 用了 kebab-case（P1，confirmed）

**现象**：微信链接不可用，app-server 拒绝 sandbox variant。

**根因**：Codex++ 内部以 kebab-case 存储沙箱档位，三处同一套映射且都是 kebab-case：`settings.rs:1909-1911`（`"workspace-write" => "workspace-write", _ => "read-only"`）、`crates/codex-plus-core/src/connect/mod.rs:457-459` 与 `:469`、`connect/app_server.rs:493-495`。`app_server.rs` 是真正构造 app-server 请求的地方，`:493-495` 的 match 只做内部档位透传与兜底，**没有任何地方把它转成 camelCase**。

**修复方案**：在 `connect/app_server.rs:493` 起的 match 旁新增 `sandbox_variant_wire_value`，把内部档位映射为 app-server 接受的枚举：`read-only → readOnly`、`workspace-write → workspaceWrite`、`danger-full-access → dangerFullAccess`。**只在构造 app-server 请求那一处转换**，内部存储与设置界面保持 kebab-case。确认 `connect/mod.rs:457` 那条 match 是否也在构造请求路径上——是则复用同一函数，若只是内部归一化则保持原样。参照 `ad76ae51` 的既有处理保持一致。

**影响面**：纯字符串映射，不影响 `settings.json` 既有值与 UI 文案。

**需要补的测试**：`connect/mod.rs` 的 `#[cfg(test)]` 块（`:500` 附近已有 sandbox config 断言）补三条映射用例。纯字符串映射，单测即可完全覆盖。

### 4.30 #2340 + #2199 —— 会话删除后被 catalog repair 复活（P1，likely）

**现象**：#2340 对话无法删除和归档；#2199 会话删除失效。**现象与直觉相反——不是删不掉，是删了又被写回索引。** #2199 的「切到设置再返回（= 触发一次 repair）就复活」是最佳时序指纹。

**根因**：删除走删 rollout jsonl + 删 threads 行，而侧边栏索引由 catalog repair 重建。可疑点在 `crates/codex-plus-data/src/provider_sync.rs:4744` 的副本择优逻辑：`source_updated_at` 相等时用「ineligible 一方胜出」防复活，但另一份 catalog 副本若 `source_detail`（rollout_path）仍指向未清理的路径，`rollout_exists`（`:4685-4689`）就仍为真，`eligible`（`:4735-4741`）成立，已删会话被重新写回。`eligible` 只看 `archived == 0 && has_user_content == 1 && rollout_exists`，**没有「刚被显式删除」这个状态**。
**正交线索**：用户说「归档反而等于删除」——归档置 `archived=1` 恰使 `eligible` 为假而逃过复活，从反面印证 `eligible` 缺一个删除态判断。

**修复方案**：在 catalog repair 里引入「删除墓碑」：
- 删除会话时除删 threads 行与 rollout 文件外，把 `thread_id` 写入一张本地 tombstone 表（或在 catalog 副本里保留删除标记）；
- repair 的 `eligible` 判定（`provider_sync.rs:4735-4741`）增加 `&& !tombstoned(thread_id)`，且 `rollout_exists` 为假时不得作为复活依据；
- 把 `:4744` 的平局规则从「ineligible 胜出」改为「显式删除标记胜出」——现有规则只在双方都是 catalog 副本、都未被显式删除时才成立。
- **opt-in 隔离**：tombstone 只影响索引重建，不得改 per-profile 单值行为；不得让 provider sync 在 provider 切换（`e6bdac1a` 那条路径）时把 tombstone 误当有效记录同步过去。**不要把归档与删除合并语义**——归档是 `archived=1` 的正常状态，删除必须走独立标记。

**影响面**：改数据层索引重建；不碰注入层与 per-profile 单值。

**需要补的测试**：
1. 写入一个会话 → 删除 → 触发一次 catalog repair（模拟切页面）→ 断言该 `thread_id` 不在索引里；
2. provider 两侧各留一份副本、时间戳相同、其中一份已删，断言删除方不被复活；
3. 语义隔离：归档的会话不因 repair 被转成删除、删除的会话不因 repair 变成归档。

---

## 5. P2 / P3 与需求类

以下条目不进 P0/P1 详表，按类型给出方案与工作量。工作量：S < 1 天，M = 1~3 天，L > 3 天。

### 5.1 confirmed 缺陷（需修，非 P0/P1）

| issue | 主题 | 一句话方案 | 量 |
|---|---|---|---|
| #2302 | `model_context_window = 1` 导致任务重复执行 | 前端 `App.tsx:8302-8304` 的 `replace(/[^\d]/g,"")` 改保留输入 + 校验换算（`1M`→`1000000`）；Rust `apply_context_limits_to_config`（`relay_config.rs:2301-2316`）加物理下限（< 1024 返回 Err）。**下限只作用于 `context_window`/`auto_compact_limit` 两个根键，不碰 catalog 里的 per-model 窗口** | S |
| #2235 | 工具 schema 顶层 `oneOf/anyOf/allOf/enum/const/not` 导致 400 | `protocol_proxy.rs:4223` 的 `normalize_schema_object` 对顶层加降级（合并分支 / 剥 enum·const / 丢弃并打日志）；照 `strict_provider_accepts`（`:1773` 起）加 `strict_provider_rejects_top_level_oneof` 用例 | M |
| #2138 | 市场注册名取自 `marketplace.json` 而 CLI 快照是 `api_marketplace.json` | `plugin_marketplace.rs:678-679` 的写死常量改为以快照真实 `name` 为准；`[marketplaces.<name>]` 与 `[plugins."<id>@<marketplace>"]` 同源。**注意 2ce62815 改的是注入层解锁显示，与这条是两条独立链路** | M |
| #2147 | 通用配置合并丢 Obsidian MCP → invalid transport | 落盘前加 config.toml 校验（遍历 `[mcp_servers.*]`，任一条目既无 `command` 也无 `url` 就中止并报条目名）；补 `unmanaged_mcp_server_survives_common_merge` 测试。前端「通用配置不可编辑」单独查 | M |
| #2123 | `codex_app` 缺 command/url + `model_catalog_json` 指向不存在文件 | ①复核 `494b12a8` 的「未展开变量」判定是否覆盖 `%userprofile%` 形态，不覆盖则补 Windows `%VAR%` 正则；②`apply_model_catalog_to_config`（`:2348`）写入前加 exist 校验；③对 `id == "codex_app"` 且既无 command 也无 url 的条目直接丢弃 | M |
| #2236 | 本地转发缺少 session 请求头 | `relay_headers.rs` 加显式透传名单（profile 字段 `passthrough_request_headers`），**必须显式不能全透传**（否则覆盖供应商凭据）；前端在 `relay-advanced-fields` 加入口；优先保证 responses 路径 | M |

### 5.2 likely（需先取证）与 insufficient-info

| issue | 主题 | 一句话方案 | 量 |
|---|---|---|---|
| #2185 | 并行 custom_tool_call 后 502，上游丢配对 | `send_upstream_request_for_responses`（`protocol_proxy.rs:1198-1250`）识别配对类错误（**用正则不写字面量**），把对不上账的 tool_output 降级为文本重试**一次**，避免与 `channel_protection` 叠加 | M |
| #2331 | 新版本无法回传图片（`x-openai-actor-authorization`） | 先修可观测性（失败分支写状态码与响应体前若干字节），再明确 `http_headers` 与自动注入认证头的优先级；两套认证打架时给校验警告 | M |
| #2220 | Gemini 新会话一直思考无输出 | 给 `ChatSseToResponsesConverter`（`protocol_proxy.rs:868` 起）补「上游流无任何 content delta 直接断开/只有 usage 块」→ 显式 error 事件；先要用户贴 `upstream_response` 日志 | M |
| #2336 | 1.4.0 报「该进程没有程序包标识符」，latest_launch 停在 starting | 不单独修，靠 #2351 收口——回退分支（`launcher.rs:1013-1026`）日志带上 activation 失败的 HRESULT + 是否提权两个字段 | S |
| #2204 | macOS 27 安装后打不开，`app_paths.rs:290-296` 只搜两个根 | 扩探测面（`/Applications` 一层子目录 + `mdfind` 按 bundle id 兜底）+ 可观测性（与 #2343 同一处）。**本条与系统版本无关，是搜索路径白名单太窄** | M |
| #2147 前端 | 通用配置面板不可编辑 | 实机确认 `App.tsx:9815-9880` 是 disabled 还是事件被吞 | S |
| #2349 | Windows 微信消息触发后 Codex 处理失败 | 微信失败提示带真实错误 + 校验 Codex CLI 路径（带版本号路径升级即失效）；端口 9329 vs 实测 9229 的不一致单独查 | M |
| #2199 相关 | sidebar 索引同步（#2092）是否覆盖「切页面返回」 | 复核后确认修复点落在 repair 侧（见 §4.30） | S |
| #2134 | Chat history lost（前端不显示，数据在） | **只能 CDP 实机取证**；改 `10-style.js` 的选择器按「有归属/无归属」两档收窄。**不要加回 82fb0924 删掉的 350ms 兜底 setInterval** | L |
| #2237 | 软重启后消息区变空白 | 在 v1.5.0 上复现确认；修复落在 `95-conversation-view.js` 的重建入口加「等宿主容器就绪」门，或 `98-scan-schedule.js` 的启动重试。**同样不要加回定时兜底** | L |
| #2151 | 更新后布局错位（按钮与文字叠在一起） | 先升 1.5.0 复测（页面形态对齐已覆盖一类）；仍复现则 CDP 抓 DOM 锚点（优先 `data-*`，避免 hash class） | M |
| #2219 | 图标布局错位 + 按模型设思考级别 | 布局同 #2151 流程；思考级别是新能力，与 #2279 同构——`model_suffix.rs:509-580` 加按模型 reasoning effort 覆盖字段，前端 `model-windows.ts:26-31` 加一列。**先探 catalog schema 是否接受该字段** | M |
| #2215 | 微信连接支持图片（带补丁） | 按 feature 流程：下载 `weixin-image-support.patch` 检查能否干净应用；确认用 app-server 官方 `localImage`；补「图片下载失败不阻塞文本通道」用例。**合入与否由用户决定** | M |
| #2226 | 主代理等子代理期间无响应 | 先抓 SSE 帧序列区分「上游不发」与「代理攒批」；若是代理，在 responses 流式转发路径放行不应聚合的事件。**没抓到帧序列之前不要动代码** | M |
| #2273 | 同一 key 在 CCS 正常、Codex++ 报 401 | 让用户自查 profile（relayMode / no_auth / custom_headers）；临时解法显式加 `Authorization: Bearer` 头；代码侧加「请求未带鉴权」的诊断可见化（与 #2236 同一改动） | S |
| #2270 | 换账号后会话不见 | 安抚 + 取证：会话按身份隔离是上游设计，rollout 不会被删；要用户贴切换前后 auth.json 字段对比与 relayMode，重点看切回官方是否保留认证状态 | S |
| #2261 | 纯 API 下界面部分汉化 | 先确认用户是否开了「强制中文」与 `codexPlus.forceChineseLocale.managed.v1` 是否存在；若值对但仍英文，需补齐「宿主 UI 另一份语言来源」的托管。**留意 1.4.0 刚修过的刷新循环不要回归** | M |
| #2158 | 畸形路径 `D:\Users\AppData\...` os error 2 | 先分清拼接问题 vs 杀软隔离（让用户直接粘贴报错路径）；`install_root_or_default` 改用 `current_exe` 父目录为首选来源 | S |
| #2119 | 同事的模型列表里没有 GPT6 | 操作指引：确认 relayMode——官方登录需升 1.4.0+ 并开白名单解锁；第三方供应商需手工加模型名 | S |

### 5.3 not-our-bug / 直关（详见 §3）

| issue | 主题 | 处置 | 量 |
|---|---|---|---|
| #2100 | Prompt Optimize 链接超时 | 转脚本仓库 | S |
| #2107 | 最小化失效 + CUA runtime 部署 | 转上游 SOP | S |
| #2126 | unexpected status 403/404 | 关单 + 指引 | S |
| #2133 | 混入 Key 优先消耗 API 额度 | 文案澄清 + 转 feature | S |
| #2144 | 会员额度恢复仍用 API | 关单 + 文案澄清 | S |
| #2184 | 被报木马 | 关单 + README 加 Defenr 加白 | S |
| #2205 | 切换账户/重启后今日会话丢失 | 先取证（原生 codex 对照），不动 index 层 | M |
| #2238 | 别的 AI desktop 增强 | 转 Discussion | S |
| #2252 | 搜索不到插件 | 产品层澄清 + 体验补救 | S |
| #2259 | 脚本市场单条删除 / 全局开启 | 澄清（单条删除已支持 `/user-scripts/delete` by key） | S |
| #2260 | 终端启动不了 / 控制权丢失 | 澄清 + 要信息 | S |
| #2268 | workbuddy 功能 | close as not-planned | S |
| #2269 | 企业应用控制策略阻止 | 只改错误文案（识别 4551） | S |
| #2325 | 中转站缓存不命中 | 转讨论 | S |
| #2344 | 权限修改设置 | 关单 + 指引 sandbox_mode | S |
| #2369 / #2287 / #2373 | Computer Use in onlyAPI | 合并到 #2188 统一追踪 | S |
| #2372 | 第三方 API 后插件安装失败 | 澄清内置/远程插件区别 | S |
| #2375 | 官方 dots 与第三方 API 并存 | 要需求澄清 | L |

---

## 6. 横向根因

### 6.1 注入层的选择器失配与扫描调度（覆盖 #2363 #2350 #2364 #2258 #2227 #2151 #2219 #2134 #2237）

**根因**：选择器全是「全等/字面量」，每次 Codex 改类名或改压缩标识符都要人肉发现。五条 issue 是同一个病的不同实例：
- `90-action-groups.js:386-405` 类名数组 + `conversationViewHasAllClasses`（`:420-424`）的全等匹配 → #2258 一个类名变了匹配归零；
- `dream-skin.css:1420` 的 `[class*="_MainContentTopFade_"]` 子串匹配 → #2350 / #2363 子串边界不清导致误伤；
- `50-navigation.js:235-247` 的结构正则（`2ce62815` 已改进）→ #2364，是**已经做对的那一例**；
- `20-menu.js:16-18` 三条硬编码版本线 → #2364 的另一半。

**架构级改法**：
1. **统一「选择器契约」治理**，参照已存在的 `SELECTOR_CONTRACT` 机制（`assets/inject/upstream/dream-skin/windows/renderer-inject.js` 第 3 行，带 `tier` / `scope` / `required` 三档）推广到主分片：每个跨版本选择器登记为 `{key, selector, tier, required}`，`required: true` 的在运行时探针失配时**主动上报诊断**而不是静默返回 null。今天的 `conversationViewAlignNow`（`90-action-groups.js:760`）与 `installCodexPlusSidebarNavigation`（`40-backend-settings.js:1391` 附近的 `if (!navigation) return`）都是静默失败——用户报「没生效」时我们手上没有证据。
2. **扫描调度与自喂防护复用**：`98-scan-schedule.js:59-82` 的 `shouldScheduleScan` 补 composition/contenteditable 排除（见 §4.11），并把 `1960` 那套 `data-codex-plus-ext` 防护模式复用为标准——**拓展插入的节点必须带该属性**，否则触发自喂扫描循环。

### 6.2 config.toml 切换的对称性问题（覆盖 #2203 #2216 #2147 #2123 #2207 #2081 #2374）

**根因**：**写入与清理的不对称是这个主题的原生病**。以「每个写入 `model_providers.<id>` 的函数都必须有一个对应的删除/保留函数」为规则，`relay_config.rs` 里 13 处 `model_providers` 相关写入点需逐一配对核对。三处已知不对称：
- #2216 的教训是 `clear_relay_config_to_home_with_auth`（`:1004`）删得不够彻底（对 custom 只删认证字段，留 `base_url`）——**已修**，但同一份代码里 `complete_relay_profile_config`（`:3708`）用 `retain_only_provider_table` 只保留当前 transport provider，而 `ensure_active_protocol_proxy_config_in_home`（`:291`）做修复时不清理旧 provider。
- #2173(c) 的 `relay_config.rs:721-722` PureApi 分支整体覆盖 auth.json，而 Aggregate/Official 先合并 live——第三条不对称。
- #2374 vs #2081 的**诉求方向相反**：前者要「别覆盖我手动配的 catalog」，后者要「切换后刷新 catalog」。正解是不要把两种意图都寄托在同一个 `model-catalogs/<id>.json` 上——用户声明走 `RelayProfile.model_metadata`（经 `relay_config.rs:2769` 的 `apply_model_metadata_overrides` 在生成后覆盖），自动生成走 catalog 文件本身。**只修一边必然把另一边变成回归**，这是本主题最容易踩的坑。

**架构级改法**：
1. 落盘前**加 config.toml 校验**（现在只有 auth.json 有 `validate_auth_json`，config.toml 侧无等价校验），把「invalid transport 整份配置加载失败」变成「本次切换被拒绝并告出具体条目」。这一条同时降低 #2147 / #2123 / #2126 / #2273 的分诊成本。
2. 加一条**请求路径诊断记录**：本次请求走了哪条链路、带了哪些鉴权头、`model_catalog_json` 指向哪里。多条报错的文本完全看不出请求走哪条链路，用户和排查者都要靠 URL 或响应体形态反推。

### 6.3 协议翻译层跨消息边界的状态机缺陷（覆盖 #2275 #2257 #2210 #2367 #2235）

**根因**：三个缺陷簇的共同特征是**单条 item 转换都正确，失败发生在 item 之间的相对顺序与 pending 状态的交接上**。
- 簇一（配对被非 tool 消息打断）：`enforce_tool_call_pairing`（`:3377`）建立在「tool 消息在 assistant 之后连续排列」假设上；
- 簇二（pending 被提前 return 吞掉）：`flush_tool_calls` 的 merge 分支（`:3658-3663`）直接 return；
- 簇三（schema 归一化只做半程）：`normalize_schema_object` 只处理 `type:null` 与 `$ref` 内联，顶层组合器完全没碰。

**架构级改法**：
1. **测试要覆盖「消息边界」而非只有「单条消息形态」**——107 条 `protocol_proxy` 测试里，reasoning × tool_calls、developer × tool 区、oneOf × schema 三组交叉全是空白。
2. **判定输入用结构特征而非字面量**——`2ce62815` 刚因写死压缩后标识符吃过亏；#2185 的上游错误识别同样必须用正则。反过来，定位时 grep 字面量命中率最高：本次 `oneOf` / `image_resize_notice` 双双零命中，正是两个缺陷的直接证据。
3. **opt-in 隔离与 per-profile 兼容**：#2345 的 key 归一化只改读写两侧，不动已发布的 `modelVlm` 字段名与值域；#2240 的提示与预设初值不得改变「未配置 = Codex 默认 272K」的既有语义。

### 6.4 错误可观测性的系统性缺失（覆盖 #2343 #2314 #2204 #2129 #2244 #2336 #2351 #2140 #2354）

**根因**：`DefaultLaunchHooks::resolve_app_dir`（`launcher.rs:792-802`）在任何探测失败时只抛一句 `anyhow!("Codex App directory not found")`，三个平台、四种失败原因（目录不存在 / 被误判为 Codex++ 目录 / 枚举不到 exe / 包名判据不匹配）**共用这一句**。同一错误文案「该进程没有程序包标识符」对应至少四条互不相同的产生路径。这也是 #2244 那条「80~110 秒内 codex-plus.log 一条都不写」的同源问题。

**架构级改法**：
1. 给 `launcher.rs:497-660` 的每个前置步骤加「进入/离开 + 耗时」诊断日志，让所有路径解析失败都带上探测轨迹与拒绝理由。
2. 「该进程没有程序包标识符」分流：在回退分支（`launcher.rs:1013-1026`）固定带三个字段——activation 失败的 HRESULT、本次是否提权、解析出的 AUMID 及其依据（manifest 还是硬编码）。这三个字段组合起来就能把 #2140 / #2336 / #2351 / #2354 四类反馈一次性分流。
3. **引入「静默失效」清单**：本轮「补丁照装但从不生效」或「值照写但不是用户想要的」出现四次（#2256 熔断被绕过、#2138 市场名对不上、#2302 单位被吞、#2119 白名单解锁无声失效），共性是**没有一处会主动报错**。2ce62815 已在插件解锁上引入「结构正则替代写死标识符」，同样适用于 #2138（以快照真实 name 为准而非写死常量）。

### 6.5 平台进程/路径判据只认名字不认身份（覆盖 #2222 #2204 #2140）

同一类设计缺陷重复三次：macOS 用可执行文件名判断「Codex 桌面应用是否在运行」（`watcher.rs:809-824` 匹配 `"ChatGPT"`，命中 ChatGPT Classic，造成 #2222 的假阻塞）；macOS 用固定白名单搜索应用（`app_paths.rs:853-866`）；Windows 用目录名像不像 MSIX 包反推 AUMID（`app_paths.rs:693-712` + `:952-974`，造成 #2140 的 0x80073CF1）。**三处都是「名字像就算是」，而名字在任何一处都不是身份的可靠证据。** 统一方向：能用系统权威来源就用——macOS 读 `.app` 内 `Info.plist` 的 `CFBundleIdentifier`（复用 `macos_app_plist_value`）、Windows 用 `GetPackagesByPackageFamily` 查注册状态（复用 `registered_windows_packages`）。这条改动稍大，但能一次消掉三个 issue 且不引入新依赖。

---

## 7. 建议的执行顺序

按「先修什么收益最大」排序。标注可并行组。

### 第 0 批（立刻，互不冲突，可全并行）

| 动作 | 理由 |
|---|---|
| 合入 PR #2365（macOS Dock reopen） | 一个 `Reopen` 分支消掉 #2157 + #2304 两个长期抱怨，本轮投入产出比最高 |
| 合入 PR #2368（NSIS 等待进程退出） | P0 #2362，改动隔离在安装器，风险最低 |
| 合入 PR #2370（URL 协议注册） | P1 #2354，纯注册表；建议同批带上启动自检 |
| 合入 PR #2371（提权回退阻断） | P1 #2351，非提权路径零行为变化 |
| 修 #2339（卸载键名统一） | P1，改 6 条 `WriteRegStr` + 1 条 `DeleteRegKey`，最便宜的确定性修复 |

### 第 1 批（注入层根治，收益最大，可并行）

1. **PR #2337 + 分片迁移**（#2330 / #2169 / #2267 / #2201）。**必须先做这一步**——它同时是 CPU 空转、卡顿、内存增长、60 秒重复解析四条链路的主因；且必须先把改动搬进 `20-menu.js` 再重跑 assemble，否则下一次组装会静默抹掉。
2. **PR #2303**（原生压缩保真）——合入前唯一阻断项是重跑并发测试确认时序修复生效（`cargo test -p codex-plus-core --test protocol_proxy` 并发连跑 5 次）。它是压缩线的收官动作（main 仍缺「原生 encrypted compaction 历史遇上不支持 v2 的上游」时的显式拒绝，会把 opaque checkpoint 当明文重放）。**建议把其中 App.tsx 的 dead `metadataImportPreview` 重复 useState 声明作为独立 commit 先合**——它会让所有 PR 的 TS/Vite 构建失败。

### 第 2 批（协议翻译层，P0 但改动小，可与第 1 批并行）

3. **#2275 + #2257**（一次改动关两个 P0，新增 `relocate_interleaved_non_tool_messages`）；
4. **#2210**（P0，改 `flush_tool_calls` 的 merge 分支，几行）；
5. **#2367 / #2235**（schema 顶层组合器，单测可完全覆盖）。

### 第 3 批（纯 API 能力边界 + 供应商对称性，可与第 2 批并行）

6. **#2173(c) 的 auth.json 对称性**——改动最小、风险最低，且是「本来能用现在不能用」这类最伤信任的故障。**建议提到第 1 批末尾**，因为它静默摧毁唯一可用的 workaround。
7. **#2294**（CUA runtime 结构校验），按 #2294 的 fix 一次到位（结构 + 锚点正则），别再堆第三个哈希常量。
8. **#2203**（外部 catalog 改为 warning 降级而非拒绝切换）。
9. **#2340 + #2199**（catalog repair 删除墓碑，改动集中在 provider_sync）。

### 第 4 批（可观测性与体验）

10. #2343 / #2314 / #2204 的探测轨迹（同一处改动，先合可观测性再放宽判据）；
11. #2244 的 sqlite 清理短路 + 启动阶段埋点；#2359 的主题 token 真值探测；
12. #2364 / #2227 的插件门控解耦；#2258 / #2085 的对话居中宽度。

### 第 5 批（先测量后优化 / 先取证）

13. #2249 / #2134 / #2237 —— **先测量后优化**（不测量不要动帧循环），需要 CDP 实机；
14. #2226 / #2220 / #2325 —— 先抓帧序列 / 先要日志。

### 纪律（跨批次）

- 注入层任何改动必须走完 **assemble + npm test + `--check` 三步**；
- **`10-style.js` 的改动要先 `codexDeleteStyleVersion` +1**，否则 `installStyle` 因版本号相同静默不重建，改了等于没改；
- 修完重注入循环后，把「旧实例资源不回收」当成独立课题做（PR #2337 作者实测 window 上留着 65 个 keydown），现成隔离点是 `40-backend-settings.js:89-90` 的 `codexPlusBackendGeneration` 世代号——**不要引入第二套**；
- 根因 B（原生 app-server 认证）的任何「绕过」方案（把 `auth_method` 从 apikey 改写成 chatgpt）**都不许做**。

---

## 8. 信息不足需回帖追问

| issue | 要问什么 | 要到的证据能定什么 |
|---|---|---|
| #2107 | Windows 上是任务栏点击还是托盘图标点击？最小化后多久失效？是否装了 Codex++ 托盘？ | 区分「托盘/任务栏不 restore」与「上游 cua_node 部署」两件事 |
| #2114 | 脱敏 config.toml 的 `model_providers` 段与顶层 `model/model_provider`；profile 的 protocol；复现时含协议代理转发条目的 `codex-plus.log` | 二分「协议代理是否收到请求」 |
| #2139 | 模式控件的 DOM 片段（DevTools 复制 outerHTML）；官方直启对照截图；`codexAppVersion` | 是否是 Codex++ 主动改的外观 |
| #2205 | 干净环境下用原生 codex（不经 Codex++）复现同一操作序列，记录 `.codex\sqlite\codex-dev.db` 的 mtime 与会话列表 | 判断是否与 Codex++ 无关 |
| #2206 | `gh issue view 2206 --json body,comments` 取正文；核对 `gh pr list` 是否仍有 open PR | 按其自身主题（Taskboard）单独立项 |
| #2223 | Computer Use 每个动作（cua 枚举窗口、截图、点击、键盘输入）单独的成功/失败与错误串；版本号 | 填 `docs/` 的四列依赖表 |
| #2226 | 用 CDP 抓该会话的 SSE 事件序列（或开转发诊断日志），对比 GPT 官方直连与 deepseek 经代理两条路径 | 区分「上游不发增量」与「代理攒批」 |
| #2270 | 切换前后的 auth.json 内容对比（脱敏，只留字段名与是否为空）与 relayMode | 判断是否某条路径把 auth.json 写成影响身份识别的形状 |
| #2261 | 管理工具「强制中文」是否开启；浏览器 localStorage 里 `codexPlus.forceChineseLocale.managed.v1` 是否存在且为 zh-CN | 区分「注入没生效」与「宿主 UI 语言来源不止一处」 |
| #2297 | 脚本名与来源；「不生效」的具体表现（列表里没有 vs 列表里有但功能没出现）；`codex-plus.log` 的 user-scripts 诊断行；文件名是否含中文 | 区分安装失败 / 加载失败 / 路径编码 |
| #2300 | 脱敏 config.toml（重点 `model_reasoning_effort`、`model`、`model_provider`）；一次原始响应里 `content` 字段开头几行 | 判断思维链是模型自出还是上游兼容层混进 content |
| #2315 | 管理工具里模型列表的原文（两行完整拼写）与 profile 的 `modelList / modelWindows / modelMetadata` 三字段 | 确认是两个不同 slug 还是别名污染 |
| #2349 | 抓一份含安装动作的 `/diagnostics/log`，确认 `plugin_install_request_debug` 是否出现 | 区分是策略通道没覆盖还是改写错 |
| #2375 | 「dots」的确切含义（Codex 客户端里的入口位置、是模型还是服务、降智的具体表现） | 评估是否与 #2144 的「按额度切身份」合并 |

---

## 9. 未合并 PR 的处置建议

截至 2026-10-02 的 open PR（`gh pr list`，均 `MERGEABLE`，均无 reviewDecision）：

| PR | 标题 | 处置 | 理由 |
|---|---|---|---|
| **#2371** | fix(launcher): 提权运行时 AUMID 激活失败不再静默回退 | **建议合并** | 方案经代码核对正确（纯函数 `packaged_activation_fallback_block_reason` 便于测试；非提权路径零行为变化）；新增 `current_process_is_elevated()` 而不复用 `relay_config.rs:2185` 的私有函数，语义判断正确。合入后 #2351 / #2336 一并关闭。 |
| **#2370** | fix(installer): 安装器注册 `codexplusplus://` 与 `dreamskin://` | **建议合并** | 与 `install/windows.rs:172-200` 的 `register_url_protocol_key` 逐字对齐是关键；建议同批追加「manager 启动时幂等自检」覆盖存量用户。合入后 #2354 关闭。 |
| **#2368** | fix(installer): taskkill 后轮询等待进程退出 | **建议合并** | 纯新增 +50 行，三个已知坑（PATH 里 Unix `find.exe`、Win11 tasklist 返回码、语言相关输出）都规避；等待宏无 `Abort` 分支、异常收敛到「立即放行」，失败方向正确。合入后 #2362 关闭。 |
| **#2366** | feat(manager): 支持模型列表拖动排序 | **要求修改（或与 #2333 合并）** | 与 #2333（按名称排序）功能重叠、同区域改动，同时合入会冲突。请两作者对齐成一条「模型列表排序」PR：拖动排序 + 名称排序二选一或合并实现，避免两次改动同一块 UI。 |
| **#2365** | fix(manager): reopen hidden macOS window from Dock | **建议合并** | 正是 #2157 / #2304 的正解（`RunEvent::Reopen` → `show_main_window`），改动小。合入前确认没有动 `lib.rs:519-575` 的端口守卫逻辑，且 `--transient` 模式仍直接退出。合入后 #2157 / #2304 关闭。 |
| **#2337** | fix(bridge): 窗口隐藏时不判定桥接失效 | **要求修改后合并（本轮最高优先级）** | 思路正确，三点都值得合（`visibilityState` 早返回、`healthy_since` 持续健康 60 秒才清零、asset 查找结果与失败缓存挂 window）。**阻断项**：reviewer 已指出 `assemble --check` 报产物与分片不一致（269 pass / 1 fail）——必须把改动搬进 `assets/inject/renderer-inject/20-menu.js:99` 与 `:128` 并重跑 `node scripts/assemble-renderer-inject.mjs`，否则下一次组装会静默抹掉修复。另两点注意：hidden 早返回必须放在 bridge/health 存在性检查之后；`launcher.rs:2870` 的 `browser_identity_changed` reset 必须保留。合入后 #2330 / #2169 / #2267 / #2201 一并受益。 |
| **#2333** | feat(manager): 模型列表支持按名称排序 | **要求修改（见 #2366）** | 与 #2366 重叠，先合并再评审。 |
| **#2313** | fix: make launch and restart lifecycle deterministic | **要求修改** | 方向与本轮 §6.4 的可观测性诉求一致，但「deterministic lifecycle」范围过大，需拆分为可独立验证的小改动（重启时序 / 状态写回 / 清理），并要求作者给出 `latest-status.json` 的阶段字段设计——这与 #2244 的诉求 3 直接重合，避免两处各写一套。 |
| **#2309** | fix: 新版 Codex App 兼容适配（面板边界、顶部入口、注入卡顿缓解） | **要求修改** | 与 #2337 在「注入卡顿」上重叠，需明确分工：卡顿主因归 #2337，本 PR 只做面板边界与顶部入口的形态适配。要求作者**附 CDP 实测证据**（改前改后的 DOM 锚点与布局值），因为注入层布局类问题静态阅读容易误判。 |
| **#2303** | fix: preserve native compaction with safe summary fallback | **要求修改后合并** | PR（非 issue）。它在 main 上确实还有未覆盖的真实缺口（`native_checkpoint_cannot_be_downgraded_to_chat_text` / `native_compaction_history_returns_responses_error_without_summary_retry` 两个测试名在 main 搜不到，会把 opaque checkpoint 当明文重放）。**唯一合入阻断项**：拿当前 head 重跑并发验证 `cargo test -p codex-plus-core --test protocol_proxy` 连续 5 次（09-30 的失败是在 rebase 前版本上复现的，作者声称已改，需实测）。隔离要求：新逻辑只作用于「OpenAI 会话身份 + 本地代理 + v2 远程压缩」，纯官登与纯 API 不受影响；#2301 的 `UpstreamProxyResponse.compaction: bool` 单一标记路线保留，不要用 `native_compaction_passthrough` 字段整份替换。**App.tsx 的 dead `metadataImportPreview` 重复声明应与功能改动分开成独立 commit 先合。** |

---

### 9.1 执行结果（2026-10-02 晚，实测更新）

上表是审计当时的判断。实际执行时逐条复核，**有两处结论被推翻**，另有一处判断被更正。

| PR | 上表处置 | 实际执行 | 变动原因 |
|---|---|---|---|
| #2366 | 要求修改（与 #2333 合并） | ✅ **已合入** `ea82bc74` | 实测不碰 `crates/`、无冲突、331 passed；与 #2333 的重叠可留待后者 rebase 时处理，不必阻塞 |
| #2337 | 要求修改后合并 | ✅ **关闭**，由 `5bb4f636` 覆盖 | 已按其思路落地且改的是分片而非产物 |
| #2371 | 建议合并 | ✅ **关闭**，由 `93a66453` 覆盖 | 其两处缺陷（cfg 属性被注释挤偏、测试断言 CI 非提权）在落地时修正 |
| #2333 | 与 #2366 合并 | ⚠️ **要求修改** | **实测发现真实回归**，见下 |
| #2309 | 要求修改 | ⚠️ **要求修改** | 两个阻断项经核实成立，见下 |
| #2313 | 要求修改（范围过大） | ⚠️ **可合，先 rebase** | 更正：冲突是陈旧基线而非设计问题 |
| #2303 | 要求修改后合并 | ⚠️ **可合，先 rebase** | **推翻「并发验证是阻断项」**，见下 |

#### 推翻 1：#2333 的回归（上表未发现）

上表只提「与 #2366 冲突」，未发现它引入了**静默换模型**。

实测（独立 worktree，PR 分支 `43c53e50`）：

```
test model_catalog_uses_active_relay_profile_model_list_and_actual_provider ... FAILED
  left: String("deepseek-coder")   right: "qwen3-coder"
```

链条：`relay_config.rs:4079-4082` 的 `merge_model_into_model_list` 无条件把配置模型提到首位 →
`model_catalog.rs:227` 的 `any("item == &model")` 必然命中 → PR 改成「已在列表中就不提升」后首位变化 →
落空 → 走 `models.first()` → 默认模型被换成 `modelList` 第一条。

作者只跑 `--lib` 与 `--test relay_config`，恰好漏掉会红的 `tests/model_catalog.rs`，因此标了 MERGEABLE。

**教训**：审查贡献者 PR 时，「PR 自带测试是否覆盖了整个 workspace」必须单独验；只信 PR 描述里的测试命令会漏掉跨 target 的回归。

#### 推翻 2：#2303 的「并发验证阻断项」（上表判断有误）

上表称「唯一合入阻断项 = 重跑并发验证连续 5 次」。实测：

- 默认并发下 5 次**全过**（139 passed × 5），未能复现失败；
- `--test-threads=32` 加压后本 PR 分支 5 次挂 3 次，失败用例是
  `upstream_request_returns_when_provider_accepts_but_never_sends_headers`
  （`assert!(started.elapsed() < Duration::from_secs(1))`，墙钟断言）；
- **决定性对照：main 上同样加压同样会挂同一个用例**（`protocol_proxy.rs:3269`，函数体逐字节相同）。

结论：那是 main 上**既有的**时间敏感 flake，不是本 PR 引入的。把它当阻断项会误判。
真正的阻断项是分支落后 main 34 个提交、且 `App.tsx` 那条改动在 main 上早已由 `443c8348` 完成。

另更正上表的一条约束：「保留 `compaction: bool` 单一标记路线」是反的——加入原生透传后，
`compaction` 语义已分裂为「需要代理重组」，单靠它无法区分透传与合成，多字段反而更清晰。

#### 更正：#2313 的冲突性质

上表称「范围过大需拆分」。复核后：方案质量高（真锁替换轮询消除 TOCTOU、退出改为事后核实、
停止顺序短路），冲突只有 1 处且是**陈旧基线**——main 的 `04ab6290`（#2294）把
`recovery_material` 从 3 参改成 2 参，取 main 侧即可、不丢任何语义。
在临时 worktree 按此解冲突后整棵树编译并全绿（1395 / 75 / 331）。

需作者改的只有一处幽灵参数（`verify_restored_state` 的 `contract` 已零使用）。
**不应因这次冲突要求改设计。**

#### #2309 的两个阻断项（均经实测核实）

1. **`codexDeleteStyleVersion` 撞车**：main 与本 PR 分支**都写成 `"25"`**，内容不同、值相同 →
   git 判无冲突静默取 25 → 本 PR 的 78 行标题栏 CSS 在已装过 main 版 25 的页面上永不生效。
   必须 rebase 成 26。
2. **`Err(_)` 抵消 `5bb4f636`**：`Ok(true) | Err(_) => backoff.reset()` 立即清零退避；
   且 `evaluate_bridge_health_script` 把 renderer 抛异常判为 `Err` → **越忙越清零**，方向相反。

另：1039 行混了 5 个关注点，其中看门狗主动探测那组（`bridge.rs` / `launcher.rs` / `cdp_bridge.rs`）
应整组拆出单独讨论；标题栏入口本身是真能力。

---

### 9.2 #2378（当晚新提，补审）

@Yuimi-chaya 的第三个 PR，`fix(browser): Windows API 浏览器语义契约兼容与可验证恢复`，
+1370/-16、11 文件。**处置：要求修改。**

它与已合入的 `04ab6290`（issue #2294）改同一块 `native_browser.rs`，是本次审查的核心问题。

**结论：技术增量真实存在，但当前提交是「替换式」而非「叠加式」，会收窄 main 刚建立的降级通道。**

三条经独立核实的阻断项：

1. **`FileCheck::Structural` 降级通道被整体旁路**。main 的 `for_manifest` 收完整 manifest 字节并在
   内部做结构校验；本 PR 改成只收哈希字符串（`native_browser.rs:95-102`），调用点传
   `&sha(&manifest)`，于是**未知版本必然 `Err`** → 每次转向 `structural::detect`，
   main 的 `Structural` 分支在未知版本上变成不可达代码。
2. **`ENTRY_SHA` 重新引入哈希白名单**。`native_browser_contract.rs:5`、`:201-205` 对入口
   `cua-repl.mjs` 做**无条件的硬哈希准入**。main 没有该常量（只用存在性检查）。
   后果：下一个 CUA 版本动一次入口文件就全链路 fail-closed —— 正是 issue #2294 的病根，
   从原件层搬到了 AST 层（`PROTOCOL_SHAPES` 双元素白名单同理）。
   作者「不是重复追加 0.0.27 哈希」的说法字面属实，但语义层面不成立。
3. **版本下限检查被静默弱化**：`detect` 不解析 manifest 版本，main 的
   `MIN_RUNTIME_VERSION` 检查丢失。

**确属增量的部分（应保留）**：组件级事务守卫（PE x64 头 + `package.json` exports 校验 +
现算哈希写入 `contract.files`）、schema-2 可重算恢复（重算最小变换并要求逐字节相等）、
绑定级 AST 校验（实测能做到 main 做不到的语义漂移拒绝）。

**依赖合规**：`@babel/parser` / `@babel/traverse` **零膨胀**（main lock 里本就有，经
`@vitejs/plugin-react` → `@babel/core` 传递引入，版本恰好相同），188 → 188 packages。
精确版本锁定是正确的（AST 哈希依赖 Babel 输出形状）。但要补：构建脚本 `--check` 未接 CI、
`esbuild` 未显式声明（靠 vite 传递依赖，脆弱）。

**实测**：`codex-plus-core` 1383 passed / 0 failed / 6 ignored（作者称 1708，属 workspace 口径差异）；
`npm test` 315 pass / 1 skip；tsc、assemble --check 通过。
**但**最有价值的负例（真实版本识别、歧义、遮蔽）全在被 CI skip 的 fixture 测试里，
`structural::detect` 新代码路径**在 CI 上零覆盖**。

**安全**：子进程隔离（`env_clear` + `--no-addons` + 内存上限 + 20s 截止）与写盘原子性均落实；
**worker 身份认证 / 伪造登录已独立确认未触碰**（grep 命中全落在 Babel 许可证文本，
`require-identification.mjs` 与 main 逐字节相同）。建议文档补上「`node.exe` 只验 PE 头是信任根」。

**合并顺序**：#2378 / #2309 / #2313 **都不含 `04ab6290`**，谁先合都会互相覆盖，都必须先 rebase。
建议先合 #2378 的 rebase 版（去掉硬编码准入面后），另外两个再来。
注意 `native-browser-status.ts`（main 新增的 `browserRecognizedSuffix`）不在 #2378 的改动列表里，
rebase 冲突不会提示，属**静默丢失风险**，需人工确认。

---

## 附：结论计数

- issue 结论总数：**116 条**（`confirmed-in-code` 31 / `likely` 37 / `already-fixed` 19 / `not-our-bug` 17 / `insufficient-info` 12）
- 另处理 PR **1 条**（#2303，在 §9）
- 待处置 open PR：**10 条**（§9）
- P0：9 条；P1：约 30 条；其余为 P2/P3 或需求类

---

## 附录 A：核查过程订正的事实（2026-10-02 执行期）

执行本报告时，各条线的复核推翻或收紧了下面几处原判。**以本节为准**，正文按当时证据保留。

### A.1 报告的一处事实错误（#2227）

正文 §4.14 称 #2227 的日志里 `plugin_install_request_debug` **一条都没有**，据此推断
「安装请求根本没走到打过补丁的通道」。**该推断错误**：日志第 93 行有这条事件——
`requestMethod=install-plugin`，`originalMarketplacePath` 与 `requestMarketplacePath`
完全相同（`C:\Users\kobet\.codex\.tmp\plugins\.agents\plugins\api_marketplace.json`），
`originalRemoteMarketplaceName` 与 `requestRemoteMarketplaceName` 均为 `null`。

即：请求**确实**走了补丁通道，改写是 no-op——该路径不含 `remote:` 前缀，改写分支不可达。
另：同一日志里 `plugin_build_flavor_filter_patch_installed` 计数为 0 与过滤器逻辑无关，
该日志采集于 2026-09-17，早于 `241d6ca4`（2026-10-02）。

### A.2 #2364 / #2252 / #2227 的真实定性：发版问题，不是代码缺口

三条都是**已修但未发版**：#2364 命中的是 v1.5.0 / v1.4.0 产物里仍是
`isKnownFilterSource = source.includes("!u(e.marketplaceName)||e.marketplaceName===r")`
且 `unlockVersion = 15` 的旧实现，而修复（结构正则 + `unlockVersion = 16`）
**不在任何 tag 里**（`git merge-base --is-ancestor 241d6ca4 v1.5.0` 返回 NO）。

处置：不要改代码（重复劳动），应**发版**后请用户升级复测。
#2252 报的 `chatgpt authentication required for remote plugin catalog; api key auth is not supported`
在本仓零命中，出自 app-server 侧对远端插件目录的门控，属 §3.2 的 not-our-bug。

### A.3 App.tsx 的 dead `metadataImportPreview` 已不存在

§7 第 1 批末尾与 §9 的 #2303 行都提到这个 dead 声明会成为所有 PR 的构建阻断项。
实际它**已由 `443c8348`（2026-09-30）移除**，且该 commit 是 HEAD 的祖先。
那两处描述指的是 PR #2303 分支的基线，不是 main。当前 `App.tsx` 里
`metadataImportPreview` 是派生值而非 hook，无需处理。

### A.4 #2240 的定性修正：能力已存在，缺的是可见性

原判「用户配不了」不准确。per-model 上下文窗口 UI **已存在且已接通**：
`App.tsx` 模型列表区已暴露「上下文窗口」与「自动压缩」两列（占位符 `1M` / `90%`），
经 `serializeModelWindowRows` → `modelWindows` / `modelAutoCompact` →
`apply_model_catalog_to_config` → catalog 的 `context_window` / `max_context_window` 生效。

用户原话是「会有自定义上下文窗口的设置吗」——他要的是**能力存在 + 可知**。
处置改为：回帖说明「模型列表每一行都有上下文窗口列，填 `1M`/`200K` 即生效」，
并把「窗口列做成带醒目提示或预设带初值」作为独立 feature 提案——那会改变
「未配置 = Codex 默认 272K」的既有语义，属产品决策。

### A.5 #2345 的完整根因（补充 §4.28）

不止「按模型配图片处理没入口」，实际故障链是：前端把带 `[1M]` 后缀的**行名原样**
写成 map key，而读写两侧查表用**剥掉后缀**的 slug，key 对不上就静默回落
send-as-is ——用户表现为「保存后图片处理全部变回原样发送」。前端已修（`ecb71213`）；
后端一半（`vision.rs` 的 `image_handling_mode` 全字匹配、不剥后缀）原判遗漏，已补做。

### A.6 #2294 的实现补充

原判只点了「白名单过窄」。实测还有第二个落点：锚点从 `new eh(...je...,sv)`
漂到 `new uh(...We...,wv)`，**即使放开 manifest 白名单**，`transform_binding` 的
`text.matches(ANCHOR).count() == 1` 仍会失败。两处必须同时改，否则修不干净。

### A.7 #2258 / #2085 的归属修正

报告 §4.15 把它们放在「注入层」下是对的，但根因位置需要更精确：
`conversationViewContentClasses` / `conversationViewHasAllClasses`（`90-action-groups.js`）
与 `conversationViewApplyNativeWidth`（同文件）——**不在** `10-style.js`。
`10-style.js` 里只有 `normalizeConversationViewWidth` 与 `conversationViewWidth()`，
它们是「设置为上限」的输入方，不是缺陷方。

### A.8 #2116 的修复方案是错的，已否决（附权威证据）

正文 §4.29 判定「app-server 只接受 `readOnly` / `workspaceWrite` / `dangerFullAccess`，
需把内部 kebab-case 档位转成 camelCase」。**该结论错误，不应实施。**

证据来自 app-server 自身生成的协议 schema（本机 ChatGPT.app 内置
`codex-cli 0.159.2`，`codex app-server generate-json-schema --out <dir> --experimental`）：

```
v2/ThreadStartParams.json → properties.sandbox → $ref SandboxMode

definitions.SandboxMode = {
  "enum": ["read-only", "workspace-write", "danger-full-access"],
  "type": "string"
}
```

即 **wire 格式就是 kebab-case**，正是 `connect/app_server.rs:359` 现在发的值。
旁证三条：
- `readOnly` / `workspaceWrite` / `dangerFullAccess` 三种 camelCase 形态在本仓
  （排除 node_modules）零命中，唯一命中是 React 的 JSX `readOnly` 属性，与 sandbox 无关；
- 报告让参照的 `ad76ae51`（"fix: normalize Windows sandbox before launch"）**无关**——
  它改的是 `windows.sandbox = "elevated"/"unelevated"` 的 Windows 提权模式，
  与大小写转换毫无关系，属引用错误；
- `docs/` 下没有任何 app-server 协议材料支撑 camelCase 说法。

**#2116 的真实处置**：用户报障版本是 1.2.56，而整条微信链路已在
`789369e3`（2026-09-22，"fix(weixin): Windows 微信连接改用桌面版维护的标准 CLI，
零配置跑通全链路"，issue #2028/#1879）重写，该提交已进 v1.4.0 / v1.5.0 且是 main 祖先。
处置改为：**不做代码改动**，回帖请用户升级到 v1.5.0 复测；
若仍复现，需索取 app-server stderr 尾部（`collect_stderr_tail` 会把它附进错误信息）
再定位——那是握手/协议层问题，不是 sandbox 取值问题。

> 方法论教训：本轮多处结论依赖「读源码推断 wire 格式」。凡涉及与外部进程的协议契约，
> 应优先用对方自带的 schema 生成能力取一手证据（本例一行命令即可证伪），
> 而不是从调用方代码反推。
