# CodexPlusPlus 性能优化计划

基于代码审查 + GitHub issues 调研，2026-07-02 整理。

## 优化收益矩阵

| 优先级 | 问题 | 方案 | 预期效果 | 难度 | 状态 |
|--------|------|------|---------|------|------|
| P0 | 启动慢 54.9s 空白期 | ensure_injection 120×1s 轮询改事件驱动 | 启动 55s → ~0-2s | 中 | ✅ PR #1299 |
| P1 | 每次请求创建新 HTTP Client | OnceLock 全局缓存 reqwest::Client | TCP 连接复用 | 低 | ✅ PR #1299 |
| P1 | 模型列表 app-server RPC 阻塞 ~34s | bridge 捷径跳过大模型列表 RPC | −34s 启动时间 | 低 | ✅ PR #1299（启发自 #620） |
| P1 | 对话卡顿（issues #563, #865, #903）| ChatSseToResponsesConverter 解耦 SSE 解析 | 减少 per-chunk 延迟 | 中 | 📝 计划已写，审查中 |
| P2 | 重复消息耗 token (issue #1231) | Responses SSE 输出加 dedup filter | 省 5-20% token | 中 | 📝 待做 |

## 启动慢根因

核心路径: `launch_and_inject_with_hooks` → `ensure_injection`

```
helper.listening → [54.9秒空白] → helper.backend_status_ok
     ↓                       ↓                    ↓
 helper 已就绪       ensure_injection 循环    bridge 注入成功
                    最多 120 次 × 1s 轮询
                    (等待 Codex 页面准备好 CDP 连接)
```

代码位置: `crates/codex-plus-core/src/launcher.rs:153-177`

```rust
async fn ensure_injection(&self, debug_port, helper_port, app_dir) -> bool {
    for attempt in 1..=120 {          // ← 最多等 120 秒！
        match self.bridge_context(debug_port, app_dir).await {
            Ok(Some(ctx)) => self.inject_bridge(...).await?,
            Ok(None) => self.inject(...).await?,
            Err(error) => { /* retry */ }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
```

嵌套 retry: 外层 `ensure_injection` ×120 (1s each) + 内层 `try_inject` ×20 (500ms each)。

## 对话卡顿根因

每个 streaming chunk 经过完整 CPU 密集型转换:

```
bytes_stream chunk
  → append_utf8_safe()
  → take_sse_block() loop (扫描 \n\n)
  → serde_json::from_str()
  → handle_chat_chunk_into()
    → think tag 检测
    → push_reasoning_delta_into()
    → push_content_delta_into()
    → push_tool_call_delta_into()
  → push_sse() (重新序列化为 Responses SSE)
```

## 已完成的优化

| PR | 内容 | 收益 |
|----|------|------|
| #1299 (Draft) | `OnceLock` 缓存 `reqwest::Client` | TCP 连接复用, 省 TLS 握手 |
