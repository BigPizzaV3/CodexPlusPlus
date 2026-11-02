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

> 本表是**当前状态**（2026-11-01 第九期之后重写）。首期判为不兼容、后来各期补上的，都已按现状改写；每期的细节与差别见第 8~14 节。

| dsh 插件类别 | 判定依据 | 在 Codex++ 的落点 | 结论 |
|---|---|---|---|
| 工具插件（`inject` 只含 `tools` / `systemPrompt` / `logger` / `timer`） | 加载后 `ctx.tools.register` 出工具 | 兼容宿主进程内运行，经托管 MCP `codex_plus_dsh` 暴露为 Codex 工具 | **完全兼容**（工具调用、参数 schema、render、超时、取消） |
| 用到 `systemPrompt.section` / `context` / `variable` 的插件（含只做提示词的插件，如 `dsh-persona`） | 加载时截取注册 | 配套拓展包的网关请求中间件把片段追加到每次模型请求的 `instructions`（静态文本分析时求好直接嵌入；有函数片段或变量时每次请求求值）；同时汇总进 MCP `instructions` | **完全兼容**（经 Codex++ 本地代理 / 网关的请求）；官方直连的请求不经网关，只剩 MCP `instructions`；`complete` 片段只追加不替换 Codex 自己的提示词；`context` 并入 instructions 而不是逐回合刷新的用户侧快照；未注册的 `{{变量}}` 原样保留；`tools` 提供方忽略 |
| 监听 `tools/pre-execute` / `tools/post-execute` / `agent/pre-step` / `agent/turn-stopping` 的插件（策略、守卫类，如 `dsh-repeat-tool-reminder`） | `ctx.on` 这四个事件 | 配套拓展包声明对应的 Codex 钩子 PreToolUse / PostToolUse / UserPromptSubmit / Stop，钩子脚本经 `bridge.mjs` 派发给插件的监听器，把 dsh 的决定翻译成钩子输出（见 4.1） | **完全兼容**（拦截、补上下文、拒绝输入、要求继续）；dsh 决定里 Codex 钩子表达不了的部分忽略（见 4.1）；需要用户在 Codex 里信任一次「Codex++ 拓展钩子」 |
| 监听其它 dsh 事件 | `ctx.on` | 见第 8、9、14 节：大部分已有落点（会话镜像、网关、钩子、插件间事件总线，第九期起跨包互通） | 首期**不生效**；现状以第 8、9、14 节为准 |
| 工具插件用 `registerOptions.when`、`exec.agent`、`exec.deferContext`、`exec.concludeTurn`、`presentCall/presentResult` | 运行时调用 | 第九期：`when` 在经代理的每一步请求里按 agent 判断、MCP 调用时也判断；`deferContext` 并进工具结果（见第 14 节） | **部分兼容**：MCP 进程里 `exec.agent` 为空（Codex 的 MCP 调用不带会话）；`concludeTurn` 空操作；展示钩子忽略 |
| 工具 render 出的图片块 | `{ type: 'image', attachment }`（引用附件）或直接带 `data` | 引用的附件从这个插件的附件服务（数据目录 `attachments/`）读出字节，作为 MCP 图片内容交给 Codex | **兼容**；附件不在这个插件的数据目录里（例如别的插件存的）时退成文字说明 |
| skill 包（`SKILL.md` 文件，或经 `ctx.skills.register` / `registerProvider` 在代码里注册） | 包内扫描到 `SKILL.md`；或加载插件时截取 `ctx.skills` 的注册（宿主提供 `skills` 服务与 `@deepseek-ai/dsh-skill` 替身），依赖包里的 skill 也能拿到；bundle 行引用的依赖包里现成的 `SKILL.md` 也会扫描（最多 64 个，不跟软链，指向包外的依赖不扫） | 代码注册的写成 `SKILL.md` 目录（带上资源目录里的文件），与文件形式的一起复制进 Codex++ 的 Skills 源目录并启用到 Codex | **完全兼容**（内容层面）；skill 文档里写死的 dsh 专属路径/工具名需要用户自行判断 |
| MCP 配置行（`dsh-mcp-client`） | patch 行 `name` 为 `@deepseek-ai/dsh-mcp-client` | 生成标准 `mcpServers` JSON，交给现有「导入 MCP」流程写入 Codex | **完全兼容**（stdio / streamable-http）；`reconnect`、`toolCallTimeoutMs` 等 dsh 专属字段丢弃 |
| `cordis:group` 行 | patch 行 | 展开子行逐一判定 | 按子行 |
| 依赖会话、agent、LLM、命令、文件 / shell / 子进程、审批、计划模式、子 agent 等运行时服务的插件 | `inject` 的服务 | 宿主提供替身（`assets/dsh-host/services.mjs` 的 `SUPPORTED_SERVICES`）：会话与 agent 落在钩子驱动的会话镜像上，`llm` 经 Codex++ 协议代理，`fs` / `shell` / `subprocess` / `web` 是本机 Node 实现，其余见第 8~14 节 | **兼容（近似）**：跑模型的仍是 Codex 自己的循环，替身做不到的调用时说明原因（例如 `agent.cancel`、PTY、联网搜索） |
| 依赖 dsh 自己运行时的插件（`workflowEngine`、`ptcRuntime`、`sandboxPolicy` 等，或导入了宿主没有替身的 dsh 运行时包，如 `@deepseek-ai/dsh-agent-loop`） | `inject` 必需项不在替身列表，或解析不到的 `@deepseek-ai/dsh-*` 导入 | 无 | **不兼容**：建在 dsh 自己的 agent 循环与会话存储上，Codex 的这一层不在 Codex++ 手里；不加载并列出缺的服务 / 导出 |
| `Service` 子类插件 | 默认导出为 class | 第九期：按 Cordis 语义实例化，作为服务给同一个包里的插件 `inject`（支持 isolate，见第 14 节） | **兼容**（同包内）：服务对象不能跨进程，别的包拿不到 |
| 前端插件（`dsh.client`） | `package.json` 有 `dsh.client` | 页面运行时 `96-dsh-client.js`：用 Codex 页面自己的 React 渲染组件，dsh 插槽映射到 Codex++ 的插槽，对不上的作为子插槽；`connection` / `sessions` / `shortcuts` / `commandUi` / `theme` / `locale` / `remote` 等前端服务都有替身 | **兼容**：`inputTriggers`（`/` 与 `@` 菜单归 Codex）、dsh Web 客户端自己的 store / conversation / uiSession hooks、`setLocale` 不可用 |
| bundle patch 里的 `!!js` 表达式、overlay、`remove`/`update` 操作 | YAML 标签 / 操作名 | 第六期起按 dsh 的 patch 语义处理；第九期起 `!!js` 在 Node `vm` 里求值完整表达式（见第 14 节） | **兼容** |
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
| `agent/pre-step`（waterfall） | UserPromptSubmit | `reject` → `decision: "block"`；`enter` 里比原输入多出的消息 → `additionalContext` | 改写或删掉用户原话：Codex 钩子做不到，第九期起记下来、经 Codex++ 代理的请求里把那条用户消息换掉（直连时原话照旧）；`startsRequestSeries` 忽略；按步触发见第三期 |
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
| `sessionQuery` | 经 Codex++ 的只读会话查询，事件映射成 dsh 的类型与 data 形状 | `observeSession` 与 dsh 一样是某一时刻的切面；第九期起镜像里的会话给 live 切面（带投影） |
| `llm` | `stream()` 经 Codex++ 协议代理发请求（第九期起真流式）；替身 `dsh-llm` 补了 `BlockAssembler`、错误类与消息构造函数 | 第九期起 `registerAdapter` 可用，但只给宿主进程里的 `ctx.llm` 用 |
| `commands` | 配套拓展包在 Codex 输入框里注册同名斜杠命令，经后台 → bridge → 常驻进程执行，结果用提示条显示 | |
| `connection.fetch` | 前端半边的 `fetch('/api/…')` 经后台 → bridge → 常驻进程交给注册的路由 | （`connection.rpc` 见第 11、14 节） |
| `fs`、`shell`、`subprocess`、`web`、`credentials`、`storageDomain`、`userQuestions` | 本机 Node 实现；`userQuestions` 在 Codex 页面弹框；`credentials` 先看插件配置里同名的键再看环境变量 | 不做沙箱与「先读后写」策略，没有 PTY 与联网搜索 |
| `agent/created`、`subagent/start`、`subagent/end` | SessionStart、SubagentStart、SubagentStop 钩子 | |
| `tools/result`、`session/event`、`agent/status` | 由会话镜像派发 | |
| `agent/request`、`system-prompt/assemble` | 网关请求中间件（改 model / reasoning.effort；改写后的片段追加到 instructions） | 只对经 Codex++ 代理的请求生效 |
| `agent/request-error` | 网关失败重试 | 同上 |
| 前端插件 `dsh.client` | 页面运行时 `96-dsh-client.js`：用 Codex 页面自己的 React 19 渲染组件，dsh 插槽映射到 Codex++ 的插槽（`shell.overlay` → overlay、会话「…」菜单 → 会话行菜单自绘项、`conversation.input.left/right` → composer.left/right、`conversation.chat.turnTail` → turn.after、`settings.section` → 整页……） | 对不上的插槽不显示并记一条失败；dsh 专属前端服务（store、conversation、uiSession 的大部分 hooks）不可用；`remote` 第九期起可用 |

