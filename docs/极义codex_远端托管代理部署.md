# 极义codex 远端托管代理部署说明

本文档用于把 `jiyi-managed-proxy` 部署到 Linux 服务器或容器环境，让公开分发的客户端不再接触百炼、APIMart 或中转站主 Key。

## 服务职责

- `POST /jiyi/v1/identity/sync`：接收管理工具同步的脱敏账号、设备、套餐和用量摘要，并签发极义后端 session token。
- `GET /jiyi/v1/admin/users`：使用极义全量管理 API Key 或用户只读 API Key 查询脱敏用户运营概览，包含套餐、今日用量、设备数、session 数和访问状态。
- `GET /jiyi/v1/admin/teams`：使用极义全量管理 API Key 或用户只读 API Key 查询团队运营概览，包含成员数、今日用量、团队套餐和团队剩余额度。
- `POST /jiyi/v1/admin/users/entitlement`：使用极义全量管理 API Key 或计费 API Key 调整用户套餐和每日 token 额度，后续 quota 和模型请求闸门立即生效。
- `POST /jiyi/v1/admin/teams/entitlement`：使用极义全量管理 API Key 或计费 API Key 调整团队套餐和每日 token 额度，并写入服务端审计事件。
- `GET /jiyi/v1/admin/billing/renewals`：使用极义全量管理 API Key 或计费 API Key 查询续费落账记录。
- `POST /jiyi/v1/admin/billing/renewals`：使用极义全量管理 API Key 或计费 API Key 记录手工续费、企业转账或后续支付回调凭证，并同步更新用户或团队套餐额度。
- `POST /jiyi/v1/billing/payment-webhook`：使用极义支付回调 API Key 接收支付网关事件；配置 `JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET` 后会强制校验 `X-Jiyi-Payment-Timestamp` 和 `X-Jiyi-Payment-Signature`，签名内容为 `timestamp + "." + raw_body` 的 HMAC-SHA256；配置 `JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY` / `_PATH` 或 `JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY` / `_PATH` 后，支付宝和微信支付回调会在落账前强制执行官方 RSA-SHA256 验签；paid / succeeded / trade_success 会自动对账为续费记录并更新用户或团队套餐额度，重复回调按网关事件 ID 或订单号幂等处理。
- `POST /jiyi/v1/admin/billing/reconcile`：使用极义全量管理 API Key 或计费 API Key 重新处理未完成或失败的 paid 支付事件。
- `POST /jiyi/v1/admin/users/block`：使用极义全量管理 API Key 或风控 API Key 封禁用户，立即吊销该用户未过期 session。
- `POST /jiyi/v1/admin/users/unblock`：使用极义全量管理 API Key 或风控 API Key 解封用户，允许后续账号同步重新签发 session。
- `GET /jiyi/v1/admin/audit/events`：使用极义管理 API Key 或审计只读 API Key 查询审计事件，覆盖身份同步、用量写入、session 吊销、套餐/额度调整、封禁和解封，并支持按事件类型、操作者类型和用户过滤。
- `GET /v1/models`：使用极义后端 session token 鉴权后转发模型列表请求。
- `POST /v1/responses`：使用极义后端 session token 鉴权、检查额度、转发 Responses 请求，并写回服务端用量摘要。
- `GET /jiyi/v1/health`：返回监听地址、上游地址、后端库路径和 Key 配置状态。

## systemd 部署

在服务器上构建或上传 release 二进制后执行：

```bash
cargo build --release -p jiyi-managed-proxy
sudo scripts/server/linux/install-managed-proxy-systemd.sh target/release/jiyi-managed-proxy
```

首次安装会创建：

```text
/etc/jiyi-codex/jiyi-managed-proxy.env
/var/lib/jiyi-codex/jiyi-codex-backend.sqlite
/etc/systemd/system/jiyi-managed-proxy.service
```

编辑 env 文件：

```bash
sudo nano /etc/jiyi-codex/jiyi-managed-proxy.env
```

至少填入：

