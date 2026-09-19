# 会话栏 Work 标签与 API 按钮排查（2026-09-20）

## 实际原因

- API 是本地 Bennett UI Improvements 1.2.4 脚本的 `data-codexpp="usage-box"` 用量按钮，不是输入框。现场没有 API input。
- 脚本的 `findSidebarSlot` 按底部区域、紧凑按钮及行尺寸猜测底栏；现场 `usage-slot` 实际挂在 `data-codex-tab-conversation-drop-target` 内。会话操作控件会满足其几何条件，已有错误挂载位置随后又被复用。
- 原始会话选择器不是 API 的来源，已撤销 468e297 中没有证据支持的 `:has([data-thread-title])` 限制。
- 无法从本地脚本当前版本确定首次引入的历史提交。已检查的管理器更新按钮、脚本热重载提交没有改写上述定位算法。
- Work 标签会在模式切换时重建；嵌套的 thread-id 容器还会互相撤销标记。后台动画帧和定时扫描不保证立即运行，因此增加只命中标题旁类型标签的 CSS 结构规则，不依赖异步标记完成。

## 修复

- 保留标题旁标签的占位，在 hover、focus-within、菜单展开时隐藏。
- 标记更新仅处理最近所属会话行，并在同步扫描及已有按钮路径中更新。
- 仅将误挂于会话内的 Bennett 用量节点移至底部个人资料栏；复用节点，保留事件和状态。没有安装该用户脚本时无操作。

## 验证

- `cargo test -p codex-plus-core --test cdp_bridge sidebar_`：8 项通过，包括同名标题保护、嵌套行标记、节点复用、用量控件重复修复及无底栏情况。
- 在运行中的客户端实际切换 ChatGPT Work → Codex → ChatGPT Work。切换后强制 hover 的 Work 标签计算样式为 `hidden`；会话拖放容器内的 API 用量节点数量为 0。
- 前端及两个 Windows Release EXE 构建；桌面快捷方式目标为 `%LOCALAPPDATA%/Programs/Codex++`。替换前备份、替换后验证 SHA256。
- 当前页面已加载修复脚本；运行中的旧 EXE 不强制结束，新 EXE 在正常重启后加载。