第三期补上（同日）：

| dsh 服务 / 事件 | 落点 | 差别 |
|---|---|---|
| `tools/post-execute` 改写结果（accept 带 content / value、block） | 钩子里记下决定（按调用 id 落盘），之后每次经 Codex++ 代理的请求在网关里把这条工具输出换掉；钩子没跑到的结果网关补跑一次 | 只对经代理的请求生效；Codex 界面里显示的仍是原结果 |
| `agent/pre-step` 按步触发 | 第一步由 UserPromptSubmit 钩子处理；工具调用之后的每一步由网关请求中间件（`gateway/step.mjs`）触发，可拒绝这一步、可追加消息；钩子之外 `agent.inject` 排下的上下文也在这里带上 | 只对经代理的请求生效 |
| `llm/stream`、`agent/assistant-stream` | 网关响应流中间件（`gateway/response.mjs`）：上游 Responses 事件翻成 dsh 流式块交给常驻进程里的插件，插件改写的文字增量换回去，收尾事件里的完整文本一并改；只观察时原样放行 | 只翻文字、推理、工具调用参数与用量；改写时 `*.done` 收尾事件等插件产出完再发 |
| `schedule` | 记录存在插件数据目录（MCP 进程与常驻进程共用），配套拓展包的后台每分钟 tick，到点经 Codex++ 在会话里发起一轮；支持 after / at / every / daily / weekly / cron 与时区 | Codex++ 没在运行时不触发，不补跑 |
| `webhookRuntime` | 配套拓展包开 `backend.http`，`/ext/dsh-<插件>/webhook/<kind>` 的请求交给规则，规则给出 prompt 就新建 Codex 会话 | 不校验签名 |
| `jobs` | 进程内登记处；第九期起跨进程（见第 14 节） | 任务变化事件只在启动它的进程里发 |
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
| 插件之间的 `ctx.emit` / `parallel` / `serial` / `bail` / `waterfall` / `off` | 同一个宿主进程（同一个包）里的监听器共用一张表，按 Cordis 五种语义派发，支持 thisArg 写法 | 第九期起不同包之间经常驻进程互通（见第 14 节） |
| `tools/change`、`skills/change`、`commands/change`、`system-prompt/change` | 宿主在注册 / 撤销时发 | |
| `tools/execute`（waterfall） | 插件自己的工具：MCP 进程里完整包住执行；Codex 的工具：PreToolUse 钩子里跑，插件不调 `next()` 直接给结果即代为执行（原工具以「拒绝 + 结果」结束，经代理的下一次请求里换成纯结果） | Codex 的工具执行不在宿主里，调 `next()` 的包裹式监听器（超时、重试）只能放行原工具 |
| `tools.guard` | pre-execute 之后、执行之前（PreToolUse 与插件自己工具的 MCP 调用） | |
| `tools.restrict` | 插件自己的工具从 MCP `tools/list` 去掉；Codex 的工具在网关里从请求 `tools` 去掉 | 后者只对经代理的请求生效 |
| `tools.execute` / `schemas` / `executionMode` | 只覆盖本包注册的工具 | `presentAs` 空操作（dsh 现在的 tools 服务已没有它，工具展示形态改由 tool-executor 决定） |
| 钩子里的工具名 | 插件自己的 MCP 工具去掉 `mcp__codex_plus_dsh__` 前缀，与插件注册的名字一致 | |
| 前端插件配置 | 配套拓展包把包自己那一行的配置（含管理器覆盖）传给前端插件的 `apply` | |
| 前端 `ready` / `dispose`、可选依赖 | 全部 `apply` 跑完后发 `ready`；`inject` 对象写法的 `required: false` 缺了照样加载，`ctx.inject` 缺服务时不激活也不报错 | |
| 前端 `shortcuts` / `commandUi` / `theme` | 分别桥接到 Codex++ 的快捷键、斜杠命令（`popupSelect` 用对话框列选项）、主题 | 快捷键只取默认绑定；第九期起支持双键组合与固定序列输入 |
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

