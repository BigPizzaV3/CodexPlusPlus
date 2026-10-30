# 导入 dsh 插件（deepseek-harness）设计

日期：2026-10-30
关联：`docs/plans/2026-10-30-codexpp-extension-system-design.md`（Codex++ 拓展体系的五个拦截层）

## 1. 目标

让用户把 dsh（[deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)）生态里的插件导入 Codex++，并在 Codex App 里真正用上。
dsh 的 agent 循环、会话存储、LLM 服务都不在 Codex App 里，所以「兼容」只能按层落地：
能映射到 Codex 已有扩展点（MCP 工具、skills、MCP 配置）的部分直接用，映射不了的明确告诉用户为什么。

## 2. dsh 插件长什么样（调研结论）

- **框架**：Cordis。插件是带 `name` / `inject` / `apply(ctx, config)` / `Config`（Schemastery）的 ES 模块，或 `Service` 子类。
  `inject` 声明依赖的服务（`ctx.tools`、`ctx.llm`、`ctx.sessions`…），服务全部就绪才会 `apply`。
  所有注册（工具、提示词片段、监听器）都是可撤销副作用，卸载时自动回收。插件在宿主进程内运行，没有沙箱。
- **分发单位是组合包（bundle）**：一个 npm 包，`package.json` 声明 `dsh.bundle.patch` 指向 `cordis.patch.yml`，
  patch 里用 `- insert: [ {id, name, config, disabled} ]` 列出插件行；`name` 是包名或相对路径，
  `cordis:group` 行把子行放在 `config` 里。没有 `dsh.bundle` 的包只是普通依赖（教程里的单文件插件也常这样直接被 patch 引用）。
- **依赖约定**：与宿主共享实例的 dsh 包（`@deepseek-ai/cordis`、`@deepseek-ai/dsh-tools` 等）放 `peerDependencies`，
  由宿主提供；独立的第三方依赖放 `dependencies`。这条约定正好让我们能用「兼容宿主」替代 dsh 运行时。
- **工具插件**：`ctx.tools.register(defineTool({ name, description, parameters, output: { schema, render }, execute }))`。
  `parameters` 是一个简化的属性表（`type` / `required` / `items` / `properties` / `oneOf` / `json`），
  `execute(args, exec)` 返回规范值，`output.render(args, value)` 转成给模型的内容块。
- **skill 包**：`assets/<name>/SKILL.md`（YAML frontmatter + 正文），通过 `ctx.skills` 注册。
  文件格式与 Codex skills 相同（`<id>/SKILL.md`）。
- **MCP**：`@deepseek-ai/dsh-mcp-client` 一行配置一个 server（`transport: stdio | streamable-http`，`serverName`、`command`/`args`/`env`/`cwd` 或 `url`/`headers`）。
- **前端插件**：`package.json` 声明 `dsh.client`，`./client` 入口通过 `ctx.slots` 向 dsh Web 客户端的 React slot 注册组件，
  依赖 dsh 的会话 store、连接与 locale 服务。
- **插件市场索引**：格式 1 的 JSON（`format`、`title`、`metadata`、`plugins[].versions[].source`），
  `source` 为 `{type: 'npm', registry?}` 或 `{type: 'git', url, commit, path?}`，列的都是组合包。

统计官方仓库里各包的 `inject`：只依赖 `tools`（可选加 `systemPrompt`）的工具插件是少数；
大部分官方工具依赖 `agents`、`sessions`、`sessionProjections`、`workingDirectory`、`shell`、`subagents`、`llm`、`workflowEngine` 等，
且多数是 Codex 已经自带的能力（文件、bash、web、todo、子 agent）。
**真正有导入价值的是第三方按教程写的工具插件、skill 包和 MCP 配置。**

## 3. 支持矩阵

