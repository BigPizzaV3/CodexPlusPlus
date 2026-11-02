# Codex App 卡顿诊断与修复（2026-11-02）

一次实测诊断的记录：用户反馈「Codex App 用起来很卡」，逐层量了 CPU / 内存 / 桥往返 / 子进程，
把原因拆成三层，并按优先级修掉。文中的数据都是这台机器上现场采集的，不是估算。

## 现象

- 打开会话后滚动、切会话有明显延迟，输入偶尔掉帧
- 活动监视器里 Codex 的渲染进程常年 GB 级
- 系统里堆着几十个 `Codex++ --codex-plus-dsh-mcp` 和一个配套的 `node host.mjs serve`

## 证据

### 一、子进程泄漏（当时最严重的项）

```
codex app-server (pid 47737)
 ├─ Codex++ --codex-plus-dsh-mcp   × 14（历史峰值 45）
 │   └─ node host.mjs serve        × 14（每个 68 MB）
 ├─ Codex++ --codex-plus-extension-mcp × 5
 └─ node cua-repl.mjs              × 7
```

- 44 个残留进程合计 **2.2 GB**
- 逐刻观察：30 秒内 8 → 14 个，泄漏速率约「每开一个会话 / 每重连一次一个」
- `~/.codex/config.toml` 里 `[mcp_servers.codex_plus_collaboration]` 的 `command` 指向
  `dist/local-build-real-config-20261017-rfb0gyic/`（该目录早已被清理），Codex 每次启动都
  spawn 失败并重试
- `dist/local-build/state/dsh-plugins/` 里只有 `host/`，**没有 registry.json**——一个 dsh 插件
  都没装，但这条 MCP 仍然常驻，纯空转
- `assets/dsh-host/host.mjs` 只在 stdin `close` 时退出（`lines.on('close')`）。Codex 重连时
  不一定关管道，孤儿就永远停在 `createInterface` 上等数据

### 二、渲染进程内存

- 两个 renderer：一个启动 2.5 分钟 **1.2 GB**，另一个启动 3 分钟 **3.2 GB**
- 另一个跑了 4.4 小时的 renderer：RSS **6.8 GB**，footprint 2.2 GB / 峰值 2.8 GB
- 空会话页（DOM 只有 5911 个节点）JS 堆 **428 MB**
- 664 MB 堆的 CDP 快照按 `self_size` 聚合后没有单一巨型泄漏，是海量碎片对象：
  `Object` × 208,604、`array` × 76,591、`closure` × 135,397、`Map`(shape) × 75,423、
  `BytecodeArray` × 34,315、`UncompiledDataWithoutPreparseData` × 53,325、
  `SourceTextModuleInfoEntry` × 27,802，外加 61 MB `ExternalStringData`
  和两份 `feature_gates` JSON（9.47 MB + 6.34 MB）
- `WindowServer` 占用 47% CPU：合成层压力（1846 条 CSS 规则 / 5911 个节点）

### 三、桥往返

统计 `codex-plus.log` 最近 1373 秒的 `bridge.request`：

| 路由 | 次数 | 速率 | 平均间隔 |
|---|---|---|---|
| `/extensions/history` | 3145 | 137.4 /分钟 | 437 ms |
| `/request-log/recent` | 1323 | 57.8 /分钟 | 1038 ms |
| `/thread-usage-live` | 857 | 37.4 /分钟 | 1603 ms |
| `/settings/get` | 271 | 11.8 /分钟 | 5068 ms（每次 166 KB） |

`/extensions/history` 的来源是 `91-extension-runtime.js` 的会话监视器：每 1.5 秒
`listSessions({limit:20})` + 对**前 3 个会话无条件** `readEventsSince`，等于每轮固定 4 次桥往返。

日志本身：39 MB、约 1584 行/分钟（26 行/秒），每条都 open + write + close 一次。

## 修复

### 前端（`assets/inject/renderer-inject/`）

1. **会话监视器降频**（`91-extension-runtime.js`）
   - 列表查询缓存，每 4 轮（约 6 秒）刷新一次
   - 只有「当前打开的会话」每轮实时读，其余按 `updatedAt` 变化才读
   - 页面隐藏时 5 秒 → 30 秒
   - 预期：137 次/分钟降到 ~40 次/分钟

2. **只读轮询合并**（`91-extension-api.js`）
   - 新增 `codexPlusExtensionCoalesceWindowMs`：`/request-log/recent`、`/thread-usage-live`、
     `/proxy-status` 1 秒，`/backend/status` 2 秒，`/settings/public` 5 秒，
     `/model-catalog/public` 30 秒
   - 窗口内同参调用复用同一个 Promise；失败不缓存
   - 拓展行为不变（拿到的仍是真实数据，最多晚一个窗口）

3. **设置改按版本拉取**（`40-backend-settings.js`）
   - 心跳只比对 `/backend/status` 新增的 `settingsRevision`，变了才拉 `/settings/get`
   - 旧后端不带版本时退回原来的全量拉取

### 后端（`crates/codex-plus-core/`）

4. **`routes.rs`**：`/backend/status` 回传 `settingsRevision`（来自已有的
   `BackendSettings::settings_revision`，零额外成本）

5. **`managed_mcp.rs`**：`upsert_managed_context` 给托管条目补 `startup_timeout_sec = 120.0`。
   dsh 宿主冷启动要加载整张插件注册表，超过 Codex 默认 MCP 启动超时就会判失败并重连

6. **`dsh_plugins.rs`**：新增 `has_enabled_plugins()`；`run_mcp` 在一个能用的插件都没有时
   直接返回，不再拉 68 MB 的 Node 宿主

7. **`diagnostic_log.rs`**：写句柄常驻进程（不再每条 open/close），每条仍是一次追加写，
   不引入用户态缓冲（进程随时退出都不丢最近的记录）。体积检查从「每行 stat」改成 5 秒一次。
   上限 50 MB → 24 MB

### 宿主（`assets/dsh-host/host.mjs`）

8. serve 入口加父进程存活检测：每 5 秒比对 `process.ppid`，被 init 收养就
   `dispose()` 后退出。实测把中间父进程杀掉后，宿主 1 秒内自行退出

### 工具链

9. `scripts/assemble-renderer-inject.mjs` 写产物时同步 `manifest.json` 的分片行数
   （以前每次改分片都要手数，`--check` 只报错不修）

## 核查后确认「不需要改」的项

- **注入脚本重复执行**：`10-style.js` 按 id + 版本去重，`99-startup.js` 会先
  `disconnect()` 旧 observer。页面上的 10 个 `<style>` 是不同用途，不是泄漏
- **`inline_image_data_url_leak`**（累计 1341 次）：网关侧 `guard_inline_image_data_urls`
  已经在请求体里清掉了 data URL，这条日志是「清理到了」的记录。渲染进程里的图片来自
  Codex 自己的消息内容，不在 Codex++ 的改动面
- **`feature_gates` 双份 15.8 MB**：statsig 客户端与 Codex 各存一份，属于上游行为

## 验证方式

```bash
# 进程数应稳定在个位数
ps aux | grep -c '[c]odex-plus-dsh-mcp --state-dir'

# 桥往返速率（改前 /extensions/history 约 137 /分钟）
tail -20000 dist/local-build/state/codex-plus.log | grep -c '"/extensions/history"'

# 内存
ps -o rss=,command= -p $(pgrep -f 'Codex (Renderer)' | head -1)
```
