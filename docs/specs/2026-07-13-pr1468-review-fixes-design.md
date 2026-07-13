# PR #1468 Review 修复设计

> 日期：2026-07-13
> 关联 PR：https://github.com/BigPizzaV3/CodexPlusPlus/pull/1468
> 关联评审意见：`#issuecomment-4955348317`（reviewer: BigPizzaV3）
> 参考 PR：https://github.com/BigPizzaV3/CodexPlusPlus/pull/1405（VLM vision analysis，同类方案）
> 分支：`strip-images-feature` -> `main`

---

## 一、背景与目标

PR #1468 实现"纯文本模型图片处理"（per-model 能力判断、VL 视觉模型中转、Reasoning 剥离）。reviewer 提了 6 条意见，其中前 3 条为阻塞项（修复后才能合并），后 3 条为改进建议。本设计覆盖全部 6 条修复。

**目标**：解决全部 6 条 review 意见，使 PR 达到可合并状态。

**非目标**：不重构 PR #1468 的整体架构（Chat 走代理 / Responses 纯透传的设计保持不变）；不引入 PR #1405 的 golden window / 两阶段分析等高级特性。

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

**Chat 路径图片处理的三种情况**（VL 与 strip 是替代关系，非顺序）：
- 模型支持图片 -> 保留（转 Chat `image_url`），不 strip、不 VL
- 模型不支持图片 + VL 启用 -> VL 描述成文字替换图片，不 strip
- 模型不支持图片 + VL 未启用 / VL 失败 -> strip 丢弃图片，不报错

**关键代码位置**：
- `apply_vl_with_fallback`（`protocol_proxy.rs:1163`，在 `:536` 被调）：VL 预处理入口，返回 `(supports_image, body)`
- `upstream_request_parts`（`protocol_proxy.rs:758`，在 `:544` 被调）：协议转换边界，Chat 分支做 strip + 转换，Responses 分支纯透传
- `responses_to_chat_completions_with_image_support`（`:149`）：Chat 转换，`supports_image=false` 时丢弃 `input_image`

---

## 三、6 个 Bug 的修复方案

### Bug 1（阻塞）：`.no_proxy()` 污染共享 client

**位置**：`crates/codex-plus-core/src/http_client.rs:8-12`

**根因**：PR #1468 给共享工厂 `proxied_client()` 加了 `.no_proxy()`。reqwest 的 `.no_proxy()` 是**彻底关闭代理解析**（忽略 `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`），不是"只绕过 127.0.0.1"。`proxied_client()` 被 13 处共用（广告、更新、模型列表、插件市场、供应商测试、真实上游请求等），所有靠系统代理上网的用户其真实上游请求被断。注释"公网域名不受 no_proxy 影响"错误。

**历史**：`handoff` 文档阶段 5 记录，当时是为修测试 502（macOS 系统代理拦截 127.0.0.1 mock server 请求），但用了"核弹级"方案。

**修复**：
1. 撤回 `proxied_client()` 的 `.no_proxy()`，恢复成 `main` 的样子：
   ```rust
   Ok(reqwest::Client::builder().user_agent(ua).build()?)
   ```
2. 测试环境的 127.0.0.1 拦截问题用 `NO_PROXY=127.0.0.1,localhost` 环境变量解决（reqwest 会读 `NO_PROXY`，只对 localhost 绕过代理，公网域名照常走系统代理）。在 `tests/protocol_proxy.rs` 测试入口（`#[ctor]` 或首个测试 setup）设置该 env。

**测试**：
- 验证 `proxied_client()` 构建的 client 不再强制 no_proxy（可通过检查 builder 行为或集成测试：有系统代理时公网域名走代理、127.0.0.1 走直连）。
- 验证 6 个原 pre-existing 测试（`chat_completions_proxy_*` / `aggregate_*` / `responses_proxy_*`）在 `NO_PROXY` env 下仍通过。

---

