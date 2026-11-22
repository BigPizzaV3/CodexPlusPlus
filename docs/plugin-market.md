# CodeX 插件市场

管理工具的「CodeX 插件市场」可以检索、筛选并按需下载安装插件。Codex 侧复用原生插件页面，注入层拦截插件请求并接入相同后端。

## 下载行为

浏览市场只获取索引（`plugin-index-v2.json`），不克隆整个插件仓库，也不检出一个包含千个插件的市场目录。点击安装时，后端读取索引中的固定 Git revision，用 `blob:none` 和精确的单目录检出获取所选插件，核对 Git tree 后写入 Codex 插件缓存。实际网络量还包括 Git 的树元数据；需要本机可运行 Git。

只有一个缓存仓库：`BigPizzaV3/CodexPlusPlusFullPluginCache`，匿名访问，不需要 `gh auth login`；raw.githubusercontent.com 连不上时会改走 api.github.com 的 contents 接口取索引。2026-11-21 的索引有 14,496 个条目（约 14.8 MiB，上限 32 MiB），覆盖当天官方目录 14,364 个里的 14,339 个；其余 25 个官方不给下载地址（账号套餐不符或依赖的应用不可用，1 个已下架），列在仓库 `summary.json` 的 `unavailable` 里。

带真实凭据的文件不原样上传，列在仓库的 `upload-exclusions.json` 里：整个不传的（`files`，目前 2 个），或者把凭据换成说明文字后再传的（`redacted`，目前 1 个）；GitHub 的推送保护会拦下它识别出的凭据，清单里的测试占位值要逐个确认后才放行。

仓库里有两份索引：

- `plugin-index-v2.json`：现在的版本读它，本机缓存为 `index-full-v2.json`。
- `plugin-index.json`：2026-10-08 的 5,491 个条目，留给旧版本（它们只收 8 MiB 以内的索引），不再更新，也不要覆盖。

旧的公开库 `BigPizzaV3/CodexPlusPlusPluginCache`（520 个，是完整库的子集）已并入完整库并归档，不再用来浏览和安装。以前从它装上的插件（安装记录里 `source` 是 `public`）照常能用：完整库里同一个插件（id 相同，或公开库里拿名字当 id 的同名插件）直接显示为已安装，详情、卸载都作用在原来那一份上；更新时装上完整库的一份、移除旧的一份。

## 登录状态

目录是缓存仓库的快照（索引里的 `updatedAt`），不是官方实时目录；原生插件页的插件详情和管理工具的插件市场页都带快照日期。原生插件页按 Codex 有没有 ChatGPT 登录（`auth.json`）分两种：

- 有 ChatGPT 登录（官方登录、「官方登录 + API」、网关借出账号）：`plugin/list` 不并入快照，目录交给 Codex 自己读官方云端目录。已经从快照装上的插件照常显示，详情也能打开。
- 没有 ChatGPT 登录：并入快照。只带 `.app.json`、不带 `.mcp.json` 的插件，连接器由 ChatGPT 托管、连不上，说明里标「需 ChatGPT 登录」；它的 Skills 仍然可用。

管理工具的插件市场页不分状态，始终显示快照。

## 分类与页签

没有 ChatGPT 登录时，索引目录和官方插件一起在原生插件页的「公开」页签里，按分类分组：

