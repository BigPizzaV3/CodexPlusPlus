# Open PR 审查 + Open Issue 分诊（2026-09-11）

审查基线：`main` @ `be6a4585`（v1.3.0）
数据来源：`gh pr list --state open`（5 个）、`gh issue list --state open`（200 个）、`gh pr list --state merged`（187 个）

---

## 一、Open PR 结论速览

| PR | 作者 | 状态 | 结论 |
|---|---|---|---|
| [#2136](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2136) 恒写 `wire_api = "responses"` | zzr767299 | MERGEABLE/UNSTABLE | **建议直接合** |
| [#2137](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2137) 自定义 Responses provider 始终生成 catalog | zzr767299 | MERGEABLE/UNSTABLE | **可以合，但须先 rebase 到含 #2136 的 main** |
| [#1770](https://github.com/BigPizzaV3/CodexPlusPlus/pull/1770) 纯标准 OpenAI 协议开关 | Story19240 | MERGEABLE（base 落后 37 commit） | **先 rebase，需补 UI 入口缺失** |
| [#2084](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2084) 历史会话流式修复 | liulinboyi | CONFLICTING/DIRTY | **不能按现状合，需按每会话架构重建** |
| [#2128](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2128) 会话任务树 userscript | Songjun113 | DRAFT | 草稿，未审 |

### #2136 — 建议直接合 ✅

**修的什么**：Codex 26.901 起不再接受 `wire_api = "chat"`，一旦生成到 `config.toml` 里整份配置作废、静默回退默认模型，用户侧表现为模型选择器显示"自定义"。

**我的验证**（`/tmp/rev-2136`，与 main 同 target 目录）：

- `cargo test -p codex-plus-core --test relay_config --test protocol_proxy --test launcher` → **148 passed / 0 failed**
- `cargo test -p codex-plus-core --lib` → 343 passed，唯一失败是 `vision::tests::test_vlm_once_send_error_on_connection_refused`（`timeout` vs `send_error`），这是**依赖真实网络连接、main 上同样会挂**的既有问题，与 PR 无关

**正确性论证**：恒写 `responses` 安全的前提是 chat 上游一定有本地代理兜底，这一点成立 —— `BackendSettings::active_relay_transport_uses_protocol_proxy()`（[settings.rs:809](crates/codex-plus-core/src/settings.rs:809)）在 `protocol == ChatCompletions` 时就返回 true，`protocol_proxy` 负责 responses→chat 转换。所以对 Codex 暴露 `responses` 不丢信息。

**配套改动是必需的，不是顺手加的**：因为 `wire_api` 恒为 `responses`，`custom_responses_provider(&config_text)`（读 config 文本判断）这个信号就失真了 —— chat 上游也会被读成 responses。PR 把它改成由 `profile.protocol` + `active_provider_id` 判定，逻辑上别无选择。旧函数已整体删除，无残留调用点。

**遗留一点风险（不阻塞）**：`custom_responses_provider` 的语义被改动后，其它路径若还有"从 config 文本反推上游协议"的做法会同样失真。`protocol_proxy` 内部走的是 `relay.protocol`（[protocol_proxy.rs:1160](crates/codex-plus-core/src/protocol_proxy.rs:1160)），没有这个问题；建议合并后再 grep 一轮同类模式。

**关联 issue**：[#2150](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2150)（chat 协议好几个版本无法使用，grok/claude 接不进来）、[#1476](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1476)、[#1795](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1795)。#2150 的现象高度吻合，但里面提到 grok/claude，可能还有上游侧独立问题，合并时建议引用而不断言已修复。

### #2137 — 可以合，但别单独合 ⚠️

**修的什么**：`apply_model_catalog_to_config`（[relay_config.rs:1962](crates/codex-plus-core/src/relay_config.rs:1962)）在模型列表没有显式窗口/元数据覆写时跳过 catalog 生成，导致自定义 Responses provider 的模型选择器只显示内置模型名。

**改动本身很克制**：只加了一个例外条件 `&& !(custom_responses && profile.has_model_routes())`，保留了"纯平铺 model_list 且无后缀不落盘"的既有契约（`apply_relay_profile_does_not_write_model_catalog_json_for_selected_models` 等测试仍过）。`custom_responses` 和 `has_model_routes` 都在作用域内，helper 已存在。

**测试**：`relay_config` + `protocol_proxy` + `launcher` 148 passed / 0 failed，同样只有那个网络 flaky 的 vision 测试挂；**但是 PR 自身没有新增测试** —— catalog 生成路径的回归守护应当补一条。

**必须先 rebase**：#2137 的 head 直接坐在 `be6a4585` 上，而 #2136 改了同一个文件同一函数，且把 `custom_responses` 的**定义**换掉了。两个都基于当前 main 各自 MERGEABLE，但先后合并必然冲突。建议顺序：先合 #2136 → 让 #2137 rebase → 冲突点是把 `custom_responses_provider(&config_text)` 的调用替换为新的 profile 判定，机械替换即可。

### #1770 — 有真问题，但得先 rebase 🟡

功能本身质量不错：纯标准协议开关是纯 opt-in（`#[serde(default, skip_serializing_if = "is_false")]`，关闭时不写 JSON，导出 round-trip 字节一致），`ChatReasoningStyle::Default` 只停发厂商私有方言、保留标准 `reasoning_effort`，新增的 `responses_request_standard_protocol_strips_vendor_reasoning_dialects` 测试覆盖了开/关两条路径。

**验证结果**：

- Rust：`relay_config`/`protocol_proxy` 通过；`tests/launcher.rs` 的 `a_busy_floating_helper_port_fails_immediately_without_waiting` **稳定失败**
- 前端：`npm test` **159 passed / 0 failed**

那个 launcher 失败**不是 PR 的锅**：main 上这个测试已改名为 `a_busy_floating_helper_port_respects_the_platform_retry_budget`，断言 macOS 重试 31 次；PR 的 merge-base 停在 `48d43158`（2026-09-04），比那次重写早，所以带的是旧版本测试。**rebase 到最新 main 后这个失败会自然消失**（该文件在 PR 里只多了一行 `standard_openai_protocol: false` 字面量）。

**真正需要作者补的是**：`settings.rs` 加了 `standard_openai_protocol` 字段和 2 个 serde 测试，但 `crates/codex-plus-core/src/ccs_import.rs` / `provider_import.rs` 里只各补了一行字面量 —— **导入路径不会让用户打开这个开关**，只能在管理器 UI 里手动勾。另外 PR 的 head commit 是 `fix: declare noAuth on RelayProfile`，把 `noAuth` 从 `RelayProfile` 类型里补齐了（`noAuth` 在 main 里早就在用但类型没声明），这属于必要的类型修正，不是夹带。

### #2084 — 现状不能合，需要重建架构 ❌

你（OWNER）在 PR 里已经连续 4 次催 rebase 并给了详细的架构性分歧说明，我复核后确认结论成立：该 PR 引入的 bulk 架构（`create_bulk_backup` + `apply_bulk_session_rewrite_plans` + `create_backup_with_session_meta_lines`）与 main 重构后的每会话循环（`create_backup` + `apply_session_changes`）互斥，不是 merge 能解的。四个文件冲突：`provider_sync.rs`（架构性）、`lib.rs`（re-export）、`App.tsx`（import）、`tests/launcher.rs`（测试语义，取 main 版本）。

**建议**：把 bulk 的**意图**（`BackingUp`/`UpdatingIndexes` 进度上报、扫描时跳过锁定文件、保护 sidebar 状态）移植进 main 的每会话循环，而不是合并两条函数体。作者上一条回复（09-05）之后没有再动过这个分支，已在 5 天僵持。要么你直接接管这个 branch 重建，要么明确告知作者按"每会话循环 + 补进度上报"重写。

---

## 二、Open Issue 分诊（200 条）

### 首先一个管理问题：label 体系定义了但完全没用

仓库有完整的 label 体系（`type: bug/feature/config/question`、`area: protocol/provider-config/context-tools/model-catalog/ui/build`），但 **200 个 open issue 里 0 个带 label**。这是所有后续分诊成本高的根源 —— 建议先把这批打上标签，之后按 `area:` 批量处理。

### 分诊口径

按"下一条该做什么"分四类，而不是按 issue 标题。

### A. 可关闭 / 可合并（约 45 条）

**已修复可关闭**（近两周合并的 PR 覆盖）：

| issue | 已修复 PR |
|---|---|
| #2090 切换 Provider 后旧会话无法继续使用当前 Provider | #2122 / #2110 |
| #2127 切回官方后旧会话报 `Model provider 'OpenAI' not found` | #2110 |
| #2073 会话仍然无法删除 | #2092 |
| #2079 删除对话不同步 / #2105 会话删了仍显示 | #2092 |
| #2003 / #1522 启动相关 | #2097 / #2098 |
| #2104 图片生成代理链路 | #2104（自 #2143）|
| #1407 `Unsupported parameter: 'reasoning_effort'` | #1770（待合）|
| #2003 白窗 | 需人工确认 |

**重复项**：`invalid transport in mcp_servers.codex_app` 一条主线把 #1997 / #2123 / #2147 串起来（#2123 明确标注"复现 #1997"），应合并到一条主 issue。`.codex` 目录删除类：#2146 / #2134 / #1948 / #1980 高度相关。插件市场类：#1338 / #1323 / #2103 / #2049 / #2072 / #2138。

**长期无响应需补信息**：大量 2026-06 ~ 07 的 issue（#1269 起的百余条）标题只有 `[Bug]:` 无正文、或已跨多个版本未回应，建议统一追问后关闭。

### B. 高严重度，建议优先处理（10 条）

| issue | 问题 | 备注 |
|---|---|---|
| [#2146](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2146) | 整个 `%USERPROFILE%\.codex` 被永久删除，454MB 会话历史只剩 1.1% | **数据不可逆，最高优先** |
| [#2123](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2123) | `invalid transport in mcp_servers.codex_app` + `model_catalog_json` 指向不存在文件致整份配置加载失败 | 两个独立故障点，附了最小复现 |
| [#2147](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2147) | 通用配置合并丢 Obsidian MCP（`url` + `http_headers`），且"编辑通用配置"打不开 | 与 #2123 同源 |
| [#2142](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2142) | 删除会话后重启出现幽灵索引，`local_thread_catalog` 有行、rollout 已无 | #2092 可能未完全覆盖 |
| [#2134](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2134) / [#1948](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1948) / [#1980](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1980) | 历史会话/项目批量消失 | 与 #2146 同族 |
| [#2140](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2140) | 启动报 `找不到包。 (0x80073CF1)` | MSIX 包标识问题 |
| [#2148](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2148) | 1.3.0 重启报 `0x80270254` | 日志显示包名已变 `OpenAI.ChatGPT-Desktop` |
| [#2074](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2074) | 环境变量误判、图标丢失、API 配置不注入 | |
| [#2072](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2072) | 市场迁移后 GitHub 已装插件不在列表/会话工具中 | |
| [#1310](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1310) / [#2076](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2076) | helper 端口 57321 绑定失败 / 端口排除范围 | 硬编码端口类 |

**#2146 值得单独说**：这是一条风险预警而非可复现 bug，报告质量很高（DiskGenius 目录项 + 逐文件校验 + 完整时间线，删于 `2026-09-10 14:39:48`，`.codex` 重建于 14:59）。我按 AGENTS.md 的安全规则扫了一遍递归删除点：`skills.rs` 的 `remove_link` 对软链用 `remove_file`/`remove_dir`、只对实体目录用 `remove_dir_all`，注释里明确写了"绝不能用 remove_dir_all 顺着链接删"；`plugin_marketplace.rs` / `skills.rs` 的 `remove_dir_all` 都指向 staging/backup 目录。**没有找到能删整个 `.codex` 的代码路径**。但用户环境是 `launchMode: "patch"` + Codex 桌面版当天自动更新 + Codex++ 自更新三者同时发生，最可疑的是更新流程里的路径失效后回退。建议：加一道"拒绝删除 `CODEX_HOME` 本身及其祖先目录"的硬校验，无论触发源是什么。

### C. 聚类 → 一份改动能关一片（约 50 条）

**C1. GPT-6 / Max / ultra / Fast 全家桶**（#2124 #2121 #2119 #2109 #2143 #1439 #1400 #1392 #1377 #1463）
本质是 model catalog 模板滞后。`#2112`（Astra 元数据）已合，但 gpt-6 系列的 max/ultra 与 `additional_speed_tiers: ["fast"]` 覆盖不全。**一份 catalog 更新能关掉 10+ 条**。

**C2. `invalid transport` / config.toml 加载失败**（#1997 #2123 #2147 #994 #2144 #1280）
两条修复线：(a) 写入前校验 `mcp_servers.*` 必须有 `command` 或 `url`，残缺条目直接丢弃；(b) 写入前校验 `model_catalog_json` 指向的文件真实存在（#2123 指出的 `%userprofile%\.codex\codex-models.json` 旧路径 + codex 核心不展开 `%userprofile%`）。

**C3. 供应商切换 / 历史会话一致性**（#2090 #2127 #2067 #2047 #1766 #1963 #1650 #1470 #1531）
#2110 + #2122 已修主干，剩余是官方↔第三方来回切时的加密内容（`invalid_encrypted_content`）与 provider 命名（统一生成 `"OpenAI"` 易与官方混淆 —— #2127 的建议 C 值得独立立项：用 `sub2api_xxx`/UUID 做稳定 provider ID）。

**C4. 会话删除 / 幽灵索引**（#2142 #2105 #2079 #2073 #1469 #1337 #1284 #1925）
#2092 修了主干，但 #2142 的 `local_thread_catalog` 残留说明覆盖不全。

**C5. 插件 / 插件市场**（#2051 #2072 #2138 #2049 #2103 #1338 #1323 #1256 #1151 #1346 #1271）
26.818+ 远程认证 + pureApi 中转用户无法安装（`plugin/read -32600`）。

**C6. 非官方模型 + 协议转换**（#2150 #1476 #1795 #1796 #1781 #1431 #1415 #2026 #1748 #1407 #1493 #1851）
#1796 / #1781 / #1431 / #1493 / #1851 都是**同一个 bug**：responses→chat 转换后 ID 前缀不对（`resp_*_msg` 被官方端拒，`invalid_id_prefix`、`[ArrayParam]` 长度问题）。#1964 修过一轮 `ctc_` 前缀，看来还有漏网。

**C7. 皮肤 / 汉化 / UI**（#2064 #1905 #1893 #1442 #1428 #1397 #1277 #1784 #2000 #2085 #2139 #2014）
#1712（Dream Skin 对齐 26.825）已合，剩余多为版本漂移。

**C8. 纯 API 模式下的工具/插件不可用**（#2038 #1346 #1271 #1436 #1409 #1590）
catalog 缺 `js_repl` / 工具能力标识导致 Codex 自动禁用。

### D. 功能请求，建议明确排期或不排（约 35 条）

高频且互相重复的：**单独配置代理**（#2135 #1973 #1557 #1340）、**Token 统计面板**（#2143 #2053 #1427 #1504）、**多供应商并存/轮换**（#2113 #1833 #1799 #1347 #1524 #1294）、**本地 models.json 合并**（#1772 #1685）、**备份恢复**（#1825）。另有 **Linux 版本**（#2032）、**云同步**（#1840）这类长期请愿，建议给个"暂不做"的明确回复。

---

## 三、建议的下一步（按性价比排序）

1. **合 #2136** —— 已实测 148/148 通过，修的是一类高频故障（`wire_api="chat"` 整份配置作废）。合并时引用 #2150。
2. **让 #2137 rebase 到含 #2136 的 main 后合** —— 改动只有 3 行 + 1 个例外条件，冲突机械。要求作者补一条 catalog 生成的回归测试。
3. **回 #1770** —— 告知需 rebase 到最新 main（那次 launcher 测试失败是过期 base 造成的），并要求补 UI 开关的导入路径 / 或在 PR 描述里说清"只能在管理器手动勾选"。
4. **处理 #2084** —— 别再等了，直接告知作者按"每会话循环 + 进度上报"重写，或你接管 branch。
5. **开一条 umbrella issue 处理 C2（`invalid transport`）** —— #1997/#2123/#2147 串起来，两条修复线（MCP 条目校验 + catalog 文件存在性校验）加起来改动量可控，能一次关 5+ 条。
6. **给 #2146 加硬校验** —— 加一道"拒绝删除 CODEX_HOME 及其祖先"的断言，成本极低，是数据丢失场景的标准防线。
7. **打标签** —— 200 个 open issue 零标签，先跑一遍批量打标，后续分诊成本会降一个量级。
