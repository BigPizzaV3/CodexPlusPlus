# 导入 dsh 插件（deepseek-harness）设计

日期：2026-10-30
关联：`docs/plans/2026-10-30-codexpp-extension-system-design.md`（Codex++ 拓展体系的五个拦截层）

## 1. 目标

让用户把 dsh（[deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)）生态里的插件导入 Codex++，并在 Codex App 里真正用上。
dsh 的 agent 循环、会话存储、LLM 服务都不在 Codex App 里，所以「兼容」只能按层落地：
能映射到 Codex 已有扩展点（MCP 工具、skills、MCP 配置、Codex 钩子）与 Codex++ 网关的部分直接用，映射不了的明确告诉用户为什么。

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
| 用到 `systemPrompt.section` / `context` / `variable` 的插件（含只做提示词的插件，如 `dsh-persona`） | 加载时截取注册 | 配套拓展包的网关请求中间件把片段追加到每次模型请求的 `instructions`（静态文本分析时求好直接嵌入；有函数片段或变量时每次请求求值）；同时汇总进 MCP `instructions` | **完全兼容**（经 Codex++ 本地代理 / 网关的请求）；官方直连的请求不经网关，只剩 MCP `instructions`；`complete` 片段只追加不替换 Codex 自己的提示词；`context` 并入 instructions 而不是逐回合刷新的用户侧快照；未注册的 `{{变量}}` 原样保留；`tools` 提供方忽略 |
| 监听 `tools/pre-execute` / `tools/post-execute` / `agent/pre-step` / `agent/turn-stopping` 的插件（策略、守卫类，如 `dsh-repeat-tool-reminder`） | `ctx.on` 这四个事件 | 配套拓展包声明对应的 Codex 钩子 PreToolUse / PostToolUse / UserPromptSubmit / Stop，钩子脚本经 `bridge.mjs` 派发给插件的监听器，把 dsh 的决定翻译成钩子输出（见 4.1） | **完全兼容**（拦截、补上下文、拒绝输入、要求继续）；dsh 决定里 Codex 钩子表达不了的部分忽略（见 4.1）；需要用户在 Codex 里信任一次「Codex++ 拓展钩子」 |
| 监听其它 dsh 事件 | `ctx.on` | 见第 8、9 节：大部分已有落点（会话镜像、网关、钩子、插件间事件总线） | 首期**不生效**；现状以第 8、9 节为准，仍不触发的事件在报告里逐条注明 |
| 工具插件用 `registerOptions.when`、`exec.agent`、`exec.deferContext`、`exec.concludeTurn`、`presentCall/presentResult` | 运行时调用 | 无对应扩展点 | **部分兼容**：`when` 视为始终可用；`exec.agent` 为空；`deferContext` / `concludeTurn` 空操作；展示钩子忽略 |
| 工具 render 出的图片块 | `type: 'image'`（引用 dsh 会话附件） | — | **不兼容**：附件存储属于 dsh 会话，转成文字占位 |
| skill 包（`SKILL.md` 文件，或经 `ctx.skills.register` / `registerProvider` 在代码里注册） | 包内扫描到 `SKILL.md`；或加载插件时截取 `ctx.skills` 的注册（宿主提供 `skills` 服务与 `@deepseek-ai/dsh-skill` 替身），依赖包里的 skill 也能拿到；bundle 行引用的依赖包里现成的 `SKILL.md` 也会扫描（最多 64 个，不跟软链，指向包外的依赖不扫） | 代码注册的写成 `SKILL.md` 目录（带上资源目录里的文件），与文件形式的一起复制进 Codex++ 的 Skills 源目录并启用到 Codex | **完全兼容**（内容层面）；skill 文档里写死的 dsh 专属路径/工具名需要用户自行判断 |
| MCP 配置行（`dsh-mcp-client`） | patch 行 `name` 为 `@deepseek-ai/dsh-mcp-client` | 生成标准 `mcpServers` JSON，交给现有「导入 MCP」流程写入 Codex | **完全兼容**（stdio / streamable-http）；`reconnect`、`toolCallTimeoutMs` 等 dsh 专属字段丢弃 |
| `cordis:group` 行 | patch 行 | 展开子行逐一判定 | 按子行 |
| 依赖 agent 循环 / 会话 / LLM / 子 agent / 工作流 / shell / 工作目录服务的插件（`agents`、`sessions`、`sessionProjections`、`llm`、`subagents`、`workflowEngine`、`shell`、`workingDirectory` 等服务…） | `inject` 必需项含不支持的服务，或导入了宿主没有替身的 dsh 运行时包（如 `@deepseek-ai/dsh-agent-loop`），或用到替身之外的导出（`@deepseek-ai/dsh-llm` 只替身了 `createUserMessage` 等构造消息的函数，`@deepseek-ai/dsh-system-prompt` 只替身了片段名常量与渲染函数） | 无 | **不兼容**：Codex App 的 agent 循环、会话存储与模型调用不在 Codex++ 手里，不加载并列出缺的服务 / 导出 |
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

