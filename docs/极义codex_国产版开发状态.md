# 极义codex 国产版开发状态

更新时间：2026-06-12

## 目标

极义codex 的目标是做一个国产版 Codex：用户先登录极义本地账号，模型能力默认走阿里百炼千问兼容接口或后续极义自建中转，APIMart 保留为备选，不要求用户使用 ChatGPT 账号。

当前采用短期方案：不重写完整 Codex 客户端，而是在 macOS 上交付“极义手机号门禁 + 内置完整 Codex 客户端 + 阿里百炼千问纯 API 配置接管 + APIMart 备选 + 禁止 ChatGPT 回退”。

## 本次新增需求清单

1. 默认模型链路从 GPT / ChatGPT 体系切到千问系列：主链路使用阿里百炼 OpenAI 兼容接口，APIMart 只作为备选。
2. 极义codex 要作为国产版 Codex 独立运行：用户体系、模型配置、状态目录、Bundle ID、内置客户端均不依赖原版 Codex 或 ChatGPT 账号。
3. 国内用户打开主应用要先走手机号验证码登录；登录后再进入 Codex 使用界面，不能直接跳官方登录页或管理页。
4. 首页要有预置能力入口：插件、Skill、用户脚本集中放在工作台可见位置。
5. 本地部署阶段要有总后台管理能力：集中管理用户、团队、套餐额度、续费落账、封禁解封、支付回调、审计和对账。
6. 完成后要输出 macOS DMG、本地发布说明、在线飞书发布说明，并上传到 GitHub 仓库，邀请 `Anesthesial` 协作。

## 已完成

### 1. macOS 主入口与管理工具分离

- `/Applications/极义codex.app` 是用户主入口。
- `/Applications/极义codex 管理工具.app` 是维护入口。
- 主应用通过 `main` 模式运行，管理工具通过 `manager` 模式运行。
- 启动模式现在优先按 `.app` 路径识别：`极义codex.app` 一律进入主入口，`极义codex 管理工具.app` 一律进入管理入口。
- 兼容旧安装或手动启动的 `JiyiCodex.bin`，不会因为可执行名差异误判成管理工具。
- macOS 下已显式设置 Tauri `ActivationPolicy::Regular`，避免出现进程已启动但窗口不可见。

### 2. 本地手机号登录门禁

- 主应用未登录时显示手机号验证码登录界面。
- 登录状态保存在本地 SQLite：`~/.codex-session-delete/jiyi-codex-local.sqlite`。
- 当前支持本地干跑验证码；管理工具设置页可保存腾讯云短信生产配置，也可用腾讯云短信环境变量覆盖配置。
- 腾讯云短信非密钥参数写入 `~/.codex-session-delete/sms-provider.json`；`SecretId` 和 `SecretKey` 写入极义自己的 macOS 钥匙串默认账号 `jiyi-keychain:tencent-sms:secret-id` / `jiyi-keychain:tencent-sms:secret-key`。
- 短信默认保持本地干跑；关闭干跑且 `SmsSdkAppId`、签名、模板 ID、`SecretId`、`SecretKey` 完整后，验证码接口会通过腾讯云 `SendSms` 真实发送。
- 真实短信发送成功后才会写入本地验证码记录；如果腾讯云返回顶层错误、缺少 `SendStatusSet` 或任一状态不是 `Ok`，本地不会落验证码。
- 未登录时不会启动内置 Codex 客户端。
- 已登录或刚完成手机号验证后，主入口也不会自动启动内置 Codex；用户必须手动点击“进入 Codex”，避免误以为绕过了极义账号门禁。
- 本地会话已增加过期时间和设备标识，默认有效期为 30 天，可用 `JIYI_CODEX_SESSION_TTL_HOURS` 调整。
- 读取本地账号状态时会自动清理过期会话，过期后重新回到手机号验证码登录。
- 手机号登录后会写入本地用户、设备绑定和套餐额度模型：`local_users`、`local_user_devices`、`local_entitlements`。
- 默认套餐为 `local_trial / 本地试用`；可用 `JIYI_CODEX_LOCAL_PLAN_ID`、`JIYI_CODEX_LOCAL_PLAN_NAME`、`JIYI_CODEX_LOCAL_DAILY_TOKEN_LIMIT` 覆盖本地默认值。
- 管理工具首页已支持编辑当前登录用户的本地套餐 ID、套餐名称和每日 token 额度，保存后写回 `local_entitlements`。

### 3. 内置完整 Codex 客户端

- 极义codex 会从包内或运行时目录启动完整 Codex 客户端。
- 运行时客户端位置：

```text
~/Library/Application Support/极义codex.noindex/embedded-client/JiyiCodexClient.app
```

- 启动时使用精确 `.app` 路径，不再通过 `open -a Codex` 模糊匹配，避免误启动 `/Applications/Codex.app`。
- 运行时已经移除 `/Applications/Codex.app` 兜底：包内客户端固定为 `Contents/Resources/JiyiCodexClient.app`，如果缺失会提示重新安装完整客户端版 DMG，不会打开原版 Codex。
- 运行时客户端会规范化为 `JiyiCodexClient.app`、`com.jiyi.codex.client` 和 `CFBundleSignature=JIYI`，并移除原版 `codex://` URL Scheme 与 Sparkle 更新身份，避免 LaunchServices、登录回调和更新链路与原版 `com.openai.codex` 混淆。
- 运行时目录放在 `.noindex` 下，减少 Launchpad / Spotlight 出现多个极义 app 的问题。
- 旧包内路径 `Contents/Resources/Codex.app` 已不再被极义启动器接受，避免和原版 `Codex.app` 名称混淆。

### 4. 阿里百炼 / 极义中转纯 API 接管

- 默认供应商已改为 `阿里百炼默认`。
- 默认 Base URL：

```text
https://dashscope.aliyuncs.com/compatible-mode/v1
```

- 备选 Base URL：

```text
https://apimart.ai
```

- 默认上游协议为 OpenAI 兼容 Chat Completions，本地 helper 会转换为 Codex 需要的 Responses 形状。

- 默认模型：

```text
qwen3.7-plus
```

- 启动内置 Codex 前会强制写入纯 API 配置。
- 如果没有阿里百炼 / 极义中转 API Key，会阻止启动并提示配置缺失，不回退到 ChatGPT 登录。
- 默认启用极义本地请求代理：内置 Codex 的 `config.toml` 只写 `http://127.0.0.1:57321/v1`，`auth.json` 只写 `jiyi-local-proxy` 占位 token。
- 真实百炼 / 极义中转 Key 留在 macOS 钥匙串、环境变量或下载目录默认百炼 Key 文件中，极义设置文件只保留 `jiyi-keychain:` 引用，由本地 helper 转发 `/responses`、`/chat/completions` 和 `/models` 请求；管理工具只展示 Key 来源枚举，不展示 Key 明文或下载目录文件路径。
- 新增极义托管代理生产路径：管理工具可配置 `jiyiManagedProxyEnabled` 和 `jiyiManagedProxyEndpoint`；启用后本地 helper 使用 `jiyi-keychain:local-backend-session:active` 作为上游 Bearer Token 转发到托管代理 Endpoint，不使用百炼或中转站主 Key。
- 本地部署阶段可在管理工具“设置 / 极义账号服务端”里直接检查、启动和停止 `jiyi-managed-proxy`；PID 和日志保存在 `~/.codex-session-delete/jiyi-managed-proxy.pid` / `~/.codex-session-delete/jiyi-managed-proxy.log`，停止前会校验 PID 命令行包含 `jiyi-managed-proxy`，避免误杀原版 Codex 或其它进程。
- 如果上游是 Chat Completions，helper 继续负责转换为 Codex 需要的 Responses 格式；如果上游已是 Responses，则直接透传。

说明：`OPENAI_API_KEY` 和 `requires_openai_auth = true` 是 Codex 客户端兼容字段，不代表用户需要 ChatGPT 账号。

### 5. 本地用量记账与额度闸门

- 本地 helper 已为 `/responses` 请求增加用量记账，记录请求字节、响应字节、估算 tokens 和上游返回的 tokens。
- 支持 Responses 和 Chat Completions 两种上游返回格式的 usage 提取。
- 管理工具首页会显示今日请求数、今日用量、当前套餐和每日额度，并可直接保存当前用户套餐额度；供应商配置页可开关“本地用量记账”并设置全局每日 token 上限。
- 如果当前登录用户有本地套餐额度，优先使用 `local_entitlements.daily_token_limit`；否则使用全局设置里的每日 token 上限。
- 如果设置了每日 token 上限，请求前会先做本地预估；超过上限会返回 `429` 和 `jiyi_quota_exceeded`，不会继续请求上游模型服务。
- 新增用量事件会带上当前 `user_id` 和 `plan_id`，为后续服务端同步、团队额度和审计预留数据。
- 本地用量表保存在：

```text
~/.codex-session-delete/jiyi-codex-local.sqlite
```

说明：这是单机用量闸门，能用于本机验收和防误刷；公开分发仍需要服务端额度、子 key 或托管代理。

### 6. 本地账号迁移报告

- 管理工具首页已支持导出本地账号迁移报告。
- 报告写入：

```text
~/.codex-session-delete/reports/jiyi-local-identity-report-*.json
```

- 报告包含本地用户、设备绑定、套餐额度、当前登录态和按用户/套餐/日期聚合的用量汇总。
- 报告不会导出明文手机号，只包含脱敏手机号和稳定哈希，用于后续服务端账号、团队和额度系统迁移审计。

说明：这是本地服务端承接和远端服务端迁移的输入数据，不等同于已经完成远端生产账号体系。

### 7. 极义服务端同步通道

- 管理工具“设置”页已新增“极义账号服务端”配置。
- 可点击“同步到本地后端”，把本机脱敏账号、设备、套餐和用量摘要写入本地账号服务端库；当前登录态有效时会签发极义本地后端 session token。
- 可填写极义服务端同步 Endpoint 和同步 API Key；同步 API Key 会写入极义自己的 macOS 钥匙串，settings 中只保留 `jiyi-keychain:identity-sync:global` 引用。
- 可生成服务端同步请求包，也可直接向配置的 Endpoint 发起 `POST` 同步。
- 本地账号服务端库写入：

```text
~/.codex-session-delete/jiyi-codex-local-backend.sqlite
```

- 本地后端库包含 `backend_users`、`backend_devices`、`backend_entitlements`、`backend_usage_summaries`、`backend_sync_batches` 和 `backend_sessions`，只落脱敏手机号、稳定哈希和 token hash，不落明文手机号或明文 token。
- 明文后端 session token 只在签发时返回给管理端一次，随后写入极义自己的 macOS 钥匙串：

```text
jiyi-keychain:local-backend-session:active
```

- 本地 helper 同步暴露最小账号后端 API：

```text
GET  /jiyi/v1/health
POST /jiyi/v1/sessions/verify
POST /jiyi/v1/sessions/revoke
GET  /jiyi/v1/me
GET  /jiyi/v1/quota/today
POST /jiyi/v1/usage/record
```

- API 支持 `Authorization: Bearer <token>`，`sessions/verify` 也支持请求体 `accessToken`；服务端只用 token hash 查 `backend_sessions`，返回脱敏用户、设备、套餐和服务端额度快照，不返回明文手机号或明文 token。
- `/jiyi/v1/sessions/revoke` 只接受 `POST`，用于把当前后端 session 标记为 revoked；管理工具本地账号退出时会尝试调用同一吊销能力，并清理 `jiyi-keychain:local-backend-session:active`。
- `/jiyi/v1/quota/today` 会按服务端 session 查当前用户、当天用量汇总、套餐上限和剩余额度；无效 token 返回 `401`，不会匿名暴露用量或套餐。
- `/jiyi/v1/usage/record` 只接受 `POST`，用于按后端 session 实时增量写入 `backend_usage_summaries`；helper 在记录本地用量后，如果能读取 `jiyi-keychain:local-backend-session:active`，会同步写入本地后端额度摘要。

- 请求包写入：

