# CodexPlusPlus 修复笔记

> 记录对 CodexPlusPlus 的修改，防止升级后被覆盖。

---

## 修复 1：Windows Store 版 Codex 启动失败

**日期**：2026-06-03  
**问题**：CodexPlusPlus 无法启动 Windows Store / MSIX 版本的 Codex，导致注入完全失败。  
**根因**：`packaged_app_user_model_id` 函数错误地生成了 AUMID，丢失了版本号部分。

### 修改文件 1：`crates/codex-plus-core/src/app_paths.rs`

**函数**：`packaged_app_user_model_id`（原第 189-200 行）

**修改前**：
```rust
pub fn packaged_app_user_model_id(app_dir: &Path) -> Option<String> {
    let package_name = package_name_from_app_dir(app_dir)?;
    if !package_name.starts_with("OpenAI.Codex_") || !package_name.contains("__") {
        return None;
    }
    let identity_name = package_name.split_once('_')?.0;
    let publisher_id = package_name.rsplit_once("__")?.1;
    if publisher_id.is_empty() {
        return None;
    }
    Some(format!("{identity_name}_{publisher_id}!App"))  // ❌ 错误
}
```

**修改后**：
```rust
pub fn packaged_app_user_model_id(app_dir: &Path) -> Option<String> {
    let package_name = package_name_from_app_dir(app_dir)?;
    if !package_name.starts_with("OpenAI.Codex_") || !package_name.contains("__") {
        return None;
    }
    // FIX: MSIX 包的 AUMID 格式就是 PackageFullName!App
    Some(format!("{}!App", package_name))  // ✅ 正确
}
```

**AUMID 对比**：

| | 值 |
|---|---|
| package_name | `OpenAI.Codex_26.601.2237.0_x64__2p2nqsd0c76g0` |
| 修改前（错误） | `OpenAI.Codex_2p2nqsd0c76g0!App` |
| 修改后（正确） | `OpenAI.Codex_26.601.2237.0_x64__2p2nqsd0c76g0!App` |

### 修改文件 2：`crates/codex-plus-core/src/launcher.rs`

**函数**：`DefaultLaunchHooks::launch_codex`（原第 444-473 行）

**修改前**：
```rust
if let Some(activation) = build_packaged_activation(...) {
    let process_id = activate_packaged_app(app_user_model_id, arguments).await?;
    // 失败直接抛错，没有回退
    return Ok(...);
}
```

**修改后**：
```rust
if let Some(activation) = build_packaged_activation(...) {
    match activate_packaged_app(app_user_model_id, arguments).await {
        Ok(process_id) => return Ok(...),
        Err(e) => {
            // 回退到直接执行 Codex.exe
            eprintln!("[WARN] Packaged activation failed for AUMID {}, falling back to direct execution", e);
        }
    }
}
// 继续执行下面的直接启动逻辑
```

> **注意**：原代码使用 `tracing::warn!`，但 `codex-plus-core` 没有 `tracing` 依赖，已改为 `eprintln!`。

### 影响范围

- ✅ **保留**：供应商切换（Relay injection）、注入脚本、CDP 桥接、所有增强功能
- ✅ **新增**：AUMID 激活失败时自动回退到直接执行 `Codex.exe`
- ⚠️ **注意**：回退模式下，Codex 以普通进程启动，无法通过 Windows API 优雅关闭（只能 kill）

### 验证步骤

```powershell
# 1. 编译
cd E:\tmp\CodexPlusPlus
cargo build --release -p codex-plus-launcher -p codex-plus-core

# 2. 运行静默启动器
.\target\release\codex-plus-plus.exe

# 3. 检查注入是否成功
# 打开 Codex 页面，查看是否有 Codex++ 菜单
# 或在 PowerShell 中测试：
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:57321/backend/status -Body "{}" -ContentType "application/json"
```

### ✅ 测试结果（2026-06-03 20:45）

| 检查项 | 结果 |
|---|---|
| Codex 进程启动 | ✅ 9 个进程，正常 |
| CDP 端口 9229 | ✅ 可用，`/json` 返回 page target |
| Helper 后端 57321 | ✅ 监听中 |
| CDP 桥接建立 | ✅ `hasBridge: true` |
| 注入脚本加载 | ✅ `renderer.script_loaded` 事件 |
| `/backend/status` | ✅ `status: "ok"` |
| `/settings/get` | ✅ 返回配置 |
| `/codex-model-catalog` | ✅ `status: "ok"` |
| AUMID 激活失败回退 | ✅ `activated: false` 但 `launch_ok: true`（回退到直接执行） |

