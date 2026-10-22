# 2026-10-21 本地协议代理常驻（不经过 Codex++ 启动 Codex 也能用）

## 现象

选择需要协议转换 / 聚合 / 请求避震的供应商后，`config.toml` 的 `base_url` 会写成
`http://127.0.0.1:57321/v1`。但本地 helper 过去只在 `launch_and_inject` 中随
Codex 会话启动、Codex 退出即关闭，于是：

- 从终端跑 `codex`、IDE 插件或其它客户端时，`57321` 没人监听，请求直接连接失败；
- Codex++ 退出但保留官方 Codex 时，正在用的会话也会因为代理消失而断流。

## 方案

把 helper 变成管理器进程内的**常驻实例**（`codex-plus-core/src/resident_helper.rs`）：

- 管理器启动、恢复 Codex、自动接管、供应商切换/应用后调用 `sync_resident_helper()`；
- 判定条件取并集：设置里当前供应商需要固定协议代理端口，或 live `config.toml`
  已指向本地代理（`relay_config::live_config_uses_managed_protocol_proxy_in_home`）；
- 端口取 `protocol_proxy::protocol_proxy_port()`（与写进 config 的端口同源）；
- Codex 会话退出**不**停止常驻 helper；只有 Codex++ 退出时在退出清理流程里
  `stop_resident_helper()`；
- 启动链路的 `DefaultLaunchHooks::start_helper` 检测到常驻 helper 占用同一端口时
  直接复用（记 `helper.reuse_resident`），避免二次 bind 失败；
- 会话 helper 收尾时只删除本端口的听写凭据；若常驻 helper 仍在，收尾后把凭据补回
  常驻端口，避免听写路由失效；
- 重启流程 `prepare_fixed_helper_port_for_restart` 把「常驻 helper 占着端口」视为
  预期状态直接跳过等待，否则会白等到超时。

这是「A 方案」：要求 Codex++ 保持运行。完全不启动 Codex++ 的场景（独立守护进程 /
登录项）不在本次范围内。

## 改动文件

- `crates/codex-plus-core/src/resident_helper.rs`（新增）：常驻 helper 生命周期
  （独立线程 + current-thread runtime，幂等 ensure、串行化、停止时 join）。
- `crates/codex-plus-core/src/launcher.rs`：`start_helper` 复用常驻实例；
  `shutdown_helper` 不再无条件删除听写凭据，并补回常驻端口的凭据。
- `crates/codex-plus-core/src/relay_config.rs`：
  `live_config_uses_managed_protocol_proxy_in_home`（只认活动 provider，覆盖根键
  `openai_base_url` 的官方会话身份）。
- `apps/codex-plus-manager/src-tauri/src/commands.rs`：`sync_resident_helper()` +
  启动/恢复/自动接管/切换供应商/应用注入/重启等待接线。
- `apps/codex-plus-manager/src-tauri/src/lib.rs`：退出清理时停止常驻 helper。
- `apps/codex-plus-manager/src/App.tsx` / `i18n-en.ts`：本地协议代理说明文案。

## 验证

- `cargo test -p codex-plus-core --test resident_helper`（新增集成测试）：常驻实例
  直接服务 HTTP、幂等 ensure、会话 start/shutdown 后仍在、听写凭据不丢、端口被占
  时 ensure 报错、失败后可恢复。
- `cargo test -p codex-plus-core --test launcher`（101）、`cargo test -p
  codex-plus-core --lib`（1062 通过；2 个 macOS DMG 用例因沙箱禁用 hdiutil 失败，
  与本次改动无关）、`cargo test -p codex-plus-launcher`（35）。
- `cargo test -p codex-plus-manager`：177 通过；4 个用例因沙箱禁止写真实家目录
  （Environment/Writes outside workspace）失败，与本次改动无关。
- `npx tsc --noEmit`、`npm test`（752 通过 / 1 skipped）。
- 本地 `dist/local-build/Codex++.app` 启动后：`lsof -iTCP:57321` 的监听进程为管理器
  进程；日志出现 `manager.resident_helper.ready` 与 `helper.reuse_resident`（会话复用
  常驻实例，而不是各起一个）。
