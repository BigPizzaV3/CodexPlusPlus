# Issue #2169 分析:v1.3.0 调试端口连接导致 CPU 高占用

> 基于上游 `upstream/main@1446873`(v1.3.0 之后、无后续相关修复)静态分析。
> 原始 issue:BigPizzaV3/CodexPlusPlus#2169(OPEN,无评论)。

## 1. Issue 报告的症状

| 进程 | 现象 |
|------|------|
| Codex++ 主进程 | ~68% CPU 持续空转 |
| ChatGPT 主进程(Electron/Node) | 初始 burst;采样显示 `uv_run → CheckImmediate` 紧密循环,~80 次/秒 sysctl |
| Codex (Renderer) | 采样显示 `node::profiler::V8HeapProfilerConnection::Start()` 激活未释放 + PerfettoTrace 线程;物理内存 792.5M、峰值 1.9G |

报告者验证:退出 Codex++ 后 CPU 立即恢复(即 CPU 消耗需要 Codex++ 的 CDP 客户端在场)。

## 2. Codex++ 与 ChatGPT 的真实连接架构(代码确认)

1. **启动参数**(`crates/codex-plus-core/src/launcher.rs:2400`):
   - `--remote-debugging-port=9229`(Chromium CDP,默认端口,见 launcher.rs:107);
   - 可选 `--inspect=127.0.0.1:9329`(原生菜单本地化,默认开启 `settings.rs:617`,**v1.2.19 起就存在,非 v1.3.0 新增**)。
2. **常驻 CDP 会话**(`bridge.rs:253 install_bridge`):每条会话发送
   `Runtime.enable` → `Runtime.removeBinding/addBinding(codexSessionDeleteV2)` →
   `Page.addScriptToEvaluateOnNewDocument`(bridge 脚本 + ~480KB 皮肤脚本 + 用户脚本) →
   `Runtime.evaluate`(立即执行全部脚本) → spawn 常驻消息循环。
3. **看门狗**(`launcher.rs:941 start_bridge_watchdog`):每 5s 执行
   `browser_identity`(HTTP /json/version)+ `sync_pet_real_mouse_overlay`(list_targets + 可能 evaluate)+
   `bridge_health_ok`(新建 WS + `Runtime.evaluate(awaitPromise)`)。
   健康检查返回 false 连续 **2 次**(`BRIDGE_HEALTH_FAILURE_THRESHOLD = 2`,launcher.rs:24)→ 重注入。
4. **重注入代价**(`try_inject_with_context`,launcher main.rs:1020):重新走一遍第 2 步,
   即每 10s 重新传输并编译执行 ~480KB JS;旧会话靠 generation 机制退出。
5. **渲染层心跳**(`renderer-inject.js:3971`):`setInterval(checkBackendStatus, 5000)`,
   经 bridge 调 `/backend/status`,成功即刷新 `__codexPlusBridgeHealth.lastSuccessAt`(renderer-inject.js:3888)。
   **稳态下心跳把健康检查撑绿,不会周期性重注入**(修正 issue 作者"轮询调试协议无 backoff"的部分猜测)。

## 3. 与症状逐条对照

### 3.1 ✅ "通过调试端口持续连接 ChatGPT" —— 属实
常驻 bridge WS + 每 5s 看门狗(HTTP×2 + WS×1~3)+ 每 5s 渲染层心跳,与 `lsof` 观察一致。

### 3.2 ❌ "V8 堆分析器由 Codex++ 的 CDP 会话激活" —— 不成立
- 全仓库检索 `HeapProfiler|Profiler.|Tracing|Perfetto` **零命中**(Rust 与前端均无);
  Codex++ 只发 `Runtime.*` / `Page.addScriptToEvaluateOnNewDocument` / `Page.captureScreenshot`。
- Node 源码证实(nodejs/node `src/inspector_profiler.{h,cc}` + `env.h`):
  `V8HeapProfilerConnection` 由 `Environment::heap_profiler_connection_` 持有,
  仅随 **`--heap-prof` CLI 选项族**(`heap_prof_name/dir/interval`)创建,用于落盘 `.heapprofile`;
  远程 CDP 客户端无法创建该连接。
- 推论:Renderer 里有 `V8HeapProfilerConnection::Start()` 意味着该进程被传入了
  `--heap-prof`(或等价 env)。Codex++ 不传此参数 → 最可能是 **Codex/ChatGPT 应用自身诊断**
  (应用菜单含 "Start Trace Recording"/"Toggle React Scan"/"Toggle Query Devtools")
  在检测到调试环境后自启。PerfettoTrace 线程同理。**需 macOS 实机对照验证**。

### 3.3 ⚠️ "ChatGPT 主进程每秒 80 次 sysctl 轮询" —— 非 Codex++ 注入
Codex++ 对主进程唯一注入是一次性菜单翻译脚本(`native_menu.rs`,20×500ms 重试后即止,
无循环)。`CheckImmediate` + `os.*` 类 sysctl 是主进程自有 JS 在跑紧密 setImmediate 循环,
推测同为应用自身诊断/采样行为,与 3.2 同源。需实机 sample 对照。

### 3.4 ⚠️ "Codex++ 主进程 68% 空转" —— 静态分析未找到忙等循环
- 所有 Rust 循环均有 sleep/await:看门狗 5s(launcher.rs:949)、退出等待 2s(launcher.rs:1038)、
  注入重试 500ms×20、helper 为事件驱动;macOS 下无 pet 鼠标驱动(仅 `#[cfg(windows)]`)。