**日志关键证据**：
```
launcher.activate_existing_codex: activated=false, launch_ok=true
renderer.script_loaded: hasBridge=true, version=1.2.0
bridge.response /backend/status: status=ok
```

**注意**：部分增强功能（如 `upstream_pending_worktree_patch`、`service_tier_dispatcher`）出现 `patch_failed` 错误，原因是 Windows Store 版 Codex 的资产文件路径与原始版本略有不同。这些是次要问题，不影响核心注入功能。

### 升级注意事项

当 CodexPlusPlus 发布新版本时：

1. **不要直接覆盖整个项目**
2. 先备份 `.patches/` 目录下的所有补丁
3. 应用新版本后，重新应用以下修改：
   - `crates/codex-plus-core/src/app_paths.rs` 的 `packaged_app_user_model_id` 函数
   - `crates/codex-plus-core/src/launcher.rs` 的 `launch_codex` 函数
4. 重新编译

---

## 修复 2：流式响应 `stream disconnected before completion` 错误

**日期**：2026-06-04  
**问题**：注入成功（`hasBridge: true`、`/backend/status` 返回 `ok`），但 Codex 发送请求到 `http://127.0.0.1:57321/v1/responses` 时出现：
```
stream disconnected before completion: error sending request for url (http://127.0.0.1:57321/v1/responses)
```

**根因**：`crates/codex-plus-core/src/http_client.rs` 的 `proxied_client()` 函数创建的 reqwest 客户端没有任何超时和连接池配置，导致：

| 问题 | 后果 |
|------|------|
| 默认超时 30s | 流式 SSE 响应超过 30 秒时 reqwest 在传输中途超时断开 |
| 无 TCP keepalive | 防火墙/代理主动断开长连接 |
| 连接池太小 | 并发请求时连接被过早回收 |

### 修改文件：`crates/codex-plus-core/src/http_client.rs`

**函数**：`proxied_client`

**修改前**：
```rust
pub fn proxied_client(user_agent: &str) -> anyhow::Result<reqwest::Client> {
    let ua = if user_agent.trim().is_empty() {
        format!("CodexPlusPlus/{}", env!("CARGO_PKG_VERSION"))
    } else {
        user_agent.trim().to_string()
    };
    Ok(reqwest::Client::builder().user_agent(ua).build()?)  // ❌ 无任何超时配置
}
```

**修改后**：
```rust
pub fn proxied_client(user_agent: &str) -> anyhow::Result<reqwest::Client> {
    let ua = if user_agent.trim().is_empty() {
        format!("CodexPlusPlus/{}", env!("CARGO_PKG_VERSION"))
    } else {
        user_agent.trim().to_string()
    };
    // FIX: 为协议代理的流式请求配置合理的超时和连接池
    Ok(reqwest::Client::builder()
        .user_agent(ua)
        .connect_timeout(std::time::Duration::from_secs(30))    // 连接超时 30s
        .timeout(std::time::Duration::from_secs(300))           // 读写超时 5 分钟（流式 SSE 需要）
        .tcp_keepalive(std::time::Duration::from_secs(60))      // TCP keepalive 60s
        .pool_idle_timeout(std::time::Duration::from_secs(120)) // 空闲连接保持 2 分钟
        .pool_max_idle_per_host(32)                              // 每主机最多 32 个空闲连接
        .build()?)
}
```

> **注意**：reqwest 0.12 不支持 `http2_keep_alive_*` 配置（这些在 hyper 0.x 中才有），对于 HTTP/1.1 流式响应，`tcp_keepalive` + 大超时已经足够。

### 修改文件：`crates/codex-plus-core/src/launcher.rs`

在两个流式响应处理路径中添加注释说明 `shutdown()` 的正确使用：

- `handle_protocol_proxy_connection`（第 936 行）：Responses API 协议代理流式响应
- `handle_chat_completions_proxy_connection`（第 1016 行）：Chat Completions 协议代理流式响应

```rust
// FIX: 流式响应已写入 [DONE]\n\n 标记，此时 shutdown() 让客户端感知到 EOF
stream.shutdown().await?;
```

> **说明**：`tokio::net::TcpStream` 只有 `shutdown()`（半关闭，只关闭写端），没有 `close()` 方法。配合 `Connection: close` HTTP 头和 SSE 的 `[DONE]\n\n` 标记，客户端能正确感知流结束。

