# Anthropic Messages 上游（Codex → Claude 系网关）

日期：2026-10-15
状态：已实现

## 背景

Codex 客户端只会说 Responses API，而很多 Claude 系中转网关、企业内部网关只暴露
Anthropic Messages（`/v1/messages`）。把这类地址直接填进 Codex 的 provider，请求
`/responses` 必然 404。本特性让 Codex 照旧连本机协议代理，由代理把请求转成
Anthropic Messages 发给上游，再把响应转回 Responses。

## 形态：为什么以 Chat Completions 作中间态

请求方向 `Responses → Chat → Anthropic Messages`，响应方向
`Anthropic Messages → Chat → Responses`（非流式 JSON 与 SSE 同构）。

理由是复用面最大化：Codex 历史里的脏数据（工具配对修复、`custom` 工具、
`tool_search` 的 MCP 命名空间展开、图片与 tool 输出图片搬迁、系统提示收敛、
compaction）以及响应侧那台数千行的 `ChatSseToResponsesConverter` 状态机都已经在
Chat 链路上跑通并有测试覆盖。新代码只落在 `anthropic_bridge.rs` 这一层，
出问题时可以对拍「是 Chat ↔ Anthropic 这段错，还是既有链路错」。

代价（已知）：
- Anthropic 的 `thinking.signature`、`cache_control` 断点、`[1m]` beta 头在 Chat
  形态里没有落点，因此当前不回放思考内容、不做提示缓存标记注入；
- 多一次纯内存 JSON 变换（微秒级，可忽略）。

要支持「思考内容跨轮原样回放」或「缓存断点注入」时，需要给中间态加旁路状态
（把签名随 Responses 的 `encrypted_content` 带回客户端），不做推倒重来。

## 落点

| 位置 | 作用 |
| --- | --- |
| `crates/codex-plus-core/src/anthropic_bridge.rs` | Chat ↔ Anthropic 的请求/响应/SSE 转换 |
| `settings.rs::RelayProtocol::AnthropicMessages` | 协议枚举（JSON 值 `anthropicMessages`） |
| `settings.rs::RelayProfile::anthropic_auth_header` / `anthropic_max_tokens` | 认证字段与 `max_tokens` 兜底 |
| `relay_headers.rs::apply_for_protocol` | 按协议注入 `x-api-key` / `Authorization` + `anthropic-version` |
| `protocol_proxy.rs::upstream_request_parts` | 选路 `/v1/messages`、生成 Anthropic 请求体、标注 `UpstreamWireApi::AnthropicMessages` |
| `launcher.rs` | 流式/非流式的响应转换与「缺完成事件」判定 |
| `relay_config.rs` | Codex 侧 base_url 指向本机代理、供应商测试走 `/v1/messages` |

## 行为约定

- **认证字段**：默认 `x-api-key`（Anthropic 原生），可切 `Authorization: Bearer`
  （多数中转网关）。显式配置的同名自定义头优先，两种都不发重复头。
  `anthropic-version: 2023-06-01` 始终注入，用户可覆盖。
- **`max_tokens` 必填**：Codex 请求带 `max_output_tokens` 时原样映射，否则用
  profile 的 `anthropicMaxTokens`（默认 8192）兜底。
- **`temperature` 收敛**：OpenAI 允许到 2.0，Anthropic 只到 1.0，超出会被夹到 1.0。
- **消息形态**：`system` 消息收敛成顶层 `system`；同角色消息合并；首条必须是 user，
  否则补一条 `(continuing the conversation)`；`tool` 消息转成 `tool_result` 内容块。
- **流式终态**：`message_stop` 是完成事件。缺失时不补 `[DONE]`，按「流被掐断」上报，
  不把截断的回答伪装成成功。
- **协议能力开关不联动**：`standardOpenaiProtocol`、远程原生压缩、搜索历史压缩兼容
  等 Chat/Responses 专有开关仍只认各自协议。

## 测试

- `crates/codex-plus-core/tests/anthropic_upstream.rs`：请求体/URL/认证头、
  Bearer 模式、流式转换（含跨 chunk 缓冲）、供应商测试探针、profile 落盘指向代理、
  协议变体与默认值契约。
- `anthropic_bridge.rs` 模块内测试：Chat→Anthropic 映射、Anthropic→Chat 映射、
  SSE 事件映射、截断与错误事件语义。
