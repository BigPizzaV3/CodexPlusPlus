# 脚本市场 PR 审查：#96 Codex Usage & Resets（2026-10-07）

仓库：`BigPizzaV3/CodexPlusPlusScriptMarket`
基线：`main` @ `7c19dae`（"fix(market): 移除 codex-zhcn-translate 条目"）
PR head：`7ba9f8e`（`codex/add-usage-reset`，作者 `small32`）
规模：3 文件，+463 / -1（`index.json` +26/-1、`scripts/codex-usage-reset.js` +416、`tests/codex-usage-reset.test.js` +21）

**当前只有这一个开放的 PR**（`gh pr list` 只返回 #96）。

**结论：代码质量是这批里最干净的一个，可以合。唯一的判断点在于它是本市场第一个会真正改动服务端账号状态的脚本——消费掉一次用量重置是不可逆的。**

---

## 核对结果（全部通过）

| 项 | 结果 |
|---|---|
| SHA-256 | ✅ `5eb9d36490aa3f912959af590c882d562389f78e6fb3d44d66fbb3cb35ee9168`，与 index.json 声明逐字节一致 |
| 脚本体积 | 28083 字节 / 416 行 |
| 自带测试 | ✅ 3/3 通过（`node --test tests/codex-usage-reset.test.js`） |
| 合并性 | ✅ `merge-tree --write-tree origin/main pr96` 干净，exit 0 |
| 清单 schema | ✅ 11 键，与 `codex-relay-balance` / `deepseek-token-usage` 同一形状（含 `requirements`/`limitations`） |
| 外连审计 | ✅ **零外部 host**（见下） |
| DOM sink | ✅ 无 `innerHTML`/`outerHTML`/`eval`/`new Function`/`document.write`，全部走 `textContent` |

### 外连审计细节

脚本里出现的所有网络相关字面量只有两处**相对路径**：`/wham/usage`、`/wham/rate-limit-reset-credits`。没有硬编码 host、没有第三方端点。请求走 Codex 自己的两条通道，按可用性择一：

1. `findServices()` 从 `performance.getEntriesByType("resource")` 里找 `/assets/rpc-*.js`，动态 `import()` 后取 `mod.appServices.httpFetch`（带 `/* @vite-ignore */`）；
2. 回退到 `window.electronBridge.sendMessageFromView({type:"fetch",...})`，`hostId` 取自当前 URL 的 `hostId` 参数，默认 `"local"`。

### 拆卸与生命周期

`destroy()` 清得比较全：`clearInterval`、`cancelAnimationFrame`、`observer.disconnect()`、四个事件监听（`pointerdown`/`keydown`/`resize`/`focus`/`visibilitychange`）、`pending` 里所有在途请求的 cancel、注入的 `root`/`panel`/`style` 三个节点，最后条件式 `delete window[KEY]`（只在 `window[KEY].destroy === destroy` 时删，避免误删后加载的实例）。开头 `window[KEY]?.destroy?.()` 保证重载先拆旧实例。

**比同类脚本更规范的一点**：它调用了 `window.__codexPlusUserScripts.registerCleanup(destroy)`。这个接口在宿主里是真实存在的（`assets/inject/user-scripts-runtime.js:3`、`assets/inject/renderer-inject/91-extension-api.js:258`，契约测试见 `crates/codex-plus-core/tests/user_scripts_reload.rs`）。我逐个 grep 了市场里其余 16 个脚本，**没有一个调用 `registerCleanup`**——这条是 #96 的作者先做对的。

---

## 需要你判断的唯一一件事：它是本市场第一个写服务端状态的脚本

`consume()` 会 `POST /wham/rate-limit-reset-credits/consume`，**真的消费掉账号的一次用量重置**。这是不可逆的。

