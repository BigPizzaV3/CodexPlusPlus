# 官方 App 自动接管设计方案

日期：2026-10-13
范围：`crates/codex-plus-core/src/{launcher,watcher,ports,status,auto_takeover}.rs`、`apps/codex-plus-launcher/src/main.rs`、`apps/codex-plus-manager/src-tauri/src/{lib,commands,auto_takeover}.rs`、`apps/codex-plus-manager/src/App.tsx`
状态：设计稿 v2，未实施（待确认）

**一句话**：用户自己双击 / Dock / `open -a` 启动官方 App 时，Codex++ 自动把它接管过来——已在运行且没带调试端口的，先优雅退出再带调试参数重开，然后照常 CDP 注入，效果与「从管理工具点启动」完全一致。

> 依据：本设计经两轮独立对抗性审查（`work/review-auto-takeover-code.md`、`work/review-auto-takeover-design.md`）修订，两份调研见 `work/research-auto-takeover-macos.md`、`work/research-auto-takeover-settings.md`。v2 修正了 v1 的 4 处硬伤：会话归属无法自证、有任务在跑的闸门无执行路径、多开判据不存在、SIGTERM 目标 PID 无来源。

---

## 一、现状梳理

### 1.1 接管链路（已存在，缺的只是触发源）

| 阶段 | 位置 | 说明 |
| --- | --- | --- |
| 入口 | `apps/codex-plus-launcher/src/main.rs:277` `start()` | 只有「管理工具点启动」与「恢复已有实例」两条路；**阻塞到 launch+inject 全部完成才返回** |
| 会话线程 | `main.rs:479` `run_embedded_session` → `main.rs:575` `launcher_session` | 专用线程 + `LocalSet`，会话内取 `launcher_guard_port()` |
| 主链路 | `crates/codex-plus-core/src/launcher.rs:566-791` `launch_and_inject_with_hooks` | 端口选择 → helper → provider sync → 协议代理 → 拉起 App → 注入 → watchdog → `ready` |
| macOS 拉起 | `launcher.rs:1107-1147` | `app_dir` 是 `.app` 且非多开时走 `select_macos_debug_launch_action` + `build_macos_open_command`（`launcher.rs:3717`） |
| 已在跑但无 CDP | `launcher.rs:3754-3762` 真值表 → `RestartRunningApp` → `quit_macos_app_and_wait`（`launcher.rs:3785`） | **「退出后带参重开」的逻辑已经写好** |
| 注入 | `launcher.rs:3106` `retry_injection`（20×500ms）／`LaunchHooks::ensure_injection` 默认实现（`launcher.rs:323`，120×1s） | 实测冷启动注入窗口 2522/3080/3394 ms |
| 常驻守护 | `launcher.rs:1230` watchdog（5s tick，断链重注入，退避 10s→300s，`launcher.rs:28-33`） | 接管成功后一直有效 |

**关键**：`codex-plus-launcher` 是**被当库链接进管理器进程**的（`commands.rs:1541`、`:2305`、`:2322`），会话与 `RuntimeState` 都在管理器进程内存里（`main.rs:215-247`）；它的公开 API 目前只有 `set_navigation_handler` / `start` / `resume_if_running` / `stop` / `shutdown`。**没有「本进程是否已有会话、是否 ready」的查询**——这是 v1 设计踩坑的地方（见 §3.6）。

### 1.2 官方 App 的事实（本机实测）

- `/Applications/ChatGPT.app`，`CFBundleIdentifier=com.openai.codex`，主可执行 `Contents/MacOS/ChatGPT`。
- 官方 CLI：`/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex`，bundle id **`com.openai.codex.cli`**（`codex-package.json` version `0.162.0-alpha.17.2`）；App 运行期会拉起 `codex … app-server`。
- `open -a X --args ...` 对**已在运行**的 X **不传参**（实测：不新起进程、参数丢弃、只激活窗口；`-n` 才新起实例，`-W` 阻塞到 App 退出）。
  → **无法给运行中的实例补调试端口，必须先退出再带参启动**——这正是既有真值表的做法。
- 多开实例：由管理器用 `--codex-plus-multi-worker <options>` 启动自身进程（`commands.rs:1178-1191`），带 `--user-data-dir=~/Library/Application Support/CodexPlusPlus/multi/<slug>`，**Args 里没有任何实例标识**；清单在 `~/.codex-session-delete/codex-multi/<slug>/worker-options.json`。

### 1.3 五个既有缺陷（全部实测/代码取证）