## 11. 第六期：子插槽与 factory、前端服务实化、RPC、llm 工具调用、patch 语义、市场索引、近似运行时服务（2026-11-01）

| dsh 能力 | 落点 | 差别 |
|---|---|---|
| 子插槽（`renderSlot` / `renderSlotChain` / keyed / `registerFactory` / `renderFactorySlot` / `useFactorySlot`） | 所有 dsh 前端插件的插槽登记跨包共享；对不上 Codex++ 位置的插槽作为子插槽，由父组件渲染；chain 取第一个 `select` 命中的；keyed 按 `entryKey`；factory 的局部子插槽可被调用方替换 | `defineStore` 的 store 只把 `store` 原样交给组件；对不上位置、3 秒内也没人渲染的插槽才记「不会显示」 |
| 前端 `connection` | `rpc.call(channel, endpoint, payload)` 经配套拓展包的后台转给宿主半边 `ctx.connection.rpc.handle / intercept` 注册的处理器（拦截器优先），附件字节经 base64 往返 | 第九期起有流式 `open`；连接状态恒为已连接 |
| 前端 `sessions` / `uiWorkspace` / `locale` | `useSessionStatus` 跟随会话轮次的 running / idle；`uiWorkspace.selection` 跟随 Codex 当前会话；locale 的快照随语言切换递增 revision | `setLocale` 不生效（语言跟随 Codex） |
| `llm.stream` 工具与多模态 | 工具、图片、助手的工具调用与 tool 结果翻成 Responses 条目；回复里的推理与工具调用拆成 reasoning / tool-call 块，finish 为 `tool-calls` | 第九期起文字 / 推理真流式；工具调用在回复结束时给出 |
| bundle patch | 照 dsh 的 `applyEntryPatches`：`{ insert }` 追加、`{ id, insert }` 插进组、其余按 id 覆盖字段（写了 name 要对得上）；`cordis:include` 读包内条目文件并应用它自己的 patches；`group: true` 也算组 | 第九期起 `!!js` 求值完整表达式 |
| 插件市场索引（格式 1） | 管理器「浏览索引」列出最新版本，npm 来源翻成 `name@version`（第九期起带上 `#registry:<地址>`），git 来源翻成 `url#commit&path:子目录`，点「导入」走现有流程 | |
| 拓展多实例 | 原生拓展清单 `instances`：页面脚本按实例各应用一次 | 钩子、网关、后台不分实例 |
| `planMode` | 按 agent 记开关，开着时 PreToolUse 拒绝 Codex 的写类工具（apply_patch 等），切换时排一条说明 | 只认工具名，管不到 shell 里的写操作 |
| `goals` | 目标 active 时 Codex 每轮要停下就要求继续（钩子续过的轮次也算），完成 / 阻塞 / 暂停 / 清除后放行，连续 8 次自动暂停 | 没有 dsh 的目标工具，完成要由插件调用 `complete` |
| `subagents` | `start` / `prompt` 新建一个 Codex 会话跑子任务并等它结束，发 `subagent/start` / `subagent/end`；`interrupt`、`sendMessage`、`listChildren` 可用 | 不是 Codex 原生子 agent 树；provider 只有 codex |
| `fs/write-intent` / `fs/edit-intent` / `fs/observed` | 宿主的 fs 服务写 / 改前先问插件，读到 / 确认不存在时发观察 | 只管插件自己经 `ctx.fs` 的读写，管不到 Codex 的工具 |

