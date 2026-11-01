# 拓展包的发布与收录设计

日期：2026-10-30
状态：D1 已实施（社区栏收录与安装），D2~D4 为规划
相关：`2026-10-30-codexpp-extension-system-design.md`（拓展体系，本文是其中 P4「拓展市场支持拓展包」的展开）、
`crates/codex-plus-core/src/script_market.rs`（现有拓展市场）、`crates/codex-plus-core/src/user_scripts.rs`（拓展包加载）

---

## 一、目标

作者在管理器里点一下，就能把本地拓展包发布到自己的 GitHub 仓库，并自动被所有 Codex++ 用户的「拓展市场」看到；
用户能按版本安装、更新，并在安装前看清它要哪些权限。

不需要我们审核每一次提交，也不需要我们托管任何服务。

## 二、现状

- **拓展市场**只认单文件脚本：清单是 `BigPizzaV3/CodexPlusPlusScriptMarket` 仓库里的 `index.json`，
  每条给一个 `scriptUrl`，安装就是下载这一个 `.js`。条目靠向市场仓库提 PR 加入，等于全量人工审核。
- **拓展包**（目录 + `codexpp.json`）只能本地手写，或在 Codex 对话里经 `codex_plus_extensions` MCP 写出来，
  市场装不了，也没有更新机制。
- 可复用的部分：
  - `skills.rs` 已能用 trees API / codeload 读取 GitHub 仓库，支持 `GITHUB_TOKEN` / `GH_TOKEN`，并给出限流提示。
  - `plugin_market.rs` 已依赖 `gh`（`gh_binary()` 会找 `~/.local/bin`、Homebrew 路径）。
  - `UserScriptManager::write_extension_package` 已有相对路径、软链逃逸、同 id 覆盖等写入约束。

## 三、参照做法

另一款同类桌面工具的插件市场分三路来源，我们借鉴第二路：

1. 官方组织仓库里的一份 `registry.json`，提 PR 进入，界面标「社区编写」。
2. **打 topic 自动收录**：作者给自己的仓库加约定的 GitHub topic，客户端用搜索 API 按
   `topic:<约定名> archived:false fork:false` 按星数排序拉取，内存 + 磁盘缓存 10 分钟；逐个读仓库默认分支的清单文件，
   确认入口文件确实提交在仓库里。这一路**单独成栏，明确标「未审核」**。
3. npm 搜索兜底。

它没有一键发布：作者自己 `git push` 并在仓库设置里加 topic。它从仓库安装时取默认分支最新提交，没有版本锁定。
统计只有两项：GitHub 星数，以及 npm 的上周下载量（`api.npmjs.org/downloads/point/last-week/<包名>`），
卡片和详情里展示；没有评分、等级或热度排序，列表只能按名称排序。

我们在此基础上补三点：**按版本标签安装并锁定提交**、**管理器里一键发布**、**不自建服务也能有下载量、热度、等级和评分**（第十一节）。

## 四、总体方案

```
作者（管理器）                         GitHub                          用户（管理器 · 拓展市场）
本地拓展包 ──发布──▶ gh：建仓库 / 推送 / 打 tag / 建 release / 加 topic
                                     │
                                     ├─ topic:codexpp-extension ─┬─ 市场仓库工作流每小时汇总 → community.json（含统计）──▶ 社区栏（未审核）
                                     │                           └─ 客户端实时搜索，补汇总里还没有的新仓库 ─────────────▶
                                     └─ index.json（精选，PR 收录）─────────────────────────────────────────────────▶ 精选栏
                                                                          │ 选中 → 读 codexpp.json → 展示权限
                                                                          ▼
                                    按 tag 对应的提交下载（优先 release 包文件，计下载量）→ 校验 → 写入 user_scripts/<id>/
```

## 五、清单约定

拓展包仓库**根目录**必须有 `codexpp.json`（暂不支持一个仓库放多个拓展包）。在现有字段之外，新增几个可选的展示字段，
加载器忽略它们，不影响现有包：

