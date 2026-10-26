# codexapp 的 Computer Use 与 Chrome 控制能否移植进 Claude Code 桌面应用

- 日期：2026-10-23
- 取证对象：
  - codexapp = `/Applications/ChatGPT.app`（Electron 155.0.8059.80，内置 `Codex Framework`，bundle 别名 `Codex`）+ `~/.codex/**` 落盘插件与 helper
  - Claude 侧 = `/Applications/Claude.app`（Electron 44.9.0 / Chromium 152 / Node 24.22.0，bundle id `com.anthropic.claudefordesktop`，v3.1623.4）
- 方法：asar 内存解析（未解包、未改动任何文件）+ `nm/otool/objdump/strings` + 磁盘明文插件读取

---

## 0. 结论速览

| 能力 | 能否移植 | 判断依据一句话 | 最快落地 |
|---|---|---|---|
| Computer Use 的**模型面**（sky/cua JS API、code-mode 工具） | ✅ 能 | 它就是一条 stdio MCP（`cua_repl`，工具 `js/js_reset/turn_ended`），协议与宿主无关 | 换启动命令即可跑在 Claude 的 MCP 配置里（前提是执行器自备） |
| Computer Use 的**执行器**（`SkyComputerUseService`） | ❌ 不能直接用 | 服务端对 IPC 客户端做 **code signature / teamID** 校验（`SkyIPCRequirement`、`_senderParentTeamID`、`LOCAL_PEERTOKEN`），只认 OpenAI 的 `2DC432GLL2` | 自己写一个 Swift helper（公开 API 足够复刻主干） |
| **后台点击 + 虚拟鼠标** | ⚠️ 机制能，成品不能 | 机制 = `AXUIElementPerformAction(AXPress)`（元素）+ `CGEventPostToPid`（坐标，真光标不动）+ 绘制式 `VirtualCursor`；这三样都可复刻 | 而且 **Claude.app 自己已经有等价实现**（`app-cu-helper`：`SLEventPostToPid` / `CGEventSetWindowLocation` / `PhantomCursor`）→ 不需要移植，只需要接线 |
| **Chrome 控制** | ✅ 能，且移植成本最低 | 真实 Chrome 的通道是「MV3 扩展 + native messaging（4 字节 LE + JSON-RPC 2.0）+ UDS」，全部宿主无关 | 自写扩展 + NMH，Claude 侧零改动 |
| **标签分组（tab groups）** | ✅ 能，但**只有一条路** | `chrome.tabs.group` / `chrome.tabGroups.update` 只存在于扩展上下文；CDP 的 `Target` 域做不到（Codex 自己在 IAB 里都把 `Target.*` 禁了） | 扩展里加 `tabGroups` 权限 + 一张方法白名单 |
| Codex 的 `node_repl` 特权管道、app-server 协议、审批模型 | ❌ 不能 | 它们是 ChatGPT.app 宿主私有能力（`nodeRepl.nativePipe` / `requestComputerUseApproval`） | 重做（Claude 侧有对等物：MCP + `computer:request_access` 权限卡） |

**一句话总判断：**
> Codex 这套东西**分层极其干净**：模型面是 MCP、浏览器面是 Chrome 扩展 + native messaging、桌面面是一个独立签名的 helper.app。**前两层可以直接照搬到 Claude.app；第三层不能照搬，但 Claude.app 自己已经有一个更硬的（SkyLight SPI 级）执行器。** 真正的工程量不在「移植 CU」，而在「把 Chrome 扩展 + 标签分组搬到 Claude 侧，并把 CU 的语义层接到 Claude 已有的执行器上」。

---

## 1. codexapp 解包后的三条链路

```
① Computer Use
模型 ──MCP(stdio)──► cua_repl ──► node_repl 内跑 JS ──► cua.* / sky.*
                                              │
                                              └─ unix socket（4B LE 帧 + JSON-RPC 2.0）
     ~/Library/Group Containers/2DC432GLL2.com.openai.sky.CUAService/IPC/computeruse.sock
                                                 └─► SkyComputerUseService（Swift，独立签名 app）
                                                     ├─ AX：AXUIElementPerformAction / SetAttributeValue / CopyElementAtPosition
                                                     ├─ CGEventPostToPid（坐标点击 / 按键，定向投递）
                                                     ├─ 私有 SAI：SAIVirtualKeyPress
                                                     └─ ScreenCaptureKit：SCScreenshotManager / SCContentFilter

② Browser（真实 Chrome）
node_repl ──nodeRepl.rpc("browser")──► browser-service.mjs
          ──UDS /tmp/codex-browser-use/<uuid>.sock?token=──► extension-host（Rust「ChatGPT for Chrome」）
          ──native messaging（stdio，4B LE + JSON-RPC 2.0）──► Chrome MV3 扩展 background.js
          ──► chrome.tabs / tabGroups / windows / debugger(CDP) / scripting / webNavigation

③ Browser（内嵌 IAB）
Electron WebContentsView + webContents.debugger(CDP 1.3)，Target.* 被裁剪，只留 Input.dispatchMouseEvent/KeyEvent/insertText
```

