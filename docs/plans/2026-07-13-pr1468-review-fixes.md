# PR #1468 Review 修复实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 解决 PR #1468 reviewer 提出的 6 条意见（3 阻塞 + 3 改进），使 PR 可合并，并把 VL 中转升级为生产可用（并发+两层缓存+重试+入口）。

**架构：** Chat 路径走代理做 strip/VL/reasoning 剥离；Responses 纯透传不干预。VL 逻辑抽到独立 `vision.rs` 模块，两层缓存（历史图 URL 缓存 + 最新图 (URL,问题) 缓存），proxy 驱动的重发入口（架构 A+，非 tool-call）。

**技术栈：** Rust + tokio（async）+ reqwest + serde_json。测试用 `tokio::net::TcpListener` 自建 mock VL server（项目无 wiremock）。缓存用 `std::collections::hash_map::DefaultHasher` + `LazyLock<Mutex<HashMap>>`。零新依赖。

**规格：** `docs/specs/2026-07-13-pr1468-review-fixes-design.md`

---

## 文件结构

| 文件 | 职责 | 改动 |
|------|------|------|
| `crates/codex-plus-core/src/http_client.rs` | 共享 HTTP client 工厂 | Bug 1：撤回 `.no_proxy()` |
| `crates/codex-plus-core/src/protocol_proxy.rs` | 协议转换 + 转发 | Bug 2：reasoning 剥离；Bug 5：endpoint；Bug 6：日志；Bug 4：移出 VL 代码到 vision.rs |
| `crates/codex-plus-core/src/vision.rs`（新） | VL 中转全部逻辑 | Bug 4：抽模块 + 两层缓存 + 并发 + 批次 + 重试 + 超时 + 两 prompt |
| `crates/codex-plus-core/src/lib.rs` | 模块注册 | 注册 `vision` 模块 |
| `crates/codex-plus-core/tests/protocol_proxy.rs` | 集成测试 | 每个 bug 的测试 + `NO_PROXY` env setup + 更新受影响 VL 测试 |

**设计边界**：`vision.rs` 对外暴露 `apply_vl_with_fallback`（protocol_proxy.rs:536 调用）+ 必要类型；内部封装缓存/并发/重试。`protocol_proxy.rs` 只保留协议转换 + 调用 `vision::apply_vl_with_fallback`。

---

## 测试基础设施（现有，复用）

- `mock_vl_server(listener, response_body, captured)`（`tests/protocol_proxy.rs:1311`）：tokio TCP mock，回写指定 response，捕获请求体。
- `vl_response(description)`（`:1367`）：构造 VL 响应 JSON。
- VL 测试模式：`TcpListener::bind(("127.0.0.1", 0))` -> spawn `mock_vl_server` -> `analyze_images_with_vl`/`apply_vl_with_fallback` 指向 mock -> 断言。
- **注意**：mock 在 127.0.0.1，需 `NO_PROXY=127.0.0.1`（Bug 1 修复后）。

---

## Phase 1：Bug 5 + Bug 6（+Bug 3）— trivial，同区域

### 任务 1：Bug 5 — VL endpoint 复用 url helper

**文件：**
- 修改：`crates/codex-plus-core/src/protocol_proxy.rs:1039-1042`
- 测试：`crates/codex-plus-core/tests/protocol_proxy.rs`（新增 endpoint 规范化测试）

- [ ] **步骤 1：编写失败的测试**

在 `tests/protocol_proxy.rs` 末尾新增（复用 `mock_vl_server`，验证请求打到正确 path）：

```rust
#[tokio::test]
async fn vl_endpoint_normalizes_bare_domain_with_v1() {
    // 裸域名 base_url -> VL 请求应打到 /v1/chat/completions
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let cap2 = captured.clone();
    tokio::spawn(mock_vl_server(listener, vl_response("desc"), cap2));
    let mut body = json!({"input":[{"role":"user","content":[
        {"type":"input_text","text":"q"},
        {"type":"input_image","image_url":"data:image/png;base64,iVBOR"}
    ]}]});
    let mut config = default_vision_relay_config();
    config.enabled = true;
    config.protocol = RelayProtocol::ChatCompletions;
    config.base_url = format!("http://{addr}"); // 裸域名，无 /v1
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    analyze_images_with_vl(&mut body, &config, &client).await.unwrap();
    let req = String::from_utf8(captured.lock().unwrap()[0].clone()).unwrap();
    assert!(req.contains("POST /v1/chat/completions"), "bare domain should add /v1, got: {req}");
}

#[tokio::test]
async fn vl_endpoint_does_not_duplicate_path() {
    // 完整 endpoint base_url -> 不重复拼 /chat/completions
    // ... 同上，但 config.base_url = format!("http://{addr}/v1/chat/completions")
    // 断言 req.contains("POST /v1/chat/completions HTTP") 且不含 "//chat/completions/chat"
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test protocol_proxy vl_endpoint_ -- --nocapture`
预期：FAIL（当前裸拼，裸域名打成 `/chat/completions` 无 `/v1`）

- [ ] **步骤 3：修改实现**

`protocol_proxy.rs:1039-1042` 替换为：