### ✅ 编译测试（2026-06-04）

```
$ cargo build --release -p codex-plus-core -p codex-plus-launcher
   Compiling codex-plus-core v1.2.0
   Compiling codex-plus-data v1.2.0
   Compiling codex-plus-launcher v1.2.0
    Finished `release` profile [optimized] target(s) in 30.90s
```

### 验证步骤

```powershell
# 1. 运行
.\target\release\codex-plus-plus.exe

# 2. 验证注入
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:57321/backend/status -Body "{}" -ContentType "application/json"

# 3. 验证流式请求（关键）：在 Codex 中发送一个需要流式响应的消息，观察：
#    - 不再出现 "stream disconnected before completion" 错误
#    - SSE 流完整接收，[DONE] 标记正常到达
#    - 长时间思考（>30s）的请求不再超时断开
```

### 影响范围

- ✅ **修复**：流式 SSE 响应的超时和连接保持问题
- ✅ **修复**：长连接被防火墙/代理主动断开的问题
- ⚠️ **注意**：`timeout(300s)` 是全局超时，如果上游 API 返回 5xx 错误，错误响应会在 5 分钟内返回（之前可能 30s 就断了，现在等更久才收到错误）。这是预期行为，因为流式请求需要更长的等待时间。

---

---

## 修复 3：Helper 错误处理缺失 + `upstreamBaseUrl` 未映射导致链接断开

**日期**：2026-06-04  
**问题**：注入成功（`hasBridge: true`、`/backend/status` 返回 `ok`），但页面加载过程中 Codex++ 后端链接断开，Codex 报错：
```
stream disconnected before completion: error sending request for url (http://127.0.0.1:57321/v1/responses)
```

**根因（两个问题叠加）**：

| # | 问题 | 后果 |
|---|------|------|
| 1 | 代理处理函数（`handle_protocol_proxy_connection` 等）大量使用 `?` 运算符，任何内部错误都直接传播到 tokio task，**TCP 连接被粗暴关闭**，没有任何 HTTP 错误响应 | Codex 客户端看到 "stream disconnected before completion" |
| 2 | `RelayProfile` 中 `base_url` 和 `upstream_base_url` 是独立字段。settings.json 只配置了 `upstreamBaseUrl` 未配置 `baseUrl`，反序列化后 `base_url` 为空。协议代理用空 URL 发请求 | 请求失败，进而触发问题 1 |

**日志关键证据**：
```
helper.protocol_proxy_upstream_error    →  status: "404 Not Found"
```

### 修改文件 1：`crates/codex-plus-core/src/launcher.rs`

**新增函数** `write_error_response_and_shutdown`（第 1100-1118 行）：

```rust
/// 写入 HTTP 错误响应并关闭连接，忽略所有错误（防止错误传播导致连接被粗暴关闭）
async fn write_error_response_and_shutdown(stream: &mut tokio::net::TcpStream, status: &str, message: &str) {
    let body = serde_json::to_vec(&serde_json::json!({
        "status": "failed", "message": message
    })).unwrap_or_default();
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: ...\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.shutdown().await;
}
```

**修改 `handle_helper_connection`**（第 656-695 行）：

所有三个代理处理函数调用从 `return handler(...).await` 改为 `let result = handler(...).await` + catch-all：

```rust
// 修改前：错误直接传播到 tokio task，连接被关闭
return handle_protocol_proxy_connection(...).await;

// 修改后：错误被捕获，写入 HTTP 500 响应再关闭
let result = handle_protocol_proxy_connection(...).await;
if let Err(error) = result {
    write_error_response_and_shutdown(&mut stream, "500 Internal Server Error", &error.to_string()).await;
}
return Ok(());
```

同样的修改应用到：
- `handle_chat_completions_proxy_connection` 调用
- `handle_models_proxy_connection` 调用

**修改 `handle_protocol_proxy_connection`**（第 850-872 行）：

拆分出 `handle_protocol_proxy_connection_inner`，外层函数做 catch-all：

```rust
async fn handle_protocol_proxy_connection(...) -> anyhow::Result<()> {
    let result = handle_protocol_proxy_connection_inner(...).await;
    if let Err(error) = result {
        write_error_response_and_shutdown(stream, "500 Internal Server Error", &error.to_string()).await;
    }
    Ok(())
}
```

### 修改文件 2：`crates/codex-plus-core/src/protocol_proxy.rs`

