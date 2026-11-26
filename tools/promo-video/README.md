# Codex++ 宣传片

2 分钟、1920×1080、30fps 的宣传片，画面和配乐都由这里的脚本生成，界面截图取自当前源码构建的管理器网页演示版（演示数据，不读本机配置）。

## 生成

需要 macOS（编码用系统自带的 AVFoundation，不用 ffmpeg）、Google Chrome、Node 22+。

```bash
node tools/promo-video/capture-shots.mjs   # 构建演示版并按 1440×900@2x 截当前界面 → dist/promo/shots/
node tools/promo-video/render.mjs          # 逐帧渲染、编码、并入配乐 → dist/promo/codex-plus-plus-promo.mp4
```

界面改了之后重跑第一步，视频里的截图和高亮框位置（`shots/regions.json`）会一起更新。

检查用：

- `render.mjs --stills 9,24,48` 只出几张关键帧 PNG 到 `dist/promo/stills/`
- `render.mjs --from 40 --to 52` 只出一段（不带配乐）
- 起一个静态服务后用浏览器打开 `index.html` 可以边放边拖进度条（`?t=48` 从第 48 秒开始）；`render.mjs` 里的挂载方式（`/shots/`、`/promo/`、`/repo/`）照着配

## 文件

| 文件 | 作用 |
| --- | --- |
| `index.html` / `promo.css` / `promo.js` | 整片是时间 t 的纯函数，`window.__seek(t)` 把画面摆到第 t 秒；不用 CSS 动画，逐帧可复现 |
| `capture-shots.mjs` | 构建 `apps/codex-plus-manager/web-preview` 演示版，截图并记录关键元素位置 |
| `music.mjs` | 程序合成的 120 BPM 配乐（一小节 2 秒），段落和画面切点对齐 |
| `encoder.swift` | 从管道读帧编 H.264；再把配乐直通并进 mp4 |
| `render.mjs` | 串起以上步骤 |
| `lib/cdp.mjs` | 无头 Chrome 驱动和静态服务，只用 Node 自带模块 |

## 段落

| 时间 | 内容 |
| --- | --- |
| 0–12s | 开场：打字提问 → Codex++ 标志落下 |
| 12–20s | 50+ Agent 快切 |
| 20–40s | 01 万物皆拓展：在 Codex 对话里把发送按钮改成狗头、上下文圆环改成彩虹进度条；拓展页精选 / 本地 / 社区；能力标签 |
| 40–64s | 02 本地服务：请求流拓扑（429 限流自动换下一家）；共享供应商、路由（三种模型名写法）、Agent 接入 |
| 64–76s | 03 每个模型各有空间：`[1M]` 后缀 → config.toml 指针 → model catalog；供应商详情按模型设置 |
| 76–92s | 04 用量与请求：统计、请求日志、托盘快捷面板 |
| 92–108s | 会话、MCP 与 Skills、Codex 多开、Codex 插件、Claude Desktop 增强、⌘K；截图墙 |
| 108–120s | 片尾：下载地址 |

对话造拓展那段是按 README 里的实测流程重绘的示意界面，其余界面都是真实截图。
