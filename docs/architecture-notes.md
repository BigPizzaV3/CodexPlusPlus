# Codex++ 架构笔记

> 本文记录 Codex++ 核心机制的源码走读结论,聚焦"注入链路"与"协议适配"两条主线。
> 行号引用基于走读时的 `main` 分支,源码变动后可能漂移,以函数名为准。

## 总览

Codex++ 不修改 Codex App 本体(不动 `app.asar`、不写 DLL),而是:

1. 用外部 launcher 带 `--remote-debugging-port` 启动 Codex(Electron),开启 Chromium DevTools Protocol(CDP)。
2. 通过 CDP 把增强脚本 `assets/inject/renderer-inject.js` 注入渲染进程。
3. 注入脚本经 CDP Runtime Binding 这条桥与 Rust 后端通信,完成删除/导出/中转切换等需要后端能力的操作。
4. 中转模式下,本地起一个协议代理,把 Codex 的 Responses API 流量翻译成各类中转站支持的 Chat Completions。

```
launcher ──open/spawn(--remote-debugging-port)──▶ Codex(Electron + CDP)
   │                                                      ▲
   │  CDP: list_targets / addBinding / addScriptToNewDoc  │ 注入 renderer-inject.js
   ▼                                                      │
Rust 后端 ◀── Runtime Binding 桥(codexSessionDeleteV2)──┘
   │
   └─(中转模式)protocol_proxy: Responses ⇄ Chat Completions ──▶ 第三方中转站
```

---

## 1. CDP 注入(`crates/codex-plus-core/src/cdp.rs`)

- `list_targets(debug_port)`:请求 `http://127.0.0.1:<port>/json`(同时尝试 IPv4/IPv6 loopback),拿到所有 CDP target。
- `pick_page_target()`:筛 `type == "page"` 且有 `webSocketDebuggerUrl` 的目标,优先标题/URL 含 "codex" 的页面,否则取第一个 page。
- 拿到 page 的 WebSocket 调试地址后,交给 `bridge.rs` 注入脚本。

---

## 2. Bridge:前端 ⇄ 后端通信(`bridge.rs` + `routes.rs`)

主通道不是 HTTP,而是 **CDP Runtime Binding**(`BRIDGE_BINDING_NAME = "codexSessionDeleteV2"`)。

**建桥**(`install_bridge`,bridge.rs:107):
```
Runtime.enable
Runtime.addBinding { name: "codexSessionDeleteV2" }      // 注册原生绑定
Page.addScriptToEvaluateOnNewDocument(bridge_script)     // 每个新文档自动注入包装层(页面刷新也生效)
```

**前端 → 后端**:`build_bridge_script`(bridge.rs:27)在 `window` 上造 Promise 化 API:
```js
window.__codexSessionDeleteBridge = (path, payload) => new Promise(resolve => {
  const id = String(++seq);
  callbacks.set(id, { resolve });
  window.codexSessionDeleteV2(JSON.stringify({ id, path, payload }));  // 调原生 binding → CDP 事件回 Rust
});
```

**后端 → 前端**:Rust 处理完用 `Runtime.evaluate` 注入 `window.__codexSessionDeleteResolve(id, result)` 把结果塞回对应 Promise(`resolve_bridge_expression`,bridge.rs:187)。`CdpSession` 用 message-id 配对、`binding_calls` 队列在后台 `tokio::spawn` 循环里排空。

**路由表**(`handle_bridge_request`,routes.rs:96):一个 `match path` 大分发,按职责拆成三个 trait:

| 前缀 | trait | 例子 |
|------|------|------|
| `/settings/*` | `BridgeSettingsService` | get/set |
| `/user-scripts/*`、`/devtools/*`、`/backend/*`、`/zed-remote/*`、`/upstream-worktree/*`、`/ads` | `BridgeRuntimeService` | 脚本启停、修复后端、Zed 远程、worktree |
| `/delete`、`/undo`、`/export-markdown`、`/thread-usage-history`、`/move-thread-workspace`、`/thread-sort-key(s)` | `BridgeDataService` | 落盘类操作 |

每个请求前后写 `diagnostic_log`(path / payload keys / 耗时 / status);`/backend/status` 兼做健康检查(`bridge_health_check_script`,bridge.rs:55)。

---

## 3. 注入脚本如何 hook React 界面(`assets/inject/renderer-inject.js`,约 7600 行)

