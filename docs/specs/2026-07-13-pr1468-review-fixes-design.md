# PR #1468 Review 修复设计

> 日期：2026-07-13
> 关联 PR：https://github.com/BigPizzaV3/CodexPlusPlus/pull/1468
> 关联评审意见：`#issuecomment-4955348317`（reviewer: BigPizzaV3）
> 参考 PR：https://github.com/BigPizzaV3/CodexPlusPlus/pull/1405（VLM vision analysis，同类方案）
> 分支：`strip-images-feature` -> `main`

---

## 一、背景与目标

**为什么做这个**：PR #1468 实现"纯文本模型图片处理"（per-model 能力判断、VL 视觉模型中转、Reasoning 剥离）。reviewer 提了 6 条意见（前 3 阻塞、后 3 改进）。本设计的目的：解决全部 6 条使 PR 达到可合并状态，同时借机把 VL 逻辑抽成独立模块、引入缓存/并发/重试，让 VL 中转在生产中真正可用。

**实现了什么目的**：
- 3 个阻塞项修复（no_proxy 污染、reasoning 死代码、UTF-8 panic）-> 满足合并门槛。
- VL 中转从"串行 300s 无缓存"升级为"并发 + 两层缓存 + 重试 + 入口"-> 生产可用。
- VL 逻辑抽成 `vision.rs` -> 可独立测试维护。

**非目标**：
- 不改 PR #1468 整体架构（Chat 走代理 / Responses 纯透传）。
- 不引入 PR #1405 的 golden window / 两阶段后台分析 / 上下文溢出动态保护。
- 不做 tool-call 架构（基座自主调 VL）--留作未来独立 PR。

---

## 二、架构上下文

PR #1468 的核心设计决策（保持不变）：

```
Codex App -> Proxy (127.0.0.1:57321) -> 上游
              │
              ├─ Chat Completions 协议：代理拦截，做协议转换 + strip + VL + reasoning 剥离
              └─ Responses 协议：纯透传（pureApi），代理不干预
```

| 路径 | 图片处理 | reasoning 剥离 | VL 中转 |
|------|---------|---------------|---------|
| Chat（转换路径） | ✅ 支持（VL 描述 / strip 二选一） | ✅（Bug 2 修复后） | ✅ |
| Responses（透传） | ❌ 不处理 | ❌ 不处理 | ❌ |

**Chat 路径图片处理三种情况**（VL 与 strip 是替代关系，非顺序）：
- 模型支持图片 -> 保留，不 strip、不 VL
- 模型不支持图片 + VL 启用 -> VL 描述成文字替换图片，不 strip
- 模型不支持图片 + VL 未启用/失败 -> strip 丢弃，不报错

**关键位置**：`apply_vl_with_fallback`（VL 入口）、`upstream_request_parts`（协议转换边界，图片+reasoning 剥离点）。

---

## 三、6 个 Bug 的修复设计

每个 bug 按"为什么 / 问题 / 方案 / 好处 / 坏处"描述；操作细节附"实现要点"，详细步骤留待 plan。

### Bug 1（阻塞）：`.no_proxy()` 污染共享 client

**为什么**：生产真实上游请求（广告、更新、模型列表、插件市场、供应商测试、上游 API）依赖系统代理（VPN/公司代理）。`.no_proxy()` 全断了，影响所有靠代理上网的用户。

**问题**：PR #1468 给共享工厂 `proxied_client()` 加了 `.no_proxy()`。reqwest 的 `.no_proxy()` 是"彻底关闭代理解析"（忽略所有代理 env），不是"只绕过 127.0.0.1"。该工厂被 13 处共用，所有生产请求受影响。当初是为修测试 502（macOS 系统代理拦截 127.0.0.1 mock），但用了核弹级方案。

**方案**：撤回 `proxied_client()` 的 `.no_proxy()`，恢复生产代理支持。测试的 127.0.0.1 拦截用 `NO_PROXY=127.0.0.1,localhost` env 解决（reqwest 会读，只绕 localhost，公网照常走代理）。

**好处**：生产恢复系统代理；测试 127.0.0.1 仍绕过；改动小。
**坏处/局限**：测试需确保 `NO_PROXY` env 在 client 构建前设置（reqwest 构建时读 env）；若有 static client 缓存需注意。

**实现要点**：`http_client.rs:8-12` 撤回 `.no_proxy()`；`tests/protocol_proxy.rs` 入口设 `NO_PROXY` env。

---

### Bug 2（阻塞）：reasoning 剥离是死代码

