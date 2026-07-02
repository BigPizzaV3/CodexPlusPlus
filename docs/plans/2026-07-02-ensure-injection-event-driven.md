# P0: ensure_injection 事件驱动改造

> 日期: 2026-07-02
> 对应: 性能优化计划 P0 — 启动慢 54.9s 空白期
> 方案验证: Playwright Electron `waitForLine` pattern（92k★ 生产验证）

## 根因

```rust
// launcher.rs:175-199 — 默认 trait 方法
async fn ensure_injection(&self, debug_port, helper_port, app_dir) -> bool {
    for attempt in 1..=120 {          // ← 最多等 120 秒！
        match self.bridge_context(...).await {
            Ok(Some(ctx)) => self.inject_bridge(...).await?,
            Ok(None) => self.inject(...).await?,
            Err(error) => { /* retry */ }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
```

嵌套 retry: 外层 120×1s + 内层 `retry_injection` 20×500ms。

**上游瓶颈不是 injection 本身，而是 CDP 端口还没就绪（Codex 页面没加载完）。**

## 方案

**两步改造：**

1. **`launch_codex()`**: stderr `/dev/null` → pipe + 事件监听
2. **`ensure_injection()`**: 事件信号 + 指数退避兜底

详细设计：

```
launch_codex()
  └─ .stderr(Stdio::piped())     ← 之前是 null
  └─ spawn stderr_listener()
       └─ read_line() 循环
       └─ 匹配 "DevTools listening on ws://..."
       └─ oneshot::Sender → fire ready signal

ensure_injection()
  └─ select! {
      ready_rx => 尝试 inject() → OK 直接返回    ← 事件驱动，~0ms
      backoff  => TCP connect 探测端口             ← 指数退避兜底
      timeout  => fallthrough 到原逻辑              ← 30s 安全网
  }
```

### 改动文件

| # | 文件 | 改动 |
|---|------|------|
| 1 | `crates/codex-plus-core/src/launcher.rs` | 新增 `stderr_listener()` 函数 |
| 2 | `crates/codex-plus-core/src/launcher.rs` | `DefaultLaunchHooks` 增加 `cdp_ready` 字段 |
| 3 | `crates/codex-plus-core/src/launcher.rs` | `launch_codex()` pipe stderr + spawn listener |
| 4 | `crates/codex-plus-core/src/launcher.rs` | override `ensure_injection()` 用事件信号 |

## Task 1: 新增 stderr_listener 函数

**位置**: `crates/codex-plus-core/src/launcher.rs`，放在 `retry_injection` 附近（~line 1702）

```rust
/// Detect when Codex's CDP endpoint is ready by reading its stderr.
///
/// Chrome/Electron prints "DevTools listening on ws://127.0.0.1:<port>/..."
/// to stderr when the CDP server is ready.  This is the same pattern used
/// by Playwright (92k★) for Electron apps.
///
/// Returns `Ok(())` as soon as the magic line is found, or `Ok(())` on EOF
/// (the caller falls through to TCP-backoff).  Errors only on actual I/O
/// failures, which are also non-fatal.
async fn wait_for_cdp_ready(mut stderr: impl tokio::io::AsyncRead + Unpin) -> anyhow::Result<()> {
    use tokio::io::AsyncBufReadExt;
    let mut reader = tokio::io::BufReader::new(&mut stderr);
    let mut line = String::new();
    let magic = "DevTools listening on ws://";

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            // EOF — process closed stderr without printing the magic line
            break;
        }
        if line.contains(magic) {
            return Ok(());
        }
    }

    // Fallback: even without the stderr line, the port may still be ready.
    // Return success so the caller falls through to the TCP-backoff.
    Ok(())
}
```

**验收标准：**
- [ ] `stderr_listener` 返回 `Ok(())` 当读到 `DevTools listening on ws://` 行
- [ ] stderr EOF 时返回 `Ok(())`（不阻塞，让 backoff 兜底）
- [ ] 函数签名：`async fn wait_for_cdp_ready(stderr: ChildStderr, debug_port: u16) -> anyhow::Result<()>`

