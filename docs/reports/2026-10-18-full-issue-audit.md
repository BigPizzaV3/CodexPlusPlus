# CodexPlusPlus 全量 Issue 审核与可修复清单

日期：2026-10-18 12:37:57 UTC+08:00。

后续代码实施与验收范围见[本轮修复结果](2026-10-18-issue-fix-results.md)。本文件保留审核时的快照。

全库索引 2060 条 issue；逐项审核本轮开放范围 720 条，其中初始快照 719 条，执行前新增 #2592 一并纳入。参照 13 个开放 PR。已关闭的历史 issue 只核对索引、重复与修复关联；未重新打开或宣称逐条重审其全部正文。

结论：已发布的问题、明确重复项和已答复咨询按具体范围结案；尚无根因的启动、Work/插件认证、浏览器、会话可见性报告继续保留。剩余可开工工作归为 25 组：10 组缺陷/兼容/交互改进、15 组功能；部分协议与原生接口改动必须先补契约验收。

## 代码与发布基线

- 正式版：v1.10.0，北京时间 2026-10-17 23:53:59 发布，远端 main/tag 为 `85f9117f`。
- 本地审核：`7211c146`；比发布点多一笔管理器更新说明渲染提交。本次未修改产品源代码、合并 PR 或发布安装包。
- 已获取开放 issue 的正文及全部评论，逐项结合已有诊断复核；对没有新增证据的条目沿用有效诊断，针对发布修复与可修复组回查当前实现。下载的原始正文/评论仅作为本地临时材料，不进入仓库报告。

## 实际执行结果

| 项目 | 数量 |
| --- | ---: |
| 本轮已发回复 | 150 |
| 其中回复后关闭 | 20 |
| 回复后继续开放 | 130 |
| 保留已有有效诊断并未重复留言 | 570 |
| 开放范围内剩余开放 | 700 |
| 尚未完成或并发更新需复核 | 0 |

87 条此前没有维护者回复的条目已纳入本轮答复（初始 86 条，加新 #2592）。4 条已经回答完整的使用咨询按原题范围关闭。回复写入前逐项校验 state/updated_at，避免覆盖新评论；每条回复有本轮幂等标记。关闭原因区分 completed 与 duplicate/not_planned。没有仅因为版本旧、缺日志或发布了新版本就关闭。

远端最终复核：20 条目标均为 CLOSED；全库剩余 700 条开放 issue、1360 条已关闭 issue，另有 13 个开放 PR。

### 已关闭条目

