# Vision Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 PR #1550 管理员 review 指出的四项问题（缓存碰撞、BATCH_SIZE 分批、Phase 2 后台分析、function_call_output 图片透传），使视觉模块达到上线标准。

**Architecture:** 在现有 vision.rs / protocol_proxy.rs 上做增量修复，不重构整体架构。R1 缓存结构键是基础，R2 分批和 R3 后台分析依赖 R1 的 CacheKey 类型，R4 工具图片处理依赖 R1+R2。

**Tech Stack:** Rust（codex-plus-core crate），reqwest + wiremock（测试），serde_json。无新依赖（不用 regex/sha2）。

## Global Constraints

- 对话用中文，代码可用英文，注释尽量中文
- 不引入新依赖（不用 regex、不新增 sha2 到 vision.rs）
- 改动隔离 + opt-in，不破坏现有 per-profile 行为
- 测试沿用上游 `#[test]` + `#[tokio::test]` + wiremock 风格
- 全局 VLM_CACHE 共享，并发竞争测试用 `--test-threads=1`
- `cargo test -p codex-plus-core --lib` 为主要测试命令

---

## File Structure

| 文件 | 职责 | 改动 |
|---|---|---|
| `crates/codex-plus-core/src/vision.rs` | VLM 分析核心：缓存、分批、Phase 2、工具图片 | R1/R2/R3/R4 全部 |
| `crates/codex-plus-core/src/protocol_proxy.rs` | 请求转发：图片处理接入点 | R4 接入 |

vision.rs 已 2691 行，本 plan 不做文件拆分（遵循现有大文件模式），但新增函数聚集在明确分隔的区块。

---

### Task 1: R1 缓存结构键 CacheKey

**Files:**
- Modify: `crates/codex-plus-core/src/vision.rs:8-9`（imports）、`:45-46`（VLM_CACHE 定义）、`:52-67`（url_hash/url_question_hash）、`:69-94`（cache_get/cache_put/cache_contains）
- Modify: `crates/codex-plus-core/src/vision.rs` 所有 `cache_get(key)` 调用点（5a/5b/5c 约 1040/1115/1145 行）、测试中 `cache_put`+`cache_get` 连续调用模式

**Interfaces:**
- Produces: `enum CacheKey { Url(String), UrlQuestion(String, String) }`；`fn url_hash(&str) -> CacheKey`；`fn url_question_hash(&str, &str) -> CacheKey`；`fn cache_get(&CacheKey) -> Option<String>`；`fn cache_put(CacheKey, String)`；`fn cache_contains(&CacheKey) -> bool`

- [ ] **Step 1: 写失败测试--CacheKey 基本行为 + Tier1/Tier2 互不串扰**

在 `mod tests` 中（约 1680 行 `cache_put_evicts_oldest_when_full` 测试之后）新增：

```rust
    #[test]
    fn cachekey_url_and_url_question_do_not_collide() {
        cache_clear_for_tests();
        // 同 URL，Tier1 key vs Tier2 key 应互不命中
        let tier1 = url_hash("https://example.com/img.png");
        let tier2 = url_question_hash("https://example.com/img.png", "问题A");
        cache_put(tier1, "tier1-desc".to_string());
        // Tier2 key 不应命中 Tier1 的描述
        assert_eq!(cache_get(&tier2), None);
        assert_eq!(cache_get(&url_hash("https://example.com/img.png")), Some("tier1-desc".to_string()));
    }

    #[test]
    fn cachekey_different_questions_do_not_collide() {
        cache_clear_for_tests();
        let k1 = url_question_hash("https://example.com/img.png", "问题A");
        let k2 = url_question_hash("https://example.com/img.png", "问题B");
        cache_put(k1, "desc-A".to_string());
        assert_eq!(cache_get(&k2), None);
    }

    #[test]
    fn cachekey_empty_question_uses_tier1() {
        cache_clear_for_tests();
        // user_text 为空时 use_tier2=false，走 url_hash (Tier1)
        let key = url_hash("https://example.com/solo.png");
        cache_put(key.clone(), "solo-desc".to_string());
        assert_eq!(cache_get(&key), Some("solo-desc".to_string()));
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p codex-plus-core --lib -- vision::tests::cachekey 2>&1 | head -20`
Expected: 编译失败（`cache_get` 接收 `u64` 而非 `&CacheKey`，`url_hash` 返回 `u64` 而非 `CacheKey`）

- [ ] **Step 3: 实现 CacheKey enum + 改缓存函数**

在 vision.rs `// ── Global state ──` 区块，替换缓存定义和函数：