| 字段 | 说明 |
| --- | --- |
| `repository` | `owner/repo`。发布时自动写入；收录时与实际仓库不一致则以实际仓库为准并在详情里提示 |
| `author` | 作者显示名 |
| `homepage` | 主页，缺省为仓库地址 |
| `icon` | https 图片地址或仓库内相对路径，展示前按现有图标规则校验 |
| `license` | SPDX 标识，GitHub 读不出 `LICENSE` 时作为后备 |
| `keywords` | 搜索与筛选用 |

版本号沿用现有 `version`，要求是 semver。发布打的 tag 为 `v<version>`。

## 六、收录

### 6.1 来源

| 栏目 | 来源 | 标识 |
| --- | --- | --- |
| 精选 | 现有 `index.json` 新增 `kind: "package"` 条目：`{ id, repo, tag, sha, … }`，由我们审核后锁定到具体提交 | 精选 |
| 社区 | GitHub 搜索 `topic:codexpp-extension archived:false fork:false`，按星数降序 | **未审核** |

同一个仓库同时在两栏时只在精选栏出现。两个仓库声明了同一个 `id` 时都列出，卡片上带上 `owner/repo` 以区分；
本机已装的同 id 拓展来源不同时，安装需要用户确认替换。

### 6.2 拉取与限额

**主路：读汇总文件。**市场仓库里的定时工作流（第十一节）每小时把社区栏整理成 `community.json`，
客户端只读这一个 raw 文件，不占 GitHub API 次数，统计数据也一并带上。

**补充：实时搜索。**汇总文件最长滞后一小时；为了让作者发布后几分钟内就能看到，客户端在汇总之外再做一次实时搜索，
只补汇总里没有的仓库，补进来的条目没有统计数据。汇总文件读不到时，实时搜索就是唯一来源。

GitHub 未登录时搜索 API 每分钟 10 次、普通 API 每小时 60 次，所以实时搜索按「列表少读、详情再读」拆开：

- **列表**：一次搜索请求（每页 100 条，最多翻到 1000 条）拿到名称、描述、星数、所有者头像、许可证、默认分支、最近推送时间。
  再对每个仓库从 `raw.githubusercontent.com` 读默认分支的 `codexpp.json`（不占 API 次数），读不到或解析失败的不列出。
  并发上限 8，单个仓库 15 秒超时；没读完的下次再补，已读到的先显示。
- **详情 / 安装**：才去读 `releases/latest`（没有 release 再读 `tags`）、README 和锁定提交的 `codexpp.json`。
- **缓存**：内存与磁盘（`<配置目录>/extension_discovery.json`）各存 10 分钟。搜索失败时用上次磁盘缓存并标注「列表来自 N 分钟前」。
- **鉴权**：有 `GITHUB_TOKEN` / `GH_TOKEN` 就带上；否则可选读 `gh auth token`。token 只发往 `api.github.com`，不落盘。
- **网络慢**：国内直连 api.github.com 可能单页几十秒，搜索单页超时 30 秒，超时即用缓存，不阻塞整个市场页。
- **开关**：设置项可关闭社区栏；关闭后不发任何搜索请求。

### 6.3 过滤

列表阶段就隐藏：

- `codexpp.json` 缺 `id` / `version`，或 `id` 不合规则；
- `engines.codexpp` 高于本机 `EXTENSION_API_VERSION` 的不隐藏，排到最后并显示「需要更新 Codex++」，不能安装；
- 在精选清单的 `blocked` 列表里（见第九节）。

## 七、安装与更新

### 7.1 安装

1. 解析目标版本：有 release 取最新 release 的 tag，否则取 semver 最大的 `v*` tag，都没有才退回默认分支（界面提示「作者未发布版本，装的是最新提交」）。
2. 把 tag 解析为提交 SHA，作者之后强推或移动 tag 不会影响已装版本。下载来源：
   - release 带有发布流程上传的包文件 `<id>-v<version>.zip` 时，下载这个文件，并用 GitHub 给出的 `digest`（sha256）校验。
     这样下载会计入该文件的 `download_count`，是下载量的来源（第十一节）；
   - 否则按 SHA 下载 `codeload.github.com/<owner>/<repo>/zip/<sha>`，这种安装不计下载量。
