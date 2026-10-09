# 能把 open issue 关掉的 PR —— 逐一核对（2026-09-11）

基线：`main` @ `be6a4585`（v1.3.0）
范围：200 个 open issue × 372 个 PR（6 个 open / 187 merged / 179 closed）

---

## 结论：真正"合一个 PR 就关一条 issue"的，只有 3 组

我做了两轮交叉核对（PR 正文引用 issue、issue 正文引用 PR），200 个 open issue 里**只有 1 条被 open PR 显式引用**（#2148 ← PR #2154）。其余靠 PR 实际改的东西反推。

| open issue | 能关它的 PR | PR 状态 | 我要做的动作 |
|---|---|---|---|
| #2148 `0x80270254` | **#2154** | open，MERGEABLE，零 review | 直接 review + 合 |
| #2150 chat 协议无法使用 | **#2136** | open，MERGEABLE | 直接合 |
| #1407 `Unsupported parameter: 'reasoning_effort'` | **#1770** | open，已 APPROVED，CI 挂 | 本地 rebase 后 CI 即绿 |

---

## 1. #2154 → 修 #2148（唯一一条显式链接的）

**#2148**：Windows 升 1.3.0 后点"重启 Codex++"报 `此应用不支持指定的合约或未安装该应用。 (0x80270254)`。

**PR #2154**（Story19240，+91/-1，2 文件）：根因是 `packaged_app_user_model_id`（[app_paths.rs:488](crates/codex-plus-core/src/app_paths.rs:488)）把 AUMID 应用段硬编码成 `App`，而 ChatGPT Desktop `1.2026.190.0` 更新后 manifest 里的 `<Application Id>` 已不是 `App`。改成从包内 `AppxManifest.xml` 读第一个 `<Application>` 的 `Id`，读不到再回退旧值。

**我的验证**：新增的两个测试 `packaged_app_user_model_id_reads_application_id_from_manifest` / `..._falls_back_to_default_id_without_manifest` → **2 passed**。回退分支保证了 `OpenAI.Codex` / `CodexBeta` 行为不变。

**注意**：同一个作者还有一个 **#2153 已被关闭**，是同一改动的上一版。#2154 是继续推进的版本，且当前 head `2a41afb9` 与 #2153 的说明一致。合 #2154，不要试图恢复 #2153。

**一个我建议顺手加的边界**：`manifest_first_application_id` 用 `find("<Application")` + 下一个字符必须是空白来跳过 `<Applications>` 容器，这在真实 manifest 上是对的；但 manifest 是 XML，属性可能带命名空间前缀。当前实现只按 `attr != name` 精确匹配 `Id`，遇到 `uap:Id` 会读不到而走回退 —— 回退是安全的，不会更坏，所以不阻塞合并，但可以在 review 里提一句。

---

## 2. #2136 → 能关 #2150

**#2150**："codex++ 好几个版本在 chat 协议下无法使用，grok 和 claude 都接不进来，客户端各个高低版本都试过了"。

**PR #2136**：Codex 26.901 起 `wire_api = "chat"` 会让**整份 config.toml 判为无效**、静默回退内置默认模型。PR 把三处写出路径全部改成恒写 `responses`，chat 上游由 `protocol_proxy` 做 responses→chat 转换。

**我的验证**（已 rebase 到 `be6a458` 的 head `2ab1ca8a`）：`relay_config` + `protocol_proxy` + `launcher` → **148 passed / 0 failed**。

**但 #2150 不完全等于 #2136**：#2150 说的是 grok/claude 接不进来，其中 grok/claude 走 chat 协议可能是另一个独立问题。建议合并时在 PR 里写 `related to #2150` 而不是 `fixes #2150`，合完让用户复测再关。

**顺带能关的**：#1476（Chat Completions 模式下 config 始终被重置为 `wire_api = "responses"`）、#1795（Chat Completions 转 Responses 测试通过但实际 502）也是同一根因域，但这两条描述的现象在 26.901 之前就存在，合完让用户验。

---

## 3. #1770 → 能关 #1407（已 APPROVED，卡在 CI）

**#1407**：用 GPT 5.6 系列时报 `Unsupported parameter: 'reasoning_effort'`。

**PR #1770** 加了一个 opt-in 的"纯标准协议"开关：开启后强制 `ChatReasoningStyle::Default`，不再注入 `reasoning_split` / `thinking` / `enable_thinking` / OpenRouter `reasoning`，但保留标准 `reasoning_effort`。

**这个 PR 已经你本人在 9/09 APPROVED**，随后因为 CI 失败撤销：

```
src/App.tsx(10289,21): error TS2304: Cannot find name 'noAuth'.
```

作者 9/10 已按你给的修法推送 `5f733399`（`RelayProfile` 类型补 `noAuth`、各构造点补 `noAuth: false`、normalize 处改读 `profile.noAuth`）。

**我做的验证**：

- 前端 `npm test` → **159 passed / 0 failed**
- 本地 `git rebase main`（从 `48d4315` 到 `be6a458`）→ **干净通过，无冲突**
- rebase 后 `cargo test -p codex-plus-core --test launcher` → **83 passed / 0 failed**

rebase 前 `tests/launcher.rs` 那条失败（`a_busy_floating_helper_port_fails_immediately_without_waiting`）**确认是过期 base 造成的**：main 已用 #2098 把它重写成 `..._respects_the_platform_retry_budget`（macOS 断言重试 31 次）。**rebase 后 CI 就该绿了**，不用再让作者改代码。

**合并后能一并处理的功能请求**：#1392（模型速度选择）、#1382（自定义模型在 codex 里显示切换）也与这个开关高度相关。

