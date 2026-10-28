# 本地网关模式

两种模式完全独立、可以随时切换：

- **直连模式**（原来的方式）：每个 Agent 有自己的供应商列表（Codex、Claude、Grok 各管各的），切换时直接改写 Agent 的配置文件。
- **网关模式**：网关有自己的一份供应商池和组，所有接入的 Agent 共用整个池。Agent 的配置指向 Codex++ 本地 helper，
  请求在网关里按模型名选供应商。

网关的数据只存在 `<状态目录>/gateway.json`，不读也不写 `settings.json` 里直连模式的供应商；
从直连模式「导入」是复制一份，之后两边各改各的。没接入网关的 Agent 不受任何影响。

## 能做什么

- **共用供应商池**：内置厂商预设（复用 `presets.ts`，另加 Anthropic、xAI、Ollama、LM Studio）、自定义地址，
  或从直连模式的 Codex / Claude / Grok 供应商复制。官方登录账号不能复制：refresh token 每用一次就换新，
  刷新结果只写回原来那一条，复制会让两边互相把对方刷失效，订阅账号需要在网关里单独登录（待做）。
- **按模型名路由**：裸模型名发给池里提供它的供应商，多家时按池顺序依次尝试；`<供应商 id>/<模型>` 只发给那一家；
  `group/<组 id>` 按组展开；多家都有的裸模型名按它的自动组展开。
- **Claude Code 用任意协议的上游**：Anthropic Messages 请求在网关里转成 Responses，
  再复用现有代理链路发往 Chat / Responses / Anthropic 上游；上游本身是 Anthropic 协议时原样转发。
- **多 Key 轮换**：供应商填多个 Key（每行一个）时逐个展开成候选。429 冷却该 Key（按 Retry-After），
  额度耗尽冷却更久，401 停用到 Key 被改掉，连续 5xx 熔断供应商 60 秒；400/413/422 不切换。
  默认「会话粘性」，同一会话固定一个 Key，保护上游 prompt cache。
- **组**：网关自己的组，在「路由」页新建（Agent 里以 `group/<id>` 选）。成员是池里的模型（`<供应商 id>/<模型>` 或裸模型名）、别的组（可嵌套，
  出现环会跳过），或带 `*` 的通配符（按 `<供应商 id>/<模型>` 或模型名匹配池里现有的模型，上游新增的自动算进来）。
  Agent 把组当一个模型来选。
  - 模式：依次、轮换、按权重、额度优先（订阅账号剩余额度多的先试，没有数据的其次，用完的最后）、
    速度优先（按最近成功请求的平均首包耗时，没测过的先试一次）、手动（只发给选中的那个成员，失败也不换）。
  - 隐藏成员：组里的模型不再单独出现在 Agent 的模型列表里，只能通过组来选（直接写名字仍能用）。
- **自动组**：池里多家都有的模型自动成一个组（`route::auto_groups`，默认开，「自动建组」总开关）。成员是每家固定的那份
  `<供应商 id>/<模型>`，按池顺序；Agent 选不带供应商的模型名时走它。改过的设置（模式、会话固定、隐藏成员、手动选中、
  隐藏整个自动组）存在 gateway.json 的 `autoGroups`（键是小写模型名），没改过的不落盘、跟着池子变。
  - 会话固定：记住每个会话上次是哪个上游回答的，下次先试它，保住上游提示词缓存。缓存期内固定（默认，10 分钟）、
    整个会话固定、不固定。记录只在内存（`route::remember_answer`）。
- **从直连模式迁移**：供应商池「添加供应商 → 导入…」（池空时总览和供应商池顶部直接给入口）多选、一次导入：
  直连 Codex / Claude Code / Grok 的供应商、OpenCode 保存的 Key 复制进池（已导入的跳过）；
  聚合供应商搬成「成员进池 + 同名模型组」（故障转移 → 依次，按会话轮换 → 轮换 + 整个会话固定，
  按请求轮换 → 轮换 + 不固定，加权 → 按权重；按模型聚合不建组），模型路由规则没有对应功能、会在结果里列出；
  官方登录账号不能复制，列表里直接「去登录」。命令 `import_gateway_providers`，聚合转换见 `apply::import_codex_aggregate`。
- **订阅额度**：ChatGPT 订阅账号读每次响应的 `x-codex-*` 头，并每 10 分钟查 `wham/usage`；Copilot 查
  `copilot_internal/user` 的本月高级请求。只在内存（`gateway::quota`），供应商池的账号行显示剩余比例和各窗口重置时间。