3. 在临时目录解包并校验：
   - 拒绝软链、硬链、绝对路径和 `..`；
   - 解包后总量不超过 10 MiB、文件不超过 2000 个；
   - `codexpp.json` 能被现有加载器解析，且 `id` 与列表里看到的一致。
4. **展示权限并等用户确认**（见第八节）。
5. 原子替换到 `user_scripts/<id>/`；原目录如存在，先移到备份目录 `extension_backups/`。来源记进单独的
   `extension_community.json`（`installs.<id> = { repo, version, tag, sha, fromDefaultBranch, fromAsset, installedAt }`），
   不改 `user_scripts.json` 的结构：它只认得自己的字段，多写的键会在下次保存时丢掉。
6. 默认**不启用**，用户在确认框里勾选「安装后启用」才启用；涉及 `hooks` 时同步钩子分发条目，并提示到 Codex 里信任一次。

### 7.2 更新

- 进入拓展页时（受同一缓存约束）比对已装 `sha` 与最新 release 的 tag，有新版本就在卡片上提示。
- 更新流程同安装，配置值保留（按 `user:<id>` 存在 `user_scripts.json`，与包目录分离）。
- **权限变化要重新确认**：新版本比旧版本多出钩子事件、网关中间件、后台进程、模型工具或路由时，逐项列出新增部分。
- 不做自动更新。

## 八、权限展示

拓展包与 Codex 页面同权，部分能力还会以用户身份跑 Node 进程。安装确认框按 `codexpp.json` 列出，按风险从高到低：

| 能力 | 展示 |
| --- | --- |
| `backend.script` | 常驻本机进程，可读写本机文件、访问网络 |
| `hooks` | 每个事件与匹配条件；可拦截或改写 Codex 的工具调用 |
| `gateway.request` / `gateway.response` | 可读取并改写发往模型的完整请求与模型回复 |
| `tools` | 模型可调用的工具名与描述 |
| `routes` | 每条可调用的 Codex++ 本地路由 |
| `renderer` | 在 Codex 页面里运行脚本 |

社区栏的条目在确认框顶部固定显示：「未经 Codex++ 审核，作者可以在新版本里加入任意代码。只安装你信任的作者的拓展。」

## 九、下架

- 精选 `index.json` 新增 `blocked: ["owner/repo", …]`：列表里隐藏；本机已装的显示警告并建议停用，**不自动删除**。
- 仓库被删、改为私有或去掉 topic 后，不再出现在列表里；已装的照常运行，只是查不到更新。

## 十、一键发布

### 10.1 入口与前提

- 管理器「本地拓展」里，拓展包卡片上的「发布到 GitHub」按钮；已发布过的显示「发布新版本」。
- 不提供发布入口：单文件脚本（先转成拓展包）、内置拓展、dsh 导入生成的配套包（目录里有 `dsh-bridge.json` 标记）。
- 鉴权完全交给 `gh`：先跑 `gh auth status`。没装或没登录时，给出安装与 `gh auth login` 的指引，以及手动发布的步骤（第十二节）。
  Codex++ 不读、不存 token，也不修改用户的全局 git 配置。

### 10.2 发布前检查

发布框里展示检查结果，有错误时不能发布：

- **清单**：`id`、`name`、`description`、semver `version` 齐全；`engines.codexpp` 不高于本机版本。
- **版本**：`v<version>` 在远端已存在则拒绝，提供「补丁号 +1」「次版本号 +1」按钮直接改 `codexpp.json`。
- **密钥扫描**（任一命中即拦截，列出文件与行号，不展示命中内容）：
  - 文件名：`.env*`、`auth.json`、`config.toml`、`*.pem`、`*.key`、`id_rsa*`、`user_scripts.json`；
  - 内容：`sk-` 开头的长串、`ghp_` / `github_pat_` / `gho_`、`AKIA` 开头的访问密钥、`-----BEGIN … PRIVATE KEY-----`；
  - `config` 里 `secret: true` 的字段，`default` 必须为空。
