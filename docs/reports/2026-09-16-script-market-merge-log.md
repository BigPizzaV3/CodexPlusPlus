# 三个仓库 PR 合并执行记录（2026-09-16）

承接两份审查报告：
- [2026-09-16-script-market-open-pr-review.md](2026-09-16-script-market-open-pr-review.md)（脚本市场）
- [2026-09-16-three-feature-pr-review.md](2026-09-16-three-feature-pr-review.md)（主仓库三功能）

本轮处理范围：主仓库 `#2224`、脚本市场剩余 4 个、主题市场 25 个。
按用户指示：**能合的合掉，不能合的回复或关闭**。

---

## 一、主仓库 `BigPizzaV3/CodexPlusPlus`

### ✅ #2224 会话视图对齐引擎改事件驱动收敛（已合并 `82fb0924`）

修 issue #2221（开着会话视图白烧约 1 个 CPU 核）。合并前验证：

- 删掉 `setInterval(() => scheduleConversationViewAlign(2), 350)` —— 它每 350ms 把
  `settleFramesLeft` 托底为 ≥2，把本该 16 帧就停的 rAF 循环无限续命，是烧 CPU 的直接原因。
- 对齐改两阶段批量：先对所有目标统一写 style，再统一读几何，消除写-读-写交替。
- 新增心跳同步 `syncBackendSettingsFromHeartbeat`（5s 一次），带 `syncBackendSettingsInFlight`
  防重入；`loadBackendSettingsState` 本身有 `seq` 守卫，不会覆盖用户正在改的设置。
- **实测 `cargo test -p codex-plus-core --test cdp_bridge` → 157 passed / 0 failed**，
  含两个新增测试。`injection_script_keeps_session_action_buttons_in_pr_style`（记忆里标注的
  既有失败）现在也通过。
- 无悬挂引用：`conversationViewHasRoomForHtmlCenter` 删除后无残留调用者；`pollId` 保留仅用于
  cleanup 时 `clearInterval`。
- 注入脚本经 `assets.rs` 的 `include_str!` 直接内嵌，无构建产物漂移风险。

作者在 PR 正文里**纠正了 issue 对问题 2 的根因判断**（issue 认为 `/settings/get` 会返回 `{}`
导致校验失败；作者指出 `BackendSettings` 所有字段无 `skip_serializing_if`，响应永远全量），
这个纠正是对的。

### 未处理 #2206 独立 Taskboard 应用

`acc-c`，+59952/-0，128 文件。此前审查结论为「385 通过 / 1 失败（真 bug），先修」。
本轮未动，**仍开放**，是三个仓库里唯一剩下的开放 PR。

---

## 二、脚本市场 `BigPizzaV3/CodexPlusPlusScriptMarket`

本轮**合并 0 个**，**关闭 4 个**（含关闭说明），在上一轮已合并 #84/#85/#90/#91 的基础上，
该仓库现在 **0 个开放 PR**。

| PR | 处理 | 关闭理由 | 关闭说明 |
|---|---|---|---|
| #86 | 关闭 | 17220 行 / 841 KB 打包产物不可审计；patch dispatcher `dispatchMessage`、经 bridge 读写 AGENTS/Memories，影响面在每次对话分发路径 | [#issuecomment-5701363144](https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket/pull/86#issuecomment-5701363144) |
| #87 | 关闭 | 拦截发送动作 + 派发合成鼠标/键盘事件自动改模型选择，超出只读监控的权限模型 | [#issuecomment-5701366116](https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket/pull/87#issuecomment-5701366116) |
| #88 | 关闭 | `isLikelyApiUrl` 把 loopback 与 `/responses` 一并纳入解析，范围过宽；描述写 1.13.0 而产物 1.16.1 | [#issuecomment-5701369266](https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket/pull/88#issuecomment-5701369266) |
| #89 | 关闭 | 覆写 Statsig `checkGate`/`getFeatureGate`，在客户端强制打开厂商灰度关闭的入口 | [#issuecomment-5701374118](https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket/pull/89#issuecomment-5701374118) |