我核了其余脚本的写操作：`prompt-optimize` 的 POST 打向的是用户自己配的 LLM endpoint，`deepseek-token-usage`/`codex-live-token-cost` 的 POST 字面量在包装层里（不改请求语义）。**没有任何一个此前的市场脚本会改动 Codex 官方账号的额度状态。** 这是新引入的能力面，属于产品判断，不是代码质量问题。

**作者对这个不可逆操作的处理相当克制**，我认为值得肯定：

- **二次点击确认**：第一次点把按钮文字改成"确认使用1次"，第二次才真正发。
- **发送前钉死通道**：`transportAttempt` 一旦建立就复用，注释写明 "Never re-submit a POST via another transport"——避免第一条通道失败后从另一条重发导致双扣。
- **幂等重试**：重试复用同一个 `redeem_request_id`，是同一个逻辑尝试；`already_redeemed` **只在 `retry` 为真时**才被当作成功（`resetResult`），这条有专门的测试覆盖，非重试时它是报错的。
- **失败不静默**：传输层出错时提示"结果未确认，点击'重试同一次重置'核实"，并只给这一条重试路径，不自动重发。
- `index.json` 的 `limitations` 已如实写明"使用重置会消耗账号可用重置次数，需要二次点击确认"。

**如果你决定放它进市场**，这个脚本在 UI 上有必要让用户明确知道"这一下会扣掉一次重置"——目前文案已经说了，没问题。**如果你认为市场不该分发会改账号状态的脚本**，这是唯一要否的理由。

---

## 小瑕疵（不阻塞）

1. **PR 正文与产物不符**：正文写 "all 18 Node tests passed"，提交进仓库的测试文件只有 **3 个 `test()` 块**（虽然每块内多条断言）。上一轮 #88 也是同类问题（正文写 1.13.0、产物是 1.16.1）。合并前让作者更正正文，或在合并说明里注明即可。**另外**：市场仓库的 `tests/` 目录是已存在的约定（还有 `codex-daily-token-usage.test.js`、`prompt-optimize-prosemirror-writer.test.js`），投稿带测试是好事。

2. **`homepage` 指向市场仓库自己**：`https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket`。这个字段在 manager 里是可点击外链（`App.tsx:7542` 的 `openExternalUrl`），指向市场自身等于没有主页。**但这不是 #96 的独创**——`codex-relay-balance`、`codex-bundled-plugin-doctor`、`prompt-optimize` 三个已有条目也是这么写的。属于既有习惯，如果在意，建议整体清理而非卡这一个 PR。

3. **确认态会被每 60 秒的重渲染冲掉**：`render()` 的 `signature` 里含 `fresh(creditsAt)`，而 `used.disabled` 也依赖它。列表重建时局部变量 `confirming` 归零，用户点了一次"确认使用1次"后如果恰好跨过重建，按钮会弹回"使用重置"。实际影响很小（保持可见时每 60 秒都会刷新 `creditsAt`，`fresh()` 一直为真），但若有 credit 恰好在此时过期就会触发一次重建。**顺带一提，保留重试态是显式做了的**（`transportAttempt` 存在时单独渲染"重试同一次重置"按钮并进 signature），说明作者意识到了重建问题，只是没覆盖确认态。

4. **`updated_at` 冲突面**：本 PR 把 `updated_at` 从 `2026-09-23T02:43:09Z` 改成 `2026-10-05T06:58:01Z`。目前它是唯一开放 PR，无冲突。合并后按上一轮报告的建议，统一刷成合并当天。

---

## 建议

**可以合。** 合并前建议让作者做两件事：改正文的 "18 tests" 说法、给 `homepage` 换成实际作者仓库（如果这是 `small32` 的作品，目前市场上没有指向作者主页的入口）。

合并动作：`gh pr merge 96 --repo BigPizzaV3/CodexPlusPlusScriptMarket --squash`，然后重算一遍全仓库 SHA 一致性、把 `updated_at` 刷成合并当天。

**关于"要不要放行会改账号状态的脚本"，这是需要你拍板的产品决策**，代码侧我已经确认没有可指摘之处。