### Bug 2（阻塞）：reasoning 剥离是死代码

**位置**：
- 定义：`protocol_proxy.rs:857`（`model_supports_reasoning`）、`:888`（`strip_reasoning_in_place`）
- 调用点：仅 `tests/protocol_proxy.rs:1155-1218`，生产零调用

**根因**：图片剥离已接入生产（`apply_vl_with_fallback` -> `model_supports_image` -> `responses_to_chat_completions_with_image_support`），但 reasoning 这对函数只写了测试，转发链路从不调用。UI 勾选"不支持 reasoning"后配置能存（reviewer 已确认存配置无问题），但请求照带 `reasoning` 发上游。

**为什么放 `upstream_request_parts` 而非 `apply_vl_with_fallback`**：
- `apply_vl_with_fallback` 在 `:1173-1174` 有提前返回：`if base_supports_image || !vision_relay.enabled { return }`。VL 未启用时（多数用户的常见情况）直接返回，放在这里的 reasoning 剥离不会执行。reasoning 剥离必须与 VL 无关地独立运行。
- `upstream_request_parts` 是每个走代理请求的必经之路，不受 VL 开关影响；图片剥离就在其 Chat 分支（`:775`）；`match` 的两个分支天然对应"Responses 不剥离 / Chat 剥离"的决策。

**修复**：在 `upstream_request_parts` 的 Chat 分支内、转换之前剥离 reasoning：
```rust
RelayProtocol::ChatCompletions => {
    let mut body = request_json;
    let model = body.get("model").and_then(Value::as_str).unwrap_or("").to_string();
    let supports_reasoning = model_supports_reasoning(relay, &model);
    strip_reasoning_in_place(&mut body, supports_reasoning);  // 转换前剥离
    Ok((chat_completions_url(&relay.base_url),
        responses_to_chat_completions_with_image_support(body, supports_image)?,
        UpstreamWireApi::ChatCompletions))
}
```

**为什么转换前剥离**：`reasoning` 是 Responses 顶层字段；转换函数内有 `apply_chat_reasoning_options`（`:203`）会读 `reasoning` 套到 Chat 结果。必须先删顶层 `reasoning`，否则转换会把它带过去。

**`supports_reasoning` 内部现算**：不耦合进 `apply_vl_with_fallback`（reasoning 与 VL 无关）。`upstream_request_parts` 已有 `relay`，从 body 读 `model` 即可。

**死代码清理**：`strip_input_images_in_place`（`:865`）也是死代码（仅测试调用）。它当初为"Responses 透传 strip 图片"而写，但当前 Responses 分支纯透传不调它。按"Responses 不剥离图片"决策，它无生产用途，**清除**（连同其测试）。清除安全：当前是死的，删除不改变生产行为。

**测试**：
- 集成测试：Chat 路径，模型不支持 reasoning 时，转发上游的 body 不含 `reasoning` 字段。
- 集成测试：Chat 路径，模型支持 reasoning 时，`reasoning` 字段保留。
- 集成测试：Responses 路径，`reasoning` 字段原样透传（不剥离）。
- 回归：现有 `model_supports_reasoning` / `strip_reasoning_in_place` 单元测试仍通过。

---

### Bug 3（阻塞）：UTF-8 字节截断 panic

**位置**：`protocol_proxy.rs:1131`

```rust
"description_preview": &description[..description.len().min(200)]
```

**根因**：`[..N]` 是字节切片。中文 UTF-8 占 3 字节，200 字节极可能落在汉字中间 -> `panic: byte index 200 is not a char boundary`。VL 描述以中文为主，几乎必现。

**修复**：改为 Unicode 字符截断，永不 panic：
```rust
"description_preview": description.chars().take(200).collect::<String>()
```
语义从"前 200 字节"变"前 200 字符"，对调试预览完全够用。

**测试**：新增用例，构造 201+ 中文字符的 VL 描述，验证不 panic 且预览被正确截断为 200 字符。