关键文件（全部实测存在）：

| 层 | 路径 |
|---|---|
| MCP 注册 | `~/.codex/plugins/cache/openai-bundled/unified-computer-use/26.1016.61520/.mcp.json` |
| code-mode 运行时 | `/Applications/ChatGPT.app/Contents/Resources/cua_node/{bin/node,bin/node_repl,lib/node_modules/@oai/*}` |
| sky JS→IPC | `…/@oai/sky/dist/project/cua/sky_js/src/targets/mac/native-pipe.js` |
| 执行器 | `~/.codex/computer-use/Codex Computer Use.app/Contents/MacOS/SkyComputerUseService`（25 MB，Swift，`com.openai.sky.CUAService`） |
| 浏览器服务 | `~/.codex/plugins/cache/openai-bundled/chrome/26.1016.61520/scripts/{browser-service.mjs,browser-client.mjs}` |
| 浏览器扩展 | `…/Google/Chrome/Default/Extensions/hehggadaopoacecdllhhajmbjkdcmajg/1.26.1020.11845_0/{manifest.json,background.js}` |
| NMH 注册 | `~/.codex/chrome-native-hosts-v2.json` + `…/Chrome/NativeMessagingHosts/com.openai.codexextension.json` |

**注意：Codex 没把 CU 写进自家 asar，而是作为「插件 + 独立 helper.app」外挂。** 这本身就是「可移植」的第一信号：它已经把自己解耦成一个 MCP server + 一个 app bundle。

---

## 2. Computer Use 分层拆解与逐层可移植性

| 层 | 内容 | 证据 | 能不能搬 |
|---|---|---|---|
| L1 模型面 | MCP server `cua_repl`，工具 `js` / `js_reset` / `turn_ended`；模型写 JS 而不是调离散工具（code-mode） | `.mcp.json`：`enabled_tools:["js","js_reset","turn_ended"]`、`tools.js.output_token_limit:25000` | ✅ 通用。Claude 支持 stdio MCP，但**没有** code-mode 的 `omit_tools_from:["code_mode","deferred"]` 语义 → 要么保留 JS 工具、要么拆成离散工具 |
| L2 运行时 | `@oai/cua` / `@oai/sky` / `@oai/cua-repl`，跑在 `node_repl` 里 | `NODE_REPL_TRUSTED_SERVICES={"browser":"@oai/browser-desktop/service","sky":"@oai/sky/service"}` | ⚠️ 半可搬。这些包依赖宿主注入的 `globalThis.nodeRepl`（`nativePipe.createConnection`、`launchServices.openApplication`、`env`、`requestMeta`）→ 需要写 ~30 行 shim（`net.connect` 就能替 `createConnection`） |
| L3 传输 | unix socket + 4 字节 LE 长度前缀 + JSON-RPC 2.0；方法 `ping` / `request` / `close`，带 `clientApiVersion` + `codexTurnMetadata` + `deadlineUnixMilliseconds` | `native-pipe.js`：`encodeMessageFrame` / `decodeMessageFrames` / `class MacNativePipeTransport` | ✅ 纯 Node 可实现 |
| L4 执行器 | `SkyComputerUseService`（Swift，模块 `ComputerUseCore` / `AccessibilitySupport` / `SlimCore`），另有 `SkyComputerUseClient.app`（ScreenCaptureKit + XPC）、`CUALockScreenGuardian.app`、`Codex Computer Use Installer`（含 auth plugin） | `embedded.provisionprofile` + `codesign -dv`：`Identifier=com.openai.sky.CUAService`、`TeamIdentifier=2DC432GLL2`、hardened runtime | ❌ **不能复用**。IPC 侧校验 `SkyIPCRequirement` / `sender_parent_team_id` / `LOCAL_PEERTOKEN`，只放行 OpenAI 签名的调用方；TCC 也绑在它的 bundle id 上 |
| L5 系统原语 | AX + `CGEventPostToPid` + ScreenCaptureKit（+ 私有 SAI） | 见 §3 | ✅ 可复刻（公开 API 覆盖主干，私有 SAI 可降级） |
| L6 权限/审批 | Accessibility + Screen Recording 授权窗；per-app approval 卡片；`computer-use-approval:` / `requestComputerUseApproval`；已批准 app 列表 | asar `computer-use-app-approval-request-card-*.js`、`computer-use-settings-*.js`、main bundle `Ms="computer-use-approval:"` | ❌ 要重做（Claude 侧有对等物：`computer:request_access` + `request_access` 工具） |