| Issue | 关闭依据 | 回复 |
| --- | --- | --- |
| [#2587](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2587) | 已确认你报告的是 v1.10.0 接管后白色任务栏图标，与 #2499 的现象和启动入口对照一致。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2587#issuecomment-6245627126) |
| [#2580](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2580) | 更正上一条诊断后的发布状态：目录构建现已为无权威能力声明的模型补布尔 supports_parallel_tool_calls=false，并在应用全部元数据后校验最终目录。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2580#issuecomment-6245670920) |
| [#2579](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2579) | 两层已同时修复：模型选择器按权威 groupId 全局合组，非相邻同成员可合并、同名不同成员不会误合；保存活动聚合的成员模型时会重建并发布活动目录。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2579#issuecomment-6245671669) |
| [#2567](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2567) | 两条用量读取现已按目标 thread 遍历候选库，不再被空主库或含其他会话的主库遮住；坏 schema、空 rollout 路径、缺文件等候选也有回退/错误区分。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2567#issuecomment-6245672370) |
| [#2548](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2548) | source_app 与 MSIX 分类现已统一：同时识别 Claude/AnthropicPBC.Claude 包内 app\Claude.exe 与已注册 execution alias，并排除普通路径/相似包名。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2548#issuecomment-6245680633) |
| [#2541](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2541) | 更正兼容级别仍为 1.5.12 的旧答复：DreamSkin v1.5.20 资源及配套桥接已同步，客户端包兼容常量现为 1.5.20；原报告的 1.5.18/1.5.19 门限已在支持范围。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2541#issuecomment-6245673749) |
| [#2539](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2539) | 未安装 Claude 时的同步探测已改为手动、后台执行与超时回收，避免仅进入页面就卡死。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2539#issuecomment-6245681297) |
| [#2492](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2492) | 普通生成路径已补 base_instructions；此前补充的 metadata 将它覆盖成 null 的后门也已封住：前端校验关键类型，后端在最终合成目录发布前再次校验，失败保留原目录。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2492#issuecomment-6245673026) |
| [#2471](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2471) | “强制中文界面”已经恢复，入口在 Codex 增强 → 常用增强，默认关闭。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2471#issuecomment-6245678994) |
| [#2456](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2456) | “强制中文界面”已经恢复，入口在 Codex 增强 → 常用增强，默认关闭。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2456#issuecomment-6245678238) |
| [#2339](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2339) | 更正兼容级别仍为 1.5.12 的旧答复：DreamSkin v1.5.20 资源及配套桥接已同步，客户端包兼容常量现为 1.5.20；原报告的 1.5.18/1.5.19 门限已在支持范围。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2339#issuecomment-6245674466) |
| [#1355](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1355) | 几百 KB 的图标是快捷方式，删除它不会卸载应用。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1355#issuecomment-6245715794) |
| [#1341](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1341) | 会话列表读取已移到后台执行，缓存索引并只读取当前页详情；数据库/WAL 变化会让缓存失效。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1341#issuecomment-6245675182) |
| [#1294](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1294) | 现在已有“Codex 多开”管理入口：把相同模型配置在两个不同单一供应商下，可分别启动并行实例，使用各自的 CODEX_HOME、会话/SQLite、用户数据目录和 helper/CDP 端口。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1294#issuecomment-6245676756) |
| [#1214](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1214) | 本条主要诉求已有“模型聚合”路径覆盖：创建一个聚合配置，加入官方与第三方成员，始终使用同一个聚合 provider，再在原生模型选择器切换成员模型；不需要每次换全局供应商、重启或创建新 thread。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1214#issuecomment-6245677513) |
| [#1136](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1136) | 现在已有“Codex 多开”管理入口：把相同模型配置在两个不同单一供应商下，可分别启动并行实例，使用各自的 CODEX_HOME、会话/SQLite、用户数据目录和 helper/CDP 端口。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1136#issuecomment-6245675942) |
| [#1093](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1093) | http://127.0.0.1:57321/v1 是本地模型 API 前缀，不提供网页首页；浏览器对 /v1 发 GET 打不开是正常的，不能据此判断没有实现路由。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1093#issuecomment-6245715101) |
| [#346](https://github.com/BigPizzaV3/CodexPlusPlus/issues/346) | 可以绕过 ccx，直接接 llama.cpp 的 OpenAI 兼容服务：在 Codex++ 新建自定义供应商，选纯 API、Chat Completions，上游填你示例服务的 http://127.0.0.1:7080/v1，并手工填写该服务实际发布的模型 ID；服务不鉴权时使用对应的无鉴权设置。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/346#issuecomment-6245714329) |
| [#322](https://github.com/BigPizzaV3/CodexPlusPlus/issues/322) | 图片先转文字的 VLM 路径已提供，并修复了之前补充的失败策略缺口：视觉服务失败或未正确配置时会明确返回错误，不再把未识别的图片继续发给纯文本主模型；成功时发送文字描述。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/322#issuecomment-6245679685) |
| [#66](https://github.com/BigPizzaV3/CodexPlusPlus/issues/66) | 独立启动图标已提供：Windows 正式安装版创建桌面与开始菜单的 Codex++ 入口，可右键固定到任务栏；macOS 将正式 DMG 中的 Codex++.app 安装到 Applications，再拖到 Dock 保留。 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/66#issuecomment-6245716570) |

### 局部已修但继续开放

数据安全 #144/#240/#606、历史备份保留 #2544、会话修复入口 #2521、多开资源/身份 #2523/#2491、Windows Claude #2517/#2556、视觉主题 #2562、legacy 压缩 #2532、推理/搜索/附件 #2446/#2464/#1297 等已补发布状态。关闭一个已定位分支不会被用来替代整个现场验收。

## 可修复和可开发清单

按 P1 → P2 → P3 排序。工作量仅表示相对规模。缺陷组的实施范围以本栏验收条件为准；功能组默认 opt-in，沿用已有 profile 行为。

### 1 P1 WSL 模型目录路径按实际运行环境生成

- Issue：[#2591](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2591)。类型：缺陷；规模：中。
- 当前依据：model_catalog_json 相对指针被宿主 absolute 迁移；原生 Windows 与 WSL 需要不同解析策略。
- 修改位置：[crates/codex-plus-core/src/relay_config.rs:3096](../../crates/codex-plus-core/src/relay_config.rs#L3096)。
- 完成条件：保留/生成运行环境可读的目录指针；覆盖原生 Windows、WSL 共享 home、外部目录、保存与重启。

### 2 P1 Windows 窗口图标与任务栏身份分别判断成功

- Issue：[#2499](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2499)。类型：缺陷；规模：中。
- 当前依据：当前两个步骤任一成功即返回 true，窗口图标失败可能过早停止重试；v1.10.0 的重复报告 #2587 已并入。
- 修改位置：[crates/codex-plus-core/src/windows_integration.rs:1143](../../crates/codex-plus-core/src/windows_integration.rs#L1143)。
- 完成条件：分别记录图标/任务栏结果，对失败步骤有界重试；同 HWND/AUMID 对照 Store 26.1013.4894.0。

### 3 P1 明确区分继承思考档位与不发送思考参数

- Issue：[#2549](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2549)。类型：缺陷；规模：中。
- 当前依据：清空 supported_reasoning_levels 会删除覆盖、恢复内置档位；Chat 方言仍按模型名选择。
- 修改位置：[apps/codex-plus-manager/src/model-reasoning.ts:116](../../apps/codex-plus-manager/src/model-reasoning.ts#L116)。
- 完成条件：新增 opt-in 的继承/自定义/不发送语义；保存、目录与真实请求一致，不把厂商不支持的档位硬映射为另一档。

### 4 P1 保留上游显式 Lite 能力并提供清晰配置入口

- Issue：[#2554](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2554)。类型：兼容；规模：中。
- 当前依据：导入目录中的 true 仍可能被上游安全默认覆盖；已有逐模型 JSON 覆盖可临时使用。
- 修改位置：[crates/codex-plus-core/src/relay_config.rs:3924](../../crates/codex-plus-core/src/relay_config.rs#L3924)。
- 完成条件：记录能力来源，显式用户覆盖优先；覆盖导入、重建、聚合、多开，默认不为所有供应商开启 Lite。

### 5 P1 严格 Responses 历史的推理与加密内容兼容

- Issue：[#1851](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1851)、[#2067](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2067)、[#2047](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2047)。类型：兼容；规模：中至大。
- 当前依据：新评论提供 reasoning_text 与 encrypted_content 同时存在的失败结构；不同账号密文还可能无法解密。
- 修改位置：[crates/codex-plus-core/src/protocol_proxy.rs](../../crates/codex-plus-core/src/protocol_proxy.rs)。
- 完成条件：先明确目标契约，再做受控投影或明确拒绝；不改原 rollout、不见密文就删明文；补跨账号与占位密文负例。

### 6 P1 用户脚本安全启动与失控后的恢复

- Issue：[#2585](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2585)。类型：功能；规模：大。
- 当前依据：第三方 MutationObserver 自写可锁死 renderer；同线程 try/catch/定时器不能终止这种循环。
- 修改位置：[apps/codex-plus-manager/src-tauri/src/commands.rs](../../apps/codex-plus-manager/src-tauri/src/commands.rs)。
- 完成条件：独立管理器提供下一次启动跳过脚本；由外部进程判定启动健康并提示隔离；保留脚本及可恢复配置。

### 7 P2 获取模型与聊天共用兼容端点规则

- Issue：[#2506](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2506)。类型：缺陷；规模：小。
- 当前依据：models_endpoint 只识别末段版本，/v1beta/openai 后仍会追加 /v1/models；聊天已有保留路径的构造。
- 修改位置：[crates/codex-plus-core/src/model_catalog.rs:968](../../crates/codex-plus-core/src/model_catalog.rs#L968)。
- 完成条件：统一 endpoint 构造，回环断言 /v1beta/openai/models、完整 /models、常见 /v1 和自定义版本路径。

### 8 P2 语言切换保留当前工具与草稿

- Issue：[#2559](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2559)。类型：交互；规模：中。
- 当前依据：setLanguage 仍整窗 reload，工具栏先按默认 Codex 重建。
- 修改位置：[apps/codex-plus-manager/src/i18n.ts:70](../../apps/codex-plus-manager/src/i18n.ts#L70)。
- 完成条件：优先原地切语言；必须重载时先恢复选中工具、页签与未保存草稿，避免回跳/图标空窗。

### 9 P2 会话多选支持取消选择并澄清页码

- Issue：[#2559](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2559)。类型：交互；规模：小。
- 当前依据：实际 SessionPage 只有全选当前列表；1 · 14 混合页码与会话总数。
- 修改位置：[apps/codex-plus-manager/src/ManagementExtra.tsx:245](../../apps/codex-plus-manager/src/ManagementExtra.tsx#L245)。
- 完成条件：增加取消本页/全部已选，明确当前页/总会话数；搜索、翻页和删除失败后选择状态一致。

### 10 P2 MCP 启动入口识别来源并按需迁移失效路径

- Issue：[#2583](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2583)。类型：兼容；规模：中。
- 当前依据：任意 MCP command 按原值存储；微信 CLI 路径恢复不覆盖 MCP；桌面 ChatGPT.exe 也不等于 stdio MCP 入口。
- 修改位置：[crates/codex-plus-core/src/mcp_config.rs](../../crates/codex-plus-core/src/mcp_config.rs)。
- 完成条件：记录来源，按实际 runtime manifest 定位入口，限时验证 initialize/tools/list 后更新；不自动重启正在工作服务器。

### 11 P2 Windows Claude 脚本子进程隐藏与副本校验

- Issue：[#2556](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2556)、[#2517](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2517)、[#2560](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2560)。类型：缺陷；规模：小至中。
- 当前依据：探测使用隐藏标志，但 run_script 仍直接创建 PowerShell；副本校验还依赖 Get-FileHash。PR #2522 提供候选，未合并。
- 修改位置：[crates/codex-plus-core/src/claude_localization.rs:588](../../crates/codex-plus-core/src/claude_localization.rs#L588)。
- 完成条件：隐藏 PowerShell 父进程，保留 Claude 窗口与注入；校验改为可靠 SHA-256 实现，验证 hash 不符及失败回滚，不给 Claude 加 WindowStyle Hidden。

### 12 P2 开机静默与用户主动启动分别处理

- Issue：[#2572](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2572)、[#1697](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1697)、[#2494](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2494)、[#209](https://github.com/BigPizzaV3/CodexPlusPlus/issues/209)。类型：功能；规模：小至中。
- 当前依据：--autostart 没有让主窗口默认隐藏；自动接管和打开管理器后启动 Codex 是不同动作。
- 修改位置：[apps/codex-plus-manager/src-tauri/src/lib.rs](../../apps/codex-plus-manager/src-tauri/src/lib.rs)。
- 完成条件：默认关闭的开机静默选项，托盘可恢复；主动打开显示窗口，避免重复启动。

### 13 P2 全局听写快捷键

- Issue：[#2427](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2427)。类型：功能；规模：中。
- 当前依据：现有网页按键不覆盖失焦窗口；需求已收敛为 Windows 优先、一次开始/再按停止。
- 修改位置：[assets/inject/renderer-inject/93-dictation.js](../../assets/inject/renderer-inject/93-dictation.js)。
- 完成条件：原生注册可配置键位，报告冲突；目标会话绑定、权限等待、导航、禁用和退出均可取消/释放。

### 14 P2 Skills 分组折叠与批量启停

- Issue：[#2528](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2528)。类型：功能；规模：中。
- 当前依据：当前 Skills 界面优化未完整实现自定义分组及组级操作。
- 修改位置：[apps/codex-plus-manager/src](../../apps/codex-plus-manager/src)。
- 完成条件：先按来源折叠并记忆偏好，再做自定义组；折叠不改变启用状态，批量失败逐项显示。

### 15 P2 供应商独立网络出口

- Issue：[#1557](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1557)、[#2135](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2135)、[#897](https://github.com/BigPizzaV3/CodexPlusPlus/issues/897)、[#34](https://github.com/BigPizzaV3/CodexPlusPlus/issues/34)、[#317](https://github.com/BigPizzaV3/CodexPlusPlus/issues/317)。类型：功能；规模：中。
- 当前依据：公共 HTTP client 仍基于系统/环境代理，没有每供应商独立代理字段。
- 修改位置：[crates/codex-plus-core/src/http_client.rs:1](../../crates/codex-plus-core/src/http_client.rs#L1)。
- 完成条件：opt-in 代理贯通测试、模型拉取、聊天和聚合成员；loopback 直连，旧配置默认不变。

### 16 P2 供应商显式 CA 配置

- Issue：[#1663](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1663)。类型：功能；规模：中。
- 当前依据：当前缺少显式 CA PEM/CODEX_CA_CERTIFICATE 对应入口；系统信任链失败本身仍需独立定位。
- 修改位置：[crates/codex-plus-core/src/http_client.rs](../../crates/codex-plus-core/src/http_client.rs)。
- 完成条件：只给选定供应商添加 CA，校验 PEM，测试/真实请求一致；不通过关闭 TLS 校验绕过。

### 17 P2 可配置且有界的 helper 请求体上限

- Issue：[#2198](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2198)。类型：功能；规模：中。
- 当前依据：解码 32 MiB、编码 64 MiB 仍是常量。
- 修改位置：[crates/codex-plus-core/src/launcher.rs:3138](../../crates/codex-plus-core/src/launcher.rs#L3138)。
- 完成条件：opt-in 配置与最大边界，Content-Length/chunked/gzip 使用同一预算，超限明确拒绝。

### 18 P2 备份与旧更新包保留策略

- Issue：[#2544](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2544)。类型：功能；规模：中。
- 当前依据：流式备份已发布，但历史 GB 备份及多版本安装包没有自动保留策略。
- 修改位置：[crates/codex-plus-data/src/backup.rs](../../crates/codex-plus-data/src/backup.rs)。
- 完成条件：扫描大小/时间，预览后选择清理，保护撤销/恢复依赖；本轮没有删除任何历史备份。

### 19 P2 跨页项目级会话批量操作

- Issue：[#1752](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1752)、[#141](https://github.com/BigPizzaV3/CodexPlusPlus/issues/141)。类型：功能；规模：中。
- 当前依据：当前项目分组只基于已加载页，“全选当前列表”不等于全项目。
- 修改位置：[apps/codex-plus-manager/src/ManagementExtra.tsx:245](../../apps/codex-plus-manager/src/ManagementExtra.tsx#L245)。
- 完成条件：后端按稳定项目标识查询全部目标，预览数量，允许取消；删除保留现有备份与部分失败语义。

### 20 P2 批量移动与项目目录重定位

- Issue：[#1085](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1085)、[#497](https://github.com/BigPizzaV3/CodexPlusPlus/issues/497)、[#187](https://github.com/BigPizzaV3/CodexPlusPlus/issues/187)、[#1316](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1316)。类型：功能；规模：大。
- 当前依据：原生归属、cwd 与列表刷新需要一致，现有批量删除不是批量移动。
- 修改位置：[crates/codex-plus-data/src/storage.rs](../../crates/codex-plus-data/src/storage.rs)。
- 完成条件：先实现只读预览和可信来源筛选，再原子更新受支持元数据；不伪造不支持的远程来源。

### 21 P2 按轮次与供应商统计用量

- Issue：[#2557](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2557)、[#2143](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2143)、[#2053](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2053)、[#1028](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1028)、[#1427](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1427)。类型：功能；规模：中。
- 当前依据：现有最近调用比率/本机用量不能替代整轮加权统计和各供应商累计账单。
- 修改位置：[crates/codex-plus-data/src/storage.rs](../../crates/codex-plus-data/src/storage.rs)。
- 完成条件：按实际路由身份归属并去重；整轮 cached 总和/input 总和，区别原始用量与平台费用校准。

### 22 P2 点击原生会话后退出内嵌功能页

- Issue：[#1982](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1982)。类型：功能；规模：小至中。
- 当前依据：已有原生返回按钮修复不覆盖任意会话导航自动关闭功能页。
- 修改位置：[assets/inject/renderer-inject/40-backend-settings.js](../../assets/inject/renderer-inject/40-backend-settings.js)。
- 完成条件：可信 thread 导航后关闭面板、恢复宿主；保留原生点击与返回历史，重复注入不重复监听。

### 23 P3 手动刷新 UI 远程开关

- Issue：[#2551](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2551)。类型：功能；规模：中至大。
- 当前依据：强制中文本地兼容不等于真正刷新 Statsig；报告者明确不需要后台自动检测。
- 修改位置：[assets/inject/renderer-inject](../../assets/inject/renderer-inject)。
- 完成条件：先验证当前客户端可用刷新 API 与订阅通知，再实现仅手动触发、异步超时，区分未获取/false/true。

### 24 P3 特定历史注入块的前缀稳定实验

- Issue：[#2448](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2448)。类型：功能；规模：大。
- 当前依据：旧 PR #2449 已关闭，主线没有该稳定层；每轮前缀变化已有用户对照。
- 修改位置：[crates/codex-plus-core/src/protocol_proxy.rs](../../crates/codex-plus-core/src/protocol_proxy.rs)。
- 完成条件：先只做脱敏结构/前缀诊断，再以 opt-in 白名单处理明确 notice；不重排系统指令或附件正文。

### 25 P3 Claude Desktop 供应商管理

- Issue：[#2498](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2498)、[#363](https://github.com/BigPizzaV3/CodexPlusPlus/issues/363)。类型：功能；规模：大。
- 当前依据：当前 Claude 增强提供副本/汉化/语音；Codex 的 Anthropic 上游不是 Claude Desktop 供应商接入。
- 修改位置：[crates/codex-plus-core/src/claude_localization.rs](../../crates/codex-plus-core/src/claude_localization.rs)。
- 完成条件：先确认原生部署/模型映射契约，再做独立保存切换及回滚；不把页面出现当作请求接通。

## 仍需取证或原生验收的重点

| Issue | 继续开放的原因 | 下一步 |
| --- | --- | --- |
| [#2590](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2590) | 新版仍回退 272k，现有日志主要为正常轮询 | 对照实际模型与最终目录条目、config/read 首错、是否 WSL；不先改窗口猜测根因 |
| [#2592](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2592) | Chrome 扩展菜单与桌面模型菜单的列表来源未确认 | 补扩展 ID/版本及桌面同模型对照，再查扩展列表来源 |
| [#2582](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2582)、[#2584](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2584)、[#2407](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2407) | Work/工作区认证尚未建立同条件归因 | 官方直启与增强启动同账号/模式对照；绕过成功不等于全体已修 |
| [#2574](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2574)、[#2573](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2573)、[#1677](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1677) | helper 退出、辅助窗口可见、官方 runtime 初始化可能不同根因 | 对齐进程身份/实际端口/退出事件，Windows 实机验收 |
| [#2556](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2556)、[#2517](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2517)、[#2560](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2560)、[#2564](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2564) | 包发现已修，Windows prepare/check/launch 仍需实测 | 可先审 PR #2522 的脚本子进程隐藏和副本 hash，再跑真实商店包流程 |
| [#2486](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2486)、[#2451](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2451)、[#2421](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2421)、[#2446](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2446) | 空搜索完成/保护不可用不等于一个已修 Chat 映射问题 | 最小脱敏 wire 结构、实际上游协议和终止状态；不伪造成功或重放已执行工具 |
| [#1980](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1980)、[#1543](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1543)、[#2134](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2134)、[#368](https://github.com/BigPizzaV3/CodexPlusPlus/issues/368) | 文件存在、侧栏可见和助手/工具消息投影是不同层 | 人工会话复现完整元数据，不凭索引修复通过宣布正文恢复 |
| [#1050](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1050)、[#805](https://github.com/BigPizzaV3/CodexPlusPlus/issues/805)、[#1231](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1231) | 当前合法分片往返对照不证明原坏参数由转换器制造 | 同一个调用的入站/出站 arguments 分片与终态，先定位再修 |

## 开放 PR 的使用方式

| PR | 与本轮清单的关系 |
| --- | --- |
| #2522 | Windows Claude 脚本子进程隐藏、.NET hash 校验；候选修复，尚未合并。 |
| #2561 | 聚合热刷新与保存后延后重启；原生能力不能保证热加载，不能把它当作窗口已实时更新。 |
| #2568 | 用量候选库查找已有主线等价实现并通过回归；本轮关闭 #2567，不修改 PR 状态。 |
| #2563 | 主题遮挡已有主线等价实现；真实视觉验收仍保留在 #2562，不修改 PR 状态。 |
| #2483 | apply_patch 包装与 SSE 候选修复，需要逐事件/工具副作用回归，未据其描述直接认定可合并。 |
| #2504 | CC Switch 外部接管的诊断增强；既有拒绝保护仍要保留，不能静默采纳另一供应商凭据。 |
| #2386 | Linux 构建/打包候选；不等同主仓已发布 Linux 安装包。 |

## 本地验证与交付

- 通过 `cargo test` 重跑目录/模型分组、协议安全、数据同步与撤销、候选库、分页缓存、Claude MSIX、主题包、launcher 和 UI 回归：421 passed，4 ignored。`ui_issue_regressions` 内含仓库已有前端测试及 TypeScript 检查。跳过项包括原生浏览器环境依赖，未冒称 Windows 真机验收。
- 按仓库要求运行 `node scripts/relaunch-local.mjs`，完成前端与 Rust release 构建、签名、固定 bundle 替换和启动；`codesign --verify --deep --strict` 通过，已核对固定路径实例在运行。历史构建清理脚本报告无需清理。
- 固定实例：`dist/local-build/Codex++.app`。本轮仅增加审核报告，不实施上述待修产品代码。

## 全量明细

- [逐 issue 审核清单](2026-10-18-full-issue-inventory.md)：覆盖全部本轮开放范围，逐条链接到处理或既有诊断。
- [结构化审核与实际回复记录](2026-10-18-full-issue-audit.json)：审核范围、25 组清单、逐条结论、本轮发出的确切文本与回执。