脚本完全寄生在 Codex 的 DOM 上,不碰 React 内部 API:

1. **MutationObserver 持续挂载**:监听 `document.body` 的 `childList + subtree`,每次 DOM 变动重新扫描会话行并补按钮。按钮带版本号 `dataset`(如 `codexDeleteVersion`)避免重复挂载。
2. **反查 React Fiber 拿数据**:找节点上 `__reactFiber$` / `__reactInternalInstance$` 私有 key,顺 fiber 链往上爬最多 20 层,从 `memoizedProps` 抠 `archivedThread.id`(renderer-inject.js:6121)。拿不到则回退 DOM 文本解析。
3. **拿到 session 后调 bridge** 完成后端操作。

> 脆弱点:依赖 Codex 的 class 名与 DOM 结构,Codex 改版可能失效,因此脚本里有大量 `querySelector`(150+ 处)+ 版本号兜底。

---

## 4. 协议代理(`protocol_proxy.rs`,约 3650 行)

Codex 只说 OpenAI **Responses API**;多数中转站只支持 **Chat Completions**。代理在 `127.0.0.1:57321/v1` 起一个假 Responses 服务器(`local_responses_proxy_base_url`),做双向翻译。中转配置里 `codex_base_url_for_protocol`(relay_config.rs)在 ChatCompletions 协议下把 Codex 的 base_url 指到这个本地端口。

### 4.1 请求方向:Responses → Chat(`responses_to_chat_completions`:127)

- `instructions` → 开头的 system 消息;`input` → messages(`append_responses_input`)。
- `max_output_tokens` → `max_tokens`(o 系列用 `max_completion_tokens`)。
- 流式时强制 `stream_options.include_usage = true`,否则拿不到 token 用量。
- `collapse_system_messages_to_head`:把散落的 system 消息归并到最前(部分后端不接受中间 system 消息)。
- `tools` → Chat function 工具(含 apply_patch 特殊处理,见 4.4)。

### 4.2 历史回放:扁平 item → 嵌套消息(`append_responses_input`:1492)

Responses 历史是扁平 typed item 列表;Chat 要求 `assistant.tool_calls` + `role:tool` 的嵌套结构。用两个挂起缓冲做延迟合并:

- `pending_reasoning` / `pending_tool_calls`:把先后到达的 `reasoning` + `function_call` 合并进同一条 assistant 消息(`flush_tool_calls`:1775)。
- `seen_tool_call_ids`:检测孤儿工具结果(对应 call 从未出现),降级成 user 消息 `"Function call output (id): ..."`(`orphan_tool_output_message`:1706),避免严格后端拒绝。
- 兼容多厂商方言:OpenAI `function_call`、Anthropic `tool_use`/`tool_result` 都归一到 Chat 格式。
- 遍历结束后必须再 flush 一次,否则历史最后一轮的挂起项会丢。

### 4.3 返回方向(流式):Chat SSE → Responses SSE(`ChatSseState`:713)

把扁平的 Chat delta 流重组成 Responses 要求的严格事件序列:

- 首个 chunk 凭空补 `response.created` + `response.in_progress`(`ensure_response_started_into`:890)。
- 三类 output item(text / reasoning / tool_call)各走 `output_item.added → delta → output_item.done`,用 `next_output_index` 分配序号。
- 正文首 token 到达时强制收尾 reasoning(`finalize_reasoning_into`:1136)——UI 思考块据此先于正文定型。
- 工具调用按 `index` 在 `BTreeMap<usize, ToolCallState>` 累积分片(id/name/arguments 分多 chunk)。
- 内联 `<think>` 思维链:三态机 `InlineThinkMode`(Detecting/Reasoning/Text),识别 DeepSeek R1 类把 CoT 塞进 content 的情况(`leading_think_prefix_decision`:1378)。
- UTF-8 安全分块(`append_utf8_safe`)处理多字节字符跨 chunk 边界。
- usage 重映射:`prompt/completion_tokens` → `input/output_tokens`,`reasoning_tokens` 进 `output_tokens_details`。
- 错误 → `response.failed`(而非 `completed`),且不发 `[DONE]`。

### 4.4 apply_patch 编解码

Codex 的 `apply_patch` 是 freeform 工具,参数是纯文本 patch(`*** Begin Patch ... *** End Patch`),多数中转/模型不支持。代理做了 JSON ⇄ 文本编解码器:

