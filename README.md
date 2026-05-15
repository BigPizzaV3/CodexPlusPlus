# Codex++

<p align="center">
  <img src="docs/images/codex-plus-plus.png" alt="Codex++ 图标" width="160">
</p>

<p align="center">
  中文 | <a href="README_EN.md">English</a>
</p>

<p align="center">
  <img alt="Release" src="https://img.shields.io/github/v/release/BigPizzaV3/CodexPlusPlus">
  <img alt="Stars" src="https://img.shields.io/github/stars/BigPizzaV3/CodexPlusPlus">
  <img alt="License" src="https://img.shields.io/github/license/BigPizzaV3/CodexPlusPlus">
  <img alt="Python" src="https://img.shields.io/badge/python-3.11%2B-blue">
</p>

Codex++ 是面向 Codex App 的外部增强启动器：不修改原始安装文件，通过 Chromium DevTools Protocol 注入增强脚本。

## 快速使用

Windows 用户双击项目根目录的 `setup.bat`，选择：

```text
[1] Install Codex++
```

安装后双击桌面 `Codex++.lnk` 启动。

命令行安装/启动：

```bash
python -m pip install -e .
python -m codex_session_delete setup
python -m codex_session_delete launch
```

macOS：

```bash
python -m codex_session_delete setup
```

安装后会生成 `/Applications/Codex++.app`。

## 交流与支持

欢迎扫码加入 Codex++ 交流群，反馈问题、交流使用体验或提出新功能建议：

<img src="docs/images/discussion-group-qr.jpg" alt="Codex++ 交流群二维码" width="260">

如果 Codex++ 帮到了你，可以请我喝杯咖啡，或者随手赞赏支持一下继续维护。

<p align="center">
  <img src="docs/images/sponsor-alipay.jpg" alt="支付宝赞赏码" width="220">
  <img src="docs/images/sponsor-wechat.jpg" alt="微信赞赏码" width="220">
</p>

## 赞助商

<table>
  <tr>
    <th>🏆 赞助商 🏆</th>
  </tr>
  <tr>
    <td>👉 <a href="https://rawchat.cn">RawChat｜Codex 中转站</a> 老牌中转站，支持包月套餐。低倍率调用，高缓存命中，Pro/Plus 号池，全天专人维护。</td>
  </tr>
</table>

## 功能亮点

- 顶部 `Codex++` 菜单：集中管理增强功能。
- 插件入口解锁：API Key 模式下显示并启用插件入口。
- 特殊插件强制安装：解除 App unavailable / 应用不可用导致的前端安装禁用。
- 会话删除：悬停显示删除按钮，删除前确认并支持撤销。
- Markdown 导出：按本地 rollout 导出带时间戳的会话 Markdown。
- 会话项目移动：把会话移动到普通对话或其他本地项目。
- 对话 Timeline：右侧显示用户提问时间线，悬停摘要，点击跳转。
- Provider 同步：切换 model_provider 或供应商时不丢历史会话。
- Windows 快捷方式、卸载项、可选 watcher 自动接管、GitHub Release 更新。
- macOS `/Applications/Codex++.app` 生成。

## 痛点与解决

API Key 登录模式下，Codex 原生插件入口会提示需要登录 ChatGPT，导致插件功能无法正常使用：

![API Key 模式下插件入口不可用](docs/images/pain-plugin-disabled.png)

Codex 原生会话列表只有归档入口，没有真正的删除按钮：

![原生会话列表缺少删除能力](docs/images/pain-no-delete-button.png)

Codex++ 启动后会解锁插件入口，并在会话列表悬停时显示删除按钮：

![Codex++ 解锁插件入口并添加删除按钮](docs/images/solution-plugin-and-delete.png)

顶部菜单栏会出现 `Codex++`，可以查看后端状态并打开设置面板：

![Codex++ 后端状态指示灯](docs/images/backend-status-indicator.png)
![Codex++ 设置面板](docs/images/settings-panel.png)

## 工作方式

1. 外部启动 Codex App，并附加 CDP 参数：
   - `--remote-debugging-port=9229`
   - `--remote-allow-origins=http://127.0.0.1:9229`
