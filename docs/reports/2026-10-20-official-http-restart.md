# 官方 HTTP 配置在重启 Codex 后失效

## 原因

管理器的 `restart_codex_plus_after_stop` 在同步当前官方供应商时调用
`clear_relay_config_to_home_with_auth`。该函数无条件删除 `model_provider`，
使手工选择的 `openai-http` 失效；保留下来的 `supports_websockets = false`
属于未选中的 provider，不能继续约束官方默认连接。

## 修复边界与验证计划

- 仅保留显式使用官方认证、Responses 协议并禁用 WebSocket，且没有额外路由或凭据字段的活动 provider。
- 连续执行两次重启同步，验证 provider 选择、HTTP 标记、模型和官方登录不变。
- 覆盖普通、自带托管名称的 provider，避免依赖 `openai-http` 名字。
- 带第三方 URL、Bearer Token、环境凭据或自定义请求头时仍清除托管路由。
- 沿用既有 relay_config 测试检查官方与 API 切换行为。

## 验证结果

- 修复前新增的连续重启用例失败：期望 `openai-http`，实际 provider 选择为空。
- 修复后 `cargo test --offline -p codex-plus-core --test relay_config --test relay_config_safety`
  全部通过（194 + 23 项）。
- `npm run vite:build` 与 `cargo build --release -p codex-plus-manager` 通过。
- 固定测试应用 `dist/local-build/Codex++.app` 已签名启动，进程与 `manager.start` 日志正常。
  测试实例使用独立状态和 Codex 配置目录；真实网络对话仍需用户在使用环境中复测。

## PR 的 Windows CI 修正

首次 CI（run `39370111077`）在原有单元测试
`paths::tests::settings_path_pointer_accepts_only_absolute_targets` 失败：
测试写死 `/tmp/codex-settings.json`，该路径在 Windows 上不满足 `is_absolute()`。
改为使用 `tempfile::tempdir()` 下的目标文件，保留末尾换行和相对路径拒绝断言。
本次仅修正跨平台测试数据，不改变路径解析或 HTTP 修复的运行行为。
本地 `cargo test --offline -p codex-plus-core --lib paths::tests` 的 10 项测试通过。

第二轮 CI（run `39371606043`）已通过上述路径断言与 HTTP 回归测试，随后在
`provider_sync_skips_rollout_locked_after_planning` 失败。扫描后被锁定的 rollout
在恢复记录预计算阶段重新读取时，通过 `?` 提前返回错误，绕过了原有锁定文件跳过机制。
回归验证：Windows 保留真实独占锁用例；Unix 在 Planning 阶段撤销文件读取权限，
验证受阻文件保持原样并被报告跳过，同时可写文件仍完成同步。
新增 Unix 用例修复前返回 `Failed`，修复后会话同步测试组 60 项全部通过。
