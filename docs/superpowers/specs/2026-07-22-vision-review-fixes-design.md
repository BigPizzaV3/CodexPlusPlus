# PR #1550 Review 修复设计

> 日期：2026-07-22
> 分支：vision-enhancement
> 背景：PR #1550 收到仓库管理员 Request changes（[issuecomment-5037045840](https://github.com/BigPizzaV3/CodexPlusPlus/pull/1550#issuecomment-5037045840)），四项问题需修复后方可合并
> 原始设计：`docs/superpowers/specs/2026-07-16-vision-enhancement-design.md`
> 基线对比：`main` 分支 vision.rs（PR #1405 落地版）

---

## 一、管理员意见与回应

### 意见 (1)：vision.rs Phase 2 后台分析被实际禁用

**管理员指出**：`bg_config_opt` 固定为 None，结尾只做 `let _ = bg_config_opt`。原实现会收集黄金窗口外/深层未缓存 URL 并 `tokio::spawn` 写缓存。请恢复实现或明确移除，不能保留永远不可达的占位代码。

**核实**：属实。当前 `bg_config_opt = None`（vision.rs:1171），main 有完整 `background_analyze_and_cache` + `tokio::spawn`。执行日志已记录"Task 5 重构时 gutted，留后续 PR"。

**回应**：恢复 Phase 2 后台分析。深层历史图（11~50 轮）需要后台补分析写入缓存，否则这些图的描述永远缺失。不采用"移除"方案，因为该能力对多轮历史图片对话有实际价值。

### 意见 (2)：BATCH_SIZE = 5 未被使用

**管理员指出**：当前轮 `round_urls` 不限量直接传给 `call_vlm_batch`，多图一次性发送给 VLM 而非按 5 张分批，会触发 provider 图片数/请求体限制，单批解析失败影响整轮。请恢复 chunking 并补测试。

**核实**：属实。当前 `call_vlm_batch` 整批发送，无 chunk；main 的 `analyze_all` 用 `chunks(BATCH_SIZE)` 分批。`BATCH_SIZE` 定义但未引用。

**回应**：恢复分批。在 `call_vlm_batch` 内部按 5 张分批，每批一次 API 调用（VLM 在批内同时看到 5 张图），各自重试，合并描述。所有调用点自动受益。补"超过 5 张触发多次调用"和"逐图描述对应"测试。

### 意见 (3)：缓存从 SHA-256 改成 u64，碰撞风险

**管理员指出**：Map 中不再保留原始 URL/question 用于相等性校验，64 位 hash 碰撞会把另一张图的描述注入当前会话。请使用完整结构键（URL + question，HashMap 自己处理碰撞），或保留强摘要并校验原 key。

**核实**：属实。当前 `HashMap<u64>`（DefaultHasher），main 为 `HashMap<String>`（SHA256）。

**回应**：采用管理员的第一个建议--完整结构键。详见第二节 R1。

### 意见 (4)：function_call_output 图片绕过 Strip/VLM

**管理员指出**：纯文本模型核心路径，尤其 view_image 工具返回图片时会继续透传。至少需明确覆盖范围和回归测试；若暂不支持应拆 PR。

**核实**：属实，且是重大缺陷。所有 strip 函数只扫描 `content` 的 array 形式，不扫描 tool 消息 `output`/`content` 字符串里的 base64 data URL。data URL 以文本原样透传给纯文本模型，浪费巨量 token + 模型看不懂 + 可能死循环。CHANGELOG 已记录此缺陷。

**回应**：不拆 PR，本 PR 直接修复。此缺陷不补齐，视觉模块无法达到上线标准。新增 data URL 识别 + VLM 分析/Strip 替换，覆盖 Strip 和 Vlm 两个模式。详见第二节 R4。

---

## 二、修复方案

### R1. 缓存结构键

**问题**：缓存 key 是 DefaultHasher 的 u64，两个不同 (URL, question) 若 hash 碰撞，HashMap 比较的是 u64 会误判相等，注入错误描述。