仍未做（原因）：`workflowEngine`、`ptcRuntime`（dsh 自己的工作流 DSL 与执行环境，建在它的 agent 循环上）。（typert 类型化 RPC、Service 类与 Cordis 的 isolate 第九期已补上，见第 14 节；`mcpResources`、`webServer` 见第 12 节。）

## 12. 第七期：零碎事件、附件 / 文件引用 / 网页路由 / MCP 资源（2026-11-01）

| dsh 能力 | 落点 | 差别 |
|---|---|---|
| `agent/error` | 经 Codex++ 代理的请求失败时，网关失败重试带上请求所属会话（`prompt_cache_key`），先发 `agent/error` 再问 `agent/request-error`（后者现在也拿得到 agent） | 只对经代理的请求生效 |
| `workspace/session-stop` / `workspace/session-activity` | 前者在 Stop 钩子里发；后者经 `ctx.workspaceRegistry.activity()` 跑 waterfall | |
| `goal/changed` | `ctx.goals` 每次创建 / 编辑 / 暂停 / 恢复 / 完成 / 阻塞 / 清除时发 | |
| `domain/changed` | `ctx.storageDomain` 的 put / delete / update 后发 | |
| `credentials/reference-updated` / `record-updated` | `ctx.credentials.set / unset` 可写了（插件数据目录的 credentials.json，权限 600），写入后发；读取顺序：插件存的 → 插件配置 → 环境变量 | |
| `attachments` | 存在插件数据目录的 attachments/，图片类型与 20 MiB 上限校验，`readImageRequest` 给出 base64 | |
| `fileReferences` | 在 agent 的工作目录下按名字找文件（4 层、2000 项、跳过 node_modules / .git 等） | |
| `webServer` | `register` / `registerFallback` 的路由挂到配套拓展包后台 HTTP（`/ext/<配套拓展>/<路径>`），处理器拿到 Node 风格的 req / res | 第九期起有 WebSocket 升级（见第 14 节）；首页注入相关方法是空操作（Codex++ 没有 dsh 的 Web 客户端首页） |
| `mcpResources` | 插件登记的资源经 dsh 插件 MCP 的 `resources/list`、`resources/templates/list`、`resources/read` 暴露给 Codex，URI 前加 `dsh-<server>+` | |

另外：拓展网关中间件新增 `stage: "route"`（轮换选好候选供应商后重排 / 筛选），`gateway.replaces` 可关 `builtin:relay-rotation`（不再自动换下一个供应商）。内置拓展「语音听写」「轮次回退」「插件市场解锁」的工作改由拓展登记的扫描步骤做，「模型分组」停用后不再轮询。

## 13. 第八期：配置原地更新、网关模式的候选重排（2026-11-01）

- 拓展配置原地更新（对应 Cordis 的 `internal/update` 否决重启）：`ctx.onConfigUpdate((next, prev) => true)` 接住时，管理器保存配置后不再整体热重载，`ctx.config` 原地换成新值；有实例没接就照常热重载。dsh 插件改配置仍是重新分析、常驻进程按版本重起。
- 网关模式的候选（供应商 + Key）同样经过拓展网关中间件的 `route` 段；`gateway.replaces` 可关 `builtin:gateway-failover`（一个候选失败就回错误，不换下一个）。
- 内置拓展「会话 ID 角标」改由拓展登记的扫描步骤做；会话行操作按钮组与归档页行按钮是删除、导出和第三方 rowAction 共用的宿主，留在内核。