### 2.1 模型面长什么样（决定「搬过去要不要改提示词」）

`@oai/sky/docs/skills/oai_sky_lib/macos/SKILL.md` 明确禁止 `AppleScript/osascript/JXA/System Events`，只允许 `node_repl` + `@oai/sky`：

```js
var sky = (globalThis.sky ??= (await import("@oai/sky")).sky);
var state = await sky.get_app_state({ app: "com.google.Chrome" });
nodeRepl.write(state.text);   // 返回带 element_index 的 AX 树（默认返回 diff）
```

API 面（macOS：`target:"mac"`）：

```
list_apps / get_app_state({app, disableDiff})
click({app, element_index?, x?, y?, mouse_button?, click_count?, key?, duration?})
press_key({app, key}) / type_text({app, text})
scroll({app, element_index?, direction, pages?, x?, y?})
set_value({app, element_index, value})
select_text({app, element_index, text, prefix?, suffix?, selection_type?})
drag({app, from_x, from_y, to_x, to_y})
perform_secondary_action({app, element_index, action})
paste({app, text, format:"text"|"md"|"html"})
```

macOS 面**没有 `move`**（Linux full-desktop 面才有）——与 §3 的机制一致：macOS 上不需要移动真实光标。

---

## 3. 「后台点击 + 虚拟鼠标」的真实机制（决定性证据）

### 3.1 点击：两条路，都不动真实光标

| 场景 | 实现 | 证据 |
|---|---|---|
| 元素点击（`element_index`） | `AXUIElementPerformAction(el, "AXPress")`，**零鼠标事件** | `objdump` 反汇编 `sendClick` 区内两条命名 stub → `_AXUIElementPerformAction`；`AXPress` 字符串；`AXUIElementCopyElementAtPosition` 2 处（坐标→元素反查） |
| 坐标点击（`x,y`） | **`CGEventPostToPid`** 定向投递（真光标不动） | `sendClick` 4 KB 内 `bl` 统计：`CGEventAPI.postToPid(_:pid:)` ×5、`setTimestamp` ×5；**`CGEventAPI.post(_:tap:)` 全局投递 0 次**；`post` / `postToPid` 两个独立 GOT 槽 → 两个不同投递入口 |

> 反证与坑：`nm -u` 与 `strings` 都查不到 `CGEventPost*`（`grep -a -c "CGEventPost" = 0`），因为这些调用走**运行期绑定**（GOT 惰性解析）。所以「字符串查不到 = 不存在」在这里是错的——判定必须看反汇编的 `bl` 目标。
> 可以确定的否定项：`IOHID*` 全族 0 命中、`CGWarpMouseCursorPosition` 无、`AXUIElementCreateWithRemoteToken` 无。

### 3.2 键盘：三条并存

1. **私有 SAI**：`_OBJC_CLASS_$_SAIVirtualKeyPress` + `<SAIVirtualKeyPress: %d, %04x, %@>` + `pressKeys([...])` / `pressKeysForHolding(keyDown:keyUp:)` —— 批量施键、长按。
2. **CGEvent 层**：`SystemSoftware.CGEventAPI.createKeyboardEvent(source:virtualKey:keyDown:)` + `postToPid`。
3. **菜单 accelerator 解析**：`AXMenuItemCmdChar` / `AXMenuItemCmdGlyph` / `AXMenuItemCmdModifiers` / `AXMenuItemCmdVirtualKey` 各 1 处 —— 这是**读**菜单项声明的快捷键（把 `cmd_char + cmd_modifiers` 还原成 chord），不是注入通道。

`keyboardSetUnicodeString = 0` ⇒ 不做 Unicode 直塞；`type_text` 走 AX 赋值 / paste。

### 3.3 虚拟光标：屏上画出来的假光标

```
$s20AccessibilitySupport13VirtualCursorP        ← 协议
createVirtualCursorIfNeeded                     ← 工厂
SkyLensView / SkyLensViewRepresentable          ← 镜头视图
Contents/Resources/LensSequence/Lens_frame_00..44.png   ← 45 帧动画
$s14SystemSoftware15WindowServerSPI…setCursorLocation…   ← 真光标定位，独立包装
```