- **请求方向**:一个 `apply_patch` custom 工具 → 5 个标准 JSON function 工具(`apply_patch_proxy_tools`:2287):`_add_file` / `_delete_file` / `_update_file` / `_replace_file` / `_batch`。`update_file` 的 `hunks` schema 把 `@@`/`+`/`-`/空格行建模成 `{op, text}` 结构化数组,模型只填 JSON。
- **返回方向**:模型回的 JSON 工具调用 → 还原成 patch 文本。`proxy_action_from_upstream_name`(:2457)按后缀识别 action,`reconstruct_apply_patch_input`(:3142)+ `build_apply_patch_text`(:3187)拼文本(`line_op_prefix`:add→`+`/remove→`-`)。若模型直接吐了 `raw_patch` 则原样透传;`replace_file` 展开成 Delete+Add 两步。
- **历史方向**:`parse_apply_patch_operations`(:3265)是逐行状态机,把 patch 文本反解析回 operations;`build_custom_tool_call_history`(:3096)单操作→对应工具、多操作→`_batch`(附 `raw_patch` 保险)。
- `parse_*` 与 `build_*` 互为逆函数,三个方向共用 `{type, path, content/hunks}` 中间表示。

### 4.5 URL 鲁棒性

`is_*_proxy_path` 容忍 `/responses`、`/v1/responses`、`/v1/v1/responses`、`/codex/v1/responses` 等变体;`chat_completions_url`(:583)处理末尾 `#`(跳过 `/v1` 前缀)、自动塌缩 `/v1/v1` → `/v1`。

---

## 5. macOS Launcher(`launcher.rs`)

### 5.1 启动

`launch_codex`(:444)按形态分叉:Windows packaged app、macOS `.app`、裸可执行文件。macOS 走 `.app` 分支。

调试端口参数(`build_codex_arguments`:1144):
```
--remote-debugging-port=<port>            // 默认 9229,开启 CDP
--remote-allow-origins=http://127.0.0.1:<port>  // 新版 Chromium 必须放行 origin,否则 WS 握手被拒
```

macOS 用系统 `open` 而非直接 spawn(`build_macos_open_command`:1291):
```
open -W -a /Applications/Codex.app --args --remote-debugging-port=9229 --remote-allow-origins=...
```
- `-a`:经 LaunchServices 正常激活;`--args`:之后参数透传给 Codex;`-W`:阻塞到 app 退出。
- `open` 是短命中间进程,真正的 Codex 不是其子进程。launcher 追踪的 `child` 是 `open`,`wait_strategy = ExternalWaitCommand`。

### 5.2 退出清理(`MacosCleanupPolicy`)

启动前 `is_macos_app_running`(:1353)用 `osascript` 问 `application "Codex" is running`:
- 本来没开 → `QuitIfNotPreviouslyRunning`:退出时帮用户关。
- 本来就开 → `SkipQuitBecauseAlreadyRunning`:不擅自关用户自己的实例。

`terminate_codex`(:581)先 `child.kill()`(杀 `open`),再按 policy `run_macos_cleanup_command` 发 `osascript -e 'tell application "Codex" to quit'`(:1307)。

### 5.3 注入生命周期

`ensure_injection`(:152)轮询最多 120 次、间隔 1s(因 `open` 返回后 Electron 页面未必 ready,CDP target 可能查不到):
- 失败写 `launcher.ensure_injection_retry_failed` 诊断日志。
- 超时不杀 Codex,标 `running_degraded`(原版 Codex 仍可用),由 `keep_launched_on_error` 控制。
- 注入成功后 `start_bridge_watchdog`:每 5s `check_and_reinject_bridge`(:1189),页面导航导致 binding 掉了自动补回。

---

## 6. GitHub Release 自动更新(`update.rs`)

### 6.1 数据源:`latest.json` 静态跳转

走 Release 附带的静态文件而非 GitHub API(update.rs:6):
```
https://github.com/BigPizzaV3/CodexPlusPlus/releases/latest/download/latest.json
```
`/releases/latest/download/<asset>` 永远指向最新 Release 的同名 asset。好处:不吃 GitHub API 速率限制(未认证 60 次/时)、纯静态 CDN 快且稳、格式可控。

两套解析器并存且字段做 `or_else` 容错:
- `release_from_github_payload`(:70):标准 API 的 `tag_name` / `assets[].browser_download_url`。
- `release_from_latest_json_payload`(:106):自定义 `version`(↔`tag_name`)/ `assets[].url`(↔`browser_download_url`)/ `body`(↔`release_summary`↔`notes`)。