## 14. 第九期：补齐还能补的差距（2026-11-01）

| dsh 能力 | 落点 | 差别 |
|---|---|---|
| 自定义 npm registry | 市场索引的 `source: { type: 'npm', registry }` 翻成 `name@version#registry:<地址>`；手动导入同样可写 `#registry:`。`npm pack` 与依赖安装都带 `--registry`（同 dsh 安装组合包时整次用同一个 registry），注册表里记住它 | 只认 http(s) 地址 |
| bundle patch 的 `!!js` | 照 Cordis Loader 在 Node `vm` 里求值完整表达式：能用 `ctx`（本包已提供的服务与宿主替身，`ctx.dshHomePath(...)` 指到插件数据目录）、`env` / `process.env` 与 JS 内置对象；`disabled` 按真假判断；`!!str` / `!!int` / `!!float` / `!!bool` / `!!null` 按类型转换。插件代码本来就在宿主进程无沙箱运行，求值不额外扩大权限；每个表达式 1 秒超时，求不出的记诊断（disabled 按启用、配置保留原样） | 表达式在分析与加载时求一次，不随服务变化重新求值 |
| `Service` 子类插件 | 按 Cordis 语义 `new Class(ctx, config)`：构造里 `super(ctx, name)` 经替身 Service 登记服务，随后跑 `[Service.init]()`（可返回 / yield 撤销函数），没有就 `start()`；卸载时 `[Service.dispose]()` / `stop()`。`ctx.set` / `ctx.provide` 同样能提供服务，`ctx.plugin(Class)` 也行。依赖插件服务的行先跳过、别的行加载完再试（写在提供方前面也行）；常驻进程只映射了消费方时也会加载提供方 | 服务对象不能跨进程：只给同一个包（同一个宿主进程）里的插件用 |
| Cordis isolate | 行上的 `isolate: { 服务名: true \| '标签' }`（组行对整组生效）与 `ctx.isolate(name, label?)`：true / 不给标签是私有作用域，同名标签共享作用域；作用域里提供与 inject 的服务与外面互不可见 | |
| 不同包之间的插件事件 | 常驻进程之间的事件总线（`bridge.mjs` 的 `createEventBus`）：本包派发后，按注册表里各包报告的 `listens` 找到听这个事件的包，经它们常驻进程的本地 socket 转过去（没起就按需起）。emit 不等；parallel 并发等全部；serial / bail 本包没人给结果时按顺序问别的包；waterfall 本包监听器在前，链尾依次交给别的包的监听器，它们调 `next()` 时经 socket 回到发起方继续，最后才是调用方的 next。只监听事件的包也会生成配套拓展包（标记文件 + 后台），别的包才找得到它 | 参数与结果按 JSON 传（函数、循环引用丢掉）；每次往返 30 秒超时，出错按「无意见」继续；跨进程时 `bail` 返回 Promise（做不到同步）；MCP 进程里所有包本来就同在一个进程，直接派发、不再转发 |
| `jobs` 跨进程 | 任务快照写在所有插件共用的 `data/.jobs/`：别的进程（MCP 进程、各包常驻进程）list / get / read / wait 读快照，kill 写停止请求文件，由跑任务的进程每 300ms 看一次并接手；跑任务的进程退出后快照按失败处理 | 任务变化事件（`jobs/change`、`events.subscribe`）只在启动它的进程里发 |
| typert 类型化 RPC | 宿主半边：`ctx.typert`（`register` / `local` / `remotes` / `lookups` / `contexts` / `resources` / 按键取 schema）；包导出 `./typert` 时同 dsh 的 typert-loader 自动登记它的 `TYPERT` 清单。调用经 `/api` 通道：按描述把线上 `args` 对上方法参数（多余 / 缺少的参数拒绝），严格 codec 有 `create()` 就用它的 zod 解析器校验、没有就放行，lookup 参数经登记的提供方解析成宿主对象，`this.ctx.invocation` 可读（含 `uplink()`），错误码沿用 dsh Gateway 的 `gateway/*`。前端半边：`ctx.remote.<namespace>.<method>(...)` 与 `remote.<namespace>` 子服务（启动时向宿主要一次描述，`$mount` 可再补；没有描述的 namespace 按位置传参），stream 方法返回 dsh 的 RemoteStreamHandle（可迭代，`send` / `end` / `dispose`） | `$on` 只对上 Codex++ 拓展内核的同名事件，不转发宿主自己的 dsh 事件；没有 dsh 的访问控制（`remoteAccess`），调用方只有本机的 Codex 页面 |
| `connection.rpc.open` | 前端打开流 → 后台 `rpcOpen` 在插件常驻进程里开流（目前承载 typert 的 stream 方法），`rpcPull` 分批长轮询取值，`rpcPush` 送上行，`rpcClose` 取消 | 只经常驻进程（流要跨多次往返保持状态）；每次拉取最多等 1 秒 |
| `llm.stream` 真流式 | 宿主调用通道 `--codex-plus-host-call` 支持行式流：Codex++ 收到上游的文字 / 推理增量就写一行 `{ delta }`（SSE 按行拼接，字节在任意处断开都行；Responses 与 Chat Completions 都认），最后一行是整理好的结果；替身边收边吐 `block-start` / `*-delta` / `block-end` | 工具调用与用量在回复结束时给出 |
| `llm.registerAdapter` | 插件注册的供应商在 `ctx.llm.stream({ provider })` 对得上时直接接管，`listProviders` / `listModels` 里也有它 | 只给宿主进程里的 `ctx.llm` 用；Codex 自己的模型请求不经过它（要给 Codex 加供应商请用拓展清单的 `providers`） |
| `webServer.registerUpgrade`（WebSocket） | Codex++ 本地代理在 `/ext/<配套拓展>/<路径>` 上认出升级请求，问配套拓展包后台要一个本机 socket（常驻进程里只听这个 socket 的 Node http 服务），把原始升级请求原样写过去，之后两边字节对拷；插件处理器拿到真的 `IncomingMessage` 与 socket，协议协商与帧由它自己负责。还有升级后的长连接开着时常驻进程不算空闲 | Host 必须是本机回环地址（挡 DNS 重绑定），网页发来的升级只接受同源（页面本身由这条拓展路由提供）；普通 HTTP 路由仍不接受带 Origin 的请求 |
| 前端快捷键双键 | dsh 的 `secondCode`（两个键同时按住，不分先后）由 dsh 前端运行时自己的键盘监听匹配，`regions` / `modals` 同 dsh；`registerFixed` 进目录、`observeFixedInput` 把按键与 reset（组合输入、焦点切换、点击、失焦）交给插件，插件按 `stopSequenceMs`（默认 500）自己认 `g g` 这类序列；`catalog` / `fixedCatalog` / `describeBinding` 同 dsh 的键帽 | 单键绑定照旧交给 Codex++ 的快捷键 |
| `registerOptions.when` | 经 Codex++ 代理的每一步请求里按这一步的 agent 与模型判断，不满足就从请求的 `tools` 去掉（同 dsh 每步解析工具集）；MCP 调用时也判断一次 | MCP 进程里没有 agent（Codex 的 MCP 调用不带会话）；判断出错按可用处理；直连请求里模型仍看得到工具，调用时才被拒 |
| `exec.deferContext` | 插件自己的工具：推迟的上下文并进这次工具结果（模型紧跟着结果看到）；代为执行 Codex 的工具时并进钩子的补充上下文 | |
| `sessionQuery.observeSession` | 会话在本进程的镜像里时给 `source: 'live'` 的切面（带投影的当前值），否则从 Codex 会话记录读 `prepared` 切面；`cursor` 为最后一个事件的 seq | 与 dsh 一样是切面，之后的变化用 `session/event` / `sessionProjections.onChanged` 跟 |
| `agent/pre-step` 删改用户原话 | UserPromptSubmit 钩子里记下「原话 → 插件给的消息」（落盘），经 Codex++ 代理的请求里每次都把那条用户消息换掉 | 官方直连的请求不经代理，原话照旧 |