| dsh 插件类别 | 判定依据 | 在 Codex++ 的落点 | 结论 |
|---|---|---|---|
| 工具插件（`inject` 只含 `tools` / `systemPrompt` / `logger` / `timer`） | 加载后 `ctx.tools.register` 出工具 | 兼容宿主进程内运行，经托管 MCP `codex_plus_dsh` 暴露为 Codex 工具 | **完全兼容**（工具调用、参数 schema、render、超时、取消） |
| 工具插件用到 `systemPrompt.section` | 同上 | 片段汇总进 MCP `instructions` | **部分兼容**：只在 MCP 初始化时给一次，不按回合重组；`context` / `variable` / `tools` 提供方忽略 |
| 工具插件用 `registerOptions.when`、`exec.agent`、`exec.deferContext`、`exec.concludeTurn`、`presentCall/presentResult` | 运行时调用 | 无对应扩展点 | **部分兼容**：`when` 视为始终可用；`exec.agent` 为空；`deferContext` / `concludeTurn` 空操作；展示钩子忽略 |
| 工具 render 出的图片块 | `type: 'image'`（引用 dsh 会话附件） | — | **不兼容**：附件存储属于 dsh 会话，转成文字占位 |
| skill 包（`SKILL.md`） | 包内扫描到 `SKILL.md` | 复制进 Codex++ 的 Skills 源目录并启用到 Codex | **完全兼容**（内容层面）；skill 文档里写死的 dsh 专属路径/工具名需要用户自行判断 |
| MCP 配置行（`dsh-mcp-client`） | patch 行 `name` 为 `@deepseek-ai/dsh-mcp-client` | 生成标准 `mcpServers` JSON，交给现有「导入 MCP」流程写入 Codex | **完全兼容**（stdio / streamable-http）；`reconnect`、`toolCallTimeoutMs` 等 dsh 专属字段丢弃 |
| `cordis:group` 行 | patch 行 | 展开子行逐一判定 | 按子行 |
| 依赖 agent 循环 / 会话 / LLM / 子 agent / 工作流 / shell / 工作目录服务的插件（`agents`、`sessions`、`sessionProjections`、`llm`、`subagents`、`workflowEngine`、`shell`、`workingDirectory`、`skills` 以外的服务…） | `inject` 必需项含不支持的服务 | 无 | **不兼容**：Codex App 的 agent 循环、会话存储与模型调用不在 Codex++ 手里，不加载并列出缺的服务 |
| `Service` 子类插件 | 默认导出为 class | 无 | **不兼容**：它们是给别的插件提供服务的，Codex 里没有消费者 |
| 前端 slot 插件（`dsh.client`） | `package.json` 有 `dsh.client` | 理论上落在 renderer 层（`codexPlus.extension`），但 slot、store、React 组件树都不同 | **不兼容（需移植）**：给出说明，建议改写为 Codex++ 拓展包 |
| bundle patch 里的 `!!js` 表达式、overlay、`remove`/`update` 操作 | YAML 标签 / 操作名 | — | **不支持**：只读取 `insert` 行；表达式值按字符串忽略，并在诊断里提示 |
| 引用 dsh 官方内置模块的行（`@deepseek-ai/dsh-*`，未随包安装） | 解析不到模块 | — | **不兼容**：多为 Codex 已自带的能力或依赖 agent 循环 |

## 4. 架构

```
Codex App ──stdio MCP──▶ Codex++ 可执行文件 --codex-plus-dsh-mcp --state-dir <state>
                              │ 定位 Node 运行时（ChatGPT.app 自带的 cua_node → PATH 里的 node）
                              │ 写出内嵌的兼容宿主脚本（内容不变不重写）
                              ▼
                         node <state>/dsh-plugins/host/host.mjs serve <registry.json>
                              ├─ 模块解析钩子：插件自身装不到的 peer 包回落到 shim
                              │    @deepseek-ai/dsh-tools  → defineTool（参数表 → JSON Schema）
                              │    @deepseek-ai/schemastery → 宽松的 Schema 构造器（只负责填默认值）
                              │    @deepseek-ai/cordis     → 占位 Context / Service
                              ├─ 每个插件一个受限 ctx：tools / systemPrompt / logger / effect / on / setTimeout…
                              └─ MCP：initialize / tools/list / tools/call / ping / notifications/cancelled
```

- **为什么走 Node 子进程而不是 renderer**：dsh 工具插件是 Node 代码（读写文件、子进程、网络），
  Codex 调用工具的唯一外部入口是 MCP；renderer 插件内核（`91-plugin-runtime.js`）在页面里，拿不到工具调用。
- **为什么是一个聚合 MCP**：所有导入的工具插件挂在同一个 `[mcp_servers.codex_plus_dsh]` 下，
  `config.toml` 只多一条由 Codex++ 托管的条目（读写、归属、备份复用 `managed_mcp.rs`）；启停插件只改注册表，不动 Codex 配置。