```text
~/.codex-session-delete/reports/jiyi-identity-sync-request-*.json
```

- 响应审计写入：

```text
~/.codex-session-delete/reports/jiyi-identity-sync-response-*.json
```

- 请求体包含脱敏账号、设备、本地套餐、当前登录态和用量摘要；请求包授权头只保留 `Bearer <redacted>` 占位，响应预览会对同步 API Key 做脱敏，不落明文 Key。
- 设置页会展示本地托管代理运行状态、PID、健康检查、监听地址、上游地址、上游 Key 和同步 Key 配置状态，并提供“检查托管代理”“启动本地托管代理”“停止本地托管代理”按钮。点击启动会把托管代理 Endpoint 固定为本机 `http://127.0.0.1:57421`，上游仍读取当前激活的百炼 / 中转站供应商配置，避免把本机 Endpoint 当成上游造成循环。

说明：本地部署阶段已经有本地账号服务端库承接；公开分发仍需要真实远端服务端实现对应接口和跨设备账号能力。

### 8. 本地账号服务端库

- 新增 `codex_plus_core::local_backend`，作为本地部署阶段的极义账号服务端最小模型。
- 本地后端同步按批次记录 `backend_sync_batches`，并幂等 upsert 用户、设备、默认团队、团队成员、套餐和用量摘要。
- 当前本地账号 active session 有效时，本地后端会轮换签发 `jiyi-local-*` 访问 token；数据库 `backend_sessions` 仅保存 hash，旧 token 会被标记 revoked。
- 模型请求经 helper 完成后会先写入本地用量库；如果当前有本地后端 session token，会同步调用本地后端用量写入能力，增量更新当天用户/套餐维度的 `backend_usage_summaries`，让 `/jiyi/v1/quota/today` 不再只依赖手动同步后的历史摘要。
- 管理工具设置页展示本地后端库状态、同步批次、承接用户数、承接团队数、团队成员数、服务端 session 总数、有效 session、已吊销 session、最后同步时间、最近签发时间和最近吊销时间。
- 管理工具“安装维护”页发布前检查新增 `local_identity_backend`，用于证明本地服务端库 schema 和服务端 session 表可用。
- 托管代理新增 `GET /jiyi/v1/admin/teams` 团队运营查询和 `POST /jiyi/v1/admin/teams/entitlement` 团队套餐/额度调整；用户只读 Key 可查团队，计费 Key 可调整团队额度，团队额度变更会写入 `team_entitlement_updated` 审计事件。
- 本地后端新增 `backend_billing_renewals`，托管代理新增 `GET /jiyi/v1/admin/billing/renewals` 和 `POST /jiyi/v1/admin/billing/renewals`；计费 Key 可记录手工续费、企业转账或后续支付回调凭证，并在同一事务里更新用户或团队套餐额度，落账会写入 `billing_renewal_recorded` 审计事件。
- 本地后端新增 `backend_billing_payment_events`，托管代理新增 `POST /jiyi/v1/billing/payment-webhook` 和 `POST /jiyi/v1/admin/billing/reconcile`；支付回调 Key 只给网关回调使用，`paid` / `succeeded` / `trade_success` 等事件会幂等生成续费记录并更新用户或团队额度，重复回调按网关事件 ID 或订单号去重；审计会记录 `billing_payment_webhook_received` 和 `billing_payment_event_reconciled`，响应和审计只保存原始 payload 的 SHA，不暴露明文支付 payload。
- 托管代理新增通用支付回调 HMAC-SHA256 验签：配置 `JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET` 后，`POST /jiyi/v1/billing/payment-webhook` 必须携带 `X-Jiyi-Payment-Timestamp` 和 `X-Jiyi-Payment-Signature`，签名内容为 `timestamp + "." + raw_body`，5 分钟有效，错误、缺失或过期签名都会在落账前返回 401；健康检查和管理工具设置页会展示 `paymentWebhookSignatureConfigured` / “支付验签”状态。
- 托管代理新增支付宝/微信支付官方 RSA-SHA256 验签：配置 `JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY` / `_PATH` 后，匹配 `alipay` 的支付回调必须携带支付宝 `sign` / `sign_type=RSA2` 并按支付宝参数规则验签；配置 `JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY` / `_PATH` 后，匹配 `wechatpay` / `wxpay` / `weixin` / `微信` 的支付回调必须携带 `Wechatpay-Timestamp`、`Wechatpay-Nonce` 和 `Wechatpay-Signature` 并按微信支付 v3 规则验签；健康检查和管理工具设置页会展示 `paymentWebhookAlipaySignatureConfigured` / `paymentWebhookWechatpaySignatureConfigured`。

说明：这是“后端现部署本地”的账号、团队、额度、手工续费、通用支付回调承接、HMAC 回调验签、支付宝/微信支付官方 RSA 验签和单节点自动对账闭环；不是远端多租户账号、正式管理员 RBAC、支付平台证书自动下载/轮换、商户后台对账单拉取或复杂风控系统。

### 9. 管理工具总后台

- 管理工具新增独立“总后台”导航页。
- 总后台集中展示用户数、团队数、今日请求量、今日 token、最近续费和审计事件。
- 用户运营表展示脱敏手机号、访问状态、套餐、每日额度、今日使用、剩余额度、设备数、session 数和最近同步时间。
- 可在总后台直接编辑用户套餐 ID、套餐名称、每日 token 额度，也可封禁或解封用户。
- 团队运营表展示团队名称、团队套餐、额度、今日用量、成员数和最近同步时间。
- 可在总后台编辑团队套餐 ID、套餐名称和每日 token 额度。
- 续费落账表单支持用户或团队维度的手工续费、企业转账或支付凭证记录，并在同一事务里更新套餐额度。
- 对账按钮可触发本地单节点支付事件重对账。
- 审计表展示事件类型、actor、subject、时间和脱敏 metadata 摘要。
- 总后台复用本地后端和托管代理管理接口，当前是单机本地部署版；远端多租户管理员账号、正式 RBAC 和复杂风控仍属于下一阶段。

### 10. 禁止官方 ChatGPT 登录回退

- 管理端供应商默认值已从官方登录改为极义纯 API。
- 后端已阻止 `clear_relay_injection` 切回官方 ChatGPT 登录模式。
- 后端已阻止 `apply_relay_injection` 使用官方登录或混合 API 模式。
- 前端供应商编辑器不再提供“官方登录”作为可选模式。
- 历史官方/混合 profile 只作为兼容读取显示，不能切换生效。

### 11. ChatGPT 登录页兜底遮罩

- 注入脚本会识别 Codex 官方登录页文本，例如 `Continue with ChatGPT`。
- 如果意外进入官方登录页，会显示极义账号接管提示，不引导用户登录 ChatGPT。

### 12. 原版 Codex 隔离

- `/Applications/极义codex.app`、`/Applications/极义codex 管理工具.app` 和内置客户端都使用极义自己的 bundle 身份：

```text
com.jiyi.codex
com.jiyi.codex.manager
com.jiyi.codex.client
```

- 极义运行态不再使用原版 `~/.codex` 作为工作目录。
- 极义自己的状态目录：

```text
~/.codex-session-delete
```

- 极义内置 Codex 的 `CODEX_HOME`：

```text
~/.codex-session-delete/codex-home
```

- 极义内置 Codex 的隔离 `HOME`：

```text
~/.codex-session-delete/home
```

- 极义内置客户端的浏览器用户数据目录：

```text
~/.codex-session-delete/codex-client-user-data
```

- macOS 启动内置客户端时使用：

```text
open -n -W --env CODEX_HOME=~/.codex-session-delete/codex-home --env HOME=~/.codex-session-delete/home --env XDG_CONFIG_HOME=~/.codex-session-delete/home/.config --env OPENAI_API_KEY= --env APIMART_API_KEY= ... --args --user-data-dir=~/.codex-session-delete/codex-client-user-data
```

- 主壳启动和内置客户端启动时都会开启原版配置守护：如果上游运行时把极义路径、百炼/APIMart、`jiyi-local-proxy`、`jiyi-keychain:` 或 `qwen3.7-plus` 写回原版 `~/.codex/config.toml` / `auth.json`，会自动恢复启动前快照；原版用户自己配置的普通 `OPENAI_API_KEY` 不再被误判为极义污染。
- 管理工具“安装维护”页新增“修复原版隔离”：扫描原版 `~/.codex/config.toml`、`~/.codex/auth.json` 和原版 Codex App Support 的 `Preferences`、`Local State`、`Network Persistent State`、`Reporting and NEL` 等关键状态文件；发现极义、百炼或 APIMart 痕迹时会先备份到 `~/.codex-session-delete/original-codex-isolation-backups/`，再清理对应字段或缓存文件，不修改 `/Applications/Codex.app` 本体。
- 旧版本极义 app 备份已经迁入 `~/.codex-session-delete/app-backups.noindex/`，并写入 `.metadata_never_index`；安装脚本会把备份内顶层和嵌套 `.app` 递归改名为 `.app.disabled` 并反注册，避免 Launchpad / Spotlight 把备份包当成多个可打开应用。

### 11. 发布前检查闸门

- 管理工具“安装维护”页已新增“发布前检查”。
- 检查项包括主应用、管理工具、内置客户端的 bundle id，codesign 校验，DMG 是否为完整客户端包，托管代理 sidecar 是否存在，主入口是否无原版兜底，内置客户端 URL Scheme 是否隔离，内置客户端浏览器数据是否隔离，内置客户端环境变量隔离，原版 `~/.codex` 和原版 Codex App Support 是否被极义污染，本地账号 session，腾讯云短信生产配置、本地短信配置文件和极义 Keychain 密钥来源，本地用户套餐模型，本地用量记账，本地账号服务端库，极义账号服务端同步，极义托管代理配置，上游 Key 分发风险，极义隔离 Codex Home 是否写入真实 Key，以及 Developer ID / 公证环境。
- 检查结果分为 `正常`、`风险`、`失败`；风险项不会阻止本机验收，但公开发布前必须处理。

### 12. 在线飞书发布文档

- 已创建飞书在线发布说明：

```text
https://bchje44bsl.feishu.cn/docx/GIxYdIdbSokkIexO3oGcLyGinZg
```

- 本地源文件：

```text
docs/release/极义codex_1.2.4_macOS_发布说明.xml
```

- 已用 `lark-cli docs +fetch --api-version v2` 读取验证在线文档可访问。

## 本机验收记录

### 2026-06-09 本机安装状态

已重新构建并覆盖安装到：

```text
/Applications/极义codex.app
/Applications/极义codex 管理工具.app
```

签名校验结果：

```text
/Applications/极义codex.app: valid on disk
/Applications/极义codex 管理工具.app: valid on disk
```

已重新生成完整客户端 DMG：

```text
dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg
```

DMG 大小：

```text
453M
```

DMG 挂载验收：

```text
JiyiCodex
com.jiyi.codex
com.jiyi.codex.client
com.jiyi.codex.manager
/Volumes/极义codex/极义codex.app: valid on disk
/Volumes/极义codex/极义codex 管理工具.app: valid on disk
```

构建验证：