- 分类来自包清单的 `interface.category`，由索引脚本写进索引的 `category`。清单没写分类的条目，在官方分组里的按官方分组排，其余归到 "Other"。
- 页签由市场名决定（Codex 26.1117）：市场的名字或显示名去掉首尾空白、转小写、非 `[a-z0-9]` 连成一个 `-` 后，是 `codex-official`、`openai-bundled`、`openai-curated`、`openai-curated-remote`、`openai-primary-runtime` 之一的进「公开」，其余都进「个人」。索引市场的显示名是「Codex official 目录快照」，规整后正好是 `codex-official`；名字里再加任何字母或数字（比如日期）就会掉回「个人」页签，那里一个市场一组、不分类。
- 分组、标题、组内顺序照官方目录：仓库根目录的 `directory-layout.json` 记着官方目录的分组（slug、中英文标题、是否人工编排的合集、成员顺序）和插件图标地址，索引脚本把它写进 `layoutDate` / `sections` 和每个条目的 `iconUrl`、`iconUrlDark`、`shortDescription`。注入层把各组名换成当前界面语言的官方组名（`codexPlusPluginNativeArrange`），页面就分到这些组里；人工编排的合集（热门、新品推荐、小型企业、Real Estate、销售）另外补一份条目，和官方一样同一个插件既在合集里、又在自己的分类里。组的先后和组内先后按官方顺序写成元素上的 `order`（`codexPlusPluginNativeApplyLayout`），页面重排后再套一遍。
- 列表有上万条：`plugin/list` 里每个条目不带完整描述（只在 `plugin/read` 的详情里给），简介没写的取描述开头 300 个字符。
- 「公开」页签每个分类只显示 6 个，官方插件排在前面。Codex 只给云端目录（要 ChatGPT 登录）的分类标题做了「看这一类全部」的链接；本机目录的标题是纯文字，但分类页 `/plugins?category=<分类>` 对本机目录同样有效。注入层在索引目录并入列表后把纯文字标题补成链接（`codexPlusPluginCategoryOpen`，样式在 `10-style.js`）。
- 官方市场里已有同名插件（如 Notion、Linear、GitHub）时不再列索引里的那一个，搜索结果也一样；已经从索引装上的保留。

## 原生接口适配

注入层支持直接 `sendRequest`、原型客户端、旧 `fetch` 消息、`mcp-request` 消息，以及页面到宿主进程的 MessagePort RPC。接管方法包括：

- `plugin/list` / `list-plugins`：没有 ChatGPT 登录时在原生列表中加入我们的索引市场，保留内置插件与其他市场。
- `plugin/read` / `read-plugin`：仅接管带有 CodeX 索引市场标记的插件详情；尚未安装时不虚构本地 Skills 路径。
- `plugin/install` / `install-plugin`：仅接管我们的索引条目，调用单包后端；不再次调用原生安装器。
- `plugin/installed`：从本地安装记录提供已安装条目，不为聊天初始化下载完整索引。

虚拟市场的路径使用 Codex home 内的绝对哨兵路径，实际安装由每个来源和插件 ID 的独立市场标识管理。同名插件的变体不会互相覆盖。远程主机请求不安装到本机。开关沿用「插件市场解锁」；关闭后停止接管。

Codex 26.1117 的插件请求只走 MessagePort RPC（capnweb 帧 `["push", ["pipeline", id, ["sendRequest"], [method, params]]]`），前几种传输都收不到，缺了这一层原生插件页就只剩官方的公开目录。这一层在 `91-extension-rpc.js`：

- 回包只带序号，序号由宿主按 push 顺序分配、帧里看不到。每个端口从接管起数 push，再用页面 `pull` 和宿主回包里出现的序号校准起点，据此把回包对回请求。
- 列表、已安装、搜索：请求原样发出，回包摘下来、补进索引条目后再投回同一端口；限时 15 秒，超时或出错把原回包原样投回。
- 索引条目的详情和安装：给这次 push 附一个随即被拒绝的参数，宿主不会执行它，并把拒绝理由原样带回来；凭这条理由认出回包，换成后端的结果。

安装请求只提交一次。桥接超时后查询原任务，失败或无法确认时显示错误，不回退到原生安装器，也不自动重试安装。安装后新开聊天或重启 Codex，使插件被重新加载。

## 连接器

安装不走原生 `plugin/install`：它会不经询问直接打开浏览器授权页。装完后按包里声明的连接器提示下一步（安装结果里的 `codexPlusMcpServers` 和 `codexPlusHostedOnly`）：

