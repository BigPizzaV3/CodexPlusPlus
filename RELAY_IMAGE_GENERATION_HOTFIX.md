# Codex++ 混合 API 生图特性热修记录

记录时间：2026-05-24

## 背景

本次问题发生在 Codex++ 的“官方登录 + 混入 API Key”模式。

这个模式会保留 ChatGPT 官方登录态，同时把模型请求转到用户配置的中转 API。部分中转 API 不支持 OpenAI Responses API 的 hosted image generation 能力。当 Codex 在 Responses 请求里带上生图工具时，中转会返回类似错误：

```text
unexpected status 403 Forbidden: Image generation is not enabled for this group
```

只要看到这个错误，就说明上游实际收到了 `image_generation` 工具，而不是单纯配置文件显示问题。

纯 API 模式不是这条链路，所以本次修补不能把纯 API Responses 强行改成本地代理，也不能给纯 API 额外写入生图禁用字段。

## 第一次修补

第一次修补只处理了配置层：

```toml
[features]
image_generation = false
```

修改点在：

```text
crates/codex-plus-core/src/relay_config.rs
crates/codex-plus-core/tests/relay_config.rs
```

当 Codex++ 写入混合中转配置时，会自动给 `[features]` 加上 `image_generation = false`。如果用户原本写过 `image_generation = true`，切到混合中转时会保存注释并临时改成 `false`；清除中转或切回纯 API 时会恢复。

这个修补仍然保留，因为它是合理的配置表达，也能兼容官方未来修好后的行为。

## 二次复现

用户再次复现后，纯 API 模式的插件解锁已经连续两次成功，但混合模式仍然报：

```text
Image generation is not enabled for this group
```

这说明 `image_generation = false` 没有完全阻止 Codex app-server 把 hosted image generation tool 放进 Responses 请求。也就是说，仅靠配置字段不够。

本地进一步看到 Codex 资源里存在 `imageGeneration` 类型相关痕迹，也看到配置解析里有 `[features]` legacy toggle 提示。因此这次不能继续堆配置项，必须在出站请求层兜住。

## 最终根因

最终根因分成两层：

1. 混合 Responses 模式下，`~/.codex/config.toml` 过去直接把 `base_url` 指向真实中转。
2. Codex 仍可能在发往该 `base_url` 的 Responses 请求体里带上：

```json
{
  "tools": [
    { "type": "image_generation" }
  ]
}
```

中转不支持该能力，于是返回 403。

因此最终修补点不是“再找一个配置字段”，而是让混合 Responses 请求先走 Codex++ 本地 helper，再由 helper 转发到真实中转；转发前删除 hosted image generation 相关字段。

## 最终修补策略

本次属于针对混合模式的根因治理，范围保持很窄：

1. 混合模式，包括“官方登录 + 混入 API Key”的 Responses 协议，统一把 Codex 配置里的 `base_url` 写成本地代理：

```toml
base_url = "http://127.0.0.1:57321/v1"
```

2. 本地 helper 读取 Codex++ 设置里的真实中转地址和 Key，再把请求转发到真实中转：

```text
https://relay.example.test/v1/responses
```

3. 转发前清理请求体里的 hosted image generation：

```json
{
  "type": "image_generation"
}
```

以及兼容 camelCase：

```json
{
  "type": "imageGeneration"
}
```

4. 如果 `tool_choice` 强制选择生图工具，则删除 `tool_choice`。
5. 如果 `tool_choice` 使用 `allowed_tools` 包住工具列表，则只删除其中的生图工具，保留普通 function tool。
6. 如果 `include` 里请求 `image_generation_call.*`，也删除对应 include 项。
7. 纯 API Responses 继续直连真实中转，不进入这个本地 Responses 透传代理。
8. Chat Completions 转 Responses 的旧本地代理路径继续保留。

## 修改位置

核心修改：

```text
crates/codex-plus-core/src/protocol_proxy.rs
crates/codex-plus-core/src/relay_config.rs
crates/codex-plus-core/src/launcher.rs
crates/codex-plus-core/src/settings.rs
apps/codex-plus-manager/src/App.tsx
```

相关测试：

```text
crates/codex-plus-core/tests/protocol_proxy.rs
crates/codex-plus-core/tests/relay_config.rs
crates/codex-plus-core/tests/launcher.rs
```

前一次同时修过连续重启时插件解锁失败的问题，相关文件仍在本次改动里：

```text
apps/codex-plus-manager/src-tauri/src/commands.rs
crates/codex-plus-core/src/watcher.rs
crates/codex-plus-core/tests/watcher.rs
```

## 关键代码行为

### 1. 混合 Responses 写入本地代理 URL

`apply_relay_config_to_home_with_protocol` 现在走混合模式专用 URL 选择：

```rust
let codex_base_url = codex_base_url_for_mixed_relay_protocol(base_url, protocol, proxy_port);
```

混合模式下，无论是 Responses 还是 Chat Completions，Codex 看到的都是：

```text
http://127.0.0.1:57321/v1
```

纯 API 仍然走 `codex_base_url_for_pure_api_protocol`，Responses 保持真实中转 URL。

