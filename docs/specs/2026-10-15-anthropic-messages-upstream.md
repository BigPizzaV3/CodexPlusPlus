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

中途态表达力弱于 Anthropic 的部分，用**旁路状态**补齐（不推倒中间态）：

- **思考签名**：Anthropic 的 `thinking.signature` 在 Chat 形态里没有落点，于是把整组
  签名块 base64 后塞进 Responses reasoning item 的 `encrypted_content`
  （前缀 `codex-plus-anthropic-thinking-v1:`）。Codex 下一轮会把该 item 原样放回
  `input`，桥接层解出来再插到助手消息最前面——签名随客户端走，代理无状态。
  多块思考合并成一个信封，因为 Chat 中枢每轮只产出一个 reasoning item。
- **提示缓存**：`cache_control` 断点由桥接层注入（工具定义末尾 → 系统提示末尾 →
  最新可缓存消息 → 更早的 user 锚点，最多 4 个，已有标记不动）。
- **1M 上下文**：复用本仓已有的窗口声明（`modelWindows` / `contextWindow`），
  声明值 ≥ 1,000,000 时自动附上 `context-1m-2025-08-07` beta 头。

代价只剩一次纯内存 JSON 变换（微秒级）。

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
- **`max_tokens` 优先级**：供应商表单显式填写的值 > 请求自带 `max_output_tokens` >
  内置 8192。用户填了值就不该被请求悄悄盖掉（显式配置优先）。
- **思考（extended thinking）** 有两种下发方式，由 `anthropicThinkingMode` 选择：
  - `auto`（默认）：新代模型（opus-5 / sonnet-5 / haiku-5 / fable-5 / mythos-5 与
    4.6 以后的家族）走**等级透传**——`thinking: {type: adaptive}` +
    `output_config: {effort}`，等级不再被预算夹取；旧代模型回落经典预算。
    依据是实测：旧代模型收到 `adaptive` 不报错但会静默忽略，交给预算更可控。
  - `adaptive` / `budget` / `off`：强制其中一种，或彻底关掉。
  - 等级词表：`minimal`/`low` → `low`，`medium` → `medium`，`high` → `high`，
    `xhigh`/`max`/`ultra` → `max`（Anthropic 只认这四档，网关普遍不校验取值，
    必须自己收敛）；`none`/`off`/缺省不发 thinking。
  - 经典预算路径：low 2048 / medium 8192 / high 16384 / xhigh·max·ultra 24576，
    并按 `max_tokens / 2` 夹取给可见回答留余量；夹取后低于 1024 就不开思考。
  - 两种方式都会移除 `temperature` / `top_p`；历史里已有工具调用却拿不到签名块时
    **不开启**思考——缺签名必然 400，宁可不思考也不发一个失败请求。
- **伪装 Claude Code**（默认关）：UA 换成 `claude-cli/1.0.119 (external, cli)`、
  附 `x-app: cli` 与 `anthropic-beta: claude-code-20250219`，并在系统提示首行注入官方
  身份串；仅用于网关限制「只能通过 Claude Code 使用」的场景。
- **流式终态**：`message_stop` 是完成事件。缺失时不补 `[DONE]`，按「流被掐断」上报，
  不把截断的回答伪装成成功。
- **协议能力开关不联动**：`standardOpenaiProtocol`、远程原生压缩、搜索历史压缩兼容
  等 Chat/Responses 专有开关仍只认各自协议。

## 真机验收

常规回归全部使用本地 mock 上游；真机验收单独放在
`crates/codex-plus-core/tests/live_anthropic_upstream.rs`，默认 `#[ignore]`，
需要真实网关与额度时用环境变量驱动：

```text
LIVE_ANTHROPIC_BASE_URL=https://your-gateway.example.com \
LIVE_ANTHROPIC_API_KEY=sk-... \
LIVE_ANTHROPIC_MODEL=claude-sonnet-4-6 \
cargo test -p codex-plus-core --test live_anthropic_upstream -- --ignored --nocapture --test-threads=1
```

覆盖 9 项：认证/端点探针、非流式、流式事件序列、工具调用回传、**思考签名跨轮回放**、
截断上报、1M beta 头、Claude Code 伪装、提示缓存命中；请求带 3 次重试，
网关按通道路由时的偶发 5xx 不会误报。

## 测试

- `crates/codex-plus-core/tests/anthropic_upstream.rs`：请求体/URL/认证头、
  Bearer 模式、流式转换（含跨 chunk 缓冲）、供应商测试探针、profile 落盘指向代理、
  协议变体与默认值契约。
- `anthropic_bridge.rs` 模块内测试：Chat→Anthropic 映射、Anthropic→Chat 映射、
  SSE 事件映射、截断与错误事件语义。