- **体积**：同安装限制（10 MiB / 2000 个文件），并拒绝软链。
- **README / LICENSE**：没有 README 时按清单生成一份（名称、描述、安装方式、权限表）；没有 LICENSE 时让用户选 MIT、Apache-2.0 或不加。

### 10.3 发布步骤

仓库名默认 `codexpp-<id>`，可改；可见性固定为公开（私有仓库无法被收录）。

1. 在 `<状态目录>/extension_publish/<id>/` 准备工作副本：
   - 远端仓库已存在：`gh repo clone` 或 `git fetch` 到该目录；
   - 不存在：`gh repo create <owner>/<name> --public --description <description>`，再在该目录 `git init` 并关联远端。
2. 用拓展包当前文件**整体替换**工作副本里的已跟踪文件（保留 `.git`），写回 `codexpp.json` 的 `repository` 字段。
3. 提交 `release v<version>`，打 tag `v<version>`。
4. 推送提交与 tag。推送时只在这一条命令上临时指定凭据助手：
   `git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push origin HEAD --follow-tags`。
5. 把要发布的文件打成 `<id>-v<version>.zip`（顶层即包根目录），`gh release create v<version> <包文件> --generate-notes`。
   用户从这个文件安装，下载才会被计数。
6. `gh repo edit <owner>/<name> --add-topic codexpp-extension`（已有则无副作用）。
7. 把 `repo` 记进 `user_scripts.json` 里这个拓展的发布信息，供下次「发布新版本」使用。

每一步失败都停下，并显示 `gh` / `git` 的原始输出；已完成的步骤不回滚（仓库已建、提交已推都可以安全重试）。
提交作者用用户本机的 git 身份；没配置时提示先设置 `user.name` / `user.email`，不替用户写。

发布成功后提示：社区栏最长 10 分钟后可见（缓存周期），以及可以提 PR 申请进入精选。

## 十一、下载量、热度、等级与评分

前提是不自建服务器。数据只来自 GitHub，由市场仓库里的定时工作流汇总，客户端只读汇总文件。

### 11.1 数据从哪来

| 指标 | 来源 | 说明 |
| --- | --- | --- |
| 星数 | 搜索结果的 `stargazers_count` | 一个 GitHub 账号只能点一次，最难刷 |
| 总下载量 | 各 release 里包文件的 `download_count` 之和 | 只统计从包文件安装的；只走 codeload 安装的不计 |
| 周下载量 | 今天的总下载量减 7 天前的快照 | 快照由工作流按天存在市场仓库 |
| 最近更新 | 最新 release 的发布时间，没有 release 时用 `pushed_at` | — |
| 评分 | 每个拓展在市场仓库 Discussions 里有一个帖子，取帖子的 👍 / 👎 反应数 | 见 11.4 |

### 11.2 汇总工作流

市场仓库 `CodexPlusPlusScriptMarket` 里加一个 GitHub Actions 工作流，每小时运行一次：

1. 用仓库自带的 `GITHUB_TOKEN` 搜索 `topic:codexpp-extension`，逐个读 `codexpp.json`、releases 和 Discussions 反应
   （带 token 的配额是每小时 1000~5000 次，覆盖几百个仓库绰绰有余）；
2. 套用第 6.3 节的过滤，算出热度和等级；
3. 写 `community.json`（列表加统计），并每天追加一份下载量快照到 `stats/YYYY-MM-DD.json`，只保留 90 天；
4. 内容有变化才提交，避免刷出无意义的提交记录。

`community.json` 每条大致是：