**为什么**：UI 让用户勾选"模型不支持 reasoning"，配置能存，但请求照带 reasoning 发上游，不支持 reasoning 的模型（kimi-k2.6 等）报错。功能形同虚设。

**问题**：`model_supports_reasoning` / `strip_reasoning_in_place` 只在测试调用，生产转发链路零调用。图片剥离已接入生产，reasoning 没有。

**方案**：在 `upstream_request_parts` 的 Chat 分支、转换之前调 `strip_reasoning_in_place`（与图片剥离同一位置）。`supports_reasoning` 内部现算（不耦合 VL）。放这里而非 `apply_vl_with_fallback`，因后者 VL 未启用时提前返回，reasoning 剥离会漏执行。

**好处**：reasoning 剥离真正生效；与图片剥离同位置，逻辑统一；不受 VL 开关影响。
**坏处/局限**：只覆盖 Chat 路径。**已知局限**：Responses 协议下 text-only 模型不支持 reasoning 仍会报错（Responses 纯透传不剥离，用户已接受）。顺带清除死代码 `strip_input_images_in_place`（同为死代码，按"Responses 不剥离图片"决策无生产用途）。

**实现要点**：`upstream_request_parts` Chat 分支转换前调 `strip_reasoning_in_place`；删 `strip_input_images_in_place` 及其测试。

---

### Bug 3 + Bug 6（合并，同一行）：UTF-8 截断 panic + 日志泄露正文

> 自审发现：Bug 3 与 Bug 6 在同一行（`protocol_proxy.rs:1131`）且方案矛盾--Bug 3 要 char-safe 保留 preview，Bug 6 要删 preview。删 preview 同时解决两者，故合并。

**为什么**：VL 描述日志会 panic（中文场景几乎必现），且泄露截图敏感内容（密钥/PII/代码）。

**问题**：`description_preview: &description[..description.len().min(200)]` 有两个问题：
1. 字节截断：中文 3 字节，200 落在汉字中间 -> `panic: byte index 200 is not a char boundary`。
2. 记录描述正文：可能含截图敏感信息，写进 diagnostic_log 落盘。

**方案**：删除 `description_preview`，只记元数据（`description_len`、`description_chars`、vlModel、状态）。

**好处**：一石二鸟--删 preview 即无 truncation（Bug 3 panic 消失），又不记正文（Bug 6 泄露解决）。**Bug 6 吸收 Bug 3**，无需单独 char-safe 截断。
**坏处/局限**：丢失调试用描述预览（调试时可临时加回 char-safe 版本，生产不记正文）。

**实现要点**：`protocol_proxy.rs:1131` 删 `description_preview`，换 `description_len` / `description_chars`。

---

### Bug 4（改进）：VL 串行 + 无缓存 + 无并发 + 无重试 + 无入口

**为什么**：VL 中转在生产中不可用--10 张图 × 30s = 最坏 300s 请求卡死；failover/重复轮次重复调 VL 浪费费用；一张坏图拖垮整批；缓存后无法针对同一图追问新信息。

**问题**：
- 串行：逐图 await，无并发。
- 无缓存：failover 重跑、后续轮次重发历史图都重复调 VL。
- 无总超时：单次 30s × 10 = 300s。
- 无重试：瞬时失败直接降级 strip。
- 无入口：缓存后无法针对同一图追问新信息（拿不到旧描述没有的元素）。

**方案**（架构 A+，proxy 驱动两层）：

1. **抽 `vision.rs` 模块**：VL 逻辑从 `protocol_proxy.rs` 独立，可单独测试，`protocol_proxy.rs` 不再膨胀。

2. **并发**：`Semaphore(5)` + `JoinSet`（tokio 自带，零新依赖）。最多 5 个 VL 调用同时飞。

3. **批次（多图一调用）**：5 张图/次 API 调用（一个 messages 放 5 个 image_url + 1 个文本 prompt），省调用次数和费用。

4. **两层缓存**（核心，解决"无缓存 + 无入口"）：
   - **Tier 1（历史图）**：URL key + comprehensive 描述（**不含问题**）。图从"最新"变"历史"时生成一次，之后每轮命中。question-invariant -> URL 缓存稳定。这是省调用大头。
   - **Tier 2（最新/重发图）**：(URL, 问题) key + comprehensive+侧重描述。最新消息里的图用此层；重发图+新问题 = **入口**，触发新调用拿新信息。
   - **检测**：最新 user 消息里的图 = Tier 2；context_window 内的历史图 = Tier 1。
   - **为什么两层而非单层 (URL,Q)**：单层 (URL,Q) 对所有图 -> 每个新问题所有历史图都重调（贵，N 图 × M 问题）；两层 -> 历史图 URL 缓存（便宜），只有最新图随问题重调（入口）。用历史 URL 缓存换便宜，代价是最新/历史检测复杂度。
   - 500 条 / 24h TTL / 删最旧；`DefaultHasher`（比 SHA256 快，缓存 key 不需密码学强度；sha2 已是项目依赖但此处不必用）。

