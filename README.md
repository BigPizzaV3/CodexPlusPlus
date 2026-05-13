# Codex++

<p align="center">
  <img src="docs/images/codex-plus-plus.png" alt="Codex++ 图标" width="160">
</p>

<p align="center">
  中文 | [English](README_EN.md)
</p>

<p align="center">
  <img alt="Release" src="https://img.shields.io/github/v/release/BigPizzaV3/CodexPlusPlus">
  <img alt="Stars" src="https://img.shields.io/github/stars/BigPizzaV3/CodexPlusPlus">
  <img alt="License" src="https://img.shields.io/github/license/BigPizzaV3/CodexPlusPlus">
  <img alt="Python" src="https://img.shields.io/badge/python-3.11%2B-blue">
</p>

Codex++ 是面向 Codex App 的外部增强启动器。它不修改 Codex App 原始安装文件，而是通过外部 launcher 启动 Codex，并使用 Chromium DevTools Protocol 注入增强脚本。

## 目录

- [讨论交流](#讨论交流)
- [赞赏支持](#赞赏支持)
- [快速使用](#快速使用)
- [功能亮点](#功能亮点)
- [界面预览](#界面预览)
- [Provider 同步](#provider-同步)
- [友情链接](#友情链接)
- [工作方式](#工作方式)
- [环境要求](#环境要求)
- [Windows 使用](#windows-使用)
  - [图形菜单安装/卸载](#图形菜单安装卸载)
  - [命令行安装](#命令行安装)
  - [命令行卸载](#命令行卸载)
  - [Windows 自动接管（可选）](#windows-自动接管可选)
- [自动更新](#自动更新)
- [macOS 使用](#macos-使用)
- [直接启动](#直接启动)
- [数据与备份](#数据与备份)
- [常见问题](#常见问题)
- [贡献者与 Star](#贡献者与-star)
- [开发](#开发)

## 讨论交流

欢迎扫码加入 Codex++ 交流群，反馈问题、交流使用体验或提出新功能建议：

<img src="docs/images/discussion-group-qr.jpg" alt="Codex++ 交流群二维码" width="260">

## 赞赏支持

如果 Codex++ 帮到了你，可以请我喝杯咖啡，或者随手赞赏支持一下继续维护。

<p align="center">
  <img src="docs/images/sponsor-alipay.jpg" alt="支付宝赞赏码" width="220">
  <img src="docs/images/sponsor-wechat.jpg" alt="微信赞赏码" width="220">
</p>

## 快速使用

Windows 用户可以直接双击项目根目录的：

```text
setup.bat
```

然后选择：

```text
[1] Install Codex++
```

安装后桌面会生成 `Codex++.lnk`，双击即可启动带增强功能的 Codex。

也可以在项目目录通过命令行安装和启动：

```bash
python -m pip install -e .
python -m codex_session_delete setup
python -m codex_session_delete launch
```

macOS 用户可以执行：

```bash
python -m codex_session_delete setup
```

安装后会生成 `/Applications/Codex++.app`。

## 功能亮点

- 顶部菜单栏加入 `Codex++` 菜单，可集中管理增强功能。
- 插件选项解锁：让 API Key 模式显示并启用插件入口。
- 特殊插件强制安装：解除 App unavailable / 应用不可用导致的前端安装禁用。
- 会话删除：在会话列表悬停显示删除按钮，删除前确认并支持撤销。
- Markdown 导出：按本地 rollout 导出带时间戳的会话 Markdown。
- Context Guard / UI handoff：线程接近上下文窗口上限时，生成轻量交接文件，辅助在新聊天里继续任务。
- 会话项目移动：把会话移动到普通对话或其他本地项目。
- 对话 Timeline：在对话右侧显示用户提问时间线，悬停查看摘要并快速跳转。
- Provider 同步：切换 model_provider 或供应商时不丢历史会话。
- Windows 快捷方式安装/卸载、常驻 watcher 自动接管、GitHub Release 自动更新。
- macOS 生成 `/Applications/Codex++.app`。

## 界面预览

API Key 登录模式下，Codex 原生插件入口会提示需要登录 ChatGPT，导致插件功能无法正常使用：

![API Key 模式下插件入口不可用](docs/images/pain-plugin-disabled.png)

Codex 原生会话列表只有归档入口，没有真正的删除按钮：

![原生会话列表缺少删除能力](docs/images/pain-no-delete-button.png)

Codex++ 启动后会解锁插件入口，并在会话列表悬停时显示删除按钮：

![Codex++ 解锁插件入口并添加删除按钮](docs/images/solution-plugin-and-delete.png)

顶部菜单栏会出现 `Codex++`，可以查看后端状态并打开设置面板：

![Codex++ 后端状态指示灯](docs/images/backend-status-indicator.png)

![Codex++ 设置面板](docs/images/settings-panel.png)

## Provider 同步

启用 `Provider 同步` 后，Codex++ 会在启动 Codex 前同步本地会话 metadata。它会把 rollout 文件、SQLite 线程记录和项目路径缓存同步到当前 `model_provider`，让你切换供应商时不丢历史会话。

适合这些场景：

- 从 OpenAI 切换到第三方 provider 后，旧会话在 Desktop 或 `/resume` 中不可见。
- 切回其他 provider 后，希望历史对话继续出现在原项目下。
- Windows 路径带有 `\\?\` 前缀导致 Desktop 项目列表匹配不到旧会话。

同步只修复会话可见性相关 metadata，不改写对话内容；如果 Codex 正在占用某个会话文件或 SQLite 忙碌，Codex++ 会跳过并继续启动，避免阻塞 Codex。

## 友情链接

- [LINUX DO](https://linux.do)

## 工作方式

Codex++ 使用外部启动方式运行 Codex：

1. 启动 Codex App，并附加：
   - `--remote-debugging-port=9229`
   - `--remote-allow-origins=http://127.0.0.1:9229`
2. 如果启用了 Provider 同步，启动 Codex 前先同步历史会话 metadata。
3. 启动本地 helper 服务，保留健康检查和运行生命周期。
4. 通过 CDP 注入 `renderer-inject.js`。
5. 渲染端通过 CDP bridge 调用本地服务；默认不开放 HTTP 删除/撤销入口，避免本机其他页面误触发删除类操作。
6. 启动 Codex 时继承现有 `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY`；如果这些环境变量未设置，会自动探测常见本地代理端口（如 `127.0.0.1:7897`），帮助 Codex 加载需要访问 GitHub 的技能资源。

这种方式不会修改 Codex 的 `app.asar`，也不需要往 Codex 安装目录写 DLL。

## 环境要求

- Python 3.11+
- Windows 或 macOS
- 已安装 Codex App

安装依赖：

```bash
python -m pip install -e .
```

如需运行测试：

```bash
python -m pip install -e .[test]
python -m pytest -q
```

## Windows 使用

### 图形菜单安装/卸载

双击项目根目录的：

```text
setup.bat
```

然后按菜单选择：

```text
[1] Install Codex++
[2] Uninstall Codex++
[3] Update Codex++
[4] Exit
```

### 命令行安装

在项目目录执行：

```bash
python -m codex_session_delete setup
```

安装后会在桌面生成：

```text
Codex++.lnk
```

双击该快捷方式启动 Codex++。

### 命令行卸载

可以在系统“设置 → 应用 → 已安装的应用”里卸载 `Codex++`。

也可以在项目目录执行：

```bash
python -m codex_session_delete remove
```

如需同时删除 Codex++ 自己的日志和备份数据：

```bash
python -m codex_session_delete remove --remove-data
```

### Windows 自动接管（可选）

默认情况下 Codex++ 只在你从 `Codex++` 快捷方式启动时生效。如果你从开始菜单、任务栏或系统原生入口直接启动 Codex，那一次不会有注入，`Codex++` 菜单和插件解锁都不会出现。

Windows 可以注册一个常驻 watcher 解决这个问题。它会每 3 秒探测一次本机 CDP 端口，发现 Codex 在跑但 CDP 没起来，会先短暂等待并二次确认，确认仍没有 CDP 后再把 Codex Desktop App 进程重拉为带注入的版本。

安装：

```bash
python -m codex_session_delete watch-install
```

卸载：

```bash
python -m codex_session_delete watch-remove
```

临时开关：

```bash
python -m codex_session_delete watch-disable
python -m codex_session_delete watch-enable
```

日志：

```text
%USERPROFILE%\.codex-session-delete\watcher.log
```

## 自动更新

Codex++ 会在启动时检查 GitHub Release。如果发现比本地版本更新的 Release，会在控制台提示版本号、Release 地址和更新命令；检查失败不会影响 Codex++ 启动。

手动检查更新：

```bash
python -m codex_session_delete check-update
```

从最新 GitHub Release 更新：

```bash
python -m codex_session_delete update
```

更新流程：

1. 请求 `https://api.github.com/repos/BigPizzaV3/CodexPlusPlus/releases/latest`。
2. 比较最新 Release tag 与本地版本。
3. 优先下载 Release 中的 `.whl` asset。
4. 执行 `python -m pip install --upgrade <wheel>`。
5. 自动重新执行 `python -m codex_session_delete setup`，刷新快捷方式、Windows 卸载项或 macOS app bundle。

发布新版本时，请在 GitHub Release 里附加 wheel 文件，例如：

```bash
python -m build
```

然后把 `dist/codex_session_delete-<version>-py3-none-any.whl` 上传到对应 Release。

## macOS 使用

### 安装

```bash
python -m codex_session_delete setup
```

默认会自动查找 `/Applications/Codex.app`、`/Applications/OpenAI Codex.app` 或用户 Applications 目录下的 Codex 应用，并生成：

```text
/Applications/Codex++.app
```

### 卸载

```bash
python -m codex_session_delete remove
```

## 直接启动

不安装快捷方式时，也可以直接运行：

```bash
python -m codex_session_delete launch
```

常用参数：

```bash
python -m codex_session_delete launch \
  --app-dir "/Applications/OpenAI Codex.app" \
  --debug-port 9229 \
  --helper-port 57321
```

Windows 也可以手动指定 Codex 安装目录：

```bash
python -m codex_session_delete launch \
  --app-dir "C:/Program Files/WindowsApps/OpenAI.Codex_xxx/app" \
  --debug-port 9229 \
  --helper-port 57321
```

## 数据与备份

Codex++ 默认读取 Codex 本地数据库：

```text
~/.codex/state_5.sqlite
```

删除前会把相关记录备份到：

```text
~/.codex-session-delete/backups
```

Provider 同步会把同步前状态备份到：

```text
~/.codex/backups_state/provider-sync
```

隐藏启动失败日志位于：

```text
~/.codex-session-delete/launcher.log
```

## 常见问题

### 双击 Codex++ 没反应

先查看日志：

```text
%USERPROFILE%\.codex-session-delete\launcher.log
```

常见原因：

- Codex App 没有安装或路径变化
- 9229 端口被占用
- Python 环境不可用

### 技能推荐加载失败

如果技能页提示 `git fetch failed`、`unable to access 'https://github.com/openai/skills.git/'` 或无法连接 GitHub，通常是本机网络不能直连 GitHub。Codex++ 启动时会优先继承现有代理环境变量；如果未设置，会自动探测常见本地代理端口。也可以手动指定：

```powershell
$env:HTTP_PROXY="http://127.0.0.1:7897"
$env:HTTPS_PROXY="http://127.0.0.1:7897"
python -m codex_session_delete launch
```

### Codex++ 菜单没出现

确认是从 `Codex++` 快捷方式启动，而不是直接启动原版 Codex。

也可以检查 Codex 是否带了 CDP 参数：

```text
--remote-debugging-port=9229
```

### 切换供应商后旧会话不见了

打开 `Codex++` 设置面板，启用 `Provider 同步` 后重新启动 Codex++。它会在启动 Codex 前同步当前 `model_provider`，让历史会话重新匹配当前供应商。

### 长任务提示 context window 不够

当 Codex 报出 `Codex ran out of room in the model's context window` 时，可以生成一个轻量交接包，再开新聊天继续，不需要把旧 rollout、完整日志或 base64 图片重新塞进上下文。

```bash
python -m codex_session_delete context-guard handoff <thread-id-or-rollout-file>
python -m codex_session_delete context-guard handoff <thread-id> --copy
python -m codex_session_delete context-guard watch-once --copy
```

这个能力提炼自 [codex-context-guard](https://github.com/zjq12333/codex-context-guard)。Codex++ 集成的是保守 UI handoff 流程：只写本地 Markdown 交接文件、可选复制提示，不直接修改 Codex Desktop 内部数据库，不自动打开或重启 Codex Desktop，也不会替用户自动发送消息。

### Windows 系统卸载失败

请先更新到当前版本后重新安装一次：

```bash
python -m codex_session_delete setup
```

新版会写入稳定的系统卸载项，并使用绝对 Python 路径执行卸载。

## 贡献者与 Star

<a href="https://github.com/BigPizzaV3/CodexPlusPlus/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=BigPizzaV3/CodexPlusPlus" alt="Codex++ contributors">
</a>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=BigPizzaV3/CodexPlusPlus&type=Date&theme=dark">
  <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=BigPizzaV3/CodexPlusPlus&type=Date">
  <img alt="Codex++ Star History" src="https://api.star-history.com/svg?repos=BigPizzaV3/CodexPlusPlus&type=Date">
</picture>

## 开发

运行测试：

```bash
python -m pytest -q
```

项目结构：

```text
codex_session_delete/
  cli.py                 CLI 入口
  launcher.py            启动 Codex 并注入脚本
  cdp.py                 CDP 通信与 bridge
  helper_server.py       本地 helper 服务
  storage_adapter.py     本地 SQLite 删除/撤销
  provider_sync.py       Provider 同步
  settings_store.py      Codex++ 后端设置
  windows_installer.py   Windows 快捷方式与卸载项
  macos_installer.py     macOS app bundle 安装
  watcher.py             Windows 常驻 watcher（可选，原生启动接管）
  inject/renderer-inject.js

tests/                   自动化测试
```

## 说明

Codex++ 是外部增强工具，不修改 Codex App 原始文件。Codex App 更新后，如果页面结构变化，可能需要更新注入脚本。