- `setCursorLocation` 在 `sendClick` 区内 **0 次调用** ⇒ 点击全程不移动真光标。
- HUD 文案来自 `~/.codex/computer-use/config.json`：`{"accentColor":"#339cff","strings":{"usingComputer":"ChatGPT is using your computer","escToCancel":"Esc to cancel"}}`。
- 另有「Show backgrounded apps that Computer Use is working on in Picture-in-Picture mode」「Enable the virtual cursor in Computer Use.」等开关。

### 3.4 截图与权限

- 截图：`SkyComputerUseClient.app` 引 `_OBJC_CLASS_$_SCScreenshotManager / SCContentFilter / SCShareableContent / SCStream / SCStreamConfiguration`，带「排除自己窗口」语义（`captureExcluding`）；service 侧负责 `_CGPreflightScreenCaptureAccess` / `_CGRequestScreenCaptureAccess`。
- 权限提示直给：「Computer Use permissions are still pending… granting Accessibility and Screen Recording permissions in the ChatGPT Computer Use window」。
- 深水区（不建议照搬）：`CUALockScreenGuardian.app` + `CodexComputerUseAuthorizationPlugin.bundle`（SecurityAgent 授权插件，锁屏/登录场景），需 root 安装。

---

## 4. Chrome 控制与标签分组

### 4.1 五个后端

`browser-service.mjs`：`L8=["chrome","iab","dot","cdp","mcpapps"]`，可用 `BROWSER_USE_AVAILABLE_BACKENDS` 裁剪。
`chrome` = 真 Chrome + 扩展；`iab` = 内嵌 `WebContentsView` + Electron CDP（桌面默认）；`dot` = 远端（`thread_source==="aeon_child"`）；`cdp` = 用户自备 Chromium（`full_cdp_access_enabled` 才放开）；`mcpapps` = MCP App 内嵌浏览器。

### 4.2 真实 Chrome 的通道

```
扩展 background.js ──chrome.runtime.connectNative("com.openai.codexextension")──►
  extension-host「ChatGPT for Chrome」(Rust, ~1MB)
    ──ws://127.0.0.1:<port>/?token=  或  UDS /tmp/codex-browser-use/<uuid>.sock──►
  browser-service.mjs ──nodeRepl.nativePipe（特权桥）──► @oai/browser-desktop 客户端（模型侧 JS）
```

- 帧格式：native messaging = 4 字节 LE 长度 + JSON；host↔service = WS 握手（`Sec-WebSocket-Key` / `258EAFA5-…`）或 UDS + `?token=`。
- JSON-RPC 方法（扩展侧 handler 键名）：`ping / getInfo / getTabs / getUserTabs / getUserHistory / getBookmarks / getRecentlyClosedSessions / getTopSites / createTab / claimUserTab / markTab / finalizeTabs / nameSession / moveMouse / notifyCursorArrived / turnEnded / attach / attachTarget / detach / detachTarget / executeCdp / getCommittedTabUrl / createNotification / addDownloadChangeListener`。
- 安全：NMH `allowed_origins` 精确绑定两个扩展 ID（`hehggadaopoacecdllhhajmbjkdcmajg` / `odlomjlbamekndcpllcnffbgeohgkmjh`）；无 `externally_connectable`；隐身上下文过滤。

### 4.3 「后台」靠什么（四个手段，都不是 Chromium 节流开关）

1. `chrome.windows.create({focused:false})` + `chrome.tabs.create({active:false})`
2. attach 后立刻 `Emulation.setFocusEmulationEnabled{enabled:true}` —— 页面以为自己是前台（`rAF`/`visibilityState`/`document.hasFocus()` 正常），用户看不到，**这是不抢焦点又能可靠自动化的关键一招**
3. `chrome.debugger.sendCommand` 对后台标签直接生效
4. `visibility.set(false)` 逻辑隐藏

> 纠偏：`browser-background-switches-*.js` **不是**节流开关，而是 `disable-background-networking` / `disable-component-update` 这类 Chromium 启动 flag 的门控常量，且**默认不生效**（要带 `--codex-browser-background-networking-disabled` 启动）。全库无 `backgroundThrottling` / `--disable-renderer-backgrounding`。

### 4.4 标签分组：会话 ↔ 原生分组 1:1

```js
// background.js：Kb 单例（managedGroupIds / sessionGroupTitles / storageKey="TAB_GROUPS"）
async createGroup(tabId, title) {
  const {windowId} = await chrome.tabs.get(tabId);
  const gid = await chrome.tabs.group({createProperties:{windowId}, tabIds:[tabId]});
  this.managedGroupIds.add(gid);
  if (chrome.tabGroups?.update)
    await chrome.tabGroups.update(gid, {color: randomOf9Colors(), title: title ?? "ChatGPT"});
}
```