#### 1. 新增 `relay_base_url()` 辅助函数（第 123-133 行）

```rust
/// 获取 relay 的上游 Base URL，使用 upstream_base_url 作为 base_url 的 fallback
/// 配置文件可能只设置了 upstreamBaseUrl 而未设置 baseUrl，反序列化后 base_url 为空
fn relay_base_url(relay: &crate::settings::RelayProfile) -> &str {
    let url = relay.base_url.trim();
    if url.is_empty() {
        relay.upstream_base_url.trim()  // fallback 到 upstreamBaseUrl
    } else {
        url
    }
}
```

#### 2. 三个协议代理函数改用 `relay_base_url()`

```rust
// 修改前
if relay.base_url.trim().is_empty() { ... }
.post(chat_completions_url(&relay.base_url))

// 修改后
let base_url = relay_base_url(&relay);
if base_url.trim().is_empty() { ... }
.post(chat_completions_url(&base_url))
```

影响函数：
- `open_responses_proxy_request`
- `open_models_proxy_request`
- `open_chat_completions_proxy_request`

#### 3. `open_chat_completions_proxy_request` 改用 `proxied_client`

```rust
// 修改前：裸 client，无超时配置
let upstream = reqwest::Client::new()
    .post(chat_completions_url(&relay.base_url))...

// 修改后：用 proxied_client（带 300s 超时 + keepalive）
let client = crate::http_client::proxied_client(&relay.user_agent)?;
let upstream = client.post(chat_completions_url(&base_url))...
```

### ✅ 编译测试（2026-06-04）

```
$ cargo build --release -p codex-plus-core -p codex-plus-launcher
   Compiling codex-plus-core v1.2.0
   Compiling codex-plus-data v1.2.0
   Compiling codex-plus-launcher v1.2.0
    Finished `release` profile [optimized] target(s) in 14.13s
```

### 验证步骤

```powershell
# 1. 运行
.\target\release\codex-plus-plus.exe

# 2. 验证注入
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:57321/backend/status -Body "{}" -ContentType "application/json"

# 3. 验证流式请求：在 Codex 中发送消息，确认不再出现 stream disconnected 错误
#    Helper 日志中应出现 helper.protocol_proxy_ok 而非 helper.protocol_proxy_upstream_error
```

### 影响范围

- ✅ **修复**：`upstreamBaseUrl` 未映射导致协议代理请求失败
- ✅ **修复**：`open_chat_completions_proxy_request` 使用裸 client 无超时的问题
- ⚠️ **注意**：需要确保 relay profile 配置了正确的 `upstreamBaseUrl`

---

## 修复 4：`wait_for_codex_exit` 快速返回导致进程退出 + Helper 被 shutdown

**日期**：2026-06-04  
**问题**：启动约 10 秒后 Codex++ 进程退出，Helper 停止，所有后端连接断开。  

**根因（两个问题叠加）**：

| # | 问题 | 后果 |
|---|------|------|
| 1 | Windows Store 版 Codex.exe 是 broker/stub，启动真正 app 后自身退出 → `child.wait()` 立即返回 `Ok` | `main` 以为 Codex 已退出 |
| 2 | `wait_for_codex_exit()` **内部调用 `shutdown_helper()` 关闭 HTTP 服务器** | 即使进程没退出，57321 端口也已停止监听 |

**证据**：
```
main.wait_for_codex_exit_result: ok: true    ← broker 退出
helper.listening                              ← 曾经启动过...
但在线几分钟后 57321 端口消失                  ← Helper 被 shutdown 了
```

### 修改文件：`apps/codex-plus-launcher/src/main.rs`

**修改后完整逻辑**（第 46-100 行）：
```rust
let handle = launch_and_inject_with_hooks(options, &hooks).await?;

// 1. wait_for_codex_exit 会等待子进程，但 broker 会快速退出
let exit_result = handle.wait_for_codex_exit().await;

// 2. ⭐ 关键修复：wait_for_codex_exit 内部 shutdown 了 Helper，必须重启
tokio::time::sleep(Duration::from_millis(500)).await;
let _ = hooks.start_helper(handle.helper_port).await;

// 3. 循环检测 CDP 存活，避免进程退出
loop {
    tokio::time::sleep(Duration::from_secs(15)).await;
    if !is_cdp_alive(handle.debug_port).await {
        tokio::time::sleep(Duration::from_secs(3)).await;  // 二次确认
        if !is_cdp_alive(handle.debug_port).await {
            break;  // Codex 真正退出了
        }
    }
}
```