5. **双层超时**：per-batch `15+8n` 秒；总 120s 硬截断（`tokio::time::timeout`）。

6. **混合重试**：批量 2 次（治瞬时故障：网络/429/5xx）-> 失败拆单张各 3 次（隔离坏图）。指数退避 0.3/0.6s + 抖动。120s 总超时兜底。坏图自己重试自己的，最后只丢坏图，不拖累好图。

7. **两个 VL prompt**（关键：Tier 1 不含问题，Tier 2 含问题）：
   - Tier 1（历史，无问题）：`请详细描述这张图片，涵盖所有视觉信息：文字（逐字 OCR）、UI 元素、颜色、形状、布局结构、错误信息等。请用中文回复。`
   - Tier 2（最新，含问题）：Tier 1 + `用户当前问题：{question}\n在全面描述基础上，对与上述问题相关的内容做更详细说明。`
   - "涵盖所有视觉信息" -> 不漏（解决"全面描述也会漏主要情节"）；"对问题相关处加深" -> 侧重深度。不发多轮历史给 VLM（底座模型自持完整对话上下文）。

8. **入口机制**：用户重发图 + 问新问题 -> 图成"最新" -> Tier 2 新调用 -> 新侧重描述（旧 comprehensive 没覆盖的元素）。天然入口，无需额外机制。重复问题命中缓存；新问题新调用（这是入口的本意，非缓存失效）。

9. **不做完整溢出保护**：主模型服务端 tokenizer 自动把 VL 描述计入上下文，正常自管；`max_tokens` 限描述长度。廉价兜底：描述 >2000 字符截断（**char-safe**，`chars().take(2000)`，避免重蹈 Bug 3 覆辙）。

**好处**：
- 快：并发 + 批次，300s -> 120s 内。
- 省：两层缓存，5 图 × 10 轮 ~50 调用 -> ~10 调用（省 ~80%）。
- 鲁棒：重试 + 坏图隔离，一张坏图不拖垮整批。
- 入口：重发图能追问新信息（解决"缓存后拿不到新元素"）。
- 模块化：`vision.rs` 独立，可测试可维护。

**坏处/局限**：
- **入口需重发图**：文本追问（不重发）不触发重查，历史图用缓存 comprehensive，若没覆盖追问点基座答不出。tool-call 架构（基座自主决定）能解决但需工具调用基础设施，留作未来。
- **两层复杂度**：两层缓存 + 两 prompt + 最新/历史检测，比单层复杂（用历史 URL 缓存换便宜，代价是检测复杂度）。
- **不做后台静默补齐**（PR #1405 有）：首对话多历史图时仍同步处理（120s 内），非后台预取。
- **不做溢出动态保护**：极端场景（多图 + 超长历史 + 小上下文）可能溢出报错（边缘场景，接受）。

**澄清：缓存容量 vs context_window（两者独立）**
- 缓存容量（500 条）：控制**记住多少描述**（内存）。命中 = 不消耗 token（COST 优化）。
- `context_window`（token 预算）：控制**哪些图被处理**（visibility，EFFECT 决策）。窗口内图被描述+注入 -> 底座"看见"；窗口外图被 strip -> 底座**完全看不见**，即使缓存有描述也不注入。
- 缓存让大窗口可负担（窗口内命中的图免费），但窗口仍决定可见范围。窗口设小 -> 老图被 strip -> 底座丢失早期视觉上下文（影响效果，非只影响成本）。

**实现要点**：抽 `vision.rs`；`Semaphore`+`JoinSet`；`call_vlm_batch`；两层 `HashMap` 缓存；`tokio::time::timeout`；混合重试 loop；两 prompt；最新/历史检测。详细步骤留待 plan。

---

### Bug 5（改进）：VL endpoint 裸拼路径

**为什么**：VL 调用的 endpoint 裸拼路径，裸域名缺 `/v1`、完整 endpoint 重复拼，VL 调用失败。

**问题**：`protocol_proxy.rs:1039-1042` 用 `format!("{}/chat/completions", base_url)` 裸拼，没复用已有的 `chat_completions_url()` / `responses_url()`（它们处理 `/v1` 补全、`/v1/v1` 去重、`#` 跳过、origin-only 判断）。