关闭前均确认**作者未回复、无新提交**（#86 head 仍 `c835698e`、#87 `3606548c`、#88 `b23da79f`、
#89 `816700a6`）。四个关闭说明都给了替代路径：自行在作者仓库分发；#87/#89 另建议开 issue
到主仓库从产品层面处理；#86/#88 明确「补材料后可重新提交」。

---

## 三、主题市场 `BigPizzaV3/CodexPlusPlus-Themes`

### ✅ 25 个主题投稿全部合并（`00f3b97`），25 个 PR 全部关闭并附说明

一次性并入 **24 个 Dream Skin 主题（axdlee）+ Iron Reactor Particle（sks-curry）**。

**为什么必须一次性合**：25 个 PR 各自都要往 `index.json` 的 `themes` 同一数组插一项，
且都基于 7 月的旧状态，两两全部冲突（`merge-tree` 实测 PR#1 vs #2/#3/#4/#25 均 CONFLICT）。
逐个合不可行。

**合并方式**：在 `origin/main` 上建临时分支，按 #25→#1→...→#24 顺序逐个 `merge --squash`，
每个 PR 的 `index.json` 冲突用脚本确定性重建（取 main 现有条目 + 追加该 PR 新增的 1 项），
最后压成**一个干净 commit** 直接推 main（该仓库 push 权限已验证：`admin: true`）。
推前确认为 `origin/main` 的 fast-forward。

**合并前逐项验证（全部通过）**：

- 39 个主题（14 → 39），原有 14 项**逐字节保持不变**，无丢失无篡改。
- 每个主题的 `theme.json` / `image` / `preview` 均按 index 声明的
  `theme_sha256` / `image_sha256` / `preview_sha256` 重算比对通过。
- 39 / 39 都有 `LICENSE.md`；39 / 39 都有 `source_url`。
- `node scripts/validate.mjs` → `Validated 39 theme(s)`。
- `theme.json` 键集合与既有主题完全一致；颜色值全部合法（无格式错误）。
- **人工看图**：抽查 blush-garden / changan-mechanica / iron-reactor-particle 的 preview，
  均为 AI 生成的原创视觉，**无第三方 Logo、商标或受保护角色素材**。

**发现并已反馈的一个来源问题**：`axdlee/Codex-Dream-Skin` 经 API 确认是
`Fei-Away/Codex-Dream-Skin` 的**真实 GitHub fork**（fork 创建于 2026-07-16），
但 axdlee 的主题 `LICENSE.md` 把「原始来源」写成自己的仓库、并把图片作者记为自己。
25 个关闭说明里都加了这条建议：后续更新时写明 fork 关系
（例如「原始来源：Fei-Away/Codex-Dream-Skin；本主题在其基础上创作」）。
另外 axdlee 的 fork 相对上游 ahead 97 / behind 0，所以他对新增主题的署名是成立的，
这条只是**署名边界**建议，不影响本次合并。

---

## 四、最终状态

| 仓库 | 开放 PR |
|---|---|
| `BigPizzaV3/CodexPlusPlus` | **1**（#2206 Taskboard，待修那 1 个失败测试） |
| `BigPizzaV3/CodexPlusPlusScriptMarket` | **0** |
| `BigPizzaV3/CodexPlusPlus-Themes` | **0** |

本地 `CodexPlusPlus` 工作区已恢复原状（`main` @ `82fb0924`，7 个未跟踪文件原样保留，
期间为切分支临时 stash 的 WIP 已 pop 回来）。

## 五、遗留项

1. **#2206** 仍是唯一开放 PR，需先修那 1 个失败测试再合。
2. 脚本市场 `updated_at` 仍是 `2026-08-31T13:52:10Z`，非合并当天。未擅自修改——
   不确定主程序是否把它当「是否需要重新拉取清单」的判据。
3. 主题市场 `updated_at` 仍是 `2026-07-18`，同理未动。
4. 脚本市场 `install_market_script_content` 不校验 SHA（CHANGELOG 1.1.8 起有意移除阻断），
   `sha256` 目前是纯完整性展示字段，建议单独开 issue 决定其语义。
5. 主题市场的 `axdlee/Codex-Dream-Skin` fork 署名边界问题，待作者后续投稿时再看是否修正。
6. `codex-zhcn-translate` 脚本托管在他人 fork（`hL091015`），本仓库无法审查其变更——供应链盲点。
