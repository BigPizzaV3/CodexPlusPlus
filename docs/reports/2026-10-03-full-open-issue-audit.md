# CodexPlusPlus 全量 open issue 审计报告

- 日期：2026-10-03
- 仓库：`/Users/mac/Desktop/CodexPlusPlus`，基线 `main` @ `f55bb646`
- 范围：**全部 1041 条 open issue**（不含 6 条 open PR）
- 方法：30 批并行审计 agent，每条回代码核实；本地信号（git 提交交叉引用、标题归一化、评论分布）预筛
- 与既有报告的关系：`docs/reports/2026-10-02-issue-audit-and-fix-plan.md` 覆盖 116 条、`2026-10-03-issue-audit-latest-100.md` 覆盖 100 条，去重后已覆盖 83 条；**本报告覆盖其余 958 条**

---

## 1. 结论分布

| verdict | 条数 | 含义 |
|---|---:|---|
| `already-fixed-released` | 106 | 已在正式版（v1.5.0 及之前）修复，用户升级即可 |
| `already-fixed-unreleased` | 33 | 已在 main 修复但晚于 v1.5.0，**用户拿不到，需要发版** |
| `confirmed-in-code` | 44 | 已在代码中定位到确定缺陷 |
| `likely-in-code` | 30 | 强指向本仓缺陷，缺最后一环现场证据 |
| `not-our-bug` | 175 | 上游 Codex / 第三方供应商 / 用户环境 / 产品范围外 |
| `duplicate` | 83 | 与已有 issue 重复 |
| `invalid-insufficient-info` | 76 | 正文不足以定位，需回帖补证据 |
| `question-answerable` | 31 | 使用咨询，可直接回答 |
| `feature-request` | 130 | 新功能诉求 |
| `needs-investigation` | 250 | 证据不足，需进一步排查 |
| **合计** | **958** | |

## 3. 可修复清单（待你决定是否动手）

共 **74** 条。置信度 `H`=高 / `M`=中 / `L`=低；`工作量` 为修复规模估计。

按优先级：P1 **23** 条、P2 **45** 条、P3 **6** 条。

### P1