---

## 4. #2084 —— 修的是最痛的一批 issue，但现状不能合

**PR #2084**（历史会话流式修复）覆盖的 issue 面很广：#1424（WSL 下修复会话失效）、#1366 / #1465（历史会话修复失败）、#2080（启动前自动修复历史会话无效）、#2090 相关路径。

**状态**：`CONFLICTING/DIRTY`，你已在 PR 里连续 4 次催 rebase 并给出架构性分歧说明。作者最后一条回复是 09-05，**已僵持 6 天**。

按你上次的说法，这不属于"直接合 PR 就行" —— 需要重建。**结论：不指望这个 PR，这批 issue 要么你接管 branch 重做，要么明确告知作者按"每会话循环 + 补进度上报"重写。**

---

## 5. 已被合并 PR 修掉、但现在还 open 的 issue（建议直接关）

这些不需要任何新 PR，**验证一下就能关**：

| issue | 已合并的修复 | 需要确认 |
|---|---|---|
| #1480 纯 API 图片生成不出来 | #2104（本地代理加 `/v1/images/generations` 路由，我已在 [protocol_proxy.rs:523](crates/codex-plus-core/src/protocol_proxy.rs:523) 确认落地） | 让用户复测 |
| #2064 换肤后首页输入框不可见 | #1712（`fix: clip home composer corners on Windows` / `round home composer corners` 都在 main） | 让用户复测 |
| #2026 deepseek 反复循环对话 | #1964 `fix(proxy): preserve ctc_ IDs for custom tool calls`（[protocol_proxy.rs:2336](crates/codex-plus-core/src/protocol_proxy.rs:2336) 已落地） | 让用户复测 |
| #1748 Agent 工具调用无限循环 | 同上（#1964 的 ID 保真问题） | 让用户复测 |
| #2016 Kimi 所有请求报错（`$ref` sibling） | #2106（`inline_ref_siblings` 已在 [protocol_proxy.rs:3385](crates/codex-plus-core/src/protocol_proxy.rs:3385) 落地） | 让用户复测 |
| #2076 `failed to bind helper runtime on 57321` | #2098（macOS 主进程探测） | 需确认是否 Windows 也覆盖 |
| #2073 会话仍然无法删除 / #2079 删除不同步 | #2092（`remove_thread_sidebar_references` 已在 [provider_sync.rs:2815](crates/codex-plus-data/src/provider_sync.rs:2815) 落地） | #2142 说明覆盖不全，见下 |
| #1948 历史会话全部消失 | #1969 | 关联弱，建议只当作参考 |
| #1439 GPT 5.6 不能开 fast | main 的 `assets/gpt56-model-metadata-compat.json` 里 `gpt-5.6-sol` / `terra` 都已带 `additional_speed_tiers: ["fast"]` | 直接可关 |

**#2142 要单独说**：它报告删除会话后重启出现幽灵索引，`local_thread_catalog` 有行、rollout 已无。**#2092 明确修了这个，但 #2142 的复现步骤（"完全退出后重启"）说明还有残留路径** —— 这类"删了一半"的 issue 建议不要跟着 #2092 一起关。

---

## 6. 看着像、其实没有 PR 能关的（避免误判）

我核过这几条，**不要指望现有 PR**：

- **#1692 / #1480 的"图片已生成但对话不显示"** —— #2104 修的是**代理 404**（请求根本没到上游），#1692 说的是**上游已返回 `image_generation_call.result` 但没转成 Codex 可渲染的图片项**。这是两个问题，`grep` 了 main 里没有把 `image_generation_call` 转成图片附件的逻辑。**#2104 能关 #1480，关不了 #1692。**
- **#2067 / #1766 切回官方后 `invalid_encrypted_content`** —— 有过 PR #1749（在 chat 转换里保留 `encrypted_content`），但**已 CLOSED 未合**，main 里 `protocol_proxy.rs` 搜不到 `encrypted_content`。**没有任何 open PR 能关这两条。**
- **#2124 / #2121 / #2119 / #2109 GPT-6 相关** —— #2112 只补了 `gpt-6-astra` 一个模型（`assets/astra-model-metadata-compat.json`），main 里没有其它 `gpt-6-*` 条目。这 4 条需要新写 catalog，**没有现成 PR**。
- **#1796 / #1781 / #1431 / #1493 / #1851 的 ID 前缀族** —— 都在报 `resp_*_msg` / `invalid_id_prefix` / `[ArrayParam]` 长度。#1964 修的是 `ctc_` 命名空间，是问题的一半；这几条的 `resp_*_msg` 前缀问题**没有对应 PR**。
- **#1594 上下文窗口被硬编码 258K** —— 有过两个 PR（#1722、#1786）**都已 CLOSED 未合**，issue 仍 open。这条值得单独看：**有人试过两次都没进去**，说明有障碍。

---

## 建议的执行顺序

1. **review + 合 #2154**（#2148）—— 零 review、零评论，+91/-1 且两个新测试我都跑过了。
2. **合 #2136**（#2150 + #1476 + #1795）—— 148/148 实测通过。
3. **#1770 触发 CI** —— 代码不用改，只需重新触发；rebase 后本地 83/83。
4. **批量关第 5 节那批** —— 已有合并修复的 issue，让用户复测后关，能把 open 数直接砍掉约 9 条。
5. **#2084 做决断** —— 别再等作者，直接接管或明确要求重写。
6. **#1594 单独看一眼** —— 两次尝试都被关，值得查是为什么。

要把第 1–3 步直接做掉吗？合并前我会问你（按你之前的规矩）。