### 2. 完整 config.toml 也会被修正

如果供应商保存了完整 `config.toml` / `auth.json`，`apply_relay_files_to_home` 也会在写入前修正 CodexPlusPlus provider 的 `base_url`：

```rust
relay_config_with_local_responses_proxy_guard(...)
```

这样旧设置里已经保存的原始中转 URL 不会继续绕过本地代理。

### 3. helper 会为混合 Responses 启动

`launcher.rs` 中的代理启动判断从“只有 Chat Completions 才启动”改成：

```rust
relay.protocol == RelayProtocol::ChatCompletions
    || (relay.protocol == RelayProtocol::Responses && relay.uses_official_api_key_mix())
```

这样混合 Responses 即使页面增强处于兼容模式，也会启动本地 helper。

### 4. Responses 透传代理会清洗请求体

`protocol_proxy.rs` 新增：

```rust
sanitize_responses_request_for_relay
```

它会删除：

```json
{ "type": "image_generation" }
{ "type": "imageGeneration" }
```

同时删除指向生图工具的 `tool_choice`，清理 `allowed_tools` 中的生图工具，以及删除 `include` 中的 `image_generation_call.*`。

清洗后，如果请求原本还有普通 function tool，会保留普通工具，只删除生图工具。

### 5. Responses 和 Chat 两种代理分开处理返回体

本地代理现在区分：

```rust
UpstreamResponseBodyMode::ResponsesPassthrough
UpstreamResponseBodyMode::ChatCompletionsToResponses
```

Responses 上游返回的内容直接透传给 Codex；Chat Completions 上游仍然按原逻辑转换回 Responses。

## 验证命令

Rust 核心测试：

```bash
cargo test -p codex-plus-core --test protocol_proxy --test relay_config --test launcher
cargo test -p codex-plus-core --test watcher
```

本次结果：

```text
launcher: 39 passed
protocol_proxy: 17 passed
relay_config: 21 passed
watcher: 8 passed
```

前端类型检查：

```bash
cd apps/codex-plus-manager
npm run check
```

结果：通过。

前端构建：

```bash
npm run vite:build
```

结果：通过。

release 构建：

```bash
cargo build -p codex-plus-launcher -p codex-plus-manager --release
```

结果：通过。

测试期间仍有仓库既有 warning，例如 Windows uninstall key 和 proxy 辅助函数未使用；这些 warning 与本次修补无关。

## 本机 APP 更新过程

构建后生成的二进制：

```text
target/release/codex-plus-plus
target/release/codex-plus-plus-manager
```

本次手动替换到：

```bash
cp target/release/codex-plus-plus \
  "/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus"

cp target/release/codex-plus-plus-manager \
  "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager"
```

替换后重新签名：

```bash
codesign --force --deep --sign - "/Applications/Codex++.app"
codesign --force --deep --sign - "/Applications/Codex++ 管理工具.app"
```

验证：

```bash
codesign --verify --deep --strict "/Applications/Codex++.app"
codesign --verify --deep --strict "/Applications/Codex++ 管理工具.app"
```

本次验证结果：

```text
Codex++ signature ok
Manager signature ok
```

两个 app 的可执行文件更新时间：

```text
2026-05-24 09:33
```

管理工具已退出旧进程并重新打开。当前 Codex 主进程没有被强行关闭，避免打断正在进行的会话；下一次通过管理工具重启 Codex 时，会使用新的 Codex++ 启动器。

## 下次如果还要重复修补

如果未来官方仍未修复，或上游合并时这段逻辑被覆盖，按以下顺序复查：

1. 检查混合 Responses 是否写入本地代理 URL：

```rust
codex_base_url_for_mixed_relay_protocol
relay_config_with_local_responses_proxy_guard
```

2. 检查纯 API Responses 是否仍然直连真实中转：

```rust
codex_base_url_for_pure_api_protocol
```

3. 检查 helper 是否会为混合 Responses 启动：

```rust
relay_protocol_proxy_enabled
RelayProfile::uses_official_api_key_mix
```

4. 检查请求体清洗是否仍然存在：

```rust
sanitize_responses_request_for_relay
UpstreamResponseBodyMode::ResponsesPassthrough
```

5. 运行测试：

```bash
cargo test -p codex-plus-core --test protocol_proxy --test relay_config --test launcher
cargo test -p codex-plus-core --test watcher
```

6. 重建并替换本机 app：

```bash
cargo build -p codex-plus-launcher -p codex-plus-manager --release

cp target/release/codex-plus-plus \
  "/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus"

cp target/release/codex-plus-plus-manager \
  "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager"

codesign --force --deep --sign - "/Applications/Codex++.app"
codesign --force --deep --sign - "/Applications/Codex++ 管理工具.app"
```

7. 完全退出旧的 Codex++ 管理工具进程，重新打开管理工具。

## 不要记录的内容

排查时可以查看 `~/.codex-session-delete/settings.json`、`~/.codex/config.toml` 和 `~/.codex/auth.json`，但不要把真实 API Key、JWT、refresh token 或账号 token 写入文档、issue、commit message 或日志摘录。

本记录只描述修补过程和字段，不包含任何真实密钥。
