# 最近 24 小时 open issues 补充审核

日期：2026-10-09，北京时间。

本文件保留实施前的审核快照；后续实施与 GitHub 处置见[修复结果](2026-10-09-issue-fix-results.md)。

范围：上一轮查询中，2026-10-08 01:07:49 至 2026-10-09 01:07:49 新建且仍 open 的 14 条 issue。不是所有历史 open issues 的重新审核。

代码基线：`41048a2c0d687fd126cb12cfbf1a7c533f96d781`，本地 HEAD 与公开 GitHub main 一致。v1.7.0 已于北京时间 2026-10-09 00:25 发布；不能把“已发新版本”当作每条 issue 已修的证据。

上一轮 [10 月 8 日处置记录](2026-10-08-issue-fix-and-triage-results.md) 已审核 #2412、#2413、#2414。本轮补审另外 11 条，同时复核这 3 条的新增证据。所有条目核查时仍 open。

本次是代码与证据审查，只新增本报告；未实施产品代码修复、合并 PR、发布安装包或在 GitHub 留言。下述方案均为待实施/待验证，现有测试通过不等于 issue 已修。

## 总表

| Issue | 本轮结论 | 处理方向 | 优先级 |
| --- | --- | --- | --- |
| [#2426](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2426) | 找到仍存在的 Windows 生命周期缺口，日志吻合，尚无实机复现 | 实际进程身份追踪、CDP 连续失败宽限、退出终态与原因日志 | P1 |
| [#2424](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2424) | 未知内容类型会被 Chat 转换丢弃，但真实 agent payload 的协议形态尚未证明 | 先核真实脱敏 wire 样本；审 #2425 的加密内容处理，不直接按明文合并 | P1 定位 |
| [#2423](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2423) | 作者提供签名失效证据，与 #2410 现有修复相关；版本描述冲突 | 完整重装正式 v1.7.0 两个 app 并复核签名，验证已有修复 | P1 验证 |
| [#2421](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2421) | 压缩保护报错仍未定位，不能以旧压缩修复认定已解决 | 分清原生 Responses 压缩、Chat 摘要和上游保护能力；核 #2303 | P1 定位 |
| [#2419](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2419) | 附图显示分享按钮误挂右侧 review 区；代码宿主 fallback 过宽 | 限定当前会话 header，缺少可靠宿主则不插；增加独立显示开关 | P2 |
| [#2418](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2418) | 截图与代码均确认多个分组默认全部折叠，属于操作成本问题 | 默认展开模型配置，记住分组展开状态，错误分组自动展开 | P2 |
| [#2429](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2429) | 首条消息后白块遮挡主题；具体节点未确定 | 获取遮挡层 DOM/计算样式后，修局部材质或受限重识别 | P2 定位 |
| [#2417](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2417) | 无平台、版本和阶段日志，不能确认同 #2426 根因 | 先识别卡住的启动阶段；provider_sync 慢时评估 #2382 | P2 定位 |
| [#2422](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2422) | 发帖者确认“纯标准协议”可绕过 adaptive 错误 | 补 Kimi-K3 网关回归与设置提示，原生厂商行为保持兼容 | P2 回归 |
| [#2420](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2420) | Daybreak 模型门控报错，用户用 1.5.1，尚未最新版复现 | 核第三方/官方供应商和客户端模型门控，先用当前版本复测 | P2 定位 |
| [#2427](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2427) | 当前缺少独立全局听写入口，属于新增功能 | launcher/native 热键服务接现有录音状态机；按住说话需 key-up 能力 | P3 功能 |
| [#2414](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2414) | 已审；新增评论指向“对话宽度居中”，不能归为纯皮肤 | 收窄居中候选至当前会话内容/输入框，排除整个 pane 和布局祖先 | P1 复核 |
| [#2413](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2413) | 已审；没有新增可定位材料 | 固定官方版本与主题，对照图片层/居中开关，定位历史记录遮挡层 | P1 取证 |
| [#2412](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2412) | 已审；Windows 设置循环仍缺复现信息 | 收 Windows build、官方版本、完整错误及原生直启对照 | 继续取证 |

## #2426：重启后 launcher 静默退出

确认的代码缺口：`crates/codex-plus-core/src/launcher.rs:1139` 等待 Windows AUMID 激活返回的 PID，失败后仅记录 `packaged_process_wait_failed_nonfatal`。`:1155` 的存活循环只要一次探测失败就结束；非 macOS 的 `launcher_owned_process_alive` 在 `:2938` 恒为 false。

`crates/codex-plus-core/src/cdp.rs:8` 的同步探测超时只有 300 ms，`:47` 的判定还要求 `/json` 当前包含匹配的 Codex 页面 target。页面重载、target 暂空或慢响应可产生 false。等待返回后，`launcher.rs:203` 关闭 helper，却没有明确保存停止终态。这与“ready 后端口消失，latest-status 停在 running”的报告吻合。

尚未确认：实际失败的 Win32 错误、AUMID PID 是否只是激活中转进程、退出前是否确有 CDP 短暂不可达。10/04 的 #2393 也出现过该日志，不能把“官方 26.1002 比已验证版本新”直接认定为原因。

最小修复：

1. 记录完整 Win32 错误链以及退出时的进程身份、CDP 状态和连续失败次数。
2. 启动/注入成功后跟踪本次 debug port 对应的实际 Codex 主进程；使用句柄或 PID 加创建时间，排除 PID 复用与其他实例。创建时间机制可参考 `windows_integration.rs:365`。
3. 无可靠进程身份时保留有界连续失败宽限，避免一次失败关闭 helper；仅放大 300 ms 不能解决身份判定问题。
4. 确认退出后记录原因并保存 `stopped` 终态。

验证：激活 PID 先退、CDP 短暂中断后恢复、真实退出释放端口、其他实例不保活、PID 复用；Windows Store 实机连续重启至少十次，保留阶段/进程/端口对照。

## #2419：分享按钮误挂与开关需求

已查看 issue 附图：按钮出现在右侧“变更”review 标签下方，额外占一行，主要现象不是按钮内部文字换行。

`assets/inject/renderer-inject/80-session-share.js:309` 会从整个 document 的任意 `header .ms-auto` 选择宿主，`:325` 又回退到任意 header，最终甚至回退 body。`95-conversation-view.js:894` 无条件调用 installer，没有独立显示开关。附图与这些误挂风险吻合，真实宿主 DOM 尚需复现确认。

最小修复：只在当前会话的可靠 header/action group 安装按钮；未知布局则移除误挂节点并停止安装。新增增强设置中的显示开关，保持已发布的 `codex-session-share-button` 类名和 constants。现有 CSS 已有不收缩与 `whitespace-nowrap` 处理，不应仅修防换行样式。

验证：多个 header、聊天加 review/browser/terminal pane、没有原生 Share、窄窗口、会话切换和 header 重建；关闭后无残留，恢复后只有一个正确位置的按钮。新增节点遵守 `data-codex-plus-ext` 归属规则。

## #2418：供应商配置全部折叠

已查看附图：模型配置、请求设置、渠道保护、更多选项及文件预览均收起。`apps/codex-plus-manager/src/App.tsx:7753` 的 `RelayFold` 没有默认展开参数、展开状态记录或错误自动展开；`:8340` 的模型配置也沿用这一行为。`styles.css:1946` 明确隐藏所有未 open 的内容。v1.5.4 尚无这些 `RelayFold`，不是旧版同一入口被用户自行收起。

最小修复：增加以稳定 section ID 为键的 `defaultOpen`/`onToggle` 行为；模型配置初次默认展开，高级参数和凭据预览继续按需展开。展开偏好存为本机界面偏好，不混入 RelayProfile。校验错误首次出现时展开对应分组，避免只在标题显示“需要检查”而隐藏错误字段。保持字段挂载和未保存草稿。

验证：首次进入、重进同一/另一供应商、切换语言后偏好仍有效、错误字段可见、展开/收起不改配置且不丢草稿。该项不需要后端协议改动。

## #2427：全局听写快捷键

确认缺口：`DictationSettings`（`crates/codex-plus-core/src/settings.rs:437`）没有热键字段；检索当前注册代码未发现独立全局热键服务。`assets/inject/renderer-inject/93-dictation.js:450` 的网页 `keydown` 只处理录音中的 Enter 和 Escape，并不是系统级开始录音入口。现有开始/停止/回填流程位于 `:156`、`:243`、`:254`。

最小实现方向：在随 Codex 生命周期运行的 launcher/native 服务注册 opt-in 热键，避免依赖用户一直打开管理器；将开始、停止、取消命令接到现有 renderer 状态机。触发时选定并聚焦目标窗口/composer，继续绑定会话与 composerId，导航或目标变化后保留可复制文字，默认仅插入不自动发送。配置需覆盖启用、键位、暂时停用和冲突提示；关闭、退出、重启都释放注册。

“按住说话”需要可靠 key-down 与 key-up。若采用 Electron 主进程注册，[globalShortcut 文档](https://www.electronjs.org/docs/latest/api/global-shortcut)只提供按下回调和注册成功返回值，不能声称仅靠 `register` 就完成松开停止。该路径还需新增并验证主进程接入，不能重新启用已退役的菜单本地化模块。可以分阶段先交付按一次开始/再按停止，再补有 key-up 能力的平台实现；完整满足原需求仍须按住/松开的实机验收。

验证：无焦点、多个窗口、无活动会话、快捷键占用、按键重复、开始授权期间松键、切会话、禁用或退出释放、重启不重复注册；ASR 密钥、音频传输和原生回填沿用既有边界。

## #2423：macOS 签名损坏

作者[评论](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2423#issuecomment-6061724538)报告两个 app 未签名，崩溃为 `SIGKILL (Code Signature Invalid)` / `Taskgated Invalid Signature`。正文说 1.6.0，评论说 1.5.4，包版本与来源需要核对。没有取得实际包独立验证。

已修 #2410 的 `ff517796` 覆盖旧“修复入口”改写已签名 bundle 的问题：`install/macos.rs:80`、`:121` 保留原生入口与签名资源，先预检两个 app；`release-assets.yml:213` 和 `scripts/installer/macos/package-dmg.sh:147` 要求正式签名、公证；`update/macos.rs:359` 校验并完整替换 app。v1.7.0 包含这些变更。

处理：完整重新安装 [正式 v1.7.0 DMG](https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v1.7.0) 的两个 app，先验证现有修复。不用损坏旧包的“修复入口”修签名，也不把去除 quarantine 当作签名修复。

验收资料：DMG 来源/hash、两个 app 的版本、是否用过修复入口、`codesign --verify --deep --strict`、`spctl --assess` 与崩溃报告。比较启动与修复入口操作前后签名仍有效。

## #2424：agent payload 与 encrypted_content

确认的转换行为：`crates/codex-plus-core/src/protocol_proxy.rs:4125` 只识别普通文本、refusal 和图片，`:4147` 跳过未知片段；`:3530` 处理未知独立项时只在存在 content 字段的情况下转换。issue 手工构造的 encrypted_content 明文片段/独立项确实会被忽略。Responses 上游在 `:1881` 保留原请求。

但“真实 Codex 把 agent payload 以明文放在该类型”没有真实脱敏 wire 样本支持。[官方 Agents API](https://developers.openai.com/api/reference/resources/beta/subresources/agents/subresources/sessions/subresources/turns/subresources/items/methods/list)确实定义了 agent_message、create_subagent_call、send_subagent_input_call 的 encrypted_content，但说明它是加密内容，字段为 encrypted_content，不是普通 text。这也不能反向证明本 issue 所用 Codex build 必然采用相同具体封装，需要实际样本。

开放 [PR #2425](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2425)直接将 encrypted_content 字符串转为 Chat 普通文本，独立项强制成为 user 消息。它不能解密真实 payload，还可能丢失结构与角色；两条手写 PINEAPPLE 测试只验证构造输入的转发，不建立真实协议语义。该 PR 没有改既有 reasoning/compaction 分支，不能笼统称其会解密所有加密推理。

最小处理：先取得真实 NEW_TASK/MESSAGE 的脱敏结构，确认类型、字段、编码和消息角色。普通明文走标准 input_text/output_text；没有已知转换契约的 opaque agent 内容遇到 Chat 上游，应给明确的不支持错误，避免静默丢弃或假装解密。支持该契约的 Responses 上游原样透传。#2425 需据此修订后再评审，不能按当前方案直接合入。

验证：真实脱敏 fixture、明文顺序与角色、opaque sentinel 不进入 Chat 普通文本、不支持时明确失败、Responses 结构透传、reasoning/compaction 负例；另覆盖字段为空、非字符串、缺字段等形态，PR 的 or_else 只在键不存在时回退，并不是所有无效值都回退。

## #2421：压缩时 response protection unavailable

三条评论均没有维护者诊断。普通用户[反馈](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2421#issuecomment-6057318481)官方登录压缩也会断连但报错不同，sub2api 才是 protection unavailable；不能据此断言同一根因。

`protocol_proxy.rs:1866` 检测 compact，仅 Chat 上游走合成摘要；Responses v2 在 `:1951` 保留 trigger 并调用 /responses；beta header 在 `:1356` 透传。`channel_protection.rs:56` 则管理排队、速率和 cooldown，源码没有生成本条报错。不要因为 protection 字面相近建议关闭本仓“渠道保护”，也不要改压缩阈值代替协议定位。

开放 [PR #2303](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2303)的 safe replay/fallback 提交没有进入当前 main。它处理 native checkpoint 与本地摘要的区分、明确拒绝 trigger 后的受控降级等问题，不能承诺解决服务端保护失败。真实 failed 事件、401/403/429/5xx 或截断流都不应触发普通摘要重试。

确认的可观测性缺口：`crates/codex-plus-core/src/launcher.rs:2344` 原生 Responses 流只观察传输错误；SSE 内 response.failed 或正常 HTTP EOF 缺少成功终止事件，仍可能透传后返回成功，日志记 stream_ok。这不足以证明本条故障由它导致，但适合作为第一步诊断改进。

最小处理：在原样透传旁统计 compact_v2、上游协议、终止事件、错误 code 和 request ID，不记正文或加密 payload；比较同一模型的新/旧会话在官方直连、sub2api 直连、经 relay 三条路径。区分上游保护能力、代理断流和显式不支持 trigger。只有明确 400/422 拒绝 trigger 才评估受控降级，保留历史与 opaque checkpoint。

验证：成功 JSON/SSE/checkpoint 原样保留；protection unavailable 的 failed 不重试、不伪造成功；EOF 缺 completed 记失败；unknown compaction_trigger 与普通输入错误分开；429/Retry-After 继续 cooldown；原生加密历史转 Chat 明确不兼容。现有 `tests/protocol_proxy.rs:5558` 的成功透传测试不能代替这些失败路径。

## #2422：Kimi-K3 拒绝 adaptive

发帖者[反馈](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2422#issuecomment-6061600813)开启“供应商配置 → 请求设置 → 纯标准协议”后解决，尚无维护者独立验证。当前 main 已有开关：`settings.rs:123` 与 `App.tsx:8867`。

`protocol_proxy.rs:6485` 在标准协议关闭时按模型名选择厂商方言，`:6493` 注入 thinking，`:6641` 对 Kimi coding 名返回 adaptive。报告者的网关拒绝这个私参，与上述绕过吻合；真实上游地址缺失，不能反推所有官方 Kimi 接口都应改成 enabled。旧 #2183 审查恰是另一端拒绝 enabled 而需要 adaptive，`c5ab8a1f` 已在 main。

最小处理：保留 per-profile 标准开关与原有默认行为，补 Kimi-K3 开关专项回归和明确错误提示。不要全局将 adaptive 改为 enabled，不隐式改其他供应商配置。

验证：standard=true 时不发送 thinking、false 保留旧方言、设置 round-trip、不同网关不互相污染。当前 `supports_reasoning_effort`（`:6649`）不把 Kimi alias 识别为标准推理模型，开启标准协议可能也省略 effort，不能承诺 xhigh 效果完全保留。既有测试 `tests/protocol_proxy.rs:809`、`:859` 只覆盖默认 Kimi 与其他标准开关场景，需补这个组合。

## #2420：Daybreak 模型门控

无评论、关联 PR 或新版复现。报告使用 1.5.1，未确认最新版仍复现。当前相关源码未找到 Daybreak gate；bundled catalog 的模型/权限字段定向检查也未发现对应条目，不能直接归因到本仓某一 guard。

[官方 Daybreak 文档](https://developers.openai.com/api/docs/guides/daybreak)说明模型与访问项目需匹配，参数本身不授予访问。因此模型/权限不匹配是候选，而非已确认原因。

`model_catalog.rs:915` 的 visibility/supported_in_api 过滤和 `model_suffix.rs:544`、`:603` 的模板生成仅是调查入口，不是它们导致报错的证据。

最小处理：在 v1.7.0 核对实际 model ID、官方 Codex build、登录/供应商模式和元数据来源，先用有权限的普通模型复测。若确认客户端列表把不可用模型当可用，再基于实际 entitlement 限定该模式的过滤和提示；不伪造权限，不默认开启 Daybreak。

验证：有/无权限与普通/受限模型组合、官登/第三方 provider、新/恢复会话、旧选择恢复、自定义同名 alias。实际请求模型和模式的诊断应脱敏。

## #2417：启动等待，原因未明

没有平台、版本、日志或评论。不能直接与 #2426 合并。

当前 `crates/codex-plus-core/src/launcher.rs:529` 仍在拉起窗口前同步等待 provider sync；`apps/codex-plus-launcher/src/main.rs:491` 内还串行等待首次消息索引修复。已有开放 [PR #2382](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2382)优化历史扫描并把索引修复移到启动/注入后，但不修改重启状态机。

先取 `latest-status` 与脱敏 `launcher.phase` 的最后阶段和耗时，区分 `restart_stopping`、`provider_sync`、`helper_listening`、`launch_codex`、`ensure_injection`。只有卡在 provider_sync 才评估 #2382；已经 ready 后 helper 消失则参考 #2426。验证需包括大量历史、关闭同步、冷启动与重启对照。

## 已审核三条的复核

- **#2414**：新增用户评论说明关闭“对话宽度居中”、保存并重启恢复，另有无皮肤也留白的反馈。`90-action-groups.js:499`、`:512`、`:540`、`:897` 的居中候选会扩展到整个 document；计算样式的 CSS 变量可能只是继承，不能据此识别内容容器。候选需限定当前会话滚动区/footer，排除侧栏、工具 pane 与布局祖先。加新版多 pane 和继承变量夹具，验证输入框、消息和布局宽度；尚不能确认每位报告者同根因。
- **#2413**：没有新增官方完整版本、主题名称或遮挡节点资料。Dream Skin 引擎与图片覆盖层是不同链路；不能因症状近似 #2429 直接认定同源。继续收主题、计算样式和关闭图片层/居中的对照。
- **#2412**：[已有维护者回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2412#issuecomment-6051533336)仍在等 Windows build、官方版本、完整错误及原生直启对照。`relay_config.rs:2628` 仅处理明确 elevated 且非提权的窄情形，不能扩大成所有设置循环的修复，不能为了绕过问题关闭沙盒。

## #2429 与图片层 PR #2428

#2429 没有截图、评论或实际遮挡节点信息。Windows Dream Skin 的 parts 重识别位于 `assets/inject/upstream/dream-skin/windows/renderer-inject.js:768`，SPA 调度在 `:920`，整树 observer 在 `:988` 只监听 childList，`:1007` 另有 30 秒 safety pass。白块出现再自动恢复可能涉及材质规则或识别时序，但这只是候选，不能直接认定。

先记录白块节点、computed background/position/z-index、出现恢复时间和皮肤 state/metrics；定位后只修真实遮挡层 selector。若是属性变化遗漏，增加受限区域/属性观察，不全局清空背景或提升所有内容 z-index，不回退整树无过滤扫描。

验证：固定官方版本和主题，默认/自定义主题、居中、图片覆盖层开关矩阵；新会话首次发送、streaming、完成、切会话、深浅色，以及历史、输入框、图片、弹窗仍可见可用。

开放 [PR #2428](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2428)有社区主题 footer 材质与图片层级修复，可作候选；尚未合并且 API 显示合并冲突。其 diff 没改分享 installer 与居中目标识别，不能据此认定 #2419/#2414 已修。还回退了 `0616c8b4` 的远端删除归属 guard、不可撤销提示及 DOM 删除限制，审合前必须 rebase 并保留已有修复。

`git diff v1.6.0..41048a2c` 表明 Windows Dream Skin renderer/CSS 和分享 installer 没变化，居中仅增加 custom layout owns-element 排除；v1.7.0 不足以作为本组故障已修证据。

## 验证边界

当前基线的现有 Cargo 测试：launcher 集成 92、installers 集成 21、`owned_launcher` 单测 2、protocol_proxy 集成 143、`proxy_stream_tests` 单测 3、`dictation_renderer_contract_harness` 1，合计 262 passed、0 failed。只计实际执行的测试，不把 filtered out 或历史报告计入本轮。

命令使用已安装 `/Users/mac/.cargo/bin/cargo`，没有安装依赖或切换工具链。现有编译 warnings 与本次报告无关。测试说明当前基线已覆盖的路径可运行，不说明上述新场景已修；Windows Store 重启、真实上游压缩和实际 GPU/DOM 时序未完成实机验收。

后续实际修改 renderer 分片时，遵守现有拓展类名/constants、路由白名单、注册表生命周期和 `data-codex-plus-ext` 契约，并执行 `node scripts/assemble-renderer-inject.mjs` 重新组装产物。本轮只编辑报告，不需要重新组装。
