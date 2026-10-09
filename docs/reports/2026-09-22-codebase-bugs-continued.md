# 当前代码库 bug 排查（2026-09-17 → 09-22 续查）

基线：`main` @ `b1ed92e5`（与首次排查同一 commit，期间 main 未前进）
本次补查范围：首次排查未逐行审的 8 个文件（+699 行）

**结论：新增 1 个已确认缺陷，另修正首次排查中一处过度推断。**

---

## 本次新确认的缺陷

### 缺陷 3：网关同时返回模型列表与错误码时被误判为成功

`model_catalog.rs:570` `interpret_models_response`，严重度**低–中**。

**证据级别：★★★ 已用 crate 内测试跑出**（临时探针，验证后已还原）

```
输入: {"code":401,"msg":"令牌已过期","success":false,"data":[{"id":"glm-5.3"}]}
输出: models=["glm-5.3"]  status={"status":"ok","models":1}
```

网关明确报告令牌过期，函数却返回 **`status: "ok"`**。

**根因**：判定顺序把「有模型」放在「有业务错误」之前：

```rust
let models = unique_strings(parse_model_payload(&payload));
if !models.is_empty() {
    safe_source["status"] = json!("ok");        // ← 提前返回，业务错误码不再被检查
    return (models, safe_source);
}
if let Some(message) = business_error_message(&payload) {   // ← 永远走不到
    return failed_source(safe_source, message);
}
```

`business_error_message` 本身写得是对的（`success` 字段优先，其次 `code != 0/200`，再次 `error` 非空），但**只在 models 为空时才被调用**。

**为什么算缺陷**：两个问题叠加——(1) 用户看到「获取成功」而实际令牌已失效，排障时会往错误方向找；(2) 若网关在错误态下回了个占位列表，这个列表会被当成可用模型写进 catalog。

**修法**：把 `business_error_message` 的检查提到 `models` 判定之前。它的测试已覆盖 `{"code":200,"success":true,"data":[]}` 这类成功信封会被忽略，提前检查不会误伤正常响应。

**触发条件**：需要网关同时返回 `data` 与错误码。智谱在 key 缺失时返回的是「无 data + 错误码」（因此 #2190 的修复对那个场景有效）；这个组合更像「部分失败」或网关行为不一致的情况。

---

## 修正首次排查中的一处过度推断

首次排查我把 `native_browser` 的 `plain_path` 列为确认缺陷。**结论不变，但当时的证据描述需要收紧**：

- 已确认的部分：macOS `/var` 系统软链导致 19 个测试无法运行（实际发生，已用改测试夹具绕过）
- **未验证的部分**：Windows 上被重定向的 `TEMP`/`LOCALAPPDATA` 会让特性静默失效——这是我**从代码推演**的，没有 Windows 环境实证（只有类型检查）

我没有把推演当结论写进结论段，但两处并列陈述容易让人以为是同等强度的证据。特此分开。

---

## 本次审过、确认无缺陷的文件

| 文件 | 结论 |
|---|---|
| `protocol_proxy.rs` | `protocol_proxy_port()` 每次读环境变量；写入与检测两侧一致。`standard_openai_protocol` 的透传链路完整 |
| `connect/mod.rs` | `retrying` 状态只有一处消费者（`App.tsx:4022`），已正确纳入 running 列表 |
| `env_conflicts.rs` | `vars()` → `vars_os()` 容错非 UTF-8；只按名字过滤 + `sort`/`dedup`，不依赖 Windows 的大小写规范化 |
| `relay_environment.rs` | 同上模式 |
| `settings.rs` | `atomic_write_with` 的 `flush()` + 权限保留正确；两个新字段 serde 默认值正确 |
| `launcher.rs` | 端口错误分类的三条分支互斥且完整 |
| `watcher.rs` | `LauncherExitSnapshot` 用 `process_birth_id` 防 PID 复用，`wait_for_exit` 超时后不强杀 |
| `status.rs` / `model_suffix.rs` | 改动极小，语义正确 |
| `native_browser.rs` 的 `pin_parents` | TOCTOU 防护正确：`share_mode` 排除 `FILE_SHARE_DELETE` 钉住父目录，`OPEN_REPARSE_POINT` 确保检查链接本身而非目标 |
| `App.tsx` 事件监听 | `unlisten` 在 `finally` 释放，无泄漏 |

---

## 未修的既有缺陷（首次排查发现，仍在）

1. **`clear_relay_config_to_home_with_auth` 残留中转配置**（issue #2216，已独立复现）——切回官方后 base_url / provider 段 / model 全残留，请求仍发往中转站。本次补查确认了**同族不一致**：`relay_config.rs:3742` 在切换 provider 时**会**删 `custom` 表，而清理函数不删。
2. **`plain_path` 祖先链校验过严**（#2208 引入）——见上「修正」段。

---

## 覆盖边界

| 范围 | 状态 |
|---|---|
| Rust 测试套件 | ✅ 42 target（本次未重跑，代码未变） |
| 前端测试 | ✅ 234 全过 |
| 本轮全部源码改动 | ✅ 已逐行读完（约 +6451/-579 行） |
| 注入脚本运行时行为 | ❌ 仍只做静态阅读 |
| Tauri 端到端 / 打包流程 | ❌ 未做真实运行验证 |
| 912 个 open issue | ❌ 未逐条复核 |
| Windows 实机 | ❌ 无环境 |

**三个缺陷的证据强度**：#2216 残留（独立可执行复现） > 缺陷 3（crate 内测试跑出） > `plain_path` 的 Windows 推演（纯静态）。