- 建组：`chrome.tabs.group`（**只此一条**，`tabGroups` 没有 `create`）；强制同窗口（跨窗口直接抛错）。
- 命名/配色/折叠/移动：`chrome.tabGroups.update({collapsed,color,title})` / `chrome.tabGroups.move`。
- 删组：无 API，靠 `tabs.ungroup` + 组空自动消失 + `tabGroups.onRemoved`。
- 保护：`shared` 分组一律拒绝修改。
- 颜色：Chrome 9 个命名色随机取一次后固定；标题 = 会话标题（`nameSession` 同步），未命名 = `"ChatGPT"`。
- 审计：`chrome.storage.session["BROWSER_MANAGEMENT_AUDIT_TRAIL"]`，≤50 条 / TTL 3 天。
- 暴露给模型的字段：

```ts
tabLayout: {
  groups: {collapsed, color, id, title?, windowId}[],
  tabs:   {autoDiscardable, groupId, id, index, pinned, url?, windowId}[],   // url 仅在“被操作涉及”时带
},
windows: {focused, height?, id, left?, state?, top?, width?}[],
bookmarks: {id, index?, parentId?, title, url?}[]
```

- 方法白名单（唯一权威）：`tabGroups:{get,move,query,update}` + `tabs:{get,group,move,query,remove,ungroup,update}` + `windows:{create,get,getAll,getCurrent,getLastFocused,update}` + `bookmarks:{…}`；属性白名单 `collapsed/color/title`。

---

## 5. Claude.app 侧现状：为什么「移植」更多是「接线」

### 5.1 它已经有一个更硬的 CU 底座

| 组件 | 位置 | 关键点 |
|---|---|---|
| in-process MCP | `.vite/build/index.chunk-uXhgx7My.js` | `SERVER_NAME="computer-use"`、`MIN_COMPUTER_USE_MACOS_MAJOR=14`、`getComputerUseHostAdapter().executor` |
| 工具面 | 同上 + `index.chunk-Bmha9cgf.js` | `computer_screenshot / left_click / double_click / triple_click / right_click / type / key / scroll / left_click_drag / mouse_move / hold_key / left_mouse_down / left_mouse_up / cursor_position / read_clipboard / write_clipboard / wait / batch …` + app 定向家族 `app_click / app_type / app_screenshot / app_list_windows / request_full_control` |
| 原生模块 | `@ant/claude-swift/build/Release/computer_use.node`（N-API，1.08 MB） | 导出类：`PhantomCursor` / `WindowCover` / `OwnWindowsClickThrough` / `BackgroundMenuGuard` / `CollisionWatch`；`appScoped.{setPhantomCursor, armFocusStealGuard, beginBackgroundAction, showWindowBehindTheFrontWindow, setOwnWindowClickThrough}` |
| 事件注入 | `Contents/Helpers/app-cu-helper`（1.0 MB，Anthropic 签名，JSON-RPC `probe` / `dispatchRaw`） | 符号：`SLPSSetFrontProcessWithOptions` / **`SLEventPostToPid`** / `SLPSPostEventRecordTo` / **`CGEventSetWindowLocation`** / `viaWinLocalPts` / `chromiumMouseRoute` / `secure_input_active` |
| 截图 | `computer_use.node` | `SCScreenshotManager` / `SCContentFilter` / `captureExcluding` / `captureRegion` / `captureWindow` |
| 权限 | `ComputerUseTcc`（7 方法） | `getState/requestAccessibility/requestScreenRecording/…`；`_AXIsProcessTrustedWithOptions` |
| 教学/录屏 | `computerUseTeach.js` / `computerUseWatchRecord.js` / `watchRecordChooser.js` | 与「控制」无关，别被文件名骗 |

**结论：Claude.app 的 CU 不是「云端虚拟机」，而是「模型在云上、执行在本机」，且已经是 SkyLight SPI 级的定向事件投递。** 就「后台点击 + 虚拟鼠标」这项，Codex 与 Claude 同一档（Codex：`CGEventPostToPid` + `VirtualCursor`；Claude：`SLEventPostToPid` + `CGEventSetWindowLocation` + `PhantomCursor`）。这项**不需要移植**。

### 5.2 它也有 Chrome 桥，但没装扩展

