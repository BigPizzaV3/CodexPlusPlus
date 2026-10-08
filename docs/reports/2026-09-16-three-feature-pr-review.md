# 三个新功能 PR 审查（2026-09-16）

基线：`main` @ `7d75ffeb`
方法：逐个 merge 验证 + 实跑测试 + 读核心实现

**结论：三个都能干净合并，但 #2206 有一个会打到生产路径的真 bug。**

| PR | 规模 | 侵入性 | 实测结果 | 建议 |
|---|---|---|---|---|
| **#2128** 会话任务树 userscript | +4050 / 34 文件 | **零侵入** | 70 测试全过 | ✅ 可合 |
| **#2208** 纯 API 模式浏览器插件兼容 | +2649 / 22 文件 | 中等 | Rust 42 target 全过，前端 234 全过 | ✅ 可合（有一处需注意） |
| **#2206** 独立 Taskboard 应用 | **+59952 / 128 文件** | 低（自包含） | **385 通过 / 1 失败（真 bug）** | ⚠️ 先修 |

---

## #2128 — 会话任务树 userscript

**解决什么**：长对话里「试过什么、放弃了什么、最后选了哪个」难以回溯。在对话头部加一个入口，打开任务树画布，支持缩放/折叠/节点详情/跳回原消息。历史整理可走 OpenAI 兼容 API 或桌面后台任务。

**实测**：70 个测试全过。产物 `public/canvas.user.js` 105 KB / 1210 行。

**侵入性**：**零**。全部 34 个文件都在 `tools/conversation-canvas/` 下，不改 `renderer-inject.js`、不改任何 Rust、不改 relay 配置。产物里只有 1 处 `fetch(`，示例地址是 `https://api.example.com/v1`（占位符）。

**作者自己声明的限制**（诚实，值得肯定）：
- 依赖私有桌面 API，**不承诺跨版本兼容**
- Desktop 26.908.9136.0 上的原生后台整理**尚不支持**，必须选外部 API
- 「Live desktop end-to-end verification is still pending」——新 Desktop 版本上的端到端行为未验证
- 本地没有 Cargo，所以 Rust 测试和 clippy 没跑过

**要说的一点**：分类上它是 `tools/` 下的独立工具，不是产品功能。合并的实际影响是把代码放进仓库、并让署名归到本项目（AGPL-3.0-only）。如果不想承接这个维护面，可以要求作者放自己的仓库、在本项目 README 里链过去——这是你的偏好问题，不是技术问题。

---

## #2208 — 纯 API 模式浏览器插件兼容

**解决什么**：你之前回复过的 #2173。纯 API Key 模式下 Chrome/Edge 插件能显示但用不了，浏览器服务读取云端请求标识决策时触发 `unsupported Codex auth method: apikey`。

**我重点审的是「有没有伪造身份」**——因为这类修复很容易滑向伪造登录。结论是**没有**：

注入模块 [require-identification.mjs](assets/native-browser/require-identification.mjs) 的逻辑是：
- 只在本地下达「要求标识」的决策，**不伪造 ChatGPT 登录、不提供其他服务的认证能力**
- 请求头带的是**真实** `x-browser-agent: ChatGPT/<session-id>`，session ID 取自当前 turn
- 严格校验：必须是 `type === "extension"`、family 与 extension ID 精确匹配白名单（edge `odlomjlbamekndcpllcnffbgeohgkmjh` / chrome `hehggadaopoacecdllhhajmbjkdcmajg`）、`agentRequestHeaderEnabled` 必须是 boolean
- **I/O 后重新校验** `this.clientInfo`、session_id、turn_id 未变，防止「决策被应用到已被替换的客户端或已结束的 turn」
- 控制文件必须 `lstat().isFile()` 且非软链且 ≤1KB

适配层对运行时做固定指纹校验（browser service / native worker / node / manifest / CUA 入口各一个 SHA-256），拒绝软链、未知版本、外部修改。文档 [native-browser-identification.md](docs/native-browser-identification.md) 明确写了「不安装另一个浏览器引擎、不冒充 ChatGPT 登录、不提供其他已认证服务的访问」，并且**如实标注了局限**：不声明 macOS 支持、不保证未来 Codex 版本兼容、关闭选项**不会**清除扩展里已保留的标识状态。

**实测**：Rust 42 个 target 全过，前端 234 全过（比当前 main 多 44 个测试）。设置默认关闭。

