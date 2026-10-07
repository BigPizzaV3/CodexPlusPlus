# API Key 语音输入设计与验证记录

日期：2026-10-07。该 feature 为 opt-in，独立配置 ASR 服务，不改变模型列表、编码模型供应商或现有 ChatGPT 原生听写逻辑。

## 选择独立 CPP 控件的原因

26.930.61225 安装包的原生输入框能力链在 app-shared 中检查 `authMethod === "chatgpt"`；原生流式服务由主进程注入 ChatGPT 认证，默认使用 `wss://chatgpt.com/backend-api/dictation/stream`。批量路径默认为 `https://chatgpt.com/backend-api/transcribe`。它们不会因编码供应商配置 API Key 而转向 `/v1/audio/transcriptions`。

仓库旧补丁向动态 import 返回的 ESM namespace 赋值，赋值失败后仍可能报告成功；仅移除按钮 disabled 也不能绕过 React capability 或主进程认证。app-host 使用 Cap’n Web MessageChannel，其 RPC stub 明确禁止 set、defineProperty 和 setPrototypeOf。仓库现有 CDP bridge 支持新文档注入及条件断点捕获内部类，没有现成加载时 bundle source transform。完整代理 port 还需要正确维护 import/export ID、异步 pipeline、字节和取消的生命周期，不适合作为这次功能的第一条落地路径。

因此 CPP 提供独立的“语音输入”入口和录音状态；只调用用户配置的独立 OpenAI 兼容 ASR；结果复用原生 composer 插入入口与发送按钮。原生 ChatGPT 听写和实时语音仍使用其原有流程。

## 流程与边界

`语音输入 → 麦克风权限 → getUserMedia（单声道）→ MediaRecorder → 停止并收集最终 chunk → CPP 特权 bridge（base64）→ Rust multipart → ASR /audio/transcriptions → text → 原生 composer 插入 → 可选原生发送`。

真实页面第一次联调暴露了 CSP 限制：`app://` 页面 connect-src 不允许直接访问 `http://127.0.0.1:*`，因此 renderer 的 HTTP 上传在到达 helper 前报 `Failed to fetch`。最终实现复用现有特权 bridge 传递音频，保持原生 CSP；HTTP 转写接口保留供后端自检和兼容使用，renderer 不再依赖它。

录音最长 595 秒，少于 250ms 或没有音频不发出 ASR 请求。上传限制为 25 MiB；错误后保留同一 Blob 可重试，取消则停止 tracks、录音器和当前请求。会话或目标变化后保留可复制文本，不写入其他会话。CPP 路径不启用原生 dictationStreaming，也不依赖其 ChatGPT 认证。

目标输入框录音/转写期间按 Enter，会先等待转写再发送；Esc 取消。Shift+Enter 及输入法组词确认继续由原生编辑器处理。

ASR 请求只发送音频、model、response_format=json，以及可选 language。不会自动把聊天记录、周边输入文本、字典或编码模型上下文发送给 ASR。该实现是批量听写，不产生流式预览。

## 原生 composer 的可靠插入

本安装包 ProseMirror 的 `dom.pmViewDesc` 保存 NodeViewDesc，不能据此假定存在 `.view`。根描述对象没有到 EditorView 的稳定反向引用，故 `.pmViewDesc.view` 只能作为经能力检查后的兼容路径，不能作为主要入口。

`app-initial-69cd8dbddec5.js` 导出 native composer registry 函数，可按函数源码能力识别，不依赖压缩导出名：

- 上下文 getter：源码同时包含 `document.activeElement?.closest`、`[data-codex-composer-root]`、`composerId:` 和 `root:`。本包 IVn，字符偏移 2484447。返回 root 是 composer 的祖先容器，不是编辑器 DOM；必须检查 `root.contains(editor)`。
- 定向 append：源码同时包含 `.composerId===`、`.appendPromptText`、`.isDictationInProgress`、`[readonly]`、`.focus()` 和 `return!1`。本包 CVn，字符偏移 2481916。输入为 `(composerId, text, optionalAttribution)`，拒绝禁用/不可编辑目标和原生听写冲突。
- controller 的 `appendPromptText` 位于字符偏移 10341127：使用 `Qi.atEnd(doc)`，构造原生 transaction 追加到文档末尾，并 dispatch/focus；保持已有节点，按原生逻辑加空格。

录音启动时绑定 composerId、root 与当前 href；完成时重新确认目标。不能调用按“当前 active composer”插入的无目标版本，否则录音期间切换会话可能误插。兼容旧版本时，textarea 用原型 value setter 并发送 input；contenteditable 只能用浏览器编辑命令并检查结果，失败保留可复制文本。不得直接覆盖 innerHTML/textContent。

