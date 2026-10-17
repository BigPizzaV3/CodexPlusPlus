# HTTP 429 自动继续验证

本功能基于 Codex++ 1.8.0，默认关闭。维护者要求 Windows artifacts 和 macOS DMG CI 通过，并验证默认关闭、冷却等待、重复点击防护。真实上游 429 的恢复是另一个尚待实测的项目，不应与模拟测试或安装包构建混为一谈。

## Windows 本机快速验证

准备 Node.js 22.18+（或 24+）、Rust 和现有项目依赖。首次安装前端依赖可在 `apps/codex-plus-manager` 执行 `npm ci`；后续仅重复验证无需重新安装。

在仓库目录打开 PowerShell：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-rate-limit-auto-resume.ps1
```

脚本运行 16 项 HTTP 429 行为测试、10 项既有容量重试测试、4 项两种功能的设置持久化测试，并检查两个注入产物是否与源码一致。任何命令失败都会停止，日志保存在 `target/429-validation/`。

重点结果：

| 要求 | 对应断言 |
| --- | --- |
| 默认关闭 | 未设置或设为 false 时没有轮询定时器、原生读取和按钮点击；旧 settings.json 加载为 false |
| 冷却等待 | 29,999 ms 时不点击；到 30,000 ms 再读最新轮次，仍满足条件才点击 |
| 重复点击防护 | 同轮次 ID 只点击一次；并发心跳/到点重复调用也只点击一次 |
| 连续快速失败 | 即使没观察到按钮中间状态，也能识别新的失败轮次，并重新等待 30 秒 |
| 退出条件 | 草稿、未知主机、隐藏或禁用按钮、切换会话、停止及异步迟到结果不会误触发 |
| 次数上限 | 本次会话访问最多自动继续 5 次 |

行为测试使用假时钟，30 秒等待通过推进时钟验证，执行时不用真的等 30 秒。DOM 和 `thread/read` 是测试替身；这不是对实际 Codex 原生客户端的端到端验证。

## 与 Windows CI 对齐的本机检查

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-rate-limit-auto-resume.ps1 -Full -Build
```

此模式运行完整前端测试、TypeScript 检查、完整 Rust workspace 测试、前端生产构建和 `cargo build --release`。比快速验证更久，并需要 Windows 的 MSVC 构建环境。

本地 exe 位于 `target/release/codex-plus-plus.exe`。此命令不安装、不替换现用程序，也不运行 NSIS 安装包构建或 macOS DMG 构建。Windows 安装包和 macOS 通用 DMG 的完整产物验证仍使用现有 `PR build artifacts` workflow。

在当前开发机上，未改动的上游用例 `app_paths_resolves_portable_current_link_to_directory_version` 创建符号链接时返回 Windows 1314（没有所需特权）。不修改系统权限也可明确排除这一项，以继续检查其他用例：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-rate-limit-auto-resume.ps1 -Full -Build -SkipSymlinkPrivilegeTest
```

该参数会在日志中明示排除项，不能将结果报告为完整 CI 全通过。CI 仍运行未排除用例的 `cargo test --workspace`，需在具备符号链接创建权限的 Windows runner 上确认结果。

## GitHub CI

更新 PR 后，在 PR 的 Checks 页检查当前提交对应的：

- `Windows artifacts`：完整前端和 Rust 测试、类型检查、构建以及 NSIS 安装包。
- `macOS DMG (universal)`：HTTP 429/容量行为测试、429 设置测试、注入产物检查、Intel + Apple Silicon 通用 DMG 构建与 bundle 检查。

若显示需要批准外部贡献者的 workflow，必须由上游维护者点击批准。没有执行记录不能算通过；fork 自己的 Actions 成功也不等于上游 PR 的两个检查已变绿。

如需在自己的 fork 上手动运行同一 workflow（GitHub Actions 须已启用，且默认分支上存在该 workflow）：

```powershell
gh workflow run pr-build.yml --repo YOUR_ACCOUNT/CodexPlusPlus --ref feat/429-auto-resume
gh run list --repo YOUR_ACCOUNT/CodexPlusPlus --workflow pr-build.yml --branch feat/429-auto-resume
gh run watch RUN_ID --repo YOUR_ACCOUNT/CodexPlusPlus --exit-status
```

确认运行页的 commit SHA 是当前 PR head，并保留两个 job 的链接与日志。不要运行 `release-assets.yml`，它用于正式版本产物发布。当前 PR 保持 Draft，两个构建检查成功后再考虑 Ready for review。

## 实际应用手测

1. 从系统托盘退出现用 Codex++，等其后台服务结束；再启动上述本地 exe。只关闭窗口可能仍驻留托盘，造成新程序未接管后台。
2. 先保持「429 错误自动继续」关闭，确认它不会自动继续。确认测试的是该分支构建，界面和启动日志显示 1.8.0；测试完成后可退出并重新打开原安装版。
3. 开启该功能。如果遇到真实 429，检查最新一轮已经失败、输入框为空、原生 Resume/继续按钮可用，并出现 30 秒倒计时。
4. 确认倒计时内没有点击；结束时只点击一次。同一错误留在原地不应连点。新一轮再次因 429 失败时需重新等待，最多 5 次。
5. 另一次倒计时中输入草稿、切换会话或点击状态提示停止，分别确认不再点击。

不需要用大量请求制造限流。若无法复现真实 429，只记录自动化测试和窗口/默认开关实测结果，保留实际原生读取、真实按钮点击及任务恢复为未验证。提交证据时请去除凭据和对话内容，只保留版本、提交 SHA、用例结果及相关时间。
