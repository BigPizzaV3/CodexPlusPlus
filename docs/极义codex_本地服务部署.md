# 极义codex 本地服务部署说明

本文档用于把 `jiyi-managed-proxy` 作为 macOS 本地 LaunchAgent 常驻服务部署。它承接两个本地服务端职责：

- 极义账号同步入口：`POST /jiyi/v1/identity/sync`
- 极义用户运营查询：`GET /jiyi/v1/admin/users`
- 极义用户套餐/额度调整：`POST /jiyi/v1/admin/users/entitlement`
- 极义用户访问控制：`POST /jiyi/v1/admin/users/block`、`POST /jiyi/v1/admin/users/unblock`
- 极义服务端审计查询：`GET /jiyi/v1/admin/audit/events`
- 模型请求托管代理：`GET /v1/models`、`POST /v1/responses`

## 前置条件

1. 已安装完整客户端版 `极义codex.app`。
2. 已完成手机号登录，并在管理工具中同步过本地账号服务端库。
3. 已准备阿里百炼、APIMart 或自建中转站上游 Key。
4. 已准备极义账号同步 API Key。
5. 已准备极义全量管理 API Key，或按角色分别准备用户只读、计费、风控和审计只读 API Key。

## 安装服务

从源码目录运行：

```bash
bash scripts/server/macos/install-managed-proxy-launchd.sh
```

从已安装 App 包运行：

```bash
bash /Applications/极义codex.app/Contents/Resources/server/macos/install-managed-proxy-launchd.sh
```

安装脚本会从 App 包内读取 `Contents/MacOS/jiyi-managed-proxy`，复制到下面的运行副本后由 LaunchAgent 执行：

```text
~/.codex-session-delete/bin/jiyi-managed-proxy
```

这样可以避免 macOS 对 `/Applications/极义codex.app` 大型 App bundle 内 ad-hoc sidecar 的直接执行限制，同时仍然只使用极义状态目录，不影响 `/Applications/Codex.app`。

首次安装会创建私有环境变量文件：

```text
~/.codex-session-delete/jiyi-managed-proxy.env
```

把下面值填入该文件。`JIYI_MANAGED_PROXY_ADMIN_API_KEY` 是全量管理 Key，可调用所有管理接口；公开分发建议同时配置角色 Key，减少运营人员共用超级管理 Key：

```bash
JIYI_MANAGED_PROXY_UPSTREAM_API_KEY="你的百炼或中转站上游 Key"
JIYI_MANAGED_PROXY_SYNC_API_KEY="你的极义账号同步 Key"
JIYI_MANAGED_PROXY_ADMIN_API_KEY="你的极义管理 Key"
JIYI_MANAGED_PROXY_USER_READ_API_KEY="你的极义用户只读 Key"
JIYI_MANAGED_PROXY_BILLING_API_KEY="你的极义计费 Key"
JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY="你的极义支付回调 Key"
JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET="你的极义支付回调 HMAC 验签密钥"
JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH="/path/to/alipay-public.pem"
JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH="/path/to/wechatpay-public.pem"
JIYI_MANAGED_PROXY_ACCESS_API_KEY="你的极义风控 Key"
JIYI_MANAGED_PROXY_AUDIT_API_KEY="你的极义审计只读 Key"
```

然后重新执行安装脚本，或执行：

```bash
launchctl kickstart -k "gui/$(id -u)/com.jiyi.codex.managed-proxy"
```

## 验证服务

```bash
curl -s http://127.0.0.1:57421/jiyi/v1/health
```

健康检查会返回 `backendDbPath`、`upstreamKeyConfigured`、`identitySyncKeyConfigured`、`adminKeyConfigured`、`userReadKeyConfigured`、`billingKeyConfigured`、`paymentWebhookKeyConfigured`、`paymentWebhookSignatureConfigured`、`paymentWebhookAlipaySignatureConfigured`、`paymentWebhookWechatpaySignatureConfigured`、`accessKeyConfigured` 和 `auditKeyConfigured`。公开分发前，上游 Key、同步 Key、支付回调 Key、支付回调验签密钥、支付宝/微信支付官方公钥和需要使用的管理角色 Key 配置状态都应该为 `true`。

查询用户运营概览：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_USER_READ_API_KEY" \
  "http://127.0.0.1:57421/jiyi/v1/admin/users?limit=50"
```

该接口也接受全量管理 Key。返回脱敏手机号、访问状态、套餐、今日请求数、今日 token 用量、剩余额度、设备数、session 数和最近同步时间，不返回明文手机号、session token、同步 Key、管理 Key 或上游 Key。

调整用户套餐和每日额度：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_BILLING_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"userId":"user-1","planId":"jiyi_pro","planName":"极义 Pro","dailyTokenLimit":5000,"reason":"renewal paid"}' \
  http://127.0.0.1:57421/jiyi/v1/admin/users/entitlement
```