registry 不公开 sendPrompt callback。停止并发送需先插入，再等待原生渲染，且仅在目标 href/composerId/root 仍一致时点击该 root 内可见、启用的原生发送按钮。无法辨认发送按钮、处于 Stop/Resume 状态或导航已变化时保留草稿供用户发送。按钮原生提交回调继续处理附件与提交权限。

## 配置

设置字段位于 `BackendSettings.dictation`，前端使用 camelCase：

管理器的「Codex增强」页面分为「常用增强」「语音输入」「下一步建议」三个标签，共用总开关、配置草稿和顶部保存入口。下一步建议原名 Stepwise，原配置字段与旧版 `settings/stepwise` 跳转保持兼容。语音的语言、超时和环境变量在高级参数中。增强已统一，不再区分兼容与完整模式，旧 `launchMode` 字段忽略并在保存时移除。关闭增强总开关会停用语音入口和转写，但不会清除独立语音 Key 或原来的启用设置，重新打开后恢复。

```json
{
  "dictation": {
    "enabled": false,
    "baseUrl": "https://api.groq.com/openai/v1",
    "apiKey": "",
    "apiKeyEnv": "",
    "model": "whisper-large-v3-turbo",
    "language": "",
    "timeoutSeconds": 120
  }
}
```

预设只调整 baseUrl/model，不覆盖用户的 key、语言或超时：Groq 为 `https://api.groq.com/openai/v1` / `whisper-large-v3-turbo`；OpenAI 为 `https://api.openai.com/v1` / `whisper-1`；本地为 `http://127.0.0.1:8000/v1` / `whisper-1`。预设是便捷配置，并非服务可用性保证；用户仍可填兼容服务自己的地址与模型。

baseUrl 只接受 HTTP(S)，禁止 URL userinfo、query 和 hash。model 不能为空。language 留空表示自动识别。timeoutSeconds 归一化到 1–600 秒。直接 apiKey 优先于明确指定的 apiKeyEnv；不隐式读取 OPENAI_API_KEY，不回退到编码供应商密钥。本地回环地址可无 key，远程服务必须有 key。

## 接口与文件

- `crates/codex-plus-core/src/dictation.rs`：URL/Key/配置检查、multipart 解析与重建、ASR 请求及错误归一化。
- `crates/codex-plus-core/src/launcher.rs`：helper `/dictation/transcribe` HTTP POST；公开 `/dictation/status` HTTP GET 只返回无密钥状态。
- `crates/codex-plus-core/src/routes.rs`：特权 bridge `/dictation/status` 提供当前 helper capability token；`/dictation/transcribe` 接收有界 base64 音频，`/dictation/cancel` 通过 requestId 取消上游请求。与公开 HTTP status 是两层接口。
- `crates/codex-plus-core/src/bridge.rs`：CDP 单 frame/message 上限为 40 MiB，容纳 25 MiB 音频的 base64 和 JSON 封装；仍保留明确的传输上限。
- `apps/codex-plus-manager/src/dictation-settings.ts` 及 App.tsx：opt-in 配置和预设。
- `assets/inject/renderer-inject/93-dictation.js`：录音、重试、取消、结果插入、发送和恢复 UI；分片改动后需组装 renderer-inject.js。

前端转写上传携带随机进程内 helper token、随机 requestId、音频和格式元数据，不携带 ASR Key。bridge 的 settings 输出移除 ASR apiKey，仅返回 apiKeyConfigured；ASR Bearer 由 Rust 后端加入；不跟随重定向，错误不回显供应商响应体，公共 status 不返回 key/token，音频和 key 不写入诊断日志。转写和取消路由不在第三方拓展路由白名单开放。

编码分段进行，避免大音频展开耗尽 JS 调用栈，并在编码期间响应取消。后端验证 token/UUID、base64 长度及解码后 25 MiB 上限；MIME 使用白名单，文件名按格式重新生成。最多同时处理 4 个桥接转写；取消先到或请求完成的记录最多保留 128 条、120 秒过期。取消会释放正在等待的上游 future，前端同时丢弃晚到结果；成功后的清理不会再发送取消请求。

## 验证范围与尚需完成的验证

本记录的 native capability、RPC 不可变性、registry 和 append 语义来自本地安装包静态拆包。静态研究未录制真实音频、调用付费 ASR、发送聊天或提取用户凭据。后续隔离注入沿用现有 CPP 流程，只读配置中的插件启用名称；没有修改用户 auth.json、config.toml 或编码供应商配置。