第九期之后，Codex++ 自己这一侧也补齐了：Rust 侧生效的内置功能可被拓展顶替（新增 `preload` / `launch` 两个时机），网关的请求转换、鉴权头、官方整理、请求日志都可由拓展接管，管理器页面可由拓展贡献与顶替，见 `2026-10-30-codexpp-extension-system-design.md`「补齐剩余差距」。

判定做不到的（原因）：

- **替换 Codex 的 agent 循环与会话存储、`workflowEngine` / `ptcRuntime`、`setLocale`**：内核不在 Codex++ 手里（任务明确不做）。
- **`exec.concludeTurn`**：要由 agent 循环在工具结果之后结束本轮；Codex 的循环不在宿主手里，MCP 工具结果与钩子输出都表达不了「到此为止」。
- **MCP 工具里的 `exec.agent`**：Codex 调 MCP 工具时不带会话 id，工具执行时认不出是哪个 agent。
- **`tools.presentAs` / `presentCall` / `presentResult`**：dsh 现在的 tools 服务已没有 `presentAs`（展示形态改由 tool-executor 决定）；Codex 自己渲染 MCP 工具调用，不读插件给的展示视图。
- **`tools/execute` 包裹 Codex 自己的工具（超时、重试）**：Codex 的工具在 Codex 进程里执行，钩子只能在执行前后介入。
- **`tools/pre-execute` 的 `ask`**：Codex 钩子的 permissionDecision 只有 allow / deny，没有「改走审批」。
- **Service 对象跨包共享**：服务是活的 JS 对象（方法、闭包、状态），各包在各自的进程里；跨包只能传 JSON 事件，见上表。
- **会话镜像里的模型流式内容与推理**：要让每个回复流都经过常驻进程（每个事件一次本地往返），只为镜像付出的延迟不值得；镜像只记每轮最后一条回复。
- **`subprocess` 的 PTY、`web` 的联网搜索**：宿主不装依赖（PTY 需要原生模块），Codex 的联网搜索不对插件开放。
- **前端 `inputTriggers`、`store` / `conversation` / `uiSession` 的大部分 hooks**：`/` 与 `@` 菜单归 Codex 自己；这些前端服务绑定 dsh Web 客户端自己的状态树。

