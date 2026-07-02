# 2026-07-02: CodexPlusPlus 性能优化合集

## 结果

PR #1299 (`perf/cached-http-client`) — 4 个优化，91 测试通过，3 次 subagent 审查全部 APPROVED。

## 优化列表

| # | 优先级 | 优化 | 文件 | 收益 |
|---|--------|------|------|------|
| 1 | P0 | CDP 事件驱动启动检测 | launcher.rs | 启动 −55s |
| 2 | P1 | reqwest Client OnceLock 缓存 | http_client.rs | TCP 连接复用 |
| 3 | P1 | 模型列表桥接捷径 | launcher.rs | 启动 −34s |
| 4 | P1 | SSE 转换器解耦 | protocol_proxy.rs | per-token 延迟 ↓ |

## 关键决策

- **CDP**: Playwright 已验证的 stderr pipe pattern（匹配 "DevTools listening on ws://"）+ 指数退避 TCP 兜底
- **模型列表**: Cherry-pick PR #620 的 bridge 捷径，保留事件驱动 CDP（vs 200ms 轮询）
- **SSE 解耦**: 参考 cc-switch（150 行 Python），将 SSE/JSON 解析从 3955 行 converter 分离
- **向后兼容**: push_bytes 接口保留，内部委托给新 API（等价性测试验证）

## 踩坑

1. `oneshot::Receiver` 需要 `Mutex<Option<>>` 包装才能在 async context 中安全传递
2. `#[derive(Default)]` 对 `Mutex<Option<T>>` 天然兼容（Mutex::new(None)）
3. PR #620 存在 CDP 轮询冲突，采用 cherry-pick + 提案协作策略
4. `extract_chat_sse_error` 需要从 `fn` 改为 `pub fn` 因为 launcher.rs 直接调用

## 审查历史

- Gate 2 (plan review): REQUEST_CHANGES → 补充 TDD + 删除 debug_port → APPROVED
- Gate 3.5 (test subagent): 3 wait_for_cdp_ready tests — 全部通过
- Gate 4 (code review): 91 测试通过 — APPROVED
- SSE plan review: APPROVED with 3 notes (impl Default, extract_chat_sse_error, is_text_started)
- SSE code review: APPROVED — 7 tests pass, all plan notes addressed