> 注：此行同时涉及 Bug 6（日志泄露正文），两 bug 同一行，一起改。

---

### Bug 4（改进）：VL 串行 + 无缓存 + 无并发 + 无重试

**位置**：
- 常量：`protocol_proxy.rs:13`（`VL_IMAGE_LIMIT=10`）、`:15`（`VL_SINGLE_TIMEOUT=30s`）
- 串行循环：`:1140-1165`，`for &idx in &window_indices { ... describe_image_with_vl(...).await? }`

**根因**：10 张图 × 30s = 最坏 300s；failover 重跑、后续轮次重发历史图片都重复调 VL，已成功的调用费用浪费。

**修复**：抽 `vision.rs` 模块，重度参考 PR #1405 的 `vision.rs` 但用零新依赖方案。

#### 4.1 模块抽取

将 VL 相关逻辑从 `protocol_proxy.rs` 抽到 `crates/codex-plus-core/src/vision.rs`：
- `describe_image_with_vl` / `analyze_images_with_vl` / `apply_vl_with_fallback` / `items_within_vl_window` / `collect_input_text` / `estimate_item_tokens` / VL 常量
- `protocol_proxy.rs` 保留 `upstream_request_parts` 对 `apply_vl_with_fallback` 的调用（`:536`）
- 跨模块访问的 helper（`chat_completions_url` / `responses_url` / `diagnostic_log`）已是 `pub`

**必要性**：`protocol_proxy.rs` 已 3000+ 行，Bug 4 重构（缓存全局状态、并发调度、重试）再加几百行会严重污染协议转换文件的职责。抽模块后 VL 逻辑可独立单测。

#### 4.2 并发控制

- `tokio::sync::Semaphore(5)`：限同时在飞的 VL 调用最多 5 个。
- `tokio::task::JoinSet`：并发跑一批任务 + 收集结果。
- 两者分工：Semaphore 管"最多几个并发"，JoinSet 管"怎么并发跑 + 收结果"。均为 tokio 自带，**零新依赖**。

#### 4.3 批次（多图一调用）

- `BATCH_SIZE = 5`：5 张图塞进一次 VL API 调用（一个 `messages` 放 5 个 `image_url` part + 1 个文本 prompt）。
- 新增 `call_vlm_batch(urls, config) -> Result<Vec<String>>`：一次调多图，返回每张图的描述。
- 5 张是效率 vs 风险的平衡点（PR #1405 验证过的经验值），保守安全。

#### 4.4 缓存

- key：图片 URL 经 `std::collections::hash_map::DefaultHasher` 哈希为 `u64`（std 自带，无新依赖；500 条缓存碰撞概率 ~10⁻¹⁴，可忽略）。
- value：`(String 描述, Instant 写入时刻)`。
- 结构：`LazyLock<Mutex<HashMap<u64, (String, Instant)>>>`（全 std）。
- 容量上限：500 条。满时淘汰最旧（按 `Instant` 排序删最旧）。
- TTL：24 小时，过期条目在写入时顺带清理。
- 作用：failover 重跑 / 下一轮重发同一张图时命中缓存，不重复调 VL，不浪费费用。

**澄清**：缓存容量（条数，500）与 `VisionRelayConfig.context_window`（token 预算，控制哪些历史图片参与 VL）是**两个不同概念**，互不相关。

#### 4.5 超时（双层）

- **per-batch 超时**：`基础 15s + 每张 8s`。1 张 = 23s，5 张 = 55s。随批量线性增长。
- **单张超时**（拆单张阶段用）：`15 + 8 = 23s`。
- **总超时**：`120s` 硬截断（`tokio::time::timeout(120s, 整个 analyze_all)`）。到点停，已完成用，未完成降级 strip。

#### 4.6 重试（混合策略：批量 + 拆单张）

两阶段，专治不同故障类型：