```json
{
  "repo": "alice/codexpp-word-count", "id": "word-count", "name": "字数统计", "version": "1.2.0",
  "tag": "v1.2.0", "sha": "…", "description": "…", "owner": "alice", "ownerAvatar": "…", "license": "MIT",
  "stars": 42, "downloads": 1830, "weeklyDownloads": 120, "updatedAt": "2026-10-28T09:00:00Z",
  "rating": { "up": 37, "down": 2 }, "heat": 63.4, "level": 3, "badges": ["release", "license", "readme", "fresh"]
}
```

### 11.3 热度与等级

**热度**用于默认排序，偏向「最近有人用」，并对每一项取对数，避免某一项被刷高后压过其它：

```
热度 = 3 × ln(1 + 周下载量) + 2 × ln(1 + 星数) + 1 × ln(1 + 👍 − 👎，下限为 0) + 新鲜度
新鲜度 = 30 天内有更新为 2，90 天内为 1，否则为 0
```

权重先按上式实现，具体数值等有了真实数据再调。

**等级**只看可客观检查的条件，不掺主观判断。每一级包含上一级的全部条件：

| 等级 | 条件 |
| --- | --- |
| 1 | 能被收录（第 6.3 节） |
| 2 | 有 release 和包文件、有 LICENSE、有 README |
| 3 | 2 级，且 180 天内有更新、`engines.codexpp` 为当前版本、评分好评率 ≥ 80%（不少于 5 票） |
| 精选 | 进入精选清单，由我们审核 |

卡片展示等级与对应徽章；详情里逐条列出达成或缺少的条件，作者照着补就能升级。

### 11.4 评分

真正的 1~5 星评分需要记录「谁评了几分」，没有服务器做不了，所以用 GitHub Discussions 代替：

- 工作流为每个收录的拓展在市场仓库建一个 Discussion（分类「拓展评价」），标题为 `owner/repo`；
- 管理器详情页显示 👍 / 👎 计数和好评率；「去评价」按钮在浏览器里打开这个帖子，用户用自己的 GitHub 账号点反应、写评论；
- 已登录 `gh` 的用户可以直接在管理器里点 👍 / 👎 和点星（调用 GitHub API，记在用户自己的账号下）；没登录就只能跳转到网页。

一个账号对一个帖子只能点一次同种反应，防刷能力与星数相同。

### 11.5 防刷与局限

- 下载量能被反复下载刷高，所以热度里只取对数，等级条件也不看下载量。
- 只走 codeload 安装的拓展没有下载量。卡片上不显示 0，而是不显示这一项，避免看起来像没人用。
- 汇总最长滞后一小时；实时搜索补进来的新条目在下一次汇总之前没有统计数据，排在有数据的条目之后。
- 工作流挂了时，客户端继续用上次的 `community.json`，并显示它的生成时间。

### 11.6 管理器展示

- 卡片：星数、周下载量、好评率、等级徽章、「未审核」或「精选」标识。
- 排序：热度（默认）、周下载量、星数、评分、最近更新、名称。
- 详情：以上全部指标、等级条件清单、「去评价」与点星按钮。

## 十二、手动发布（不用管理器）

写进 `EXTENSIONS.md`：

1. 仓库根目录放拓展包（`codexpp.json` 及其文件），推到公开仓库；
2. 仓库设置 → Topics 加 `codexpp-extension`；
3. 发布版本：打 `v<version>` tag 并建 release，把包根目录打成 `<id>-v<version>.zip` 上传到 release
   （不建 release 也能被收录，但用户装的是默认分支最新提交；不上传包文件就不计下载量）。

