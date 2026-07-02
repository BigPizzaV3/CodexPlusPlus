# P1: ChatSseToResponsesConverter 解耦 SSE 解析与协议转换

基于 cc-switch 启发——将 SSE/JSON 解析从转换器中分离到 HTTP 层，转换器只做纯字段映射。

## 现状

```
launcher.rs (handle_protocol_proxy_connection)
  bytes_stream chunk
    ↓
  converter.push_bytes(&bytes)       ← 一次性做完 3 件事
    ├─ append_utf8_safe()            ← UTF-8 缓冲
    ├─ take_sse_block()              ← SSE 边界扫描
    ├─ handle_block()                ← SSE 行解析 + JSON parse
    │   └─ serde_json::from_str()
    └─ handle_chat_chunk_into()      ← 字段映射 + Think tag 检测
        └─ push_sse()                ← re-encode → Responses SSE
```

## 目标架构（cc-switch 模式）

```
launcher.rs (handle_protocol_proxy_connection)
  bytes_stream chunk
    ↓
  SseBlockParser.push_bytes(&bytes)  ← 只负责 SSE/JSON 解析
    ├─ append_utf8_safe()
    ├─ take_sse_block()
    └─ handle_block() → event: Option<String>, data: Option<String>
    ↓
  serde_json::from_str(&data)        ← 一次 JSON parse（不可省）
    ↓
  converter.feed_chunk(&Value)       ← 只负责字段映射 + emit
    ├─ handle_chat_chunk_into()      ← 复用现有逻辑
    └─ push_sse()
```

## Task 列表

### Task 1: 新增 `SseBlockParser` 工具（protocol_proxy.rs）

将 `ChatSseToResponsesConverter` 里的 SSE 解析逻辑抽出来：

```rust
pub struct SseBlockParser {
    buffer: String,
    utf8_remainder: Vec<u8>,
}

pub struct ParsedSseBlock {
    pub event: Option<String>,
    pub data: Option<String>,
}

impl SseBlockParser {
    pub fn new() -> Self;
    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<ParsedSseBlock>;
    pub fn drain_remainder(&mut self) -> Option<String>;  // for finish()
}
```

将现有 `append_utf8_safe`、`take_sse_block`、`strip_sse_field` 和 `handle_block` 的数据提取部分移入此结构体。

**拒绝范围蔓延**：不改变 `take_sse_block`/`strip_sse_field` 的算法，只挪位置。

### Task 2: 新增 `ChatSseToResponsesConverter::feed_chunk()` 方法

```rust
impl ChatSseToResponsesConverter {
    /// 处理一个已解析的 Chat Completions SSE chunk，返回 Responses SSE 事件字节。
    pub fn feed_chunk(&mut self, chunk: &Value) -> Vec<u8>;

    /// 处理 [DONE] 信号，返回最终事件。
    pub fn feed_done(&mut self) -> Vec<u8>;

    /// 处理 error 事件，返回 error SSE。
    pub fn feed_error(&mut self, message: String, error_type: Option<String>) -> Vec<u8>;
}
```

内部复用现有的 `handle_chat_chunk_into`、`finalize_into`、`failed_into`。

**保留 `push_bytes` 作为兼容接口**，内部调用 `SseBlockParser` + `feed_chunk`。

### Task 3: 改造 launcher.rs 流处理循环

将 `handle_protocol_proxy_connection` 的流处理（line 1328-1360）改为：

```rust
let mut parser = SseBlockParser::new();
let mut converter = request_json
    .as_ref()
    .map(ChatSseToResponsesConverter::with_request)
    .unwrap_or_default();

while let Some(chunk) = bytes_stream.next().await {
    match chunk {
        Ok(bytes) => {
            for block in parser.push_bytes(&bytes) {
                match (block.event.as_deref(), block.data.as_deref()) {
                    (Some("error"), _) | (_, Some(data)) if data.contains("\"error\"") => {
                        // error handling → converter.feed_error()
                    }
                    (_, Some("[DONE]")) => {
                        let tail = converter.feed_done();
                        stream.write_all(&tail).await?;
                    }
                    (_, Some(data)) => {
                        if let Ok(value) = serde_json::from_str::<Value>(data) {
                            let converted = converter.feed_chunk(&value);
                            stream.write_all(&converted).await?;
                        }
                    }
                    _ => {}
                }
            }
        }
        Err(error) => { /* error handling */ }
    }
}
```

### Task 4: Content-delta 快速路径（叠加优化）

在 `feed_chunk()` 内部，对最常见的 content-only delta 做快速路径：

```rust
pub fn feed_chunk(&mut self, chunk: &Value) -> Vec<u8> {
    let mut output = String::new();

    // Fast path: simple content delta (90%+ of chunks)
    if let Some(content) = extract_content_delta(chunk) {
        if self.state.text_active() {
            self.state.push_content_delta_fast(&content, &mut output);
            if !output.is_empty() {
                return output.into_bytes();
            }
        }
    }

    // Full path: handle all field types
    self.state.handle_chat_chunk_into(chunk, &mut output);
    output.into_bytes()
}
```

快速路径条件：
- `choices[0].delta` 只含 `content` 字段（无 tool_calls、无 reasoning_content、无 refusal）
- text item 已经 started（`response.output_item.added` 已发过）

快速路径输出：单条 `response.output_text.delta` SSE 事件。

### Task 5: 单元测试

**SseBlockParser 测试**（`tests/protocol_proxy.rs`）：
- `test_parser_single_block` — 基本 SSE 解析
- `test_parser_multi_line_data` — 多行 data 拼接
- `test_parser_done_signal` — [DONE] 识别
- `test_parser_empty_blocks` — 空行跳过
- `test_parser_utf8_partial` — UTF-8 跨 chunk 处理

**feed_chunk 测试**：
- `test_feed_content_delta` — 普通文本 delta
- `test_feed_reasoning_delta` — 推理文本
- `test_feed_tool_call_delta` — 工具调用
- `test_feed_content_fast_path` — 快速路径触发
- `test_feed_full_equivalence` — push_bytes 和 feed_chunk 输出一致

**回归测试**：确保现有 `push_bytes` 接口不变（兼容性）。

## 验收标准

1. `cargo check -p codex-plus-core` — 编译通过
2. `cargo test -p codex-plus-core` — 全量测试通过（新增 + 回归）
3. 现有 `push_bytes` 接口行为不变（向后兼容）
4. `feed_chunk` 输出的 Responses SSE 与 `push_bytes` 输出逐字节一致（协议等价性）
5. Content-delta 快速路径覆盖 90%+ 的 streaming chunk（可通过 diagnostic log 验证）

## 风险与降级

| 风险 | 缓解 |
|------|------|
| SSE 解析逻辑拆分引入 bug | 保留原 `push_bytes` 作为黄金标准，新路径用等效性测试验证 |
| 快速路径遗漏边界情况 | 快速路径只对简单 content delta 生效，其他全部走完整路径 |
| UTF-8 跨 chunk 处理不一致 | `SseBlockParser` 直接复用现有的 `append_utf8_safe` |
| 现有测试依赖内部实现 | 新增公开 API 的测试，不修改现有测试 |

## 参考

- cc-switch: https://github.com/farion1231/cc-switch
- ccswitch-deepseek SSE translator: https://github.com/yangfei4913438/codex-deepseek/blob/main/src/sse.py
- 相关 issues: #563, #865, #903