导入 / 重新分析 / 启停 / 改配置后，按报告生成配套拓展包 <用户脚本目录>/dsh-<插件 id>/（只有钩子与网关，没有页面脚本）：
  codexpp.json  hooks: { PreToolUse | PostToolUse | UserPromptSubmit | Stop: [hooks/dsh.mjs] }
                gateway: { request: [gateway/instructions.mjs] }
  dsh-bridge.json  注册表位置 + 插件 id（也是「由 Codex++ 生成」的标记，移除时只动带它的目录）
  prompt.txt       静态提示词（分析时求好）

Codex 钩子 ──▶ Codex++ --codex-plus-hook <事件>（拓展钩子统一分发，一个 Node 进程）
                └─ hooks/dsh.mjs ─▶ <state>/dsh-plugins/host/bridge.mjs runHook
Codex++ 本地代理 / 网关 ──▶ 网关中间件链 ─▶ gateway/instructions.mjs（静态：读 prompt.txt；动态：bridge requestInstructions）

bridge ──本地 socket──▶ 常驻进程 node bridge.mjs daemon <dsh-bridge.json>（每个插件一个，按需启动）
                          加载插件里有映射的行；每个 Codex 会话一个稳定的 agent 替身，插件按 agent 记的状态跨事件保留；
                          空闲 20 分钟、配置文件不在了、插件 / 配置 / 宿主文件变了（版本号不符）都会退出，下次请求再起；
                          起不来时退回到当前进程临时加载一次（这时状态不跨事件）。
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

### 4.1 dsh 事件 → Codex 钩子的语义对照

与 dsh 自带的 Codex 钩子桥（`@deepseek-ai/dsh-hooks-codex`，把 Codex 钩子接进 dsh）方向相反。
Codex 的负载转成 dsh 的事件参数：`tool_name` → `exec.name`（`Bash` 映射为 dsh 的 `bash`），`tool_input` → `exec.arguments`，
`tool_use_id` → `exec.callId`，`tool_response` → 结果的文本内容块，`prompt` → 一条 `source.kind = 'user'` 的用户消息，
`session_id` → `agent.session.id`（同一会话同一个 agent 对象）。

| dsh 事件（模式） | Codex 钩子 | dsh 的决定 → Codex 钩子输出 | 做不到的部分 |
|---|---|---|---|
| `tools/pre-execute`（waterfall） | PreToolUse | `deny` → `permissionDecision: "deny"` + 原因；`cancel` → 同样 deny（原因写明是取消）；`allow` / `next()` → 不表态 | `ask`：Codex 钩子没有「改走审批」，不表态，交给 Codex 自己的审批与沙箱；dsh 本身也不允许改写参数，所以没有 `updatedInput` |
| `tools/post-execute`（waterfall） | PostToolUse | `block` → `decision: "block"` + `feedback` 文本作原因；`additionalContexts` → `additionalContext` | `accept` 带 `content` / `value` 替换结果：Codex 钩子不能替换工具输出，忽略；拿到的结果一律当作成功（Codex 负载里分不出失败） |
| `agent/pre-step`（waterfall） | UserPromptSubmit | `reject` → `decision: "block"`；`enter` 里比原输入多出的消息 → `additionalContext` | 改写或删掉用户原话、`startsRequestSeries`：Codex 钩子做不到，忽略；dsh 每一步都会触发，Codex 只在用户提交时触发 |
| `agent/turn-stopping`（serial） | Stop | 监听器里 `agent.steer(消息)` → `decision: "block"` + 消息文本，Codex 会再走一步 | `stop_hook_active` 为真（已经因为钩子多走过一轮）时不再拦，避免无条件 steer 的插件让 Codex 停不下来 |

