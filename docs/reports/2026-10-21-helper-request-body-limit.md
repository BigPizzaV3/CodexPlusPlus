# 2026-10-21 本地协议代理请求体上限修复（413 Payload Too Large）

## 现象

用户截图：Codex 会话经 Codex++ 本地代理（`http://127.0.0.1:57321/v1/responses`）发请求时返回

```
unexpected status 413 Payload Too Large:
{"status":"failed","message":"HTTP 请求体超过 33554432 字节限制（可通过 CODEX_PLUS_MAX_HTTP_BODY_BYTES 调大）"}
```

对话随即中断（「你在 0 秒后停止了」）。会话是 1M 上下文窗口的长会话，状态栏显示
68% 上下文占用。

## 定位

- 413 由 `crates/codex-plus-core/src/launcher.rs` 的 helper HTTP 读取层产生；
  旧默认上限 `MAX_HTTP_BODY_BYTES = 32 MiB`（33,554,432 字节），来自 #2198 的
  「默认保持 32 MiB、仅允许环境变量调高」决定。
- 现场诊断日志（`dist/local-build/state/codex-plus.log`）里，崩溃前的
  `helper.request` 请求体大小已连续逼近上限：

  | body_bytes | 占 32 MiB |
  | --- | --- |
  | 32,655,376 | 97.3% |
  | 33,297,694 | 99.2% |

  同时请求日志显示该会话 `inputTokens = 684,700`、缓存命中 99.98%。按此比例，
  1M tokens 的请求体约 50 MB，32 MiB 默认值在正常长会话里必然先撞上。
- 结论：不是请求异常膨胀，而是 **Codex++ 自己支持的「按模型配置大上下文窗口」
  与 helper 的 32 MiB 默认硬上限互相矛盾**。另外旧错误提示只提环境变量，
  GUI 用户（Finder/LaunchServices 启动）实际上无法设置。

## 修复

1. **默认上限 32 MiB → 128 MiB**：给 1M 窗口留约 2.5 倍余量；
   上限（512 MiB）与下限（32 MiB）保持不变。
2. **新增设置项 `helperMaxRequestBodyMib`**（默认 128，范围 32~512）：
   - 管理器「代理设置 → 本地协议代理 → 请求体上限（MiB）」可视化修改；
   - 保存后立即对运行中的 helper 生效（`save_settings` 调用
     `launcher::set_http_body_limit_bytes`），不必重启 Codex++/Codex；
   - 启动流程在 helper 绑定端口前从设置读取并应用；管理器进程启动时也会应用；
   - `CODEX_PLUS_MAX_HTTP_BODY_BYTES`（字节）仍优先，兼容既有用法。
3. **设置落盘安全**：`settings.json` 读写、`/settings/set` 部分更新白名单、
   管理器保存前都夹取到 32~512 MiB；旧配置没有该字段时补默认 128。
4. **413 提示**改为「请在 Codex++『代理设置』调大『请求体上限』，或设置环境变量」，
   直接指到可操作入口。

内存权衡：helper 会整段缓冲请求体后再解析/转发，128 MiB 上限对应单次请求约
数百 MB 的瞬时内存；这是长上下文会话可用性的必要代价，且仍可用设置/环境变量调低
到 32 MiB。

## 改动文件

- `crates/codex-plus-core/src/launcher.rs`：默认值/上下限、运行期上限注入
  （`set_http_body_limit_bytes`）、env 优先级、413 文案、`read_http_request_with_limits`
  便于小上限测试。
- `crates/codex-plus-core/src/settings.rs`：`helper_max_request_body_mib` 字段、
  默认值常量、范围夹取、load/save/update 归一化、部分更新白名单。
- `apps/codex-plus-manager/src-tauri/src/commands.rs`：保存前夹取、保存后立即应用。
- `apps/codex-plus-manager/src-tauri/src/lib.rs`：管理器启动时应用设置。
- `apps/codex-plus-manager/src/App.tsx`（+ `web-preview/tauri-web-shim.ts`）：
  类型、默认值、归一化与「请求体上限（MiB）」输入框。
- `crates/codex-plus-core/tests/helper_body_limit_settings.rs`（新增）：
  默认 128、落盘/部分更新、范围夹取、字节换算防溢出。

## 测试

- `cargo test -p codex-plus-core --lib`：1059 通过；3 项环境相关失败与本改动无关
  （`native_browser` 兼容锁并行重取 `WouldBlock`，单独复跑通过；两项 macOS 更新
  DMG 测试需要 `hdiutil`，沙箱内被系统拒绝）。
- 受影响的集成测试全绿：`launcher`（101）、`bridge_routes`（38）、
  `protocol_proxy`（203）、`settings_load_recursion`、`tool_shards`、
  `live_settings_migration`。
- `cargo test -p codex-plus-core --test helper_body_limit_settings`（3）、
  `--test proxy_auto_retry_settings`（4）。
- `cargo test -p codex-plus-manager --lib normalize_settings_before_save`（10）。
- `npm run check`（tsc）通过；`npm test` 743 项通过、1 跳过。

## 验收

构建并 `node scripts/relaunch-local.mjs` 重启固定实例 `dist/local-build/Codex++.app`
（20:05 新进程 44067，helper 正常绑定 57321）后实测：

| 场景 | 结果 |
| --- | --- |
| 40 MiB 实际请求体（旧上限下必 413） | `HTTP/1.1 404 Not Found`（未知路径），**请求体已被完整接收** |
| 声明 `Content-Length: 128MiB + 1` | `HTTP/1.1 413 Payload Too Large`，提示「超过 128 MiB（134217728 字节）上限：请在 Codex++『代理设置』调大『请求体上限』…」 |
| 声明 `Content-Length: 512MiB + 1` | `HTTP/1.1 413 Payload Too Large`（上限仍生效） |

用户原会话（请求体约 33.5 MB）重试即可继续，无需再设环境变量；重启后注入桥已重新连上
（`bridge.resolve_ok`）。管理器「代理设置」页可见「请求体上限（MiB）」输入框，默认 128。