- `.mcp.json` 里的 MCP 服务：Codex 读配置时就会加载，不用等重启。服务处于未登录时提示里有「登录」，点了才调用原生的 `mcpServer/oauth/login`，授权页在系统浏览器打开，回调和凭据都由 Codex 的 app-server 处理，注入层不经手 token。以后重新登录在原生的「设置 → MCP」。
- 只有 `.app.json`、Codex 又没有 ChatGPT 登录：提示连接器要 ChatGPT 登录，按钮打开管理工具的「连接与供应商」。

能否登录由服务方决定：它要支持客户端自注册，或者包里带了预注册的客户端 ID。

## 代码与维护

- `crates/codex-plus-core/src/plugin_market.rs`：索引、分包下载、安装和原生协议适配。
- `assets/inject/renderer-inject/62-plugin-market-adapter.js`：原生页面请求拦截与响应合并。
- `apps/codex-plus-manager/src/PluginMarketScreen.tsx`：管理页面。
- `scripts/build-plugin-market-index.mjs`：从缓存仓库的已提交文件生成轻量索引。

索引生成示例：

```sh
node scripts/build-plugin-market-index.mjs /path/to/CodexPlusPlusFullPluginCache /tmp/plugin-index-v2.json
```

条目里的 `hasApps` / `hasMcp` 标出包内有没有 `.app.json` / `.mcp.json`；没有这两项的旧索引只能显示「需服务授权」，分不出是哪一种。

将输出作为缓存仓库根目录的 `plugin-index-v2.json` 提交（先提交插件内容，再生成并提交索引）。索引 revision 固定到插件内容所在的已有提交，发布索引不会改变包的校验值。索引生成只读取 Git 已提交内容，忽略本机未提交的文件。

## 验证

```sh
cargo test -p codex-plus-core plugin_market::tests --lib
node --test assets/inject/plugin-market-adapter.test.cjs
node --test apps/codex-plus-manager/src/plugin-market-model.test.ts
node scripts/assemble-renderer-inject.mjs --check
```

显式联网测试在临时 Codex home 中安装两个样本，不修改用户的 Codex 配置，也不调用原生授权流程：

```sh
cargo test -p codex-plus-core --test all plugin_market_live:: -- --ignored --nocapture
```

2026-10-08 验证：公开和完整索引读取成功，分别安装 Google Drive 和 Life Sciences Databases；原生 `skills/list` 加载 49 个 Skills、0 个错误。原生客户端、两类消息协议、超时恢复、远程主机隔离和插件身份去重复均有行为测试。当前运行的旧版本需要重新构建管理工具和启动器后重启，才能加载新增后端和注入代码。

2026-11-21 验证（Codex 26.1117 自带 CLI，没有 ChatGPT 登录，自定义 Responses 供应商）：外部写入配置装上的插件，其 MCP 服务立即出现在 `mcpServerStatus/list`；Notion 经 `mcpServer/oauth/login` 登录成功，凭据由 Codex 保存，重启后仍在；连接器工具以 `mcp__notion` 命名空间发给上游，工具调用结果能回传。同日在真实应用里（Codex 26.1117.31558，纯 API Key）复查：原生插件页出现索引目录，搜索能搜到索引条目；装 Linear（带 `.mcp.json`）后原生详情列出 MCP 服务，提示「连接器 linear 要登录后才能使用」；装只有托管连接器的 Notion 后提示要 ChatGPT 登录。换上带分类的索引后，「公开」页签按 14 个分类加 Other 分组，点 Travel 的标题打开分类页、列出 390 个插件，插件详情里有分类和快照日期。2026-11-22 换成单仓库和 `plugin-index-v2.json` 后复查（Codex 26.1117.41037，纯 API Key）：应用从 GitHub 取到 14,494 个条目（之后又补到 14,496 个）；「公开」页签 19 个分组，顺序、中文组名和官方目录一致，每组 6 个都有图标，从别的页面进插件页约 1.3 秒；点「旅行」打开分类页列出 854 个；联网测试在临时目录里装上了一个旧分组的插件和一个新分组的插件。提示里的「登录」之后的授权、以及有 ChatGPT 登录的两种状态（原生云端目录、托管连接器经中转模型调用）尚未实测。