各事件里监听器调用的 `agent.steer` / `agent.inject` 也会并入 `additionalContext`。监听器抛错只记日志，按「无意见」处理，不挡 Codex。

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
5. 有提示词或可映射事件时生成配套拓展包 `dsh-<插件 id>`，并同步 Codex 配置里的拓展钩子分发条目（新增事件时提示用户在 Codex 里信任一次）；
   之后插件的启停、单行启停、配置修改、重新分析都会重写或停用它，移除插件时它移入 dsh 插件的备份目录。
6. 首次导入工具插件时提示「接入 Codex」（写托管 MCP 条目）；之后新开 Codex 对话即可使用。

**插件市场索引能否直接复用**：可以复用格式 1 的 `source` 语义——`npm` 对应「npm 包」来源（`name@version`），
`git` 对应「git 仓库」来源（`url` + `commit` + `path`）。首期不做索引浏览界面，后续可在导入面板里加「从索引 URL 浏览」。

## 6. 验证结论

- 按 dsh 教程写法的工具插件（`inject: ['tools', 'systemPrompt']`、`defineTool`、Schemastery `Config`）与组合包
  （`cordis:group`、MCP 行、依赖会话服务的行、引用 dsh 内置模块的行）逐行判定正确，工具经 MCP 调用结果一致；
- npm 上真实发布的 `@deepseek-ai/dsh-tool-workspace-dependencies`：`npm pack` + `--ignore-scripts` 安装 `dependencies`，
  peer 由替身提供；真实 Schemastery 生效（缺必填配置时报错），填写配置后工具可在 Codex 中调用；
- 官方 `@deepseek-ai/dsh-tool-todo` 等依赖 `sessionProjections` / `agents` 的工具判为不兼容，符合预期。
- 提示词与事件映射（夹具 `tests/fixtures/dsh/policy-plugin`，按 dsh 写法用到 `systemPrompt.section` / `variable` 与四个事件）：
  经 `extension_hooks::dispatch` 与网关中间件链跑通 deny、block、拒绝输入、补上下文、要求继续、静态 / 动态提示词，
  插件按 agent 记的状态跨钩子进程保留、不同会话互不影响；停用、改配置、移除后同步生效；
- npm 上真实发布的插件：`@deepseek-ai/dsh-deliverable-file-references`（静态片段）与 `@deepseek-ai/dsh-persona`
  （填 `text` 配置后）的提示词追加到 instructions；`@deepseek-ai/dsh-repeat-tool-reminder` 判为完全兼容并映射到
  PostToolUse / UserPromptSubmit（它按 agent 计数，依赖常驻进程保留状态）。

## 7. 首期范围与后续

首期实现：

- 兼容宿主（MCP 服务 + inspect）、三种来源导入、注册表、启停（整包 / 单行）与移除、托管 MCP 接入；
- 按行的插件配置覆盖（JSON，浅合并到 bundle patch 的 `config` 之上，保存后重新分析）——
  实测 npm 上的 `@deepseek-ai/dsh-tool-workspace-dependencies` 有必填配置，没有这一步无法加载；
- skill 导入、MCP 行转换为可导入的 JSON；
- 系统提示词 → 网关 instructions、四个 dsh 事件 → Codex 钩子（配套拓展包 + 常驻进程，见 4、4.1）；
- 管理器「拓展」页的「dsh 插件」分栏：每行显示映射去处（「钩子 PreToolUse」「提示词 → instructions」），
  卡片上说明配套拓展包的现状（未生成 / 拓展总开关关闭 / 已停用）与生效条件。

后续（按价值排序）：

1. 插件市场索引浏览（格式 1）；
2. 按插件 `Config` schema 生成表单（目前是 JSON 文本框）；
3. 前端 slot 插件的移植指引 / 半自动改写为 Codex++ 拓展包；
4. 官方直连（不经 Codex++ 代理）时的提示词注入：目前只有 MCP `instructions` 兜底；
5. 更多 dsh 事件：`agent/created`（可对上 SessionStart，但 dsh 插件在这里多半调用 agent 循环的能力）、
   `tools/execute` 包裹式超时 / 重试（Codex 钩子没有包裹执行的能力）暂不映射。