### 6.2 版本比较:数值化分段(`is_newer_version`:61)

`parse_version_tag`(:42)去前缀 `v`/`V`,只取开头连续数字与点(遇 `-beta` 等后缀即停),`"v1.2.10"` → `[1,2,10]`;比较时两边 `resize` 补 0 后按 `Vec<u64>` 比。**数值比较而非字符串**,避免 `"10" < "9"` 误判。

### 6.3 平台 asset 选择(`select_update_asset`:149 / `platform_asset_rank`:252)

用 `cfg!` 编译期分平台:
- Windows:含 `codex`+`plus` 且 `.msi`/`-setup.exe`/`setup.exe`/`installer.exe` 结尾。
- macOS:含 `codex`+`plus` 且 `.dmg` 结尾。

rank `0`=命中本平台直接选,其余忽略。一个 Release 挂多平台多架构包,各平台只下自己那个。

### 6.4 下载与安装(`perform_update`:193)

1. `proxied_client`(UA `Codex++/<version>`,走系统代理)下载 asset。
2. `download_asset_to`(:218)写下载目录;文件名经 `safe_asset_name`(:234)做**路径穿越防护**(`path.components().count() != 1` 即拒,挡 `../`/绝对路径)。
3. `launch_installer`(:276):Windows spawn 安装器(`CREATE_NO_WINDOW`),macOS `open <dmg>`,其他平台 bail。

不做原地自更新,而是下载官方安装包交系统安装,规避签名/权限/占用问题。

### 6.5 接入点

- 静默 launcher(`apps/codex-plus-launcher/src/main.rs:167` `notify_manager_when_update_available`):启动 Codex 后顺手检查,有新版则用 `--show-update` 拉起管理工具弹提示(自身不弹 UI)。
- 管理工具(`apps/codex-plus-manager/src-tauri/src/commands.rs:982`):Tauri command 暴露给前端,用户手动检查/更新。

两边都以 `codex_plus_core::version::VERSION` 为当前版本基准。

---

## 7. Zed Remote 打开(`zed_remote.rs` + `zed_remote/fallback.rs`)

让会话界面识别当前文件所在的远程 SSH 上下文,用 Zed Remote Development 打开。对应 bridge 路由 `/zed-remote/{status,resolve-host,fallback-request,open}`。

### 7.1 四步链路

1. **探测 Zed**(`zed_remote_status`:56):`find_zed_app_path`(:30,扫 `/Applications/Zed*.app` 与 `~/Applications/`)+ `find_zed_cli_path`(PATH 上找 `zed`,Windows 加 `.exe`)。返回 `platformSupported/zedAppFound/zedCliFound/路径`。
2. **解析 SSH target**:
   - `target_from_payload`(:204):前端已知 ssh 信息,字段多别名兜底(`host`/`hostname`/`hostName`、`user`/`username`)。
   - `resolve_ssh_target_for_host_id`(:364):读 `~/.codex/.codex-global-state.json`(`codex_global_state_path`:306,尊重 `CODEX_HOME`),在 `codex-managed-remote-connections` 按 `hostId` 匹配 `sshHost`/`sshUser`/`sshPort`(`target_from_managed_remote_connection`:314)。
   - `split_ssh_authority`(:103):拆 `user@host:port`,正确处理 IPv6 `[::1]:22` 与单冒号端口歧义。
3. **构造 URL**(`build_zed_remote_url`:243):`ssh://user@host:port/percent/encoded/path`。`encode_remote_path`(:229)强制绝对路径并逐段 `percent_encode_segment`(:267,只留 `A-Za-z0-9-._~`)。
4. **拉起 Zed**(`launch_zed_url`:280):macOS `open -a Zed.app <url>`,否则 CLI `zed <url>`,都没有则报错。

### 7.2 Fallback:从会话反查工作区/文件(`fallback.rs`)

前端只知 thread、不知路径/host 时,多源回退:

- `workspace_root_from_sqlite`(fallback.rs:101):查 Codex 的 `state_5.sqlite`,`SELECT cwd FROM threads WHERE id = ?1`(`rusqlite`)拿会话工作目录。
- `thread-workspace-root-hints`(:133):按 thread_id 查 hint(支持 `local:` 前缀变体)。
- `host_id_for_remote_path`(:158):未指定 host 时遍历远程项目,用 `project_path_matches`(前缀 + 边界检查,避免 `/foo` 误配 `/foobar`)定位 host。
- 优先级(`fallback_open_request_from_global_state_with_context`:196):显式 `workspace_root` → hint → SQLite。