- `cargo test -p codex-plus-core -- --nocapture` 通过。
- `cargo test -p codex-plus-core local_account -- --nocapture` 通过。
- `cargo test -p codex-plus-core local_usage -- --nocapture` 通过。
- `cargo test -p codex-plus-core secret_store -- --nocapture` 通过。
- `cargo test -p codex-plus-core official_codex_config_guard -- --nocapture` 通过。
- `cargo test -p codex-plus-manager startup_options -- --nocapture` 通过。
- `cargo test -p codex-plus-manager release_readiness -- --nocapture` 通过。
- `cargo test -p codex-plus-manager -- --nocapture` 通过。
- `cargo test -p codex-plus-core --test protocol_proxy -- --nocapture` 通过。
- `cargo test -p codex-plus-core --test launcher -- --nocapture` 通过。
- `npm run check` 通过。
- `cargo build -p codex-plus-launcher -p codex-plus-manager --release` 通过。
- `codesign --verify --deep --strict` 已验证主应用和管理工具。
- `JiyiCodex-1.2.4-macos-arm64.dmg` 挂载后已验证主应用、管理工具、内置客户端 bundle id 和签名。
- 2026-06-09 22:57 重新生成 DMG，包含管理工具首页本地套餐编辑入口；挂载检查确认 DMG 460M、主应用 981M、内置客户端 952M，bundle id 仍为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`。
- 2026-06-09 23:15 重新生成 DMG，包含管理工具首页“导出账号报告”入口；挂载检查确认 DMG 459M、主应用 981M、内置客户端 952M，bundle id 仍为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`。
- 2026-06-09 23:37 重新生成 DMG，包含设置页“极义账号服务端”和“生成同步请求包”入口；挂载检查确认 DMG 456M，SHA-256 为 `41e27f5f516974b98bafff9eed37be0423f9b21420a2a52b56688e4e15ca3e95`，主应用 981M、管理工具 28M、内置客户端 952M，bundle id 仍为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`。
- 2026-06-09 23:42 已将 23:37 DMG 安装到 `/Applications`，旧极义 app 备份到 `~/.codex-session-delete/app-backups/20260609-234121-identity-sync-request`；验证原版 `/Applications/Codex.app` PID 727 仍在运行，`~/.codex/config.toml` 与 `~/.codex/auth.json` 无极义/APIMart/Keychain 痕迹；当前 `auth_state_count=0`，打开极义主应用后只启动 `JiyiCodex`，未启动 `JiyiCodexClient.app`。
- 2026-06-09 23:54 重新生成并安装 DMG，包含发布前检查新增的“腾讯云短信生产配置”和“极义账号服务端同步”风险项；挂载检查确认 DMG 460M，SHA-256 为 `f2ac230559d1112ccf2363577d1984b84e468fe2c688b359f9c4680050249ad4`，主应用 981M、管理工具 28M、内置客户端 952M，bundle id 仍为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`；安装后原版 `/Applications/Codex.app` PID 727 仍在运行，`auth_state_count=0` 时打开极义主应用只启动 `JiyiCodex`，未启动 `JiyiCodexClient.app`。
- 2026-06-10 00:09 重新生成并安装 DMG，包含“同步到服务端”实际 POST 能力和响应审计脱敏；挂载检查确认 DMG 460M，SHA-256 为 `ac7a466c10d611a0e95f550344e1ce04a5ee015efa4b5414c86eb53772821323`，主应用 981M、管理工具 28M、内置客户端 952M，bundle id 仍为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`；安装后原版 `/Applications/Codex.app` PID 727 仍在运行，`auth_state_count=0` 时打开极义主应用只启动 `JiyiCodex`，未启动 `JiyiCodexClient.app`。
- 2026-06-10 00:30 重新生成并安装 DMG，包含“同步到本地后端”和 `local_identity_backend` 发布门禁；挂载检查确认 DMG 460M，SHA-256 为 `fd97c560e8d8d55ee78e2cb31ee1ad60a7916707093cf1c70309d3f1cc056e56`，主应用 981M、管理工具 28M、内置客户端 952M，bundle id 仍为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`；安装后原版 `/Applications/Codex.app` PID 727 仍在运行，原版 `~/.codex` 未检测到极义/APIMart/Key 痕迹，`auth_state_count=0` 时打开极义主应用只启动 `JiyiCodex`，未启动 `JiyiCodexClient.app`；本地后端库 `~/.codex-session-delete/jiyi-codex-local-backend.sqlite` 已初始化 `backend_*` schema。
- 2026-06-10 00:46 新增本地账号服务端 session 签发：`backend_sessions` 只保存 token hash，管理端把明文 token 写入 `jiyi-keychain:local-backend-session:active`；`cargo test -p codex-plus-core local_backend -- --nocapture`、`cargo test -p codex-plus-core secret_store -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture` 和 `npm run check` 均通过。
- 2026-06-10 00:50 重新生成并安装 DMG，包含本地后端 session token 签发和 Keychain 保存；挂载检查确认 DMG 459M，SHA-256 为 `26f7620bff6c233ebd488ba2f21495e3f5fc4c9c9f9f5f20efd21b62f16d22e2`，主应用 981M、管理工具 28M、内置客户端 952M，bundle id 仍为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`；安装后原版 `/Applications/Codex.app` PID 727 仍在运行，原版 `~/.codex` 未检测到极义/APIMart/Key 痕迹，`auth_state_count=0` 时打开极义主应用只启动 `JiyiCodex`，未启动 `JiyiCodexClient.app`；安装包二进制已包含 `backend_sessions` 和 `local-backend-session:active`。
- 2026-06-10 01:08 重新生成并安装 DMG，完成极义与原版 Codex 的路径级隔离：包内客户端改为 `Contents/Resources/JiyiCodexClient.app`，`Contents/Resources/Codex.app` 已不存在；SHA-256 为 `49ef1d714c44773c6ebfa11d6058a5420217f1658fd6af92e06bcfe71a93dfa6`，DMG 460M，主应用 981M、管理工具 28M、内置客户端 952M。安装后 `/Applications/Codex.app` 仍为 `com.openai.codex`，极义为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`；原版 `~/.codex` 未检测到极义/APIMart/Key 痕迹；`auth_state_count=0` 时打开极义只启动 `JiyiCodex`，未启动 `JiyiCodexClient.app`，会停在手机号门禁。旧极义备份目录中的 `.app` 已改名为 `.app.disabled` 并反注册，Spotlight 只剩 `/Applications/极义codex.app` 和 `/Applications/极义codex 管理工具.app` 两个极义入口。
- 2026-06-10 01:18 新增极义本地账号后端 HTTP API：helper 端口提供 `/jiyi/v1/health`、`/jiyi/v1/sessions/verify` 和 `/jiyi/v1/me`；`verify_session_token` 只按 token hash 查 `backend_sessions`，返回脱敏用户、设备和套餐。`cargo test -p codex-plus-core local_backend -- --nocapture`、`cargo test -p codex-plus-core default_helper_serves_local_backend_account_api -- --nocapture` 和 `cargo fmt --all --check` 均通过。
- 2026-06-10 01:35 重新生成并安装 DMG，完成 URL Scheme 和 Spotlight 级隔离：SHA-256 为 `b39b8e121ea6a5bab21681fd8517c2b704fee63d490781b0f0d3ceaebc4f2065`，DMG 460M，主应用 981M、管理工具 28M、内置客户端 952M；包内和已安装的 `JiyiCodexClient.app` 均为 `com.jiyi.codex.client`，`CFBundleURLTypes`、`SUPublicEDKey` 已移除，`CFBundleSignature=JIYI`；`mdfind` 只剩 `/Applications/Codex.app`、`/Applications/极义codex.app` 和 `/Applications/极义codex 管理工具.app` 三个真实入口；原版 `/Applications/Codex.app` PID 727 保持运行，`auth_state_count=0` 时打开极义只启动 `JiyiCodex`，未启动 `JiyiCodexClient.app`。
- 2026-06-10 02:06 新增腾讯云短信生产配置管理：管理工具设置页可保存短信区域、`SmsSdkAppId`、签名、模板 ID、有效期、模板参数顺序和干跑开关；`SecretId` / `SecretKey` 只写入极义 Keychain 默认账号，前端保存后清空密钥输入框。验证码接口会读取本地短信配置文件和 Keychain，腾讯云响应必须无顶层错误且 `SendStatusSet` 全部为 `Ok` 才写入本地验证码。`cargo test -p codex-plus-core local_account -- --nocapture`、`cargo test -p codex-plus-core secret_store -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`、`cargo fmt --all --check`、`git diff --check` 和 `npm run check`（在 `apps/codex-plus-manager`）均通过。
- 2026-06-10 02:17 重新生成并安装 DMG，包含腾讯云短信生产配置管理入口和 `hdiutil create -fs HFS+` 打包修复；新包 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg` 为 446M，SHA-256 为 `b77fcf4cc02370efb0ee79c7854ba53f15f7292936b609b1fd88f4ba4ae12e72`。挂载校验：主应用 981M、管理工具 28M、内置客户端 952M，bundle id 仍为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，内置客户端 `CFBundleURLTypes` 和 `SUPublicEDKey` 缺失且 `CFBundleSignature=JIYI`。安装后 `/Applications/Codex.app` 仍为 `com.openai.codex` 且 PID 727 保持运行；`/Applications/极义codex.app` 为 988M，内置客户端 959M；`auth_state_count=0` 时通过 `open -n /Applications/极义codex.app` 启动只出现 `JiyiCodex` 主进程，未启动 `JiyiCodexClient.app`。
- 2026-06-10 02:41 去除主入口自动进入 Codex 行为：已有本地登录态或刚完成手机号验证时，主应用仍停留在极义账号门禁页，只有点击“进入 Codex”才启动内置客户端；新增 `main_entry_does_not_auto_launch_codex_after_local_auth` 回归测试。重新生成并安装 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg`，包体 446M，SHA-256 为 `94ad5a44c49c10a2a56d7ab09cc2a2c096547cfcdd1f55c609cd0ca05bd19503`；旧极义 app 备份到 `~/.codex-session-delete/app-backups/20260610-024035-manual-gate-isolation`。安装后 `/Applications/Codex.app` 仍为 `com.openai.codex` 且 PID 727 保持运行，`/Applications/极义codex.app` 为 `com.jiyi.codex`，内置客户端为 `com.jiyi.codex.client`；`auth_state_count=0` 时通过 `open -n /Applications/极义codex.app` 启动只出现 `JiyiCodex` 主进程，未启动 `JiyiCodexClient.app`，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义/APIMart/Keychain 痕迹。
- 2026-06-10 03:06 新增本地账号服务端额度查询 API：`GET /jiyi/v1/quota/today` 使用 `Authorization: Bearer <token>` 校验 `backend_sessions`，返回当前用户当天用量、套餐上限、剩余额度和 `limit_source`；无效 token 返回 `401`。这把本地后端从“只验证账号”推进到“可提供服务端额度快照”，为后续 APIMart 子 key、托管代理和跨设备额度服务预留接口。`cargo fmt --all --check`、`cargo test -p codex-plus-core local_backend -- --nocapture`、`cargo test -p codex-plus-core default_helper_serves_local_backend_account_api -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`、`cargo build -p codex-plus-launcher -p codex-plus-manager --release` 和 `npm run check` 均通过。重新生成并安装 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg`，包体 446M，SHA-256 为 `1569a2570cefe40cd4b4cc2ec19b2ef2d83863cdf74aadbe19289f147b913bd1`；包内和安装后均验证 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，内置客户端位于 `Contents/Resources/JiyiCodexClient.app`，没有 `Contents/Resources/Codex.app`；旧极义 app 备份到 `~/.codex-session-delete/app-backups/20260610-030137-quota-api`。安装后 `/Applications/Codex.app` 仍为 `com.openai.codex` 且 PID 727 保持运行，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义/APIMart/Keychain 痕迹，`auth_state_count=0` 时打开极义只启动 `JiyiCodex`，未启动 `JiyiCodexClient.app`。在线飞书发布说明已追加同一结论并回读确认，revision 为 50。
- 2026-06-10 03:19 新增本地账号服务端 session 吊销能力：`POST /jiyi/v1/sessions/revoke` 使用同一 `Authorization: Bearer <token>` / `accessToken` 校验方式，只接受 `POST`，成功后把 `backend_sessions.revoked_at_ms` 和 `last_seen_at_ms` 写为吊销时间；后续 `/sessions/verify`、`/me` 和 `/quota/today` 都会拒绝该 token。管理工具本地账号退出时会尝试吊销当前 `jiyi-keychain:local-backend-session:active` 对应 token，并清理 Keychain 条目；设置页和发布前检查同步展示有效 session 与已吊销 session 数。`cargo fmt --all --check`、`cargo test -p codex-plus-core local_backend -- --nocapture`、`cargo test -p codex-plus-core default_helper_serves_local_backend_account_api -- --nocapture`、`cargo test -p codex-plus-core secret_store -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`、`cargo build -p codex-plus-launcher -p codex-plus-manager --release` 和 `npm run check` 均通过。重新生成并安装 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg`，包体 446M，SHA-256 为 `77e4bb4e3df457be65a8568daa6c486cc9432ffa39dfdb7ef122d16a40e9869b`；包内挂载校验主应用 981M、管理工具 28M、内置客户端 952M，bundle id 仍为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，codesign 通过。旧极义 app 备份到 `~/.codex-session-delete/app-backups/20260610-032544-session-revoke`；安装后 `/Applications/Codex.app` 仍为 `com.openai.codex` 且 PID 727 保持运行，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义/APIMart/Keychain 痕迹，`auth_state_count=0` 时打开极义只启动 `JiyiCodex`，未启动 `JiyiCodexClient.app`。在线飞书发布说明已追加同一结论并回读确认，revision 为 51。
- 2026-06-10 03:36 新增本地账号服务端实时用量记账：helper 在本地用量记账后，会尝试读取 `jiyi-keychain:local-backend-session:active` 并调用 `LocalBackendStore::record_usage_event`，按当前后端 session 增量更新 `backend_usage_summaries`；同时暴露 `POST /jiyi/v1/usage/record`，供后续托管代理或远端服务按同一 JSON 形状写入服务端用量。该能力让 `/jiyi/v1/quota/today` 能反映最新请求后的服务端额度摘要，而不是只依赖管理工具手动同步。`cargo fmt --all --check`、`cargo test -p codex-plus-core local_backend -- --nocapture`、`cargo test -p codex-plus-core default_helper_serves_local_backend_account_api -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`、`cargo build -p codex-plus-launcher -p codex-plus-manager --release`、`npm run check` 和 `git diff --check` 均通过。重新生成并安装 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg`，包体 446M，SHA-256 为 `ed3cc8f15a9508d1b4df51e401c2300b122691dd57b844555afd0261487cce81`；挂载校验主应用 981M、管理工具 28M、内置客户端 952M，bundle id 仍为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，codesign 通过。旧极义 app 备份到 `~/.codex-session-delete/app-backups/20260610-034241-usage-record`；安装后 `/Applications/Codex.app` 仍为 `com.openai.codex` 且 PID 727 保持运行，`/Applications/极义codex.app` 为 `com.jiyi.codex`，管理工具为 `com.jiyi.codex.manager`，内置客户端为 `com.jiyi.codex.client`；原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义/APIMart/Keychain 痕迹；`auth_state_count=0` 时通过 `open -n /Applications/极义codex.app` 启动只出现 `JiyiCodex` 主进程，未启动 `JiyiCodexClient.app`。在线飞书发布说明已追加同一结论并回读确认，revision 为 52。
- 2026-06-10 04:04 新增极义托管代理生产路径：设置页新增“启用极义托管代理”和托管代理 Endpoint；底层 `effective_relay_target` 会在托管代理开启时强制使用 Responses 协议，把上游 Base URL 改为托管代理 Endpoint，并使用 `jiyi-keychain:local-backend-session:active` 中的极义后端 session token 作为 Bearer Token，不读取或分发 APIMart 主 Key。启动内置 Codex 时仍强制写入本机 `127.0.0.1` 代理配置，不把 session token 写入隔离 Codex Home；发布前检查新增 `managed_proxy_service`，并在托管代理开启且 settings 无明文 Key 时把 `api_key_distribution` 判为正常。`cargo fmt --all --check`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-core managed_proxy -- --nocapture`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-manager release_readiness -- --nocapture` 和 `npm run check` 均通过。在线飞书发布说明已追加同一结论并回读确认，revision 为 53。
- 2026-06-10 04:15 新增内置客户端浏览器用户数据隔离：极义启动器在打开 `JiyiCodexClient.app` 时会创建 `~/.codex-session-delete/codex-client-user-data`，并通过 `open ... --args --user-data-dir=...` 强制 Electron/Chromium 使用极义专用用户数据目录；发布前检查新增 `embedded_client_user_data_isolation`，防止内置客户端复用原版 `/Users/lv/Library/Application Support/Codex` 的窗口、缓存、登录态或请求配置。`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-core launcher_macos_open_command -- --nocapture`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-core default_jiyi_browser_user_data_dir -- --nocapture` 和 `CARGO_BUILD_JOBS=1 cargo test -p codex-plus-manager release_readiness -- --nocapture` 均通过。在线飞书发布说明已追加同一结论并回读确认，revision 为 54。
- 2026-06-10 04:26 重新生成并安装 DMG，包含内置客户端浏览器用户数据隔离和托管代理客户端路径；新包 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg` 为 446M，SHA-256 为 `a5a37cc79e54c5fe64819248f9881e1e3a6b9ccd49d45b2288c4c26ec27d3afe`。挂载校验：主应用 981M、管理工具 28M、内置客户端 952M，bundle id 为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，codesign 通过；内置客户端没有旧 `Contents/Resources/Codex.app`、没有 `CFBundleURLTypes`、没有 `SUPublicEDKey`，`CFBundleSignature=JIYI`。安装后旧极义 app 备份到 `~/.codex-session-delete/app-backups/20260610-042604-browser-user-data-isolation`，`/Applications/极义codex.app` 为 990M、内置客户端 960M、管理工具 28M；原版 `/Applications/Codex.app` 仍为 `com.openai.codex` 且 PID 727 保持运行，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹；安装后的极义主入口和静默 launcher 二进制均包含 `codex-client-user-data` 与 `--user-data-dir=`。在线飞书发布说明已追加同一结论并回读确认，revision 为 55。
- 2026-06-10 04:53 重新生成并安装 DMG，彻底收口原版 Codex 隔离：内置客户端启动命令新增 `XDG_CONFIG_HOME` / `XDG_DATA_HOME` / `XDG_CACHE_HOME` 隔离，并显式清空通用 `OPENAI_*`、`CUSTOM_OPENAI_API_KEY`、`APIMART_API_KEY` 和 `JIYI_CODEX_API_KEY` 环境变量；原版配置守护不再把普通 `OPENAI_API_KEY` 误判为极义污染。新包 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg` 为 447M，SHA-256 为 `ff951af94c8926394fd7ff338afe0e07bef7fe737c91eca710ca7e0a363315a0`。挂载校验：主应用 981M、管理工具 28M、内置客户端 952M，三者 `CFBundleSignature=JIYI`、bundle id 为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，codesign 通过；内置客户端没有旧 `Contents/Resources/Codex.app`，没有 `CFBundleURLTypes`。安装后旧极义 app 备份到 `~/.codex-session-delete/app-backups.noindex/20260610-045300-env-isolation`，历史 `app-backups` 已迁入 `.noindex` 归档并重建 LaunchServices 索引；`/Applications` 与 `~/Applications` 下只剩 `/Applications/极义codex.app` 和 `/Applications/极义codex 管理工具.app` 两个极义入口。实际打开主应用只启动 `JiyiCodex`，未启动 `JiyiCodexClient.app`；原版 `/Applications/Codex.app` 仍为 `com.openai.codex` 且 PID 727 保持运行，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。在线飞书发布说明已追加同一结论并回读确认，revision 为 56。
- 2026-06-10 05:40 新增本地可运行极义托管代理服务：`codex_plus_core::managed_proxy` 提供 `GET /jiyi/v1/health`、`GET /v1/models`、`POST /v1/responses`，只接受极义本地后端 session token；上游 APIMart / 中转站 Key 只从 `JIYI_MANAGED_PROXY_UPSTREAM_API_KEY`、`JIYI_CODEX_UPSTREAM_API_KEY` 或 `APIMART_API_KEY` 环境变量读取，不进入客户端配置；Responses 返回后会把 token 用量写入本地后端额度摘要。新增 `apps/jiyi-managed-proxy` 二进制，DMG 主应用和管理工具均内置 `Contents/MacOS/jiyi-managed-proxy` sidecar，发布前检查新增 `managed_proxy_sidecar`。新增 `scripts/installer/macos/install-local-dmg.sh`，用于本机验收时挂载 DMG、备份旧极义 App 到 `.noindex`、复制新 App、清理 xattr、就地 ad-hoc 重签、验证不影响 `/Applications/Codex.app` 并刷新 LaunchServices。`cargo fmt --all --check`、`git diff --check`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-core --test managed_proxy -- --nocapture`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-manager release_readiness -- --nocapture` 和 `CARGO_BUILD_JOBS=1 cargo build -p codex-plus-launcher -p codex-plus-manager -p jiyi-managed-proxy --release` 均通过。新包 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg` 为 451M，SHA-256 为 `9ae3af89922baec5e3b6dfd07dc8d8552794095e80aa1f26e0c480b3fcc5dc3d`；挂载校验和安装后均确认 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`、`CFBundleSignature=JIYI`、无旧 `Contents/Resources/Codex.app`、内置客户端无 `CFBundleURLTypes`，两个极义 App 内均有 5.9M 的 `jiyi-managed-proxy`。本机安装脚本备份到 `~/.codex-session-delete/app-backups.noindex/20260610-053847-local-install`；`open -n /Applications/极义codex.app` 可启动常驻 `JiyiCodex`，诊断日志显示 `manager.window_visible`；原版 `/Applications/Codex.app` 仍为 `com.openai.codex` 且 PID 727 保持运行，原版 `~/.codex` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。
- 2026-06-10 06:18 新增“修复原版隔离”闭环：管理工具注册 `repair_official_codex_isolation` 命令和安装维护页按钮，会扫描原版 `~/.codex` 与原版 Codex App Support 关键状态文件；检测到 `.codex-session-delete`、APIMart、`jiyi-local-proxy`、`jiyi-keychain:`、`gpt-5.5` 等极义痕迹时，先备份到 `~/.codex-session-delete/original-codex-isolation-backups/`，再清理污染字段或删除 Chromium 网络缓存状态文件，不修改 `/Applications/Codex.app` 本体。发布前检查的 `official_codex_isolation` 已扩展到原版 App Support，避免只看 `~/.codex` 导致原版窗口残留 APIMart 状态无法被发现。本机复核显示原版 `/Applications/Codex.app` 仍为 `com.openai.codex`，极义应用为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，原版 `~/.codex/config.toml` 与 `auth.json` 当前未检测到极义痕迹；原版 Codex 的 Chromium 网络状态中曾出现 APIMart 请求记录，属于可由新修复按钮备份后清理的 App Support 残留。验证命令：`cargo fmt --all --check`、`git diff --check`、`npm run check`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-manager official_codex -- --nocapture`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-manager release_readiness -- --nocapture`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-manager identity_sync_response -- --nocapture`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-core --test managed_proxy -- --nocapture` 均通过。
- 2026-06-10 06:26 重新生成并安装 DMG，包含“修复原版隔离”按钮和扩展后的原版 App Support 隔离检查；新包 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg` 为 452M，SHA-256 为 `4c3864b0ee4c785dc5e314b26727ef3291fb63aa257dc0e4bc1e2a2f395637c1`。安装脚本只替换 `/Applications/极义codex.app` 与 `/Applications/极义codex 管理工具.app`，旧极义 App 备份到 `~/.codex-session-delete/app-backups.noindex/20260610-062638-local-install`；安装后 `/Applications/Codex.app` 仍为 `com.openai.codex` 且 PID 727 保持运行，极义主应用、管理工具、内置客户端分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，包内无旧 `Contents/Resources/Codex.app`，主应用和管理工具均内置 `jiyi-managed-proxy`。安装后启动 `/Applications/极义codex.app` 只出现 `JiyiCodex` 进程，未启动 `JiyiCodexClient.app`，即仍停留在极义手机号门禁/账号入口，不会打开原版 Codex。
- 2026-06-10 06:54 新增管理页本地托管代理启停：后端命令 `managed_proxy_status`、`start_managed_proxy`、`stop_managed_proxy` 已接入设置页，状态区展示运行状态、PID、健康检查、监听地址、上游地址、上游 Key 和同步 Key 配置状态；启动时写入本机 Endpoint `http://127.0.0.1:57421`，上游仍读取当前激活的 APIMart / 中转站供应商，避免本机 Endpoint 自我循环；停止前校验 PID 命令行包含 `jiyi-managed-proxy`，避免影响原版 Codex。临时端口 `127.0.0.1:57431` 健康检查通过，脚本退出后确认 PID 和端口均已清理。重新生成并安装 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg`，包体 452M，SHA-256 为 `fc40e3fe892cef5600b27830ed55c25d902855fc7709fd65afcb8429bfc97f4a`；安装备份写入 `~/.codex-session-delete/app-backups.noindex/20260610-065214-local-install`。安装后 `/Applications/Codex.app` 仍为 `com.openai.codex`，极义主应用、管理工具、内置客户端分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，包内无旧 `Contents/Resources/Codex.app`，内置客户端无原版 URL Scheme 和 Sparkle 更新键，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。在线飞书发布说明已追加同一结论并回读确认，revision 为 60。
- 2026-06-10 07:16 新增托管代理后端库路径配置：`jiyi-managed-proxy` 默认仍使用 `~/.codex-session-delete/jiyi-codex-local-backend.sqlite`，也可通过 `JIYI_MANAGED_PROXY_DB_PATH` 指定服务端 SQLite 路径，兼容 `JIYI_BACKEND_DB_PATH` 兜底；`/jiyi/v1/health` 返回实际 `backendDbPath`，启动日志打印 `backend_db=`，管理工具设置页展示“托管代理后端库”。管理工具启动本地托管代理时显式传入默认后端库路径，避免外部环境变量把本地代理带到不可预期位置；CLI / 服务端手动启动仍可用环境变量覆盖。验证通过：`npm run check`、`cargo fmt --all --check`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-core backend_db_path_from_env_values -- --nocapture`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-core --test managed_proxy -- --nocapture`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-manager managed_proxy -- --nocapture`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-manager release_readiness -- --nocapture`、`git diff --check`、`CARGO_BUILD_JOBS=1 cargo build -p codex-plus-launcher -p codex-plus-manager -p jiyi-managed-proxy --release` 均通过；release 二进制和安装后的 sidecar 都用临时 `JIYI_MANAGED_PROXY_DB_PATH` 健康检查通过并确认端口清理。重新生成并安装 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg`，包体 452M，SHA-256 为 `760de2aa6818913283b7b5842050e08e26d3ab83bdda29b83ad42bb8ae3a15ac`；安装备份写入 `~/.codex-session-delete/app-backups.noindex/20260610-071641-local-install`。安装后 `/Applications/Codex.app` 仍为 `com.openai.codex`，极义主应用、管理工具、内置客户端分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，包内无旧 `Contents/Resources/Codex.app`，内置客户端无原版 URL Scheme，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。
- 2026-06-10 07:34 新增正式签名与公证打包链路：`scripts/installer/macos/package-dmg.sh` 默认仍生成本机 ad-hoc 验收包；设置 `JIYI_CODESIGN_IDENTITY` 后会使用 Developer ID Application 身份、`--options runtime` 和 `--timestamp` 签名 App / sidecar / 内置客户端，并签名 DMG；设置 `JIYI_NOTARIZE=1` 且提供 `APPLE_ID` / `APPLE_APP_SPECIFIC_PASSWORD` / `APPLE_TEAM_ID` 或 `ASC_KEY_ID` / `ASC_ISSUER_ID` / `ASC_KEY_PATH` 后，会执行 `xcrun notarytool submit --wait`、`xcrun stapler staple`、`xcrun stapler validate` 和 `spctl -a -vv -t install`。发布前检查新增 `notarization_packager`，用于确认打包脚本具备 Developer ID / notarytool / stapler 能力；`notarization_env` 仍根据本机 Apple 凭据是否配置给出 warning。验证通过：`bash -n scripts/installer/macos/package-dmg.sh`、`bash -n scripts/installer/macos/install-local-dmg.sh`、`cargo fmt --all --check`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-manager macos_packager_hides_silent_launcher_but_not_manager -- --nocapture`、`CARGO_BUILD_JOBS=1 cargo test -p codex-plus-manager release_readiness -- --nocapture`、`git diff --check`、`CARGO_BUILD_JOBS=1 cargo build -p codex-plus-launcher -p codex-plus-manager -p jiyi-managed-proxy --release` 均通过。默认 ad-hoc 模式重新生成并安装 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg`，包体 452M，SHA-256 为 `5c0a30710e417927ed999c237b5bc69bbfde551431495e92bb6ba088e8a14fbe`；安装备份写入 `~/.codex-session-delete/app-backups.noindex/20260610-073448-local-install`。安装后 `/Applications/Codex.app` 仍为 `com.openai.codex`，极义主应用、管理工具、内置客户端分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。
- 2026-06-10 07:42 重置本机极义登录态用于重新验收手机号门禁：已把 `~/.codex-session-delete/jiyi-codex-local.sqlite` 和 `~/.codex-session-delete/jiyi-codex-auth.sqlite` 移入 `~/.codex-session-delete/auth-reset-backups.noindex/20260610-074241`。重置后 `jiyi-codex-local.sqlite` 不存在，下次打开 `/Applications/极义codex.app` 会先进入手机号验证码登录；同时验证 `/Applications/Codex.app`、`~/.codex/config.toml` 和 `~/.codex/auth.json` 仍存在且未被修改，`/Applications` 下只剩 `Codex.app`、`极义codex.app` 和 `极义codex 管理工具.app` 三个相关入口。
- 2026-06-10 07:57 新增托管代理 macOS 本地常驻服务部署包：新增 `scripts/server/macos/install-managed-proxy-launchd.sh`、`uninstall-managed-proxy-launchd.sh` 和 `jiyi-managed-proxy.env.example`，可把 `jiyi-managed-proxy` 安装为 `com.jiyi.codex.managed-proxy` LaunchAgent；首次安装会创建 `~/.codex-session-delete/jiyi-managed-proxy.env`，真实上游 Key 和同步 Key 只写入该私有 env 文件。新增 `docs/极义codex_本地服务部署.md`，并让 `package-dmg.sh` 把部署脚本打入主应用和管理工具的 `Contents/Resources/server/macos/`；`install-local-dmg.sh` 与发布前检查新增 `managed_proxy_launchd_deploy` 校验，防止缺少本地服务部署脚本的包被验收通过。验证通过：`bash -n` 四个脚本、`cargo fmt --all --check`、`cargo test -p codex-plus-manager --test windows_subsystem macos_packager_hides_silent_launcher_but_not_manager -- --nocapture`、`cargo test -p codex-plus-manager --test windows_subsystem managed_proxy_launchd_scripts_keep_service_isolated -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`、`cargo test -p codex-plus-core --test managed_proxy -- --nocapture`、`cargo build -p codex-plus-launcher -p codex-plus-manager -p jiyi-managed-proxy --release` 和 `git diff --check` 均通过。重新生成并安装 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg`，包体 452M，SHA-256 为 `2b9174437bbb212e74dfe08b7d6667b4f1e718708bdb2e99073a4f5466d311ed`；安装备份写入 `~/.codex-session-delete/app-backups.noindex/20260610-075728-local-install`。安装后主应用包内的 `install-managed-proxy-launchd.sh`、`uninstall-managed-proxy-launchd.sh` 和 `jiyi-managed-proxy.env.example` 均存在，包内脚本语法通过；`/Applications/Codex.app` 仍为 `com.openai.codex`，极义主应用、管理工具、内置客户端仍分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。
- 2026-06-10 08:13 新增远端托管代理部署模板：新增 `scripts/server/linux/install-managed-proxy-systemd.sh`、`uninstall-managed-proxy-systemd.sh`、`jiyi-managed-proxy.service`、`jiyi-managed-proxy.env.example`，以及 `apps/jiyi-managed-proxy/Dockerfile` 和 `docs/极义codex_远端托管代理部署.md`。Linux systemd 模板默认使用 `/etc/jiyi-codex/jiyi-managed-proxy.env`、`/var/lib/jiyi-codex/jiyi-codex-backend.sqlite`、`/usr/local/bin/jiyi-managed-proxy`；Docker 模板使用 `rust:1-bookworm` 构建并以 `jiyi-codex` 非 root 用户运行，监听 `0.0.0.0:8080`。`package-dmg.sh` 会把 `scripts/server` 全量打入 `Contents/Resources/server/` 并把 Dockerfile 放入 `Contents/Resources/server/docker/Dockerfile`；`install-local-dmg.sh` 和发布前检查新增 `managed_proxy_remote_deploy`，确保安装包含 Linux/systemd/Docker 远端部署模板。验证通过：Linux/macOS/server/installer 脚本 `bash -n`、`cargo fmt --all --check`、`cargo test -p codex-plus-manager --test windows_subsystem macos_packager_hides_silent_launcher_but_not_manager -- --nocapture`、`cargo test -p codex-plus-manager --test windows_subsystem managed_proxy_remote_deploy_templates_keep_server_keys_out_of_client -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`、`cargo build -p codex-plus-launcher -p codex-plus-manager -p jiyi-managed-proxy --release` 和 `git diff --check` 均通过。重新生成并安装 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg`，包体 452M，SHA-256 为 `303b7fbab6f2f87d5affa26f745089579992b323b9ddbde5aee6aef716687306`；安装备份写入 `~/.codex-session-delete/app-backups.noindex/20260610-081328-local-install`。安装后主应用包内的 Linux systemd 安装脚本、service、env 示例和 Dockerfile 均存在，脚本语法通过；`/Applications/Codex.app` 仍为 `com.openai.codex`，极义主应用、管理工具、内置客户端仍分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。
- 2026-06-10 08:35 新增服务端用户访问控制和封禁闭环：`LocalBackendStore` 新增 `backend_user_access` 表、`block_user` / `unblock_user` 能力和 `blocked_user_count` 状态；封禁用户会立即吊销该用户未过期 session，并阻止后续账号同步重新签发 session。`jiyi-managed-proxy` 新增 `POST /jiyi/v1/admin/users/block` 和 `POST /jiyi/v1/admin/users/unblock`，通过 `JIYI_MANAGED_PROXY_ADMIN_API_KEY` 鉴权；`/jiyi/v1/health` 返回 `adminKeyConfigured`，管理工具设置页展示“管理 Key”状态。本地和远端 env 示例、macOS LaunchAgent 脚本、Linux systemd 脚本、Dockerfile 和发布前检查均纳入管理 Key，防止没有封禁能力的服务包通过验收。验证通过：`cargo fmt --all --check`、`npm run check`、`cargo test -p codex-plus-core local_backend_blocks_user_and_prevents_new_session_issue -- --nocapture`、`cargo test -p codex-plus-core --test managed_proxy managed_proxy_admin_block_revokes_session_before_upstream -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`、`cargo test -p codex-plus-manager --test windows_subsystem managed_proxy_remote_deploy_templates_keep_server_keys_out_of_client -- --nocapture`。
- 2026-06-10 08:49 重新生成并安装 DMG，包含服务端用户访问控制、托管代理管理 Key 和封禁/解封接口；新包 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg` 为 452M，SHA-256 为 `bc6e7fb153ca5c6c5250c0e5f452c6710731f6ce9d7e7e23a28936e1c85faf7f`。安装脚本只替换 `/Applications/极义codex.app` 与 `/Applications/极义codex 管理工具.app`，安装备份写入 `~/.codex-session-delete/app-backups.noindex/20260610-084708-local-install`；安装后 `/Applications/Codex.app` 仍为 `com.openai.codex`，极义主应用、管理工具、内置客户端分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，包内无旧 `Contents/Resources/Codex.app`。包内 macOS/Linux env 示例和 Dockerfile 均包含 `JIYI_MANAGED_PROXY_ADMIN_API_KEY`；已安装 sidecar 用临时端口 `127.0.0.1:57440` 健康检查通过，返回 `upstreamKeyConfigured=true`、`identitySyncKeyConfigured=true`、`adminKeyConfigured=true`；原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹，三份极义 app codesign 深度校验通过。
- 2026-06-10 09:08 新增服务端审计留存：本地账号后端新增 `backend_audit_events` 表、`recent_audit_events` 查询和状态计数，身份同步、用量写入、session 吊销、封禁/解封都会写入审计事件；审计 metadata 不保存明文 session token、同步 Key、管理 Key 或上游 Key。`jiyi-managed-proxy` 新增 `GET /jiyi/v1/admin/audit/events?limit=50`，通过 `JIYI_MANAGED_PROXY_ADMIN_API_KEY` 查询最近事件；管理工具工作台展示“审计事件”和“最近审计”。验证通过：`cargo test -p codex-plus-core local_backend_records_audit_events_without_plain_tokens -- --nocapture`、`cargo test -p codex-plus-core --test managed_proxy managed_proxy_admin_block_revokes_session_before_upstream -- --nocapture`、`cargo test -p codex-plus-core --test managed_proxy -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`、`cargo fmt --all --check` 和 `git diff --check`。
- 2026-06-10 09:25 修复安装后托管代理 sidecar 直接执行失败：系统日志显示 `/Applications/极义codex.app/Contents/MacOS/jiyi-managed-proxy` 作为 App bundle 内 ad-hoc sidecar 直接执行时会被 `amfid` 拒绝；同一二进制复制到外部运行目录后可正常启动。管理工具 `start_managed_proxy` 和 macOS LaunchAgent 安装脚本已改为从包内复制 sidecar 到 `~/.codex-session-delete/bin/jiyi-managed-proxy` 后执行，仍只使用极义状态目录，不触碰 `/Applications/Codex.app`。手动验证运行副本监听 `127.0.0.1:57448`，`/jiyi/v1/health` 返回 `upstreamKeyConfigured=true`、`identitySyncKeyConfigured=true`、`adminKeyConfigured=true`，`/jiyi/v1/admin/audit/events?limit=5` 返回 200。
- 2026-06-10 09:33 重新生成并安装 DMG，包含服务端审计留存和托管代理运行副本修复；新包 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg` 为 452M，SHA-256 为 `bc5a32ec06af5033a0c05890d75ba7015d01f7b33bef77b05f177cacc2c497bf`。安装备份写入 `~/.codex-session-delete/app-backups.noindex/20260610-093237-local-install`；安装后 `/Applications/Codex.app` 仍为 `com.openai.codex`，极义主应用、管理工具、内置客户端分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，包内无旧 `Contents/Resources/Codex.app`，三份极义 app codesign 深度校验通过，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。使用临时 LaunchAgent label 验证已安装脚本：`bash /Applications/极义codex.app/Contents/Resources/server/macos/install-managed-proxy-launchd.sh` 会把 sidecar 复制到临时状态目录 `bin/jiyi-managed-proxy` 后启动；`/jiyi/v1/health` 和 `/jiyi/v1/admin/audit/events?limit=5` 均返回 200；随后已卸载临时服务，确认默认 `com.jiyi.codex.managed-proxy`、测试 label 和 57451 端口均未残留。
- 2026-06-10 09:48 新增服务端用户运营查询：本地账号后端新增 `admin_user_overviews`，按用户汇总脱敏手机号、访问状态、套餐、今日请求数、今日 token 用量、剩余额度、设备数、session 数、最近同步和最近使用时间；`jiyi-managed-proxy` 新增 `GET /jiyi/v1/admin/users?limit=50`，通过 `JIYI_MANAGED_PROXY_ADMIN_API_KEY` 查询，不返回明文手机号、session token、同步 Key、管理 Key 或上游 Key。验证通过：`cargo test -p codex-plus-core local_backend_admin_user_overviews_include_quota_and_access_status -- --nocapture`、`cargo test -p codex-plus-core --test managed_proxy managed_proxy_admin_block_revokes_session_before_upstream -- --nocapture`。
- 2026-06-10 10:16 重新生成并安装 DMG，包含服务端用户运营查询和托管代理运行副本修复后的最终本机包；`dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg` 为 452M，SHA-256 为 `370e3edcfc5a949f5c9a1d0410707e656701b71aca2a0e7216436d7df42ef4bd`。安装备份写入 `~/.codex-session-delete/app-backups.noindex/20260610-100406-local-install`；安装后 `/Applications/Codex.app` 仍为 `com.openai.codex`，极义主应用、管理工具、内置客户端分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，内置客户端位于 `Contents/Resources/JiyiCodexClient.app`，包内无旧 `Contents/Resources/Codex.app`，三份极义 app codesign 深度校验通过，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。本地 `auth_state` 为空时实际打开 `/Applications/极义codex.app` 只启动 `JiyiCodex`，未启动 `JiyiCodexClient.app`，说明未登录仍停留在手机号门禁；验证后已退出测试窗口。使用临时 `com.jiyi.codex.managed-proxy.check` LaunchAgent、临时状态目录和 `127.0.0.1:57452` 验证已安装脚本：服务执行临时状态目录 `bin/jiyi-managed-proxy`，`/jiyi/v1/health`、`/jiyi/v1/admin/users?limit=5` 和 `/jiyi/v1/admin/audit/events?limit=5` 均返回 200；随后已卸载临时服务，确认默认服务、测试服务、57421/57452 端口和 `jiyi-managed-proxy` 进程均无残留。父工作区旧 `app-backups.noindex` 2.0G 备份已移动到 `~/.codex-session-delete/app-backups.noindex/workspace-legacy-20260610-101608`，并新增父级 `.gitignore` 忽略 `app-backups.noindex/`，避免 Git/Codex 工作区继续扫描旧 app 备份。
- 2026-06-10 10:31 新增服务端套餐/额度调整接口：本地账号后端新增 `set_user_entitlement_with_actor`，可通过管理 actor 更新 `backend_entitlements` 的套餐 ID、套餐名和每日 token 上限，并写入 `user_entitlement_updated` 审计事件；`jiyi-managed-proxy` 新增 `POST /jiyi/v1/admin/users/entitlement`，通过 `JIYI_MANAGED_PROXY_ADMIN_API_KEY` 鉴权。更新后 `quota_snapshot`、`GET /jiyi/v1/admin/users` 和托管代理 quota 闸门会立即使用新额度，不返回明文 session token、同步 Key、管理 Key 或上游 Key。验证通过：`cargo test -p codex-plus-core local_backend_admin_updates_entitlement_and_records_audit -- --nocapture`、`cargo test -p codex-plus-core --test managed_proxy managed_proxy_admin_updates_entitlement_before_quota_check -- --nocapture`、`cargo test -p codex-plus-core --test managed_proxy -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`。
- 2026-06-10 10:44 重新生成并安装 DMG，包含服务端套餐/额度调整接口；`dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg` 为 452M，SHA-256 为 `bd3b3fc401e7c420ad2067103bbd8da35bd388ad1d27b1a96b32d5b8c06db212`。安装备份写入 `~/.codex-session-delete/app-backups.noindex/20260610-103920-local-install`；安装后 `/Applications/Codex.app` 仍为 `com.openai.codex`，极义主应用、管理工具、内置客户端分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，包内无旧 `Contents/Resources/Codex.app`，三份极义 app codesign 深度校验通过，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。使用安装后的 LaunchAgent 脚本和临时 `com.jiyi.codex.managed-proxy.entitlement-check` 服务验证新增接口：同步测试用户后，`POST /jiyi/v1/admin/users/entitlement` 把套餐更新为 `jiyi_pro`、每日额度更新为 5000，`GET /jiyi/v1/admin/users?limit=5` 返回 `todayRemainingTokens=4930`，`GET /jiyi/v1/admin/audit/events?limit=10` 包含 `user_entitlement_updated`；随后已卸载临时服务，确认测试服务和 57453 端口无残留。
- 2026-06-10 11:00 新增审计只读 Key 和审计过滤查询：`LocalBackendStore::audit_events` 支持按 `event_type`、`actor_type`、`subject_user_id` 和 `limit` 查询；`jiyi-managed-proxy` 新增 `JIYI_MANAGED_PROXY_AUDIT_API_KEY` / `JIYI_BACKEND_AUDIT_API_KEY`，审计只读 Key 只能访问 `GET /jiyi/v1/admin/audit/events`，不能访问用户列表、套餐调整、封禁或解封接口。审计接口支持 `eventType`、`actorType`、`subjectUserId` 和 `limit` 查询参数，`/jiyi/v1/health` 返回 `auditKeyConfigured`；管理工具设置页展示“审计 Key”。macOS/Linux env 示例、安装脚本、Dockerfile 和发布前检查均纳入审计 Key，metadata 不保存明文 session token、同步 Key、管理 Key、审计 Key 或上游 Key。验证通过：`cargo test -p codex-plus-core local_backend_ -- --nocapture`、`cargo test -p codex-plus-core --test managed_proxy -- --nocapture`、`cargo test -p codex-plus-manager managed_proxy_ -- --nocapture`。
- 2026-06-10 11:08 重新生成并安装 DMG，包含审计只读 Key 和审计过滤查询；`dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg` 为 452M，SHA-256 为 `0e4e334805ea807f1d882a0ca0f96378acecc67210b1fd87576971856fa2f1b7`。安装备份写入 `~/.codex-session-delete/app-backups.noindex/20260610-110822-local-install`；安装后 `/Applications/Codex.app` 仍为 `com.openai.codex`，极义主应用、管理工具、内置客户端分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，内置客户端位于 `Contents/Resources/JiyiCodexClient.app`，包内无旧 `Contents/Resources/Codex.app`，三份极义 app codesign 深度校验通过，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。使用安装后的 LaunchAgent 脚本和临时 `com.jiyi.codex.managed-proxy.audit-check` 服务验证：`/jiyi/v1/health` 返回 `adminKeyConfigured=true` 和 `auditKeyConfigured=true`；身份同步 200，管理员套餐调整 200，审计只读 Key 可按 `eventType=user_entitlement_updated`、`actorType=managed_proxy_admin_api`、`subjectUserId=user-1` 查询审计事件 200；同一审计 Key 访问用户列表和套餐调整均返回 401；响应未泄露测试同步 Key、管理 Key、审计 Key 或上游 Key。随后已卸载临时服务，确认测试 LaunchAgent、57454 端口、`jiyi-managed-proxy` 进程和临时状态目录均无残留。
- 2026-06-10 11:26 新增托管代理单节点管理员 RBAC Key：保留 `JIYI_MANAGED_PROXY_ADMIN_API_KEY` 作为全量管理 Key，同时新增 `JIYI_MANAGED_PROXY_USER_READ_API_KEY`、`JIYI_MANAGED_PROXY_BILLING_API_KEY`、`JIYI_MANAGED_PROXY_ACCESS_API_KEY`，分别只允许用户运营查询、套餐/额度调整、封禁/解封；审计只读 Key 仍只允许查询审计事件。`/jiyi/v1/health` 新增 `userReadKeyConfigured`、`billingKeyConfigured`、`accessKeyConfigured`；管理工具设置页展示“用户只读 Key / 计费 Key / 风控 Key”；本地/远端 env 示例、macOS LaunchAgent 脚本、Linux systemd 脚本、Dockerfile、发布前检查和模板测试均纳入角色 Key。审计事件的 `actorId` 会记录 `billing_api_key` 或 `access_api_key`，但响应不泄露任何明文 Key。验证通过：`cargo test -p codex-plus-core --test managed_proxy -- --nocapture`、`npm run check`。
- 2026-06-10 11:44 重新生成并安装 DMG，包含托管代理单节点角色 Key 和安装脚本备份去应用化修复；`dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg` 为 453M，SHA-256 为 `5aacde4129804a699ddeabba1fbbdc0cebb6d6f2ebff5017ef1f698c18fe8699`。安装脚本只替换 `/Applications/极义codex.app` 与 `/Applications/极义codex 管理工具.app`，安装备份写入 `~/.codex-session-delete/app-backups.noindex/20260610-114348-local-install`；安装后 `/Applications/Codex.app` 仍为 `com.openai.codex`，极义主应用、管理工具、内置客户端分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`，包内无旧 `Contents/Resources/Codex.app`，主应用 988M、管理工具 35M、内置客户端 952M，三份极义 app codesign 深度校验通过，原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 未检测到极义、APIMart、Keychain、本地代理或 `gpt-5.5` 痕迹。安装脚本已递归把 `~/.codex-session-delete/app-backups.noindex` 内所有 `.app` 备份去应用化，`find "$HOME/.codex-session-delete/app-backups.noindex" -type d -name '*.app'` 无输出，Spotlight 精确查询 `极义codex.app` 和 `极义codex 管理工具.app` 只返回 `/Applications` 下两个真实入口。实际打开 `/Applications/极义codex.app` 复核：`auth_state_count=0`，只启动 `/Applications/极义codex.app/Contents/MacOS/JiyiCodex`，未启动 `JiyiCodexClient.app`，原版 `/Applications/Codex.app` 仍是独立 PID 727。使用安装后的 LaunchAgent 脚本和临时 `com.jiyi.codex.managed-proxy.role-check` 服务验证角色 Key：`/jiyi/v1/health` 返回 `userReadKeyConfigured=true`、`billingKeyConfigured=true`、`accessKeyConfigured=true`、`auditKeyConfigured=true` 且 `adminKeyConfigured=false`；身份同步 200，用户只读 Key 查用户 200 且调套餐 401，计费 Key 调套餐 200 且查用户 401，风控 Key 封禁 200 且调套餐 401，审计 Key 查审计 200 且查用户 401；审计包含 `billing_api_key` 和 `access_api_key` actorId，响应不泄露测试 Key。随后已卸载临时服务，确认测试 LaunchAgent、57455 端口、`jiyi-managed-proxy` 进程和临时状态目录均无残留。
- 2026-06-12 新增支付回调和自动对账单节点闭环：本地后端新增 `backend_billing_payment_events` 和幂等回调处理，托管代理新增 `POST /jiyi/v1/billing/payment-webhook` 专用支付回调 Key，以及 `POST /jiyi/v1/admin/billing/reconcile` 重新对账接口；`paid` / `succeeded` / `trade_success` 等状态会自动生成续费记录并更新用户或团队额度，重复回调不会重复落账，响应与审计不暴露原始支付 payload。验证通过：`cargo test -p codex-plus-core local_backend_records_payment_webhook_idempotently_and_reconciles -- --nocapture`、`cargo test -p codex-plus-core --test managed_proxy -- --nocapture`、`npm run check`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`。
- 2026-06-12 新增支付回调通用 HMAC-SHA256 验签：托管代理读取 `JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET` / `JIYI_BACKEND_PAYMENT_WEBHOOK_SIGNATURE_SECRET`；配置后支付回调必须携带 `X-Jiyi-Payment-Timestamp` 和 `X-Jiyi-Payment-Signature`，签名内容为 `timestamp + "." + raw_body`，默认 5 分钟有效；无签名、错误签名或过期签名都会在写入支付事件前返回 401。`/jiyi/v1/health` 新增 `paymentWebhookSignatureConfigured`，管理工具设置页展示“支付验签”，macOS/Linux env 示例、LaunchAgent/systemd 安装脚本、Dockerfile 和发布前检查均纳入签名密钥。验证通过：`cargo test -p codex-plus-core --test managed_proxy managed_proxy_payment_webhook_requires_hmac_signature_when_configured -- --nocapture`、`npm run check`。
- 2026-06-12 新增支付宝/微信支付官方 RSA-SHA256 验签：托管代理读取 `JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY` / `_PATH`、`JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY` / `_PATH` 及兼容别名；配置后支付宝回调按 RSA2 参数串验签，微信支付回调按 v3 Header 的 `timestamp\nnonce\nbody\n` 验签，缺签、错签或不支持的签名类型都会在写入支付事件前返回 401。`/jiyi/v1/health` 新增 `paymentWebhookAlipaySignatureConfigured` 和 `paymentWebhookWechatpaySignatureConfigured`，管理工具设置页展示“支付宝验签 / 微信验签”，macOS/Linux env 示例、LaunchAgent/systemd 安装脚本、Dockerfile 和发布前检查均纳入官方公钥。验证通过：`cargo test -p codex-plus-core --test managed_proxy managed_proxy_payment_webhook_verifies -- --nocapture`。
- 2026-06-12 补齐百炼 Key 来源状态：真实请求仍优先使用百炼 / 千问链路，Key 可来自极义 Keychain、百炼环境变量或下载目录默认百炼 Key 文件，APIMart 只作为备选；`RelayPayload` 新增 `apiKeyConfigured` 和 `apiKeySource`，管理工具首页能显示“下载目录百炼 Key”等来源枚举，不回传 Key 明文或下载路径。下载目录 fallback 单元测试已串行化，避免并发修改 `HOME` / `JIYI_CODEX_API_KEY_FILE` 导致随机失败。验证通过：`cargo test -p codex-plus-core protocol_proxy::tests::resolved_relay_api_key_falls_back -- --nocapture`、`cargo test -p codex-plus-manager relay_payload_does_not_expose_token_text -- --nocapture`、`npm run check`。
- 2026-06-12 23:20 补齐千问模型目录和增强菜单品牌收口：`model_catalog` 在无显式 provider / `model_catalog_json` 时可用 `QWEN_API_KEY` 等百炼环境变量或下载目录百炼 Key 诊断阿里百炼模型列表；已有显式 provider 或模型目录 JSON 时不会被下载目录真实 Key 污染。默认 HTTP User-Agent 已改为 `JiyiCodex/<version>`，直连 Chat Completions 路径和供应商测试也统一使用极义标识。内置增强菜单的可见标题、关于说明、插件市场显示名和 worktree 兜底提示已从 `Codex++` / `OpenAI插件` 收口为“极义codex / 极义内置插件”。验证通过：`cargo test -p codex-plus-core --test model_catalog -- --nocapture`、`cargo test -p codex-plus-core model_catalog::tests -- --nocapture`、`cargo test -p codex-plus-core --test protocol_proxy -- --nocapture`、`cargo test -p codex-plus-core --test cdp_bridge -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`、`npm run check`、`cargo fmt --all --check`、`git diff --check`。最终重新生成 `dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg`，包体 `453M`，SHA-256 为 `8d1be0aa3c68740413d1e5f6fe0be3195263ec09eab420fc36e98316f39642e2`；`hdiutil verify`、挂载内检、三份 app 深度签名校验和敏感扫描均通过。

### 2026-06-09 22:34 Keychain 隔离与重装验证

已完成一轮“极义和原版 Codex 彻底分离”的重装验证：

- 原版 `/Applications/Codex.app` 保持运行，PID 727 未被结束。
- 只结束并替换极义自己的 `/Applications/极义codex.app` 与 `/Applications/极义codex 管理工具.app`。
- 旧极义应用备份到：

```text
~/.codex-session-delete/app-backups/20260609-223457-keychain-isolation-retry
```

- 新 DMG 已安装，bundle id 和包体大小：

```text
/Applications/Codex.app                              com.openai.codex         953M
/Applications/极义codex.app                         com.jiyi.codex           981M
/Applications/极义codex 管理工具.app                 com.jiyi.codex.manager   28M
/Applications/极义codex.app/Contents/Resources/JiyiCodexClient.app  com.jiyi.codex.client    952M
```

- 主应用、管理工具和内置客户端 `codesign --verify --deep --strict` 均通过。
- 当前 `auth_state_count=0`，打开 `极义codex` 后只启动 `/Applications/极义codex.app/Contents/MacOS/JiyiCodex`，未启动 `JiyiCodexClient.app`，因此会先停留在手机号验证码登录页。
- 新版主入口不会在已有本地登录态时自动进入 Codex，必须用户点击“进入 Codex”才会启动内置客户端。
- 极义 settings 会把现有上游 Key 迁移到 macOS 钥匙串，settings 中无 `sk-` 明文，只保留 `jiyi-keychain:` 引用。
- 钥匙串条目 `com.jiyi.codex.apimart / relay:global` 已存在。
- 原版 `~/.codex/config.toml` 与 `~/.codex/auth.json` 无极义、百炼/APIMart、`sk-`、`jiyi-keychain:` 或本地代理痕迹。

### 未登录门禁

已验证：

- `auth_state_count=0`
- `auth_state` schema 已包含 `session_expires_at_ms` 和 `device_id`
- `local_device` 已生成本机设备标识
- 只启动 `/Applications/极义codex.app/Contents/MacOS/JiyiCodex`
- 未启动 `JiyiCodexClient.app`
- 诊断日志确认主入口模式：

```text
manager.start app_mode=main
manager.setup_start url=index.html?mode=main
manager.window_visible app_mode=main
```

- 主应用显示手机号验证码登录界面。
- 2026-06-09 21:47 重新安装新版后再次清空 `auth_state` 验证：登录态为 0，只启动 `/Applications/极义codex.app/Contents/MacOS/JiyiCodex`，未启动 `JiyiCodexClient.app`。

### 已登录进入 Codex

已用临时本地登录态验证：

- 本地账号登录态存在后，主应用先停留在极义账号门禁页。
- 用户点击“进入 Codex”后，主应用会启动：

```text
~/Library/Application Support/极义codex.noindex/embedded-client/JiyiCodexClient.app
```

- CDP 读取页面文本显示已进入 Codex 使用界面。
- 启动命令包含隔离环境：

```text
CODEX_HOME=/Users/lv/.codex-session-delete/codex-home
HOME=/Users/lv/.codex-session-delete/home
```

- 进程标识显示内置客户端为 `JiyiCodexClient_Mac`。
- 极义内置 Codex 配置写在 `~/.codex-session-delete/codex-home/config.toml`，默认 `base_url = "http://127.0.0.1:57321/v1"`，真实百炼或中转站请求由极义 helper 读取钥匙串、环境变量或下载目录 key 文件后转发。
- 验证后已再次清空本地登录态，当前重新打开仍先进入手机号登录页。

### 原版 Codex 隔离验证

已验证原版目录当前不含极义路径、百炼/APIMart、极义 Keychain、本地代理或极义模型默认值：

```text
clean=/Users/lv/.codex/config.toml
clean=/Users/lv/.codex/auth.json
```

检查范围：

```text
~/.codex/config.toml
~/.codex/auth.json
```

### 当前配置状态

极义本地设置文件位于：

```text
~/.codex-session-delete/settings.json
```

当前设置已验证：

```text
launchMode = relay
relayBaseUrl = https://dashscope.aliyuncs.com/compatible-mode/v1
relayApiKey = jiyi-keychain:relay:global
activeRelayId = default
relayMode = pureApi
```

说明：真实 API Key 不写入本文档，也不再明文写入极义 settings。

## 仍未完成

这些不是当前短期方案已经完成的部分，需要后续输入或后端建设：

1. 真实生产手机号体系

- 腾讯云短信发送代码、管理工具配置入口、Keychain 密钥保存和腾讯云响应校验已完成。
- 仍需要配置可正式发送的腾讯云短信 `SecretId`、`SecretKey`、短信签名、模板 ID 和短信应用 ID，并在管理工具中关闭本地干跑模式。
- 当前本地版已完成短信验证码、会话过期、设备标识、本地账号服务端承接库和短信生产配置入口。
- 仍需要把本地手机号 session 升级为真实远端服务端 session。

2. 极义服务端用户体系

- 已完成本地单节点用户表、设备绑定表、默认团队表、团队成员表、用户套餐额度表、团队套餐额度表、续费记录表、支付事件表、套餐编辑入口、脱敏迁移报告、session 门禁、本地账号服务端库、本地后端 session token 签发、session 吊销、登出清理、用户运营查询、团队运营查询、服务端用户/团队套餐额度调整、手工续费/支付凭证落账、支付网关回调承接、paid 事件自动对账、用户封禁/解封访问控制、服务端审计留存和远端同步请求/POST 通道。
- 仍需要把本地单节点账号服务端升级为远端多租户服务端账号体系。
- 用户运营查询、团队运营查询、服务端用户/团队套餐额度调整、手工续费/支付凭证落账、支付网关回调承接、通用 HMAC-SHA256 回调验签、支付宝/微信支付官方 RSA 验签、paid 事件自动对账、封禁/解封、审计留存、审计只读 Key、基础过滤查询和单节点角色 Key 已完成；仍需要真实成员权限模型、支付平台证书自动下载/轮换、商户后台对账单拉取、登录风控、远端管理员账号 RBAC 和多租户审计治理能力。

3. 上游 Key 与额度控制

- 当前短期方案是本机 macOS 钥匙串、环境变量或下载目录 key 文件保存一个阿里百炼 / 中转站 API Key，极义本地请求代理读取后转发；settings 只保存钥匙串引用。
- 已完成本地用量记账、按用户记账、本地套餐额度编辑、用量汇总导出、单机每日 token 上限闸门、本地服务端 `GET /jiyi/v1/quota/today` 额度快照 API，以及 `POST /jiyi/v1/usage/record` 服务端实时用量写入。
- 已完成托管代理客户端路径、本地可运行托管代理服务、macOS LaunchAgent 常驻部署包、Linux systemd 部署模板和 Dockerfile：公开版可配置极义托管代理 Endpoint，用极义后端 session token 鉴权模型请求；本地部署可启动 `jiyi-managed-proxy`，也可通过 `install-managed-proxy-launchd.sh` 安装为 `com.jiyi.codex.managed-proxy` 常驻服务；远端单机可用 `install-managed-proxy-systemd.sh` 安装为 Linux systemd 服务，容器环境可用 `apps/jiyi-managed-proxy/Dockerfile` 构建镜像；由服务端环境变量或私有 env 文件持有百炼 / 中转站上游 Key、账号同步 Key、全量管理 Key、用户只读 Key、计费 Key、支付回调 Key、支付回调验签密钥、支付宝/微信支付官方公钥、风控 Key 和审计只读 Key，客户端不需要保存上游主 Key；托管代理后端库支持 `JIYI_MANAGED_PROXY_DB_PATH` 显式配置，健康检查返回实际 `backendDbPath`；托管代理已提供用户运营查询、团队运营查询、用户/团队套餐额度调整、手工续费/支付凭证落账、支付网关回调承接、通用 HMAC-SHA256 回调验签、支付宝/微信支付官方 RSA 验签、paid 事件自动对账、封禁/解封、审计过滤查询和单节点角色 Key，封禁后模型请求会在转发上游前被拒绝。
- 面向公开分发时，不能把主 Key 直接内置到客户端。
- 仍需要把托管代理实际部署到真实远端公网服务或自建中转站，并记录跨设备用量、支付平台证书自动下载/轮换、商户后台对账单拉取、远端管理员账号 RBAC、多租户审计治理和复杂风控。

4. 自建中转站

- 当前默认使用阿里百炼千问兼容接口，APIMart 保留为备用 Base URL。
- 后续如果要控制成本、审计和额度，需要部署极义自己的中转站。

5. 发布与签名

- 当前本机可用，已做本地 ad-hoc codesign；本机验收建议使用 `scripts/installer/macos/install-local-dmg.sh` 安装并就地重签，避免手工拖拽后被 `amfid` 直接杀掉。
- 在线飞书发布文档已创建。
- 正式签名 / 公证脚本链路已完成：`package-dmg.sh` 支持 Developer ID Application、Hardened Runtime、DMG 签名、notarytool 公证和 stapler 固化。
- 仍需要配置 Apple Developer 证书、App 专用密码或 App Store Connect API Key，并制定 DMG 版本策略。

6. 更深层品牌替换

- 运行中的内置客户端标题仍可能显示 `Codex`，这是上游客户端内部标题。
- 短期方案已通过外层入口、登录门禁和模型配置完成产品路径接管。
- 长期如果要完全品牌化，需要改内置 Codex 客户端资源和内部文案。

## 验收命令

未登录门禁：

```bash
sqlite3 "$HOME/.codex-session-delete/jiyi-codex-local.sqlite" 'DELETE FROM auth_state;'
open -n /Applications/极义codex.app
sleep 7
sqlite3 "$HOME/.codex-session-delete/jiyi-codex-local.sqlite" 'select count(*) from auth_state;'
ps -axo pid,ppid,etime,command | rg '/Applications/极义codex.app|JiyiCodexClient.app|codex-plus-plus --app-path'
tail -n 40 "$HOME/.codex-session-delete/codex-plus.log" | rg 'manager.start|manager.setup_start|manager.window_visible|app_mode'
```

原版 Codex 隔离：

```bash
rg -n '极义|Jiyi|codex-session-delete|dashscope\.aliyuncs\.com|api\.apimart\.ai|jiyi-keychain|jiyi-local-proxy|gpt-5\.5' \
  "$HOME/.codex/config.toml" "$HOME/.codex/auth.json"
