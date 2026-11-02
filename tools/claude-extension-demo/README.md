# Claude 拓展连通测试

这是 opt-in 的最小示例包，不会自动安装。

1. 在 Codex++ 的「万物皆拓展」中导入本目录，并启用。
2. 在「Claude → 增强」中打开「万物皆拓展」，重新启动增强版 Claude。
3. 打开 Claude Desktop 的 Code 本地会话，点击输入框上方的「拓展状态」。
4. 开始一个回合，再点击按钮，应看到 `turn.start`、`turn.complete` 事件。
5. 可以让 Claude 调用名称包含 `cpx_` 和 `status` 的拓展工具，读取同一个后台状态。

状态只保存在当前后台内存中；关闭会话后重置。示例只接收会话和回合的标识及状态，不接收提示词、回答或完整记录。完整接口见 [适配说明](../../docs/claude-desktop-extensions.md)。