```bash
JIYI_MANAGED_PROXY_UPSTREAM_API_KEY="你的百炼或中转站上游 Key"
JIYI_MANAGED_PROXY_SYNC_API_KEY="你的极义账号同步 Key"
JIYI_MANAGED_PROXY_ADMIN_API_KEY="你的极义全量管理 Key"
JIYI_MANAGED_PROXY_USER_READ_API_KEY="你的极义用户只读 Key"
JIYI_MANAGED_PROXY_BILLING_API_KEY="你的极义计费 Key"
JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY="你的极义支付回调 Key"
JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET="你的极义支付回调 HMAC 验签密钥"
JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH="/path/to/alipay-public.pem"
JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH="/path/to/wechatpay-public.pem"
JIYI_MANAGED_PROXY_ACCESS_API_KEY="你的极义风控 Key"
JIYI_MANAGED_PROXY_AUDIT_API_KEY="你的极义审计只读 Key"
```

重启服务：

```bash
sudo systemctl restart jiyi-managed-proxy
sudo systemctl status jiyi-managed-proxy --no-pager
```

## Docker 部署

从仓库根目录构建镜像：

```bash
docker build -f apps/jiyi-managed-proxy/Dockerfile -t jiyi-managed-proxy:1.2.4 .
```

运行容器：

```bash
docker run -d --name jiyi-managed-proxy \
  -p 8080:8080 \
  -v jiyi-codex-data:/var/lib/jiyi-codex \
  -e JIYI_MANAGED_PROXY_UPSTREAM_API_KEY="你的百炼或中转站上游 Key" \
  -e JIYI_MANAGED_PROXY_SYNC_API_KEY="你的极义账号同步 Key" \
  -e JIYI_MANAGED_PROXY_ADMIN_API_KEY="你的极义全量管理 Key" \
  -e JIYI_MANAGED_PROXY_USER_READ_API_KEY="你的极义用户只读 Key" \
  -e JIYI_MANAGED_PROXY_BILLING_API_KEY="你的极义计费 Key" \
  -e JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY="你的极义支付回调 Key" \
  -e JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET="你的极义支付回调 HMAC 验签密钥" \
  -e JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH="/run/secrets/alipay-public.pem" \
  -e JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH="/run/secrets/wechatpay-public.pem" \
  -e JIYI_MANAGED_PROXY_ACCESS_API_KEY="你的极义风控 Key" \
  -e JIYI_MANAGED_PROXY_AUDIT_API_KEY="你的极义审计只读 Key" \
  jiyi-managed-proxy:1.2.4
```

健康检查：

```bash
curl -s http://127.0.0.1:8080/jiyi/v1/health
```

## 反向代理建议

生产环境建议用 Nginx、Caddy、云负载均衡或 API 网关终止 TLS，再转发到 `127.0.0.1:8080` 或容器端口。客户端设置页中的托管代理 Endpoint 应使用 HTTPS：

```text
https://codex-proxy.example.com/v1
```

账号同步 Endpoint 应使用：

```text
https://codex-proxy.example.com/jiyi/v1/identity/sync
```

用户运营查询 Endpoint 示例：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_USER_READ_API_KEY" \
  "https://codex-proxy.example.com/jiyi/v1/admin/users?limit=50"
```

团队运营查询 Endpoint 示例：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_USER_READ_API_KEY" \
  "https://codex-proxy.example.com/jiyi/v1/admin/teams?limit=50"
```

用户套餐/额度调整 Endpoint 示例：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_BILLING_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"userId":"user-1","planId":"jiyi_pro","planName":"极义 Pro","dailyTokenLimit":5000,"reason":"renewal paid"}' \
  https://codex-proxy.example.com/jiyi/v1/admin/users/entitlement
```

团队套餐/额度调整 Endpoint 示例：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_BILLING_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"teamId":"jiyi-default-team","planId":"team_pro","planName":"团队 Pro","dailyTokenLimit":50000,"reason":"team renewal paid"}' \
  https://codex-proxy.example.com/jiyi/v1/admin/teams/entitlement
```