### 7.3 安全

`validate_ssh_host`(:176)拒绝控制字符/空白/`/?#@`,IPv6 必须成对中括号且可 parse `Ipv6Addr`(host 要拼进 `ssh://` URL 并交系统命令,防注入);端口拒绝 0 与越界。与 `safe_asset_name` 同属"外部数据先校验"。git 历史 `fix zed remote host id validation` 即此处。

---

## 8. Upstream Worktree 创建(`upstream_worktree/`)

从 `upstream/<base-branch>` 创建 git worktree,创建前先 fetch 远端分支,避免从陈旧本地 HEAD 派生导致冲突。对应 bridge 路由 `/upstream-worktree/{status,defaults,prepare,create}`。

### 8.1 模块拆分(`upstream_worktree.rs` 仅门面)

| 子模块 | 职责 |
|---|---|
| `types.rs` | 结构化错误码 `UpstreamWorktreeCode`(10 种)、请求/结果类型 |
| `git.rs` | git 命令封装、payload 解析、输入校验 |
| `defaults.rs` | 探测默认值(当前分支/remote/upstream refs/已有 worktree) |
| `create.rs` | prepare/create 核心流程 |
| `remote.rs` | 远程 SSH 项目:在远端机器跑 git |

### 8.2 本地创建流程(`create_worktree`,create.rs:177)

逐步前置检查,任一失败返回带 code 的结构化错误:

1. `repo_root`:`git -C <path> rev-parse --show-toplevel`。
2. `ensure_remote_exists`:`git remote` 列表含目标 remote。
3. `ensure_branch_is_available`:`git show-ref --verify --quiet refs/heads/<branch>`,存在报 `branch-exists`。
4. `ensure_worktree_path_available`:目标路径未占用(`normalize_worktree_path` 相对路径拼到 root)。
5. **fetch(关键)**:`git fetch <remote> +refs/heads/<base>:refs/remotes/<remote>/<base>` —— 强制 refspec(`+` 前缀)先更新远端跟踪分支,这是"不从陈旧 HEAD 派生"的核心。
6. `ensure_source_ref_exists`:`git rev-parse --verify refs/remotes/<remote>/<base>^{commit}` 确认源 ref 并拿 head。
7. `add_worktree`:`git -C <root> worktree add -b <branch> <path> refs/remotes/<remote>/<base>`。

`prepare_response`(create.rs:147)是"干跑":只做到第 6 步,返回 `sourceRef`/`qualifiedSourceRef`/`sourceHead` 供前端预览。

### 8.3 defaults(defaults.rs:149)

`current_branch`(`git branch --show-current`)作默认 base(空则 `main`);`default_remote_name` 优先级 **upstream > origin > 第一个**(:11);`upstream_refs`(`git for-each-ref refs/remotes/<remote>`)列分支;`worktree_branches`(`git worktree list --porcelain`)列已有 worktree。

### 8.4 防 git 参数注入