## 结构

```
Claude Code ──POST /gw/claude/v1/messages──┐
Grok ─────────POST /gw/grok/v1/responses───┤  helper（常驻，默认 127.0.0.1:57321）
                                           │
                     gateway::server ──────┤ 鉴权（每个 Agent 一个本地 token）
                     gateway::route  ──────┤ 绑定 → 模型 → 候选（供应商 × Key）
                     gateway::pipeline ────┤ 逐个尝试；分类失败、冷却、切换
                                           │
             Anthropic 上游：原样转发 ◄────┤
             其它上游：protocol_proxy（ForcedRelay）◄─ Responses 中间格式
                                           │
             Responses SSE ──anthropic_outbound──► Anthropic SSE（Claude）
                           └─────────────────────► 原样（Grok）
```

- 状态文件：`<状态目录>/gateway.json`（接入设置、token、选项、供应商池、模型组），新建时权限 0600。
- Codex：开启时在直连模式的供应商列表里放一个托管供应商 `codex-plus-gateway`（地址 `/gw/codex/v1`，Responses），
  用直连模式自己的切换流程写进 `~/.codex`（回填、冲突检测、模型目录照旧）；关闭时切回开启前的供应商并删掉托管供应商。
  被网关接管时，Codex 供应商页里别的供应商不能直接选（前端按钮置灰，后端 `switch_relay_profile` 也会拦下），
  要先「断开并还原」；断开后「Codex++ 网关」那一项还在时，选回它等于重新接入。
  Codex 用 ChatGPT 账号登录着时，托管供应商带 openai 会话身份（`model_provider = "openai"`，传输在
  `[model_providers.custom]`）：Codex 保留官方登录（会话、额度、远程控制照旧），`openai_base_url` 指向本地代理，
  本地代理这一跳转给网关、不另记请求日志。Codex 自己没登录、池里有网关登录的 ChatGPT 账号时，接入时把它
  「借给」Codex：token 写进 Codex 的 auth.json，网关里标记 `codex_plus_lent_to_codex`；借出期间同一账号以 Codex
  那份为准，网关刷新后也写回 auth.json（始终一条 token 链，不会互相刷失效）。断开时把最新一代收回 gateway.json、
  从 auth.json 拿掉登录；删除借出的账号前也会先收回。都没有时才是普通自定义供应商。启动时若登录状态与
  托管供应商的身份不一致会重写一次。
  实时配置被别的工具改过、与直连模式的记录对不上时，接入前把 `config.toml` / `auth.json` 原样存到
  `backups/codex-before-gateway.json`（0600），不往上一个供应商回填；断开时切回原供应商后原样写回。改完设置会通知主窗口重新读取，避免旧设置整份写回。
- Claude Code：开启时写入托管供应商 `codex-plus-gateway`（`ANTHROPIC_BASE_URL` 指向网关），
  复用「关键字段」切换逻辑，hooks / permissions 等用户键不动；关闭时切回开启前的供应商。
- Grok：写入一个指向网关的 Responses 供应商；关闭时重新应用 Grok 页当前选中的供应商。
- Codex++ 不再提供 Claude Code / Grok 的供应商配置页：不接入网关时，它们照自己的配置文件直连；「断开并还原」恢复接入前的配置。

## 接入方式

Agent 打开「接入」后，池里所有打开的供应商的模型和所有模型组都会出现在 Agent 自己的模型列表里
（Grok 写进 `config.toml` 的模型列表，Claude Code 通过 opus / sonnet / haiku 档位映射）。每个 Agent 可在「挑选」里
排除某些供应商。池或模型组变化后自动重写开着的 Agent 配置。

## 界面

「本地网关」页分四个标签：

- **智能体**：已接入的 Agent 在上，「未设置」的在下。每行：图标、名字和本机 CLI 版本、状态、档位 / 模型入口、模型下拉、接入开关。
  模型下拉顶部是「默认（不经 Codex++）」和「断开 Codex++（还原配置）」，左侧可切到全部 / 收藏 / 某个供应商；
  未接入时选一个模型即接入。展开后逐行显示：在 Agent 里怎么切模型、模型列表（可挑选供应商）、Claude 档位、其它选项、
  改动的配置文件（可在访达中查看）和「断开并还原」。
