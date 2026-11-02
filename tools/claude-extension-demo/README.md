# Claude 拓展连通测试

这是 opt-in 的最小示例包，不会自动安装。

1. 将完整的 `claude-extension-demo` 目录安装到用户脚本目录：macOS 默认为 `~/.config/Codex++/user_scripts/`，Windows 为 `%APPDATA%\Codex++\user_scripts\`。在「万物皆拓展」顶部切换到 Claude，刷新并启用本地拓展，同时打开「启用 Claude 拓展」总开关。
2. 在「Claude → 增强」中制作并启动增强版 Claude；已有增强版时重新启动即可。
3. 打开 Claude Desktop 的 Code 本地会话，点击输入框上方的「拓展状态」。
4. 开始一个回合，再点击按钮，应看到 `turn.start`、`turn.complete` 事件。
5. 可以让 Claude 调用名称包含 `cpx_` 和 `status` 的拓展工具，读取同一个后台状态。

状态只保存在当前后台内存中；关闭会话后重置。示例只接收会话和回合的标识及状态，不接收提示词、回答或完整记录。完整接口见 [适配说明](../../docs/claude-desktop-extensions.md)。