2. 启动本地 helper 服务，用于健康检查、设置、导出、移动、删除等操作。
3. 通过 CDP 注入 `renderer-inject.js`。
4. 渲染端通过 CDP bridge 调用本地服务；默认不开放 HTTP 删除/撤销入口，避免本机其他页面误触发。
5. 启动时继承现有代理环境变量；若未设置，会自动探测常见本地代理端口帮助加载 GitHub 资源。

这种方式不会修改 Codex 的 `app.asar`，也不需要往 Codex 安装目录写 DLL。

## Provider 同步

启用 `Provider 同步` 后，Codex++ 会在启动前同步本地会话 metadata，让切换供应商后历史会话仍能在 Desktop 和 `/resume` 中显示。

同步范围包括 rollout 文件、SQLite 线程记录和项目路径缓存；只修复会话可见性 metadata，不改写消息内容。遇到文件锁或 SQLite 忙碌时会跳过并继续启动。

## 常用命令

```bash
# 安装依赖
python -m pip install -e .

# 启动
python -m codex_session_delete launch

# 安装快捷方式 / app bundle
python -m codex_session_delete setup

# 卸载
python -m codex_session_delete remove

# 同时删除日志和备份
python -m codex_session_delete remove --remove-data

# 检查更新 / 更新
python -m codex_session_delete check-update
python -m codex_session_delete update

# Windows watcher 自动接管
python -m codex_session_delete watch-install
python -m codex_session_delete watch-remove
python -m codex_session_delete watch-disable
python -m codex_session_delete watch-enable
```

直接指定 Codex 安装目录：

```bash
python -m codex_session_delete launch \
  --app-dir "C:/Program Files/WindowsApps/OpenAI.Codex_xxx/app" \
  --debug-port 9229 \
  --helper-port 57321
```

## 数据位置

- Codex 本地数据库：`~/.codex/state_5.sqlite`
- 删除备份：`~/.codex-session-delete/backups`
- Provider 同步备份：`~/.codex/backups_state/provider-sync`
- 启动失败日志：`~/.codex-session-delete/launcher.log`
- watcher 日志：`%USERPROFILE%\.codex-session-delete\watcher.log`

## 常见问题

### 双击 Codex++ 没反应

查看日志：`%USERPROFILE%\.codex-session-delete\launcher.log`

常见原因：Codex App 未安装或路径变化、9229 端口被占用、Python 环境不可用。

### Codex++ 菜单没出现

确认是从 `Codex++` 快捷方式启动，而不是原版 Codex。也可以检查 Codex 是否带有 `--remote-debugging-port=9229`。

### 技能推荐加载失败

如果提示 `git fetch failed` 或无法连接 GitHub，通常是网络无法直连 GitHub。Codex++ 会继承代理环境变量，也会自动探测常见本地代理端口。也可以手动指定：

```powershell
$env:HTTP_PROXY="http://127.0.0.1:7897"
$env:HTTPS_PROXY="http://127.0.0.1:7897"
python -m codex_session_delete launch
```

### 切换供应商后旧会话不见了

打开 `Codex++` 设置面板，启用 `Provider 同步` 后重启 Codex++。

## 开发

```bash
python -m pip install -e .[test]
python -m pytest -q
```

主要结构：

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
  watcher.py             Windows watcher（可选）
  inject/renderer-inject.js

tests/                   自动化测试
```

## 友情链接

- [LINUX DO](https://linux.do)

## 贡献者与 Star

<a href="https://github.com/BigPizzaV3/CodexPlusPlus/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=BigPizzaV3/CodexPlusPlus" alt="Codex++ contributors">
</a>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=BigPizzaV3/CodexPlusPlus&type=Date&theme=dark">
  <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=BigPizzaV3/CodexPlusPlus&type=Date">
  <img alt="Codex++ Star History" src="https://api.star-history.com/svg?repos=BigPizzaV3/CodexPlusPlus&type=Date">
</picture>

## 说明

Codex++ 是外部增强工具，不修改 Codex App 原始文件。Codex App 更新后，如果页面结构变化，可能需要更新注入脚本。