```rust
/// 缓存键：Tier1 只看 URL（历史轮/无文字当前轮），Tier2 看 URL+问题（有文字当前轮）。
/// 用结构键而非 hash 值，HashMap 的 Eq 比较原始字符串，彻底消除碰撞误命中。
#[derive(Hash, Eq, PartialEq, Clone)]
enum CacheKey {
    Url(String),
    UrlQuestion(String, String),
}

/// 图片描述缓存：key=CacheKey，value=(描述, 写入时间)。
static VLM_CACHE: LazyLock<Mutex<HashMap<CacheKey, CacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 全局 VLM 信号量，限制跨请求并发数。
static VL_SEMAPHORE: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(5));

fn url_hash(url: &str) -> CacheKey {
    CacheKey::Url(url.to_string())
}

fn url_question_hash(url: &str, question: &str) -> CacheKey {
    CacheKey::UrlQuestion(url.to_string(), question.to_string())
}

fn cache_get(key: &CacheKey) -> Option<String> {
    let mut cache = VLM_CACHE.lock().unwrap();
    if let Some((desc, written)) = cache.get(key).cloned() {
        if written.elapsed() < CACHE_TTL {
            return Some(desc);
        }
        cache.remove(key);
    }
    None
}

fn cache_put(key: CacheKey, desc: String) {
    let mut cache = VLM_CACHE.lock().unwrap();
    if cache.len() >= CACHE_CAPACITY {
        // clone oldest key 以便 remove（cache 已被 &mut 借用，不能直接拿 &key）
        if let Some((oldest, _)) = cache.iter().min_by_key(|(_, (_, t))| *t) {
            let oldest = oldest.clone();
            cache.remove(&oldest);
        }
    }
    cache.insert(key, (desc, Instant::now()));
}

fn cache_contains(key: &CacheKey) -> bool {
    let cache = VLM_CACHE.lock().unwrap();
    cache
        .get(key)
        .map(|(_, t)| t.elapsed() < CACHE_TTL)
        .unwrap_or(false)
}
```

- [ ] **Step 4: 修复所有调用点**

`cache_get` 从值传递改为引用，需在以下位置加 `&`：

1. analyze_and_inject 内部（约 1012 行）：`cache_put(url_hash(url), ...)` -- 不变（值传递 OK）
2. 5a 当前轮（约 1040 行）：`cache_get(key)` -> `cache_get(&key)`
3. 5a 当前轮写入（约 1060 行）：`cache_put(key, desc.clone())` -- 不变
4. 5b 黄金窗口（约 1115 行）：`cache_get(key)` -> `cache_get(&key)`
5. 5c 深层（约 1145 行）：`cache_get(key)` -> `cache_get(&key)`

测试中的调用点修复（`cache_put(key, ...)` 后 `cache_get(key)` 需改为 `cache_get(&key)` 且 `cache_put` 用 `key.clone()` 或分开构造）：

```rust
    // cache_put_and_get_roundtrip 测试改写（约 1646 行）：
    fn cache_put_and_get_roundtrip() {
        cache_clear_for_tests();
        let key = url_hash("https://example.com/cache-test.png");
        cache_put(key.clone(), "cached description".to_string());
        let got = cache_get(&key);
        assert_eq!(got, Some("cached description".to_string()));
    }

    // cache_contains_returns_false_for_missing_key 改写（约 1655 行）：
    fn cache_contains_returns_false_for_missing_key() {
        cache_clear_for_tests();
        let key = url_hash("https://example.com/missing.png");
        assert!(!cache_contains(&key));
    }

    // cache_put_evicts_oldest_when_full 改写（约 1662 行）：
    // 所有 cache_put(url_hash(...), ...) 不变；cache_contains(key) -> cache_contains(&key)
    // cache_get 在此测试中未直接调用，cache_contains 改 & 即可
```

对于集成测试（如 `strip_image_blocks_all_cache_hits_no_vlm_call` 等）中 `cache_put(url_hash(...), ...)` 模式：`url_hash` 返回 CacheKey，`cache_put` 接收 CacheKey，**无需改动**。但若有 `let key = url_hash(...); cache_put(key, ...); cache_get(&key)` 模式需确保 `cache_put` 用 `key.clone()`。