**一处需要你拍板**：它的核心机制是「**不使用服务端 rollout 决策**」来满足标识要求。文档说这是 local identification requirement，不是 identity/rollout/approval 结果——技术上说得通（标识要求本就是本机策略），但这确实绕过了上游的灰度判断。如果哪天上游把「要求标识」当作安全策略而非功能开关，这个改动就会变成策略绕过。**这是产品判断，不是代码质量问题。**

**一个技术上的好处**：它顺带重写了 `is_filesystem_root`，用 `path.has_root() && path.parent().is_none()`——比我在 `dcaefeb6` 里写的 `parent().is_none()` 更精确（能区分 `C:` 和 `C:\`），而且补了 `\\?\C:\`、`\\server\share\` 等 6 种 Windows 前缀的测试。合并后我那版会被替换掉，是好事。

---

## #2206 — 独立 Taskboard 应用

**这是什么**：`apps/codex-taskboard/` 下一个完全独立的应用——服务端（SQLite + Cloudflare D1 迁移）、Web UI（React + @xyflow/react）、CLI（`taskctl`）、云适配（wrangler）、Codex 注入脚本、386 个测试。自带 CI workflow。

**实测：有真 bug。**

```
not ok 281 - task thread migration excludes comment-only aggregate entries
error: 'no such column: tasks.id'   code: 'ERR_SQLITE_ERROR'
    server/database.mjs:468
```

作者报告「386 passed」，我实测 **385 passed / 1 failed**，且单独跑稳定复现。

**根因**（我做了最小复现）：`server/database.mjs:468` 的迁移语句在 `ORDER BY` 里嵌套 `EXISTS` 子查询，并引用外层表的列：

```sql
UPDATE tasks SET thread_id = COALESCE(thread_id, (
  SELECT task_threads.thread_id FROM task_threads
  WHERE task_threads.task_id = tasks.id          -- ← 这里 OK
  ORDER BY
    CASE WHEN EXISTS (
      SELECT 1 FROM comments
      WHERE comments.task_id = tasks.id          -- ← 这层引用外层列就报错
        AND comments.thread_id = task_threads.thread_id
    ) THEN 1 ELSE 0 END,
    task_threads.created_at DESC,
    task_threads.thread_id DESC
  LIMIT 1
)) WHERE thread_id IS NULL
```

最小复现验证：把这个 `EXISTS` 子查询去掉 → 通过；保留 → 报 `no such column: tasks.id`。**这是 Node 内置 `node:sqlite`（实验特性）对相关子查询中 `ORDER BY` 表达式的解析限制**，SQLite 本体没这个问题。

**这不只是测试问题**——`hasTaskThreads` 为真时（即任何带 `task_threads` 表的旧数据库）就会执行，属于生产迁移路径。用户在旧版本数据上升级会遇到。

**为什么作者没发现**：他在 Windows 上开发，Node 小版本不同。CI 用 `node-version: "22"` 会拿到最新 22.x。而且**这个 PR 的 CI 从未跑过**（`gh pr checks` 返回 "no checks reported"——workflow 是新加的，只在 `apps/codex-taskboard/**` 变动时触发，而它自己就是首次引入）。

**除这个 bug 外**：结构是自包含的（只加了自己的 CI workflow，不碰仓库其它文件）；依赖克制（6 个运行时依赖 + 7 个开发依赖）；作者如实报告了 `npm ci` 的 6 项 audit 发现（1 moderate + 5 high，全在 `undici` 这条传递依赖上，来自 wrangler/miniflare，不是这个应用自己引入的）。

**建议**：让作者修掉那个迁移语句（把 `EXISTS` 提到 `ORDER BY` 外面，或用 `LEFT JOIN` 重写），在能看到这个 Node 版本的环境里跑一遍 CI。修完可以合。

---

## 汇总建议

| PR | 动作 |
|---|---|
| **#2128** | 可以合。但先决定：它是「独立工具」还是「本项目功能」？前者建议让作者放自己仓库、README 链过去 |
| **#2208** | 可以合。技术上干净，但「绕过上游 rollout 决策」这一点请你确认是可接受的定位 |
| **#2206** | **先让作者修 `database.mjs:468`**，并让新加的 CI 真跑一次。修完可合 |

三个都能干净合并，不需要 rebase（都在当前 main 之前的分叉上，无冲突）。

要我现在合哪个？还是先给 #2206 的作者留评论说明那个 bug？