- `Contents/Helpers/chrome-native-host`（Rust，1.1 MB）：socket server + `permission_request` / `tool_use_id` / `execute_tool` / `tool_request` / `tool_response` / `notification` —— Claude 自己的 native messaging host。
- IPC：`ChromeExtension`（`isInstalled / installExtension / restartChrome / getBrowserActivity / launchBrowser / getConnectedExtensions`）。
- 现状：`~/Library/Application Support/Google/Chrome/NativeMessagingHosts/` 只有 `com.notion.*` 与 `com.openai.codexextension`，**没有 Anthropic 的 NMH**，Chrome 里也没装 Claude 扩展 ⇒ 这条链目前空转，正好可自建。

### 5.3 扩展点（决定改造成本）

| 方式 | 位置 | 备注 |
|---|---|---|
| `mcpServers`（stdio/sse/http/ws/sdk） | `~/Library/Application Support/Claude/claude_desktop_config.json` | `type:"stdio"` 就是 `spawn(command,args,{env})`，**任意本地可执行文件**；`strictMcpConfig` + `disallowedTools:["mcp__*"]` 可一键收紧 |
| 插件 / marketplace | `LocalPlugins` / `CustomPlugins` IPC；`.claude-plugin/plugin.json` + `.mcp.json` | 与 Codex 的 plugin 结构几乎同构 |
| DXT / MCPB | `Extensions`（24 方法，`installDxt*`） | 可打包分发 |
| Skills | `resources/bundled-skills/*.skill` | — |

### 5.4 硬约束（改造前必须知道）

| 约束 | Claude.app | 影响 |
|---|---|---|
| Electron fuses | `RunAsNode=off`、`NodeOptions=off`、`NodeCliInspect=off`、**`OnlyLoadAppFromAsar=on`**、**`EnableEmbeddedAsarIntegrityValidation=on`** | 不能 `ELECTRON_RUN_AS_NODE`、不能 `--inspect`、不能把 asar 解成目录跑 |
| asar 完整性范围 | 只校验 **header JSON 的 sha256**（`aa065331…`），不是整包 | **同长度原地改写** → header 不变 → 校验照过、无需重签；改大小 → 必须同步改 `Info.plist:ElectronAsarIntegrity` 再重签 |
| entitlements | 主 app **没有** `disable-library-validation` | 新增/替换的 `.node` 必须是 Anthropic/Apple 签名，否则 dlopen 失败；要自加模块必须重签并补 `com.apple.security.cs.disable-library-validation` + 保留 `allow-jit` |
| N-API | Node 24.22 / ABI 149，`computer_use.node` 是 N-API | N-API 跨 Electron 版本可用；理论上可整文件搬走 |
| TCC 身份 | 辅助功能/录屏绑 bundle id | 换 helper 的 bundle id = 用户要重新授权一次 |

---

## 6. 可移植性矩阵（逐项）

| # | 资产 | 判定 | 做法 | 工作量 |
|---|---|---|---|---|
| 1 | sky/cua 的 JS API 形状（元素索引 + AX 树 diff + click/type/scroll…） | ✅ 照抄设计 | 在 Claude 的 MCP server 里实现同名 API | S |
| 2 | MCP 传输 + tool schema + 审批钩子 | ✅ 直接复用 | 写 `mcpServers` 条目；审批用 Claude 的 `computer:request_access` / 权限卡 | S |
| 3 | 元素点击（AXPress） | ✅ 复刻 | Swift/ObjC 20 行 | S |
| 4 | 坐标点击（`CGEventPostToPid`） | ✅ 可复刻 | `CGEventCreateMouseEvent` + `CGEventPostToPid`；或直接用 Claude 的 `app-cu-helper`（`dispatchRaw` 已支持 `viaWinLocalPts`） | S |
| 5 | 键盘（`SAIVirtualKeyPress` 私有） | ⚠️ 降级 | 公开替代：`CGEventPostToPid` + `CGEventKeyboardSetUnicodeString`；或 Apple Events（要 automation 授权） | S/M |
| 6 | 虚拟光标（`VirtualCursor` 绘制） | ✅ 复刻 | 一个 `NSPanel`（`ignoresMouseEvents` + `level=.screenSaver`）画 PNG 序列；Claude 已有 `PhantomCursor`，直接调 | S |
| 7 | 截图（SCScreenshotManager + captureExcluding） | ✅ 直接复用 | Claude `computer_use.node` 已有 `captureExcluding/captureWindow/captureRegion` | XS |
| 8 | `SkyComputerUseService`（执行器本体） | ❌ 不可复用 | teamID 校验 + TCC 身份 | — |
| 9 | `CUALockScreenGuardian` + 授权插件 | ❌ 不建议 | 需 root、SecurityAgent 插件、越界 | — |
| 10 | 浏览器扩展（MV3）+ native messaging 协议 | ✅ 直接搬 | 自建扩展 + 自建 NMH，协议一个字节都不用改 | M |
| 11 | `browser-service.mjs` 的 command set / 后端抽象 | ✅ 可搬 ~70% | 换掉 `nodeRepl.rpc` 依赖 | M |
| 12 | tab groups 管理器（`Kb` 单例 + 白名单 + 审计） | ✅ 近乎逐行复用 | 纯 `chrome.*` API | S |
| 13 | `Emulation.setFocusEmulationEnabled` 后台招 | ✅ 直接抄 | 扩展 `chrome.debugger` 或 IAB `webContents.debugger` | XS |
| 14 | `node_repl` 特权管道 / app-server 协议 / `codexHome` 契约 | ❌ 不可搬 | 用 Claude 的 MCP + stdio 替代 | — |
| 15 | IAB（`WebContentsView` + Electron CDP） | ✅ 可搬（Electron 44 支持） | 但**不提供原生 tab groups** | M |