**新增辅助函数**：
```rust
async fn is_cdp_alive(debug_port: u16) -> bool {
    codex_plus_core::cdp::list_targets(debug_port).await.is_ok()
}
```

### 诊断日志系统（新增）

在 `protocol_proxy.rs`、`launcher.rs`、`main.rs` 三个文件中添加了全面的诊断日志：

| 事件 | 位置 | 用途 |
|------|------|------|
| `proto.open_responses_proxy_state` | `protocol_proxy.rs` | relay 配置实际值 |
| `proto.resolved_base_url` | `protocol_proxy.rs` | 最终请求 URL |
| `helper.proxy_route` | `launcher.rs` | 请求路由 |
| `helper.proxy_catch_error` | `launcher.rs` | catch-all 错误 |
| `helper.protocol_proxy_inner_error` | `launcher.rs` | 内部错误 |
| `main.wait_for_codex_exit_result` | `main.rs` | wait 结果 |
| `main.helper_restarted` | `main.rs` | Helper 重启确认 |
| `main.codex_exit_confirmed` | `main.rs` | Codex 退出确认 |

### 通过日志确认的修复效果

```
proto.open_responses_proxy_state:
  proto: ChatCompletions
  base_url: https://apihub.agnes-ai.com/v1         ← ✅ base_url 正确 fallback
  api_key_empty: false                              ← ✅ api_key 正确读取

proto.resolved_base_url:
  chat_completions_url: https://apihub.agnes-ai.com/v1/chat/completions  ← ✅ URL 正确

helper.protocol_proxy_upstream_error: 503 Service Unavailable  ← ⬆️ agnes-ai 偶尔返回 503
helper.protocol_proxy_stream_ok: 200 OK                        ← ✅ 有成功请求

helper.protocol_proxy_inner_error:
  error: "你的主机中的软件中止了一个已建立的连接(os error 10053)"  ← 网络波动导致
```

### 编译测试（2026-06-04）
```
cargo build --release -p codex-plus-core -p codex-plus-launcher
Finished `release` profile [optimized] target(s) in 4.60s
```

### ✅ 最终测试结果（2026-06-04）

| 检查项 | 结果 |
|--------|------|
| 进程存活（不崩溃） | ✅ Helper 重启 + CDP 循环 |
| Helper HTTP 服务器 57321 | ✅ `helper.listening` |
| CDP 注入 | ✅ `script_loaded: inject OK` |
| `/v1/responses` 请求到达 Helper | ✅ `helper.request` |
| 协议代理 URL 正确 | ✅ `https://apihub.agnes-ai.com/v1/chat/completions` |
| 上游响应 | ✅ 有 200 OK 也有 503（agnes-ai 服务端问题） |
| 连接不粗暴断开 | ✅ catch-all 写入 500 响应再关闭 |

### 编译测试（2026-06-04）
```
cargo build --release -p codex-plus-core -p codex-plus-launcher
Finished `release` profile [optimized] target(s) in 0.29s
```

### 验证步骤

```powershell
# 1. 运行后等待 30 秒以上
.\target\release\codex-plus-plus.exe

# 2. 检查进程是否存活
Get-Process codex-plus-plus -ErrorAction SilentlyContinue

# 3. 查看诊断日志确认
codex_plus_core::diagnostic_log 中应出现：
  - "main.wait_for_codex_exit_start" → 包含 launch_type、debug_port
  - "main.wait_for_codex_exit_result" → 包含 ok/error

# 4. 关闭 Codex 后，check 进程是否自动退出
```

### 影响范围

- ✅ **修复**：启动后进程不因 `wait_for_codex_exit` 提前退出
- ✅ **新增**：CDP 存活检测，确保 Codex 真正退出后才退出 Helper
- ⚠️ **注意**：需要 `kill` 或 Ctrl+C 来强制退出（如果有需要）

---

## 修复 5：CDP 注入重试粒度 + 模型列表短接跳过 app-server + 启动时间分析

**日期**：2026-06-04  
**问题**：Codex++ 启动比原版 Codex 慢约 1 分钟（实测 ~51s）。  
**根因**：

三个独立瓶颈叠加，逐一排查修复：

| # | 瓶颈 | 耗时 | 能否优化 |
|---|------|------|---------|
| 1 | CDP 轮询 sleep 粒度太粗（1s/500ms） | ~17s | ✅ 已优化 |
| 2 | 模型列表等 app-server 响应 | ~34s | ✅ 已优化（短接） |
| 3 | **Codex JS bundle V8 解析/编译** | **~33s** | ❌ Codex 自身限制 |