| 置信 | issue | 主题 | 修复位置 | 工作量 |
|---|---|---|---|---|
| H | [#1931](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1931) | 粘贴修复开启后大文本不转文件导致卡死 | assets/inject/renderer-inject/zz-paste-fix.js:20 —— 增加长度阈值（超过 N 字符时不拦截，交给 Code | small |
| H | [#1209](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1209) | [Feature Request] web_search_call 被错译为 custom_ | crates/codex-plus-core/src/protocol_proxy.rs:5083 的 tool_call_added_item（以及对应的 | medium |
| H | [#1012](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1012) | [Bug]: gemini3.5flash 错误（thought_signature） | crates/codex-plus-core/src/protocol_proxy.rs：在把上游 response 转回 Codex 格式时保留 Gemi | medium |
| H | [#332](https://github.com/BigPizzaV3/CodexPlusPlus/issues/332) | gemini-3.5-flash 报缺少 thought_signature | crates/codex-plus-core/src/protocol_proxy.rs —— 需在响应侧捕获上游返回的 thought_signature | medium |
| H | [#410](https://github.com/BigPizzaV3/CodexPlusPlus/issues/410) | system message 顺序问题导致 MiniMax API 报 2013 | 不适用（已修复，待发布） | n/a |
| H | [#247](https://github.com/BigPizzaV3/CodexPlusPlus/issues/247) | Codex++ 注入失败：SkyComputerUseService 占用调试端口 9229 | crates/codex-plus-core/src/ports.rs:72-88 —— 去掉 `!is_windows` 短路，让 macOS 也走 `c | medium |
| H | [#240](https://github.com/BigPizzaV3/CodexPlusPlus/issues/240) | 历史会话修复后 Codex App 历史会话只剩 1 个 | crates/codex-plus-data/src/provider_sync.rs:1211-1255（audit_provider_sync_stat | medium |
| M | [#1394](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1394) | [Bug]: 官方登录+混入API后无法使用GPT 5.6模型（system 消息为空） | crates/codex-plus-core/src/protocol_proxy.rs:3729 附近：构造 system 消息时若拼接结果为空（或仅空白 | small |
| M | [#1888](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1888) | 供应商切换失败：回填当前供应商配置失败 | crates/codex-plus-core/src/relay_switch.rs:117-133 —— 错误信息应带上底层原因（profile 不存在  | small |
| M | [#890](https://github.com/BigPizzaV3/CodexPlusPlus/issues/890) | Provider switch drops existing Codex plugin co | `crates/codex-plus-core/src/relay_config.rs:2110` 附近，在 `preserve_missing_table | small |
| M | [#1083](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1083) | [Bug]: MacOS Intel 第一次能打开，退出后不重启系统就打不开 | crates/codex-plus-core/src/ports.rs:214 —— 在 AddrInUse + can_connect 分支增加活性校验： | medium |
| M | [#754](https://github.com/BigPizzaV3/CodexPlusPlus/issues/754) | [Bug]: 官方登录 + API 混入模式无法调用模型，一直重连 | crates/codex-plus-core/src/relay_config.rs:3612 pure_api_auth_contents_with_li | medium |
| M | [#489](https://github.com/BigPizzaV3/CodexPlusPlus/issues/489) | [Bug]: MCP server 已 enabled，但工具没有注入到当前模型会话 | 已修：crates/codex-plus-core/src/protocol_proxy.rs（工具 schema 摊平）+ crates/codex-pl | n/a |
| M | [#467](https://github.com/BigPizzaV3/CodexPlusPlus/issues/467) | [Bug]: MCP 无法调用/插件报错/reasoning_content 报 400/上 | 已修：crates/codex-plus-core/src/protocol_proxy.rs:257 / :3342 / :3661（reasoning_ | n/a |
| M | [#459](https://github.com/BigPizzaV3/CodexPlusPlus/issues/459) | [Bug]: 对话记录不随官方登录/API 切换而调整 | 已修：crates/codex-plus-core/src/relay_config.rs:3608-3645（纯 API 分支合并 live 登录态键）。 | n/a |
| M | [#1540](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1540) | 进入 codex 之后界面卡死，切换会话就点不了 | crates/codex-plus-core/src/bridge.rs（重注入退避与健康判定） | small |
| M | [#1522](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1522) | codex++ 静默启动入口无法稳定注入程序（四种状态随机） | 待定：crates/codex-plus-core/src/launcher.rs（静默入口与重启入口的启动时序/端口占用处理） | medium |
| M | [#1372](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1372) | [Bug]: 有gtpplus无法使用官方登入(已经过了验证码) | 不适用（修复已在 main/v1.5.0，用户版本过旧） | n/a |
| M | [#1359](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1359) | [Bug]: 侧边会话消失 | assets/inject/renderer-inject/ 侧边栏分片（fa3529f7 的落点） | small |
| M | [#1971](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1971) | 切换供应商后重启只能进入登录界面 | crates/codex-plus-core/src/relay_switch.rs —— 沿用 68429a26 的 pure_api_auth_cont | small |
| M | [#1851](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1851) | [ArrayParam] input[63].content array too long（ | crates/codex-plus-core/src/protocol_proxy.rs（历史 item 的 content 数组规整，需先抓取转换前后请求 | medium |
| M | [#1032](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1032) | [Bug]: 频繁出现Codex和后台Codex++后端断连 | 不适用（已有未发布修复 5bb4f636 / c509c06d，本批无需再改码；若要根治需在 40-backend-settings.js 的自动重试与 l | n/a |
| L | [#742](https://github.com/BigPizzaV3/CodexPlusPlus/issues/742) | 切换 gpt 账号后供应商 API Key 消失、纯 API 被改成官方登录 | 待定；先核对用户 auth.json 与 settings.json 的 providerSync/profiles 快照，再看 crates/codex- | medium |

### P2

| 置信 | issue | 主题 | 修复位置 | 工作量 |
|---|---|---|---|---|
| H | [#2048](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2048) | Windows 微信连接启动 app-server 时弹出 CMD/Windows Term | crates/codex-plus-core/src/connect/app_server.rs:141-148：`#[cfg(windows)] comm | trivial |
| H | [#2031](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2031) | GLM-5.3-Flash 本地图片 data URL 被上游拒绝，建议对 glm-* 规范 | crates/codex-plus-core/src/protocol_proxy.rs:3869（image_part_to_chat）：新增模型感知的  | small |
| H | [#1488](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1488) | [Bug]: 供应商测试 测试「OpenAI Official」失败：Base URL 不能 | crates/codex-plus-core/src/relay_config.rs:856-862（test_relay_profile 入口）：官方登录 | small |
| H | [#1160](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1160) | 免安装 codex，后端连接异常，且 codex 版本检测不到 | crates/codex-plus-core/src/app_paths.rs:676-690 —— 在 codex_version_file 之后加一档  | small |
| H | [#862](https://github.com/BigPizzaV3/CodexPlusPlus/issues/862) | Codex 内自建的 SKILL 在 Codex++ 管理中显示为 0 | crates/codex-plus-core/src/skills.rs — 在 SkillsManager 的列表聚合里增加一趟「扫描 $CODEX_HO | small |
| H | [#772](https://github.com/BigPizzaV3/CodexPlusPlus/issues/772) | [Bug]: Fast 开着但提示「当前模型未读取」（GPT-5.5 中转） | assets/inject/renderer-inject/20-menu.js:448-454 —— 让 codexServiceTierFastAvai | small |
| H | [#586](https://github.com/BigPizzaV3/CodexPlusPlus/issues/586) | [Bug]: 纯API mimo-v2.5-pro web_search 原生工具被翻译为  | crates/codex-plus-core/src/protocol_proxy.rs:5098（tool_call_added_item）与 :5241 | medium |
| H | [#2080](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2080) | [Bug]: 启动前自动修复历史会话通过快捷方式打开时无效 | apps/codex-plus-launcher/src/main.rs:88-102（早退分支需补 provider sync / 修复调用） | small |
| H | [#2072](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2072) | [Bug]: v1.2.56 市场迁移后 GitHub 已安装但不在插件列表或会话工具中 | crates/codex-plus-core/src/plugin_marketplace.rs:663-708（需在清理市场条目时迁移/清理悬空的 plu | medium |
| H | [#959](https://github.com/BigPizzaV3/CodexPlusPlus/issues/959) | [Question]: 用 DeepSeek 这种纯文本模型时，只要在项目文件夹和聊天里面有 | crates/codex-plus-core/src/vision.rs:161（image_handling_mode）+ 管理端模型配置面板 | trivial |
| H | [#944](https://github.com/BigPizzaV3/CodexPlusPlus/issues/944) | [Bug]: 在codex中glm-5.1无法开启思考 | crates/codex-plus-core/src/protocol_proxy.rs:6117-6151, 6182-6213 | small |
| H | [#1463](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1463) | Fast service tier UI misdetects support after  | assets/inject/renderer-inject/30-service-tier.js:212/:235/:271/:393/:417 把当前线程 | medium |
| M | [#1316](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1316) | 移动会话失败：未找到 Codex App asset: vscode-api- | assets/inject/renderer-inject/20-menu.js:188 附近：移动会话链路改为走 loadOptionalCodexApp | small |
| M | [#1297](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1297) | responses→chat 转换未处理图片、文件，只能纯文本 | crates/codex-plus-core/src/protocol_proxy.rs 的 responses→chat 转换函数（把 input_ima | large |
| M | [#1424](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1424) | [Bug]: 智能体环境为 WSL 时修复会话功能失效 | crates/codex-plus-data/src/provider_sync.rs：会话/rollout 路径解析处复用 storage.rs 的 ws | medium |
| M | [#1141](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1141) | 窗口焦点变化或者移动鼠标指针时主窗口未预期闪烁 | assets/inject/renderer-inject/10-style.js 的预隐藏规则段（约 :1150-1250）与 60-plugin-mar | medium |
| M | [#875](https://github.com/BigPizzaV3/CodexPlusPlus/issues/875) | [Bug]: 删除会话失败 Database not found: C:\Users\Hap | `crates/codex-plus-data/src/storage.rs:181-186`：把「Database not found」从单一路径改为「列 | small |
| M | [#985](https://github.com/BigPizzaV3/CodexPlusPlus/issues/985) | [Bug]: 下载 arm64 版本的 codex++ 之后 codex++.app 仍然是 | scripts/installer/macos/package-dmg.sh：打包后加一步产物校验（对 Contents/MacOS 下二进制跑 `file | small |
| M | [#641](https://github.com/BigPizzaV3/CodexPlusPlus/issues/641) | [Bug]: 经常几个问题后出现 Expecting property name enclo | crates/codex-plus-core/src/protocol_proxy.rs:2511 push_tool_call_delta_into —— | medium |
| M | [#618](https://github.com/BigPizzaV3/CodexPlusPlus/issues/618) | [Config]: 供应商切换回填失败：config.toml TOML 解析失败 | apps/codex-plus-manager/src-tauri 的 backfill_relay_profile_from_live 命令（经 rela | small |
| M | [#609](https://github.com/BigPizzaV3/CodexPlusPlus/issues/609) | [Bug]: Codex++ 重写 config.toml 并清空 bundled 插件缓存 | crates/codex-plus-core/src/relay_config.rs:2082 preserve_live_app_settings ——  | small |
| M | [#597](https://github.com/BigPizzaV3/CodexPlusPlus/issues/597) | [Config]: 添加插件或 MCP 导致已有插件全部消失 | crates/codex-plus-core/src/relay_config.rs:2082 preserve_live_app_settings ——  | small |
| M | [#500](https://github.com/BigPizzaV3/CodexPlusPlus/issues/500) | [Bug]: 侧边的Timeline不准，而且时不时会抖动 | **已不适用**：Timeline 已于 v1.2.19 整体移除（见 §3.1 订正说明），本仓当前无此组件 | n/a |
| M | [#300](https://github.com/BigPizzaV3/CodexPlusPlus/issues/300) | macOS 自动更新只挂载 DMG 未执行安装 | apps/codex-plus-manager/src-tauri/src/commands.rs（更新命令）—— 需在挂载后增加把 .app 复制到 /A | medium |
| M | [#239](https://github.com/BigPizzaV3/CodexPlusPlus/issues/239) | 启动 Codex++ 失败：failed to query CDP targets | 先复测：升级到 v1.5.0 后重跑，若仍失败则看 `ports.rs:72-80` 是否把端口让到了 9229 之外而状态文件仍报 9229（`statu | small |
| M | [#1650](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1650) | 纯官方登录供应商无法持久化/恢复专属 config.toml | 待定：crates/codex-plus-core/src/relay_switch.rs:35-46 与 relay_config.rs 的 backfi | medium |
| M | [#1498](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1498) | 界面 UI 频闪，切浅色主题显示异常（只有聊天框白，其余黑字） | assets/inject/renderer-inject/ 配色与防闪烁分片（需 assemble-renderer-inject.mjs 重组） | small |
| M | [#1341](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1341) | [Question]: 点击会话管理的调度问题 | apps/codex-plus-manager/src-tauri/src/commands.rs（会话同步命令）+ 前端点击处理 | medium |
| M | [#1329](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1329) | [Bug]: Codex++无法通过系统代理连接到GitHub | crates/codex-plus-core/src/http_client.rs:proxied_client —— 把 detect_system_pr | small |
| M | [#1905](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1905) | 主题/皮肤重启后失效，每次都要重新应用 | crates/codex-plus-core/src/dream_skin_runtime.rs —— 启动时按持久设置重新注入，而非依赖上次会话的运行时状 | medium |
| M | [#1893](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1893) | 强制 codex 界面中文失效 | assets/inject/renderer-inject/00-prelude.js:97 起 —— 重载/重试路径；核对 api-key 模式重启后 l | small |
| M | [#1784](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1784) | 换肤功能无法使用（原生输入框不可见 / 首页横幅不匹配） | crates/codex-plus-core/src/dream_skin.rs 的校验锚点 —— 按 Codex 新版 DOM 结构调整选择器（优先 da | medium |
| M | [#1213](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1213) | [Bug]: agnes 支持 512 了，默认显示还是 256 上下文 | crates/codex-plus-core/assets/codex-models.json（新增 agnes 条目，填 context_window 与 | small |
| M | [#1226](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1226) | [Bug]: Codex 26.623.42026 版本 连接 deepseek | 前半段：升级到含 f55bb646 的版本后按诊断日志定位；后半段：改用 Chat Completions 上游（crates/codex-plus-cor | small |
| M | [#982](https://github.com/BigPizzaV3/CodexPlusPlus/issues/982) | [Bug]: 历史会话记录修复失败（从第三方切到官方后记录消失） | 待确认：crates/codex-plus-data/src/provider_sync.rs 的目标会话筛选（changed_session_files  | medium |
| M | [#979](https://github.com/BigPizzaV3/CodexPlusPlus/issues/979) | [Bug]: Macbook 盒盖导致后端连接断开 | assets/inject/renderer-inject/40-backend-settings.js:110-134（状态机）与 launcher 侧代 | medium |
| M | [#975](https://github.com/BigPizzaV3/CodexPlusPlus/issues/975) | [Bug]: 模型映射后 codex 内模型没有改变或者消失 | 已在 ca69f48e 修（待发版）；若要真正「与 ccswitch 兼容」还需后续支持合并外部 catalog 条目而非仅降级到顶层窗口值 | medium |
| M | [#1082](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1082) | [Bug]: 代理配置后 glm / minimax 等模型无法使用（Unsupported | crates/codex-plus-core/src/protocol_proxy.rs:6193 infer_chat_reasoning_style — | medium |
| M | [#1036](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1036) | [Bug]: 未检测到 Codex 应用版本，打不开 codex++ 只能打开管理工具 | crates/codex-plus-core/src/app_paths.rs:532 is_codex_plus_plus_path —— 收紧判定：不能 | small |
| M | [#495](https://github.com/BigPizzaV3/CodexPlusPlus/issues/495) | [Bug]: 插件栏无法激活 | assets/inject/renderer-inject/60-plugin-marketplace.js:1-52（修复已存在，需发版送达用户） | small |
| M | [#728](https://github.com/BigPizzaV3/CodexPlusPlus/issues/728) | 长时间挂着 codex 不动会导致后端连接失败 | 已修在 main（crates/codex-plus-core/src/bridge.rs bridge_health_check_script 的 hid | n/a |
| M | [#702](https://github.com/BigPizzaV3/CodexPlusPlus/issues/702) | 用 codex++ 启动时点设置会卡很久、页面卡住、codex++ 显示红色 | 已修在 main：crates/codex-plus-core/src/bridge.rs（hidden 早返回）、crates/codex-plus-co | n/a |
| M | [#2156](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2156) | v1.3.0 在单个请求里发出两份重复 MCP namespace，上游拒绝 | crates/codex-plus-core/src/protocol_proxy.rs:4700-4744（已有去重）+ responses 直通分支 : | medium |
| L | [#1062](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1062) | [Bug]: 几轮对话之后出现 BadRequestError（Expecting prop | crates/codex-plus-core/src/protocol_proxy.rs:3374 与 3661（tool_calls / reasonin | medium |
| L | [#406](https://github.com/BigPizzaV3/CodexPlusPlus/issues/406) | MAC M1 升级 1.1.8 后对话 502 Bad Gateway（127.0.0.1: | crates/codex-plus-core/src/launcher.rs:1828（已在 main 补齐上游失败原因落日志；本条待新版发布后凭日志定位） | small |

### P3

| 置信 | issue | 主题 | 修复位置 | 工作量 |
|---|---|---|---|---|
| H | [#692](https://github.com/BigPizzaV3/CodexPlusPlus/issues/692) | renderer-inject.js 硬编码中文显示名导致插件市场标签错乱 | assets/inject/renderer-inject/50-navigation.js:82-90：改回 `return fallback`（或把编号 | small |
| M | [#1975](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1975) | 统一侧边栏/列表项右侧状态指示灯右边距 | apps/codex-plus-manager/src/styles.css —— 让顶部标题行与列表项容器共用同一右内边距（列表项预留滚动条 gutter | trivial |
| M | [#1863](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1863) | macOS Dock 中管理工具图标显示为白色 | apps/codex-plus-manager/src-tauri/tauri.conf.json bundle.icon 增加 icons/icon.ic | small |
| M | [#595](https://github.com/BigPizzaV3/CodexPlusPlus/issues/595) | [Bug]: 无法切换回官方登录态，中转界面没有清除 api 选项 | apps/codex-plus-manager/src/App.tsx 供应商/登录态切换区 + crates/codex-plus-core/src/re | medium |
| M | [#949](https://github.com/BigPizzaV3/CodexPlusPlus/issues/949) | [Bug]: fast按钮消失了。 | assets/inject/renderer-inject/30-service-tier.js:212-249、:271-273 | n/a |
| M | [#1876](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1876) | 最大化时窗口底部越过 Windows 任务栏 | apps/codex-plus-manager/src-tauri/src/lib.rs 窗口创建处 —— 最大化尺寸按 SPI_GETWORKAREA 计 | small |

### 3.1 逐条根因

**#1931** — 粘贴修复开启后大文本不转文件导致卡死

- 结论：`confirmed-in-code` / P1 / 置信 high / 工作量 small
- 根因：粘贴修复无条件对任意长度的 text/plain 调 execCommand('insertText')，绕过了 Codex 自身的「超长文本转附件」逻辑，长文本因此整段塞进输入框造成卡死。
- 证据：assets/inject/renderer-inject/zz-paste-fix.js:20 起：只判 `text.length === 0`，无任何长度上限即 `e.preventDefault(); e.stopImmediatePropagation()` 后 insertText；全文件 39 行、无长度阈值常量。开关由 settings.codexAppPasteFix 注入。
- 落点：assets/inject/renderer-inject/zz-paste-fix.js:20 —— 增加长度阈值（超过 N 字符时不拦截，交给 Codex 原生转附件逻辑）；改完须跑 node scripts/assemble-renderer-inject.mjs

**#1209** — [Feature Request] web_search_call 被错译为 custom_tool_call（关联 #586）

- 结论：`confirmed-in-code` / P1 / 置信 high / 工作量 medium
- 根因：确认是代码缺陷：翻译层只把 web_search 登记成 custom 代理工具并把 kind 标为 BuiltIn，但生成响应 item 时只特判了 tool_search，其余一律落到 custom_tool_call，BuiltIn 这个枚举值全仓没有任何读取点，因此 web_search 调用永远无法还原成 web_search_call。
- 证据：crates/codex-plus-core/src/protocol_proxy.rs:4057 与 4138/4198 将 web_search 登记为 context.custom_tools 且 CodexCustomToolKind::BuiltIn；同文件 5083-5116 的 tool_call_added_item 只在 is_tool_search_proxy 时输出 tool_search_call，否则输出 type=custom_tool_call、name=original_custom_tool_name。grep `BuiltIn` 全仓仅 158（枚举定义）、4067、4200 三处，无任何匹配/读取分支。测试 crates/codex-plus-core/tests/protocol_proxy.rs:2228-2248 只断言 tools 中保留了 w
- 落点：crates/codex-plus-core/src/protocol_proxy.rs:5083 的 tool_call_added_item（以及对应的 completed 分支，约 5236-5255）中新增 BuiltIn 分支：当 kind 为 BuiltIn 且原始工具类型为 web_search 时，输出 {type:"web_search_call", id, status, query/action} 而非 custom_tool_call；同处需同步输出 response.output_item.done，并保证 SSE 事件类型一致。可先只做 web_search，code_interpreter/file_search 视 Codex 是否识别再补。

**#1012** — [Bug]: gemini3.5flash 错误（thought_signature）

- 结论：`confirmed-in-code` / P1 / 置信 high / 工作量 medium
- 根因：Gemini 3 系模型要求 functionCall 携带 thought_signature，本仓协议翻译层完全没有处理该字段（全仓零命中），带工具调用时整轮 400；这是本仓在 Gemini 新版签名规则上的真实缺失。
- 证据：全仓 grep `thought_signature` / `thoughtSignature` 在 crates/ 零命中；协议翻译实现在 crates/codex-plus-core/src/protocol_proxy.rs（工具消息编解码），无该字段的透传与回填逻辑。
- 落点：crates/codex-plus-core/src/protocol_proxy.rs：在把上游 response 转回 Codex 格式时保留 Gemini 返回的 thought_signature，并在下一轮把 functionCall 对应的签名原样回传；需按 Gemini 官方文档（ai.google.dev/gemini-api/docs/thought-signatures）确认字段挂在 part 级还是 functionCall 级

**#332** — gemini-3.5-flash 报缺少 thought_signature

- 结论：`confirmed-in-code` / P1 / 置信 high / 工作量 medium
- 根因：接入 Gemini 后 functionCall 缺少 thought_signature 被上游 400 拒绝。本仓协议翻译层完全没有 thought_signature 的提取与回传逻辑，历史 assistant 的 tool_calls 回放时不会带上该字段，属于确认的协议兼容缺口。
- 证据：全仓 grep "thought_signature" / "thoughtSignature" 在 crates/ 与 apps/ 下零命中。协议翻译的 tool_calls 组装位于 crates/codex-plus-core/src/protocol_proxy.rs:2653 附近（`"reasoning_content": self.reasoning.text`）与 :3342、:4954，均无 thought_signature 透传。上游报错原文：「Function call is missing a thought_signature in functionCall parts ... position 3」。
- 落点：crates/codex-plus-core/src/protocol_proxy.rs —— 需在响应侧捕获上游返回的 thought_signature/thoughtSignature，并在 history 回放 tool_calls 时原样带回（现有 reasoning 保真逻辑可作参照，见 :3661-3688、:3768-3818）

**#410** — system message 顺序问题导致 MiniMax API 报 2013

- 结论：`confirmed-in-code` / P1 / 置信 high / 工作量 n/a
- 根因：报障描述的根因成立且已被修复：转换后的 messages 现在会把所有 system 消息折叠并前置到数组头部，MiniMax 要求的「system 必须在最前」得到满足；但该修复尚未发布。
- 证据：crates/codex-plus-core/src/protocol_proxy.rs:259 `let messages = collapse_system_messages_to_head(messages);`，函数定义在同文件 :3713，实现是把全数组 role==system 的 content 收集后用 `\n\n` 拼接成单条 system 推到数组首位（:3729-3737），紧随 :258 normalize_chat_messages 之后。该行由 a8dcd29f（2026-10-02 17:03，"fix(protocol): 修复两处 tool 消息边界缺陷"）引入，`git tag --contains a8dcd29f` 输出为空，即晚于最新正式版 v1.5.0(2026-10-01)。
- 落点：不适用（已修复，待发布）

**#247** — Codex++ 注入失败：SkyComputerUseService 占用调试端口 9229 导致 CDP 连接失败

- 结论：`confirmed-in-code` / P1 / 置信 high / 工作量 medium
- 根因：确认是本仓缺陷：macOS 路径从不做调试端口占用检测，只无条件把 `--remote-debugging-port=9229` 交给 Codex；当 SkyComputerUseService 已占着 9229 时，新起的 Codex 拿不到该端口，注入永久失败。端口复用判定只看端口是否在监听，不看监听者是不是 Codex，建议按举报者方向补 macOS 的端口探测/让位。
- 证据：`crates/codex-plus-core/src/ports.rs:72-88`：`select_packaged_codex_debug_port_with` 第一句就是 `if !is_windows \|\| can_bind(requested) \|\| is_existing_cdp(requested) { requested }`，非 Windows 直接返回请求端口，不做任何占用检测。`crates/codex-plus-core/src/launcher.rs:515`（`let debug_port = hooks.select_debug_port(options.debug_port)`）→ `launcher.rs:825-830` 的 macOS hook 落到 `select_packaged_codex_debug_port`。`crates/cod
- 落点：crates/codex-plus-core/src/ports.rs:72-88 —— 去掉 `!is_windows` 短路，让 macOS 也走 `can_bind(requested) → find_available()`；并在 launcher.rs:1064-1072 的 macOS 分支启动前用 `cdp::endpoint_available(debug_port)` 判定端口归属，非 Codex 占用则改用候选端口重试。同时给端口冲突补一条诊断日志（现在只有 `launcher.macos_existing_app_without_cdp_restart_requested`，覆盖不到「端口被别的进程占」）。

**#240** — 历史会话修复后 Codex App 历史会话只剩 1 个

- 结论：`confirmed-in-code` / P1 / 置信 high / 工作量 medium
- 根因：确认存在破坏性的可能：供应商同步会删除「只在 catalog 表里存在、threads 表没有对应行」的记录（sqliteCatalogRowsRemoved），而 `threads` 才是 Codex 列表读取的 canonical 表；一旦这些行本来只靠 user_threads 投影撑着，同步后会话就从列表里消失且不会自动重建。修复代码自身也承认「未自动重建缺失的 canonical 会话」，属已知缺口。
- 证据：`crates/codex-plus-data/src/provider_sync.rs:1211-1255` 的 `audit_provider_sync_state`：`canonical_thread_ids`（threads 表）与 `catalog_thread_ids`（user_thread 投影）求差集，得出 `catalog_only`；`provider_sync.rs:1192-1204` 的 `provider_sync_message_with_audit` 明说「审计发现 N 条仅存在于本地会话目录的记录……**未自动重建缺失的 canonical 会话**」。对应删除计数 `sqlite_catalog_rows_removed` 在 `apps/codex-plus-manager/src-tauri/src/commands.rs:3689` 透出。备份路
- 落点：crates/codex-plus-data/src/provider_sync.rs:1211-1255（audit_provider_sync_state）—— 删除 catalog 行之前，对 `catalog_only` 且 `has_current_rollout` 的记录先补建 canonical `threads` 行，而不是只记录下来；`provider_sync.rs:527` 的 `run_provider_sync` 增加 dry-run 模式（只回审计、不写）。另 `apps/codex-plus-manager/src-tauri/src/commands.rs:3689` 的 UI 应在执行前显示「将影响 N 个会话、备份在 <path>」并要求确认。

**#1394** — [Bug]: 官方登录+混入API后无法使用GPT 5.6模型（system 消息为空）

- 结论：`confirmed-in-code` / P1 / 置信 medium / 工作量 small
- 根因：官方登录 + 混入 API Key，用官方 GPT-5.6 时报「the message at position 1 with role 'system' must not be empty」，5.5 及以下正常。属协议层请求构造缺陷的可能性高：system 段在某种模型/工具组合下被构造成空内容，上游严格校验即 400。
- 证据：crates/codex-plus-core/src/protocol_proxy.rs:3729-3730 存在按 system_chunks 是否为空决定是否输出 system 消息的分支（let mut output = Vec::with_capacity(rest.len() + usize::from(!system_chunks.is_empty())); if !system_chunks.is_empty() { ... }），说明 system 段是拼接产物；全仓 grep 无对「空 system 内容」的显式防御。相关邻近修复：a8dcd29f（tool 消息边界 + reasoning 丢失）、09a9f1c5（工具 schema 顶层组合器致 Anthropic 上游 400）。
- 落点：crates/codex-plus-core/src/protocol_proxy.rs:3729 附近：构造 system 消息时若拼接结果为空（或仅空白）应整条不发出，而不是发一条空 content 的 system 消息；同时检查混入 API Key（MixedApi）路径下 system 段的合并顺序

**#1888** — 供应商切换失败：回填当前供应商配置失败

- 结论：`confirmed-in-code` / P1 / 置信 medium / 工作量 small
- 根因：该报错串在本仓有确切出处，触发条件是切换前回填「上一个」供应商时失败；最常见原因就是上一个供应商已不在配置列表里（被删除或 id 变化），另有读取 home 配置失败等分支。
- 证据：crates/codex-plus-core/src/relay_switch.rs:132 `.with_context(\|\| "回填当前供应商配置失败")`，上下文函数 backfill_profile_before_switch（同文件 :117 起）在找不到 `previous_active_relay_id` 对应 profile 时先返回「当前供应商已不在配置列表中，已停止切换以避免覆盖用户改动。」；apps/codex-plus-manager/src-tauri/src/commands.rs:4922 拼成用户看到的「回填当前供应商配置失败：{error}」，i18n-en.ts:1520 有对应英文条目。
- 落点：crates/codex-plus-core/src/relay_switch.rs:117-133 —— 错误信息应带上底层原因（profile 不存在 / 读取 home 失败），便于自助

**#890** — Provider switch drops existing Codex plugin config

- 结论：`confirmed-in-code` / P1 / 置信 medium / 工作量 small
- 根因：切换供应商时写 live config.toml 会丢掉 `[plugins.*]`/`[marketplaces.*]`/`[features]` 等运行时段。现状是 marketplaces、features、desktop、sandbox、hooks、mcp_servers 都有从 live 补缺的处理，但**没有对 root 级 `plugins` 表做同样的保留**——用户配置的插件启停状态在切换后可能被抹掉，这条报障在代码上站得住。
- 证据：`crates/codex-plus-core/src/relay_config.rs:2082-2130` 的 `preserve_live_app_settings` 兜底列表包含：`desktop`、`sandbox_mode` / `approval_policy` / `sandbox_workspace_write` / `windows`、`repair_mcp_servers_from_live`、`preserve_missing_table_keys(target, live, "features")`、`preserve_live_hook_definitions`、`preserve_live_hook_state` —— grep 全文**没有任何 `preserve_missing_table_keys(..., "plugins")` 调用**（`grep -
- 落点：`crates/codex-plus-core/src/relay_config.rs:2110` 附近，在 `preserve_missing_table_keys(&mut target_doc, &live_doc, "features")` 之后补一行 `preserve_missing_table_keys(&mut target_doc, &live_doc, "plugins");`（只补缺、不覆盖，与 features 同口径）。同时在 `crates/codex-plus-core/tests/relay_config.rs` 补回归：断言切换供应商后 `[plugins.*]` 条目仍在、且旧 provider 段与 root `OPENAI_API_KEY` 已被清除。需要先核实 `plugins` 在 live config 里是表还是带引号键的表（`[plugins."figma@openai-curated"]`），确认 `as_table_like` 能取到 dotted key。

**#1083** — [Bug]: MacOS Intel 第一次能打开，退出后不重启系统就打不开

- 结论：`confirmed-in-code` / P1 / 置信 medium / 工作量 medium
- 根因：单实例锁的获取逻辑存在误判：当目标端口被一个「还能 connect 但没有真实服务」的残留进程占着时，代码直接返回 AddrInUse 判定「已有实例在跑」，造成用户重启管理工具却被静默判定为已在运行、窗口不出现。
- 证据：crates/codex-plus-core/src/ports.rs:201-225 acquire_resilient_loopback_port_guard_with：`Err(error) if error.kind()==AddrInUse && can_connect(port) => Err(error)`；apps/codex-plus-manager/src-tauri/src/lib.rs:559-583 收到 AddrInUse/WouldBlock 即记录 manager.already_running 并返回 None（不建窗口）。锁文件本身是 flock 建议锁（ports.rs:233-247 try_lock_exclusive），进程死后锁会释放，所以真正的判据是 can_connect——而僵尸端口仍然 connect 成功，于是被永久判为「已运行」。用
- 落点：crates/codex-plus-core/src/ports.rs:214 —— 在 AddrInUse + can_connect 分支增加活性校验：仅当该端口上的进程确实是本产品（例如探测 /health 或校验进程名/可执行路径）才报 Err；否则记录一次告警并降级为 fallback_lock 继续启动。也可在 manager 侧（lib.rs:571 分支）在判定已运行前先 focus 一次已有窗口，focus 失败则视为僵尸端口继续启动。

**#754** — [Bug]: 官方登录 + API 混入模式无法调用模型，一直重连

- 结论：`confirmed-in-code` / P1 / 置信 medium / 工作量 medium
- 根因：官方登录 + 混入模式下会话一直重连；日志同时给出两条独立线索：auth.json 只剩 OPENAI_API_KEY、没有 tokens 与 auth_mode，以及 service_tier_dispatcher_patch_failed。前者对应 PureApi 分支整体覆盖 auth.json 抹掉 ChatGPT 登录态（68429a26，issue #2173）——该提交晚于 v1.5.0，main 上已修、用户拿不到；后者是混入模式下 dispatcher patch 失败，需要单独取证。
- 证据：crates/codex-plus-core/src/relay_config.rs:712-730 PureApi 分支已改为 pure_api_auth_contents_with_live_login（:3612 起，保留 live 的 tokens / auth_mode），git log -S 定位 68429a26「fix(relay): 纯 API 切换不再抹掉 live 的 ChatGPT 登录态 (issue #2173)」2026-10-02，git merge-base --is-ancestor 68429a26 v1.5.0 → 否；用户报告的 service_tier_dispatcher_patch_failed / Codex dispatcher unavailable 对应 assets/inject/renderer-inject/30-service
- 落点：crates/codex-plus-core/src/relay_config.rs:3612 pure_api_auth_contents_with_live_login（已修，待发布）；dispatcher patch 失败另需在 assets/inject/renderer-inject/30-service-tier.js:940-1000 附近定位 dispatcher 未就绪的时序

**#489** — [Bug]: MCP server 已 enabled，但工具没有注入到当前模型会话

- 结论：`confirmed-in-code` / P1 / 置信 medium / 工作量 n/a
- 根因：第三方 Responses 上游不认 codex 注入的 MCP 工具 schema（被摊平/带组合器），导致 @Chrome/@Browser/@Computer Use 的 tool 定义整轮 400 或被上游丢弃。09a9f1c5 已修工具 schema 顶层组合器这一半，但浏览器/CUA 的 runtime 白名单识别 04ab6290 才是完整修复，两者都落在 main 未发布。
- 证据：09a9f1c5 'fix(protocol): 摊平工具 schema 顶层组合器，修复 Anthropic 上游整轮 400 (issue #2367)'；04ab6290 'fix(native-browser): CUA runtime 白名单改认结构不认哈希，修复浏览器控制不可接入 (issue #2294, #2209)' 提交信息写明：RuntimeContract 只看 0.0.11/0.0.24 哈希，新版落在 Err → control.json 写 requireIdentification:false → require-identification.mjs 回落云端策略 → 报 'unsupported Codex auth method: apikey'。git merge-base --is-ancestor 04ab6290 v1.5.0 → NO（未发布）
- 落点：已修：crates/codex-plus-core/src/protocol_proxy.rs（工具 schema 摊平）+ crates/codex-plus-core/src/native_browser_contract.rs（FileCheck 结构校验）。待发布。

**#467** — [Bug]: MCP 无法调用/插件报错/reasoning_content 报 400/上下文默认 258/沙盒无法切换

- 结论：`confirmed-in-code` / P1 / 置信 medium / 工作量 n/a
- 根因：混装的多个问题。其中「The `reasoning_content` in the thinking mode must be passed back to the API」是本仓真实缺陷且已修复并进入 v1.5.0（468e0ae2 → a8dcd29f）；同条还夹了 502/图片占满上下文/沙盒切换等独立问题，建议拆分跟踪。
- 证据：crates/codex-plus-core/src/protocol_proxy.rs:3661-3662 注释直接引用了该报错原文：'DeepSeek thinking 模式要求带 `tool_calls` 的 assistant 消息回传 `reasoning_content`，否则报 400（The `reasoning_content` in the thinking mode must be passed back to the API）'；修复函数 ensure_tool_call_reasoning_content 在 :257 被调用。提交：468e0ae2 'Fixes #1860'，git merge-base --is-ancestor 468e0ae2 v1.5.0 → YES（已发布）；后续 a8dcd29f 'fix(protocol): 修复两处 tool 
- 落点：已修：crates/codex-plus-core/src/protocol_proxy.rs:257 / :3342 / :3661（reasoning_content 回传与占位补齐）。v1.5.0 已含主修复，a8dcd29f 的补强待发布。

**#459** — [Bug]: 对话记录不随官方登录/API 切换而调整

- 结论：`confirmed-in-code` / P1 / 置信 medium / 工作量 n/a
- 根因：切换官方登录 ↔ 第三方 API 时，会话记录归属不对（全被丢到 API 侧）。根因与本仓 auth.json / 会话身份处理相关：纯 API 切换过去会覆盖 live 的 ChatGPT 登录态（tokens/auth_mode），导致会话身份判断错位。68429a26 已修，但落在 v1.5.0 之后。
- 证据：68429a26 'fix(relay): 纯 API 切换不再抹掉 live 的 ChatGPT 登录态 (issue #2173)'；代码 crates/codex-plus-core/src/relay_config.rs:3608-3640 `pure_api_auth_contents_with_live_login`，注释明确：'那把 live 里的 `tokens` / `auth_mode` 整体覆盖掉，而 `tokens` 正是「OpenAI 会话身份」…让 CUA 浏览器插件可用的前提，于是纯 API 供应商每切换一次就可能静默弄坏插件'，LOGIN_STATE_KEYS = ["tokens", "auth_mode"]。git merge-base --is-ancestor 68429a26 v1.5.0 → NO（未发布）。
- 落点：已修：crates/codex-plus-core/src/relay_config.rs:3608-3645（纯 API 分支合并 live 登录态键）。待发布。

**#1540** — 进入 codex 之后界面卡死，切换会话就点不了

- 结论：`likely-in-code` / P1 / 置信 medium / 工作量 small
- 根因：切会话后整页无响应。仓库有直接对应的修复：b6a00c27「修复 Codex++ 重复注入导致卡死和白屏」，以及 5bb4f636（窗口隐藏不判定桥接失效、退避持续健康才清零，根治每分钟整页重注入）。但需确认这些提交是否已进 v1.5.0，且用户说「禁用插件/用兼容模式」未必真关掉了注入。
- 证据：commit b6a00c27 修复 Codex++ 重复注入导致卡死和白屏；commit 5bb4f636 fix(bridge): 窗口隐藏不判定桥接失效 + 退避持续健康才清零，根治每分钟整页重注入。均需 git tag --contains 核实是否入 v1.5.0（5bb4f636 与 v1.5.0 notes 同期，疑似已发布）。
- 落点：crates/codex-plus-core/src/bridge.rs（重注入退避与健康判定）

**#1522** — codex++ 静默启动入口无法稳定注入程序（四种状态随机）

- 结论：`likely-in-code` / P1 / 置信 medium / 工作量 medium
- 根因：静默启动入口时好时坏：绿点但脚本不生效、红灯断连、完全无指示，四种状态随机；用管理工具的「重启 Codex++」则稳定正常。核心差异是静默入口与重启走的路径/时序不同，仓库近年修过多次注入时序与桥接健康判定，需确认是否覆盖了静默入口这条路径。
- 证据：相关提交：b6a00c27 修复重复注入导致卡死和白屏；5bb4f636 fix(bridge): 窗口隐藏不判定桥接失效 + 退避持续健康才清零，根治每分钟整页重注入；3ce9e74c fix(manager): 无效的 Codex 应用路径不再落库，避免启动永久失败。静默入口与重启入口的差异点在 launcher.rs（debug_port/helper_port 固定为 9229/57321，见用户日志）。用户版本 1.2.37，跨多个版本。
- 落点：待定：crates/codex-plus-core/src/launcher.rs（静默入口与重启入口的启动时序/端口占用处理）

**#1372** — [Bug]: 有gtpplus无法使用官方登入(已经过了验证码)

- 结论：`likely-in-code` / P1 / 置信 medium / 工作量 n/a
- 根因：用户付费 ChatGPT Plus 后用官方登录方式登不上（验证码已过），1.2.12 老版本。这正是本仓已知的凭据被覆盖类缺陷：切换/写入 relay 时把 live 的 ChatGPT 登录态（auth.json 的 tokens）抹掉，1.5.0 之前的版本尤其明显。
- 证据：d09d5e46「聚合模式切换保留 auth.json 认证状态，修复弹登录页（issue #1604）」在 v1.4.0/v1.5.0（git tag --contains 命中）；68429a26「纯 API 切换不再抹掉 live 的 ChatGPT 登录态 (issue #2173)」在 v1.5.0 之后，未发布；relay_config.rs:3565 注释「同时保留已有凭据（官方 OAuth tokens 等）」。用户版本 1.2.12 远早于两条修复。
- 落点：不适用（修复已在 main/v1.5.0，用户版本过旧）

**#1359** — [Bug]: 侧边会话消失

- 结论：`likely-in-code` / P1 / 置信 medium / 工作量 small
- 根因：升级到 1.2.32 后侧边会话栏消失，会话修复无效，但托盘里仍能看到并进入会话。典型的侧边栏注入/索引被破坏：托盘读的是数据，侧边栏读的是注入后的 DOM + global state，两者不一致说明注入或 catalog 修复这一侧的问题。
- 证据：fa3529f7「fix: stabilize Codex app sidebar injection and bridge watchdog」(2026-09-26, 无 tag，晚于 v1.5.0)；52ffe102「页面顶替原生侧边栏，而不是并排多出一列」；9552597a「preserve paginated user threads during sidebar catalog repair」；e6bdac1a「protect sidebar state during provider sync」。用户版本 1.2.32 早于全部这些。
- 落点：assets/inject/renderer-inject/ 侧边栏分片（fa3529f7 的落点）

**#1971** — 切换供应商后重启只能进入登录界面

- 结论：`likely-in-code` / P1 / 置信 medium / 工作量 small
- 根因：切供应商重启后落到登录页、重输 API 报错，与「切换写入的 auth.json 与 live 登录态关系处理不当」高度吻合；相邻缺陷 68429a26 已修但晚于 v1.5.0。
- 证据：commit 68429a26「fix(relay): 纯 API 切换不再抹掉 live 的 ChatGPT 登录态 (issue #2173)」描述了同一族症状（切换时把 live 的 tokens/auth_mode 抹掉，导致会话身份丢失）；`git tag --contains 68429a26` 为空，即未进 v1.5.0。另有 e94c2b60 仅钉住「切换不串配置」回归测试，未覆盖登录态丢失。
- 落点：crates/codex-plus-core/src/relay_switch.rs —— 沿用 68429a26 的 pure_api_auth_contents_with_live_login 收窄逻辑，确认覆盖 requires_openai_auth 场景

**#1851** — [ArrayParam] input[63].content array too long（切回官方账号后）

- 结论：`likely-in-code` / P1 / 置信 medium / 工作量 medium
- 根因：同一会话先在 DeepSeek 上游用 Responses↔Chat 转换跑过、切回官方账号后继续时出现 `[ArrayParam] array_above_max_length`，属于转换后消息结构残留导致旧会话无法继续；协议层已有多轮同类前缀/边界修复但仍可能漏。
- 证据：crates/codex-plus-core/src/protocol_proxy.rs 已有针对同类上游拒绝的修复注释与实现：:2145-2149（`resp_xxx`→`xxx`，避免 `resp_xxx_msg` 被拒，#1431）、:3003-3068（`[ApiIdParam] [input[N].id] [invalid_id_prefix]` 前缀重建，识别 item_/cp_/resp_ 后缀）；前轮审计 docs/reports/2026-09-11-open-pr-and-issue-triage.md:129-130 已把 #1851 与 #1796/#1781/#1431/#1493 归为同一族「responses→chat 转换后 ID 前缀/内容长度不对」。新开会话正常、旧会话报错，说明残留落在历史 item 上。
- 落点：crates/codex-plus-core/src/protocol_proxy.rs（历史 item 的 content 数组规整，需先抓取转换前后请求体）

**#1032** — [Bug]: 频繁出现Codex和后台Codex++后端断连

- 结论：`likely-in-code` / P1 / 置信 medium / 工作量 n/a
- 根因：「重启 Codex 后约 70% 概率连不上后端、状态灯红色、修复后端一直失败」与 #1001/#1008/#1014 同属「启动/重启生命周期 + 桥接看门狗」一条链路；main 上已有两处针对性修复（桥接在窗口隐藏时不再误判失效、启动与重启生命周期确定化），但两者都落在 v1.5.0 tag 之后，用户用的 1.2.9 拿不到。
- 证据：5bb4f636「fix(bridge): 窗口隐藏不判定桥接失效 + 退避持续健康才清零，根治每分钟整页重注入」（2026-10-02，`git tag --contains 5bb4f636` 空 = 未发布）；c509c06d「fix: make launch and restart lifecycle deterministic (#2313)」（2026-10-03，`git tag --contains c509c06d` 空 = 未发布）；已在 v1.5.0 的更早修复 e06c9fb5「桥接重注入指数退避、陈旧会话主动关闭与桥接降级可见化」(issue #2169) 与 ade2001d「修复后端状态灯瞬时变黄和桥接重复重注入」`git tag --contains` 均含 v1.4.0/v1.5.0。相关状态机在 assets/inject/renderer-inject
- 落点：不适用（已有未发布修复 5bb4f636 / c509c06d，本批无需再改码；若要根治需在 40-backend-settings.js 的自动重试与 launcher 重启路径上加一次「重启后强制重注入」兜底）

**#742** — 切换 gpt 账号后供应商 API Key 消失、纯 API 被改成官方登录

- 结论：`likely-in-code` / P1 / 置信 low / 工作量 medium
- 根因：在其他平台切换 ChatGPT 账号后，本工具的供应商 API Key 丢失、纯 API 模式被改回官方登录——方向与本仓 auth.json 写入链路一致（纯 API 覆盖写会抹掉 live 登录态，反向亦然），但缺用户现场证据确认是哪一条分支触发。
- 证据：同族缺陷已确认并修复一例：68429a26「fix(relay): 纯 API 切换不再抹掉 live 的 ChatGPT 登录态 (issue #2173)」，提交正文明确『PureApi 直接 apply_relay_files_to_home(home, config, &profile.auth_contents) 把 profile 快照整体写进去』→『每切换一次就可能把 live 里的 tokens / auth_mode 抹掉』。相关代码：crates/codex-plus-core/src/relay_config.rs:4000（store-only 分支按 auth_contents_looks_like_chatgpt_auth 决定是否保留 OpenAI key）、:4103、:1027-1055（切回官方时清 provider 段）。注意 68429a26 经 g
- 落点：待定；先核对用户 auth.json 与 settings.json 的 providerSync/profiles 快照，再看 crates/codex-plus-core/src/relay_config.rs:3990-4010 的 store-only 分支

**#2048** — Windows 微信连接启动 app-server 时弹出 CMD/Windows Terminal 窗口

- 结论：`confirmed-in-code` / P2 / 置信 high / 工作量 trivial
- 根因：确认是本仓缺陷，用户定位准确。CodexAppServer::start 构造 tokio Command 时只设了三路 piped 和 kill_on_drop，Windows 下没有设 CREATE_NO_WINDOW，子进程因此拿到可见控制台，连带其 MCP 子进程（memory、sequential-thinking）也各有 conhost。仓库别处（launcher、watcher、update、install、native_browser_contract）都已经用了 CREATE_NO_WINDOW，唯独这条路径漏了。修法就一行。
- 证据：crates/codex-plus-core/src/connect/app_server.rs:141-148（`Command::new(&executable)` 后仅 `.arg("app-server").current_dir().stdin/stdout/stderr(Stdio::piped()).kill_on_drop(true)`，无 creation_flags）。对照：crates/codex-plus-core/src/launcher.rs:1136、watcher.rs:893、update.rs:415、install/mod.rs:301、native_browser_contract.rs:121/452 均调用了 creation_flags(CREATE_NO_WINDOW)；常量定义在 crates/codex-plus-core/src/win
- 落点：crates/codex-plus-core/src/connect/app_server.rs:141-148：`#[cfg(windows)] command.creation_flags(codex_plus_core::windows_create_no_window());`（或 crate::windows_integration::CREATE_NO_WINDOW），放在 spawn 之前，保留现有 stdio 管道与 kill_on_drop

**#2031** — GLM-5.3-Flash 本地图片 data URL 被上游拒绝，建议对 glm-* 规范化为纯 Base64

- 结论：`confirmed-in-code` / P2 / 置信 high / 工作量 small
- 根因：确认是缺失能力，用户隔离验证做得很干净。responses→chat 转换里 image_part_to_chat 把 Responses 的 image_url 原样搬进 Chat 形态，本地图片会带着完整 `data:image/...;base64,` 前缀发出去；GLM 上游只接受纯 base64。仓库里 GLM 相关逻辑集中在其它模块（vision 的图片处理模式、protocol_proxy 的模型名匹配），没有针对 glm-* 剥离 data URL 前缀的规范化。修法按用户建议做即可，且应限定在模型级开关以免影响标准 data URL 的上游。
- 证据：crates/codex-plus-core/src/protocol_proxy.rs:3869-3887（`image_part_to_chat` 只做形态归一，不剥离前缀；url 为空才返回 None）、同文件 3897-3933（`responses_content_to_chat_content` 对 input_image/image_url 直接调它）；`IMAGE_DATA_URL_PREFIX` 只在 protocol_proxy.rs:5578-5595 的 `redact_image_data_urls` 里作为日志脱敏使用，不参与出站改写。GLM 现有引用仅见 protocol_proxy.rs:6207（模型名匹配）与 vision.rs 的图片处理模式映射。
- 落点：crates/codex-plus-core/src/protocol_proxy.rs:3869（image_part_to_chat）：新增模型感知的 data URL 规范化，仅对 glm-* 把 `data:image/...;base64,<payload>` 截成纯 payload，保留 detail 等其它字段，远程 https URL 不动；模型开关建议复用 vision 里既有的 per-model 配置机制而非硬编码

**#1488** — [Bug]: 供应商测试 测试「OpenAI Official」失败：Base URL 不能为空

- 结论：`confirmed-in-code` / P2 / 置信 high / 工作量 small
- 根因：官方登录模式下供应商测试必然报「Base URL 不能为空」：官方 profile 的 configContents 在归一化时被清空，relay_profile_base_url 于是返回空串，被 test_relay_profile 的空值校验直接拒绝。官方模式本应使用内置 base_url，根本没有输入框，所以用户无处可填。
- 证据：造成空串的入口：apps/codex-plus-manager/src/App.tsx:11972 `configContents: relayMode === "official" && !officialMixApiKey ? "" : profile.configContents \|\| ""`。base_url 解析：crates/codex-plus-core/src/relay_config.rs:3741 `relay_profile_base_url`，对 official 无特殊兜底，取 `provider_base_url` 为空后回落 `profile.base_url`，官方 profile 该字段为空。校验点：同文件 `:856` `let base_url = relay_profile_base_url(profile);` 紧接 `:859` `anyho
- 落点：crates/codex-plus-core/src/relay_config.rs:856-862（test_relay_profile 入口）：官方登录模式（profile.relay_mode == RelayMode::Official && !official_mix_api_key）改为走 ChatGPT 登录态校验而非 HTTP 探测，或用内置 base_url（https://chatgpt.com/backend-api/codex 一类）兜底；同时在 apps/codex-plus-manager/src/App.tsx:7389 对官方 profile 隐藏/禁用测试按钮。

**#1160** — 免安装 codex，后端连接异常，且 codex 版本检测不到

- 结论：`confirmed-in-code` / P2 / 置信 high / 工作量 small
- 根因：确认是本仓缺口：自定义解包目录（…\Codex\app）下 codex_app_version 取不到版本号，导致健康检查显示「未检测到」、插件解锁策略退化为 unknown。现有回退链没有「读解包目录里的 *.manifest 文件名」这一档。
- 证据：crates/codex-plus-core/src/app_paths.rs:676-690 的 codex_app_version 回退链为 codex_package_version → codex_directory_version → codex_version_file，四者全部只认 MSIX 目录名、路径末段目录名与 `version` 文件；全仓 grep「149.0.7827.115.manifest」与「*.manifest 文件名解析」零命中，launcher.rs 测试只有 app_paths_extracts_codex_version_from_portable_version_file（tests/launcher.rs:171）与 directory_version 两档，没有 manifest 文件名档。
- 落点：crates/codex-plus-core/src/app_paths.rs:676-690 —— 在 codex_version_file 之后加一档 fallback：若不大于 12 位数字点分且以 .manifest 结尾的文件存在于 app_dir（有的解包目录放在 app/ 下），取其文件名去掉扩展名作为版本；同时按 is_version_like 校验，避免误吞无关文件。配套在 crates/codex-plus-core/tests/launcher.rs 增加 portable/unpacked 形态用例。

**#862** — Codex 内自建的 SKILL 在 Codex++ 管理中显示为 0

- 结论：`confirmed-in-code` / P2 / 置信 high / 工作量 small
- 根因：用户让 Codex 自己创建 skill 并已落在 skills 目录、Codex 里能看到 45 个，但 Codex++「工具与插件」显示 Skills 为 0；根因是 Codex++ 只列「自家安装的 + 远端仓库缓存的 + 内置 .system」三类，从不扫描 $CODEX_HOME/skills 下的本地 skill 目录。
- 证据：crates/codex-plus-core/src/skills.rs:1-20 的模块头明确写「codex 的 skill 是文件系统约定……它扫描 $CODEX_HOME/skills/<id>/SKILL.md」，但列表实现只遍历 `state.installed`（skills.rs:296-311）与 `self.list_bundled_skills()`（skills.rs:318-350，只读 linked_dir()/.system）。全仓无「扫描 $CODEX_HOME/skills 目录」的代码：`grep -rn "skills" crates/codex-plus-core/src/codex_home.rs` 仅命中一条测试用的 home.join("skills").join("x")（codex_home.rs:235）。管理器侧入口 list_insta
- 落点：crates/codex-plus-core/src/skills.rs — 在 SkillsManager 的列表聚合里增加一趟「扫描 $CODEX_HOME/skills 目录」：read_dir linked_dir()，凡是有 SKILL.md 且 id 不在 installed/bundled 集合里的，作为「本地/外部」条目返回（只读、不参与卸载），并给 SkillEntry 加一个来源字段区分。管理器侧 commands.rs:4964 的 skills_payload 无需改结构，前端按新来源字段展示分组即可。

**#772** — [Bug]: Fast 开着但提示「当前模型未读取」（GPT-5.5 中转）

- 结论：`confirmed-in-code` / P2 / 置信 high / 工作量 small
- 根因：同一份代码里有两个 Fast 支持判定函数，条件不一致：codexServiceTierFastSupportedForModel（实际请求覆写用，会额外查模型元数据里的 priority 声明）与 codexServiceTierFastAvailability（只查内置集合，界面文案与状态用）。后者只看 gpt-5.4/gpt-5.5 与少数官方集合，中转场景下模型名带前缀或走元数据声明时就判成不支持，于是按钮显示未读取/不支持、而实际请求路径判定却是支持的——两个函数对话，用户就看到自相矛盾的提示。
- 证据：assets/inject/renderer-inject/20-menu.js:107 `const codexServiceTierSupportedFastModels = new Set(["gpt-5.4", "gpt-5.5"])`；:418-427 codexServiceTierFastSupportedForModel（集合命中 \|\| 元数据 serviceTiers 含 priority）；:448-454 codexServiceTierFastAvailability 只做 `codexServiceTierSupportedFastModels.has(normalizedModel)`；调用点 assets/inject/renderer-inject/30-service-tier.js:163 / :212 / :235 / :271 / :393 / 
- 落点：assets/inject/renderer-inject/20-menu.js:448-454 —— 让 codexServiceTierFastAvailability 复用 codexServiceTierFastSupportedForModel 的判定（含模型元数据 priority 声明），消除两套判据；改完须跑 node scripts/assemble-renderer-inject.mjs 重新组装

**#586** — [Bug]: 纯API mimo-v2.5-pro web_search 原生工具被翻译为 custom_tool_call，Codex 客户端无法执行

- 结论：`confirmed-in-code` / P2 / 置信 high / 工作量 medium
- 根因：本仓已把 web_search 等内置工具登记为 CodexCustomToolKind::BuiltIn，但回程组装 item 时该枚举值从未被读取，仍然一律产出 custom_tool_call，客户端因此回 unsupported custom tool call: web_search——确认是本仓代码缺陷。
- 证据：protocol_proxy.rs:158 定义 BuiltIn；:4057-4070 在 build_codex_tool_context 里把 type:"web_search"\|"local_shell"\|"computer_use" 登记为 BuiltIn；:4200 detect_codex_custom_tool_kind 也返回 BuiltIn。但全仓 grep「BuiltIn」只有这 3 处定义/写入，零处读取：回程 response_tool_call_item（:5241-5250）只判 is_tool_search_proxy（即 kind==ToolSearch），否则落到 custom_tool_call 分支，tool_call_added_item（:5098-5110）同理。→ BuiltIn 分支是死代码。
- 落点：crates/codex-plus-core/src/protocol_proxy.rs:5098（tool_call_added_item）与 :5241（response_tool_call_item）：在两处 is_tool_search_proxy 判断旁补 BuiltIn 分支，按 spec.openai_name 产出对应的原生 item 类型（web_search → {"type":"web_search_call","call_id","status","action"}；local_shell/computer_use 同理解析），并确认流式增量事件不走 response.custom_tool_call_input.delta。改前需要先确认本仓是纯 API 直连上游时该调用由谁执行（若执行方是 Codex 客户端，客户端本就只认 web_search_call）。

**#2080** — [Bug]: 启动前自动修复历史会话通过快捷方式打开时无效

- 结论：`confirmed-in-code` / P2 / 置信 high / 工作量 small
- 根因：确认是代码里的真实差异：快捷方式启动时若 Codex 已在运行，启动器走「激活已有实例」的早退分支直接 return，跳过了 provider sync 与随后的会话索引修复；而管理工具的重启路径会走完整流程。这解释了「重启能切换、快捷方式不能」。
- 证据：apps/codex-plus-launcher/src/main.rs:88-102：`acquire_single_instance_guard` 返回 None（已有实例）时执行 `activate_existing_codex_app(&options)` 并保存状态后 `return Ok(())`，不进入后续流程。完整路径在 :109-111：`launch_and_inject_with_hooks` 后 `run_periodic_until_exit(..., \|\| repair_session_index_automatically(true))`；修复实现在 :134-149（`repair_session_index_automatically`，`check_setting` 为 true 时读 `provider_sync_enabled`）。管理工具侧入口
- 落点：apps/codex-plus-launcher/src/main.rs:88-102（早退分支需补 provider sync / 修复调用）

**#2072** — [Bug]: v1.2.56 市场迁移后 GitHub 已安装但不在插件列表或会话工具中

- 结论：`confirmed-in-code` / P2 / 置信 high / 工作量 medium
- 根因：确认是本仓改名引入的回归。为避免 codex 保留名，内置市场从 `openai-curated-remote` 改名为 `codex-plus-curated`，清理逻辑只删除了 `[marketplaces.*]` 条目，没有迁移或清理仍指向旧名的 `[plugins."github@openai-curated"]`，于是插件条目悬空、codex 不再列出它。
- 证据：crates/codex-plus-core/src/plugin_marketplace.rs:663-708 `cleanup_managed_reserved_marketplace_configs` 只 `marketplaces.remove(marketplace_name)`，managed_entries 覆盖 `openai-curated` / `openai-api-curated` / `openai-curated-remote`（:677-681），函数内无任何 plugins 侧处理。改名动因见同文件 :13-27 的注释表（`openai-*` 为 codex 保留名，注册其下会被静默忽略，issue #1974 / #1968）。相关提交 `02a23e14 fix(plugins): 换掉被 codex 保留的 marketplace 名`（2026-0
- 落点：crates/codex-plus-core/src/plugin_marketplace.rs:663-708（需在清理市场条目时迁移/清理悬空的 plugins 条目）

**#959** — [Question]: 用 DeepSeek 这种纯文本模型时，只要在项目文件夹和聊天里面有图片，对话就会出现报错

- 结论：`confirmed-in-code` / P2 / 置信 high / 工作量 trivial
- 根因：这是已实现的 per-model 图片处理能力：DeepSeek 等纯文本模型收到图片块会 400，Codex++ 提供「按模型配置图片处理」三档（原样发送 / 剥离占位符 / VLM 转述）。用户未配置时默认是 send-as-is，因此仍会把图片原样发给纯文本模型而报错。
- 证据：crates/codex-plus-core/src/vision.rs:109-119 ImageHandling 枚举三档 send-as-is/strip/vlm，:161 image_handling_mode 未命中时回退 SendAsIs；:188 strip_images_only 把图片块替换为「[图片已省略]」；per-model 查表 key 归一化在 :126-155（配套修复 ecb71213 / 064775ab，issue #2345）。前端入口为模型行「模型配置」图片处理选项。
- 落点：crates/codex-plus-core/src/vision.rs:161（image_handling_mode）+ 管理端模型配置面板

**#944** — [Bug]: 在codex中glm-5.1无法开启思考

- 结论：`confirmed-in-code` / P2 / 置信 high / 工作量 small
- 根因：GLM 走 Chat Completions 上游时被归入 Thinking 方言，Codex++ 会据请求体的 reasoning 字段生成 thinking 参数；用户看到 thinking.type=disabled 说明该请求没有被识别为「已开启推理」。当前代码里 reasoning_requested 会把 reasoning.effort 为 none/off/disabled 或 reasoning 缺失/为 null 判为未开启——需要用户侧的 reasoning 字段实况才能定位是 Codex 端未下发还是我们判反。
- 证据：crates/codex-plus-core/src/protocol_proxy.rs:6117-6151 apply_chat_reasoning_options：reasoning_enabled 为假时写 "thinking":{"type":"disabled"}；:6193-6213 infer_chat_reasoning_style 把含 glm / zhipu / z.ai / kimi / moonshot / mimo 的模型判为 ChatReasoningStyle::Thinking；:6182-6191 reasoning_requested 的判定口径（effort 为 none/off/disabled，或无 reasoning/为 null → 未开启）。用户版本 1.2.5 早于 e94efdb4（2026-06-01，含于 v1.1.9）之后的多轮 t
- 落点：crates/codex-plus-core/src/protocol_proxy.rs:6117-6151, 6182-6213

**#1463** — Fast service tier UI misdetects support after switching a thread model mid-session

- 结论：`likely-in-code` / P2 / 置信 high / 工作量 medium
- 根因：真实缺陷：线程中途从 gpt-5.6-sol 切到 gpt-5.5 后，Fast 档位的可用性判定仍用全局 catalog 的模型（gpt-5.6-sol），UI 显示「不支持」，而实际 turn/start 请求用的是线程当前模型。作者自己抓到了对照证据：请求侧 fastSupported=true，UI 侧仍按全局模型判。
- 证据：判定函数 assets/inject/renderer-inject/20-menu.js:448 `codexServiceTierFastAvailability(modelName = codexServiceTierCurrentModelName())`，默认值走 `codexServiceTierCurrentModelName()`（:411）= 全局 `codexModelCatalog.model \|\| default_model`；而调用方 assets/inject/renderer-inject/30-service-tier.js:212、:235、:271、:393、:417 全部**不传参**，于是永远按全局模型判。对照：请求侧 :441 `const fastSupported = !requestedFast \|\| codexServiceTier
- 落点：assets/inject/renderer-inject/30-service-tier.js:212/:235/:271/:393/:417 把当前线程的 active model 传进 codexServiceTierFastAvailability(activeModel)；active model 需从线程状态（thread_settings_applied / turn_context 一类事件）读取并缓存，不要回落到全局 catalog。改完跑 node scripts/assemble-renderer-inject.mjs 重新组装。

**#1316** — 移动会话失败：未找到 Codex App asset: vscode-api-

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 small
- 根因：错误串确认出自本仓注入脚本的 asset loader（assets/inject/renderer-inject/20-menu.js:188）。Codex 侧改名/换 hash 后按前缀名找不到模块就会整条流程失败。属于本仓可改进点：把「可选依赖」的失败降级而不是让移动操作失败。
- 证据：assets/inject/renderer-inject/20-menu.js:188 `if (!url) throw new Error(\`未找到 Codex App asset: ${namePart}\`)`；同文件 :205 loadOptionalCodexAppModule 只在匹配该错误串时返回 null，说明「可选」的判定靠字符串而非结构化标记，调用方容易漏用；相关测试 crates/codex-plus-core/tests/cdp_bridge.rs:3343-3387 断言前缀列表。
- 落点：assets/inject/renderer-inject/20-menu.js:188 附近：移动会话链路改为走 loadOptionalCodexAppModule（或捕获该错误串后降级），并把 asset 前缀探测改成结构正则而非固定 "vscode-api-" 前缀；改后必须跑 node scripts/assemble-renderer-inject.mjs 重新组装，并同步更新 tests/cdp_bridge.rs 的断言。

**#1297** — responses→chat 转换未处理图片、文件，只能纯文本

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 large
- 根因：属本仓协议翻译层的能力缺口：responses→Chat Completions 转换里对多模态内容未做映射，非官方模型下图片/附件会丢失。可从代码层确认缺口存在（转换侧重文本）。
- 证据：crates/codex-plus-core/src/protocol_proxy.rs（6376 行）为协议翻译层；UPSTREAM_IMAGE_HEADER_TIMEOUT（:35）与 EXTRA_CHAT_PASSTHROUGH_FIELDS（:39）显示图片链路与字段透传是分开处理的，转换侧未见 responses 的 input_image/附件到 chat content 数组的完整映射。同类多模态问题在仓库有独立模块 vision.rs。
- 落点：crates/codex-plus-core/src/protocol_proxy.rs 的 responses→chat 转换函数（把 input_image / 附件转成 chat 的 content 数组 image_url/多段结构）；需同时补 tests/protocol_proxy.rs 用例。

**#1424** — [Bug]: 智能体环境为 WSL 时修复会话功能失效

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：WSL 智能体环境下会话修复失效。本仓已有 WSL 路径视角互转（/mnt/c ↔ C:/）的能力，但只接入了会话删除与撤销的备份校验链路（#162），修复会话走的是 sessions 目录直读，未做同样的视角互转，因此 WSL 下找不到 rollout 文件。
- 证据：crates/codex-plus-data/src/storage.rs:1374 wsl_path_alternative 定义互转；storage.rs:1216 与 1339 两处调用点都在删除/撤销的 __files 备份与校验路径；storage.rs:1320 注释明确「issue #162：WSL 模式下 codex 写入 threads.rollout_path 的是 WSL 视角路径」。修复会话侧 crates/codex-plus-data/src/provider_sync.rs:14 SESSION_DIRS 直接按 sessions/archived_sessions 读取，grep provider_sync.rs 中 wsl/WSL 零命中。
- 落点：crates/codex-plus-data/src/provider_sync.rs：会话/rollout 路径解析处复用 storage.rs 的 wsl_path_alternative（需提升为 pub(crate)），在按 rollout_path 读取失败时尝试另一种视角；SESSION_DIRS 扫描同样需要覆盖 WSL 侧路径

**#1141** — 窗口焦点变化或者移动鼠标指针时主窗口未预期闪烁

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：这是注入层与焦点/可见性反复交互的已知脆弱面：注入脚本在页面里监听可见性变化并重建 UI，且历史上多次出现「CSS 预隐藏规则误伤」类缺陷，与用户描述的「切焦点就闪一下」吻合，属本仓渲染层的可修问题。
- 证据：注入层相关缺陷与修复集中在同一族：b6044eb3「精准化额度横幅匹配与 CSS 防闪烁规则，避免误伤普通通知」、396c2b33「完善输入框额度横幅及父容器的 CSS 预隐藏规则消除闪烁」、4cbaa51d「防止新版防闪烁 CSS 误拦截非额度提示横幅」，说明「预隐藏 + 命中后显示」这套机制（用于消除闪烁）在命中判定失败时反而会造成一次可见的闪动；可见性监听见 assets/inject/renderer-inject/60-plugin-marketplace.js:1427/1441 的 visibilitychange 处理。
- 落点：assets/inject/renderer-inject/10-style.js 的预隐藏规则段（约 :1150-1250）与 60-plugin-marketplace.js:1427-1441 的 visibilitychange handler —— 需要确认预隐藏规则在窗口失焦/重新可见时是否被重新应用并立即撤销，导致一次空帧。改动后须跑 node scripts/assemble-renderer-inject.mjs。

**#875** — [Bug]: 删除会话失败 Database not found: C:\Users\Happier\.codex\state_5.sqlite

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 small
- 根因：删除会话报 `Database not found: C:\Users\Happier\.codex\state_5.sqlite`，说明写入 storage 适配器的 db_path 与该机器实际的 Codex 数据库位置不一致。本仓确实有多处「解析 sqlite home」的历史修复（含 CODEX_HOME / CODEX_SQLITE_HOME / threads 与 sessions 二选一），但当前路径组合未覆盖该用户环境，报错文案本身也是硬编码 `db_path` 而非给出候选路径，属可改进的代码缺陷。
- 证据：错误文案出自 `crates/codex-plus-data/src/storage.rs:184`（`format!("Database not found: {}", self.db_path.to_string_lossy())`，`delete_local` 入口判存在性）与 `:400`（用量历史路径）。相关既有修复（均进正式版）：`5338bdd9`「fix: honor CODEX_SQLITE_HOME for session databases」、`f163b379`「fix: resolve sqlite home across session/thread reference/logs」、`6fa0a57d`「修复多数据库会话撤销 (#1560)」、`e0d77c33`「fix: prefer threads database for session path sel
- 落点：`crates/codex-plus-data/src/storage.rs:181-186`：把「Database not found」从单一路径改为「列出所有候选路径 + 实际探测结果」，并复核 `crates/codex-plus-core/src/codex_sqlite.rs` 的 `resolve_sqlite_home`（`:68`，读 `CODEX_SQLITE_HOME`）在 Windows 下的候选顺序是否漏了 `state_5.sqlite` 之外的库名。

**#985** — [Bug]: 下载 arm64 版本的 codex++ 之后 codex++.app 仍然是 intel 版本

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 small
- 根因：发布侧确实按 arch 分两个 job 交叉编译并分别打包（文件名带 arm64），但需要确认 App 内二进制是否真为 arm64 切片，以及管理工具「检查更新」是否可能把 arm64 用户指向 x64 资产；打包脚本只按传入的 arch 命名，不校验产物架构，存在「名对内容错」的可能。
- 证据：.github/workflows/release-assets.yml:73-96 矩阵含 arm64/aarch64-apple-darwin，:110 用 `--target ${{ matrix.target }}` 构建、:118 把 target 目录交给 package-dmg.sh；但 scripts/installer/macos/package-dmg.sh:5-10 只用 `ARCH` 变量拼 DMG 文件名（`CodexPlusPlus-${VERSION}-macos-${ARCH}.dmg`），全脚本无 `lipo`/`file` 之类的架构校验。更新侧 crates/codex-plus-core/src/update.rs:152-169 `select_update_asset` 用 platform_asset_rank（:347-364）优先原生架构，
- 落点：scripts/installer/macos/package-dmg.sh：打包后加一步产物校验（对 Contents/MacOS 下二进制跑 `file`/`lipo -archs`，与 ${ARCH} 不符即 exit 1），CI 侧在 release-assets.yml 的 Verify macOS bundle structure 步骤使用同一断言

**#641** — [Bug]: 经常几个问题后出现 Expecting property name enclosed in double quotes

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：报错文本来自上游网关对非法 JSON 的 400 回应，但触发链是本仓 protocol_proxy 把工具调用 arguments 原样拼接转发；若上游返回的 arguments 分片本身不是合法 JSON 前缀，就会被下游拒绝。
- 证据：protocol_proxy.rs:5723、5733 对 arguments 做 serde_json::from_str 并已加兜底（解析失败包装成 {"input": 原文}，见 tests/protocol_proxy.rs:5059-5064）；但流式 delta 拼接路径 push_tool_call_delta_into（protocol_proxy.rs:2511 起）逐片累积字符串，未在拼接过程中做合法性校验，坏分片会被原样传出。错误消息本体 "Expecting property name enclosed in double quotes" 出自第三方网关的 Python json 解析。
- 落点：crates/codex-plus-core/src/protocol_proxy.rs:2511 push_tool_call_delta_into —— 在累积 arguments 时同时维护一份可解析性检查，对首个坏分片做一次「整体重编码」而不是原样透传

**#618** — [Config]: 供应商切换回填失败：config.toml TOML 解析失败

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 small
- 根因：回填当前供应商配置时直接对 live config.toml 做整份 TOML 解析，用户手工编辑出的非法 TOML 会让整个回填流程失败并弹错；调用链上有两处未做容错。
- 证据：relay_config.rs:1622 parse_toml_document 对非空内容失败即 `anyhow::bail!("config.toml TOML 解析失败：{error}")`。对比之下，同文件的另外两处读取点已有容错：relay_config.rs:2089 `let Ok(live_doc) = parse_toml_document(&live_text) else { return Ok(...) }`（preserve_live_app_settings 在 live 解析失败时降级而非报错）；relay_config.rs:2350 apply_external_catalog_fallback 同样降级。前端 App.tsx:3085-3102 snapshotActiveRelayFilesBeforeSwitch 在 !isSuccessStatus 
- 落点：apps/codex-plus-manager/src-tauri 的 backfill_relay_profile_from_live 命令（经 relay_config.rs 的读取路径）应比照 relay_config.rs:2089 的写法，在 parse 失败时降级返回原 settings 并给出「config.toml 有语法错误，请先修复」的可操作提示，而不是让整条切换流程失败

**#609** — [Bug]: Codex++ 重写 config.toml 并清空 bundled 插件缓存

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 small
- 根因：切换/启动时重写 config.toml 会丢掉 plugin/marketplace/mcp_servers 段，插件变 not installed；仓库已逐步补齐各类保留逻辑，但「plugins 段」在 preserve_live_app_settings 的保留清单里未见显式处理，需确认是否仍会丢。
- 证据：preserve_live_app_settings（relay_config.rs:2082-2130）目前的保留清单为：desktop、sandbox_mode、approval_policy、sandbox_workspace_write、windows、mcp_servers（repair_mcp_servers_from_live，注释 #2263）、features（preserve_missing_table_keys）、hooks 定义与 state。清单中**没有 plugins / marketplace 段**，也未见到针对 `[plugins."..."]` 的补回调用。已有 0fd58cc3 处理 mcp_servers 保留、68429a26（未发版）处理 ChatGPT 登录态保留。plugin_marketplace.rs 负责 marketplace 配置
- 落点：crates/codex-plus-core/src/relay_config.rs:2082 preserve_live_app_settings —— 比照 repair_mcp_servers_from_live 补一个 plugins 段的「以 live 补缺」逻辑，避免模板里没有 plugins 时把用户的 enabled 插件表整段丢掉

**#597** — [Config]: 添加插件或 MCP 导致已有插件全部消失

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 small
- 根因：新增一条插件/MCP 后已有插件全部消失，最可能是重写 config.toml 时模板里没有 plugins 段、导致 live 的 `[plugins."..."]` 被整段覆盖；mcp_servers 一侧已有保留逻辑（#2263），plugins 一侧未见。
- 证据：preserve_live_app_settings（relay_config.rs:2082-2130）的保留清单含 mcp_servers（repair_mcp_servers_from_live，注释指向 #2263）、features、hooks、desktop、sandbox 系列，**未包含 plugins / marketplace 段**。0fd58cc3 fix(protocol_proxy): chat/completions 转发支持 tool_search，保留 mcp_servers (issue #2263) 只覆盖 mcp_servers。plugin_marketplace 负责 marketplace 注册但不负责保留用户手写的 plugins 表。
- 落点：crates/codex-plus-core/src/relay_config.rs:2082 preserve_live_app_settings —— 增加对 `plugins` 段的 live 补缺（与 repair_mcp_servers_from_live 同形），并在写入前移除模板中可能的空 plugins 表

**#500** — [Bug]: 侧边的Timeline不准，而且时不时会抖动

- 结论：**`needs-investigation`（已订正）** —— 原结论 `confirmed-in-code` 是错的。
- **订正说明（2026-10-03 复核）**：Timeline **已于 v1.2.19（`2eeb4d69`）整体移除**，不是「并入其它分片」：
  - `grep -ic timeline assets/inject/renderer-inject.js` = **0**；各分片同样为 0。
  - `2eeb4d69` 从旧 `renderer-inject.js` 删除 113 行 timeline 代码、新增 0 行，README 同步移除 `Timeline` 一词。
  - 原证据列的提交链（f82a813f、65d550f4、cd07cb03…）都是**删除之前**的历史提交，被误读成「现存实现」。
- **影响**：按原结论会向用户承认「我们这边确实有问题，标记是随对话更新重算的」——而该组件根本不存在。本条目**始终未关闭**（在 needs-investigation 组），因此错误回复从未发出，无实际损害。
- **正确处置**：回复用户说明该功能在 1.2.19 已下线、请确认版本；若仍有需求属新功能提案。
- 原落点（`assets/inject/renderer-inject/` 的 Timeline 标记逻辑）**已不存在**，不可执行。

**#300** — macOS 自动更新只挂载 DMG 未执行安装

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：点「下载并运行安装包」后只挂载 DMG、弹出 Finder，不会自动把 .app 覆盖到 /Applications。仓库里没有找到自动复制并重启的安装逻辑，与该 issue 描述吻合，属于功能缺口（缺失自动安装步骤）。
- 证据：全仓 grep "自动更新" / "install_dmg" / "hdiutil" 相关实现未命中自动安装步骤；.github/workflows/release-assets.yml:141 附近只有 codesign 检查。用户描述「只完成了第 1、2 步（下载、挂载），缺少第 3 步自动覆盖安装」。
- 落点：apps/codex-plus-manager/src-tauri/src/commands.rs（更新命令）—— 需在挂载后增加把 .app 复制到 /Applications 并重启应用的步骤，或用 .app.zip 后台解压覆盖

**#239** — 启动 Codex++ 失败：failed to query CDP targets

- 结论：`confirmed-in-code` / P2 / 置信 medium / 工作量 small
- 根因：Windows 上 Debug 9229 查询不到 CDP target，属于启动/注入链路的已知失败面；错误文本出自本仓 `cdp.rs`。端口虽在 Windows 有让位逻辑，但用户是 1.1.5（早于让位逻辑与 CDP 恢复改动），需按新版复测后判定是否仍有残留缺陷。
- 证据：`crates/codex-plus-core/src/cdp.rs:140`（`failed to query CDP targets on loopback addresses: {}`）与 `cdp.rs:193`（`.context("failed to query CDP targets")`）为本仓产出。Windows 端口让位：`crates/codex-plus-core/src/ports.rs:72-80`（`is_windows` 分支）。相关历史修复 `3ab45574 fix launcher recovery for existing Codex CDP (#1658) (#1659)`（2026-07-27，改动 `ports.rs`/`cdp.rs`/`launcher.rs`），已确认包含于 v1.5.0（`git merge-base --is-ance
- 落点：先复测：升级到 v1.5.0 后重跑，若仍失败则看 `ports.rs:72-80` 是否把端口让到了 9229 之外而状态文件仍报 9229（`status.rs` 的 `LaunchStatus` 无阶段字段，见 docs/reports/2026-10-02-issue-audit-and-fix-plan.md:284）。

**#1650** — 纯官方登录供应商无法持久化/恢复专属 config.toml

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：用户指的是 relayMode=official 且 officialMixApiKey=false 时未走 per-profile 配置恢复链路。仓库确实有相关修复（1b214349 切回官方时移除中转站 provider 整段配置），但该提交解决的是「切回官方残留中转配置」，与用户指出的「官方 profile 自身 configContents 不被恢复」不是同一件事，核心缺陷可能仍在。
- 证据：commit 1b214349 fix(relay): 切回官方时移除中转站 provider 整段配置，并清掉其模型名（2026-09-22，含 v1.4.0/v1.5.0）——处理的是残留方向；crates/codex-plus-core/src/relay_switch.rs:35-46 切换时先 backfill_relay_profile_from_home_with_common 再 apply，回滚已覆盖 settings + live 文件（见 LiveFilesSnapshot），但正文提到的「纯官方登录不读取自身 configContents」这条链路未见对应修复提交。git log --all --grep="#1650" 零命中。
- 落点：待定：crates/codex-plus-core/src/relay_switch.rs:35-46 与 relay_config.rs 的 backfill/apply 路径，需先复现确认 official 模式是否跳过 configContents 恢复

**#1498** — 界面 UI 频闪，切浅色主题显示异常（只有聊天框白，其余黑字）

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 small
- 根因：浅色主题下配色错乱、界面频闪。仓库有直接对应的修复：43e6c55c「浅色主题不再黑底黑字，换掉整层在 Codex 产物里不存在的死 token」（issue #2359），以及额度横幅相关的防闪烁规则。但用户是 1.2.35，需确认这些是否已发布。
- 证据：commit 43e6c55c fix(inject): 浅色主题不再黑底黑字，换掉整层在 Codex 产物里不存在的死 token (issue #2359)；配套 10ff2fe6 fix(inject): 补漏 #2359 三处残留死名（hover 态与 tooltip）；频闪侧有 4cbaa51d/396c2b33/b6044eb3 的 CSS 预隐藏规则。这些提交需 git tag --contains 核实是否入 v1.5.0（#2359 编号远大于 v1.5.0 同期，疑似未发布）。
- 落点：assets/inject/renderer-inject/ 配色与防闪烁分片（需 assemble-renderer-inject.mjs 重组）

**#1341** — [Question]: 点击会话管理的调度问题

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：会话数据库数据量大时，点一次会话管理就同步一次，且同步疑似跑在主线程上，导致整个程序卡住。这条描述具体、可验证，指向同步是否离开 UI 线程。
- 证据：apps/codex-plus-launcher/src/main.rs:454 `tokio::task::spawn_blocking(\|\| codex_plus_data::run_provider_sync(None))`、:471 与 :738 也是 spawn_blocking —— 同步本身是放在阻塞线程池跑的。但 Tauri 命令层（apps/codex-plus-manager/src-tauri/src/commands.rs）中触发同步的那个命令是否 await 而非阻塞、以及点击是否每次都全量跑，需要逐个命令确认，本批未逐行核对。
- 落点：apps/codex-plus-manager/src-tauri/src/commands.rs（会话同步命令）+ 前端点击处理

**#1329** — [Bug]: Codex++无法通过系统代理连接到GitHub

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 small
- 根因：脚本市场加载失败、检查更新失败，因为 Codex++ 不走系统代理。代码里确实有系统代理探测（detect_system_proxy），但出网用的 reqwest client 只设了 user_agent，没有把探测到的代理挂上去——默认只认环境变量。
- 证据：crates/codex-plus-core/src/proxy.rs:13 `pub fn detect_system_proxy()` 存在，并有完整的平台实现（Windows 读注册表 Internet Settings 的 windows_system_proxy()，macOS 走 scutil --proxy 的 parse_macos_scutil_proxy()），说明系统代理是被探测的；但 crates/codex-plus-core/src/http_client.rs:1-8 的 `proxied_client` 只有 `.user_agent(ua)`，没有 `.proxy(...)`——全仓 grep `.proxy(` 在 crates/ 与 apps/ 的 Rust 源码里零命中。reqwest 的默认行为是读 HTTP(S)_PROXY 环境变量，因此「开了
- 落点：crates/codex-plus-core/src/http_client.rs:proxied_client —— 把 detect_system_proxy() 的结果显式挂到 reqwest builder 上

**#1905** — 主题/皮肤重启后失效，每次都要重新应用

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：皮肤应用状态在重启后没有从持久设置里恢复，需要重新应用才生效，指向 dream_skin 持久化/恢复链路。
- 证据：crates/codex-plus-core/src/dream_skin_runtime.rs:223/285 的生效判定依赖 `settings.codex_app_dream_skin_enabled` 等设置；同文件 237/291 有暂停分支；无对应修复提交（git log --grep 1905 无命中）。此前审计 docs/reports/2026-09-11-open-pr-and-issue-triage.md:132 把 #1905 归入「皮肤/汉化/UI」族，仅部分随 #1712 Dream Skin 对齐修掉。
- 落点：crates/codex-plus-core/src/dream_skin_runtime.rs —— 启动时按持久设置重新注入，而非依赖上次会话的运行时状态

**#1893** — 强制 codex 界面中文失效

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 small
- 根因：强制中文依赖写入 Codex 的 localeOverride 设置并触发一次页面重载；在 api-key 模式重启后落回英文，可能是设置写入未生效或重载标记（sessionStorage）在新会话里丢失导致跳过重载。
- 证据：assets/inject/renderer-inject/00-prelude.js:97-256 `installCodexPlusForceChineseLocale`：通过 electronBridge 调 `set-setting key=localeOverride`，并用 sessionStorage 的 `codexPlus.forceChineseLocale.reload.v1` 标记防止重复 reload（写入失败即 return，不刷新）；crates/codex-plus-core/src/assets.rs:731-732 注入 `{enabled, locale: zh-CN}`（settings.rs:661 默认 true）。sessionStorage 在重启后为空，若设置写入被 api-key 模式覆盖，则不会再有重载动作。
- 落点：assets/inject/renderer-inject/00-prelude.js:97 起 —— 重载/重试路径；核对 api-key 模式重启后 localeOverride 是否被覆盖

**#1784** — 换肤功能无法使用（原生输入框不可见 / 首页横幅不匹配）

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：皮肤校验报「原生输入框不可见」「首页横幅不符合目标项目要求」，属于皮肤选择器锚点随 Codex 版本漂移失配；此前审计已把该族归为「版本漂移」，仅部分随 Dream Skin 对齐修复。
- 证据：docs/reports/2026-09-11-open-pr-and-issue-triage.md:132 把 #1784 列入「皮肤/汉化/UI（#2064 #1905 #1893 ... #1784 ...）」并注「#1712（Dream Skin 对齐 26.825）已合，剩余多为版本漂移」；仓库侧皮肤模块为 crates/codex-plus-core/src/dream_skin*.rs（含 community/library/market/package/runtime）。
- 落点：crates/codex-plus-core/src/dream_skin.rs 的校验锚点 —— 按 Codex 新版 DOM 结构调整选择器（优先 data-* 锚点）

**#1213** — [Bug]: agnes 支持 512 了，默认显示还是 256 上下文

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 small
- 根因：仓库内置模型元数据里没有 agnes 条目，因此该模型无法识别窗口、回落到默认值并在 256K 处触发压缩。需要补充该模型的 context_window 元数据。
- 证据：全仓 grep `agnes`（crates/apps/assets，排除 node_modules）零命中。内置元数据位于 crates/codex-plus-core/assets/codex-models.json 与 assets/deepseek-model-metadata.json，每条含 context_window 与 web_search_tool_type；按模型覆盖窗口的机制见 crates/codex-plus-core/src/model_suffix.rs 的 model_list 后缀语法（slug[1M]）与 model_catalog.rs。
- 落点：crates/codex-plus-core/assets/codex-models.json（新增 agnes 条目，填 context_window 与对应 auto_compact_token_limit）；短期用户可用 model_list 后缀 `agnes[512K]` 自行声明窗口，路径 crates/codex-plus-core/src/model_suffix.rs:28 的 parse_model_suffix。

**#1226** — [Bug]: Codex 26.623.42026 版本 连接 deepseek

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 small
- 根因：chatcompletions 走本地 57321 得到 502，说明协议代理没起来或转发挂了；而 responses 直连 api.deepseek.com/responses 得到 404，是 DeepSeek 官方根本没有 /responses 端点——两部分要拆开看，前半段像本仓问题。
- 证据：502 诊断已在 main 补上：crates/codex-plus-core/src/launcher.rs:2257-2280 `log_helper_proxy_failure`（f55bb646「fix(launcher): 代理 502 的失败原因写入诊断日志 (issue #2385)」），但该提交 `git tag --contains f55bb646` 为空，即晚于 v1.5.0，用户拿不到。第二个错（404 https://api.deepseek.com/responses）是用户把 wire_api 设成 responses 直连官方 DeepSeek，官方无该端点，确属配置/上游问题。
- 落点：前半段：升级到含 f55bb646 的版本后按诊断日志定位；后半段：改用 Chat Completions 上游（crates/codex-plus-core/src/relay_config.rs:558 `apply_deepseek_responses_compatibility` 已对 DeepSeek 做 Responses 兼容处理）

**#982** — [Bug]: 历史会话记录修复失败（从第三方切到官方后记录消失）

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：「同步一次：0 个会话文件，0 行索引」说明同步器判定无需改动或扫描不到目标范围内的会话，随后官方登录下看不到历史；这既可能是会话文件确实不属于可迁移范围（encrypted_content 保护），也可能是扫描/筛选条件过严，现有信息指向后者需实测。
- 证据：提示文案来自 apps/codex-plus-manager/src-tauri/src/commands.rs:3668-3674 的 provider_sync_command_result；同步器对跨供应商历史有明确的能力边界与提示，见 crates/codex-plus-data/src/provider_sync.rs:3323-3342 的 build_encrypted_content_warning（检测到别的供应商的 encrypted_content 时只同步可见元数据并告警）。批量同步已改为两阶段流式（333ffb98，PR #2084）。
- 落点：待确认：crates/codex-plus-data/src/provider_sync.rs 的目标会话筛选（changed_session_files 恒为 0 时是否漏了 custom→openai 的候选集）；需拿到该用户的 .codex/sessions 目录结构与诊断日志才能定论

**#979** — [Bug]: Macbook 盒盖导致后端连接断开

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：合盖唤醒后后端变红，与 #1032 同源（窗口隐藏/后台时桥接被误判失效），但 macOS 合盖还会触发代理进程休眠/端口重建，需单独确认是否需要额外的唤醒重连。
- 证据：同源的桥接误判修复 5bb4f636（未发布，`git tag --contains` 空）已把「窗口隐藏不判定桥接失效」纳入；但该提交只覆盖可见性判据，未见对 macOS 休眠唤醒后本地代理端口重建的处理。
- 落点：assets/inject/renderer-inject/40-backend-settings.js:110-134（状态机）与 launcher 侧代理进程：唤醒后应重新探测本地代理端口并强制重注入一次；先由 5bb4f636 发版验证是否已足够

**#975** — [Bug]: 模型映射后 codex 内模型没有改变或者消失

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：与 CCswitch 的模型映射共存时，Codex 内模型列表消失或未变；本仓对「外部已有 model_catalog_json 指针」的处理正是这条冲突路径，已有一次降级修复（未发布）。
- 证据：提交 ca69f48e「fix(relay): 外部 model_catalog_json 冲突时降级而非拒绝切换 (issue #2203)」把两处 bail! 改为 apply_external_catalog_fallback：保留用户手写的外部指针原样不动，改在 config.toml 顶层写 model_context_window / model_auto_compact_token_limit 兜底；`git tag --contains ca69f48e` 为空（未发布）。相关实现入口 crates/codex-plus-core/src/relay_config.rs:2362 apply_model_catalog_to_config。
- 落点：已在 ca69f48e 修（待发版）；若要真正「与 ccswitch 兼容」还需后续支持合并外部 catalog 条目而非仅降级到顶层窗口值

**#1082** — [Bug]: 代理配置后 glm / minimax 等模型无法使用（Unsupported parameter）

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：代理层按模型名做「推理方言」推断，glm/mimo 被判为 Thinking（于是向上游发 thinking），minimax 被判为 ReasoningSplit（发 reasoning_split）；而用户接入的那条上游不认这些参数，返回 Unsupported parameter(s)。方言是按名称子串猜的，模型名撞上规则就会误发，且 profile 侧没有显式覆盖方言的开关。
- 证据：crates/codex-plus-core/src/protocol_proxy.rs:6193-6224 infer_chat_reasoning_style：`contains("glm")\|\|contains("mimo")` → Thinking；后面 6214 `contains("minimax")` → ReasoningSplit。注入点 6117-6150 apply_chat_reasoning_options：Thinking 分支写 result["thinking"]，ReasoningSplit 分支写 result["reasoning_split"]。用户报错原文 'Unsupported parameter(s): `thinking`' 与 'Unsupported parameter(s): `reasoning_split`' 正对应这两处。s
- 落点：crates/codex-plus-core/src/protocol_proxy.rs:6193 infer_chat_reasoning_style —— 建议：(1) 把「按名字猜方言」降级为默认值，并在 RelayProfile 增加 per-model 的方言/参数白名单覆盖（settings.rs:28 起），UI 可留空即沿用推断；(2) 对返回 400 Unsupported parameter 的上游做一次性重试：剥掉 thinking/reasoning_split/reasoning_effort 后重发，避免用户需要手工排查。

**#1036** — [Bug]: 未检测到 Codex 应用版本，打不开 codex++ 只能打开管理工具

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 small
- 根因：用户把 Codex 装在了 C:\codex++ 目录，诊断里 codex_app 显示 found 但 codex_version 为 null，且启动报 'failed to launch Codex executable C:\codex++\codex.exe'。路径解析对含着「codex++」字样的目录名有明确的排除规则，极可能是这一条把用户的真实 Codex 目录一并排除了。
- 证据：crates/codex-plus-core/src/app_paths.rs:532-569 is_codex_plus_plus_path：逐段检查路径组件，只要任一段（小写后）等于 "codex++" / "codexplusplus" / "codex-plus-plus" 或包含 "codex-plus-manager" 即返回 true → normalize_codex_app_path 在第 515-517 行直接返回 None。用户路径 C:\codex++ 的组件正是 "codex++"，因此被判为「Codex++ 自己的安装目录」而拒绝。版本探测失败（codex_version=null）与启动失败同源于此。
- 落点：crates/codex-plus-core/src/app_paths.rs:532 is_codex_plus_plus_path —— 收紧判定：不能只凭目录名含 'codex++' 就排除，应同时要求该目录里确实存在本产品可执行文件（如 codex-plus-plus-manager.exe / uninstall.exe + 本产品注册表键），否则视为用户自定义路径放行；同时建议在路径解析失败时把「被判定为 Codex++ 安装目录而跳过」写进诊断日志，避免这类静默拒绝。

**#495** — [Bug]: 插件栏无法激活

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 small
- 根因：重启 Codex++ 或通过管理工具启动后都无法激活插件栏。插件解锁依赖对客户端压缩产物中某个 filter 回调的形态匹配，客户端一升级形态变化就会失灵；仓库在 v1.5.0 之后刚改过这处识别方式（改用结构正则），但该修复尚未发布，所以用户装 v1.5.0 仍会踩到。
- 证据：assets/inject/renderer-inject/60-plugin-marketplace.js:1-6 注释明确「结构式匹配 <arr>.filter(p => !<list>.includes(p.name))，必须锚定 filter 箭头形态且箭头参数与 .name 的宿主同名」，:8-15 isCodexPluginMarketplaceHiddenFilter、:22-52 patches Array.prototype.filter 并发送 plugin_marketplace_hidden_filter_bypassed 诊断。修复提交 241d6ca4（2026-10-02 fix(inject): 插件解锁改用结构正则识别 marketplace 过滤器），git tag --contains 241d6ca4 无 v1.* 命中，即晚于 v1.5.0、尚未发
- 落点：assets/inject/renderer-inject/60-plugin-marketplace.js:1-52（修复已存在，需发版送达用户）

**#728** — 长时间挂着 codex 不动会导致后端连接失败

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 n/a
- 根因：长时间挂后台后后端连接失败、且「后端修复运行」按了也没用，只能重启——与已定位的「窗口隐藏被判为桥接失效、重注入退避被永久清零」链路高度吻合，但缺现场日志。
- 证据：同链路根因已确认并修复：5bb4f636（2026-10-02）提交正文说明「窗口最小化后 Chromium 把后台定时器钳到约每分钟一次，注入脚本的 5s 心跳停摆，而 bridge_health_check_script 只看 lastInjectionAt/lastSuccessAt/lastAttemptAt、没有 visibilityState 判据，于是每轮探测都判失效…结果是每分钟重发整份 616KB 脚本」，明确挂靠 issue #2330 / #2169 / #2267 / #2201 同一链路；对应代码 crates/codex-plus-core/src/bridge.rs 的 bridge_health_check_script 加 hidden 早返回、crates/codex-plus-core/src/launcher.rs 的 BridgeReinjectB
- 落点：已修在 main（crates/codex-plus-core/src/bridge.rs bridge_health_check_script 的 hidden 早返回、launcher.rs 的 BridgeReinjectBackoff.healthy_since），待下一次发版

**#702** — 用 codex++ 启动时点设置会卡很久、页面卡住、codex++ 显示红色

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 n/a
- 根因：打开设置页时长时间卡顿、连接状态转红后自行恢复——与已定位的「重注入强占主线程 / 全量 rescan」链路吻合，且该修复尚未发布。
- 证据：同链路根因已确认并修复：5bb4f636（2026-10-02）提交正文——『窗口最小化后 Chromium 把后台定时器钳到约每分钟一次…结果是每分钟重发整份 616KB 脚本，连带全量 asset rescan』，明确挂靠 #2330 / #2169 / #2267 / #2201；修法为 crates/codex-plus-core/src/bridge.rs 的 bridge_health_check_script 加 hidden 早返回 + crates/codex-plus-core/src/launcher.rs 的 BridgeReinjectBackoff 新增 healthy_since（持续健康 60s 才清零退避）+ assets/inject/renderer-inject/20-menu.js 的 asset 查找缓存挂 window。该提交**不在 v1.5
- 落点：已修在 main：crates/codex-plus-core/src/bridge.rs（hidden 早返回）、crates/codex-plus-core/src/launcher.rs（BridgeReinjectBackoff.healthy_since）、assets/inject/renderer-inject/20-menu.js（asset 查找缓存），待发版

**#2156** — v1.3.0 在单个请求里发出两份重复 MCP namespace，上游拒绝

- 结论：`likely-in-code` / P2 / 置信 medium / 工作量 medium
- 根因：报告成立，但影响面比正文描述的小：chat/completions 转发路径已经有 namespace 去重，只有 `wire_api = "responses"` 的直通路径会把客户端原始（含重复）的 input 原样发给上游。直通是刻意设计，所以「在本仓去重」这条路要慎重。
- 证据：crates/codex-plus-core/src/protocol_proxy.rs:4700-4729 `collect_tool_search_output_namespaces` 用 `seen: HashSet<String>` 按 namespace 名去重，:4732 `dedup_chat_tools_by_name` 再按函数名去重；两处在 `responses_to_chat_completions` 中于 :298-310 调用。而 responses 直通分支（protocol_proxy.rs:1195 `if wire_api == UpstreamWireApi::Responses`）不做 input 改写。重复的 `tool_search_output` item 由 Codex 客户端自己按「每轮检索命中重新追加」生成，本仓只在中转时参与。
- 落点：crates/codex-plus-core/src/protocol_proxy.rs:4700-4744（已有去重）+ responses 直通分支 :1195

**#1062** — [Bug]: 几轮对话之后出现 BadRequestError（Expecting property name enclosed in double quotes）

- 结论：`likely-in-code` / P2 / 置信 low / 工作量 medium
- 根因：多轮之后上游报 JSON 解析失败（line 1 column 2 char 1），与 #1050 是同一类症状：工具调用的 arguments 字符串在往返过程中被写坏成非法 JSON，下一次请求带着坏数据发出去就 400。两轮之后才出现，符合「历史里的工具调用被回传」这个触发条件。
- 证据：同批 #1050 提供了更详细的损坏形态（arguments 内引号未转义 + 末尾多出 {}），且用户自述修好该 jsonl 后错误消失——指向同一根因。相关代码：crates/codex-plus-core/src/protocol_proxy.rs:3374 与 3661-3662 都在处理「带 tool_calls 的 assistant 消息回传」的特殊要求（DeepSeek thinking 模式等），说明这条往返路径确实有做格式化改写，是可能的污染点；另有 relay_config.rs:2978f '模型名统一 sanitize，阻断被污染 model 的转义放大循环' 表明历史上出现过转义/污染放大问题。
- 落点：crates/codex-plus-core/src/protocol_proxy.rs:3374 与 3661（tool_calls / reasoning_content 回传构造）—— 建议在序列化 assistant 工具调用时统一走 serde_json，禁止字符串拼接；并加一条断言/自检：回传前对 arguments 做一次 JSON 解析，不合法则修复或降级为文本。先需要用户提供请求体样本确认污染点。

**#406** — MAC M1 升级 1.1.8 后对话 502 Bad Gateway（127.0.0.1:57321）

- 结论：`likely-in-code` / P2 / 置信 low / 工作量 small
- 根因：502 是本地代理把上游失败原样抛回客户端，真正原因（上游 4xx、Base URL 写错、或协议不匹配）单看正文无法判定；仓库刚补了让失败原因写进诊断日志的改动，但尚未发布，因此现在也无法从日志里读到原因。
- 证据：crates/codex-plus-core/src/launcher.rs:1828 记录 `helper.protocol_proxy_upstream_error`；提交 f55bb646「fix(launcher): 代理 502 的失败原因写入诊断日志 (issue #2385)」(2026-10-03)，`git tag --contains f55bb646` 为空，未进任何正式版。用户配置里 `requires_openai_auth = true` + `experimental_bearer_token` 指向本地代理，属正常形态。同族 issue：b30.json #59、b20.json #738、b11.json #1152。
- 落点：crates/codex-plus-core/src/launcher.rs:1828（已在 main 补齐上游失败原因落日志；本条待新版发布后凭日志定位）

**#692** — renderer-inject.js 硬编码中文显示名导致插件市场标签错乱

- 结论：`confirmed-in-code` / P3 / 置信 high / 工作量 small
- 根因：插件市场显示名被硬编码成中文编号「OpenAI插件1/2/3(Codex++)」，覆盖了上游返回的原始 displayName；函数在现仓库仍原样存在，只是行号已漂移。
- 证据：assets/inject/renderer-inject/50-navigation.js:82-89 定义 displayNameForPluginMarketplaceName，:83 `openai-bundled`→"OpenAI插件1(Codex++)"、:84 `openai-curated`→"OpenAI插件2(Codex++)"、:85 `openai-primary-runtime`→"OpenAI插件3(Codex++)"、:86 `openai-api-curated`→"OpenAI插件4"、:89 codex-plus-curated/openai-curated-remote→"OpenAI插件5"；组装产物 assets/inject/renderer-inject.js:6337-6345，调用点 :6351 用 marketplace.displayN
- 落点：assets/inject/renderer-inject/50-navigation.js:82-90：改回 `return fallback`（或把编号改为英文如 "Official Marketplace"）；同步更新 crates/codex-plus-core/tests/cdp_bridge.rs:2505/2568-2584 的字面量断言，然后跑 node scripts/assemble-renderer-inject.mjs 重组产物。注意该函数同时承担「多个 marketplace 去重展示」职责，纯 return fallback 前需确认不会退回同名重复条目。

**#1975** — 统一侧边栏/列表项右侧状态指示灯右边距

- 结论：`confirmed-in-code` / P3 / 置信 medium / 工作量 trivial
- 根因：侧边栏顶部「Codex++」行的状态灯比下方列表项更靠右，同一垂直线未对齐，是纯 CSS 内边距不一致，修起来很小。
- 证据：app 侧样式集中在 apps/codex-plus-manager/src/styles.css（含 scrollbar-gutter 等布局规则，见 443 / 1124 / 5381 行）；正文准确指出了「顶部指示灯更靠右」的对齐差异。
- 落点：apps/codex-plus-manager/src/styles.css —— 让顶部标题行与列表项容器共用同一右内边距（列表项预留滚动条 gutter 时顶部也要预留）

**#1863** — macOS Dock 中管理工具图标显示为白色

- 结论：`confirmed-in-code` / P3 / 置信 medium / 工作量 small
- 根因：Tauri 打包只配了 icons/icon.ico（Windows 图标），图标目录里没有 macOS 需要的 .icns / 多分辨率 icon.png，Dock 因此取不到有效图标显示为白块。
- 证据：apps/codex-plus-manager/src-tauri/tauri.conf.json 的 bundle 段为 `{"active": false, "targets": "all", "macOS": {"infoPlist": "Info.plist"}, "icon": ["icons/icon.ico"]}`；`ls apps/codex-plus-manager/src-tauri/icons/` 只有 icon.ico(142KB) 与 icon.png(51KB)，无 icon.icns；Info.plist 内无 CFBundleIconFile/CFBundleIconName 键；lib.rs:493-497 会在窗口不可见时把 activation policy 切到 Accessory。
- 落点：apps/codex-plus-manager/src-tauri/tauri.conf.json bundle.icon 增加 icons/icon.icns（并由 icon.png 生成 icns）；icons/ 下补 .icns 文件

**#595** — [Bug]: 无法切换回官方登录态，中转界面没有清除 api 选项

- 结论：`confirmed-in-code` / P3 / 置信 medium / 工作量 medium
- 根因：用户切到中转后回不到官方登录态，且界面缺少「清除 API 配置」入口；切换回官方时对残留中转凭据的清理逻辑存在但覆盖不完整。
- 证据：仓库有邻近修复：cb481938 fix: return OpenAI session to Custom when selecting Chat Completions、68429a26 fix(relay): 纯 API 切换不再抹掉 live 的 ChatGPT 登录态 (issue #2173，未发版)。relay_config.rs:3612-3615 注释提到 auth_contents_looks_like_chatgpt_auth 用于判定 CUA 浏览器插件可用，说明官方态识别依赖 auth.json 内容判定，残留中转字段会干扰。用户版本写的 1.20（应为 1.2.0）。
- 落点：apps/codex-plus-manager/src/App.tsx 供应商/登录态切换区 + crates/codex-plus-core/src/relay_config.rs 的官方态恢复路径（需补一个显式的「恢复官方登录」动作，清掉中转 provider 与凭据键）

**#949** — [Bug]: fast按钮消失了。

- 结论：`confirmed-in-code` / P3 / 置信 medium / 工作量 n/a
- 根因：Fast 按钮的显示受三重门控：功能开关打开、后端已连接、且当前模型在 Fast 支持模型列表内（不支持时按钮变灰并提示「不支持」）。用户开的是纯第三方模型（DeepSeek 等），Fast 支持列表里没有该模型，因此按钮按预期不可用——这与 issue 描述「开了 fast 开关但按钮消失」吻合。
- 证据：assets/inject/renderer-inject/30-service-tier.js:271-273：fastDisabled = !featureEnabled \|\| !backendConnected \|\| status==='loading' \|\| !fastAvailability.supported；:212-214 与 :246-249 在不支持时给出「不支持」档与提示文案；:244 说明 Fast 仅对支持模型发 service_tier="priority"。相关修复 bbfd8121「fix(fast): 从 Fast 支持列表移除 DeepSeek，文案不再做出错误承诺」与 3d6d360d「fix(service-tier): fast 档位补入 gpt-6 系列模型」（v1.5.0）、dc81180d「fix: restrict fast s
- 落点：assets/inject/renderer-inject/30-service-tier.js:212-249、:271-273

**#1876** — 最大化时窗口底部越过 Windows 任务栏

- 结论：`likely-in-code` / P3 / 置信 medium / 工作量 small
- 根因：管理工具窗口最大化时覆盖任务栏，属窗口与 Windows 工作区边界处理问题；当前 tauri.conf 主窗口为 create:false 由 Rust 侧创建，需要在创建/最大化时按工作区（SPI_GETWORKAREA）约束。
- 证据：apps/codex-plus-manager/src-tauri/tauri.conf.json 的 app.windows[0] 为 `"create": false, label "main"`（尺寸仅 1180x820/minWidth 960/minHeight 720，无 maximizable/max 约束相关键）；窗口创建与显示逻辑在 apps/codex-plus-manager/src-tauri/src/lib.rs（focus/unminimize/show/set_focus）。
- 落点：apps/codex-plus-manager/src-tauri/src/lib.rs 窗口创建处 —— 最大化尺寸按 SPI_GETWORKAREA 计算，避免把任务栏算进客户区

---

## 4. 建议关闭清单

共 **197** 条高置信度关闭候选（已逐条校验：duplicate 目标编号真实存在、already-fixed 的提交确认在正式版内）。

| verdict | 条数 |
|---|---:|
| `not-our-bug` | 71 |
| `already-fixed-released` | 67 |
| `invalid-insufficient-info` | 36 |
| `duplicate` | 23 |

### 4.1 已在正式版修复（升级即可关闭）（67 条）

| issue | 主题 | 证据 |
|---|---|---|
| [#162](https://github.com/BigPizzaV3/CodexPlusPlus/issues/162) | WSL 模式下删除会话提示成功，但重启后会恢复 | commit d90c0c05（2026-09-30）「fix(sessions): WSL 模式下删除会话真正删除 rollout 文件 (#162)」，`git tag --contains d90c0c05` 返回 v1.5.0 → 已发布。改动落在 c |
| [#215](https://github.com/BigPizzaV3/CodexPlusPlus/issues/215) | 最新的Codex侧边栏pin按钮与Codex++删除按钮重叠 | `assets/inject/renderer-inject/80-session-share.js:1461-1470` 的 `nativeActionButtonsFromRow` 明确把 `/(pin\|archive\|置顶\|归档)/i` 纳入识别； |
| [#275](https://github.com/BigPizzaV3/CodexPlusPlus/issues/275) | MCP Server tools marked as 'Unsupported' — n | 6b02b74a「fix: 写盘前用 live 补齐 mcp_servers 条目，修掉 node_repl 被打散」，git tag --contains 6b02b74a 含 v1.4.0、v1.5.0；实现见 crates/codex-plus-core |
| [#323](https://github.com/BigPizzaV3/CodexPlusPlus/issues/323) | 加 session 批量删除功能 | `git log -S "批量删除会话" -- apps/codex-plus-manager/src/App.tsx` → 6023b1ba「feat: release 1.2.20」；`git merge-base --is-ancestor 6023b1 |
| [#326](https://github.com/BigPizzaV3/CodexPlusPlus/issues/326) | 希望 manager 与 codex-plus 端口可修改 | apps/codex-plus-manager/src/App.tsx:6819-6832 两个 Field：「Debug 端口」「Helper 端口」；默认值 :1220-1221 `debugPort: "9229"` / `helperPort: "57 |
| [#365](https://github.com/BigPizzaV3/CodexPlusPlus/issues/365) | 供应商配置把本地 config.toml 全部覆盖，尤其是 mcp 配置 | crates/codex-plus-core/src/relay_config.rs:1317 `preserve_unmanaged_live_context_entries`、:1208 `merge_common_config_into_config`（ |
| [#430](https://github.com/BigPizzaV3/CodexPlusPlus/issues/430) | 1.1.8 接入 DSV4 多轮会话后丢失 tool_call_id 导致会话中断 | crates/codex-plus-core/src/protocol_proxy.rs:3489 enforce_tool_call_pairing（收集已回答 id、把无应答的 tool_call 降级而非整轮丢弃）；:3395 relocate_inte |
| [#454](https://github.com/BigPizzaV3/CodexPlusPlus/issues/454) | [Bug]: 无法选择免安装版 Codex 应用路径 | 538e3d9a 'fix(manager): grant dialog plugin capability so file pickers actually open'；capabilities/default.json:7 `"dialog:default |
| [#465](https://github.com/BigPizzaV3/CodexPlusPlus/issues/465) | [Bug]: 安装维护下 Codex 应用路径选择按钮无反应 | 538e3d9a 'fix(manager): grant dialog plugin capability so file pickers actually open'；apps/codex-plus-manager/src-tauri/capabiliti |
| [#507](https://github.com/BigPizzaV3/CodexPlusPlus/issues/507) | [Config]: 无法设置想要的上下文长度（模型显示 256k，实际有 1M） | crates/codex-plus-core/src/model_suffix.rs:1-4（后缀语法与剥离规则）与 :25-40 parse_model_suffix；引入提交 74d0a0d6（2026-06-24），git tag --contains  |
| [#524](https://github.com/BigPizzaV3/CodexPlusPlus/issues/524) | 使用codex++启动语言一直显示英文（挂香港 VPN 直启是中文） | assets/inject/renderer-inject/00-prelude.js:100 读 window.__CODEX_PLUS_FORCE_CHINESE_LOCALE__、:214-237 syncOfficialLocaleSetting 写  |
| [#535](https://github.com/BigPizzaV3/CodexPlusPlus/issues/535) | [Feature Request]: 实现 localeOverride 配置项，支持中 | assets/inject/renderer-inject/00-prelude.js:214/225/237 syncOfficialLocaleSetting 读写 codex setting api 的 localeOverride 并在变更后 relo |
| [#540](https://github.com/BigPizzaV3/CodexPlusPlus/issues/540) | [Config]: deepseekAPI接入无法开启1M上下文 | crates/codex-plus-core/src/model_suffix.rs:1-4 明确「后缀语法：deepseek-v4-pro[1M] 表示 slug=deepseek-v4-pro、context_window=1000000，后缀在生成 ca |
| [#549](https://github.com/BigPizzaV3/CodexPlusPlus/issues/549) | [Bug]: Skills 始终显示 0；插件配置无法跨供应商 Profile 继承 | ed0c6ff2「feat: Skills 改成文件系统模型」：新增 crates/codex-plus-core/src/skills.rs，明确 codex 的 skill 发现机制是文件系统（$CODEX_HOME/skills/<id>/SKILL.m |
| [#576](https://github.com/BigPizzaV3/CodexPlusPlus/issues/576) | [Bug]: 更新到1.1.9之后插件异常（只有三个插件） | 02a23e14 提交信息给出实测对照：openai-curated / openai-curated-remote / openai-bundled / openai-api-curated 等整个 openai-* 前缀都返回 No plugin mark |
| [#582](https://github.com/BigPizzaV3/CodexPlusPlus/issues/582) | [Bug]: 添加新供应商后要求设置智能体沙盒，一直不成功 | 0dd16a31「fix: preserve Windows sandbox settings across relay switches」：relay_config.rs 的 preserve_live_app_settings 保留键列表加入 "windo |
| [#588](https://github.com/BigPizzaV3/CodexPlusPlus/issues/588) | [Question]: 插件安装失败 | 02a23e14「fix(plugins): 换掉被 codex 保留的 marketplace 名，修复插件装不上」：注册名从 openai-curated-remote 改为 codex-plus-curated（openai-* 整个前缀是 codex  |
| [#605](https://github.com/BigPizzaV3/CodexPlusPlus/issues/605) | [Bug]: 供应商测试提示模型名不受支持（DeepSeek 收到 gpt-5.4-mi | 当前实现 commands.rs:5256-5267 的三级回退：profile.testModel → relay_profile_model(&profile)（该供应商 config.toml 的 model）→ settings.relay_test_ |
| [#614](https://github.com/BigPizzaV3/CodexPlusPlus/issues/614) | [Bug]: output_item.done 事件中 tool_call name 被 | 739d34a7 fix: 修复中转硅基流动时，Codex提示"unsupported call"问题（2026-06-08），经 PR #771 合并为 6107d8d5，改动在 crates/codex-plus-core/src/protocol_pro |
| [#627](https://github.com/BigPizzaV3/CodexPlusPlus/issues/627) | [v1.2.1 macOS-arm64] DMG 只装了 shell 脚本桩，真实二进制 | bd872071 fix(update): select macOS DMG matching current arch (was picking first asset, sometimes arm64 on x64 Mac)；54c1b621 fix(in |
| [#632](https://github.com/BigPizzaV3/CodexPlusPlus/issues/632) | [Feature]: 增加多个供应商轮询和一个供应商多个api轮询功能 | d3f1f1fb feat: add aggregate relay provider rotation（tag v1.2.55 起，含 v1.5.0）；d7a0a727 feat: route aggregate providers by model（v1. |
| [#652](https://github.com/BigPizzaV3/CodexPlusPlus/issues/652) | [Bug]: macos 检查和下载的安装包错误（intel 机器下到 arm 包） | crates/codex-plus-core/src/update.rs:347-371 platform_asset_rank（注释明确 0=当前 OS+本机架构、1=同 OS 异架构、2=平台不符，macOS 分支按 is_macos_native_arc |
| [#684](https://github.com/BigPizzaV3/CodexPlusPlus/issues/684) | [Bug]: 调用 qwen-vl-max 反复重连 | 修复提交 ac987617「fix(proxy): always emit output_tokens_details.reasoning_tokens in usage」，代码在 crates/codex-plus-core/src/protocol_pro |
| [#694](https://github.com/BigPizzaV3/CodexPlusPlus/issues/694) | 中转站切回官方登录会强制加上中转站的地址 | 对应修复提交 1b214349「fix(relay): 切回官方时移除中转站 provider 整段配置，并清掉其模型名」（2026-09-22 19:12），提交正文说明根因：『clear_relay_config_to_home_with_auth 只对旧 |
| [#737](https://github.com/BigPizzaV3/CodexPlusPlus/issues/737) | 希望增加 RPM 限速设置以避免触发 429 | 实现见 crates/codex-plus-core/src/channel_protection.rs:101 async fn reserve_request(state, request_limit) —— 以 REQUEST_WINDOW = 60s  |
| [#739](https://github.com/BigPizzaV3/CodexPlusPlus/issues/739) | codex++ 启动的 Codex 在任务栏没有应用图标 | 实现见 crates/codex-plus-core/src/windows_integration.rs:414 apply_codexplusplus_icon_to_process_window（调 apply_window_icons + :608 a |
| [#752](https://github.com/BigPizzaV3/CodexPlusPlus/issues/752) | [Bug]: 沙盒设置权限改不改、config 被覆盖 | crates/codex-plus-core/src/relay_config.rs:2098 `for key in ["sandbox_mode", "approval_policy", "sandbox_workspace_write", "window |
| [#774](https://github.com/BigPizzaV3/CodexPlusPlus/issues/774) | [Bug]: stream disconnected before completion | crates/codex-plus-core/src/protocol_proxy.rs:5402-5408「Codex 把 output_tokens_details.reasoning_tokens 当必填解析」后写入 `"output_tokens_de |
| [#801](https://github.com/BigPizzaV3/CodexPlusPlus/issues/801) | 最新版 Codex++ 开启 Fast 失败（service_tier_dispatch | assets/inject/renderer-inject/30-service-tier.js:945-951 loadDispatcher 依次尝试 ["setting-storage-", "vscode-api-", "app-initial-"]，并 |
| [#825](https://github.com/BigPizzaV3/CodexPlusPlus/issues/825) | 检查更新下载了 arm 版（Intel Mac） | commit bd872071 (2026-06-21) "fix(update): select macOS DMG matching current arch (was picking first asset, sometimes arm64 on x64 |
| [#826](https://github.com/BigPizzaV3/CodexPlusPlus/issues/826) | WSL2+代理导致 57321 bind 失败，建议支持自定义端口 | commit 0e9a86a7 (2026-09-15) "fix: helper 端口绑定失败分类报错，协议代理端口支持环境变量整体挪动 (issue #2189)"；新增 CODEX_PLUS_PROTOCOL_PROXY_PORT 覆盖并统一 proto |
| [#886](https://github.com/BigPizzaV3/CodexPlusPlus/issues/886) | [Bug]: 脚本市场 index.json 语法错误 | 索引 URL 常量在 `crates/codex-plus-core/src/script_market.rs:7-8`；解码失败文案在同文件 `:73`。实测：`curl -s https://raw.githubusercontent.com/BigPiz |
| [#899](https://github.com/BigPizzaV3/CodexPlusPlus/issues/899) | [Bug]: 没办法启动code++ | `crates/codex-plus-core/src/ports.rs:226-230` 明确把 os error 10013 判为「端口被系统禁止绑定（Windows 保留端口区间）」；`ports.rs:9-40` 的 `guard_port_offse |
| [#904](https://github.com/BigPizzaV3/CodexPlusPlus/issues/904) | [Bug]: 对话删除bug | `crates/codex-plus-data/src/storage.rs:206-210` 注释原文：「于是重启后 UI 从索引读，会话又冒出来，再删再冒（#1979）」。相关提交：`51c5cc05`（2026-09-03「fix: 删除会话时同步清理侧 |
| [#911](https://github.com/BigPizzaV3/CodexPlusPlus/issues/911) | [Bug]: 每次关闭了codex之后重启就无法通过codex++启动，只能重启电脑 | 修复提交 `0e9a86a7`（2026-09-15，「fix: helper 端口绑定失败分类报错，协议代理端口支持环境变量整体挪动 (issue #2189)」），`git tag --contains 0e9a86a7` = v1.4.0 v1.5.0。 |
| [#916](https://github.com/BigPizzaV3/CodexPlusPlus/issues/916) | [Bug]: 更新后会话被清空, 无法同步了 | 代码：crates/codex-plus-core/src/codex_sqlite.rs:24-45 codex_session_db_paths_in_home（先取 sqlite/ 下的候选，再追加 legacy state_5.sqlite，去重）、: |
| [#938](https://github.com/BigPizzaV3/CodexPlusPlus/issues/938) | [Bug]: 会话无法删除（thread not found in local stor | crates/codex-plus-data/src/storage.rs:21（默认失败文案）、:48-60（注释明确「纯 API 模式下 threads 表是空的……UI 读的是 session_index.jsonl，那条记录没人清（#1998）」，de |
| [#942](https://github.com/BigPizzaV3/CodexPlusPlus/issues/942) | [Bug]: Mac自动更新下错安装包 | bd872071「fix(update): select macOS DMG matching current arch (was picking first asset, sometimes arm64 on x64 Mac)」（2026-06-21，git |
| [#948](https://github.com/BigPizzaV3/CodexPlusPlus/issues/948) | [Bug]: codex++调用deepseek v4pro，codex显示上下文最大容 | assets/deepseek-model-metadata.json：deepseek-v4-pro/deepseek-v4-flash 等 8 个模型的 context_window 与 max_context_window 均为 1048576（DS v |
| [#994](https://github.com/BigPizzaV3/CodexPlusPlus/issues/994) | apply_relay_injection fallback 和 save_relay_ | 修复提交 7daabac8「写盘前用 live 补齐 mcp_servers 条目，修掉 node_repl 被打散 (#2288)」，`git tag --contains 7daabac8` = v1.4.0、v1.5.0（已发布）。当前代码：write_ |
| [#1097](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1097) | [Config]: 切换供应商覆盖 live 的桌面设置；model 字段可膨胀 set | preserve_live_app_settings（crates/codex-plus-core/src/relay_config.rs:2078 起）把 live 的 `desktop` 表整体 merge 回来（relay_config.rs:2092- |
| [#1102](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1102) | [Question]: 为什么 1.2.14 版本没有 cc-switch 联动了？ | 加入：c16d1cec（2026-05-28 feat: link supplier profiles with cc-switch）新增 crates/codex-plus-core/src/ccs_import.rs 与 App.tsx 联动 UI；904 |
| [#1147](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1147) | 会话管理界面添加批量删除功能 | 实现见 apps/codex-plus-manager/src/App.tsx:2040-2079 的 deleteLocalSessions（去重、预览前 6 条、确认后逐条删除并汇总成功/失败），调用点由「批量删除会话」按钮触发；引入提交为 6023b1b |
| [#1162](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1162) | 通用配置提取/合并及 MCP 开关写入异常，启用 node_repl 或 cloudfl | 6b02b74a fix: 写盘前用 live 补齐 mcp_servers 条目，修掉 node_repl 被打散（2026-09-23），新增 repair_mcp_servers_from_live（crates/codex-plus-core/src/ |
| [#1179](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1179) | [Bug]: 每次启动 codex 插件市场里只有默认的三个插件 | crates/codex-plus-core/src/plugin_marketplace.rs:17-33 的保留名对照表与 CODEX_PLUS_MARKETPLACE 常量；修复提交 02a23e14、6b65c094，`git merge-base - |
| [#1198](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1198) | [Bug]: 每次电脑重启后打开 codex 都只有默认三个插件 | crates/codex-plus-core/src/plugin_marketplace.rs:17-33 的实测对照表（openai-curated / openai-curated-remote / openai-bundled 等全部被忽略，codex |
| [#1224](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1224) | 协议代理收到 429 时无退避重试，导致 Codex 报 exceeded retry  | ec980c1f「feat: add shared channel protection」+ 07c421bb「feat: resume provider requests after channel cooldown」+ 6bfa6b2c「fix: cap  |
| [#1309](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1309) | Windows 排除端口范围导致无法重新拉起 Codex（os error 10013） | git show e2521d44 改 apps/codex-plus-launcher/src/main.rs、crates/codex-plus-core/src/launcher.rs、ports.rs；`git tag --contains e2521 |
| [#1337](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1337) | [Bug]: 删除会话，报错 Thread not found In storage | crates/codex-plus-data/src/storage.rs:48-53 注释逐字描述该现象（「纯 API 模式（model_provider = "custom"）下 threads 表是空的…于是直接返回「Thread not found i |
| [#1340](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1340) | [Bug]: 使用聚合供应商时 Codex 弹出登录页面，且无法识别 .env 中的代理 | (1) bfdde341 / d09d5e46「fix: 聚合模式切换保留 auth.json 认证状态，修复弹登录页（issue #1604）」——commit message 与用户症状逐字对应，git tag --contains d09d5e46 命中 |
| [#1357](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1357) | [Feature]: 对话居中宽度自适应 | c8f8ee30「fix(inject): 对话居中宽度改按结构查找并自适应容器 (issue #2258, #2085)」(2026-10-02)。注意：git tag --contains c8f8ee30 为空 —— 该修复在 v1.5.0 之后，尚未发 |
| [#1432](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1432) | [Bug]: 读取 live 工具与插件失败：config.toml TOML 解析失败 | crates/codex-plus-core/src/relay_config.rs:1741 normalize_duplicate_toml_text → merge_duplicate_toml_blocks（按表头切块后逐块 parse 再语义合并，修 |
| [#1444](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1444) | [Bug]: TOML 结构冲突（codex++ 注入时把子表插到父表前面） | crates/codex-plus-core/src/relay_config.rs:1741 `normalize_duplicate_toml_text`（按顶层表头切块 + 语义合并），其中 :1760-1766 的注释明确写「父表头与它的真子表必须留在 |
| [#1467](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1467) | [Bug]: 删除历史对话显示 Thread not found in local st | 同一处代码 crates/codex-plus-data/src/storage.rs:36-70（兜底分支）与 :527、:702（两处 `"Thread not found in local storage"` 的失败返回点）。提交 d90c0c05 /  |
| [#1531](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1531) | 供应商切换报错后界面显示与实际请求路由不一致（可能走错供应商扣费） | crates/codex-plus-core/src/relay_switch.rs:40 LiveFilesSnapshot::capture(home)，:55-73 失败分支同时 store.save(&original_settings) 与 live |
| [#1604](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1604) | 聚合运营商启动 codex 会弹登录页 | commit d09d5e46 fix: 聚合模式切换保留 auth.json 认证状态，修复弹登录页（issue #1604）(#2283)，2026-09-23；git tag --contains d09d5e46 含 v1.4.0 v1.5.0（同内容 |
| [#1661](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1661) | 读取不到供应商配置（CCS 装在非 C 盘） | commit 080611c5（2026-08-27），git tag --contains 含 v1.2.56 v1.3.0 v1.4.0 v1.5.0；改动 crates/codex-plus-core/src/ccs_import.rs(+137)、se |
| [#1685](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1685) | 三方 AI API 请求时支持配置 HTTP header | commit ea0ac5d1 feat: 供应商支持配置自定义上游请求头（issue #1685）(#2285)，git tag --contains 含 v1.4.0 v1.5.0。代码 crates/codex-plus-core/src/relay_h |
| [#1691](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1691) | Dream Skin theme backup belongs to a differe | commit 9211e4f4 fix(dream-skin): recover stale config backup identities (#1691)，2026-08-27；git tag --contains 含 v1.2.56 v1.3.0 v1. |
| [#1739](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1739) | 能不能设置 API Key 为空，用于无需鉴权的公益站点 | commit 2fa2a6c5 feat(relay): support unauthenticated upstreams (#1739)，git tag --contains 2fa2a6c5 含 v1.5.0。代码：crates/codex-plus-c |
| [#1948](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1948) | 更新后历史会话全部消失：no rollout found for thread id | commit 5a3ed5c3 提交信息明列「#1948 的部分修复（value_asserts_non_root_agent）；rollout_exists 那一半仍依赖尚未合并的 PR #1922」；crates/codex-plus-data/src/p |
| [#1989](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1989) | 插件市场/技能拉取失败，四个仓库文件树返回错误状态 | commit 3873d255「fix(skills): GitHub 请求失败时说清是限流还是仓库不存在」，提交信息明确写 #1989；`git tag --contains 3873d255` 含 v1.5.0。实际成因多为未认证请求 60 次/小时配额耗 |
| [#2063](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2063) | 二次通过桌面快捷方式打开的窗口右上角关闭按钮失效 | apps/codex-plus-manager/src-tauri/src/lib.rs:345-370（Focused(true) 里 `#[cfg(windows)] let _ = focus_event_window.show();`，注释原文「外部实 |
| [#2103](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2103) | [Bug]: 插件无法加载（config.toml:74:1: invalid tran | `cd1eba23 fix(relay-config): 用语义合并替代行级去重, 修复 config.toml invalid transport`（2026-08-27），`git tag --contains cd1eba23` 含 v1.3.0 / v |
| [#2113](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2113) | [Feature]: 同时打开多个供应商的 api | `d3f1f1fb feat: add aggregate relay provider rotation`（2026-06-01），`git tag --contains d3f1f1fb` 含 v1.2.15 起的全部后续 tag 直至 v1.5.0。数据 |
| [#2124](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2124) | GPT6 模型未更新 | `27fd20dd feat(models): 从 openai/codex 官方仓库同步 gpt-6-astra 与 gpt-5.6-sol 元数据`（2026-09-18），`git tag --contains 27fd20dd` 含 v1.4.0 与  |
| [#2162](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2162) | [Feature]: 主界面 Codex++ 图标旁的绿点能否消除 | apps/codex-plus-manager/src/App.tsx:3571 的 `className="update-dot"` 仅在 `const hasUpdate = update?.updateAvailable === true`（App.ts |

### 4.2 重复提单（23 条）

| issue | 主题 | 重复于 |
|---|---|---|
| [#274](https://github.com/BigPizzaV3/CodexPlusPlus/issues/274) | 通过 DeepSeek 使用 CodexPlusPlus 时，自动审批请求因 codex | #293 |
| [#302](https://github.com/BigPizzaV3/CodexPlusPlus/issues/302) | codex 移动版功能无法使用 | #340 |
| [#330](https://github.com/BigPizzaV3/CodexPlusPlus/issues/330) | mac tahoe 26.5 提示文件已损坏 | #333 |
| [#375](https://github.com/BigPizzaV3/CodexPlusPlus/issues/375) | 备份 codex 所有数据缓存与插件，两台电脑无缝衔接 | #647 |
| [#389](https://github.com/BigPizzaV3/CodexPlusPlus/issues/389) | MAC M1 配好供应商后重启 codex 无 deepseek 模型选择 + 502 | #406 |
| [#548](https://github.com/BigPizzaV3/CodexPlusPlus/issues/548) | [Bug]: 自动审批功能模型名配置错误，DeepSeek 上游返回 502 | #564 |
| [#674](https://github.com/BigPizzaV3/CodexPlusPlus/issues/674) | [Bug]: 无法更新 — 更新 1.2.2 报毒 | #682 |
| [#681](https://github.com/BigPizzaV3/CodexPlusPlus/issues/681) | [Bug]: Trojan:Win32/Wacatac.B!ml | #682 |
| [#784](https://github.com/BigPizzaV3/CodexPlusPlus/issues/784) | [Bug]: openai-bundled marketplace 同步不完整，brow | #818 |
| [#797](https://github.com/BigPizzaV3/CodexPlusPlus/issues/797) | Intel Mac 点自动更新下载的是 M 芯片版本 | #825 |
| [#834](https://github.com/BigPizzaV3/CodexPlusPlus/issues/834) | 希望提供强制汉化（公司大批量安装、没有梯子） | #831 |
| [#893](https://github.com/BigPizzaV3/CodexPlusPlus/issues/893) | [Question]: 脚本市场加载失败 | #886 |
| [#920](https://github.com/BigPizzaV3/CodexPlusPlus/issues/920) | [Bug]: 对话记录无法正常读取 | #916 |
| [#1009](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1009) | [Bug]:无法修复后端（装 fast-patch skill 后） | #1013 |
| [#1011](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1011) | 【配置】Gemini3.5flash接入无法使用 | #1012 |
| [#1059](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1059) | [Question]: 接入 mimo 同一项目里不能切换模型，显示重连 404 | #1058 |
| [#1096](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1096) |  | #1102 |
| [#1410](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1410) | [Bug]: stream disconnected before completion | #1411 |
| [#1799](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1799) | hot-switch API providers without restarting  | #2113 |
| [#1833](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1833) | 聚合供应商：多会话分别使用不同供应商 | #2113 |
| [#1973](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1973) | 给 codex++ 单独配置代理 | #2135 |
| [#2025](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2025) | [Question] 供应商选择组合供应商的问题：卡登录页 + 同对话仍轮转 | #2036 |
| [#2214](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2214) | Codex++ v1.3.0 在 Codex Desktop 26.527.3686.0 | #2211 |

### 4.3 不是本仓的问题（71 条）

| issue | 主题 | 为什么不修 |
|---|---|---|
| [#135](https://github.com/BigPizzaV3/CodexPlusPlus/issues/135) | macos启动失败，之前能用，codex更新后不可用（crs 不支持 /1res | 提示是中转站不支持 Codex 要用的 Responses 接口（端点未找到）。这是中转服务端能力问题，工具的提示文案本身就是在明确告知要去换一个支持 Responses API 的中转或启用协议转换代理。不在本仓代码缺 |
| [#173](https://github.com/BigPizzaV3/CodexPlusPlus/issues/173) | 点击更新闪退，微软应用商店更新失败 0X80D03805 | 0x80D03805 是 Windows 应用商店（App Installer / Microsoft Store）在下载或安装阶段失败的通用错误码，属于分发渠道侧的问题，不是 CodexPlusPlus 代码缺陷。且  |
| [#183](https://github.com/BigPizzaV3/CodexPlusPlus/issues/183) | 重启失败（macOS AppTranslocation 路径消失） | 报错路径是 /private/var/folders/.../AppTranslocation/<uuid>/d/Codex++.app/...，这是 macOS Gatekeeper 的「鉴定路径随机化」（App Tr |
| [#218](https://github.com/BigPizzaV3/CodexPlusPlus/issues/218) | codex使用三方api代理出现的问题 | `encrypted content ... could not be verified` 是官方加密内容（用于官方登录态的加密载荷）在换到第三方 provider 后无法解密/校验，属于上游服务端的加密归属校验；本仓不 |
| [#234](https://github.com/BigPizzaV3/CodexPlusPlus/issues/234) | 接第三方api报错 | `Unknown parameter: 'stream_options.include_usage'` 是上游供应商（经 litellm/Azure）不接受该字段，而该字段是 Codex 协议转译时按 Chat Comp |
| [#333](https://github.com/BigPizzaV3/CodexPlusPlus/issues/333) | MacOS 26.5 无法打开提示文件受损 | macOS 提示「已损坏，无法打开」。原因是安装包未签名/未公证，被 Gatekeeper 隔离。README 已给出官方解法（xattr 去隔离）。与 #330 同一根因。 |
| [#334](https://github.com/BigPizzaV3/CodexPlusPlus/issues/334) | 能否支持 ChatGPT for PowerPoint | 诉求是接入 ChatGPT for PowerPoint。那是 OpenAI 自家的 Office 插件产品线，不在 Codex++（Codex 桌面客户端的外部启动器）的能力范围内。 |
| [#354](https://github.com/BigPizzaV3/CodexPlusPlus/issues/354) | exec_command 完全失效 CreateProcess No such  | 错误串出自 Codex 客户端自身的统一执行沙箱（create unified exec process），本仓代码里不存在这句错误，也没有参与 exec_command 的进程创建。属于上游 Codex runtime |
| [#367](https://github.com/BigPizzaV3/CodexPlusPlus/issues/367) | Gemini 思维链签名 thought_signature 报错 | Gemini 要求把上一轮 functionCall 里的加密 thought_signature 原样带回，而该字段是 Google 私有字段、不在 OpenAI 协议里；走 chat/responses 翻译通道时它 |
| [#377](https://github.com/BigPizzaV3/CodexPlusPlus/issues/377) | mac 升级系统后无法安装，报「已损坏」，没有「仍要打开」选项 | 「应用已损坏，无法打开」是新版 macOS 对未公证/带隔离属性的 app 的 Gatekeeper 提示，官方出路就是去掉隔离属性；用户说「隐私与安全性里没有『仍要打开』」正是因为该提示走的是损坏分支而非未验证开发者分 |
| [#399](https://github.com/BigPizzaV3/CodexPlusPlus/issues/399) | 接入国产模型 mimo 后不能访问外部链接 / 有没有 WebSearch | 联网检索是 Codex 客户端按模型能力下发的内置工具（web_search），第三方模型若不在客户端的支持名单里就不会拿到该工具，Codex++ 只做协议翻译不注入检索能力。 |
| [#405](https://github.com/BigPizzaV3/CodexPlusPlus/issues/405) | 优化管理工具 UI 设计 | 纯主观审美诉求，没有指出任何具体页面/组件的可用性问题，不构成可跟进的缺陷。 |
| [#418](https://github.com/BigPizzaV3/CodexPlusPlus/issues/418) | 切换供应商后无法拉起 Codex（AppTranslocation 路径不存在） | 错误路径在 /private/var/folders/.../AppTranslocation/ 下，说明 app 是直接从 dmg 里运行、被 macOS 的 Gatekeeper 路径重定向到随机隔离目录，重启或移动 |
| [#427](https://github.com/BigPizzaV3/CodexPlusPlus/issues/427) | config.toml 里 mcp_servers.node_repl.env  | 报错出自 Codex 客户端对 config.toml 的解析：`[mcp_servers."node_repl.env"]` 这种把 env 写成子表、且与 `[mcp_servers.node_repl]` 并存的结 |
| [#471](https://github.com/BigPizzaV3/CodexPlusPlus/issues/471) | [Question]: 这个会覆盖安装好的 codex 吗 | 纯咨询，答案明确：CodexPlusPlus 是外部增强工具，不改 Codex 安装本体，配置写在 ~/.codex/config.toml 与 auth.json。 |
| [#479](https://github.com/BigPizzaV3/CodexPlusPlus/issues/479) | [Bug]: 窗口最大化时左侧菜单栏变透明 | 用户自己找到了成因与解法：Codex 的「设置 → 外观 → 半透明侧边栏」在最大化时背景合成异常。这是上游客户端的渲染问题，且用户侧可自行关闭。 |
| [#481](https://github.com/BigPizzaV3/CodexPlusPlus/issues/481) | [Question]: CHAT 格式转 response 时上下文无法正确设置 | 用户手写的键名是 `model_context_windows`（复数），Codex 只认 `model_context_window`（单数）。键名写错所以顶层兜底不生效，仍走默认窗口。这是写法问题，不是本仓缺陷；但我 |
| [#483](https://github.com/BigPizzaV3/CodexPlusPlus/issues/483) | [Question]: chat completions 上传图片出现 Stee | 「Steered conversation」是 Codex 客户端自身的会话状态提示，不是 CodexPlusPlus 的产物；全仓零命中。用户自己也说图像理解正常。 |
| [#564](https://github.com/BigPizzaV3/CodexPlusPlus/issues/564) | 第三方模型下 guardian_approval 的 codex-auto-re | 自动审批调用 codex-auto-review 模型是 Codex 客户端 guardian_approval 的行为，模型名只存在于 OpenAI 官方 API；本仓不参与该调用，grep 零命中。 |
| [#643](https://github.com/BigPizzaV3/CodexPlusPlus/issues/643) | [Question]: 无法使用DeepSeek：测试发送 hi HTTP 40 | 供应商测试是拿一个模型名去打对方 /chat/completions 或 /responses，返回 404 + 空响应说明该 DeepSeek 中转地址下不存在这个模型名或路径不对，属于第三方服务端行为。 |
| [#669](https://github.com/BigPizzaV3/CodexPlusPlus/issues/669) | [Bug]: @chrome 浏览器自动化功能不可用 | 缺的是 Chrome 扩展的 native messaging host manifest 与注册表项（HKCU\...\NativeMessagingHosts\com.openai.codexextension），这 |
| [#677](https://github.com/BigPizzaV3/CodexPlusPlus/issues/677) | [Config]: 供应商模型配置读取授权环境变量key 缺失提示 | 用户在自定义 provider 里写了 `env_key= LM_Studio_auth_key`（等号后带空格、未加引号），并期望 GUI 程序能读到 .zshrc 里的 export。这是用户配置写法 + 系统环境变 |
| [#682](https://github.com/BigPizzaV3/CodexPlusPlus/issues/682) | [Bug]: 1.2.2 windows 版 setup 被杀软报 Trojan | Windows Defender 对 1.2.2 安装包的启发式误报。注入机制（写脚本文件 + 回环调试端口）本身就是启发式检测的高命中特征，代码开源无混淆，属于误报而非真实威胁。 |
| [#709](https://github.com/BigPizzaV3/CodexPlusPlus/issues/709) | Codex 本地中转服务与代理服务器冲突（502） | WebView2 走系统代理时把 127.0.0.1:57321 的请求发给代理导致 502——用户自己已给出正确结论，属 WebView2 行为，非本仓可修。 |
| [#710](https://github.com/BigPizzaV3/CodexPlusPlus/issues/710) | 请求更新 Codex 官方插件（Product Design） | 希望新增官方 Product Design 插件——插件目录由 OpenAI 官方远端目录下发，本工具只做显示层解锁，无法凭空新增插件。 |
| [#731](https://github.com/BigPizzaV3/CodexPlusPlus/issues/731) | 希望支持图片/视频生成 API | 要求新增 /v1/images/generations 与 /v1/videos 支持——这超出本工具「Codex 客户端配置与协议翻译」的范围，属产品边界外。 |
| [#759](https://github.com/BigPizzaV3/CodexPlusPlus/issues/759) | [Question]: 用 DeepSeek 但模型自称 GPT | 询问第三方模型为何自称 GPT。Codex++ 只写 config.toml / auth.json 与注入层，不参与也不会改写发给上游的 system prompt；模型自述身份来自上游供应商自己的模板或蒸馏数据。 |
| [#780](https://github.com/BigPizzaV3/CodexPlusPlus/issues/780) | [Config]: 导入 agnes 模型后仍要登录 GPT 账号且显示额度不足 | 报错原文是上游返回：The 'agnes-2.0-flash' model is not supported when using Codex with a ChatGPT account。这是 Codex 官方对「Ch |
| [#782](https://github.com/BigPizzaV3/CodexPlusPlus/issues/782) | [Config]: kimicode 无法接入（HTTP 403） | 供应商测试返回 HTTP 403，错误信息是 Kimi 服务端给出的：Kimi For Coding is currently only available for Coding Agents such as Kimi  |
| [#852](https://github.com/BigPizzaV3/CodexPlusPlus/issues/852) | 401 Unauthorized: Invalid token（aidraw36 | 错误是第三方中转站返回的 401「Invalid token」，且带对方 request id；这是上游鉴权拒绝，与本仓无关，正文自己也没勾「已确认是最新版」。 |
| [#896](https://github.com/BigPizzaV3/CodexPlusPlus/issues/896) | [Question]: 使用第三方api 缓存(cache)=0 问题 | 对比 Claude Code 接火山方舟缓存命中 75%、Codex 客户端走 Codex++ 命中 0。缓存命中由上游与请求体前缀稳定性决定，本仓不参与 prompt cache 的构造或改写，属上游/供应商侧行为。 |
| [#909](https://github.com/BigPizzaV3/CodexPlusPlus/issues/909) | [Bug]: 上下文问题 | 报的是上游 `agnes-2.0-flash` 返回 262144 上限的 ContextWindowExceededError，且正文被用户自己的提示词/对话日志淹没（含「霓虹弹力球实验室」那段），属于供应商侧窗口限制 |
| [#927](https://github.com/BigPizzaV3/CodexPlusPlus/issues/927) | [Question]: 无法更新（error sending request f | 报错是 HTTP 请求发不出去（error sending request = reqwest 连接层失败），即本机到 GitHub 的网络被阻断。检查更新走的是 GitHub Releases 的 latest.jso |
| [#971](https://github.com/BigPizzaV3/CodexPlusPlus/issues/971) | [Bug]: kimi coding plan 报错 403 | 403 响应体明确说明 Kimi For Coding 只对 Coding Agents（Kimi CLI、Claude Code、Roo Code 等）开放，是服务端对调用方的准入判断，与是否携带 agent head |
| [#1067](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1067) | [Bug]: 无法使用 computer use（node_repl kerne | 报错来自上游 Codex 的 cua_node 运行时（node_repl 内核），错误码 CreateProcessAsUserW failed: 5 是 Windows 的 ERROR_ACCESS_DENIED—— |
| [#1086](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1086) | [Question]: 用 ai 更换项目路径后侧边栏会话点击后会消失 | 用户用 sqlite3 直接 UPDATE threads.cwd，又用 sed 批量改写 ~/.codex/sessions 下的 jsonl，破坏了 Codex 自身的会话索引一致性；侧边栏消失是 Codex 客户端 |
| [#1088](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1088) | [Bug]: 图片无法发送 | 报错来自上游中转/供应商——它明确回「No endpoints found that support image input」，说明 mimo-v2.5-pro 这条链路上没有支持图片输入的 endpoint；Codex |
| [#1105](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1105) | Launching Codex via Codex++ appears to t | 崩溃二进制是 /Applications/Codex.app/Contents/PlugIns/CodexDockTilePlugin.plugin，栈在 setDockTile:，用户自己也已确认「Crashing b |
| [#1107](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1107) | [Question]: Codex 无法根据命令下载文件和写入文件修改，只「拟真 | 用户看到的「由于当前的沙盒模式限制，我需要提升权限才能执行 git clone」是上游 Codex 的沙盒/审批提示，属于 Codex 的安全策略表现，不是 Codex++ 的缺陷；Codex++ 反而专门做了「跨供应商 |
| [#1128](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1128) | [Question]: codex 内部的 chrome 插件显示连接正常但无法 | 与 1135 同源：解析出来的根因（node_repl MCP 要求 sandboxPolicy 元数据、Codex++ 未传）是 Codex 自己的推断，仓库里没有任何注入/剥离该字段的代码。Chrome connec |
| [#1135](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1135) | [Bug]: 官方 hotfix 后 sandboxPolicy 不再报错，但混 | 混合 API 模式下 Chrome connector 接不上，两种失败表现（工具列表里没有 node_repl/js、agent.browsers.get('extension') 报 Browser is not a |
| [#1153](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1153) | The 'agnes-2.0-flash' model is not suppo | 该报错由 Codex 客户端在 ChatGPT 账号登录态下发出：官方登录态只允许官方模型，第三方模型必须走 API 供应商（纯 API / 混合）模式。属于上游行为，不是本仓缺陷。 |
| [#1159](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1159) | 权限批准问题：按钮被 requirements.toml 锁定 | 「权限被 requirements.toml 锁定」是 Codex 客户端自身的策略提示文案，本仓既不读也不写该文件，不产生这个提示。用户在本机找不到该文件也说明它来自 Codex 的内置默认策略或远端下发。 |
| [#1164](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1164) | 添加的 Gmail 和 GitHub 插件无法登录和认证 | 插件的登录/授权流程由 Codex 官方插件的连接器机制完成，Codex++ 只负责把插件市场注册到 config.toml 并解锁列表，不代办 OAuth。 |
| [#1165](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1165) | Access is denied，没有办法绕过沙盒限制来修改文件 | 沙盒与审批策略由 Codex 客户端（及其 requirements.toml 策略）决定，Codex++ 不介入也不提供绕过。用户已经在和 LM Studio 侧对话，这是本机沙盒/文件权限问题。 |
| [#1168](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1168) | code++ 是否支持接入 Claude 模型 | 用户看到的报错「该模型为 Claude 模型，仅支持 Anthropic (/v1/messages) 协议调用」来自第三方中转站的服务端，不是本仓代码产生；仓库的 protocol_proxy 只实现 Response |
| [#1174](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1174) | agnes-video-v2.0 已注册 /v1/models，但 /v1/vi | 「未知后端路径」是本仓本地 helper 后端的 404 兜底响应，它只代理 responses/chat/completions/audio transcriptions/images generations 这几条固 |
| [#1184](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1184) | [Bug]: macOS M4 点击启动 Codex++ 立马闪退 | 崩溃报告显示崩溃进程是 Codex 本体（com.openai.codex），触发线程为 CrBrowserMain，栈顶在 Codex Framework 的 temporal_rs_PlainDateTime_hou |
| [#1193](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1193) | [Bug]: Windows Computer Use 读取窗口报 SetIsB | 报错出自 Windows 平台接口调用（SetIsBorderRequired / E_NOINTERFACE），属上游 computer-use 插件的实现，本仓无该字符串。 |
| [#1207](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1207) | [Bug]: macOS 15.7.5 管理工具完全无法运行 - AMFI 拦截 | 根因是 macOS 的 AMFI/amfid 在加载阶段拒绝该二进制，属于系统签名校验，不是本仓代码能干预的行为。 |
| [#1222](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1222) | [Feature]: The following plugins are mis | 用户说插件列表缺少 Cowart / Product Design，缺的是上游 OpenAI 官方插件目录里的条目，不是 Codex++ 的清单，本仓无任何可改代码。 |
| [#1225](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1225) | 桥接 Responses API 时，原生 Computer Use 工具引发  | 「tool type 'custom' is not supported by this gateway phase」是 MiMo 网关自己拒收 Responses 原生 custom 工具，wire_api=respo |
| [#1238](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1238) | [Bug]: mimo 的 plan 在测试中能正常，进入 codex 后不能使 | 报错 tool type 'web_search' is not supported by this gateway phase 是 MiMo 网关拒收 Codex 注入的内置 web_search 工具，与 #1225 |
| [#1249](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1249) | [Question]: 经常对话抛出异常，就无法继续了 | 上游返回 400 BadRequestError「Expecting property name enclosed in double quotes: line 1 column 2」，是 agnes-ai 中转站把请求 |
| [#1266](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1266) | [Bug]: 在 SQL 脚本中指定了数据库，执行 SQL 时还需要在应用内手动 | 报告的是 SQL 脚本/数据库执行工具的需求（脚本里写 USE db 后自动选中），与 Codex++（Codex 客户端增强+供应商中转）完全无关，明显发错仓库。 |
| [#1383](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1383) | [Bug]: Chrome Browser Control fails with | 报错来自 CreateProcessWithLogonW（Windows 二次登录创建进程）失败 1385（用户无「作为批处理作业登录」权利），调用方是 Codex 客户端的统一 exec 通道，不是本仓代码。用户已自行 |
| [#1409](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1409) | [Bug]: Invalid Value: 'tools'. Function  | 报错是「同一个请求里 image_gen.imagegen 与一个 hosted tool 冲突」。本仓从不向请求注入生图工具（只做工具类型识别与转发），冲突来自客户端/插件侧的 imagegen 技能与官方内置 hos |
| [#1411](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1411) | [Bug]: stream disconnected before comple | 报错里的端口 15721 不是 Codex++ 的协议代理端口（本仓默认 57321），说明流量根本没走 Codex++ 的代理，是别的东西（另一个代理软件/残留配置）占用了；与 #1410 同一条。 |
| [#1428](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1428) | [Bug]: 更新codex 26.707版本后，设置菜单改为中文，大部分仍为英 | 汉化不全与启动变慢都来自上游 Codex 与 ChatGPT 合并后的客户端本身：语言包由官方 Statsig 下发的翻译覆盖决定，本仓只做 navigator.language 覆写，翻不了官方没提供的条目；启动变慢是 |
| [#1454](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1454) | [Bug]: Error from provider (Console Go): | 「Console Go」不是 Codex++ 里的任何概念：全仓零命中，报错来自用户配置的那个中转/供应商上游。文本请求走 OpenCode Go 正常、图片请求被分流到 Console Go 报上游失败，是用户侧的多供 |
| [#1456](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1456) | [Bug]: 无法调用子agent（Provide either message | 报错「Provide either message or items, but not both」出自上游 Codex 客户端的请求校验：调用子 agent 时同时提交了 message 和 items。全仓没有这个校验 |
| [#1459](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1459) | [Bug]: The 'agnes-2.0-flash' model is no | 上游 Codex 的限制：用 ChatGPT 账号登录时不允许自定义模型。用户在 ChatGPT 登录态下切了自定义模型 agnes-2.0-flash，被上游客户端按账号模式拦下，不是 Codex++ 的 bug。 |
| [#1495](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1495) | Microsoft Store 版 codex 未注册 codex:// 协议， | MS Store 版安装后没注册 codex:// 协议，Codex++ 拉起失败；用户自己也说「已手动修复」。注册 Windows 协议处理器应由 Codex 的安装包负责，本仓 grep 不到 codex:// 相关 |
| [#1497](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1497) | 与 cc-switch 冲突导致 config.toml 混乱（名字是供应商 1 | 两个工具都在写同一份 ~/.codex/config.toml，后写的覆盖先写的，产生名字与 base_url 混用。这是两个管理器争抢同一文件的固有冲突，不是 Codex++ 单方面缺陷；仓库有 cc-switch 导 |
| [#1766](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1766) | 从 API 供应商切换到 GPT Plus 后，旧会话派生新会话报 invali | encrypted_content 是上游 Codex 服务端的加密推理内容，只有签发它的那条链路（同一供应商/账号）才能解密校验。跨供应商派生必然失败，Codex++ 已能检测并提示，属上游机制限制，非本仓缺陷。 |
| [#1945](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1945) | 自动安全审查始终失败 20012 Model does not exist (g | 自动审查走的模型由 Codex 内核/桌面端固定选择，不经过 Codex++ 的别名或映射层；用户已自测 model_aliases 与 approval_policy 均无效，属上游策略。 |
| [#1997](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1997) | invalid transport in mcp_servers.codex_a | 报错来自 Codex 内核校验一条只含 enabled_tools、缺 command/url 的 mcp_servers.codex_app；全仓搜索 codex_app 作为 MCP 条目的字面量零命中，本仓不生成该 |
| [#2037](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2037) | [Question] 静默启动入口被删，怎么重新下载 exe | 用户误删了静默启动快捷方式，问 exe 从哪重新下载。这是使用问题不是缺陷：静默启动靠 `--background` 参数，重装或手动新建指向同一 exe 加该参数的快捷方式即可，无需重新下载。可回答。 |
| [#2071](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2071) | [Bug]: macOS 报 bundle binary is unavaila | 错误路径里的 `AppTranslocation/.../d/Codex++.app/...` 是 macOS Gatekeeper 的隔离态随机挂载路径，说明应用是从 DMG/下载位置直接运行、未移入「应用程序」并去除 |
| [#2125](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2125) | Opencode Go 接入后提示缺少请求头 x-opencode-sessio | 该 400 由 OpenCode 服务端返回，要求客户端带 `x-opencode-session`（用于把同一会话路由到同一后端）。这个头是调用方语义，Codex++ 不生成会话 ID，属于供应商侧策略。但本仓支持自定 |
| [#2132](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2132) | kimi k3 几轮对话后 exceeded retry limit, last | 429 由上游/中转站限流产生，重试耗尽后由 codex 客户端抛出该错误。本仓对 429 有冷却与排队机制，不是缺陷；m1 pro 上重启无效也符合「服务端限流」的特征。 |

### 4.4 信息不足（36 条）

| issue | 主题 | 建议回复 |
|---|---|---|
| [#171](https://github.com/BigPizzaV3/CodexPlusPlus/issues/171) | 对话派生不了 | 这条正文是空的（只有一个空链接），没法判断问题，先关闭。如果还需要跟进请重开并说明：你说的「派生对话」是 Codex++ 的哪个功能（还是 Codex 客户端自带的分支/fork 对话），失败时有没有报错提示，以及完整操作步骤。 |
| [#172](https://github.com/BigPizzaV3/CodexPlusPlus/issues/172) | 插件安装失败 | 缺少可定位的信息，先关掉，需要时重开。请在重开时补充：1) 装的是哪个插件（名字或来源）；2) 失败时的完整报错文案/截图；3) Codex 客户端版本与本工具版本；4) 设置里是「完整增强」还是「兼容增强」模式。插件市场链路在兼容增强模式 |
| [#281](https://github.com/BigPizzaV3/CodexPlusPlus/issues/281) | 插件无法支持设置目标 | 这条没有正文，我不确定「设置目标」指的是哪一处：是插件市场里选安装目标，还是插件运行时指定的目标？麻烦补充具体位置和期望行为（最好带截图文字），我们再判断是否在支持范围内。 |
| [#290](https://github.com/BigPizzaV3/CodexPlusPlus/issues/290) | codex cli无法使用 后期可以加一下适配吗 | 这条没有正文，看不出具体卡在哪一步。麻烦补充： 1) 你说的 codex cli 是 Codex 自带的命令行，还是独立安装的 codex CLI； 2) 具体报错原文； 3) 你期望它读到的配置（~/.codex/config.toml  |
| [#294](https://github.com/BigPizzaV3/CodexPlusPlus/issues/294) | 删除会话后，再选项目工作目录或模型报错 | 这条缺的信息比较多，麻烦补一下才能查： 1) 报错弹窗/控制台的原文（截图里的文字也请贴出来，我们这边拿不到图片内容）； 2) 版本号（管理工具和 Codex++ 启动器）； 3) 复现步骤：删的是哪个会话（已归档还是普通会话）、之后是在哪 |
| [#307](https://github.com/BigPizzaV3/CodexPlusPlus/issues/307) | 点击删除后重新发起会话报错 | 正文是空的，麻烦补充：1) 完整报错原文或截图；2) Codex++ 版本；3) 是「删除会话后立刻新建会话」还是「删除后回到原会话继续对话」时报错；4) 会话是本地会话还是来自 Codex 云端。补充后我来定位。这条与 #343（删除后重 |
| [#347](https://github.com/BigPizzaV3/CodexPlusPlus/issues/347) | 后端一直连接不上 | 需要这些信息才能定位，麻烦补充：1) Codex++ 版本与管理工具版本；2) 「后端连接不上」出现在哪个入口（启动 Codex / 切换供应商 / 模型列表 / 状态页）；3) 报错原文截图或文字；4) 诊断日志：Windows 在 `% |
| [#364](https://github.com/BigPizzaV3/CodexPlusPlus/issues/364) | 打开 codex 异常 | 这条没有可判断的信息（当前行为是空的，也没有日志或截图），先关闭。麻烦重开时补上：Codex++ 版本、系统、点的是哪个按钮、以及「打开」之后具体看到什么——黑屏／卡住／报错弹窗／直接退出？如果有报错弹窗，把弹窗文字或截图贴上。另外诊断日志 |
| [#372](https://github.com/BigPizzaV3/CodexPlusPlus/issues/372) | 开启 codex 后后端突然崩溃 | 信息不足，暂时没法定位。「后端崩溃」可能是桥接看门狗、代理进程或 Codex 客户端本身退出，三者的日志位置和修法完全不同。麻烦补：(1) 崩溃发生时管理工具状态页显示的是什么（桥接已连接/未连接、代理端口）；(2) 诊断日志里崩溃前后 1 |
| [#416](https://github.com/BigPizzaV3/CodexPlusPlus/issues/416) | 配置问题求解：用 chat 协议如何转换，使用 llama 部署的本地模型 | 信息太少没法给具体答案，先按下面填一遍就能通：本地 llama 一般走 OpenAI 兼容的 Chat Completions —— 在「新增供应商」里选「纯 API」，Base URL 填你 llama 服务的地址加 `/v1`（例如 ` |
| [#458](https://github.com/BigPizzaV3/CodexPlusPlus/issues/458) | [Question]: （空）这是什么bug | 这条只有一句话，我们看不到你说的 bug 是什么。请补充现象描述 + 截图 + Codex++ 与 Codex 客户端版本，重新提一条我们就能跟进。  如果不确定该附什么：把界面上出问题的地方截图、把报错文本原样贴出来，基本就够了。 |
| [#462](https://github.com/BigPizzaV3/CodexPlusPlus/issues/462) | [Question]: 删除会话有时候会出现这种情况 | 这条正文是空的，我们看不到「这种情况」指什么。麻烦补充：  1. 具体现象（删除后列表没更新？残留？报错弹窗？） 2. 复现步骤 3. Codex++ 版本 + Codex 客户端版本 + 系统 4. 有报错的话贴错误文本  合上一条新 i |
| [#477](https://github.com/BigPizzaV3/CodexPlusPlus/issues/477) | [Bug]: 无法使用 Computer Use，脚本市场的脚本没加载成功，无法 | 这条把三个不相关的问题塞在一起了，而且预期行为/复现步骤/日志都是占位符，我们没法定位。麻烦拆成三条分别提，每条带上：Codex++ 版本、Codex 客户端版本、复现步骤、以及对应的日志片段。  其中 Computer Use 无法使用可 |
| [#506](https://github.com/BigPizzaV3/CodexPlusPlus/issues/506) | [Question]: gpt-5.3-codex-spark用不了，codex | 麻烦把具体现象补一下：「用不了」是指模型下拉里选不到它、选了之后请求报错（那请把报错原文发来）、还是客户端提示它需要图片输入？另外告诉我们是走官方账号还是第三方供应商。模型名与能力（是否支持图片）由上游和供应商决定，我们需要看到报错才能判断 |
| [#508](https://github.com/BigPizzaV3/CodexPlusPlus/issues/508) | [Bug]: 安装的插件全部授权失败 | 这条描述里能用的信息太少，我们没法定位。麻烦补上：1) 确切版本号（1.19 应该是 1.1.9？管理工具关于页能看到）；2) 「授权失败」出现的位置和原文提示（是插件市场里点安装时弹的，还是 Codex 里点授权时弹的）；3) 一个具体插 |
| [#523](https://github.com/BigPizzaV3/CodexPlusPlus/issues/523) | [Question]: 安装后启动报错（双击快捷方式弹窗） | 麻烦把弹窗上的文字（或截图）贴出来——是缺 DLL、权限不足、还是找不到 Codex 安装路径，处理方式完全不同。另外顺手发一下 %USERPROFILE%\.codex-plus\diagnostic.log 里启动那几行。已经试过卸载重 |
| [#584](https://github.com/BigPizzaV3/CodexPlusPlus/issues/584) | [Bug]: 接入外部模型不成功怎么回事 | 信息不足以定位，麻烦补充：1) 用的是哪种接入模式（官方 / 纯 API / 聚合）与上游协议（Responses 还是 Chat Completions）；2) 供应商详情页里「测试」按钮的结果文案；3) ~/.codex-session |
| [#604](https://github.com/BigPizzaV3/CodexPlusPlus/issues/604) | [Bug]: 新对话选择不了大模型 | 需要更多信息：1) 用的是官方登录还是第三方 API；2) 点开模型下拉是空白、还是选项点不动；3) Codex++ 版本与 Codex 版本；4) 诊断日志里 /codex-model-catalog 那次请求的返回状态。补充后再跟。 |
| [#612](https://github.com/BigPizzaV3/CodexPlusPlus/issues/612) | [Question]: 无法卸载 | 需要补充具体信息才能处理：1) Windows 还是 macOS；2) 是在「设置-应用」里找不到卸载项，还是点了卸载报错、报什么错；3) 是否用了绿色/免安装版本。补充后我们再跟。Windows 上如果是卸载项残留，可以检查注册表里的 C |
| [#656](https://github.com/BigPizzaV3/CodexPlusPlus/issues/656) | [Bug]: 使用子代理subagent对话截断 | 信息不足，无法定位。麻烦补充：1) 用的是哪家供应商和哪个模型；2) 是否走协议翻译（chatCompletions 接入）还是原生 responses；3) 「截断」的具体表现——是子代理输出中途停止、工具调用被切断，还是界面只显示了一部 |
| [#708](https://github.com/BigPizzaV3/CodexPlusPlus/issues/708) | 复杂多轮进程任务只执行第一轮就终止 | 这条缺关键信息，麻烦补：(1) 「只执行第一轮就终止」时的**具体表现**——是界面停在某一步没有继续、报了错误、还是整轮直接结束？有没有报错文案？(2) 用同样配置换一个主流模型（如官方 GPT）跑同一任务，是否也终止？这能区分是模型/插 |
| [#720](https://github.com/BigPizzaV3/CodexPlusPlus/issues/720) | 插件安装失败怎么解决 | 这条没有正文，没法定位。麻烦补充：(1) 插件名与来源（官方精选 / 本地安装包 / 第三方市场）；(2) 失败时的**完整报错文案**（截图或复制文字）；(3) 本工具与 Codex 的版本号；(4) 当前接入模式（官方登录 / 纯 AP |
| [#820](https://github.com/BigPizzaV3/CodexPlusPlus/issues/820) | [Bug]: 一直不行 deepseek | 信息不太够，麻烦补三样：Codex++ 的具体版本号、DeepSeek 供应商里填的 Base URL 与所选协议（Responses 还是 Chat Completions）、以及点"发送 hi 测试"后弹出的完整报错。DeepSeek  |
| [#1051](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1051) | [Question]: codex++ 工具连不上 codex 更新之后 | 信息太少了，麻烦补充： 1. Codex 客户端更新到哪个版本、Codex++ 是哪个版本； 2. 「连不上」的具体表现——是管理工具里显示未检测到 Codex、还是启动后页面空白、对话报错？ 3. 管理工具概览里的诊断 JSON（里面 c |
| [#1123](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1123) | 空 issue（仅标题「我确认这不是已有功能可以完成的操作。」） | 这条 issue 只有标题、没有正文，标题看起来是表单里「提交前确认」那一行的残留文本，我们没法从里面知道你要反馈什么。  如果是误提交，麻烦直接关掉；如果有具体问题，重新开一条并写清楚：现象、复现步骤、Codex++ 版本号、系统，以及相 |
| [#1208](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1208) | [Config]: 火山引擎额度充足但总是尝试连接、跑不起来 | 这条缺少可定位的信息，麻烦补充：1) config.toml 中对应的 model / model_provider / [model_providers.*] 片段（API Key 打码）；2) 在供应商管理里点「测试」的结果；3) ~/ |
| [#1219](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1219) | [Bug]: openai_error bad_response_status_ | 这条报告里的内容主要是你自己终端的一段 shell 报错，看不到 Codex++ 这边的实际错误。麻烦补充：1) Codex++ 版本与 Codex 版本；2) 管理工具里的日志（~/.codex-session-delete/codex- |
| [#1228](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1228) | [Feature]: 希望做账号AI切换功能 | 这条信息太少，我没法判断你指的是哪种切换。麻烦补充：(1)「账号」是指多个 ChatGPT 官方账号之间切换，还是多个中转站 Key（供应商）之间切换？(2) 现在你是怎么做的，哪里不够用？(3) 期望的交互在哪一层（管理器里一键切，还是想 |
| [#1237](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1237) | [Config]: CCS 配置 API 登录不上 | 这条没法定位，麻烦补充：(1) 完整的报错文案（Codex 界面上的原文）；(2) Base URL 和上游协议（Chat Completions 还是 Responses）；(3) 是纯 API 模式还是官方登录混入 Key；(4) 管理 |
| [#1278](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1278) | [Question]: 越更新越卡 | 这条需要更多信息才能处理。请补充：①Codex++ 版本与系统；②卡的是管理工具界面还是 Codex 对话界面（或两者）；③大概什么时候开始卡（打开即卡、长时间使用后、还是特定操作时）；④任务管理器里哪个进程占用高。另外可先用 v1.5.0 |
| [#1327](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1327) | [Bug]:（无标题）历史对话修复时报错卡死 | 这条报告里没有可定位的信息，我们没法复现或判断。麻烦补充：①修复历史对话时控制台的完整报错文本（或 Codex++ 诊断日志里对应的那几行）；②出问题的会话是哪个供应商/哪种接入模式；③先升级到最新版 v1.5.0 复测一次，如果还复现请把 |
| [#1332](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1332) | [Bug]: 为什么每次出现api报错 | 这条没有可用的信息（正文只有一个「111」），我这边没法判断是什么报错。麻烦重新开一条，带上：1) 报错原文（完整那一行）；2) 是哪个供应商、哪个模型；3) ~/.codex/config.toml 相关字段（脱敏）；4) 管理工具的诊断 |
| [#1412](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1412) | [Bug]: apply_patch 出现问题 | 信息不足，需要补充：1) apply_patch 报错的完整文字；2) 出问题的具体操作（改哪个文件、patch 内容大致长什么样）；3) 是否只在某个供应商/模型下出现。补充：apply_patch 是 Codex 客户端自带的工具，不是 |
| [#1466](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1466) | [Feature]: 点击codex插件出现错误 | 信息不足，无法定位。请补充：1）报错的完整文案或截图；2）Codex++ 版本与 Codex 客户端版本；3）是否用的是纯 API 中转（而非官方登录）；4）管理工具诊断日志里报错前后的记录。插件市场有已知的「26.818+ 远程认证 /  |
| [#1889](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1889) | 换肤换不了（无正文） | 这条内容为空，无法定位。请参照 #1784 的格式补充：Codex++ 版本、系统、点了哪些步骤、皮肤管理页的报错文字或截图。补全后我们再看；如果你遇到的就是 #1784 里的报错，直接到那条跟进即可。 |
| [#1994](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1994) | 重启 codex++ 无法遍历路径 | 这条报告缺少可定位的信息：「当前行为 / 预期行为 / 日志」三栏都是空的，只写了「点击重启 codex++」。麻烦补充：1) 具体的报错原文或截图；2) 「无法遍历路径」指的是管理工具里哪个界面、哪条路径；3) 诊断日志（管理工具→日志→ |

---

## 5. 可直接回答的咨询

31 条使用咨询，正文与代码已能给出答案，建议回帖后关闭。

| issue | 主题 | 回答要点 |
|---|---|---|
| [#322](https://github.com/BigPizzaV3/CodexPlusPlus/issues/322) | 请求加中间层处理图片（多模态） | 用户希望有个中间层先把图片处理掉，再以纯文本交给不支持多模态的国内模型。这个能力本仓已经实现：per-model 的图片处理模式含 Strip（剥离图片换占位符）与 Vlm（本地小模型分析后转文字）。 |
| [#346](https://github.com/BigPizzaV3/CodexPlusPlus/issues/346) | 不通过 ccx-windows-amd64 如何直连 llama.cpp | 用户在问怎么绕过 ccx 直连本地 llama.cpp。可以答：用管理工具的「自定义供应商」填 llama.cpp 的 OpenAI 兼容地址即可；ollama 与 lmstudio 供应商 ID 在本仓是保留名。 |
| [#555](https://github.com/BigPizzaV3/CodexPlusPlus/issues/555) | [Question]: 多台电脑之间怎么同步codex++的配置文件？ | 询问如何跨机同步 Codex++ 的工具与供应商配置，属可回答的用法问题；本仓未提供云端同步功能，但配置集中存放且供应商配置可复制。 |
| [#794](https://github.com/BigPizzaV3/CodexPlusPlus/issues/794) | 当前版本不支持 mimo-v2.5-pro（无思考模式、无 1M 上下文） | 用户的 config.toml 里 model_catalog_json 指向 cc-switch 生成的 catalog，同时顶层已写 model_context_window=1000000——按现有实现，外部 catalog 指针存在 |
| [#802](https://github.com/BigPizzaV3/CodexPlusPlus/issues/802) | 请问这个项目的开源协议是什么？ | 问许可证。仓库根目录有 LICENSE，README 明确写 AGPL-3.0-only。 |
| [#828](https://github.com/BigPizzaV3/CodexPlusPlus/issues/828) | [Question]: 怎么使用本地模型 | 纯使用提问——怎么接本地模型。Codex++ 的 RelayProfile 本就支持自定义 base_url、协议与无鉴权上游，指向本机服务即可。 |
| [#831](https://github.com/BigPizzaV3/CodexPlusPlus/issues/831) | 如何把界面语言切换成英文 | 用户问管理器界面怎么切成英文；答案是界面右上角有语言切换按钮（点一次持久化并重载），无需改配置文件或环境变量。 |
| [#835](https://github.com/BigPizzaV3/CodexPlusPlus/issues/835) | 老是提示 invalid params, context window exce | 这条错误由上游供应商返回（错误码 2013 不是本仓文案），说明实际发送的上下文超过了该模型声明的窗口；要么是模型窗口配得比供应商实际支持的大，要么是自动压缩阈值没压住。 |
| [#840](https://github.com/BigPizzaV3/CodexPlusPlus/issues/840) | deepseek-v4 的 1M 上下文改 config 不生效 | 用户手写 model_context_window=1000000 但不生效；原因是他手写的值与 Codex++ 生成的 catalog 并存时会被降级——只要存在外部/非托管的 model_catalog_json 指针，每模型窗口就退化 |
| [#850](https://github.com/BigPizzaV3/CodexPlusPlus/issues/850) | 硅基流动的 deepseek-v4pro/kimi2.6/glm5.1 都无法调 | 用户把 Base URL 填成 https://api.siliconflow.cn/v1 时测试报 400、换成 https://siliconflow.cn 测试 200 但对话无回复；核心是 base_url 该不该带 /v1、以及第 |
| [#854](https://github.com/BigPizzaV3/CodexPlusPlus/issues/854) | 配好第三方模型后仍要求登录账号或用 key | 用户配置第三方模型后仍被要求登录；这通常不是配置没生效，而是 Codex 侧仍判定必须走官方登录态，或者是切换后 auth.json 与 provider 不匹配（历史上有一笔正是这个），需要按步骤排查。 |
| [#860](https://github.com/BigPizzaV3/CodexPlusPlus/issues/860) | opencode go 套餐如何配置供应商 | 用户在问 opencode 的 Go 套餐能否纳入 Codex++ 供应商配置；这是使用咨询，仓库里没有 opencode 预设，需要按「自定义供应商 + 其 OpenAI 兼容端点」回答。 |
| [#923](https://github.com/BigPizzaV3/CodexPlusPlus/issues/923) | [Question]: 自动化执行的时候，默认使用的GPT模型，能调整吗？ | 与 #966 同一问题：自动化任务由 Codex 客户端发起，用的模型来自 Codex 默认模型，Codex++ 不参与该链路的模型选择，因此报错里仍是 gpt-5.4。 |
| [#940](https://github.com/BigPizzaV3/CodexPlusPlus/issues/940) | [Config]: 可以使用ccswitch的配置来配置1M上下文 | 用户给出的做法（让 ccswitch 生成 cc-switch-model-catalog.json，再在 config.toml 顶层写 model_catalog_json 指向它）在 Codex++ 上是可行的，但会被 Codex++ |
| [#966](https://github.com/BigPizzaV3/CodexPlusPlus/issues/966) | [Question]: 自动化任务模型报错 | 自动化（Automation）是 Codex 客户端自身的能力，它执行时用的模型由 Codex 内部选定/继承当时默认模型，而不是由 CodexPlusPlus 的供应商配置下发；报错显示上游确实收到了 gpt-5.4 而不是 deepse |
| [#1003](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1003) | [Config]: 切换供应商 api，配置文件是不是可以用同一个呢？ | 用户在问「切换供应商后原来配置的没了」是不是设计如此；可以明确回答：每个 profile 各自生成配置并覆盖 live config.toml，但非托管的第三方条目（MCP/hooks/features 等）会被保留，所以看起来「原来的没了 |
| [#1093](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1093) | [Bug]: 各种配置都填好了，http://127.0.0.1:57321/v | 用户把 127.0.0.1:57321/v1 当成可以在浏览器里打开的地址去测，其实它是 Codex++ 写进 config.toml 的本地协议代理入口，只接受 Codex 客户端发出的 Responses 请求，用浏览器 GET 当然打 |
| [#1190](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1190) | [Bug]: ~/.agent/skill 中手动添加的 skill 无法加载 | 用户使用的路径不对：Codex 的 skill 是文件系统约定，扫描的是 $CODEX_HOME/skills/<id>/SKILL.md，不是 ~/.agent/skill，因此放在那里永远不会被加载。 |
| [#1212](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1212) | [Question]: Responses 转 Chat Completions | 不会丢失。翻译层会保留 tools 声明并双向映射工具调用：请求侧把 Responses 的 function/custom 工具转成 Chat 的 function 工具，响应侧把上游的 tool_calls 回译成 function_c |
| [#1257](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1257) | [Question]: 中文转换问题（换电脑后界面不出中文） | 界面回退英文是因为官方 zh-CN 语言包没加载成功（Codex 走 Statsig/网络下发语言包），Codex++ 的「强制中文界面」开关就是为此准备的，属设置/环境问题而非 bug。 |
| [#1260](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1260) | [Bug]: config.toml 中的 model_context_wind | 报告本身判断正确：codex 的窗口以模型目录 ModelInfo 为准，config.toml 的 model_context_window 对目录里不存在的 slug 不构成硬覆盖。这正是本仓 model_catalog_json 机制 |
| [#1267](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1267) | [Question]: Windows 上想用手机远程控制，除登录 GPT 账号 | 问的是能否在不登录 ChatGPT 官方账号的前提下用手机远程控制 Codex；远程控制是官方 Codex 的能力，且依赖官方账号身份，第三方中转无法替代。 |
| [#1289](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1289) | [Feature]: 每次都要审批，怎么让审批消失 | 实为使用问题（如何减少 Codex 的审批弹窗），不是 Codex++ 的功能缺陷。审批策略由 Codex 本体的 approval 设置决定，可在 Codex 侧调整。可直接答复。 |
| [#1319](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1319) | [Question]: 新增/删除供应商后其它供应商的链接会变动 | 这是设计使然：走协议翻译的供应商 base_url 指向本机协议代理（127.0.0.1:57321/v1），代理端口按环境变化时所有供应商的链接会一起变；聚合/无鉴权模式的凭据也是统一占位符。可直接答复。 |
| [#1355](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1355) | [Question]: 如何彻底删除codex++ | 问如何彻底卸载 Codex++，是不是删掉几百 KB 的图标就行。纯支持类问题，仓库里有卸载相关的实现（installer 的卸载项、watcher 的 uninstall、startup 快捷方式清理）可以直接照此回答。 |
| [#1375](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1375) | [Question]: 历史会话误点归档后，如何恢复正常 | 问的是归档后如何恢复。归档由 Codex 客户端维护（archived_sessions 目录 + threads.archived 标记），应在客户端侧撤销，不是缺陷问题。 |
| [#1440](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1440) | [Question]: 之前用＋＋接的ds的api，怎么切换回gpt | 可回答的操作问题：怎么从第三方（DeepSeek）切回官方 GPT 订阅。本仓有官方模式（relay_mode = Official，不写 API 文件）与官方登录态检测，切换是既有能力。 |
| [#1449](https://github.com/BigPizzaV3/CodexPlusPlus/issues/1449) | [Question]: 权限选项为啥没有完全访问权限选项啊 | 可回答的问题：用户只看到「替我审批」和「请求批准」，找不到完全访问。完全访问（danger-full-access）在 Codex 客户端自己的权限/沙箱设置里，不在 Codex++ 的审批模式二选一里。 |
| [#2020](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2020) | [Question] 能否一个对话用官方账号、另一个用 deepseek API | 可回答的功能性问题。Codex 的模型/供应商是进程级配置（config.toml 的 model + model_provider），不是对话级，所以同一个桌面端进程里做不到逐对话切换；用户看到的那句「not supported when |
| [#2051](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2051) | [Question] pureApi 中转站用户当前最适合哪个 Codex 版本 | 这是提问而非缺陷，且用户自己做了很完整的版本矩阵实测。可回答的部分是第 2 问：9229 是 Codex++ 自己下发的 --remote-debugging-port，9329 是开「原生菜单本地化」时下发的 --inspect 端口（d |
| [#2120](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2120) | 大佬不更新了吗？ | 纯咨询「是否还维护」。事实是持续在更新：v1.5.0 发布于 2026-10-01，此后 main 上仍有提交。可直接据实答复。 |

---

## 6. 需进一步排查

250 条证据不足以定论，主要是报障时未附日志/版本/复现步骤。建议统一回帖要求补 `latest-status.json`、Codex++ 版本号与完整报错原文。

---

## 7. 执行结果（2026-10-03 当天）

| 项 | 数量 |
|---|---:|
| 审计覆盖 | 958 条（另有 83 条前两轮已审） |
| 本轮实际关闭 | **196** |
| 其中 `already-fixed-released` | 66 |
| 其中 `not-our-bug` | 71 |
| 其中 `duplicate` | 23 |
| 其中 `invalid-insufficient-info` | 36 |
| 关闭后 open 总数 | 1041 → **845** |
| 失败 | 0 |

每条关闭都附了针对性回复（非模板），并说明「补充证据后回复即可重开」。

### 关闭前的机器校验

- `already-fixed-released` 的 66 条逐条提取 commit sha，用 `git cat-file -e` 确认对象存在、`git tag --contains` 确认落在正式版内。**62 条自动通过**；4 条无 sha 的人工核实通过（#326 端口配置、#2162 绿点语义、#886 索引已修复、#1444 `bc3669e2` 在 v1.5.0 内）。
- **#1357 被改判**：原判 `already-fixed-released`，但 `git tag --contains c8f8ee30` 为空 —— 修复在 v1.5.0 之后，尚未发布。已改为 `already-fixed-unreleased`，不关闭。

---

## 8. 本轮修复交付

27 条 issue 已落地代码，分 10 笔提交（基线 `f55bb646` → `9caebfaa`）：

| 提交 | 覆盖 issue | 主题 |
|---|---|---|
| `0c2ba7e5` | #1931 #247 | 粘贴卡死 · macOS 调试端口冲突 |
| `3531ea18` | #862 #1488 | 本地 skill 显示 0 · 官方模式测试误报 |
| `2ac9f736` | #1209 #586 | web_search 错译为 custom_tool_call |
| `198db931` | #2031 | GLM 图片 data URL 前缀 |
| `cf7ee3b8` | #1394 #410 | system 消息未前置 / 空内容 |
| `a718bdd7` | #2072 #1975 | 插件悬空 · 状态灯对齐 |
| `f92286f7` | #1083 #1036 #1160 #2048 #2080 | 单实例误判 · 目录误判 · 版本检测 · Windows 弹窗 · 快捷方式同步 |
| `77f395cb` | #692 #1463 #1316 #772 | 市场显示名 · Fast UI · 可选依赖降级 |
| `000f1bfa` | #890 #597 #609 #1888 | 插件表保留 · 回填错误带原因 |
| `9caebfaa` | #240 #1424 #875 #1341 | canonical 会话补建 · WSL · 报错可诊断 · spawn_blocking |

**验证**：`cargo test --workspace` 523 passed。2 条 `native_browser` 失败经在会话前 HEAD（`f55bb646`）另开 worktree 复跑确认是**基线既有**（本机 tmpdir 不支持符号链接），非本次引入。

**判定无需改码**：#944（v1.2.8 起已修）、#1297、#2156、#495、#979。

**明确不动、需单独评估的三条**：

| issue | 为什么不改 |
|---|---|
| #1650 | 官方态不读 `config_contents` 属实，但直接叠加会重新引入 `1b214349` 修掉的 #2216 回归；需先剔除 provider 段与 `OPENAI_API_KEY` |
| #1082 | 推理方言按模型名子串猜，上游不认就 400。正解是在 `RelayProfile` 加显式方言开关（新配置项）；试过的「400 后剥离参数重试」因 `reqwest::Response` 只能消费一次、须重建响应而回退 |
| #1329 | **审计结论被推翻**：reqwest 已开 `system-proxy` feature 且 `auto_sys_proxy` 默认为真，系统代理本就自动生效；审计建议的显式挂载反而会关掉它 |

---

## 9. 下一步

1. **33 条「已修未发」需要一次发版**才能让存量用户拿到，其中含本报告的 P0/P1 项（#2330 #2363 #2350 #2362 等）与本轮 27 条。
2. 剩余可修复清单见 §3（74 条，本轮消化 27 条，剩余 47 条）。
3. `needs-investigation` 250 条建议统一回帖要求补 `latest-status.json`、版本号、完整报错原文与复现步骤。