---

## 7. 三条落地路线

### 路线 A（推荐先做）：纯外挂 —— 零改 Claude.app

```
Claude.app ── mcpServers(stdio) ──► your-cu-server（Node/Python）
                                      ├─ 自写 helper.app（Swift：AXPress + CGEventPostToPid + SCScreenshotManager + 虚拟光标）
                                      └─ chrome bridge ──► 你的 Chrome 扩展 + native host
```

1. 建 `cu-helper.app`（Swift，`LSUIElement`，自己的 bundle id）：实现 `probe` / `dispatchRaw` 两个 JSON-RPC 方法，内部 AX 树导出（稳定 element_index）、`AXPress`、`CGEventPostToPid`（坐标点击/键盘）、`SCScreenshotManager` 截图（排除自己窗口）、虚拟光标 overlay。
2. 建 `cu-mcp`（stdio MCP），把 Claude 的工具面映射到 helper。
3. 写进 `claude_desktop_config.json` 的 `mcpServers`（或用 plugin 的 `.mcp.json` 分发）。
4. 首次运行引导授权「辅助功能 + 屏幕录制」。
5. 建 Chrome MV3 扩展：`permissions: [tabs, tabGroups, windows, scripting, nativeMessaging, debugger, webNavigation, storage]`，`background.js` 照抄 Codex 的 `ku` 白名单 + `Kb` 分组管理器。
6. 建 native host，把 NMH manifest 写到 `~/Library/Application Support/{Google/Chrome,Microsoft Edge,BraveSoftware/Brave-Browser}/NativeMessagingHosts/com.<you>.bridge.json`（`allowed_origins` 填自己的扩展 ID）。
7. host ↔ 扩展：native messaging（4B LE + JSON-RPC 2.0）；host ↔ cu-mcp：UDS + token。
8. 工具面暴露 `browser_tabs_*` / `browser_group_*` / `browser_page_*`；页面操作走 `chrome.debugger` + `Emulation.setFocusEmulationEnabled`。

工作量：helper 1.5–2 人周、MCP 0.5、扩展 2、host 1.5、联调 1 → **约 6 人周**；先做「CU（helper + MCP）」两周内可用。

风险：扩展要 `debugger` + `<all_urls>`，商店审核严；`chrome.debugger` 与用户 DevTools 互斥（抢 attach 会掉），需 fallback 到 `scripting.executeScript`。

### 路线 B（最快见效）：只补「Chrome + 分组」，CU 用 Claude 自带的

- Claude.app 已有 `app-cu-helper` + `computer_use.node` + `computer_*` 工具；**CU 不需要动**。
- 只做 §7A 的第 5–8 步（扩展 + host + MCP）。
- 甚至可以先不写 host：扩展直接连 `ws://127.0.0.1:<port>?token=`（MCP server 兼做 WS 服务端），省掉 NMH 注册。

工作量：**2–3 人周**。

### 路线 C（不推荐）：改 Claude.app 本体

- 「同长度热补」：把 `.vite/build/*.js` 里的门禁字符串等长替换（asar 校验只覆盖 header，签名仍有效）；适合**改开关**，不适合加功能。
- 「重打包 + 重签」：解 asar → 加模块 → 重算 header hash → 改 `Info.plist:ElectronAsarIntegrity` → 补 `disable-library-validation` → `codesign --force --deep --sign -`。会丢公证、每次更新要重做。
- 只有在需要「复用官方 CU 的 UI/IPC/审批流」时才值得。

---

## 8. 明确不要做的事

