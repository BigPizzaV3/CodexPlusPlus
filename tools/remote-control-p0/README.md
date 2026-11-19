# 远程控制 P0：协议参考实现与回归

验证「从另一个进程（将来是手机）操控 Codex 桌面」的全部链路，结论见
[docs/reports/2026-11-15-remote-control-research.md](../../docs/reports/2026-11-15-remote-control-research.md) 第 10 节。

| 文件 | 作用 |
|---|---|
| `ipc-client.mjs` | Codex 桌面 IPC 路由客户端（帧、握手、请求、广播、discovery） |
| `follower.mjs` | 跟随一个会话：应用快照 / 补丁维护本地状态；发消息、插话、中断、审批、回答提问 |
| `sidecar.mjs` | 侧车 app-server：读会话、建会话（建完立刻放掉写锁） |
| `ipc-versions.mjs` | 从已安装 Codex 的 `app.asar` 里读出 IPC 方法版本表 |
| `mock-responses.mjs` | 假的 Responses 上游，用标记触发审批 / 提问 / 慢速输出 |
| `run-p0.mjs` | 端到端回归，12 项（直接走 IPC，验证协议本身） |
| `run-p1.mjs` | 端到端回归，26 项（走 Rust 远程桥的 HTTP / WebSocket 接口，验证 P1 实现、会话设置与安全检查） |
| `run-p2.mjs` | 端到端回归，28 项（像手机一样走局域网 HTTPS：证书指纹、配对、权限分级、wss、吊销） |
| `run-p3.mjs` | 端到端回归，18 项（没在桌面打开的会话在后台执行，桌面当前页面全程不变；需 `P3_CDP_PORT`） |
| `run-p4.mjs` | 端到端回归，18 项（发图片 / 文件：上传、上游收到图片与文件引用、上传目录外路径被拒；新建会话的文件夹浏览） |
| `run-p5.mjs` | 端到端回归，12 项（桌面镜像：JPEG 画面帧、点击聚焦、中文整段输入、退格、Cmd+A、换行、非法输入被拒、只读设备被拒、局域网 wss） |
| `poc-headless-*.mjs` | 后台执行的协议实测：新建并跑一轮、接管已有会话 + 插话 + 中断 + 放锁、通知结构 |

P1 的正式实现在 `crates/codex-plus-core/src/remote_control/`，网页在
`crates/codex-plus-core/assets/remote-control/index.html`。

入口在 **Codex 左侧图标栏的「远程控制」**（`assets/inject/renderer-inject/45-remote-control-page.js`，
经 bridge 路由 `/remote-control` 调管理器的同一份分发）；管理器侧栏「工具 → 远程控制」保留，两边状态一致。
网页有两种对话样式（Codex 形态 / 按微信做的聊天形态），手机上默认聊天，样式按钮可切，按设备记住。

## 只在隔离实例上跑

所有步骤都用独立的 `CODEX_HOME`、独立的 `--user-data-dir`，不碰 `~/.codex` 和正在用的 Codex。
`run-p0.mjs` 在 `P0_HOME` 指向 `~/.codex` 时直接退出。

**不要用 `open codex://…` 测试**：macOS 会把链接交给默认的 Codex（也就是你正在用的那个）。

## 运行

```bash
APP=/path/to/ChatGPT.app            # 可以是解包出来的另一份，不必是已安装的
P0=/tmp/codex-p0
mkdir -p $P0/home $P0/userdata $P0/work

cat > $P0/home/config.toml <<EOF
model = "mock-model"
model_provider = "mock"
approval_policy = "on-request"
sandbox_mode = "read-only"

[model_providers.mock]
name = "Mock"
base_url = "http://127.0.0.1:18765/v1"
wire_api = "responses"
experimental_bearer_token = "sk-mock"

[projects."$P0/work"]
trust_level = "trusted"

[features]
default_mode_request_user_input = true   # 让模型在默认模式也能提问
EOF

# 1. 假上游
node mock-responses.mjs 18765 $P0/work &

# 2. 隔离桌面（清掉代理变量，开 CDP 供脚本让桌面打开会话）
env -u HTTPS_PROXY -u HTTP_PROXY -u ALL_PROXY CODEX_HOME=$P0/home \
  "$APP/Contents/MacOS/ChatGPT" --user-data-dir=$P0/userdata \
  --remote-debugging-port=9333 --remote-allow-origins=http://127.0.0.1:9333 &

# 3. 等主窗口出来后跑回归
P0_HOME=$P0/home P0_WORK=$P0/work \
P0_CODEX="$APP/Contents/Resources/codex-cli/bin/codex" \
node run-p0.mjs
```

预期输出：

```
✓ IPC 握手
✓ 侧车建会话后放掉写锁
✓ 侧车读会话不占写锁
✓ 新建会话并由桌面接管
✓ 跟随会话并收到快照
✓ 远程发消息并收到流式回复
✓ 远程同意命令
✓ 远程拒绝命令
✓ 运行中插话
✓ 中断
✓ 回答模型提问
✓ 侧车读取历史

12/12 通过
```

Codex 每次升级后跑一遍：任何一项失败说明 IPC 协议或页面消息变了。

### P1：Rust 远程桥

```bash
TOKEN=$(node -e 'console.log(require("crypto").randomBytes(32).toString("hex"))')
# 4. 在隔离实例上单独起远程桥（不经管理器，端口与令牌自定）
P1_HOME=$P0/home P1_APP="$APP" P1_CDP_PORT=9333 P1_PORT=57399 P1_TOKEN=$TOKEN \
  cargo run -p codex-plus-core --example remote_control_serve &
# 5. 回归
P1_PORT=57399 P1_TOKEN=$TOKEN P1_WORK=$P0/work node run-p1.mjs
# 浏览器手动验证：http://127.0.0.1:57399/#token=$TOKEN
```

### P2：局域网 HTTPS 与配对设备

```bash
# 远程桥加开局域网入口；状态文件（设备表、证书）放隔离目录。
# example 代替管理器按 P1_PAIR_ROLES 依次发配对码，写到 $P0/rc-state/pairing-code.txt；
# 往 $P0/rc-state/revoke.txt 写设备 ID 即吊销。
mkdir -p $P0/rc-state
P1_HOME=$P0/home P1_APP="$APP" P1_CDP_PORT=9333 P1_PORT=57399 P1_TOKEN=$TOKEN \
P1_LAN_PORT=57398 P1_STATE=$P0/rc-state/remote-control.json P1_PAIR_ROLES=view,full \
  cargo run -p codex-plus-core --example remote_control_serve &
# 证书自签：关掉 Node 默认校验，脚本自己按状态文件核对指纹
NODE_TLS_REJECT_UNAUTHORIZED=0 P1_PORT=57399 P1_TOKEN=$TOKEN P2_LAN_PORT=57398 \
  P2_DIR=$P0/rc-state P1_WORK=$P0/work node run-p2.mjs
# 手机手动验证：同一 Wi-Fi 下打开 https://<电脑局域网 IP>:57398/#pair=<配对码>
```
另外用 `node ipc-versions.mjs <app.asar>` 看方法版本有没有变。

## 已知限制

- 只在 macOS 上验证过。Windows 的 IPC 是命名管道 `\\.\pipe\codex-ipc`，`defaultSocketPath()` 已按此返回，未实测。
- 让桌面打开会话这一步在这里用 CDP 发页面消息；正式实现由 Codex++ 的注入脚本发同一条消息。
- 文件改动审批（`thread-follower-file-approval-decision`）、权限请求审批、MCP 表单回答已在协议里确认存在，
  假上游还没有触发它们的场景。