**第 1 阶段：批量重试（治瞬时故障）**
- 一批 5 张，最多 2 次尝试（1 + 1 重试），退避 0.3s + 抖动 ±20%。
- 瞬时故障（网络抖动、429、5xx）重试整批即可全过，享批量效率。

**第 2 阶段：拆单张（隔离坏图）**
- 批量 2 次仍失败 -> 拆成 5 个单张调用并发跑，每张各自 3 次尝试，退避 0.3s / 0.6s + 抖动。
- 好图各自 1 次成功；坏图自己重试自己的，最后只丢坏图，不拖累好图。
- 持久故障（坏图：损坏 / 格式不支持）在这里被隔离。

**为什么 VL API 整批失败**：VL API 一次收 5 张图，任何一张坏图会让整个请求返回错误，API 不会"部分成功"。所以批量阶段拿不到"3 成 2 败"的结果，必须拆单张才能隔离。

**总超时兜底**：所有重试受 120s 总超时硬截断，到点即停。

**时间线示例**（5 张含 1 张坏图 img3）：
```
t=0s     批量尝试1: 5张 -> 失败(img3坏)
t=0.3s   批量尝试2: 5张 -> 失败
t=0.6s   拆单张, 并发5个:
           img1: 成功 ✅  img2: 成功 ✅  img4: 成功 ✅  img5: 成功 ✅
           img3: 失败->重试->失败->重试->失败 -> strip ❌ (只丢img3)
结果: img1/2/4/5 描述成功, 只 img3 被丢 ✅
```

#### 4.7 上下文溢出保护

- **不做完整溢出保护**（预估 token 总量 + 预裁剪）。
- 理由：主模型服务端 tokenizer 自动把 VL 描述（文字）计入上下文窗口，正常情况主模型自管；`VisionRelayConfig.max_tokens` 已从源头限 VL 描述长度；溢出是边缘场景（很多图 + 超长历史 + 小上下文模型），多数用户上下文 128k/1M。
- **廉价兜底**：VL 描述文本 > 2000 字符时截断（防单条描述失控）。

**测试**：
- 并发：5+ 张图，验证并发执行（总耗时 << 串行）。
- 缓存命中：同一 URL 二次处理不重复调 VL（mock VL API 计数）。
- 批量重试：VL API 前 1 次失败、第 2 次成功，验证整批重试成功。
- 坏图隔离：1 张坏图 + 4 张好图，验证好图描述成功、坏图 strip。
- 总超时：mock VL API 永久挂起，验证 120s 后降级 strip 不卡死。
- 回归：现有 VL 测试（`apply_vl_with_fallback` / `analyze_images_with_vl`）在抽模块后仍通过。

---

### Bug 5（改进）：VL endpoint 裸拼路径

**位置**：`protocol_proxy.rs:1039-1042`

```rust
ChatCompletions => format!("{}/chat/completions", vl_config.base_url.trim_end_matches('/')),
Responses       => format!("{}/responses",       vl_config.base_url.trim_end_matches('/')),
```

**根因**：本文件 `:1257` 已有 `chat_completions_url()`、`:1277` 已有 `responses_url()`，处理了 `/v1` 补全、`/v1/v1` 去重、`#` 跳过、origin-only 判断。裸拼会：裸域名缺 `/v1`；完整 endpoint 重复拼路径。

**修复**：复用现有 helper：
```rust
ChatCompletions => chat_completions_url(&vl_config.base_url),
Responses       => responses_url(&vl_config.base_url),
```

**测试**：裸域名（`https://api.x.com`）-> 自动加 `/v1`；完整 endpoint（`https://api.x.com/v1/chat/completions`）-> 不重复。

> 注：PR #1405 有相同 bug，本修复可反向提醒。

---

### Bug 6（改进）：日志泄露图片描述正文

**位置**：`protocol_proxy.rs:1131`（与 Bug 3 同一行）

**根因**：`description_preview` 记录 VL 描述正文前 200 字节，可能含截图中的密钥、PII、代码等敏感内容，写进 `diagnostic_log` 落盘。