**为什么不用 SHA256**：管理员给了两个选项--(a) 完整结构键，(b) 强摘要 + 校验原 key。选项 (b) 等于"先算 SHA256 再存原始值校验"，但既然最终要靠原始值防碰撞，直接用原始值当 key 即可，多算一层 SHA256 是多此一举。具体对比：

| 维度 | 结构键 (a) | SHA256+校验 (b) |
|---|---|---|
| 碰撞防护 | 彻底消除（HashMap Eq 比较原始值） | 降低概率（2^256，极低但理论存在） |
| 依赖 | 无（HashMap 自带 Hash+Eq） | 需要 sha2 |
| 改动量 | 改 url_hash 返回类型 + 调用点适配 | 同左 + CacheEntry 存原始值 + get 时校验逻辑 |
| key 长度 | 完整 URL（可能较长） | 64 字符 hex |
| key 长度影响 | 缓存仅 500 条，无影响 | 无影响 |

结构键改动量不比 SHA256 大，但无依赖、彻底防碰撞。SHA256 增加依赖和 review 成本，换来的"key 更短"在 500 条缓存面前毫无意义。

**方法**：用 `CacheKey` enum 替代 u64，包含 `Url(String)`（Tier1，question 为空）和 `UrlQuestion(String, String)`（Tier2，有 question）两个变体。HashMap 对 CacheKey 自行算 Hash 定位桶，桶内碰撞时用 `Eq` 比较原始字符串，不同就不命中。无需手动 hash，无需 sha2，无需额外校验代码。

**最终效果**：不同 (URL, question) 绝不误命中；相同则命中。Tier1（历史轮只看 URL）和 Tier2（当前轮 URL+问题）互不串扰。两层缓存语义不变。

### R2. BATCH_SIZE 分批

**问题**：多图整批发送给 VLM，触发 provider 限制，单批失败影响整轮。

**方法**：`call_vlm_batch` 内部按 `BATCH_SIZE`（=5）分批。每批一次 `call_vlm_batch_once` 调用，VLM 在批内同时看到该批图片并返回逐图描述（`[[图片K]]` 标记），各自重试，合并所有描述。12 张图 = 3 批（5+5+2）= 3 次 API 调用。

**分批不影响主模型整体理解**：分批发生在 VLM 调用层，所有描述合并后注入 messages，主模型一次性看到全部文字描述、只回复一次。批内 VLM 能综合该批图片；跨批虽分开调用，但主模型拿到所有描述仍能整体理解。

**最终效果**：规避 provider 图片数/请求体限制；单批 API 调用范围隔离；所有调用点（当前轮、历史轮、后台分析）自动受益。

### R3. Phase 2 后台分析恢复

**问题**：`bg_config_opt` 固定 None，深层历史图（11~50 轮）永远不会被分析、永远不进缓存。

**根因**：非 spec 冲突。spec 第八章"不引入新后台机制"指不在 #1405 Phase 2 之外加新东西，不是移除 Phase 2。未保留的原因是 Task 5 重构两层缓存时，后台函数的 key 类型适配未完成。

**方法**：从 main 迁移收集逻辑（`x_budget > 10` 时收集黄金窗口外 + 深层未缓存 URL）+ 后台分析函数。后台分析用 Tier1 key（`CacheKey::Url`）+ TIER1_PROMPT（无用户问题上下文），调 `call_vlm_batch`（带分批），写入缓存。`tokio::spawn` 异步执行，不阻塞当前请求。后台失败静默跳过。

**最终效果**：深层历史图在后台被分析写入缓存，后续请求翻看深层历史时直接命中缓存注入描述，无需等待 VLM。当前请求不受影响、立即返回。

### R4. function_call_output 图片处理

**问题**：view_image 等工具返回 base64 data URL，存于 function_call_output 的 `output` 字段（Responses）或 tool 消息的 `content` 字符串（Chat）。所有 strip 函数只检查 `content` 的 array 形式，不扫描字符串里的 data URL，导致 base64 原样透传给纯文本模型。后果：浪费巨量 token + 模型看不懂 + 可能死循环。