- 报告未提供 Codex++ 自身的 `sample`,68% 缺乏直接证据。候选解释:
  a. bridge 失效场景下的重注入风暴 + 会话堆积(见 3.5);
  b. psutil 对 launcher/helper 多进程的归并统计;
  c. 其他待实机定位。

### 3.5 ✅ 真实存在的退化机制:bridge 失效时的重注入风暴
健康检查口径(`bridge.rs:123`):`lastInjectionAt ≤5s` 或 `lastSuccessAt ≤15s` 才算健康。
当 bridge 因任何原因失效(应用更新改 DOM、绑定失效等,对应 "align with Codex 26.825" 一类变化):
- 每 10s 重注入一次(阈值 2 × 5s 节拍),每次重发 ~480KB JS 并全量 `scan()`;
- 旧会话只在"socket 再收到消息"时才退出(bridge.rs:322-352 的循环阻塞在 select 上);
  bridge 失效 = 无 binding 事件 = **旧会话无限期滞留**,每个都挂着 `Runtime.enable`;
- 泄漏会话数量线性增长(每小时 ~360 个),渲染层每条 console 消息需向 N 个会话序列化
  → 渲染进程 CPU 与内存随时间恶化(与 792.5M→1.9G 吻合),Codex++ 侧 N 条 socket 的读循环也同步堆积。
健康时心跳每 5s 产生 binding 事件,泄漏会话 ≤5s 内退出,该机制仅在失效场景发作。

### 3.6 渲染层常驻基线负载(背景,关联 #1816/#2043/#2181)
- `document.body` 子树 MutationObserver(childList + 2 个属性)(renderer-inject.js:10578);
- 350ms 对齐轮询 + rAF settle(≤16 帧)(renderer-inject.js:9411-9449);
- 4s `ensure`、1.5s `enforceVoice`、5s 心跳;`scheduleScan` 200ms 防抖后全文档 `scan()`。
空闲时这些定时器持续运转,是"空闲不下垂"的基线;流式输出时叠加全量重扫(fork 已修 #2181,upstream 未修)。

## 4. v1.3.0 回归点排查(v1.2.56 → v1.3.0 diff)

| 变更 | 与本 issue 的关系 |
|------|------------------|
| ade2001 + 2e5da00:健康检查改为"渲染层真实后端请求结果",窗口 5s/15s,阈值 2 | 判定口径变严,应用更新后更容易进入重注入风暴;与"v1.3.0 才出现"部分吻合 |
| Dream Skin v1.5.16 runtime 同步 | 渲染层基线负载变化,待比对 |
| #2098 macOS 进程检测改 `ps -axo pid=,args=`(每 2s) | 开销约 2-4% CPU,非主因 |
| `--inspect` / `--remote-debugging-port` / watchdog 结构 | v1.2.56 完全相同,排除 |
| v1.3.0 之后 upstream 未再改这些文件 | 无既存修复可依赖 |

## 5. 建议的验证步骤(需 macOS 实机)

1. 对 **Codex++ 主进程本体** `sample <pid> 3`(报告缺失的关键数据);
2. 手动 `open -a ChatGPT --args --remote-debugging-port=9229`(不经 Codex++)+
   手动 ws 连接发 `Runtime.enable` → 复测;再不带调试参数启动 → 三方对照;
3. `ps -axo pid,args | grep -E "heap-prof|NODE_V8_COVERAGE|NODE_OPTIONS"` 确认 Renderer 的
   `--heap-prof` 来源;
4. 复现时统计 `diagnostic_log` 中 `bridge.health_check_failed` / `bridge.reinject_*` 频率,
   验证 3.5 的风暴假设;`lsof -i :9229 | wc -l` 观察会话数是否随时间增长。

## 6. 修复方向(供后续 fix 分支)

1. **旧会话主动退出**:generation 过期后不能依赖"下一条消息"——在旧会话 select 中加
   超时分支(如 1s)轮询 generation 并 close;或新会话建立时向旧 socket 写入消息促其退出。
2. **重注入退避**:连续失败后指数退避(10s→20s→40s,封顶),并把阈值从 2 放宽或引入时间窗。
3. **脚本注册清理**:记录 `Page.addScriptToEvaluateOnNewDocument` 返回的 identifier,
   重注入前 `Page.removeScriptToEvaluateOnNewDocument` 清掉本会话旧脚本。
4. **健康检查降载**:`browser_identity` 每 5s 一次 HTTP 可与 health 检查合并复用同一目标列表。

## 7. 本修复补充：清理 new-document 脚本注册

每次 `install_bridge` 都会调用 `Page.addScriptToEvaluateOnNewDocument`。旧版本只
关闭 WebSocket，会话虽然退出，但 Chromium 目标上的脚本注册仍可能保留；连续重注入
会让后续新文档执行重复脚本，增加 renderer 的工作量。当前修复保存 CDP 返回的
`identifier`，在 generation 被替换或会话退出前发送
`Page.removeScriptToEvaluateOnNewDocument`。CDP 未返回 identifier 时保持兼容并跳过清理。

该路径由 `crates/codex-plus-core/tests/cdp_bridge.rs` 的陈旧会话测试覆盖。由于当前
Windows 开发环境未安装 Rust/Cargo，编译测试和 10--20 分钟真实 Codex 对话压力测试
需在具备 Rust 工具链的 Windows 环境中完成；本分支不宣称已完成 Linux/macOS 实测。