```

如果没有输出，说明原版 Codex 配置未被极义污染。

发布前检查：

```bash
cargo test -p codex-plus-manager release_readiness -- --nocapture
```

也可以在 `极义codex 管理工具` 的“安装维护”页点击“运行发布前检查”。

本地用量记账：

```bash
cargo test -p codex-plus-core local_usage -- --nocapture
sqlite3 "$HOME/.codex-session-delete/jiyi-codex-local.sqlite" \
  'select usage_day, count(*), sum(coalesce(reported_total_tokens, estimated_total_tokens)) from local_usage_events group by usage_day;'
```

本地账号和套餐模型：

```bash
cargo test -p codex-plus-core local_account -- --nocapture
```

本地账号迁移报告：

```bash
cargo test -p codex-plus-core export_state_redacts_phone -- --nocapture
cargo test -p codex-plus-core export_summary_groups_usage -- --nocapture
```

本地账号服务端库：

```bash
cargo test -p codex-plus-core local_backend -- --nocapture
cargo test -p codex-plus-core default_helper_serves_local_backend_account_api -- --nocapture
cargo test -p codex-plus-manager release_readiness -- --nocapture
```

本地托管代理服务：

```bash
cargo test -p codex-plus-core --test managed_proxy -- --nocapture
cargo build -p jiyi-managed-proxy --release
```

配置检查：

```bash
node - <<'NODE'
const fs = require("fs");
const path = require("path");
const settings = JSON.parse(fs.readFileSync(path.join(process.env.HOME, ".codex-session-delete/settings.json"), "utf8"));
const active = settings.relayProfiles.find((profile) => profile.id === settings.activeRelayId);
console.log(settings.launchMode);
console.log(settings.relayBaseUrl);
console.log(settings.relayApiKey && settings.relayApiKey.startsWith("jiyi-keychain:"));
console.log(active && active.relayMode);
console.log(JSON.stringify(settings).includes("sk-"));
NODE
```

## 当前结论

短期目标已经达成：极义codex 当前不依赖 ChatGPT 用户体系完成主流程，用户打开主应用先走极义手机号登录，登录后仍停留在极义账号门禁页，点击“进入 Codex”后才进入内置完整 Codex 客户端，模型配置由阿里百炼 / 极义中转纯 API、APIMart 备选和极义本地请求代理接管。

长期目标仍需要远端服务端用户体系、跨设备额度控制、自建中转站和正式发布链路。

## 2026-06-12 进度补充

### 已完成（与本轮目标对齐）

- 已将默认 API 基础链路改为阿里百炼千问兼容接口：主基址 `https://dashscope.aliyuncs.com/compatible-mode/v1`，备选 `https://apimart.ai/`；默认模型为 `qwen3.7-plus`，默认协议为 Chat Completions，由本地 helper 转成 Codex 所需 Responses。
- 已补齐百炼 / 上游 Key 的多源读取与容灾链路：settings / `JIYI_CODEX_API_KEY` / `DASHSCOPE_API_KEY` / `BAILIAN_API_KEY` / `ALIYUN_BAILIAN_API_KEY` / `QWEN_API_KEY` / 下载目录默认百炼 Key 文件（支持 `.csv` 或单行密钥文本）/ `APIMART_API_KEY` / `CUSTOM_OPENAI_API_KEY`，并支持上游候选逐级重试；管理工具 Relay 状态新增 `apiKeyConfigured` 和 `apiKeySource`，首页可识别“下载目录百炼 Key”且不回传明文；模型目录诊断在无显式来源时会默认查百炼，已有 provider 或 `model_catalog_json` 时不会被下载目录 Key 污染。
- 登录链路已落地“先手机号验证码登录、后手动进入内置客户端”：未登录状态不能直接启动 Codex，`launch_embedded_codex` 会拦截返回并提示。
- 已禁止官方 ChatGPT 登录回退：官方/混合 mode 在主入口和注入面板被阻断，`clear_relay_injection`/`apply_relay_injection` 均返回国产提示。
- 已固定极义主应用和管理工具不依赖 `/Applications/Codex.app`，主入口会强制启动内置 `JiyiCodexClient.app`。
- 已做一轮用户可见品牌收口：增强菜单标题、关于说明、插件市场显示名和 worktree 兜底提示已显示“极义codex / 极义内置插件”，默认 HTTP User-Agent 也改为 `JiyiCodex`。
- 已继续完善账号服务端闭环：本地单节点后端、默认团队、团队成员、用户运营查询、团队运营查询、用户/团队额度编辑、手工续费/支付凭证落账、支付回调承接、通用 HMAC-SHA256 回调验签、支付宝/微信支付官方 RSA 验签、paid 事件自动对账、封禁解封、审计留存、托管代理 RBAC 与管理脚本已接入。
- 管理工具已新增独立“总后台”导航页：可集中查看用户、团队、续费和审计，支持用户/团队套餐额度编辑、封禁解封、手工续费落账和支付事件重对账；这是本地单节点管理后台闭环。
- 已把主页能力清单“插件 / Skill / 用户脚本”集中在工作台可见入口，满足“首页有预置能力入口”的要求。
- 已重新打包 DMG：`dist/macos/JiyiCodex-1.2.4-macos-arm64.dmg`，大小约 `453M`，SHA-256：
  `cc8d8c1b762d330c3d4dcce09b70baf17220d231c3834d4e978514c439e8e385`