**触发时点**：与现有 VLM 路由完全相同的时点--每次 codex 向上游发请求时经过 `upstream_request_parts`。proxy 不区分"用户发消息"还是"工具调用后继续"，只看请求里的 messages/input 数组。工具返回 base64 后，codex 下一次发请求时，proxy 拦截并处理这些 data URL。不需要新的触发机制。

**base64 格式说明**：view_image 工具读取本地图片返回 `data:image/png;base64,...`，这是 OpenAI API 支持的标准格式。`image_url` content part 的 `url` 字段同时支持 http URL 和 data URL，API 层解析 data URL 转成模型格式，VLM 不直接面对 base64 文本。测试入口 `test_vlm_once` 已用 data URL 验证通过。data URL 与 http URL 对 VLM 处理方式完全相同。

**方法**：
1. **识别**：手动解析 data URL 模式（`data:image/...;base64,...`），提取 tool/function_call_output 消息 `output`/`content` 字符串里的 data URL。不引入 regex 依赖（项目无 regex），用前缀匹配 + base64 字符集扫描。
2. **Vlm 模式**：提取 data URL，调 `call_vlm_batch`（TIER1_PROMPT + Tier1 key + 分批），VLM 返回描述后替换回字符串（`[图片描述] {desc}`），base64 本体被删除。VLM 失败时 fail-open 替换为 `[图片描述失败]`（不保留 base64，避免浪费 token）。
3. **Strip 模式**：data URL 替换为 `[图片已省略]`。
4. **SendAsIs 模式**：不动（多模态模型能处理）。
5. **协议适配**：Responses 检查 `output` 字段，Chat 检查 `content` 字符串。
6. **接入点**：protocol_proxy.rs 的 Strip 分支（`strip_images_only_counted` 之后调同步替换）和 Vlm 分支（`strip_image_blocks` 之后调异步 VLM 分析）。
7. **上下文溢出保护**：tool 图片描述同样计入 token 预算，溢出时 fail-open 替换为占位符。

**最终效果**：工具调用的 base64 图片不再透传给主模型，而是被 VLM 转成文字描述（或 Strip 替换为占位符）。主模型上下文只含文字描述，不含 base64，大幅节省 token 并让模型真正"看到"图片内容。死循环问题消除。

---

## 三、测试计划

| 修复 | 测试要点 |
|---|---|
| R1 | 不同 URL/question 不误命中；同 key 命中；Tier1/Tier2 互不串扰 |
| R2 | >5 张触发多次 VLM 调用（wiremock 计数）；逐图描述与 URL 顺序对应；单批重试 |
| R3 | x_budget>10 时后台分析触发、未缓存 URL 写入缓存；x_budget<=10 不触发；后台失败静默跳过 |
| R4 | tool 消息含 data URL -> Vlm 模式 VLM 分析 + 描述替换、base64 删除；Strip 模式 -> 占位符替换；VLM 失败 -> fail-open；SendAsIs -> 不动；无 data URL -> 不影响原文本；Responses/Chat 两协议 |

**回归**：`cargo test --workspace --no-fail-fast` + `npx tsc --noEmit` + binary build。沿用上游 `#[test]` + tempfile + wiremock 风格。

---

## 四、实施顺序

1. **R1 缓存结构键** -- 基础改动，R2/R3/R4 均依赖 CacheKey 类型
2. **R2 BATCH_SIZE 分批** -- call_vlm_batch 内部分批
3. **R3 Phase 2 恢复** -- 依赖 R1 的 CacheKey + R2 的分批
4. **R4 function_call_output 图片处理** -- 依赖 R1 的 CacheKey + R2 的分批
5. **全量回归测试**

每步独立 commit，可单独 revert。具体代码实现由 plan 文档细化。

---

## 五、不在本次范围

- **function_call_output 图片带上文发给 VLM**（用户信息+工具调用+VLM结果+主模型消息+图片整体发给 VLM）-- 本 PR 仅做 data URL 识别 + VLM 分析替换，不带对话上下文。带上文分析留后续 PR。
- **tool-call 架构**（模型自主决定调 VL）-- 留未来。
- **SendAsIs 模式 base64 透传优化** -- 多模态模型能处理图片，非阻塞。