`validate_branch_name`(git.rs:12):拒绝空/`-` 开头/含 `\`,再用 `git check-ref-format --branch` 判合法;remote 名(git.rs:97)拒绝 `-` 开头与 `/`、`\`;所有调用走 `git_in_repo`(`-C <repo>` 显式指定仓库,不依赖 cwd)。与第 6/7 章"外部输入先校验"一脉相承。

### 8.5 远程 SSH 项目(remote.rs)

`projectId` 命中 `~/.codex/.codex-global-state.json` 的 `remote-projects` 时,git 操作在远端执行:

1. `remote_project_for_id`(:67):读 global-state 取 `hostId`+`remotePath`(要求绝对路径)。
2. **复用 `zed_remote::resolve_ssh_target_for_host_id`**(:14)解析 SSH target——与第 7 章同一函数。
3. `remote_git_command`(:83):拼 `git -C <remotePath> <args>`,每个参数 `shell_quote`(:71)防 shell 注入。
4. `spawn_remote_git`(:102):`ssh -o BatchMode=yes -o ConnectTimeout=8 [-p port] <dest> "<command>"`。

本地/远程在 `create_worktree`/`prepare_source_ref` 入口用 `remote_project_for_id().is_some()` 分流(create.rs:179),逻辑对称。

---

## 9. 中转配置与 Profile 切换(`relay_config.rs` + `settings.rs`)

第 4 章讲"中转模式下怎么翻译协议",本章讲"怎么落到 Codex 的 `~/.codex/config.toml` 与 `auth.json` 上"。Codex 本身只读这两个文件;Codex++ 的全部中转能力,最终都收敛成"**安全地改写这两个文件**"。`relay_config.rs`(约 2077 行)就是这个改写器。

### 9.1 两个正交维度:模式 × 协议

配置由两个独立维度决定,组合出全部形态:

**模式 `RelayMode`**(settings.rs:134),决定鉴权落点:

| 模式 | 鉴权放哪 | auth.json | config.toml |
|---|---|---|---|
| `Official` | ChatGPT 登录直连官方 | 保留 ChatGPT tokens,**去掉** `OPENAI_API_KEY` | 无 custom provider(除非开 `officialMixApiKey` 叠加) |
| `MixedApi`(默认) | 中转 provider,key 进 config | 保留 ChatGPT 登录态(去 `OPENAI_API_KEY`),满足 `requires_openai_auth` 门槛 | `experimental_bearer_token` = 中转 key |
| `PureApi` | 纯 API key | 写 `OPENAI_API_KEY` = 中转 key | provider 里**移除** `experimental_bearer_token` |

"混合(MixedApi)"是这里的精髓:**既保留 ChatGPT 登录态、又把流量导向中转 provider**——`requires_openai_auth = true` 的门槛靠 auth.json 里的 ChatGPT tokens 过,实际 key 则藏在 config 的 `experimental_bearer_token`。

**协议 `RelayProtocol`**(settings.rs:126),决定 base_url 指向(`codex_base_url_for_protocol`:510):

- `Responses`:provider `base_url` 直接指中转站(透传)。
- `ChatCompletions`:provider `base_url` 指向**本地协议代理** `local_responses_proxy_base_url(port)`,真实上游存进 `upstream_base_url` / `codex_plus_chat_base_url`(:13)。这正是第 4 章那个本地假 Responses 服务器的接线点——两章在 `codex_base_url_for_protocol` 这一个函数上闭合。

### 9.2 Profile:一套可命名、可切换的中转配置

`RelayProfile`(settings.rs:41)是 UI 里一行"中转配置",字段含 `relay_mode`/`protocol`/`upstream_base_url`/`api_key`/`config_contents`/`auth_contents`/`context_selection`/`context_window` 等。多 profile 即多套并存、随时切换。

切换落盘的主入口 `apply_relay_profile_to_home_with_switch_rules`(:356)是一条四步流水:

1. **筛 common**:`filter_common_config_for_selection`(:734)按本 profile 的 `context_selection`(选了哪些 mcp/skills/plugins)裁出共享配置子集(`use_common_config` 为假则跳过)。
2. **补全 provider**:`complete_relay_profile_config`(:1502)把 profile 拼成完整 config.toml——定 `model_provider` id(保留自定义 id,缺省回退 `"custom"`:11)、`retain_only_provider_table` 只留当前 provider(连带删 `LEGACY_RELAY_PROVIDERS`:12 的 `CodexPlusPlus`/`CodexPP` 残留)、补 `wire_api="responses"`/`requires_openai_auth=true`、按协议写 `base_url`、按模式写/删 `experimental_bearer_token`。
3. **合并 + 限额**:`merge_common_config_into_config`(:665)并入 common;`apply_context_limits_to_config`(:1080)把 `context_window`/`auto_compact_limit` 写成 `model_context_window` / `model_auto_compact_token_limit`。
4. **按模式定 auth 后落盘**(:374):`PureApi` 直接写 `profile.auth_contents`;`Official`/`MixedApi` 走 `official_profile_auth_for_switch`(:1429)→ `remove_openai_api_key_from_auth_contents`,**只摘 `OPENAI_API_KEY`、保留 ChatGPT 登录的其余字段**,再落盘。

### 9.3 Common Config:跨 profile 共享的那部分

切来切去不该丢掉 MCP servers、skills、plugins 这些通用设置。所以它们抽到独立的 **common config**,在落盘时才并进当前 profile:

- `extract_common_config_from_config`(:619):反向抽取——剥掉 `model`/`model_provider`/`base_url`/`model_providers` 等 profile 私有键,剩下的就是共享部分。
- `sanitize_common_config_contents`(:634):再去掉 provider 专属键,确保 common 干净。
- `merge_common_config_into_config`(:665)/ `strip_common_config_from_config`(:644):TOML 级合并/剥离,基于 `toml_value_is_subset`(:1095)做"值匹配才剥离",避免误删用户手改的同名键。
- 上下文条目增删查:`{list,upsert,delete}_context_entry_*`(:681/693/717),按 `mcp_servers`/`skills`/`plugins` 三表管理,`sync_live_config_context_entries`(:744)据 enabled 态开关写进活动 config。

### 9.4 原子写 + 备份 + 校验(`write_codex_live_atomic`:788)

改的是用户正在用的活动配置,出错即 Codex 起不来,所以写入有三重保护:

1. **先校验**:`validate_toml_config`(:1047)/ `validate_auth_json`(:1057) 解析通过才继续,坏内容绝不落盘。
2. **先备份**:`create_live_backup`(:1783)把旧 config.toml/auth.json 打包成带时间戳的备份(`RelayApplyResult.backup_path` 回传给 UI 作撤销点)。
3. **写 + 回滚**:`atomic_write` 先写 auth 再写 config;若 config 写失败而 auth 已写,`restore_optional_file` 把两个文件都还原回旧字节——不留半套坏状态。

### 9.5 反向:回填与清除

- **回填** `backfill_relay_profile_from_home`(:579):把当前活动的 config.toml/auth.json 反读回 `RelayProfile`(恢复 provider id、`sync_profile_mode_from_backfilled_live` 推断模式、抽出 common),让 UI 永远反映 Codex 真实在用的配置,而非陈旧缓存。
- **清除** `clear_relay_config_to_home_with_auth`(:523):删掉 `model_providers.custom`(+ legacy)、抹掉 `model_provider`/`base_url` 等根键、按需清 `OPENAI_API_KEY`,把 Codex 还原成"无中转"。

### 9.6 ChatGPT 账号识别(`auth_json_chatgpt_account_label`:1853)

UI 要显示"当前登录的是哪个 ChatGPT 账号":读 auth.json,确认 `auth_mode == "chatgpt"` 且 tokens 含登录密钥,再 `account_label_from_jwt`(:1893)——base64url 解 `id_token`/`access_token` 的 JWT payload,取 `email`(或 `.../profile.email`、`name`)。纯本地解码,不发网络请求。

### 9.7 接入点(注意:走 command,不走 bridge)

与第 7/8 章经 CDP bridge 不同,中转配置由**管理工具的 Tauri command** 直接调用(`apps/codex-plus-manager/src-tauri/src/commands.rs`):`apply_relay_profile_to_home_with_switch_rules`(commands.rs:1538/1643)、`apply_relay_config_to_home_with_protocol`(:1590)、`apply_pure_api_config_to_home_with_protocol`(:1685)、`backfill_*`(:1237)、`clear_*`(:1740)。原因:改的是文件系统而非 Codex 页面 DOM,无需注入脚本参与。

### 9.8 安全 / 鲁棒

- `RESERVED_MODEL_PROVIDER_IDS`(:14):自定义 provider id 不得撞 Codex 内建(openai 等),`is_custom_provider_id`(:845)把关,防覆盖内建 provider。
- `move_model_providers_before_profiles`(:1819):重排 TOML,保证 `[model_providers.*]` 在 `[profiles.*]` 之前,避免 toml 解析歧义。
- 落盘前一律 validate + backup + atomic(9.4),与第 6/7/8 章"外部输入先校验、危险操作可回滚"同源。

---

## 走读验证

- `protocol_proxy` 的转换逻辑有完整单测:`crates/codex-plus-core/tests/protocol_proxy.rs`(35 个 test,覆盖请求/响应/SSE/内联 think/UTF-8 边界/apply_patch/URL 归一化)。`cargo test -p codex-plus-core --test protocol_proxy` 全绿。
- `relay_config` 的模式/协议/common/回填/清除逻辑有 `crates/codex-plus-core/tests/relay_config.rs`(67 个 test,覆盖三模式落盘、common 合并/剥离、上下文增删、限额写入、回填恢复 provider id、清除还原)。`cargo test -p codex-plus-core --test relay_config` 走读时实跑 **67 passed; 0 failed**。