管理器实现已通过 TypeScript 检查、Vite 生产构建、全部管理器回归测试（344 通过、1 跳过、0 失败）与 diff whitespace 检查。修复 CSP 上传问题后重新构建原生程序成功；`cargo build -p codex-plus-launcher -p codex-plus-manager --locked` 成功，`cargo test -p codex-plus-core dictation --locked` 的 21 个听写及桥接传输相关测试全部通过。构建仅获取 Cargo.lock 声明的依赖，未新增依赖或工具链。当前安装包 exports 的静态指纹扫描中，定向 append 只匹配 CVn，上下文 getter 只匹配 IVn。

renderer 的行为测试已通过，覆盖原生接口按能力发现、草稿与选区保留、readonly 拒绝、录音最终 chunk、取消后晚到结果、权限请求期间取消、同 Blob 重试、会话切换、延迟发送期间导航和取消、大小与时长限制。该测试已接入管理器的 `node --test src/*.test.ts` 入口。录音计时只更新文本，不销毁按钮，避免失去键盘焦点或取消正在进行的点击。

CSP 修复后的行为测试强制让 renderer fetch 抛错，验证语音上传仍使用特权 bridge；分段 base64 编码保持跨段二进制内容，编码期间取消不启动上传，转写取消发送匹配的 requestId，成功清理不发送取消。Rust 取消测试使用延迟 TCP 上游并检查取消后连接 EOF；CDP 传输测试通过本地模拟 WebSocket 单 frame 发送完整 25 MiB 音频的 base64/JSON，验证约 33.4 MiB 消息不会触发旧 16 MiB frame 上限。

浏览器使用真实 DOM 加模拟麦克风、ASR 响应和 composer registry 验证了入口显示、停止并插入、停止并发送、麦克风释放以及保留已有草稿/附件。此验证没有采集真实音频或调用第三方服务；验证截图位于 `/tmp/codex-plus-dictation-verification.jpg`。

新构建的原生 manager 已独立启动，并通过 CUA 验证默认关闭、本地预设、非法 URL 提示，以及刷新恢复未保存的表单。CSP 修复后已重新构建并启动最新版 manager，设置页截图位于 `/tmp/codex-plus-dictation-settings-latest.png`。真实 Codex 界面操作被 CUA 安全限制拒绝，未使用其他界面操控渠道绕过限制。用户手测初次录音遇到上述 CSP 上传失败；修复后专用 mock 已收到实际录音，字段校验通过，bridge 成功返回测试文本。用户随后提供截图，确认“本地语音链路测试成功”已写入目标 Codex 原生输入框，完成实际录音、桥接上传与文字回填验证。该次手测使用专用本地 mock；实际 UI 发送尚未手测。

用户创建 Groq Key 并授权配置后，已启用独立听写配置：官方 Groq Base URL、`whisper-large-v3-turbo`、`zh`、120 秒超时。仅更新现有 settings 的 dictation 字段，文件权限为 0600，未修改其他设置；Key 未写入仓库或诊断日志。最新版 manager 的设置页已显示启用状态、上述字段及掩码 Key，截图位于 `/tmp/codex-plus-groq-settings-configured.png`。

新增 `crates/codex-plus-core/examples/dictation_live.rs`，只读取已有配置；探针固定使用首次校验过的 Groq 配置快照，避免并发设置变化切换请求目的地。通过真实 helper 和特权 bridge 上传系统语音生成的非敏感中文 WAV（130324 字节），Groq 返回 `这是音输入测试,请把这句话转成文字。`，耗时 1544 ms。音频原文为“这是语音输入测试，请把这句话转成文字。”，此次漏识别了“语”字；仅这一条样本不能代表真人录音的识别质量。真实转写后，现有 CPP 注入流程已成功接入隔离 Codex 窗口。用户随后手测真人录音、停止并插入，并确认“成功回填文字”，完成真实 Groq 服务的录音、转写与原生回填验证。示例编译通过，失败输出仅使用后端已去敏的 message；不保存音频内容、Key 或 helper token 到日志。

新增 `crates/codex-plus-core/examples/dictation_smoke.rs`，使用严格新建、权限为 0600 的临时 settings、假 Key 和专用本地 mock ASR。同一进程启动真实 helper，经 privileged bridge 获取 token 后上传 60 字节静音 WAV；公开 status 未包含 Key/token、无 token 上传返回 403、带 token HTTP 上传及 bridge base64 转写 text 断言均通过，成功后的 cancel 返回 false。mock 验证收到 `/v1/audio/transcriptions`、file、model、response_format=json 和 language。该自检没有调用麦克风或第三方服务。

smoke 通过现有产品接口注入单独启动的 Codex userData 窗口，未调用启动器的供应商同步、数据库迁移或关闭现有 Codex 流程。注入完成仅证明产品注入链路成功，不能代替真实界面和音频验证。进程到期或 stop marker 出现时关闭 helper；它不会卸载已注入 renderer，测试结束后应关闭隔离窗口或正常重新注入 CPP。

