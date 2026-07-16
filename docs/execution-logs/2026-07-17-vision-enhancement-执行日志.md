# 视觉与推理能力增强 — 操作日志

> 日期：2026-07-16 ~ 2026-07-17
> 分支：vision-enhancement（从 `origin/main` @ 285f40e 拉出）
> Spec：docs/superpowers/specs/2026-07-16-vision-enhancement-design.md
> Plan：docs/superpowers/plans/2026-07-16-vision-enhancement.md
> 执行方式：Subagent-Driven Development（每 task 独立实施 + spec/quality 双审查）

---

## 概览

| Task | 功能 | Commit | 状态 |
|---|---|---|---|
| 0 | 建分支 vision-enhancement | - | ✅ |
| 1 | F1 settings 字段 | ed53bfe | ✅ |
| 2 | F1 reasoning 剥离 + 钩子 | 71415e1 | ✅ |
| 3 | F5 model-windows.ts checkbox 类型 | 479bf32 | ✅ |
| 4 | F5 App.tsx checkbox UI | e9d3709 | ✅ |
| 5 | F2 vision.rs 两层缓存 + VLM 调用链 | 17d426e | ✅ |
| 6 | F2 strip_image_blocks 两层接入 | 4a51425 | ✅ |
| 7 | F3 fail-open + 状态注入 | 99bb35c | ✅ |
| 8 | F4 追问注入 | 2504b80 | ✅ |
| 9 | F6 埋点查漏 + 全量回归 | - | ✅ |

---

## 测试结果

### Rust（最终验证 @ 2026-07-17）
- **vision 单元测试：** 65 passed, 0 failed（`cargo test -p codex-plus-core --lib -- vision::`）
- **protocol_proxy 集成测试：** 41 passed, 7 pre-existing failures (HTTP proxy 环境依赖)
- **ads 测试：** 6 passed, 1 pre-existing failure (网络依赖)
- **其他 crate：** 全部通过
- **Binary build：** `codex-plus-plus` + `codex-plus-plus-manager` 编译成功

### 前端（最终验证 @ 2026-07-17）
- **TypeScript 编译：** 0 errors（`npx tsc --noEmit`）
- **model-windows 测试：** 13/13 pass（`npx tsx --test`）

---

## 与 Spec 的核查

| Spec 功能 | 实现状态 | 备注 |
|---|---|---|
| F1 reasoning 剥离（两协议） | ✅ | Chat 路径在 `apply_chat_reasoning_options` 之前剥离；Responses 透传也在转换前剥离 |
| F2 两层缓存 (URL, 问题) + 重发入口 | ✅ | 当前轮 Tier2 key + tier2_prompt；历史轮 Tier1 URL key + TIER1_PROMPT；`collect_input_text` 含无文字回退上文 |
| F2 无文字回退上文问题 | ✅ | `collect_input_text` 从最新 user 消息回溯，复用上文问题做 Tier2 焦点 |
| F3 剥图后状态注入 (fail-open) | ✅ | VLM 失败 → strip 当前轮 + 注入看不到图提示 + vl_strip；溢出 → 强注入；strip 模式 → 注入 + vl_strip |
| F4 追问注入 | ✅ | 纯文本追问 + 历史有图 → 注入「从描述答/重发图」提示；若已 strip（fail-open/overflow）则跳过 |
| F5 前端 checkbox 化 | ✅ | 只支持文本 / 不支持推理 双 checkbox；textOnly 禁用 = 纯 Responses（vlmUnsupportedProtocol） |
| F5 旧值迁移 | ✅ | modelVlm 中 vlm/strip → textOnly=true；丢失的 vlm/strip 区别由 VL 配置派生 |
| F6 埋点 | ✅ | vl_call（7 种 status）+ vl_strip（3 种 reason：overflow / vl_failed / strip_mode） |
| #1405 保留项 | ✅ | golden window(10)、分析深度(50)、溢出保护(90% margin)、per-relay VL 配置、Phase 2 骨架 均不动 |

### 微小差异

| 项 | 说明 |
|---|---|
| over_limit vl_strip reason | Spec 提及但 #1405 架构无 per-request 图片数量上限，该 reason 无对应路径 |
| Phase 2 后台分析 | #1405 的 Phase 2 逻辑在 Task 5 中被 gutted（bg_config_opt 始终 None），类型签名保留但实际逻辑未恢复。留作后续独立 PR |

---

## 环境说明

- **Rust 工具链：** 手动下载 rustc 1.97.1 + cargo 1.97.1 + rust-std 至 `~/.rustup/toolchains/stable-aarch64-apple-darwin/`（rustup 下载因网络问题失败，改为直接下载 tarball 解压）
- **Node.js：** v26.5.0（nvm 管理）
- **Pre-existing 测试失败：** 7 个 HTTP proxy 测试 + 1 个 ads 网络测试 + 6 个 wiremock 视觉测试（环境依赖，非本分支引入）

---

## 提交历史

```
2504b80 feat(vision): 纯文本追问+历史有图注入强化提示，防追问胡说
99bb35c feat(vision): VLM 失败 fail-open + 剥图/溢出/strip 注入状态提示与操作引导
4a51425 feat(vision): 当前轮 Tier2(URL,问题)+tier2_prompt，历史轮 Tier1(URL)+TIER1，重发入口
17d426e feat(vision): 两层缓存(DefaultHasher) + 逐图 [[图片K]] VLM 调用 + 双 prompt + 埋点（替换 #1405 内部）
71415e1 feat(reasoning): Chat/Responses 两协议剥离不支持模型的 reasoning 字段
ed53bfe feat(settings): RelayProfile 新增 model_reasoning_support 字段
e9d3709 feat(ui): per-model 三态下拉框 -> 只支持文本/不支持推理 双 checkbox
479bf32 feat(model-windows): checkbox 类型 textOnly/noReasoning + 派生 modelVlm + 旧值迁移
```

---

## 已知局限

1. **Phase 2 后台补齐未恢复**：Task 5 重构时保留了类型签名，但实际后台分析逻辑已移除。留作后续独立 PR。
2. **追问注入的假阳性**：proxy 侧无法判断追问是否与图片相关，所有纯文本追问均注入。tool-call 架构（模型自主决定是否调 VL）为正解。
3. **无文字回退上文问题**：仅覆盖最新消息纯无文字的情况。若最新消息带文字但只是指针（如「看这个例子」），启发式无法判定。
4. **cache_put_evicts_oldest_when_full 偶发失败**：共享全局 `LazyLock<Mutex<HashMap>>` 缓存导致多线程测试竞争，`--test-threads=1` 下通过。