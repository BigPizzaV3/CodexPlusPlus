# Computer Use 直通与 Codex 调用方式的一致性核对

- 日期：2026-10-25
- 对象：`CodexPlusPlus --codex-plus-cu-mcp`（原生直通，见 [直通设计稿](../plans/2026-10-23-codexpp-computer-use-native-passthrough-design.md)）
- 对照：本机 Codex CLI `0.163.0-alpha.11.2` 对应的开源代码（tag `rust-v0.163.0-alpha.11.2`）、
  `SkyComputerUseClient`（CUA 1001540）二进制字符串、`~/.codex/config.toml`、近期 Codex 会话记录
- 相关提交：`7a626928`（子进程环境白名单）、`faf50b38`（超长消息不再结束会话）

## 结论

工具面一致：直通的是同一个 `SkyComputerUseClient mcp`，字节不改。
不一致的是「Codex 怎么调用它」。逐项核对与实测后，**只有环境变量与超长消息两处值得改，已改**；
其余差异实测不影响使用效果，或属于产品选择，不做。

## 差异清单

| # | Codex | Codex++ 直通 | 处理 |
|---|---|---|---|
| 1 | `tools/call` 带 `_meta`：`callId` + `x-codex-turn-metadata`（session_id / thread_id / turn_id / codex_version / `user_input_requested_during_turn`）。源码 `core/src/mcp_tool_call.rs` `build_mcp_tool_call_request_meta`、`core/src/turn_metadata.rs` | 不带 | 不做，见实测 |
| 2 | 每轮结束 `notify = [SkyComputerUseClient, "turn-ended"]`，负载 `agent-turn-complete` | 没有 | 不做，见实测 |
| 3 | elicitation 默认弹卡；「始终允许」回 `_meta.persist:"always"`，写进原生持久授权；`approval_policy=never` 时按权限配置决定接受或拒绝（`core/src/session/mcp.rs`） | 默认 `auto` 全部放行；「弹卡确认」（`claude`）只在客户端声明 elicitation 时转发 | **已改**：转发的卡片加「始终允许」勾选项，勾上回 `_meta.persist:"always"`，由原生写进 `ComputerUseAppApprovals.json`（与 Codex 共用）；页面列出并可撤销。默认值仍为 `auto` |
| 4 | initialize：`codex-mcp-client` / title `Codex`，协议 `2025-06-18`，声明 elicitation 与 `openai/*` 扩展 | 透传客户端自己的 clientInfo，补 elicitation | 原生 client 不读 `openai/*` 扩展；未见影响，不做 |
| 5 | 环境只给 11 个白名单变量 + 非空的自定义 CA 变量（`rmcp-client/src/utils.rs` `create_env_for_mcp_server`） | 继承调用方全部环境（含 Claude `settings.json` 注入的 `ANTHROPIC_AUTH_TOKEN` 等） | **已改**（`7a626928`） |
| 6 | 进程链：签名的 `codex` 直接拉起 client，cwd `.` | 客户端 → Codex++ → 签名 node → shim → client | 签名校验通过，未见影响，不做 |
| 7 | — | 超过 8 MiB 的帧被当成 EOF，CU 回一条超长消息整个会话结束；客户端侧则卡住 | **已改**（`faf50b38`） |

另：Codex 26.1016 同时通过 `cua_repl` 的 `js`（`cua.getApp` / `cua.listApps` / `cua.computer.launch_app`）
操作桌面，会话记录里两条路径都在用。Codex++ 桌面面只给 10 个离散工具，浏览器单独一个 server。

## 实测一：子进程需要哪些环境变量

只读探针，签名 node 直接拉起原生 client，遇到授权请求回 `decline`：

| 环境 | `list_apps` | `get_app_state` |
|---|---|---|
| 只有 11 个白名单变量 | `-10005 codex app-server exited` | 同左 |
| 白名单 + `CODEX_CLI_PATH` | 正常 | 正常走到授权请求 |
| 再加 `CODEX_HOME` + `SKY_CUA_SERVICE_PATH` | 同上 | 同上 |

`CODEX_CLI_PATH` 必须；`CODEX_HOME` 只在 Codex 目录不在默认位置时需要；`SKY_CUA_SERVICE_PATH` 无作用。
Codex 不传 `CODEX_CLI_PATH` 也能用，推测是因为那边的直接父进程就是 `codex` 本身（未逐行核实）。

## 实测二：回合信息对 `get_app_state` 的影响

Finder，只读；授权回 `accept`（不带 `persist`，进程结束即失效）。

| 情况 | 结果 |
|---|---|
| 不带回合信息，连续 3 次 | 每次完整界面，1527 字符 |
| 同一会话，回合 A → A → B → B → C | 每次完整界面，1527 字符 |
| 换会话编号、有无回合信息混用 | 每次完整界面，1527 字符 |
| 完整环境重跑第一组 | 同上（排除环境白名单的影响） |
| 回合 C 发 `turn-ended` 后仍用 C | 被拒：`Computer Use is unavailable because the current turn ended. It will work again after the next user message` |
| 之后换回合 D | 恢复正常 |
| 授权请求 | 每个 client 进程只问一次，与回合、会话编号无关 |

现版本原生 CU 不再返回 diff / 「无变化」，回合信息不影响返回内容。Codex 自己的会话记录印证了这一点：

| Codex 会话 | `<app_state>` | diff | 无变化 |
|---|---|---|---|
| 2026-10-21T17-17 | 62 | 46 | 40 |
| 2026-10-23T20-11 | 10 | 0 | 0 |
| 2026-10-23T20-20 | 2 | 0 | 0 |

client 二进制 10-20 之后没有更新（一直是 1001540），推测 diff 是被远端开关关掉的，本地无法确认。

回合信息唯一可观察的作用是 `turn-ended` 之后拦截同一回合的调用。对 Claude 价值不大，
实现时回合编号递增出错还会把下一轮整个拦掉，所以不做。Claude Desktop / DeepSeek Harness /
ZCode / WorkBuddy 同理。

## 何时需要重新评估

- 原生 CU 重新返回 diff：缺回合边界时，长会话里模型可能拿到相对于已压缩上下文的 diff。
  届时优先给 Claude Code 补（已有 Stop / SubagentStop 钩子，`claude_hooks.rs`），其它客户端只能补会话编号。
- Codex 更新改变调用方式。设计稿 §7.2 的对照脚本 `scripts/cu-parity.mjs` 仍未建，本报告的两组实验可作为它的起点。