| # | 缺陷 | 证据 | 对自动接管的影响 |
| --- | --- | --- | --- |
| D1 | macOS 退出预算 `MACOS_DEBUG_TAKEOVER_WAIT_MS = 5000`（`launcher.rs:86`）可能太紧，但**因果未证实** | 25 次 `macos_existing_app_without_cdp_restart_requested`：17 次成功（752–3422 ms，均值 1813.0）、7 次失败（5175/5240/5260/5265/5275/5290/5293 ms）、1 次无终态 | 失败后 App 处于「在跑但无 CDP」，无退避机制就会重试风暴。**注意**：7 次全部落在 5s 窗口外，只能说明「5s 内没退」，不能区分「快退了」与「quit 根本没被受理」——加预算前必须先能读出原因（见 D3 修法） |
| D2 | `find_codex_processes()` 在 macOS 把 codex-cli 当官方 App | `watcher.rs:1261-1267` 的「点子标识符」规则放行 `com.openai.codex.cli`；实测 PID 17014 被匹配。**Windows 侧有对应排除**（`watcher.rs:117-135` 要求 `app\` 且非 `app\resources\`，回归测试 `tests/watcher.rs:207-219`），macOS 侧完全没有 | 不能拿它判「App 是否在跑」；`provider_sync.rs:976-990` 的闸门与 `runtime_health.rs:45` 也受污染 |
| D3 | 退出路径两处静默失败 | ① `run_macos_cleanup_command`（`launcher.rs:3765-3781`）`let _ = …status().await.with_context(...)?` 丢弃 osascript 退出码，stdout/stderr 都是 null；② `is_macos_app_running`（`launcher.rs:3815-3841`）把「osascript 失败」与「未运行」压成同一个 `false` | ②比①更危险：`quit_macos_app_and_wait`（`launcher.rs:3789`）**第一次查询失败就认为 App 已退出** → 直接 `open -W -a`（对已在运行的 App 不传参）→ 注入必然失败 |
| D4 | `open -W` 被 SIGKILL 造成「假退出」 | `terminate_codex`（`launcher.rs:1360-1388`）对 `ExternalWaitCommand` 先 `child.kill()`（tokio = SIGKILL，杀的是 `open -W` 进程，不是 App）；而 `wait_for_codex_exit`（`launcher.rs:1292-1296`）正是靠 `child.wait()` 判 App 是否退出。本机存在孤儿 `open -W`（PPID=1） | 会话收尾会 `shutdown_helper`，而 App 还开着 → `config.toml` 里写死的 `57321` 失联 |
| D5 | 管理器退出后官方 App 进入不可用状态 | 托盘退出 → `codex_plus_launcher::shutdown()`（`lib.rs:243-261`）→ 会话收尾关 helper；`launcher.rs` 自身注释承认「helper 一关，写死 57321 的 config 就没对端」 | **既有问题**，自动接管会把它从偶发变成常见；管理器重启时必须先走 `resume_if_running`（`main.rs:292-299`，只在端点可用时恢复），不要无条件 quit 重启 |

D2/D3/D4/D5 都需要独立修复；D1 的修法与它们绑定（先能读出原因，再决定预算）。

---

## 二、设计目标与非目标

**目标**

1. 官方 App 由用户自己启动时（Dock / Spotlight / `open -a`），Codex++ 在数秒内自动接管，不必再从管理工具点启动。
2. 接管在跑且无调试端口的实例时，允许**自动重启**它（已确认的产品决策），全程状态可见。
3. 接管后与既有链路完全一致（CDP 注入、协议代理、供应商同步、watchdog、`latest-status.json`）。
4. 默认关闭（opt-in）；任何异常都不产生重启循环；用户能随时关闭、跳过本次、立即接管。

**非目标**

- 不接管终端里的官方 CLI（TUI 无渲染层，注入只能是配置级；另立需求）。
- 不引入 `objc2` / NSWorkspace 原生依赖，不写 `~/Library/LaunchAgents`（下一期）。
- 不改 `window.codexPlus` 拓展接口层、不新增对外路由、不改 Codex App 内注入面板。
- ~~本期只做 macOS~~ **已补 Windows**（见 §3.13，2026-10-13 真机实测后落地）。

---

## 三、方案设计

### 3.1 触发点选型：管理器进程内轮询

候选：管理器内轮询 / launchd LaunchAgent / NSWorkspace 通知。选第一个：零新依赖（macOS 目标下 core crate 目前没有任何原生 crate）、能直接复用 `LaunchOptions`/`LaunchHooks`/`StatusStore`、有现成先例（`lib.rs:522-540` 的 300ms 轮询线程）。

代价：**只有管理器活着才生效**。macOS 没有开机启动实现（`watcher::install_watcher` 非 Windows 直接 bail，`watcher.rs:529-532`），所以诚实表述是「**开启后，仅在 Codex++ 正在运行时生效**」，并配合登录项引导（§3.9）。

### 3.2 架构

```
管理器进程 (Codex++.app)
├── Tauri/UI 线程 ── 开关、状态行、立即接管/本次不接管
├── 接管守护线程 (auto_takeover_supervisor)          ← 新增
│     ├── 探针循环（只做判据，绝不阻塞在启动上）
│     └── 触发线程（调 start()，可被 stop 信号打断等待）
├── 进程内启动串行锁 try_acquire_launch_guard()      ← 复用（commands.rs:1705/1715）
└── launcher 运行时（既有，专用线程 + LocalSet）
      └── launch_and_inject_with_hooks → 注入 → watchdog
```

三条硬约束：

1. **串行化必须复用 `try_acquire_launch_guard()`**（`commands.rs:1705` 的 `LAUNCH_IN_PROGRESS`）。v1 说「共用 `launcher_guard_port()` 天然串行」是错的：那个锁由会话线程在**同一进程内**持有，既不可见也不能偷看，而且管理器点启动根本不走它。
2. **探针绝不阻塞**：`start()` 会阻塞到注入完成（冷启动可达 80–110 s，见 `status.rs` 注释）。探针线程只判据，触发交给独立线程；宽限期必须在调用 `start()` **之前**打点。
3. **会话归属必须查内存态**：新增 `codex_plus_launcher::session_snapshot()`（§3.6），不从锁或端口反推。

### 3.3 状态机与判据（纯逻辑，可跨平台单测）

状态：`Disabled → Idle → Probing → TakingOver → Binding → Monitoring`，外加 `Degraded`、`Paused`。

```rust
pub struct TakeoverInputs {
    pub enabled: bool,                    // 设置开关
    pub platform_supported: bool,         // 仅 macOS 为 true
    pub session: SessionSnapshot,         // 见 §3.6：owned / ready / status / started_at_ms / debug_port
    pub app: AppProbe,                    // Running / NotRunning / Unknown（见 §3.4）
    pub cdp_ready: bool,                  // cdp::endpoint_available(actual_port)（严格判据）
    pub actual_port: u16,                 // 取自 latest-status / 本次 LaunchOptions，禁止硬编码
    pub multi_instance_running: bool,     // §3.8
    pub takeover_in_progress: bool,       // LAUNCH_IN_PROGRESS
    pub ms_since_our_launch: Option<u64>, // 宽限期：调用 start() 之前打点
    pub ms_since_last_attempt: Option<u64>,
    pub failures_in_window: u32,          // 滑动 1 小时
    pub paused_instance: Option<InstanceKey>, // (pid, lstart)
    pub now: Instant,
}

