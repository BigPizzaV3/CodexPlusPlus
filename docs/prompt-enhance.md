# 提示词增强

在 Codex 输入框底部加一个增强按钮。点击后把当前草稿交给模型改写，结果
在可编辑的预览框里显示；用户点「替换」或「插入到末尾」后才写回输入框，
点「取消」或按 Esc 时输入框保持原样。

## 配置

设置保存在 `settings.json` 的 `promptEnhance` 对象中，默认关闭，并受
`enhancementsEnabled` 总开关控制。管理器的 Codex 增强页有独立的「提示词增强」
标签，保存后无需重启 Codex。

- `useStepwiseConnection` 默认开启，沿用下一步建议的 Base URL、API Key、协议
  和模型；关闭后使用本组的 `baseUrl`、`apiKey` / `apiKeyEnv`、`protocol`、`model`。
- `systemPrompt` 留空时使用内置改写规则：保留原意以及代码、路径、命令等细节，
  不编造需求，按需整理为目标、上下文、要求和预期结果，并沿用草稿语言。
- `timeoutSeconds` 范围 5–300 秒，默认 60 秒。

与 `dictation.apiKey` 一样，桥接下发设置时删掉 `promptEnhance.apiKey`，注入页
只拿到 `apiKeyConfigured`。

## 实现

- 渲染端入口是 manifest 的 `93-prompt-enhance.js`，复用语音输入的输入框
  定位和 footer 去重逻辑。开关关闭时不挂载 DOM。
- 页面经桥接调用 `/prompt-enhance/generate`，Rust 的 `prompt_enhance.rs`
  请求模型，并复用 `stepwise.rs` 的协议列表和请求头工具，支持 Chat
  Completions、Responses、Anthropic Messages 以及自动回退。页面不直接访问
  模型服务，也不接触 API Key。
- 写回 ProseMirror 时用一次事务替换或追加段落，可以 Ctrl+Z 撤销；textarea
  通过原型 setter 写入并派发 `input`。

以下情况不允许整段替换，只能插入到末尾或复制：

- 草稿中含有 @文件、技能、附件等非文本节点（替换会丢掉它们）；
- 生成期间草稿被修改过；
- 输入框或会话已经变化（这时插入也会被拒绝）。

## 验证

```sh
cargo test -p codex-plus-core --lib prompt_enhance
cargo test -p codex-plus-core --test cdp_bridge prompt_enhance
node assets/inject/prompt-enhance.test.cjs
cd apps/codex-plus-manager && node --test src/prompt-enhance-settings.test.ts src/enhancement-settings.test.ts
```