**方案**：复用现有 helper：`chat_completions_url(&base_url)` / `responses_url(&base_url)`。

**好处**：复用已验证的规范化逻辑；一行改动；顺带 PR #1405 有同样 bug 可反向提醒。
**坏处/局限**：无。

**实现要点**：`protocol_proxy.rs:1039-1042` 换 helper 调用；补裸域名/完整 endpoint 测试。

---

## 四、实施顺序

全部 6 个本批次改完。按依赖与风险排序：

1. **Bug 5、6**（trivial，`protocol_proxy.rs:1031-1042` 与 `:1131` 同区域，顺路；Bug 3 并入 Bug 6）
2. **Bug 1、2、3**（3 阻塞项，中等改动）
3. **Bug 4**（抽 `vision.rs` + 并发/缓存/超时/重试，最大改动，单独 commit）

每个 bug 遵循 TDD：RED（写失败测试）-> GREEN（最小实现）-> 回归（跑全套件）。

---

## 五、测试计划

| Bug | 新增测试 | 回归测试 |
|-----|---------|---------|
| 1 | `proxied_client()` 不强制 no_proxy；`NO_PROXY` env 下 6 个原测试通过 | 现有 http_client 测试 |
| 2 | Chat 剥离 reasoning（不支持时）/ 保留（支持时）；Responses 透传不剥离 | `model_supports_reasoning` / `strip_reasoning_in_place` 单元测试 |
| 3+6 | 日志不含描述正文（仅元数据）；无 panic | 现有 VL 日志测试 |
| 4 | 并发、Tier1 URL 命中、Tier2 重发入口（新问题新调用/重复问题命中）、批量重试、坏图隔离、总超时、context_window strip | `apply_vl_with_fallback` / `analyze_images_with_vl` 现有测试 |
| 5 | 裸域名加 `/v1`；完整 endpoint 不重复 | 现有 url helper 测试 |

**全套件回归**：`cargo test --workspace --no-fail-fast` + 前端 `npx tsx --test src/model-windows.test.ts` + `npx tsc --noEmit`。

---

## 六、风险与回滚

| 风险 | 缓解 |
|------|------|
| Bug 1 撤回 no_proxy 后测试 502 复现 | `NO_PROXY=127.0.0.1,localhost` env 覆盖；若仍复现，单独给测试 client 加 no_proxy（不动生产） |
| Bug 2 reasoning 剥离误伤支持 reasoning 的模型 | `model_supports_reasoning` 默认返回 `true`（未命中 map 时），保守不误伤 |
| Bug 4 抽模块引入回归 | 逐函数迁移 + 每步跑现有 VL 测试；`vision.rs` 公开 API 与原内联函数签名一致 |
| Bug 4 缓存无界增长 | 500 条上限 + 24h TTL + 写入时淘汰最旧 |
| Bug 4 重试拖慢请求 | 120s 总超时硬截断兜底 |
| Bug 4 两层检测复杂度 | 先单测最新/历史分类逻辑；若检测过复杂，可回退单层 (URL,Q)（接受多调用） |

**回滚**：每个 bug 独立 commit，可单独 revert。Bug 4 抽模块单独 commit，回滚不影响前 5 个。

---

## 七、与 PR #1405 的关系

PR #1405（VLM vision analysis）是同类方案，本设计借鉴其：独立 `vlm_http_client()` 不碰 `proxied_client()`（Bug 1）、`upstream_request_parts` 接入 strip 模式（Bug 2）、`vision.rs` 独立模块 + 并发批次 + 缓存 + 总超时 + 重试（Bug 4）。

本设计与 PR #1405 的差异：
- 缓存 key 用 `DefaultHasher`（u64，std）而非 SHA256。`sha2` 已是项目依赖（PR #1405 也未加运行依赖，仅加 `wiremock` 测试依赖），故非"省依赖"优势；DefaultHasher 更简单更快，缓存 key 不需密码学强度。
- 缓存采用两层（历史图 URL 缓存 + 最新图 (URL,问题) 缓存），提供重发入口；PR #1405 用 SHA256 URL 缓存 + 后台静默补齐（两阶段）。
- VL prompt 用"全面+侧重"（保留问题引导）；PR #1405 用纯描述（不引用用户问题）。
- 不做 golden window / 两阶段同步+后台分析 / 上下文溢出动态保护。
- 重试用混合策略（批量 2 次 + 拆单张 3 次）；PR #1405 仅 per-batch 重试。
- Bug 5（endpoint 裸拼）PR #1405 有同样问题，本设计修复后可反向提醒。
