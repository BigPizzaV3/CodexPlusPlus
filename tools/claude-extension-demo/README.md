# Claude 拓展连通测试

这是 opt-in 的最小示例包，不会自动安装。它同时验证两部分：注入 Claude 页面的脚本（`claudeDesktop.renderer`），和 Code 本地会话里的按钮、工具、事件。

1. 将完整的 `claude-extension-demo` 目录安装到用户脚本目录：macOS 默认为 `~/.config/Codex++/user_scripts/`，Windows 为 `%APPDATA%\Codex++\user_scripts\`。在「万物皆拓展」顶部切换到 Claude，打开「启用 Claude 拓展」总开关和这个拓展的开关。
2. 在「Claude → Desktop 增强」启动 Claude（完整功能模式或补丁版都行）；已经开着的话刷新 Claude 页面（⌘R / Ctrl+R）。

## 页面部分

1. 页面加载后右下角出现「Claude 拓展已加载（claudeDesktop）」的提示；提示文字可以在拓展的设置里改，保存后刷新页面生效。
2. 右下角的悬浮按钮里有「Claude 拓展连通测试」页面入口，点开后点「调后台 status」，显示后台返回的内容。
3. 按 `⌘⇧E`（Windows 是 `Ctrl+Shift+E`）应弹出「快捷键生效」。
4. 回到「万物皆拓展」的 Claude 页签，这一行应显示「已加载」。

## Code 会话部分

1. 重新启动 Claude（Code 会话里的按钮、工具走 Claude Code Mods，重启后才加载），打开一个 Code 本地会话，点击输入框上方的「拓展状态」。
2. 开始一个回合，再点击按钮，应看到 `turn.start`、`turn.complete` 事件。
3. 可以让 Claude 调用名称包含 `cpx_` 和 `status` 的拓展工具，读取同一个后台状态。
4. Codex++ 开着时，页面里的按钮和这里的按钮调的是同一个后台实例：返回文字里「Code 会话按钮」「页面调用」两个计数会一起增长。

状态只保存在当前后台内存中；后台重启后重置。示例只接收会话和回合的标识及状态，不接收提示词、回答或完整记录。完整接口见 [适配说明](../../docs/claude-desktop-extensions.md)。
