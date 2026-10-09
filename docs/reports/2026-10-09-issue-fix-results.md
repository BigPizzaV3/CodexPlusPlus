# 最新 open issues 修复与沟通记录

日期：2026-10-09，北京时间。范围为[补充审核](2026-10-09-latest-open-issue-audit.md)的 14 条，以及后续新增 #2432、#2433。

用户授权实施分享开关和其他可定位修复，并关闭已解决条目、对不确定条目询问证据。本轮没有修改凭据、安装依赖、修改 manifest 或合入未经审核的外部 PR。

## 实施内容

| Issue | 实施结果 | 关闭边界 |
| --- | --- | --- |
| #2419 | 管理器与内置增强菜单新增分享显示开关，默认保持开启；保存后清理/恢复，跟随总开关；宿主限定唯一会话工具栏，避免误挂 review header | 代码交付后按已满足设置需求处理 |
| #2418 | 模型配置首次默认展开，7 个分组记住展开偏好，新校验错误展开；不改变 profile 或草稿 | 代码交付后按交互修复处理 |
| #2414 | 排除继承变量误选、布局祖先及侧栏/工具 pane；恢复失效目标原样式；保留兄弟 sticky footer、无 footer 的原生 composer、新旧首页拓扑 | 修复可确认的识别缺陷，原报告的全部现象仍需复测 |
| #2426 | Windows 捕获本次进程身份与退出宽限，记录终态；身份查询失败/端点替换不跟随其他实例 | Windows CI 与真实 Store 重启验证分别记录；不以 macOS 测试代替实机验收 |
| #2424 | Chat 转换发送前明确拒绝无法解释的 encrypted_content，HTTP 400 / unsupported_encrypted_agent_content；Responses 原样透传 | 防止静默删除，未恢复加密任务正文，因此继续开放 |
| #2421 | 原生 SSE 旁路观察 failed/incomplete/error/缺 completed 的 EOF，记录固定分类；事件前缀有界且原字节透传 | 诊断增强不是上游 protection 故障已解决，因此继续开放 |
| #2422 | 补 Kimi-K3 标准模式/默认模式回归及提示，保留每供应商的原有默认协议 | 发帖者已确认配置解决，已关闭 |
| #2433 | 暂停事件有界保存，跟踪当前暂停序号/归属；求值失败仍恢复自身暂停，过期、超时和失败检查断点移除/disable/关闭；捕获验证 constructor/RPC 能力 | 确认的控制流缺陷已修，用户 macOS 27 / Codex 26.924 的现场仍待复测，保持开放 |
| #2432 | 补原生配对的 host/会话元数据；删除前后复核身份，取消成功删除后的整页 reload，采用原生导航和列表刷新；原生插件入口存在时去重 | 原用户官方版本/字段形态未知、整体 UI 反馈未等同解决，保持开放 |

renderer 公开类名/constants 与拓展路由白名单保持兼容，新增节点带 data-codex-plus-ext。分享设置 codexAppSessionShare 默认 true，关闭后移除既有按钮；无可靠宿主不回退 body。

居中查找新增真实函数 DOM 夹具，涵盖嵌套 sidebar/summary、继承变量、多个 footer、多个会话、兄弟页脚、原生 composer、旧首页输入框、状态 aside、合法分享/听写拓展控件和旧目标样式恢复。没有针对未知皮肤全局清空背景或抬高 z-index。

## GitHub 沟通

| Issue | 当前处置 | 本轮回复 |
| --- | --- | --- |
| #2422 | CLOSED / completed，按作者已确认的配置方案 | [说明](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2422#issuecomment-6065389994) |
| #2421 | OPEN，询问上游协议、失败事件、新旧会话与直连对照 | [取证](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2421#issuecomment-6065534048) |
| #2424 | OPEN，询问真实 agent 请求结构/构造代码；说明 PR #2425 尚缺协议依据 | [取证](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2424#issuecomment-6065534655) |
| #2423 | OPEN，请完整重装正式 v1.7.0 后核对版本、签名与启动结果 | [复测](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2423#issuecomment-6065535196) |
| #2429 | OPEN，询问主题/官方版本/遮挡节点及覆盖层开关矩阵 | [取证](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2429#issuecomment-6065535657) |
| #2417 | OPEN，询问最后启动阶段与耗时，区分扫描和 ready 后退出 | [取证](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2417#issuecomment-6065536214) |
| #2420 | OPEN，询问当前版本复现、实际 model ID、供应商模式与权限 | [取证](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2420#issuecomment-6065536710) |
| #2427 | OPEN，独立原生热键功能，询问平台与按住/切换交互要求 | [需求确认](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2427#issuecomment-6065537145) |
| #2412 / #2413 | OPEN，上次针对性的取证问题仍未补齐 | 沿用已有跟踪，避免重复索要相同材料 |
| #2433 | OPEN，说明捕获版本差异，询问单项开关与暂停日志对照 | [取证](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2433#issuecomment-6074006374) |
| #2432 | OPEN，分别说明 host 守卫、页面 reload 和重复入口，询问官方版本与会话类型 | [取证](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2432#issuecomment-6074006617) |

## 验证与隔离

先建立失败回归，再实现修复。共享目录途中出现外部的 custom-layout-runtime.test.ts 未完成改动；保留该文件，用托管隔离 worktree 从 41048a2c 仅复制本轮源文件和新测试，并重新组装 renderer 产物。提交不包含外部布局测试、README 预览或宣传素材。

最终隔离目录 `cargo test --offline --locked --workspace`：**1977 passed / 0 failed / 8 ignored**。含完整管理器行为测试和 TypeScript 检查，renderer 分片组装一致性、Vite 生产构建与 `git diff --check` 通过。使用已安装工具链、`TMPDIR=/private/tmp` 和已有依赖缓存。

新增暂停捕获测试先复现暂停通知早于命令回包而丢失的失败，再验证当前/历史暂停序号、其它调试器、求值与恢复失败、8 项/1 MiB 队列上限、过期和未知断点清理。删除测试覆盖确认等待、返回期间切会话、复用 DOM、可信/未知/冲突 host、当前/非当前与远端会话。原生字段形态依据本机官方 26.930.61225 静态程序资源，未操作真实会话或安装应用。

修复分支为 `codex/latest-open-issues`，提交后的跨平台结果以 PR 检查为准。全仓 cargo fmt --check 有既有格式漂移，没有对无关文件做全局格式化。真实 Windows Store 连续重启、macOS 27 的加载现场、真实供应商压缩失败及实际皮肤时序仍需原用户复测；不把本机通过冒充这些验收。