### 14.1 补充：dshmk 市场「已验证」插件实测补缺口（2026-11-02）

拿 dshmk.com「已验证」页里的 9 个包在沙箱里（禁网、禁写、禁读凭据目录）做导入分析，按报告里的缺口补替身：

| dsh 能力 | 落点 | 差别 |
|---|---|---|
| `ctx.settings`（`get` / `describe` / `mutate` / `update` / `configure`）与 `@deepseek-ai/dsh-settings` 的 `SettingsConflictError` | 按插件建，值落在插件数据目录的 `settings/` 下（临时文件加 rename 的原子写），默认命名空间是插件自己那一行的 id，初始值取该行配置；`expectedRevision` 对不上抛 `SettingsConflictError`；`describe({ redactSecrets })` 把名字含 key / token / secret / password 的字符串值遮成 `********` | 与管理器里的拓展配置是两份，管理器改配置不会同步进来；没有 schema 校验与表单生成（`configure` 只记策略） |
| `ctx.loader`（`entries()` / `ctx.baseUrl` / `await()`） | 宿主记下加载过的行；`options.name` 是这一行引用的模块名（通常是包名），同一行 `ctx.fiber` 与 `entries()` 里的 `fiber` 是同一个对象；`!!js` 表达式里也能读 | 只有只读的 `entries()`，没有 dsh Loader 的重载、增删行 |
| `ctx.sessionPersistence` | `findLog` 恒返回空串 | 刻意不暴露 Codex 的会话日志：插件（如归档管理）会对 `findLog` 返回路径所在目录整个删除，返回真路径会连同别的会话一起删掉 |
| `@deepseek-ai/dsh-skill-filesystem` | `FileSystemSkillProvider` 只扫 `customSkillDirs` 里的 `SKILL.md`（最深 3 层） | 不监听目录变化，不扫默认根目录 |
| `@deepseek-ai/dsh-session/surface`、`dsh-session` 的 `snapshotJsonValue` / `isJsonValue`、`dsh-subagent` 的 `snapshotSubagentDescriptor` / `foldSubagentDescriptor`、`dsh-agent` 的 `installModelSelection` | 照 npm 上已发布包的源码（BSD-3-Clause / MIT）生成，文件头带来源、版本与许可全文 | 只提供这几个导出；`installModelSelection` 挂在宿主 agent 替身的 `on` 上，Codex 自己的请求不走它 |
| `ctx.typert.getPackage(包, 面)` | 记下 `register` 登记过的清单的「包 / 面」 | 只能回答「是否登记过」 |

9 个包的变化（第 4 轮对第 1 轮）：

| 包 | 之前 | 之后 |
|---|---|---|
| `dsh-plugin-hub` | 不兼容（缺 `loader`） | 兼容（21 行全过） |
| `dsh-reasoning-effort` | 不兼容（缺 `settings`） | 兼容 |
| `dsh-skills-manager` | 不兼容（缺 `dsh-skill-filesystem`） | 兼容 |
| `dsh-status-rotator` | 兼容 | 兼容 |
| `dsh-archive-manager` | 不兼容（缺 `sessionPersistence`） | 部分兼容（只剩近似说明） |
| `dsh-better-sidebar` | 不兼容（缺 `dsh-settings`、`dsh-session/surface`、`dsh-agent`、`dsh-subagent` 导出） | 部分兼容（侧边对话与子 agent 页仍依赖 dsh 的 agent 循环） |
| `dsh-cost-meter` | 报错（沙箱禁写 HOME 的误伤，之后是缺 `typert.getPackage`） | 部分兼容 |
| `dsh-ego-browser` | 部分兼容 | 部分兼容（TUI 才有的可选服务不生效，设计内） |
| `dsh-free-search` | 不兼容（缺 `dsh-settings`） | 报错：`blocks[toolsLang].tools` 取到 `undefined`。该包 0.9.26 的 `buildBuiltinPromptBlocks` 返回 `{ state, tools, constraints }`，按语言分在每段里面，读 `blocks[语言].tools` 本来就取不到，换到真 dsh 里也会同样报错，是插件自身的问题，不在宿主里绕 |

没有测：`dsh-ios`（要 iOS 模拟器）、`meow-memory`（npm 发布者与卡片上的仓库作者对不上，未核实来源）。这一轮只验证导入分析，不代表装进 Codex 后每个功能都正常。

## 15. 生态实测（2026-11-01）

**方法**：npm 上带 `dsh` 字段（`dsh.bundle` / `dsh.client`）的包共 730 个（`@deepseek-ai` 官方 59 个、第三方 671 个）。
官方全测，第三方随机抽 160 个（固定种子），共 219 个，走产品里真实的导入路径（`DshPlugins::import`：`npm pack` →
装依赖 → 宿主分析）。分析会执行插件代码，批量跑时把 node 换成 macOS `sandbox-exec` 包装（禁网、只允许写结果目录）。
复现：`dsh_plugins::dsh_ecosystem_survey`（`#[ignore]`，环境变量见测试注释）。