**修复**：只记元数据，去掉正文：
```rust
"description_len": description.len(),
"description_chars": description.chars().count(),
```

**测试**：验证 `vl_described` 日志不含描述正文，只含长度字段。

---

## 四、实施顺序

全部 6 个在本批次改完。按依赖与风险排序：

1. **Bug 5、6**（trivial，`protocol_proxy.rs:1031-1042` 与 `:1131` 同一区域，顺路一起改）
2. **Bug 1、2、3**（3 个阻塞项，中等改动）
3. **Bug 4**（抽 `vision.rs` + 并发/缓存/超时/重试，最大改动，单独 commit）

每个 bug 遵循 TDD：RED（写失败测试）-> GREEN（最小实现）-> 回归（跑全套件）。

---

## 五、测试计划

| Bug | 新增测试 | 回归测试 |
|-----|---------|---------|
| 1 | `proxied_client()` 不强制 no_proxy；`NO_PROXY` env 下 6 个原测试通过 | 现有 http_client 相关测试 |
| 2 | Chat 路径剥离 reasoning（不支持时）/ 保留（支持时）；Responses 透传不剥离 | `model_supports_reasoning` / `strip_reasoning_in_place` 单元测试 |
| 3 | 201+ 中文字符描述不 panic、截断为 200 字符 | 现有 VL 日志测试 |
| 4 | 并发、缓存命中、批量重试、坏图隔离、总超时 | `apply_vl_with_fallback` / `analyze_images_with_vl` 现有测试 |
| 5 | 裸域名加 `/v1`；完整 endpoint 不重复 | 现有 url helper 测试 |
| 6 | 日志不含描述正文 | 现有 VL 日志测试 |

**全套件回归**：`cargo test --workspace --no-fail-fast` + 前端 `npx tsx --test src/model-windows.test.ts` + `npx tsc --noEmit`。

---

## 六、风险与回滚

| 风险 | 缓解 |
|------|------|
| Bug 1 撤回 no_proxy 后测试 502 复现 | `NO_PROXY=127.0.0.1,localhost` env 已覆盖；若仍复现，单独给测试 client 加 no_proxy（不动生产 `proxied_client()`） |
| Bug 2 reasoning 剥离误伤支持 reasoning 的模型 | `model_supports_reasoning` 默认返回 `true`（未命中 map 时），保守不误伤 |
| Bug 4 抽模块引入回归 | 逐函数迁移 + 每步跑现有 VL 测试；`vision.rs` 公开 API 与原内联函数签名一致 |
| Bug 4 缓存无界增长 | 500 条上限 + 24h TTL + 写入时淘汰最旧 |
| Bug 4 重试拖慢请求 | 120s 总超时硬截断兜底 |

**回滚**：每个 bug 独立 commit，可单独 revert。Bug 4 抽模块单独 commit，回滚不影响前 5 个。

---

## 七、与 PR #1405 的关系

PR #1405（VLM vision analysis）是同类方案，本设计借鉴其：
- 独立 `vlm_http_client()` 不碰 `proxied_client()`（Bug 1 思路）
- `upstream_request_parts` 接入 strip 的模式（Bug 2 思路）
- `vision.rs` 独立模块 + 并发批次 + 缓存 + 总超时 + 重试（Bug 4 思路）

本设计与 PR #1405 的差异：
- 缓存 key 用 `DefaultHasher`（u64，std）而非 SHA256（无 `sha2` 依赖）
- 不做 golden window / 两阶段同步+后台分析 / 上下文溢出动态保护
- 重试用混合策略（批量 2 次 + 拆单张 3 次），PR #1405 仅 per-batch 重试
- Bug 5（endpoint 裸拼）PR #1405 有同样问题，本设计修复后可反向提醒

PR #1405 也有 Bug 5 相同的 endpoint 裸拼问题，不作为借鉴来源。