用 `rg -n 'cache_get\(|cache_contains\(' crates/codex-plus-core/src/vision.rs` 全量检查所有调用点是否已加 `&`。

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test -p codex-plus-core --lib -- vision::tests 2>&1 | tail -5`
Expected: 全部通过（含新增 3 个 cachekey 测试 + 现有缓存测试）

边缘场景验证：
Run: `cargo test -p codex-plus-core --lib -- vision::tests::cachekey 2>&1 | tail -5`
Expected: 3 passed

- [ ] **Step 6: Commit**

```bash
git add crates/codex-plus-core/src/vision.rs
git commit -m "fix(vision): 缓存 key 从 u64 改为结构化 CacheKey，消除 hash 碰撞误命中 (R1)"
```

---

### Task 2: R2 BATCH_SIZE 分批

**Files:**
- Modify: `crates/codex-plus-core/src/vision.rs:816-840`（call_vlm_batch 函数体）

**Interfaces:**
- Consumes: `BATCH_SIZE`（已定义，=5）、`call_vlm_batch_once`、`backoff_delay`
- Produces: `call_vlm_batch` 行为变更（内部分批），签名不变

- [ ] **Step 1: 写失败测试--分批触发多次 VLM 调用 + 逐图对应**

在 `mod tests` 中新增（需要 wiremock，放在 wiremock 测试区块附近，约 2530 行之前）：

```rust
    /// 7 张图（>BATCH_SIZE=5）应触发 2 次 VLM 调用（5+2），描述按顺序对应。
    #[tokio::test]
    async fn call_vlm_batch_chunks_by_batch_size() {
        let mock_server = MockServer::start().await;
        // 2 次 mock：第 1 批 5 张返回 [[图片1]]..[[图片5]]，第 2 批 2 张返回 [[图片1]]..[[图片2]]
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "[[图片1]]batch1-1\n[[图片2]]batch1-2\n[[图片3]]batch1-3\n[[图片4]]batch1-4\n[[图片5]]batch1-5"}}]
            })))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "[[图片1]]batch2-1\n[[图片2]]batch2-2"}}]
            })))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "k".into(), model: "m".into(), base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let urls: Vec<String> = (0..7).map(|i| format!("https://chunk.example.com/img{i}.png")).collect();

        let descs = call_vlm_batch(&urls, TIER1_PROMPT, &config, &client, BATCH_MAX_ATTEMPTS)
            .await
            .unwrap();

        assert_eq!(descs.len(), 7, "7 张图应返回 7 个描述");
        assert!(descs[0].contains("batch1-1"), "第 1 张描述对应第 1 批第 1 图");
        assert!(descs[4].contains("batch1-5"), "第 5 张描述对应第 1 批第 5 图");
        assert!(descs[5].contains("batch2-1"), "第 6 张描述对应第 2 批第 1 图");
        assert!(descs[6].contains("batch2-2"), "第 7 张描述对应第 2 批第 2 图");
    }

    /// 恰好 5 张（=BATCH_SIZE）应只触发 1 次调用。
    #[tokio::test]
    async fn call_vlm_batch_exact_batch_size_single_call() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "[[图片1]]d1\n[[图片2]]d2\n[[图片3]]d3\n[[图片4]]d4\n[[图片5]]d5"}}]
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "k".into(), model: "m".into(), base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let urls: Vec<String> = (0..5).map(|i| format!("https://exact.example.com/img{i}.png")).collect();

        let descs = call_vlm_batch(&urls, TIER1_PROMPT, &config, &client, BATCH_MAX_ATTEMPTS)
            .await
            .unwrap();
        assert_eq!(descs.len(), 5);
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p codex-plus-core --lib -- vision::tests::call_vlm_batch_chunks 2>&1 | tail -10`
Expected: FAIL -- 7 张图当前一次性发送，mock 的 `up_to_n_times(1)` 导致第 1 批响应被消耗后第 2 次调用无 mock 匹配，或描述顺序不对应

- [ ] **Step 3: 实现 call_vlm_batch 分批**

替换 `call_vlm_batch` 函数体（vision.rs:816-840）：

```rust
async fn call_vlm_batch(
    urls: &[String],
    prompt: &str,
    config: &VlmConfig,
    client: &reqwest::Client,
    max_attempts: u32,
) -> anyhow::Result<Vec<String>> {
    if urls.is_empty() {
        return Ok(Vec::new());
    }
    let salt = urls.first().map(String::as_str).unwrap_or("");
    let mut all_descs: Vec<String> = Vec::with_capacity(urls.len());
    // 按 BATCH_SIZE 分批，每批独立重试，避免一次性发送过多图片触发 provider 限制。
    for chunk in urls.chunks(BATCH_SIZE) {
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..max_attempts {
            if attempt > 0 {
                tokio::time::sleep(backoff_delay(attempt, salt)).await;
            }
            match call_vlm_batch_once(chunk, prompt, config, client).await {
                Ok(v) => {
                    all_descs.extend(v);
                    last_err = None;
                    break;
                }
                Err(e) => last_err = Some(e),
            }
        }
        if let Some(e) = last_err {
            return Err(e);
        }
    }
    Ok(all_descs)
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p codex-plus-core --lib -- vision::tests::call_vlm_batch 2>&1 | tail -10`
Expected: 2 passed（chunks + exact_batch_size）

验证现有测试不回归：
Run: `cargo test -p codex-plus-core --lib -- vision::tests 2>&1 | tail -5`
Expected: 全部通过

- [ ] **Step 5: Commit**

```bash
git add crates/codex-plus-core/src/vision.rs
git commit -m "fix(vision): 恢复 BATCH_SIZE 分批，每批 5 张独立调用 (R2)"
```

---

### Task 3: R3 Phase 2 后台分析恢复

**Files:**
- Modify: `crates/codex-plus-core/src/vision.rs:1168-1171`（bg_config_opt 占位 -> 收集逻辑）、`:1227-1229`（let _ = bg_config_opt -> tokio::spawn）
- Add: `crates/codex-plus-core/src/vision.rs` 新增 `background_analyze_and_cache` 函数（放在 `call_vlm_batch` 之后，约 840 行）

**Interfaces:**
- Consumes: `call_vlm_batch`（Task 2 分批版）、`cache_put`/`cache_contains`（Task 1 CacheKey 版）、`TIER1_PROMPT`、`url_hash`
- Produces: `async fn background_analyze_and_cache(&[String], &VlmConfig, &reqwest::Client)`

- [ ] **Step 1: 写失败测试--x_budget>10 时后台写入缓存 + x_budget<=10 不触发**

在 `mod tests` 中新增（放在 `strip_image_blocks_multi_round_depth_and_per_round_limit` 测试之后，约 2190 行）：

```rust
    /// x_budget>10 时，深层未缓存 URL 在后台被分析写入缓存。
    #[tokio::test]
    async fn phase2_background_analyzes_deep_urls_when_x_budget_gt_10() {
        cache_clear_for_tests();
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "deep-desc"}}]
            })))
            .mount(&mock_server)
            .await;

        let vlm_config = VlmConfig {
            api_key: "k".into(), model: "m".into(), base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        // 构造 x_budget>10 的场景：大 context_window(900000) -> X=8100>10
        // 深层历史（第 11+ 轮）有 1 张未缓存图
        let mut messages = vec![
            serde_json::json!({
                "role": "user", "content": [
                    {"type": "text", "text": "deep history"},
                    {"type": "image_url", "image_url": {"url": "https://phase2.example.com/deep.png"}},
                ]
            }),
            serde_json::json!({
                "role": "user", "content": [
                    {"type": "text", "text": "current question"},
                ]
            }),
        ];

        strip_image_blocks(&mut messages, &vlm_config, "{}", "900000", "gpt-4", &client).await;

        // 等待后台 spawn 完成（fire-and-forget，需 sleep 等待）
        tokio::time::sleep(Duration::from_millis(500)).await;

        // 深层 URL 应已写入缓存
        let key = url_hash("https://phase2.example.com/deep.png");
        assert_eq!(
            cache_get(&key),
            Some("deep-desc".to_string()),
            "Phase 2 后台应分析深层 URL 并写入缓存"
        );
    }

    /// x_budget<=10 时不触发 Phase 2，深层 URL 不被分析。
    #[tokio::test]
    async fn phase2_not_triggered_when_x_budget_le_10() {
        cache_clear_for_tests();
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "should-not-appear"}}]
            })))
            .expect(0) // 不应有 VLM 调用（深层 URL 不分析，黄金窗口内无未缓存图时）
            .mount(&mock_server)
            .await;

        let vlm_config = VlmConfig {
            api_key: "k".into(), model: "m".into(), base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        // context_window=800 -> X=6<=10，不触发 Phase 2
        let mut messages = vec![
            serde_json::json!({
                "role": "user", "content": [
                    {"type": "image_url", "image_url": {"url": "https://phase2-no.example.com/hist.png"}},
                ]
            }),
            serde_json::json!({
                "role": "user", "content": [{"type": "text", "text": "q"}]
            }),
        ];

        strip_image_blocks(&mut messages, &vlm_config, "{}", "800", "gpt-4", &client).await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        let key = url_hash("https://phase2-no.example.com/hist.png");
        assert_eq!(cache_get(&key), None, "x_budget<=10 时不应后台分析");
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p codex-plus-core --lib -- vision::tests::phase2 2>&1 | tail -10`
Expected: FAIL -- `bg_config_opt = None`，后台不执行，深层 URL 未写入缓存

- [ ] **Step 3: 实现 background_analyze_and_cache 函数**

在 `call_vlm_batch` 函数之后（约 840 行，`// ── Description injection ──` 之前）新增：

```rust
// ── Phase 2 后台分析 ────────────────────────────────────────────────────

/// 后台分析图片并写入缓存（不注入到消息中）。
/// 失败静默跳过，缓存保持未命中状态供后续请求重试。
async fn background_analyze_and_cache(
    urls: &[String],
    config: &VlmConfig,
    client: &reqwest::Client,
) {
    if urls.is_empty() {
        return;
    }
    match call_vlm_batch(urls, TIER1_PROMPT, config, client, BATCH_MAX_ATTEMPTS).await {
        Ok(descs) => {
            for (url, desc) in urls.iter().zip(descs.iter()) {
                let desc = truncate_char_safe(desc, DESC_MAX_CHARS);
                cache_put(url_hash(url), desc);
            }
        }
        Err(_) => {
            // 后台失败 -> 静默跳过，缓存保持未命中。
        }
    }
}
```

- [ ] **Step 4: 恢复 bg_config_opt 收集逻辑**

替换 `let bg_config_opt: Option<(VlmConfig, Vec<String>)> = None;`（约 1171 行）为从 main 迁移的收集逻辑，适配 CacheKey：

```rust
    // 6. Phase 2 后台准备：在 strip 之前收集未缓存的 URL 列表。
    // Phase 2 仅当 X > 10 时触发，分析 50 轮深度内未缓存的图片，写入缓存供后续请求使用。
    let bg_config_opt: Option<(VlmConfig, Vec<String>)> = if x_budget > 10 {
        let bg_target = x_budget.saturating_sub(historical_injected);
        if bg_target > 0 {
            let mut bg_urls: Vec<String> = Vec::new();
            // 6a. 黄金窗口中未被 Phase 1 覆盖的未缓存 URL（N > cap 场景）。
            for (_, (msg_idx, urls)) in all_image_msgs.iter().enumerate() {
                if Some(*msg_idx) == current_round_msg_idx || *msg_idx < golden_user_cutoff {
                    continue;
                }
                if bg_urls.len() >= bg_target {
                    break;
                }
                for url in urls {
                    if bg_urls.len() >= bg_target {
                        break;
                    }
                    let key = url_hash(url);
                    if !cache_contains(&key) {
                        bg_urls.push(url.clone());
                    }
                }
            }
            // 6b. 深层历史中未缓存的 URL（推进到 50 轮边界）。
            if bg_urls.len() < bg_target {
                for (msg_idx, urls) in all_image_msgs.iter() {
                    if Some(*msg_idx) == current_round_msg_idx || *msg_idx >= golden_user_cutoff {
                        continue;
                    }
                    if bg_urls.len() >= bg_target {
                        break;
                    }
                    for url in urls {
                        if bg_urls.len() >= bg_target {
                            break;
                        }
                        let key = url_hash(url);
                        if !cache_contains(&key) {
                            bg_urls.push(url.clone());
                        }
                    }
                }
            }
            if !bg_urls.is_empty() {
                Some((vlm_config.clone(), bg_urls))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
```

- [ ] **Step 5: 恢复 tokio::spawn 调用**

替换 `let _ = bg_config_opt;`（约 1229 行）为：

```rust
    // 10. Phase 2 后台：异步分析未缓存图片写入缓存（X > 10 时触发）。
    if let Some((bg_config, bg_urls)) = bg_config_opt {
        let bg_client = client.clone();
        tokio::spawn(async move {
            let n = bg_urls.len();
            let _ = background_analyze_and_cache(&bg_urls, &bg_config, &bg_client).await;
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "vlm_phase2_done",
                json!({"urls_analyzed": n}),
            );
        });
    }
```

- [ ] **Step 6: 运行测试验证通过**

Run: `cargo test -p codex-plus-core --lib -- vision::tests::phase2 2>&1 | tail -10`
Expected: 2 passed

验证现有测试不回归：
Run: `cargo test -p codex-plus-core --lib -- vision::tests --test-threads=1 2>&1 | tail -5`
Expected: 全部通过

- [ ] **Step 7: Commit**

```bash
git add crates/codex-plus-core/src/vision.rs
git commit -m "fix(vision): 恢复 Phase 2 后台分析，深层历史图写入缓存 (R3)"
```

---

### Task 4: R4 function_call_output 图片处理

**Files:**
- Modify: `crates/codex-plus-core/src/vision.rs` -- 新增 `extract_data_urls`、`strip_data_urls_in_messages`、`analyze_data_urls_in_messages`
- Modify: `crates/codex-plus-core/src/protocol_proxy.rs:846-893` -- Strip/Vlm 分支接入

**Interfaces:**
- Consumes: `call_vlm_batch`（Task 2）、`cache_put`/`url_hash`（Task 1）、`TIER1_PROMPT`、`truncate_char_safe`
- Produces: `fn extract_data_urls(&str) -> Vec<(usize, usize, String)>`；`fn strip_data_urls_in_messages(&mut [Value]) -> usize`；`async fn analyze_data_urls_in_messages(&mut [Value], &VlmConfig, &reqwest::Client) -> usize`

- [ ] **Step 1: 写失败测试--extract_data_urls 识别 + 边缘场景**

在 `mod tests` 中新增（放在 `count_images` 测试附近，约 1700 行）：

```rust
    #[test]
    fn extract_data_urls_finds_single_image() {
        let text = "data:image/png;base64,iVBORw0KGgo=";
        let urls = extract_data_urls(text);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].2, "data:image/png;base64,iVBORw0KGgo=");
    }

    #[test]
    fn extract_data_urls_finds_url_in_mixed_text() {
        let text = "result: data:image/png;base64,abc123== done";
        let urls = extract_data_urls(text);
        assert_eq!(urls.len(), 1);
        assert!(urls[0].2.starts_with("data:image/png;base64,abc123"));
        // 前后有文本
        assert_eq!(&text[..urls[0].0], "result: ");
        assert_eq!(&text[urls[0].1..], " done");
    }

    #[test]
    fn extract_data_urls_finds_multiple_urls() {
        let text = "a:data:image/png;base64,AAA b:data:image/jpeg;base64,BBB";
        let urls = extract_data_urls(text);
        assert_eq!(urls.len(), 2);
        assert!(urls[0].2.contains("png"));
        assert!(urls[1].2.contains("jpeg"));
    }

    #[test]
    fn extract_data_urls_ignores_non_image_data_url() {
        let text = "data:text/plain;base64,hello";
        let urls = extract_data_urls(text);
        assert_eq!(urls.len(), 0, "data:text/plain 不是 image/，不应识别");
    }

    #[test]
    fn extract_data_urls_returns_empty_for_no_url() {
        let urls = extract_data_urls("just plain text, no images here");
        assert!(urls.is_empty());
    }

    #[test]
    fn strip_data_urls_in_messages_replaces_with_placeholder() {
        let mut messages = vec![serde_json::json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "content": "view_image result: data:image/png;base64,iVBORw0KGgo="
        })];
        let n = strip_data_urls_in_messages(&mut messages);
        assert_eq!(n, 1);
        let content = messages[0]["content"].as_str().unwrap();
        assert!(content.contains("[图片已省略]"));
        assert!(!content.contains("base64"));
    }

    #[test]
    fn strip_data_urls_handles_responses_function_call_output() {
        let mut messages = vec![serde_json::json!({
            "type": "function_call_output",
            "call_id": "call_1",
            "output": "data:image/png;base64,iVBOR="
        })];
        let n = strip_data_urls_in_messages(&mut messages);
        assert_eq!(n, 1);
        let output = messages[0]["output"].as_str().unwrap();
        assert!(output.contains("[图片已省略]"));
    }

    #[test]
    fn strip_data_urls_noop_for_no_data_urls() {
        let mut messages = vec![serde_json::json!({
            "role": "tool", "content": "just text output, no images"
        })];
        let n = strip_data_urls_in_messages(&mut messages);
        assert_eq!(n, 0);
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p codex-plus-core --lib -- vision::tests::extract_data_urls vision::tests::strip_data_urls 2>&1 | tail -10`
Expected: 编译失败（函数未定义）

- [ ] **Step 3: 实现 extract_data_urls + strip_data_urls_in_messages**

在 vision.rs `// ── URL collection ──` 区块之后（约 390 行 `collect_urls` 之后）新增：

```rust
// ── Data URL extraction (for tool output images) ─────────────────────

/// 提取字符串中所有 base64 图片 data URL，返回 (start, end, url) 列表。
/// 手动解析（不引入 regex 依赖），匹配模式：data:image/{subtype};base64,{base64数据}
fn extract_data_urls(text: &str) -> Vec<(usize, usize, String)> {
    let prefix = "data:image/";
    let mut results = Vec::new();
    let mut search_from = 0;
    while search_from < text.len() {
        let Some(rel_start) = text[search_from..].find(prefix) else {
            break;
        };
        let abs_start = search_from + rel_start;
        let rest = &text[abs_start..];
        let Some(sep_pos) = rest.find(";base64,") else {
            search_from = abs_start + prefix.len();
            continue;
        };
        let data_start = abs_start + sep_pos + ";base64,".len();
        let data_end = text[data_start..]
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '+' && c != '/' && c != '=' && !c.is_whitespace())
            .map(|p| data_start + p)
            .unwrap_or(text.len());
        let url = text[abs_start..data_end].to_string();
        results.push((abs_start, data_end, url));
        search_from = data_end;
    }
    results
}

/// Strip 模式：把 tool/function_call_output 消息中的 data URL 替换为占位符。
/// 检查 output 字段（Responses function_call_output）和 content 字段（Chat tool 消息）。
fn strip_data_urls_in_messages(messages: &mut [Value]) -> usize {
    let mut total = 0;
    for msg in messages.iter_mut() {
        for key in &["output", "content"] {
            let Some(text) = msg.get(key).and_then(Value::as_str).map(String::from) else {
                continue;
            };
            let urls = extract_data_urls(&text);
            if urls.is_empty() {
                continue;
            }
            let mut new_text = text;
            // 从后往前替换，避免位置偏移
            for (start, end, _) in urls.iter().rev() {
                new_text.replace_range(*start..*end, "[图片已省略]");
                total += 1;
            }
            if let Some(field) = msg.get_mut(key) {
                *field = Value::String(new_text);
            }
        }
    }
    total
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p codex-plus-core --lib -- vision::tests::extract_data_urls vision::tests::strip_data_urls 2>&1 | tail -10`
Expected: 7 passed

- [ ] **Step 5: 写失败测试--analyze_data_urls_in_messages（Vlm 模式）**

在 `mod tests` 中新增：

```rust
    /// Vlm 模式：tool 消息含 data URL -> VLM 分析 + 描述替换 + base64 删除。
    #[tokio::test]
    async fn analyze_data_urls_replaces_with_vlm_description() {
        cache_clear_for_tests();
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "这是一张截图"}}]
            })))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "k".into(), model: "m".into(), base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let mut messages = vec![serde_json::json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "content": "view_image: data:image/png;base64,iVBORw0KGgoAAAANS"
        })];

        let n = analyze_data_urls_in_messages(&mut messages, &config, &client).await;
        assert_eq!(n, 1, "应处理 1 张 data URL 图片");
        let content = messages[0]["content"].as_str().unwrap();
        assert!(content.contains("这是一张截图"), "描述应替换 data URL: {content}");
        assert!(!content.contains("base64"), "base64 本体应被删除: {content}");
    }

    /// VLM 失败 -> fail-open 替换为失败提示（不保留 base64）。
    #[tokio::test]
    async fn analyze_data_urls_failopen_on_vlm_error() {
        cache_clear_for_tests();
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "k".into(), model: "m".into(), base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let mut messages = vec![serde_json::json!({
            "role": "tool",
            "content": "data:image/png;base64,iVBOR="
        })];

        let n = analyze_data_urls_in_messages(&mut messages, &config, &client).await;
        assert_eq!(n, 1);
        let content = messages[0]["content"].as_str().unwrap();
        assert!(content.contains("图片描述失败"), "应注入失败提示: {content}");
        assert!(!content.contains("base64"), "base64 不应残留: {content}");
    }

    /// 无 data URL 时不影响原文本。
    #[tokio::test]
    async fn analyze_data_urls_noop_for_plain_text() {
        cache_clear_for_tests();
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "k".into(), model: "m".into(), base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let mut messages = vec![serde_json::json!({
            "role": "tool", "content": "plain text output, no images"
        })];
        let n = analyze_data_urls_in_messages(&mut messages, &config, &client).await;
        assert_eq!(n, 0);
        assert_eq!(messages[0]["content"], "plain text output, no images");
    }
```

- [ ] **Step 6: 运行测试验证失败**

Run: `cargo test -p codex-plus-core --lib -- vision::tests::analyze_data_urls 2>&1 | tail -10`
Expected: 编译失败（函数未定义）

- [ ] **Step 7: 实现 analyze_data_urls_in_messages**

在 `strip_data_urls_in_messages` 之后新增：

```rust
/// Vlm 模式：调 VLM 分析 tool/function_call_output 消息中的 data URL，
/// 把描述替换回字符串，base64 本体被删除。VLM 失败时 fail-open 替换为失败提示。
/// 返回处理的图片数。
async fn analyze_data_urls_in_messages(
    messages: &mut [Value],
    vlm_config: &VlmConfig,
    client: &reqwest::Client,
) -> usize {
    let mut total = 0;
    for msg in messages.iter_mut() {
        for key in &["output", "content"] {
            let Some(text) = msg.get(key).and_then(Value::as_str).map(String::from) else {
                continue;
            };
            let urls = extract_data_urls(&text);
            if urls.is_empty() {
                continue;
            }
            let url_strings: Vec<String> = urls.iter().map(|(_, _, u)| u.clone()).collect();
            match call_vlm_batch(&url_strings, TIER1_PROMPT, vlm_config, client, BATCH_MAX_ATTEMPTS).await {
                Ok(descs) => {
                    let mut new_text = text;
                    // 从后往前替换，避免位置偏移
                    for ((start, end, url), desc) in urls.iter().rev().zip(descs.iter().rev()) {
                        let desc = truncate_char_safe(desc, DESC_MAX_CHARS);
                        cache_put(url_hash(url), desc.clone());
                        let replacement = format!("[图片描述] {desc}");
                        new_text.replace_range(*start..*end, &replacement);
                        total += 1;
                    }
                    if let Some(field) = msg.get_mut(key) {
                        *field = Value::String(new_text);
                    }
                }
                Err(_) => {
                    // fail-open：替换为失败提示（不保留 base64，避免浪费 token）
                    let mut new_text = text;
                    for (start, end, _) in urls.iter().rev() {
                        new_text.replace_range(*start..*end, "[图片描述失败，视觉模型调用失败]");
                        total += 1;
                    }
                    if let Some(field) = msg.get_mut(key) {
                        *field = Value::String(new_text);
                    }
                }
            }
        }
    }
    total
}
```

- [ ] **Step 8: 运行测试验证通过**

Run: `cargo test -p codex-plus-core --lib -- vision::tests::analyze_data_urls 2>&1 | tail -10`
Expected: 3 passed

- [ ] **Step 9: 在 protocol_proxy.rs 接入 R4**

在 `protocol_proxy.rs` 的 `upstream_request_parts` 图片处理区块（约 846-893 行），Strip 和 Vlm 分支新增 tool 图片处理。

Strip 分支（在 `strip_images_only_counted` + `inject_cannot_see_note_slice` 之后，`for key` 循环内或之后）：

```rust
            ImageHandling::Strip => {
                for key in &["messages", "input"] {
                    if let Some(arr) = body.get_mut(key).and_then(Value::as_array_mut) {
                        let n = crate::vision::strip_images_only_counted(arr);
                        // 处理 tool 消息中的 data URL（R4）
                        let n_data = crate::vision::strip_data_urls_in_messages(arr);
                        let total = n + n_data;
                        if total > 0 {
                            let _ = crate::diagnostic_log::append_diagnostic_log(
                                "protocol_proxy.vl_strip",
                                json!({"reason": "strip_mode", "n": total, "n_data_url": n_data}),
                            );
                            crate::vision::inject_cannot_see_note_slice(
                                arr, total, "未配置视觉模型中转，图片已剥离",
                            );
                        }
                    }
                }
            }
```

Vlm 分支（在 `strip_image_blocks` 之后）：

```rust
            ImageHandling::Vlm => {
                if !relay.vlm_api_key.is_empty()
                    && !relay.vlm_model.is_empty()
                    && !relay.vlm_base_url.is_empty()
                {
                    let vlm_config = crate::vision::VlmConfig {
                        api_key: relay.vlm_api_key.clone(),
                        model: relay.vlm_model.clone(),
                        base_url: relay.vlm_base_url.clone(),
                        protocol: relay.vlm_protocol,
                    };
                    let vlm_client = crate::http_client::proxied_client("")
                        .context("failed to build VLM HTTP client")?;

                    for key in &["messages", "input"] {
                        if let Some(arr) = body.get_mut(key).and_then(Value::as_array_mut) {
                            crate::vision::strip_image_blocks(
                                arr, &vlm_config, &relay.model_windows,
                                &relay.context_window, &model, &vlm_client,
                            ).await;
                            // 处理 tool 消息中的 data URL（R4）
                            let n_data = crate::vision::analyze_data_urls_in_messages(
                                arr, &vlm_config, &vlm_client,
                            ).await;
                            if n_data > 0 {
                                let _ = crate::diagnostic_log::append_diagnostic_log(
                                    "protocol_proxy.vl_tool_image",
                                    json!({"n_data_url": n_data}),
                                );
                            }
                        }
                    }
                }
            }
```

注意：`strip_data_urls_in_messages` 和 `analyze_data_urls_in_messages` 需要 `pub` 可见性。在 vision.rs 定义时加 `pub`。

- [ ] **Step 10: 运行全量 vision 测试验证**

Run: `cargo test -p codex-plus-core --lib -- vision::tests --test-threads=1 2>&1 | tail -5`
Expected: 全部通过

Run: `cargo test -p codex-plus-core --test protocol_proxy 2>&1 | tail -5`
Expected: 通过（含已有 strip_image_blocks 集成测试）

- [ ] **Step 11: Commit**

```bash
git add crates/codex-plus-core/src/vision.rs crates/codex-plus-core/src/protocol_proxy.rs
git commit -m "fix(vision): function_call_output 图片走 VLM 分析/Strip 替换，消除 base64 透传 (R4)"
```

---

### Task 5: 全量回归验证

**Files:**
- 无代码改动，仅运行验证

- [ ] **Step 1: Rust 全量测试**

Run: `cargo test --workspace --no-fail-fast 2>&1 | tail -20`
Expected: 全部通过（已知 pre-existing 失败除外：HTTP proxy 环境依赖测试）

- [ ] **Step 2: Rust 编译检查**

Run: `cargo build --workspace 2>&1 | tail -5`
Expected: 无编译错误

Run: `cargo build -p codex-plus-manager 2>&1 | tail -5`
Expected: 通过

- [ ] **Step 3: 前端编译检查**

Run: `cd apps/codex-plus-manager && npx tsc --noEmit 2>&1 | tail -5`
Expected: 0 errors

- [ ] **Step 4: 检查 BATCH_SIZE 不再是 dead_code**

Run: `cargo build -p codex-plus-core 2>&1 | rg -i 'BATCH_SIZE|dead_code|unused'`
Expected: 无 warning（BATCH_SIZE 现在被 call_vlm_batch 引用）

- [ ] **Step 5: 检查 bg_config_opt 占位代码已移除**

Run: `rg -n 'let _ = bg_config_opt|bg_config_opt.*None' crates/codex-plus-core/src/vision.rs`
Expected: 无匹配（占位代码已替换为实际逻辑）

- [ ] **Step 6: 检查 u64 缓存已移除**

Run: `rg -n 'HashMap<u64.*CacheEntry|fn cache_get\(key: u64\)|fn cache_put\(key: u64\)' crates/codex-plus-core/src/vision.rs`
Expected: 无匹配（已改为 CacheKey）