## 十三、分阶段

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| D1 ✅ | 社区栏收录与安装：搜索、缓存、过滤，按 tag→SHA 下载、校验、权限确认、来源记录 | 给一个测试仓库加 topic，10 分钟内出现在社区栏，能装上并锁定到 tag 对应的提交 |
| D2 ✅（部分） | 已做：精选清单支持 `kind: "package"`（按 sha 锁定安装）与 `blocked`；官方档（清单 `official` 且仓库属于 `OFFICIAL_OWNERS`）；社区栏去掉下架与已进精选的仓库、同 id / 自称官方标「可疑」、官方 / 精选版本不被社区同 id 替换。未做：社区拓展的更新检查与权限变化确认 | 发新 tag 后卡片提示更新，新增钩子时更新需要再次确认 |
| D3 ✅ | 一键发布：检查、生成 README、建仓库、推送、tag、带包文件的 release、topic（`extension_publish.rs`，测试用假 gh + 本地裸仓库跑通全流程） | 从本地拓展点一下，新仓库带 topic 与 release，换一台机器能在社区栏装上 |
| D4 | 统计：市场仓库汇总工作流、`community.json`、下载量快照、热度与等级；管理器排序、徽章、评分与点星 | 装一次后下次汇总下载量 +1；在讨论帖点 👍 后好评率更新；按热度排序与公式一致 |
| 后续 | 一个仓库多个拓展包；「申请精选」一键提 PR | — |

## 十四、D1 实施记录

- 代码：`crates/codex-plus-core/src/extension_discovery.rs`（搜索、缓存、版本解析、下载、解包、安装），
  管理器 `src-tauri/src/extension_community_commands.rs`（`extension_community_request`：list / prepare / install / discard / setEnabled）、
  `src/CommunityExtensionsPanel.tsx`（「拓展 → 社区拓展」分栏）。
- 与上文的出入：
  - 包文件用 `.zip`，不用 `.tar.gz`：解包复用现有的 `zip` 依赖，不新增 tar / gzip 依赖。
  - 汇总文件 `community.json` 属于 D4，D1 只有实时搜索这一路。
  - 列表里的图标只认 GitHub 上的地址（仓库相对路径或 `*.githubusercontent.com`），作者填的其它外链不显示，
    免得每个打开市场的人都去访问作者指定的主机。
  - 鉴权只读 `GITHUB_TOKEN` / `GH_TOKEN`，没有接 `gh auth token`。
  - 校验暂存包时直接用拓展加载器扫描暂存目录（临时的 `UserScriptManager`），和正式加载走同一套清单解析。
- 测试：`tests/all/extension_discovery.rs`，wiremock 模拟 GitHub API / raw / codeload / release 附件，
  覆盖缓存与失败退回、release 附件摘要校验、tag 与默认分支回退、本地同 id 冲突与备份、软链 / id 不符 / 缺清单的拒绝；
  另有一条默认跳过的联网用例核对真实接口。

## 十五、测试

- **纯函数**：搜索结果解析、过滤、版本选择（release / tag / 默认分支）、密钥扫描规则、权限差异计算，按现有 `#[test]` 风格放 `tests/all/`。
- **网络**：GitHub API、raw 与 codeload 的基地址可经环境变量改到本地 HTTP 夹具，测试不访问外网。
- **安装**：用夹具 zip 覆盖软链、`..` 路径、超限、`id` 不一致、`engines` 过高等拒绝路径。
- **统计**：热度、等级、周下载量（快照差）的计算放进工作流脚本的单元测试，用固定的 API 响应夹具；
  客户端只测 `community.json` 的解析、排序和缺数据时的展示。
- **发布**：`gh` 路径可经环境变量替换为测试脚本，记录收到的参数；git 操作对一个本地裸仓库执行，校验提交内容、tag 与推送结果。

## 十六、不做

- 不托管任何服务器或镜像：列表和统计只来自 GitHub、精选清单和市场仓库工作流生成的汇总文件。
- 不做 1~5 星评分：需要记录评分人，没有服务器做不到，用讨论帖的 👍 / 👎 代替。
- 不支持私有仓库收录与安装。
- 不做代码沙箱；风险靠权限展示、版本锁定、未审核标识与下架列表控制。
- 不自动更新，客户端不在后台定时搜索（定时汇总在市场仓库的工作流里做）。