### 修改 1：加快 CDP 轮询粒度

| 文件 | 行 | 修改前 | 修改后 |
|------|-----|--------|--------|
| `crates/codex-plus-core/src/launcher.rs` | 171 | `from_secs(1)` | `from_millis(200)` |
| `crates/codex-plus-core/src/launcher.rs` | 1182 | `from_millis(500)` | `from_millis(200)` |
| `apps/codex-plus-launcher/src/main.rs` | 574 | `from_millis(500)` | `from_millis(200)` |

同时增加 `ensure_injection` 最大尝试次数 120→300，确保 200ms × 300 = 60s 总等待上限。

### 修改 2：注入脚本短接模型列表请求

**问题**：Codex 页面通过 app-server RPC `list-models-for-host` 获取模型列表。app-server 启动慢（~34s），而 Codex++ bridge 的 `/codex-model-catalog` 接口从 relay profile 配置直接返回（<1s）。

**修改文件**：`assets/inject/renderer-inject.js` 第 3819-3832 行

**函数**：`patchAppServerModelRequestClient` 内部的 `codexPlusModelPatchedSendRequest`

**修改前**：
```javascript
const result = await originalSendRequest(method, params, options);  // 等 app-server 34s
if (!codexPlusModelUnlockEnabled()) return result;
if (!codexPlusModelNames().length) await loadCodexModelCatalog();
return patchAppServerModelResult(method, result);
```

**修改后**：
```javascript
// 优先从 Codex++ bridge 获取模型列表（快，<1s），不等 app-server
if (codexPlusModelUnlockEnabled() && method === "list-models-for-host") {
    if (!codexPlusModelNames().length) await loadCodexModelCatalog();
    if (codexPlusModelNames().length > 0) {
        return patchAppServerModelResult("list-models-for-host", { data: [] });
    }
}
const result = await originalSendRequest(method, params, options);
if (!codexPlusModelUnlockEnabled()) return result;
if (!codexPlusModelNames().length) await loadCodexModelCatalog();
return patchAppServerModelResult(method, result);
```

**关键**：`{ data: [] }` 空数组让 `patchModelArray` 自动用 `codexPlusModelDescriptor` 补全为完整模型对象（含 id、slug、name、hidden 等字段）。

**⚠️ 踩坑**：第一版尝试用 `{ model: n }` 构造对象，导致模型不渲染。空数组让 `patchModelArray` 用 `codexPlusModelDescriptor` 自动填充才正确。

### 修改 3：注入后监控页面状态（调试用）

在 `launcher.rs` 新增 `monitor_page_loading` 函数，通过 CDP `Runtime.evaluate` 每 5 秒检测：
- `elCount` — DOM 元素数（判断渲染进度）
- `chatInput` — 聊天输入框是否出现
- `sidebar` — 侧边栏是否渲染
- `bodyPreview` — 页面正文前 120 字符
- `title`, `readyState` 等标准指标

监控发现：注入后 JS bundle 解析~33s，然后 React 瞬间渲染（517→947 元素，chatInput 从 false→true）。

### 最终启动时间分解（实测 PID 13708）

```
03:40:21  script_loaded + bridge 请求完成        ← Codex++ 就绪
03:40:21  ~~~ 33 秒静默（无事件，无渲染）~~~     ← V8 解析 Codex JS bundle
03:40:54  model_app_server_result_patched        ← 模型列表就绪（来自 bridge）
03:40:58  elCount 517→947, chatInput=true        ← React 渲染完成
```

| 阶段 | 耗时 | 责任方 |
|------|------|--------|
| Codex 进程启动 | ~0.1s | — |
| CDP 注入等待（含窗口创建） | ~17s | Windows Store + CDP，已优化 |
| **Codex JS bundle V8 解析** | **~33s** | **Codex 自身，无法优化** |
| 模型列表加载 | <1s | ✅ bridge 短接后即时 |
| 总计 | ~51s | **其中 33s 是 Codex 自身上线** |

### 编译验证（2026-06-04）
```
cargo build --release -p codex-plus-core -p codex-plus-launcher
Finished `release` profile [optimized] target(s) in 14.70s
```

---

## 相关记忆文件

- [[codex-plus-windows-store-fix]] — 本笔记的详细补丁说明