- 已挂载验证新 DMG：主应用约 `988M`、管理工具约 `35M`、内置客户端约 `952M`；三者 bundle id 分别为 `com.jiyi.codex` / `com.jiyi.codex.manager` / `com.jiyi.codex.client`；包内无旧 `Contents/Resources/Codex.app`，内置客户端无 `CFBundleURLTypes` / `SUPublicEDKey` / `SUFeedURL`，`CFBundleSignature=JIYI`；三份 app `codesign --verify --deep --strict` 均通过；镜像 `hdiutil verify` 通过；二进制可见 `总后台` 和 `admin_console_*` 命令标记；macOS/Linux/Docker 托管代理模板默认上游均为 `https://dashscope.aliyuncs.com/compatible-mode/v1`，并包含支付回调 Key、支付回调验签密钥和支付宝/微信支付官方公钥配置位；镜像内未发现默认业务空间 apiKey 文件。
- 2026-06-12 23:45 已新增总后台产品化界面并重新生成 DMG：验证通过 `npm run check`、`cargo fmt --all --check`、`cargo test -p codex-plus-manager startup_options -- --nocapture`、`cargo test -p codex-plus-manager release_readiness -- --nocapture`、`cargo test -p codex-plus-core local_backend_admin -- --nocapture`、release build、`hdiutil verify`、挂载内检、codesign 深度校验和敏感扫描。
- 在线飞书发布说明已同步并回读确认，revision 为 `117`；线上文档已包含最新 SHA-256、总后台、千问模型目录默认链路、极义内置插件品牌收口和支付宝/微信支付官方 RSA 验签内容。

### 本轮遗留（不影响当前国产独立版闭环运行）

- 远端多端账号体系、正式短信资源、支付平台证书自动下载/轮换、商户后台对账单拉取、持续发布签名公证流程、远端管理员 RBAC、复杂风控和深层品牌内置资源替换，仍需下一阶段单独推进。
