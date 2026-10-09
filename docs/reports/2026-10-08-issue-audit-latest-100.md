# CodexPlusPlus 最新 100 个 issue 审核

审核日期：2026-10-08，北京时间。仓库：`BigPizzaV3/CodexPlusPlus`。范围按**创建时间倒序、包含 OPEN 与 CLOSED、排除 PR**取最新 100 条，覆盖 **#2257 至 #2414**，创建时间为 2026-09-20 至 2026-10-08。编号间隔来自 PR 和其他记录；不是连续 100 个编号，也不是最近更新的 100 条。

代码基线：`main @ 1db62e9239ce564e670219922584f66f35508ef8`，已与 GitHub main SHA 核对一致。最新正式版为 [v1.6.0](https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v1.6.0)，北京时间 2026-10-08 01:15 发布；macOS 与 Windows 下载资产均已上传。

**结论：优先处理 macOS 修复入口破坏签名、发送阻塞和新布局回归；重新审视社区主题和桌面图标两条已关闭 issue。旧报告中大量“已修未发布”提醒已过期，但发版或关单都不能代替现场验证。**

## 审核统计

| GitHub 状态 | 数量 |
| --- | ---: |
| OPEN | 27 |
| CLOSED | 73 |
| 合计 | 100 |

以下结论互斥，每条 issue 只归入一类；与 GitHub 状态分别统计。

| 审核结论 | 数量 | 含义 |
| --- | ---: | --- |
| 待修 | 4 | 当前代码可确认缺口；其中 2 条已关闭 |
| 部分处理 | 11 | 仅覆盖一部分诉求，或实现与产品语义仍需拆开 |
| 待定位 | 26 | 保留问题，需要新版本复现或受控证据 |
| 已修待验证 | 43 | 相关修复存在并已发版；不代表每条都获用户确认 |
| 咨询或需求 | 16 | 使用咨询、范围问题或产品需求 |
| 合计 | 100 | |

P1 表示启动、发送、安装、工具使用等核心能力受阻，P2 表示有绕过办法的功能或体验问题，P3 表示产品讨论。优先级反映问题影响，不表示已经在 v1.6.0 复现。此次没有足够证据认定新的 P0。

## 当前代码可确认的四项缺口

### macOS 修复入口会破坏正式安装