全量国际化校验存在仓库原有的缺失/陈旧键；本次新增 16 个 UI 翻译键均已覆盖，未扩大这个缺口。

后续按用户要求撤下远程 Zed、Upstream worktree、强制中文和原生菜单汉化：管理器导航、开关、Tauri 命令、核心路由与注入适配器均已移除，旧设置不能重新启用。旧源码文件未物理删除，但对应模块不再注册。普通原生 SSH/worktree、会话分享与复制、原生菜单位置能力继续保留；发布过的拓展常量和类名字面量保持兼容，拓展路由白名单未扩大。原来的 Upstream 分支占用拦截属于该适配器的附属能力，一并撤下，原生 worktree 使用原生行为。

强制中文的撤回迁移只在旧 managed 标记存在且当前 `localeOverride` 仍等于标记中的 appliedLocale 时恢复合法的 previousValue；用户后续自行修改的语言保持原值。原生接口失败时保留标记，等待下次注入继续撤回；不覆盖 navigator 或 Statsig，不自动重载页面。已经运行的旧菜单汉化补丁会随 Codex 主进程退出消失，新构建不再安装。

调整后的管理器回归测试为 351 通过、1 跳过、0 失败；TypeScript、Vite 构建及 launcher/manager 原生构建成功。核心单测 573 通过、2 忽略，CDP 集成 159 通过，旧设置退役兼容 3 通过；其他核心集成测试已通过。macOS 单测使用 `TMPDIR=/private/tmp`，避免默认 `/var` 链接路径触发原生浏览器 fixture 的路径限制。renderer 的语音、生命周期与退役迁移相关测试 22 通过。另修复语音 UI 安装异常影响已成功加载设置的边界：独立隔离 UI 安装错误，Fast/provider 设置加载不受影响。

新构建的管理器已在独立窗口检查：侧边栏无 Zed 入口，增强页无上述四项功能；语音与 Stepwise 的完整字段和测试连接位于增强页，设置页无这两个面板。临时关闭总开关时，语音和 Stepwise 开关禁用，底部出现统一保存按钮；未保存此临时修改，刷新后原来的增强启用状态与 Groq 配置恢复。分类截图位于 `/tmp/codex-plus-enhancements-final.png`，Stepwise 参数截图位于 `/tmp/codex-plus-enhancement-services.png`。

功能验证应覆盖：默认关闭；旧设置迁移；独立 ASR key 不混用编码 key；multipart 二进制透传与模型/语言字段；key/env、本地匿名服务、超时、取消、上游错误去敏；renderer 录音停止最终 chunk、同音频重试、权限失败、25 MiB 限制；原生 registry 追加保留 mention/附件；录音期间切换会话和完成后发送期间导航不能误发送；未知原生按钮布局保留草稿。真实麦克风配合 mock 的原生回填、真实 Groq 的后端转写、真人录音配合真实服务的完整回填均已通过；实际发送仍待手测。

## 最终布局与统一增强验证

增强页使用三个页内标签：常用增强、语音输入、下一步建议（原 Stepwise）。总开关、保存状态与保存按钮在顶部固定显示；切换标签只改变显示状态，三个配置面板保持挂载，共享配置草稿。语音控件统一高度并使用等宽双列，语言、超时、环境变量归入高级参数；下一步建议的三个开关横排，完整连接与高级参数保留。外部深链先选中对应标签，再定位配置区域。

不再区分兼容与完整增强。Rust 移除 LaunchMode 字段，旧配置仍能加载，更新时清理旧 launchMode，保存不再输出；供应商 RelayMode 与语音密钥保持原样。插件市场只受总开关和自己的开关控制。管理器、增强菜单及后端提示均使用“下一步建议”，内部字段与接口名称保持兼容。另修复移除菜单汉化后留下的扫描调用。

最终验证：管理器 Node 测试 357 通过、1 跳过、0 失败；TypeScript、Vite、launcher/manager Cargo 构建通过。核心单测 575 通过、2 忽略，bridge 32、launcher 93、relay switch 18、CDP 159、assets 22 均通过；Tauri 旧配置归一化专项测试通过。当前新增文案均有英文词条；全量国际化校验仍有历史缺项，未扩大本次修改范围。

在独立原生管理器窗口验证了标签切换、键盘左右键和 End 导航、草稿保留及顶部保存状态。临时草稿已恢复，未保存测试值或重新发送录音。最终语音截图为 /tmp/codex-plus-enhance-final-voice.png，中文建议截图为 /tmp/codex-plus-enhance-final-suggestions.png。预览应用为 /tmp/codex-plus-enhance-final-dSwiYn/CodexPlusPlus Enhance Preview.app。
