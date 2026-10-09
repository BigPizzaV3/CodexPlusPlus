# PR 合并执行记录（2026-09-23）

承接 [2026-09-23 新一轮审查](#) 的结论。本轮按用户指示：**能合的都合，不能合的回复或关闭**。

三个仓库的开放 PR 从 13 个收敛到 2 个。

---

## 一、已合并（11 个）

### 主仓库 `BigPizzaV3/CodexPlusPlus`（10 个）

| PR | 类型 | commit | 验证 |
|---|---|---|---|
| #2283 | fix（P0，修 #1604） | `d09d5e4` | relay_switch 16 passed、relay_config 166 passed |
| #2290 | ci（官方元数据定期检查） | `4e1d8df` | 只读 workflow，含 `--check` 语义核对 |
| #2225 | fix（Responses 默认值） | `993b48e` | — |
| #2281 | feat（GPT-6 Sol/Luna 元数据） | `7e19e06` | — |
| #2282 | fix（官登额度锁） | `0bbcb76` | npm 237 passed |
| #2286 | fix（混合 API 额度拦截） | `a9af207` | api_quota_gate 2 passed、npm 237 passed |
| #2285 | feat（自定义上游请求头） | `ea0ac5d` | — |
| #2280 | feat（15 家供应商元数据） | `7e7d44a` | model_suffix 24、relay_config 170 passed |
| #2288 | fix（mcp_servers 写盘补齐） | `7daabac` | node_repl_flattened_profile 1、relay_* 全过 |
| #2239 | fix（微信 CLI 校验） | `55cfb41` | manager 72+21、launcher 12 passed |

### 脚本市场 `CodexPlusPlusScriptMarket`（1 个）

| PR | 类型 | commit | 说明 |
|---|---|---|---|
| #92 | feat/fix（DeepSeek Token Usage 1.19.11） | `380b1726` | 逐条回应了 #88 的驳回意见 |

### 冲突的手工解决（3 个）

这三个 PR 都与 main 冲突，未让作者 rebase，而是本地解析后直推 main（署名保留在提交信息里）：

**#2280** — 与先行合入的 #2281 在 `model_suffix.rs` 冲突。两边都是新增，按「都保留」解决：
保留 #2281 的 `GPT6_SOL_LUNA_METADATA_JSON`、保留 #2280 的 `VENDOR_METADATA_JSONS` 列表，
`compatibility_metadata_entry` 先查供应商列表再回退 sol/luna。两侧 slug 不相交。

**#2288** — 与 main 在 `relay_config.rs` 冲突。按 PR 意图解决：采用其新增的
`fill_missing_toml_item` / `repair_mcp_servers_from_live`，`mcp_servers` 的保留调用改为后者
（补齐条目内缺失键 + 清掉混进父表的 env 键），`features` 仍走 `preserve_missing_table_keys`。
**这是 PR 有意的语义替换，不是陈旧基线。**

**#2239** — 与 main 两处冲突。`lib.rs`：PR 侧是旧基线（未含 main 已重构的
`is_background_launch` 与其测试），取 main 侧。`commands.rs`：保留 PR 核心意图
（先验证 CLI 可启动再采纳，失败回退用户目录独立 CLI），去掉与 main 重复的 `app_dir` 解析。

---

## 二、关闭并回复（4 个，脚本市场）

| PR | 关闭理由 |
|---|---|
| #86 | 17220 行打包产物不可审计；要求补源码仓库与可复现构建 |
| #87 | 拦截发送 + 合成事件自动改模型选择，权限超出只读监控 |
| #88 | 响应读取范围过宽 + 描述与产物版本不一致（**已在 #92 修复后重投并合并**） |
| #89 | 覆写 Statsig gate 在客户端打开厂商灰度关闭的入口 |

## 三、回复但保持开放（1 个）

**#2262**（恢复原生会话中缺失的用户消息，+3150，130040167）

功能与写盘安全性审过，**没有发现问题**：写盘前 `VACUUM INTO` 备份、`Immediate` 事务取锁后重新核验、
未知 schema 直接 `bail!`、冲突项不阻断其它安全项、截断尾行不缓存为成功、证据不足不猜测恢复、
旧 `inProgress` 需日志与库共同证明才补消息。

**阻塞点是它自带测试的隔离缺陷**：

```
cargo test -p codex-plus-data --lib                        # 默认并行：每次 1~3 个失败
cargo test -p codex-plus-data --lib -- --test-threads=1   # 单线程：75 passed, 0 failed
```

并行连续 5 次失败数 `2/3/3/2/1`，**失败集合每次不同**，错误为
`会话索引修复正在运行，请稍后查看报告`（`Resource temporarily unavailable (os error 35)`），
对应 `session_index_repair.rs:537` 的 `lock.try_lock_exclusive()`。

失败集合随机 + 单线程全过 → 测试之间争用独占锁，非逻辑错误。该模块（`session_index_repair.rs`
/ `session_index_scan.rs`）与 51 个测试均为本 PR 新增；同期 main 的 `codex-plus-data --lib`
并行 3 次均 25 passed / 0 failed，故确认由本 PR 引入。

**CI 抓不到**：该 PR 的检查只有三个构建任务（Windows artifacts / macOS DMG x64 / arm64），不跑
`cargo test`。已请作者修隔离后用默认并行连续验证。

---

## 四、仍开放（1 个，需决策）

**#2206**（独立 Taskboard 应用，acc-c，+59987/-0，129 文件）

- 对当前 main **无冲突**（`merge-tree` CLEAN）。
- **测试实测 388 passed / 0 failed**（需先 `npm install`；先前在别处看到的失败是缺依赖导致的假象）。
- 自我隔离：除 `.github/workflows/taskboard-build.yml` 外，改动全部落在 `apps/codex-taskboard/`，
  不触碰主仓共享代码。自带 CI（typecheck / web build / test）。
- 作者已知 `npm ci` 报 6 个 audit findings（1 moderate / 5 high），依赖版本未变。

**这是 feat 类、体量大（近 6 万行）的新应用，按既定政策由用户决定是否承接维护面**，未擅自合并或关闭。

---

## 五、最终状态

| 仓库 | 开放 PR |
|---|---|
| `BigPizzaV3/CodexPlusPlus` | **2**（#2262 待修测试、#2206 待决策） |
| `BigPizzaV3/CodexPlusPlusScriptMarket` | **0** |
| `BigPizzaV3/CodexPlusPlus-Themes` | **0** |

## 六、遗留项

1. **#2262** 测试隔离缺陷，待作者修复后重新提交。
2. **#2206** 待用户决定是否承接。
3. 主仓 `codex-models.json` 与 `gpt56/astra-model-metadata-compat.json` 存在 4 个重复 slug
   （`gpt-5.6-sol` / `gpt-5.6-terra` / `gpt-5.6-luna` / `gpt-6-astra`），**main 既有**、非本批引入。
4. 脚本市场 `index.json` 的 `updated_at` 现为 `2026-09-23T02:43:09Z`（#92 写入）。
5. #2239 的 macOS DMG (x64) 在旧 run（35416145150）有过一次 FAILURE，本次未重跑 CI；
   用本地解析后的树验证后直推，若那条失败是环境抖动可忽略。
6. 合并期间发现用户在本地 `CodexPlusPlus` 工作区有一次未完成的 merge
   （detached HEAD at `pr/2283`，`MERGE_HEAD=cd78c756`）。**未触碰**，审查全部在独立 clone 中进行。