该接口也接受全量管理 Key。它会更新 `backend_entitlements`，后续 `/jiyi/v1/quota/today`、托管代理 quota 检查和用户运营查询都会使用新套餐额度；审计事件类型为 `user_entitlement_updated`，metadata 不保存明文 Key 或 session token。

模拟支付网关回调：

```bash
body='{"provider":"mockpay","gatewayEventId":"evt-001","externalOrderId":"pay-order-001","status":"trade_success","subjectType":"user","subjectId":"user-1","planId":"jiyi_pro","planName":"极义 Pro","dailyTokenLimit":5000,"amountCents":19900,"currency":"CNY","reason":"gateway callback"}'
ts="$(date +%s)"
sig="$(printf '%s.%s' "$ts" "$body" | openssl dgst -sha256 -hmac "$JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET" -binary | xxd -p -c 256)"
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY" \
  -H "X-Jiyi-Payment-Timestamp: $ts" \
  -H "X-Jiyi-Payment-Signature: sha256=$sig" \
  -H "Content-Type: application/json" \
  -d "$body" \
  http://127.0.0.1:57421/jiyi/v1/billing/payment-webhook
```

paid / succeeded / trade_success 会自动生成续费记录并更新套餐额度；重复回调会按网关事件 ID 或订单号幂等返回已有处理结果，不重复加额度。未配置 `JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET` 时，回调只校验支付回调 API Key；真实收款场景应同时配置 API Key、HMAC 验签密钥，以及支付宝/微信支付官方公钥。配置官方公钥后，`provider` 或 `paymentChannel` 匹配 `alipay` / `wechatpay` 的回调会在落账前强制执行 RSA-SHA256 验签。

重新触发支付事件对账：

```bash
curl -s \
  -X POST \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_BILLING_API_KEY" \
  "http://127.0.0.1:57421/jiyi/v1/admin/billing/reconcile?limit=50"
```

封禁用户：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_ACCESS_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"userId":"user-1","reason":"abuse review"}' \
  http://127.0.0.1:57421/jiyi/v1/admin/users/block
```

该接口也接受全量管理 Key。封禁后，该用户已有未过期 session 会被吊销，后续 `GET /v1/models` 和 `POST /v1/responses` 会在转发上游前返回 `user_blocked`。

查询最近审计事件：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_AUDIT_API_KEY" \
  "http://127.0.0.1:57421/jiyi/v1/admin/audit/events?limit=50&eventType=user_entitlement_updated&subjectUserId=user-1"
```

审计事件包含身份同步、用量写入、session 吊销、套餐/额度调整、封禁和解封等服务端行为，支持按 `eventType`、`actorType`、`subjectUserId` 和 `limit` 过滤；审计只读 Key 只能读取该接口，不能调用用户列表、套餐调整、封禁或解封接口。metadata 已脱敏，不保存明文 token、同步 Key、管理 Key、审计 Key 或上游 Key。

## 卸载服务

```bash
bash scripts/server/macos/uninstall-managed-proxy-launchd.sh
```

如果要同时删除本地 env 文件：

```bash
bash scripts/server/macos/uninstall-managed-proxy-launchd.sh --purge-env
```

## 隔离边界

- 服务只默认监听 `127.0.0.1:57421`，不会开放局域网访问。
- 上游 Key 只在本机私有 env 文件或服务端环境变量中出现，不写入极义 settings，不写入内置 Codex Home。
- 全量管理 Key 只用于本地服务端管理接口，不和上游 Key 混用。
- 用户只读 Key 只用于 `GET /jiyi/v1/admin/users`。
- 计费 Key 只用于 `POST /jiyi/v1/admin/users/entitlement`。
- 风控 Key 只用于 `POST /jiyi/v1/admin/users/block` 和 `POST /jiyi/v1/admin/users/unblock`。
- 审计只读 Key 只用于 `GET /jiyi/v1/admin/audit/events`，不具备用户写权限。
- 审计事件只写入极义后端库 `backend_audit_events`，不读取也不修改原版 Codex 的本地状态。
- LaunchAgent 执行的是 `~/.codex-session-delete/bin/jiyi-managed-proxy` 运行副本，不直接执行 `/Applications/极义codex.app/Contents/MacOS/jiyi-managed-proxy`。
- 服务使用 `~/.codex-session-delete/jiyi-codex-local-backend.sqlite` 作为本地账号后端库，不读取原版 `~/.codex`。
- 安装和卸载脚本只管理 `com.jiyi.codex.managed-proxy` LaunchAgent，不修改 `/Applications/Codex.app`。