## 8. 第二期：替身服务、会话镜像、命令与前端插件（2026-10-31）

目标是让导入的 dsh 插件常用的服务都有替身（还没覆盖的见第 9 节的覆盖表）：Codex 的 agent 循环与会话存储仍不在 Codex++ 手里，做不到的部分调用时说明原因。上面第 3 节里判为不兼容的几类，现在的去处：

| dsh 服务 / 事件 | 现在的落点 | 差别 |
|---|---|---|
| `sessions`、`agents`、`sessionProjections` | 会话镜像（`assets/dsh-host/services.mjs`）：钩子常驻进程把 Codex 钩子翻成 dsh 会话日志（turn/start、user/message、tool/call、tool/result、turn/end、compaction/*），插件 append 的领域事件一并进镜像，按会话落盘到 `data/<插件>/mirror/`；投影在镜像上折叠 | 没有模型的流式内容与推理；`create` / `fork` 抛错（改用 `ctx.sessions.codex.*` 经 Codex++ 读写真实 Codex 会话）；用到 agents / 投影 / 镜像事件的插件，配套拓展包会挂上 SessionStart、UserPromptSubmit、Pre/PostToolUse、Stop、Pre/PostCompact 全部钩子 |
| `agent.steer` / `inject` / `followup` | 钩子里翻成要求继续 / 补充上下文；钩子之外 steer、followup 经 Codex++ 往 Codex 会话插话 / 发起一轮，inject 排到下一次钩子带上 | `cancel` 不可用（拿不到正在跑的那一轮）；只声明 `sessions` 的插件不挂镜像钩子（多半只读 cwd） |
| `sessionQuery` | 经 Codex++ 的只读会话查询，事件映射成 dsh 的类型与 data 形状 | `observeSession` 是一次性快照 |
| `llm` | `stream()` 经 Codex++ 协议代理一次性返回再拆成流式块；替身 `dsh-llm` 补了 `BlockAssembler`、错误类与消息构造函数 | 不能注册 adapter |
| `commands` | 配套拓展包在 Codex 输入框里注册同名斜杠命令，经后台 → bridge → 常驻进程执行，结果用提示条显示 | |
| `connection.fetch` | 前端半边的 `fetch('/api/…')` 经后台 → bridge → 常驻进程交给注册的路由 | `connection.rpc` 不可用 |
| `fs`、`shell`、`subprocess`、`web`、`credentials`、`storageDomain`、`userQuestions` | 本机 Node 实现；`userQuestions` 在 Codex 页面弹框；`credentials` 先看插件配置里同名的键再看环境变量 | 不做沙箱与「先读后写」策略，没有 PTY 与联网搜索 |
| `agent/created`、`subagent/start`、`subagent/end` | SessionStart、SubagentStart、SubagentStop 钩子 | |
| `tools/result`、`session/event`、`agent/status` | 由会话镜像派发 | |
| `agent/request`、`system-prompt/assemble` | 网关请求中间件（改 model / reasoning.effort；改写后的片段追加到 instructions） | 只对经 Codex++ 代理的请求生效 |
| `agent/request-error` | 网关失败重试 | 同上 |
| 前端插件 `dsh.client` | 页面运行时 `96-dsh-client.js`：用 Codex 页面自己的 React 19 渲染组件，dsh 插槽映射到 Codex++ 的插槽（`shell.overlay` → overlay、会话「…」菜单 → 会话行菜单自绘项、`conversation.input.left/right` → composer.left/right、`conversation.chat.turnTail` → turn.after、`settings.section` → 整页……） | 对不上的插槽不显示并记一条失败；dsh 专属前端服务（store、remote、conversation、uiSession 的大部分 hooks）不可用 |

第三期补上（同日）：

| dsh 服务 / 事件 | 落点 | 差别 |
|---|---|---|
| `tools/post-execute` 改写结果（accept 带 content / value、block） | 钩子里记下决定（按调用 id 落盘），之后每次经 Codex++ 代理的请求在网关里把这条工具输出换掉；钩子没跑到的结果网关补跑一次 | 只对经代理的请求生效；Codex 界面里显示的仍是原结果 |
| `agent/pre-step` 按步触发 | 第一步由 UserPromptSubmit 钩子处理；工具调用之后的每一步由网关请求中间件（`gateway/step.mjs`）触发，可拒绝这一步、可追加消息；钩子之外 `agent.inject` 排下的上下文也在这里带上 | 只对经代理的请求生效 |
| `llm/stream`、`agent/assistant-stream` | 网关响应流中间件（`gateway/response.mjs`）：上游 Responses 事件翻成 dsh 流式块交给常驻进程里的插件，插件改写的文字增量换回去，收尾事件里的完整文本一并改；只观察时原样放行 | 只翻文字、推理、工具调用参数与用量；改写时 `*.done` 收尾事件等插件产出完再发 |
| `schedule` | 记录存在插件数据目录（MCP 进程与常驻进程共用），配套拓展包的后台每分钟 tick，到点经 Codex++ 在会话里发起一轮；支持 after / at / every / daily / weekly / cron 与时区 | Codex++ 没在运行时不触发，不补跑 |
| `webhookRuntime` | 配套拓展包开 `backend.http`，`/ext/dsh-<插件>/webhook/<kind>` 的请求交给规则，规则给出 prompt 就新建 Codex 会话 | 不校验签名 |
| `jobs` | 进程内登记处 | 不跨进程 |
| `workspaceRegistry` | 按 Codex 会话的工作目录归出来 | 只读 |
| 前端插槽 `tool.call.toolview`、`conversation.plan-review.actions`、`conversation.input.model`、`settings.section`、`sidebar.right.pane.tab` | 工具调用视图、计划条目视图、composer.model 插槽、Codex 设置页分区、悬浮球页签 | |
| 拓展包里直接带 dsh 插件 | `codexpp.json` 的 `dsh` 指向包内插件目录，随拓展启停 / 删除导入、停用、移除 | |

宿主调用通道：所有 Codex++ 起的 Node 进程都带 `CODEX_PLUS_HOST_EXE`，替身服务经 `CodexPlusPlus --codex-plus-host-call`
（stdin 一个 `{ service, method, params }`）读写会话、调模型、问用户（`assets/dsh-host/host-call.mjs`）。

仍然做不到：替换 Codex 的 agent 循环、会话存储格式；官方直连（不经 Codex++ 代理）时的按步 `agent/pre-step`、工具结果改写与回复流改写；
`subagents` / `workflowEngine` / `typert` / `sandboxPolicy` 这类 dsh 自己的运行时服务。

## 9. 第四期：插件间事件、工具运行时、前端服务（2026-11-01）

| dsh 能力 | 落点 | 差别 |
|---|---|---|
| 插件之间的 `ctx.emit` / `parallel` / `serial` / `bail` / `waterfall` / `off` | 同一个宿主进程（同一个包）里的监听器共用一张表，按 Cordis 五种语义派发，支持 thisArg 写法 | 不同包之间不互通（各自一个进程） |
| `tools/change`、`skills/change`、`commands/change`、`system-prompt/change` | 宿主在注册 / 撤销时发 | |
| `tools/execute`（waterfall） | 插件自己的工具：MCP 进程里完整包住执行；Codex 的工具：PreToolUse 钩子里跑，插件不调 `next()` 直接给结果即代为执行（原工具以「拒绝 + 结果」结束，经代理的下一次请求里换成纯结果） | Codex 的工具执行不在宿主里，调 `next()` 的包裹式监听器（超时、重试）只能放行原工具 |
| `tools.guard` | pre-execute 之后、执行之前（PreToolUse 与插件自己工具的 MCP 调用） | |
| `tools.restrict` | 插件自己的工具从 MCP `tools/list` 去掉；Codex 的工具在网关里从请求 `tools` 去掉 | 后者只对经代理的请求生效 |
| `tools.execute` / `schemas` / `executionMode` | 只覆盖本包注册的工具 | `presentAs` 忽略 |
| 钩子里的工具名 | 插件自己的 MCP 工具去掉 `mcp__codex_plus_dsh__` 前缀，与插件注册的名字一致 | |
| 前端插件配置 | 配套拓展包把包自己那一行的配置（含管理器覆盖）传给前端插件的 `apply` | |
| 前端 `ready` / `dispose`、可选依赖 | 全部 `apply` 跑完后发 `ready`；`inject` 对象写法的 `required: false` 缺了照样加载，`ctx.inject` 缺服务时不激活也不报错 | |
| 前端 `shortcuts` / `commandUi` / `theme` | 分别桥接到 Codex++ 的快捷键、斜杠命令（`popupSelect` 用对话框列选项）、主题 | 快捷键只取默认绑定，不支持双键序列 |
| 前端 `inputTriggers` | 服务存在但注册不进去：`/` 与 `@` 菜单归 Codex，记一条失败说明 | |
| 前端插槽 | 新增 `conversation.approval.detail`、`conversation.input.activity`、`conversation.hero.*`、`conversation.view`、`main.conversation`、`conversation.header*`、`conversation.session.header(.lineage)`、`sidebar.session.row.hover`、`shell.quota-notice`、`rightbar.session`（侧边面板）、`tool.view.cordis` | |

`HOST_PROVIDED_PACKAGES`（不必 npm install 的宿主包）与 `host.mjs` 的替身表保持一致，测试守住。

## 10. 第五期：会话与 agent 生命周期、审批、第二批替身服务、Config 表单（2026-11-01）

| dsh 能力 | 落点 | 差别 |
|---|---|---|
| `sessions.create` / `fork` | 经 Codex++ 新建 / 分叉真实的 Codex 会话，返回它的镜像 | 镜像只有之后的事件；dsh 的会话日志格式不在 Codex 里 |
| `agents.create` / `resume` | 给了 `sessionId` 就挂到那个 Codex 会话；没给就先新建 Codex 会话。返回 agent 替身 | 跑模型的仍是 Codex 自己的循环 |
| `agent.cancel` | 经 Codex++ 中断会话里进行中的那一轮（没给 turnId 时读会话找正在跑的一轮） | |
| `agent.inbox` 的 `replace` / `remove` / `splice`，`agent/inbox/inserted / claimed / discarded` | 还没交给 Codex 的排队消息可以改；排队、被下一次钩子取走、被清掉时发事件 | 已经进了 Codex 这一轮的改不了 |
| `agent/disposed`、`session/disposed`、`session/flush` | 常驻进程退出时按 dsh 的顺序发（先 flush 落盘） | |
| `approval` 服务、`approval/request` | Codex 的 PermissionRequest 钩子 → `approval/request` waterfall：`allowed-once` 放行、`rejected` / `cancelled` 拒绝、不表态交还 Codex；`ctx.approval.request` 链尾按策略（`never` 放行，`ask` 在 Codex 页面弹框） | 需要在 Codex 里信任一次 PermissionRequest 钩子 |
| `user-questions/request` | `userQuestions.ask` 先过 waterfall，插件能自己答；链尾才在 Codex 页面弹框 | |
| `system-prompt/assemble` | assembly 里多一段 `codex:instructions`（经代理请求里 Codex 自己的系统提示词）：没动它就只追加，删了或改了它就整段替换 instructions | 只对经代理的请求生效 |
| 镜像里的助手回复 | Stop 负载的 `last_assistant_message` 写成 `assistant/message` | 只有每轮最后一条，没有中间的流式内容与推理 |
| `storage`、`spillStore`、`shellEnv`、`sessionTitle`、`tokenMeter`、`sessionTelemetry` | storage 的 domain 形态即插件的 storageDomain；spill 写插件数据目录；sessionTitle.rename 改 Codex 会话名；tokenMeter 按字数粗估；遥测丢弃 | shellEnv 只影响宿主自己的 shell / subprocess，不影响 Codex 的 shell |
| `dsh-spill` / `dsh-user-approval` / `dsh-session-title` / `dsh-token-meter` / `dsh-shell-env` / `dsh-storage` / `dsh-session-telemetry` 包 | 替身导出品牌构造函数、错误类与抽象类 | |
| 插件 Config → 设置表单 | 分析时记下 `Config.toJSON()`（替身的 schemastery 也给出同形 JSON），管理器转成拓展设置表单（文本、密钥、数字范围、下拉、数组、嵌套对象）；表达不了的字段改用 JSON 编辑 | 配置缺必填项时也能先填表单 |