pub enum TakeoverDecision { Wait(&'static str), Skip(&'static str), Degraded(&'static str), Takeover }
```

判据顺序（先排除，后动作）：

1. `!enabled || !platform_supported` → `Skip("disabled")`。
2. `session.owned && session.status == "running_degraded"` → **`Degraded`**：注入失败且不会自愈（`running_degraded` 时不启 watchdog，`launcher.rs:743-745`；再点启动只走 `activate` 不重注入，`main.rs:350-363`），**不得显示「已接管」**，也不得重启 App。
3. `multi_instance_running` → `Skip("multi_instance")`（§3.8）。
4. `app == Unknown` → `Skip("probe_failed")`：探测失败与「没在跑」必须区分（D3②），Unknown 一律保守。
5. `paused_instance == current_instance_key` → `Wait("paused_for_instance")`。
6. `app == NotRunning` → `Wait("app_not_running")`（状态机复位到 `Idle`）。
7. `app == Running && cdp_ready` → `Wait("already_attached")`（`session.owned && status == running` 时才算「已接管」；否则只算「端口在」）。
8. 宽限期内（`ms_since_our_launch < GRACE_MS`）→ `Wait("grace")`。**这条是防自杀式重启的关键**：`open` 返回到 `app://-/` target 可注入之间有实测 2.5–3.4 s 空窗。
9. 滑动 1 小时窗口内失败/重启次数达上限 → `Degraded("restart_budget_exhausted")`。
10. 退避未到期 → `Wait("backoff")`。
11. 「看起来有人在用」且策略为 `defer` → `Wait("user_active")`（§3.5）。
12. 否则 → `Takeover`。

**参数**（依据同列）：

| 参数 | 取值 | 依据 |
| --- | --- | --- |
| 宽限期 `GRACE_MS` | 10 000 ms（在调用 `start()` **之前**打点） | 实测注入窗口最坏 3.4 s，冷开机更慢 |
| 探针间隔 | 活跃 2 000 ms；App 连续 60 s 未出现后 10 000 ms | 实测单次探针 `ps` 40 ms / `osascript` 50–70 ms |
| 退出等待 | 15 000 ms（原 5 000），100 ms 轮询 | 成功样本上限 3 422 ms；**但必须先修 D3 才能确认 5 s 是不是真瓶颈** |
| 唤醒宽限 | 唤醒后 30 000 ms 内只观察 | 唤醒后 macOS 常见 CDP 短时不可连；避免误重启 |
| 退避阶梯 | 30 s → 2 min → 10 min → 30 min（节流） | 复用 `BRIDGE_REINJECT_BACKOFF_*`（`launcher.rs:28-33`）的思路 |
| 硬闸门 | 滑动 1 小时窗口内 `takeover.restart_requested` ≤ 6（唯一计数口径） | 防风暴；退避只是节流，不是闸门 |
| 时钟 | 一律 `std::time::Instant`（单调钟） | 睡眠/唤醒时挂钟会跳，导致退避与宽限期瞬时过期后连发决策 |

### 3.4 「官方 App 在跑吗」的判据（绕开 D2，三态）

```rust
pub enum AppProbe { Running(InstanceKey), NotRunning, Unknown }

pub struct InstanceKey { pub pid: u32, pub lstart: String }   // 代际键

// 仅 macOS：候选进程 = 带我们端口的 Codex 主 App + 主包可执行文件精确匹配
pub fn probe_desktop_app(app_dir: &Path, actual_port: u16) -> AppProbe;
```

规则：

- **候选来源**：`watcher::find_codex_processes_for_debug_port(actual_port)`（`watcher.rs:1073-1076`，是 `pub`，返回 `Vec<u32>`）+ 新增 `find_macos_codex_desktop_main_processes()`。**不调用 `find_codex_processes()`**（D2 污染）。
- **精确匹配**：进程可执行路径必须等于 `<app_dir>/Contents/MacOS/<Info.plist 的 CFBundleExecutable>`。这条必须落成**真判据**，不能只写在注释里——挡住 renderer 的目前是「路径含 `/Helpers/`」这条旁路，而不是「主 App」这条。
- **uid 过滤**：`ps` 快照带 uid，只认本进程 uid；`kill(pid, 0)` 必须区分 `EPERM`（活着但不可操作 → `Unknown`）与 `ESRCH`（真死）。
- **代际核对**：缓存 `(pid, lstart)`；判活时用 `ps -o lstart=,comm= -p <pid>` 核对，`kill(pid,0)` 只作快速否定（返回 `ESRCH` 才敢说死）。防僵尸态、PID 复用、换代。
- **`osascript` 降级为「只在需要发 quit 时使用」，永不用于判活**：它按应用名解析（`app_dir.file_stem()`），会（a）把一个 bundle 的多个实例都算成「在跑」，（b）在非 0 退出时无法与「未运行」区分。
- **端口只由调用方注入**：`actual_port` 取自 `latest-status.json` 的 `debug_port`（与 `runtime_health.rs:58-62` 同做法）或本次 `LaunchOptions.debug_port`。**禁止硬编码 9229**：端口被占时 `select_packaged_codex_debug_port` 会自换（`ports.rs:71-99`），硬编码会导致「判无 CDP → 再接管」的循环。
- **非 macOS 的分支不要复用**：`find_codex_processes_for_debug_port` 在非 macOS 退化成 `find_codex_processes()`（`watcher.rs:1078-1081`），语义完全不同；判据函数必须 `cfg(target_os = "macos")`，跨平台单测通过参数注入快照，不直接调它。

### 3.5 「有人在用」的处理：defer 优先，不靠 CDP

v1 的错误：把闸门描述成「接管前通过 CDP 判断有没有任务在跑」——而触发条件恰恰是 `!cdp_ready`，**没有调试端口就没有 CDP**，这个闸门没有任何可达路径，还会与验收自相矛盾。

改为**产品层可选策略** `codexAppAutoTakeoverWhenActive`（默认 `defer`）：

| 取值 | 行为 |
| --- | --- |
| `defer`（默认） | 探针发现「疑似有人在用」→ 记 `takeover.deferred`，等下一轮；用户点「立即接管」随时可强制 |
| `immediate` | 不判断，直接按 §3.3 判据接管（用户明确选择的「打开就接管」） |

「疑似有人在用」只用**本地只读信号**（都失败向保守，即宁可不动）：

1. `~/.codex/sessions/**/*.jsonl` 最近 N 秒内有追加（运行中的任务会持续写事件流）；
2. Codex 主进程子树的 CPU 时间在两次采样间有明显增量。

**明确不用**：`.codex-global-state.json`（白名单只含 UI 偏好键，in-flight 状态不落盘）、本地 SQLite（只是历史索引）。这两条 v1 曾考虑，会让实现者走错路。

另加**首次开启的一次性确认**：开启开关时提示「将重启官方 App 以接管，继续？」，降低「App 突然重启」的惊吓。这与「默认关闭的 opt-in」定位一致。

> 待验证 V2：上述两个本地信号在真实任务期间是否可用（验证方法见 §6）。若都不可用，`defer` 退化为「只在 `immediate` 或用户手动触发时接管」，并在 UI 说明。

### 3.6 会话归属：给 launcher 加只读快照 API

新增到 `apps/codex-plus-launcher/src/main.rs`：

```rust
pub struct SessionSnapshot {
    pub owned: bool,               // 本进程当前是否有会话（RuntimeState::Running）
    pub ready: bool,               // SessionState.ready
    pub status: Option<String>,    // latest-status.json 的 status（含 running_degraded）
    pub debug_port: Option<u16>,
    pub started_at_ms: Option<u64>,// 代际
}

pub fn session_snapshot() -> SessionSnapshot;   // 纯内存读，无副作用
```

- 读 `RUNTIME_WORKER` / `RuntimeState` / `SessionState`（`main.rs:215-247`、`main.rs:58-64`），不新增状态文件。
- `status` 从 `StatusStore::load_latest()` 补齐（同一进程，读盘成本可接受，且这是 `running_degraded` 的唯一来源）。
- **`StartupBusy` 必须映射为 `Wait`**，不是失败：会话存在但未 ready 时 `RuntimeCommand::Start` 返回 `StartupBusy`（`main.rs:74-83`、`:356-359`），计入失败会污染退避阶梯。

### 3.7 退出路径修正（D1/D3/D4）

新增可测的退出协调（替换 `launcher.rs:1127` 的调用点）：

```rust
pub struct QuitOutcome { pub exited: bool, pub escalated: bool, pub elapsed_ms: u64, pub reason: &'static str }

pub async fn quit_macos_app_with_budget(
    app_dir: &Path,
    expected: InstanceKey,        // 代际：只对这个实例动作
    graceful_wait: Duration,      // 15000
    poll_interval: Duration,      // 100
) -> QuitOutcome;
```

1. `osascript -e 'tell application "X" to quit'`，**检查退出码**；非 0 时把 stderr 摘要写进 `reason`（D3①）。
2. 轮询判活改用 §3.4 的 `probe_desktop_app`（三态），**不再用 `is_macos_app_running`**（D3②）；`Unknown` 时退化为重试而不是「已退出」。
3. 期间持续写 `latest-status.json` 的 `phase`（`takeover.quitting`）与进度。
4. 超预算后**默认不升级**：置 `Degraded` + UI 给出可操作提示（「官方 App 未响应退出请求，请在 Codex 里手动 ⌘Q，或点这里重试」）。
5. 另设 `codexAppAutoTakeoverForceQuit`（默认 `false`）：开启后允许在 `graceful_wait` 用尽时对**代际仍匹配**的实例发 `SIGTERM`，再等 5 s；代际不匹配（PID 变了 = 用户新起的实例）一律放弃。**SIGTERM 可能不 flush 状态**，这是显式产品决策，必须等 V1 结论后再考虑启用。
6. `quit_macos_app_and_wait`（`launcher.rs:3785`）全仓只有 `launcher.rs:1127` 一个调用点，替换后**删除**（否则 dead_code）；`run_macos_cleanup_command` 的另一个调用点 `launcher.rs:1374` 用 `let _ =` 吞错，一并改为记日志。
7. **D4 单独修**：`wait_for_codex_exit` 对 `ExternalWaitCommand` 不能只靠 `child.wait()` 判 App 退出（`open -W` 一旦被杀就假退出，`launcher.rs:1292-1296`），要叠加 §3.4 的进程判据。**先修 D4，再谈退出预算**——否则预算调多大都治不了「假退出」。

### 3.8 多开实例判据

多开 = 管理器用 `--codex-plus-multi-worker` 启动自身进程（`commands.rs:1178-1191`），带 `--user-data-dir=.../CodexPlusPlus/multi/<slug>`，清单在 `~/.codex-session-delete/codex-multi/<slug>/worker-options.json`。官方 App 的 Args 里**没有**任何实例标识，v1 说的「跳过带 `-p` 的实例」不存在。

- 判据：**多开 worker 进程存活**（读 `codex-multi/*/worker-options.json` 里的 `debug_port` + `find_codex_processes_for_debug_port(那个端口)`，或直接复用 `list_codex_multi_instances_blocking`，`commands.rs:1273+`）→ `Skip("multi_instance")`。
- 必要性：`osascript quit` 按**应用名**发事件，会把用户正在用的多开实例一起退出。**这是最容易漏掉的破坏性路径**。

### 3.9 守护线程实现与生命周期

新增 `apps/codex-plus-manager/src-tauri/src/auto_takeover.rs`：

```rust
pub struct AutoTakeoverState {
    pub state: &'static str,           // disabled | idle | probing | taking_over | degraded | paused
    pub last_takeover_at_ms: Option<u64>,
    pub last_skip_reason: Option<String>,
    pub last_error: Option<String>,
    pub actual_debug_port: Option<u16>,
    pub restarts_last_hour: u32,
    pub restarts_total: u32,
    pub paused_instance: Option<String>,
}

pub fn start_supervisor();                                  // 开关打开 / 管理器启动时（幂等）
pub fn stop_supervisor();                                   // 仅退出时
pub fn set_enabled(enabled: bool);                          // 开关切换
pub fn pause_until_next_instance();                         // 「本次不接管」
pub fn take_over_now();                                     // 「立即接管」
pub fn state() -> AutoTakeoverState;
```

- 线程模型：专用 `std::thread` + `AtomicBool` 停止位，并遵守既有的 `SHUTDOWN_STARTED` / `APP_EXITING`（`lib.rs:13-21`）；探针用 `blocking` 系统调用即可，**不需要 tokio**（`is_macos_app_running` 是 async，本设计已不依赖它）。
- 「本次不接管」的**语义边界**（v1 含糊）：`pause_until_next_instance` 记 `InstanceKey = (pid, lstart)`，落盘到 `~/.codex-session-delete/auto-takeover-skip.json`；默认持续到**该实例结束**（不是管理器重启即失效）。三入口语义必须分开：`set_enabled(false)`（关功能）/ `pause_until_next_instance()`（跳过本次实例）/ `stop_supervisor()`（仅退出）。
- **开关落点**：保存钩子不在 `lib.rs`，而是 `commands.rs:2835 save_settings` 的 `Ok(())` 分支（`commands.rs:2881-2888` 已有「保存后同步运行态」的先例）；管理器启动兜底走 `lib.rs:99` 的 `resume_existing_codex_background`。
- **首次 tick 立即执行**（不要等 2 s），且首次 tick **不做宽限期检查**（宽限期只对「我们自己刚启动的实例」有意义）。
- **App 路径变更重绑定**：探针每 tick 重解析 `app_dir`（`resolve_codex_app_dir_with_saved`）；路径变化时重置状态机并记 `takeover.app_path_changed`。
- 唤醒处理：用 `Instant`（单调钟）+ 唤醒后丢弃首个 tick + 30 s CDP 恢复宽限。

### 3.10 平台门禁

守护线程与 `decide()` 必须带 `platform` 输入，**非 macOS 恒 `Skip`**：Windows 走 AUMID 激活路径（`launcher.rs:1026-1103`），语义完全不同；且 `find_codex_processes_for_debug_port` 在 Windows 退化成 `find_codex_processes()`，会让判据退化为「任何 Codex 在跑且端口无 CDP → 重启」。这条要有单测：`decide()` 在 `platform_supported == false` 时恒 `Skip`。

### 3.11 设置、命令面与 UI 落点

**设置字段**（照 `codexAppPasteFix` 写法，`settings.rs:531-532`）：

| # | 文件 | 动作 |
| --- | --- | --- |
| 1 | `settings.rs:500-719` | `#[serde(rename = "codexAppAutoTakeover", default)] pub codex_app_auto_takeover: bool`（默认 false） |
| 2 | `settings.rs:721-803` `impl Default` | 同步初始化（与 `#[serde(default)]` 两处必须一致） |
| 3 | `settings.rs:1619-1995` `merge_known_setting_fields` | `merge_bool_setting(target, source, "codexAppAutoTakeover")`——`/settings/set` 的键白名单在 `settings.rs:1494/1519`，不是 `routes.rs:435-437`（那里只是 `set_settings` 转发）。理由：与同族开关一致，并让任何走 `store.update` 的部分更新都能改它 |
| 4 | 同文件测试 | `settings_default_matches_expected_behavior`（`:2394`）补断言；仿 `:3376` 补 update 持久化用例 |
| 5 | 同结构 | `codexAppAutoTakeoverWhenActive`（`defer`/`immediate`）、`codexAppAutoTakeoverForceQuit`（bool，默认 false）、`codexAppAutoTakeoverQuitBudgetMs`（u64，默认 15000） |

**命令面**（照 `install_watcher` / `load_watcher_state` 模板，`commands.rs:5333-5370`）：

```rust
load_auto_takeover_state() -> CommandResult<AutoTakeoverPayload>
take_over_now() -> CommandResult<AutoTakeoverPayload>
pause_auto_takeover_for_instance() -> CommandResult<AutoTakeoverPayload>
```

注册到 `lib.rs:102-239` 的 `generate_handler!`（漏注册没有测试兜底——全仓 `generate_handler` 只出现一次，建议顺手加一条「命令名集合」断言）。

**UI 落点：维护页**（`MaintenanceScreen`，`App.tsx:7489`），与「开机打开 Codex++」并列新增卡：

- 标题 `自动接管官方 App`（**新 key**，不复用死键 `t("自动接管")`）；
- 状态行：`未开启 / 监听中 / 正在接管 / 已接管（增强未开启）/ 已暂停（原因）/ 已降级（原因）`；
- 明细行：实际调试端口、最后接管时间、上次跳过原因、1 小时内重启次数；
- 开关用维护页既有 `check-row`（`App.tsx:7530-7533`），不用 `FeatureToggle`；
- 两个按钮：`立即接管` / `本次不接管`；
- 未检测到登录项时提示「仅在 Codex++ 运行时生效」，并并列展示「开机打开 Codex++」卡与一键跳转（`open "x-apple.systempreferences:com.apple.LoginItems-Settings.extension"`）。

**为什么不放增强页**：`enhancement-settings.test.ts:193` 断言 `EnhanceScreen` **恰好 3 个 tabpanel**，`:99` 要求其中每个 `FeatureToggle` 的 `disabled` 必须是 `!masterEnabled`；且「接管」与「增强」正交。放维护页同时避开两条硬契约。

**增强关闭时的语义**：`enhancements_enabled=false` 时 `ensure_injection` 与 watchdog 都不跑（`launcher.rs:712`、`:743-745`），但仍会启 helper、落供应商配置与协议代理。此时状态行显示 **「已接管（增强未开启）」**，不显示「已接管」。

**i18n**：新 key 落 `EN_PLAIN`（`i18n-en.ts:7-1416`），Rust 消息落 `EN_BACKEND`（`:1557-1646`）/ `EN_BACKEND_PATTERNS`（`:1647-1721`）；顺手清理 3 个死键（`:467/:468/:1129`）。`tools/i18n-verify.mjs` 不是门禁（实测 `exit=1`，未接 CI）。

### 3.12 Windows 实现（2026-10-13 追加，已在 Windows 11 ARM64 真机实测）

**实测出来的三条硬事实**（都写进代码注释，避免后人踩）：

| # | 事实 | 验证方式 | 对实现的影响 |
| --- | --- | --- | --- |
| W1 | **AUMID 重激活不会把参数送达已有实例** | 在已运行的 Codex 上 `ActivateApplication(aumid, "…--remote-debugging-port=9229")`：主进程 pid 不变、9229 未监听 | 与 macOS 同构：**必须先结束再带参激活**，不能"补端口" |
| W2 | `taskkill /PID <pid> /T`（**不带** `/F`）杀不掉 GUI 应用 | 实测 11 个 ChatGPT 进程一个都没退，且返回失败信息 | 优雅退出改用 **`WM_CLOSE`**（`windows_integration::request_process_close`）；`/F /T` 只作兜底 |
| W3 | renderer / gpu / utility / **crashpad** 与主进程**同名同路径**，且会继承 `--remote-debugging-port` | `Get-CimInstance Win32_Process` 快照：9 个 `ChatGPT.exe` 里只有 1 个主进程；crashpad 还会再 fork 一个 crashpad | 判据必须排除**所有** `--type=` 子进程（只看父进程会漏掉 crashpad） |

**实现落点**：

- 探针：`auto_takeover::probe_desktop_app` 的 `#[cfg(windows)]` 分支 —— 枚举进程 → 同名同路径 → 排除 `--type=` 子进程 → 用进程启动时刻当实例代际。
- 退出：`request_desktop_exit` 的 Windows 分支走 `WM_CLOSE`，失败再 `taskkill /F /T`。
- 多开：`has_other_desktop_instance` 用「主实例 pid 集合 − 当前实例 pid − 带我们端口的 pid」，非空才算多开。
- 守护线程：不再按编译期平台门禁，改为 `cfg!(any(target_os = "macos", windows))`。

**真机验收**（Windows 11 ARM64 + 官方 Codex MSIX 26.1002.7124.0，2026-10-13）：

| 场景 | 结果 |
| --- | --- |
| 用户自己双击（裸启动，无调试端口） | `cdp_ready=false`、`other_instance=false` → **`takeover`** ✅ |
| Codex++ 带参启动后（有调试端口） | `cdp_ready=true` → **`wait(already_attached)`** ✅ |
| App 未运行 | `NotRunning` → **`wait(app_not_running)`** ✅ |

无 GUI 环境下的判据验证工具：`cargo run --example takeover_probe -- <app_dir> [port]`
（CI 与无人值守会话没有桌面，Tauri 起不来——实测连 notepad 都没有窗口句柄，所以判据必须能脱离 GUI 验证。）

**尚未在 Windows 上验证的部分**（需要带桌面的会话）：Tauri 管理器界面、`WM_CLOSE` 的实际退出耗时、重启后的注入链路。这三项待用户在虚拟机里跑一次完整流程。

### 3.12 诊断与可观测（验收完全依赖这一节）

新增事件族（`append_diagnostic_log`，仅状态变化时打，避免 2 s 一条刷日志）：

| 事件 | 时机 |
| --- | --- |
| `takeover.probe_decision` | 决策变化时（含 `decision` / `skip_reason` / `actual_port`） |
| `takeover.restart_requested` | 发起接管（**唯一计数口径**） |
| `takeover.deferred` | 因「疑似有人在用」推迟 |
| `takeover.quit_timeout` / `takeover.quit_escalated` | 退出超预算 / 升级到 SIGTERM |
| `takeover.ready` | 注入成功、状态写 `running` |
| `takeover.degraded` | 降级（含原因：`restart_budget_exhausted` / `already_degraded` / `probe_failed`） |
| `takeover.paused_for_instance` | 用户跳过本次 |
| `takeover.app_path_changed` | 绑定路径变化 |
| `takeover.probe_failed` | 探测失败（osascript 非 0 等） |

`origin` 字段：`status.rs` 的 `LaunchStatus` 加 `#[serde(default, skip_serializing_if = "Option::is_none")] pub origin: Option<String>`（旧文件兼容）；只由 `save_latest` 写，`save_phase`（读-改-写）里要保证不被冲掉。

---

## 四、实施顺序

> v1 的顺序是错的：SIGTERM 的目标 PID 只能来自第 3 步的新判据，所以「判据」必须排在「退出路径」之前。

**第 1 步：判据与状态机（纯逻辑，跨平台单测）**
- 新增 `crates/codex-plus-core/src/auto_takeover.rs`：`TakeoverInputs` / `TakeoverDecision` / `decide()` / `SessionSnapshot`（core 侧视图）/ `InstanceKey` / 退避与滑动窗口。
- 测试：`crates/codex-plus-core/tests/auto_takeover.rs`。必测：宽限期内不动作；宽限期打点在 `start()` 前；`cdp_ready` 不动作；未开启不动作；非 macOS 恒 `Skip`；多开实例 `Skip`；`Unknown` 探测 `Skip`；`running_degraded` → `Degraded` 且不重启；`StartupBusy` → `Wait`；暂停实例 `Wait`；滑动 1 小时闸门；唤醒后 30 s 宽限；端口漂移不触发第二次接管。

**第 2 步：进程判据与平台收紧（D2）**
- `watcher.rs`：新增 `pub fn find_macos_codex_desktop_main_processes() -> Vec<DesktopProcess>`（含 `pid/lstart/exe/bundle_id`），判据对齐 Windows 的思路（排除 `Contents/Resources/` 下的可执行文件），而不是只拉黑 `com.openai.codex.cli` 字符串。
- `launcher.rs:3815` 的 `is_macos_app_running` 改成三态并提升可见性（≥`pub(crate)`）；`probe_desktop_app` 落在 core。
- **测试位置**：`macos_codex_process_ids_with` 等是模块私有 + `cfg(any(target_os="macos", test))`，`ps` 快照测试必须写在 `src/watcher.rs` 内的 `#[cfg(test)]` 模块（`mod process_identity_tests` 附近），**不能**放 `tests/watcher.rs`（集成测试看不到私有项）。
- 独立提交：这一条修完，`runtime_health` 与 `provider_sync` 的误报同时消失。

**第 3 步：退出路径与假退出（D1/D3/D4）**
- 新增 `quit_macos_app_with_budget`（§3.7），替换 `launcher.rs:1127` 调用点并删除旧函数；`launcher.rs:1374` 的 `let _ =` 改为记日志。
- `wait_for_codex_exit` 叠加进程判据，修掉 `open -W` 假退出（D4）。
- 测试：注入式假实现测「预算内退出 / 超时 / 代际不匹配放弃 / osascript 非 0 原因可读」；`ExternalWaitCommand` 假退出用例。
- 注意：`FORCE_QUIT` 默认关，等 V1 结论。

**第 4 步：会话快照 API + 守护线程**
- `codex-plus-launcher`：加 `SessionSnapshot` 与 `session_snapshot()`（§3.6）。
- `LaunchOptions` 新增字段会破坏 **31 处穷尽结构体字面量**（`tests/launcher.rs` 26 处、`tests/bridge_routes.rs:1111`、`commands.rs:1535/2305/2322`、`main.rs:2293`）——这一步要显式列出并批量补 `..LaunchOptions::default()`。**或者干脆不加字段**：把「自动接管」上下文放在调用点而不是 `LaunchOptions`（推荐，零破坏）。
- `apps/codex-plus-manager/src-tauri/src/auto_takeover.rs`：探针线程 + 触发线程 + `LAUNCH_IN_PROGRESS` 复用 + 跳过态落盘 + 状态快照。

**第 5 步：设置、命令与前端**
- 按 §3.11 的 checklist 改 `settings.rs`、`commands.rs`（3 命令 + `save_settings` 钩子）、`lib.rs`（注册 + 启动兜底）、`App.tsx`、`i18n-en.ts`。
- **不要给 `EnhanceScreen` 加页签**；`App.tsx` 正在被他人改动，合并时按符号名核对，不要照搬行号。

**第 6 步：文档 + 构建启动（硬性要求）**
- `README.md:123-164`、`README_EN.md:42-53`/`:88-99`、`CHANGELOG.md:3-25`、`docs/release/v<下一版本>-notes.md`。
- `npm run vite:build` → `cargo build --release -p codex-plus-manager` → 装入 `dist/local-build-*/Codex++.app` → `codesign --force --deep --sign -` → `open`，确认进程与 `manager.start` 日志正常。
- 端到端实测（需用户在场）：关掉 Codex++ → 双击官方 App → 观察全流程与耗时；再验「本次不接管」与失败路径。

---

## 五、验收标准

| # | 场景 | 期望（可测口径） |
| --- | --- | --- |
| A1 | 开关关闭 | 无任何 `takeover.*` 事件；`latest-status.json` 的 `started_at_ms` 不变 |
| A2 | 开关打开，App 未运行 → 用户双击 | ≤5 s 内出现 `takeover.probe_decision`/`takeover.restart_requested`（发起）；`phase=ready` 出现，双击到 `ready` 的耗时落在实测分布内（目标 P95 ≤ 15 s，**不是 5 s**） |
| A3 | 开关打开，App 已带我们的端口运行且会话 ready | 判 `already_attached`；不重启；必要时走 `resume_if_running`（`main.rs:292-299`） |
| A4 | 开关打开，App 在跑但无端口 | 退出 + 带参重开 + 注入；同一 `started_at_ms` 代际内 `takeover.restart_requested` **恰好 1 次**（用事件族断言「没有第二次退出」，不用既有 `macos_existing_app_without_cdp_*` 事件名，那两个手动启动也会打） |
| A5 | 宽限期竞态 | `open` 之后 10 s 内不产生第二个 `takeover.restart_requested` |
| A6 | 退出失败 | 15 s 未退出 → `takeover.quit_timeout` + `Degraded` + 可读原因；滑动 1 小时内 `takeover.restart_requested` ≤6，第 7 次被拒且状态为 `Degraded` |
| A7 | 有任务在跑（`defer` 策略） | 跑一个 ≥60 s 的生成任务 → 双击无端口 App → 30 s 内无 `takeover.restart_requested`，状态行显示等待原因 |
| A8 | 点「本次不接管」 | 同一 `InstanceKey=(pid,lstart)` 生命周期内不再尝试（管理器重启后仍生效）；用户重启 App（新 lstart）后恢复 |
| A9 | 管理器未运行 | 不发生接管；`takeover.*` 事件为零（诚实表述：本功能仅在 Codex++ 运行时生效） |
| A9b | 开关开启但无登录项 | 维护页状态行显示「仅在 Codex++ 运行时生效」+ 一键跳转登录项设置；跳转返回后 ≤5 s 状态刷新 |
| A10 | 管理器重启（App 仍带我们的端口在跑、helper 已死） | **不立即 quit 重启**：先 `resume_if_running`（`lib.rs:99` 的启动兜底 → `commands.rs` `resume_existing_codex_background`），失败则提示；断言无 `takeover.restart_requested` |
| A11 | 存在多开实例 | 跳过并记 `Skip("multi_instance")`；多开实例不被 quit |
| A12 | 9229 被第三方占用 | 实际端口取自动换位后的值；不产生第二次接管 |
| A13 | 睡眠/唤醒 | 合盖 5 min 唤醒后 30 s 内无 `takeover.restart_requested` |
| A14 | 注入降级（`running_degraded`） | 状态行显示「已降级」而非「已接管」；不重启 App；提供「重试注入」入口 |
| A15 | 非 macOS 构建 | `decide()` 恒 `Skip`；Windows 的 AUMID 启动路径行为不变 |

---

## 六、风险、开放问题与待验证项

**实现前必须验证（V1–V3）**

| # | 待验证 | 方法 | 不利结论时的退路 |
| --- | --- | --- | --- |
| V1 | 官方 App 收到 quit / SIGTERM 时是否 flush 未落盘草稿 | 输入唯一标记（不发送）→ 等 5 s 本地 grep 状态文件 → `osascript quit` 后 grep → 重开再输入标记 → `kill -TERM` 后 grep。同时清点孤儿子进程 | 若 SIGTERM 丢数据：`ForceQuit` 保持默认关闭（本设计已是默认关） |
| V2 | 「疑似有人在用」的两个本地信号是否可用 | 跑一个 ≥60 s 任务，连续采样 `~/.codex/sessions/**/*.jsonl` 的 mtime/size 与 Codex 进程 CPU 时间增量 | 都不可用则 `defer` 退化为「仅手动触发」，UI 说明 |
| V3 | `osascript quit` 是否触发 TCC 自动化授权弹窗 | 首次在新签名/路径下调用时观察；并确认退出码能被读到（本设计已要求） | 弹窗则在状态行给指引；授权被拒归入 `probe_failed`/`quit` 原因 |
| V4 | 5 s 预算到底是不是 D1 的瓶颈 | 先修 D3 让原因可见，再统计「预算内退出 / 超时」比例 | 若超时均为「quit 未被受理」，加预算无用，应改走 ForceQuit opt-in |

**残余风险**

1. **重启打断**：无 CDP 时无法可靠判断「有任务在跑」，只能靠 V2 的保守信号 + `defer` 默认 + 首次确认。产品文案必须提前说明「开启后可能自动重启官方 App」。
2. **调试端口**：`--remote-debugging-port` 是本地回环、无鉴权的 CDP，本机任意进程可控制 App 渲染层。既有启动器已是这个模型，但自动接管把窗口从「用户主动启动期间」扩大到「App 运行的任意时刻」——设置说明必须写明，且默认关闭。
3. **D5（管理器退出 ⇒ App 失去 helper）**：既有问题，本设计只能通过 A10 的「先 resume」缓解；根治要把 helper 生命周期与 App 解耦，不在本期。
4. **D2 未修完前**：`runtime_health` 与 `provider_sync` 闸门仍会被孤儿 codex-cli 污染；第 2 步修完才消除。
5. **他人正在改 `App.tsx`/`commands.rs`/`i18n-en.ts`**：本设计的行号是快照，落地按符号名核对。

**开放问题（需要拍板）**

1. `defer` 作为默认策略是否接受？（即：疑似有人在用时**不**自动重启，等用户手动或空闲）
2. 登录项：本期接受「检测 + 引导」还是直接实现 LaunchAgent 写入？
3. 接管成功/失败是否发系统通知（`osascript display notification`，零依赖）？
4. `ForceQuit`（SIGTERM）是否保留为可选项，还是等 V1 之后再决定要不要做？