关联 [#2410](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2410)，P1。

调用链为 [管理器 install.rs:14](/Users/mac/Desktop/codexplusplus/apps/codex-plus-manager/src-tauri/src/install.rs:14) → [repair_entrypoints:108](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/install/mod.rs:108) → [install_app_bundles:80](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/install/macos.rs:80)。

正式 DMG 在 [package-dmg.sh:74](/Users/mac/Desktop/codexplusplus/scripts/installer/macos/package-dmg.sh:74) 将 Mach-O 二进制直接安装为 `CodexPlusPlus` 和 `CodexPlusPlusManager`，随后在 [159 行](/Users/mac/Desktop/codexplusplus/scripts/installer/macos/package-dmg.sh:159) 签名二进制和 bundle。但修复流程在 [macos.rs:115](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/install/macos.rs:115) 重写 `Info.plist`，在 [130 行](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/install/macos.rs:130) 将同名入口原地覆成 shell wrapper，没有重新签名。

标准双 app 安装且目标可写时，这条路径确定会修改已签名的包。只读卷、权限不足或缺少源 launcher 可能使其提前失败。**签名破坏可静态确认；当场闪退的具体时机尚未动态复现。** Apple 明确讨论了原地改写已签名可执行文件触发签名缓存相关崩溃的问题，并要求修改后的 bundle 重新签名。[Updating Mac Software](https://developer.apple.com/documentation/security/updating-mac-software)、[TN2206](https://developer.apple.com/library/archive/technotes/tn2206/)。

建议让“修复入口”恢复入口注册或使用完整的已签名安装包恢复 app，避免在运行中的正式 bundle 内重建 wrapper。验证应覆盖正式签名 DMG 安装 → 点击修复 → 退出重开 → 签名仍有效。

### 已删除桌面图标仍会被升级重新生成

关联 [#2376](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2376)，P2，当前 CLOSED。

[NSIS:85](/Users/mac/Desktop/codexplusplus/scripts/installer/windows/CodexPlusPlus.nsi:85) 和 [Rust windows.rs:102](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/install/windows.rs:102) 都是“存在就跳过、不存在就创建”。用户删除快捷方式后文件不存在，升级正好进入创建分支。

`be62bc16` 已随 v1.5.1 发布，但只避免覆盖已有快捷方式，没有实现“首次安装可建、升级尊重删除选择”。[关闭评论](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2376#issuecomment-5965357273) 对用户承诺的行为与实现相反。

建议重新跟踪这条需求，区分首次安装、升级和用户主动修复；用首次安装标记或持久化选择表达用户意愿。必须验证“首次安装 → 删除图标 → 覆盖升级 → 图标仍缺失”，仅验证已有图标不被覆盖不够。

### 社区主题版本仍按错误的版本体系比较

关联 [#2339](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2339)，P2，当前 CLOSED。

2026-10-07 用户已补充 v1.5.4 实测：社区包 `minClientVersion=26.805.11740` 或 `26.818.31338`，客户端仍拒绝安装。当前 [dream_skin_package.rs:923](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/dream_skin_package.rs:923) 继续比较 `manifest.min_client_version` 与 CodexPlusPlus 自身的 `crate::version::VERSION`。v1.6.0 仍小于这些 `26.*` 值，因此升级产品版本不能解决这些包的拒绝。

还需区分另一层契约：上游校验器把 `minClientVersion` 解释为 **Dream Skin 引擎最低版本**，Windows 调用方从引擎目录 `VERSION` 读取当前值；`skinApiVersion` 是独立的整数协议字段。存量包写成 `26.*` 与该契约不一致，不能未经验证就换成比较 Codex Desktop build 或删除校验。[上游校验器](https://github.com/Fei-Away/Codex-Dream-Skin/blob/6f72d849e8de36aa7eef6bca9f2634e96b5706f7/runtime/theme-package-validator.mjs#L319)、[Windows 调用方](https://github.com/Fei-Away/Codex-Dream-Skin/blob/6f72d849e8de36aa7eef6bca9f2634e96b5706f7/windows/scripts/theme-windows.ps1#L1700)。

建议重新跟踪 #2339，明确本仓内置引擎的经过验证的兼容版本，并与市场处理错误标注的存量包。市场 `applyCompatible=true` 不能替代本地兼容校验。

### 本地代理没有动态透传会话请求头

关联 [#2391](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2391)，P1。

[launcher.rs:1238](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/launcher.rs:1238) 只提取 User-Agent 与 `x-codex-beta-features`。代理入口 [protocol_proxy.rs:1147](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/protocol_proxy.rs:1147) 没有原始请求头参数，构建上游请求时不会动态携带 `session-id`、`x-opencode-session`。

供应商自定义请求头可配置固定值，但不能恢复每个会话的实际标识。代码可确认透传缺口；issue 所述缓存和路由后果仍应由上游请求对照验证。

建议使用明确白名单透传客户端会话头，并尊重用户显式配置的优先级。若增加按会话生成逻辑，应只用于有明确契约的上游，避免把 `previous_response_id` 这种可能随轮次变化的值误当稳定会话 ID。

## 优先调查的发送与界面问题

| 优先组 | issue | 当前证据 | 下一步 |
| --- | --- | --- | --- |
| 原型补丁后无法续发 | [#2402](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2402) | 1.5.0 与 1.3.0 同机对照；抓取成功且补丁已安装，第一条成功、第二条阻塞 | 用同一 Codex 26.908 及同一会话复测 1.6.0，记录 turn/start 是否实际发出以及前置 RPC 完成时间 |
| 工作空间策略查询失败 | [#2407](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2407) | 经启动器出现大量 Cloudflare 403，直接启动基本不出现；代理变量未完全控制 | 固定账号、节点、客户端、启动参数，分“启动器未注入”和“注入”对照，查账号查询请求 |
| Windows 沙盒设置失败 | [#2393](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2393)、[#2412](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2412) | 前者旧日志有锚点缺失，后者在 1.6.0 新报设置循环 | 提取沙盒初始化的实际错误；不能只凭 anchor_missing 认定已修 |
| 主题与居中布局回归 | [#2414](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2414)、[#2413](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2413)、[#2408](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2408) | 1.5.3 至 1.6.0 报错位或遮挡；#2408 有关闭居中后恢复的评论 | 固定官方版本，对照默认/自定义皮肤、居中开/关、侧栏与全屏；采集计算样式和尺寸 |
| Gemini schema 兼容 | [#2404](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2404) | 上游 TYPE_STRING 拒绝布尔 enum；当前归一化仅处理默认字段、引用和顶层组合器 | 获取第 28 个失败工具的脱敏 schema 与上游协议；做模型或协议感知兼容，保留参数类型 |
| app-server 抓取第二阶段 | [#2399](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2399) | 锚点修好后的评论记录 Debugger.enable 5 秒超时；现有流程没有该步骤重试 | 冷启动与稳定状态分别采样；为该阶段设计有限重试和可观测失败状态 |

#2402 对本 fork 尤其重要：[70-model-catalog.js:770](/Users/mac/Desktop/codexplusplus/assets/inject/renderer-inject/70-model-catalog.js:770) 在真正发送前等待后端设置和模型刷新；[522 行](/Users/mac/Desktop/codexplusplus/assets/inject/renderer-inject/70-model-catalog.js:522) 的切模型路径还会先做 `thread/resume`。当前只在启用 per-model 配置且已有模型发生变化时触发 resume，不能直接断言它造成所有第二条阻塞，但必须验证新会话、续发、切模型及 helper 故障四种条件。

#2399 的发现阶段已由 `3e85706f` 在 v1.5.2 修复；[bridge.rs:377](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/bridge.rs:377) 的 Debugger.enable 仍是单次调用，通用命令超时仍为 [5 秒](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/bridge.rs:21)。不能把第一阶段修复算成整条 issue 完成。

主题回归也不能统一归为旧 top-fade：旧裸属性选择器已去掉且随 v1.5.0 发布，今天的空白侧栏、溢出和覆盖历史有不同表现。

## 部分结论需要更正

### 渠道保护

[#2406](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2406) 描述的实现事实大部分成立，但“短 Retry-After 不该等 30 秒”不是当前文案与实现不符。[App.tsx:8837](/Users/mac/Desktop/codexplusplus/apps/codex-plus-manager/src/App.tsx:8837) 明确写“至少 30 秒，较长上游时间优先”，与 [channel_protection.rs:141](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/channel_protection.rs:141) 一致。

RPM 确实仅在队列开启时生效，且 UI 将其放在队列区说明。队列 mutex 又由 [UpstreamProxyResponse:524](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/protocol_proxy.rs:524) 保留到整个流消费完毕，因此长响应会串行阻塞后续请求。应明确产品选择：完整请求串行化还是仅限请求启动频率，并让关闭队列后的 RPM 输入状态表达真实效果。

另有边界：[retry_after_duration:156](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/channel_protection.rs:156) 只解析整数秒，未处理 HTTP-date；此次 issue 的实测值都是整数，不能把该边界说成此次故障根因。

### 关闭理由不能替代根因证据

- [#2385](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2385)：`f55bb646` 增加错误日志，解决可诊断性，不等于解决产生 502 的原因。应获取升级后 `helper.*_proxy_failed.error` 再判定。
- [#2379](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2379)：报错字符串不在本仓，只能说明错误由其他组件报出，不能排除本仓生成了错误输入。应比较代理前后的工具 call_id 和传输方式。
- [#2390](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2390)：零评论关单，仍有切换改写认证的对照信号。需要脱敏字段名、认证模式和请求目的地，不能直接排除。
- [#2331](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2331)：最后回复称本仓不生成或改写 `requires_openai_auth`，与 [relay_config.rs:4039](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/relay_config.rs:4039) 和 [5263 行](/Users/mac/Desktop/codexplusplus/crates/codex-plus-core/src/relay_config.rs:5263) 相冲突。图片生成、下载回传和鉴权需要分开定位。
- [#2322](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2322)：同版本重装恢复支持“持久化状态参与”的判断，但重装改变的状态不只一项，不能称为单变量根因已证。仍须新版本原地升级复验。
- [#2273](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2273)：同 Key 在另一个客户端可用是有价值的对照；需要最终请求头和目标 URL，401 不是直接排除本仓的依据。
- [#2325](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2325)：没有直连/代理缓存数据，最后回复的绝对上游归因过强；应先验证稳定请求前缀与会话标识。
- [#2349](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2349)：原文已确认 WindowsApps 内置 CLI 能执行，旧回复仍断言该路径必不可执行，忽略了用户证据。路径自愈和无窗口启动修复已发布，但应获取实际微信 app-server 错误。
- [#2372](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2372)：插件列表过滤器、远程目录认证与安装过程是不同层；修列表不能证明安装失败已解决。
- [#2374](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2374)：Luna 的内置 metadata 没有 ultra；“max/ultra 都补齐”的后续回复不够准确。用户另提的可视化自定义档位入口也未完成。
- [#2259](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2259)：v1.5.2 已补市场卸载，提交说明明确全局开关入口未做；保留 OPEN 合理。
- [#2261](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2261)、[#2395](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2395)：最后模板分别与“Layer 缺陷”和“Linux 不在发行范围”的正文矛盾，应更正回复。

CUA 相关 #2294/#2287/#2369/#2373 应区分 runtime 识别、原生浏览器连接和具体认证调用。已发布的兼容修复不能保证每条认证链路成功，也不能只凭错误字符串所在组件断言所有纯 API 使用方式都不可能。

## 已发布修复与旧报告变化

以本地 Git tag 祖先关系逐项核对，以下提交均被 v1.6.0 包含：

| 首个正式 tag | 相关提交 | 覆盖主题 |
| --- | --- | --- |
| v1.4.0 | `2b4d8d68`、`bed5277d`、`0fd58cc3`、`7e19e069` | 动态 AUMID、重扫熔断、tool_search、Sol/Luna 元数据 |
| v1.5.0 | `549551d3`、`8baffa91`、`35ede662` | 远程压缩事件、top-fade、6.1 Sol metadata |
| v1.5.1 | `8610d2f3`、`5bb4f636`、`be62bc16`、`93a66453`、`c8f8ee30` | Windows 安装器竞态、重注入、Layer 与版本/图标初次处理、提权提示、宽度兼容 |
| v1.5.1 | `a8dcd29f`、`09a9f1c5`、`ecb71213`、`064775ab`、`04ab6290` | 工具结果配对、组合 schema、模型 key 归一化、native runtime 识别 |
| v1.5.1 | `f55bb646`、`e5cd4de7`、`b6e992b3`、`43e6c55c`、`10ff2fe6`、`ea82bc74` | 502 诊断、Dock 唤回、K/M 输入、颜色 token、模型排序 |
| v1.5.2 | `3e85706f`、`4704ac43`、`9b4a19b8`、`6ebfce05`、`7a17b1c1` | 资产发现、路径优先、重启清理、删除墓碑、市场卸载 |

[10 月 3 日最新 100 条报告](/Users/mac/Desktop/codexplusplus/docs/reports/2026-10-03-issue-audit-latest-100.md) 中“#2362、#2330、Dock、图标等修复尚未发布”的提醒应以本次表为准；旧报告保留作为当时快照。

**tag 包含提交只证明代码进入发布基线。** #2376 和 #2339 正是“提交已发布但需求仍未满足”的反例；#2399 还有未解决的第二阶段，#2385 仅修诊断。

OPEN 中 #2403、#2394 的针对性修复已发布，适合先请用户复测；#2340 已有删除墓碑，但“归档失败”没有一并证实，不能直接关单。此次没有修改 GitHub 评论、标签或状态。

## 逐条审核清单

每条均核对当前正文和已抓取评论；长日志按事件与错误特征检查。截图作为用户报障附件，不作未实际检查的像素级归因。静态核对不替代 Windows/macOS 现场复现。

| 序号 | issue | 状态 | 结论 | 优先级 | 主题 | 处置依据与下一步 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | [#2414](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2414) | OPEN | 待定位 | P1 | 主界面空白与侧栏溢出 | v1.6.0 新报，多人复现；应与 #2408 联合查布局，先做居中开关及皮肤对照。 |
| 2 | [#2413](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2413) | OPEN | 待定位 | P1 | 背景遮挡历史记录 | v1.6.0 新报；默认背景可见是有效对照，查覆盖层与侧栏层级。 |
| 3 | [#2412](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2412) | OPEN | 待定位 | P1 | Windows 设置循环阻断登录 | v1.6.0 新报；须补沙盒初始化错误，不能与旧锚点失败混为一谈。 |
| 4 | [#2410](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2410) | OPEN | 待修 | P1 | macOS 修复入口后闪退 | 修复路径重写已签名 bundle；先处理安装布局和签名保持，再复测。 |
| 5 | [#2409](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2409) | OPEN | 待定位 | P2 | macOS 网络变化伴随能力与市场异常 | 代理改变结果且回退无效；分开查网络 gate、官方能力、插件列表。 |
| 6 | [#2408](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2408) | CLOSED | 待定位 | P1 | 更新后界面错位 | 已有关闭居中或恢复皮肤的绕过办法，未见修复提交或最终验证；作为布局主跟踪候选。 |
| 7 | [#2407](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2407) | OPEN | 待定位 | P1 | 工作空间设置失败无法发送 | 有启动方式 A/B 和 Cloudflare 403；仍有代理变量，需受控对照，不能直接归咎补丁。 |
| 8 | [#2406](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2406) | OPEN | 部分处理 | P2 | 渠道保护冷却及限速语义 | 30 秒下限符合当前文案；RPM 耦合队列、流式持锁成立，需澄清或拆分能力。 |
| 9 | [#2405](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2405) | CLOSED | 咨询或需求 | P2 | 官方 WebSocket 重连 | 作者选择代理环境方案后自关；属于网络配置与可选产品能力。 |
| 10 | [#2404](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2404) | OPEN | 待定位 | P1 | Gemini 工具 schema 被拒绝 | TYPE_STRING 拒绝布尔 enum；当前归一化未做此兼容，需取失败工具和转换前后 schema。 |
| 11 | [#2403](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2403) | OPEN | 已修待验证 | P1 | 重启被旧启动器 guard 阻断 | 9b4a19b8 随 v1.5.2 发布；重启异步及超时清理已改，端口漂移需另测。 |
| 12 | [#2402](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2402) | OPEN | 待定位 | P1 | 每会话只能成功发送一条 | 补丁安装成功后阻塞；旧锚点修复不覆盖此结论，保留独立跟踪。 |
| 13 | [#2401](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2401) | CLOSED | 已修待验证 | P2 | 无网络时强制中文无效 | be62bc16 的 Layer 补丁已在 v1.5.1；离线首读是否正常仍需实测。 |
| 14 | [#2400](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2400) | OPEN | 待定位 | P1 | 使用中后端失联 | 只有 launcher ready 和版本差异；需最新健康日志，不能据此证明具体根因。 |
| 15 | [#2399](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2399) | OPEN | 部分处理 | P1 | app-server 锚点漂移及 CPU 异常 | 3e85706f 在 v1.5.2 修发现；评论另报 Debugger.enable 超时，当前 5 秒单次流程仍在。 |
| 16 | [#2398](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2398) | OPEN | 咨询或需求 | P3 | 任务复杂度智能路由 | 新产品提案；现有 failover 不等价于决策路由，需独立设计与评估。 |
| 17 | [#2397](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2397) | CLOSED | 已修待验证 | P2 | 侧栏闪动 | 5bb4f636 在 v1.5.1；未见发帖人升级确认，不等于所有闪动都同源。 |
| 18 | [#2396](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2396) | CLOSED | 已修待验证 | P1 | 内存持续增长 | 相关负缓存与重注入修复已发布；需堆或长时采样确认内存问题消失。 |
| 19 | [#2395](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2395) | CLOSED | 咨询或需求 | P3 | Ubuntu 社区移植 | 平台范围讨论；末尾套用“v1.5.1 已修”模板与正文矛盾，需更正文案。 |
| 20 | [#2394](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2394) | OPEN | 已修待验证 | P2 | 手选启动路径未获优先使用 | 4704ac43 在 v1.5.2 改路径优先级；需多安装版本条件下复测。 |
| 21 | [#2393](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2393) | OPEN | 待定位 | P1 | 发送失败并要求沙盒更新 | 旧日志有 anchor_missing，但沙盒初始化是独立线索；补 1.6.0 原始错误。 |
| 22 | [#2392](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2392) | OPEN | 咨询或需求 | P2 | 官方与 API 切换后续聊 | 需区分历史不可见与已见会话不能继续；保留会话迁移产品需求。 |
| 23 | [#2391](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2391) | OPEN | 待修 | P1 | OpenCode 会话请求头缺失 | 转发接口未携带客户端会话头；静态自定义头仅绕过 400，未保留会话隔离。 |
| 24 | [#2390](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2390) | CLOSED | 待定位 | P1 | MiMo 切换改写认证后不可用 | 零评论关单；有官方认证可用的对照，但缺脱敏字段差异，不能判上游问题。 |
| 25 | [#2385](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2385) | CLOSED | 待定位 | P1 | 本地代理 502 | f55bb646 只增加诊断，不证明转发故障已修；更新错误日志后再定因。 |
| 26 | [#2384](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2384) | OPEN | 待定位 | P1 | 官方或混入登录 hit a snag | 用户已答复混入及官方均复现；检查恢复官方流程与登录态，不应继续等待旧问题答案。 |
| 27 | [#2380](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2380) | OPEN | 待定位 | P1 | 交互即卡死 | 重注入修复在 v1.5.1；缺新版本性能轨迹，不能把全部交互卡死认作同根因。 |
| 28 | [#2379](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2379) | CLOSED | 待定位 | P1 | 工具输出缺 call_id | 报错字符串不在本仓不构成排除证据；查代理前后 call_id/previous_response_id 和传输。 |
| 29 | [#2377](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2377) | CLOSED | 待定位 | P1 | 管理器重启失败 | 已合并 #2400；用户补 Native browser cleanup/guard 线索，需保持在主单跟踪。 |
| 30 | [#2376](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2376) | CLOSED | 待修 | P2 | 升级重新生成桌面图标 | be62bc16 仅跳过已存在图标；删除后仍会补建，修复承诺与实现相反。 |
| 31 | [#2375](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2375) | OPEN | 咨询或需求 | P3 | 官方 dots 与第三方模型协作 | 产品与认证边界需明确；现有回复对混入能力前后冲突，不宜给绝对承诺。 |
| 32 | [#2374](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2374) | CLOSED | 部分处理 | P2 | catalog 推理档位和编辑入口 | Sol 元数据已补；Luna 无 ultra，用户另提可视化自定义档位入口仍未完成。 |
| 33 | [#2373](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2373) | CLOSED | 部分处理 | P1 | CUA auth token 不可用 | 04ab6290 修 runtime 识别层并已发布；token 认证失败需按具体工具链诊断。 |
| 34 | [#2372](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2372) | CLOSED | 部分处理 | P1 | 插件安装失败 | 过滤器兼容已发布，但列表解锁与远程目录认证/安装失败不同，不能等同根治。 |
| 35 | [#2369](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2369) | CLOSED | 部分处理 | P2 | 纯 API Computer Use | runtime 识别兼容已发布；具体 CUA 路径与认证失败需分层复测。 |
| 36 | [#2367](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2367) | CLOSED | 已修待验证 | P1 | Claude 顶层组合 schema 被拒绝 | 09a9f1c5 在 v1.5.1 摊平组合器；布尔 enum 是另一问题。 |
| 37 | [#2364](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2364) | CLOSED | 已修待验证 | P2 | 插件列表为空 | 结构匹配过滤器随 v1.5.1；仍须区分网络和认证。 |
| 38 | [#2363](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2363) | CLOSED | 已修待验证 | P1 | 换皮肤后对话及输入框消失 | 8baffa91 在 v1.5.0；新 1.6.0 遮挡或错位不自动归为旧 top-fade。 |
| 39 | [#2362](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2362) | CLOSED | 已修待验证 | P1 | Windows 安装器不能覆盖管理器 | 8610d2f3 已在 v1.5.1；旧报告“尚未发布”已过期。 |
| 40 | [#2360](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2360) | CLOSED | 已修待验证 | P1 | remote compaction 0 output items | 549551d3 在 v1.5.0；修复事件序列，后续复现需保留 SSE。 |
| 41 | [#2359](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2359) | CLOSED | 已修待验证 | P2 | Codex++ 页面黑底黑字 | 43e6c55c 与 10ff2fe6 在 v1.5.1；浅深色切换复测即可。 |
| 42 | [#2358](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2358) | CLOSED | 已修待验证 | P2 | gpt-6.1-sol 档位缺失 | 35ede662 在 v1.5.0；切换供应商重新生成 catalog。 |
| 43 | [#2357](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2357) | CLOSED | 咨询或需求 | P2 | 中文界面依赖网络 | 作者确认开代理恢复；后来 Layer 补丁另已发布，不能由此否定离线缺陷。 |
| 44 | [#2355](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2355) | CLOSED | 已修待验证 | P2 | 模型排序 | ea82bc74 在 v1.5.1 支持模型行拖拽。 |
| 45 | [#2354](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2354) | CLOSED | 已修待验证 | P2 | Windows 深链协议未注册 | 当前 NSIS 与 Rust install 已注册 codexplusplus；需 Windows 安装后点击验证。 |
| 46 | [#2351](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2351) | CLOSED | 已修待验证 | P1 | 管理员运行导致无包身份 | 93a66453 在 v1.5.1 改明确失败；普通权限正常启动需现场确认。 |
| 47 | [#2350](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2350) | OPEN | 待定位 | P1 | 皮肤下聊天栏消失 | 关闭皮肤可恢复；还包含社区版本门限与白块问题，不能一并关为 top-fade 已修。 |
| 48 | [#2349](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2349) | OPEN | 待定位 | P1 | 微信收到消息但 Codex 处理失败 | CLI 选址及无窗口修复已发布；作者已测内置 CLI，不能直接断言 WindowsApps 必不可执行。 |
| 49 | [#2345](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2345) | CLOSED | 已修待验证 | P2 | 单模型图片策略未生效 | ecb71213 与 064775ab 在 v1.5.1 统一 slug key。 |
| 50 | [#2344](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2344) | CLOSED | 咨询或需求 | P2 | 读写权限设置 | 作者改标题已解决；权限配置咨询。 |
| 51 | [#2343](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2343) | CLOSED | 已修待验证 | P1 | Windows 找不到包身份 | AUMID 与提权防线已发布；需新版日志区分启动失败与后续代理 502。 |
| 52 | [#2341](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2341) | CLOSED | 已修待验证 | P1 | 压缩 SSE 缺 output_item.done | v1.5.0 已补生命周期事件，与 #2360 同组。 |
| 53 | [#2340](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2340) | OPEN | 部分处理 | P1 | 删除和归档失效 | 6ebfce05 在 v1.5.2 加删除墓碑；归档失败及实际症状仍待确认。 |
| 54 | [#2339](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2339) | CLOSED | 待修 | P2 | 社区主题版本校验失败 | 1.5.4 新评论已证仍失败；minClientVersion 与产品版本混用，不能只升级到 1.6.0。 |
| 55 | [#2338](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2338) | CLOSED | 已修待验证 | P1 | v1.4.0 无法压缩 | v1.5.0 已修 SSE 序列，同 #2341/#2360。 |
| 56 | [#2336](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2336) | CLOSED | 已修待验证 | P1 | 升级后仍报无程序包标识 | 动态 AUMID 加提权提示均已发布，用户有取消管理员后恢复的对照。 |
| 57 | [#2332](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2332) | CLOSED | 已修待验证 | P1 | Dream Skin top-fade 隐藏对话 | 8baffa91 在 v1.5.0，含用户 A/B 定位证据。 |
| 58 | [#2331](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2331) | CLOSED | 待定位 | P1 | 生成成功但图片未回传 | 关单称不生成 requires_openai_auth 与代码事实冲突；分查生成、下载和客户端鉴权。 |
| 59 | [#2330](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2330) | CLOSED | 已修待验证 | P1 | 每分钟重注入和 asset 重扫 | 5bb4f636 在 v1.5.1，旧报告发版提醒已满足。 |
| 60 | [#2329](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2329) | CLOSED | 部分处理 | P2 | 中文 Layer、注入时机和死端口 | Layer 已修在 v1.5.1；首读竞态与真实调试端口需独立跟踪。 |
| 61 | [#2328](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2328) | CLOSED | 咨询或需求 | P3 | 更新与启动咨询 | 已答复发版；当前最新为 v1.6.0。 |
| 62 | [#2327](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2327) | CLOSED | 已修待验证 | P1 | 持续 asset 重扫致卡顿 | bed5277d 在 v1.4.0；周期性重注入另由 v1.5.1 覆盖。 |
| 63 | [#2326](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2326) | CLOSED | 已修待验证 | P1 | tool_search 丢失 MCP 不可见 | 0fd58cc3 在 v1.4.0；端到端工具发现仍须复测。 |
| 64 | [#2325](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2325) | CLOSED | 待定位 | P2 | 中转缓存不命中 | 缺直连与代理对照；最后回复绝对归因上游过强，须查稳定会话头与请求前缀。 |
| 65 | [#2324](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2324) | CLOSED | 已修待验证 | P1 | 主题下对话不可见 | 8baffa91 在 v1.5.0，重复 #2332。 |
| 66 | [#2323](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2323) | CLOSED | 已修待验证 | P1 | 升级后主题隐藏对话 | 已证 top-fade 误伤并修复；不沿用评论里未证的死循环解释。 |
| 67 | [#2322](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2322) | CLOSED | 待定位 | P1 | 原地升级第三方模式无限转圈 | 重装同版本恢复提示残留状态；不能仅靠重注入修复宣称升级流程已修。 |
| 68 | [#2321](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2321) | CLOSED | 咨询或需求 | P1 | 关闭皮肤恢复工作区 | 作者确认绕过；历史症状同 #2332，保留关闭但不能将绕过说成修复。 |
| 69 | [#2315](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2315) | OPEN | 待定位 | P2 | DeepSeek 模型名、窗口和图片异常 | 预设两条 v4 模型；需完整 slug 和 catalog，258K 提示模板回落但未证明原因。 |
| 70 | [#2314](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2314) | CLOSED | 已修待验证 | P1 | 管理器找不到应用 | 路径识别、单实例和 AUMID 修复已发布；无升级复测结果。 |
| 71 | [#2312](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2312) | OPEN | 咨询或需求 | P3 | 多来源统一模型工作流 | 既有路由可用但产品入口仍有缺口；界面简化不等于全案完成。 |
| 72 | [#2310](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2310) | CLOSED | 已修待验证 | P1 | AUMID 变化无法启动 | 2b4d8d68 在 v1.4.0；重复 #2308。 |
| 73 | [#2308](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2308) | CLOSED | 已修待验证 | P1 | 官方更新后无包身份 | v1.4.0 读取 manifest，当前再失败需区分提权和包注册。 |
| 74 | [#2306](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2306) | CLOSED | 已修待验证 | P1 | 显式路径可启而管理器失败 | v1.4.0 AUMID 及 v1.5.2 路径优先修复已发布，差异需新版复验。 |
| 75 | [#2304](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2304) | CLOSED | 已修待验证 | P2 | macOS Dock 无法唤回窗口 | e5cd4de7 在 v1.5.1 接 Reopen。 |
| 76 | [#2302](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2302) | CLOSED | 已修待验证 | P1 | 1M 被剥成 1 token | b6e992b3 在 v1.5.1 解析 K/M；与本 fork 核心功能直接相关。 |
| 77 | [#2300](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2300) | CLOSED | 咨询或需求 | P2 | 推理过程进正文 | 缺原始 reasoning/content 对照；咨询可关闭，归因不宜写死。 |
| 78 | [#2299](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2299) | CLOSED | 已修待验证 | P1 | 250ms 重扫绕过熔断 | 用户确认 v1.4.0 主问题已修；另一个重复解析问题由 #2330 跟踪。 |
| 79 | [#2298](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2298) | CLOSED | 咨询或需求 | P2 | 启动等十几分钟后恢复 | 作者自行恢复；冷启动过慢仍未定因，复现时转启动诊断。 |
| 80 | [#2297](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2297) | OPEN | 待定位 | P2 | 市场汉化脚本无法加载 | 作者已提供第三方市场和脚本描述；应转脚本兼容性，主仓不直接承担脚本实现。 |
| 81 | [#2295](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2295) | CLOSED | 咨询或需求 | P2 | 官方额度耗尽时发送灰色 | 作者接受额度门控解释；若纯 API 仍受官方 gate 需另给受控证据。 |
| 82 | [#2294](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2294) | CLOSED | 部分处理 | P1 | 浏览器拒绝 API key 身份 | 04ab6290 在 v1.5.1；native browser 兼容与 CUA auth token 是不同层。 |
| 83 | [#2289](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2289) | CLOSED | 咨询或需求 | P2 | GPT 与 DeepSeek 同列表切换 | 模型路由已有；保存失败须以具体配置另定位。 |
| 84 | [#2287](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2287) | CLOSED | 部分处理 | P2 | 第三方模型 Computer Use | 并入 #2294/#2369；不能由 runtime 识别修复承诺全部模式可用。 |
| 85 | [#2284](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2284) | CLOSED | 已修待验证 | P2 | gpt-6-sol 和 luna 支持 | 7e19e069 在 v1.4.0 兼容元数据。 |
| 86 | [#2279](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2279) | CLOSED | 已修待验证 | P2 | 第三方模型元数据缺失 | 当前查找链已有供应商目录；不能把所有任意别名或新模型都视作已覆盖。 |
| 87 | [#2275](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2275) | CLOSED | 已修待验证 | P1 | 图片通知拆断工具结果配对 | a8dcd29f 在 v1.5.1 重排非 tool 消息并处理孤立结果。 |
| 88 | [#2273](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2273) | CLOSED | 待定位 | P1 | 同 Key 在 CCS 可用而本仓 401 | 有可用性对照但缺最终请求头；不能只凭服务端错误排除本仓。 |
| 89 | [#2270](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2270) | CLOSED | 待定位 | P2 | 切换账号或 API 后历史不可见 | 身份隔离合理；跨 API 同步与切回后是否恢复仍未证，关单归因过强。 |
| 90 | [#2269](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2269) | CLOSED | 咨询或需求 | P1 | Windows 控制策略阻断 | os error 4551 为环境阻断；需平台策略或签名排查，非协议代理修复。 |
| 91 | [#2268](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2268) | CLOSED | 咨询或需求 | P3 | Workbuddy 功能 | 其他产品的功能请求，超出仓库范围。 |
| 92 | [#2267](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2267) | CLOSED | 已修待验证 | P1 | 使用一段时间后卡顿 | 5bb4f636 实际在 v1.5.1；旧回帖已更正 v1.5.0 说法。 |
| 93 | [#2266](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2266) | CLOSED | 已修待验证 | P1 | 输入逐字卡顿 | 相关重注入修复在 v1.5.1；需新版交互性能复测。 |
| 94 | [#2264](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2264) | CLOSED | 已修待验证 | P1 | 目标任务恢复后被换模型 | 相关修复在 v1.4.0；仍须覆盖目标模式恢复及计费模型，不能只验证新会话。 |
| 95 | [#2263](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2263) | CLOSED | 已修待验证 | P1 | chat 路径丢 tool_search | 0fd58cc3 在 v1.4.0；重复 #2326。 |
| 96 | [#2261](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2261) | CLOSED | 已修待验证 | P2 | 原生菜单中文而正文英文 | Layer 修复已发布；最后“非本仓问题”模板应改成已修与复测说明。 |
| 97 | [#2260](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2260) | CLOSED | 待定位 | P1 | 终端与控制软件能力消失 | 只有诊断猜测、缺真实 CLI 路径和失败栈；catalog 指针正常不等于能力故障已排除。 |
| 98 | [#2259](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2259) | OPEN | 部分处理 | P2 | 市场卸载与全局启用 | 7a17b1c1 在 v1.5.2 加市场卸载；管理器全局开关入口仍未完成。 |
| 99 | [#2258](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2258) | CLOSED | 已修待验证 | P2 | 容器改名导致居中宽度失效 | c8f8ee30 在 v1.5.1；新布局回归另跟 #2408/#2414。 |
| 100 | [#2257](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2257) | CLOSED | 已修待验证 | P1 | 并行 view_image 出孤立 tool 结果 | a8dcd29f 在 v1.5.1，重复 #2275。 |

## 建议处理顺序

1. 修 #2410 的签名破坏路径；对 #2402 和 #2407 保持独立 P1 调查，不与锚点修复一起关。
2. 将 #2414/#2413/#2408 作为新布局兼容组复现；临时关闭居中或恢复默认皮肤可帮助缩小范围。
3. 重新跟踪 #2339、#2376；同时更正已关闭 issue 中与代码冲突的解释。
4. 修 #2391 的会话头透传，调查 #2404 的 schema 兼容；按现有 opt-in 与旧行为兼容要求设计。
5. 对 #2403/#2394 获取已发布版本复测；对 #2340/#2259/#2374 保留尚未完成的子诉求。
6. 本 fork 回归重点：K/M 输入、模型 slug 归一化、推理档位、图片策略、切模型 thread/resume，以及目标任务恢复后继续使用原模型。

本次交付是审核报告，没有实现修复、运行跨平台动态测试、修改既有报告或执行 GitHub 关单。需要修复时，按行为添加必要的 cargo test 回归，修 renderer 分片后重新组装产物。