1. **不要直连 Codex 的 `computeruse.sock`**：服务端做 `LOCAL_PEERTOKEN` + 代码签名/teamID 校验（`SkyIPCRequirement`、`_senderParentTeamID`、`_senderResponsibleTeamID`），只认 `2DC432GLL2`。socket 是 `srw-------`（同用户可连），但握手会被签名校验挡掉。
2. **不要复制 `Codex Computer Use.app` 当自己的 helper**：签名/公证属 OpenAI，重签后 TCC 会重新授权，且内部 SAI / 授权插件路径对身份敏感。
3. **不要照搬锁屏/授权插件**（SecurityAgent 插件 + guardian）：需 root，收益低、风险高。
4. **不要指望 CDP 做原生标签分组**：`Target` 域没有 group 语义；Codex 自己在 IAB 里都禁用了 `Target.*`（除 `setAutoAttach`）。
5. **不要把 `browser-background-switches` 当「后台能力」**：它只是 Chromium 启动 flag 门控；真正的后台来自 `active:false` + `focused:false` + `Emulation.setFocusEmulationEnabled` + `chrome.debugger`。

---

## 9. 附录：速查

### 9.1 关键路径

```
# Codex
~/.codex/plugins/cache/openai-bundled/unified-computer-use/<ver>/.mcp.json          # CU 的 MCP 注册
~/.codex/plugins/cache/openai-bundled/chrome/<ver>/scripts/browser-service.mjs     # 浏览器服务
~/.codex/computer-use/Codex Computer Use.app/Contents/MacOS/SkyComputerUseService  # CU 执行器
~/Library/Group Containers/2DC432GLL2.com.openai.sky.CUAService/IPC/computeruse.sock
~/.codex/chrome-native-hosts-v2.json                                               # NMH 注册表
/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/{sky,cua,cua-repl,browser-desktop}

# Claude
/Applications/Claude.app/Contents/Helpers/{app-cu-helper,chrome-native-host,permission-fixer}
/Applications/Claude.app/Contents/Resources/app.asar.unpacked/node_modules/@ant/claude-swift/build/Release/computer_use.node
~/Library/Application Support/Claude/claude_desktop_config.json
```

### 9.2 协议字段

```jsonc
// sky → service（4 字节 LE 长度前缀 + UTF-8 JSON）
{"id":1,"jsonrpc":"2.0","method":"request","params":{
  "clientApiVersion":123,"codexTurnMetadata":{},"deadlineUnixMilliseconds":0,
  "request":{ /* click/pressKey/typeText/scroll/drag/setValue/paste/selectText/performSecondaryAction/getAppState/listApps */ },
  "requestType":"jsonRPC"}}
{"id":1,"jsonrpc":"2.0","method":"ping","params":{"clientApiVersion":123}}
// 应答：{"id":..,"jsonrpc":"2.0","result":{...}} / {"error":{"code","message"}}
// 单帧上限 8 MiB（8388608）；版本不匹配 → ServerErrorCode.incompatibleClientVersion
```

```jsonc
// Chrome 扩展 ↔ host（native messaging，4 字节 LE + JSON-RPC 2.0）
{"jsonrpc":"2.0","id":7,"method":"executeCdp","params":{"target":{"tabId":12},
  "method":"Emulation.setFocusEmulationEnabled","commandParams":{"enabled":true}}}
// tab groups
{"jsonrpc":"2.0","id":8,"method":"browser_management_call","params":{"namespace":"tabs",
  "method":"group","args":[{"tabIds":[7,9],"createProperties":{"windowId":123}}]}}
{"jsonrpc":"2.0","id":9,"method":"browser_management_call","params":{"namespace":"tabGroups",
  "method":"update","args":[{"groupId":42,"title":"会话标题","color":"blue","collapsed":false}]}}
```

### 9.3 关键字符串（快速定位）

```
# Codex CU
computer-use-native-pipe / computer-use-native-pipe-server / computer-use-approval:
Sky Computer Use native pipe / SAIVirtualKeyPress / VirtualCursor / AXMenuItemCmdVirtualKey
Enable the virtual cursor in Computer Use. / Show backgrounded apps that Computer Use is working on in Picture-in-Picture mode.
# Codex Browser
chrome / iab / dot / cdp / mcpapps / Emulation.setFocusEmulationEnabled / connectNative
com.openai.codexextension / TAB_GROUPS / BROWSER_MANAGEMENT_AUDIT_TRAIL
# Claude
app-cu-helper / dispatchRaw / viaWinLocalPts / SLEventPostToPid / PhantomCursor / armFocusStealGuard
chrome-native-host / computer:request_access / claude.internal.computerUse
```