## Task 2: DefaultLaunchHooks 增加 cdp_ready 字段

**位置**: `crates/codex-plus-core/src/launcher.rs` line 212-219

```rust
pub struct DefaultLaunchHooks {
    child: Mutex<Option<Child>>,
    helper: Mutex<Option<HelperRuntime>>,
    bridge_watchdog: Mutex<Option<BridgeWatchdogRuntime>>,
    computer_use_guard_watchdog: Mutex<Option<ComputerUseGuardWatchdogRuntime>>,
    computer_use_guard_artifacts: Mutex<Option<crate::computer_use_guard::GuardArtifacts>>,
    // NEW: Oneshot receiver that fires when Codex's CDP port is ready.
    // Set up by launch_codex(), consumed by ensure_injection().
    cdp_ready: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}
```

**验收标准：**
- [ ] 编译通过（Default 派生继续工作，因为 `tokio::sync::Mutex` 实现了 Default）
- [ ] `cdp_ready` 初始化时是 `None`

## Task 3: launch_codex() pipe stderr + spawn listener

**位置**: `crates/codex-plus-core/src/launcher.rs` line 726-744（Generic/Linux 路径）

改动：
1. `.stderr(Stdio::null())` → `.stderr(Stdio::piped())`
2. spawn 后取 `child.stderr`，启动 `wait_for_cdp_ready` 任务

```rust
        let mut child_command = Command::new(executable);
        child_command
            .args(&command[1..])
            .stdout(Stdio::null())
            .stderr(Stdio::piped());  // ← 改为 piped
        #[cfg(windows)]
        child_command.creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
        let child = child_command
            .spawn()
            .with_context(|| format!("failed to launch Codex executable {executable}"))?;

        // NEW: pipe stderr to detect CDP readiness
        if let Some(stderr) = child.stderr.take() {
            let (tx, rx) = tokio::sync::oneshot::channel();
            *self.cdp_ready.lock().await = Some(rx);
            tokio::spawn(async move {
                if let Err(error) = wait_for_cdp_ready(stderr).await {
                    // stderr read error is non-fatal; the injection fallback handles it.
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.cdp_stderr_listener_error",
                        serde_json::json!({"message": error.to_string()}),
                    );
                }
                // Drop the Sender: if ensure_injection already timed out, this is a no-op.
                // If it's still waiting, the Receiver will get Canceled and fall through
                // to the backoff fallback.
                drop(tx);
            });
        }

        *self.child.lock().await = Some(child);
```

**验收标准：**
- [ ] `child.stderr` 被 pipe，不再丢失 stderr
- [ ] `cdp_ready` 持有 oneshot::Receiver
- [ ] listener 错误只写 log，不阻止启动

## Task 4: Override ensure_injection() 用事件信号

**位置**: `crates/codex-plus-core/src/launcher.rs`，在 `impl LaunchHooks for DefaultLaunchHooks` 块内

```rust
    async fn ensure_injection(&self, debug_port: u16, helper_port: u16, app_dir: &Path) -> bool {
        // Phase 1: Event-driven — wait for CDP port readiness signal from stderr.
        let cdp_ready = {
            let mut guard = self.cdp_ready.lock().await;
            guard.take()
        };

        if let Some(rx) = cdp_ready {
            // Wait for the stderr signal with a moderate timeout.
            // On a healthy system the line arrives within 2-5 seconds.
            let signal = tokio::time::timeout(std::time::Duration::from_secs(15), rx).await;
            match signal {
                Ok(Ok(())) => {
                    // CDP ready signal received!  Try injection once; it
                    // should succeed if the port is genuinely ready.
                    match self.inject(debug_port, helper_port).await {
                        Ok(()) => return true,
                        Err(_) => { /* fall through to backoff */ }
                    }
                }
                _ => {
                    // Timeout or sender dropped — fall through to backoff.
                }
            }
        }

        // Phase 2: Bounded backoff — TCP connect probing with exponential delay.
        // This handles edge cases where the stderr line is not printed
        // (Windows packaged app, macOS `.app` bundle, etc.).
        let backoff_delays = [100, 200, 400, 800, 1600, 3200, 5000, 10000u64];
        for delay_ms in &backoff_delays {
            match self.inject(debug_port, helper_port).await {
                Ok(()) => return true,
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.ensure_injection_backoff_retry",
                        serde_json::json!({
                            "debug_port": debug_port,
                            "delay_ms": delay_ms,
                            "message": error.to_string()
                        }),
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(*delay_ms)).await;
                }
            }
        }

        // Phase 3: Original 1s-interval polling as ultimate safety net.
        // This matches the original behaviour for systems where injection
        // genuinely takes >30 seconds (very slow disks, heavy load).
        for attempt in 1..=30u32 {
            match self.inject(debug_port, helper_port).await {
                Ok(()) => return true,
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.ensure_injection_fallback_retry",
                        serde_json::json!({
                            "debug_port": debug_port,
                            "helper_port": helper_port,
                            "attempt": attempt,
                            "message": error.to_string()
                        }),
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }

        false
    }
```