```rust
let endpoint = match vl_config.protocol {
    crate::settings::RelayProtocol::ChatCompletions => chat_completions_url(&vl_config.base_url),
    crate::settings::RelayProtocol::Responses => responses_url(&vl_config.base_url),
};
```

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --test protocol_proxy vl_endpoint_`
预期：PASS

- [ ] **步骤 5：Commit**

```bash
git add crates/codex-plus-core/src/protocol_proxy.rs crates/codex-plus-core/tests/protocol_proxy.rs
git commit -m "fix(VL): endpoint 复用 chat_completions_url/responses_url，修裸拼路径 (#1468 Bug 5)"
```

---

### 任务 2：Bug 6 + Bug 3 — 删 description_preview，日志只记元数据

**文件：**
- 修改：`crates/codex-plus-core/src/protocol_proxy.rs:1126-1135`（`vl_described` 日志块）
- 测试：`crates/codex-plus-core/tests/protocol_proxy.rs`

- [ ] **步骤 1：编写失败的测试**

新增（用中文描述触发 panic 的场景，验证不 panic + 日志无正文）：

```rust
#[tokio::test]
async fn vl_log_does_not_panic_on_chinese_description_and_omits_body() {
    // VL 返回 201 个中文字符的描述 -> 旧代码字节截断 panic；新代码应不 panic 且日志无正文
    let chinese = "中".repeat(201);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(mock_vl_server(listener, vl_response(&chinese), Arc::new(Mutex::new(vec![]))));
    let mut body = json!({"input":[{"role":"user","content":[
        {"type":"input_text","text":"q"},
        {"type":"input_image","image_url":"data:image/png;base64,iVBOR"}
    ]}]});
    let mut config = default_vision_relay_config();
    config.enabled = true;
    config.base_url = format!("http://{addr}");
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    // 不应 panic：
    analyze_images_with_vl(&mut body, &config, &client).await.unwrap();
    // 日志不含描述正文（检查 diagnostic_log 输出无 "中" 字符串正文）：
    // 通过临时捕获 diagnostic_log 或读取日志文件验证仅含 description_len/description_chars
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test protocol_proxy vl_log_does_not_panic`
预期：FAIL（旧代码 `&description[..200]` 对中文 panic）

- [ ] **步骤 3：修改实现**

`protocol_proxy.rs:1131` 的 `description_preview` 行替换为元数据（删除 preview）：

```rust
let _ = crate::diagnostic_log::append_diagnostic_log(
    "protocol_proxy.vl_described",
    json!({
        "vlModel": vl_config.model,
        "image_url_len": img_url.len(),
        "image_url_is_data": img_url.starts_with("data:"),
        "description_len": description.len(),
        "description_chars": description.chars().count()
    }),
);
```

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test --test protocol_proxy vl_log_does_not_panic`
预期：PASS（无 panic，日志无正文）

- [ ] **步骤 5：回归 + Commit**

运行：`cargo test --test protocol_proxy`（全 VL 测试）
预期：PASS

```bash
git add crates/codex-plus-core/src/protocol_proxy.rs crates/codex-plus-core/tests/protocol_proxy.rs
git commit -m "fix(VL): 日志删 description_preview 只记元数据，修 UTF-8 截断 panic (#1468 Bug 3+6)"
```

---

### Phase 1 Review 关卡

- [ ] **自审**：Bug 5 改动是否只动了 endpoint 行？Bug 6 是否完全删除 preview（无残留字节截断）？
- [ ] **回归**：`cargo test --test protocol_proxy` 全绿。
- [ ] **Oracle 审查**（可选，参考 handoff 模式）：调 review 子代理审查 Phase 1 改动，确认无副作用。

---

## Phase 2：Bug 1 + Bug 2 — 阻塞项

### 任务 3：Bug 1 — 撤回 no_proxy + 测试 NO_PROXY env

**文件：**
- 修改：`crates/codex-plus-core/src/http_client.rs:7-13`
- 修改：`crates/codex-plus-core/tests/protocol_proxy.rs`（入口设 NO_PROXY env）

- [ ] **步骤 1：编写失败的测试**

新增（验证 `proxied_client()` 不再强制 no_proxy——通过行为：构建的 client 在有 `HTTP_PROXY` env 时会读它；此处验证撤回后构建不 panic 且 helper 可用）：

```rust
#[test]
fn proxied_client_builds_without_no_proxy() {
    // 撤回后应正常构建（生产恢复系统代理支持）
    let client = codex_plus_core::http_client::proxied_client("test").unwrap();
    assert!(client.get("https://example.com").build().is_ok());
}
```

并在 `tests/protocol_proxy.rs` 顶部加 `NO_PROXY` env setup（确保 mock 127.0.0.1 不被系统代理拦）：

```rust
// 文件顶部，use 之后：
fn init_no_proxy() {
    // reqwest 构建时读 env；测试打 127.0.0.1 mock，需绕过系统代理
    if std::env::var("NO_PROXY").is_err() {
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    }
}

// 用 std::sync::Once 保证只设一次：
static INIT: std::sync::Once = std::sync::Once::new();
// 在每个测试开头或用 #[ctor] 调 init_no_proxy：
// INIT.call_once(init_no_proxy);
```

> 注意：若测试用 `LazyLock` 缓存 client，需确保 env 在首次构建前设置。用 `Once` 在每个 VL/集成测试开头调用。

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test protocol_proxy proxied_client_builds`
预期：当前能过（no_proxy 也能构建），但 mock 测试可能因系统代理失败——先跑全 VL 测试确认基线：
`cargo test --test protocol_proxy analyze_images_with_vl_`
若 FAIL（502/连接代理），说明需 NO_PROXY env。

- [ ] **步骤 3：修改实现**

`http_client.rs` 撤回 `.no_proxy()`，恢复 `main` 样子：

```rust
pub fn proxied_client(user_agent: &str) -> anyhow::Result<reqwest::Client> {
    let ua = if user_agent.trim().is_empty() {
        format!("CodexPlusPlus/{}", env!("CARGO_PKG_VERSION"))
    } else {
        user_agent.trim().to_string()
    };
    Ok(reqwest::Client::builder().user_agent(ua).build()?)
}
```

并在测试入口确保 `INIT.call_once(init_no_proxy)` 被调用（在每个 `#[tokio::test]` 开头，或用测试 fixture）。

- [ ] **步骤 4：运行测试验证通过**

```bash
cargo test --test protocol_proxy
cargo test --lib http_client
```
预期：全绿（包括原 6 个 pre-existing `chat_completions_proxy_*`/`aggregate_*`/`responses_proxy_*`）

- [ ] **步骤 5：Commit**

```bash
git add crates/codex-plus-core/src/http_client.rs crates/codex-plus-core/tests/protocol_proxy.rs
git commit -m "fix(http): 撤回 proxied_client 的 no_proxy，测试用 NO_PROXY env 绕 localhost (#1468 Bug 1)"
```

---

### 任务 4：Bug 2 — Chat 路径接入 reasoning 剥离 + 清死代码

**文件：**
- 修改：`crates/codex-plus-core/src/protocol_proxy.rs`（`upstream_request_parts` Chat 分支；删 `strip_input_images_in_place`）
- 测试：`crates/codex-plus-core/tests/protocol_proxy.rs`

- [ ] **步骤 1：编写失败的测试**

新增（Chat 路径 reasoning 剥离）：

```rust
#[test]
fn chat_path_strips_reasoning_when_model_unsupported() {
    let relay = relay_with_reasoning_map(json!({"kimi-k2.6": false}));
    relay.protocol = RelayProtocol::ChatCompletions;
    let body = json!({"model":"kimi-k2.6","reasoning":{"effort":"low"},"input":[]});
    let (endpoint, upstream_body, wire) = upstream_request_parts_with_image_decision(&relay, body, true).unwrap();
    assert!(upstream_body.get("reasoning").is_none(), "reasoning should be stripped for unsupported model");
    assert_eq!(wire, UpstreamWireApi::ChatCompletions);
}

#[test]
fn chat_path_preserves_reasoning_when_supported() {
    let relay = relay_with_reasoning_map(json!({"kimi-k2.6": false}));
    relay.protocol = RelayProtocol::ChatCompletions;
    let body = json!({"model":"deepseek-v4","reasoning":{"effort":"low"},"input":[]});
    let (_e, upstream_body, _w) = upstream_request_parts_with_image_decision(&relay, body, true).unwrap();
    assert!(upstream_body.get("reasoning").is_some(), "reasoning preserved for supported model");
}

#[test]
fn responses_path_preserves_reasoning_passthrough() {
    let relay = relay_with_reasoning_map(json!({"kimi-k2.6": false}));
    relay.protocol = RelayProtocol::Responses;
    let body = json!({"model":"kimi-k2.6","reasoning":{"effort":"low"},"input":[]});
    let (_e, upstream_body, _w) = upstream_request_parts_with_image_decision(&relay, body, true).unwrap();
    assert!(upstream_body.get("reasoning").is_some(), "Responses passthrough keeps reasoning");
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test protocol_proxy chat_path_strips_reasoning`
预期：FAIL（当前 `upstream_request_parts` 不调 `strip_reasoning_in_place`，reasoning 保留）

- [ ] **步骤 3：修改实现**

`upstream_request_parts`（`protocol_proxy.rs:762`）Chat 分支改为转换前剥离 reasoning：

```rust
RelayProtocol::ChatCompletions => {
    let mut body = request_json;
    let model = body.get("model").and_then(Value::as_str).unwrap_or("").to_string();
    let supports_reasoning = model_supports_reasoning(relay, &model);
    strip_reasoning_in_place(&mut body, supports_reasoning);
    Ok((
        chat_completions_url(&relay.base_url),
        responses_to_chat_completions_with_image_support(body, supports_image)?,
        UpstreamWireApi::ChatCompletions,
    ))
}
```

删除 `strip_input_images_in_place`（`:865`）及其测试（`tests/protocol_proxy.rs` 中所有 `strip_input_images_in_place` 调用，约 6 处）。

- [ ] **步骤 4：运行测试验证通过**

```bash
cargo test --test protocol_proxy chat_path_
cargo test --test protocol_proxy model_supports_reasoning
cargo test --test protocol_proxy strip_reasoning_in_place
```
预期：全绿

- [ ] **步骤 5：回归 + Commit**

```bash
cargo test --test protocol_proxy
```
预期：全绿（删 `strip_input_images_in_place` 后无残留引用）

```bash
git add crates/codex-plus-core/src/protocol_proxy.rs crates/codex-plus-core/tests/protocol_proxy.rs
git commit -m "feat(reasoning): Chat 路径接入 reasoning 剥离 + 清 strip_input_images 死代码 (#1468 Bug 2)"
```

---

### Phase 2 Review 关卡

- [ ] **自审**：Bug 1 生产 `proxied_client()` 确无 `.no_proxy()`？测试 `NO_PROXY` env 在所有 127.0.0.1 测试前生效？Bug 2 reasoning 剥离只在 Chat 分支（Responses 透传未动）？`strip_input_images_in_place` 彻底删除无残留？
- [ ] **回归**：`cargo test --workspace --no-fail-fast` 全绿。
- [ ] **Oracle 审查**：调 review 子代理，重点查 reasoning 剥离是否误伤支持 reasoning 的模型（默认 true 保守）、Responses 透传是否真的不动 reasoning。
- [ ] **手动验证**（可选）：`bash scripts/dev.sh` 启动，配置一个不支持 reasoning 的模型，发带 reasoning 的请求，确认上游不报 "reasoning not supported"。

---

## Phase 3：Bug 4 — 抽 vision.rs + 两层缓存 + 并发 + 重试 + 超时 + 两 prompt

> 本阶段最大。按"先抽模块（行为不变）-> 加缓存 -> 加并发 -> 加重试 -> 加超时"顺序，每步 TDD + 全绿再下一步。

### 任务 5：抽 vision.rs 模块（纯移动，行为不变）

**文件：**
- 创建：`crates/codex-plus-core/src/vision.rs`
- 修改：`crates/codex-plus-core/src/lib.rs`（加 `pub mod vision;`）
- 修改：`crates/codex-plus-core/src/protocol_proxy.rs`（移出 VL 代码，改为 `vision::` 调用）

- [ ] **步骤 1：基线测试**

运行：`cargo test --test protocol_proxy` 记录当前通过数（基线，迁移后应不变）。

- [ ] **步骤 2：创建 vision.rs，移入 VL 代码**

从 `protocol_proxy.rs` 移到 `vision.rs`（保持函数签名不变，改 `pub` 可见性）：
- 常量：`VL_IMAGE_LIMIT`、`VL_SINGLE_TIMEOUT`（`:13-15`）
- 函数：`estimate_item_tokens`、`items_within_vl_window`（`:949`）、`collect_input_text`（`:967`）、`describe_image_with_vl`（`:1000`）、`analyze_images_with_vl`（`:1090`）、`apply_vl_with_fallback`（`:1163`）、`extract_image_url`、`default_vision_relay_protocol`（若在此文件）

`vision.rs` 顶部：
```rust
//! VL 视觉模型中转：纯文本模型图片理解。
//! 两层缓存 + 并发批次 + 混合重试 + 双层超时 + 重发入口（架构 A+）。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use serde_json::{Value, json};
use crate::settings::{RelayProfile, RelayProtocol, VisionRelayConfig};
use crate::http_client::proxied_client;
use crate::{chat_completions_url, responses_url}; // 需要 pub(crate)

pub const VL_IMAGE_LIMIT: usize = 10;
pub const VL_SINGLE_TIMEOUT: Duration = Duration::from_secs(30);
// ... 移入的函数（签名不变）
```

`lib.rs` 加：`pub mod vision;`

`protocol_proxy.rs`：
- 删除移出的 VL 代码
- `:536` 调用改 `vision::apply_vl_with_fallback(...)`
- `upstream_request_parts_with_image_decision` 等若引用 VL 函数，改 `vision::` 前缀
- `chat_completions_url`/`responses_url` 改 `pub(crate)` 供 vision.rs 用（若非 pub）

- [ ] **步骤 3：编译 + 测试验证行为不变**

```bash
cargo build -p codex-plus-core
cargo test --test protocol_proxy
```
预期：编译通过，测试数 = 步骤 1 基线（行为不变）

- [ ] **步骤 4：Commit**

```bash
git add crates/codex-plus-core/src/vision.rs crates/codex-plus-core/src/lib.rs crates/codex-plus-core/src/protocol_proxy.rs
git commit -m "refactor: 抽 vision.rs 模块，VL 逻辑独立（行为不变）(#1468 Bug 4.1)"
```

---

### 任务 6：两层缓存 + 最新/历史检测 + 两 prompt

**文件：**
- 修改：`crates/codex-plus-core/src/vision.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[tokio::test]
async fn tier1_history_image_cached_by_url_no_recall() {
    // 同一图第二次（作为历史）处理时不重复调 VL
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let call_count = Arc::new(AtomicU32::new(0));
    let cc = call_count.clone();
    tokio::spawn(async move {
        // mock 计数 + 回写
        // ... mock_vl_server_with_count(listener, cc, vl_response("desc"))
    });
    let mut config = default_vision_relay_config();
    config.enabled = true; config.base_url = format!("http://{addr}");
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    // 第一次（最新）：1 次调用
    let mut b1 = body_with_image();
    analyze_images_with_vl(&mut b1, &config, &client).await.unwrap();
    // 第二次（历史，URL 命中 Tier1）：0 次新调用
    vision::cache_clear(); // 测试隔离用 helper
    // ... 实际：先调一次填充 Tier2，再调一次作为历史验证命中
    assert_eq!(call_count.load(Ordering::SeqCst), 1, "history image should hit Tier1 cache");
}

#[tokio::test]
async fn tier2_resend_new_question_triggers_new_call() {
    // 重发图 + 新问题 -> 新调用（入口）；重复问题 -> 命中
    // ... mock 计数；第一次 (img, Q1) 1 调用；重发 (img, Q2) 1 新调用；重发 (img, Q1) 0 调用（命中）
}

#[tokio::test]
async fn tier1_prompt_has_no_question() {
    // 历史图（Tier1）的 VL 请求体不含 user question
    // ... 捕获请求体，断言 prompt 是 comprehensive 无 {question}
}

#[tokio::test]
async fn tier2_prompt_includes_question() {
    // 最新图（Tier2）的 VL 请求体含 user question
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test protocol_proxy tier1_ tier2_`
预期：FAIL（当前单层、单 prompt）

- [ ] **步骤 3：实现两层缓存 + 检测 + 两 prompt**

`vision.rs` 加：

```rust
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

const CACHE_CAPACITY: usize = 500;
const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

type CacheEntry = (String, Instant);
static VL_CACHE: LazyLock<Mutex<HashMap<u64, CacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn url_hash(url: &str) -> u64 {
    let mut h = DefaultHasher::new();
    url.hash(&mut h);
    h.finish()
}

fn url_question_hash(url: &str, question: &str) -> u64 {
    let mut h = DefaultHasher::new();
    url.hash(&mut h);
    question.hash(&mut h);
    h.finish()
}

fn cache_get(key: u64) -> Option<String> {
    let mut cache = VL_CACHE.lock().unwrap();
    if let Some((desc, written)) = cache.get(&key).cloned() {
        if written.elapsed() < CACHE_TTL {
            return Some(desc);
        }
        cache.remove(&key); // 过期
    }
    None
}

fn cache_put(key: u64, desc: String) {
    let mut cache = VL_CACHE.lock().unwrap();
    if cache.len() >= CACHE_CAPACITY {
        // 淘汰最旧
        if let Some((&oldest_key, _)) = cache.iter().min_by_key(|(_, (_, t))| *t) {
            cache.remove(&oldest_key);
        }
    }
    cache.insert(key, (desc, Instant::now()));
}

#[cfg(test)]
pub fn cache_clear() { VL_CACHE.lock().unwrap().clear(); }

const TIER1_PROMPT: &str = "请详细描述这张图片，涵盖所有视觉信息：文字（逐字 OCR）、UI 元素、颜色、形状、布局结构、错误信息等。请用中文回复。";

fn tier2_prompt(question: &str) -> String {
    format!("{TIER1_PROMPT}\n用户当前问题：{question}\n在全面描述基础上，对与上述问题相关的内容做更详细说明。")
}

/// 判断图片是否在最新 user 消息（input 数组的最后一项 role=user）。
fn is_latest_message_image(input: &[Value], item_idx: usize) -> bool {
    // 最新 user item 的索引
    let latest_user = input.iter().enumerate().rev()
        .find(|(_, it)| it.get("role").and_then(Value::as_str) == Some("user"))
        .map(|(i, _)| i);
    latest_user == Some(item_idx)
}
```

修改 `analyze_images_with_vl`：对每个 window 内图片，判断 latest vs history：
- latest：用 `tier2_prompt(question)`，cache key = `url_question_hash`。miss -> 调 VL -> cache_put。
- history：用 `TIER1_PROMPT`，cache key = `url_hash`。miss -> 调 VL -> cache_put。

`question` = `collect_input_text(input)`（最新 user 文本）。

- [ ] **步骤 4：运行测试验证通过**

```bash
cargo test --test protocol_proxy tier1_ tier2_
```
预期：PASS

- [ ] **步骤 5：回归 + Commit**

```bash
cargo test --test protocol_proxy
```
预期：全绿（注意更新受影响的旧 prompt 测试：`analyze_images_with_vl_forwards_user_question_as_prompt` 改为验证 Tier2 含问题；`falls_back_to_generic_prompt_without_user_text` 改为验证 Tier1 无问题）

```bash
git add crates/codex-plus-core/src/vision.rs crates/codex-plus-core/tests/protocol_proxy.rs
git commit -m "feat(VL): 两层缓存(URL/(URL,问题)) + 最新/历史检测 + 两 prompt + 重发入口 (#1468 Bug 4.4/4.8)"
```

---

### 任务 7：并发（Semaphore + JoinSet）+ 批次（多图一调用）

**文件：**
- 修改：`crates/codex-plus-core/src/vision.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[tokio::test]
async fn vl_processes_images_concurrently_faster_than_serial() {
    // 5 张图，每张 mock 延迟 200ms；并发应 << 5×200ms=1000ms（<300ms）
    // ... mock_vl_server_with_delay(200ms)，5 图，计时
    let elapsed = ...; // analyze_images_with_vl 耗时
    assert!(elapsed < Duration::from_millis(500), "concurrent should beat serial: {elapsed:?}");
}

#[tokio::test]
async fn vl_batches_multiple_images_per_call() {
    // 5 张图 -> mock 应收到 1 次请求含 5 个 image_url（BATCH_SIZE=5）
    // ... 捕获请求体，断言 image_url 出现 5 次
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test protocol_proxy vl_processes_images_concurrently vl_batches`
预期：FAIL（当前串行 + 单图）

- [ ] **步骤 3：实现并发 + 批次**

`vision.rs` 加：

```rust
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const BATCH_SIZE: usize = 5;
const MAX_CONCURRENCY: usize = 5;
static VL_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(MAX_CONCURRENCY));

/// 批量调 VL：一组图（≤BATCH_SIZE）一次 API 调用，返回每图描述。
async fn call_vlm_batch(
    urls: &[String], prompt: &str, config: &VisionRelayConfig, client: &reqwest::Client,
) -> anyhow::Result<Vec<String>> {
    let endpoint = match config.protocol {
        RelayProtocol::ChatCompletions => chat_completions_url(&config.base_url),
        RelayProtocol::Responses => responses_url(&config.base_url),
    };
    // 构造多图 messages/input（ChatCompletions: content 含多个 image_url；Responses: input 含多个 input_image）
    let body = build_batch_body(urls, prompt, config);
    let _permit = VL_SEMAPHORE.acquire().await.unwrap();
    let resp = client.post(&endpoint).bearer_auth(&config.api_key)
        .json(&body).timeout(per_batch_timeout(urls.len())).send().await?;
    // 解析每图描述（按顺序）
    parse_batch_response(resp, urls.len(), config.protocol).await
}
```

修改 `analyze_images_with_vl`：收集未命中缓存的图 URL（分 Tier1/Tier2 组），按 `BATCH_SIZE` 分批，用 `JoinSet` 并发跑各批：

```rust
let mut set = JoinSet::new();
for batch in uncached_urls.chunks(BATCH_SIZE) {
    let prompt = if tier { TIER1_PROMPT.to_string() } else { tier2_prompt(&question) };
    let cfg = config.clone(); let cl = client.clone(); let b = batch.to_vec();
    set.spawn(async move { (b, call_vlm_batch(&b, &prompt, &cfg, &cl).await) });
}
while let Some(res) = set.join_next().await {
    let (urls, result) = res.unwrap();
    // 填充缓存 + 替换 input_image -> input_text
}
```

- [ ] **步骤 4：运行测试验证通过**

```bash
cargo test --test protocol_proxy vl_processes_images_concurrently vl_batches
```
预期：PASS

- [ ] **步骤 5：回归 + Commit**

```bash
cargo test --test protocol_proxy
git commit -m "feat(VL): 并发(Semaphore+JoinSet) + 批次(5图/调用) (#1468 Bug 4.2/4.3)"
```

---

### 任务 8：混合重试（批量 2 次 -> 拆单张 3 次）

**文件：**
- 修改：`crates/codex-plus-core/src/vision.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[tokio::test]
async fn vl_batch_retries_on_transient_failure() {
    // mock 前 1 次返回 500，第 2 次 200 -> 批量重试成功
    // ... mock_vl_server_flaky(fail_first=1)
    // 断言最终成功，调用 2 次
}

#[tokio::test]
async fn vl_isolates_bad_image_via_single_fallback() {
    // 5 图含 1 坏图（mock 对含坏图的批次总 500，单张好图 200）
    // 批量 2 次失败 -> 拆单张 -> 4 好图成功，1 坏图 strip
    // 断言 4 个 input_text + 1 个被 strip
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test protocol_proxy vl_batch_retries vl_isolates`
预期：FAIL（当前无重试，瞬时失败直接 strip）

- [ ] **步骤 3：实现混合重试**

`vision.rs` 加：

```rust
const BATCH_MAX_ATTEMPTS: u32 = 2;   // 批量 2 次
const SINGLE_MAX_ATTEMPTS: u32 = 3;  // 单张 3 次

async fn with_retry<F, Fut, T>(max_attempts: u32, mut f: F) -> anyhow::Result<T>
where F: FnMut() -> Fut, Fut: std::future::Future<Output = anyhow::Result<T>> {
    let mut last = None;
    for attempt in 0..max_attempts {
        if attempt > 0 {
            let backoff = backoff_delay(attempt);
            tokio::time::sleep(backoff).await;
        }
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap())
}

fn backoff_delay(attempt: u32) -> Duration {
    let base = 0.3 * 2u32.pow(attempt - 1); // 0.3, 0.6, 1.2...
    let jitter = (rand_u64() % 20) as f64 / 100.0; // ±20%（用简单 hash 替代 rand，免新依赖）
    Duration::from_secs_f64(base * (0.8 + jitter))
}
```

修改批次处理：`call_vlm_batch` 包 `with_retry(BATCH_MAX_ATTEMPTS, ...)`；批量仍失败 -> 对该批每张图单独 `call_vlm_batch(&[url], ...)` 包 `with_retry(SINGLE_MAX_ATTEMPTS, ...)`。单张仍失败 -> 该图 strip（替换为空/标记，retain 清理）。

- [ ] **步骤 4：运行测试验证通过**

```bash
cargo test --test protocol_proxy vl_batch_retries vl_isolates
```
预期：PASS

- [ ] **步骤 5：回归 + Commit**

```bash
cargo test --test protocol_proxy
git commit -m "feat(VL): 混合重试(批量2次+拆单张3次) + 坏图隔离 (#1468 Bug 4.6)"
```

---

### 任务 9：双层超时 + char-safe 描述截断

**文件：**
- 修改：`crates/codex-plus-core/src/vision.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[tokio::test]
async fn vl_total_timeout_degrades_to_strip() {
    // mock 永久挂起（不回写）-> 总超时 120s 后降级 strip 不卡死
    // 用短超时常量测试（暴露 VL_TOTAL_TIMEOUT 为可配置或测试用短值）
    // 断言：图片被 strip，函数返回 Ok（不阻断）
}

#[tokio::test]
async fn vl_description_truncated_char_safe() {
    // VL 返回 5000 字符 -> 截断为 2000 字符（char-safe，不 panic）
    // ... 断言替换后的 input_text 长度 ≤ 2000 chars
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test protocol_proxy vl_total_timeout vl_description_truncated`
预期：FAIL（当前无总超时；无截断）

- [ ] **步骤 3：实现超时 + 截断**

`vision.rs` 加：

```rust
const VL_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
const DESC_MAX_CHARS: usize = 2000;

fn per_batch_timeout(n: usize) -> Duration {
    Duration::from_secs(15 + 8 * n as u64)
}

fn truncate_char_safe(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}
```

修改 `apply_vl_with_fallback`（或 `analyze_images_with_vl` 入口）包总超时：

```rust
match tokio::time::timeout(VL_TOTAL_TIMEOUT, analyze_images_with_vl_inner(...)).await {
    Ok(Ok(())) => Ok((true, vl_body)),
    Ok(Err(_)) | Err(_) => {
        // 降级 strip（不阻断）
        Ok((false, original_body))
    }
}
```

描述注入前截断：`let desc = truncate_char_safe(&desc, DESC_MAX_CHARS);`（在 cache_put 前）。

> 测试总超时用真实 120s 太慢。方案：`VL_TOTAL_TIMEOUT` 用 `cfg!(test)` 时取短值（如 2s），或暴露 `analyze_images_with_vl_with_timeout` 测试 helper。

- [ ] **步骤 4：运行测试验证通过**

```bash
cargo test --test protocol_proxy vl_total_timeout vl_description_truncated
```
预期：PASS

- [ ] **步骤 5：全量回归 + Commit**

```bash
cargo test --workspace --no-fail-fast
```
预期：全绿

```bash
git commit -m "feat(VL): 双层超时(per-batch+总120s) + char-safe 描述截断 (#1468 Bug 4.5/4.7)"
```

---

### Phase 3 Review 关卡

- [ ] **自审**：
  - vision.rs 公开 API 是否只有 `apply_vl_with_fallback` + 必要类型？缓存/并发/重试内部封装？
  - 两层缓存检测（`is_latest_message_image`）边界对吗（input 为空、多 user 消息、role 非 user）？
  - 重试退避无新依赖（用简单 hash 做 jitter，非 rand crate）？
  - 总超时降级是否真的不阻断用户（返回 strip 后的 body）？
  - char-safe 截断用 `chars().take`（非字节）？
- [ ] **回归**：`cargo test --workspace --no-fail-fast` 全绿。
- [ ] **Oracle 审查**：调 review 子代理审查 vision.rs，重点：缓存淘汰是否真防无界增长、并发下缓存 Mutex 是否死锁、重试 + 总超时交互是否合理。
- [ ] **手动验证**：`bash scripts/dev.sh`，配 VL（mock 或真实 VL 模型），发多图请求，确认并发 + 缓存 + 重发入口行为。

---

## Review 环节（详细）

### 每任务自审（commit 前）
- [ ] 改动是否最小化（只动必要行）？
- [ ] 测试是否覆盖正反例（happy path + 边界 + 失败）？
- [ ] 无新增 `unwrap()` 在生产路径（`?` + 降级）？
- [ ] 无占位符/TODO？
- [ ] commit message 符合项目规范（中文，带 `#1468 Bug N`）？

### Phase 关卡审查（每 phase 结束）
- [ ] 跑该 phase 全部测试 + 回归。
- [ ] 用 review 子代理（参考 handoff 的 Oracle 模式）审查本 phase diff：
  - 正确性：逻辑是否符合 spec？
  - 边界：空输入、单图、超限、坏图、中文、并发竞争。
  - 回归：是否破坏现有行为？
  - 风格：是否匹配项目代码风格（中文注释、命名）？

### PR 自审（全部完成后）
- [ ] `cargo test --workspace --no-fail-fast` 全绿。
- [ ] `cd apps/codex-plus-manager && npx tsx --test src/model-windows.test.ts && npx tsc --noEmit` 全绿（前端未受影响，确认无回归）。
- [ ] `cargo clippy -p codex-plus-core -- -D warnings` 无 warning。
- [ ] 逐文件 review diff，确认无意外改动。
- [ ] spec 的 6 个 bug 全部有对应 commit + 测试。
- [ ] 更新 PR 描述，列出 6 个 bug 的修复 + 测试证据。

### 重点审查项（易错处）
- **Bug 1**：确认生产 `proxied_client()` 无 `.no_proxy()`；测试 `NO_PROXY` env 在 static/缓存的 client 构建前生效。
- **Bug 2**：确认 Responses 分支未动（透传保留 reasoning）；`model_supports_reasoning` 默认 true 不误伤。
- **Bug 4**：缓存 Mutex 在并发 JoinSet 下无死锁（持锁时间短，仅 HashMap 操作）；总超时降级返回 strip body 不 panic；两层 prompt 检测边界（空 input、无 user 消息）。

---

## 测试环节（详细）

### 测试金字塔
1. **单元测试**（`#[test]`，纯函数）：`model_supports_reasoning`、`strip_reasoning_in_place`、`url_hash`、`is_latest_message_image`、`truncate_char_safe`、`backoff_delay`、`per_batch_timeout`。
2. **集成测试**（`#[tokio::test]`，mock VL server）：每个 bug 的行为测试（见各任务）。
3. **回归测试**：`cargo test --workspace --no-fail-fast` + 前端。

### Mock VL server 使用
- 复用 `mock_vl_server`（`tests/protocol_proxy.rs:1311`）。
- 扩展变体（任务 7/8 需要）：
  - `mock_vl_server_with_delay(listener, body, delay)`：测并发计时。
  - `mock_vl_server_flaky(listener, fail_first_n, body)`：测重试。
  - `mock_vl_server_with_count(listener, body, counter)`：测缓存命中（调用计数）。
  - `mock_vl_server_hang(listener)`：测总超时（不回写）。
- 所有 mock 在 127.0.0.1，依赖 `NO_PROXY` env（任务 3 设置）。

### 每个 Bug 的测试矩阵

| Bug | 测试 | 关键断言 |
|-----|------|---------|
| 1 | `proxied_client_builds_without_no_proxy` + 6 个原 pre-existing 测试 | 生产 client 无 no_proxy；mock 127.0.0.1 不被拦 |
| 2 | `chat_path_strips/preserves_reasoning`、`responses_path_preserves` | Chat 不支持时剥离、支持时保留、Responses 透传 |
| 3+6 | `vl_log_does_not_panic_on_chinese` | 201 中文字符不 panic、日志无正文 |
| 5 | `vl_endpoint_normalizes_bare_domain`、`vl_endpoint_does_not_duplicate` | 裸域名加 /v1、完整不重复 |
| 4 | `tier1_history_cached`、`tier2_resend_new_question`、`tier1_prompt_no_question`、`tier2_prompt_includes_question`、`concurrently_faster`、`batches_multiple`、`batch_retries`、`isolates_bad_image`、`total_timeout_degrades`、`description_truncated_char_safe`、`context_window_strips` | 见各任务 |

### 受影响的现有测试（需更新，非删除）
- `analyze_images_with_vl_forwards_user_question_as_prompt`（`:1481`）：改为验证 Tier2（最新图）含问题。
- `analyze_images_with_vl_falls_back_to_generic_prompt_without_user_text`（`:1585`）：改为验证 Tier1（历史图）无问题。
- `analyze_images_with_vl_strips_old_images_outside_context_window`（`:1713`）：保留，验证 context_window strip 行为不变。
- 所有 `strip_input_images_in_place` 测试（约 6 处）：删除（函数已移除）。

### 测试命令速查
```bash
# 单个 bug 测试
cargo test --test protocol_proxy vl_endpoint_          # Bug 5
cargo test --test protocol_proxy vl_log_does_not_panic # Bug 3+6
cargo test --test protocol_proxy chat_path_            # Bug 2
cargo test --test protocol_proxy tier1_ tier2_         # Bug 4 缓存
cargo test --test protocol_proxy vl_batch vl_isolates  # Bug 4 重试

# 全套件
cargo test --workspace --no-fail-fast
cd apps/codex-plus-manager && npx tsx --test src/model-windows.test.ts && npx tsc --noEmit

# 质量
cargo clippy -p codex-plus-core -- -D warnings
```

### 覆盖目标
- 6 个 bug 各有 ≥1 个失败测试（RED）先于实现。
- Bug 4 每个 sub-feature（缓存/并发/重试/超时/prompt）各有正反例测试。
- 无回归：迁移前基线测试数 ≤ 迁移后通过数。

---

## 自检

**1. 规格覆盖度**：
- Bug 1 -> 任务 3 ✅
- Bug 2 -> 任务 4 ✅
- Bug 3 -> 任务 2（合并入 Bug 6）✅
- Bug 4.1 模块抽取 -> 任务 5 ✅
- Bug 4.2 并发 -> 任务 7 ✅
- Bug 4.3 批次 -> 任务 7 ✅
- Bug 4.4 两层缓存 -> 任务 6 ✅
- Bug 4.5 超时 -> 任务 9 ✅
- Bug 4.6 重试 -> 任务 8 ✅
- Bug 4.7 溢出保护（廉价兜底截断）-> 任务 9 ✅
- Bug 4.8 prompt + 入口 -> 任务 6 ✅
- Bug 5 -> 任务 1 ✅
- Bug 6 -> 任务 2 ✅
- 实施顺序 5,6 -> 1,2,3 -> 4 ✅（Phase 1=5,6；Phase 2=1,2；Phase 3=4；Bug 3 并入 Phase 1）

**2. 占位符扫描**：无 TODO/待定。部分 mock 变体（`mock_vl_server_with_delay` 等）标了"扩展变体"需实现，已给出签名方向。

**3. 类型一致性**：
- `apply_vl_with_fallback` 签名迁移后不变（`protocol_proxy.rs:536` 调用不需改参数）。
- `analyze_images_with_vl` 签名不变（测试复用）。
- `upstream_request_parts_with_image_decision` 签名不变（Bug 2 测试复用）。
- 新增：`CacheKey` 实际用 `u64`（`url_hash`/`url_question_hash` 返回 u64），全文一致。
- `call_vlm_batch(urls: &[String], prompt: &str, config: &VisionRelayConfig, client: &reqwest::Client) -> Result<Vec<String>>`，任务 7 定义、任务 8 复用，签名一致。

**4. 潜在风险点已记入 Review 环节**（缓存死锁、超时降级、检测边界）。

无遗漏。计划可执行。