- **供应商**：网关池里的供应商一行一个，可拖动排序、开关、展开看模型和 Key 状态、编辑、删除；下面是「添加供应商」目录：
  订阅、供应商（厂商 API）、中转、本机四组内置预设，「导入…」从直连模式复制。
- **供应商池「按模型」**：供应商 × 模型矩阵，每个模型一行、每家供应商一列，带各家当前状态和平均耗时；点列头回到列表展开那一家。
- **路由**：「我的组」和「自动组」两段卡片。卡片上直接改路由方式、会话固定、隐藏成员；成员显示状态、额度、平均耗时，
  手动模式下点成员锁定。自动组可隐藏 / 恢复 / 恢复默认，顶部是「自动建组」总开关。页面每 2.5 秒刷新状态。
- **总览「最近请求」**：每个 Agent 最近一次请求，点开逐条说明这次怎么走的（计划、每次尝试、谁回答）。
  数据来自 `gateway::activity`（只在内存，重启清零）。
- **会话**：Codex（现有会话库）、Claude Code（`~/.claude/projects/*/*.jsonl`）、Grok（`~/.grok/sessions/session_search.sqlite`）
  的本地会话放在一起，按 Agent 筛选、搜索，一键在终端执行 Agent 自己的恢复命令。只读。
- **用量**：请求结束时按「日期 × Agent × 供应商 × 模型」累加到 `<状态目录>/gateway-usage.json`（不受请求日志裁剪影响）；
  显示 token / 缓存读取 / 推理 / 调用、按天柱状图、按供应商 / 模型 / Agent 明细。供应商填了每百万 token 价格才估算费用；
  余额只查有公开接口的 DeepSeek、Kimi / Moonshot、OpenRouter、SiliconFlow。
- **网关**：监听地址、各 Agent 的接入地址与 token、多 Key 轮换方式、退出时恢复直连、托盘快捷面板开关。

托盘快捷面板（默认开）：左键点托盘图标弹出，能给各 Agent 换模型、看最近的请求；右键仍是原菜单、双击打开主窗口。
关掉开关时托盘恢复原来的点击行为。

## 代理设置（新旧两套）

- **新旧共用**（「网关 → 网关设置」末尾「本地服务与请求避震」）：本地服务状态、请求体上限、请求避震。网关与旧版
  本地协议代理跑在同一个本地服务里，这些设置对两套同时生效；Claude Code 经网关的请求也走请求避震。
- **旧版专用**（Codex「旧版代理设置」）：目录外模型映射（接入网关的 Agent 用网关自己的规则：没有的模型改写成主模型）、
  旧版供应商的独立代理。Codex 被网关接管时这页的旧版专用内容隐藏。
- **网关供应商的独立代理**：编辑供应商时的「代理」一栏，存在 gateway.json 的 `proxy` 字段；载入时放进
  `RelayProfile.gateway_proxy`，发请求时只用它（`relay_proxy::proxy_for_relay`），不按 id 查旧版代理表——
  两套的供应商 id 各自独立，按 id 查会误用同名旧版供应商的代理。

## 被别的工具接管

开着网关的 Agent，每次读网关状态时都会检查它的配置还指不指向网关（`gateway/drift.rs`）：
- Codex：`model_provider` 为 openai 时看 `openai_base_url` 是否指向本地代理，否则看该供应商的 `base_url` 是否是 `/gw/codex`；
- Claude Code：`settings.json` 的 `env.ANTHROPIC_BASE_URL` 是否是 `/gw/claude`；
- Grok：`config.toml` 里是否还有 `/gw/grok`。

被改掉时 Agent 卡片不再显示「已接入」，改成「配置被 X 改了，Agent 现在不经过 Codex++」并给「重新接入」；
能按地址或文件名认出 CC Switch（`:15721`、`cc-switch-*`）。窗口回到前台时会静默重读。

## 订阅登录

- **ChatGPT**：在网关里走与 Codex CLI 相同的浏览器 PKCE 登录（本机 1455 端口接回调）。token 只存在 gateway.json，
  凭据带 `codex_plus_gateway` 标记，刷新结果写回网关、不采纳 Codex 的 live 登录——与 Codex 自己的登录是两次独立的登录。