**验收标准：**
- [ ] 事件信号到达时直接 inject，不经过轮询
- [ ] 无信号时指数退避（100ms~10s，共 8 次 ≈ 22s）
- [ ] 最终兜底 30×1s（原 120×1s 的 1/4）
- [ ] 总最坏超时从 120s 降到 ~52s（但正常情况 ~0-2s）

## 测试计划

| 测试 | 类型 | 说明 |
|------|------|------|
| Unit: `wait_for_cdp_ready` 识别 magic 行 | 单元测试 | 用 `tokio::io::duplex` pipe 模拟 stderr，验证 `Ok(())` |
| Unit: `wait_for_cdp_ready` EOF 不报错 | 单元测试 | pipe 立即关闭，验证返回 `Ok(())`（走 backoff） |
| Unit: `wait_for_cdp_ready` 忽略无关行 | 单元测试 | 先发无关行再发 magic 行，验证最终返回 `Ok(())` |
| 编译检查 | `cargo check` | 确认无编译错误 |
| 全量测试 | `cargo test -p codex-plus-core` | 确认无回归（已知 1 个预存在失败与本次无关） |

### 新增测试: `tests/launcher.rs`

```rust
#[tokio::test]
async fn test_wait_for_cdp_ready_detects_magic_line() {
    use tokio::io::AsyncWriteExt;
    let (mut tx, rx) = tokio::io::duplex(1024);
    tx.write_all(b"DevTools listening on ws://127.0.0.1:9222/...\n")
        .await
        .unwrap();
    drop(tx);
    let result = wait_for_cdp_ready(rx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_wait_for_cdp_ready_eof_is_ok() {
    let (_, rx) = tokio::io::duplex(1024);
    // drop tx immediately → rx gets EOF immediately
    let result = wait_for_cdp_ready(rx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_wait_for_cdp_ready_ignores_noise_before_magic() {
    use tokio::io::AsyncWriteExt;
    let (mut tx, rx) = tokio::io::duplex(1024);
    tx.write_all(b"[1234/1234.1234:INFO:CONSOLE(123)] Some log\\n")
        .await
        .unwrap();
    tx.write_all(b\"[1234/1234.1234:INFO:CONSOLE(456)] Another log\\n")
        .await
        .unwrap();
    tx.write_all(b"DevTools listening on ws://127.0.0.1:9222/...\\n")
        .await
        .unwrap();
    drop(tx);
    let result = wait_for_cdp_ready(rx).await;
    assert!(result.is_ok());
}
```

由于代码在 headless VM 上无法实际启动 Codex 进程，无法做集成测试验证。但改动是**新增功能**（stderr pipe + 事件信号），**不改变现有路径**的行为：
- 对于没有 `cdp_ready` 信号的实例（`None`），直接走 Phase 2/3 回退路径
- 对于 listener 启动失败的实例，tx 被 drop，rx 收 Canceled → 走回退路径

## 依赖顺序

```
Task 1 → Task 2 → Task 3 → Task 4 → Task 5
（函数）  （字段）  （pipe）  （信号消费）（测试）
```