- **Node 运行时**：优先 Codex App 自带的 `ChatGPT.app/Contents/Resources/cua_node/bin/node`（Computer Use 接入已在用），
  其次 PATH 和常见安装目录；可用 `CODEX_PLUS_NODE` 覆盖。
- **stdout 保护**：宿主启动时把 `console.log/info/debug` 改到 stderr，插件的日志不会污染 MCP 帧。
- **安全边界**：与 dsh 一样，插件代码以用户权限在本机运行、没有沙箱。导入（会执行插件以列出工具）与启用都由用户在管理器里显式操作；
  npm 依赖安装一律 `--ignore-scripts`，不执行任何安装脚本。

## 5. 导入流程

来源三种，都落到「一个带 `package.json` 的包目录」：

| 来源 | 写法 | 处理 |
|---|---|---|
| 本地目录 | 绝对路径 | 原地链接（同 dsh `link:`），不复制；改源码后重启 MCP 生效 |
| npm 包 | `@scope/name`、`name@1.2.3` | `npm pack` 下载 tarball → 解包到 `<state>/dsh-plugins/packages/<id>/` |
| git 仓库 | `https://…/repo.git`，可加 `#path:sub/dir` 与 `#ref` | `git clone --depth 1` → 取子目录复制到包目录 |

之后统一：

1. 有 `dependencies` 时在包目录执行 `npm install --omit=dev --legacy-peer-deps --ignore-scripts`：
   peer（dsh 宿主包）不装，由兼容宿主提供；需要 npm，缺失时明确报错。
2. 调用兼容宿主 `inspect`：读 `package.json`、解析 bundle patch（YAML 子集）、逐行加载插件收集工具，
   扫描 `SKILL.md`，转换 MCP 行，输出逐行的兼容性报告。
3. 写入注册表 `<state>/dsh-plugins/registry.json`（来源、包目录、启用状态、最近一次报告）。
4. skill 复制进 Codex++ Skills 并启用到 Codex；MCP 行生成 `mcpServers` JSON，由用户确认后走现有「导入 MCP」流程。
5. 首次导入工具插件时提示「接入 Codex」（写托管 MCP 条目）；之后新开 Codex 对话即可使用。

**插件市场索引能否直接复用**：可以复用格式 1 的 `source` 语义——`npm` 对应「npm 包」来源（`name@version`），
`git` 对应「git 仓库」来源（`url` + `commit` + `path`）。首期不做索引浏览界面，后续可在导入面板里加「从索引 URL 浏览」。

## 6. 验证结论

- 按 dsh 教程写法的工具插件（`inject: ['tools', 'systemPrompt']`、`defineTool`、Schemastery `Config`）与组合包
  （`cordis:group`、MCP 行、依赖会话服务的行、引用 dsh 内置模块的行）逐行判定正确，工具经 MCP 调用结果一致；
- npm 上真实发布的 `@deepseek-ai/dsh-tool-workspace-dependencies`：`npm pack` + `--ignore-scripts` 安装 `dependencies`，
  peer 由替身提供；真实 Schemastery 生效（缺必填配置时报错），填写配置后工具可在 Codex 中调用；
- 官方 `@deepseek-ai/dsh-tool-todo` 等依赖 `sessionProjections` / `agents` 的工具判为不兼容，符合预期。

## 7. 首期范围与后续

首期实现：

- 兼容宿主（MCP 服务 + inspect）、三种来源导入、注册表、启停（整包 / 单行）与移除、托管 MCP 接入；
- 按行的插件配置覆盖（JSON，浅合并到 bundle patch 的 `config` 之上，保存后重新分析）——
  实测 npm 上的 `@deepseek-ai/dsh-tool-workspace-dependencies` 有必填配置，没有这一步无法加载；
- skill 导入、MCP 行转换为可导入的 JSON；
- 管理器「脚本」页新增「dsh 插件」分栏。

后续（按价值排序）：

1. 插件市场索引浏览（格式 1）；
2. 按插件 `Config` schema 生成表单（目前是 JSON 文本框）；
3. `systemPrompt` 片段改为 Codex 原生 developer instructions 注入（经模型网关层）；
4. 前端 slot 插件的移植指引 / 半自动改写为 Codex++ 拓展包。
