# Windows 主题 CSS 的标题栏高度和边距

新版 Codex 的侧栏和聊天标题栏结构发生变化。主题作者需要在 `theme.css` 中调整相应部件的高度、边距；原有 Safe CSS 校验不接受这些属性。本扩展提供按主题配置的入口，未声明时沿用原有布局。

本次仅开放以下 Windows shell 属性：

| Skin API 部件 | 属性 | 支持值 |
| --- | --- | --- |
| `sidebar` | `margin-top` | `0` 或 `0px`～`64px` |
| `header` | `margin-inline-end` | `0` 或 `0px`～`64px` |
| `header` | `height` | `32px`～`128px` |

例如，主题作者可选择：

```css
[data-ds-part="sidebar"] {
  margin-top: 0;
}

[data-ds-part="header"] {
  margin-inline-end: 0;
  height: 52px;
}
```

这些只是示例数值，不会自动写入任何主题。也可使用不同的合法数值，或删除相应声明以恢复原有布局。

将声明放入主题包的 `theme.css`，更新 manifest 中对应文件的大小与 SHA-256 后，按原有流程导入、保存及应用主题。无需 `!important`，客户端会在校验通过后添加覆盖优先级。Safe CSS 文件不接受注释。

布局声明只在主题启用且运行于 Windows 时生效。颜色、字体等原有外观声明仍沿用原来的跨平台规则。没有改动 macOS renderer，也没有 macOS 实机测试结果。

回归测试可先通过 `cargo test --release -p codex-plus-core --test all dream_skin_package::` 验证校验、包导入和保存。设置 `CODEX_THEME_LAYOUT_CSS_OUTPUT` 为一个可写 JSON 文件路径后，该测试会导出实际编译结果；沿用此环境变量运行 `node scripts/test-theme-shell-performance.cjs`，可在浏览器中验证自定义数值、原生 important 层叠、换肤及关闭主题后的恢复。
