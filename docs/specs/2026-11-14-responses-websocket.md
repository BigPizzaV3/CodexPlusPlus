# Responses 原生 WebSocket response.create 代理

目标：让标准 Responses WS 客户端经过 Codex++ 普通代理或聚合网关后，仍能连接原生 Responses WS 上游。关联 [#2963](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2963)。

```text
response.create
  → 现有模型选择、子代理模型继承、请求/路由/上游中间件
  → 现有供应商认证、HTTP 代理与 CA 配置
  → GET Upgrade → 原生 Responses WebSocket
  → 现有 Responses 事件整理、响应中间件、逐响应用量记录
```

## 范围

- 普通 `/responses`、`/v1/responses` 等已有普通 Responses 别名；网关现有 Responses Agent 路由。
- 普通代理与网关在升级和鉴权前拒绝带非空 `Origin` 的 WS 请求（403），避免网页使用本地上游凭据并读取输出；不带 Origin 或只带空值的客户端保持兼容。
- 每个逻辑响应复用现有准备链。握手只是准备成功，终态才记录请求成功。
- 同一连接多轮、工具结果回传、`previous_response_id`、`generate:false`；按 `stream_id` 分流、同流 FIFO。
- 子代理使用同一入口；保留父子线程及逐回合元数据。多个客户端连接并行，状态独立。
- 已知响应 ID 固定原上游连接与 Key；切换模型导致路由不再包含原上游时明确报错，客户端可提交完整上下文开始新响应。
- 官方登录按签发方、用户与工作区的完整身份隔离。同账号 token 刷新可续接存活连接；换用户、工作区或签发方时拒绝旧 ID，身份不完整时保留 token 隔离。
- 请求提交生成后不自动重发；断连、失败和取消释放名额、结束日志。握手/准备中取消同时退还 RPM。
- 断线或连接级错误时，先原样转发阶段填补暂存的消息完成事件并结束响应中间件，再清理响应；不猜测缺失的 phase。
- 路由动态、Key 成功与速度统计在终态记账；`incomplete` 保留失败日志，但不惩罚健康供应商。
- 并发或 RPM 限制导致的临时换 Key、换供应商沿用 HTTP 的粘性记账条件，不覆盖会话原有首选路由。

## 能力边界

- 本次提供原生 Responses WS 的 `response.create` 传输，不实现 WS → HTTP/SSE 桥接。
- `response.steer` 等回合中控制事件不在本次范围，明确返回请求错误；不声明完整 steering / 自动 successor 生命周期支持。
- Chat Completions、Anthropic 兼容路由和 HTTP 专用 exec / 合成压缩转换继续使用 HTTP。
- `/responses/compact` 保持 HTTP POST。普通生成请求中的原生上游工具事件正常透传。
- 不自动修改客户端 `supports_websockets`。自定义 provider 在应用配置后显式设置此项为 true 才选择 WS；既有 false 选择保留。
- 混合协议供应商池不要整体声明 WS 可用。配置 HTTP-only 路由时继续选择 HTTP。
- 本地代理可同时接受 HTTP/SSE 与 WS，不增加独立的子代理 WS API。

## 会话上限与失败恢复

每个下游连接最多 8 个上游身份、16 个活动/准备中的响应、32 个命名流、64 个排队请求（排队文本共 32 MiB）；单帧/消息 16 MiB。已知响应 ID 不淘汰亲和；达到 4096 项后拒绝新建响应，要求客户端重连并提供完整上下文。上游重连后不会把旧 ID 的增量输入发到新连接。

上游不接受升级时返回 WS 错误，客户端应选择 HTTP。这里只在未提交生成的握手阶段刷新 OAuth 或换候选；不会在上游关闭、生成失败或工具可能已执行后重放请求。

## 验证与审查

- 保留修改前 426 行为的基线证据；无效升级请求返回 400，有效请求完成 101。
- 使用本地原生 WS 上游验证真实事件、模型改写、认证、双轮与工具续接、warmup。
- 验证同流排队、跨流并发、主/子客户端并行、断连后不重发。
- 验证网关鉴权、原 Key 续接与名额释放；握手失败不能提前改变成功粘性。
- 验证临时 Key/供应商换路后保留原会话粘性、断线和连接级错误放行暂存消息、同用户 OAuth 刷新续接及共享工作区用户隔离。
- 验证 HTTP-only / 无效握手失败边界；HTTP/SSE、压缩和既有配置回归。
- Rust 测试、构建启动、独立代码审查、修正并复核后提交 PR。

协议依据：[OpenAI WebSocket Mode](https://developers.openai.com/api/docs/guides/websocket-mode)。本地测试不证明任意第三方上游完整实现了该协议。