- **ChatGPT API**：OpenAI 给本机开源应用开放的「用 ChatGPT 登录」。首次登录以 `dynamic_agent_client` 注册，回调带回
  OpenAI 签发的 client_id（每个账号各存一份），本机标识 `urn:uuid:` 存在状态目录的 `chatgpt-api-host`。授权范围必须包含
  `chatgpt.tokens.use.direct`，否则账号不符合条件。请求直接发 `api.openai.com/v1/responses`，保留 Agent 自己的指令；
  去掉接口不收的字段（采样参数、`previous_response_id` 等）和 OpenAI 托管工具，`system` 改 `developer`，`store:false`、
  只流式（非流式客户端由网关拼回完整响应）。用量与 ChatGPT 本身共享 5 小时额度，超出时按额度用尽冷却。
  删除供应商时会让 OpenAI 吊销它的 refresh token。与上面的 ChatGPT（Codex 订阅）是两种独立的登录。
- **GitHub Copilot**：GitHub 设备码登录（VS Code Copilot 的 OAuth App），用 GitHub token 换 Copilot 的短期 token
  （约 30 分钟），请求前快过期时自动换新并写回 gateway.json；Chat Completions 协议，带 VS Code Copilot Chat 的请求头。
- **SuperGrok**：auth.x.ai 浏览器 PKCE 登录（Grok CLI 的公开客户端），推理走订阅额度的 `cli-chat-proxy.grok.com/v1`
  （Responses 协议，带 Grok CLI 的客户端标识头，版本号被 xAI 提高时会回 426，需要跟着改）；access token 快过期时用
  refresh token 换新。部分 SuperGrok 账号会被 xAI 的白名单拒绝（403），这时只能用 API Key。
- 账号类凭据都带 `codex_plus_account` / `codex_plus_gateway` 标记，与各 Agent 自己的登录互不影响。
- 没有接入：Claude 订阅（Anthropic 条款只允许官方客户端使用订阅凭据）、Gemini CLI（Google 已于 2026-06-18 停止个人账号的
  Code Assist，且其说明写明第三方使用违反政策）、Antigravity（客户端密钥未公开，只能从客户端里提取）。
- **OpenCode Go / Zen**（预设，用 OpenCode 控制台的 Key；也可在「导入…」里用 `opencode auth login` 保存在
  `~/.local/share/opencode/auth.json` 的 Key）：同一个网关里每个模型只在一种协议上提供，网关按 models.dev 目录（与价格表
  共用的缓存）逐个模型切换协议和地址——GPT / Grok / Muse 走 Responses，Claude、MiniMax、部分 Qwen 走 Anthropic，
  其余走 Chat；目录里没有的按模型名推断。每个请求带 `x-opencode-session`（按对话保持不变，缺了上游回 400）。
  Gemini 模型只走 Google 接口、免费模型只对 OpenCode 自己开放，都不列出。
- 其它订阅（Cursor、Kiro、Devin、Qoder、Zed、WorkBuddy、ZCode、Factory 等）需要模拟各家客户端调用私有接口，
  暂不内置。
- 除 ChatGPT API 外，以上登录都是冒用官方客户端身份调用未公开的接口，可能随时失效，也有账号被限制的风险。

## 限制

- 网关只在 Codex++ 运行时可用。可在「本地网关 → 网关」标签打开「退出时恢复直连」。
- 上游是 OpenAI 协议时，Anthropic 独有能力会有损失：`cache_control` 被忽略、
  服务端工具（web_search 等）只有原生 Responses 上游支持、PDF 附件取决于上游、
  思考块签名由网关隧道保存（`cpxgw1.` 前缀），换回 Anthropic 上游时会被剔除。
- ChatGPT 订阅作为 Claude Code 的上游依赖官方后端对请求的限制，需自行评估。

## 代码位置

| 模块 | 说明 |
|---|---|
| `crates/codex-plus-core/src/gateway/` | 网关全部逻辑 |
| `protocol_proxy.rs` 的 `ForcedRelay` | 让代理只发往网关指定的上游 |
| `launcher.rs` 的 `SseSink` / `ProxyOpener` | 复用 Responses 归一化链路 |
| `apps/codex-plus-manager/src-tauri/src/gateway_commands.rs` | 管理器命令 |
| `apps/codex-plus-manager/src/GatewayPage.tsx` | 「本地网关」页 |

测试：`tests/gateway_e2e.rs`（真实 helper + 模拟上游）、`tests/gateway_apply.rs`、
`tests/gateway_forced_relay.rs`，以及各模块内单元测试。