**修复前 → 修复后**（同一批样本）：

| | 修复前 | 修复后 |
|---|---|---|
| 导入成功 | 192 | 213 |
| 整包可用（每一行插件、前端都能用） | 89（41%） | 130（59%） |
| 部分可用 | 56 | 54 |
| 完全不可用 / 导入失败 | 71 | 31 |
| 前端插件实际加载 | 106 / 139 | 152 / 156 |

官方 59 个：整包可用 40、部分可用 10、不可用 6、导入失败 3；第三方 160 个：整包可用 90、部分可用 44、不可用 19、
没有可加载内容 4、导入失败 3。

**这轮修的**（按卡住的包数排序）：

- 装依赖：npm 即使 `--omit=dev` 也会先解析 dev 依赖树，dsh 插件常把宿主包写进 dev（含没发布的 `dsh-compact` 等），整个安装 404。
  导入副本去掉 `devDependencies`；直接依赖里 npm 上没有的、版本写成 `workspace:` / `link:` 的挪成 peer 不装，报告里逐个说明。
- dsh 自己的纯工具包（`packages/util` 与 typert 协议包，见 `DSH_UTILITY_PACKAGES`）：写成 peer、或根本没声明但代码里 import 的，
  导入时从 npm 装真包，不再判为「依赖 dsh 运行时」。
- 前端插件入口上限 240 KiB → 4 MiB（打包后的前端插件常在 0.3~3 MiB；配套拓展包按这个上限写页面脚本）。
- 替身补齐：dsh-llm 的 `LlmAdapter` 基类、品牌 id（`CallId` / `ReasoningEffortId` …）、`assertNever`、`deepFreeze`、`attributionHeaders`、
  `resolveRetryPolicy` 等纯函数；`llm.registerAdapter(providers, adapter)` 改成 dsh 的签名（带 `replace`）；cordis timer 的
  `interval` / `timeout` / `throttle` / `debounce`；`ctx.fiber.entry.options.id`；`Context.is`（宿主 ctx 认 false，调用方跳过 Cordis 内部操作）；
  `commands.registerFileReceiptResolver`（登记但不会被调用）。

**剩下的**：

- 依赖 dsh 运行时（多数）：`dsh-agent` / `dsh-agent-preset`、`loader`、`cmdlineArgs`、`sandbox` / `sandboxPolicy`、`settings`、`toolCatalog`、
  `agentPresets`、`sessionPersistence`、`httpServer` 等——建在 dsh 自己的 agent 循环、命令行、沙箱与设置上，Codex 的这一层不在 Codex++ 手里。
- 作者打包问题：依赖没声明或没发布、包里没有构建产物、传递依赖里写着 `link:` / `workspace:`、MCP 行缺 `command`。
- 只在实测沙箱里失败（9 行）：插件直接写 `~/.dsh/...` 或开调试端口，真实导入时不受影响。
- 3 个前端插件入口超过 4 MiB（4.8~22 MiB），不加载。

**第二轮（同日，同一批样本）**：按剩下的原因里「替身缺口」那一类再补，整批重跑对比，没有包变差；
完全没有问题行的包 137 → 143，问题行 104 → 98，导入失败 6 → 5。

- 子路径导出：`@deepseek-ai/dsh-llm/message`、`@deepseek-ai/dsh-token-meter/estimate`（替身表按完整说明符匹配，子路径要单独登记）。
- 新替身 `@deepseek-ai/dsh-client-connection`：插件只用它的协议常量（`API_PATH`、`RpcId`）与 RPC 信封校验器（`clientRequestSchema` 等，
  实现 `parse` / `safeParse`）；真包依赖 dsh-llm / dsh-tools 等运行时，不能装。
- 补导出：dsh-llm 的图片 / 文件占位文字（`offloadedImageText` 等，与 dsh 逐字一致）；dsh-credentials 改成 dsh 现在的写法
  （引用就是环境变量名字符串、键是 `<scope>/<id>`，加 `isCredentialRefName` 等）；dsh-tools 的 `JsonSchemaError` 与子集检查
  `assertSupportedJsonSchema` / `assertObjectJsonSchema`；`renderToolsSdk` 只在 PTC 模式用到，Codex 没有，调用时说明原因；
  dsh-session-title 的 `SessionTitleProviderId`、`normalizeSessionTitle` 等纯函数。
- 装上的 dsh 包（bundle 行引用的官方模块、纯逻辑库）把工具包写成 peer 时，补装后再装一轮；`dsh-session-title-llm` 当纯逻辑库装真包。
- 插件调用 `process.exit`（`dsh-plugin-boom` 在模块顶层就退出）：宿主自己的进程（分析、MCP、常驻进程）里换成抛错，
  这一行报出原因、同包别的行照常；之前整个分析没有报告。

这一轮之后剥出来的下一层都是 dsh 运行时（`sessionPersistence`、`dsh-workspace`、`dsh-sandbox-policy`、`dsh-subagent` 运行时包等）。
另有 3 个包在 `dependencies` 里写了宿主提供的包（真 dsh-tools 等），装上的是真包、又引用替身没有的旧导出，暂不处理。
