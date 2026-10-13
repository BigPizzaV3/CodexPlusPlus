# Codex 多开 · Windows 实机验证清单

本次改动让「Codex 多开」在 Windows 上真正可用。改动分四层，对应下面四组用例。
**本机是 macOS，Windows 侧行为无法在本机实测**，所以这份清单就是验收依据：按顺序跑，
任何一条不符合预期都属于缺陷，请把「实际命令行 / 现象 / 日志片段」回给我。

## 0. 准备

- Windows 上安装的是**官方 Store / MSIX 版** Codex（默认形态），确认：

  ```powershell
  Get-AppxPackage -Name OpenAI.Codex | Select-Object PackageFullName, InstallLocation
  Get-AppxPackageManifest (Get-AppxPackage -Name OpenAI.Codex) |
    Select-Object -ExpandProperty Package |
    ForEach-Object { $_.Applications.Application.Id }
  ```

  记下 `PackageFullName` 与 `Application Id`（预期是 `App`），后面要用。
- 打开 Codex++ 管理工具的「设置 → 日志」，能实时看到 `codex-plus.log`；
  日志与状态文件在 `%USERPROFILE%\.codex-session-delete\`。
- 准备好**两个**不同供应商身份（例如 DeepSeek 与 Zhipu GLM），都能单开跑通。
- 需要抓进程命令行时用：

  ```powershell
  Get-CimInstance Win32_Process -Filter "Name='ChatGPT.exe'" |
    Select-Object ProcessId, CommandLine | Format-List
  ```

## 1. 回归：主 Codex 本身不能被改坏（最高优先级）

| # | 操作 | 预期 |
|---|---|---|
| 1.1 | 点「启动 Codex」 | 正常起来并注入增强（与改动前一致） |
| 1.2 | 点「重启 Codex」 | 正常重启，不出现「该进程没有程序包标识符」 |
| 1.3 | 关掉再启动管理工具 | 主实例状态显示正确，`latest-status.json` 是主实例自己的 |

> 这一组若失败，说明改动影响了单实例路径，请立刻停止后续测试并回报。

## 2. 多开主路径：包上下文启动是否真的起得来

| # | 操作 | 预期 |
|---|---|---|
| 2.1 | 多开页点供应商 A 的「启动」 | 90 秒内出现第二个 Codex 窗口（标题带 `Codex++ 多开 · <身份>`） |
| 2.2 | 多开列表 | 该实例显示**运行中**，PID / CDP / API 三列有值 |
| 2.3 | 用上面 `Get-CimInstance` 抓命令行 | 必须能看到 `--remote-debugging-port=<端口>`、`--user-data-dir=<实例目录>`，端口与列表里 CDP 一致 |
| 2.4 | 看 `%USERPROFILE%\.codex-session-delete\codex-plus.log` | 有 `launcher.packaged_multi_instance_activated`；**不应**出现 `launcher.packaged_multi_instance_fallback`（出现了说明降级了，请把该行连同上下文回我） |
| 2.5 | 增强是否注入成功 | 多开窗口里能打开 Codex++ 的增强界面（不是裸 Codex） |

**关键失败表现**：点了没窗口、列表却显示运行中 → 说明激活那条路没走通；
90 秒后列表变「启动失败」并给出原因 → 把 `launch-result.json` 一起回我。

## 3. 隔离：三个目录必须各归各的

| # | 操作 | 预期 |
|---|---|---|
| 3.1 | 多开实例起来后 | `%LOCALAPPDATA%\CodexPlusPlus\multi\<slug>\` 下有 `codex-home` / `sqlite-home` / `worker-home` / `user-data` / `packaged-launch.cmd` |
| 3.2 | 对照主实例 | 主实例的 `~\.codex` 与多开的 `codex-home` 内容各自独立，新会话只出现在多开自己的目录里 |
| 3.3 | Windows 上 `HOME` 无效的回归点 | `worker-home\.codex-session-delete\latest-status.json` 与 `settings.json` **确实被写入**（这是本次修的 P0：改动前 worker 读的是主实例的 settings，`--user-data-dir` 根本传不下去） |
| 3.4 | 两个多开实例并行 | 各自 `codex-home` / `user-data-dir` 不同，互不覆盖；两个窗口登录态、会话列表互不影响 |

## 4. 判活与结束：不能再「一停全停」（本次另一个 P0）

| # | 操作 | 预期 |
|---|---|---|
| 4.1 | 主 Codex + 多开 A + 多开 B 同时在跑 | 列表里三行都显示运行中，端口各不相同 |
| 4.2 | 在多开页对 A 点「结束」 | **只有 A 退出**；主 Codex 与 B 继续运行且不受影响 |
| 4.3 | 手动关掉 B 的窗口 | 列表里 B 变成「未运行」，A 仍显示运行中 |
| 4.4 | 点主 Codex 的「重启 Codex」 | 只重启主实例；A / B 不受影响（改动前会连带杀掉全部 Codex） |
| 4.5 | 对已经退出的实例点「结束」 | 提示「该实例当前未在运行，未结束任何 Codex 进程」，不误杀别的进程 |

## 5. 失败路径：这次必须「看得见」

| # | 操作 | 预期 |
|---|---|---|
| 5.1 | 模拟激活失败：临时把 Appx 包停用（`Remove-AppxPackage` 后仍指向原路径），或在非管理员 PowerShell 里验证 cmdlet 不可用 | 实例不会假装成功：列表 90 秒内变「启动失败」，卡片上直接显示原因 |
| 5.2 | 看日志 | 有 `manager.multi_instance_not_ready` 或 `launcher.packaged_multi_instance_*`，含 `debug_port` / `app_path` / launcher 阶段 |
| 5.3 | 失败后检查端口 | 进程退出后 helper / 协议代理端口已释放，没有残留占用 |
| 5.4 | 失败后重试 | 再点「启动」能重新走一遍，不会因为上一次失败卡住 |

## 6. 已知不确定点（出现即回报，不算你的操作问题）

1. `Invoke-CommandInDesktopPackage` 是微软的**调试用 cmdlet**，文档明确「不保证行为、可能变更」。
   若报「无法识别 cmdlet / 需要导入 Appx 模块」，请回报 PowerShell 版本与完整报错。
2. `-Args '/d /c "<wrapper>"'` 的引号可能被 cmd.exe 吃掉，尤其实例路径含空格时。
3. cmdlet 可能不返回带 `Id` 的对象 → 代码按 `UNKNOWN_PROCESS_ID`(0) 处理，**不算失败**，
   真正判据是 CDP 就绪。若列表 PID 列一直是空，属预期内。
4. `-PreventBreakaway` 的包身份是否真的传到 `start` 出来的孙进程（文档说「所有子进程」，
   但令牌「相似但不同」）。
5. 用户名含中文/非 ASCII 时，cmd 按 OEM 代码页解析 `.cmd` 是否正确。
6. **以管理员身份运行 Codex++ 时**：提权上下文激活 MSIX 会失败（issue #2351 的已知限制），
   这时应看到明确的失败提示，而不是静默卡住。
7. 同一台机器上**从未装过 Store 版**（免安装/自解包形态）时，多开不走包上下文，
   直接按路径启动——若此时 `--user-data-dir` 未被 Electron 尊重导致第二实例只是聚焦第一个窗口，
   请回报（这属于新暴露的问题，不是本次引入）。

## 7. 回报模板

```
Windows 版本 / Codex 包版本：
用例号：
操作：
实际现象：
关键命令行（Get-CimInstance 输出）：
相关日志行（codex-plus.log / launch-result.json）：
是否可复现：
```