续费落账 Endpoint 示例：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_BILLING_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"subjectType":"user","subjectId":"user-1","planId":"jiyi_pro","planName":"极义 Pro","dailyTokenLimit":5000,"amountCents":19900,"currency":"CNY","paymentChannel":"manual","externalOrderId":"order-001","reason":"manual renewal"}' \
  https://codex-proxy.example.com/jiyi/v1/admin/billing/renewals
```

续费记录查询 Endpoint 示例：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_BILLING_API_KEY" \
  "https://codex-proxy.example.com/jiyi/v1/admin/billing/renewals?limit=50"
```

支付网关回调 Endpoint 示例：

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
  https://codex-proxy.example.com/jiyi/v1/billing/payment-webhook
```

如果未配置 `JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET`，回调仍只校验支付回调 API Key；公开分发或真实收款场景应同时配置 API Key、HMAC 验签密钥和支付宝/微信支付官方公钥。官方公钥支持直接填 PEM/base64，也支持用 `_PATH` 指向 PEM 文件；生产环境建议使用 `_PATH` 或密钥管理器挂载文件。

支付自动对账 Endpoint 示例：

```bash
curl -s \
  -X POST \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_BILLING_API_KEY" \
  "https://codex-proxy.example.com/jiyi/v1/admin/billing/reconcile?limit=50"
```

封禁用户 Endpoint 示例：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_ACCESS_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"userId":"user-1","reason":"abuse review"}' \
  https://codex-proxy.example.com/jiyi/v1/admin/users/block
```

审计查询 Endpoint 示例：

```bash
curl -s \
  -H "Authorization: Bearer $JIYI_MANAGED_PROXY_AUDIT_API_KEY" \
  "https://codex-proxy.example.com/jiyi/v1/admin/audit/events?limit=50&eventType=user_entitlement_updated&subjectUserId=user-1"
```

## 当前边界

- 当前远端部署仍使用 SQLite，适合单实例试运行和早期生产验证。
- 已具备单实例用户运营查询、团队运营查询、用户/团队套餐额度调整、手工续费/支付凭证落账、支付网关回调承接、通用 HMAC-SHA256 回调验签、支付宝/微信支付官方 RSA-SHA256 验签、paid 事件自动对账、用户封禁/解封、审计事件留存、用户只读 Key、计费 Key、支付回调 Key、支付回调验签密钥、支付宝/微信支付公钥、风控 Key、审计只读 Key 和基础过滤能力；多实例、跨区、真实成员权限、支付平台证书自动下载与轮换、商户后台对账单拉取、管理员账号 RBAC、多租户审计治理和复杂风控需要后续迁移到正式数据库和账号服务。
- 上游 Key 只能放在服务器 env、密钥管理器或容器运行时变量中，不能写入客户端、镜像层、Git、文档或截图。
- `JIYI_MANAGED_PROXY_SYNC_API_KEY` 应定期轮换，并通过 HTTPS 传输。
- `JIYI_MANAGED_PROXY_ADMIN_API_KEY` 是全量管理 Key，应独立于同步 Key 和上游 Key，并只保留给超级管理员或自动化维护任务。
- `JIYI_MANAGED_PROXY_USER_READ_API_KEY` 只发给用户和团队运营查询角色。
- `JIYI_MANAGED_PROXY_BILLING_API_KEY` 只发给用户或团队计费、续费处理角色，可调整额度、记录续费凭证和触发自动对账。
- `JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY` 只配置给支付网关回调地址，不应发给运营后台账号。
- `JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET` 只配置给支付网关或自建收银台签名模块，不应发给运营后台账号；轮换时应先支持新旧双发，再切换服务端密钥。
- `JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH` 和 `JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH` 建议指向只读 PEM 文件；证书/公钥轮换时应先部署新公钥并确认网关新签名生效，再下线旧公钥。
- `JIYI_MANAGED_PROXY_ACCESS_API_KEY` 只发给风控或客服封禁处理角色。
- `JIYI_MANAGED_PROXY_AUDIT_API_KEY` 应独立于管理 Key，只发给只读审计或运营查询角色。
