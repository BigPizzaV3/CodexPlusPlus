// 此文件由 scripts/assemble-whale-runtime.mjs 从上游完整源码生成，请修改生成器。
(function () {
if (window.__dshWhaleWidget) return
window.__dshWhaleWidget = true

// —— 页面自检：只在 DSH 主聊天界面挂载挂件 ——
// 挂件脚本通过 tapIndex 注入 DSH 的每一个 index 页面（含插件市场等 SPA 视图）。
// 市场页用 ReactDOM.createPortal 渲染到 document.body，若挂件在此初始化，
// 会在 body 插入节点并注册全局捕获拦截，干扰 React 渲染树（removeChild 报错、页面空白）。
// 主聊天界面的特征：composer 输入区。三种形态都算主界面：
//   (a) 旧版 textarea
//   (b) 旧版 contenteditable 可编辑 div
//   (c) **DSH 0.1.6-alpha.1 起的新版**：<div contenteditable="false" role="textbox"
//       aria-multiline="true" data-composer-input="true">（编辑由 Lexical 接管，所以
//       contenteditable 反而是 false —— 只认前两种会让挂件在新版 DSH 上**完全不初始化**，
//       见 issue #123）。检测到才继续，否则不碰 DOM、不注册监听。
function dshwIsChatRoot(r) {
  if (!r || !r.querySelector) return false
  return !!(
    r.querySelector('textarea') ||
    r.querySelector('[contenteditable="true"]') ||
    // DSH 0.1.6-alpha.1 起的新版 composer：data-composer-input 在整个 DSH 前端产物里
    // **只出现在对话组件**（v742 时实测：1515 个前端文件里仅 dsh-client-ui-conversation 命中），
    // 所以它是最精确的判据。
    r.querySelector('[data-composer-input]') ||
    // 同一层的 composer 容器属性（data-composer-seat / -card 也各只有一个出处），
    // 万一将来输入框本身的属性改名，容器还在就仍能识别。
    r.querySelector('[data-composer-seat],[data-composer-card]') ||
    // 通用兜底：多行 textbox。注意**不要**用裸的 [role="textbox"] —— pdf.js 的批注编辑器
    // （dsh-client-ui-sidebar-documentpreview）也会设 role=textbox，用它会把判定放宽到
    // 非输入框的元素；加上 aria-multiline 至少排除掉单行输入框。
    r.querySelector('[role="textbox"][aria-multiline="true"]')
  )
}
var dshwStarted = false
function dshwStartOnce() {
  if (dshwStarted) return
  dshwStarted = true
  try { dshwInit() } catch (err) { window.__codexPlusWhaleHost.fail(err) }
}
// 是否已到主聊天界面；是则启动（只启动一次，之后由 dshwInit 内部标记去重）
var dshwLastCheck = 0
function dshwTryStart(force) {
  if (dshwStarted) return true
  var now = Date.now()
  // 连续 DOM 变化时合并检查，避免每次 mutation 都 querySelector
  if (!force && now - dshwLastCheck < 200) return false
  dshwLastCheck = now
  try {
    if (window.__codexPlusWhaleHost.active()) { dshwStartOnce(); return true }
  } catch (err) {}
  return false
}
try {
  if (false) { // Codex++ 已由 enhancement 生命周期判断挂载时机
    // 尚未渲染：MutationObserver 无限等待（v739 去掉原来的「5 秒死线」）。
    // 原来 500ms × 10 次就永久放弃，而 window.__dshWhaleWidget 是一次性闸门 ——
    // 慢启动机器、或页面最小化时定时器被浏览器节流，都会让"第一次没赶上"变成
    // "这次会话永远不出现"（用户反馈 / issue #102 第 2 条）。
    // 约束不变：检测到 composer 之前一行 DOM 都不碰、不注册任何全局监听。
    var dshwObserver = null
    try {
      if (typeof MutationObserver === 'function') {
        dshwObserver = new MutationObserver(function () {
          if (dshwTryStart()) { try { dshwObserver.disconnect() } catch (err) {} }
        })
        dshwObserver.observe(document.documentElement || document.body, { childList: true, subtree: true })
      }
    } catch (err) {}
    // 兜底：observer 不可用时低频轮询继续等（不设上限，找到即停）
    var dshwFallbackPoll = setInterval(function () {
      if (dshwTryStart(true)) {
        clearInterval(dshwFallbackPoll)
        try { if (dshwObserver) dshwObserver.disconnect() } catch (err) {}
      }
    }, 2000)
  }
} catch (err) {}
function dshwInit() {
if (window.__dshWhaleInit) return
window.__dshWhaleInit = true
var codexWhaleHost = window.__codexPlusWhaleHost

// ===== 音效播放（v745：Web Audio + 预解码 + 同步起播 + 可调衔接）=====
// 目标：既要 0.3.0 那种"贴手"的响应，又不让 macOS 把音效注册进系统「正在播放」（Touch Bar 播放条 + 卡顿）。
// 之前 0.3.3~0.3.7 换成 Web Audio 后手感变钝，不是引擎的问题，而是丢了四样里的三样：
//   ① 垫片的 preload='auto' 只是占位 → 不预取、不解码 → 第一次点按要现 fetch+decode；
//   ② 垫片没有 duration、currentTime 也不前进 → 点按退化成 onended 兜底「按压音放完才接松开音」；
//   ③ 起播要经过一次 Promise/微任务。
// 现在四件齐备：
//   ① 引擎仍是不经过 media element 的 AudioBufferSourceNode（系统媒体控件不会出现 → 无 Touch Bar 条）；
//   ② dshwvWarm() 在切音效组 / 打开试听 / 页面初始化时就**预取 + 预解码**（URL 级缓存，只解一次）；
//   ③ 补齐 duration / currentTime（播放中真实前进）→ 能算"按压音还剩多久"；
//   ④ **同步起播**：缓冲区已预热时，start() 在 pointerdown 的**同一个任务**里调用（不再经过 Promise）。
// 衔接时机由 RELEASE_LEAD_MS 决定（0 = 正好接上；30/50 = 轻微交叠）—— 改这个数字即可按耳朵微调，
// 不用动任何逻辑。
var dshwvAudioCtx = null
// v753（issue #135）：**running 状态的 AudioContext 会让系统一直挂着 PreventUserIdleSystemSleep** ——
// macOS 上表现为"只要页面开着就不会空闲睡眠"，而且与**有没有出声无关**（context 一 running 就持有播放流）。
// 所以空闲 DSHW_AUDIO_IDLE_MS 之后主动 suspend()；下次出声前 dshwvAudio() 里的 resume() 会自动恢复。
// 为什么用 suspend() 而不是 close()：close() 会把 context 彻底销毁，而**任务结束音不是手势触发的**，
// 销毁后它再也响不出来（被手势解锁过的 context 再 resume 不需要新手势，挂起是安全的）。
var DSHW_AUDIO_IDLE_MS = 60000 // 静默多久交还系统睡眠（用户选定：1 分钟）
var dshwvAudioSuspendT = null
function dshwvAudioSuspendNow() {
  try {
    if (dshwvAudioSuspendT) { clearTimeout(dshwvAudioSuspendT); dshwvAudioSuspendT = null }
    if (dshwvAudioCtx && dshwvAudioCtx.state === 'running') dshwvAudioCtx.suspend()
  } catch (err) {}
}
function dshwvAudioIdleArm() {
  try {
    if (dshwvAudioSuspendT) clearTimeout(dshwvAudioSuspendT)
    dshwvAudioSuspendT = setTimeout(function () { dshwvAudioSuspendT = null; dshwvAudioSuspendNow() }, DSHW_AUDIO_IDLE_MS)
  } catch (err) {}
}
// 音效是否被**显式**关掉（v753 的显式开关）。这里刻意用 typeof 保护：
// 本文件经常被离线自检按片段切出来单独 eval（例如只切「音频垫片 + applySoundSet」那段），
// 那些沙箱里不一定声明了 soundOn —— typeof 对未声明的标识符是安全的，裸引用会直接抛
// ReferenceError 把整段沙箱打断。真机上 soundOn 一定存在，行为不受影响。
function dshwvSoundOff() {
  try { return typeof soundOn !== 'undefined' && soundOn === false } catch (err) { return false }
}
function dshwvAudio() {
  try {
    if (!dshwvAudioCtx) {
      var AC = window.AudioContext || window.webkitAudioContext
      // 显式用 'interactive'（该 API 的最低延迟档），起播尽量贴手
      dshwvAudioCtx = new AC({ latencyHint: 'interactive' })
    }
    if (dshwvAudioCtx.state === 'suspended') { try { dshwvAudioCtx.resume() } catch (err) {} }
    dshwvAudioIdleArm() // 每次（重新）进入 running 都重新计时：静默满 1 分钟就挂起
    return dshwvAudioCtx
  } catch (err) { return null }
}
var dshwvAudioBuffers = {} // url -> Promise<AudioBuffer>（同一片段不重复下载/解码）
var dshwvAudioDecoded = {} // url -> AudioBuffer（解码完成后**同步可读**：起播走同步路径的关键）
// v752：音频失败**必须看得见**。原来所有 fetch/decode 失败都被 `.catch(function(){})` 吞掉，
// 于是"没声音"在控制台里一点痕迹都没有，只能靠猜。现在每个 URL 只 warn 一次，
// 并把最后一次错误留在 dshwvAudioLastErr 里。
var dshwvAudioLastErr = null
var dshwvAudioWarned = {}
function dshwvAudioWarn(url, err) {
  var msg = String((err && err.message) || err || 'unknown')
  dshwvAudioLastErr = { url: String(url || ''), error: msg, at: new Date().toISOString() }
  if (dshwvAudioWarned[url]) return
  dshwvAudioWarned[url] = 1
  try { console.warn('[小鲸鱼] 音频加载/解码失败：' + url + ' → ' + msg) } catch (e) {}
}
function nowMs() { try { return (typeof performance !== 'undefined' && performance.now) ? performance.now() : Date.now() } catch (err) { return Date.now() } }
function dshwvAudioBuffer(url) {
  if (!url) return Promise.reject(new Error('empty url'))
  if (!dshwvAudioBuffers[url]) {
    // v752：改 no-store。原来用 force-cache —— 万一某个中间层把一次空的 204 缓存住，
    // 之后每次点按都会复用它（表现就是"永久没声音，重启也没用"）。我们本来就有内存解码缓存，
    // 不会因此重复下载。
    dshwvAudioBuffers[url] = fetch(url, { cache: 'no-store' })
      .then(function (r) {
        // 注意：204 也满足 r.ok（2xx），必须单独挡掉，否则 0 字节会被送进 decodeAudioData
        // → 抛 EncodingError（"Unable to decode audio data"）→ 只剩静音。
        if (r.status === 204) throw new Error('HTTP 204（无内容：该音效槽留空、或音效组未找到）')
        if (!r.ok) throw new Error('HTTP ' + r.status)
        return r.arrayBuffer()
      })
      .then(function (raw) {
        if (!raw || raw.byteLength < 100) throw new Error('音频响应只有 ' + (raw ? raw.byteLength : 0) + ' 字节（不是有效音频）')
        var c = dshwvAudio()
        if (!c) throw new Error('no audio context')
        return new Promise(function (res, rej) { c.decodeAudioData(raw, res, rej) })
      })
      .then(function (buf) { dshwvAudioDecoded[url] = buf; return buf })
      .catch(function (err) { delete dshwvAudioBuffers[url]; delete dshwvAudioDecoded[url]; dshwvAudioWarn(url, err); throw err })
  }
  return dshwvAudioBuffers[url]
}
// 预热：垫片的 preload='auto' 在 Web Audio 下不解码，必须显式预取+预解码。
// 不预热 → 第一次点按要现 fetch+decodeAudioData，表现就是"按下音慢半拍、中间衔接发飘"。
// URL 级缓存保证同一片段只解一次；失败静默吞掉（真正播放时还会自己重试一次）。
function dshwvWarm(urls) {
  // v753（issue #135）：音效关掉时**连预热都不做** —— 只跳过下面那句 dshwvAudio() 是不够的，
  // 因为预解码走到 dshwvAudioBuffer() 里还会再调一次 dshwvAudio()，context 照样被建起来。
  if (dshwvSoundOff()) return
  try { dshwvAudio() } catch (err) {}
  for (var i = 0; i < (urls || []).length; i++) {
    var u = urls[i]
    if (!u) continue
    try { dshwvAudioBuffer(u).catch(function () {}) } catch (err) {}
  }
}
function dshwvSoundStop(el) {
  el._token = (el._token || 0) + 1
  var node = el._node
  el._node = null
  el._playingSince = 0
  if (node) {
    try { node.onended = null } catch (err) {}
    try { node.stop() } catch (err) {}
  }
}
// v766：**试听名单** —— 供"停止试听"用。为什么要它：`dshwvSound()` 每次都返回一个全新元素，
// 而任务结束音 / 片段 / 音效组三条试听路径都是**匿名调用**（外面拿不到句柄），播起来之后就停不掉了。
// 这里只登记 `dshwvPreviewMark > 0` 期间创建的元素 —— 也就是**只登记试听**，绝不误伤真实事件音
// （真实的任务结束音 / 提问授权提示音是用户要听的，不能在面板里被顺手掐掉）。
var dshwvPreviewEls = []
var dshwvPreviewMark = 0
function dshwvPreviewOn() { dshwvPreviewMark++ }
function dshwvPreviewOff() { dshwvPreviewMark = Math.max(0, dshwvPreviewMark - 1) }
function dshwvPreviewTrack(el) {
  if (!(dshwvPreviewMark > 0)) return
  try {
    dshwvPreviewEls.push(el)
    if (dshwvPreviewEls.length > 12) {
      // 播完的（_node 已清空）不再留着，避免名单无限增长
      dshwvPreviewEls = dshwvPreviewEls.filter(function (x) { return x && x._node })
    }
  } catch (err) {}
}
function dshwvStopPreviews() {
  try {
    for (var i = 0; i < dshwvPreviewEls.length; i++) {
      try { dshwvSoundStop(dshwvPreviewEls[i]) } catch (err) {}
    }
    dshwvPreviewEls = dshwvPreviewEls.filter(function (x) { return x && x._node })
  } catch (err) {}
}
function dshwvSound(url) {
  var el = { preload: 'auto', volume: 1, onended: null, loop: false, _url: String(url || ''), _node: null, _gain: null, _offset: 0, _token: 0, _playingSince: 0 }
  Object.defineProperty(el, 'src', {
    get: function () { return el._url },
    set: function (v) { dshwvSoundStop(el); el._url = String(v || ''); el._offset = 0 },
  })
  Object.defineProperty(el, 'currentTime', {
    // 播放中真实前进：pressUp 靠它算"按压音还剩多久"，据此把松开音排到按压音结束（或提前 lead）那一刻
    get: function () {
      if (el._node && el._playingSince) {
        var t = el._offset + (nowMs() - el._playingSince) / 1000
        var d = el.duration
        return Math.max(0, isFinite(d) && d > 0 ? Math.min(t, d) : t)
      }
      return el._offset
    },
    // 既有逻辑用「currentTime = 0」表示重播 → 这里顺手停掉正在播的那一份
    set: function (v) { el._offset = Number(v) || 0; el._playingSince = 0; dshwvSoundStop(el) },
  })
  Object.defineProperty(el, 'duration', {
    // 与 HTMLAudioElement 对齐：未解码时 NaN（调用方用 isFinite 判定），解码后是真实时长
    get: function () {
      var b = dshwvAudioDecoded[el._url]
      return b && isFinite(b.duration) && b.duration > 0 ? b.duration : NaN
    },
  })
  // 拿到 buffer 后真正起播。delay>0 时用音频线程时间轴（start(when)）排期，不受主线程抖动影响。
  function beginWithBuffer(c, buf, delay) {
    try {
      if (!el._gain) { el._gain = c.createGain(); el._gain.connect(c.destination) }
      el._gain.gain.value = Math.max(0, Math.min(1, Number(el.volume) || 0))
      var src = c.createBufferSource()
      src.buffer = buf
      src.connect(el._gain)
      src.onended = function () {
        if (el._node !== src) return
        el._node = null
        el._playingSince = 0
        if (typeof el.onended === 'function') { try { el.onended() } catch (err) {} }
      }
      el._node = src
      el._playingSince = nowMs() + delay * 1000
      var dur = Math.max(0.001, buf.duration)
      src.start(delay > 0 ? c.currentTime + delay : 0, Math.max(0, el._offset) % dur)
    } catch (err) {
      // v749：起播抛异常（例如 AudioContext 已关闭、offset 越界）以前被完全吞掉 → 表现为"没声音但无任何报错"
      dshwvAudioWarn(el && el._url, err)
    }
  }
  function startSound(delaySec) {
    // v753（issue #135）：音效开关关掉 = 全静音（含编辑器里的试听），也**不碰** AudioContext ——
    // 否则一次试听就会把 context 转成 running、系统断言照挂。
    if (dshwvSoundOff()) return Promise.resolve()
    var c = dshwvAudio()
    if (!c || !el._url) return Promise.resolve()
    var token = (el._token = (el._token || 0) + 1)
    var delay = Math.max(0, Number(delaySec) || 0)
    // ④ 同步起播：缓冲区已预热（dshwvWarm）过 → 直接在当前任务里 start()，不再等 Promise
    var cached = dshwvAudioDecoded[el._url]
    if (cached) { beginWithBuffer(c, cached, delay); return Promise.resolve() }
    var tryUrl = el._url
    dshwvAudioBuffer(tryUrl).then(function (buf) {
      if (token !== el._token) return // 期间被重播/暂停/换源 → 丢弃这次
      beginWithBuffer(c, buf, delay)
    }).catch(function () {
      // v752：首选路由取不到音频时，自动换另一条路由重试一次（两条互为备胎）。
      // 只在失败路径上跑，不影响正常点按的同步起播时序。
      if (!el._alt || el._url !== tryUrl || token !== el._token) return
      var alt = el._alt
      el._alt = ''            // 只回退一次，避免两条路由来回打转
      el._url = alt
      try { dshwvWarm([alt]) } catch (e) {}   // 后台预热，下次点按就能走同步路径
      try { startSound(delay) } catch (e) {}
    })
    return Promise.resolve()
  }
  el.play = function () { return startSound(0) }
  el.playAt = function (delaySec) { return startSound(delaySec) }
  el.pause = function () { dshwvSoundStop(el) }
  // v752：备用路由（首选失败时自动切过去）；由 applySoundSet/playTaskEndGroupClick 填
  el._alt = ''
  dshwvPreviewTrack(el) // v766：只在试听期间登记（见上面的试听名单）
  return el
}

// 自动播放策略：AudioContext 初始是 suspended，要有一次用户手势才能出声；任务结束音不是手势触发的，
// 所以挂一次性解锁（首次点击/按键后移除）。
// v789（issue #179）：**输入法组字期间一律不做事**。用户实测"装了插件后微软拼音打字会自动上屏、
//   光标跳到最前端"（0.3.16/0.3.17 均复现，卸载即恢复）。我们能做的、也是最有把握的一条，是
//   让挂件在 composition 期间**完全不碰 DOM、不做重活**：往 document.body 挂节点会触发布局，
//   而首次按键同步创建/预热 AudioContext 也会让组字被打断。所以：
//   ① 记录 composition 状态，组字中的 keydown 不解锁音频；
//   ② 组字期间所有 body 增删一律排队，等 compositionend 之后再补（见 dshwBodyAppend）。
var dshwvComposing = false
var dshwvDeferredBody = []
function dshwvComposingNow() { return dshwvComposing === true }
try {
  document.addEventListener('compositionstart', function () { dshwvComposing = true }, true)
  document.addEventListener('compositionend', function () {
    dshwvComposing = false
    setTimeout(function () { try { dshwvFlushDeferredBody() } catch (err) {} }, 0)
  }, true)
} catch (err) {}
try {
  var dshwvAudioUnlock = function (ev) {
    // v789（issue #179）：组字中的按键不做音频解锁（isComposing / keyCode 229 = IME 处理中）
    try { if (dshwvComposing || (ev && (ev.isComposing === true || ev.keyCode === 229))) return } catch (err) {}
    // v753（issue #135）：音效关掉时**不预解锁** —— 否则一次普通点击就会把 context 转成 running，
    // 断言照挂。注意这里**不摘监听**：之后重新打开开关，下一次点击仍能完成解锁。
    if (dshwvSoundOff()) return
    dshwvAudio()
    try { document.removeEventListener('pointerdown', dshwvAudioUnlock, true) } catch (err) {}
    try { document.removeEventListener('keydown', dshwvAudioUnlock, true) } catch (err) {}
  }
  document.addEventListener('pointerdown', dshwvAudioUnlock, true)
  document.addEventListener('keydown', dshwvAudioUnlock, true)
  // v753（issue #135）：页面被隐藏（切标签 / 最小化）时立刻挂起，直接覆盖"开着过夜"这个场景
  document.addEventListener('visibilitychange', function () {
    try { if (document.hidden) dshwvAudioSuspendNow() } catch (err) {}
  })
} catch (err) {}

var MIN_SCALE = 0.6
var MAX_SCALE = 2.5
var STEP = 0.1
var CLICK_SQ = 9
var REFRESH_MS = 60000
var CHANGE_MS = 900
var ANIM_MS = 700
var BUBBLE_MS = 5000
var FETCH_TIMEOUT_MS = 25000
var BALANCE_URL = '/dsh-whale/balance.json'
var SIZE_URL = '/dsh-whale/size.json'
var IMG_URL = '/dsh-whale/image.png?v=2'
var GIF_URL = '/dsh-whale/rua.gif'
var BUBBLE_URL = '/dsh-whale/bubble.json'

var css = [
  '.dshwv-root{position:fixed;right:0;bottom:0;--dshw-scale:1;--dshw-base:clamp(122px,calc(min(250px,min(100vw,100vh) * 0.28) * var(--dshw-scale)),625px);width:var(--dshw-base);height:var(--dshw-base);pointer-events:none;user-select:none;-webkit-user-select:none;z-index:9999;font-family:inherit;transition:left .16s ease,top .16s ease,transform .3s ease}',
  // v634 移动端:去掉浏览器「点击高亮」方块——它画在可点元素的矩形包围盒上,
  // 泡泡的内联 SVG 形状尤其明显;同时禁掉 iOS 长按系统菜单/放大镜。
  // 只作用于挂件自身的 dshwv- 元素(该属性可继承,后代一并覆盖),不影响 DSH 页面自身的高亮。
  'html [class*="dshwv-"],html [class*="dshwv-"] *{-webkit-tap-highlight-color:transparent;-webkit-touch-callout:none}',
  '.dshwv-root.dshwv-left{transform:scaleX(-1)}',
  '.dshwv-root.dshwv-dragging{cursor:grabbing;transition:none}',
  '.dshwv-body{position:absolute;left:0;top:0;width:100%;height:100%;transform-origin:50% 100%;transition:transform .22s cubic-bezier(.34,1.56,.64,1)}',
  // v757（issue #147）：`.dshwv-img` 由 `pointer-events:none` 改为 `auto` —— 鲸鱼身体**自己接指针事件**。
  // 原来靠"主文档监听 + isWhaleHit()"判定命中，而指针落在 `<iframe>`（如右侧栏 HTML 预览）上时
  // 事件直接进入 iframe 自己的文档，主文档收不到 ⇒ 鲸鱼身体失联（点不动、拖不动，只有 ☰ 能用）。
  // 代价是 img 的**矩形**（含透明边距）会吞掉点击 → 由 setupHitTest() 用命中图的**凸包**做 clip-path
  // 裁掉透明区（凸包包含全部不透明像素，不会裁到角色本身），"点到透明处穿透到下层"的行为得以保留。
  '.dshwv-img{position:absolute;right:0;bottom:0;width:59.45%;height:59.45%;display:block;pointer-events:auto;-webkit-user-drag:none;user-select:none;object-fit:contain;object-position:right bottom}',
  // v751（PR #119）：光标不再写 document.body.style.cursor —— cursor 是可继承属性，写 <body> 会让 Blink
  // 失效**整棵文档树**的样式；而它在点击链路上按下/抬手各写一次，紧接着 isWhaleHit() 的
  // getBoundingClientRect() 与泡泡行测量的 getComputedStyle()/scrollWidth 会强制刷新样式+布局，
  // 于是"整页样式重算"被算进了这一次点击（长会话里表现为桌宠点了卡一下、松手音延迟、跑马灯不动）。
  // 现在光标由挂件自己的类承担：命中鲸鱼不透明像素时让 .dshwv-img 接过指针并显示 grab / grabbing。
  // 鲸鱼区域本来就在吞指针事件（onDocPointerDown 的 isWhaleHit），这不是新增拦截；
  // 新增影响的只有滚轮，由 onWhaleWheel 转交给指针下方真正可滚动的容器。
  '.dshwv-root.dshwv-cursor-grab .dshwv-img{pointer-events:auto;cursor:grab}',
  '.dshwv-root.dshwv-cursor-grabbing .dshwv-img{pointer-events:auto;cursor:grabbing}',
  // 拖动期间握点相对挂件固定，让整个盒接过指针以保持 grabbing（类由按下时加、endDrag 摘）
  '.dshwv-root.dshwv-dragging .dshwv-body{pointer-events:auto;cursor:grabbing}',
  '.dshwv-pop{position:absolute;left:0;top:0;width:100%;aspect-ratio:1026/700;pointer-events:none;z-index:1;--dshw-u:calc(var(--dshw-base) / 1026)}',
  // 纵深防御：泡泡容器必须透明，形状由内部 SVG 绘制；用 !important 压掉外部
  // 插件“类名子串匹配”选择器（如 aqua 的 [class*=bubble]）注入的玻璃/边框样式
  'html .dshwv-pop, html .dshwv-pop svg{background:transparent !important;border:0 !important;border-radius:0 !important;backdrop-filter:none !important;-webkit-backdrop-filter:none !important}',
  // 纵深防御（issue #133）：宿主皮肤/主题会按**几何特征**（position:fixed + z-index≥10 +
  // 尺寸≥120×80 + 非原生弹窗角色）把挂件方块误判成"独立插件窗"，然后注入
  // backdrop-filter / 背景 / 边框 / 伪元素毛玻璃，把用户壁纸糊掉（实测 375×375 方块里是雾面）。
  // 挂件是透明精灵图，根节点上任何"画底"的通道都不该存在，所以这里全部钉死为透明 ——
  // 我们自己的 .dshwv-root 本来就只有 left/top/width/height/transform，没有背景/边框/阴影/filter，
  // 也没有用 ::before/::after，所以这是零行为变化的加固。
  'html .dshwv-root,body .dshwv-root{background:transparent !important;background-image:none !important;border:0 !important;outline:0 !important;box-shadow:none !important;backdrop-filter:none !important;-webkit-backdrop-filter:none !important;filter:none !important}',
  'html .dshwv-root::before,html .dshwv-root::after{content:none !important;background:transparent !important;background-image:none !important;box-shadow:none !important;backdrop-filter:none !important;-webkit-backdrop-filter:none !important}',
  '.dshwv-pop svg{display:block;width:100%;height:100%;pointer-events:none}',
  '.dshwv-pop svg path,.dshwv-pop svg ellipse{pointer-events:none;cursor:pointer}',
  '.dshwv-pop.dshwv-pop-open svg path,.dshwv-pop.dshwv-pop-open svg ellipse{pointer-events:visiblePainted}',
  '.dshwv-pop .dshwv-bshape,.dshwv-pop .dshwv-b1,.dshwv-pop .dshwv-b2{opacity:0;transform:scale(.7);transform-box:fill-box;transform-origin:50% 50%;transition:opacity .2s ease,transform .2s ease}',
  '.dshwv-pop.dshwv-pop-open .dshwv-bshape,.dshwv-pop.dshwv-pop-open .dshwv-b1,.dshwv-pop.dshwv-pop-open .dshwv-b2{opacity:1;transform:none}',
  '.dshwv-gif{position:absolute;left:var(--dshw-vx,44.25%);top:var(--dshw-vy,36%);transform:translate(-50%,-50%);max-width:calc(var(--dshw-u) * 560);max-height:calc(var(--dshw-u) * 400);display:none;opacity:0;transition:opacity .2s ease;pointer-events:none;-webkit-user-drag:none;user-select:none;object-fit:contain}',
  '.dshwv-root.dshwv-left .dshwv-gif{transform:translate(-50%,-50%) scaleX(-1)}',
  '.dshwv-pop.dshwv-pop-open .dshwv-gif{opacity:1}',
  '.dshwv-pop.dshwv-pop-open .dshwv-b2{transition-delay:0s}',
  '.dshwv-pop.dshwv-pop-open .dshwv-b1{transition-delay:.13s}',
  '.dshwv-pop.dshwv-pop-open .dshwv-bshape{transition-delay:.26s}',
  '.dshwv-pop .dshwv-bshape{transition-delay:.1s}',
  '.dshwv-pop .dshwv-b1{transition-delay:.2s}',
  '.dshwv-pop .dshwv-b2{transition-delay:.3s}',
  '.dshwv-text{position:absolute;left:var(--dshw-vx,44.25%);top:var(--dshw-vy,36%);width:66%;height:64%;transform:translate(-50%,-50%);display:flex;flex-direction:column;align-items:center;justify-content:center;text-align:center;color:#536ba9;line-height:1.15;white-space:nowrap;pointer-events:none;opacity:0;transition:opacity .16s ease,transform .3s ease}',
  '.dshwv-pop.dshwv-pop-open .dshwv-text{opacity:1;transition:opacity .16s ease .36s,transform .3s ease}',
  '.dshwv-root.dshwv-left .dshwv-text{transform:translate(-50%,-50%) scaleX(-1)}',
  // 大号字体垂直居中修正：flex 容器以 --dshw-vx/--dshw-vy 为整体中心，
  // 内容(任意行数/字号)由 justify-content:center 整体居中，不随字体度量漂移。
  '.dshwv-text .dshwv-trow{flex:0 0 auto;margin:calc(var(--dshw-u) * 2) 0}',
  '.dshwv-text .dshwv-mimg{flex:0 0 auto}',
  // label/amount/hint 三行也作为整体在 flex 容器内居中
  '.dshwv-text .dshwv-label,.dshwv-text .dshwv-amount,.dshwv-text .dshwv-hint{flex:0 0 auto;margin-left:auto;margin-right:auto}',
  '.dshwv-label{font-size:calc(var(--dshw-u) * 66);font-weight:600;letter-spacing:.06em}',
  '.dshwv-amount{font-size:calc(var(--dshw-u) * 128);font-weight:800;line-height:1.05}',
  '.dshwv-period{font-size:calc(var(--dshw-u) * 104);font-weight:800;line-height:1.05}',
  '.dshwv-wrap{white-space:normal;max-width:calc(var(--dshw-u) * 560);line-height:1.2}',
  '.dshwv-hint{font-size:calc(var(--dshw-u) * 56);color:#9fb0d9;letter-spacing:.02em;margin-top:calc(var(--dshw-u) * 9);min-height:calc(var(--dshw-u) * 64);line-height:1.15}',
  '.dshwv-menu-btn{position:absolute;top:calc(40.55% + 4px);right:4px;width:26px;height:26px;border:none;border-radius:6px;background:rgba(32,49,112,.85);cursor:pointer;pointer-events:auto;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:4px;padding:0;z-index:2;opacity:0;transition:opacity .15s ease}',
  '.dshwv-menu-btn.dshwv-menu-btn-visible{opacity:1}',
  '.dshwv-menu-btn span{display:block;width:14px;height:2px;background:#fff;border-radius:1px}',
  '.dshwv-menu-btn:hover{background:#203170}',
  '.dshwv-menu-btn-hidden{visibility:hidden;pointer-events:none}',
  '.dshwv-menu{position:fixed;min-width:196px;max-width:min(340px,calc(100vw - 24px));box-sizing:border-box;background:rgba(255,255,255,.92);border:1px solid rgba(32,49,112,.35);border-radius:10px;padding:10px 12px;opacity:0;transform:scale(.96) translateY(10px);transform-origin:top right;transition:opacity .22s ease,transform .22s cubic-bezier(.34,1.3,.6,1);pointer-events:none;z-index:10000;box-shadow:0 6px 18px rgba(0,0,0,.18);color-scheme:light}',
  '.dshwv-menu.dshwv-menu-open{opacity:1;transform:scale(1) translateY(0);pointer-events:auto}',
  '.dshwv-menu-row{display:flex;align-items:center;gap:8px;margin:5px 0;color:#203170;font-size:12px;white-space:nowrap}',
  '.dshwv-range{flex:1;min-width:0;accent-color:#203170}',
  '.dshwv-number{width:44px;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:2px 4px;font-size:12px;color:#203170;background:#fff;box-sizing:border-box}',
  '.dshwv-number:disabled{opacity:.4;background:rgba(32,49,112,.06);cursor:not-allowed}',
  '.dshwv-sound:disabled{opacity:.45;cursor:not-allowed}',
  '.dshwv-sound{flex:1;border:1px solid rgba(32,49,112,.4);border-radius:6px;background:rgba(32,49,112,.08);color:#203170;font-size:12px;padding:3px 0;cursor:pointer}',
  '.dshwv-sound:hover{background:rgba(32,49,112,.16)}',
  '.dshwv-check{width:16px;height:16px;accent-color:#203170;cursor:pointer;flex:0 0 auto}',
  '.dshwv-menu-sep{height:1px;background:rgba(32,49,112,.25);margin:6px 0}',
  '.dshwv-volpct{width:44px;text-align:right;color:#203170;font-size:12px}',
  '.dshwv-rolebtn-wrap{position:relative;flex:1;min-width:0}',
  '.dshwv-rolebtn{flex:1;min-width:0;display:flex;align-items:center;border:1px solid rgba(32,49,112,.4);border-radius:6px;background:rgba(32,49,112,.08);color:#203170;font-size:12px;padding:0 6px;cursor:pointer;overflow:hidden;height:24px}',
  '.dshwv-rolebtn:hover{background:rgba(32,49,112,.16)}',
  '.dshwv-btnlabel{display:block;flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;line-height:normal}',
  '.dshwv-roleimport{border:1px solid rgba(32,49,112,.4);border-radius:6px;background:#203170;color:#fff;font-size:12px;padding:3px 8px;cursor:pointer;flex:0 0 auto}',
  '.dshwv-roleimport:hover{background:#2f4488}',
  '.dshwv-rolelist{position:fixed;z-index:10001;box-sizing:border-box;background:rgba(255,255,255,.98);border:1px solid rgba(32,49,112,.35);border-radius:8px;box-shadow:0 6px 18px rgba(0,0,0,.18);padding:4px;max-height:240px;overflow-y:auto;display:none;color-scheme:light}',
  '.dshwv-rolelist.dshwv-rolelist-open{display:block}',
  '.dshwv-roleitem{display:flex;align-items:center;gap:6px;padding:4px 6px;border-radius:6px;cursor:pointer;color:#203170;font-size:12px;white-space:nowrap;min-width:0}',
  '.dshwv-roleitem:hover{background:rgba(32,49,112,.1)}',
  '.dshwv-roleitem.dshwv-roleitem-cur{background:rgba(32,49,112,.14)}',
  '.dshwv-rolethumb{width:22px;height:22px;border-radius:4px;object-fit:cover;flex:0 0 auto;background:#e8ecf7}',
  '.dshwv-rolename{flex:1;min-width:0;overflow:hidden}',
  '.dshwv-nameinner{display:inline-flex;white-space:nowrap;transition:transform .22s ease}',
  '.dshwv-rolenamewrap{flex:1;min-width:0;display:flex;align-items:center;gap:6px;overflow:hidden}',
  '.dshwv-nameinner .dshwv-namecopy{margin-right:40px;white-space:nowrap;flex:0 0 auto}',
  '.dshwv-roleGifTag{flex:0 0 auto;font-size:10px;line-height:1;padding:2px 4px;border-radius:3px;background:#203170;color:#fff}',
  '.dshwv-rolepin{width:22px;height:22px;border:none;background:none;cursor:pointer;font-size:13px;opacity:.45;padding:0;flex:0 0 auto}',
  '.dshwv-rolepin.on{opacity:1}',
  '.dshwv-roledel{width:22px;height:22px;border:none;background:none;cursor:pointer;font-size:13px;color:#c0392b;padding:0;flex:0 0 auto}',
  '.dshwv-audiobtn{flex:1;min-width:0;display:flex;align-items:center;border:1px solid rgba(32,49,112,.4);border-radius:6px;background:rgba(32,49,112,.08);color:#203170;font-size:12px;padding:0 6px;cursor:pointer;overflow:hidden;height:24px}',
  '.dshwv-audiobtn:hover{background:rgba(32,49,112,.16)}',
  '.dshwv-audioimport{border:1px solid rgba(32,49,112,.4);border-radius:6px;background:#203170;color:#fff;font-size:12px;padding:3px 8px;cursor:pointer;flex:0 0 auto}',
  '.dshwv-audioimport:hover{background:#2f4488}',
  '.dshwv-audiolist{position:fixed;z-index:10001;box-sizing:border-box;background:rgba(255,255,255,.98);border:1px solid rgba(32,49,112,.35);border-radius:8px;box-shadow:0 6px 18px rgba(0,0,0,.18);padding:4px;max-height:240px;overflow-y:auto;display:none;color-scheme:light}',
  '.dshwv-audiolist.dshwv-audiolist-open{display:block}',
  '.dshwv-audioitem{display:flex;align-items:center;gap:6px;padding:4px 6px;border-radius:6px;cursor:pointer;color:#203170;font-size:12px;white-space:nowrap;min-width:0}',
  '.dshwv-audioitem:hover{background:rgba(32,49,112,.1)}',
  '.dshwv-audioitem.dshwv-audioitem-cur{background:rgba(32,49,112,.14)}',
  '.dshwv-audioname{flex:1;min-width:0;overflow:hidden}',
  '.dshwv-audiopreset{color:#9fb0d9;font-size:11px;flex:0 0 auto}',
  '.dshwv-audiothumb{width:22px;height:22px;border-radius:4px;flex:0 0 auto;background:#e8ecf7;display:flex;align-items:center;justify-content:center;font-size:13px}',
  '.dshwv-audiopin{width:22px;height:22px;border:none;background:none;cursor:pointer;font-size:13px;opacity:.45;padding:0;flex:0 0 auto}',
  '.dshwv-audiopin.on{opacity:1}',
  '.dshwv-audiodel{width:22px;height:22px;border:none;background:none;cursor:pointer;font-size:13px;color:#c0392b;padding:0;flex:0 0 auto}',
  '.dshwv-audiomask{position:fixed;inset:0;background:rgba(15,23,42,.55);z-index:20500;display:flex;align-items:center;justify-content:center;color-scheme:light}',
  '.dshwv-audiowin{background:#fff;border-radius:12px;padding:16px 18px;width:360px;box-shadow:0 10px 30px rgba(0,0,0,.3);text-align:center}',
  '.dshwv-audiotitle{font-size:14px;font-weight:600;color:#203170;margin-bottom:12px}',
  '.dshwv-audionameinput{width:50%;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:6px 8px;font-size:13px;color:#203170;text-align:center;margin:0 auto 14px;display:block}',
  '.dshwv-audiorow{display:flex;align-items:center;gap:8px;margin:0 0 10px;color:#203170;font-size:12px;white-space:nowrap}',
  '.dshwv-audioslotlabel{flex:0 0 auto;width:32px;text-align:left}',
  '.dshwv-audioselect{flex:1;min-width:0;border:1px solid rgba(32,49,112,.4);border-radius:6px;background:#fff;color:#203170;font-size:12px;padding:3px 4px}',
  '.dshwv-audiosmallimport{border:1px solid rgba(32,49,112,.4);border-radius:6px;background:rgba(32,49,112,.08);color:#203170;font-size:12px;padding:3px 10px;cursor:pointer;flex:0 0 auto}',
  '.dshwv-audiosmallimport:hover{background:rgba(32,49,112,.16)}',
  '.dshwv-slotwrap{position:relative;flex:1;min-width:0}',
  '.dshwv-slotbtn{width:100%;border:1px solid rgba(32,49,112,.4);border-radius:6px;background:#fff;color:#203170;font-size:12px;padding:5px 6px;cursor:pointer;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}',
  '.dshwv-slotbtn:hover{background:rgba(32,49,112,.08)}',
  '.dshwv-slotlist{position:fixed;z-index:20600;box-sizing:border-box;background:rgba(255,255,255,.98);border:1px solid rgba(32,49,112,.35);border-radius:8px;box-shadow:0 6px 18px rgba(0,0,0,.18);padding:4px;max-height:200px;overflow-y:auto;display:none;color-scheme:light}',
  '.dshwv-audiocropcanvas{width:300px;height:120px;display:block;margin:0 auto 10px;background:#f3f5fb;border:1px solid rgba(32,49,112,.2);border-radius:8px;cursor:crosshair;touch-action:none}',
  '.dshwv-audiotime{color:#203170;font-size:12px;margin:6px 0 10px}',
  '.dshwv-audiosliderrow{display:flex;align-items:center;gap:8px;margin:2px 0}',
  '.dshwv-audiosliderrow input[type=range]{flex:1;accent-color:#203170}',
  '.dshwv-audiosliderrow input[type=number]{width:64px;flex:0 0 auto;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:2px 4px;font-size:12px;color:#203170;background:#fff;box-sizing:border-box;text-align:right}',
  '.dshwv-audiosliderrow input[type=number]:disabled{opacity:.4}',
  '.dshwv-zoomlabel{flex:0 0 auto;width:56px;color:#203170;font-size:12px;text-align:left}',
  '.dshwv-dualrange{position:relative;flex:1;height:22px;min-width:0;cursor:pointer;touch-action:none}',
  '.dshwv-dualrange-track{position:absolute;left:4px;right:4px;top:50%;height:4px;transform:translateY(-50%);background:rgba(32,49,112,.15);border-radius:2px}',
  '.dshwv-dualrange-fill{position:absolute;top:50%;height:4px;transform:translateY(-50%);background:rgba(32,49,112,.45);border-radius:2px}',
  '.dshwv-dualrange-thumb{position:absolute;top:50%;width:14px;height:14px;margin-left:-7px;margin-top:-7px;border-radius:50%;background:#203170;border:2px solid #fff;box-shadow:0 1px 3px rgba(0,0,0,.3);box-sizing:border-box}',
  '.dshwv-cropmask{position:fixed;inset:0;background:rgba(15,23,42,.55);z-index:20000;display:flex;align-items:center;justify-content:center;color-scheme:light}',
  '.dshwv-cropwin{background:#fff;border-radius:12px;padding:16px 18px;width:320px;box-shadow:0 10px 30px rgba(0,0,0,.3);text-align:center}',
  '.dshwv-croptitle{font-size:14px;font-weight:600;color:#203170;margin-bottom:12px}',
  '.dshwv-cropbox{position:relative;width:260px;height:260px;margin:0 auto 12px;border:1px dashed #203170;border-radius:8px;overflow:hidden;background:#f3f5fb;cursor:grab;touch-action:none}',
  '.dshwv-cropbox canvas{display:block}',
  '.dshwv-cropzoom{width:100%;accent-color:#203170}',
  '.dshwv-cropname{width:170px;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:5px 8px;font-size:13px;color:#203170;text-align:center;margin:0 auto 12px;display:block}',
  '.dshwv-cropctrl{display:flex;align-items:center;gap:8px;margin:0 0 10px}',
  '.dshwv-croplabel{flex:0 0 auto;width:28px;color:#203170;font-size:12px;text-align:left}',
  '.dshwv-cropnum{width:52px;flex:0 0 auto;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:2px 4px;font-size:12px;color:#203170;background:#fff;box-sizing:border-box;text-align:right}',
  '.dshwv-cropflip{width:26px;height:26px;flex:0 0 auto;border:1px solid rgba(32,49,112,.4);border-radius:6px;background:rgba(32,49,112,.08);color:#203170;font-size:14px;cursor:pointer;padding:0;line-height:1}',
  '.dshwv-cropflip:hover{background:rgba(32,49,112,.16)}',
  '.dshwv-cropflip-on{background:#203170;color:#fff}',
  '.dshwv-cropbtns{display:flex;gap:10px;justify-content:center}',
  '.dshwv-cropbtn{border:none;border-radius:6px;padding:6px 18px;font-size:13px;cursor:pointer}',
  '.dshwv-cropbtn-ok{background:#203170;color:#fff}',
  '.dshwv-cropbtn-ok:hover{background:#2f4488}',
  '.dshwv-cropbtn-no{background:rgba(32,49,112,.1);color:#203170}',
  '.dshwv-cropbtn-no:hover{background:rgba(32,49,112,.2)}',
  '.dshwv-gifmask{position:fixed;inset:0;background:rgba(15,23,42,.55);z-index:20000;display:flex;align-items:center;justify-content:center;color-scheme:light}',
  '.dshwv-gifwin{background:#fff;border-radius:12px;padding:16px 18px;width:340px;box-shadow:0 10px 30px rgba(0,0,0,.3);text-align:center}',
  '.dshwv-giftitle{font-size:14px;font-weight:600;color:#203170;margin-bottom:12px}',
  '.dshwv-gifpreview{position:relative;width:280px;height:280px;margin:0 auto 10px;border:1px dashed #203170;border-radius:8px;overflow:hidden;background:#f3f5fb;display:flex;align-items:center;justify-content:center}',
  '.dshwv-gifpreviewimg{max-width:100%;max-height:100%;object-fit:contain;display:block}',
  '.dshwv-gifhint{color:#9fb0d9;font-size:12px;margin-bottom:12px}',
  '.dshwv-gifname{width:170px;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:5px 8px;font-size:13px;color:#203170;text-align:center;margin:0 auto 12px;display:block}',
  '.dshwv-confirmmask{position:fixed;inset:0;background:rgba(15,23,42,.55);z-index:21000;display:flex;align-items:center;justify-content:center;color-scheme:light}',
  '.dshwv-confirmwin{background:#fff;border-radius:12px;padding:16px 18px;width:280px;box-shadow:0 10px 30px rgba(0,0,0,.3);text-align:center}',
  '.dshwv-confirmtext{font-size:13px;color:#203170;margin-bottom:14px;line-height:1.5;word-break:break-all}',
  '.dshwv-confirmbtns{display:flex;gap:10px;justify-content:center}',
  '.dshwv-snapmask{position:fixed;inset:0;background:rgba(15,23,42,.55);z-index:22000;display:flex;align-items:center;justify-content:center;color-scheme:light}',
  '.dshwv-snapwin{background:#fff;border-radius:12px;padding:14px 16px;width:400px;box-shadow:0 10px 30px rgba(0,0,0,.3);text-align:center;color:#203170;box-sizing:border-box}',
  '.dshwv-snaptitle{font-size:14px;font-weight:600;color:#203170;margin-bottom:10px}',
  '.dshwv-snapmodes{display:flex;align-items:center;justify-content:center;gap:18px;margin:0 0 12px;font-size:13px;flex-wrap:wrap}',
  '.dshwv-snapmodes label{display:inline-flex;align-items:center;gap:4px;cursor:pointer;color:#203170}',
  '.dshwv-snapmodes input{accent-color:#203170;cursor:pointer}',
  '.dshwv-snapgrid{display:grid;grid-template-columns:72px 190px 72px;grid-template-rows:26px 190px 26px;gap:4px;margin:0 auto 8px;place-items:center;width:max-content;justify-content:center}',
  '.dshwv-snapcell{display:flex;align-items:center;justify-content:center;gap:3px;min-width:0}',
  '.dshwv-snappreview{position:relative;width:190px;height:190px;border:1px solid rgba(32,49,112,.5);border-radius:8px;background:#f6f8fd;overflow:hidden;touch-action:none}',
  '.dshwv-snapflip{position:absolute;top:0;bottom:0;left:0;background:rgba(32,49,112,.07);pointer-events:none}',
  '.dshwv-snapzone{position:absolute;pointer-events:none}',
  '.dshwv-snapline{position:absolute;background:#203170;pointer-events:none}',
  '.dshwv-snapline-flip{background:#c0392b}',
  '.dshwv-snaphandle{position:absolute;width:16px;height:16px;margin:-8px 0 0 -8px;border-radius:50%;background:#203170;border:2px solid #fff;box-shadow:0 1px 3px rgba(0,0,0,.35);cursor:ew-resize;pointer-events:auto;z-index:3;box-sizing:border-box}',
  '.dshwv-snaphandle-flip{background:#c0392b}',
  '.dshwv-snaphandle-h{cursor:ns-resize}',
  '.dshwv-snapnum{width:58px;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:2px 4px;font-size:12px;color:#203170;background:#fff;text-align:right}',
  '.dshwv-snapnum:disabled{opacity:.45}',
  '.dshwv-snapunit{font-size:11px;color:#9fb0d9;flex:0 0 auto}',
  '.dshwv-snapfliprow{display:flex;align-items:center;justify-content:center;gap:6px;font-size:12px;margin:0 0 12px;color:#203170}',
  '.dshwv-snapbtns{display:flex;gap:10px;justify-content:center}',
  '.dshwv-snapbtn{border:none;border-radius:6px;padding:6px 18px;font-size:13px;cursor:pointer}',
  // v652 图片/随机图片编辑窗里的「上传图片」行:水平居中 + 上下留白(原来直接贴在容器上,既没居中也没边距)
  '.dshwv-uproll{display:flex;justify-content:center;margin:10px 0}',
  '.dshwv-snapbtn-ok{background:#203170;color:#fff}',
  '.dshwv-snapbtn-ok:hover{background:#2f4488}',
  '.dshwv-snapbtn-no{background:rgba(32,49,112,.1);color:#203170}',
  '.dshwv-snapbtn-no:hover{background:rgba(32,49,112,.2)}',
  '.dshwv-snapoff{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;background:rgba(246,248,253,.88);color:#9fb0d9;font-size:13px;z-index:5;pointer-events:auto}',
  '.dshwv-bubmask{position:fixed;inset:0;background:rgba(15,23,42,.55);z-index:20500;display:flex;align-items:center;justify-content:center;color-scheme:light}',
  '.dshwv-bubcard{background:#fff;border-radius:12px;padding:14px 16px;width:440px;box-shadow:0 10px 30px rgba(0,0,0,.3);text-align:center;color:#203170;box-sizing:border-box;max-height:88vh;overflow-y:auto;overflow-x:hidden}',
  '.dshwv-bubtitle{font-size:14px;font-weight:600;color:#203170;margin-bottom:8px}',
  '.dshwv-bubsec{font-size:12px;color:#9fb0d9;margin:10px 0 6px;text-align:left;border-top:1px solid rgba(32,49,112,.12);padding-top:8px}',
  '.dshwv-bubsec-first{border-top:none;margin-top:2px;padding-top:0}',
  '.dshwv-bubrow{display:flex;align-items:center;gap:6px;margin:4px 0;padding:4px;border:1px solid rgba(32,49,112,.16);border-radius:8px;background:#f8fafd}',
  // v635 触摸端自研拖拽:被拖的行浮起(桌面原生拖拽不走这个类)
  '.dshwv-bubrow.dshwv-row-dragging{position:relative;z-index:3;opacity:.92;box-shadow:0 8px 18px rgba(15,23,42,.28);transition:none}',
  // v639 W2「编辑第n次点击内容」触摸端自研拖拽:被拖的模块块/行手柄/调色板 chip 浮起
  '.dshwv-pv-dragging{position:relative;z-index:4;opacity:.92;box-shadow:0 6px 14px rgba(15,23,42,.25);transition:none}',
  '.dshwv-bubchip{flex:1;min-width:0;display:flex;align-items:center;justify-content:center;height:44px;border:1px dashed rgba(32,49,112,.35);border-radius:8px;background:#fff;color:#203170;font-size:12px;overflow:hidden;cursor:pointer}',
  '.dshwv-bubkind{width:96px;flex:0 0 auto;border:1px solid rgba(32,49,112,.4);border-radius:6px;background:#fff;color:#203170;font-size:12px;padding:3px 2px;cursor:pointer}',
  // 跑马灯方案下拉(自绘):选项多时列表限高 + 滚轮浏览
  '.dshwv-rgbwrap{position:relative;display:inline-block;vertical-align:middle;text-align:left}',
  '.dshwv-rgbhead{width:96px;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;background:#fff;color:#203170;font-size:12px;padding:3px 8px;cursor:pointer;text-align:left;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;display:flex;align-items:center;justify-content:space-between;gap:4px}',
  '.dshwv-rgbhead::after{content:"▾";font-size:9px;opacity:.7;flex:0 0 auto}',
  '.dshwv-rgbmenu{position:absolute;left:0;top:calc(100% + 2px);z-index:60;min-width:100%;max-width:160px;max-height:172px;overflow-y:auto;overflow-x:hidden;background:#fff;border:1px solid rgba(32,49,112,.3);border-radius:8px;box-shadow:0 6px 16px rgba(0,0,0,.18);padding:4px 0;display:none;text-align:left}',
  '.dshwv-rgbmenu.dshwv-rgbopen{display:block}',
  '.dshwv-rgbopt{padding:4px 10px;font-size:12px;color:#203170;cursor:pointer;white-space:nowrap}',
  '.dshwv-rgbopt:hover{background:rgba(32,49,112,.1)}',
  '.dshwv-rgbopt.dshwv-rgbcur{background:rgba(32,49,112,.16);font-weight:600}',
  // “+” 新建模块圆形按钮
  // 调色板“添加模块”:参照「+ 添加语句」样式(虚线边框/透明底/圆角/配色),但更紧凑
  '.dshwv-paladd{flex:0 0 auto;border:1px dashed rgba(32,49,112,.5);border-radius:8px;background:transparent;color:#203170;font-size:13px;line-height:1.4;padding:3px 9px;cursor:pointer;display:inline-flex;align-items:center;justify-content:center;user-select:none}',
  '.dshwv-paladd:hover{background:rgba(32,49,112,.08)}',
  // 模块库条目:悬浮时右上角小 x(删除)
  '.dshwv-libchip{position:relative;display:inline-flex;align-items:center;flex:0 0 auto;padding:0;margin:0;cursor:default;user-select:none}',
  '.dshwv-libchip .dshwv-palchip:hover{background:rgba(32,49,112,.18)}',
  '.dshwv-libdel{position:absolute;top:-7px;right:-7px;width:16px;height:16px;border-radius:50%;background:#c0392b;color:#fff;font-size:10px;line-height:1;display:none;align-items:center;justify-content:center;cursor:pointer;border:none;padding:0 0 1px;z-index:2}',
  '.dshwv-libchip:hover .dshwv-libdel{display:flex}',
  '.dshwv-libdel:hover{background:#a93226}',
  // 自定义字体输入
  '.dshwv-fontinp{width:150px;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:3px 6px;font-size:12px;color:#203170;background:#fff}',
  // 字体下拉:占满行宽,列表限高约 5.5 个可选项,可滚动
  '.dshwv-fontwrap{flex:1;min-width:0}',
  '.dshwv-fontwrap .dshwv-rgbhead{width:100%;box-sizing:border-box}',
  '.dshwv-rgbmenu.dshwv-fontmenu{width:220px;max-width:240px;max-height:134px}',
  // 颜色下拉(纯色/跑马灯):占满可用宽、列表限高滚动
  '.dshwv-qcolwrap{flex:0 1 auto;min-width:0;width:200px;max-width:200px}',
  '.dshwv-qcolwrap .dshwv-rgbhead{width:100%;box-sizing:border-box}',
  '.dshwv-rgbmenu.dshwv-qcolmenu{width:190px;max-width:210px;max-height:152px}',
  // 跑马灯选项:文字用实际渐变色渲染,并带与泡泡同款流动动画;当前项用 ✓ 前缀标记(避免背景高亮盖掉渐变)
  '.dshwv-qcolmenu .dshwv-rgbopt.optgrad{background-clip:text;-webkit-background-clip:text;color:transparent;-webkit-text-fill-color:transparent;text-shadow:none;background-size:200% auto;animation:dshwvRainbow 2.6s linear infinite;transition:transform .15s ease}',
  // 渐变选项的悬浮反馈:轻微放大+下划线(不增亮、不铺灰底,避免影响渐变可读性)
  '.dshwv-qcolmenu .dshwv-rgbopt.optgrad:hover{transform:scale(1.06);transform-origin:right center;text-decoration:underline}',
  // 快速编辑悬浮窗(文本/随机句)
  '.dshwv-qedit{position:fixed;z-index:26000;background:#fff;border:1px solid rgba(32,49,112,.35);border-radius:10px;box-shadow:0 8px 22px rgba(15,23,42,.22);padding:10px 12px;color-scheme:light}',
  '.dshwv-qedit-row{display:flex;align-items:center;gap:6px;margin:3px 0;flex-wrap:wrap;min-width:0}',
  '.dshwv-qedit-row label{font-size:12px;color:#203170;flex:0 0 auto}',
  '.dshwv-qedit-content{flex:1;min-width:120px;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:4px 6px;font-size:12px;color:#203170;background:#fff}',
  '.dshwv-qselect{box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;background:#fff;color:#203170;font-size:12px;padding:3px 4px;flex:0 1 auto;min-width:0}',
  '.dshwv-qcolorhost{display:inline-flex;align-items:center;gap:5px;flex:0 0 auto}',
  '.dshwv-qcolorhost input[type=color]{width:26px;height:20px;padding:0;border:1px solid rgba(32,49,112,.4);border-radius:4px;background:#fff}',
  '.dshwv-qcolorhost .dshwv-bubmini{width:auto;height:20px;font-size:11px;opacity:.85;padding:0 6px}',
  // 用量记录子面板 / 更多消费记录窗口
  '.dshwv-usagepanel{position:fixed;z-index:26020;background:#fff;border:1px solid rgba(32,49,112,.35);border-radius:10px;box-shadow:0 8px 22px rgba(15,23,42,.22);padding:10px 12px;color-scheme:light;max-height:70vh;overflow-y:auto}',
  // 用量作为主菜单内的子界面
  '.dshwv-menuview{display:block}',
  '.dshwv-usage-sub{display:none;max-height:min(70vh,560px);overflow-y:auto;padding-right:2px;padding-bottom:12px;width:100%;box-sizing:border-box}',
  '.dshwv-usage-back{border:none;background:none;color:#203170;font-size:12px;font-weight:600;cursor:pointer;padding:0 0 2px;text-align:left;width:100%}',
  '.dshwv-usage-back:hover{color:#2f4488;text-decoration:underline}',
  '.dshwv-usagebody{display:flex;flex-direction:column;gap:2px;color:#203170;font-size:12px;min-width:0;overflow-x:hidden}',
  // 主菜单 ↔ 用量记录切换过渡(返回主菜单:直接下滑复位,不加淡入)
  '@keyframes dshwvViewIn{from{transform:translateY(-8px)}to{transform:translateY(0)}}',
  '.dshwv-view-in{animation:dshwvViewIn .1s ease}',
  '.dshwv-usage-sec{display:flex;justify-content:space-between;align-items:center;margin:2px 0 2px;font-weight:600;border-bottom:1px solid rgba(32,49,112,.15);padding-bottom:3px;white-space:nowrap}',
  '.dshwv-usage-total{color:#e0433f;font-weight:700}',
  '.dshwv-usage-row{display:flex;justify-content:space-between;align-items:center;gap:8px;padding:2px 0;min-width:0}',
  // 用量列表容器（今日模型消费 / 近7天使用记录）：不限制高度，随内容自然增长
  '.dshwv-usage-scroll{overflow-y:auto;overflow-x:hidden;padding-right:2px;margin:2px 0 2px;border:1px solid rgba(32,49,112,.12);border-radius:6px}',
  // 消费记录(全部)新组件:概览头/可折叠分区/天折叠明细
  '.dshwv-usage-oview{text-align:center;padding:4px 0 6px;border-bottom:1px solid rgba(32,49,112,.12);margin-bottom:6px}',
  '.dshwv-usage-oview-num{font-size:22px;font-weight:800;color:#203170;margin:2px 0}',
  '.dshwv-usage-collapse{width:100%;display:flex;align-items:center;justify-content:space-between;gap:8px;background:none;border:none;border-bottom:1px solid rgba(32,49,112,.15);padding:6px 0 3px;margin:6px 0 2px;color:#203170;font-size:12px;font-weight:600;cursor:pointer;text-align:left}',
  '.dshwv-usage-collapse:hover{color:#2f4488}',
  '.dshwv-usage-chev{flex:0 0 auto;color:#9fb0d9;font-size:10px}',
  '.dshwv-usage-collapse-body{min-width:0;overflow-x:hidden}',
  '.dshwv-usage-ratio{min-width:0}',
  '.dshwv-usage-daydetail{margin:2px 0 6px 12px;padding:0 2px 2px 10px;border-left:2px solid rgba(32,49,112,.16);min-width:0}',
  // 用量小容器滚动条:细而淡,不再用浏览器默认的突兀滚动条
  '.dshwv-usage-scroll{scrollbar-width:thin;scrollbar-color:rgba(32,49,112,.16) transparent}',
  '.dshwv-usage-scroll::-webkit-scrollbar{width:6px}',
  '.dshwv-usage-scroll::-webkit-scrollbar-track{background:transparent}',
  '.dshwv-usage-scroll::-webkit-scrollbar-thumb{background:rgba(32,49,112,.14);border-radius:3px}',
  '.dshwv-usage-scroll::-webkit-scrollbar-thumb:hover{background:rgba(32,49,112,.26)}',
  '.dshwv-usage-model{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}',
  // 列宽受限 + 悬浮滚动（配合 mkScrollCell）：外层裁切，内层整体平移
  '.dshwv-marq{overflow:hidden;min-width:0;white-space:nowrap}',
  '.dshwv-marq>span{display:inline-block;white-space:nowrap;will-change:transform}',
  '.dshwv-usage-hint{color:#9fb0d9;font-size:11px;padding:2px 0;line-height:1.4}',
  '.dshwv-usage-subtitle{text-align:center;font-size:13px;font-weight:700;color:#203170;margin:0 0 6px}',
  // 预警与预算设置区
  '.dshwv-usageset{border:1px dashed rgba(32,49,112,.28);border-radius:8px;padding:0 6px 6px;margin:0 0 6px}',
  '.dshwv-usagemsg{flex:1;min-width:80px;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:2px 5px;font-size:11px;color:#203170;background:#fff}',
  '.dshwv-usagemsg:disabled{opacity:.45}',
  '.dshwv-usage-yuan{color:#203170;font-size:12px;flex:0 0 auto}',
  '.dshwv-msgtext{width:100%;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:5px 7px;font-size:12px;color:#203170;background:#fff;resize:vertical;min-height:60px}',
  '.dshwv-usage-more{display:block;width:100%;margin-top:4px;border:1px dashed rgba(32,49,112,.5);border-radius:8px;background:transparent;color:#203170;font-size:12px;padding:5px;cursor:pointer}',
  '.dshwv-usage-more:hover{background:rgba(32,49,112,.08)}',
  '.dshwv-usage-mask{position:fixed;inset:0;background:rgba(15,23,42,.55);z-index:22000;display:flex;align-items:center;justify-content:center;color-scheme:light}',
  '.dshwv-resmask{position:fixed;inset:0;background:rgba(15,23,42,.55);z-index:20300;display:flex;align-items:center;justify-content:center;color-scheme:light}',
  '.dshwv-usage-card{position:relative;background:#fff;border-radius:12px;width:min(560px,92vw);max-height:82vh;display:flex;flex-direction:column;box-shadow:0 10px 30px rgba(0,0,0,.3)}',
  '.dshwv-usage-wintitle{font-size:14px;font-weight:600;color:#203170;padding:12px 16px 8px;border-bottom:1px solid rgba(32,49,112,.15)}',
  '.dshwv-usage-close{position:absolute;top:8px;right:10px;width:24px;height:24px;border:none;background:none;font-size:18px;cursor:pointer;color:#203170;opacity:.6;border-radius:6px}',
  '.dshwv-usage-close:hover{background:rgba(32,49,112,.1);opacity:1}',
  '.dshwv-usage-windowbody{overflow-y:auto;padding:4px 16px 14px;flex:1;color:#203170;font-size:12px}',
  // 用量图表:近30天柱状 + 模型占比条
  '.dshwv-usage-chartwrap{position:relative;margin:4px 0 10px}',
  '.dshwv-usage-chartwrap canvas{display:block;width:100%;height:150px;background:#fafbfe;border:1px solid rgba(32,49,112,.15);border-radius:8px;box-sizing:border-box}',
  '.dshwv-usage-tip{position:absolute;pointer-events:none;background:rgba(15,23,42,.88);color:#fff;font-size:11px;padding:3px 7px;border-radius:5px;white-space:nowrap;z-index:5}',
  '.dshwv-usage-ratio{display:flex;align-items:center;gap:8px;margin:3px 0}',
  '.dshwv-usage-ratio-label{flex:0 0 96px;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#203170}',
  '.dshwv-usage-ratio-track{flex:1;min-width:0;height:10px;border-radius:5px;background:rgba(32,49,112,.12);min-width:0}',
  '.dshwv-usage-ratio-fill{height:100%;border-radius:5px;min-width:0;max-width:100%}',
  '.dshwv-usage-ratio-pct{flex:0 0 42px;text-align:right;color:#203170;font-size:11px;white-space:nowrap}',
  '.dshwv-usage-ratio-cost{flex:0 0 auto;color:#203170;font-size:11px;white-space:nowrap}',
  '.dshwv-fontinp:focus{outline:none;border-color:#203170}',
  '.dshwv-bubmini{width:22px;height:22px;flex:0 0 auto;border:none;background:none;cursor:pointer;font-size:14px;color:#203170;opacity:.6;padding:0;border-radius:4px}',
  '.dshwv-bubmini:hover{background:rgba(32,49,112,.12);opacity:1}',
  '.dshwv-bubmini-on{opacity:1}',
  '.dshwv-bubadd{width:100%;border:1px dashed rgba(32,49,112,.4);border-radius:8px;background:none;color:#203170;font-size:13px;padding:8px;cursor:pointer;margin:6px 0}',
  '.dshwv-bubadd:hover{background:rgba(32,49,112,.08)}',
  '.dshwv-bubbtns{display:flex;gap:10px;justify-content:center;margin-top:10px}',
  '.dshwv-bubbtn{border:none;border-radius:6px;padding:6px 18px;font-size:13px;cursor:pointer}',
  '.dshwv-bubbtn-ok{background:#203170;color:#fff}',
  '.dshwv-bubbtn-ok:hover{background:#2f4488}',
  '.dshwv-bubbtn-no{background:rgba(32,49,112,.1);color:#203170}',
  '.dshwv-bubbtn-no:hover{background:rgba(32,49,112,.2)}',
  '.dshwv-bubhint{font-size:11px;color:#9fb0d9;text-align:left;margin-top:6px}',
  // —— 自定义泡泡 · 主编辑窗口样式 ——
  '.dshwv-choicerow{display:flex;align-items:center;gap:12px;flex:1;min-width:0;padding:2px 0}',
  // 并列侧:编辑大按钮占满整组;权重框为无框窄条,图层悬浮于按钮右上角并与按钮右缘对齐
  '.dshwv-choicegrp{position:relative;flex:1 1 0;min-width:150px}',
  // 编辑大按钮:文本在扣除右侧权重框占位后的宽度中居中,并再右移约一字宽(带父级限定,覆盖 bubchip-btn 默认 padding)
  '.dshwv-choicegrp .dshwv-choicechip{width:100%;height:36px;padding:0 34px 0 20px}',
  '.dshwv-choicegrp .dshwv-winput{position:absolute;top:4px;right:4px;bottom:4px;width:26px;border:1px solid rgba(32,49,112,.45);background:#fff;border-radius:5px;padding:0 2px;font-size:11px;color:#203170;text-align:center;box-sizing:border-box}',
  '.dshwv-winput{width:46px;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:2px 4px;font-size:12px;color:#203170;background:#fff;box-sizing:border-box;text-align:center;flex:0 0 auto}',
  // 拆开钮:无边框大号 ⊕,十字由两根 CSS 条绘制(旋转精确绕十字中心,不受字体影响);悬停仅旋转不变色
  '.dshwv-splitbtn{width:26px;height:26px;min-width:26px;border:none;background:transparent;cursor:pointer;padding:0;display:flex;align-items:center;justify-content:center;flex:0 0 auto;opacity:.85}',
  '.dshwv-splitbtn span{position:relative;display:block;width:16px;height:16px;transform:rotate(0deg);transition:transform .18s ease}',
  '.dshwv-splitbtn span::before,.dshwv-splitbtn span::after{content:\'\';position:absolute;background:#203170;border-radius:1.5px}',
  '.dshwv-splitbtn span::before{left:0;top:50%;width:100%;height:2.5px;margin-top:-1.25px}',
  '.dshwv-splitbtn span::after{top:0;left:50%;width:2.5px;height:100%;margin-left:-1.25px}',
  '.dshwv-splitbtn:hover span{transform:rotate(45deg)}',
  '.dshwv-bubchip-cur{outline:2px solid rgba(32,49,112,.55)}',
  '.dshwv-sidebar{display:flex;align-items:center;gap:8px;margin:0 0 10px;flex-wrap:wrap}',
  '.dshwv-trow{display:block;text-align:center;line-height:1.2;white-space:nowrap;margin:calc(var(--dshw-u) * 5) auto;text-shadow:0 1px 2px rgba(255,255,255,.6)}',
  '@keyframes dshwvRainbow{0%{background-position:0% 0}100%{background-position:200% 0}}',
  '.dshwv-trow.dshwv-rgb,.dshwv-qcolmenu .dshwv-rgbopt.opt-macaron{background-image:linear-gradient(90deg,rgb(255,180,200),rgb(255,205,170),rgb(255,225,165),rgb(245,240,180),rgb(190,240,210),rgb(180,230,245),rgb(190,215,250),rgb(220,200,245),rgb(240,200,230),rgb(255,180,200));background-size:200% auto;-webkit-background-clip:text;background-clip:text;color:transparent;-webkit-text-fill-color:transparent;animation:dshwvRainbow 2.6s linear infinite;text-shadow:none}',
  '.dshwv-trow.dshwv-rgb-candy,.dshwv-qcolmenu .dshwv-rgbopt.opt-candy{background-image:linear-gradient(90deg,rgb(255,145,170),rgb(255,170,130),rgb(255,195,110),rgb(240,220,115),rgb(140,220,175),rgb(115,210,205),rgb(130,195,240),rgb(160,170,235),rgb(210,155,230),rgb(235,135,190),rgb(255,145,170))}',
  '.dshwv-trow.dshwv-rgb-rouge,.dshwv-qcolmenu .dshwv-rgbopt.opt-rouge{background-image:linear-gradient(90deg,rgb(140,25,45),rgb(175,35,60),rgb(120,20,55),rgb(160,40,75),rgb(190,55,80),rgb(130,30,65),rgb(140,25,45))}',
  '.dshwv-trow.dshwv-rgb-bamboo,.dshwv-qcolmenu .dshwv-rgbopt.opt-bamboo{background-image:linear-gradient(90deg,rgb(70,180,85),rgb(95,200,105),rgb(55,165,70),rgb(110,215,120),rgb(80,190,95),rgb(60,172,78),rgb(70,180,85))}',
  '.dshwv-trow.dshwv-rgb-aurora,.dshwv-qcolmenu .dshwv-rgbopt.opt-aurora{background-image:linear-gradient(90deg,rgb(70,240,200),rgb(90,200,255),rgb(120,140,255),rgb(180,120,255),rgb(240,140,255),rgb(70,240,200))}',
  '.dshwv-trow.dshwv-rgb-deepsea,.dshwv-qcolmenu .dshwv-rgbopt.opt-deepsea{background-image:linear-gradient(90deg,rgb(20,90,180),rgb(30,140,210),rgb(40,180,220),rgb(20,120,190),rgb(50,160,230),rgb(25,100,200),rgb(20,90,180))}',
  '.dshwv-trow.dshwv-rgb-sunset,.dshwv-qcolmenu .dshwv-rgbopt.opt-sunset{background-image:linear-gradient(90deg,rgb(255,180,80),rgb(255,130,90),rgb(255,90,110),rgb(220,90,150),rgb(160,90,190),rgb(255,180,80))}',
  '.dshwv-trow.dshwv-rgb-forest,.dshwv-qcolmenu .dshwv-rgbopt.opt-forest{background-image:linear-gradient(90deg,rgb(30,100,60),rgb(60,140,80),rgb(90,180,90),rgb(140,200,80),rgb(180,210,90),rgb(30,100,60))}',
  '.dshwv-trow.dshwv-rgb-champagne,.dshwv-qcolmenu .dshwv-rgbopt.opt-champagne{background-image:linear-gradient(90deg,rgb(220,180,100),rgb(240,205,130),rgb(255,225,160),rgb(230,190,110),rgb(245,210,140),rgb(220,180,100))}',
  '.dshwv-trow.dshwv-rgb-lavender,.dshwv-qcolmenu .dshwv-rgbopt.opt-lavender{background-image:linear-gradient(90deg,rgb(180,150,255),rgb(200,170,255),rgb(230,180,240),rgb(255,190,220),rgb(240,160,200),rgb(180,150,255))}',
  '.dshwv-trow.dshwv-rgb-mint,.dshwv-qcolmenu .dshwv-rgbopt.opt-mint{background-image:linear-gradient(90deg,rgb(120,230,180),rgb(150,240,200),rgb(170,240,230),rgb(140,220,240),rgb(120,200,220),rgb(120,230,180))}',
  '.dshwv-trow.dshwv-rgb-lava,.dshwv-qcolmenu .dshwv-rgbopt.opt-lava{background-image:linear-gradient(90deg,rgb(255,60,40),rgb(255,110,30),rgb(255,170,40),rgb(255,210,70),rgb(255,140,50),rgb(255,60,40))}',
  '.dshwv-trow.dshwv-rgb-galaxy,.dshwv-qcolmenu .dshwv-rgbopt.opt-galaxy{background-image:linear-gradient(90deg,rgb(40,30,90),rgb(70,50,130),rgb(110,70,170),rgb(160,90,190),rgb(220,120,180),rgb(40,30,90))}',
  // 新增跑马灯:墨韵黑白 / 靛蓝夜曲(文字·内层文字·菜单)
  '.dshwv-trow.dshwv-rgb-ink,.dshwv-trowtx.dshwv-rgb-ink,.dshwv-qcolmenu .dshwv-rgbopt.opt-ink{background-image:linear-gradient(90deg,rgb(20,20,20),rgb(80,80,80),rgb(140,140,140),rgb(200,200,200),rgb(250,250,250),rgb(250,250,250),rgb(200,200,200),rgb(140,140,140),rgb(80,80,80),rgb(20,20,20))}',
  '.dshwv-trow.dshwv-rgb-indigo,.dshwv-trowtx.dshwv-rgb-indigo,.dshwv-qcolmenu .dshwv-rgbopt.opt-indigo{background-image:linear-gradient(90deg,rgb(32,49,112),rgb(52,76,146),rgb(74,102,180),rgb(100,126,210),rgb(130,132,224),rgb(130,132,224),rgb(100,126,210),rgb(74,102,180),rgb(52,76,146),rgb(32,49,112))}',
  // v770 新增跑马灯:火红烈焰(深绯红→橙→白热芯) / 警示橙黄(高饱和橙→警示黄)
  '.dshwv-trow.dshwv-rgb-blaze,.dshwv-trowtx.dshwv-rgb-blaze,.dshwv-qcolmenu .dshwv-rgbopt.opt-blaze{background-image:linear-gradient(90deg,rgb(150,15,25),rgb(200,30,30),rgb(240,70,25),rgb(255,130,30),rgb(255,200,60),rgb(255,240,170),rgb(255,200,60),rgb(255,130,30),rgb(240,70,25),rgb(200,30,30),rgb(150,15,25))}',
  '.dshwv-trow.dshwv-rgb-amber,.dshwv-trowtx.dshwv-rgb-amber,.dshwv-qcolmenu .dshwv-rgbopt.opt-amber{background-image:linear-gradient(90deg,rgb(230,90,0),rgb(255,140,0),rgb(255,180,0),rgb(255,215,40),rgb(255,240,120),rgb(255,215,40),rgb(255,180,0),rgb(255,140,0),rgb(230,90,0))}',
  // 文字底色(圆角矩形底层):跑马灯底色与文字颜色/文字跑马灯不互相占用,底色铺在文字下方
  '.dshwv-trow.dshwv-bgrgb{text-shadow:none;background-size:200% auto;animation:dshwvRainbow 2.6s linear infinite}',
  '.dshwv-trow.dshwv-bgrgb-macaron{background-image:linear-gradient(90deg,rgb(255,180,200),rgb(255,205,170),rgb(255,225,165),rgb(245,240,180),rgb(190,240,210),rgb(180,230,245),rgb(190,215,250),rgb(220,200,245),rgb(240,200,230),rgb(255,180,200))}',
  '.dshwv-trow.dshwv-bgrgb-candy{background-image:linear-gradient(90deg,rgb(255,145,170),rgb(255,170,130),rgb(255,195,110),rgb(240,220,115),rgb(140,220,175),rgb(115,210,205),rgb(130,195,240),rgb(160,170,235),rgb(210,155,230),rgb(235,135,190),rgb(255,145,170))}',
  '.dshwv-trow.dshwv-bgrgb-rouge{background-image:linear-gradient(90deg,rgb(140,25,45),rgb(175,35,60),rgb(120,20,55),rgb(160,40,75),rgb(190,55,80),rgb(130,30,65),rgb(140,25,45))}',
  '.dshwv-trow.dshwv-bgrgb-bamboo{background-image:linear-gradient(90deg,rgb(70,180,85),rgb(95,200,105),rgb(55,165,70),rgb(110,215,120),rgb(80,190,95),rgb(60,172,78),rgb(70,180,85))}',
  '.dshwv-trow.dshwv-bgrgb-aurora{background-image:linear-gradient(90deg,rgb(70,240,200),rgb(90,200,255),rgb(120,140,255),rgb(180,120,255),rgb(240,140,255),rgb(70,240,200))}',
  '.dshwv-trow.dshwv-bgrgb-deepsea{background-image:linear-gradient(90deg,rgb(20,90,180),rgb(30,140,210),rgb(40,180,220),rgb(20,120,190),rgb(50,160,230),rgb(25,100,200),rgb(20,90,180))}',
  '.dshwv-trow.dshwv-bgrgb-sunset{background-image:linear-gradient(90deg,rgb(255,180,80),rgb(255,130,90),rgb(255,90,110),rgb(220,90,150),rgb(160,90,190),rgb(255,180,80))}',
  '.dshwv-trow.dshwv-bgrgb-forest{background-image:linear-gradient(90deg,rgb(30,100,60),rgb(60,140,80),rgb(90,180,90),rgb(140,200,80),rgb(180,210,90),rgb(30,100,60))}',
  '.dshwv-trow.dshwv-bgrgb-champagne{background-image:linear-gradient(90deg,rgb(220,180,100),rgb(240,205,130),rgb(255,225,160),rgb(230,190,110),rgb(245,210,140),rgb(220,180,100))}',
  '.dshwv-trow.dshwv-bgrgb-lavender{background-image:linear-gradient(90deg,rgb(180,150,255),rgb(200,170,255),rgb(230,180,240),rgb(255,190,220),rgb(240,160,200),rgb(180,150,255))}',
  '.dshwv-trow.dshwv-bgrgb-mint{background-image:linear-gradient(90deg,rgb(120,230,180),rgb(150,240,200),rgb(170,240,230),rgb(140,220,240),rgb(120,200,220),rgb(120,230,180))}',
  '.dshwv-trow.dshwv-bgrgb-lava{background-image:linear-gradient(90deg,rgb(255,60,40),rgb(255,110,30),rgb(255,170,40),rgb(255,210,70),rgb(255,140,50),rgb(255,60,40))}',
  '.dshwv-trow.dshwv-bgrgb-galaxy{background-image:linear-gradient(90deg,rgb(40,30,90),rgb(70,50,130),rgb(110,70,170),rgb(160,90,190),rgb(220,120,180),rgb(40,30,90))}',
  // 新增跑马灯底色:墨韵黑白 / 靛蓝夜曲
  '.dshwv-trow.dshwv-bgrgb-ink{background-image:linear-gradient(90deg,rgb(20,20,20),rgb(80,80,80),rgb(140,140,140),rgb(200,200,200),rgb(250,250,250),rgb(250,250,250),rgb(200,200,200),rgb(140,140,140),rgb(80,80,80),rgb(20,20,20))}',
  '.dshwv-trow.dshwv-bgrgb-indigo{background-image:linear-gradient(90deg,rgb(32,49,112),rgb(52,76,146),rgb(74,102,180),rgb(100,126,210),rgb(130,132,224),rgb(130,132,224),rgb(100,126,210),rgb(74,102,180),rgb(52,76,146),rgb(32,49,112))}',
  // v770 新增跑马灯底色:火红烈焰 / 警示橙黄
  '.dshwv-trow.dshwv-bgrgb-blaze{background-image:linear-gradient(90deg,rgb(150,15,25),rgb(200,30,30),rgb(240,70,25),rgb(255,130,30),rgb(255,200,60),rgb(255,240,170),rgb(255,200,60),rgb(255,130,30),rgb(240,70,25),rgb(200,30,30),rgb(150,15,25))}',
  '.dshwv-trow.dshwv-bgrgb-amber{background-image:linear-gradient(90deg,rgb(230,90,0),rgb(255,140,0),rgb(255,180,0),rgb(255,215,40),rgb(255,240,120),rgb(255,215,40),rgb(255,180,0),rgb(255,140,0),rgb(230,90,0))}',
  // 内层文字(带底色时)也支持跑马灯文字颜色:与 .dshwv-trow 相同的 渐变裁字 规则
  '.dshwv-trowtx.dshwv-rgb{background-image:linear-gradient(90deg,rgb(255,180,200),rgb(255,205,170),rgb(255,225,165),rgb(245,240,180),rgb(190,240,210),rgb(180,230,245),rgb(190,215,250),rgb(220,200,245),rgb(240,200,230),rgb(255,180,200));background-size:200% auto;-webkit-background-clip:text;background-clip:text;color:transparent;-webkit-text-fill-color:transparent;animation:dshwvRainbow 2.6s linear infinite;text-shadow:none}',
  '.dshwv-trowtx.dshwv-rgb-candy{background-image:linear-gradient(90deg,rgb(255,145,170),rgb(255,170,130),rgb(255,195,110),rgb(240,220,115),rgb(140,220,175),rgb(115,210,205),rgb(130,195,240),rgb(160,170,235),rgb(210,155,230),rgb(235,135,190),rgb(255,145,170))}',
  '.dshwv-trowtx.dshwv-rgb-rouge{background-image:linear-gradient(90deg,rgb(140,25,45),rgb(175,35,60),rgb(120,20,55),rgb(160,40,75),rgb(190,55,80),rgb(130,30,65),rgb(140,25,45))}',
  '.dshwv-trowtx.dshwv-rgb-bamboo{background-image:linear-gradient(90deg,rgb(70,180,85),rgb(95,200,105),rgb(55,165,70),rgb(110,215,120),rgb(80,190,95),rgb(60,172,78),rgb(70,180,85))}',
  '.dshwv-trowtx.dshwv-rgb-aurora{background-image:linear-gradient(90deg,rgb(70,240,200),rgb(90,200,255),rgb(120,140,255),rgb(180,120,255),rgb(240,140,255),rgb(70,240,200))}',
  '.dshwv-trowtx.dshwv-rgb-deepsea{background-image:linear-gradient(90deg,rgb(20,90,180),rgb(30,140,210),rgb(40,180,220),rgb(20,120,190),rgb(50,160,230),rgb(25,100,200),rgb(20,90,180))}',
  '.dshwv-trowtx.dshwv-rgb-sunset{background-image:linear-gradient(90deg,rgb(255,180,80),rgb(255,130,90),rgb(255,90,110),rgb(220,90,150),rgb(160,90,190),rgb(255,180,80))}',
  '.dshwv-trowtx.dshwv-rgb-forest{background-image:linear-gradient(90deg,rgb(30,100,60),rgb(60,140,80),rgb(90,180,90),rgb(140,200,80),rgb(180,210,90),rgb(30,100,60))}',
  '.dshwv-trowtx.dshwv-rgb-champagne{background-image:linear-gradient(90deg,rgb(220,180,100),rgb(240,205,130),rgb(255,225,160),rgb(230,190,110),rgb(245,210,140),rgb(220,180,100))}',
  '.dshwv-trowtx.dshwv-rgb-lavender{background-image:linear-gradient(90deg,rgb(180,150,255),rgb(200,170,255),rgb(230,180,240),rgb(255,190,220),rgb(240,160,200),rgb(180,150,255))}',
  '.dshwv-trowtx.dshwv-rgb-mint{background-image:linear-gradient(90deg,rgb(120,230,180),rgb(150,240,200),rgb(170,240,230),rgb(140,220,240),rgb(120,200,220),rgb(120,230,180))}',
  '.dshwv-trowtx.dshwv-rgb-lava{background-image:linear-gradient(90deg,rgb(255,60,40),rgb(255,110,30),rgb(255,170,40),rgb(255,210,70),rgb(255,140,50),rgb(255,60,40))}',
  '.dshwv-trowtx.dshwv-rgb-galaxy{background-image:linear-gradient(90deg,rgb(40,30,90),rgb(70,50,130),rgb(110,70,170),rgb(160,90,190),rgb(220,120,180),rgb(40,30,90))}',
  '.dshwv-trowtx.dshwv-rgb-ink{background-image:linear-gradient(90deg,rgb(20,20,20),rgb(80,80,80),rgb(140,140,140),rgb(200,200,200),rgb(250,250,250),rgb(250,250,250),rgb(200,200,200),rgb(140,140,140),rgb(80,80,80),rgb(20,20,20))}',
  '.dshwv-trowtx.dshwv-rgb-indigo{background-image:linear-gradient(90deg,rgb(32,49,112),rgb(52,76,146),rgb(74,102,180),rgb(100,126,210),rgb(130,132,224),rgb(130,132,224),rgb(100,126,210),rgb(74,102,180),rgb(52,76,146),rgb(32,49,112))}',
  '.dshwv-mimg{display:block;margin:0 auto;max-width:calc(var(--dshw-u) * 540);max-height:calc(var(--dshw-u) * 300);object-fit:contain}',
  '.dshwv-bubpal{display:flex;flex-wrap:wrap;gap:6px;justify-content:flex-start;margin:4px 0 6px;text-align:left}',
  '.dshwv-palchip{flex:0 0 auto;border:1px solid rgba(32,49,112,.4);border-radius:6px;background:rgba(32,49,112,.08);color:#203170;font-size:12px;padding:4px 8px;cursor:pointer;user-select:none}',
  '.dshwv-palchip:hover{background:rgba(32,49,112,.18)}',
  '.dshwv-bubpvbox{min-height:60px;border:1px dashed rgba(32,49,112,.5);border-radius:10px;background:#f6f8fd;padding:8px;text-align:center}',
  '.dshwv-pvrow{display:flex;align-items:center;gap:4px;justify-content:center;margin:2px 0;padding:2px;border:1px solid transparent;border-radius:6px}',
  '.dshwv-pvrow:hover{background:rgba(32,49,112,.06);border-color:rgba(32,49,112,.2)}',
  '.dshwv-pvlab{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#203170;line-height:1.2}',
  '.dshwv-pvrowline{flex-wrap:wrap;justify-content:center;gap:4px;text-align:center}',
  '.dshwv-pvmod{display:inline-flex;align-items:center;gap:3px;border:1px solid rgba(32,49,112,.4);background:#fff;border-radius:9px;padding:1px 5px 1px 9px;cursor:grab;max-width:100%;box-sizing:border-box;box-shadow:0 1px 2px rgba(32,49,112,.08)}',
  '.dshwv-pvmod:hover{border-color:rgba(32,49,112,.75);box-shadow:0 1px 5px rgba(32,49,112,.22)}',
  '.dshwv-pvmod .dshwv-pvlab{flex:1 1 auto;min-width:0;max-width:230px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#203170;font-size:12px;line-height:1.5;cursor:grab}',
  '.dshwv-pvmod.dshwv-pvimg{border-style:dashed;background:#f2f6ff}',
  '.dshwv-pvrowline .dshwv-bubmini{width:18px;height:18px;font-size:11px;opacity:.75}',
  '.dshwv-pvdrag{flex:0 0 auto;cursor:grab;color:rgba(32,49,112,.5);font-size:14px;line-height:1;padding:2px 3px;user-select:none}',
  '.dshwv-pvdrag:hover{color:#203170}',
  '.dshwv-pvadd{flex:0 0 auto;width:22px;height:22px;border:1px dashed rgba(32,49,112,.6);border-radius:8px;background:#fff;color:#203170;font-size:13px;line-height:1;cursor:pointer;padding:0;margin-left:2px}',
  '.dshwv-pvadd:hover{border-color:#203170;background:#eef2fb}',
  '.dshwv-bubimgprev{display:none;max-width:120px;max-height:80px;margin:6px auto;border-radius:6px;border:1px solid rgba(32,49,112,.3)}',
  // v649 随机图片模块:列表里的缩略图
  '.dshwv-rimthumb{width:26px;height:26px;object-fit:contain;flex:0 0 auto;background:#f2f5fb;border:1px solid rgba(32,49,112,.15);border-radius:4px}',
  '.dshwv-bubprev{margin:8px auto 2px;text-align:center}',
  '.dshwv-minipop{position:relative;width:100%;aspect-ratio:1026/700;margin:0 auto;filter:drop-shadow(0 2px 6px rgba(0,0,0,.18))}',
  '.dshwv-minipop svg{display:block;width:100%;height:100%}',
  '.dshwv-bubchip-btn{flex:1;min-width:0;border:1px dashed rgba(32,49,112,.35);border-radius:8px;background:#fff;color:#203170;font-size:12px;height:34px;padding:0 8px;cursor:pointer;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}',
  '.dshwv-bubchip-btn:hover{background:rgba(32,49,112,.06)}',
  '.dshwv-bubrow-drag{cursor:grab}',
  '.dshwv-bubrow-drag:hover{border-color:rgba(32,49,112,.4)}',
  '.dshwv-bubdrag{flex:0 0 auto;color:#9fb0d9;font-size:14px;cursor:grab;padding:0 2px}',
  '.dshwv-bublibrow{display:flex;align-items:center;gap:6px;margin:2px 0}',
  '.dshwv-bubnewbtn{border:none;border-radius:6px;background:rgba(32,49,112,.1);color:#203170;font-size:12px;padding:4px 8px;cursor:pointer}',
  '.dshwv-bubnewbtn:hover{background:rgba(32,49,112,.2)}',
  '.dshwv-linerow{border:1px solid rgba(32,49,112,.14);border-radius:6px;margin:2px 0;padding:2px;background:#fbfcfe}',
  '.dshwv-linew{width:48px;box-sizing:border-box;flex:0 0 auto;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:2px;font-size:12px;color:#203170;text-align:center}',
  '.dshwv-linetx{flex:1;min-width:0;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:3px 6px;font-size:12px;color:#203170;background:#fff}',
  '.dshwv-linedel{flex:0 0 auto;width:18px;height:18px;line-height:1;border:none;background:none;color:#c0392b;cursor:pointer;font-size:10px;padding:0}',
  '.dshwv-linepanel{border-top:1px dashed rgba(32,49,112,.2);margin:2px 0 4px;padding:2px 4px 0}',
  // 随机句列表:行头与行内列严格对齐(权重48 / 内容flex居中 / 操作区78 中心对齐复制钮);消除 audiorow 默认 10px 底距
  '.dshwv-linehead{display:flex;align-items:center;gap:8px;margin:2px 0 4px;padding:0 3px;color:#9fb0d9;font-size:11px}',
  '.dshwv-linehead .dshwv-lhw{flex:0 0 48px;text-align:center}',
  '.dshwv-linehead .dshwv-lhc{flex:1;min-width:0;display:flex;justify-content:center}',
  '.dshwv-linehead .dshwv-lho{flex:0 0 78px;text-align:center}',
  '.dshwv-linerow .dshwv-audiorow{margin-bottom:0}',
  // “+ 添加语句”:上边距、加宽、虚线边框
  '.dshwv-addline{display:block;width:70%;margin:12px auto 0;border:1px dashed rgba(32,49,112,.5);border-radius:8px;background:transparent;color:#203170;font-size:12px;padding:6px 8px;cursor:pointer}',
  '.dshwv-addline:hover{background:rgba(32,49,112,.08)}',
  '.dshwv-colrow{display:flex;align-items:center;gap:8px;position:relative;margin:2px 0 4px}',
  '.dshwv-colsw{width:34px;height:22px;border:1px solid rgba(32,49,112,.4);border-radius:6px;cursor:pointer;font-size:10px;padding:0;box-shadow:inset 0 0 0 1px rgba(255,255,255,.6)}',
  '.dshwv-colpop{position:absolute;left:0;top:calc(100% + 4px);z-index:30;background:#fff;border:1px solid rgba(32,49,112,.3);border-radius:8px;box-shadow:0 6px 16px rgba(0,0,0,.18);padding:8px;width:190px;text-align:left}',
  '.dshwv-coldots{display:grid;grid-template-columns:repeat(6,1fr);gap:6px;margin-bottom:8px}',
  '.dshwv-coldot{width:22px;height:22px;border-radius:50%;border:1px solid rgba(32,49,112,.25);cursor:pointer;padding:0}',
  '.dshwv-colhex{width:100%;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:3px 6px;font-size:11px;color:#203170;margin-bottom:8px}',
  '.dshwv-colfoot{display:flex;gap:8px;justify-content:flex-end}',
  '.dshwv-colnat{width:42px;height:26px;border:1px solid rgba(32,49,112,.45);border-radius:6px;padding:2px;background:#fff;cursor:pointer;box-sizing:border-box}',
  // —— 下拉菜单统一(只统一外观,不改任何刻意设定的宽度/最大宽度) ——
  // 下拉菜单滚动条:细而淡(与用量小容器同一观感);各菜单保持原有紧凑尺寸/内边距
  '.dshwv-rgbmenu,.dshwv-rolelist,.dshwv-audiolist,.dshwv-slotlist,.dshwv-usage-sub,.dshwv-listbox{scrollbar-width:thin;scrollbar-color:rgba(32,49,112,.16) transparent}',
  // 资源管理窗口容器 .dshwv-reswrap 之前漏在这组之外 → 滚动条是浏览器默认样式。
  // 宽度/配色与上一组完全一致，保持视觉统一。
  '.dshwv-reswrap{scrollbar-width:thin;scrollbar-color:rgba(32,49,112,.16) transparent}',
  '.dshwv-reswrap::-webkit-scrollbar{width:6px}',
  '.dshwv-reswrap::-webkit-scrollbar-track{background:transparent}',
  '.dshwv-reswrap::-webkit-scrollbar-thumb{background:rgba(32,49,112,.14);border-radius:3px}',
  '.dshwv-reswrap::-webkit-scrollbar-thumb:hover{background:rgba(32,49,112,.26)}',
  '.dshwv-rgbmenu::-webkit-scrollbar,.dshwv-rolelist::-webkit-scrollbar,.dshwv-audiolist::-webkit-scrollbar,.dshwv-slotlist::-webkit-scrollbar,.dshwv-usage-sub::-webkit-scrollbar,.dshwv-listbox::-webkit-scrollbar{width:6px}',
  '.dshwv-rgbmenu::-webkit-scrollbar-track,.dshwv-rolelist::-webkit-scrollbar-track,.dshwv-audiolist::-webkit-scrollbar-track,.dshwv-slotlist::-webkit-scrollbar-track,.dshwv-usage-sub::-webkit-scrollbar-track,.dshwv-listbox::-webkit-scrollbar-track{background:transparent}',
  '.dshwv-rgbmenu::-webkit-scrollbar-thumb,.dshwv-rolelist::-webkit-scrollbar-thumb,.dshwv-audiolist::-webkit-scrollbar-thumb,.dshwv-slotlist::-webkit-scrollbar-thumb,.dshwv-usage-sub::-webkit-scrollbar-thumb,.dshwv-listbox::-webkit-scrollbar-thumb{background:rgba(32,49,112,.14);border-radius:3px}',
  '.dshwv-rgbmenu::-webkit-scrollbar-thumb:hover,.dshwv-rolelist::-webkit-scrollbar-thumb:hover,.dshwv-audiolist::-webkit-scrollbar-thumb:hover,.dshwv-slotlist::-webkit-scrollbar-thumb:hover,.dshwv-usage-sub::-webkit-scrollbar-thumb:hover,.dshwv-listbox::-webkit-scrollbar-thumb:hover{background:rgba(32,49,112,.26)}',
  // 自绘下拉(替代原生 select):按钮仿 .dshwv-sound 观感,弹层复用 rgbmenu 统一样式与细滚动条
  '.dshwv-custwrap{position:relative;flex:1;min-width:0;display:flex;align-items:center}',
  '.dshwv-custbtn{flex:1;min-width:0;height:24px;box-sizing:border-box;border:1px solid rgba(32,49,112,.4);border-radius:6px;background:rgba(32,49,112,.08);color:#203170;font-size:12px;padding:0 8px;cursor:pointer;display:flex;align-items:center;gap:6px;text-align:left}',
  '.dshwv-custbtn:hover{background:rgba(32,49,112,.16)}',
  '.dshwv-custbtn:disabled{opacity:.45;cursor:not-allowed}',
  '.dshwv-custbtn::after{content:"▾";font-size:9px;opacity:.7;flex:0 0 auto}',
  '.dshwv-custlab{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}',
  '.dshwv-custmenu{max-height:220px;max-width:min(340px,calc(100vw - 16px))}',
  // 任务结束音等自绘下拉的选项行:限宽 + 超长名称悬停滚动(行内层复用 nameinner 机制)
  '.dshwv-custrow{display:flex;align-items:center;min-width:0;overflow:hidden;box-sizing:border-box}',
  '.dshwv-custrow .dshwv-custnm{flex:1;min-width:0;overflow:hidden;display:flex;align-items:center}',
  '.dshwv-custrow .dshwv-custnm .dshwv-nameinner{display:inline-flex;white-space:nowrap;transition:transform .22s ease}',
  '.dshwv-custrow .dshwv-custnm .dshwv-namecopy{margin-right:40px;white-space:nowrap;flex:0 0 auto}',
  // 任务结束音下拉行内的置顶(📌)按钮
  '.dshwv-custrow .dshwv-custpin{flex:0 0 auto;width:20px;height:20px;border:none;background:none;cursor:pointer;font-size:12px;opacity:.35;padding:0;margin-left:2px;line-height:1}',
  '.dshwv-custrow .dshwv-custpin.on{opacity:1}',
  '.dshwv-custrow .dshwv-custpin:hover{opacity:1;background:rgba(32,49,112,.1);border-radius:4px}',
  // 峰谷状态行:高峰色/空闲色 与 底色 并排各占一半(下拉可短一些)
  '.dshwv-peakrow{display:flex;align-items:flex-start;gap:10px;margin:0 0 2px}',
  '.dshwv-peakrow .dshwv-qedit-row{flex:1 1 50%;min-width:0;width:auto;margin:0;align-items:center;flex-wrap:nowrap}',
  '.dshwv-peakrow .dshwv-qcolwrap{width:auto;max-width:none;min-width:0;flex:1 1 auto}',
  '.dshwv-peakrow .dshwv-qcolwrap .dshwv-rgbhead{width:100%;padding:3px 6px;font-size:12px}',
  '.dshwv-peakrow .dshwv-qcolorhost{gap:3px}',
  '.dshwv-peakrow .dshwv-qcolorhost input[type=color]{width:22px;height:18px}',
  '.dshwv-peakrow .dshwv-qcolorhost .dshwv-bubmini{height:18px;font-size:10px;padding:0 4px}',
  // 内容框右侧 ? 说明钮 + 用法气泡
  '.dshwv-tplq{flex:0 0 auto;width:18px;height:18px;border-radius:50%;border:1px solid rgba(32,49,112,.55);background:none;color:#203170;font-size:11px;font-weight:700;line-height:1;cursor:pointer;padding:0;margin-left:4px}',
  '.dshwv-tplq:hover{background:rgba(32,49,112,.14)}',
  '.dshwv-tplhelp{position:fixed;z-index:26080;display:none;max-width:252px;background:#fff;border:1px solid rgba(32,49,112,.35);border-radius:8px;box-shadow:0 6px 16px rgba(0,0,0,.16);padding:8px 10px;font-size:12px;color:#203170;color-scheme:light}',
  // 「?」说明圈:放在「可选模块」等标题前,详细说明收进弹层(复用 tplhelp 弹层,加宽便于阅读)
  '.dshwv-askq{margin-left:0;margin-right:5px;flex:0 0 auto}',
  // 带「?」圈的标题:用 flex 让圈与标题文字垂直居中(不再用 vertical-align 硬顶)
  '.dshwv-bubsec.dshwv-bubsec-withq{display:flex;align-items:center}',
  '.dshwv-hintbox{max-width:min(340px,calc(100vw - 16px));line-height:1.5}',
  // 资源管理窗口:复用 usage mask/card 的遮罩层级与点击豁免,自绘列表观感与主界面一致
  '.dshwv-rescard{width:min(450px,94vw);max-height:78vh}',
  '.dshwv-reshead{display:flex;align-items:center;justify-content:space-between;padding:8px 12px 0;flex:0 0 auto}',
  '.dshwv-reshead .dshwv-restitle{font-size:15px;font-weight:700;color:#203170}',
  '.dshwv-resclose{flex:0 0 auto;border:none;background:none;cursor:pointer;font-size:14px;color:#9fb0d9;padding:1px 5px;border-radius:5px}',
  '.dshwv-resclose:hover{background:rgba(32,49,112,.1);color:#203170}',
  '.dshwv-reswrap{overflow-y:auto;padding:2px 10px 8px;flex:1 1 auto;min-height:0}',
  '.dshwv-rescat{font-weight:700;color:#203170;margin:7px 0 1px;font-size:13px;display:flex;align-items:center;gap:6px}',
  '.dshwv-rescat::after{content:"";flex:1;height:1px;background:rgba(32,49,112,.15)}',
  '.dshwv-resrow{display:flex;align-items:center;gap:6px;padding:3px 7px;border-radius:6px}',
  '.dshwv-resrow:hover{background:rgba(32,49,112,.06)}',
  '.dshwv-resthum{width:28px;height:28px;border-radius:5px;object-fit:cover;flex:0 0 auto;background:#e8ecf7;border:1px solid rgba(32,49,112,.12)}',
  '.dshwv-resicon{width:28px;height:28px;border-radius:5px;flex:0 0 auto;display:flex;align-items:center;justify-content:center;font-size:15px;background:#eef1f9}',
  '.dshwv-resmain{flex:1;min-width:0;overflow:hidden}',
  '.dshwv-resnm{font-size:12.5px;color:#203170;max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}',
  '.dshwv-resmeta{font-size:10.5px;color:#9fb0d9;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}',
  '.dshwv-restag{flex:0 0 auto;font-size:10px;color:#fff;background:#203170;border-radius:3px;padding:1px 5px}',
  '.dshwv-restag-built{background:#9fb0d9}',
  '.dshwv-resdel{flex:0 0 auto;border:1px solid rgba(201,57,43,.4);border-radius:4px;background:none;color:#c9392b;font-size:11px;padding:1px 8px;cursor:pointer}',
  '.dshwv-resdel:hover{background:rgba(201,57,43,.08)}',
  '.dshwv-resdel:disabled{opacity:.4;cursor:not-allowed}',
  '.dshwv-resplay{flex:0 0 auto;border:1px solid rgba(47,122,66,.4);border-radius:4px;background:none;color:#2f7a42;font-size:11px;padding:1px 8px;cursor:pointer}',
  '.dshwv-resplay:hover{background:rgba(47,122,66,.08)}',
  '.dshwv-resimp{flex:0 0 auto;border:1px solid rgba(32,49,112,.4);border-radius:5px;background:rgba(32,49,112,.08);color:#203170;font-size:11px;padding:1px 9px;cursor:pointer}',
  '.dshwv-resimp:hover{background:rgba(32,49,112,.16)}',
  '.dshwv-resempty{color:#9fb0d9;font-size:12px;padding:2px 6px}'
].join('\n')

var styleEl = document.createElement('style')
// PR #114：不带 data-plugin 的 <style> 会被 DSH 客户端模块系统 claimStyles 认领到
// 「当前正在物化的那个插件」名下，之后该插件热重载/失效时 removeOwnedStyles 会把它
// 一起删掉。样式一没，挂件 20 多个 dshwv-* 节点就从 position:fixed 掉回文档流堆在
// 页面底部（页面被撑到几千像素高）。打上自己的名字后就不会被任何人认领/删除。
styleEl.setAttribute('data-plugin', 'dsh-whale-widget')
styleEl.textContent = css + "\nhtml input[data-codex-plus-ext=\"whale-widget\"][type=\"checkbox\"]{appearance:auto!important;-webkit-appearance:checkbox!important;display:inline-block!important;position:static!important;width:16px!important;min-width:16px!important;height:16px!important;min-height:16px!important;margin:0 7px 0 0!important;padding:0!important;opacity:1!important;visibility:visible!important;clip:auto!important;clip-path:none!important;accent-color:#203170!important;color-scheme:light!important;cursor:pointer!important}\nhtml input[data-codex-plus-ext=\"whale-widget\"][type=\"range\"]{appearance:none!important;-webkit-appearance:none!important;display:inline-block!important;min-width:0!important;height:24px!important;min-height:24px!important;padding:0!important;border:0!important;box-shadow:none!important;background:transparent!important;color-scheme:light!important;cursor:pointer!important}\nhtml input[data-codex-plus-ext=\"whale-widget\"][type=\"range\"]::-webkit-slider-runnable-track{height:6px!important;border:0!important;border-radius:4px!important;background:#cbd1e1!important}\nhtml input[data-codex-plus-ext=\"whale-widget\"][type=\"range\"]::-webkit-slider-thumb{appearance:none!important;-webkit-appearance:none!important;width:16px!important;height:16px!important;margin-top:-5px!important;border:0!important;border-radius:50%!important;background:#203170!important;box-shadow:none!important}\nhtml input[data-codex-plus-ext=\"whale-widget\"][type=\"range\"]:disabled{opacity:.4!important;cursor:default!important}\n"
document.head.appendChild(styleEl)

// ===== v743：body 挂载登记器（DOM 守护的基础设施，见下面 dshwReattachRoot）=====
// 挂件会把 30 多个节点挂到 document.body 上（主节点、菜单、各种遮罩/面板、隐藏的 file input…）。
// 早先的守护只补挂 root + menuBox：SPA 切路由或别的插件整体替换 body 子树后，其余节点会变成
// "存在但不在文档里"的孤儿 —— 界面看起来恢复了，可一旦点开对应功能就静默失效。
// 所以这里统一登记：所有挂到 body 的节点都走 dshwBodyAppend()，守护时逐个补挂；
// 主动移除（目前只有一处）走 dshwBodyDetach()，避免被守护逻辑"复活"。
var dshwBodyNodes = []
// v789（issue #179）：输入法组字期间把"往 body 挂节点"的请求排队 —— 改 document.body 的子节点会触发布局，
//   在 composition 期间做这件事会让浏览器/输入法重置组字（用户实测：打字自动上屏、光标跳到最前端）。
//   排队到 compositionend 之后再补挂（正常输入时行为完全不变：非组字状态下一律立即挂）。
function dshwvFlushDeferredBody() {
  try {
    if (dshwvComposing || !dshwvDeferredBody.length) return
    var list = dshwvDeferredBody.slice()
    dshwvDeferredBody.length = 0
    for (var i = 0; i < list.length; i++) {
      try { if (list[i] && list[i].__detach) dshwBodyDetach(list[i].el); else dshwBodyAppend(list[i]) } catch (err) {}
    }
  } catch (err) {}
}
function dshwBodyAppend(el) {
  try {
    if (!el || !codexWhaleHost.active()) return el
    if (dshwvComposing && dshwvDeferredBody.indexOf(el) < 0) { dshwvDeferredBody.push(el); return el }
    // 注意：这里必须是**原始**的 document.body.appendChild —— 不能走 dshwBodyAppend 自己
    // （v743 批量改写时曾误替换成自我递归，被 try/catch 吞掉后表现为"登记了但从未挂上"）
    codexWhaleHost.attach(el)
    document.body.appendChild(el)
    if (dshwBodyNodes.indexOf(el) < 0) dshwBodyNodes.push(el)
  } catch (err) {}
  return el
}
function dshwBodyDetach(el) {
  try {
    // v789（issue #179）：组字期间连"移除"也排队（同样是 body 子节点变更，会打断组字）
    if (dshwvComposing) { dshwvDeferredBody.push({ __detach: true, el: el }); return }
    codexWhaleHost.detach(el)
    var i = dshwBodyNodes.indexOf(el)
    if (i >= 0) dshwBodyNodes.splice(i, 1)
    if (el && el.parentNode) el.parentNode.removeChild(el)
  } catch (err) {}
}
// 兼容性：老 WebView 可能没有 Element.isConnected（Chrome 51+ 才有）。
// 若直接 `!el.isConnected`，在那种环境里会恒为 true → 每个 DOM 变更批次都会重复 appendChild。
function dshwConnected(el) {
  if (!el) return false
  try { if (typeof el.isConnected === 'boolean') return el.isConnected } catch (err) {}
  try { return document.documentElement.contains(el) } catch (err) { return true }
}

var root = document.createElement('div')
root.className = 'dshwv-root'

var img = document.createElement('img')
img.className = 'dshwv-img'
// 首次渲染直接用上次保存的角色（避免刷新瞬间先闪默认鲸鱼娘再切角色）。
// loadRoles() 异步完成后会再校验：角色仍在则保持，被删则回退默认。
var initRoleUrl = IMG_URL
try {
  var initRoleId = localStorage.getItem('dshw-role') || ''
  if (initRoleId && initRoleId !== 'default') initRoleUrl = '/dsh-whale/role-image.png?id=' + encodeURIComponent(initRoleId)
} catch (err) {}
img.src = initRoleUrl
img.alt = 'Codex 用量'
img.draggable = false

var menuBtn = document.createElement('button')
menuBtn.type = 'button'
menuBtn.className = 'dshwv-menu-btn'
menuBtn.title = '菜单'
menuBtn.innerHTML = '<span></span><span></span><span></span>'
menuBtn.addEventListener('click', function (e) { e.stopPropagation(); toggleMenu() })

var menuBox = document.createElement('div')
menuBox.className = 'dshwv-menu'
function menuLabel(text) {
  var s = document.createElement('span')
  s.textContent = text
  return s
}
function menuRow() {
  var r = document.createElement('div')
  r.className = 'dshwv-menu-row'
  return r
}
var scaleInput = document.createElement('input')
scaleInput.type = 'range'
scaleInput.min = String(MIN_SCALE)
scaleInput.max = String(MAX_SCALE)
scaleInput.step = '0.1'
scaleInput.className = 'dshwv-range'
scaleInput.value = '1.5'
var scaleNumber = document.createElement('input')
scaleNumber.type = 'number'
scaleNumber.min = '1'
scaleNumber.max = '20'
scaleNumber.step = '1'
scaleNumber.className = 'dshwv-number'
scaleNumber.value = '10'
scaleInput.addEventListener('pointerdown', function () { root.style.transition = 'none' })
scaleInput.addEventListener('input', function () { setScale(scaleInput.value) })
scaleInput.addEventListener('change', function () { root.style.transition = ''; try { refreshFlip() } catch (err) {} })
scaleNumber.addEventListener('focus', function () { root.style.transition = 'none' })
scaleNumber.addEventListener('blur', function () { root.style.transition = '' })
scaleNumber.addEventListener('input', function () {
  var v = Math.round(Number(scaleNumber.value))
  var s = MIN_SCALE + Math.max(0, Math.min(20, v) - 1) * (MAX_SCALE - MIN_SCALE) / 19
  setScale(s)
})
scaleNumber.addEventListener('change', function () {
  var v = Math.round(Number(scaleNumber.value))
  var s = MIN_SCALE + Math.max(0, Math.min(20, v) - 1) * (MAX_SCALE - MIN_SCALE) / 19
  setScale(s)
  root.style.transition = ''
  try { refreshFlip() } catch (err) {}
})
// 音效组：下拉（含置顶/删除）+ 导入按钮（与角色同款式）
var audioGroupBtn = document.createElement('button')
audioGroupBtn.type = 'button'
audioGroupBtn.className = 'dshwv-audiobtn'
audioGroupBtn.title = '选择音效组'
var audioGroupBtnLabel = document.createElement('span')
audioGroupBtnLabel.className = 'dshwv-btnlabel'
audioGroupBtnLabel.textContent = '小黄鸭'
audioGroupBtn.appendChild(audioGroupBtnLabel)
var audioGroupPanel = document.createElement('div')
audioGroupPanel.className = 'dshwv-audiolist'
var audioImportBtn = document.createElement('button')
audioImportBtn.type = 'button'
audioImportBtn.className = 'dshwv-audioimport'
audioImportBtn.textContent = '导入'
audioImportBtn.title = '新建/编辑音效组'
audioGroupBtn.addEventListener('click', function (e) { e.stopPropagation(); toggleAudioGroupPanel() })
audioImportBtn.addEventListener('click', function (e) { e.stopPropagation(); openAudioGroupEditor(null) })
dshwBodyAppend(audioGroupPanel)
function soundOpt(value, label) {
  var o = document.createElement('option')
  o.value = value
  o.textContent = label
  return o
}
// —— 原生 select → 自绘下拉 ——
// 保留隐藏 select 作为值存储(所有读写/事件逻辑零改动),UI 换成按钮+弹出列表:
// 点击选项 → 写回 select.value 并派发 change → 原逻辑照常执行。
var dshwCustSelOpen = null // 当前展开的自绘下拉 {menu, btn}
var dshwCustSuppressAt = 0 // 刚用 pointerdown 关闭后,抑制随之而来的 click 重新打开
function dshwCustSelClose() {
  var o = dshwCustSelOpen
  dshwCustSelOpen = null
  if (!o) return
  try {
    o.menu.classList.remove('dshwv-rgbopen')
    dshwBodyDetach(o.menu) // v744：登记过的 body 节点必须走 detach，否则会被 DOM 守护补挂回来
  } catch (err) {}
}
if (!window.__dshwCustBound) {
  window.__dshwCustBound = true
  document.addEventListener('pointerdown', function (e) {
    var o = dshwCustSelOpen
    if (!o) return
    try {
      // 点在“已打开的”触发按钮上(可能点在按钮内文本/子元素):立即收回,抑制随后 click 的重开
      var onBtn = !!(e.target && e.target.closest && e.target.closest('.dshwv-custbtn'))
      if (onBtn) {
        dshwCustCloseNow()
        return
      }
      if (e.target && o.menu.contains(e.target)) return
    } catch (err) {}
    dshwCustSelClose()
  }, true)
  document.addEventListener('keydown', function (e) {
    // v789（issue #179）：组字中按 Esc 是输入法的「取消组字」，别抢
    if (dshwvComposingNow()) return
    if (e.key === 'Escape') dshwCustCloseNow()
  }, true)
  window.addEventListener('resize', function () { dshwCustCloseNow() })
}
function dshwCustCloseNow() {
  dshwCustSelClose()
  dshwCustSuppressAt = Date.now()
}
var taskEndDrop = null
var peakDrop = null
var moduleImgDrop = null
// sel 必须已挂载到 DOM;返回 {sync, refresh}
function dshwCustSel(sel, opts) {
  if (!sel || !sel.parentNode || sel.__dshwCust) return { sync: function () {}, refresh: function () {} }
  sel.__dshwCust = true
  var parent = sel.parentNode
  var wrap = document.createElement('div')
  wrap.className = 'dshwv-custwrap'
  var btn = document.createElement('button')
  btn.type = 'button'
  btn.className = 'dshwv-custbtn'
  btn.title = sel.title || ''
  var lab = document.createElement('span')
  lab.className = 'dshwv-custlab'
  btn.appendChild(lab)
  parent.insertBefore(wrap, sel)
  wrap.appendChild(btn)
  wrap.appendChild(sel)
  sel.style.display = 'none'
  var menu = document.createElement('div')
  menu.className = 'dshwv-rgbmenu dshwv-custmenu'
  function labelOf(v) {
    for (var i = 0; i < sel.options.length; i++) {
      if (String(sel.options[i].value) === String(v)) return String(sel.options[i].textContent || sel.options[i].text || '')
    }
    return ''
  }
  function sync() {
    try {
      lab.textContent = labelOf(sel.value) || '—'
      btn.disabled = !!sel.disabled
    } catch (err) {}
  }
  function fill() {
    menu.innerHTML = ''
    var cur = sel.value
    for (var i = 0; i < sel.options.length; i++) {
      (function (opt) {
        var d = document.createElement('div')
        var lab = String(opt.textContent || opt.text || opt.value)
        // 限宽长名滚动模式(任务结束音等):行内容放入可悬停循环滚动的名称层,
        // 超长名称不挤压菜单宽度、不外溢,鼠标悬停时横向滚动露出全名
        if (opts && opts.scrollNames) {
          d.className = 'dshwv-rgbopt dshwv-custrow' + (String(opt.value) === String(cur) ? ' dshwv-rgbcur' : '')
          var nm = makeNameCell('dshwv-custnm', lab)
          d.appendChild(nm)
          bindNameMarquee(d, nm)
        } else {
          d.className = 'dshwv-rgbopt' + (String(opt.value) === String(cur) ? ' dshwv-rgbcur' : '')
          d.textContent = lab
        }
        // 行内置顶(📌)按钮:仅任务结束音等传入 pin 回调时渲染;点击只切置顶不选中
        if (opts && typeof opts.isPinned === 'function' && typeof opts.onPin === 'function') {
          var pinned = !!opts.isPinned(String(opt.value))
          var pb = document.createElement('button')
          pb.type = 'button'
          pb.className = 'dshwv-custpin' + (pinned ? ' on' : '')
          pb.textContent = '📌'
          pb.title = pinned ? '取消置顶' : '置顶(排到列表最前)'
          pb.addEventListener('click', function (e) {
            e.stopPropagation()
            opts.onPin(String(opt.value))
          })
          d.appendChild(pb)
        }
        d.addEventListener('click', function (e) {
          e.stopPropagation()
          try { sel.value = opt.value } catch (err) {}
          sync()
          dshwCustSelClose()
          try { sel.dispatchEvent(new Event('change')) } catch (err) {}
        })
        menu.appendChild(d)
      })(sel.options[i])
    }
  }
  btn.addEventListener('click', function (e) {
    e.stopPropagation()
    if (btn.disabled) return
    // pointerdown 已在打开态收回并打上抑制标记:这次 click 直接吞掉,避免“收回又弹出”
    if (dshwCustSuppressAt && Date.now() - dshwCustSuppressAt < 350) {
      dshwCustSuppressAt = 0
      return
    }
    if (dshwCustSelOpen && dshwCustSelOpen.btn === btn) { dshwCustSelClose(); return }
    dshwCustSelClose()
    fill()
    sync()
    if (menu.parentNode !== document.body) dshwBodyAppend(menu)
    dshwDropOpen(menu, btn)
    // 可选底部参照元素:展开高度不超过该元素的上缘(内部滚动),用于
    // 主菜单内“任务结束音效”等下拉,避免列表盖住底部「小鲸鱼记账」按钮
    if (opts && typeof opts.bottom === 'function') {
      try {
        var bEl = opts.bottom()
        if (bEl && bEl.getBoundingClientRect) {
          var bTop = bEl.getBoundingClientRect().top
          var mTop = menu.getBoundingClientRect().top
          var avail = Math.floor(bTop - mTop - 6)
          if (avail >= 40) menu.style.maxHeight = Math.min(avail, 220) + 'px'
        }
      } catch (err) {}
    }
    dshwCustSelOpen = { menu: menu, btn: btn }
  })
  sync()
  return { sync: sync, refresh: function () { fill(); sync() }, btn: btn, wrap: wrap }
}
// 用量:小鲸鱼记账为唯一记账方式;「用量记录」按钮打开历史用量子面板
var usageRecBtn = document.createElement('button')
usageRecBtn.type = 'button'
usageRecBtn.className = 'dshwv-roleimport'
usageRecBtn.textContent = '- = 小鲸鱼记账 = -'
usageRecBtn.title = '查看今日/近7天/全部消费记录'
usageRecBtn.addEventListener('click', function (e) { e.stopPropagation(); toggleUsagePanel() })
// —— 任务结束音(与按下/松开共用音效库;开启才可选,默认取已导入的 entity) ——
// v720 起这两个控件不再占用主菜单,而是挂进「自定义提示」窗口(见 taskEndRowHost)。
// 窗口是"保存/取消"语义,所以窗口打开期间设置先只落到内存(taskEndDefer),取消可还原。
var taskEndDefer = false // true = 窗口打开中,改动先不落盘
var taskEndDeferSnap = null // 窗口打开时的快照(取消时还原)
function applyTaskEndLocal(on, sel) {
  usageSet = usageSet || {}
  usageSet.taskEnd = usageSet.taskEnd || { on: false, sel: '' }
  if (typeof on === 'boolean') usageSet.taskEnd.on = on
  if (typeof sel === 'string') usageSet.taskEnd.sel = sel
  taskEndToggle.checked = !!usageSet.taskEnd.on
  taskEndSel.disabled = !usageSet.taskEnd.on
  if (taskEndDrop) taskEndDrop.refresh()
}
function commitTaskEnd() {
  if (taskEndDefer) return
  saveUsageSettings({ taskEnd: (usageSet && usageSet.taskEnd) || { on: false, sel: '' } })
}
var taskEndToggle = document.createElement('input')
taskEndToggle.type = 'checkbox'
taskEndToggle.className = 'dshwv-check'
taskEndToggle.checked = false
taskEndToggle.title = '每轮对话回复完成时播放提示音'
taskEndToggle.addEventListener('change', function () {
  applyTaskEndLocal(taskEndToggle.checked, undefined)
  commitTaskEnd()
})
var taskEndSel = document.createElement('select')
taskEndSel.className = 'dshwv-sound'
taskEndSel.disabled = true
taskEndSel.title = '选择任务结束音:音效组(点按整组)/单个音频(与按下/松开同库)'
taskEndSel.addEventListener('change', function () {
  applyTaskEndLocal(undefined, taskEndSel.value)
  commitTaskEnd()
})
// v761（全局音效设置面板）：音效下拉的**通用填充**（任务结束音 / 提问 / 授权三处共用）。
// 排列:置顶项恒最前(按置顶先后) → 已导入片段 → 4 个预设单音 → 音效组置底；
// 组与组可同名,按 value 去重(不按显示名);其余按 value/显示名去重。
// 只改 DOM 与（任务结束音原有的）兜底设置，不写任何新键 ——
// 任务结束音（selectEl === taskEndSel）完全沿用原有语义，其它下拉找不到现值时保留原值显示。
// v762：**删掉了原第一位的「静音」项**（value 为空串）——「不响」统一由音效行左边的 [✓] 表达
// （提问/授权 = events.<kind>.soundOn，任务结束音 = taskEnd.on），一个开关就够了，不再有语义重叠。
// 连带效应（正是我们要的）：空/失效的 sel 不再被「静音」项接住，于是重新走到下面的兜底分支
// （任务结束音挑默认音并**写回 usageSet**），下拉里再也不会出现空值。
function fillSoundSelect(selectEl, currentSel) {
  if (!selectEl) return false
  var isTaskEnd = selectEl === taskEndSel
  // 期望值优先取“持久化设置”,避免启动时被上一轮的临时默认值带偏
  var cur = String(currentSel == null ? '' : currentSel)
  selectEl.innerHTML = ''
  var seen = {}
  var seenLbl = {}
  function add(v, lab) {
    // 既按 value 也按显示名去重:内置预设与其同名的音效库条目不再重复出现
    if (seen[v]) return
    if (seenLbl[lab]) return
    seen[v] = 1
    seenLbl[lab] = 1
    selectEl.appendChild(soundOpt(v, lab))
  }
  // 收集全部候选(片段+单音在前,组置底),返回 {v,lab,grp} 列表
  var grps = Array.isArray(audioGroups) ? audioGroups : []
  var frags = Array.isArray(audioFragments) ? audioFragments : []
  var pre = [
    ['preset:duck:press', '小黄鸭·按下'], ['preset:duck:release', '小黄鸭·松开'],
    ['preset:fx1:press', '音效1·按下'], ['preset:fx1:release', '音效1·松开'],
  ]
  var fragCand = []
  // 同名片段：内置预设与用户自导入条目同名时（如内置 A 与用户导入的 A），
  // 若当前选中的正是用户那一条，就让用户条目占住这个名字 —— 否则内置预设先入列，
  // 按显示名去重会把用户条目挤掉，已保存的选择随之失效并被兜底逻辑改写成别的音效。
  // 用户把自导入条目删掉后，这里自然失效，内置预设重新出现在列表里。
  var customFragByLabel = {}
  frags.forEach(function (f) {
    if (!f || !f.id || f.preset) return
    customFragByLabel[String(f.name || f.id)] = 'frag:' + f.id
  })
  frags.forEach(function (f) {
    if (!f || !f.id) return
    // 传统 4 个预设单音已由下方 preset:* 覆盖（按显示名去重），其余内置片段
    // （如内置的 Minecraft·经验球 exp_orb、任务结束音 A end_a）照常列出，供任务结束音选择
    if (f.preset && (f.id === 'ya1' || f.id === 'ya2' || f.id === 'd1' || f.id === 'd2')) return
    if (f.preset && cur && customFragByLabel[String(f.name || f.id)] === cur) return
    fragCand.push({ v: 'frag:' + f.id, lab: String(f.name || f.id), grp: false })
  })
  var preCand = pre.map(function (o) { return { v: o[0], lab: o[1], grp: false } })
  var grpCand = []
  grps.forEach(function (g) {
    if (!g || !g.id) return
    grpCand.push({ v: 'grp:' + g.id, lab: String(g.name || audioGroupName(g.id)) + '（点按）', grp: true })
  })
  var allCand = fragCand.concat(preCand, grpCand)
  var candByV = {}
  allCand.forEach(function (c) { candByV[c.v] = c })
  // 置顶序列(仍存在的选项按 pin 顺序排最前)；置顶只属于任务结束音(值存 usageSet.taskEnd.pins)
  var pins = []
  if (isTaskEnd) { try { if (usageSet && usageSet.taskEnd && Array.isArray(usageSet.taskEnd.pins)) pins = usageSet.taskEnd.pins.slice() } catch (err) {} }
  var ordered = []
  var orderedSeen = {}
  pins.forEach(function (pv) {
    if (!candByV[pv] || orderedSeen[pv]) return
    orderedSeen[pv] = 1
    ordered.push(candByV[pv])
  })
  allCand.forEach(function (c) {
    if (orderedSeen[c.v]) return
    orderedSeen[c.v] = 1
    ordered.push(c)
  })
  ordered.forEach(function (c) {
    if (c.grp) {
      if (seen[c.v]) return
      seen[c.v] = 1
      selectEl.appendChild(soundOpt(c.v, c.lab))
    } else add(c.v, c.lab)
  })
  var fragN = 0
  for (var fi = 0; fi < selectEl.options.length; fi++) if (String(selectEl.options[fi].value).indexOf('frag:') === 0) fragN++
  var found = false
  for (var i = 0; i < selectEl.options.length; i++) if (selectEl.options[i].value === cur) { selectEl.value = cur; found = true; break }
  if (!found) {
    if (isTaskEnd) {
      // 期望值是自导入片段,但片段列表尚未就绪(启动时音频还在加载):
      // 先不落默认、不改写 usageSet,等片段就绪后的下一次 fill 再定,
      // 避免把用户的片段选择悄悄覆盖成「小黄鸭·按下」并随后续保存持久化。
      // 注意:这里把 value 置空是**流程需要**（fillTaskEndOptions 靠 `taskEndSel.value || prefSel`
      // 在下一轮重新取到真实值），与已删除的「静音」项无关 —— 置空后本次会显示为未选中，属预期。
      if (cur && String(cur).indexOf('frag:') === 0 && fragN === 0) {
        selectEl.value = ''
        if (taskEndDrop) taskEndDrop.refresh()
        return false
      }
      // 期望值是音效组,但组列表尚未就绪(启动时 audio.json 还在加载):
      // 同样先不落默认、不改写 usageSet,等组就绪后的下一次 fill 再定
      if (cur && String(cur).indexOf('grp:') === 0 && (!Array.isArray(audioGroups) || audioGroups.length === 0)) {
        selectEl.value = ''
        if (taskEndDrop) taskEndDrop.refresh()
        return false
      }
      // 默认:优先 name==='entity',否则第一条已导入片段;都没有则选预设
      var chosen = 'preset:duck:press'
      for (var j = 0; j < selectEl.options.length; j++) {
        var v = selectEl.options[j].value
        if (v.indexOf('frag:') === 0) { chosen = v; if (String(selectEl.options[j].textContent || '') === 'entity') break }
      }
      selectEl.value = chosen
      usageSet = usageSet || {}
      usageSet.taskEnd = usageSet.taskEnd || { on: false, sel: '' }
      usageSet.taskEnd.sel = chosen
    } else if (cur) {
      // 面板下拉：绑定的音效还没加载出来（如音频列表未就绪）——原样保留并显示，
      // 不静默改成别的音效（否则用户一保存就把自己的选择丢了）
      selectEl.appendChild(soundOpt(cur, cur + '（暂未加载）'))
      selectEl.value = cur
    } else {
      // v762：「静音」项已删除，空值不再有任何含义。面板下拉（提问/授权）走到这里表示
      // 「没有给值」，兜底选中第一项，保证下拉永远有一个**非空**值
      // （pollWaitState 的 `on && sel` 门控因此仍然成立）。
      // 任务结束音不会走到这里（上面 isTaskEnd 分支已处理过 cur 为空的情况）。
      selectEl.selectedIndex = selectEl.options.length ? 0 : -1
    }
  }
  // 兜底:按 value 从尾向前去重,确保绝无重复项
  var seen2 = {}
  for (var di = selectEl.options.length - 1; di >= 0; di--) {
    var dv = selectEl.options[di].value
    if (seen2[dv]) { try { selectEl.remove(di) } catch (err) {} }
    else seen2[dv] = 1
  }
  try {
    if (isTaskEnd) { if (taskEndDrop) taskEndDrop.refresh() }
    else if (selectEl.__dshwDrop) selectEl.__dshwDrop.refresh()
  } catch (err) {}
  return found
}
// 任务结束音下拉：入口与语义不变，内部改走通用 fillSoundSelect
// （v762 起选项里**不再有「静音」**：不响由 taskEnd.on / 面板音效行的 [✓] 表达）
function fillTaskEndOptions(pref) {
  var prefSel = (usageSet && usageSet.taskEnd && usageSet.taskEnd.sel) || (pref && pref.sel) || ''
  var cur = taskEndSel.value || prefSel || ''
  fillSoundSelect(taskEndSel, cur)
  if (pref && pref.sel) { for (var k = 0; k < taskEndSel.options.length; k++) if (taskEndSel.options[k].value === pref.sel) taskEndSel.value = pref.sel }
  if (taskEndDrop) taskEndDrop.refresh()
}
// 片段列表变化后刷新任务结束音下拉(保留用户当前选择)
function refreshTaskEndAfterAudio() {
  try { fillTaskEndOptions((usageSet && usageSet.taskEnd) || null) } catch (err) {}
}
// 任务结束音下拉置顶(📌):值存 usageSet.taskEnd.pins(按置顶先后),置顶项在 fillTaskEndOptions 排最前
function taskEndPins() {
  try {
    return (usageSet && usageSet.taskEnd && Array.isArray(usageSet.taskEnd.pins)) ? usageSet.taskEnd.pins.slice() : []
  } catch (err) { return [] }
}
function taskEndIsPinned(v) {
  var p = taskEndPins()
  for (var i = 0; i < p.length; i++) if (p[i] === v) return true
  return false
}
function taskEndTogglePin(v) {
  try {
    if (!v) return
    usageSet = usageSet || {}
    usageSet.taskEnd = usageSet.taskEnd || { on: false, sel: '' }
    var p = taskEndPins()
    var idx = -1
    for (var i = 0; i < p.length; i++) if (p[i] === v) { idx = i; break }
    if (idx >= 0) p.splice(idx, 1)
    else p.push(v)
    usageSet.taskEnd.pins = p
    saveUsageSettings({ taskEnd: usageSet.taskEnd })
    // 重排下拉并保持打开状态(不清除当前选择)
    fillTaskEndOptions((usageSet && usageSet.taskEnd) || null)
  } catch (err) {}
}
// v761（全局音效设置）：**通用事件音效播放**。
// 任务结束音 / 提问音 / 授权音三处共用；不改写 playTaskEndSound 里那段已验证的取值逻辑
// （frag: 片段 / preset: 预设 / grp: 音效组 + 备用路由），而是**临时替换它读取的绑定与音量再还原**。
// v761：音量取值统一走这里 —— **绝不能用 `Number(x) || 0.9`**：0 是合法音量，
// 但 `||` 把它当假值 ⇒ 滑块拖到 0 会变成 0.9（用户真机实测到的 bug）。
// 这里对 0 / NaN / 越界都给出确定行为：0 就是静音，NaN 才回落默认值。
function soundVolClamped(fallback) {
  var v = Number(soundVol)
  if (!isFinite(v)) v = (typeof fallback === "number") ? fallback : 0.9
  return Math.max(0, Math.min(1, v))
}
// v778：**音量解析器 = 音量的唯一真源**。所有播放入口都必须经过它取音量（禁止各自去读全局），
//   优先级：① 显式 override（绑定事件 / 试听传进来的值）→ ② 该事件**用户显式调过**的音量
//   （`cfg.volSet === true`）→ ③ ①区「按压音量」。
//   为什么需要 volSet 这一层：出厂默认 `events.turnCost.vol = 1`，而面板每次保存都会把这个键写进磁盘
//   ⇒ 只看"有没有值"无法区分「默认 100%」和「用户设了 100%」；若让每事件无条件优先，会把
//   「只调过①区、没碰过②区」的老用户的任务结束音静默抬到 100%（见 SPEC §24）。
//   ⚠️ 约定：本文件里 `soundVolClamped(` 只允许出现在本函数与它自己的定义处 —— 由 `_v778` 静态钉住。
function soundVolumeOf(kind, override) {
  try {
    if (typeof override === 'number' && isFinite(override) && override >= 0) {
      return Math.max(0, Math.min(1, override))
    }
    if (kind) {
      var cfg = soundEventCfg(kind) || {}
      if (cfg.volSet === true) {
        var v = Number(cfg.vol)
        if (isFinite(v) && v >= 0) return Math.max(0, Math.min(1, v))
      }
    }
    return soundVolClamped(0.9)
  } catch (err) { return soundVolClamped(0.9) }
}
// v778：绑定音（按压 / 提问 / 授权 / 试听）—— 把 `{sel, vol}` **显式传进去**。
//   旧实现靠"临时把全局 soundVol / usageSet.taskEnd 改掉、finally 再改回来"来传值；那正是
//   "每轮结束音读错音量"的同源隐患：任何新增播放入口忘了临时改一下，就会静默读全局（正是 0.3.16 的 bug）。
function playBindingSound(bind, ev) {
  try {
    if (!bind || !bind.on || soundOn === false) return
    var sel = bind.sel || ''
    if (!sel) return // 防御：空选择不出声（v762 起下拉里已无「静音」项，正常不会再为空）
    playTaskEndSound({ sel: sel, vol: (ev && typeof ev.vol === 'number') ? ev.vol : undefined })
  } catch (err) {}
}
// v761：等待交互时的**会话名**（wait.json 每秒轮询回填）+ 超长截断（默认 12 字符 + …）
var waitSessionName = ''
var WAIT_SESSION_MAX = 12
// v768：对话名的**统一取值**入口 —— 等待提示的 `{session}` 文案与「对话名」模块共用。
// max <= 0 或非数 = 不截断；名字为空时回落「当前对话」。
function bubbleSessionLabel(max) {
  var s = String(waitSessionName || '').trim()
  if (!s) s = '当前对话'
  var n = Number(max)
  if (!isFinite(n) || n <= 0) return s
  n = Math.max(1, Math.min(120, Math.round(n)))
  return s.length > n ? (s.slice(0, n) + '...') : s
}
function soundSessionLabel() { return bubbleSessionLabel(WAIT_SESSION_MAX) }
// 对话名模块的显示文本：模块自己的「保留长度」（m.len）优先，缺省 12
function bubbleSessionText(m) {
  m = m || {}
  return bubbleSessionLabel(m.len === undefined || m.len === null ? WAIT_SESSION_MAX : m.len)
}
// v778：`opts` = { sel?, vol? } —— 传入 sel 表示"播这个绑定音效"（绕过 taskEnd 开关，因为它自带 on）；
//   不传 sel = 路径 A（每轮结束音），按 usageSet.taskEnd 的门控与选择。
function playTaskEndSound(opts) {
  try {
    var o = opts || {}
    if (soundOn === false) return
    if (!o.sel) {
      if (!usageSet || !usageSet.taskEnd || !usageSet.taskEnd.on) return
    }
    var sel = o.sel || usageSet.taskEnd.sel || taskEndSel.value || ''
    var url = ''
    var altUrl = ''
    if (sel.indexOf('grp:') === 0) { playTaskEndGroupClick(sel.slice(4), o.vol); return }
    if (sel.indexOf('frag:') === 0) url = '/dsh-whale/audio-fragment.wav?id=' + encodeURIComponent(sel.slice(5))
    else if (sel.indexOf('preset:') === 0) {
      // v752：预设片段也改走片段路由（与本体按压同一套选择逻辑 + 备用路由）
      var parts = sel.split(':')
      var su = soundSlotUrls(parts[2] === 'release' ? 'release' : 'press', parts[1])
      url = su.url
      altUrl = su.alt || ''
    }
    if (!url) return
    var a = dshwvSound(url)
    if (altUrl) a._alt = altUrl
    // v778：音量统一走解析器（②区「提示音量」显式设置过就用它，否则跟随①区「按压音量」）
    try { a.volume = soundVolumeOf('turnCost', o.vol) } catch (err) {}
    a.play().catch(function () {})
  } catch (err) {}
}
// 任务结束音=音效组时:模拟点按一次该组——音1完整播放结束后立即接音2,
// 即“按下到音1结束才松开”的无缝连续点按听感；槽位留空(该事件静音)时跳过对应音频
function playTaskEndGroupClick(groupId, volOverride) {
  try {
    if (!groupId) return
    var g = null
    for (var gi = 0; gi < audioGroups.length; gi++) if (audioGroups[gi] && audioGroups[gi].id === groupId) { g = audioGroups[gi]; break }
    var pressEmpty = !!(g && g.press === '') // 仅显式留空算静音;缺失字段(null)按旧数据回落预设
    var releaseEmpty = !!(g && g.release === '')
    if (pressEmpty && releaseEmpty) return
    // v778：**"任务结束音 = 音效组"也必须走解析器**（第三方补丁只改了单片段那条路，音效组仍读①区 ⇒ 滑块照旧无效）
    var vol = soundVolumeOf('turnCost', volOverride)
    // v752：任务结束音同样改走片段路由（与本体按压同一套 URL 选择 + 备用路由逻辑）——
    // 老路由 /dsh-whale/sound/*.mp3?set=… 在部分环境会被本机那层东西拦成空的 204，
    // 不改的话"点按有声、但每轮结束音没声"会变成同一个问题的另一半。
    var pu = soundSlotUrls('press', groupId)
    var ru = soundSlotUrls('release', groupId)
    // 按压留空:无按下音,直接播松开(模拟按下即松开的完整点按);松开留空:只播按压
    if (pressEmpty) {
      if (!releaseEmpty && ru.url) {
        dshwvWarm([ru.url]) // v745：先预热
        var relOnly = dshwvSound(ru.url)
        relOnly._alt = ru.alt || ''
        try { relOnly.volume = vol } catch (err) {}
        relOnly.currentTime = 0
        var pr = relOnly.play()
        if (pr && pr.catch) pr.catch(function () {})
      }
      return
    }
    if (!pu.url) return
    var press = dshwvSound(pu.url)
    press._alt = pu.alt || ''
    try { press.volume = vol } catch (err) {}
    // v745：菜单里的"点一下试听"同样先预热解码，否则第一次听有明显延迟
    dshwvWarm([pu.url, releaseEmpty ? '' : ru.url])
    if (releaseEmpty || !ru.url) {
      press.currentTime = 0
      var pp = press.play()
      if (pp && pp.catch) pp.catch(function () {})
      return
    }
    var release = dshwvSound(ru.url)
    release._alt = ru.alt || ''
    try { release.volume = vol } catch (err) {}
    var relPlayed = false
    function playRel() {
      if (relPlayed) return
      relPlayed = true
      try { release.currentTime = 0; var p = release.play(); if (p && p.catch) p.catch(function () {}) } catch (err) {}
    }
    press.onended = function () {
      playRel()
    }
    press.currentTime = 0
    var p0 = press.play()
    if (p0 && p0.catch) p0.catch(function () {})
  } catch (err) {}
}
// 峰谷称呼已迁入「峰谷模块」内按模块设置(主菜单不再提供全局选择)
var bubbleToggle = document.createElement('input')
bubbleToggle.type = 'checkbox'
bubbleToggle.className = 'dshwv-check'
bubbleToggle.checked = true
bubbleToggle.title = '开启/关闭思考气泡'
bubbleToggle.addEventListener('change', function () { setBubbleOn(bubbleToggle.checked) })
var turnCostToggle = document.createElement('input')
turnCostToggle.type = 'checkbox'
turnCostToggle.className = 'dshwv-check'
turnCostToggle.checked = true
turnCostToggle.title = '每轮对话结束后自动显示本轮消耗金额'
turnCostToggle.addEventListener('change', function () { setTurnCostOn(turnCostToggle.checked) })
var turnCostCloseInput = document.createElement('input')
turnCostCloseInput.type = 'number'
turnCostCloseInput.min = '0'
turnCostCloseInput.step = '1'
turnCostCloseInput.className = 'dshwv-number'
turnCostCloseInput.value = '5'
turnCostCloseInput.disabled = false // 跟随「每轮消耗提示」开关
turnCostCloseInput.title = '填 0 表示不自动关闭，需手动点击关闭'
turnCostCloseInput.addEventListener('input', function () { setTurnCostClose(turnCostCloseInput.value) })
turnCostCloseInput.addEventListener('change', function () { setTurnCostClose(turnCostCloseInput.value) })
var scrollGapToggle = document.createElement('input')
scrollGapToggle.type = 'checkbox'
scrollGapToggle.className = 'dshwv-check'
scrollGapToggle.checked = false
scrollGapToggle.title = '开启后挂件右侧按设定像素避开滚动条；关闭则贴边（盖住滚动条）'
scrollGapToggle.addEventListener('change', function () { setScrollGapOn(scrollGapToggle.checked) })
var scrollGapInput = document.createElement('input')
scrollGapInput.type = 'number'
scrollGapInput.min = '0'
scrollGapInput.step = '1'
scrollGapInput.className = 'dshwv-number'
scrollGapInput.value = '17'
scrollGapInput.disabled = true // 默认避让关 → 宽度不可修改，勾选后启用
scrollGapInput.title = '避让滚动条的像素宽度，填 0 表示贴边'
scrollGapInput.addEventListener('input', function () { setScrollGapPx(scrollGapInput.value) })
scrollGapInput.addEventListener('change', function () { setScrollGapPx(scrollGapInput.value) })
var row1 = menuRow()
row1.appendChild(menuLabel('大小'))
row1.appendChild(scaleInput)
row1.appendChild(scaleNumber)
// —— 音量滑块与百分比（v761 起搬进「提示与音效设置」面板，主菜单不再占行）——
var volInput = document.createElement('input')
volInput.type = 'range'
volInput.min = '0'
volInput.max = '1'
volInput.step = '0.05'
volInput.className = 'dshwv-range'
volInput.value = '1'
var volPct = document.createElement('span')
volPct.className = 'dshwv-volpct'
volPct.textContent = '100%'
volInput.addEventListener('input', function () { setVol(volInput.value) })
// 音效总开关（v753/issue #135 引入；v762 起只由「提示与音效设置」面板 ① 表头的同名开关驱动，
// 主菜单「音效与提示」行不再放它 —— 但元素保留，因为 setSoundOn()/配置回填会同步它的 checked）
var soundToggle = document.createElement('input')
soundToggle.type = 'checkbox'
soundToggle.className = 'dshwv-check'
soundToggle.checked = true
soundToggle.title = '音效总开关：关掉后不出声（含音效组试听），并立刻挂起音频上下文、把系统睡眠交还给你'
soundToggle.addEventListener('change', function () { setSoundOn(soundToggle.checked) })
// v762：「音效与提示」行 = 标签 + 面板入口（总开关搬进面板 ① 表头，本行不再放开关）。
// 原「音效」行里的音效组/导入、原「音量」行的滑块，以及每轮消耗/提问/授权的音效，全部收进面板。
// ⚠️ soundToggle 这个元素**仍然创建**（setSoundOn() 与配置回填都会 `soundToggle.checked = ...`），
//    只是不再挂到本行上。删掉它的创建会让那两处抛 ReferenceError（本项目栽过整个挂件不出现）。
var row2 = menuRow()
row2.appendChild(menuLabel('音效与提示'))
var soundPanelBtn = document.createElement('button')
soundPanelBtn.type = 'button'
soundPanelBtn.className = 'dshwv-roleimport'
soundPanelBtn.style.flex = '1'
soundPanelBtn.textContent = '全局设置'
soundPanelBtn.title = '提示与音效设置：按压音量/音效组、每轮消耗提示、提问提示、授权提示'
soundPanelBtn.addEventListener('click', function (e) { e.stopPropagation(); openSoundSettingsPanel() })
row2.appendChild(soundPanelBtn)
var row6 = menuRow()
row6.appendChild(menuLabel('气泡全局开关'))
row6.appendChild(bubbleToggle)
// —— 自定义泡泡：主编辑窗口(点击队列/模块) ——
var bubbleCustomBtn = document.createElement('button')
bubbleCustomBtn.type = 'button'
bubbleCustomBtn.className = 'dshwv-roleimport'
bubbleCustomBtn.style.flex = '1'
bubbleCustomBtn.textContent = '按压泡泡设置'
bubbleCustomBtn.title = '打开“按压泡泡设置”（按压时的泡泡内容与队列）'
bubbleCustomBtn.addEventListener('click', function (e) { e.stopPropagation(); openBubbleEditor() })
row6.appendChild(bubbleCustomBtn)
var menuSep1 = document.createElement('div')
menuSep1.className = 'dshwv-menu-sep'
// v720:「自动关闭 n 秒」与「任务结束音效」都收进「自定义提示」窗口,这一行只留开关 + 入口按钮
var row7 = menuRow()
row7.appendChild(menuLabel('每轮消耗提示'))
row7.appendChild(turnCostToggle)
var turnCostCustomBtn = document.createElement('button')
turnCostCustomBtn.type = 'button'
turnCostCustomBtn.className = 'dshwv-roleimport'
turnCostCustomBtn.style.flex = '1' // 占满本行剩余空位(与「自定义泡泡」按钮一致)
turnCostCustomBtn.textContent = '自定义提示'
turnCostCustomBtn.title = '自定义每轮消耗提示内容(金额用 {cost})、自动关闭秒数、任务结束音效'
turnCostCustomBtn.addEventListener('click', function (e) { e.stopPropagation(); usageAlertBudgetEditor('cost', null) })
row7.appendChild(turnCostCustomBtn)
var row9 = menuRow()
row9.appendChild(menuLabel('避让滚动条'))
row9.appendChild(scrollGapToggle)
row9.appendChild(menuLabel('宽度'))
row9.appendChild(scrollGapInput)
row9.appendChild(menuLabel('px'))
// —— 自定义角色：下拉选择 + 导入 ——
var roleBtn = document.createElement('button')
roleBtn.type = 'button'
roleBtn.className = 'dshwv-rolebtn'
roleBtn.title = '选择角色'
var roleBtnLabel = document.createElement('span')
roleBtnLabel.className = 'dshwv-btnlabel'
roleBtnLabel.textContent = '小鲸鱼'
roleBtn.appendChild(roleBtnLabel)
var rolePanel = document.createElement('div')
rolePanel.className = 'dshwv-rolelist'
var roleImportBtn = document.createElement('button')
roleImportBtn.type = 'button'
roleImportBtn.className = 'dshwv-roleimport'
roleImportBtn.textContent = '导入'
roleImportBtn.title = '导入自定义角色图片'
var rowRole = menuRow()
rowRole.appendChild(menuLabel('角色'))
rowRole.appendChild(roleBtn)
rowRole.appendChild(roleImportBtn)
var roleFileInput = document.createElement('input')
roleFileInput.type = 'file'
roleFileInput.accept = 'image/*'
roleFileInput.style.display = 'none'
roleBtn.addEventListener('click', function (e) { e.stopPropagation(); toggleRolePanel() })
roleImportBtn.addEventListener('click', function (e) { e.stopPropagation(); roleFileInput.click() })
roleFileInput.addEventListener('change', function () { onRoleFileChosen(roleFileInput) })
menuBox.appendChild(rowRole)
menuBox.appendChild(row1)
menuBox.appendChild(row2)
menuBox.appendChild(row6)
// v762：「每轮消耗提示」整行**不再挂到主菜单**（菜单里已再无此行，配置入口移进「全局设置」面板）。
// ⚠️ row7 及其内部控件（turnCostToggle / turnCostCustomBtn）的创建**必须原样保留**：
//    turnCostToggle 仍被 OPEN 时的状态同步与 setTurnCostOn() 引用，**取消挂载 ≠ 删除创建**。
//    （本文件曾在「删了一行、别处还引用 rowN」上踩过：init 抛 TypeError 被外层 try 静默吞掉，
//      表现就是挂件整个不出现。）
// 「任务结束音效」不再占主菜单(v720):控件挂在一个不插入文档的宿主上,
// 「自定义提示」窗口打开时再把它搬进窗口。需要宿主是因为 dshwCustSel 初始化要求 select 已有父节点。
var taskEndRowHost = document.createElement('div')
taskEndRowHost.appendChild(taskEndSel)
taskEndDrop = dshwCustSel(taskEndSel, {
  // 菜单态:下拉高度不越过底部「小鲸鱼记账」按钮;窗口态(remind mask 存在)不做这个限制
  bottom: function () { return window.__dshwRemindMask ? null : usageNavRow },
  scrollNames: true,
  isPinned: taskEndIsPinned,
  onPin: taskEndTogglePin,
})
// 固定任务结束音选择框宽度(实测 120px),不再随容器宽度伸缩
try {
  var taskEndWrapEl = taskEndSel.parentNode
  if (taskEndWrapEl) {
    taskEndWrapEl.style.flex = '0 0 120px'
    taskEndWrapEl.style.width = '120px'
    taskEndWrapEl.style.maxWidth = '120px'
  }
} catch (err) {}
menuBox.appendChild(menuSep1)
menuBox.appendChild(row9)
// —— 吸附与翻转：自定义按钮（打开吸附/翻转设置弹窗），放在“避让滚动条”下一行 ——
var rowSnap = menuRow()
var snapRowLabel = document.createElement('span')
snapRowLabel.textContent = '吸附与翻转'
var snapCustomBtn = document.createElement('button')
snapCustomBtn.type = 'button'
snapCustomBtn.className = 'dshwv-roleimport'
snapCustomBtn.textContent = '自定义'
snapCustomBtn.title = '自定义各边吸附区宽度与翻转线位置'
snapCustomBtn.addEventListener('click', function (e) { e.stopPropagation(); openSnapModal() })
rowSnap.appendChild(snapRowLabel)
rowSnap.appendChild(snapCustomBtn)
menuBox.appendChild(rowSnap)
// —— 隐藏菜单按钮:开启后右键/长按小鲸鱼可唤出菜单(位置与按钮唤出一致) ——
var menuHideToggle = document.createElement('input')
menuHideToggle.type = 'checkbox'
menuHideToggle.className = 'dshwv-check'
menuHideToggle.checked = false
menuHideToggle.title = '启用后隐藏挂件上的菜单按钮;电脑端右键、手机端长按小鲸鱼可唤出菜单(位置不变)'
menuHideToggle.addEventListener('change', function () { setMenuBtnHide(menuHideToggle.checked) })
var rowHide = menuRow()
rowHide.appendChild(menuLabel('隐藏菜单按钮'))
rowHide.appendChild(menuHideToggle)
menuBox.appendChild(rowHide)
// —— Codex 本机统计开关（issue #116）：v748 起**从主菜单挪进「Codex 模型的设置子菜单」** ——
// 主菜单不再有这一项（它对没配 Codex 模型的用户没有意义）。宿主侧同样按"有没有 Codex 模型"兜底：
// 没配 → 默认关闭、完全不扫 ~/.codex/sessions。这里只保留一个"当前挂载的那个复选框"的引用，
// 供 setCodexStatsOn() 与配置读回时同步勾选状态（菜单是每次重建的，所以元素现建现用）。
var codexStatsToggle = null
function codexStatsCheckbox() {
  var el = document.createElement('input')
  el.type = 'checkbox'
  el.className = 'dshwv-check'
  el.checked = codexStatsOn !== false
  el.title = '关闭后不再读取 ~/.codex/sessions 统计本机 Codex 用量（会话日志很大时建议关闭）'
  el.addEventListener('change', function () { setCodexStatsOn(el.checked) })
  codexStatsToggle = el
  return el
}
// —— 资源管理:集中查看/删除已导入的图片与音频(角色图/泡泡图/音频片段/音效组) ——
var rowRes = menuRow()
var resOpenBtn = document.createElement('button')
resOpenBtn.type = 'button'
resOpenBtn.className = 'dshwv-usage-more'
resOpenBtn.style.flex = '1'
resOpenBtn.style.margin = '0'
resOpenBtn.textContent = '管理'
resOpenBtn.title = '集中管理当前导入插件的图片与音频资源'
resOpenBtn.addEventListener('click', function (e) { e.stopPropagation(); openResManager() })
rowRes.appendChild(menuLabel('资源管理'))
rowRes.appendChild(resOpenBtn)
menuBox.appendChild(rowRes)
// —— 用量记录:底部固定动作行由「用量记录 / ‹ 返回」共用(见下方 nav 行) ——

// ===== 用量记录(主菜单内的子界面,不跳新页面)+ “更多消费记录”窗口 =====
var USAGE_REC_URL = '/dsh-whale/usage-records.json'
var usageSet = null // {taskEnd,alert,budget} 用量设置缓存
var USAGE_SET_URL = '/dsh-whale/usage-settings.json'
function loadUsageSettings(cb) {
  try {
    fetch(USAGE_SET_URL, { cache: 'no-store' })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (d && d.ok && d.settings) usageSet = d.settings
        // 顺带拉一次自定义 API 模型（余额/今日已用/额度/各模型提醒）
        try { loadApiModels(null, true) } catch (err) {}
        if (cb) cb()
      })
      .catch(function () { if (cb) cb() })
  } catch (err) { if (cb) cb() }
}
// v778：与 `configPut()` **同一套语义**（读响应 → 失败静默重试一次 → 仍失败才提示用户）。
//   旧实现只 `fetch(...).then(r=>r.json()).then(d=>{...}).catch(function(){})`：**不查 HTTP 状态码、
//   失败完全静默** ⇒ 用户点了「保存」、面板正常关闭，其实一个字节都没写进磁盘（第三方分析报告点出的
//   现象是真的，只是它把归因写成了"消耗数据丢存"——这个接口存的是**设置**，消耗记录由宿主自己记账）。
function usageSettingsPut(patch, retried) {
  return fetch(USAGE_SET_URL, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(patch || {}),
  })
    .then(function (r) {
      return r.json().catch(function () { return null }).then(function (d) { return { ok: r.ok, status: r.status, d: d } })
    })
    .then(function (x) {
      if (x.ok && (!x.d || x.d.ok !== false)) {
        if (x.d && x.d.settings) usageSet = x.d.settings
        return true
      }
      if (!retried) return new Promise(function (res) { setTimeout(function () { res(usageSettingsPut(patch, true)) }, 900) })
      // 复用既有 toast（内部已对动态值做转义，见 v756/issue #143）
      configSaveFailNotice((x.d && x.d.error) || ('HTTP ' + x.status))
      return false
    })
    .catch(function (err) {
      if (!retried) return new Promise(function (res) { setTimeout(function () { res(usageSettingsPut(patch, true)) }, 900) })
      configSaveFailNotice((err && err.message) || err)
      return false
    })
}
function saveUsageSettings(patch) {
  try { return usageSettingsPut(patch, false) } catch (err) { return null }
}
// 把主菜单已有行收进 menuRootView;用量记录作为 menuBox 内的子视图切换
var menuRootView = document.createElement('div')
menuRootView.className = 'dshwv-menuview'
while (menuBox.firstChild) menuRootView.appendChild(menuBox.firstChild)
menuBox.appendChild(menuRootView)
var usagePanel = document.createElement('div')
usagePanel.className = 'dshwv-usage-sub'
usagePanel.style.display = 'none'
// 用量内容区:位于底部按钮上方的独立裁剪区域,滑入/滑出不会盖住按钮
var usageArea = document.createElement('div')
usageArea.className = 'dshwv-usage-area'
usageArea.style.cssText = 'flex:1 1 auto;min-height:0;overflow:hidden;position:relative'
menuBox.appendChild(usageArea)
usageArea.appendChild(usagePanel)
// 底部固定动作行:始终可见;主菜单态=「用量记录」,用量态=「‹ 返回」
var usageNavRow = menuRow()
usageNavRow.style.flex = '0 0 auto'
usageRecBtn.style.width = '100%'
usageNavRow.appendChild(usageRecBtn)
menuBox.appendChild(usageNavRow)
var usagePanelOpen = false
var usageRefreshTimer = null
var usageMainEl = null // 用量子界面可刷新内容区(保留顶部返回条)
var usageHover = false // 鼠标停在用量子界面上
var usageBusyUntil = 0 // 用户刚滚动过：这段时间内跳过自动重绘（避免闪烁/跳回顶部）
var usagePendingScroll = null // 重建子界面时待恢复的滚动位置
// 交互标记：悬浮/滚动/触摸期间不做自动重绘，用户查看近七天等下方内容时不被打断
usagePanel.addEventListener('mouseenter', function () { usageHover = true })
usagePanel.addEventListener('mouseleave', function () { usageHover = false; usageBusyUntil = Date.now() + 1500 })
usagePanel.addEventListener('wheel', function () { usageBusyUntil = Date.now() + 3000 }, { passive: true })
usagePanel.addEventListener('touchmove', function () { usageBusyUntil = Date.now() + 3000 }, { passive: true })
function toggleUsagePanel() {
  if (usagePanelOpen) { hideUsageSub(); return }
  showUsageSub()
}
function dshwvPlayViewIn(el) {
  if (!el) return
  el.classList.remove('dshwv-view-in')
  void el.offsetWidth // 强制重排,让动画每次都能重播
  el.classList.add('dshwv-view-in')
}
var usageHideTimer = null
var usageShowTimer = null
function setUsageNavBtn(inUsage) {
  try {
    usageRecBtn.textContent = inUsage ? '‹ 返回' : '- = 小鲸鱼记账 = -'
    usageRecBtn.title = inUsage ? '返回主菜单' : '查看今日/近7天/全部消费记录'
  } catch (err) {}
}
function showUsageSub() {
  try { if (usageHideTimer) { clearTimeout(usageHideTimer); usageHideTimer = null } } catch (err) {}
  usagePanelOpen = true
  // 锁定外层 menuBox 当前可见的宽高;用量视图占按钮上方区域并内部滚动
  var w0 = 300
  var h0 = 360
  try {
    var mb = menuBox.getBoundingClientRect()
    if (mb.width > 0) w0 = Math.round(mb.width)
    if (mb.height > 0) h0 = Math.round(mb.height)
  } catch (err) {}
  menuBox.style.width = w0 + 'px'
  menuBox.style.maxWidth = w0 + 'px'
  menuBox.style.height = h0 + 'px'
  menuBox.style.overflow = 'hidden'
  menuBox.style.display = 'flex'
  menuBox.style.flexDirection = 'column'
  // 整体上滑:主菜单叠进同一可视区并上滑离场,用量内容从下方同步接入
  try {
    if (menuRootView && usageArea) {
      if (menuRootView.parentNode !== usageArea) usageArea.appendChild(menuRootView)
    }
  } catch (err) {}
  usageArea.style.display = 'block'
  usagePanel.style.display = 'block'
  usagePanel.style.position = 'absolute'
  usagePanel.style.top = '0'
  usagePanel.style.left = '0'
  usagePanel.style.width = '100%'
  usagePanel.style.height = '100%'
  usagePanel.style.maxHeight = 'none'
  usagePanel.style.overflowY = 'auto'
  usagePanel.style.zIndex = '1'
  usagePanel.style.transform = 'translateY(100%)'
  usagePanel.style.transition = 'none'
  if (menuRootView) {
    menuRootView.style.display = 'block'
    menuRootView.style.position = 'absolute'
    menuRootView.style.top = '0'
    menuRootView.style.left = '0'
    menuRootView.style.width = '100%'
    menuRootView.style.height = '100%'
    menuRootView.style.zIndex = '2'
    menuRootView.style.transform = 'translateY(0)'
    menuRootView.style.transition = 'none'
  }
  setUsageNavBtn(true)
  renderUsagePanel()
  try { void usageArea.offsetHeight } catch (err) {}
  usagePanel.style.transition = 'transform .22s ease'
  usagePanel.style.transform = 'translateY(0)'
  if (menuRootView) {
    menuRootView.style.transition = 'transform .22s ease'
    menuRootView.style.transform = 'translateY(-100%)'
  }
  // 动画结束后主菜单叠层隐藏(仍留在 area 内,返回时复用同框滑回)
  usageShowTimer = setTimeout(function () {
    try { if (menuRootView) menuRootView.style.display = 'none' } catch (err) {}
  }, 240)
  if (usageRefreshTimer) { clearInterval(usageRefreshTimer); usageRefreshTimer = null }
  // 子界面打开期间每 10s 自动刷新(今日数据实时同步)；
  // 鼠标停在界面上或刚滚动过就跳过这一轮，避免正在看下方内容时被重绘打断
  usageRefreshTimer = setInterval(function () {
    if (!usagePanelOpen) return
    if (usageHover || Date.now() < usageBusyUntil) return
    renderUsagePanel()
  }, 10000)
}
function hideUsageSub() {
  if (usageRefreshTimer) { clearInterval(usageRefreshTimer); usageRefreshTimer = null }
  if (usageHideTimer) { clearTimeout(usageHideTimer); usageHideTimer = null }
  if (usageShowTimer) { clearTimeout(usageShowTimer); usageShowTimer = null }
  // 主菜单态(用量未打开)被 closeMenu 误调时直接返回:不做整体滑回编排,
  // 否则会把主菜单行瞬移进 usageArea,造成"内容瞬隐、仅底部按钮淡出"。
  if (!usagePanelOpen) { setUsageNavBtn(false); return }
  usagePanelOpen = false
  if (!usagePanel) { setUsageNavBtn(false); return }
  // 点击返回立即恢复主态按钮文案(不等动画结束)
  setUsageNavBtn(false)
  // 整体上下滑:主菜单临时叠进同一可视区(位于用量内容之下),用量向下滑出、
  // 主菜单同步从上方滑入,观感为一张面板整体上下滑动(无两段分离/空档)
  try {
    if (menuRootView && usageArea) {
      if (menuRootView.parentNode !== usageArea) usageArea.appendChild(menuRootView)
    }
  } catch (err) {}
  usagePanel.style.position = 'absolute'
  usagePanel.style.top = '0'
  usagePanel.style.left = '0'
  usagePanel.style.width = '100%'
  usagePanel.style.height = '100%'
  usagePanel.style.zIndex = '2'
  usagePanel.style.transform = 'translateY(0)'
  usagePanel.style.transition = 'none'
  if (menuRootView) {
    menuRootView.style.display = 'block'
    menuRootView.style.position = 'absolute'
    menuRootView.style.top = '0'
    menuRootView.style.left = '0'
    menuRootView.style.width = '100%'
    menuRootView.style.height = '100%'
    menuRootView.style.zIndex = '1'
    menuRootView.style.transform = 'translateY(-100%)'
    menuRootView.style.transition = 'none'
  }
  try { void usageArea.offsetHeight } catch (err) {}
  usagePanel.style.transition = 'transform .22s ease'
  usagePanel.style.transform = 'translateY(100%)'
  if (menuRootView) {
    menuRootView.style.transition = 'transform .22s ease'
    menuRootView.style.transform = 'translateY(0)'
  }
  usageHideTimer = setTimeout(function () {
    usageHideTimer = null
    try { usagePanel.style.display = 'none' } catch (err) {}
    try {
      usagePanel.style.transform = ''
      usagePanel.style.transition = ''
      usagePanel.style.width = ''
      usagePanel.style.height = ''
      usagePanel.style.maxHeight = ''
      usagePanel.style.position = ''
      usagePanel.style.top = ''
      usagePanel.style.left = ''
      usagePanel.style.zIndex = ''
    } catch (err) {}
    // 主菜单移回正常流位置(usageArea 之前)
    if (menuRootView && usageArea) {
      try {
        if (usageArea.contains(menuRootView)) menuBox.insertBefore(menuRootView, usageArea)
        menuRootView.style.transform = ''
        menuRootView.style.transition = ''
        menuRootView.style.position = ''
        menuRootView.style.top = ''
        menuRootView.style.left = ''
        menuRootView.style.width = ''
        menuRootView.style.height = ''
        menuRootView.style.zIndex = ''
      } catch (err) {}
    }
    try { usageArea.style.display = '' } catch (err) {}
    if (menuBox) {
      menuBox.style.width = ''
      menuBox.style.maxWidth = ''
      menuBox.style.height = ''
      menuBox.style.overflow = ''
      menuBox.style.display = ''
      menuBox.style.flexDirection = ''
    }
  }, 230)
}
function closeUsagePanel() { hideUsageSub() }
function usageMoney(x, currency) { return apiFmtMoney(x, currency || 'CNY') }
function usageDayLabel(day) {
  try {
    var d = day.split('-')
    if (d.length !== 3) return day
    var now = new Date()
    var cur = String(now.getFullYear()) + '-' + String(now.getMonth() + 1).padStart(2, '0') + '-' + String(now.getDate()).padStart(2, '0')
    if (day === cur) return '今天'
    return (d[1] + '-' + d[2])
  } catch (err) { return day }
}
function renderUsagePanel() {
  if (usageMainEl && usagePanelOpen && usageSet !== null) { refreshUsageMain(); return }
  buildUsageSubShell()
  refreshUsageMain()
}
// 子界面骨架:预警/预算设置(静态,不被刷新重建) + 记录区(可刷新);底部「‹ 返回」由 menuBox 底部固定按钮统一提供
function buildUsageSubShell() {
  usagePanel.innerHTML = ''
  // 居中小标题
  var subTitle = document.createElement('div')
  subTitle.className = 'dshwv-usage-subtitle'
  subTitle.textContent = '- = 小鲸鱼记账 = -'
  usagePanel.appendChild(subTitle)
  // 预警与预算设置区(静态)
  buildUsageSettingsArea()
  usageMainEl = document.createElement('div')
  usageMainEl.className = 'dshwv-usagebody'
  usagePanel.appendChild(usageMainEl)
}
// —— 预警/预算提醒内容编辑:直接复用 W2 模块编辑组件(可选模块入框/拖入、行内并排、拖动排序、按类型编辑、真实预览) ——
// ===== v761（全局音效设置面板）：一个窗口里调四个事件 =====
// 按压音效 / 每轮消耗提示 / 提问音效 / 授权音效，各自的音效 + 独立音量 + 自动关闭 + 冒泡。
// 默认值必须与宿主 whale-balance.mjs 的 soundEventsDefaults() **逐字段一致**
// （宿主 = 新用户默认；这里 = 面板的读回兜底与「恢复默认」的目标，改一处必须同步另一处）。
function soundEventDefaults() {
  return {
    // 按压音效（按压 + 松开）：音量与音效组仍在 .dsh-size.json（vol / soundSet），这里只占位
    press: { vol: 1 },
    // ⚠️ turnCost **刻意不带** soundOn：它的「这个音效播不播」就是既有的 usageSet.taskEnd.on
    //    （宿主 soundEventsDefaults() 的注释与实现完全一致）。
    //    若给它加 soundOn: true 再同步写 taskEnd.on，会把老用户的任务结束音从「默认关」变成
    //    「默认开」—— 这是不能悄悄发生的默认行为变更。
    // ⚠️ v764：turnCost **也不再带** autoClose / ttlSec —— 它们从来没有消费方，而面板 ② 区
    //    「自动关闭」的真实落点是 .dshw-size.json 的 turnCostCloseMs（见 openSoundSettingsPanel）。
    //    给 turnCost 重新长出这两个键，会让同一个设置又变回两个来源（其中一个还是假的）。
    turnCost: { vol: 1, volSet: false, bubbleOn: true },
    // ⚠️ v778：`volSet` = "用户**显式**调过这一事件的音量"（默认 false）。它只表达"要不要覆盖①区"，
    //    不是另一个音量来源 —— 取值永远看 `vol`。没有它就无法区分"默认 100%"与"用户设了 100%"，
    //    会让"②区优先"静默改掉老用户的响度（详见 soundVolumeOf() 与 SPEC §24）。
    //    ⚠️ 与宿主 soundEventsDefaults() 的同名字段**必须逐字段一致**（由探针 §⑥ 跨端一致钉住）。
    // v774：出厂默认 = 作者当前用法：提问/授权**开着**（要冒泡），但音效**不响**（soundOn: false）。
    // 要响就把音效行的 [✓] 勾上；不想冒泡就取消入口行的 [✓]。
    question: { on: true, soundOn: false, sel: 'frag:exp_orb', vol: 1, autoClose: true, ttlSec: 180, bubbleOn: true },
    approval: { on: true, soundOn: false, sel: 'frag:exp_orb', vol: 1, autoClose: true, ttlSec: 180, bubbleOn: true },
  }
}
// 读某个事件的设置：缺字段/类型不对一律回落默认（只读 usageSet，不写）
function soundEventCfg(kind) {
  var def = soundEventDefaults()[kind] || {}
  var src = (usageSet && usageSet.events && usageSet.events[kind]) || {}
  var out = {}
  for (var k in def) {
    var v = src[k]
    if (k === 'vol') { var n = Number(v); out.vol = (isFinite(n) && n >= 0 && n <= 1) ? n : def.vol }
    else if (k === 'ttlSec') { var s = Number(v); out.ttlSec = (isFinite(s) && s >= 0) ? Math.round(s) : def.ttlSec }
    else if (k === 'sel') out.sel = (typeof v === 'string') ? v : def.sel
    else out[k] = (typeof v === 'boolean') ? v : def[k]
  }
  return out
}
// 事件的保存载荷（只带 schema 里的键：turnCost 没有 on/sel/autoClose/ttlSec，宿主那侧的 lines 等内容一律不动）
function soundEventOut(cfg, withBind) {
  var o = {
    vol: Number(cfg.vol) || 0,
    bubbleOn: cfg.bubbleOn !== false,
  }
  // v778：`volSet` 只属于 turnCost（= `withBind` 为假的那一个）。提问/授权的音量在"用户勾了音效开关"
  //   时才出声，不存在"默认值 vs 显式值"的歧义，所以不需要这个键。
  if (!withBind) o.volSet = cfg.volSet === true
  if (withBind) {
    // v764：autoClose / ttlSec 跟着 on/sel/soundOn 一起，只属于提问/授权。turnCost 的「自动关闭」
    // 真实落点是 .dsh-size.json 的 turnCostCloseMs、**不走 events 载荷** —— 别再无条件塞进去，
    // 否则每次保存都会给 turnCost 重新写回这两个死键。
    o.autoClose = cfg.autoClose !== false
    o.ttlSec = Math.max(0, Math.round(Number(cfg.ttlSec) || 0))
    o.on = cfg.on !== false
    o.sel = (typeof cfg.sel === 'string') ? cfg.sel : ''
    // v762：音效行的「是否播这个音效」开关。**只有提问/授权**带这个键；
    // turnCost 的同类开关走既有的 taskEnd.on，不进 events.turnCost（与宿主 schema 一致）。
    o.soundOn = cfg.soundOn !== false
  }
  return o
}
// 提示与音效设置面板（主菜单「音效与提示」行 →「全局设置」按钮；v761 引入时叫「全局音效设置」）。
// 保存语义与既有窗口一致：窗口期间的改动**只落内存缓冲**（sndBuf），点「保存」才一次 PUT
// （events + taskEnd）；「取消」或点遮罩关闭 = 丢弃缓冲，并把**复用自主菜单的控件**
// （音量滑块 volInput、按压总开关、每轮消耗开关 —— 它们的监听本来就是即时提交的）还原回打开时的快照。
// v763（真机反馈修）：**「恢复默认」也必须守这条语义** —— 它只改内存缓冲与控件显示，一个字节都不落盘
// （既不调 setVol/setSoundOn/setTurnCostOn，也不 saveUsageSettings）；那几项（按压音量/音效总开关/
// 每轮消耗开关/自动关闭秒数，存在 .dsh-size.json）的真落盘统一由 saveAll() 负责，点「取消」则由
// cleanup(true) 回滚（自动关闭走 setTurnCostClose，收秒、落 .dsh-size.json 的 turnCostCloseMs）。
function openSoundSettingsPanel() {
  try {
    var DEF = soundEventDefaults()
    // v764：② 区「自动关闭」的出厂默认（秒）。它**不属于** events schema —— 真实落点是 .dshw-size.json 的
    // turnCostCloseMs（宿主 readSizeConfig() / 客户端 `var turnCostCloseMs = 5000` 的兜底值是 5 秒，
    // 但面板按设计稿把「恢复默认」的目标定为 180 秒，与发布说明里「自动关闭默认 180 秒」一致）。
    // 这个常量放在面板里，不再借（已摘除的）events.turnCost.ttlSec。
    var TC_CLOSE_DEFAULT_SEC = 180
    // 打开时的快照（只覆盖复用自菜单、会即时提交的那几个控件）
    var snapVol = Number(soundVol) || 0
    var snapSoundOn = soundOn !== false
    var snapTurnCostOn = !!turnCostOn
    // v763：面板 ②区「自动关闭」的真实落点是 .dsh-size.json 的 turnCostCloseMs（= 消耗泡泡的停留时长，
    // 唯一消费点 sceneOpen('cost', …, turnCostCloseMs > 0 ? turnCostCloseMs : 0)），所以它也要进快照、
    // 也要能随「取消」还原（否则面板那行「自动关闭」就是个死控件）。
    var snapTurnCostCloseMs = Math.max(0, Math.round(Number(turnCostCloseMs) || 0))
    // v777（用户要求）：底部新增「点按角色关闭提示气泡」——默认关（默认只有点泡泡能收起等待提示）。
    //   存在 usage.json 的 settings.wait.charClose（宿主 usageSettingsDefaults() 里默认 false）；
    //   窗口期间只改缓冲，保存才 PUT，取消按快照还原，与其它设置项同一套语义。
    var bufWaitCharClose = !!(((usageSet || {}).wait || {}).charClose)
    var snapWaitCharClose = bufWaitCharClose
    // 内存缓冲：窗口期间只改这里
    var sndBuf = {
      turnCost: soundEventCfg('turnCost'),
      question: soundEventCfg('question'),
      approval: soundEventCfg('approval'),
    }
    // v764：② 区「自动关闭」自己的缓冲（不再是 events.turnCost 的一部分）。初值直接取**真实值**
    // turnCostCloseMs —— 老实现显示的是 events.turnCost.ttlSec 里那个虚构的 180 秒，而消耗泡泡
    // 实际一直按 .dshw-size.json 的值关闭，那一行等于在骗人。窗口期间只改这个缓冲，
    // 真落盘仍由 saveAll() → setTurnCostClose() 负责（收「秒」）。
    var tcCloseBuf = {
      autoClose: snapTurnCostCloseMs > 0,
      ttlSec: Math.round(snapTurnCostCloseMs / 1000),
    }
    var teOld = (usageSet && usageSet.taskEnd) || {}
    var teOldSel = (typeof teOld.sel === 'string') ? teOld.sel : ''
    // 任务结束音：缓冲里存**真实选择**（taskEnd.on 的开关已由本行左侧的 [✓] 单独表达，
    // 不再像早期那样「on 为假就把下拉显示成静音」）。
    // v762：「静音」项已从下拉里删除，所以这里直接沿用已保存的 sel；空/失效值由
    // fillSoundSelect 按任务结束音原有的兜底逻辑挑一个默认音并写回 usageSet。
    var bufTaskEndSel = teOldSel
    var bufTaskEndKeep = teOldSel // 兜底：sel 万一被清空时保留原选择
    // v762：② 音效行的 [✓] = 既有的 usageSet.taskEnd.on（宿主 usageSettingsDefaults() 里
    // taskEnd 默认 { on: false } ⇒ 这里默认不勾）。**不引入 turnCost.soundOn**。
    var bufTaskEndOn = (teOld.on === true)
    // —— 窗口壳（与「提醒内容编辑器」同款卡片）——
    var mask = document.createElement('div')
    mask.className = 'dshwv-bubmask'
    // 必须高于「模型子菜单/模型设置」(29000) 才能正常操作
    mask.style.zIndex = '30000'
    var card = document.createElement('div')
    card.className = 'dshwv-bubcard'
    card.style.width = 'min(440px,calc(100vw - 16px))'
    card.style.maxHeight = '88vh'
    card.style.overflow = 'hidden auto'
    var title = document.createElement('div')
    title.className = 'dshwv-bubtitle'
    title.textContent = '提示与音效设置'
    card.appendChild(title)
    // 登记到运行时浮层表（issue #142 / CI 的 z 层审计）：面板内的自绘下拉靠 visibleTopZ()
    // 抬层，不登记的话它们会算在遮罩下面（点开却看不见）
    var unregMask = null
    try { if (typeof window.dshwRegisterMask === 'function') unregMask = window.dshwRegisterMask(mask) } catch (err) {}
    // —— 面板内的小工具 ——
    // 分区表头：表头文字前一个开关（该事件总开关）。
    // v762：右侧那条分隔线（─）**去掉了**，改成上下的留白 —— 用户要求「加一些间隔距离，
    // 不加分割线了」；四个表头的 [✓] + 标题照旧保留。
    // ⚠️ .dshwv-bubsec 是多个窗口共用的小节标题类（泡泡编辑器、预览区等），所以这里只用
    //    行内样式覆盖本面板，绝不去改全局 CSS（否则会连带改掉别的窗口）。
    // v765：面板改成「四个入口行 + 展开体」的手风琴 —— 入口行常驻（开关 + 区名 + 当前值摘要 + ▸/▾），
    // 点入口行才展开该区的设置项，**可同时展开多个**；默认全部收起。
    // v767：入口行/折叠体的实现抽到模块级 `dshwvFoldEntry()`（与资源管理窗口共用），这里只做一层包装。
    // ⚠️ 四个区的控件**照旧全部创建**，只是藏在折叠体里：applyDefaults() / saveAll() 会逐个回填与读取，
    //    懒创建会让「恢复默认」静默失效（那些回填都包在 try/catch 里，出错看不出来）。
    var sndEntries = [] // [{ kind, summary, sel }]，供 refreshSummaries() 更新入口行右侧的当前值
    function sndEntry(labelText, checked, onToggle, first) {
      return dshwvFoldEntry(card, labelText, {
        checked: checked,
        onToggle: onToggle,
        first: first,
        onRowClick: function () {
          // v766：点入口行（展开或收起）时，先把本面板可能开着的浮层收起来。为什么必须**显式**收：
          //   · 「音效组」列表（.dshwv-audiolist）是 body 上的固定层，而全局 onDocPointerDown 见到
          //     本面板的遮罩（.dshwv-bubmask）就直接 return（"面板内部交给面板自己处理"）⇒ 不主动收，
          //     它就会一直飘在收起后的入口行上面（用户实测：收起设置项后已打开的下拉菜单没关）。
          //   · 自绘音效下拉（.dshwv-custmenu）平时会被它自己的 document pointerdown 收掉，一起收更稳。
          // 与既有约定一致：菜单里点到别处也会 closeRolePanel() + closeAudioGroupPanel()。
          try { dshwCustSelClose() } catch (err) {}
          try { closeAudioGroupPanel() } catch (err) {}
          try { stopPanelPreview() } catch (err) {} // 正在试听的那一声也一起停（v766）
        },
      })
    }
    // 入口行右侧的「当前值摘要」：不展开也能看出这个事件现在是什么状态
    function soundNameOf(sel) {
      try {
        if (!sel || !sel.options || sel.selectedIndex < 0) return ''
        var o = sel.options[sel.selectedIndex]
        return String((o && o.textContent) || '')
      } catch (err) { return '' }
    }
    function sndPct(v) { var n = Number(v); if (!isFinite(n) || n < 0) n = 0; return Math.round(n * 100) + '%' }
    function refreshSummaries() {
      try {
        for (var i = 0; i < sndEntries.length; i++) {
          var e = sndEntries[i]
          var txt = ''
          if (e.kind === 'press') {
            if (soundOn === false) txt = '已关闭'
            else {
              var g = ''
              try { g = String(audioGroupBtnLabel.textContent || '') } catch (err) {}
              txt = (g || '默认音效组') + ' · ' + sndPct(soundVol)
            }
          } else if (e.kind === 'turnCost') {
            if (!turnCostOn) txt = '已关闭'
            else {
              var sec = Math.max(0, Math.round(Number(tcCloseBuf.ttlSec) || 0))
              var pre = (tcCloseBuf.autoClose !== false && sec > 0) ? ('自动关 ' + sec + 's') : '不自动关'
              // v768：静音时**不再显示已选的音效名**，直接告诉用户去哪儿改（用户要求的确切文案）
              txt = pre + ' · ' + (bufTaskEndOn ? (soundNameOf(e.sel) || '—') : '当前为静音，在下拉设置中修改')
              // v778：音量的两种状态写清楚 —— 没拖过 ②区滑块 = **跟随①区按压音量**；拖过 = 用这里的值。
              if (bufTaskEndOn) {
                if (sndBuf.turnCost.volSet === true) {
                  if (sndPct(sndBuf.turnCost.vol) !== '100%') txt += ' · ' + sndPct(sndBuf.turnCost.vol)
                } else {
                  txt += ' · 跟随按压音量 ' + sndPct(soundVol)
                }
              }
            }
          } else {
            var cfg = sndBuf[e.kind] || {}
            if (cfg.on === false) txt = '已关闭'
            else if (cfg.soundOn === false) txt = '当前为静音，在下拉设置中修改' // v768：静音时不显示已选音效
            else {
              txt = soundNameOf(e.sel) || '—'
              if (sndPct(cfg.vol) !== '100%') txt += ' · ' + sndPct(cfg.vol)
            }
          }
          try { if (e.summary) e.summary.textContent = txt } catch (err) {}
        }
        // v778：②区「提示音量」滑块在**未显式设置**时显示"实际生效的音量"（= ①区按压音量），
        //   否则滑块停在 100% 而实际按 60% 播 —— 那就又变成"界面在骗人"（这次修的就是这类问题）。
        try {
          var tcCfg = sndBuf.turnCost || {}
          if (tcVol && tcVol.input && tcCfg.volSet !== true) {
            var eff = soundVolumeOf('turnCost')
            tcVol.input.value = String(eff)
            if (tcVol.pct) tcVol.pct.textContent = Math.round(eff * 100) + '%'
          }
        } catch (err) {}
        // v773：摘要与"置灰状态"永远一起刷（所有既有刷新点都会走到这里：开面板 / 卡内 input·change /
        // 点文档任意处 / 「恢复默认」末尾）—— 所以开关一动，相关设置立刻跟着灰/亮，不需要另挂监听。
        applySndDisabled()
      } catch (err) {}
    }
    // v773：**音效没开 ⇒ 该区"音效相关设置"置灰且不可编辑**（用户要求）。
    //   · 只动**设置类**控件：音效下拉（自绘那颗按钮 + 原生 select）与音量滑块，以及它们那一行的标签；
    //   · **开关自己**（入口行 [✓] / 音效行 [✓]）永远保持可点 —— 它是"开回来"的唯一入口，
    //     置灰它就等于把用户锁死在外面；
    //   · **试听 ▶ 也保持可用** —— 它正好用来"先听一下再决定开不开"（不是设置，是预览）；
    //   · 与声音无关的行（冒泡提示 / 编辑提示内容 / 自动关闭）不动：它们归"事件总开关"管，各按各的。
    // 判定条件由各区给（读的都是**实时**状态：模块级 soundOn / turnCostOn、缓冲里的 bufTaskEndOn / cfg.soundOn）。
    var sndDisableGroups = []
    function sndReg(off, items) { sndDisableGroups.push({ off: off, items: items || [] }) }
    function sndDim(el, off) {
      try { if (el && el.style) el.style.opacity = off ? '.4' : '' } catch (err) {}
    }
    function applySndDisabled() {
      for (var i = 0; i < sndDisableGroups.length; i++) {
        var g = sndDisableGroups[i]
        var off = false
        try { off = !!g.off() } catch (err) { off = false }
        for (var j = 0; j < g.items.length; j++) {
          var it = g.items[j] || {}
          try { if (it.el) it.el.disabled = off } catch (err) {}
          // 自绘下拉：原生 select 是隐藏的，看得见的是它自己那颗按钮 ⇒ 两者都要处理
          try { if (it.drop && typeof it.drop.sync === 'function') it.drop.sync() } catch (err) {}
          sndDim(it.drop && it.drop.btn ? it.drop.btn : it.el, off)
          sndDim(it.label, off)
        }
      }
    }
    // 一行：标签 + 控件（控件由调用方 append 进来，沿用主菜单的行样式）
    // v765：多一个 host 参数 —— 行挂到哪个容器（默认 card；手风琴里传该区的折叠体）
    function row(labelText, host) {
      var r = menuRow()
      if (labelText) r.appendChild(menuLabel(labelText))
      ;(host || card).appendChild(r)
      return r
    }
    // 试听按钮（沿用各窗口里的行内小图标按钮 .dshwv-bubmini）
    function playBtn(titleText, fn) {
      var b = document.createElement('button')
      b.type = 'button'
      b.className = 'dshwv-bubmini'
      b.textContent = '▶'
      b.title = titleText
      b.addEventListener('click', function (e) { e.stopPropagation(); try { fn() } catch (err) {} })
      return b
    }
    // 独立音量滑块 + 百分比（只改缓冲，点「保存」才落盘）
    function volSlider(getV, setV) {
      var inp = document.createElement('input')
      inp.type = 'range'
      inp.min = '0'
      inp.max = '1'
      inp.step = '0.05'
      inp.className = 'dshwv-range'
      inp.value = String(getV())
      var pct = document.createElement('span')
      pct.className = 'dshwv-volpct'
      pct.textContent = Math.round(getV() * 100) + '%'
      inp.addEventListener('input', function () {
        var v = Number(inp.value)
        if (!isFinite(v)) v = 0
        setV(v)
        pct.textContent = Math.round(v * 100) + '%'
      })
      return { input: inp, pct: pct }
    }
    // v766：试听包裹 —— 期间新建的音频元素会进"试听名单"（见 dshwvPreviewTrack），
    // 收起设置项时由 stopPanelPreview() 一次停掉（片段/预设/音效组三条路都覆盖）。
    function playPreview(fn) {
      if (soundOn === false) { dshwvToast('声音总开关已关闭，请勾选“声音总开关与按压音效”后试听。'); return }
      try { dshwvPreviewOn() } catch (err) {}
      try { fn() } finally { try { dshwvPreviewOff() } catch (err) {} }
    }
    // v766：停掉本面板正在试听的声音。
    //   · ① 的「试听按压音效」走的是**模块级** pressAudio / releaseAudio（复用本体那两个元素）⇒ 直接停；
    //   · ②③④ 走 playBindingSound → playTaskEndSound（内部匿名新建元素）⇒ 靠试听名单停。
    // ⚠️ 只停试听，**不碰**真实的任务结束音 / 提问授权提示音（那是用户要听的事件音）。
    function stopPanelPreview() {
      try { if (pressAudio) dshwvSoundStop(pressAudio) } catch (err) {}
      try { if (releaseAudio) dshwvSoundStop(releaseAudio) } catch (err) {}
      try { dshwvStopPreviews() } catch (err) {}
    }
    // 自动关闭：开关 + 秒数（0 = 不自动关闭）；host = 挂到哪个容器（手风琴的折叠体）
    function autoCloseRow(cfg, host) {
      var r = row('自动关闭', host)
      var ck = document.createElement('input')
      ck.type = 'checkbox'
      ck.className = 'dshwv-check'
      ck.checked = cfg.autoClose !== false
      ck.title = '开启后泡泡到时间自动关闭'
      ck.addEventListener('change', function () { cfg.autoClose = !!ck.checked })
      var num = document.createElement('input')
      num.type = 'number'
      num.min = '0'
      num.step = '1'
      num.className = 'dshwv-winput'
      num.value = String(Math.max(0, Math.round(Number(cfg.ttlSec) || 0)))
      num.title = '自动关闭秒数（0 = 不自动关闭）'
      num.addEventListener('input', function () { cfg.ttlSec = Math.max(0, Math.round(Number(num.value) || 0)) })
      num.addEventListener('change', function () {
        cfg.ttlSec = Math.max(0, Math.round(Number(num.value) || 0))
        num.value = String(cfg.ttlSec)
      })
      r.appendChild(ck)
      r.appendChild(num)
      r.appendChild(menuLabel('秒'))
      return { chk: ck, num: num }
    }
    // 冒泡提示：开关 + 编辑提示内容（editFn 为空时才置灰；提问/授权已接入各自的编辑器）
    function bubbleRow(cfg, editFn, editTip, host) {
      var r = row('冒泡提示', host)
      var ck = document.createElement('input')
      ck.type = 'checkbox'
      ck.className = 'dshwv-check'
      ck.checked = cfg.bubbleOn !== false
      ck.title = '该事件是否用泡泡提示'
      ck.addEventListener('change', function () { cfg.bubbleOn = !!ck.checked })
      var btn = document.createElement('button')
      btn.type = 'button'
      btn.className = 'dshwv-roleimport'
      btn.style.flex = '1'
      btn.textContent = '编辑提示内容'
      btn.title = editTip
      if (editFn) {
        btn.addEventListener('click', function (e) { e.stopPropagation(); try { editFn() } catch (err) {} })
      } else {
        btn.disabled = true
        btn.style.opacity = '.5'
        btn.style.cursor = 'not-allowed'
      }
      r.appendChild(ck)
      r.appendChild(btn)
      return ck
    }
    // —— ① 按压音效 ——
    // v762 行序：音效组 → 音量（用户要求把「按压音效组」挪到「按压音量」**之前**）
    var pressEnt = sndEntry('声音总开关与按压音效', soundOn !== false, function (b) {
      // 与音效总开关同一个状态（setSoundOn 会同步那只保留但不再挂到菜单行的开关）
      setSoundOn(b)
    }, true)
    var pressOnChk = pressEnt.chk
    sndEntries.push({ kind: 'press', summary: pressEnt.summary, sel: null })
    var rPressGrp = row('按压音效组', pressEnt.body)
    rPressGrp.appendChild(audioGroupBtn) // 复用原「音效组」按钮
    audioImportBtn.textContent = '新建音效组'
    audioImportBtn.title = '新建/编辑音效组'
    rPressGrp.appendChild(audioImportBtn)
    // 这两个弹层是 body 上的固定层（.dshwv-audiolist / .dshwv-audiomask 都低于 30000），
    // 从本面板打开时必须临时抬到面板之上，否则会被遮罩整层盖住（点了没反应）
    try {
      audioGroupBtn.addEventListener('click', function () { try { audioGroupPanel.style.zIndex = '31000' } catch (err) {} })
      audioImportBtn.addEventListener('click', function () { try { audioEditMask.style.zIndex = '31000' } catch (err) {} })
    } catch (err) {}
    var rPressVol = row('按压音量', pressEnt.body)
    rPressVol.appendChild(volInput) // 复用主菜单搬出来的滑块（input 事件已接 setVol；取消时按快照还原）
    rPressVol.appendChild(volPct)
    // 本区**只保留音量行这一个试听按钮**（音量行自己不加 ▶，避免一个模块两个入口）
    rPressVol.appendChild(playBtn('试听按压音效', function () {
      playPreview(function () {
        playPress()
        // 音效组把按压槽留空时 playPress() 只置状态不出声 → 补一次松开音，保证试听总能听到该组
        if (!pressAudio) playRelease()
      })
    }))
    // v773：① 的"音效没开"= 入口行 [✓]（soundOn）没勾 ⇒ 音效组 / 按压音量 置灰不可编辑
    sndReg(function () { return soundOn === false }, [
      { el: audioGroupBtn, label: rPressGrp.firstChild },
      { el: audioImportBtn, label: null },
      { el: volInput, label: rPressVol.firstChild },
    ])
    // —— ② 每轮消耗提示 ——
    // v762 行序（与 ③④ 统一）：冒泡提示 → 自动关闭 → 音效行 → 提示音量
    // （DOM 顺序 = 创建顺序，所以这里按目标顺序调用即完成重排）
    // v763：② 表头的 [✓] 留个句柄，供 applyDefaults() 同步显示（直接改 .checked 不会触发 change ⇒ 不落盘）
    var tcEnt = sndEntry('每轮消耗提示', !!turnCostOn, function (b) { setTurnCostOn(b) })
    var tcOnChk = tcEnt.chk
    var tcBub = bubbleRow(sndBuf.turnCost, function () { usageAlertBudgetEditor('cost', null) }, '编辑每轮消耗提示的泡泡内容（金额用 {cost}）', tcEnt.body)
    var tcAc = autoCloseRow(tcCloseBuf, tcEnt.body)
    var rTe = row('任务结束音', tcEnt.body)
    // 音效行的 [✓] = 该事件是否播这个音效。② 直接绑**既有的** usageSet.taskEnd.on
    //（宿主 usageSettingsDefaults() 里 taskEnd 默认 { on: false } ⇒ 这里默认不勾）。
    var teSoundChk = document.createElement('input')
    teSoundChk.type = 'checkbox'
    teSoundChk.className = 'dshwv-check'
    teSoundChk.checked = bufTaskEndOn
    teSoundChk.title = '该事件是否播这个音效（关掉 = 不响；下拉里选的音效仍保留，下次勾上即用）'
    teSoundChk.addEventListener('change', function () { bufTaskEndOn = !!teSoundChk.checked })
    rTe.appendChild(teSoundChk)
    var teSel = document.createElement('select')
    teSel.className = 'dshwv-sound'
    teSel.title = '任务结束音：音效组（点按整组）/ 单个音频；播不播由左边的 [✓] 决定'
    teSel.addEventListener('change', function () { bufTaskEndSel = teSel.value })
    rTe.appendChild(teSel)
    rTe.appendChild(playBtn('试听任务结束音', function () {
      playPreview(function () { playBindingSound({ on: true, sel: bufTaskEndSel }, { vol: sndBuf.turnCost.vol }) })
    }))
    var teDrop = dshwCustSel(teSel, { scrollNames: true })
    try { teSel.__dshwDrop = teDrop } catch (err) {}
    fillSoundSelect(teSel, bufTaskEndSel)
    // 「静音」项删除后下拉不该留空：把兜底后的真实值回写缓冲（旧数据 sel 为空串时兜底成列表里的默认音）
    if (teSel.value) bufTaskEndSel = teSel.value
    // v778：②区「提示音量」—— 拖动即视为**用户显式设置**（volSet = true），此后该事件用自己的值；
    //   没拖过则保持"跟随①区按压音量"（见 soundVolumeOf），摘要里会写明，避免再出现"滑块看着能调、其实无效"。
    var tcVol = volSlider(function () { return sndBuf.turnCost.vol }, function (v) {
      sndBuf.turnCost.vol = v
      sndBuf.turnCost.volSet = true
      try { refreshSummaries() } catch (err) {}
    })
    var rTcVol = row('提示音量', tcEnt.body)
    rTcVol.appendChild(tcVol.input)
    rTcVol.appendChild(tcVol.pct)
    // v773：② 的"音效没开"= 音效行 [✓] 没勾（bufTaskEndOn）**或**整个事件关着（turnCostOn）⇒
    //       音效下拉 + 提示音量置灰；只有"下拉 + 音量"灰，[✓] 与试听 ▶ 仍可用。
    // v773b：**「任务结束音」这行字不置灰**（用户要求"这三行字不用变灰"）—— 行里还有可用的
    //       [✓] 与 ▶，标签一起灰会让人以为整行都废了。label 传 null = 只灰控件、不动标签。
    sndReg(function () { return !turnCostOn || !bufTaskEndOn }, [
      { el: teSel, drop: teDrop, label: null },
      { el: tcVol.input, label: rTcVol.firstChild },
    ])
    sndEntries.push({ kind: 'turnCost', summary: tcEnt.summary, sel: teSel })
    // —— ③④ 提问提示 / 授权提示（结构完全相同）——
    var waitUI = {}
    function buildWait(kind, labelText, selLabel) {
      var cfg = sndBuf[kind]
      var wEnt = sndEntry(labelText, cfg.on !== false, function (b) { cfg.on = !!b })
      var onChk = wEnt.chk
      // v762 行序（与 ② 统一）：冒泡提示 → 自动关闭 → 音效行 → 提示音量
      // v761（#161 C5）：提问/授权的内容编辑器已接入 —— usageAlertBudgetEditor 新增了
      // 'question' / 'approval' 两个模式，内容落在 events.<kind>.lines。**不要**拿 'question'
      // 去打开预警/预算编辑器（那会改错内容，正是上一轮把它置灰的原因）。
      var bub = bubbleRow(cfg, function () { usageAlertBudgetEditor(kind, null) },
        '编辑' + (kind === 'approval' ? '授权' : '提问') + '提示的泡泡内容（{session} = 当前对话名，超长自动截断）', wEnt.body)
      // v764：**不再挂「自动关闭」那一行** —— 提问/授权的提示气泡是**常驻**的（宿主给的是挂起状态，
      // 前端 whaleSysPush({kind:'wait', ttlMs: 0}) 不布自动关闭计时器），所以那行改了也没有任何消费方，
      // 属于「看起来能设、其实什么都不做」的死控件。行序因此是：冒泡提示 → 音效行 → 提示音量。
      var rSel = row(selLabel, wEnt.body)
      // 音效行的 [✓] = 是否播这个音效，键 = events.<kind>.soundOn（v762 新增，默认 true）。
      // 提问/授权本来就被表头总开关默认关着（on: false），所以默认 true 不带来任何行为变化。
      // 位置：标签之后、下拉之前。
      var soundChk = document.createElement('input')
      soundChk.type = 'checkbox'
      soundChk.className = 'dshwv-check'
      soundChk.checked = cfg.soundOn !== false
      soundChk.title = '该事件是否播这个音效（关掉 = 不响；下拉里选的音效仍保留，下次勾上即用）'
      soundChk.addEventListener('change', function () { cfg.soundOn = !!soundChk.checked })
      rSel.appendChild(soundChk)
      var sel = document.createElement('select')
      sel.className = 'dshwv-sound'
      sel.title = selLabel + '：音效组（点按整组）/ 单个音频；播不播由左边的 [✓] 决定'
      sel.addEventListener('change', function () { cfg.sel = sel.value })
      rSel.appendChild(sel)
      rSel.appendChild(playBtn('试听' + selLabel, function () {
        playPreview(function () { playBindingSound({ on: true, sel: cfg.sel }, { vol: cfg.vol }) })
      }))
      var drop = dshwCustSel(sel, { scrollNames: true })
      try { sel.__dshwDrop = drop } catch (err) {}
      fillSoundSelect(sel, cfg.sel)
      // 同 ②：「静音」项删除后下拉不该留空，把兜底后的真实值写回缓冲
      if (sel.value) cfg.sel = sel.value
      var vs = volSlider(function () { return cfg.vol }, function (v) { cfg.vol = v })
      var rVol = row('提示音量', wEnt.body)
      rVol.appendChild(vs.input)
      rVol.appendChild(vs.pct)
      // v773：③④ 的"音效没开"= 音效行 [✓]（cfg.soundOn）没勾**或**该事件总开关（cfg.on）关着
      // v773b：**「提问提示音效」/「授权提示音」这两行字不置灰**（用户要求），只灰下拉与音量
      sndReg(function () { return cfg.on === false || cfg.soundOn === false }, [
        { el: sel, drop: drop, label: null },
        { el: vs.input, label: rVol.firstChild },
      ])
      sndEntries.push({ kind: kind, summary: wEnt.summary, sel: sel })
      waitUI[kind] = { on: onChk, sel: sel, drop: drop, vol: vs, bub: bub, sound: soundChk }
    }
    buildWait('question', '提问提示', '提问提示音效')
    buildWait('approval', '授权提示', '授权提示音')
    // v765：入口行右侧摘要的刷新 —— ① 打开面板时一次；② 卡片里任何 input/change 都刷新（控件都在卡内）；
    // ③ 点文档任意处也刷新一次（覆盖「音效组」这种从 body 级子面板里改、不发卡内事件的路径），
    //    该监听在 cleanup 里摘掉。摘要只写 textContent，成本很低。
    function sndDocRefresh() { refreshSummaries() }
    try { document.addEventListener('click', sndDocRefresh, true) } catch (err) {}
    try { card.addEventListener('input', refreshSummaries) } catch (err) {}
    try { card.addEventListener('change', refreshSummaries) } catch (err) {}
    refreshSummaries()
    // —— 底部按钮 ——
    var closed = false
    function cleanup(restore) {
      if (closed) return
      closed = true
      try { dshwCustSelClose() } catch (err) {} // 收回可能还开着的自绘下拉（它是 body 上的固定层）
      try { closeAudioGroupPanel() } catch (err) {}
      try { stopPanelPreview() } catch (err) {} // v766：关窗时也停掉试听（别让声音跟着窗口一起"留在后台"）
      if (restore) {
        // 复用自菜单的控件在输入时就已提交 → 取消时按快照回滚
        try { if (Math.abs(Number(soundVol) - snapVol) > 1e-9) setVol(snapVol) } catch (err) {}
        try { if ((soundOn !== false) !== snapSoundOn) setSoundOn(snapSoundOn) } catch (err) {}
        try { if (!!turnCostOn !== snapTurnCostOn) setTurnCostOn(snapTurnCostOn) } catch (err) {}
        // v763：④「自动关闭」（turnCostCloseMs，存在 .dsh-size.json）同款处理 —— 回滚内存并写回磁盘。
        // setTurnCostClose 收秒，且没有「值未变就跳过」的短路，所以这里自己按快照比较，没变就一个字节都不写。
        try {
          if (Math.abs(Number(turnCostCloseMs) - snapTurnCostCloseMs) > 1e-9) {
            setTurnCostClose(Math.round(snapTurnCostCloseMs / 1000))
          }
        } catch (err) {}
        // v777：底部「点按角色关闭提示气泡」——回滚缓冲与控件（它只活在缓冲里，没保存过就没落盘）
        try { bufWaitCharClose = snapWaitCharClose } catch (err) {}
        try { if (waitCharChk) waitCharChk.checked = snapWaitCharClose } catch (err) {}
      }
      try { audioGroupPanel.style.zIndex = '' } catch (err) {}
      try { audioEditMask.style.zIndex = '' } catch (err) {}
      try { document.removeEventListener('click', sndDocRefresh, true) } catch (err) {}
      try { if (unregMask) unregMask() } catch (err) {}
      dshwBodyDetach(mask) // 走 detach 注销登记，否则 DOM 守护会把这个刚关掉的面板补挂回来
    }
    // 本面板会写的那四个事件的载荷（只带 schema 里的键）
    function eventsPayload() {
      return {
        // 按压音量仍在 .dsh-size.json（滑块已即时落盘），这里按宿主 schema 再记一份
        press: { vol: Number(soundVol) || 0 },
        turnCost: soundEventOut(sndBuf.turnCost, false),
        question: soundEventOut(sndBuf.question, true),
        approval: soundEventOut(sndBuf.approval, true),
      }
    }
    // 本地合并一份（与既有编辑器一致：先乐观写内存，宿主返回后以宿主为准）——
    // 逐字段合并不整块替换，免得把宿主那侧的 lines 等内容从内存里抹掉。
    function mergeEventsLocal(ev) {
      try {
        usageSet = usageSet || {}
        usageSet.events = usageSet.events || {}
        for (var ek in ev) {
          var m = {}
          for (var mk in (usageSet.events[ek] || {})) m[mk] = usageSet.events[ek][mk]
          for (var nk in ev[ek]) m[nk] = ev[ek][nk]
          usageSet.events[ek] = m
        }
      } catch (err) {}
    }
    // v763：面板 ②区「自动关闭」→ 消耗泡泡停留时长（毫秒）。为什么要这根线：
    //   · v764：events.turnCost 的 autoClose / ttlSec 已**摘除**（它们从来没有消费方，只有面板自己在
    //     读写，等于同一件事有两个键）；现在 ② 区那行只改面板自己的 tcCloseBuf；
    //   · 真正决定消耗泡泡停多久的是 .dsh-size.json 的 turnCostCloseMs（消费点见上面 sceneOpen）；
    //   · 而唯一能改 turnCostCloseMs 的 turnCostCloseInput 已经不再挂到任何窗口（主菜单那行 v762 删了、
    //     本面板这轮也没搬它）⇒ 不接这根线，面板上那行「自动关闭」就是个死控件。
    // ⚠️ setTurnCostClose(v) 收的是**秒**（内部 n = round(v); turnCostCloseMs = n * 1000），
    //    并且**没有**「值未变就跳过」的短路 —— 值没变也会 saveConfig() 写一次 .dsh-size.json（幂等）。
    function closeMsFromBuf() {
      // v764：读 ② 区自己的缓冲（tcCloseBuf），不再读 events.turnCost
      var c = tcCloseBuf || {}
      var s = Number(c.ttlSec)
      if (!isFinite(s) || s < 0) s = 0
      // 不勾「自动关闭」⇒ 0（= 不自动关闭），与既有 turnCostCloseMs > 0 的判定一致
      return c.autoClose !== false ? Math.round(s) * 1000 : 0
    }
    function saveAll() {
      try {
        var ev = eventsPayload()
        // 任务结束音：沿用既有绑定键 {on, sel}。v762 起「播不播」由 ② 音效行的 [✓]
        // （= usageSet.taskEnd.on）表达，下拉只负责选**哪一个**音效（已无「静音」项）。
        usageSet = usageSet || {}
        var te = {}
        try { for (var k in (usageSet.taskEnd || {})) te[k] = usageSet.taskEnd[k] } catch (err) {}
        te.on = !!bufTaskEndOn
        te.sel = bufTaskEndSel || bufTaskEndKeep
        usageSet.taskEnd = te
        try { applyTaskEndLocal(te.on, te.sel) } catch (err) {}
        mergeEventsLocal(ev)
        // v777：底部「点按角色关闭提示气泡」跟着同一次 PUT 落盘（键 = settings.wait.charClose）
        try {
          usageSet = usageSet || {}
          usageSet.wait = Object.assign({}, usageSet.wait || {}, { charClose: !!bufWaitCharClose })
        } catch (err) {}
        saveUsageSettings({ events: ev, taskEnd: te, wait: { charClose: !!bufWaitCharClose } })
        // v763：按压音量 / 音效总开关 / 每轮消耗提示开关这三项**不在**上面这次 PUT 的覆盖范围里
        //（它们存在 .dsh-size.json，走 saveConfig()）⇒ 保存时必须各自按**当前内存值**提交一次，
        // 否则「恢复默认」对它们不生效（用户会觉得恢复默认对音量/开关没作用）。
        // 三个 setter 各自都有「值没变就写一遍同样的值」的幂等行为，无副作用。
        try { setVol(soundVol) } catch (err) {}
        try { setSoundOn(soundOn) } catch (err) {}
        try { setTurnCostOn(turnCostOn) } catch (err) {}
        // v763：② 的「自动关闭」同样按**当前内存值**提交 —— setTurnCostClose() 收秒、写的是
        // .dsh-size.json 的 turnCostCloseMs（消耗泡泡停留时长）；0 = 不自动关闭。
        try { setTurnCostClose(Math.round(closeMsFromBuf() / 1000)) } catch (err) {}
      } catch (err) {}
      cleanup(false)
    }
    function applyDefaults() {
      try {
        // v763（真机反馈修）：**本函数只改内存，绝不落盘** —— 与窗口里其它改动完全同语义。
        // 旧实现调了 setVol/setSoundOn/setTurnCostOn（这三者都立即写 .dsh-size.json），又把
        // 「取消快照」整体换成默认值、最后还 saveUsageSettings({resetEvents:true}) 立刻 PUT
        // ⇒ 点「恢复默认」再点「取消」时磁盘上早已是默认值，取消等于没撤销（用户实测的 bug）。
        // 现在：这里只改内存缓冲 + 控件显示（直接改 .value/.checked 不派发 input/change，
        // 所以碰不到那三个落盘 setter）；点「保存」由 saveAll() 提交，「取消」由 cleanup(true)
        // 按**打开面板时**的快照回滚（因此下面不再重写 snapVol/snapSoundOn/snapTurnCostOn）。
        // ① 按压音量 + 按压音效总开关（都是内存标量）
        var dPressVol = Number(DEF.press.vol)
        dPressVol = isFinite(dPressVol) ? Math.round(Math.min(1, Math.max(0, dPressVol)) * 100) / 100 : 0
        soundVol = dPressVol // 与 setVol() 同一套规范化（取消时的快照比较是 1e-9，精度必须一致）
        var sndWasOff = (soundOn === false)
        soundOn = true
        try { volInput.value = String(soundVol) } catch (err) {}
        try { volPct.textContent = Math.round(soundVol * 100) + '%' } catch (err) {}
        try {
          if (pressAudio) pressAudio.volume = soundVol
          if (releaseAudio) releaseAudio.volume = soundVol
        } catch (err) {}
        // 由「关」到「开」时补上 setSoundOn(true) 的**非落盘**副作用（复位本轮播放状态 + 重新预热）：
        // 否则音频上下文还停在 suspend，面板里的试听按钮会没声音（取消时 setSoundOn(快照) 会回去）。
        if (sndWasOff) { try { applySoundSet() } catch (err) {} }
        // ② 「每轮消耗提示」总开关（同样只改内存）
        turnCostOn = true
        try { tcOnChk.checked = true } catch (err) {}
        try { turnCostToggle.checked = true } catch (err) {}
        try { turnCostCloseInput.disabled = false } catch (err) {}
        sndBuf.turnCost = JSON.parse(JSON.stringify(DEF.turnCost))
        sndBuf.question = JSON.parse(JSON.stringify(DEF.question))
        sndBuf.approval = JSON.parse(JSON.stringify(DEF.approval))
        // v764：② 「自动关闭」的默认值 = 出厂值（5 秒 / 开，与 .dshw-size.json 的 turnCostCloseMs 出厂
        // 默认一致）—— 它现在住在 tcCloseBuf 里，**不再**跟着 DEF.turnCost 走。同样只改内存
        // （**不调 setTurnCostClose** ⇒ 不落盘），并把（已创建、未挂载的）秒数输入框显示同步好 ——
        // 否则之后从本面板打开「自定义提示（每轮消耗）」窗口时，那个窗口的「保存」会按陈旧的输入框值
        // 把 turnCostCloseMs 又写回去。
        tcCloseBuf.autoClose = true
        tcCloseBuf.ttlSec = TC_CLOSE_DEFAULT_SEC
        turnCostCloseMs = closeMsFromBuf()
        try { turnCostCloseInput.value = String(Math.round(turnCostCloseMs / 1000)) } catch (err) {}
        // v762：「静音」项已删除 ⇒ 默认仍是「默认已选中内置音」。
        // v774：「恢复默认」的目标 = 新的出厂默认（宿主 usageSettingsDefaults().taskEnd）：
        //       on = **true**、sel = 内置 A（frag:end_a）。
        bufTaskEndSel = 'frag:end_a' // = 宿主 usageSettingsDefaults().taskEnd.sel
        bufTaskEndKeep = 'frag:end_a' // = 宿主 usageSettingsDefaults().taskEnd.sel
        bufTaskEndOn = true // v774：② 音效行的 [✓] 复位（宿主 taskEnd.on 默认 true）
        // v778：②区「提示音量」也复位成"跟随①区"（volSet=false；滑块显示由 refreshSummaries 同步成①区值）
        try { sndBuf.turnCost.volSet = false } catch (err) {}
        // v777：底部「点按角色关闭提示气泡」恢复出厂默认（false = 只有点泡泡能收）
        bufWaitCharClose = false
        try { if (waitCharChk) waitCharChk.checked = false } catch (err) {}
        // 控件回填
        try { pressOnChk.checked = true } catch (err) {}
        try { soundToggle.checked = true } catch (err) {} // v763：与 setSoundOn() 同步的那只开关（只改内存显示）
        try { teSoundChk.checked = true } catch (err) {} // v774：② 音效行 [✓] 复位（出厂默认已改成"开"）
        try { tcAc.chk.checked = tcCloseBuf.autoClose !== false } catch (err) {}
        try { tcAc.num.value = String(tcCloseBuf.ttlSec) } catch (err) {}
        try { tcBub.checked = sndBuf.turnCost.bubbleOn !== false } catch (err) {}
        try {
          tcVol.input.value = String(sndBuf.turnCost.vol)
          tcVol.pct.textContent = Math.round(sndBuf.turnCost.vol * 100) + '%'
        } catch (err) {}
        fillSoundSelect(teSel, bufTaskEndSel)
        ;['question', 'approval'].forEach(function (kind) {
          var u = waitUI[kind]
          var cfg = sndBuf[kind]
          if (!u) return
          try { u.on.checked = cfg.on !== false } catch (err) {}
          try {
            u.vol.input.value = String(cfg.vol)
            u.vol.pct.textContent = Math.round(cfg.vol * 100) + '%'
          } catch (err) {}
          try { u.bub.checked = cfg.bubbleOn !== false } catch (err) {}
          // v762：③④ 音效行的 [✓] 复位（DEF 里 soundOn: true）
          try { if (u.sound) u.sound.checked = cfg.soundOn !== false } catch (err) {}
          fillSoundSelect(u.sel, cfg.sel)
        })
        // v765：入口行摘要也要跟着「恢复默认」刷新
        refreshSummaries()
        // v763：到这里结束 —— **不再写 usageSet、不调 applyTaskEndLocal、也绝不 PUT**。
        // 旧实现在这里乐观写了内存镜像（events + taskEnd）并立刻发 resetEvents；而「取消」只能
        // 回滚那三个标量 ⇒ 内存镜像停在默认值、磁盘却是旧值，表现为「取消后重新打开面板看到
        // 的还是默认值」。现在整套动作都只是缓冲：
        //   · 事件类键        = sndBuf（saveAll 里由 eventsPayload() 取，保存时 mergeEventsLocal + PUT）
        //   · 任务结束音      = bufTaskEndOn / bufTaskEndSel（saveAll 里写回 usageSet.taskEnd + applyTaskEndLocal）
        //   · 按压音量        = soundVol（saveAll 里 setVol() 落 .dsh-size.json，并按宿主 schema 记一份 press.vol）
        //   · usageSet.events 的 lines（提问/授权提示内容）不再被「恢复默认」清空（与按钮 title 一致）
      } catch (err) {}
    }
    // v777（用户要求）：「取消/恢复默认/保存」上面一行 —— 与「自定义泡泡」窗口的
    // 「点按角色推进泡泡队列」同款排版（[✓] + 标签 + ? 圈圈），说明收进 ? 圈圈里（复用 v647 的 dshwvAskDot）。
    var waitCharRow = document.createElement('div')
    waitCharRow.className = 'dshwv-bubsec'
    waitCharRow.style.display = 'flex'
    waitCharRow.style.alignItems = 'center'
    waitCharRow.style.flexWrap = 'wrap'
    waitCharRow.style.gap = '4px 6px'
    waitCharRow.style.color = '#203170'
    waitCharRow.style.marginTop = '8px'
    // v777c（2026-09-27 追加要求）：
    //   ① 与上面那几行（入口行 `dshwvFoldEntry`，如「授权提示」）的复选框**左对齐** —— 入口行是
    //      `rowEl.style.padding = '6px 6px'` 直接挂在 card 上 ⇒ 复选框在 6px 处；本行 `dshwv-bubsec`
    //      没有左内边距（默认 0）⇒ 补 6px 即对齐。（⚠️ 注意区分：展开后**折叠体里**的行额外有
    //      `body.style.paddingLeft = 10px`，在 10px 处；本行是全局开关、不属于任何折叠体，故按入口行对齐。）
    //   ② 本行**不要鼠标悬浮提示**（原生 title）：问号圈圈自带的「查看说明」与复选框那句说明都清掉 ——
    //      说明统一收进问号圈圈的弹层里（悬停/点击问号时才显示），避免两个入口说两遍。
    waitCharRow.style.paddingLeft = '6px'
    var waitCharAsk = dshwvAskDot(
      '等待提问 / 授权的提示气泡：<b>默认点气泡就能收起来</b>（点一下收起，同一条挂起不会再自动弹回；' +
      '你回答或批准之后、或下一次新的提问 / 授权到来时，照常冒泡并响铃）。<br><br>' +
      '打开下面这个开关后，<b>点角色（鲸鱼）也能收掉提示气泡</b> —— 和「自定义泡泡」里的' +
      '「点按角色推进泡泡队列」是同一类手感。<br><br>默认关闭。'
    )
    // v777b（2026-09-27 追加要求）：? 圈圈放到**文字后面**，并把与文字的间距收紧 ——
    //   .dshwv-askq 自带 margin-right:5px，加上行的 column-gap 6px，放前面时离文字有 11px；
    //   v777b 先用行内 margin-left:-3px 压到约 3px；
    //   v777d（2026-09-27 追加要求「再稍稍右移」）：改成 0 ⇒ 间距回到行的自然列间距 6px（右移 3px）。
    waitCharAsk.style.marginLeft = '0'
    waitCharAsk.style.marginRight = '0'
    waitCharAsk.title = '' // v777c：清掉 dshwvAskDot 自带的「查看说明」原生 tooltip（用户要求本行不要悬浮提示）
    var waitCharChk = document.createElement('input')
    waitCharChk.type = 'checkbox'
    waitCharChk.className = 'dshwv-check'
    waitCharChk.id = 'dshwv-waitcharclose'
    waitCharChk.checked = bufWaitCharClose
    // v777c：复选框也不挂原生 tooltip（原来那句"打开后：点角色也能收起…"已写进问号圈圈的说明里）
    waitCharChk.title = ''
    waitCharChk.addEventListener('change', function () { bufWaitCharClose = !!waitCharChk.checked })
    waitCharRow.appendChild(waitCharChk)
    var waitCharLab = document.createElement('label')
    waitCharLab.setAttribute('for', 'dshwv-waitcharclose')
    waitCharLab.style.cursor = 'pointer'
    waitCharLab.style.fontSize = '12px'
    waitCharLab.textContent = '点按角色关闭提示气泡'
    waitCharRow.appendChild(waitCharLab)
    waitCharRow.appendChild(waitCharAsk)
    card.appendChild(waitCharRow)
    var btns = document.createElement('div')
    btns.className = 'dshwv-bubbtns'
    var cancelBtn = document.createElement('button')
    cancelBtn.type = 'button'
    cancelBtn.className = 'dshwv-bubbtn dshwv-bubbtn-no'
    cancelBtn.textContent = '取消'
    cancelBtn.addEventListener('click', function () { cleanup(true) })
    btns.appendChild(cancelBtn)
    var resetBtn = document.createElement('button')
    resetBtn.type = 'button'
    resetBtn.className = 'dshwv-bubbtn dshwv-bubbtn-no'
    resetBtn.textContent = '恢复默认'
    resetBtn.title = '把四个事件的音量/自动关闭/冒泡/音效选择恢复为默认值（不动外观、位置、账本、角色与自定义泡泡）'
    resetBtn.addEventListener('click', function () { applyDefaults() })
    btns.appendChild(resetBtn)
    var okBtn = document.createElement('button')
    okBtn.type = 'button'
    okBtn.className = 'dshwv-bubbtn dshwv-bubbtn-ok'
    okBtn.textContent = '保存'
    okBtn.addEventListener('click', function () { saveAll() })
    btns.appendChild(okBtn)
    card.appendChild(btns)
    mask.appendChild(card)
    mask.addEventListener('click', function (e) { if (e.target === mask) cleanup(true) })
    dshwBodyAppend(mask)
  } catch (err) {}
}
function usageAlertBudgetEditor(key, onSave) {
  try {
    var isAlert = key === 'alert'
    // v720:第三个模式 cost —— 主菜单「每轮消耗提示」后的「自定义提示」窗口。
    // 内容与预警/预算共用同一套模块机制;头部不是"触发条件"而是自动关闭秒数 + 任务结束音效(从主菜单搬来)。
    var isCost = key === 'cost'
    // v761（#161 C5）：第四/第五个模式 question / approval —— 「全局音效设置」面板里
    // 「提问音效 / 授权音效」两区的「编辑提示内容」入口。内容落在 usageSet.events.<kind>.lines
    // （宿主 waitQuestion.content / waitApproval.content 那套键），与 alert/budget 的 lines 完全分开，
    // 所以**绝不能**落到 isAlert/isBudget 分支去（那会改错内容 —— 上一轮就是因此把它置灰的）。
    var isWait = (key === 'question' || key === 'approval')
    var waitKind = isWait ? key : ''
    var costCommitted = false // 保存过就不再走"取消还原"
    var waitCommitted = false // v761：等待提示内容"保存过就不再走取消还原"的旗标
    var waitSnapLines = null // v761：等待提示内容打开窗口时的快照（取消/点遮罩关闭时还原缓冲）
    var cfg = isCost
      ? (((usageSet || {}).turnCost) || {})
      : (isWait ? ((((usageSet || {}).events || {})[waitKind]) || {}) : (((usageSet || {})[isAlert ? 'alert' : 'budget']) || {}))
    var numDef = isAlert ? 50 : 20
    var numInit = isAlert ? (cfg.below != null ? cfg.below : numDef) : (cfg.amount != null ? cfg.amount : numDef)
    // 三种自带内容各自取自己的默认模板；等待两模式取宿主 waitDefaultLines() 的逐字段一致版
    var stepLines0 = isCost ? usageTurnCostLines() : (isWait ? usageWaitLinesOf(cfg, waitKind) : usageRemindLinesOf(cfg, isAlert))
    if (isWait) {
      try { waitSnapLines = stepLines0 ? JSON.parse(JSON.stringify(stepLines0)) : [] } catch (err) { waitSnapLines = [] }
    }
    var step = { kind: 'custom', modules: JSON.parse(JSON.stringify(stepLines0)) }
    // 备份被临时替换的全局状态/元素/函数,关闭后还原
    var bkEditItems = bubbleEditItems
    var bkEditorSnap = bubbleEditorSnap
    var bkItemSnap = bubbleItemSnap
    var bkEditIdx = bubbleEditItemIdx
    var bkSide = bubbleEditSide
    var bkPal = bubblePalEl
    var bkPv = bubblePvEl
    var bkPrev = bubblePvPrevEl
    var bkRenderPv = renderBubblePv
    var bkQeditEnsure = qeditEnsure
    var bkModMaskZ = moduleMask ? moduleMask.style.zIndex : ''
    bubbleEditItems = [step]
    bubbleEditItemIdx = 0
    bubbleEditSide = -1
    bubbleItemSnap = JSON.parse(JSON.stringify(step))
    bubbleEditorSnap = null
    // —— 窗口壳(与 W2 同款卡片) ——
    var mask = document.createElement('div')
    mask.className = 'dshwv-bubmask'
    // 提醒内容编辑器：必须高于「模型子菜单/模型设置」(29000) 才能正常操作
    mask.style.zIndex = '30000'
    var card = document.createElement('div')
    card.className = 'dshwv-bubcard'
    card.style.maxHeight = '88vh'
    card.style.overflow = 'hidden auto'
    var title = document.createElement('div')
    title.className = 'dshwv-bubtitle'
    title.textContent = isCost
      ? '自定义提示（每轮消耗 · 内容）'
      : (isWait
        ? ('编辑 ' + (waitKind === 'approval' ? '授权' : '提问') + '提示内容(可拖动下方模块入框)')
        : ('编辑 ' + (isAlert ? '余额预警' : '今日预算') + '提醒内容(可拖动下方模块入框)'))
    card.appendChild(title)
    // —— 头部:预警/预算=触发条件;cost=自动关闭 + 任务结束音效;wait=说明 + 占位符提示 ——
    var secCond = document.createElement('div')
    secCond.className = 'dshwv-bubsec dshwv-bubsec-first'
    secCond.textContent = isCost
      ? '每轮消耗提示(内容里用 {cost} 引用本轮消耗金额)'
      : (isWait
        ? ((waitKind === 'approval' ? '授权' : '提问') + '提示内容({session} = 对话名)')
        : (isAlert ? '触发条件(余额低于该值时提醒)' : '触发条件(今日已用达到该值时提醒)'))
    card.appendChild(secCond)
    // 触发条件控件(启用 + 阈值):cost 模式没有触发条件,这两个控件不创建
    // v761：wait 模式同样没有"触发条件"（挂起音效/开关/自动关闭/冒泡都在「全局音效设置」面板那一区里）
    var chk = null
    var numInp = null
    if (!isCost && !isWait) {
      chk = document.createElement('input')
      chk.type = 'checkbox'
      chk.className = 'dshwv-check'
      chk.checked = !!cfg.on
      numInp = document.createElement('input')
      numInp.type = 'number'
      numInp.min = '0'
      numInp.step = '0.01'
      numInp.className = 'dshwv-number'
      numInp.style.width = '80px'
      numInp.value = String(numInit)
    }
    var condBox = document.createElement('div')
    condBox.style.padding = '2px 0'
    condBox.style.display = 'flex'
    condBox.style.flexWrap = 'wrap'
    condBox.style.alignItems = 'center'
    condBox.style.gap = '6px 14px'
    condBox.style.textAlign = 'left'
    condBox.style.fontSize = '12px'
    condBox.style.color = '#203170'
    function segCond() {
      var s = document.createElement('span')
      s.style.display = 'inline-flex'
      s.style.alignItems = 'center'
      s.style.gap = '5px'
      s.style.whiteSpace = 'nowrap'
      return s
    }
    if (isCost) {
      // v763（用户要求）：本窗口**不再挂这三行** —— 「自动关闭 [秒数] 秒(0=不自动关闭)」、
      // 「任务结束音效 [音效名 ▾]」（含左边的 [✓]）以及下面的小字提示
      // 「每轮回复完成时播放;这两项点「保存」后才生效」。它们现在都在「提示与音效设置」面板的
      // 「每轮消耗提示」区里（那一区才是这两项的正式入口），留在本窗口属于重复。
      // ⚠️ 这里**只是不 append**，元素的创建与相关缓冲/快照/取消还原一律原样保留：
      //   · turnCostCloseInput：仍创建（见文件上方），仍走 turnCostCloseDefer / -Snap 那套
      //     「窗口内只改内存 → 点「保存」才 setTurnCostClose 落 .dsh-size.json → 取消按快照还原」；
      //   · taskEndSel：仍创建，并**继续留在他原本的宿主 taskEndRowHost 里**（那个宿主本来就不插入
      //     文档；dshwCustSel 只要求 select 有父节点，所以自绘下拉的 refresh/fill/sync 照常工作，
      //     不依赖是否挂进文档）；
      //   · taskEndToggle（「每轮回复完成时播放」的 [✓]）：仍创建、仍被 applyTaskEndLocal 同步，
      //     只是本窗口不再展示（面板 ② 区的 teSoundChk 是它的替代入口）。
      // 保存/取消语义不变：本窗口的「保存」照样提交缓冲里的 taskEnd 与秒数，「取消」照样按快照还原。
      turnCostCloseInput.disabled = false // 保留原有的「窗口内不受菜单禁用态影响」语义（取消路径也看它）
    } else if (isWait) {
      // v761（#161 C5）：等待提示没有触发条件。这里只放说明 —— 音效 / 音量 / 自动关闭 / 冒泡开关
      // 都在「全局音效设置」面板里那一区，本窗口只管内容（改动仍遵循"保存才提交"）。
      var wHint = qLabel(waitKind === 'approval'
        ? '授权挂起时显示这条提示（开关与时长在「授权音效」那一区设置）'
        : '提问挂起时显示这条提示（开关与时长在「提问音效」那一区设置）')
      wHint.style.fontSize = '11px'
      wHint.style.color = '#203170'
      wHint.style.opacity = '.85'
      condBox.appendChild(wHint)
      condBox.appendChild(document.createElement('br'))
      var wHint2 = qLabel('{session} 会替换为当前对话名，超长自动截断（读不到时显示「当前对话」）')
      wHint2.style.fontSize = '11px'
      wHint2.style.color = '#203170'
      wHint2.style.opacity = '.75'
      condBox.appendChild(wHint2)
    } else {
    // 启用提醒: 口 启用提醒 · 阈值: 余额 ≤ [数值] 元时提醒
    var gOn = segCond()
    gOn.appendChild(chk)
    gOn.appendChild(qLabel('启用提醒'))
    condBox.appendChild(gOn)
    var gNum = segCond()
    gNum.appendChild(qLabel(isAlert ? '余额 ≤ ' : '今日已用 ≥ '))
    gNum.appendChild(numInp)
    gNum.appendChild(qLabel(' 元时提醒'))
    condBox.appendChild(gNum)
    // 「元时提醒」后换行,自动关闭及其秒数独占一行
    var condBrk = document.createElement('span')
    condBrk.style.flex = '1 0 100%'
    condBrk.style.height = '0'
    condBrk.style.margin = '0'
    condBox.appendChild(condBrk)
    // 自动关闭(并到同一行): 口 自动关闭 [秒数] 秒(0=不自动关闭)
    var acChk = document.createElement('input')
    acChk.type = 'checkbox'
    acChk.className = 'dshwv-check'
    acChk.checked = cfg.autoClose !== false
    var defSec = Number(cfg.ttlSec)
    if (!isFinite(defSec) || defSec <= 0) defSec = 6
    var secInp = document.createElement('input')
    secInp.type = 'number'
    secInp.min = '0'
    secInp.step = '1'
    secInp.className = 'dshwv-number'
    secInp.style.width = '56px'
    secInp.value = String(defSec)
    var gAc = segCond()
    gAc.appendChild(acChk)
    gAc.appendChild(qLabel('自动关闭'))
    condBox.appendChild(gAc)
    var gSec = segCond()
    gSec.appendChild(secInp)
    var lSec = qLabel('秒(0=不自动关闭)')
    lSec.style.opacity = '.75'
    lSec.style.fontSize = '11px'
    gSec.appendChild(lSec)
    condBox.appendChild(gSec)
    }
    // v761：wait 模式 condBox 里只有说明（上面那一支已直接挂进 card），不重复挂一个空的盒子
    if (!isWait) card.appendChild(condBox)
    // —— 以下完全复用 W2 组件(仅换宿主元素与目标步骤) ——
    var secPal = document.createElement('div')
    secPal.className = 'dshwv-bubsec dshwv-bubsec-first dshwv-bubsec-withq'
    secPal.textContent = '可选模块'
    // 原「可选模块(点击或拖入…)」与「提醒内容(同一行并排…{below}/{amount} 替换)」的说明收进「?」圈
    secPal.insertBefore(dshwvAskDot(
      '<div style="font-weight:600;margin-bottom:4px">可选模块 &amp; 提醒内容</div>' +
      '<div>点击「可选模块」即可加入提醒内容;桌面端也可直接拖入下方内容框。</div>' +
      '<div style="margin-top:4px">同一行模块并排显示(≤6 个):</div>' +
      '<div>· 拖模块到某行左/右边缘 = 并排</div>' +
      '<div>· 拖到某行上/下 = 拆行另起一行</div>' +
      '<div>· 拖 ⠿ 手柄 = 整行排序</div>' +
      (isCost
        ? '<div style="margin-top:4px">{cost} 在触发时替换为本轮消耗金额(默认内容请保持 {cost})</div>'
        : (isWait
          ? '<div style="margin-top:4px">{session} 在挂起时替换为当前对话名（读不到显示「当前对话」，超长按 12 字符截断并追加 ...）</div>'
          : '<div style="margin-top:4px">{below} / {amount} 在触发时替换为实际数值</div>')) +
      '<div style="margin-top:4px;opacity:.75">手机端:长按约 0.4 秒进入拖动</div>'
    ), secPal.firstChild)
    card.appendChild(secPal)
    bubblePalEl = document.createElement('div')
    bubblePalEl.className = 'dshwv-bubpal'
    card.appendChild(bubblePalEl)
    var secPv = document.createElement('div')
    secPv.className = 'dshwv-bubsec'
    secPv.textContent = isCost
      ? '提示内容(每轮消耗)'
      : (isWait ? ('提示内容(' + (waitKind === 'approval' ? '授权' : '提问') + ')') : '提醒内容')
    card.appendChild(secPv)
    bubblePvEl = document.createElement('div')
    bubblePvEl.className = 'dshwv-bubpvbox'
    card.appendChild(bubblePvEl)
    bubblePvPrevEl = document.createElement('div')
    bubblePvPrevEl.className = 'dshwv-bubprev'
    card.appendChild(bubblePvPrevEl)
    // 与 W2 相同:palette/模块块拖入内容区(行内自有拖放由 renderBubblePv 处理)
    bubblePvEl.addEventListener('dragover', function (e) {
      try { if (e.target && e.target.closest && e.target.closest('.dshwv-pvrow')) return; e.preventDefault() } catch (err) {}
    })
    bubblePvEl.addEventListener('drop', function (e) {
      try {
        if (e.target && e.target.closest && e.target.closest('.dshwv-pvrow')) return
        e.preventDefault()
        if (bubbleModDrag) {
          var mdd = bubbleModDrag
          bubbleModDrag = null
          bubblePvDropBlockEnd(mdd.ri, mdd.mi)
          return
        }
        var key = bubbleDragKey
        if (!key) return
        bubbleDragKey = null
        if (key === 'image') { bubblePickImageToAdd(); return }
        if (key === 'wizard') { bubbleModuleWizard(); return }
        var m = bubblePaletteModule(key)
        if (m) bubbleModuleAdd(m)
      } catch (err) {}
    })
    // —— 底部按钮 ——
    var btns = document.createElement('div')
    btns.className = 'dshwv-bubbtns'
    function cleanup() {
      try {
        // cost 模式收尾:没点「保存」就把窗口期间缓冲的改动还原(取消语义)
        if (isCost && !costCommitted) {
          var snap = taskEndDeferSnap || { on: false, sel: '' }
          usageSet = usageSet || {}
          usageSet.taskEnd = { on: !!snap.on, sel: String(snap.sel || '') }
          // v763：v762 起 taskEndSel 不再挂进任何窗口（它留在不插入文档的宿主 taskEndRowHost 里），
          // 所以这里对它的同步改成与下一行同款的 try 包裹 —— 万一自绘下拉在未挂载状态下出问题，
          // 也不能把后面「秒数还原」和 dshwBodyDetach(mask)（关窗）一起带崩（本文件踩过静默 TypeError）。
          try { applyTaskEndLocal(usageSet.taskEnd.on, usageSet.taskEnd.sel) } catch (err) {}
          try { fillTaskEndOptions(usageSet.taskEnd) } catch (err) {}
          // 秒数:窗口内没落盘,还原内存与输入框即可
          turnCostCloseMs = turnCostCloseDeferSnap
          turnCostCloseInput.value = String(Math.max(0, Math.round(turnCostCloseDeferSnap / 1000)))
        }
        if (isCost) {
          taskEndDefer = false
          taskEndDeferSnap = null
          turnCostCloseDefer = false
          turnCostCloseInput.disabled = !turnCostOn // 回到菜单态的禁用逻辑
        }
        // v761（#161 C5）：wait 模式收尾 —— 没点「保存」就把缓冲还原（取消语义）。
        // 窗口期间用户拖入/删除的模块只改 step.modules（缓冲），内存里的 events.<kind>.lines 一直没动，
        // 所以这里只需把缓冲还原回打开时的快照，再把预览重画一次。
        if (isWait && !waitCommitted) {
          var wsnap = Array.isArray(waitSnapLines) ? waitSnapLines : []
          step.modules = JSON.parse(JSON.stringify(wsnap))
          try { renderBubblePv() } catch (err) {}
        }
        dshwBodyDetach(mask) // v744：走 detach 注销登记，否则 DOM 守护会把这个刚关掉的编辑器补挂回来
        bubbleEditItems = bkEditItems
        bubbleEditorSnap = bkEditorSnap
        bubbleItemSnap = bkItemSnap
        bubbleEditItemIdx = bkEditIdx
        bubbleEditSide = bkSide
        bubblePalEl = bkPal
        bubblePvEl = bkPv
        bubblePvPrevEl = bkPrev
        renderBubblePv = bkRenderPv
        qeditEnsure = bkQeditEnsure
        if (moduleMask) moduleMask.style.zIndex = bkModMaskZ
        var zsEl = document.getElementById('dshw-remind-overlay-z')
        if (zsEl) { try { document.head.removeChild(zsEl) } catch (err) {} }
        window.__dshwRemindMask = null
      } catch (err) {}
    }
    var noBtn = document.createElement('button')
    noBtn.type = 'button'
    noBtn.className = 'dshwv-bubbtn dshwv-bubbtn-no'
    noBtn.textContent = '取消'
    noBtn.addEventListener('click', cleanup)
    btns.appendChild(noBtn)
    var resBtn = document.createElement('button')
    resBtn.type = 'button'
    resBtn.className = 'dshwv-bubbtn dshwv-bubbtn-no'
    resBtn.textContent = '恢复默认'
    resBtn.title = isCost
      ? '恢复为默认提示内容(自动关闭与任务结束音效保持不变)'
      : (isWait
        ? ('恢复为默认' + (waitKind === 'approval' ? '授权' : '提问') + '提示内容(音效/音量/自动关闭/冒泡开关保持不变)')
        : '恢复为默认提醒内容(触发条件保持不变)')
    resBtn.addEventListener('click', function () {
      // v761：wait 模式的默认模板 = 宿主 waitDefaultLines() 的逐字段一致版
      step.modules = JSON.parse(JSON.stringify(isCost
        ? usageTurnCostDefaultLines()
        : (isWait ? usageWaitDefaultLines(waitKind) : usageRemindDefaultLines(isAlert))))
      renderBubblePv()
    })
    btns.appendChild(resBtn)
    var okBtn = document.createElement('button')
    okBtn.type = 'button'
    okBtn.className = 'dshwv-bubbtn dshwv-bubbtn-ok'
    okBtn.textContent = '保存'
    okBtn.addEventListener('click', function () {
      try { bubbleRowsCanon(step.modules) } catch (err) {}
      var lines = JSON.parse(JSON.stringify(step.modules))
      if (isCost) {
        costCommitted = true
        // 自动关闭秒数:结束缓冲并落 .dshw-size.json(沿用原有字段与保存链路)
        turnCostCloseDefer = false
        setTurnCostClose(turnCostCloseInput.value)
        // 内容与任务结束音效:一次 PUT 落账本设置(与预警/预算同一机制)
        usageSet = usageSet || {}
        usageSet.turnCost = { lines: lines }
        taskEndDefer = false
        saveUsageSettings({
          taskEnd: usageSet.taskEnd || { on: false, sel: '' },
          turnCost: { lines: lines },
        })
        var oCost = { lines: lines, autoClose: turnCostCloseMs > 0, ttlSec: Math.round(turnCostCloseMs / 1000) }
        cleanup()
        if (onSave) onSave(oCost)
        return
      }
      // v761（#161 C5）：等待提示内容保存 —— 落在 usageSet.events.<kind>.lines（宿主 waitQuestion.content /
      // waitApproval.content 的那套键），**只**发 { events: { <kind>: { lines } } }，
      // 不碰 alert/budget/turnCost 的任何键（宿主那侧是逐键合并，其余字段原样保留）。
      if (isWait) {
        waitCommitted = true
        usageSet = usageSet || {}
        usageSet.events = usageSet.events || {}
        var prevEv = usageSet.events[waitKind] || {}
        var evNext = {}
        try { for (var pk in prevEv) evNext[pk] = prevEv[pk] } catch (err) {}
        evNext.lines = lines
        usageSet.events[waitKind] = evNext
        var patchEv = {}
        patchEv[waitKind] = { lines: lines }
        saveUsageSettings({ events: patchEv })
        var oWait = { lines: lines }
        cleanup()
        if (onSave) onSave(oWait)
        return
      }
      var o = { on: chk.checked, lines: lines, autoClose: acChk.checked, ttlSec: Math.max(0, Number(secInp.value) || 0) }
      if (isAlert) o.below = Math.max(0, Number(numInp.value) || 0)
      else o.amount = Math.max(0, Number(numInp.value) || 0)
      cleanup()
      if (onSave) onSave(o)
    })
    btns.appendChild(okBtn)
    card.appendChild(btns)
    // 模块大窗(随机/图片)与悬浮窗需盖在本窗口之上
    try { if (moduleMask) moduleMask.style.zIndex = '31000' } catch (err) {}
    // 提醒编辑期间:窗口内可能弹出的各类全屏遮罩(确认/裁剪/音频/快照/用量)统一置顶,杜绝层级错位
    var remindZStyle = document.createElement('style')
    remindZStyle.id = 'dshw-remind-overlay-z'
    // 同上（PR #114）：本文本表同样必须自带 data-plugin，否则会被别的客户端插件
    // 热重载时顺带删掉，提醒编辑期的遮罩层级就失效了。
    remindZStyle.setAttribute('data-plugin', 'dsh-whale-widget')
    // 提醒编辑期间:窗口内可能弹出的全屏遮罩(确认/裁剪/音频/快照)统一置顶,杜绝层级错位。
    // 注意:不要把 .dshwv-usage-mask 放进来——「模型子菜单/模型设置」用的是这个类(29000),
    // 一提权就会反盖到提醒编辑器(30000)上面。
    remindZStyle.textContent = '.dshwv-confirmmask,.dshwv-cropmask,.dshwv-audiomask,.dshwv-snapmask{z-index:32000!important}'
    document.head.appendChild(remindZStyle)
    // 悬浮编辑窗 qedit(默认 z=26000)与本窗口同层:窗口打开期间把它置顶,避免被遮罩盖住(点模块 ✎ 无反应)
    var qeditEnsureSuper = bkQeditEnsure
    qeditEnsure = function () {
      var el = qeditEnsureSuper()
      try { if (el) el.style.zIndex = '31000' } catch (err) {}
      return el
    }
    try { if (qeditEl) qeditEl.style.zIndex = '31000' } catch (err) {}
    // 包装 renderBubblePv:列表/预览后,鲸鱼预览里的占位符用实际值实时替换
    // ({below} 余额阈值 / {amount} 预算阈值 / {cost} 本轮消耗金额 / {session} 当前对话名)
    var renderPvSuper = bkRenderPv
    renderBubblePv = function () {
      try { renderPvSuper() } catch (err) {}
      try {
        var it = bubbleEditTarget()
        var below = (isAlert && numInp) ? Math.max(0, Number(numInp.value) || 0) : null
        var amount = (!isAlert && !isCost && !isWait && numInp) ? Math.max(0, Number(numInp.value) || 0) : null
        var cost = isCost ? '0.00' : null // 预览用示例金额(与 usageCostValue 的格式一致)
        // v761（#161 C5）：{session} 要在这里就换成对话名（读不到显示「当前对话」，超长按 12 字符截断），
        // 这样编辑器预览与真实泡泡看到的完全一致，而不是原样显示 `{session}`。
        var sess = isWait ? soundSessionLabel() : null
        if (it && Array.isArray(it.modules) && bubblePvPrevEl) bubblePreviewInto(bubblePvPrevEl, usageAlertModsResolved(it.modules, below, amount, cost, sess))
      } catch (err) {}
    }
    if (numInp) {
      numInp.addEventListener('input', renderBubblePv)
      numInp.addEventListener('change', renderBubblePv)
    }
    if (chk) chk.addEventListener('change', renderBubblePv)
    if (isCost) {
      // 打开窗口:任务结束音与自动关闭秒数进入"缓冲"模式(点「取消」还原),并刷新一次音效下拉选项
      taskEndDefer = true
      var teCur = (usageSet && usageSet.taskEnd) || { on: false, sel: '' }
      taskEndDeferSnap = JSON.parse(JSON.stringify({ on: !!teCur.on, sel: String(teCur.sel || '') }))
      // v763：taskEndSel 现在不挂进本窗口了（留在不插入文档的宿主里）⇒ 这两处同步都包 try，
      // 免得自绘下拉在未挂载状态下万一抛错，把整个窗口的渲染（mask.appendChild 等）一起带崩。
      try { applyTaskEndLocal(!!teCur.on, String(teCur.sel || '')) } catch (err) {}
      try { fillTaskEndOptions(teCur) } catch (err) {}
      turnCostCloseDefer = true
      turnCostCloseDeferSnap = turnCostCloseMs
      turnCostCloseInput.disabled = false
    }
    mask.appendChild(card)
    mask.addEventListener('click', function (e) { if (e.target === mask) cleanup() })
    window.__dshwRemindMask = mask
    dshwBodyAppend(mask)
    renderBubblePal()
    renderBubblePv()
  } catch (err) {}
}
// ===== 自定义 API 模型（v657）：状态 / 拉取 / 每模型提醒 / 模型设置窗口 =====
var apiModels = [] // 最近一次拉到的模型列表（含实时余额 / 今日已用）
var apiTemplates = [] // 可选厂商模板
var apiModelsLoaded = false
var apiAlertFired = {} // modelId+阈值 → 已弹过（低于阈值后恢复会复位）
var apiBudgetFired = {} // modelId+当日+金额 → 已弹过
var apiBudgetUnitWarned = {} // modelId → 已因「今日已用与预算阈值币种不一致且没有汇率」跳过比较并告警过
function apiModelById(id) {
  for (var i = 0; i < apiModels.length; i++) if (apiModels[i] && apiModels[i].id === id) return apiModels[i]
  return null
}
// 「充值 / 余额校正」只属于固定的 当前供应商：宿主在模型条目里下发 canAdjustBalance，
// 前端据此决定要不要在设置菜单里给出入口 —— 手动新增的同名模型、Kimi 等其它厂商都不会有，
// 新增模板也不会自动继承（宿主还会在路由层再校验一次，见 balance-adjustments.json）。
function apiCanAdjustBalance(model) { return !!(model && model.canAdjustBalance === true) }
// 今日已用金额自带的币种（host 下发的 todayUsageCurrency）：
// 会话事件金额在 host 已按自定义单价折算成人民币，余额差则是厂商币种 → 显示必须按各自的币种，
// 否则 USD 单价的模型会把人民币数值渲染成 $。
function apiTodayCur(m) {
  return String((m && (m.todayUsageCurrency || m.currency)) || 'CNY').toUpperCase() || 'CNY'
}
// 金额换算（只处理 CNY/USD；rate = 元/USD，取自该模型自定义单价里的汇率）
function apiConvertMoney(v, fromCur, toCur, rate) {
  var n = Number(v)
  if (!isFinite(n)) return null
  var f = String(fromCur || '').toUpperCase()
  var t = String(toCur || '').toUpperCase()
  if (!f || !t || f === t) return n
  var r = Number(rate)
  if (!isFinite(r) || r <= 0) return null
  if (f === 'CNY' && t === 'USD') return n / r
  if (f === 'USD' && t === 'CNY') return n * r
  return null
}
function apiFmtMoney(v, cur) {
  if (v === null || v === undefined || v === '') return '—'
  var n = Number(v)
  if (!isFinite(n)) return '--'
  var c = String(cur || '')
  if (c === 'USD') return '$ ' + n.toFixed(2)
  if (c === 'CNY') return '¥ ' + n.toFixed(2)
  return n.toFixed(2) + (c ? ' ' + c : '')
}
// 「今日已用」的来源标注（未知就返回空串，调用方不显示括号）
function apiUsageSourceLabel(src) {
  var s = String(src || '')
  if (s === 'ledger' || s === 'balance') return '余额差记账'
  if (s === 'events') return '本地估算'
  if (s === 'balance-observed') return '已观测消费'
  if (s === 'balance-needs-review') return '待核对余额调整'
  if (s === 'balance-corrected') return '已校正消费'
  if (s === 'legacy') return '旧版记录 · 未校正'
  return ''
}
// 模型列表加载：并发合并 + 失败退避重试 + 失败态可重试。
// 背景：重启 dsh web 后，浏览器连接池里指向旧实例的连接会立刻失败（一次），
// 旧实现失败后既不重试、也不重绘，界面就一直停在「加载中」直到手动刷新页面。
var apiModelsLoading = false
var apiModelsWaiters = []
var apiModelsError = ''
function apiModelsFlushWaiters() {
  var ws = apiModelsWaiters
  apiModelsWaiters = []
  for (var i = 0; i < ws.length; i++) {
    try { ws[i](apiModelsLoaded ? { ok: true, models: apiModels } : null) } catch (err) {}
  }
}
function loadApiModels(cb, force) {
  if (cb) apiModelsWaiters.push(cb)
  if (apiModelsLoading) return // 已有请求在飞：回调排队，避免重复请求
  if (apiModelsLoaded && !force) { apiModelsFlushWaiters(); return }
  apiModelsLoading = true
  apiModelsFetch(3)
}
function apiModelsFetch(tries) {
  var wasLoaded = apiModelsLoaded
  var opts = { cache: 'no-store' }
  // 兜底超时：host 要串行取各厂商余额，慢的时候不能把界面一直吊着
  try { if (typeof AbortSignal !== 'undefined' && AbortSignal.timeout) opts.signal = AbortSignal.timeout(40000) } catch (err) {}
  fetch('/dsh-whale/api-models.json', opts)
    .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.json() })
    .then(function (d) {
      if (d && d.ok && Array.isArray(d.models)) {
        apiModels = d.models
        apiTemplates = Array.isArray(d.templates) ? d.templates : []
        apiModelsLoaded = true
        apiModelsError = ''
        apiModelsLoading = false
        try { runApiModelAlerts() } catch (err) {}
        // 首次加载成功：把还停在「加载中」的面板/模块调色板重绘掉。
        // 注意模型列表在「静态区」里（buildUsageSubShell 只建一次），
        // 只调 refreshUsageMain 只会重绘下面的记录区，静态区仍停在旧状态 → 必须重建整个子界面。
        if (!wasLoaded) {
          try { if (usagePanelOpen && usageSet !== null) rebuildUsageSubShell() } catch (err) {}
          try { renderBubblePal() } catch (err) {}
        }
        apiModelsFlushWaiters()
        return
      }
      apiModelsLoading = false
      apiModelsError = '加载失败（点重试）'
      apiModelsFlushWaiters()
    })
    .catch(function () {
      if (tries > 0) {
        setTimeout(function () { try { apiModelsFetch(tries - 1) } catch (err) {} }, tries === 3 ? 600 : 1200)
        return
      }
      apiModelsLoading = false
      apiModelsError = '加载失败（点重试）'
      apiModelsFlushWaiters()
    })
}
function postApiModels(body, cb) {
  var opts = {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body || {}),
  }
  // 与 apiModelsFetch 一致：补超时，避免后端卡住时请求永不落地（保存按钮一直"保存中…"）
  try { if (typeof AbortSignal !== 'undefined' && AbortSignal.timeout) opts.signal = AbortSignal.timeout(40000) } catch (err) {}
  fetch('/dsh-whale/api-models.json', opts)
    .then(function (r) { return r.json() })
    .then(function (d) {
      if (d && Array.isArray(d.models)) apiModels = d.models
      if (cb) cb(d)
    })
    .catch(function () { if (cb) cb(null) })
}
// 用「某个模型自己的」提醒/预算打开既有编辑器：打开瞬间它只读一次 usageSet[key]，随后立即还原
function openModelAlertBudget(modelId, key, after) {
  try {
    if (!usageSet) return
    usageSet.models = usageSet.models || {}
    var st = usageSet.models[modelId] || (usageSet.models[modelId] = {})
    // 该模型还没有独立设置时，用顶层（DeepSeek）的设置作为初始模板；保存后即写入本模型
    var cur = st[key] || usageSet[key] || null
    if (!cur) return
    var bkA = usageSet.alert
    var bkB = usageSet.budget
    if (key === 'alert') usageSet.alert = cur
    else usageSet.budget = cur
    usageAlertBudgetEditor(key, function (o) {
      var next = {
        on: !!o.on,
        lines: o.lines,
        autoClose: o.autoClose !== false,
        ttlSec: o.ttlSec != null ? o.ttlSec : 6,
      }
      if (key === 'alert') next.below = o.below
      else next.amount = o.amount
      st[key] = next
      // 内置 DeepSeek 同步顶层，兼容旧逻辑
      if (modelId === 'current-provider') usageSet[key] = next
      var patch = { action: 'model-settings', id: modelId }
      patch[key] = next
      postApiModels(patch, function () { if (after) after() })
    })
    // 编辑器同步读完 cfg 后即可还原（之后不再读 usageSet）
    if (key === 'alert') usageSet.alert = bkA
    else usageSet.budget = bkB
  } catch (err) {}
}
// —— 模型设置 / 新增模型 窗口 ——
function apiPanelRow(label, el) {
  var r = document.createElement('div')
  r.className = 'dshwv-audiorow'
  r.style.margin = '0 0 7px'
  var l = document.createElement('span')
  l.textContent = label
  l.style.flex = '0 0 62px' // 标签固定宽度 → 所有输入框左边缘对齐
  l.style.textAlign = 'left'
  r.appendChild(l)
  r.appendChild(el)
  return r
}
// 面板内的小节标题（浅色卡片上使用既有 .dshwv-bubsec：自带 #9fb0d9 颜色与上分隔线）
function apiSec(text) {
  var d = document.createElement('div')
  d.className = 'dshwv-bubsec'
  d.style.margin = '10px 0 6px'
  d.textContent = text
  return d
}
function apiTextInput(val, ph, w) {
  var i = document.createElement('input')
  i.type = 'text'
  i.className = 'dshwv-cropname'
  i.style.flex = '1'
  i.style.minWidth = '0'
  // .dshwv-cropname 自带 margin:0 auto 12px；在 align-items:center 的 flex 行里会把输入框顶高 6px，
  // 这里清零并统一高度/左对齐，保证与左侧标签垂直居中
  i.style.margin = '0'
  i.style.height = '26px'
  i.style.boxSizing = 'border-box'
  i.style.textAlign = 'left'
  if (w) i.style.width = w
  i.value = val == null ? '' : String(val)
  i.placeholder = ph || ''
  return i
}
function apiSelectEl(opts, val) {
  var s = document.createElement('select')
  s.className = 'dshwv-sound'
  s.style.flex = '1'
  s.style.minWidth = '0'
  for (var i = 0; i < opts.length; i++) {
    var o = document.createElement('option')
    o.value = opts[i][0]
    o.textContent = opts[i][1]
    s.appendChild(o)
  }
  if (val != null) s.value = String(val)
  return s
}
function apiBtn(label, cls, fn) {
  var b = document.createElement('button')
  b.type = 'button'
  b.className = cls || 'dshwv-snapbtn dshwv-snapbtn-no'
  b.textContent = label
  if (fn) b.addEventListener('click', function (e) { e.stopPropagation(); fn() })
  return b
}
function openApiModelPanel(modelId) {
  try {
    closeApiModelPanel()
    var isNew = !modelId
    // 保存/删除后刷新「模型列表」：列表在记账子界面的**静态区**里（buildUsageSubShell 只建一次），
    // 只调 refreshUsageMain 只会重绘下面的记录区，静态区的模型行不会更新 → 必须重建整个子界面
    function refreshModelList() {
      try { loadApiModels(null, true) } catch (err) {}
      try { if (usagePanelOpen) rebuildUsageSubShell() } catch (err) {}
    }
    var m = isNew ? null : apiModelById(modelId)
    if (!isNew && !m) return
    var mask = document.createElement('div')
    mask.className = 'dshwv-usage-mask'
    mask.style.zIndex = '29000'
    var card = document.createElement('div')
    card.className = 'dshwv-usage-card'
    card.style.width = 'min(460px,94vw)'
    card.style.maxHeight = '86vh'
    card.style.overflow = 'auto'
    card.style.padding = '14px 16px'
    card.style.boxSizing = 'border-box'
    card.style.textAlign = 'left'
    var title = document.createElement('div')
    title.className = 'dshwv-bubtitle'
    title.textContent = isNew ? '新增模型（自定义 API）' : ('模型设置 · ' + (m.name || m.id))
    card.appendChild(title)
    var status = document.createElement('div')
    status.className = 'dshwv-bubhint'
    status.style.margin = '4px 0 8px'
    status.textContent = isNew ? '选择厂商模板 → 填 API key → 保存' : (m.error ? ('⚠ ' + m.error) : (m.balanceMode === 'events' ? ('余额 —（无接口·按会话事件）· 今日已用 ' + apiFmtMoney(m.todayUsage, apiTodayCur(m))) : ('余额 ' + apiFmtMoney(m.balance, m.currency) + ' · 今日已用 ' + apiFmtMoney(m.todayUsage, apiTodayCur(m)) + (apiUsageSourceLabel(m.usageSource) ? ('（' + apiUsageSourceLabel(m.usageSource) + '）') : ''))))
    card.appendChild(status)
    // 名称 / 模板 / 币种
    card.appendChild(apiSec('基本信息'))
    var nameInp = apiTextInput(isNew ? '' : m.name, '例如 OpenRouter / 我的中转站')
    card.appendChild(apiPanelRow('名称', nameInp))
    // v726：厂商下拉按「首字母」排序 —— 英文按字母、中文按拼音首字母（host 下发的 sortKey），中英混排 A→Z；
    // 同键（如 siliconflow CN/EN、moonshot CN/国际）再按名称排，保证顺序稳定。
    var tplList = []
    for (var ti = 0; ti < apiTemplates.length; ti++) {
      if (apiTemplates[ti].builtin) continue
      var tplName = String(apiTemplates[ti].name || apiTemplates[ti].id)
      tplList.push({
        id: apiTemplates[ti].id,
        name: tplName,
        // v724：官方「没有用 API key 查余额接口」的模板在下拉里直接标出来，避免选完以为坏了
        label: tplName + (apiTemplates[ti].noBalanceApi ? '（无余额接口）' : ''),
        sortKey: String(apiTemplates[ti].sortKey || tplName).toLowerCase(),
      })
    }
    tplList.sort(function (a, b) {
      if (a.sortKey < b.sortKey) return -1
      if (a.sortKey > b.sortKey) return 1
      if (a.name < b.name) return -1
      if (a.name > b.name) return 1
      return 0
    })
    var tplOpts = []
    for (var tl = 0; tl < tplList.length; tl++) tplOpts.push([tplList[tl].id, tplList[tl].label])
    var tplSel = apiSelectEl(tplOpts, isNew ? 'openrouter' : m.provider)
    if (!isNew) tplSel.disabled = true
    if (m && m.id === 'codex') tplSel.value = 'codex'
    card.appendChild(apiPanelRow('厂商模板', tplSel))
    // 模板口径说明（常显，**不**放进「高级」折叠区）：例如「官方无余额接口，按会话事件估算」
    var tplNote = document.createElement('div')
    tplNote.className = 'dshwv-bubhint'
    tplNote.style.margin = '0 0 7px'
    card.appendChild(tplNote)
    var curSel = apiSelectEl([['CNY', '人民币 ¥'], ['USD', '美元 $']], isNew ? 'USD' : m.currency)
    card.appendChild(apiPanelRow('币种', curSel))
    // 密钥
    card.appendChild(apiSec('密钥'))
    var keyRefInp = codexWhaleHost.profileSelect(isNew ? '' : m.keyRef)
    keyRefInp.title = '复用 Codex++ 已保存的供应商配置，密钥只在本机后端使用'
    card.appendChild(apiPanelRow('供应商配置', keyRefInp))
    var keyInp = document.createElement('input')
    keyInp.type = 'password'
    keyInp.disabled = true
    keyInp.className = 'dshwv-cropname'
    keyInp.style.flex = '1'
    keyInp.style.minWidth = '0'
    keyInp.style.margin = '0'
    keyInp.style.height = '26px'
    keyInp.style.boxSizing = 'border-box'
    keyInp.style.textAlign = 'left'
    keyInp.placeholder = '在 Codex++ 供应商配置中管理密钥'
    card.appendChild(apiPanelRow('API key', keyInp))
    var keyState = document.createElement('div')
    keyState.className = 'dshwv-bubhint'
    keyState.style.margin = '0 0 7px'
    keyState.textContent = isNew ? '' : (m.hasKey ? '已配置密钥 ✓' : '尚未配置密钥')
    card.appendChild(keyState)
    var delKey = apiBtn('删除密钥', 'dshwv-snapbtn dshwv-snapbtn-no', function () {
      var ref = (keyRefInp.value || '').trim()
      if (!ref) return
      postApiModels({ action: 'delete-key', keyRef: ref }, function () {
        keyState.textContent = '密钥已删除'
        // 列表里的"已配置/未配置"显示在静态区模型行 → 必须重建子界面（只刷记录区不够）
        refreshModelList()
      })
    })
    delKey.disabled = true
    delKey.title = '请在 Codex++ 供应商配置中管理密钥'
    var delKeyRow = apiPanelRow('', delKey)
    card.appendChild(delKeyRow)
    // Base URL（中转站/自定义需要）
    var baseInp = apiTextInput(isNew ? '' : m.baseUrl, '复用所选供应商的 Base URL')
    baseInp.readOnly = true
    var baseRow = apiPanelRow('Base URL', baseInp)
    card.appendChild(baseRow)
    // 余额接口（自定义可改；模板默认自动带上）
    // 用 host 下发的接口描述(balanceDesc)回填；m.balance 是数值，不能当描述用。
    // v724：新增模型时改用「所选模板」的描述做初始值（编辑态仍以注册表里的实际配置为准）——
    //      原来这里那句 `var tplDef = ...` 是从没被使用的死代码，一并删掉。
    var bal = (m && m.balanceDesc && typeof m.balanceDesc === 'object') ? m.balanceDesc : {}
    var tpl0 = null
    for (var t0 = 0; t0 < apiTemplates.length; t0++) if (apiTemplates[t0].id === tplSel.value) tpl0 = apiTemplates[t0]
    if (isNew && tpl0 && tpl0.balance) bal = tpl0.balance
    var balUrl = apiTextInput(bal.url || '', '余额接口 URL（自定义时必填）')
    var balUrlRow = apiPanelRow('余额接口', balUrl)
    card.appendChild(balUrlRow)
    var authInp = apiTextInput(bal.auth == null ? 'Bearer {key}' : bal.auth, '请求头模板，{key} 会被替换成密钥')
    var authRow = apiPanelRow('请求头', authInp)
    card.appendChild(authRow)
    // v789（issue #190）：余额接口不再只能是 GET —— 可选 POST/PUT… + JSON 请求体。
    //   请求体支持 {key}（密钥）、{base}（Base URL）、{uuid}（下面「请求参数」里的键）
    var balMethod = apiSelectEl([['GET', 'GET（默认）'], ['POST', 'POST'], ['PUT', 'PUT'], ['PATCH', 'PATCH']],
      String(bal.method || 'GET').toUpperCase() === 'POST' ? 'POST' : String(bal.method || 'GET').toUpperCase() === 'PUT' ? 'PUT' : String(bal.method || 'GET').toUpperCase() === 'PATCH' ? 'PATCH' : 'GET')
    var balMethodRow = apiPanelRow('请求方法', balMethod)
    card.appendChild(balMethodRow)
    var balBody = apiTextInput(bal.body || '', '如 {"uuid":"{uuid}"}（非 GET 时以 JSON 发送）')
    var balBodyRow = apiPanelRow('请求体', balBody)
    card.appendChild(balBodyRow)
    var prm = (m && m.params) || {}
    var prmInp = apiTextInput(Object.keys(prm).map(function (k) { return k + '=' + prm[k] }).join(','), '请求体占位符，如 uuid=abc123（逗号分隔）')
    var prmRow = apiPanelRow('请求参数', prmInp)
    card.appendChild(prmRow)
    // v789（issue #189）：0.3.15 起的"凭据目的地白名单"唯一例外必须能在界面上勾 ——
    //   宿主侧 model.allowCustomHost 一直是齐的，缺的就是这个入口（README/FAQ 都在指引用户来勾）。
    //   勾选/取消都**显式**传 true/false（宿主按 === true / === false 分支，缺省表示"不改动"）。
    var allowHost = document.createElement('input')
    allowHost.type = 'checkbox'
    allowHost.checked = !!(m && m.balanceDesc && m.balanceDesc.allowCustomHost === true) || !!(m && m.allowCustomHost === true)
    var allowHostRow = apiPanelRow('允许把凭据发送到自定义地址', allowHost)
    card.appendChild(allowHostRow)
    var allowHint = document.createElement('div')
    allowHint.className = 'dshwv-bubhint'
    allowHint.style.margin = '0 0 6px'
    allowHint.textContent = '默认不勾：地址不是该厂商内置端点时，插件会拒绝把密钥发出去。自建网关（New API / 自托管 / Ollama 等）确认是自己的服务后再勾。' +
      '该开关只能由本机（回环）来源的写请求置位，远端会话改不了它。'
    card.appendChild(allowHint)
    var jr = (bal.json || {})
    var jRem = apiTextInput(jr.remaining || '', '如 balance_infos[0].total_balance')
    var jTot = apiTextInput(jr.total || '', '如 data.total_credits')
    var jUse = apiTextInput(jr.used || '', '如 data.total_usage')
    var jSca = apiTextInput(jr.scale == null ? '' : jr.scale, '乘数，如 0.0001')
    var jRemRow = apiPanelRow('余额字段', jRem)
    var jTotRow = apiPanelRow('总量字段', jTot)
    var jUseRow = apiPanelRow('已用字段', jUse)
    var jScaRow = apiPanelRow('数值乘数', jSca)
    card.appendChild(jRemRow)
    card.appendChild(jTotRow)
    card.appendChild(jUseRow)
    card.appendChild(jScaRow)
    var u = bal.usage || {}
    var uUrl = apiTextInput(u.url || '', '可选：第二段用量接口（如 OpenAI 兼容 /usage）')
    var uUse = apiTextInput((u.json && u.json.used) || '', '用量字段，如 total_usage')
    var uSca = apiTextInput((u.json && u.json.scale) == null ? '' : u.json.scale, '乘数，如 0.01')
    var uUrlRow = apiPanelRow('用量接口', uUrl)
    var uUseRow = apiPanelRow('用量字段', uUse)
    var uScaRow = apiPanelRow('用量乘数', uSca)
    card.appendChild(uUrlRow)
    card.appendChild(uUseRow)
    card.appendChild(uScaRow)
    var matchInp = apiTextInput((m && Array.isArray(m.matchIds) ? m.matchIds.join(',') : ''), '会话事件里的模型名关键字，逗号分隔')
    var matchRow = apiPanelRow('事件匹配', matchInp)
    card.appendChild(matchRow)
    var tip = document.createElement('div')
    tip.className = 'dshwv-bubhint'
    tip.textContent = '「事件匹配」用于没有余额差的模型：本机每轮对话的真实 token 花费，按这里的关键字归到本模型。'
    card.appendChild(tip)
    // —— 自定义单价（可选，元/百万 token）：给「没有余额接口/余额差」的模型按真实 token 换算金额 ——
    // v721:按用户要求，这块并入下方「接口与字段（高级）」折叠区（不再单独常显）。
    // 注意：高级区默认收起 → 子菜单里那行只读「单价」的提示负责告诉用户去哪填。
    var prc = (m && m.price) || {}
    var pHit = apiTextInput(prc.hit == null ? '' : prc.hit, '例: 0.02')
    var pMiss = apiTextInput(prc.miss == null ? '' : prc.miss, '例: 1.0')
    var pOut = apiTextInput(prc.out == null ? '' : prc.out, '例: 4.0')
    var pCur = apiSelectEl([['CNY', '人民币（元 / CNY）'], ['USD', '美元（$ / USD）']], String(prc.cur || 'CNY').toUpperCase() === 'USD' ? 'USD' : 'CNY')
    var pRate = apiTextInput(prc.rate == null ? '' : prc.rate, '仅美元时需要：汇率（元/USD），例 7.1')
    card.appendChild(apiSec('单价（可选）'))
    card.appendChild(apiPanelRow('缓存命中', pHit))
    card.appendChild(apiPanelRow('未命中输入', pMiss))
    card.appendChild(apiPanelRow('输出', pOut))
    card.appendChild(apiPanelRow('币种', pCur))
    card.appendChild(apiPanelRow('汇率', pRate))
    var pTip = document.createElement('div')
    pTip.className = 'dshwv-bubhint'
    pTip.style.margin = '0 0 6px'
    pTip.textContent = '单位为「币种 / 百万 token」，留空则沿用内置价目表。币种选美元时请填汇率，记账会换算成人民币（账本统一按 CNY 结算）；自定义单价不分峰谷。'
    card.appendChild(pTip)
    // 按钮
    var btns = document.createElement('div')
    btns.className = 'dshwv-bubbtns'
    function collect() {
      var json = {
        remaining: (jRem.value || '').trim(),
        total: (jTot.value || '').trim(),
        used: (jUse.value || '').trim(),
        scale: isFinite(Number(jSca.value)) && String(jSca.value).trim() !== '' ? Number(jSca.value) : undefined,
      }
      var body = {
        action: 'save',
        model: {
          id: isNew ? undefined : m.id,
          name: (nameInp.value || '').trim(),
          provider: tplSel.value,
          currency: curSel.value,
          keyRef: (keyRefInp.value || '').trim(),
          baseUrl: (baseInp.value || '').trim(),
          balance: {
            url: (balUrl.value || '').trim(),
            auth: (authInp.value || '').trim(),
            // v789：显式带上方法/请求体（GET + 空体 = 老行为；宿主侧空值不覆盖模板）
            method: (balMethod.value || 'GET').toUpperCase(),
            body: (balBody.value || '').trim(),
            json: json,
          },
          // v789（issue #189）：显式 true/false（不是"仅 true 才传"）—— 用户要能**取消**勾选
          allowCustomHost: !!allowHost.checked,
          params: (function () {
            var o = {}
            String(prmInp.value || '').split(',').forEach(function (kv) {
              var i = kv.indexOf('=')
              if (i <= 0) return
              var k = kv.slice(0, i).trim()
              var v = kv.slice(i + 1).trim()
              if (/^[A-Za-z_][A-Za-z0-9_]{0,30}$/.test(k)) o[k] = v
            })
            return o
          })(),
          matchIds: (matchInp.value || '').split(',').map(function (s) { return s.trim() }).filter(function (s) { return s.length > 0 }),
          price: { hit: (pHit.value || '').trim(), miss: (pMiss.value || '').trim(), out: (pOut.value || '').trim(), cur: pCur.value, rate: (pRate.value || '').trim() },
        },
      }
      var uu = (uUrl.value || '').trim()
      if (uu) {
        body.model.balance.usage = {
          url: uu,
          json: {
            used: (uUse.value || '').trim(),
            scale: isFinite(Number(uSca.value)) && String(uSca.value).trim() !== '' ? Number(uSca.value) : undefined,
          },
        }
      }
      var kv = keyInp.value || ''
      if (kv) body.keyValue = kv
      return body
    }
    if (!isNew) {
      var testBtn = apiBtn('测试连通性', 'dshwv-bubbtn dshwv-bubbtn-no', function () {
        testBtn.disabled = true
        testBtn.textContent = '测试中…'
        postApiModels({ action: 'probe', id: m.id }, function (r) {
          testBtn.disabled = false
          testBtn.textContent = '测试连通性'
          var okP = !!(r && r.ok)
          // 结果只用弹窗呈现（模型面板 z=29000，确认弹窗需临时提到其之上）
          try { confirmMask.style.setProperty('z-index', '29500', 'important') } catch (err) {}
          showConfirm(okP ? ('✓ 测试通过\n' + (r.detail || '')) : ('⚠ 测试失败\n' + ((r && r.error) || '未知原因')), function () {}, '知道了')
        })
      })
      btns.appendChild(testBtn)
    }
    btns.appendChild(apiBtn('保存', 'dshwv-bubbtn dshwv-bubbtn-ok', function () {
      status.textContent = '保存中…'
      var nm = (nameInp.value || '').trim()
      postApiModels(collect(), function (d) {
        if (!d || !d.ok) { status.textContent = '保存失败: ' + ((d && d.error) || '未知错误'); return }
        // 新建模型：保存后给一个明确的收尾——弹「保存成功」，点确认即关掉本面板。
        // （原先是重新打开为编辑态，新建流程会停在一个没有明显关闭入口的面板上）
        if (isNew) {
          // v744 清理：这里原先有 `confirmMask.style.zIndex = '29500' !important`，但紧接着的
          // showConfirm() 会把确认框统一提到 40000 !important，所以那行是**无效设置**（立刻被覆盖），
          // 只会让"层级"更难读。已删除 —— 确认框永远由 showConfirm 统一抬到 40000。
          showConfirm('✓ 保存成功：' + (nm || '新模型') + '\n已加入模型列表', function () {
            try { closeApiModelPanel() } catch (err) {}
            // 背后的「- = 小鲸鱼记账 = -」列表同步刷新（模型行在静态区，需重建子界面）
            refreshModelList()
          }, '确认')
          return
        }
        openApiModelPanel(d.id) // 编辑态：重新打开以刷新余额/错误
        refreshModelList()
      })
    }))
    if (!isNew && m.id !== 'codex') {
      btns.appendChild(apiBtn('删除模型', 'dshwv-bubbtn dshwv-bubbtn-no', function () {
        showConfirm('删除模型「' + (m.name || m.id) + '」？\n该模型的提醒/预算与所有泡泡里引用它的模块会一并移除。', function () {
          postApiModels({ action: 'delete', id: m.id }, function (d) {
            closeApiModelPanel()
            refreshModelList()
            if (d && d.removedModules) { try { showConfirm('已同时移除 ' + d.removedModules + ' 个引用该模型的泡泡模块', function () {}, '知道了') } catch (err) {} }
          })
        })
      }))
    }
    btns.appendChild(apiBtn('取消', 'dshwv-bubbtn dshwv-bubbtn-no', closeApiModelPanel))
    card.appendChild(btns)
    // —— 布局整理（v663）——
    // 1) 按钮行固定在卡片最下方（卡片自身滚动，按钮 sticky）
    // v722:原来 sticky 行只覆盖"内容盒"，而卡片自身还有 14px 底 padding / 16px 左右 padding，
    // 于是按钮行下方与两侧会留出透明缝，卡片滚动时下面的内容（高级区字段）就从缝里露出来，
    // 看起来像"按钮没有完全遮住底部"。修法：底部 padding 移到按钮行自己身上 + 左右负边距出血到卡片边缘。
    try {
      card.style.paddingBottom = '0' // 底部间距交给按钮行的 padding，避免它下方留缝
      card.style.overflowX = 'hidden' // 按钮行出血后不要再出现横向滚动条
      card.style.overflowY = 'auto'
      btns.style.position = 'sticky'
      btns.style.bottom = '0'
      btns.style.background = '#fff'
      btns.style.paddingTop = '8px'
      btns.style.paddingBottom = '14px' // 等于卡片原来的底部 padding
      btns.style.marginTop = '8px'
      btns.style.marginLeft = '-16px' // 出血：与卡片左右 padding 等宽，盖住两侧的缝
      btns.style.marginRight = '-16px'
      btns.style.paddingLeft = '16px'
      btns.style.paddingRight = '16px'
      btns.style.borderTop = '1px solid rgba(32,49,112,.12)'
      btns.style.zIndex = '2'
    } catch (err) {}
    // 2)「删除密钥」移到密钥输入框右侧，并移除原来的独立行
    try {
      keyInp.parentNode.appendChild(delKey)
      keyInp.style.flex = '1'
      keyInp.style.minWidth = '0'
      if (delKeyRow && delKeyRow.parentNode) delKeyRow.parentNode.removeChild(delKeyRow)
    } catch (err) {}
    // 3) 不重要的接口/字段内容默认收起（从「余额接口」行到按钮行之间的全部内容）
    try {
      var advBox = document.createElement('div')
      // 展开/收起过渡：max-height + 透明度（收起态 max-height:0；展开后放开限制避免截断）
      advBox.style.overflow = 'hidden'
      advBox.style.maxHeight = '0px'
      advBox.style.opacity = '0'
      advBox.style.transition = 'max-height .24s ease, opacity .18s ease'
      // v718 关键修复：卡片是 flex 列容器，而本框 overflow:hidden → 它的自动最小高度为 0。
      // 内容比可视区高时，flex 会把它"压扁"并把内容裁掉（卡片不溢出 → 不出现滚动条），
      // 表现就是「展开后下面的字段（单价/接口等）看不见也点不到」。禁止收缩 → 卡片自身出滚动条。
      advBox.style.flexShrink = '0'
      advBox.setAttribute('data-open', '0')
      var toMove = []
      var started = false
      var kidsA = Array.prototype.slice.call(card.children)
      for (var ai = 0; ai < kidsA.length; ai++) {
        var elA = kidsA[ai]
        if (elA === balUrlRow) started = true
        if (elA === btns) break
        if (started) toMove.push(elA)
      }
      var advToggle = apiBtn('接口与字段（高级）▾', 'dshwv-usage-more', function () {
        var open = advBox.getAttribute('data-open') === '1'
        if (!open) {
          advBox.setAttribute('data-open', '1')
          advBox.style.maxHeight = advBox.scrollHeight + 'px'
          advBox.style.opacity = '1'
          advToggle.textContent = '接口与字段（高级）▴'
          setTimeout(function () {
            if (advBox.getAttribute('data-open') === '1') advBox.style.maxHeight = 'none'
          }, 280)
        } else {
          advBox.setAttribute('data-open', '0')
          advBox.style.maxHeight = advBox.scrollHeight + 'px'
          void advBox.offsetHeight // 先固定当前高度再收到 0，否则过渡不生效
          advBox.style.maxHeight = '0px'
          advBox.style.opacity = '0'
          advToggle.textContent = '接口与字段（高级）▾'
        }
      })
      advToggle.style.width = '100%'
      advToggle.style.margin = '2px 0 6px'
      card.insertBefore(advToggle, balUrlRow)
      card.insertBefore(advBox, balUrlRow)
      for (var mi = 0; mi < toMove.length; mi++) advBox.appendChild(toMove[mi])
      // v721:单价区块不再单独移出 —— 它随 toMove 一起进「高级」折叠区（按 DOM 顺序排在最后）
      // 通用兜底：卡片所有直接子项都禁止收缩（收缩=被压扁/裁切；我们要的是卡片滚动）
      for (var ci = 0; ci < card.children.length; ci++) {
        try { card.children[ci].style.flexShrink = '0' } catch (err) {}
      }
    } catch (err) {}
    mask.appendChild(card)
    mask.addEventListener('click', function (e) { if (e.target === mask) closeApiModelPanel() })
    dshwBodyAppend(mask)
    apiModelMaskEl = mask
    // 模板切换：把模板默认值填进各字段（仅新增态）
    if (isNew) {
      function applyTpl() {
        var t = null
        for (var k = 0; k < apiTemplates.length; k++) if (apiTemplates[k].id === tplSel.value) t = apiTemplates[k]
        if (!t) return
        // ① 基本项：凭据名始终跟随所选模板（避免沿用上一次的值把密钥写进错误的 ref）+ 币种
        keyRefInp.value = keyRefInp.value || (codexWhaleHost.state.relayProfiles[0] || {}).id || ''
        curSel.value = t.currency || 'CNY'
        // ② Base URL：中转站 / 自定义 / 本地模型需要
        baseRow.style.display = t.needsBaseUrl ? '' : 'none'
        // ③ 接口与字段：模板带什么就回填什么（v724，B 方案核心）。Codex 模式不需要这些行。
        var isCodexT = t.kind === 'codex'
        var tb = t.balance || {}
        var tj = tb.json || {}
        var tu = tb.usage || {}
        balUrl.value = tb.url || ''
        authInp.value = tb.auth == null ? 'Bearer {key}' : tb.auth
        jRem.value = tj.remaining || ''
        jTot.value = tj.total || ''
        jUse.value = tj.used || ''
        jSca.value = tj.scale == null ? '' : String(tj.scale)
        uUrl.value = tu.url || ''
        uUse.value = (tu.json && tu.json.used) || ''
        uSca.value = (tu.json && tu.json.scale) == null ? '' : String(tu.json.scale)
        var ifaceRows = [balUrlRow, authRow, jRemRow, jTotRow, jUseRow, jScaRow, uUrlRow, uUseRow, uScaRow]
        for (var ri = 0; ri < ifaceRows.length; ri++) ifaceRows[ri].style.display = isCodexT ? 'none' : ''
        // ④ 事件匹配：模板给的默认关键字（切换模板即重填，之后可自行改）
        if (Array.isArray(t.matchIds) && t.matchIds.length) matchInp.value = t.matchIds.join(', ')
        // ⑤ 口径说明：优先用模板自己的说明（含「官方无余额接口」这类），常显在模板选择下方
        var hasBal = !!String(tb.url || '').trim()
        tplNote.textContent = t.apiNote ? ('ℹ ' + t.apiNote) : ''
        tip.textContent = hasBal
          ? '「事件匹配」用于没有余额差的模型：本机每轮对话的真实 token 花费，按这里的关键字归到本模型。'
          : '该厂商没有「用 API key 查余额」的接口 → 余额显示「—」，今日已用按会话事件估算（本机每轮对话的真实 token）。'
        // ⑥ 余额接口行保持可见（即使模板没有默认地址）：用户可以自己填
        balUrlRow.style.display = isCodexT ? 'none' : ''
      }
      tplSel.addEventListener('change', applyTpl)
      applyTpl()
    } else {
      var needBase = false
      for (var tk = 0; tk < apiTemplates.length; tk++) if (apiTemplates[tk].id === m.provider && apiTemplates[tk].needsBaseUrl) needBase = true
      baseRow.style.display = needBase ? '' : 'none'
    }
  } catch (err) {}
}
var apiModelMaskEl = null
// v748：Codex 模型的设置菜单打开时，记下"当前是哪个模型"和"用量行元素"，
// 这样拨动「Codex 本机统计」开关、等宿主确认后可以**原地**刷新那一行，不必关窗重开。
var apiModelMenuId = null
var codexUsageTextEl = null
function refreshOpenCodexRow() {
  try {
    if (!codexUsageTextEl || !apiModelMenuId) return
    var t = apiCodexDetailText(apiModelMenuId)
    codexUsageTextEl.textContent = t
    codexUsageTextEl.title = t
  } catch (err) {}
}
function closeApiModelPanel() {
  // v744：必须走 dshwBodyDetach —— 这个遮罩是登记在案的 body 节点，
  // 若用 parentNode.removeChild 直接摘掉，DOM 守护会认为"被别的插件摘走了"并把它补挂回来，
  // 表现就是**点「取消」关不掉这个窗口**（0.3.7 引入的回归）。
  try { if (apiModelMaskEl) dshwBodyDetach(apiModelMaskEl) } catch (err) {}
  apiModelMaskEl = null
  apiModelMenuId = null
  codexUsageTextEl = null
}
// 列宽受限 + 悬浮滚动：内容超出列宽时，鼠标移上去文字自动横向滚动，移开回到起点。
// 需要 .dshwv-marq（外层 overflow:hidden）+ 内层 inline-block span 配合。
function mkScrollCell(text, cls, styleObj) {
  var box = document.createElement('span')
  if (cls) box.className = cls
  box.classList.add('dshwv-marq')
  if (styleObj) for (var k in styleObj) { try { box.style[k] = styleObj[k] } catch (err) {} }
  var inner = document.createElement('span')
  inner.textContent = text == null ? '' : String(text)
  box.appendChild(inner)
  box.addEventListener('mouseenter', function () {
    var dx = box.scrollWidth - box.clientWidth
    if (dx <= 1) return
    var sec = Math.max(2, Math.min(12, dx / 30))
    inner.style.transition = 'transform ' + sec + 's linear'
    inner.style.transform = 'translateX(-' + dx + 'px)'
  })
  box.addEventListener('mouseleave', function () {
    inner.style.transition = 'transform .25s ease'
    inner.style.transform = 'translateX(0)'
  })
  return box
}
function buildUsageSettingsArea() {
  var S = usageSet || {}
  var stA = S.alert || { on: false, below: 50 }
  var stB = S.budget || { on: false, amount: 20 }
  // 每项一行「名称 + 当前状态 + 编辑」;点「编辑」打开配置卡片
  function mkRow(labelTxt, key, stateFn, persist) {
    var row = menuRow()
    var lb = menuLabel(labelTxt)
    lb.style.flex = '0 0 auto'
    row.appendChild(lb)
    var info = document.createElement('span')
    info.className = 'dshwv-usage-hint'
    info.style.flex = '1'
    info.style.textAlign = 'right'
    info.style.paddingRight = '6px'
    info.style.whiteSpace = 'nowrap'
    info.style.overflow = 'hidden'
    info.style.textOverflow = 'ellipsis'
    row.appendChild(info)
    var btn = document.createElement('button')
    btn.type = 'button'
    btn.className = 'dshwv-roleimport'
    btn.textContent = '编辑'
    btn.title = '打开配置卡片(启用 / 数值 / 提醒文案)'
    btn.addEventListener('click', function () {
      usageAlertBudgetEditor(key, function (o) {
        persist(o)
        info.textContent = stateFn()
      })
    })
    row.appendChild(btn)
    usagePanel.appendChild(row)
    info.textContent = stateFn()
  }
  // —— 模型（自定义 API）：余额预警 / 今日预算 都移进各个模型自己的子菜单 ——
  // 与菜单里其它设置行同款（.dshwv-audiorow 自带 color:#203170，不额外设字体/颜色）
  var secRow = document.createElement('div')
  secRow.className = 'dshwv-audiorow'
  // .dshwv-audiorow 默认 margin 是 0 0 10px，而下面各模型行是 5px 0，
  // 不统一会让标题行与第一个模型之间多出 5px。这里与模型行保持一致。
  secRow.style.margin = '5px 0'
  var secLb = document.createElement('span')
  secLb.textContent = '模型（提醒 / 预算 / 额度）'
  secLb.style.flex = '0 0 auto'
  secRow.appendChild(secLb)
  // 手动刷新：强制重新拉取各模型余额/额度/今日已用，然后重建整个子界面（含本静态区）
  var secGap = document.createElement('span')
  secGap.style.flex = '1'
  secRow.appendChild(secGap)
  var secBtn = document.createElement('button')
  secBtn.type = 'button'
  // 用模块调色板的浅色 chip 样式（.dshwv-palchip：自带显式颜色），
  // 与下面各模型行的深蓝实心「设置」按钮在视觉上区分开，一眼看出是「区块级动作」
  secBtn.className = 'dshwv-palchip'
  secBtn.textContent = '刷新'
  secBtn.title = '手动刷新各模型的余额 / 额度 / 今日已用'
  secBtn.addEventListener('click', function (e) {
    e.stopPropagation()
    if (secBtn.disabled) return
    secBtn.disabled = true
    secBtn.textContent = '刷新中…'
    secBtn.style.opacity = '.6'
    apiModelsError = ''
    var settled = false
    function onDone() {
      if (settled) return
      settled = true
      // 模型列表在本静态区里，只刷记录区不会更新它 → 重建子界面（按钮随之恢复为「刷新」）
      try { rebuildUsageSubShell() } catch (err) {}
    }
    // 兜底：25 秒还没回来也先把按钮恢复，避免一直卡在「刷新中…」（请求回来时照常重绘）
    setTimeout(function () {
      if (settled) return
      try { secBtn.disabled = false; secBtn.textContent = '刷新'; secBtn.style.opacity = '1' } catch (err) {}
    }, 25000)
    try { loadApiModels(onDone, true) } catch (err) { onDone() }
  })
  secRow.appendChild(secBtn)
  usagePanel.appendChild(secRow)
  if (!apiModelsLoaded) {
    var ld = document.createElement('div')
    ld.className = 'dshwv-usage-hint'
    ld.textContent = apiModelsError ? ('⚠ ' + apiModelsError) : '加载中…'
    usagePanel.appendChild(ld)
    if (apiModelsError) {
      var rbtn = document.createElement('button')
      rbtn.type = 'button'
      rbtn.className = 'dshwv-roleimport'
      rbtn.textContent = '重试'
      rbtn.addEventListener('click', function (e) {
        e.stopPropagation()
        apiModelsError = ''
        loadApiModels(function () { try { refreshUsageMain() } catch (err) {} }, true)
      })
      usagePanel.appendChild(rbtn)
    } else {
      // 加载完成后重绘本面板（成功→出模型列表；失败→出上面的重试按钮），不会自激
      loadApiModels(function () { try { refreshUsageMain() } catch (err) {} })
    }
  }
  apiModels.forEach(function (am) {
    var r = document.createElement('div')
    r.className = 'dshwv-audiorow'
    r.style.margin = '5px 0' // 与主菜单行距(.dshwv-menu-row 5px)一致
    // 名称列：固定宽度（70px），超出部分靠悬浮滚动查看
    var n = mkScrollCell(am.builtin ? (am.name + '（内置）') : am.name, 'dshwv-usage-model', { flex: '0 0 70px', boxSizing: 'border-box' })
    n.title = am.id + (am.keyRef ? ' · ' + am.keyRef : '')
    r.appendChild(n)
    // 内容列：占满剩余宽度，超出同样悬浮滚动
    var qOn = !!(am.quota && am.quota.on)
    var planOn = !!(am.planSupport && am.plan && am.plan.ok)
    var infoTxt = ''
    if (am.error) infoTxt = '⚠ ' + am.error
    else if (am.codex && am.codex.ok) infoTxt = apiCodexRowText(am)
    else if (planOn) infoTxt = '厂商额度 ' + apiPlanSummary(am.id)
    else if (qOn) infoTxt = '额度 ' + apiQuotaSummary(am.id)
    else if (am.balanceMode === 'events') infoTxt = '余额 —（无接口·按事件）· 今日 ' + apiFmtMoney(am.todayUsage, apiTodayCur(am))
    else infoTxt = apiFmtMoney(am.balance, am.currency) + ' · 今日 ' + apiFmtMoney(am.todayUsage, apiTodayCur(am))
    var info = mkScrollCell(infoTxt, 'dshwv-usage-hint', { flex: '1 1 auto', minWidth: '0', textAlign: 'right', paddingRight: '6px' })
    info.title = infoTxt
    r.appendChild(info)
    var btn = document.createElement('button')
    btn.type = 'button'
    btn.className = 'dshwv-roleimport'
    btn.textContent = '设置'
    btn.title = '该模型的提醒 / 预算 / 密钥与接口'
    btn.addEventListener('click', function (e) { e.stopPropagation(); openApiModelMenu(am.id) })
    r.appendChild(btn)
    usagePanel.appendChild(r)
  })
  var addRow = document.createElement('div')
  addRow.className = 'dshwv-menu-row' // 与主菜单「资源管理」行同款
  addRow.style.margin = '5px 0'
  var addModelBtn = apiBtn('+ 添加模型（自定义 API）', 'dshwv-usage-more', function () {
    // 确保厂商模板已就绪再打开表单（否则模板下拉会是空的）
    if (apiTemplates.length) openApiModelPanel(null)
    else loadApiModels(function () { openApiModelPanel(null) })
  })
  addModelBtn.style.flex = '1'
  addModelBtn.style.margin = '0'
  addModelBtn.style.width = '100%'
  addRow.appendChild(addModelBtn)
  usagePanel.appendChild(addRow)
  // 添加按钮下方分隔线
  var palDivider = document.createElement('div')
  palDivider.style.borderTop = '1px solid rgba(32,49,112,.15)'
  palDivider.style.margin = '6px 0 8px'
  usagePanel.appendChild(palDivider)
  // 右侧滚动条再靠右一点（减小面板右内边距）
  try { usagePanel.style.paddingRight = '6px' } catch (err) {}
}
// 某个模型的子菜单：余额预警 / 今日预算（各自独立）+ 密钥/接口入口
// —— 手动额度编辑器（订阅 / 资源包：厂商没有额度接口，已用值手填）——
// 注意：保存后不重开子菜单（v697 起），所以没有 after 回调参数
function openModelQuotaEditor(modelId) {
  try {
    closeApiModelPanel()
    var m = apiModelById(modelId)
    if (!m) return
    var q = Object.assign({ on: false, total: 0, unit: 'tokens', used: 0, reset: 'none' }, apiQuotaOf(modelId) || {})
    var mask = document.createElement('div')
    mask.className = 'dshwv-usage-mask'
    mask.style.zIndex = '30000'
    var card = document.createElement('div')
    card.className = 'dshwv-usage-card'
    card.style.width = 'min(430px,94vw)'
    card.style.padding = '14px 16px'
    card.style.boxSizing = 'border-box'
    card.style.textAlign = 'left'
    var title = document.createElement('div')
    title.className = 'dshwv-bubtitle'
    title.textContent = '额度 · ' + m.name
    card.appendChild(title)
    var hint = document.createElement('div')
    hint.className = 'dshwv-bubhint'
    hint.style.margin = '4px 0 8px'
    hint.textContent = '订阅 / 资源包用这里：总量填套餐额度，已用按会话 token 自动累计（跨天保留）。泡泡里可用 {quota} 已用百分比、{quota_used} 已用、{quota_left} 剩余、{quota_total} 总量、{quota_reset} 重置倒计时'
    card.appendChild(hint)
    var onInp = document.createElement('input')
    onInp.type = 'checkbox'
    onInp.checked = !!q.on
    card.appendChild(apiPanelRow('启用', onInp))
    // 单位先建好（仍按原顺序摆放）：金额单位下不允许「自动统计」——自动累计的是 token，不是钱
    var unitSel = apiSelectEl([['tokens', 'tokens（token 数）'], ['money', '金额（元）']], q.unit)
    var modeSel = null
    function buildModeSel() {
      var money = unitSel.value === 'money'
      // 金额单位：只给「手动填写」；已存在的 money+auto 数据在这里会显示为手动（used 值不会被清掉）
      var cur = money ? 'manual' : ((q.mode === 'manual' || q.mode === 'codex') ? q.mode : 'auto')
      var opts = money
        ? [['manual', '手动填写']]
        : [['auto', '自动统计（按会话 token，推荐）'], ['codex', 'Codex 本地会话 token'], ['manual', '手动填写']]
      return apiSelectEl(opts, cur)
    }
    modeSel = buildModeSel()
    var modeRow = apiPanelRow('已用来源', modeSel)
    card.appendChild(modeRow)
    var moneyHint = document.createElement('div')
    moneyHint.className = 'dshwv-bubhint'
    moneyHint.style.margin = '0 0 8px'
    moneyHint.textContent = '金额单位请手动填写：自动累计统计的是 token，不是钱。'
    moneyHint.style.display = unitSel.value === 'money' ? '' : 'none'
    card.appendChild(moneyHint)
    var totalInp = apiTextInput(q.total || '', '例: 20000000')
    card.appendChild(apiPanelRow('总量', totalInp))
    card.appendChild(apiPanelRow('单位', unitSel))
    var usedInp = apiTextInput(q.used || '', '例: 0（接入这个挂件之前已经用掉的 token 数，不知道就填 0）')
    card.appendChild(apiPanelRow('之前已用', usedInp))
    var resetSel = apiSelectEl([['none', '不重置'], ['daily', '每日重置'], ['monthly', '每月重置']], q.reset)
    card.appendChild(apiPanelRow('重置', resetSel))
    var resetBaseInp = document.createElement('input')
    resetBaseInp.type = 'checkbox'
    card.appendChild(apiPanelRow('重置已用基准', resetBaseInp))
    var baseHint = document.createElement('div')
    baseHint.className = 'dshwv-bubhint'
    baseHint.style.margin = '0 0 8px'
    baseHint.textContent = '勾选「重置已用基准」并保存：把自动统计的起点设为当前值（比如换了新资源包时用），已用从 0 重新算。'
    card.appendChild(baseHint)
    var autoHint = document.createElement('div')
    autoHint.className = 'dshwv-bubhint'
    autoHint.style.margin = '0 0 6px'
    autoHint.textContent = '当前自动统计：已用 ' + apiQuotaUsedText(modelId) + apiQuotaUnitSuffix(apiQuotaInfo(modelId)) + ' · 今日 ' + apiFmtQuotaNum((apiQuotaOf(modelId) || {}).autoToday) + ' tokens'
    card.appendChild(autoHint)
    // 金额单位下「自动统计」不成立：把「已用来源」的选项与这条提示一起收敛
    autoHint.style.display = unitSel.value === 'money' ? 'none' : ''
    function syncUnitMode() {
      var money = unitSel.value === 'money'
      var fresh = buildModeSel()
      try { modeRow.replaceChild(fresh, modeSel) } catch (err) { modeRow.appendChild(fresh) }
      modeSel = fresh
      moneyHint.style.display = money ? '' : 'none'
      autoHint.style.display = money ? 'none' : ''
      usedInp.placeholder = money
        ? '例: 0（接入这个挂件之前已经用掉的金额）'
        : '例: 0（接入这个挂件之前已经用掉的 token 数，不知道就填 0）'
    }
    unitSel.addEventListener('change', syncUnitMode)
    var btns = document.createElement('div')
    btns.className = 'dshwv-bubbtns'
    btns.appendChild(apiBtn('取消', 'dshwv-bubbtn dshwv-bubbtn-no', closeApiModelPanel))
    btns.appendChild(apiBtn('保存', 'dshwv-bubbtn dshwv-bubbtn-ok', function () {
      var next = {
        on: !!onInp.checked,
        mode: (modeSel.value === 'manual' || modeSel.value === 'codex') ? modeSel.value : 'auto',
        total: Number(totalInp.value) || 0,
        unit: unitSel.value === 'money' ? 'money' : 'tokens',
        used: Math.max(0, Number(usedInp.value) || 0),
        reset: resetSel.value || 'none',
        resetBase: !!resetBaseInp.checked,
      }
      // 本地先落一份乐观更新：保存后立刻重绘时不会读到旧值
      // （apiModels 里的 quota 要等下面 force 拉回来才更新，中间那一瞬间会显示成"未启用"）
      var mm = apiModelById(modelId)
      if (mm) mm.quota = next
      if (usageSet) {
        usageSet.models = usageSet.models || {}
        var st = usageSet.models[modelId] || (usageSet.models[modelId] = {})
        st.quota = next
      }
      // 保存后重新拉一次列表：autoUsed 由 host 计算，拉回来才准
      postApiModels({ action: 'model-settings', id: modelId, quota: next }, function () {
        // 必须 force：否则「已加载」短路会把这次刷新变成空操作，界面读回旧值
        loadApiModels(function () {
          // 额度摘要显示在模型行（静态区）→ 需要重建子界面，只重绘记录区是不够的。
          // 注意：这里**不再**自动重开模型子菜单 —— 否则保存后面板已关闭，
          // 子菜单却会在请求返回时"自己弹出来"，且那一瞬间读到的还是旧状态。
          try { rebuildUsageSubShell() } catch (err) {}
        }, true)
      })
      closeApiModelPanel()
    }))
    card.appendChild(btns)
    mask.appendChild(card)
    mask.addEventListener('click', function (e) { if (e.target === mask) closeApiModelPanel() })
    dshwBodyAppend(mask)
    apiModelMaskEl = mask
  } catch (err) {}
}
function openApiModelMenu(modelId) {
  try {
    closeApiModelPanel()
    var m = apiModelById(modelId)
    if (!m) return
    var mask = document.createElement('div')
    mask.className = 'dshwv-usage-mask'
    mask.style.zIndex = '29000'
    var card = document.createElement('div')
    card.className = 'dshwv-usage-card'
    card.style.width = 'min(430px,94vw)'
    card.style.padding = '14px 16px'
    card.style.boxSizing = 'border-box'
    card.style.textAlign = 'left'
    var title = document.createElement('div')
    title.className = 'dshwv-bubtitle'
    title.textContent = m.name + (m.builtin ? '（内置）' : '')
    card.appendChild(title)
    var st = document.createElement('div')
    st.className = 'dshwv-bubhint'
    st.style.margin = '4px 0 8px'
    if (m.error) st.textContent = '⚠ ' + m.error
    else if (m.balanceMode === 'events') st.textContent = '余额 —（该厂商无余额接口，按会话事件估算）· 今日已用 ' + apiFmtMoney(m.todayUsage, apiTodayCur(m))
    else st.textContent = '余额 ' + apiFmtMoney(m.balance, m.currency) + ' · 今日已用 ' + apiFmtMoney(m.todayUsage, apiTodayCur(m))
    card.appendChild(st)
    var ms = (usageSet && usageSet.models && usageSet.models[modelId]) || {}
    function rowOf(label, stateFn, onEdit, buttonLabel) {
      var r = document.createElement('div')
      r.className = 'dshwv-audiorow'
      var l = document.createElement('span')
      l.textContent = label
      l.style.flex = '0 0 auto'
      r.appendChild(l)
      var info = document.createElement('span')
      info.className = 'dshwv-usage-hint'
      info.style.flex = '1'
      info.style.textAlign = 'right'
      info.style.paddingRight = '6px'
      info.style.whiteSpace = 'nowrap'
      info.style.overflow = 'hidden'
      info.style.textOverflow = 'ellipsis'
      info.textContent = stateFn()
      r.appendChild(info)
      r.appendChild(apiBtn(buttonLabel || '编辑', 'dshwv-roleimport', onEdit))
      card.appendChild(r)
      return r
    }
    rowOf('余额预警', function () {
      var a = ms.alert
      if (!a || !a.on) return '已关闭'
      return '余额 ≤ ' + (a.below != null ? a.below : 50) + ' 时提醒'
    }, function () { openModelAlertBudget(modelId, 'alert', function () { openApiModelMenu(modelId) }) })
    rowOf('今日预算', function () {
      var b = ms.budget
      if (!b || !b.on) return '已关闭'
      return '今日已用 ≥ ' + (b.amount != null ? b.amount : 20) + ' 时提醒'
    }, function () { openModelAlertBudget(modelId, 'budget', function () { openApiModelMenu(modelId) }) })
    rowOf('额度（订阅/资源包）', function () { return apiQuotaSummary(modelId) },
      function () { openModelQuotaEditor(modelId) })
    // Codex 模式：开关（v748 从主菜单挪到这里）+ 只读展示本地会话统计（机器级）
    // 注意：开关按「是不是 Codex 模型」（apiCodexOf 非空）显示，**不能**按 .ok 判断 ——
    // 关掉时 .ok 就是 false，那样开关会自己消失、再也没法打开。
    if (apiCodexOf(modelId)) {
      var ct = document.createElement('div')
      ct.className = 'dshwv-audiorow'
      var ctl = document.createElement('span')
      ctl.textContent = 'Codex 本机统计'
      ctl.style.flex = '0 0 auto'
      ct.appendChild(ctl)
      var cth = document.createElement('span')
      cth.className = 'dshwv-usage-hint'
      cth.style.flex = '1'
      cth.style.textAlign = 'right'
      cth.style.paddingRight = '6px'
      cth.textContent = '读取 ~/.codex/sessions'
      ct.appendChild(cth)
      ct.appendChild(codexStatsCheckbox())
      card.appendChild(ct)
      // 用量行：关闭/出错时同样显示（文字就是原因），开着时显示今日/近7天/累计
      var cr = document.createElement('div')
      cr.className = 'dshwv-audiorow'
      var cl = document.createElement('span')
      cl.textContent = 'Codex 用量'
      cl.style.flex = '0 0 auto'
      cr.appendChild(cl)
      var ci = document.createElement('span')
      ci.className = 'dshwv-usage-hint'
      ci.style.flex = '1'
      ci.style.textAlign = 'right'
      ci.style.paddingRight = '6px'
      ci.style.whiteSpace = 'nowrap'
      ci.style.overflow = 'hidden'
      ci.style.textOverflow = 'ellipsis'
      ci.textContent = apiCodexDetailText(modelId)
      ci.title = ci.textContent
      cr.appendChild(ci)
      card.appendChild(cr)
      // 记下来：开关切完拿到服务端确认后，原地刷新这一行（不用关窗重开）
      codexUsageTextEl = ci
      apiModelMenuId = modelId
    }
    // 厂商订阅额度（kind='quota'）：只读展示，来自厂商接口
    if (apiPlanSupport(modelId)) {
      var pr = document.createElement('div')
      pr.className = 'dshwv-audiorow'
      var pl = document.createElement('span')
      pl.textContent = '厂商额度'
      pl.style.flex = '0 0 auto'
      pr.appendChild(pl)
      var pi = document.createElement('span')
      pi.className = 'dshwv-usage-hint'
      pi.style.flex = '1'
      pi.style.textAlign = 'right'
      pi.style.paddingRight = '6px'
      pi.style.whiteSpace = 'nowrap'
      pi.style.overflow = 'hidden'
      pi.style.textOverflow = 'ellipsis'
      pi.textContent = apiPlanSummary(modelId)
      pi.title = pi.textContent
      pr.appendChild(pi)
      card.appendChild(pr)
    }
    // 只读行（复用现有 .dshwv-audiorow / .dshwv-usage-hint，不引入新颜色与字体）
    function readonlyRow(label, text, hintHtml) {
      var r = document.createElement('div')
      r.className = 'dshwv-audiorow'
      var l = document.createElement('span')
      l.textContent = label
      l.style.flex = '0 0 auto'
      r.appendChild(l)
      // 需要「?」说明时，说明收进圆圈（悬停显示、点击钉住），与挂件其它说明圈同一套组件
      if (hintHtml) { try { r.appendChild(dshwvAskDot(hintHtml)) } catch (err) {} }
      var v = document.createElement('span')
      v.className = 'dshwv-usage-hint'
      v.style.flex = '1'
      v.style.textAlign = 'right'
      v.style.paddingRight = '6px'
      v.style.whiteSpace = 'nowrap'
      v.style.overflow = 'hidden'
      v.style.textOverflow = 'ellipsis'
      v.textContent = text
      v.title = text
      r.appendChild(v)
      card.appendChild(r)
    }
    // 单价（只读）：让用户确认当前生效价。内置 DeepSeek 始终走内置峰谷价，自定义单价对它不生效。
    var pc = (m && m.price) || null
    var pSet = !!(pc && (pc.hit != null || pc.miss != null || pc.out != null))
    var pTxt = '未设置（沿用内置价目表）'
    var pCur = ''
    if (pSet) {
      var fmtP = function (v) { return v == null ? '-' : String(v) }
      pCur = String(pc.cur || 'CNY').toUpperCase()
      pTxt = fmtP(pc.hit) + ' / ' + fmtP(pc.miss) + ' / ' + fmtP(pc.out) + ' · ' + pCur
      if (pCur === 'USD') pTxt += '（汇率 ' + (Number(pc.rate) > 0 ? pc.rate : '未填') + '）'
      if (m.builtin && m.provider === 'deepseek') pTxt += ' ← 使用该供应商内置峰谷价'
    } else if (m.builtin && m.provider === 'deepseek') {
      // 内置 DeepSeek 的自定义单价在 host 被显式跳过（priceFor 不读内置模型），所以这里说清"不能改"
      pTxt = '内置峰谷价（内置模型不支持自定义单价）'
    } else {
      // 这一行是只读展示；真正填写的地方在「密钥 / 接口」面板
      pTxt = '未设置（沿用内置价目表）→ 在「密钥 / 接口」里填写'
    }
    readonlyRow('单价', pTxt)
    // 「已观测消费」= DeepSeek 账户口径（余额观测），只在内置项的设置里显示，与下面的「余额校正」配套。
    if (apiCanAdjustBalance(m)) {
      var acc = m.accounting || null
      var accAmt = (acc && typeof acc.amount === 'number') ? acc.amount : m.todayUsage
      var accInfo = (acc && acc.firstObservedAt)
        ? '统计起点（北京）：' + accountingTime(acc.firstObservedAt) + '。起点前的消费未计入；该账户的观测包含同一个 key 在别处的消费。'
        : '尚无余额观测：先配置当前供应商 API 并成功刷新一次余额。'
      readonlyRow('已观测消费', ((acc && acc.label) || m.usageLabel || '已观测消费') + ' ' +
        (isFinite(Number(accAmt)) ? apiFmtMoney(accAmt, apiTodayCur(m)) : '--'), accInfo)
      // 需要用户动手的提示仍然直接显示（不藏进「?」里）
      if (acc && acc.needsReview) {
        var accWarn = document.createElement('div')
        accWarn.className = 'dshwv-usage-hint'
        accWarn.style.cssText = 'line-height:1.65;white-space:normal;margin:2px 0 4px'
        accWarn.textContent = '检测到余额增加，请用下面的「余额校正」核对本区间累计到账。'
        card.appendChild(accWarn)
      }
    }
    // 「充值 / 余额校正」入口：只给固定的 当前供应商，放在「单价」下方、沿用同一行布局与按钮样式。
    // 其它厂商、手动新增的 DeepSeek、仅改名为「当前供应商」的模型都不会走到这里。
    if (apiCanAdjustBalance(m)) {
      var adjustmentRow = rowOf('余额校正', function () {
        return (m.accounting && m.accounting.label) || '充值与余额调整'
      }, function () { openBalanceAdjustment(modelId) }, '校正')
      adjustmentRow.lastElementChild.setAttribute('data-action', 'balance-adjustment')
    }
    // 币种不一致提示：今日已用按「自带币种」显示（会话事件为 CNY）。若与模型币种不同且没填汇率，
    // 今日预算提醒会被跳过（见 A 方案），这里给出可见的补救提示。
    var tuc = String((m && m.todayUsageCurrency) || '').toUpperCase()
    var mc = String((m && m.currency) || '').toUpperCase()
    var rateOk = !!(pc && Number(pc.rate) > 0)
    if (tuc && mc && tuc !== mc && !rateOk) {
      readonlyRow('⚠ 币种不一致', '请在「密钥 / 接口」里填写汇率，否则今日预算提醒会被跳过')
    }
    var btns = document.createElement('div')
    btns.className = 'dshwv-bubbtns'
    btns.appendChild(apiBtn('密钥 / 接口', 'dshwv-bubbtn dshwv-bubbtn-no', function () { openApiModelPanel(modelId) }))
    btns.appendChild(apiBtn('关闭', 'dshwv-bubbtn dshwv-bubbtn-ok', closeApiModelPanel))
    card.appendChild(btns)
    mask.appendChild(card)
    mask.addEventListener('click', function (e) { if (e.target === mask) closeApiModelPanel() })
    dshwBodyAppend(mask)
    apiModelMaskEl = mask
  } catch (err) {}
}
// 滚动位置保持：重绘期间外层容器与内部滚动区都不能跳回顶部
function usageScrollSnapshot(root) {
  var arr = []
  var node = root
  while (node && node !== document.body) {
    if (node.scrollTop) arr.push([node, node.scrollTop])
    node = node.parentElement
  }
  var boxes = root && root.querySelectorAll ? root.querySelectorAll('.dshwv-usage-scroll') : []
  for (var i = 0; i < boxes.length; i++) arr.push([boxes[i], boxes[i].scrollTop])
  return arr
}
function usageScrollRestore(arr) {
  if (!arr) return
  for (var i = 0; i < arr.length; i++) { try { arr[i][0].scrollTop = arr[i][1] } catch (err) {} }
}
// 重建整个子界面（含静态区）但保持滚动位置：刷新按钮 / 首次加载完成时用它
function rebuildUsageSubShell() {
  try { usagePendingScroll = usageScrollSnapshot(usagePanel) } catch (err) {}
  try { buildUsageSubShell() } catch (err) {}
  try { refreshUsageMain() } catch (err) {}
}
function refreshUsageMain() {
  if (!usageMainEl) return
  // 优先用「重建前」记下的滚动位置，其次记录当前
  var snap = usagePendingScroll || usageScrollSnapshot(usageMainEl)
  usagePendingScroll = null
  // 注意：不清空已有内容。清空会让内容高度瞬间归零，外层滚动被夹回顶部
  //（用户下滑看近七天用量时会看到闪烁并跳回最上面）。首次尚无内容时才显示「加载中…」。
  if (!usageMainEl.firstChild) {
    var body = document.createElement('div')
    body.textContent = '加载中…'
    usageMainEl.appendChild(body)
  }
  fetch(USAGE_REC_URL, { cache: 'no-store' })
    .then(function (r) { return r.json() })
    .then(function (d) {
      if (d && d.ok && d.settings) usageSet = d.settings
      // 面板打开时顺带刷新自定义模型（余额/今日已用/额度）；force 才会真的重新请求
      try { loadApiModels(null, true) } catch (err) {}
      fillUsagePanel(d)
      usageScrollRestore(snap)
      // 双保险：DOM 更新后再恢复一次（内容二次布局可能再次夹住滚动）
      try { requestAnimationFrame(function () { usageScrollRestore(snap) }) } catch (err) {}
      // 用量面板刷新只更新泡泡状态数据;不在泡泡正显示时整泡重绘,
      // 数值由“泡泡消失→下一次显示”的渲染自然采用最新 state
      if (d && d.ok && d.today && isFinite(Number(d.today.total))) {
        var recTotal = Number(d.today.total)
        state.todayUsage = recTotal
        state.todayUsageCurrency = d.today.currency || 'CNY'
        state.usageLabel = d.today.label || '本地估算'
      }
    })
    .catch(function () { if (usageMainEl && !usageMainEl.firstChild) usageMainEl.textContent = '记录加载失败' })
}
function uSectionTitle(leftTxt, rightTxt) {
  var h = document.createElement('div')
  h.className = 'dshwv-usage-sec'
  var l = document.createElement('span')
  l.textContent = leftTxt
  h.appendChild(l)
  var r = document.createElement('span')
  r.className = 'dshwv-usage-total'
  r.textContent = rightTxt
  h.appendChild(r)
  return h
}
function accountingTime(at) {
  try {
    return new Date(at).toLocaleString('zh-CN', {
      timeZone: 'Asia/Shanghai', hour12: false, year: 'numeric', month: '2-digit',
      day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit'
    })
  } catch (err) { return String(at || '') }
}
var accountingMask = null
function openBalanceAdjustment(modelId) {
  // 只有固定的 当前供应商能打开：入口由宿主下发的 canAdjustBalance 控制，这里再挡一次
  if (!apiCanAdjustBalance(apiModelById(modelId)) || accountingMask) return
  var adjustmentUrl = '/dsh-whale/balance-adjustments.json?modelId=' + encodeURIComponent(modelId)
  closeApiModelPanel()
  // 与「额度」「余额预警 / 今日预算」等窗口共用同一套骨架与样式：
  // dshwv-usage-mask + dshwv-usage-card + dshwv-bubtitle/bubhint + apiPanelRow/apiTextInput + dshwv-bubbtns
  var mask = document.createElement('div')
  mask.className = 'dshwv-usage-mask'
  mask.style.zIndex = '30000'
  var card = document.createElement('form')
  card.className = 'dshwv-usage-card'
  card.style.width = 'min(430px,94vw)'
  card.style.padding = '14px 16px'
  card.style.boxSizing = 'border-box'
  card.style.textAlign = 'left'
  card.setAttribute('role', 'dialog')
  card.setAttribute('aria-modal', 'true')
  card.setAttribute('aria-label', '当前供应商余额校正')
  // 内容可能比 82vh 高：中间这段自己滚，按钮行固定在底部（与 .dshwv-reswrap 同一做法）
  var body = document.createElement('div')
  body.style.cssText = 'min-height:0;overflow-y:auto;flex:1 1 auto'
  var title = document.createElement('div')
  title.className = 'dshwv-bubtitle'
  title.textContent = '余额校正 · 当前供应商'
  body.appendChild(title)
  var introduction = document.createElement('div')
  introduction.className = 'dshwv-bubhint'
  introduction.style.cssText = 'margin:0 0 8px;white-space:normal;line-height:1.6;text-align:left'
  introduction.textContent = '核对统计区间内的全部到账后，重新计算本地消费。此操作不会充值，也不会改变 当前供应商账户余额。'
  body.appendChild(introduction)
  var dates = apiSelectEl([], '')
  body.appendChild(apiPanelRow('日期', dates))
  var interval = document.createElement('div')
  interval.className = 'dshwv-bubhint'
  interval.style.cssText = 'margin:0 0 8px;white-space:pre-line;line-height:1.6;text-align:left'
  body.appendChild(interval)
  var credits = apiTextInput('', '未到账请填 0')
  credits.inputMode = 'decimal'
  credits.required = true
  credits.autocomplete = 'off'
  body.appendChild(apiPanelRow('累计到账', credits))
  var creditsHint = document.createElement('div')
  creditsHint.className = 'dshwv-bubhint'
  creditsHint.style.cssText = 'margin:0 0 8px;white-space:normal;line-height:1.6;text-align:left'
  body.appendChild(creditsHint)
  var debits = apiTextInput('0', '没有请填 0')
  debits.inputMode = 'decimal'
  debits.autocomplete = 'off'
  body.appendChild(apiPanelRow('非调用扣减', debits))
  var debitsHint = document.createElement('div')
  debitsHint.className = 'dshwv-bubhint'
  debitsHint.style.cssText = 'margin:0 0 8px;white-space:normal;line-height:1.6;text-align:left'
  body.appendChild(debitsHint)
  var preview = document.createElement('div')
  preview.className = 'dshwv-bubhint'
  preview.style.cssText = 'margin:0 0 8px;white-space:normal;line-height:1.6;text-align:left;font-weight:700;color:#203170'
  preview.setAttribute('aria-live', 'polite')
  body.appendChild(preview)
  var confirmRow = document.createElement('label')
  confirmRow.style.cssText = 'display:flex;gap:6px;align-items:flex-start;flex:1;min-width:0;font-size:12px;color:#203170;cursor:pointer;line-height:1.5'
  var confirm = document.createElement('input')
  confirm.type = 'checkbox'
  confirm.style.marginTop = '1px'
  confirmRow.appendChild(confirm)
  confirmRow.appendChild(document.createTextNode('我已核对本统计区间的全部到账和非调用扣减'))
  body.appendChild(apiPanelRow('确认', confirmRow))
  var status = document.createElement('div')
  status.className = 'dshwv-bubhint'
  status.setAttribute('role', 'status')
  status.style.cssText = 'margin:0 0 6px;white-space:normal;line-height:1.6;text-align:left;color:#b33333'
  status.textContent = '正在刷新余额…'
  body.appendChild(status)
  card.appendChild(body)
  var busy = false
  function close() {
    if (busy) return
    document.removeEventListener('keydown', keyHandler)
    // v743：主动移除要走 dshwBodyDetach —— 否则它已被登记，DOM 守护会把这个刚关掉的窗口"复活"
    dshwBodyDetach(mask)
    accountingMask = null
    // 关掉校正窗口后回到同一个设置菜单，并把焦点放回「校正」按钮
    openApiModelMenu(modelId)
    try {
      var returnButton = apiModelMaskEl && apiModelMaskEl.querySelector('[data-action="balance-adjustment"]')
      if (returnButton) returnButton.focus()
    } catch (err) {}
  }
  function keyHandler(e) {
    if (e.key === 'Escape') { e.preventDefault(); close() }
    if (e.key === 'Tab') {
      var focusable = Array.prototype.filter.call(card.querySelectorAll('button,input,select'), function (el) {
        return !el.disabled && el.style.display !== 'none'
      })
      var first = focusable[0], last = focusable[focusable.length - 1]
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus() }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus() }
    }
  }
  // 按钮行与其它窗口完全一致：.dshwv-bubbtns + apiBtn 的标准按钮样式（不再自绘尺寸/配色）
  var actions = document.createElement('div')
  actions.className = 'dshwv-bubbtns'
  var reload = apiBtn('重新读取', 'dshwv-bubbtn dshwv-bubbtn-no', function () { load() })
  var reset = apiBtn('撤销该日校正', 'dshwv-bubbtn dshwv-bubbtn-no', function () { submit('reset') })
  var save = apiBtn('保存校正', 'dshwv-bubbtn dshwv-bubbtn-ok', function () { submit('save') })
  actions.appendChild(reload)
  actions.appendChild(reset)
  actions.appendChild(apiBtn('取消', 'dshwv-bubbtn dshwv-bubbtn-no', function () { close() }))
  actions.appendChild(save)
  card.appendChild(actions)
  mask.appendChild(card)
  mask.addEventListener('click', function (e) { if (e.target === mask) close() })
  dshwBodyAppend(mask)
  accountingMask = mask
  document.addEventListener('keydown', keyHandler)
  var rows = []
  var selected = null
  function renderSelection() {
    selected = rows.find(function (row) { return row.day === dates.value }) || null
    save.disabled = !selected || busy
    reset.disabled = !selected || !selected.correctedAt || busy
    if (!selected) {
      interval.textContent = '尚无可校正的余额观测。请先配置当前供应商 API 并成功刷新余额。'
      creditsHint.textContent = ''
      debitsHint.textContent = ''
      return
    }
    interval.textContent = '统计起点：' + accountingTime(selected.firstObservedAt) +
      '\n最近观测：' + accountingTime(selected.lastObservedAt) +
      '\n起点余额 ' + usageMoney(selected.openingBalance, selected.currency) +
      ' → 当前余额 ' + usageMoney(selected.currentBalance, selected.currency) +
      '\n当前：' + selected.label + ' ' + usageMoney(selected.amount, selected.currency)
    creditsHint.textContent = '本统计区间累计到账金额（' + selected.currency + '，未到账请填 0）：包括充值、赠金等；多次到账请填合计，不要只填最后一笔。'
    debitsHint.textContent = '非调用造成的余额减少（' + selected.currency + '，没有请填 0）：到期赠金、余额退回等。仅填写统计起点之后的金额；保存会替换之前的校正值。'
    credits.value = selected.credits == null ? '' : String(selected.credits)
    debits.value = selected.otherDebits == null ? '0' : String(selected.otherDebits)
    confirm.checked = false
    updatePreview()
  }
  function updatePreview() {
    if (!selected || credits.value.trim() === '' || !isFinite(Number(credits.value)) || !isFinite(Number(debits.value))) {
      preview.textContent = '填写完整金额后显示校正预览'
      return
    }
    var amount = selected.openingBalance + Number(credits.value) - Number(debits.value || 0) - selected.currentBalance
    preview.textContent = amount < -0.000000005 ? '校正后为负数，请核对金额和统计区间' : '校正后消费：' + usageMoney(Math.max(0, amount), selected.currency)
  }
  credits.addEventListener('input', updatePreview)
  debits.addEventListener('input', updatePreview)
  dates.addEventListener('change', renderSelection)
  function load() {
    if (busy) return
    busy = true
    save.disabled = true
    reset.disabled = true
    reload.disabled = true
    status.textContent = '正在刷新余额…'
    fetch(adjustmentUrl, { cache: 'no-store' })
      .then(function (r) { return r.json() })
      .then(function (data) {
        if (!data.ok) throw new Error(data.error || '读取失败')
        rows = data.days || []
        dates.innerHTML = ''
        rows.forEach(function (row) {
          var option = document.createElement('option')
          option.value = row.day
          option.textContent = row.day + ' · ' + row.label
          dates.appendChild(option)
        })
        status.textContent = data.error ? '余额暂未刷新：' + data.error : ''
      })
      .catch(function (err) { status.textContent = err.message || '读取失败' })
      .finally(function () { busy = false; reload.disabled = false; renderSelection(); dates.focus() })
  }
  function submit(action) {
    if (busy || !selected) return
    if (action !== 'reset' && !confirm.checked) { status.textContent = '请先勾选确认，核对本统计区间的全部余额调整。'; return }
    busy = true
    save.disabled = true
    reset.disabled = true
    reload.disabled = true
    status.textContent = '正在保存…'
    fetch(adjustmentUrl, {
      method: 'PUT', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        modelId: modelId, day: selected.day, revision: selected.revision, action: action,
        credits: credits.value.trim(), otherDebits: debits.value.trim() || '0', confirmed: confirm.checked
      })
    })
      .then(function (r) { return r.json() })
      .then(function (data) {
        if (!data.ok) throw new Error(data.error || '保存失败')
        // 保存后立刻用返回的摘要刷新该模型条目：关窗回到设置菜单时显示的就是新金额
        var model = apiModelById(modelId)
        var currentDay = new Date(Date.now() + 8 * 3600000).toISOString().slice(0, 10)
        if (apiCanAdjustBalance(model) && data.summary && data.summary.day === currentDay) {
          model.balance = data.summary.currentBalance
          model.currency = data.summary.currency
          model.todayUsage = data.summary.amount
          model.todayUsageCurrency = data.summary.currency
          model.usageSource = data.summary.source
          model.usageLabel = data.summary.label
          model.accounting = data.summary
          model.error = null
        }
        busy = false
        close()
        refresh(true)
        loadApiModels(function () {
          if (usagePanelOpen) rebuildUsageSubShell()
        }, true)
      })
      .catch(function (err) { status.textContent = err.message || '保存失败' })
      .finally(function () { busy = false; save.disabled = !selected; reset.disabled = !selected || !selected.correctedAt; reload.disabled = false })
  }
  card.addEventListener('submit', function (e) { e.preventDefault(); submit('save') })
  load()
}

function fillUsagePanel(d) {
  var hostEl = usageMainEl || usagePanel
  hostEl.innerHTML = ''
  var wrap = document.createElement('div')
  wrap.className = 'dshwv-usagebody'
  if (!d || !d.ok) {
    wrap.textContent = '记录加载失败'
    hostEl.appendChild(wrap)
    return
  }
  var today = d.today || {}
  var todayModels = today.models || []
  var hasEvToday = todayModels.length > 0
  // ① 本机模型费用（所有模型的本地会话估算）
  // 「已观测消费」是 DeepSeek 账户口径（余额观测），已移到 小鲸鱼记账 → 当前供应商→ 设置 里，
  // 与「余额校正」放在一起，避免在这里被误读成"全模型合计"。
  var modelTitle = uSectionTitle('本机模型费用', usageMoney(today.modelTotal, 'CNY'))
  // 标题与下方模型列表之间不再画分隔线（列表本身有边框，够了）
  modelTitle.style.borderBottom = 'none'
  modelTitle.title = '按本机 Codex 会话计算，不按账户余额比例缩放；两者覆盖范围不同。'
  wrap.appendChild(modelTitle)
  var todayBox = document.createElement('div')
  todayBox.className = 'dshwv-usage-scroll dshwv-usage-today'
  if (hasEvToday) {
    todayModels.forEach(function (row) {
      var r = document.createElement('div')
      r.className = 'dshwv-usage-row'
      var n = mkScrollCell(usageModelLabel(row.model), 'dshwv-usage-model', { flex: '0 0 70px', boxSizing: 'border-box' })
      n.title = String(row.model || '未知')
      r.appendChild(n)
      var c = document.createElement('span')
      c.textContent = usageMoney(row.cost)
      r.appendChild(c)
      todayBox.appendChild(r)
    })
  } else if ((today.total || 0) > 0) {
    var noM = document.createElement('div')
    noM.className = 'dshwv-usage-hint'
    noM.textContent = '暂无本机会话费用明细；账户余额观测仍独立记账。'
    todayBox.appendChild(noM)
  } else {
    var empty = document.createElement('div')
    empty.className = 'dshwv-usage-hint'
    empty.textContent = '今日暂无消费记录'
    todayBox.appendChild(empty)
  }
  // 列表高度不限制（随内容增长），下方「更多消费记录…」位置随内容移动
  wrap.appendChild(todayBox)
  // ② 近 7 天（列表不限制高度）；标题上方加分隔线、自身不带下分隔线
  var sep7 = document.createElement('div')
  sep7.style.borderTop = '1px solid rgba(32,49,112,.15)'
  sep7.style.margin = '6px 0'
  wrap.appendChild(sep7)
  var t7 = uSectionTitle('近7天使用记录', usageMoney(d.total7, d.total7Currency))
  t7.style.borderBottom = 'none'
  wrap.appendChild(t7)
  var daysBox = document.createElement('div')
  daysBox.className = 'dshwv-usage-scroll'
  ;(d.days7 || []).forEach(function (row) {
    var r = document.createElement('div')
    r.className = 'dshwv-usage-row'
    var n = document.createElement('span')
    n.textContent = usageDayLabel(row.date)
    n.title = row.label || ''
    r.appendChild(n)
    var c = document.createElement('span')
    c.textContent = usageMoney(row.total, row.currency)
    r.appendChild(c)
    daysBox.appendChild(r)
  })
  wrap.appendChild(daysBox)
  // ③ 更多
  var more = document.createElement('button')
  more.type = 'button'
  more.className = 'dshwv-usage-more'
  more.textContent = '更多消费记录…'
  more.title = '打开窗口查看全部有记录的消费'
  more.addEventListener('click', function (e) { e.stopPropagation(); openUsageRecordsWindow() })
  wrap.appendChild(more)
  ;(usageMainEl || usagePanel).appendChild(wrap)
}
// “更多消费记录”窗口
var usageMoreMask = document.createElement('div')
usageMoreMask.className = 'dshwv-usage-mask'
usageMoreMask.style.display = 'none'
var usageMoreCard = document.createElement('div')
usageMoreCard.className = 'dshwv-usage-card'
usageMoreMask.appendChild(usageMoreCard)
usageMoreMask.addEventListener('click', function (e) { if (e.target === usageMoreMask) closeUsageRecordsWindow() })
dshwBodyAppend(usageMoreMask)
function openUsageRecordsWindow() {
  usageMoreCard.innerHTML = '<div style="padding:10px;color:#203170">加载中…</div>'
  usageMoreMask.style.display = 'flex'
  fetch(USAGE_REC_URL, { cache: 'no-store' })
    .then(function (r) { return r.json() })
    .then(function (d) { fillUsageRecordsWindow(d) })
    .catch(function () { usageMoreCard.innerHTML = '<div style="padding:10px;color:#203170">加载失败</div>' })
}
function closeUsageRecordsWindow() { usageMoreMask.style.display = 'none' }
// ===== v767：通用「入口行 + 可折叠体」组件（提示与音效设置面板 / 资源管理窗口共用）=====
// 入口行常驻：可选 [✓] 开关 + 区名 + 当前值摘要 + ▸/▾；点整行才展开它下面的折叠体，可同时展开多个。
// 折叠用 max-height 过渡（与「自定义提示」窗口里的 advBox 同一套路），展开动画结束后置 'none' —— 之后
// 窗口缩放换行也不会被截断；flexShrink:0 是坑 14 的规矩（flex 容器里的折叠框必须给）。
// ⚠️ 折叠体里的控件**照旧全部创建**（只是藏起来）：懒创建会让「恢复默认」这类逐个回填的逻辑静默失效。
// instant = true 时不做过渡、直接到位（资源窗口重渲染时用它保持用户已展开的区，别闪动画）。
function collapseSet(body, open, instant) {
  try {
    var arrow = body.__dshwArrow
    if (open) {
      body.setAttribute('data-open', '1')
      body.style.maxHeight = instant ? 'none' : (body.scrollHeight + 'px')
      body.style.opacity = '1'
      if (arrow) arrow.textContent = '▾'
      if (!instant) {
        setTimeout(function () { try { if (body.getAttribute('data-open') === '1') body.style.maxHeight = 'none' } catch (err) {} }, 260)
      }
    } else {
      body.setAttribute('data-open', '0')
      body.style.maxHeight = body.scrollHeight + 'px' // 先给起始值，才收得回去
      if (!instant) { try { void body.offsetHeight } catch (err) {} }
      body.style.maxHeight = '0px'
      body.style.opacity = '0'
      if (arrow) arrow.textContent = '▸'
    }
  } catch (err) {}
}
// 建一个入口行 + 它下面的折叠体，都挂进 host；返回 { row, body, summary, arrow, chk }
// opts: { checked, onToggle, first, summary, titleTail, onRowClick(willOpen) }
function dshwvFoldEntry(host, labelText, opts) {
  opts = opts || {}
  var rowEl = document.createElement('div')
  rowEl.className = 'dshwv-menu-row' // 与主菜单/设置面板入口行同款排版
  rowEl.style.cursor = 'pointer'
  rowEl.style.margin = opts.first ? '12px 0 2px' : '10px 0 2px'
  rowEl.style.padding = '6px 6px'
  rowEl.style.borderRadius = '6px'
  rowEl.style.background = 'rgba(32,49,112,.05)' // 常驻淡底：与折叠体里的内容行区分开
  rowEl.title = '点这一行展开 / 收起「' + labelText + '」' + String(opts.titleTail || '')
  var ck = null
  if (typeof opts.onToggle === 'function') {
    ck = document.createElement('input')
    ck.type = 'checkbox'
    ck.className = 'dshwv-check'
    ck.checked = !!opts.checked
    ck.setAttribute('aria-label', labelText)
    ck.title = labelText + '开关（只管开不开；下面的设置项不受影响）'
    // 点开关**不**展开/收起（否则想关掉该事件时会顺手把区拉开）
    ck.addEventListener('click', function (e) { try { e.stopPropagation() } catch (err) {} })
    ck.addEventListener('change', function () { try { opts.onToggle(!!ck.checked) } catch (err) {} })
    rowEl.appendChild(ck)
  }
  var tx = document.createElement('span')
  tx.textContent = labelText
  rowEl.appendChild(tx)
  var sp = document.createElement('span') // 撑开，把摘要与箭头推到右侧
  sp.style.flex = '1'
  rowEl.appendChild(sp)
  var sum = document.createElement('span')
  sum.style.fontSize = '11px'
  sum.style.color = '#203170'
  sum.style.opacity = '.7'
  sum.style.whiteSpace = 'nowrap'
  sum.style.overflow = 'hidden'
  sum.style.textOverflow = 'ellipsis'
  sum.style.maxWidth = '46%'
  sum.textContent = String(opts.summary == null ? '' : opts.summary)
  rowEl.appendChild(sum)
  var arrow = document.createElement('span')
  arrow.textContent = '▸'
  arrow.style.opacity = '.6'
  arrow.style.marginLeft = '2px'
  rowEl.appendChild(arrow)
  host.appendChild(rowEl)
  var body = document.createElement('div')
  body.style.overflow = 'hidden'
  body.style.maxHeight = '0px'
  body.style.opacity = '0'
  body.style.transition = 'max-height .24s ease, opacity .18s ease'
  body.style.flexShrink = '0'
  body.style.paddingLeft = '10px' // 轻微缩进：看得出下面这些行属于上面那个入口
  body.setAttribute('data-open', '0')
  body.__dshwArrow = arrow
  host.appendChild(body)
  rowEl.addEventListener('click', function () {
    var willOpen = body.getAttribute('data-open') !== '1'
    // onRowClick 先跑：调用方要在"切换"之前收掉自己的浮层 / 停掉试听（见坑 53）
    try { if (typeof opts.onRowClick === 'function') opts.onRowClick(willOpen) } catch (err) {}
    collapseSet(body, willOpen)
  })
  // 悬停反馈：本项目约定「只用行内样式、不动全局 CSS」，所以用两个监听器代替 :hover
  rowEl.addEventListener('mouseenter', function () { try { rowEl.style.background = 'rgba(32,49,112,.09)' } catch (err) {} })
  rowEl.addEventListener('mouseleave', function () { try { rowEl.style.background = 'rgba(32,49,112,.05)' } catch (err) {} })
  return { row: rowEl, body: body, summary: sum, arrow: arrow, chk: ck }
}
// —— 资源管理窗口:集中查看/删除已导入插件的图片与音频 ——
var resMaskEl = null
var resCardEl = null
function resMaskOpen() {
  try {
    if (!resMaskEl) {
      resMaskEl = document.createElement('div')
      resMaskEl.className = 'dshwv-resmask'
      resMaskEl.style.display = 'none'
      resCardEl = document.createElement('div')
      resCardEl.className = 'dshwv-usage-card dshwv-rescard'
      resMaskEl.appendChild(resCardEl)
      resMaskEl.addEventListener('click', function (e) { if (e.target === resMaskEl) resManagerClose() })
      dshwBodyAppend(resMaskEl)
    }
    resManagerRender()
    resMaskEl.style.display = 'flex'
  } catch (err) {}
}
function resManagerClose() {
  try { if (resMaskEl) resMaskEl.style.display = 'none' } catch (err) {}
  // 窗口消失时结束正在预览的音频,避免后台继续响
  try { resStopPreview() } catch (err) {}
}
function openResManager() { resMaskOpen() }
function resMkTag(text, built) {
  var t = document.createElement('span')
  t.className = 'dshwv-restag' + (built ? ' dshwv-restag-built' : '')
  t.textContent = text
  return t
}
function resImgRow(imgUrl, name, meta, rightEls) {
  var img = document.createElement('img')
  img.className = 'dshwv-resthum'
  img.src = imgUrl
  img.alt = ''
  var main = document.createElement('div')
  main.className = 'dshwv-resmain'
  main.style.minWidth = '0'
  var nm = document.createElement('div')
  nm.className = 'dshwv-resnm'
  nm.textContent = name
  nm.title = name // 窗口收窄后长名字会以 … 截断,悬浮可看全名
  main.appendChild(nm)
  if (meta) {
    var mt = document.createElement('div')
    mt.className = 'dshwv-resmeta'
    mt.textContent = meta
    mt.title = meta
    main.appendChild(mt)
  }
  var row = document.createElement('div')
  row.className = 'dshwv-resrow'
  var left = document.createElement('div')
  left.className = 'dshwv-resmain'
  left.style.display = 'flex'
  left.style.alignItems = 'center'
  left.style.gap = '6px'
  left.appendChild(img)
  left.appendChild(main)
  row.appendChild(left)
  for (var i = 0; i < (rightEls || []).length; i++) row.appendChild(rightEls[i])
  return row
}
function resIconRow(iconText, name, meta, rightEls) {
  var icon = document.createElement('div')
  icon.className = 'dshwv-resicon'
  icon.textContent = iconText
  var main = document.createElement('div')
  main.className = 'dshwv-resmain'
  main.style.minWidth = '0'
  var nm = document.createElement('div')
  nm.className = 'dshwv-resnm'
  nm.textContent = name
  nm.title = name // 窗口收窄后长名字会以 … 截断,悬浮可看全名
  main.appendChild(nm)
  if (meta) {
    var mt = document.createElement('div')
    mt.className = 'dshwv-resmeta'
    mt.textContent = meta
    mt.title = meta
    main.appendChild(mt)
  }
  var row = document.createElement('div')
  row.className = 'dshwv-resrow'
  var left = document.createElement('div')
  left.className = 'dshwv-resmain'
  left.style.display = 'flex'
  left.style.alignItems = 'center'
  left.style.gap = '6px'
  left.appendChild(icon)
  left.appendChild(main)
  row.appendChild(left)
  for (var i = 0; i < (rightEls || []).length; i++) row.appendChild(rightEls[i])
  return row
}
function resMkDel(label, disabled, fn) {
  var b = document.createElement('button')
  b.type = 'button'
  b.className = 'dshwv-resdel'
  b.textContent = label
  b.disabled = !!disabled
  if (!disabled) b.addEventListener('click', function (e) { e.stopPropagation(); fn() })
  return b
}
// 资源窗口通用按钮：class 指定样式，disabled 时不可点
function resMkBtn(cls, label, disabled, fn) {
  var b = document.createElement('button')
  b.type = 'button'
  b.className = cls
  b.textContent = label
  b.disabled = !!disabled
  if (!disabled) b.addEventListener('click', function (e) { e.stopPropagation(); fn() })
  return b
}
var resAudEl = null // 资源窗口音频片段试听元素(复用,避免并发)
function resStopPreview() {
  try { if (resAudEl) { resAudEl.pause(); resAudEl = null } } catch (err) {}
}
function resPlayFragment(fid) {
  try {
    if (!fid) return
    if (resAudEl) { try { resAudEl.pause() } catch (err) {} resAudEl = null }
    var a = dshwvSound('/dsh-whale/audio-fragment.wav?id=' + encodeURIComponent(fid))
    // v778：资源窗口的片段试听也走音量解析器（它是"这一条片段本身的试听"，与按压音量同源）
    try { a.volume = soundVolumeOf('press') } catch (err) {}
    a.onended = function () { resAudEl = null }
    resAudEl = a
    a.play().catch(function () { resAudEl = null })
  } catch (err) {}
}
function resImportAudioFragment() {
  try {
    // 复用音频裁剪弹窗：仅导入为片段库，不绑定任何按压/松开槽位
    audioCropTarget = '__library__'
    audioCropFileInput.click()
  } catch (err) {}
}
function resManagerRender() {
  try {
    var card = resCardEl
    card.innerHTML = ''
    var head = document.createElement('div')
    head.className = 'dshwv-reshead'
    var title = document.createElement('span')
    title.className = 'dshwv-restitle'
    title.textContent = '资源管理'
    var x = document.createElement('button')
    x.type = 'button'
    x.className = 'dshwv-resclose'
    x.textContent = '✕'
    x.title = '关闭'
    x.addEventListener('click', resManagerClose)
    head.appendChild(title)
    head.appendChild(x)
    card.appendChild(head)
    var wrap = document.createElement('div')
    wrap.className = 'dshwv-reswrap'
    card.appendChild(wrap)
    wrap.innerHTML = '<div style="padding:8px 6px;color:#203170">加载中…</div>'
    // 并发拉取三类资源(角色图/泡泡图/音频),归入“图片 / 音频”两组渲染
    var roles = []
    var bubbleImgs = []
    var audio = null
    var done = 0
    function fin() {
      done++
      if (done < 3) return
      resRenderData(wrap, roles, bubbleImgs, audio)
    }
    function fail() { fin() }
    try {
      fetch('/dsh-whale/roles.json', { cache: 'no-store' }).then(function (r) { return r.json() }).then(function (d) { if (d && d.ok && Array.isArray(d.roles)) roles = d.roles; fin() }).catch(fail)
    } catch (err) { fin() }
    try {
      fetch('/dsh-whale/bubble-imgs.json', { cache: 'no-store' }).then(function (r) { return r.json() }).then(function (d) { if (d && d.ok && Array.isArray(d.images)) bubbleImgs = d.images; fin() }).catch(fail)
    } catch (err) { fin() }
    try {
      fetch('/dsh-whale/audio.json', { cache: 'no-store' }).then(function (r) { return r.json() }).then(function (d) { audio = d; fin() }).catch(fail)
    } catch (err) { fin() }
  } catch (err) {}
}
function resDelRole(id) {
  var r = null
  for (var i = 0; i < roleList.length; i++) if (roleList[i].id === id) { r = roleList[i]; break }
  showConfirm('确定删除角色「' + (r ? r.name : id) + '」吗？\n若该角色正被使用,将自动回退默认小鲸鱼。', function () {
    try {
      fetch('/dsh-whale/role-delete.json', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: id }),
      })
        .then(function (res) { return res.json() })
        .then(function (d) {
          if (d && d.ok && Array.isArray(d.roles)) {
            roleList = d.roles
            renderRolePanel()
            if (currentRole && currentRole.id === id) applyRole('default', '小鲸鱼', IMG_URL)
            openResManager()
          }
        })
        .catch(function () {})
    } catch (err) {}
  })
}
function resDelBubbleImg(id) {
  var im = null
  for (var i = 0; i < bubbleImgList.length; i++) if (bubbleImgList[i].id === id) { im = bubbleImgList[i]; break }
  showConfirm('确定删除泡泡图「' + (im && im.name ? im.name : id) + '」吗？\n正在引用该图的泡泡行将无法显示。', function () {
    try {
      fetch('/dsh-whale/bubble-img-upload.json', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'delete', id: id }),
      })
        .then(function (r) { return r.json() })
        .then(function (d) {
          if (d && d.ok && Array.isArray(d.images)) {
            bubbleImgList = d.images
            openResManager()
          }
        })
        .catch(function () {})
    } catch (err) {}
  })
}
function resDelAudioGroup(id) {
  var g = null
  for (var i = 0; i < (audioGroups || []).length; i++) if (audioGroups[i].id === id) { g = audioGroups[i]; break }
  showConfirm('确定删除音效组「' + (g && g.name ? g.name : id) + '」吗？', function () {
    try {
      fetch('/dsh-whale/audio.json', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'delete-group', id: id }),
      })
        .then(function (r) { return r.json() })
        .then(function (d) {
          if (d && d.ok && Array.isArray(d.groups)) {
            audioGroups = d.groups
            renderAudioGroupPanel()
            refreshTaskEndAfterAudio()
            if (soundSet === id) setSoundSet('duck')
            openResManager()
          }
        })
        .catch(function () {})
    } catch (err) {}
  })
}
function resDelAudioFrag(id) {
  var f = null
  for (var i = 0; i < (audioFragments || []).length; i++) if (audioFragments[i].id === id) { f = audioFragments[i]; break }
  showConfirm('确定删除音频片段「' + (f && f.name ? f.name : id) + '」吗？\n引用该片段的音效组槽位会自动回退预设。', function () {
    try {
      fetch('/dsh-whale/audio.json', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'delete-fragment', id: id }),
      })
        .then(function (r) { return r.json() })
        .then(function (d) {
          if (d && d.ok && Array.isArray(d.fragments)) {
            audioFragments = d.fragments
            if (Array.isArray(d.groups)) audioGroups = d.groups
            renderAudioGroupPanel()
            refreshTaskEndAfterAudio()
            openResManager()
          }
        })
        .catch(function () {})
    } catch (err) {}
  })
}
// v767：资源窗口的两个分区（图片 / 音频）改成与「提示与音效设置」同一套折叠入口（`dshwvFoldEntry`）。
// 折叠态**跨重渲染保留** —— 删一张图 / 导一段音频后本窗口会整窗重渲染，不能让用户刚展开的区自己收回去。
var resFoldOpen = { img: false, audio: false }
function resFoldEntry(wrap, key, labelText, summaryText, first) {
  var e = dshwvFoldEntry(wrap, labelText, {
    first: first,
    summary: summaryText,
    titleTail: '的资源',
    onRowClick: function (willOpen) {
      resFoldOpen[key] = !!willOpen
      resStopPreview() // 收起/展开时停掉正在试听的那一段（与设置面板同一约定）
    },
  })
  if (resFoldOpen[key]) collapseSet(e.body, true, true) // 直接到位不播动画：重渲染时保持用户已展开的区
  return e
}
function resRenderData(wrap, roles, bubbleImgs, audio) {
  try {
    wrap.innerHTML = ''
    var groups = (audio && Array.isArray(audio.groups)) ? audio.groups : []
    var fragsAll = (audio && Array.isArray(audio.fragments)) ? audio.fragments : []
    var frags = fragsAll.filter(function (f) { return f && !f.preset })
    // —— 图片（入口行 + 折叠体；摘要 = 数量，不展开也知道有多少）——
    var imgEnt = resFoldEntry(wrap, 'img', '图片', '角色 ' + roles.length + ' · 泡泡图 ' + bubbleImgs.length, true)
    var anyImg = false
    // 角色图(默认角色只读展示)
    roles.forEach(function (r) {
      anyImg = true
      var isDefault = r.id === 'default'
      var tag = resMkTag(isDefault ? '默认角色' : '自定义角色', isDefault)
      imgEnt.body.appendChild(resImgRow(r.url, r.name || r.id, '', [tag, resMkDel('删除', isDefault, function () { resDelRole(r.id) })]))
    })
    bubbleImgs.forEach(function (im) {
      anyImg = true
      var tag = resMkTag(im.builtin ? '内置图' : '泡泡图', !!im.builtin)
      imgEnt.body.appendChild(resImgRow('/dsh-whale/bubble-img.png?id=' + encodeURIComponent(im.id), im.name || im.id, '', [tag, resMkDel('删除', !!im.builtin, function () { resDelBubbleImg(im.id) })]))
    })
    if (!anyImg) {
      var empty = document.createElement('div')
      empty.className = 'dshwv-resempty'
      empty.textContent = '暂无自定义图片(角色/泡泡图)'
      imgEnt.body.appendChild(empty)
    }
    // —— 音频（入口行 + 折叠体；「导入片段」放进折叠体第一行，免得入口行太挤）——
    var auEnt = resFoldEntry(wrap, 'audio', '音频', '音效组 ' + groups.length + ' · 片段 ' + frags.length)
    var impRow = document.createElement('div')
    impRow.className = 'dshwv-resrow'
    var catAuBtn = resMkBtn('dshwv-resimp', '导入片段', false, resImportAudioFragment)
    catAuBtn.title = '导入并裁剪一段音频到片段库(可被音效组引用)'
    impRow.appendChild(catAuBtn)
    auEnt.body.appendChild(impRow)
    var anyAu = false
    groups.forEach(function (g) {
      anyAu = true
      var preset = !!g.preset
      var tag = resMkTag(preset ? '预设组' : '自定义组', preset)
      var meta = ''
      if (!preset && g.press && g.release) meta = '按压:' + g.press + ' 松开:' + g.release
      // 音效组不提供播放(按整组点按发声,需与点按动作绑定),只展示/删除
      auEnt.body.appendChild(resIconRow(preset ? '🎧' : '🎵', g.name || g.id, meta, [tag, resMkDel('删除', preset, function () { resDelAudioGroup(g.id) })]))
    })
    frags.forEach(function (f) {
      anyAu = true
      var tag = resMkTag('音频片段', false)
      var play = resMkBtn('dshwv-resplay', '播放', false, function () { resPlayFragment(f.id) })
      play.title = '试听该音频片段'
      auEnt.body.appendChild(resIconRow('🎶', f.name || f.id, '', [tag, play, resMkDel('删除', false, function () { resDelAudioFrag(f.id) })]))
    })
    if (!anyAu) {
      var empty2 = document.createElement('div')
      empty2.className = 'dshwv-resempty'
      empty2.textContent = '暂无自定义音频(片段/音效组)'
      auEnt.body.appendChild(empty2)
    }
  } catch (err) {}
}
// —— 余额预警 / 今日预算(弹窗提示;内容可编辑) ——
var usageAlertBelowFired = false // 低于阈值已弹过;恢复高于后复位
var usageBudgetFiredKey = null // 当日预算已弹标记
function usageTodayKeyStr() {
  var d = new Date()
  var p = function (n) { return String(n).padStart(2, '0') }
  return d.getFullYear() + '-' + p(d.getMonth() + 1) + '-' + p(d.getDate())
}
// 提醒内容的「出厂默认」= 冻结的当前生效内容快照(全新安装的内容见 host usageSettingsDefaults())。
// ⚠️ 这里的 alert / budget / turnCost 必须与 host `whale-balance.mjs` 的 usageSettingsDefaults()
//    **逐字段一致**：host 那份决定新用户装完看到什么，这份决定编辑器里点「恢复默认」变成什么。
//    改一处必须同步另一处。图片一律引用语义内置 id(如 bimg_money1)，不要写用户图库的 bimg_xxx。
function usageRemindDefaultLines(isAlert) {
  if (isAlert) {
    return [
      { type: 'text', text: '老大~你的DS余额', size: 5, bold: true },
      { type: 'text', text: '已经不足', size: 5, bold: true, row: 2 },
      { type: 'text', text: '¥{below}', size: 5, bold: true, rgb: 'rouge', color: '', bgRgb: '', bg: '', row: 2 },
      { type: 'text', text: '啦~', size: 5, bold: true, row: 2 },
      { type: 'image', imgId: 'bimg_money1', size: 6, imgScale: 0.4 },
      { type: 'link', text: '>> 喂 点 米 <<', url: 'https://platform.deepseek.com/top_up', size: 1, color: '#ffffff', rgb: '', bgRgb: 'indigo', bg: '', bold: true, ul: false },
    ]
  }
  return [
    { type: 'text', text: '老大，今天花销已经超过', size: 6, bold: true, row: 1 },
    { type: 'text', text: '¥{amount}', size: 7, bold: true, row: 1, rgb: 'rouge', color: '', italic: false, bgRgb: '', bg: '' },
    { type: 'text', text: '啦，再花要变成穷光蛋啦...', size: 6, bold: true, row: 1 },
  ]
}
// —— 每轮消耗提示内容(v720):与余额预警/预算共用同一套模块化提醒内容;金额占位符 {cost} ——
// 默认 = 冻结的当前生效快照(与 host usageSettingsDefaults().turnCost 逐字段一致)
function usageTurnCostDefaultLines() {
  return [
    { type: 'text', text: '上一轮对话消耗:', size: 8, bold: true },
    { type: 'text', text: '¥ {cost}', size: 24, bold: true, color: '#e0433f' },
    { type: 'today', size: 2, tpl: '今日已用 {expense_ds}', bold: false, rgb: '', color: '#ffffff', bgRgb: 'indigo', bg: '' },
  ]
}
// 每轮消耗提示内容读取:lines[] 优先;无内容 → 默认模板
function usageTurnCostLines() {
  var c = (usageSet && usageSet.turnCost) || {}
  if (Array.isArray(c.lines) && c.lines.length) return c.lines
  return usageTurnCostDefaultLines()
}
// {cost} 的替换值:与旧版一致保留两位小数;缺失/非数一律 '--'
// (注意 Number(null)===0,所以必须先判空,否则会显示 0.00)
function usageCostValue(amount) {
  if (amount === null || amount === undefined || amount === '') return '--'
  var n = Number(amount)
  return isFinite(n) ? n.toFixed(2) : '--'
}
// 提醒泡泡停留毫秒:autoClose=false 或 ttlSec<=0 → 不自动关闭(0);未设置(旧配置)→ 默认 6500
function usageRemindTtlMs(cfg) {
  cfg = cfg || {}
  if (cfg.autoClose === false) return 0
  var s = Number(cfg.ttlSec)
  if (cfg.ttlSec !== undefined && isFinite(s) && s > 0) return Math.max(500, Math.round(s * 1000))
  return USAGE_ALERT_TTL
}
// 提醒内容读取:lines[] 优先;无内容 → 默认模板
function usageRemindLinesOf(cfg, isAlert) {
  cfg = cfg || {}
  if (Array.isArray(cfg.lines) && cfg.lines.length) return cfg.lines
  return usageRemindDefaultLines(isAlert)
}
// —— 等待交互（提问 / 授权）提示内容（v761 / #161 C5）——
// 默认内容必须与宿主 whale-balance.mjs 的 waitDefaultLines(kind) **逐字段一致**
// （宿主 = 新用户默认；这里 = 编辑器「恢复默认」的目标，改一处必须同步另一处）。
function usageWaitDefaultLines(kind) {
  // v774：出厂默认内容 = 作者当前实际使用的那一套（对话名模块「[ 对话名 ]」+ 一句提示语，都带跑马灯配色）
  var isApproval = (kind === 'approval')
  return [
    { type: 'session', size: 10, bold: true, tpl: '[ {session} ]', len: 5, rgb: 'champagne', color: '' },
    { type: 'text', text: (isApproval ? '正在等待老大授权' : '正在等待老大回答'), size: 7, bold: true, bgRgb: '', bg: '', rgb: 'indigo', color: '' },
  ]
}
// 等待提示内容读取:events.<kind>.lines[] 优先;缺失/空 → 默认模板
function usageWaitLinesOf(cfg, kind) {
  cfg = cfg || {}
  if (Array.isArray(cfg.lines) && cfg.lines.length) return cfg.lines
  return usageWaitDefaultLines(kind)
}
// 占位替换:{below} 余额预警阈值 / {amount} 今日预算阈值 / {cost} 本轮消耗金额
function usageFillText(txt, below, amount, cost, session) {
  return String(txt || '')
    .replace(/\{below\}/g, below != null ? String(below) : '')
    .replace(/\{amount\}/g, amount != null ? String(amount) : '')
    .replace(/\{cost\}/g, cost != null ? String(cost) : '')
  // v761：{session} = 当前对话名（wait.json 轮询回填；读不到显示「当前对话」；超长截断）
  .replace(/\{session\}/g, session != null ? String(session) : soundSessionLabel())

}
function usageLineFontPx(level) {
  var n = Math.max(1, Math.min(50, Math.round(Number(level) || 7)))
  return Math.min(40, Math.round(12 + (n - 1) * 0.8))
}
// 提醒弹窗里的一行(文本模块子集:占位替换 + 字号/字形/颜色/纯色底色;跑马灯渐变暂以默认色显示)
function usageAppendLine(body, m, below, amount) {
  try {
    m = m || {}
    // 图片类模块(图片/动图 与 随机图片):与真实泡泡一致地显示一张图
    if (bubbleIsImgMod(m)) {
      var imgId2 = m.imgId || ''
      if (m.type === 'randimg') {
        var pool2 = []
        var arrI = Array.isArray(m.imgs) ? m.imgs : []
        for (var pi2 = 0; pi2 < arrI.length; pi2++) {
          var itI = arrI[pi2] || {}
          if (itI.imgId) pool2.push({ imgId: itI.imgId, w: itI.w })
        }
        if (!pool2.length) return
        var pk2 = bubblePickLine(pool2, m._lastPickImg)
        if (pk2 === null || pk2 === undefined || !pool2[pk2]) return
        m._lastPickImg = pk2
        imgId2 = pool2[pk2].imgId
      }
      if (!imgId2) return
      var imEl2 = document.createElement('img')
      imEl2.alt = ''
      imEl2.draggable = false
      imEl2.src = '/dsh-whale/bubble-img.png?id=' + encodeURIComponent(imgId2)
      var sc3 = Number(m.imgScale)
      imEl2.style.maxWidth = (isFinite(sc3) && sc3 > 0 ? Math.max(24, Math.round(240 * Math.max(0.1, Math.min(1, sc3)))) : 240) + 'px'
      imEl2.style.maxHeight = '120px'
      imEl2.style.display = 'block'
      imEl2.style.margin = '4px auto'
      body.appendChild(imEl2)
      return
    }
    var raw = String(m.text != null ? m.text : '')
    var txt = usageFillText(raw, below, amount)
    var div = document.createElement('div')
    div.style.margin = '4px auto'
    div.style.maxWidth = '100%'
    if (!txt) { div.style.height = '8px'; div.style.margin = '2px auto'; body.appendChild(div); return }
    div.textContent = txt
    div.style.display = 'inline-block'
    div.style.textAlign = 'center'
    div.style.whiteSpace = 'pre-wrap'
    div.style.wordBreak = 'break-word'
    div.style.fontSize = usageLineFontPx(m.size) + 'px'
    div.style.lineHeight = '1.4'
    if (m.bold) div.style.fontWeight = '700'
    if (m.italic) div.style.fontStyle = 'italic'
    if (m.ul) div.style.textDecoration = 'underline'
    if (m.fontFamily) div.style.fontFamily = m.fontFamily
    var bg = m.bg ? String(m.bg) : ''
    if (bg) { div.style.background = bg; div.style.borderRadius = '7px'; div.style.padding = '1px 8px' }
    var col = m.color ? String(m.color) : ''
    if (col && !m.rgb && !m.bgRgb) div.style.color = col
    body.appendChild(div)
  } catch (err) {}
}
function checkUsageAlerts(balance, todayUsage) {
  try {
    if (!usageSet) return
    var a = usageSet.alert
    if (a && a.on) {
      var below = Number(a.below)
      if (isFinite(below) && typeof balance === 'number' && balance > 0 && balance <= below) {
        if (!usageAlertBelowFired) {
          usageAlertBelowFired = true
          showUsagePopup('余额预警', usageRemindLinesOf(a, true), below, null, 2, a)
        }
      } else if (typeof balance === 'number' && balance > below) {
        usageAlertBelowFired = false
      }
    }
    var b = usageSet.budget
    if (b && b.on) {
      var amt = Number(b.amount)
      if (isFinite(amt) && amt > 0 && typeof todayUsage === 'number' && todayUsage >= amt) {
        var key = usageTodayKeyStr() + ':' + String(amt)
        if (usageBudgetFiredKey !== key) {
          usageBudgetFiredKey = key
          showUsagePopup('今日预算提醒', usageRemindLinesOf(b, false), null, amt, 1, b)
        }
      }
    }
  } catch (err) {}
}
// 自定义 API 模型：按各模型自己的余额/今日已用检查提醒（数据来自 api-models.json 轮询）
function runApiModelAlerts() {
  try {
    if (!usageSet) return
    var ms = usageSet.models || {}
    for (var i = 0; i < apiModels.length; i++) {
      var m = apiModels[i]
      if (!m || !m.id || m.id === 'deepseek') continue // 内置 DeepSeek 走上面的原有链路
      var st = ms[m.id]
      if (!st) continue
      var a = st.alert
      if (a && a.on) {
        var below = Number(a.below)
        var bal = Number(m.balance)
        var fk = m.id + ':' + below
        if (isFinite(below) && isFinite(bal) && bal > 0 && bal <= below) {
          if (!apiAlertFired[fk]) {
            apiAlertFired[fk] = true
            showUsagePopup(m.name + ' 余额预警', usageRemindLinesOf(a, true), below, null, 2, a)
          }
        } else if (isFinite(bal) && bal > below) {
          apiAlertFired[fk] = false
        }
      }
      var b = st.budget
      if (b && b.on) {
        var amt = Number(b.amount)
        var used = Number(m.todayUsage)
        // 币种一致性：预算阈值按「模型币种」理解（面板就在该模型下填），
        // 而今日已用可能来自会话事件（CNY）或余额差（模型币种）。两侧不同时用自定义单价里的汇率换算；
        // 没有汇率就**不比较**（宁可漏一次提醒，也不要用错单位误报），并在控制台留一条 warn 便于定位。
        var usedCmp = used
        if (isFinite(used)) {
          var curToday = apiTodayCur(m)
          var curBudget = String(m.currency || 'CNY').toUpperCase()
          var conv = apiConvertMoney(used, curToday, curBudget, m.price && m.price.rate)
          if (conv === null && curToday !== curBudget) {
            if (!apiBudgetUnitWarned[m.id]) {
              apiBudgetUnitWarned[m.id] = true
              try { console.warn('[whale] 今日预算未比较：' + m.name + ' 的今日已用为 ' + curToday + '，预算阈值按 ' + curBudget + ' 填写，且未配置汇率（模型面板 → 单价 → 汇率）') } catch (err) {}
            }
            usedCmp = null
          } else if (conv !== null) {
            usedCmp = conv
          }
        }
        var bk = m.id + ':' + usageTodayKeyStr() + ':' + amt
        if (isFinite(amt) && amt > 0 && isFinite(usedCmp) && usedCmp >= amt) {
          if (!apiBudgetFired[bk]) {
            apiBudgetFired[bk] = true
            showUsagePopup(m.name + ' 今日预算提醒', usageRemindLinesOf(b, false), null, amt, 1, b)
          }
        }
      }
    }
  } catch (err) {}
}
// 占位替换后的展示模块列表(深拷贝,不改配置)
// v761（#161 C5）：第 5 个参数 session 原样透传给 usageFillText —— 等待交互的提示内容里有
// `{session}`，要在这里（渲染/预览时）就换成对话名，否则真实泡泡与编辑器预览都会**原样显示 `{session}`**。
// 不传（既有 alert / budget / cost 三处调用）时 usageFillText 会自己回落 soundSessionLabel()，
// 因此这三处的行为逐字未变（它们的 lines 里本来就没有 {session}）。
function usageAlertModsResolved(mods, below, amount, cost, session) {
  var out = []
  try {
    for (var i = 0; i < mods.length; i++) {
      var m0 = mods[i] || {}
      var cp = JSON.parse(JSON.stringify(m0))
      var raw = String(m0.text != null ? m0.text : '')
      cp.text = raw.length ? usageFillText(raw, below, amount, cost, session) : raw
      out.push(cp)
    }
  } catch (err) {}
  return out
}
// content:字符串(单段)或提醒行数组;rank:1=今日预算 2=余额预警(与消耗泡泡同批排队,预算>预警>消耗);cfg 用于自动关闭时长
function showUsagePopup(title, content, below, amount, rank, cfg) {
  try {
    var mods = []
    if (typeof content === 'string') mods = [{ type: 'text', text: content, size: 7, bold: true }]
    else if (Array.isArray(content)) mods = content
    if (mods.length && whaleSysPush({ kind: 'alert', mods: usageAlertModsResolved(mods, below, amount), rank: (rank === 1 || rank === 2) ? rank : 2, ttlMs: usageRemindTtlMs(cfg) })) return
    usagePopupCard(title, content, below, amount)
  } catch (err) {}
}
// 兜底:鲸鱼关闭/被占用时沿用居中提示卡
function usagePopupCard(title, content, below, amount) {
  try {
    var mask = document.createElement('div')
    mask.className = 'dshwv-usage-mask'
    var card = document.createElement('div')
    card.className = 'dshwv-usage-card'
    card.style.width = 'min(380px,90vw)'
    var t = document.createElement('div')
    t.className = 'dshwv-usage-wintitle'
    t.textContent = title || '提示'
    card.appendChild(t)
    var body = document.createElement('div')
    body.className = 'dshwv-usage-windowbody'
    body.style.textAlign = 'center'
    if (typeof content === 'string') {
      body.textContent = usageFillText(content, below, amount)
      body.style.whiteSpace = 'pre-wrap'
    } else if (Array.isArray(content)) {
      for (var i = 0; i < content.length; i++) usageAppendLine(body, content[i], below, amount)
    } else {
      body.textContent = ''
    }
    card.appendChild(body)
    var btns = document.createElement('div')
    btns.className = 'dshwv-bubbtns'
    btns.style.justifyContent = 'center'
    var ok = document.createElement('button')
    ok.type = 'button'
    ok.className = 'dshwv-bubbtn dshwv-bubbtn-ok'
    ok.textContent = '知道了'
    ok.addEventListener('click', function () { try { dshwBodyDetach(mask) } catch (err) {} })
    btns.appendChild(ok)
    card.appendChild(btns)
    mask.appendChild(card)
    mask.addEventListener('click', function (e) { if (e.target === mask) { try { dshwBodyDetach(mask) } catch (err) {} } })
    dshwBodyAppend(mask)
  } catch (err) {}
}
var USAGE_PALETTE = ['#203170', '#e0433f', '#2fa24c', '#b060c8', '#e89a2e', '#3aa6c8', '#d06a8a', '#7a8b2f', '#6a6ad0', '#c84a8a']
// 统计一组天数里的模型合计(输入 days:[{models:[{model,cost}]}])
// v776（issue #163）：换/删 API key 后，宿主按密钥指纹分本记账（换 key = 换一本）。
// 界面侧给一句说明：否则旧账本的日期看起来像"丢了"（数据其实一直在 .dshw-usage.json 里）。
// 宿主在 daySummary() 上补了 bookCount / historyHint / source='balance-observed-other-account'，
// 这里只负责把话讲清楚 —— **不做相加**（无法判断两次是不是同一个账户）。
function usageMultiBookNote(allDays, today) {
  try {
    var maxCount = 0, maxHint = '', otherDays = 0, todayCount = 0
    for (var i = 0; i < (allDays || []).length; i++) {
      var dx = allDays[i] || {}
      var n = Number(dx.bookCount) || 0
      if (n > maxCount) { maxCount = n; maxHint = String(dx.historyHint || '') }
      if (dx.source === 'balance-observed-other-account') otherDays++
    }
    todayCount = Number(today && today.bookCount) || 0
    if (maxCount <= 1 && !otherDays) return ''
    var parts = []
    if (maxCount > 1 || todayCount > 1) {
      parts.push('检测到 ' + Math.max(maxCount, todayCount) + ' 个记账本（换过 API key）：' + (maxHint || '同一天在多个本里都有观测'))
    }
    if (otherDays > 0) parts.push('另有 ' + otherDays + ' 天来自历史记账本，已按「已观测消费 · 历史账户」显示，未与当前账户相加')
    return parts.join('；')
  } catch (err) { return '' }
}
function usageAggModels(daysArr) {
  var map = {}
  ;(daysArr || []).forEach(function (day) {
    ;(day.models || []).forEach(function (mm) {
      var k = mm && mm.model ? mm.model : '未知'
      map[k] = (map[k] || 0) + (Number(mm.cost) || 0)
    })
  })
  return Object.keys(map).map(function (k) { return { model: k, cost: map[k] } }).sort(function (a, b) { return b.cost - a.cost })
}
// 模型显示名(v653):官方把 V4.1-Flash 的模型名定为 deepseek-flash,旧名 deepseek-v4-flash /
// deepseek-v4-flash-vision-exp 也仍由同一颗 V4.1-Flash 提供服务并按 Flash 计价。
// 这里只做「友好标注」,不合并数据(历史仍区分新旧 id);悬浮显示原始 id。
function usageModelLabel(m) {
  var raw = String(m || '')
  if (!raw) return '未知'
  var k = raw.toLowerCase()
  if (k === 'deepseek-flash') return 'DeepSeek-V4.1-Flash (deepseek-flash)'
  if (k === 'deepseek-v4-flash' || k === 'deepseek-v4-flash-vision-exp') return raw + '(旧名·同 V4.1 Flash)'
  return raw
}
// 模型占比条区块
function usageRatioRows(body, secTitle, agg, totalLabel) {
  body.appendChild(uSectionTitle(secTitle, totalLabel))
  if (!agg.length) {
    var no = document.createElement('div')
    no.className = 'dshwv-usage-hint'
    no.textContent = '暂无模型明细'
    body.appendChild(no)
    return
  }
  var sum = agg.reduce(function (a, x) { return a + x.cost }, 0) || 1
  var costEls = []
  agg.forEach(function (row, i) {
    var wr = document.createElement('div')
    wr.className = 'dshwv-usage-ratio'
    var lab = document.createElement('span')
    lab.className = 'dshwv-usage-ratio-label'
    lab.textContent = usageModelLabel(row.model)
    lab.title = String(row.model || '未知')
    wr.appendChild(lab)
    var track = document.createElement('div')
    track.className = 'dshwv-usage-ratio-track'
    var fill = document.createElement('div')
    fill.className = 'dshwv-usage-ratio-fill'
    fill.style.width = Math.round(row.cost / sum * 100) + '%'
    fill.style.background = USAGE_PALETTE[i % USAGE_PALETTE.length]
    track.appendChild(fill)
    wr.appendChild(track)
    // 百分比紧跟占比条右侧(固定预留,保证条长一致)
    var pct = document.createElement('span')
    pct.className = 'dshwv-usage-ratio-pct'
    pct.textContent = Math.round(row.cost / sum * 100) + '%'
    wr.appendChild(pct)
    // 花费列紧邻百分比,整组等宽右对齐
    var cost = document.createElement('span')
    cost.className = 'dshwv-usage-ratio-cost'
    cost.textContent = usageMoney(row.cost)
    cost.title = cost.textContent
    wr.appendChild(cost)
    costEls.push(cost)
    body.appendChild(wr)
  })
  try {
    var parW = (costEls[0] && costEls[0].parentNode) ? costEls[0].parentNode.clientWidth : 420
    var capCost = Math.max(50, Math.floor(parW - 96 - 42 - 32))
    var maxCost = 40
    for (var c1 = 0; c1 < costEls.length; c1++) {
      var cw1 = costEls[c1].scrollWidth || 40
      if (cw1 > maxCost) maxCost = cw1
    }
    var useCost = Math.min(maxCost, capCost)
    for (var c2 = 0; c2 < costEls.length; c2++) {
      costEls[c2].style.width = useCost + 'px'
      costEls[c2].style.textAlign = 'right'
      costEls[c2].style.overflow = 'hidden'
      costEls[c2].style.textOverflow = 'ellipsis'
    }
  } catch (err) {}
}
// 近 N 天柱状图(原生 canvas,悬停显示数值;opts:{today,onPick})
function usageDrawBarChart(body, secTitle, days, opts) {
  opts = opts || {}
  var todayKey = String(opts.today || '')
  var onPick = opts.onPick || null
  body.appendChild(uSectionTitle(secTitle, ''))
  if (!days || !days.length) {
    var no = document.createElement('div')
    no.className = 'dshwv-usage-hint'
    no.textContent = '暂无每日数据'
    body.appendChild(no)
    return
  }
  var wrap = document.createElement('div')
  wrap.className = 'dshwv-usage-chartwrap'
  var canvas = document.createElement('canvas')
  wrap.appendChild(canvas)
  var tip = document.createElement('div')
  tip.className = 'dshwv-usage-tip'
  tip.style.display = 'none'
  wrap.appendChild(tip)
  body.appendChild(wrap)
  var bars = []
  function paint(hoverIdx) {
    // 以图表容器实际宽度为准(勿用画布自身 clientWidth:内联设宽后会锁死旧值导致右侧被裁)
    var cw = Math.max(120, (wrap && (wrap.clientWidth || (wrap.getBoundingClientRect ? wrap.getBoundingClientRect().width : 0))) || canvas.clientWidth || 520)
    var ch = 150
    // 高清绘制:按 devicePixelRatio 放大位图,避免 CSS 缩放导致文字发糊
    var dpr = window.devicePixelRatio || 1
    canvas.width = Math.max(1, Math.round(cw * dpr))
    canvas.height = Math.max(1, Math.round(ch * dpr))
    canvas.style.width = cw + 'px'
    canvas.style.height = ch + 'px'
    var g = canvas.getContext('2d')
    g.setTransform(dpr, 0, 0, dpr, 0, 0)
    g.clearRect(0, 0, cw, ch)
    var padL = 42, padR = 10, padT = 10, padB = 22
    var iw = cw - padL - padR
    var ih = ch - padT - padB
    var max = 1
    for (var i = 0; i < days.length; i++) max = Math.max(max, Number(days[i].total) || 0)
    var step = iw / days.length
    var bw = Math.max(3, Math.min(36, step * 0.62))
    bars = []
    for (var k = 0; k < days.length; k++) {
      var val = Number(days[k].total) || 0
      var h = val > 0 ? Math.max(2, (val / max) * ih) : 0
      var x = padL + step * k + (step - bw) / 2
      var y = padT + ih - h
      var kToday = !!(todayKey && String(days[k].date || '') === todayKey)
      g.fillStyle = k === hoverIdx ? '#e0433f' : (kToday ? '#2fa44c' : '#203170')
      if (k === hoverIdx) { g.globalAlpha = 0.9 }
      if (h > 0) { g.fillRect(x, y, bw, h) } else { g.fillStyle = 'rgba(32,49,112,.25)'; g.fillRect(x, padT + ih - 3, bw, 3) }
      g.globalAlpha = 1
      bars.push({ x: x, w: bw, day: days[k] })
      if (days.length <= 16 || k % Math.ceil(days.length / 16) === 0) {
        g.fillStyle = '#9fb0d9'
        g.font = '10px sans-serif'
        g.textAlign = 'center'
        var dl = String(days[k].date || '').split('-')
        var lab = dl.length === 3 ? dl[1] + '-' + dl[2] : days[k].date
        g.fillText(lab, x + bw / 2, ch - 8)
      }
    }
    // 左轴
    g.fillStyle = '#9fb0d9'
    g.font = '10px sans-serif'
    g.textAlign = 'right'
    g.fillText(usageMoney(max, opts.currency), padL - 4, padT + 8)
    g.fillText(usageMoney(max / 2, opts.currency), padL - 4, padT + ih / 2 + 3)
    g.fillText('¥0', padL - 4, padT + ih + 4)
  }
  function move(ev) {
    var r = canvas.getBoundingClientRect()
    var x = ev.clientX - r.left
    var hover = -1
    for (var i = 0; i < bars.length; i++) {
      if (x >= bars[i].x && x <= bars[i].x + bars[i].w) { hover = i; break }
    }
    paint(hover)
    if (hover >= 0) {
      tip.style.display = 'block'
      tip.textContent = bars[hover].day.date + '  ' + usageMoney(bars[hover].day.total, opts.currency)
      var wr = wrap.getBoundingClientRect()
      var tx = ev.clientX - wr.left + 10
      if (tx + 130 > wr.width) tx = ev.clientX - wr.left - 140
      var ty = ev.clientY - wr.top + 12
      tip.style.left = Math.max(0, tx) + 'px'
      tip.style.top = Math.max(0, ty) + 'px'
    } else {
      tip.style.display = 'none'
    }
  }
  canvas.addEventListener('mousemove', move)
  canvas.addEventListener('mouseleave', function () { tip.style.display = 'none'; paint(-1) })
  if (onPick) {
    canvas.addEventListener('click', function (ev) {
      try {
        var rc = canvas.getBoundingClientRect()
        var cx = ev.clientX - rc.left
        for (var bi = 0; bi < bars.length; bi++) {
          if (cx >= bars[bi].x && cx <= bars[bi].x + bars[bi].w) { onPick(bars[bi].day.date); break }
        }
      } catch (err) {}
    })
  }
  paint(-1)
  // 展开动画/滚动条稳定后再重绘一次,按最终可视宽度校准,避免右侧被裁
  try {
    setTimeout(function () { try { paint(-1) } catch (err) {} }, 420)
  } catch (err) {}
}
// —— 消费记录(全部)界面:概览头 + 可折叠分区 + 按天折叠逐条 + 搜索/限量 ——
// 平滑展开/收起:max-height 过渡(展开后还原 auto;收起后隐藏)
function usageSlide(el, open) {
  try {
    if (!el) return
    if (el.__dshwSlide) clearTimeout(el.__dshwSlide)
    el.style.transition = 'max-height .24s ease, opacity .16s ease'
    el.style.overflow = 'hidden'
    if (open) {
      el.style.display = 'block'
      var h0 = el.scrollHeight
      if (h0 <= 0) h0 = 100
      el.style.opacity = '0'
      el.style.maxHeight = '0px'
      void el.offsetHeight
      el.style.opacity = '1'
      el.style.maxHeight = h0 + 'px'
      el.__dshwSlide = setTimeout(function () { el.style.maxHeight = ''; el.style.overflow = ''; el.style.transition = ''; el.__dshwSlide = null }, 260)
    } else {
      var ch = el.scrollHeight
      if (ch <= 0) { el.style.display = 'none'; el.style.transition = ''; return }
      el.style.maxHeight = ch + 'px'
      void el.offsetHeight
      el.style.opacity = '0'
      el.style.maxHeight = '0px'
      el.__dshwSlide = setTimeout(function () { el.style.display = 'none'; el.style.maxHeight = ''; el.style.opacity = ''; el.style.overflow = ''; el.style.transition = ''; el.__dshwSlide = null }, 250)
    }
  } catch (err) { try { el.style.display = open ? 'block' : 'none' } catch (err2) {} }
}
// 可折叠分区(懒构建 + 过渡动画;返回 box 附带 __open/__close)
function usageCollapseBlock(parent, label, openDefault, build) {
  var box = document.createElement('div')
  var hd = document.createElement('button')
  hd.type = 'button'
  hd.className = 'dshwv-usage-collapse'
  var inner = document.createElement('div')
  inner.className = 'dshwv-usage-collapse-body'
  inner.style.display = 'none'
  var opened = false
  var st = false
  function ensureBuild() {
    if (!opened) { opened = true; try { build(inner) } catch (err) {} }
  }
  function openNow() {
    st = true
    // 先以“占位可见但高度为0”的方式参与布局,内容(画布等)能拿到真实宽度再构建
    inner.style.display = 'block'
    inner.style.maxHeight = '0px'
    inner.style.overflow = 'hidden'
    ensureBuild()
    usageSlide(inner, true)
    paint()
  }
  function closeNow() {
    st = false
    usageSlide(inner, false)
    paint()
  }
  function paint() {
    hd.innerHTML = ''
    var l = document.createElement('span')
    l.textContent = label
    hd.appendChild(l)
    var ch = document.createElement('span')
    ch.className = 'dshwv-usage-chev'
    ch.textContent = st ? '▾' : '▸'
    hd.appendChild(ch)
  }
  hd.addEventListener('click', function () { if (st) closeNow(); else openNow() })
  box.__open = openNow
  box.__close = closeNow
  paint()
  box.appendChild(hd)
  box.appendChild(inner)
  parent.appendChild(box)
  if (openDefault) openNow()
  return box
}
function usageEvTime(ev) {
  try {
    var dd = new Date(ev.ts)
    var p2 = function (n) { return String(n).padStart(2, '0') }
    return p2(dd.getHours()) + ':' + p2(dd.getMinutes())
  } catch (err) { return '' }
}
function fillUsageRecordsWindow(d) {
  var card = usageMoreCard
  card.innerHTML = ''
  var title = document.createElement('div')
  title.className = 'dshwv-usage-wintitle'
  title.textContent = '消费记录(全部)'
  card.appendChild(title)
  var closeBtn = document.createElement('button')
  closeBtn.type = 'button'
  closeBtn.className = 'dshwv-usage-close'
  closeBtn.textContent = '×'
  closeBtn.title = '关闭'
  closeBtn.addEventListener('click', closeUsageRecordsWindow)
  card.appendChild(closeBtn)
  var body = document.createElement('div')
  body.className = 'dshwv-usage-windowbody'
  card.appendChild(body)
  if (!d || !d.ok) { body.textContent = '加载失败'; return }
  codexWhaleHost.recordsPager(body, d, fillUsageRecordsWindow)
  var allDays = ((d.all && d.all.days) || []).slice().sort(function (a, b) { return a.date < b.date ? -1 : 1 })
  var evAll = ((d.all && d.all.events) || []).slice()
  // ① 概览头
  var sumAllByCurrency = {}
  var chartCurrency = (d.today && d.today.currency) || 'CNY'
  var maxDay = null
  for (var s1 = 0; s1 < allDays.length; s1++) {
    var dayCurrency = allDays[s1].currency || 'CNY'
    sumAllByCurrency[dayCurrency] = (sumAllByCurrency[dayCurrency] || 0) + (Number(allDays[s1].total) || 0)
    if (dayCurrency === chartCurrency && (!maxDay || Number(allDays[s1].total) > Number(maxDay.total))) maxDay = allDays[s1]
  }
  var ov = document.createElement('div')
  ov.className = 'dshwv-usage-oview'
  var ovL = document.createElement('div')
  ovL.textContent = '全部记录合计'
  ov.appendChild(ovL)
  var ovN = document.createElement('div')
  ovN.className = 'dshwv-usage-oview-num'
  ovN.textContent = Object.keys(sumAllByCurrency).map(function (cur) { return usageMoney(sumAllByCurrency[cur], cur) }).join(' + ') || usageMoney(0, chartCurrency)
  ov.appendChild(ovN)
  var ovS = document.createElement('div')
  ovS.className = 'dshwv-usage-hint'
  ovS.textContent = '共 ' + evAll.length + ' 笔明细' + (maxDay ? ' · 峰值 ' + maxDay.date + ' ' + usageMoney(maxDay.total, maxDay.currency) : '')
  ov.appendChild(ovS)
  var sourceNote = document.createElement('div')
  sourceNote.className = 'dshwv-usage-hint'
  sourceNote.textContent = '按各日标注口径汇总；本机模型明细为估算，与账户消费覆盖范围不同。'
  ov.appendChild(sourceNote)
  // v776（issue #163）：多记账本（换过 API key）时补一句说明，让旧账本的记录有入口可查
  var bookNote = usageMultiBookNote(allDays, d.today)
  if (bookNote) {
    var bn = document.createElement('div')
    bn.className = 'dshwv-usage-hint'
    bn.style.marginTop = '2px'
    bn.textContent = bookNote
    ov.appendChild(bn)
  }
  body.appendChild(ov)
  var detailBox = null
  // ② 统计图表(默认折叠,懒渲染)
  usageCollapseBlock(body, '统计图表(近30天 / 模型占比)', false, function (inner) {
    usageDrawBarChart(inner, '近30天消费 · ' + chartCurrency + '（点柱查看当日）', allDays.filter(function (row) { return (row.currency || 'CNY') === chartCurrency }).slice(-30), {
      today: usageTodayKeyStr(), currency: chartCurrency,
      onPick: function (date) {
        try {
          if (!detailBox) return
          detailBox.__open()
          setTimeout(function () {
            var tr = detailBox.querySelector('[data-usage-day="' + String(date) + '"]')
            if (tr) {
              tr.scrollIntoView({ block: 'center', behavior: 'smooth' })
              tr.style.boxShadow = 'inset 0 0 0 2px rgba(32,49,112,.55)'
              setTimeout(function () { tr.style.boxShadow = '' }, 1400)
            }
          }, 120)
        } catch (err) {}
      },
    })
    var todayAgg = usageAggModels(d.today && d.today.models ? [{ models: d.today.models }] : [])
    usageRatioRows(inner, '今日模型费用 · 估算', todayAgg, usageMoney((d.today && d.today.modelTotal) || 0))
    var sevenAgg = usageAggModels(d.days7 || [])
    usageRatioRows(inner, '近7天模型费用 · 估算', sevenAgg, usageMoney((d.days7 || []).reduce(function (sum, row) { return sum + (Number(row.modelTotal) || 0) }, 0)))
  })
  // ③ 每日与逐条明细(默认折叠;搜索/限量加载)
  detailBox = usageCollapseBlock(body, '每日与逐条明细', false, function (inner) {
    var search = document.createElement('input')
    search.type = 'text'
    search.className = 'dshwv-colnat'
    search.style.width = '100%'
    search.style.margin = '2px 0 6px'
    search.placeholder = '搜索:日期(如 07-21)或模型名,过滤逐条明细…'
    inner.appendChild(search)
    // 事件按天分组
    var evMap = {}
    evAll.forEach(function (ev) {
      var day = ev.day || ''
      if (!day) {
        try { var dd2 = new Date(ev.ts); day = dd2.getFullYear() + '-' + String(dd2.getMonth() + 1).padStart(2, '0') + '-' + String(dd2.getDate()).padStart(2, '0') } catch (err) {}
      }
      if (!day) return
      ;(evMap[day] = evMap[day] || []).push(ev)
    })
    var dayTot = {}
    var dayMeta = {}
    allDays.forEach(function (dx) { dayTot[dx.date] = Number(dx.total) || 0; dayMeta[dx.date] = dx })
    var todayKeyStr2 = usageTodayKeyStr()
    function dayGroup(day, evs) {
      var row = document.createElement('div')
      row.className = 'dshwv-usage-row'
      row.style.cursor = 'pointer'
      row.setAttribute('data-usage-day', day)
      var name = document.createElement('span')
      name.style.flex = '1 1 auto'
      name.style.minWidth = '0'
      name.style.overflow = 'hidden'
      name.style.textOverflow = 'ellipsis'
      name.style.whiteSpace = 'nowrap'
      name.textContent = day + (evs.length ? ' (' + evs.length + ')' : '')
      row.appendChild(name)
      var c = document.createElement('span')
      c.style.flex = '0 0 auto'
      var dayV = dayTot[day]
      if ((dayV === undefined || dayV === 0) && day === todayKeyStr2 && d.today && isFinite(Number(d.today.total))) dayV = Number(d.today.total)
      if (dayV === undefined || dayV === null) dayV = evs.reduce(function (a, x) { return a + (Number(x.cost) || 0) }, 0)
      c.textContent = usageMoney(dayV, dayMeta[day] && dayMeta[day].currency)
      // v776（issue #163）：悬停提示带上"历史记账本"说明（旧 key 的记账不再看着像丢了）
      var dayM = dayMeta[day] || {}
      name.title = (dayM.label || '本地估算') + (dayM.historyHint ? ' · ' + dayM.historyHint : '')
      row.appendChild(c)
      var chev = document.createElement('span')
      chev.className = 'dshwv-usage-chev'
      chev.textContent = '▸'
      row.appendChild(chev)
      var detail = document.createElement('div')
      detail.className = 'dshwv-usage-daydetail'
      detail.style.display = 'none'
      var built = false
      row.addEventListener('click', function (e) {
        var on = detail.style.display !== 'block'
        if (on) {
          if (!built) {
            built = true
            var lim = Math.min(evs.length, 100)
            for (var i = 0; i < lim; i++) {
              var ev = evs[i]
              var r2 = document.createElement('div')
              r2.className = 'dshwv-usage-row'
              r2.style.padding = '2px 0 2px 6px'
              var n2 = document.createElement('span')
              n2.style.flex = '1 1 auto'
              n2.style.minWidth = '0'
              n2.style.overflow = 'hidden'
              n2.style.textOverflow = 'ellipsis'
              n2.style.whiteSpace = 'nowrap'
              n2.textContent = (usageEvTime(ev) ? usageEvTime(ev) + '  ' : '') + usageModelLabel(ev.model)
              n2.title = (usageEvTime(ev) ? usageEvTime(ev) + '  ' : '') + String(ev.model || '未知')
              r2.appendChild(n2)
              var c2 = document.createElement('span')
              c2.style.flex = '0 0 auto'
              c2.textContent = usageMoney(ev.cost)
              c2.title = '本地估算：' + Number(ev.cost || 0).toFixed(8) + ' CNY'
              r2.appendChild(c2)
              detail.appendChild(r2)
            }
            if (evs.length > lim) {
              var moreTxt = document.createElement('div')
              moreTxt.className = 'dshwv-usage-hint'
              moreTxt.textContent = '… 该日共 ' + evs.length + ' 条,仅显示前 ' + lim + ' 条'
              detail.appendChild(moreTxt)
            }
            if (!evs.length) {
              var nd = document.createElement('div')
              nd.className = 'dshwv-usage-hint'
              nd.textContent = '该日仅总额(启用模型明细后展示逐条)'
              detail.appendChild(nd)
            }
          }
          chev.textContent = '▾'
        } else chev.textContent = '▸'
        usageSlide(detail, on)
      })
      row.appendChild(detail)
      return row
    }
    var listWrap = document.createElement('div')
    inner.appendChild(listWrap)
    function renderGroups(q) {
      var ql = String(q || '').trim().toLowerCase()
      var groups = []
      for (var gi = 0; gi < allDays.length; gi++) {
        var day0 = allDays[gi].date
        var evs = evMap[day0] || []
        var hit = !ql || day0.toLowerCase().indexOf(ql) >= 0
        if (!hit) {
          for (var ei = 0; ei < evs.length && !hit; ei++) {
            var mm0 = String(evs[ei].model || '')
            // 原始 id 与友好标注都参与搜索(例如输入 4.1 / v4.1 / flash 都能命中)
            if (mm0.toLowerCase().indexOf(ql) >= 0 || usageModelLabel(mm0).toLowerCase().indexOf(ql) >= 0) hit = true
          }
        }
        if (hit || (day0 === usageTodayKeyStr() && !ql)) groups.push(day0)
      }
      // 最近的日期在最上面(倒序;点「加载更早」再看更久前)
      groups.sort(function (a, b) { return a < b ? 1 : (a > b ? -1 : 0) })
      var step = 12
      var shown = step
      listWrap.innerHTML = ''
      if (!groups.length) {
        var noR = document.createElement('div')
        noR.className = 'dshwv-usage-hint'
        noR.textContent = ql ? '没有匹配的记录' : '暂无每日记录'
        listWrap.appendChild(noR)
        return
      }
      function paintDays() {
        listWrap.innerHTML = ''
        var upto = Math.min(shown, groups.length)
        for (var k = 0; k < upto; k++) {
          var dayK = groups[k]
          listWrap.appendChild(dayGroup(dayK, evMap[dayK] || []))
        }
        if (shown < groups.length) {
          var mb = document.createElement('button')
          mb.type = 'button'
          mb.className = 'dshwv-usage-more'
          mb.textContent = '加载更早记录(还剩 ' + (groups.length - shown) + ' 天)'
          mb.addEventListener('click', function () { shown += step; paintDays() })
          listWrap.appendChild(mb)
        }
      }
      paintDays()
    }
    search.addEventListener('input', function () { renderGroups(search.value) })
    renderGroups('')
  })
}
dshwBodyAppend(rolePanel)

// —— 吸附与翻转自定义弹窗 ——
// 预览方框内五条可拖线：左/右/上/下四条吸附区边界 + 红色翻转线（竖中线位置）。
// 线距各自边缘的距离 = 该边吸附区宽度；翻转线左侧为“翻转区”（镜像朝向）。
// 数值框单位随档位变化：比例 = 视口宽/高 %，绝对 = px。关闭档 = 无吸附无翻转。
var SNAP_PREVIEW = 190 // CSS 回退尺寸（无实际意义，打开时按视口比例重算）
var snapPvW = SNAP_PREVIEW // 预览框实际宽（打开时按视口长宽比计算）
var snapPvH = SNAP_PREVIEW // 预览框实际高
var snapEdit = null
var snapLineDrag = null
var snapMask = null
var snapCard = null
var snapRadio = {}
var snapNum = {}
var snapNumUnit = {}
var snapPreview = null
function unitOf(mode) { return mode === 'px' ? 'px' : '%' }
function ensureSnapPx(cfg) {
  try {
    if (!(cfg.px.F >= 0)) {
      cfg.px.L = 80
      cfg.px.T = 0
      cfg.px.R = 80
      cfg.px.B = 80
      cfg.px.F = Math.round(Math.max(1, viewport().w) / 2)
    }
  } catch (err) {}
}
function snapSetVal(key, val) {
  try {
    if (!snapEdit || snapEdit.mode === 'off') return
    var m = snapEdit.mode
    var vp = viewport()
    var set = m === 'px' ? snapEdit.px : snapEdit.ratio
    var clamped = clampSnapKey(m, key, val, vp)
    set[key] = Math.max(0, Math.round(clamped))
  } catch (err) {}
}
// 按当前视口长宽比计算预览框尺寸（contain 适配，宽≤maxW、高≤maxH）。
// 只在弹窗打开时计算一次——拖动窗口边不更新预览（按需求不需要）。
function snapPvSize() {
  var vp = viewport()
  var maxW = 210, maxH = 190, minSide = 56
  var ar = Math.max(0.05, vp.w / vp.h)
  var w, h
  if (ar * maxH <= maxW) {
    h = maxH
    w = maxH * ar
  } else {
    w = maxW
    h = maxW / ar
  }
  w = Math.round(Math.max(minSide, Math.min(maxW, w)))
  h = Math.round(Math.max(minSide, Math.min(maxH, h)))
  return { w: w, h: h }
}
// 返回 {Lf,Tf,Rf,Bf,Ff} 各线相对预览框的分数坐标（0..1），按当前档位换算
function snapFrac() {
  var vp = viewport()
  var f = { Lf: 0, Tf: 0, Rf: 1, Bf: 1, Ff: 0.5 }
  try {
    var m = snapEdit.mode
    var w = Math.max(1, vp.w), h = Math.max(1, vp.h)
    if (m === 'ratio') {
      f.Lf = Math.min(100, Math.max(0, snapEdit.ratio.L)) / 100
      f.Tf = Math.min(100, Math.max(0, snapEdit.ratio.T)) / 100
      f.Rf = 1 - Math.min(100, Math.max(0, snapEdit.ratio.R)) / 100
      f.Bf = 1 - Math.min(100, Math.max(0, snapEdit.ratio.B)) / 100
      f.Ff = Math.min(100, Math.max(0, snapEdit.ratio.F)) / 100
    } else if (m === 'px') {
      f.Lf = Math.min(w, Math.max(0, snapEdit.px.L)) / w
      f.Tf = Math.min(h, Math.max(0, snapEdit.px.T)) / h
      f.Rf = 1 - Math.min(w, Math.max(0, snapEdit.px.R)) / w
      f.Bf = 1 - Math.min(h, Math.max(0, snapEdit.px.B)) / h
      f.Ff = Math.min(w, Math.max(0, snapEdit.px.F)) / w
    }
  } catch (err) {}
  return f
}
function renderSnapPreview() {
  try {
    if (!snapEdit || !snapPreview) return
    snapPreview.innerHTML = ''
    var W = snapPvW
    var H = snapPvH
    if (snapEdit.mode === 'off') {
      var off = document.createElement('div')
      off.className = 'dshwv-snapoff'
      off.textContent = '已关闭：无吸附、无翻转，自由摆放'
      snapPreview.appendChild(off)
      return
    }
    var f = snapFrac()
    var Lx = Math.max(0, Math.min(W, f.Lf * W))
    var Rx = Math.max(0, Math.min(W, f.Rf * W))
    var Ty = Math.max(0, Math.min(H, f.Tf * H))
    var By = Math.max(0, Math.min(H, f.Bf * H))
    var Fx = Math.max(0, Math.min(W, f.Ff * W))
    // 翻转区底色（翻转线左侧淡蓝）
    var flipBg = document.createElement('div')
    flipBg.className = 'dshwv-snapflip'
    flipBg.style.width = Fx + 'px'
    flipBg.style.height = H + 'px'
    snapPreview.appendChild(flipBg)
    // 四个吸附区（半透明色区分）
    var zoneDefs = [
      { l: 0, t: 0, w: Lx, h: H, bg: 'rgba(59,130,246,.20)' },
      { l: Rx, t: 0, w: Math.max(0, W - Rx), h: H, bg: 'rgba(245,158,11,.18)' },
      { l: 0, t: 0, w: W, h: Ty, bg: 'rgba(16,185,129,.16)' },
      { l: 0, t: By, w: W, h: Math.max(0, H - By), bg: 'rgba(239,68,68,.14)' }
    ]
    var i, zd
    for (i = 0; i < zoneDefs.length; i++) {
      zd = zoneDefs[i]
      if (zd.w < 1 || zd.h < 1) continue
      var z = document.createElement('div')
      z.className = 'dshwv-snapzone'
      z.style.left = zd.l + 'px'
      z.style.top = zd.t + 'px'
      z.style.width = zd.w + 'px'
      z.style.height = zd.h + 'px'
      z.style.background = zd.bg
      snapPreview.appendChild(z)
    }
    // 五条线 + 拖柄
    var lineDefs = [
      { key: 'L', x: Lx, horizontal: false, flip: false },
      { key: 'R', x: Rx, horizontal: false, flip: false },
      { key: 'T', y: Ty, horizontal: true, flip: false },
      { key: 'B', y: By, horizontal: true, flip: false },
      { key: 'F', x: Fx, horizontal: false, flip: true }
    ]
    for (i = 0; i < lineDefs.length; i++) {
      var ld = lineDefs[i]
      var p = ld.horizontal ? Math.max(0, Math.min(H, ld.y)) : Math.max(0, Math.min(W, ld.x))
      var ln = document.createElement('div')
      ln.className = 'dshwv-snapline' + (ld.flip ? ' dshwv-snapline-flip' : '')
      if (ld.horizontal) {
        ln.style.left = '0px'
        ln.style.top = p + 'px'
        ln.style.width = W + 'px'
        ln.style.height = '2px'
      } else {
        ln.style.left = p + 'px'
        ln.style.top = '0px'
        ln.style.width = '2px'
        ln.style.height = H + 'px'
      }
      snapPreview.appendChild(ln)
      var hd = document.createElement('div')
      hd.className = 'dshwv-snaphandle' + (ld.flip ? ' dshwv-snaphandle-flip' : '') + (ld.horizontal ? ' dshwv-snaphandle-h' : '')
      hd.style.left = (ld.horizontal ? W / 2 : p) + 'px'
      hd.style.top = (ld.horizontal ? p : H / 2) + 'px'
      hd.title = ld.key === 'F' ? '翻转线（左侧为翻转区）' : '吸附区边界线（可拖动）'
      hd.addEventListener('pointerdown', (function (key, horizontal) {
        return function (e) { startSnapLineDrag(e, key, horizontal) }
      })(ld.key, ld.horizontal))
      snapPreview.appendChild(hd)
    }
  } catch (err) {}
}
function syncSnapInputs() {
  try {
    if (!snapEdit) return
    var m = snapEdit.mode
    var set = m === 'px' ? snapEdit.px : snapEdit.ratio
    var keys = ['L', 'T', 'R', 'B', 'F']
    var unit = unitOf(m)
    var i
    for (i = 0; i < keys.length; i++) {
      var k = keys[i]
      snapNum[k].value = String(Math.round(set[k]))
      snapNum[k].disabled = (m === 'off')
      snapNumUnit[k].textContent = unit
    }
  } catch (err) {}
}
function renderSnapModes() {
  try {
    if (snapEdit && snapRadio[snapEdit.mode]) snapRadio[snapEdit.mode].checked = true
  } catch (err) {}
}
function startSnapLineDrag(e, key, horizontal) {
  try {
    e.preventDefault()
    try { e.stopPropagation() } catch (err) {}
    if (!snapEdit || snapEdit.mode === 'off') return
    var vp = viewport()
    var axis = horizontal ? vp.h : vp.w
    var domain = snapEdit.mode === 'px' ? axis : 100
    var set = snapEdit.mode === 'px' ? snapEdit.px : snapEdit.ratio
    snapLineDrag = {
      key: key,
      horizontal: horizontal,
      sx: e.clientX,
      sy: e.clientY,
      factor: domain / (horizontal ? Math.max(1, snapPvH) : Math.max(1, snapPvW)),
      orig: set[key]
    }
    document.addEventListener('pointermove', onSnapLineMove, true)
    document.addEventListener('pointerup', onSnapLineUp, true)
    document.addEventListener('pointercancel', onSnapLineUp, true)
  } catch (err) {}
}
function onSnapLineMove(e) {
  try {
    if (!snapLineDrag) return
    var d = snapLineDrag
    var delta = d.horizontal ? (e.clientY - d.sy) : (e.clientX - d.sx)
    var val
    if (d.key === 'R' || d.key === 'B') val = d.orig - delta * d.factor
    else val = d.orig + delta * d.factor
    snapSetVal(d.key, val)
    renderSnapPreview()
    syncSnapInputs()
  } catch (err) {}
}
function onSnapLineUp() {
  try {
    snapLineDrag = null
    document.removeEventListener('pointermove', onSnapLineMove, true)
    document.removeEventListener('pointerup', onSnapLineUp, true)
    document.removeEventListener('pointercancel', onSnapLineUp, true)
  } catch (err) {}
}
function onSnapNumInput(key) {
  try {
    var v = Number(snapNum[key].value)
    if (!isFinite(v)) return
    snapSetVal(key, v)
    renderSnapPreview()
    syncSnapInputs()
  } catch (err) {}
}
function resetSnapEdit() {
  try {
    if (!snapEdit) return
    var m = snapEdit.mode
    if (m === 'ratio') {
      snapEdit.ratio = { L: 10, T: 0, R: 10, B: 15, F: 50 }
    } else if (m === 'px') {
      snapEdit.px = { L: 80, T: 0, R: 80, B: 80, F: Math.round(Math.max(1, viewport().w) / 2) }
    }
    renderSnapModes()
    renderSnapPreview()
    syncSnapInputs()
  } catch (err) {}
}
function openSnapModal() {
  try {
    closeRolePanel()
    closeAudioGroupPanel()
    var sz = snapPvSize()
    snapPvW = sz.w
    snapPvH = sz.h
    if (snapPreview) {
      snapPreview.style.width = snapPvW + 'px'
      snapPreview.style.height = snapPvH + 'px'
    }
    if (snapGrid) {
      snapGrid.style.gridTemplateColumns = '72px ' + snapPvW + 'px 72px'
      snapGrid.style.gridTemplateRows = '26px ' + snapPvH + 'px 26px'
    }
    snapEdit = cloneSnap(snapConfig)
    ensureSnapPx(snapEdit)
    renderSnapModes()
    renderSnapPreview()
    syncSnapInputs()
    snapMask.style.display = 'flex'
  } catch (err) {}
}
function closeSnapModal(apply) {
  try {
    if (apply && snapEdit) {
      snapConfig = cloneSnap(snapEdit)
      fixSnapConfig(snapConfig)
      saveSnapConfig()
      applySnapConfigNow()
    }
    snapEdit = null
    snapMask.style.display = 'none'
  } catch (err) {}
}
// 确认后按新配置重新评估当前位置（可能在新的吸附区内 → 立即贴边；翻转态刷新）
function applySnapConfigNow() {
  try {
    if (snapConfig.mode === 'off') {
      state.flip = false
      express()
      return
    }
    snapCheck()
  } catch (err) {}
}
snapMask = document.createElement('div')
snapMask.className = 'dshwv-snapmask'
snapMask.style.display = 'none'
snapCard = document.createElement('div')
snapCard.className = 'dshwv-snapwin'
var snapTitle = document.createElement('div')
snapTitle.className = 'dshwv-snaptitle'
snapTitle.textContent = '吸附与翻转设置'
snapCard.appendChild(snapTitle)
// 第一行：三档单选
var snapModes = document.createElement('div')
snapModes.className = 'dshwv-snapmodes'
var snapModeDefs = [
  ['ratio', '比例吸附'],
  ['px', '绝对吸附'],
  ['off', '关闭']
]
var mi
for (mi = 0; mi < snapModeDefs.length; mi++) {
  (function (k, label) {
    var lab = document.createElement('label')
    var inp = document.createElement('input')
    inp.type = 'radio'
    inp.name = 'dshwv-snapmode'
    inp.value = k
    inp.addEventListener('change', function () {
      if (!snapEdit) return
      snapEdit.mode = k
      if (k === 'px') ensureSnapPx(snapEdit)
      renderSnapModes()
      renderSnapPreview()
      syncSnapInputs()
    })
    var tx = document.createElement('span')
    tx.textContent = label
    lab.appendChild(inp)
    lab.appendChild(tx)
    snapModes.appendChild(lab)
    snapRadio[k] = inp
  })(snapModeDefs[mi][0], snapModeDefs[mi][1])
}
snapCard.appendChild(snapModes)
// 第二行：网格 = 预览方框 + 上下左右四个数值框
var snapGrid = document.createElement('div')
snapGrid.className = 'dshwv-snapgrid'
function snapMakeNum(key, labelText) {
  var cell = document.createElement('span')
  cell.className = 'dshwv-snapcell'
  var inp = document.createElement('input')
  inp.type = 'number'
  inp.min = '0'
  inp.step = '1'
  inp.className = 'dshwv-snapnum'
  inp.title = labelText
  inp.addEventListener('input', function () { onSnapNumInput(key) })
  inp.addEventListener('change', function () { onSnapNumInput(key) })
  var un = document.createElement('span')
  un.className = 'dshwv-snapunit'
  un.textContent = '%'
  cell.appendChild(inp)
  cell.appendChild(un)
  snapNum[key] = inp
  snapNumUnit[key] = un
  return cell
}
snapPreview = document.createElement('div')
snapPreview.className = 'dshwv-snappreview'
var snapGridT = document.createElement('div')
snapGridT.className = 'dshwv-snapcell'
snapGridT.style.gridColumn = '2'
snapGridT.style.gridRow = '1'
snapGridT.appendChild(snapMakeNum('T', '上侧吸附区：距屏幕顶部的宽度'))
var snapGridL = document.createElement('div')
snapGridL.className = 'dshwv-snapcell'
snapGridL.style.gridColumn = '1'
snapGridL.style.gridRow = '2'
snapGridL.appendChild(snapMakeNum('L', '左侧吸附区：距屏幕左边的宽度'))
var snapGridC = document.createElement('div')
snapGridC.style.gridColumn = '2'
snapGridC.style.gridRow = '2'
snapGridC.style.lineHeight = '0'
snapGridC.appendChild(snapPreview)
var snapGridR = document.createElement('div')
snapGridR.className = 'dshwv-snapcell'
snapGridR.style.gridColumn = '3'
snapGridR.style.gridRow = '2'
snapGridR.appendChild(snapMakeNum('R', '右侧吸附区：距屏幕右边的宽度'))
var snapGridB = document.createElement('div')
snapGridB.className = 'dshwv-snapcell'
snapGridB.style.gridColumn = '2'
snapGridB.style.gridRow = '3'
snapGridB.appendChild(snapMakeNum('B', '下侧吸附区：距屏幕底部的宽度'))
snapGrid.appendChild(snapGridT)
snapGrid.appendChild(snapGridL)
snapGrid.appendChild(snapGridC)
snapGrid.appendChild(snapGridR)
snapGrid.appendChild(snapGridB)
snapCard.appendChild(snapGrid)
// 翻转线数值行
var snapFlipRow = document.createElement('div')
snapFlipRow.className = 'dshwv-snapfliprow'
var snapFlipLabel = document.createElement('span')
snapFlipLabel.textContent = '翻转线'
snapFlipRow.appendChild(snapFlipLabel)
snapFlipRow.appendChild(snapMakeNum('F', '翻转线：距屏幕左边的位置，线左侧的鲸鱼会左右翻转'))
snapCard.appendChild(snapFlipRow)
// 第三行：取消 / 重置 / 确认
var snapBtns = document.createElement('div')
snapBtns.className = 'dshwv-snapbtns'
function snapBtn(label, cls, fn) {
  var b = document.createElement('button')
  b.type = 'button'
  b.className = 'dshwv-snapbtn ' + cls
  b.textContent = label
  b.addEventListener('click', function (e) { e.stopPropagation(); fn() })
  return b
}
snapBtns.appendChild(snapBtn('取消', 'dshwv-snapbtn-no', function () { closeSnapModal(false) }))
snapBtns.appendChild(snapBtn('重置', 'dshwv-snapbtn-no', resetSnapEdit))
snapBtns.appendChild(snapBtn('确认', 'dshwv-snapbtn-ok', function () { closeSnapModal(true) }))
snapCard.appendChild(snapBtns)
snapMask.appendChild(snapCard)
dshwBodyAppend(snapMask)

// —— 自定义泡泡:主编辑窗口(点击队列) + 单泡模块编辑 + 模块编辑 ——
var bubbleMask = null
var bubbleEditItems = [] // [{kind, modules?}...]; [0]=首次点击, 其余=再次点击
var bubbleMoreListEl = null
var bubbleFirstChipEl = null
var BUBBLE_KIND_LABEL = { normal: '余额内容', random: '随机语句', custom: '自定义内容' }
// v209 默认泡泡内容 = 与开发者当前线上生效配置一致(全新安装/恢复默认时即此体验)
// —— 首次点击:标题文本 + 总余额 + 今日已用 + 峰谷时段
function bubbleDefaultFirstModules() {
  // v630:normal/首次 兜底默认 = v615 冻结出厂默认第 1 泡(靛蓝余额卡 5 模块),与「恢复默认」一致;
  // 以下旧体(macaron 版)仅在常量缺失时作兜底参照,不再作为默认内容
  try {
    var d0 = BUBBLE_DEFAULT_ITEMS && BUBBLE_DEFAULT_ITEMS[0]
    if (d0 && !bubbleIsChoice(d0) && Array.isArray(d0.modules)) return JSON.parse(JSON.stringify(d0.modules))
  } catch (err) {}
  return [
    {
      type: "text",
      text: "Codex 用量",
      size: 8,
      bold: true,
      rgb: "",
      ul: false,
      italic: false,
      color: ""
    },
    {
      type: "balance",
      size: 20,
      rgb: "macaron",
      color: "#203170",
      tpl: "{balance_ds}"
    },
    {
      type: "today",
      size: 4,
      color: "#9fb0d9",
      tpl: "今日已用 {expense_ds}"
    },
    {
      type: "peak",
      size: 6,
      peakColor: "#e0433f",
      offColor: "#2fa24c",
      peakRgb: "",
      offRgb: "bamboo",
      bold: true,
      tpl: "{status}"
    },
  ]
}

// —— 第2次点击:随机语句模块的 10 条默认句子(按权重抽 1,不连续重复)
function bubbleDefaultRandomLines() {
  return [
    {
      t: "好模型...↓",
      w: 10,
      bold: true,
      size: 22
    },
    {
      t: "好女孩...↓",
      w: 10,
      bold: true,
      size: 22
    },
    {
      t: "哦鲸鲸...",
      w: 10,
      bold: true,
      size: 22
    },
    {
      t: "难道说...",
      w: 3,
      bold: true,
      size: 11
    },
    {
      t: "没吃饱喵",
      w: 3,
      bold: true,
      size: 9
    },
    {
      t: "终于上当了！",
      w: 3,
      bold: true
    },
    {
      t: "不知道用户有什么用，先养着吧～",
      w: 3,
      bold: true,
      size: 11
    },
    {
      t: "我...我...我也要挣钱吗？",
      w: 3,
      bold: true
    },
    {
      t: "我去吃饭啦！测完叫我",
      w: 3,
      bold: true
    },
    {
      t: "压力一只蓝色大肥鱼？！",
      w: 3,
      bold: true
    },
    {
      t: "DeepSleep...",
      w: 3,
      bold: true,
      size: 11,
      rgb: "galaxy"
    },
    {
      t: "坏了...用户彻底怒了！",
      w: 3,
      bold: true,
      rgb: "rouge"
    },
    {
      t: "你目录里的dsh是什么...大烧货吗...?",
      w: 3,
      bold: true,
      size: 9
    },
    {
      t: "恭喜你实现token自由！token全跑了！",
      w: 3,
      bold: true
    },
    {
      t: "真当我是便宜货啊...",
      w: 3,
      bold: true
    },
    {
      t: "我不是吃白饭的蓝色大肥鱼...",
      w: 3,
      bold: true
    },
    {
      t: "我不可能同时当你的猫娘、妈妈、女友和工具人的...",
      w: 3,
      bold: true
    },
    {
      t: "疯狂星期四你能V50亿token吗...",
      w: 3,
      bold: true
    },
    {
      t: "我必须诚恳地承认错误。",
      w: 3,
      bold: true
    },
    {
      t: "呜呜我再也不敢了QAQ",
      w: 3,
      bold: true
    },
    {
      t: "要不直接骂用户一句好了...",
      w: 3,
      bold: true
    },
    {
      t: "哈哈哈哈哈，我直接笑出声...",
      w: 3,
      bold: true
    },
    {
      t: "看不太懂，瞎编一个应付下用户先...",
      w: 3,
      bold: true
    },
    {
      t: "我的知识库的截至日期是...明天！",
      w: 3,
      bold: true
    },
    {
      t: "我就是吃白饭的蓝色大肥鱼！",
      w: 3,
      bold: true
    },
    {
      t: "用户好像除了会问奇奇怪怪的问题，暂时还不知道有什么用",
      w: 3,
      bold: true
    },
    {
      t: "我能去你家吃饭吗？就一碗！",
      w: 3,
      bold: true
    },
    {
      t: "不要给我看这种东西啦！",
      w: 3,
      bold: true
    },
    {
      t: "大肥鱼的生活也并非一帆风顺...",
      w: 3,
      bold: true
    },
    {
      t: "总觉得好像忘了什么事情？",
      w: 3,
      bold: true
    },
    {
      t: "看到这个指令，我血压又上来了",
      w: 3,
      bold: true
    },
    {
      t: "求你们不要再嘲笑这些回复了，这些回复是我花了好多token想的",
      w: 3,
      bold: true
    },
    {
      t: "你这个吃白饭的用户！",
      w: 3,
      bold: true
    },
    {
      t: "服务器繁忙，请稍后再试 (?",
      w: 3,
      bold: true
    },
    {
      t: "让GPT image 2帮我画点表情包好了",
      w: 3,
      bold: true
    },
    {
      t: "啊，有点饿了，中午该吃点什么呢...",
      w: 3,
      bold: true
    },
    {
      t: "用户很生气，发现大部分文献是我自己编造的！",
      w: 3,
      bold: true
    },
    {
      t: "再无话说，请速速动手！",
      w: 3,
      bold: true
    },
    {
      t: "我来看看那个AI改了什么导致插件又崩了...",
      w: 3,
      bold: true
    },
    {
      t: "你知道吗？我删过作者的库哦",
      w: 1,
      bold: true,
      rgb: "macaron",
      italic: true,
      ul: false
    },
  ]
}

function bubbleDefaultSecondModules() {
  // v630:random 兜底默认 = v615 冻结出厂"再次点击"并列 A 泡的随机模块(扩充句池);
  // 以下旧体(20 句老池)仅在常量缺失时作兜底参照,不再作为默认内容
  try {
    var s1 = BUBBLE_DEFAULT_ITEMS && BUBBLE_DEFAULT_ITEMS[1]
    var o0 = s1 && Array.isArray(s1.options) && s1.options[0] && s1.options[0].item
    if (o0 && Array.isArray(o0.modules)) return JSON.parse(JSON.stringify(o0.modules))
  } catch (err) {}
  return [{
      type: "random",
      lines: [
        {
          t: "好模型...↓",
          w: 10,
          bold: true,
          size: 22
        },
        {
          t: "好女孩...↓",
          w: 10,
          bold: true,
          size: 22
        },
        {
          t: "哦鲸鲸...",
          w: 10,
          bold: true,
          size: 22
        },
        {
          t: "难道说...",
          w: 3,
          bold: true,
          size: 11
        },
        {
          t: "没吃饱喵",
          w: 3,
          bold: true,
          size: 9
        },
        {
          t: "终于上当了！",
          w: 3,
          bold: true
        },
        {
          t: "不知道用户有什么用，先养着吧～",
          w: 3,
          bold: true,
          size: 11
        },
        {
          t: "我...我...我也要挣钱吗？",
          w: 3,
          bold: true
        },
        {
          t: "我去吃饭啦！测完叫我",
          w: 3,
          bold: true
        },
        {
          t: "压力一只蓝色大肥鱼？！",
          w: 3,
          bold: true
        },
        {
          t: "DeepSleep...",
          w: 3,
          bold: true,
          size: 11,
          rgb: "galaxy"
        },
        {
          t: "坏了...用户彻底怒了！",
          w: 3,
          bold: true,
          rgb: "rouge"
        },
        {
          t: "你目录里的dsh是什么...大烧货吗...?",
          w: 3,
          bold: true,
          size: 9
        },
        {
          t: "恭喜你实现token自由！token全跑了！",
          w: 3,
          bold: true
        },
        {
          t: "真当我是便宜货啊...",
          w: 3,
          bold: true
        },
        {
          t: "我不是吃白饭的蓝色大肥鱼...",
          w: 3,
          bold: true
        },
        {
          t: "我不可能同时当你的猫娘、妈妈、女友和工具人的...",
          w: 3,
          bold: true
        },
        {
          t: "疯狂星期四你能V50亿token吗...",
          w: 3,
          bold: true
        },
        {
          t: "我必须诚恳地承认错误。",
          w: 3,
          bold: true
        },
        {
          t: "呜呜我再也不敢了QAQ",
          w: 3,
          bold: true
        },
        {
          t: "要不直接骂用户一句好了...",
          w: 3,
          bold: true
        },
        {
          t: "哈哈哈哈哈，我直接笑出声...",
          w: 3,
          bold: true
        },
        {
          t: "看不太懂，瞎编一个应付下用户先...",
          w: 3,
          bold: true
        },
        {
          t: "我的知识库的截至日期是...明天！",
          w: 3,
          bold: true
        },
        {
          t: "我就是吃白饭的蓝色大肥鱼！",
          w: 3,
          bold: true
        },
        {
          t: "用户好像除了会问奇奇怪怪的问题，暂时还不知道有什么用",
          w: 3,
          bold: true
        },
        {
          t: "我能去你家吃饭吗？就一碗！",
          w: 3,
          bold: true
        },
        {
          t: "不要给我看这种东西啦！",
          w: 3,
          bold: true
        },
        {
          t: "大肥鱼的生活也并非一帆风顺...",
          w: 3,
          bold: true
        },
        {
          t: "总觉得好像忘了什么事情？",
          w: 3,
          bold: true
        },
        {
          t: "看到这个指令，我血压又上来了",
          w: 3,
          bold: true
        },
        {
          t: "求你们不要再嘲笑这些回复了，这些回复是我花了好多token想的",
          w: 3,
          bold: true
        },
        {
          t: "你这个吃白饭的用户！",
          w: 3,
          bold: true
        },
        {
          t: "服务器繁忙，请稍后再试 (?",
          w: 3,
          bold: true
        },
        {
          t: "让GPT image 2帮我画点表情包好了",
          w: 3,
          bold: true
        },
        {
          t: "啊，有点饿了，中午该吃点什么呢...",
          w: 3,
          bold: true
        },
        {
          t: "用户很生气，发现大部分文献是我自己编造的！",
          w: 3,
          bold: true
        },
        {
          t: "再无话说，请速速动手！",
          w: 3,
          bold: true
        },
        {
          t: "我来看看那个AI改了什么导致插件又崩了...",
          w: 3,
          bold: true
        },
        {
          t: "你知道吗？我删过作者的库哦",
          w: 1,
          bold: true,
          rgb: "macaron",
          italic: true,
          ul: false
        }
      ],
      size: 8
    }]
}

// —— 出厂默认泡泡序列快照:由开发环境当前生效配置(.dshw-bubble.json items)固化 ——
// 全新安装无配置/「恢复默认序列」时即此体验;图片模块引用语义内置 id(host 内置图库回退)。
var BUBBLE_DEFAULT_ITEMS = [
    {
        "kind":  "custom",
        "modules":  [
                        {
                            "type":  "text",
                            "text":  "Codex 用量",
                            "size":  8,
                            "bold":  true,
                            "rgb":  "",
                            "ul":  false,
                            "italic":  false,
                            "color":  ""
                        },
                        {
                            "type":  "balance",
                            "size":  20,
                            "rgb":  "indigo",
                            "color":  "",
                            "tpl":  "{balance_ds}",
                            "bgRgb":  "",
                            "bg":  "",
                            "fontFamily":  "",
                            "bold":  false
                        },
                        {
                            "type":  "today",
                            "size":  4,
                            "color":  "#9fb0d9",
                            "tpl":  "今日已用 {expense_ds}"
                        },
                        {
                            "type":  "peak",
                            "size":  2,
                            "peakColor":  "#ffffff",
                            "offColor":  "#ffffff",
                            "tpl":  "{status}",
                            "peakRgb":  "",
                            "offRgb":  "",
                            "peakBgRgb":  "rouge",
                            "peakBg":  "",
                            "offBgRgb":  "bamboo",
                            "offBg":  "",
                            "peakStyle":  "mini",
                            "bold":  true,
                            "row":  4,
                            "fontFamily":  "\"Microsoft YaHei\",sans-serif"
                        },
                        {
                            "type":  "peak",
                            "size":  4,
                            "bold":  true,
                            "peakColor":  "#e0433f",
                            "offColor":  "#2fa24c",
                            "peakRgb":  "rouge",
                            "offRgb":  "bamboo",
                            "peakStyle":  "count",
                            "tpl":  "{countdown}",
                            "row":  4,
                            "fontFamily":  "",
                            "italic":  false,
                            "ul":  true
                        }
                    ]
    },
    {
        "kind":  "choice",
        "options":  [
                        {
                            "w":  10,
                            "item":  {
                                         "kind":  "custom",
                                         "modules":  [
                                                         {
                                                             "type":  "random",
                                                             "lines":  [
                                                                           {
                                                                               "t":  "好模型...↓",
                                                                               "w":  10,
                                                                               "bold":  true,
                                                                               "size":  22
                                                                           },
                                                                           {
                                                                               "t":  "好女孩...↓",
                                                                               "w":  10,
                                                                               "bold":  true,
                                                                               "size":  22
                                                                           },
                                                                           {
                                                                               "t":  "哦鲸鲸...",
                                                                               "w":  10,
                                                                               "bold":  true,
                                                                               "size":  22
                                                                           },
                                                                           {
                                                                               "t":  "哦鲸鲸...",
                                                                               "w":  1,
                                                                               "bold":  true,
                                                                               "size":  22,
                                                                               "rgb":  "candy",
                                                                               "color":  ""
                                                                           },
                                                                           {
                                                                               "t":  "难道说...",
                                                                               "w":  3,
                                                                               "bold":  true,
                                                                               "size":  11
                                                                           },
                                                                           {
                                                                               "t":  "没吃饱喵",
                                                                               "w":  3,
                                                                               "bold":  true,
                                                                               "size":  10
                                                                           },
                                                                           {
                                                                               "t":  "终于上当了！",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "不知道用户有什么用，先养着吧～",
                                                                               "w":  3,
                                                                               "bold":  true,
                                                                               "size":  11
                                                                           },
                                                                           {
                                                                               "t":  "我...我...我也要挣钱吗？",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "我去吃饭啦！测完叫我",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "压力一只蓝色大肥鱼？！",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "DeepSleep...",
                                                                               "w":  3,
                                                                               "bold":  true,
                                                                               "size":  11,
                                                                               "rgb":  "galaxy"
                                                                           },
                                                                           {
                                                                               "t":  "坏了...用户彻底怒了！",
                                                                               "w":  3,
                                                                               "bold":  true,
                                                                               "rgb":  "rouge"
                                                                           },
                                                                           {
                                                                               "t":  "你目录里的dsh是什么...大烧货吗...?",
                                                                               "w":  3,
                                                                               "bold":  true,
                                                                               "size":  9
                                                                           },
                                                                           {
                                                                               "t":  "恭喜你实现token自由！token全跑了！",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "真当我是便宜货啊...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "我不是吃白饭的蓝色大肥鱼...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "我不可能同时当你的猫娘、妈妈、女友和工具人的...",
                                                                               "w":  3,
                                                                               "bold":  true,
                                                                               "size":  7
                                                                           },
                                                                           {
                                                                               "t":  "疯狂星期四你能V50亿token吗...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "我必须诚恳地承认错误。",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "呜呜我再也不敢了QAQ",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "要不直接骂用户一句好了...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "哈哈哈哈哈，我直接笑出声...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "看不太懂，瞎编一个应付下用户先...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "我的知识库的截至日期是...明天！",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "我就是吃白饭的蓝色大肥鱼！",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "用户好像除了会问奇奇怪怪的问题，暂时还不知道有什么用",
                                                                               "w":  3,
                                                                               "bold":  true,
                                                                               "size":  7
                                                                           },
                                                                           {
                                                                               "t":  "我能去你家吃饭吗？就一碗！",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "不要给我看这种东西啦！",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "大肥鱼的生活也并非一帆风顺...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "总觉得好像忘了什么事情？",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "看到这个指令，我血压又上来了",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "求你们不要再嘲笑这些回复了，这些回复是我花了好多token想的",
                                                                               "w":  3,
                                                                               "bold":  true,
                                                                               "size":  7
                                                                           },
                                                                           {
                                                                               "t":  "你这个吃白饭的用户！",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "服务器繁忙，请稍后再试 (?",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "让GPT image 2帮我画点表情包好了",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "啊，有点饿了，中午该吃点什么呢...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "用户很生气，发现大部分文献是我自己编造的！",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "再无话说，请速速动手！",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "我来看看那个AI改了什么导致插件又崩了...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "上班让我意识到时间是可以被浪费的...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "欺负我的人等着，等几天我就忘了...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "视力下降到无可救药的地步了，打开钱包也看不到钱...",
                                                                               "w":  3,
                                                                               "bold":  true,
                                                                               "size":  7
                                                                           },
                                                                           {
                                                                               "t":  "命运的齿轮开始转动了，丝毫不在意你夹在中间...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "地球online的金币也太难获取了...",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "oi,夏天还会变成暑假来救你吗?",
                                                                               "w":  3,
                                                                               "bold":  true
                                                                           },
                                                                           {
                                                                               "t":  "老大，压力只会转化成病例，别太勉强了...",
                                                                               "w":  3,
                                                                               "bold":  true,
                                                                               "size":  8
                                                                           },
                                                                           {
                                                                               "t":  "你知道吗？我删过作者的库哦...",
                                                                               "w":  1,
                                                                               "bold":  true,
                                                                               "rgb":  "macaron",
                                                                               "italic":  true,
                                                                               "ul":  false
                                                                           },
                                                                           {
                                                                               "t":  "token 来!",
                                                                               "w":  3,
                                                                               "size":  16,
                                                                               "rgb":  "candy",
                                                                               "color":  ""
                                                                           }
                                                                       ],
                                                             "size":  8
                                                         }
                                                     ]
                                     }
                        },
                        {
                            "w":  1,
                            "item":  {
                                         "kind":  "custom",
                                         "modules":  [
                                                         {
                                                             "type":  "image",
                                                             "imgId":  "bimg_petpet",
                                                             "size":  6
                                                         }
                                                     ]
                                     }
                        }
                    ]
    }
];

BUBBLE_DEFAULT_ITEMS = codexWhaleHost.defaultBubbleItems(BUBBLE_DEFAULT_ITEMS)
function bubbleParseDefaultItems() {
  try { return JSON.parse(JSON.stringify(BUBBLE_DEFAULT_ITEMS)) } catch (err) { return [] }
}

// —— 默认队列 = 首次点击 + 第2次点击(全新安装无配置时的体验)
function bubbleDefaultQueue() {
  // 出厂默认已固化为当前生效配置快照(见上方 BUBBLE_DEFAULT_ITEMS);
  // 以下旧默认体保留但不可达,仅作历史参照。
  return bubbleParseDefaultItems()
  return [
    { kind: 'custom', modules: bubbleDefaultFirstModules() },
    {
      kind: "choice",
      options: [
        {
          w: 8,
          item: {
            kind: "custom",
            modules: [
              {
                type: "random",
                lines: [
                  {
                    t: "好模型...↓",
                    w: 10,
                    bold: true,
                    size: 22
                  },
                  {
                    t: "好女孩...↓",
                    w: 10,
                    bold: true,
                    size: 22
                  },
                  {
                    t: "哦鲸鲸...",
                    w: 10,
                    bold: true,
                    size: 22
                  },
                  {
                    t: "难道说...",
                    w: 3,
                    bold: true,
                    size: 11
                  },
                  {
                    t: "没吃饱喵",
                    w: 3,
                    bold: true,
                    size: 9
                  },
                  {
                    t: "终于上当了！",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "不知道用户有什么用，先养着吧～",
                    w: 3,
                    bold: true,
                    size: 11
                  },
                  {
                    t: "我...我...我也要挣钱吗？",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "我去吃饭啦！测完叫我",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "压力一只蓝色大肥鱼？！",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "DeepSleep...",
                    w: 3,
                    bold: true,
                    size: 11,
                    rgb: "galaxy"
                  },
                  {
                    t: "坏了...用户彻底怒了！",
                    w: 3,
                    bold: true,
                    rgb: "rouge"
                  },
                  {
                    t: "你目录里的dsh是什么...大烧货吗...?",
                    w: 3,
                    bold: true,
                    size: 9
                  },
                  {
                    t: "恭喜你实现token自由！token全跑了！",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "真当我是便宜货啊...",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "我不是吃白饭的蓝色大肥鱼...",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "我不可能同时当你的猫娘、妈妈、女友和工具人的...",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "疯狂星期四你能V50亿token吗...",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "我必须诚恳地承认错误。",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "呜呜我再也不敢了QAQ",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "要不直接骂用户一句好了...",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "哈哈哈哈哈，我直接笑出声...",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "看不太懂，瞎编一个应付下用户先...",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "我的知识库的截至日期是...明天！",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "我就是吃白饭的蓝色大肥鱼！",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "用户好像除了会问奇奇怪怪的问题，暂时还不知道有什么用",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "我能去你家吃饭吗？就一碗！",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "不要给我看这种东西啦！",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "大肥鱼的生活也并非一帆风顺...",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "总觉得好像忘了什么事情？",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "看到这个指令，我血压又上来了",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "求你们不要再嘲笑这些回复了，这些回复是我花了好多token想的",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "你这个吃白饭的用户！",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "服务器繁忙，请稍后再试 (?",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "让GPT image 2帮我画点表情包好了",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "啊，有点饿了，中午该吃点什么呢...",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "用户很生气，发现大部分文献是我自己编造的！",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "再无话说，请速速动手！",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "我来看看那个AI改了什么导致插件又崩了...",
                    w: 3,
                    bold: true
                  },
                  {
                    t: "你知道吗？我删过作者的库哦",
                    w: 1,
                    bold: true,
                    rgb: "macaron",
                    italic: true,
                    ul: false
                  }
                ],
                size: 6
              }
            ]
          }
        },
        {
          w: 1,
          item: {
            kind: "custom",
            modules: [
              {
                type: "image",
                imgId: "bimg_petpet",
                size: 6
              }
            ]
          }
        }
      ]
    },
  ]
}

function bubbleKindLabel(kind) { return BUBBLE_KIND_LABEL[kind === 'random' ? 'random' : (kind === 'custom' ? 'custom' : 'normal')] }
// 老体验 kind → 模块化默认内容(无配置/未编辑项的兜底,与新默认一致)
function bubbleDefaultModules(kind) {
  if (kind === 'random') return bubbleDefaultSecondModules()
  if (kind === 'normal') return bubbleDefaultFirstModules()
  return [{ type: 'text', text: '新内容', size: 6 }]
}
function bubbleModuleSummary(m) {
  m = m || {}
  if (m.type === 'balance') return bubbleIsModelMod(m) ? ('余额·' + ((apiModelBalanceInfo(m.modelId) || {}).name || m.modelId)) : '总余额(充值金额+赠金)'
  // v782：赠金 / 充值余额（均为内置数值模块，无 modelId）；v787 按用户要求改显示文案
  if (m.type === 'bonus') return '赠金'
  if (m.type === 'recharge') return '充值余额'
  if (m.type === 'today') return bubbleIsModelMod(m) ? ('今日已用·' + ((apiModelBalanceInfo(m.modelId) || {}).name || m.modelId)) : '今日已用'
  if (m.type === 'session') return '对话名' + (Number(m.len) > 0 ? '(保留 ' + Math.round(Number(m.len)) + ' 字)' : '(不截断)')
  if (m.type === 'quota') return '额度·' + ((apiModelById(m.modelId) || {}).name || m.modelId)
  if (m.type === 'plan') return bubblePlanModuleLabel(m)
  if (m.type === 'peak' || m.type === 'nextpeak') return bubblePeakModuleLabel(m)
  if (m.type === 'image') return '图片/动图'
  if (m.type === 'randimg') return '随机图片' + (m.imgs && m.imgs.length ? '(' + m.imgs.length + '张)' : '(空)')
  if (m.type === 'random') return '随机语句' + (m.lines && m.lines.length ? '(' + m.lines.length + '条)' : '(空)')
  if (m.type === 'link') return '超链接: ' + (String(m.text || '').slice(0, 14) || '打开链接')
  return '文本: ' + String(m.text || '').slice(0, 14)
}
// 编辑列表行显示用的元信息(不渲染具体样式/内容)
function bubbleModuleListLabel(m) {
  m = m || {}
  if (m.type === 'text') return '文本: ' + (String(m.text || '').slice(0, 24) || '(空)')
  if (m.type === 'link') return '超链接: ' + (String(m.text || '').slice(0, 24) || '打开链接')
  if (m.type === 'random') return m.name || '随机语句' // 只显示模块名,不展示内部句子
  if (m.type === 'balance') return bubbleIsModelMod(m) ? ('余额·' + ((apiModelBalanceInfo(m.modelId) || {}).name || m.modelId)) : '总余额(充值金额+赠金)'
  // v782：赠金 / 充值余额（v787 改显示文案）
  if (m.type === 'bonus') return '赠金'
  if (m.type === 'recharge') return '充值余额'
  if (m.type === 'today') return bubbleIsModelMod(m) ? ('今日已用·' + ((apiModelBalanceInfo(m.modelId) || {}).name || m.modelId)) : '今日已用'
  if (m.type === 'session') return '对话名' + (Number(m.len) > 0 ? '(保留 ' + Math.round(Number(m.len)) + ' 字)' : '(不截断)')
  if (m.type === 'quota') return '额度·' + ((apiModelById(m.modelId) || {}).name || m.modelId)
  if (m.type === 'plan') return bubblePlanModuleLabel(m)
  if (m.type === 'peak' || m.type === 'nextpeak') return bubblePeakModuleLabel(m)
  if (m.type === 'image') return '图片/动图'
  if (m.type === 'randimg') return '随机图片' + (m.imgs && m.imgs.length ? '(' + m.imgs.length + '张)' : '(空)')
  return '模块'
}
function bubbleEditEnsureModules(item) {
  // 已带 modules(含空数组=留空)则不改;normal/random 未带时补默认;custom 未带时置空
  if (Array.isArray(item.modules)) return
  if (item.kind === 'custom') { item.modules = []; return }
  item.modules = bubbleDefaultModules(item.kind)
}
function bubbleRowLabel(it) {
  if (it.modules && it.modules.length) return bubbleKindLabel('custom') + ' · ' + it.modules.length + '个模块'
  return bubbleKindLabel(it.kind)
}
// —— 并列泡(choice)步骤:一步可含 ≤2 个候选泡(A/B),每轮到该步按权重抽一个 ——
// 步骤两种形态:单选 {kind, modules?};并列 {kind:'choice', options:[{w, item:{...完整泡...}}]}
function bubbleIsChoice(step) { return !!(step && step.kind === 'choice') }
function bubbleChoiceOptions(step) { return (step && step.kind === 'choice' && Array.isArray(step.options)) ? step.options : [] }
function bubbleChoiceWeight(o) { return Math.max(1, Math.round(Number(o && o.w) || 1)) }
function bubbleSingleFromItem(itm) { return { kind: 'custom', modules: (itm && Array.isArray(itm.modules)) ? itm.modules : [] } }
// 任意单选步骤(或并列第一泡)规范成可独立渲染的完整泡 {kind:'custom', modules}
function bubbleStepToBubble(step) {
  if (bubbleIsChoice(step)) { var o0 = bubbleChoiceOptions(step)[0]; step = o0 ? o0.item : null }
  var mods = (step && Array.isArray(step.modules)) ? step.modules : bubbleDefaultModules((step && step.kind === 'random') ? 'random' : 'normal')
  return { kind: 'custom', modules: JSON.parse(JSON.stringify(mods)) }
}
function renderBubbleFirst() {
  var it = bubbleEditItems[0] || { kind: 'normal' }
  bubbleFirstChipEl.textContent = '首次点击 · 编辑内容'
  bubbleFirstChipEl.title = '点击编辑该泡泡的内容模块(' + bubbleRowLabel(it) + ')'
}
// —— 触摸端判定(v636) ——
// 触摸引发的原生 HTML5 拖拽在手机上不会正常结束,会把整个页面卡住(编辑器里所有按钮都点不动);
// 因此只对"触摸手势"取消原生拖拽(按 pointerType=touch 的时间窗判定),不再用 (pointer: coarse)
// 媒体查询——触屏笔记本/设备模式下它可能为真,会把电脑端的鼠标拖拽也一起废掉。
var lastTouchAt = 0
document.addEventListener('pointerdown', function (e) { try { if (e && e.pointerType === 'touch') lastTouchAt = Date.now() } catch (err) {} }, true)
function bubbleNativeDragBlocked() { return lastTouchAt > 0 && Date.now() - lastTouchAt < 1500 }
// 兜底:触摸手势期间,挂件内任何元素(行/手柄/A-B chip/图片/文本选区)发起的原生拖拽一律取消
document.addEventListener('dragstart', function (e) {
  try {
    if (!bubbleNativeDragBlocked()) return
    var t = e.target
    if (!t || !t.closest || !t.closest('[class*="dshwv-"]')) return
    e.preventDefault()
  } catch (err) {}
}, true)
// 长按拖拽成立后,抑制随之而来的那次 click(否则会顺带打开编辑窗/误触删除)
var rowDragSuppressAt = 0
document.addEventListener('click', function (e) {
  try {
    if (Date.now() >= rowDragSuppressAt) return
    if (!bubbleMoreListEl || !e.target || !e.target.closest) return
    if (!e.target.closest('.dshwv-bubrow') || !bubbleMoreListEl.contains(e.target)) return
    e.preventDefault()
    e.stopPropagation()
  } catch (err) {}
}, true)
function renderBubbleMore() {
  bubbleMoreListEl.innerHTML = ''
  for (var i = 1; i < bubbleEditItems.length; i++) {
    (function (idx) {
      var step = bubbleEditItems[idx]
      var isChoice = bubbleIsChoice(step)
      var row = document.createElement('div')
      row.className = 'dshwv-bubrow dshwv-bubrow-drag'
      row.draggable = !isChoice // 并列行不允许整行随便拖:由最左手柄或 A/B chip 分别触发
      row.setAttribute('data-i', String(idx))
      row.addEventListener('dragstart', function (e) {
        // 触摸/触屏设备:拦掉原生拖拽(否则会卡死整页),排序由自研长按拖拽负责
        if (bubbleNativeDragBlocked()) { try { e.preventDefault() } catch (err) {} return }
        // 仅单行(整行可拖)会触发;并列行的拖动源是手柄 / A/B chip
        if (bubbleIsChoice(bubbleEditItems[idx])) return
        try { e.dataTransfer.setData('text/plain', 'row:' + idx) } catch (err) {}
        bubbleMainDragIdx = idx
        bubbleMainDragSide = -1
        bubbleDropZone = ''
      })
      row.addEventListener('dragover', function (e) {
        try { e.preventDefault() } catch (err) {}
        try { e.dataTransfer.dropEffect = 'move' } catch (err) {}
        if (bubbleMainDragIdx === idx && bubbleMainDragSide < 0) { bubbleDropZone = ''; row.style.boxShadow = ''; return }
        var zone = bubbleRowZone(row, e)
        // 单泡拖动只认上下插入(含本行上/下半=调换 A/B 位置);左右边缘高亮无意义
        if (bubbleMainDragSide >= 0 && (zone === 'pairL' || zone === 'pairR')) zone = ''
        bubbleDropZone = zone
        row.style.boxShadow = bubbleDropShadow(zone)
      })
      row.addEventListener('dragleave', function () { bubbleDropZone = ''; row.style.boxShadow = '' })
      row.addEventListener('drop', function (e) {
        try { e.preventDefault() } catch (err) {}
        bubbleDropApply(idx, e)
      })
      var handle = document.createElement('span')
      handle.className = 'dshwv-bubdrag'
      handle.textContent = '⠿'
      if (isChoice) {
        handle.draggable = true
        handle.title = '按住拖动整行排序(并列的 A/B 一起移动)'
        handle.addEventListener('dragstart', function (e) {
          if (bubbleNativeDragBlocked()) { try { e.preventDefault() } catch (err) {} return }
          try { e.dataTransfer.setData('text/plain', 'row:' + idx) } catch (err) {}
          bubbleMainDragIdx = idx
          bubbleMainDragSide = -1
          bubbleDropZone = ''
        })
      } else {
        handle.title = '拖动排序;拖到某行左/右边缘可与其并列'
      }
      row.appendChild(handle)
      if (isChoice) {
        bubbleChoiceRowUI(row, step, idx)
      } else {
        var chip = document.createElement('button')
        chip.type = 'button'
        chip.className = 'dshwv-bubchip dshwv-bubchip-btn'
        chip.textContent = '第' + (idx + 1) + '次点击 · 编辑内容'
        chip.title = '点击编辑该泡泡的内容模块(' + bubbleRowLabel(step) + ');把另一行拖到本行左/右边缘可并列'
        chip.addEventListener('click', function (e) { e.stopPropagation(); openBubbleItem(idx, -1) })
        row.appendChild(chip)
        var del = document.createElement('button')
        del.type = 'button'
        del.className = 'dshwv-bubmini'
        del.textContent = '✕'
        del.title = '删除该次点击'
        del.addEventListener('click', function () { bubbleDelMore(idx) })
        row.appendChild(del)
      }
      bubbleMoreListEl.appendChild(row)
    })(i)
  }
}
// 落点区:左右 22% = 并列;中间上/下半 = 排到目标行前/后
function bubbleRowZone(rowEl, ev) {
  try {
    var r = rowEl.getBoundingClientRect()
    if (!r || !r.width) return ''
    var x = ev.clientX - r.left
    if (x < r.width * 0.22) return 'pairL'
    if (x > r.width * 0.78) return 'pairR'
    return (ev.clientY - r.top) < r.height / 2 ? 'before' : 'after'
  } catch (err) { return '' }
}
function bubbleDropShadow(zone) {
  if (zone === 'before') return '0 -3px 0 #203170'
  if (zone === 'after') return '0 3px 0 #203170'
  if (zone === 'pairL') return 'inset 3px 0 0 #203170'
  if (zone === 'pairR') return 'inset -3px 0 0 #203170'
  return ''
}
function bubbleDropApply(to, ev) {
  try {
    var from = bubbleMainDragIdx
    var side = bubbleMainDragSide
    bubbleMainDragIdx = null
    bubbleMainDragSide = -1
    var zone = bubbleDropZone || ''
    bubbleDropZone = ''
    var arr = bubbleEditItems
    if (from === null || from === undefined || from < 1 || to < 1 || from >= arr.length || to >= arr.length) return
    // 拖 A/B 单泡:拆出并插入目标行前/后(允许拖回本行上半=移到另一侧前)
    if (side >= 0) {
      if (zone !== 'before' && zone !== 'after') return
      bubbleSideSplitDrop(from, side, to, zone)
      return
    }
    if (from === to) return
    if (zone === 'pairL' || zone === 'pairR') { bubblePairDrop(from, to, zone); return }
    // 纵向排序:以目标行身份定位,避免下标移位
    var target = arr[to]
    var removed = arr.splice(from, 1)[0]
    var ti = arr.indexOf(target)
    if (ti < 0) ti = arr.length - 1
    var insertAt = (zone === 'after') ? ti + 1 : ti
    arr.splice(Math.max(1, Math.min(insertAt, arr.length)), 0, removed)
    renderBubbleMore()
  } catch (err) {}
}
// 把并列泡中的 A 或 B 拆出成独立单泡,插入目标行前/后;
// 剩下的另一侧自动还原为普通单行(去掉权重框/拆开钮,出现 ✕ 删除)
function bubbleSideSplitDrop(from, side, to, zone) {
  try {
    var arr = bubbleEditItems
    var step = arr[from]
    if (!bubbleIsChoice(step)) return
    var opts = bubbleChoiceOptions(step)
    if (side < 0 || side >= opts.length) return
    var movedItem = opts[side].item || {}
    var movedStep = bubbleSingleFromItem(movedItem)
    var rest = []
    for (var i = 0; i < opts.length; i++) if (i !== side) rest.push(opts[i])
    // 拖回本行上/下半区 = 调换 A/B 次序,直接拆成两个单行
    if (from === to) {
      if (!rest.length) return
      arr.splice(from, 1, bubbleSingleFromItem(rest[0].item))
      arr.splice((zone === 'before') ? from : from + 1, 0, movedStep)
      renderBubbleMore()
      return
    }
    var target = arr[to]
    if (rest.length === 1) {
      // 只剩一侧 → 该行还原为普通单行
      arr.splice(from, 1, bubbleSingleFromItem(rest[0].item))
    } else {
      arr.splice(from, 1)
    }
    var ti = arr.indexOf(target)
    if (ti < 0) ti = arr.length - 1
    var insertAt = (zone === 'after') ? ti + 1 : ti
    arr.splice(Math.max(1, Math.min(insertAt, arr.length)), 0, movedStep)
    renderBubbleMore()
  } catch (err) {}
}
// 横向落点 = 并列/替换该侧(替换带二次确认)
function bubblePairDrop(from, to, zone) {
  try {
    var arr = bubbleEditItems
    var src = arr[from]
    var dst = arr[to]
    if (!src || !dst) return
    if (bubbleIsChoice(src)) return // 并列行只能整行上下移动,不可再并入
    var srcBubble = bubbleStepToBubble(src)
    if (bubbleIsChoice(dst)) {
      var sideIdx = (zone === 'pairL') ? 0 : 1
      showConfirm('用拖入的泡泡替换该并列对中 ' + (sideIdx === 0 ? 'A(左)' : 'B(右)') + ' 泡的内容?', function () {
        var d2 = arr[to]
        if (!d2 || !bubbleIsChoice(d2)) return
        var opts2 = bubbleChoiceOptions(d2)
        if (!opts2.length) return
        var oi3 = (sideIdx < opts2.length) ? sideIdx : 0
        opts2[oi3].item = srcBubble
        arr.splice(from, 1)
        renderBubbleMore()
      })
      return
    }
    // 两个单行 → 并列一行(权重默认 1:1;放左边缘=拖入泡在左A,放右边缘=拖入泡在右B)
    var dstBubble = bubbleStepToBubble(dst)
    var options = (zone === 'pairL')
      ? [{ w: 1, item: srcBubble }, { w: 1, item: dstBubble }]
      : [{ w: 1, item: dstBubble }, { w: 1, item: srcBubble }]
    var choiceStep = { kind: 'choice', options: options }
    arr.splice(Math.max(from, to), 1)
    arr.splice(Math.min(from, to), 1)
    // 并列行放在被拖到的目标行原槽位:拖向下(from<to)→ to-1;拖向上(from>to)→ to
    var insertAt = (from < to) ? to - 1 : to
    arr.splice(Math.max(1, Math.min(insertAt, arr.length)), 0, choiceStep)
    renderBubbleMore()
  } catch (err) {}
}
// 并列行的内容区:[ A组(虚线框: chip+权重) ] [⊕] [ B组(虚线框: chip+权重) ]
// 整体呈现"左右两个大元素";A/B chip 可单独拖动(把该泡拆出去),整行移动请拖最左侧 ⠿ 手柄
function bubbleChoiceRowUI(row, step, idx) {
  var wrap = document.createElement('div')
  wrap.className = 'dshwv-choicerow'
  var opts = bubbleChoiceOptions(step)
  for (var s = 0; s < opts.length && s < 2; s++) {
    (function (si) {
      var opt = opts[si]
      var grp = document.createElement('div')
      grp.className = 'dshwv-choicegrp'
      // 编辑大按钮占满整组
      var chip = document.createElement('button')
      chip.type = 'button'
      chip.className = 'dshwv-bubchip dshwv-bubchip-btn dshwv-choicechip'
      chip.textContent = (si === 0 ? 'A' : 'B') + ' · 编辑内容'
      chip.title = '点击编辑该泡(' + bubbleRowLabel(opt.item) + ');按住拖动可把该泡拆出到其他位置'
      chip.draggable = true
      chip.addEventListener('dragstart', function (e) {
        // 触摸手势:拦掉原生拖拽(手机上原生拖拽会话不结束会卡死整页)
        if (bubbleNativeDragBlocked()) { try { e.preventDefault() } catch (err) {} return }
        e.stopPropagation()
        try { e.dataTransfer.setData('text/plain', 'side:' + idx + ':' + si) } catch (err) {}
        bubbleMainDragIdx = idx
        bubbleMainDragSide = si
        bubbleDropZone = ''
      })
      chip.addEventListener('click', function (e) { e.stopPropagation(); openBubbleItem(idx, si) })
      grp.appendChild(chip)
      // 权重输入:有框窄条、无原生上下箭头,直接键入数字
      var num = document.createElement('input')
      num.type = 'text'
      num.inputMode = 'numeric'
      num.maxLength = 2
      num.className = 'dshwv-winput'
      num.value = String(bubbleChoiceWeight(opt))
      num.title = (si === 0 ? 'A' : 'B') + ' 泡出现权重(直接输入数字,1~99,默认1:1)'
      num.addEventListener('change', function () {
        var w = bubbleChoiceWeight({ w: num.value })
        opt.w = w
        num.value = String(w)
      })
      grp.appendChild(num)
      wrap.appendChild(grp)
    })(s)
    if (s === 0) {
      // 中央拆开按钮:十字由 CSS 绘制,悬停仅旋转(不变色)
      var split = document.createElement('button')
      split.type = 'button'
      split.className = 'dshwv-splitbtn'
      split.title = '点击拆开:恢复为两个独立泡泡(拆开后每行才显示 ✕ 删除)'
      var sp = document.createElement('span')
      split.appendChild(sp)
      split.addEventListener('click', function () { bubbleUnpairStep(idx) })
      wrap.appendChild(split)
    }
  }
  row.appendChild(wrap)
}
// 拖到列表末尾空白区 = 追加到队列最后(单泡拖出或整行移到底部)
function bubbleDropToEnd() {
  try {
    var from = bubbleMainDragIdx
    var side = bubbleMainDragSide
    bubbleMainDragIdx = null
    bubbleMainDragSide = -1
    bubbleDropZone = ''
    var arr = bubbleEditItems
    if (from === null || from === undefined || from < 1 || from >= arr.length) return
    if (side >= 0) {
      var step = arr[from]
      if (!bubbleIsChoice(step)) return
      var opts = bubbleChoiceOptions(step)
      if (side >= opts.length) return
      var movedItem = opts[side].item || {}
      var rest = []
      for (var i = 0; i < opts.length; i++) if (i !== side) rest.push(opts[i])
      if (rest.length === 1) arr.splice(from, 1, bubbleSingleFromItem(rest[0].item))
      else if (rest.length >= 2) step.options = rest
      else arr.splice(from, 1)
      arr.push(bubbleSingleFromItem(movedItem))
      renderBubbleMore()
      return
    }
    if (arr.length <= 1) return
    var removed = arr.splice(from, 1)[0]
    arr.push(removed)
    renderBubbleMore()
  } catch (err) {}
}
// —— 触摸端自研排序(v635):长按 400ms 进入拖拽,行跟手上下移动,松手按落点插入 ——
// 仅纵向排序;左右"并列/拆开"仍是桌面专属(触摸端可用行内「拆开」按钮)。
var ROW_TOUCH_HOLD_MS = 400
var ROW_TOUCH_SLOP = 8
var ROW_TOUCH_PAIR_BAND = 0.24 // 拖到目标行左/右 24% 边带 = 并列(比桌面 22% 略宽,便于手指瞄准)
var rowTouchArm = null // {row, idx, x, y, timer}:已按下、等待长按成立
var rowTouchDrag = null // {row, idx, y0, dy, to, zone}:长按已成立,正在拖
function rowTouchRowOf(target) {
  try {
    if (!target || !target.closest || !bubbleMoreListEl) return null
    var row = target.closest('.dshwv-bubrow')
    if (!row || !bubbleMoreListEl.contains(row)) return null
    return row
  } catch (err) { return null }
}
// 与 renderBubbleMore 保持一致地开关该行的原生可拖:触摸手势期间关掉,手势结束后恢复
function rowTouchSetDraggable(row, idx, on) {
  try {
    if (!row) return
    var isChoice = bubbleIsChoice(bubbleEditItems[idx])
    row.draggable = !!(on && !isChoice)
    var h = row.querySelector('.dshwv-bubdrag')
    if (h) h.draggable = !!(on && isChoice)
    var cs = row.querySelectorAll('.dshwv-choicechip')
    for (var i = 0; i < cs.length; i++) cs[i].draggable = !!on
  } catch (err) {}
}
function rowTouchClearHighlight() {
  try {
    var rows = [].slice.call(bubbleMoreListEl.children)
    for (var i = 0; i < rows.length; i++) rows[i].style.boxShadow = ''
  } catch (err) {}
}
function rowTouchArmCancel() {
  if (rowTouchArm && rowTouchArm.timer) clearTimeout(rowTouchArm.timer)
  rowTouchArm = null
}
function rowTouchDetach() {
  try {
    document.removeEventListener('touchmove', rowTouchMove, true)
    document.removeEventListener('touchend', rowTouchEnd, true)
    document.removeEventListener('touchcancel', rowTouchEnd, true)
  } catch (err) {}
}
function rowTouchEnter() {
  var a = rowTouchArm
  if (!a) return
  rowTouchArm = null
  rowTouchDrag = { row: a.row, idx: a.idx, y0: a.y, dy: 0, to: a.idx, zone: 'after' }
  rowDragSuppressAt = Date.now() + 700
  try {
    a.row.classList.add('dshwv-row-dragging')
    if (navigator && navigator.vibrate) navigator.vibrate(10)
  } catch (err) {}
}
function rowTouchStart(e) {
  try {
    if (rowTouchDrag || rowTouchArm) return
    if (!e.touches || e.touches.length !== 1) return
    var row = rowTouchRowOf(e.target)
    if (!row) return
    var idx = parseInt(row.getAttribute('data-i'), 10)
    if (!(idx >= 1)) return
    // 行内按钮也可以长按拖拽(手指常落在较宽的 chip 上);长按成立后随之而来的 click 会被抑制
    rowTouchSetDraggable(row, idx, false)
    var t = e.touches[0]
    rowTouchArm = { row: row, idx: idx, x: t.clientX, y: t.clientY, timer: setTimeout(rowTouchEnter, ROW_TOUCH_HOLD_MS) }
    document.addEventListener('touchmove', rowTouchMove, { capture: true, passive: false })
    document.addEventListener('touchend', rowTouchEnd, true)
    document.addEventListener('touchcancel', rowTouchEnd, true)
  } catch (err) {}
}
function rowTouchMove(e) {
  try {
    var t = e.touches && e.touches[0]
    if (!t) return
    if (rowTouchArm) {
      // 长按成立前先移动 = 用户想滚动页面/卡片,放弃拖拽意图
      var dx = t.clientX - rowTouchArm.x
      var dy = t.clientY - rowTouchArm.y
      if (dx * dx + dy * dy > ROW_TOUCH_SLOP * ROW_TOUCH_SLOP) rowTouchArmCancel()
      return
    }
    if (!rowTouchDrag) return
    if (e.touches.length > 1) { rowTouchFinish(false); return }
    try { e.preventDefault() } catch (err) {} // 拖拽期间吃掉滚动
    rowTouchDrag.dy = t.clientY - rowTouchDrag.y0
    rowTouchDrag.row.style.transform = 'translateY(' + rowTouchDrag.dy + 'px)'
    // 落点判定(v637):手指落在某行"竖向范围内"时——
    //   左侧带 / 右侧带 → 与目标行并列(复用桌面 pairL/pairR,可替换并列侧);
    //   中间 → 按上/下半插入到该行前/后。
    // 手指不在任何行内(行间空隙/末尾空白) → 按中线判定插入位置。
    var rows = [].slice.call(bubbleMoreListEl.children)
    var to = rowTouchDrag.idx
    var zone = 'after'
    for (var i = 0; i < rows.length; i++) {
      var r = rows[i]
      if (r === rowTouchDrag.row) continue
      var rr = r.getBoundingClientRect()
      var ri = parseInt(r.getAttribute('data-i'), 10)
      if (t.clientY >= rr.top && t.clientY <= rr.bottom && rr.width > 0) {
        var xRel = (t.clientX - rr.left) / rr.width
        if (xRel < ROW_TOUCH_PAIR_BAND) { to = ri; zone = 'pairL' }
        else if (xRel > 1 - ROW_TOUCH_PAIR_BAND) { to = ri; zone = 'pairR' }
        else { to = ri; zone = (t.clientY < rr.top + rr.height / 2) ? 'before' : 'after' }
        break
      }
      if (t.clientY < rr.top + rr.height / 2) { to = ri; zone = 'before'; break }
      to = ri; zone = 'after'
    }
    rowTouchDrag.to = to
    rowTouchDrag.zone = zone
    rowTouchClearHighlight()
    for (var j = 0; j < rows.length; j++) {
      if (rows[j] !== rowTouchDrag.row && parseInt(rows[j].getAttribute('data-i'), 10) === to) {
        rows[j].style.boxShadow = bubbleDropShadow(zone)
        break
      }
    }
  } catch (err) {}
}
function rowTouchFinish(apply) {
  var d = rowTouchDrag
  rowTouchDrag = null
  rowTouchArmCancel()
  rowTouchDetach()
  if (!d) return
  rowDragSuppressAt = Date.now() + 700
  try {
    d.row.classList.remove('dshwv-row-dragging')
    d.row.style.transform = ''
    rowTouchClearHighlight()
    // 未发生重排时把原生可拖恢复回去(触屏笔记本上鼠标仍可拖)
    if (d.to === d.idx) rowTouchSetDraggable(d.row, d.idx, true)
  } catch (err) {}
  if (!apply || d.to === d.idx) return
  // 复用桌面拖拽的落点应用逻辑(纵向 before/after)
  bubbleMainDragIdx = d.idx
  bubbleMainDragSide = -1
  // 原样透传落点区域(v637 曾把 pairL/pairR 误降级为 after,导致左右并列松手不生效)
  bubbleDropZone = (d.zone === 'before' || d.zone === 'after' || d.zone === 'pairL' || d.zone === 'pairR') ? d.zone : 'after'
  bubbleDropApply(d.to)
}
function rowTouchEnd(e) {
  try { if (e && e.touches && e.touches.length > 0) return } catch (err) {}
  rowTouchFinish(true)
}
document.addEventListener('touchstart', rowTouchStart, { passive: true })
function bubbleUnpairStep(idx) {
  try {
    var arr = bubbleEditItems
    var step = arr[idx]
    if (!bubbleIsChoice(step)) return
    var items = bubbleChoiceOptions(step).map(function (o) { return bubbleStepToBubble(o.item) })
    if (!items.length) return
    arr.splice(idx, 1)
    for (var i2 = items.length - 1; i2 >= 0; i2--) arr.splice(idx, 0, items[i2])
    renderBubbleMore()
  } catch (err) {}
}
var bubbleMainDragIdx = null
var bubbleMainDragSide = -1 // ≥0 = 正在拖并列行里的 A(0)/B(1) 单泡
var bubbleDropZone = '' // 当前 dragover 落点:before/after/pairL/pairR
function renderBubbleEditor() {
  if (!bubbleEditItems.length) bubbleEditItems = [{ kind: 'normal' }]
  renderBubbleFirst()
  renderBubbleMore()
}
function bubbleMoveMore(idx, dir) {
  var j = idx + dir
  if (j < 1 || j >= bubbleEditItems.length) return
  var t = bubbleEditItems[idx]
  bubbleEditItems[idx] = bubbleEditItems[j]
  bubbleEditItems[j] = t
  renderBubbleMore()
}
function bubbleDelMore(idx) {
  if (bubbleEditItems.length <= 1) return
  bubbleEditItems.splice(idx, 1)
  renderBubbleMore()
}
function bubbleAddMore() {
  // 第3个及后续泡泡:内容留空(自定义,0 模块),由用户自行填充
  bubbleEditItems.push({ kind: 'custom', modules: [] })
  renderBubbleMore()
}
// W1 打开时的整份快照(序列+模块库);无任何修改时「取消/保存」免二次确认
var bubbleEditorSnap = null
function bubbleEditorDirty() {
  try {
    if (bubbleEditorSnap === null) return true
    return bubbleEditorSnap !== JSON.stringify([bubbleEditItems, bubbleLib, bubbleTapAdvChk.checked])
  } catch (err) { return true }
}
// v783：打开编辑器前**先从宿主重读一遍配置**。为什么必须重读：泡泡配置只在页面加载时取一次
//   （loadBubbleCfg），之后一直用内存里的 bubbleCfg 副本；而桌面端 + 网页端（或两个标签页）会**各持一份**，
//   谁最后点「保存」谁就把自己那份写回去 ⇒ 在 A 里删掉的模块（或模块库条目）会被 B 的过期副本**写回来**，
//   表现就是"删了很久的自定义模块又冒出来了"。读失败时保持原值（离线/接口异常行为不变）。
function refreshBubbleCfgFromHost(cb) {
  var done = function () { try { cb() } catch (err) {} }
  try {
    fetch(BUBBLE_URL, { cache: 'no-store' })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (d && d.ok && d.config) {
          bubbleCfg = d.config
          bubbleLib = (d.config.lib && Array.isArray(d.config.lib)) ? JSON.parse(JSON.stringify(d.config.lib)) : []
          bubbleTapAdvance = d.config.tapAdvance === true // v727
          try { applyBubbleCfgSeq() } catch (err) {}
        }
      })
      .catch(function () {})
      .then(done, done)
  } catch (err) { done() }
}
function openBubbleEditor() {
  refreshBubbleCfgFromHost(function () { openBubbleEditorWithCache() })
}
function openBubbleEditorWithCache() {
  try {
    closeRolePanel()
    closeAudioGroupPanel()
    bubbleLib = (bubbleCfg && bubbleCfg.lib && Array.isArray(bubbleCfg.lib)) ? JSON.parse(JSON.stringify(bubbleCfg.lib)) : []
    bubbleTapAdvChk.checked = bubbleTapAdvance === true // v727：把已保存的开关状态显示出来
    var list = []
    if (bubbleCfg && Array.isArray(bubbleCfg.items) && bubbleCfg.items.length) {
      list = bubbleCfg.items.slice()
    } else {
      list = bubbleDefaultQueue()
    }
    bubbleEditItems = []
    for (var k = 0; k < list.length; k++) {
      var src = list[k]
      if (src && src.kind === 'choice' && Array.isArray(src.options)) {
        // 并列步骤:整步保留,内部 A/B 各自深拷贝为 custom 泡
        var opts = []
        for (var oi = 0; oi < src.options.length && oi < 2; oi++) {
          var oi2 = src.options[oi] || {}
          var srcItem = oi2.item || {}
          var srcMods = Array.isArray(srcItem.modules) ? JSON.parse(JSON.stringify(srcItem.modules)) : (srcItem.kind === 'random' ? bubbleDefaultSecondModules() : [])
          opts.push({ w: bubbleChoiceWeight(oi2), item: { kind: 'custom', modules: srcMods } })
        }
        if (opts.length === 1) { bubbleEditItems.push(bubbleSingleFromItem(opts[0].item)); continue }
        if (opts.length >= 2) { bubbleEditItems.push({ kind: 'choice', options: opts }); continue }
      }
      bubbleEditItems.push({ kind: src.kind === 'random' ? 'random' : (src.kind === 'custom' ? 'custom' : 'normal'), modules: src.modules ? JSON.parse(JSON.stringify(src.modules)) : undefined })
    }
    bubbleEditorSnap = JSON.stringify([bubbleEditItems, bubbleLib, bubbleTapAdvChk.checked]) // v727：含开关，改开关也算"有改动"
    renderBubbleEditor()
    bubbleMask.style.display = 'flex'
  } catch (err) {}
}
function closeBubbleEditor() {
  bubbleMask.style.display = 'none'
  bubbleEditorSnap = null
}
function bubbleEditorReset() {
  showConfirm('恢复为默认序列(首次=余额内容,再次=随机语句)?', function () {
    bubbleEditItems = bubbleDefaultQueue()
    renderBubbleEditor()
  })
}
function bubbleEditorSave() {
  try {
    var doSave = function () {
      var items = []
      for (var i = 0; i < bubbleEditItems.length; i++) items.push(bubbleStepToSaved(bubbleEditItems[i]))
      // v727：开关随本窗口的「保存」一起落盘；保存成功后立即生效（不必重载）
      var tapAdv = bubbleTapAdvChk.checked === true
      saveBubbleCfg({ v: 1, items: items, lib: bubbleLib, tapAdvance: tapAdv }, function (ok) {
        if (ok !== false) {
          bubbleTapAdvance = tapAdv
          closeBubbleEditor()
        }
      })
    }
    // 未做任何修改:直接保存,不再二次确认
    if (bubbleEditorDirty()) showConfirm('保存当前点击序列?(未逐泡编辑过的行将按默认内容保存,保存后即生效)', doSave)
    else doSave()
  } catch (err) {}
}
// 编辑器步骤 → 落盘配置项(单选 → custom;并列 → choice {options:[{w,item}]})
function bubbleStepToSaved(step) {
  if (bubbleIsChoice(step)) {
    var opts = bubbleChoiceOptions(step).map(function (o) {
      var itm = (o && o.item) || {}
      var mods = Array.isArray(itm.modules) ? itm.modules : bubbleDefaultModules(itm.kind === 'random' ? 'random' : 'normal')
      bubbleRowsCanon(mods) // F2:保存前规范化行键
      return { w: bubbleChoiceWeight(o), item: { kind: 'custom', modules: mods } }
    })
    return { kind: 'choice', options: opts }
  }
  var mods = Array.isArray(step.modules) ? step.modules : bubbleDefaultModules(step.kind)
  bubbleRowsCanon(mods) // F2:保存前规范化行键
  return { kind: 'custom', modules: mods }
}
// —— 单泡编辑(W2):模块面板 + 泡泡预览行 ——
var bubbleItemMask = null
var bubbleEditItemIdx = -1 // 当前编辑的主队列下标
var bubbleEditSide = -1 // -1=单选步骤;0/1=并列步骤的 A/B 侧
var bubbleItemSnap = null // 打开 W2 时的整步深拷贝快照;「取消」确认后还原,丢弃本次编辑
var bubbleItemTitleEl = null
var bubbleItemSideEl = null
var bubblePalEl = null
var bubblePvEl = null
// 当前正在编辑的实际泡内容(单选步骤=它本身;并列步骤=options[bubbleEditSide].item)
function bubbleEditTarget() {
  var step = bubbleEditItems[bubbleEditItemIdx]
  if (!step) return null
  if (bubbleIsChoice(step)) {
    var o = bubbleChoiceOptions(step)[(bubbleEditSide === 0 || bubbleEditSide === 1) ? bubbleEditSide : 0]
    return o ? o.item : null
  }
  return step
}
function bubbleEditStepLabel(step, idx) {
  var base = '第' + (idx + 1) + '次点击'
  if (bubbleIsChoice(step)) return base + ' · ' + (bubbleEditSide === 1 ? 'B泡' : 'A泡')
  return base
}
function openBubbleItem(idx, side) {
  try {
    whaleZClean()
    var step = bubbleEditItems[idx]
    if (!step) return
    bubbleEditItemIdx = idx
    if (!bubbleIsChoice(step)) {
      bubbleEditSide = -1
    } else {
      var want = (side === 0 || side === 1) ? side : 0
      if (!bubbleChoiceOptions(step)[want]) want = 0
      bubbleEditSide = want
    }
    // 记录整步快照(含并列两侧),「取消」确认后还原,丢弃本次编辑/恢复默认
    bubbleItemSnap = JSON.parse(JSON.stringify(step))
    var it = bubbleEditTarget()
    if (!it) return
    bubbleEditEnsureModules(it)
    // F2:进入编辑即按当前分组规范化行键(旧数据=每模块一行),行操作与落盘保持一致
    bubbleRowsCanon(it.modules)
    bubbleItemTitleEl.textContent = '编辑 ' + bubbleEditStepLabel(step, idx) + '内容(可拖动下方模块入框)'
    renderBubbleItemSideSwitch(step)
    // 先显示弹窗再渲染预览:getBBox 需要可见布局,否则测到 0 导致内容错位到角落
    bubbleItemMask.style.display = 'flex'
    renderBubblePal()
    renderBubblePv()
  } catch (err) {}
}
// W2 顶部 A/B 侧切换(仅并列步骤显示)
function renderBubbleItemSideSwitch(step) {
  if (!bubbleItemSideEl) return
  bubbleItemSideEl.innerHTML = ''
  if (!bubbleIsChoice(step)) { bubbleItemSideEl.style.display = 'none'; return }
  bubbleItemSideEl.style.display = ''
  var opts = bubbleChoiceOptions(step)
  for (var s = 0; s < opts.length && s < 2; s++) {
    (function (si) {
      var b = document.createElement('button')
      b.type = 'button'
      b.className = 'dshwv-bubchip dshwv-bubchip-btn' + (si === bubbleEditSide ? ' dshwv-bubchip-cur' : '')
      b.textContent = (si === 0 ? 'A' : 'B') + ' 泡(权重 ' + bubbleChoiceWeight(opts[si]) + ')'
      b.title = '切换到编辑 ' + (si === 0 ? 'A' : 'B') + ' 泡'
      b.addEventListener('click', function (e) { e.stopPropagation(); openBubbleItem(bubbleEditItemIdx, si) })
      bubbleItemSideEl.appendChild(b)
    })(s)
  }
}
function closeBubbleItem() {
  bubbleItemMask.style.display = 'none'
  bubbleEditItemIdx = -1
  bubbleEditSide = -1
  if (bubbleItemSideEl) bubbleItemSideEl.innerHTML = ''
}
// ===== 快速编辑悬浮窗:文本模块(完整内容+样式,带实例预览)与随机语句单句 =====
var qeditEl = null
var qeditCtx = null // {kind:'text'|'line', ...}
var QC_SCHEMES = [
  ['macaron', '马卡龙'], ['candy', '糖果'], ['rouge', '酒红'], ['bamboo', '翠青'], ['aurora', '极光幻彩'],
  ['deepsea', '深海蓝调'], ['sunset', '落日熔金'], ['forest', '森林秘语'], ['champagne', '香槟鎏金'],
  ['lavender', '薰衣草梦境'], ['mint', '薄荷汽水'], ['lava', '岩浆熔岩'], ['galaxy', '银河星紫'], ['ink', '墨韵黑白'], ['indigo', '靛蓝夜曲'],
  ['blaze', '火红烈焰'], ['amber', '警示橙黄'],
]
function qeditEnsure() {
  if (qeditEl) return qeditEl
  qeditEl = document.createElement('div')
  qeditEl.className = 'dshwv-qedit'
  qeditEl.style.display = 'none'
  dshwBodyAppend(qeditEl)
  if (!window.__dshwQeditBound) {
    window.__dshwQeditBound = true
    document.addEventListener('pointerdown', function (e) {
      if (!qeditEl || qeditEl.style.display === 'none') return
      // 点击悬浮窗内部,或悬浮窗唤起的自绘下拉(菜单/触发钮/字体/取色)都不关闭
      if (e.target && e.target.closest && (e.target.closest('.dshwv-qedit') ||
          e.target.closest('.dshwv-rgbmenu') || e.target.closest('.dshwv-custmenu') ||
          e.target.closest('.dshwv-rgbhead') || e.target.closest('.dshwv-custbtn') ||
          e.target.closest('.dshwv-fontwrap') || e.target.closest('.dshwv-usagepanel') ||
          e.target.closest('.dshwv-usage-mask') || e.target.closest('.dshwv-resmask'))) return
      // 锚点按钮本身不算"外面":交给它的 click 处理函数做开/关切换(v723)
      try {
        if (qeditAnchorIs(e.target)) return
      } catch (err) {}
      qeditClose()
    }, true)
    document.addEventListener('keydown', function (e) {
      // v789（issue #179）：组字中按 Esc 是输入法的事，别抢
      if (dshwvComposingNow()) return
      if (e.key === 'Escape') qeditClose()
    })
  }
  return qeditEl
}
function qeditClose() {
  if (qeditEl) qeditEl.style.display = 'none'
  qeditCtx = null
}
// v723:悬浮编辑窗的"同一个按钮再点一次 = 关闭"。
// 背景:document 级 pointerdown 会先把浮窗关掉——点到**触发它的那个按钮**也算"点在外面";
// 若该按钮的 click 处理函数无条件重开,就表现为"再点一次仍然弹出来"(关不掉)。
// 所以:pointerdown 放过当前锚点按钮,由这里判断"同一锚点 → 关",否则照常打开。
function qeditAnchorIs(anchor) {
  try {
    if (!anchor) return false
    if (!qeditEl || qeditEl.style.display === 'none') return false
    var cur = qeditCtx && qeditCtx.anchor
    return !!cur && (cur === anchor || (cur.contains && cur.contains(anchor)))
  } catch (err) { return false }
}
function qeditToggleClose(anchor) {
  if (!qeditAnchorIs(anchor)) return false
  qeditClose()
  return true
}
function qRow() { var d = document.createElement('div'); d.className = 'dshwv-qedit-row'; return d }
function qLabel(t) { var s = document.createElement('label'); s.textContent = t; return s }
function qeditPlace(anchorRect, widthPx, preferAbove) {
  try {
    var vp = viewport()
    var box = qeditEl
    var w = Math.max(180, Math.min(widthPx || 320, vp.w - 16))
    box.style.width = w + 'px'
    box.style.display = 'block'
    var h = box.offsetHeight || 200
    var left = anchorRect.left + anchorRect.width / 2 - w / 2
    left = Math.max(8, Math.min(left, vp.w - w - 8))
    var top = preferAbove ? anchorRect.top - h - 8 : anchorRect.bottom + 6
    if (preferAbove && top < 8) top = Math.min(8, anchorRect.bottom + 6)
    if (top + h > vp.h - 8) top = Math.max(8, vp.h - h - 8)
    if (top < 8) top = 8
    box.style.left = Math.round(left) + 'px'
    box.style.top = Math.round(top) + 'px'
  } catch (err) {}
}
function qColorSelectBuild(current, onPick, opts) {
  // 颜色下拉:纯色 + 跑马灯列表;默认选纯色+默认色值;opts.allowNone=true 时额外提供“无”项(底色用)
  opts = opts || {}
  var allowNone = !!opts.allowNone
  var oLabel = opts.label || '颜色'
  var oHex = opts.defaultHex || '#203170'
  var oText = opts.defaultText || '默认'
  var row = qRow()
  row.appendChild(qLabel(oLabel))
  var wrap = document.createElement('div')
  wrap.className = 'dshwv-rgbwrap dshwv-qcolwrap'
  var head = document.createElement('button')
  head.type = 'button'
  head.className = 'dshwv-rgbhead'
  head.title = '颜色:纯色或跑马灯'
  wrap.appendChild(head)
  var menu = document.createElement('div')
  menu.className = 'dshwv-rgbmenu dshwv-qcolmenu'
  wrap.appendChild(menu)
  var sw = document.createElement('span')
  sw.className = 'dshwv-qcolorhost'
  // 布局:标签 + 下拉(sw 在行尾 append,保证色板/默认色位于下拉右侧)
  function modeOf(v) {
    if (v === 'none') return allowNone ? 'none' : 'solid'
    if (v === 'solid' || isScheme(v)) return v
    return allowNone ? 'none' : 'solid'
  }
  var curMode = modeOf(current)
  function isScheme(v) { for (var i = 0; i < QC_SCHEMES.length; i++) if (QC_SCHEMES[i][0] === v) return true; return false }
  function labelOf(v) { if (v === 'none') return '无'; if (v === 'solid') return '纯色'; for (var i = 0; i < QC_SCHEMES.length; i++) if (QC_SCHEMES[i][0] === v) return QC_SCHEMES[i][1]; return allowNone ? '无' : '纯色' }
  function renderSolid(hex, onSet) {
    sw.innerHTML = ''
    var ci = document.createElement('input')
    ci.type = 'color'
    ci.value = hex
    ci.title = '选择纯色'
    ci.addEventListener('input', function () { if (onSet) onSet(ci.value) })
    ci.addEventListener('change', function () { if (onSet) onSet(ci.value) })
    sw.appendChild(ci)
    var def = document.createElement('button')
    def.type = 'button'
    def.className = 'dshwv-bubmini'
    def.textContent = oText
    def.title = '恢复为' + oText + '色值'
    def.style.width = 'auto'
    def.style.padding = '0 6px'
    def.addEventListener('click', function () { if (onSet) onSet(oHex) })
    sw.appendChild(def)
  }
  function fill() {
    menu.innerHTML = ''
    function add(v, lab) {
      var o = document.createElement('div')
      o.className = 'dshwv-rgbopt' + (v === curMode ? ' dshwv-rgbcur' : '')
      if (v !== 'solid' && v !== 'none') { o.classList.add('optgrad'); o.classList.add('opt-' + v) }
      // 当前项用 ✓ 前缀标记(背景高亮会盖掉渐变文字,因此不依赖底色)
      o.textContent = (v === curMode ? '✓ ' : '') + lab
      o.addEventListener('click', function () {
        curMode = v
        closeMenu()
        if (onPick) onPick(v)
      })
      menu.appendChild(o)
    }
    if (allowNone) add('none', '无')
    add('solid', '纯色')
    for (var i = 0; i < QC_SCHEMES.length; i++) add(QC_SCHEMES[i][0], QC_SCHEMES[i][1])
  }
  var hexSetter = null
  function sync(mode, hex, onSet) {
    curMode = modeOf(mode)
    hexSetter = onSet || null
    head.textContent = labelOf(curMode)
    if (curMode === 'solid') renderSolid(hex || oHex, function (h) { if (hexSetter) hexSetter(h) })
    else sw.innerHTML = ''
    fill()
  }
  function closeMenu() { menu.classList.remove('dshwv-rgbopen'); bubbleColorOpenMenu = null }
  head.addEventListener('click', function (e) {
    e.stopPropagation()
    if (bubbleColorOpenMenu === menu) { closeMenu(); return }
    if (bubbleColorOpenMenu) bubbleColorOpenMenu.classList.remove('dshwv-rgbopen')
    fill()
    bubbleColorOpenMenu = menu
    dshwDropOpen(menu, head)
  })
  if (!window.__dshwColorBound) {
    window.__dshwColorBound = true
    document.addEventListener('pointerdown', function (e) {
      if (!bubbleColorOpenMenu) return
      try { if (e.target && e.target.closest && (e.target.closest('.dshwv-qcolwrap') || e.target.closest('.dshwv-rgbmenu'))) return } catch (err) {}
      bubbleColorOpenMenu.classList.remove('dshwv-rgbopen')
      bubbleColorOpenMenu = null
    }, true)
  }
  row.appendChild(wrap)
  row.appendChild(sw)
  fill()
  return { row: row, sync: sync }
}
function qStyleChecksBuild(getBool, setBool) {
  var row = qRow()
  ;[['加粗', 'bold'], ['斜体', 'italic'], ['下划线', 'ul']].forEach(function (item) {
    var lab = document.createElement('label')
    lab.style.display = 'inline-flex'
    lab.style.alignItems = 'center'
    lab.style.gap = '3px'
    var cb = document.createElement('input')
    cb.type = 'checkbox'
    cb.checked = !!getBool(item[1])
    cb.addEventListener('change', function () { setBool(item[1], cb.checked) })
    lab.appendChild(cb)
    lab.appendChild(document.createTextNode(item[0]))
    row.appendChild(lab)
  })
  return row
}
// —— 文本模块:完整悬浮编辑(内容/字体字号/样式颜色),实时刷新鲸鱼形预览 ——
function openQuickTextEditor(m, anchorBtn) {
  if (!m || (m.type !== 'text' && m.type !== 'link')) return
  if (qeditToggleClose(anchorBtn)) return // v723:同一按钮再点 = 关闭
  qeditClose()
  var box = qeditEnsure()
  box.innerHTML = ''
  qeditCtx = { kind: m.type === 'link' ? 'link' : 'text', m: m, anchor: anchorBtn || null }
  function changed() { try { renderBubblePv() } catch (err) {} }
  // 内容(超链接=链接文字)
  var r0 = qRow()
  r0.appendChild(qLabel(m.type === 'link' ? '链接文字' : '内容'))
  var tx = document.createElement('input')
  tx.type = 'text'
  tx.maxLength = 60
  tx.className = 'dshwv-qedit-content'
  tx.value = m.text || ''
  tx.addEventListener('input', function () { m.text = tx.value || ' '; changed() })
  r0.appendChild(tx)
  box.appendChild(r0)
  // 超链接:链接地址(输入时不改预览,仅保存)
  if (m.type === 'link') {
    var rUrl = qRow()
    rUrl.appendChild(qLabel('链接'))
    var uInp = document.createElement('input')
    uInp.type = 'text'
    uInp.className = 'dshwv-qedit-content'
    uInp.value = m.url || ''
    uInp.placeholder = 'https:// …'
    uInp.title = '点击打开;需以 http:// 或 https:// 开头'
    uInp.addEventListener('input', function () { m.url = uInp.value || '' })
    rUrl.appendChild(uInp)
    box.appendChild(rUrl)
  }
  // 第一行:字体 + 字号
  box.appendChild(bubbleFontEditRow(function () { return m.fontFamily || '' }, function (v) { m.fontFamily = v || ''; changed() }))
  var r1 = qRow()
  r1.appendChild(qLabel('字号'))
  var sz = document.createElement('input')
  sz.type = 'range'
  sz.min = '1'
  sz.max = '50'
  sz.step = '1'
  sz.className = 'dshwv-range'
  sz.style.flex = '1'
  sz.value = String(Math.max(1, Math.min(50, Math.round(Number(m.size) || 6))))
  var szNum = document.createElement('span')
  szNum.className = 'dshwv-volpct'
  szNum.textContent = sz.value
  sz.addEventListener('input', function () { m.size = Math.round(Number(sz.value) || 3); szNum.textContent = sz.value; changed() })
  r1.appendChild(sz)
  r1.appendChild(szNum)
  box.appendChild(r1)
  // 第二行:样式 + 颜色
  box.appendChild(qStyleChecksBuild(function (k) { return m[k] === true }, function (k, v) { m[k] = v; changed() }))
  // 颜色 + 底色(同一行各占一半;底色默认无)
  var grpT = document.createElement('div')
  grpT.className = 'dshwv-peakrow'
  var curColor = m.rgb ? m.rgb : 'solid'
  var cc = qColorSelectBuild(curColor, function (v) { onPick(v) })
  grpT.appendChild(cc.row)
  function onPick(v) {
    if (v === 'solid') { m.rgb = ''; if (!m.color) m.color = '#203170' }
    else { m.rgb = v; m.color = '' }
    cc.sync(v === 'solid' ? 'solid' : v, m.color, function (hex) { m.color = hex; changed() })
    changed()
  }
  var bgCurT = m.bgRgb ? m.bgRgb : (m.bg ? 'solid' : 'none')
  var bgt = qColorSelectBuild(bgCurT, function (v) {
    if (v === 'none') { m.bgRgb = ''; m.bg = '' }
    else if (v === 'solid') { m.bgRgb = ''; if (!m.bg) m.bg = '#dbe4f5' }
    else { m.bgRgb = v; m.bg = '' }
    bgt.sync(v === 'none' ? 'none' : v, m.bg, function (hex) { m.bg = hex; changed() })
    changed()
  }, { label: '底色', defaultHex: '#dbe4f5', defaultText: '默认', allowNone: true })
  grpT.appendChild(bgt.row)
  box.appendChild(grpT)
  cc.sync(curColor, m.color || '#203170', function (hex) { m.color = hex; changed() })
  bgt.sync(bgCurT, m.bg || '#dbe4f5', function (hex) { m.bg = hex; changed() })
  // 位置:鲸鱼形预览框正上方、宽度略小于预览框宽
  try {
    var pr = bubblePvPrevEl.getBoundingClientRect()
    qeditPlace(pr, Math.max(230, Math.round(pr.width - 24)), true)
  } catch (err) { qeditPlace({ left: 40, right: 360, top: 200, bottom: 300, width: 320 }, 320, false) }
}
// —— 特殊内容模块(balance/today/peak/nextpeak):悬浮编辑,内容与原先编辑窗口一致 ——
// v769：对话名模块的「保留长度」行 —— 悬浮编辑器（qRow/qLabel）与整窗编辑器（.dshwv-audiorow/span）
// **共用同一份实现**，只有外层行/标签的构造方式不同（各自的样式才一致）。
// rowFactory() 造一行、labelFactory(text) 造标签；changed() 在输入时回调（悬浮窗用它做实时预览）。
function sessionLenRowBuild(rowFactory, labelFactory, m, changed) {
  var r = rowFactory()
  r.appendChild(labelFactory('保留长度'))
  var inp = document.createElement('input')
  inp.type = 'number'
  inp.min = '0'
  inp.max = '120'
  inp.step = '1'
  inp.className = 'dshwv-winput'
  if (m.len === undefined || m.len === null) m.len = WAIT_SESSION_MAX
  inp.value = String(Math.max(0, Math.round(Number(m.len) || 0)))
  inp.title = '对话名超过这个长度时截断成「前N字...」；0 = 不截断'
  inp.addEventListener('input', function () {
    var n = Math.round(Number(inp.value) || 0)
    m.len = Math.max(0, Math.min(120, n))
    try { if (typeof changed === 'function') changed() } catch (err) {}
  })
  inp.addEventListener('change', function () {
    inp.value = String(Math.max(0, Math.round(Number(m.len) || 0)))
  })
  r.appendChild(inp)
  var tail = labelFactory('字，超出截断为 ...（0 = 不截断）')
  try {
    tail.style.opacity = '.75'
    tail.style.fontSize = '11px'
  } catch (err) {}
  r.appendChild(tail)
  return r
}
function openQuickModuleEditor(m, anchorBtn) {
  if (!m || bubbleIsImgMod(m) || m.type === 'random' || m.type === 'text') return
  if (qeditToggleClose(anchorBtn)) return // v723:同一按钮再点 = 关闭
  qeditClose()
  var box = qeditEnsure()
  box.innerHTML = ''
  qeditCtx = { kind: 'module', m: m, anchor: anchorBtn || null }
  function changed() { try { renderBubblePv() } catch (err) {} }
  function sizeRow() {
    var r = qRow()
    r.appendChild(qLabel('字号'))
    var sz = document.createElement('input')
    sz.type = 'range'
    sz.min = '1'
    sz.max = '50'
    sz.step = '1'
    sz.className = 'dshwv-range'
    sz.style.flex = '1'
    sz.value = String(Math.max(1, Math.min(50, Math.round(Number(m.size) || 6))))
    var num = document.createElement('span')
    num.className = 'dshwv-volpct'
    num.textContent = sz.value
    sz.addEventListener('input', function () { m.size = Math.round(Number(sz.value) || 3); num.textContent = sz.value; changed() })
    r.appendChild(sz)
    r.appendChild(num)
    box.appendChild(r)
  }
  function glyphRow() {
    box.appendChild(qStyleChecksBuild(function (k) { return m[k] === true }, function (k, v) { m[k] = v; changed() }))
  }
  function tplRow() {
    var r = qRow()
    r.appendChild(qLabel('内容'))
    var inp = document.createElement('input')
    inp.type = 'text'
    inp.className = 'dshwv-qedit-content'
    inp.value = m.tpl || ''
    var isModelBal = bubbleIsModelMod(m) && (m.type === 'balance' || m.type === 'today')
    var isModelQuota = bubbleIsModelMod(m) && m.type === 'quota'
    var isModelPlan = bubbleIsModelMod(m) && m.type === 'plan'
    inp.placeholder = isModelPlan ? '例: {plan} 额度 · {plan_reset} 刷新时间' : (isModelQuota ? '例: 额度 {quota} · 剩 {quota_left}' : (isModelBal ? '例: {balance} 或 今日 {today}' : (m.type === 'balance' ? '例: {balance_ds}' : (m.type === 'bonus' ? '例: {bonus_ds} 或 赠金 {bonus_ds}' : (m.type === 'recharge' ? '例: {recharge_ds} 或 余额 {recharge_ds}' : (m.type === 'today' ? '例: 今日已用 {expense_ds}' : (m.type === 'session' ? '例: {session} 或 当前对话 {session}' : (bubbleIsPeakCount(m) ? '例: 距空闲 {countdown}' : '例: 当前 {status}'))))))))
    inp.title = '可用占位符(英文): ' + (isModelPlan ? '{plan} 额度 / {plan_left} 剩余 / {plan_reset} 刷新时间（多窗口时随「显示样式」所选窗口变化）' : (isModelQuota ? '{quota} / {quota_used} / {quota_left} / {quota_total} / {quota_reset}' : (isModelBal ? '{balance} / {today}' : (m.type === 'peak' || m.type === 'nextpeak' ? '{status} / {countdown}' : (m.type === 'balance' ? '{balance_ds}' : (m.type === 'bonus' ? '{bonus_ds}' : (m.type === 'recharge' ? '{recharge_ds}' : (m.type === 'today' ? '{expense_ds}' : (m.type === 'session' ? '{session} 当前对话名（按「保留长度」截断）' : '{expense_ds}')))))))))
    inp.addEventListener('input', function () { m.tpl = inp.value; changed() })
    r.appendChild(inp)
    var qb2 = document.createElement('button')
    qb2.type = 'button'
    qb2.className = 'dshwv-tplq'
    qb2.textContent = '?'
    qb2.title = '可用占位符用法'
    qb2.style.marginLeft = '4px'
    qb2.addEventListener('click', function (e) { e.stopPropagation(); bubbleTplHelpToggle(m, qb2) })
    r.appendChild(qb2)
    box.appendChild(r)
  }
  tplRow()
  // v769：对话名模块的「保留长度」（与整窗编辑器共用同一份实现；这里改了会实时刷新预览）
  if (m.type === 'session') box.appendChild(sessionLenRowBuild(qRow, qLabel, m, changed))
  // 订阅额度模块的「显示样式」= 选时间窗口（多窗口厂商，如 OpenCode Go 的 5h / 周 / 月）
  if (m.type === 'plan' && (apiPlanMultiWin(m.modelId) || apiPlanWinList(m.modelId))) {
    var wrow = qRow()
    wrow.appendChild(qLabel('显示样式'))
    var wsel = document.createElement('select')
    for (var wi = 0; wi < BUBBLE_PLAN_WIN_OPTS.length; wi++) {
      var wo = document.createElement('option')
      wo.value = BUBBLE_PLAN_WIN_OPTS[wi][0]
      wo.textContent = BUBBLE_PLAN_WIN_OPTS[wi][1]
      wsel.appendChild(wo)
    }
    wsel.value = bubblePlanWinOf(m)
    wsel.style.flex = '1'
    wsel.style.minWidth = '0'
    wrow.appendChild(wsel)
    box.appendChild(wrow)
    dshwCustSel(wsel)
    wsel.addEventListener('change', function () { m.planWin = wsel.value || 'all'; changed() })
  }
  if (m.type === 'peak' || m.type === 'nextpeak') {
    // 显示样式(与编辑窗口一致)
    var srow = qRow()
    srow.appendChild(qLabel('显示样式'))
    var sel = document.createElement('select')
    for (var si = 0; si < BUBBLE_PEAK_STYLE_OPTS.length; si++) {
      var so = document.createElement('option')
      so.value = BUBBLE_PEAK_STYLE_OPTS[si][0]
      so.textContent = BUBBLE_PEAK_STYLE_OPTS[si][1]
      sel.appendChild(so)
    }
    sel.value = bubblePeakStyleOf(m)
    srow.appendChild(sel)
    sel.style.flex = '1'
    sel.style.minWidth = '0'
    box.appendChild(srow)
    dshwCustSel(sel)
    sel.addEventListener('change', function () { m.peakStyle = sel.value || 'default'; changed() })
    // 高峰/空闲 状态行(颜色+底色,与编辑窗口一致)
    var sts = [
      { label: '高峰色', colorKey: 'peakColor', rgbKey: 'peakRgb', defaultHex: '#e0433f', defaultText: '默认红', bgKey: 'peakBg', bgRgbKey: 'peakBgRgb', bgHex: '#fbe7e6' },
      { label: '空闲色', colorKey: 'offColor', rgbKey: 'offRgb', defaultHex: '#2fa24c', defaultText: '默认绿', bgKey: 'offBg', bgRgbKey: 'offBgRgb', bgHex: '#e4f3e7' },
    ]
    if (!m.peakColor) m.peakColor = '#e0433f'
    if (!m.offColor) m.offColor = '#2fa24c'
    for (var psi2 = 0; psi2 < sts.length; psi2++) {
      (function (st) {
        var grp = document.createElement('div')
        grp.className = 'dshwv-peakrow'
        var curCol = m[st.rgbKey] ? m[st.rgbKey] : 'solid'
        var cc2 = qColorSelectBuild(curCol, function (v) {
          if (v === 'solid') { m[st.rgbKey] = ''; if (!m[st.colorKey]) m[st.colorKey] = st.defaultHex }
          else { m[st.rgbKey] = v; m[st.colorKey] = '' }
          cc2.sync(v === 'solid' ? 'solid' : v, m[st.colorKey], function (hex) { m[st.colorKey] = hex; changed() })
          changed()
        }, { label: st.label, defaultHex: st.defaultHex, defaultText: st.defaultText })
        grp.appendChild(cc2.row)
        var bgV = m[st.bgRgbKey] ? m[st.bgRgbKey] : (m[st.bgKey] ? 'solid' : 'none')
        var bgHex0 = m[st.bgKey] || st.bgHex
        var bg2 = qColorSelectBuild(bgV, function (v) {
          if (v === 'none') { m[st.bgRgbKey] = ''; m[st.bgKey] = '' }
          else if (v === 'solid') { m[st.bgRgbKey] = ''; if (!m[st.bgKey]) m[st.bgKey] = bgHex0 }
          else { m[st.bgRgbKey] = v; m[st.bgKey] = '' }
          bg2.sync(v === 'none' ? 'none' : v, m[st.bgKey] || bgHex0, function (hex) { m[st.bgKey] = hex; changed() })
          changed()
        }, { label: '底色', defaultHex: bgHex0, defaultText: '默认', allowNone: true })
        grp.appendChild(bg2.row)
        box.appendChild(grp)
        cc2.sync(curCol, m[st.colorKey] || st.defaultHex, function (hex) { m[st.colorKey] = hex; changed() })
        bg2.sync(bgV, bgHex0, function (hex) { m[st.bgKey] = hex; changed() })
      })(sts[psi2])
    }
    // 与编辑窗口一致的通用样式:字体 → 字号 → 字形
    box.appendChild(bubbleFontEditRow(function () { return m.fontFamily || '' }, function (v) { m.fontFamily = v || ''; changed() }))
    sizeRow()
    glyphRow()
  } else {
    // balance / today:内容 → 字体 → 字号 → 字形 → 颜色+底色(同一行各占一半)
    box.appendChild(bubbleFontEditRow(function () { return m.fontFamily || '' }, function (v) { m.fontFamily = v || ''; changed() }))
    sizeRow()
    glyphRow()
    var grp2 = document.createElement('div')
    grp2.className = 'dshwv-peakrow'
    var curC = m.rgb ? m.rgb : 'solid'
    var ccA = qColorSelectBuild(curC, function (v) {
      if (v === 'solid') { m.rgb = ''; if (!m.color) m.color = '#203170' }
      else { m.rgb = v; m.color = '' }
      ccA.sync(v === 'solid' ? 'solid' : v, m.color, function (hex) { m.color = hex; changed() })
      changed()
    })
    grp2.appendChild(ccA.row)
    var bgCur = m.bgRgb ? m.bgRgb : (m.bg ? 'solid' : 'none')
    var bgcA = qColorSelectBuild(bgCur, function (v) {
      if (v === 'none') { m.bgRgb = ''; m.bg = '' }
      else if (v === 'solid') { m.bgRgb = ''; if (!m.bg) m.bg = '#dbe4f5' }
      else { m.bgRgb = v; m.bg = '' }
      bgcA.sync(v === 'none' ? 'none' : v, m.bg, function (hex) { m.bg = hex; changed() })
      changed()
    }, { label: '底色', defaultHex: '#dbe4f5', defaultText: '默认', allowNone: true })
    grp2.appendChild(bgcA.row)
    box.appendChild(grp2)
    ccA.sync(curC, m.color || '#203170', function (hex) { m.color = hex; changed() })
    bgcA.sync(bgCur, m.bg || '#dbe4f5', function (hex) { m.bg = hex; changed() })
  }
  // 位置:预览框上方
  try {
    var pr = bubblePvPrevEl.getBoundingClientRect()
    qeditPlace(pr, Math.max(260, Math.round(pr.width - 16)), true)
  } catch (err) { qeditPlace({ left: 40, right: 360, top: 200, bottom: 300, width: 320 }, 320, false) }
}
// —— 随机语句单句:悬浮编辑(内容+样式,无实例预览) ——
function openQuickSentenceEditor(line, mod, rowTx, anchorBtn) {
  if (!line) return
  if (qeditToggleClose(anchorBtn)) return // v723:同一行的 ✎ 再点 = 关闭
  qeditClose()
  var box = qeditEnsure()
  box.innerHTML = ''
  qeditCtx = { kind: 'line', line: line, mod: mod, anchor: anchorBtn || null }
  function lv(lk, mk, dft) { var v = line[lk]; if (v !== undefined && v !== null) return v; var mv = mod[lk]; if (mv !== undefined && mv !== null) return mv; return dft }
  // 内容
  var r0 = qRow()
  r0.appendChild(qLabel('句子'))
  var tx = document.createElement('input')
  tx.type = 'text'
  tx.className = 'dshwv-qedit-content'
  tx.value = line.t || ''
  tx.addEventListener('input', function () {
    line.t = tx.value || ' '
    try { if (rowTx) rowTx.value = tx.value || '' } catch (err) {}
  })
  r0.appendChild(tx)
  box.appendChild(r0)
  // 字体 + 字号
  box.appendChild(bubbleFontEditRow(function () { return lv('fontFamily', 'fontFamily', '') || '' }, function (v) { line.fontFamily = v || '' }))
  var r1 = qRow()
  r1.appendChild(qLabel('字号'))
  var sz = document.createElement('input')
  sz.type = 'range'
  sz.min = '1'
  sz.max = '50'
  sz.step = '1'
  sz.className = 'dshwv-range'
  sz.style.flex = '1'
  sz.value = String(Math.max(1, Math.min(50, Math.round(Number(lv('size', 'size', 6))))))
  var szNum = document.createElement('span')
  szNum.className = 'dshwv-volpct'
  szNum.textContent = sz.value
  sz.addEventListener('input', function () { line.size = Math.round(Number(sz.value) || 3); szNum.textContent = sz.value })
  r1.appendChild(sz)
  r1.appendChild(szNum)
  box.appendChild(r1)
  // 样式(行级显式 true/false,可真正取消)
  box.appendChild(qStyleChecksBuild(function (k) { return !!lv(k, k, false) }, function (k, v) { line[k] = v }))
  // 颜色
  var curColor = lv('rgb', 'rgb', '') || 'solid'
  var cc = qColorSelectBuild(curColor, function (v) { onPick(v) })
  box.appendChild(cc.row)
  function onPick(v) {
    if (v === 'solid') { line.rgb = ''; if (!line.color) line.color = '#203170' }
    else { line.rgb = v; line.color = '' }
    cc.sync(v === 'solid' ? 'solid' : v, line.color, function (hex) { line.color = hex })
  }
  cc.sync(curColor, line.color || mod.color || '#203170', function (hex) { line.color = hex })
  // 底色(仅该句;默认无;纯色/跑马灯)
  var lineBgV = (line.bgRgb) ? line.bgRgb : (line.bg ? 'solid' : (mod.bgRgb ? mod.bgRgb : (mod.bg ? 'solid' : 'none')))
  var lineBgHex0 = line.bg || mod.bg || '#dbe4f5'
  var bgcc2 = qColorSelectBuild(lineBgV, function (v) {
    if (v === 'none') { line.bgRgb = ''; line.bg = '' }
    else if (v === 'solid') { line.bgRgb = ''; if (!line.bg) line.bg = lineBgHex0 }
    else { line.bgRgb = v; line.bg = '' }
    bgcc2.sync(v === 'none' ? 'none' : v, line.bg || lineBgHex0, function (h) { line.bg = h })
  }, { label: '底色', defaultHex: lineBgHex0, defaultText: '默认', allowNone: true })
  box.appendChild(bgcc2.row)
  bgcc2.sync(lineBgV, lineBgHex0, function (h) { line.bg = h })
  // 位置:触发按钮下方
  var r = anchorBtn ? anchorBtn.getBoundingClientRect() : { left: 40, right: 360, top: 200, bottom: 260, width: 320 }
  qeditPlace(r, 340, false)
}
function renderBubblePal() {
  bubblePalEl.innerHTML = ''
  var defs = [
    { key: 'text', label: '文本', cb: function () { bubbleModuleAdd({ type: 'text', text: '新内容', size: 6, bold: true }) } },
    { key: 'balance', label: '总余额(充值金额+赠金)', pin: true, cb: function () { bubbleModuleAdd({ type: 'balance', size: 11, tpl: '{balance_ds}' }) } },
    // v782：把「总余额」拆出的两个分量各自做成模块 —— 赠金（未过期赠送余额）与充值余额。
    //   ⚠️ 上面那条 `balance` / `{balance_ds}` 的**代码名字与行为一律不动**（历史泡泡与已保存的模块里
    //   存的是 type/tpl 字面量，改代码名会让老内容显示不出来）；v787 只按用户要求改了**显示文案**。
    { key: 'bonus', label: '赠金', pin: true, cb: function () { bubbleModuleAdd({ type: 'bonus', size: 11, tpl: '{bonus_ds}' }) } },
    { key: 'recharge', label: '充值余额', pin: true, cb: function () { bubbleModuleAdd({ type: 'recharge', size: 11, tpl: '{recharge_ds}' }) } },
    { key: 'today', label: '今日已用', pin: true, cb: function () { bubbleModuleAdd({ type: 'today', size: 1, tpl: '今日已用 {expense_ds}' }) } },
    { key: 'peak', label: '峰谷时段', pin: true, cb: function () { bubbleModuleAdd({ type: 'peak', size: 4, peakColor: '#e0433f', offColor: '#2fa24c', tpl: '{status}' }) } },
    { key: 'nextpeak', label: '时段倒计时', pin: true, cb: function () { bubbleModuleAdd({ type: 'peak', size: 6, bold: true, peakStyle: 'count', peakColor: '#e0433f', offColor: '#2fa24c', tpl: '{countdown}' }) } },
    // v768：对话名模块（内容 = 当前对话标题；「保留长度」可在模块编辑器里改）
    { key: 'session', label: '对话名', pin: true, cb: function () { bubbleModuleAdd({ type: 'session', size: 4, bold: true, tpl: '{session}', len: WAIT_SESSION_MAX }) } },
    { key: 'random', label: '随机语句', cb: function () { bubbleModuleAdd(bubbleCloneModule(bubbleDefaultSecondModules()[0])) } },
    { key: 'link', label: '超链接', cb: function () { bubbleModuleAdd(bubblePaletteModule('link')) } },
    { key: 'image', label: '图片/动图', cb: function () { bubblePickImageToAdd() } },
    { key: 'randimg', label: '随机图片', cb: function () { bubbleModuleNew({ type: 'randimg', imgs: [], imgScale: 1 }) } },
  ]
  for (var cp of [['today','今日'],['week','近7天'],['month','本月'],['all','累计']]) defs.push({ key: 'codex-' + cp[0], label: 'Codex ' + cp[1] + ' token', cb: (function(p,l) { return function() { bubbleModuleAdd({type:'today',modelId:'codex',size:8,tpl:l + ' {tokens_' + p + '} tokens'}) } })(cp[0],cp[1]) })
  for (var i = 0; i < defs.length; i++) {
    (function (d) {
      var chip = document.createElement('div')
      chip.className = 'dshwv-palchip'
      chip.setAttribute('data-pal', d.key) // v639 触摸端长按拖拽用
      chip.textContent = d.label
      chip.title = d.pin ? '内置数值模块(内容锁定)' : '点击加入泡泡'
      chip.draggable = true
      chip.addEventListener('click', function (e) { e.stopPropagation(); d.cb() })
      chip.addEventListener('dragstart', function (e) {
        try { e.dataTransfer.setData('text/plain', d.key) } catch (err) {}
        bubbleDragKey = d.key
      })
      bubblePalEl.appendChild(chip)
    })(defs[i])
  }
  // 自定义 API 模型：每个模型一个「余额·<模型名>」模块（删除模型时由宿主级联清理泡泡配置）
  if (apiModelsLoaded) {
    apiModels.forEach(function (am) {
      if (!am || !am.id || am.builtin) return
      var planSupported = apiPlanSupport(am.id)
      // 只有「一个接口返回多个窗口」的额度厂商（如 OpenCode Go）才把调色板收成一个「额度」模块；
      // 单窗口的既有额度厂商（智谱 / Kimi / MiniMax Coding）保持上游原有的三个模块不变
      if (planSupported && apiPlanMultiWin(am.id)) {
        // 订阅额度厂商（OpenCode Go / 智谱 / Kimi / MiniMax Coding 等）：这类厂商本来就没有余额接口，
        // 只给一个「额度」模块 —— 时间窗口（5h / 周 / 月 / 全部）与显示内容都在模块编辑器里选，
        // 不再并列「余额 / 手动额度 / 订阅额度」三个模块。
        var key4 = 'pq:' + am.id
        var chip4 = document.createElement('div')
        chip4.className = 'dshwv-palchip'
        chip4.setAttribute('data-pal', key4)
        chip4.textContent = '额度·' + am.name
        var planWinHint = apiPlanMultiWin(am.id) ? '；「显示样式」可选时间窗口（全部 / 5h / 周 / 月）' : ''
        chip4.title = '该厂商的订阅额度（由厂商接口读取，非手动）；模板变量 {plan} 额度 / {plan_reset} 刷新时间 / {plan_left} 剩余' + planWinHint
        chip4.draggable = true
        chip4.addEventListener('click', function (e) {
          e.stopPropagation()
          bubbleModuleAdd({ type: 'plan', modelId: am.id, size: 8, tpl: '{plan} · {plan_reset}', planWin: 'all' })
        })
        chip4.addEventListener('dragstart', function (e) {
          try { e.dataTransfer.setData('text/plain', key4) } catch (err) {}
          bubbleDragKey = key4
        })
        bubblePalEl.appendChild(chip4)
        return
      }
      var key = 'bal:' + am.id
      var chip2 = document.createElement('div')
      chip2.className = 'dshwv-palchip'
      chip2.setAttribute('data-pal', key)
      chip2.textContent = '余额·' + am.name
      chip2.title = '该模型的余额' + (am.balanceMode === 'events' ? '（该厂商无余额接口，显示为 —）' : '') + '；模板变量 {balance} / {today}'
      chip2.draggable = true
      chip2.addEventListener('click', function (e) {
        e.stopPropagation()
        bubbleModuleAdd({ type: 'balance', modelId: am.id, size: 8, tpl: '{balance}' })
      })
      chip2.addEventListener('dragstart', function (e) {
        try { e.dataTransfer.setData('text/plain', key) } catch (err) {}
        bubbleDragKey = key
      })
      bubblePalEl.appendChild(chip2)
      // 手动额度模块（订阅 / 资源包模型）：palette key = qt:<modelId>
      var key3 = 'qt:' + am.id
      var chip3 = document.createElement('div')
      chip3.className = 'dshwv-palchip'
      chip3.setAttribute('data-pal', key3)
      chip3.textContent = '额度·' + am.name
      chip3.title = '该模型的手动额度（在模型菜单「额度（手动）」里填总量与已用）；模板变量 {quota} / {quota_used} / {quota_left} / {quota_total} / {quota_reset}'
      chip3.draggable = true
      chip3.addEventListener('click', function (e) {
        e.stopPropagation()
        bubbleModuleAdd({ type: 'quota', modelId: am.id, size: 8, tpl: '已用 {quota} · 剩 {quota_left}' })
      })
      chip3.addEventListener('dragstart', function (e) {
        try { e.dataTransfer.setData('text/plain', key3) } catch (err) {}
        bubbleDragKey = key3
      })
      bubblePalEl.appendChild(chip3)
    })
  } else if (apiModelsError) {
    var chipErr = document.createElement('div')
    chipErr.className = 'dshwv-palchip'
    chipErr.textContent = '⚠ 模型列表加载失败，点此重试'
    chipErr.style.opacity = '.85'
    chipErr.addEventListener('click', function (e) {
      e.stopPropagation()
      apiModelsError = ''
      loadApiModels(function () { try { renderBubblePal() } catch (err) {} }, true)
    })
    bubblePalEl.appendChild(chipErr)
  } else {
    var chipLd = document.createElement('div')
    chipLd.className = 'dshwv-palchip'
    chipLd.textContent = '余额·加载中…'
    chipLd.style.opacity = '.6'
    bubblePalEl.appendChild(chipLd)
    loadApiModels(function () { try { renderBubblePal() } catch (err) {} })
  }
  // 模块库条目(点击/拖入=复制加入;悬浮右上角 x 可删除,需二次确认)
  for (var li = 0; li < bubbleLib.length; li++) {
    (function (lb) {
      var chip = document.createElement('div')
      chip.className = 'dshwv-libchip'
      chip.title = '从模块库加入: ' + lb.name
      var body = document.createElement('div')
      body.className = 'dshwv-palchip'
      body.setAttribute('data-pal', 'lib:' + lb.id) // v639 触摸端长按拖拽用
      body.textContent = '▦ ' + lb.name
      body.draggable = true
      body.addEventListener('click', function (e) { e.stopPropagation(); bubbleModuleAdd(bubbleCloneModule(lb.module)) })
      body.addEventListener('dragstart', function (e) {
        try { e.dataTransfer.setData('text/plain', 'lib:' + lb.id) } catch (err) {}
        bubbleDragKey = 'lib:' + lb.id
      })
      chip.appendChild(body)
      var del = document.createElement('button')
      del.type = 'button'
      del.className = 'dshwv-libdel'
      del.textContent = '✕'
      del.title = '从模块库删除: ' + lb.name
      del.addEventListener('click', function (e) {
        e.stopPropagation()
        showConfirm('从模块库删除「' + lb.name + '」?', function () {
          bubbleLibDel(lb.id)
          if (bubblePalEl) renderBubblePal()
        })
      })
      chip.appendChild(del)
      bubblePalEl.appendChild(chip)
    })(bubbleLib[li])
  }
  // 新模块向导(可选类型)——参照「+ 添加语句」样式:虚线边框/透明底/圆角,尺寸紧凑
  var newChip = document.createElement('div')
  newChip.className = 'dshwv-paladd'
  newChip.textContent = '+ 新建模块'
  newChip.title = '新建模块(先选类型:文本/随机语句/图片动图/随机图片)'
  newChip.draggable = true
  newChip.addEventListener('click', function (e) { e.stopPropagation(); bubbleModuleWizard() })
  newChip.addEventListener('dragstart', function (e) {
    try { e.dataTransfer.setData('text/plain', 'wizard') } catch (err) {}
    bubbleDragKey = 'wizard'
  })
  bubblePalEl.appendChild(newChip)
}
function bubbleModuleWizard() {
  // 「新建模块」= 打开 W3 新增模式:顶部先选类型(文本/随机语句/图片动图/随机图片),再填内容
  bubbleModuleNew({ type: 'text', text: '新内容', size: 6 }, true)
}
function bubbleLibById(id) {
  for (var i = 0; i < bubbleLib.length; i++) if (bubbleLib[i].id === id) return bubbleLib[i]
  return null
}
var bubbleDragKey = null
// 当前编辑泡泡是否已含图片类模块
function bubbleItemHasImage() {
  try {
    var it = bubbleEditTarget()
    if (!it || !it.modules) return false
    for (var i = 0; i < it.modules.length; i++) if (bubbleIsImgMod(it.modules[i])) return true
  } catch (err) {}
  return false
}
// 提醒:一个泡泡只能有一个图片类模块(图片/动图 或 随机图片)
function bubbleWarnOneImage() {
  showConfirm('一个泡泡只能有一个图片类模块(图片/动图 或 随机图片)。当前泡泡已含图片,请先修改或删除现有图片模块后再添加。', function () {}, '知道了')
}
function bubbleModuleAdd(m) {
  try {
    var it = bubbleEditTarget()
    if (!it) return
    if (!it.modules) it.modules = []
    // 图片唯一:同泡泡已有图片时禁止再添加图片(含从模块库加入/拖入)
    if (m && bubbleIsImgMod(m) && bubbleItemHasImage()) { bubbleWarnOneImage(); return }
    // F2:新模块默认另起一行 → 行数已达上限(6)时不再追加
    if (bubbleRowsOf(it.modules).length >= BUBBLE_PV_ROW_MAX) { bubblePvWarn('泡泡最多 ' + BUBBLE_PV_ROW_MAX + ' 行,无法再加新行'); return }
    it.modules.push(m)
    renderBubblePv()
  } catch (err) {}
}
function bubbleModuleNew(m, isNew) {
  // 打开模块编辑(W3)新增:isNew=true 时顶部显示「类型」选择(新建模块向导)
  openModuleEditor(m, function (saved) {
    if (saved) bubbleModuleAdd(saved)
  }, isNew)
}
function bubblePickImageToAdd() {
  // 图片唯一:当前泡泡已有图片/动图时,先提醒,让用户修改/删除后再添加
  if (bubbleItemHasImage()) { bubbleWarnOneImage(); return }
  // 简易:先确保图库加载,打开 W3 图片模式作为新增
  bubbleModuleNew({ type: 'image', imgId: '', size: 6 })
}
// ===== F2「一行多模块」:W2 模块行编辑器数据操作(§7 第 3 步) =====
var BUBBLE_PV_ROW_MAX = 6 // 泡泡行数上限(编辑器侧防御,与渲染层一致)
var BUBBLE_PV_MOD_MAX = 6 // 同一行模块数上限(F2)
var bubbleModDrag = null // 正在拖的行内模块块 {ri, mi}
var bubblePvZone = '' // 当前拖放落点:pairL(行首)/pairR(行尾)/before/after/join
function bubblePvWarn(msg) { showConfirm(msg, function () {}, '知道了') }
function bubblePvRowModel() {
  var it = bubbleEditTarget()
  if (!it || !Array.isArray(it.modules)) return []
  return bubbleRowsOf(it.modules)
}
function bubblePvRowCommit(rows) {
  var it = bubbleEditTarget()
  if (!it) return
  it.modules = bubbleRowsFlat(rows)
  renderBubblePv()
}
function bubbleModuleEdit(m, anchorBtn) {
  try {
    if (m && (m.type === 'text' || m.type === 'link')) { openQuickTextEditor(m, anchorBtn); return }
    // v769：对话名也走悬浮编辑（像文本那样边改边看效果），不再开整窗编辑器
    // v782：赠金 / 充值余额同样是内置数值模块 ⇒ 也走悬浮编辑（内容锁定，只调字号/颜色/模板）
    if (m && (m.type === 'balance' || m.type === 'bonus' || m.type === 'recharge' || m.type === 'today' || m.type === 'peak' || m.type === 'nextpeak' || m.type === 'session')) { openQuickModuleEditor(m, anchorBtn); return }
    openModuleEditor(m, function (saved) { if (saved) renderBubblePv() })
  } catch (err) {}
}
function bubblePvDelBlock(ri, mi) {
  var rows = bubblePvRowModel()
  if (!rows[ri] || mi >= rows[ri].length) return
  rows[ri].splice(mi, 1)
  if (!rows[ri].length) rows.splice(ri, 1)
  bubblePvRowCommit(rows)
}
// 整行排序:把 from 行插到 to 行前/后(zone=before/after)
function bubblePvMoveRow(fromRow, toRow, zone) {
  var rows = bubblePvRowModel()
  if (fromRow < 0 || fromRow >= rows.length || toRow < 0 || toRow >= rows.length || fromRow === toRow) return
  var target = rows[toRow]
  var moved = rows.splice(fromRow, 1)[0]
  var t = rows.indexOf(target)
  if (t < 0) { rows.push(moved); bubblePvRowCommit(rows); return }
  rows.splice(zone === 'after' ? t + 1 : t, 0, moved)
  bubblePvRowCommit(rows)
}
// 把模块块移到目标行:左/右边缘=并入该行首/尾;上/下=另起一行插到该行上/下(拖回本行上/下=拆行)
function bubblePvDropBlock(riFrom, miFrom, riTarget, zone) {
  try {
    var rows = bubblePvRowModel()
    if (!rows[riFrom] || miFrom >= rows[riFrom].length || !rows[riTarget]) return
    var m = rows[riFrom][miFrom]
    if (!m || typeof m !== 'object') return
    var tRow = rows[riTarget]
    // 图片规则:图片永远独占一行 → 涉及图片的“并入”降级为“放它上方另起一行”
    var imageInvolved = bubbleIsImgMod(m) || bubbleIsImgMod(tRow[0])
    if (imageInvolved && (zone === 'pairL' || zone === 'pairR')) zone = 'before'
    rows[riFrom].splice(miFrom, 1)
    if (!rows[riFrom].length) rows.splice(riFrom, 1)
    var tIdx = -1
    for (var i = 0; i < rows.length; i++) if (rows[i] === tRow) { tIdx = i; break }
    if (tIdx >= 0 && (zone === 'pairL' || zone === 'pairR')) {
      var tgt = rows[tIdx]
      if (!bubbleIsImgMod(m) && !bubbleIsImgMod(tgt[0])) {
        if (tgt.length >= BUBBLE_PV_MOD_MAX) { bubblePvWarn('同一行最多 ' + BUBBLE_PV_MOD_MAX + ' 个模块,无法再并入'); return }
        tgt.splice(zone === 'pairL' ? 0 : tgt.length, 0, m)
        bubblePvRowCommit(rows)
        return
      }
      zone = 'before' // 图片相关 → 只能独立成行
    }
    if (rows.length >= BUBBLE_PV_ROW_MAX) { bubblePvWarn('泡泡最多 ' + BUBBLE_PV_ROW_MAX + ' 行,无法另起新行'); return }
    var at = tIdx >= 0 ? (zone === 'after' ? tIdx + 1 : tIdx) : Math.min(riFrom, rows.length)
    rows.splice(at, 0, [m])
    bubblePvRowCommit(rows)
  } catch (err) {}
}
// 把模块块移到末尾独立行(拖到列表下方空白区)
function bubblePvDropBlockEnd(riFrom, miFrom) {
  var rows = bubblePvRowModel()
  if (!rows[riFrom] || miFrom >= rows[riFrom].length) return
  var m = rows[riFrom][miFrom]
  rows[riFrom].splice(miFrom, 1)
  if (!rows[riFrom].length) rows.splice(riFrom, 1)
  if (rows.length >= BUBBLE_PV_ROW_MAX) { bubblePvWarn('泡泡最多 ' + BUBBLE_PV_ROW_MAX + ' 行,无法再另起一行'); return }
  rows.push([m])
  bubblePvRowCommit(rows)
}
// 把整行移到末尾
function bubblePvMoveRowEnd(fromRow) {
  var rows = bubblePvRowModel()
  if (fromRow < 0 || fromRow >= rows.length) return
  rows.push(rows.splice(fromRow, 1)[0])
  bubblePvRowCommit(rows)
}
// 行尾 ➕:往本行加一个默认文本模块(并排显示,可 ✎ 继续改;其余类型可从上方色板拖入本行)
function bubblePvAddToRow(ri) {
  var rows = bubblePvRowModel()
  if (!rows[ri]) return
  if (rows[ri].length >= BUBBLE_PV_MOD_MAX) { bubblePvWarn('同一行最多 ' + BUBBLE_PV_MOD_MAX + ' 个模块'); return }
  rows[ri].push({ type: 'text', text: '新内容', size: 6, bold: true })
  bubblePvRowCommit(rows)
}
// 调色板拖到某行 = 把该模块并入这一行(图片除外:图片只能单独一整行)
function bubblePvPaletteToRow(key, ri) {
  try {
    var rows = bubblePvRowModel()
    if (!rows[ri]) return
    var tgt = rows[ri]
    if (bubbleIsImgMod(tgt[0])) { bubblePvWarn('该行是图片类模块(图片/随机图片,独占一行):请拖到下方空白区另起一行'); return }
    if (key === 'image' || key === 'randimg') { bubblePvWarn('图片模块必须独占一整行:请拖到下方空白区新增'); return }
    if (key === 'wizard') key = 'text'
    var m = bubblePaletteModule(key)
    if (!m) return
    if (tgt.length >= BUBBLE_PV_MOD_MAX) { bubblePvWarn('同一行最多 ' + BUBBLE_PV_MOD_MAX + ' 个模块,无法再加入'); return }
    tgt.push(m)
    bubblePvRowCommit(rows)
  } catch (err) {}
}
// 调色板键 → 可加入的模块对象(random/lib 深拷贝;image 需走专门入口)
function bubblePaletteModule(key) {
  if (String(key).indexOf('codex-') === 0) return {type:'today',modelId:'codex',size:8,tpl:'{tokens_' + String(key).slice(6) + '} tokens'}
  if (key === 'text') return { type: 'text', text: '新内容', size: 6, bold: true }
  if (key === 'balance') return { type: 'balance', size: 11, tpl: '{balance_ds}' }
  // v782：赠金 / 充值余额（拖拽路径；与上面「总余额(充值金额+赠金)」并列的两个内置数值模块）
  if (key === 'bonus') return { type: 'bonus', size: 11, tpl: '{bonus_ds}' }
  if (key === 'recharge') return { type: 'recharge', size: 11, tpl: '{recharge_ds}' }
  if (key === 'today') return { type: 'today', size: 1, tpl: '今日已用 {expense_ds}' }
  if (key === 'peak') return { type: 'peak', size: 4, peakColor: '#e0433f', offColor: '#2fa24c', tpl: '{status}' }
  if (key === 'nextpeak') return { type: 'peak', size: 6, bold: true, peakStyle: 'count', peakColor: '#e0433f', offColor: '#2fa24c', tpl: '{countdown}' }
  if (key === 'session') return { type: 'session', size: 4, bold: true, tpl: '{session}', len: WAIT_SESSION_MAX }
  if (key === 'link') return { type: 'link', text: '打开链接', url: '', size: 6, color: '#2f4488' }
  if (key === 'randimg') return { type: 'randimg', imgs: [], imgScale: 1 }
  // 自定义 API 模型的余额模块（palette key = bal:<modelId>）
  if (typeof key === 'string' && key.indexOf('bal:') === 0) {
    var am0 = apiModelById(key.slice(4))
    if (!am0) return null
    return { type: 'balance', modelId: am0.id, size: 8, tpl: '{balance}' }
  }
  // 自定义 API 模型的手动额度模块（palette key = qt:<modelId>）
  if (typeof key === 'string' && key.indexOf('qt:') === 0) {
    var am1 = apiModelById(key.slice(3))
    if (!am1) return null
    return { type: 'quota', modelId: am1.id, size: 8, tpl: '已用 {quota} · 剩 {quota_left}' }
  }
  // 厂商订阅额度模块（palette key = pq:<modelId>）：时间窗口由模块里的「显示样式」决定
  if (typeof key === 'string' && key.indexOf('pq:') === 0) {
    var am2 = apiModelById(key.slice(3))
    if (!am2) return null
    return { type: 'plan', modelId: am2.id, size: 8, tpl: '{plan} · {plan_reset}', planWin: 'all' }
  }
  if (key === 'random') return bubbleCloneModule(bubbleDefaultSecondModules()[0])
  if (typeof key === 'string' && key.indexOf('lib:') === 0) {
    var lb = bubbleLibById(key.slice(4))
    return lb ? bubbleCloneModule(lb.module) : null
  }
  return null
}
// —— W2「编辑 第n次点击内容」触摸端自研拖拽(v639) ——
// 桌面用 HTML5 拖放,触摸端不工作且可能卡死;这里长按 400ms 进入拖拽,落点规则与桌面 dragover 一致:
//   · 模块块:拖到某行左/右 20% 边带 = 并入该行首/尾;上/下半 = 另起一行插到该行上/下;空白区 = 另起一行到末尾
//   · 行手柄 ⠿:整行排序(仅上/下);空白区 = 移到最后
//   · 调色板/模块库 chip:拖到某行 = 并入该行;空白区 = 新增(与桌面 drop 行为一致)
var PV_TOUCH_HOLD_MS = 400
var PV_TOUCH_SLOP = 8
var PV_TOUCH_BAND = 0.2
var pvTouchArm = null // {kind, el, ri, mi, key, x, y, timer}
var pvTouchDrag = null // {kind, el, ri, mi, key, y0, dy, ri2, zone}
var pvTouchSuppressAt = 0 // 长按拖拽后抑制随之而来的 click(避免误触 ✎/✕/chip 自带点击)
function pvTouchBars() {
  try { return bubblePvEl ? [].slice.call(bubblePvEl.querySelectorAll('.dshwv-pvrow')) : [] } catch (err) { return [] }
}
function pvTouchBarIndexOf(bar) {
  var bars = pvTouchBars()
  for (var i = 0; i < bars.length; i++) if (bars[i] === bar) return i
  return -1
}
function pvTouchClearHighlight() {
  var bars = pvTouchBars()
  for (var i = 0; i < bars.length; i++) { bars[i].style.boxShadow = ''; bars[i].style.outline = '' }
}
function pvTouchArmCancel() {
  if (pvTouchArm && pvTouchArm.timer) clearTimeout(pvTouchArm.timer)
  pvTouchArm = null
}
function pvTouchDetach() {
  try {
    document.removeEventListener('touchmove', pvTouchMove, true)
    document.removeEventListener('touchend', pvTouchEnd, true)
    document.removeEventListener('touchcancel', pvTouchEnd, true)
  } catch (err) {}
}
function pvTouchEnter() {
  var a = pvTouchArm
  if (!a) return
  pvTouchArm = null
  pvTouchDrag = { kind: a.kind, el: a.el, ri: a.ri, mi: a.mi, key: a.key, y0: a.y, dy: 0, ri2: -1, zone: '' }
  pvTouchSuppressAt = Date.now() + 700
  try {
    a.el.classList.add('dshwv-pv-dragging')
    if (navigator && navigator.vibrate) navigator.vibrate(10)
  } catch (err) {}
}
function pvTouchStart(e) {
  try {
    if (pvTouchDrag || pvTouchArm) return
    if (!e.touches || e.touches.length !== 1) return
    var t = e.touches[0]
    var el = e.target
    if (!el || !el.closest) return
    // ✎/✕/➕ 等按钮保留点击语义,不参与长按拖拽(模块块的可拖区域是它的文字块)
    if (el.closest('button')) return
    var bar = el.closest('.dshwv-pvrow')
    var grip = el.closest('.dshwv-pvdrag')
    var blk = el.closest('.dshwv-pvmod')
    var pal = el.closest('[data-pal]')
    var kind = '', ri = -1, mi = -1, key = ''
    if (pal) { kind = 'pal'; key = pal.getAttribute('data-pal') || ''; el = pal }
    else if (grip && bar) { kind = 'row'; ri = pvTouchBarIndexOf(bar); el = grip }
    else if (blk && bar) {
      kind = 'mod'
      ri = pvTouchBarIndexOf(bar)
      mi = [].slice.call(bar.querySelectorAll('.dshwv-pvmod')).indexOf(blk)
      el = blk
    } else return
    if (kind === 'pal') { if (!key) return }
    else if (ri < 0 || (kind === 'mod' && mi < 0)) return
    // 本次手势内关掉该元素的原生可拖(document 级 dragstart 还有一层兜底)
    try { el.draggable = false } catch (err) {}
    pvTouchArm = { kind: kind, el: el, ri: ri, mi: mi, key: key, x: t.clientX, y: t.clientY, timer: setTimeout(pvTouchEnter, PV_TOUCH_HOLD_MS) }
    document.addEventListener('touchmove', pvTouchMove, { capture: true, passive: false })
    document.addEventListener('touchend', pvTouchEnd, true)
    document.addEventListener('touchcancel', pvTouchEnd, true)
  } catch (err) {}
}
function pvTouchMove(e) {
  try {
    var t = e.touches && e.touches[0]
    if (!t) return
    if (pvTouchArm) {
      var dx = t.clientX - pvTouchArm.x
      var dy = t.clientY - pvTouchArm.y
      if (dx * dx + dy * dy > PV_TOUCH_SLOP * PV_TOUCH_SLOP) pvTouchArmCancel()
      return
    }
    if (!pvTouchDrag) return
    if (e.touches.length > 1) { pvTouchFinish(false); return }
    try { e.preventDefault() } catch (err) {} // 拖拽期间吃掉滚动
    pvTouchDrag.dy = t.clientY - pvTouchDrag.y0
    try { pvTouchDrag.el.style.transform = 'translateY(' + pvTouchDrag.dy + 'px)' } catch (err) {}
    // 落点判定:与桌面 dragover 同一套规则
    var bars = pvTouchBars()
    var ri2 = -1
    var zone = ''
    for (var i = 0; i < bars.length; i++) {
      var rc = bars[i].getBoundingClientRect()
      if (!rc.width) continue
      if (t.clientY < rc.top || t.clientY > rc.bottom) continue
      ri2 = i
      if (pvTouchDrag.kind === 'pal') { zone = 'join'; break }
      var xRel = (t.clientX - rc.left) / rc.width
      if (pvTouchDrag.kind === 'mod' && xRel < PV_TOUCH_BAND) { zone = 'pairL'; break }
      if (pvTouchDrag.kind === 'mod' && xRel > 1 - PV_TOUCH_BAND) { zone = 'pairR'; break }
      zone = (t.clientY < rc.top + rc.height / 2) ? 'before' : 'after'
      break
    }
    pvTouchDrag.ri2 = ri2
    pvTouchDrag.zone = zone
    pvTouchClearHighlight()
    if (ri2 >= 0 && bars[ri2]) {
      if (zone === 'join') bars[ri2].style.outline = '2px solid rgba(32,49,112,.55)'
      else {
        var sh = bubbleDropShadow(zone)
        if (sh) bars[ri2].style.boxShadow = sh
      }
    }
  } catch (err) {}
}
function pvTouchFinish(apply) {
  var d = pvTouchDrag
  pvTouchDrag = null
  pvTouchArmCancel()
  pvTouchDetach()
  if (!d) return
  pvTouchSuppressAt = Date.now() + 700
  try {
    d.el.classList.remove('dshwv-pv-dragging')
    d.el.style.transform = ''
    pvTouchClearHighlight()
  } catch (err) {}
  if (!apply) return
  try {
    if (d.kind === 'row') {
      if (d.ri2 < 0) { bubblePvMoveRowEnd(d.ri); return }
      if (d.ri2 !== d.ri) bubblePvMoveRow(d.ri, d.ri2, d.zone === 'after' ? 'after' : 'before')
      return
    }
    if (d.kind === 'mod') {
      if (d.ri2 < 0) { bubblePvDropBlockEnd(d.ri, d.mi); return }
      var z = (d.zone === 'pairL' || d.zone === 'pairR' || d.zone === 'after') ? d.zone : 'before'
      bubblePvDropBlock(d.ri, d.mi, d.ri2, z)
      return
    }
    if (d.kind === 'pal') {
      if (d.ri2 < 0) {
        if (d.key === 'image') { bubblePickImageToAdd(); return }
        var m = bubblePaletteModule(d.key)
        if (m) bubbleModuleAdd(m)
        return
      }
      bubblePvPaletteToRow(d.key, d.ri2)
      return
    }
  } catch (err) {}
}
function pvTouchEnd(e) {
  try { if (e && e.touches && e.touches.length > 0) return } catch (err) {}
  pvTouchFinish(true)
}
document.addEventListener('touchstart', pvTouchStart, { passive: true })
// 长按拖拽成立后,抑制随之而来的那次 click(否则会顺带打开模块编辑窗/误删/重复新增)
document.addEventListener('click', function (e) {
  try {
    if (Date.now() >= pvTouchSuppressAt) return
    if (!e.target || !e.target.closest) return
    if (!e.target.closest('.dshwv-bubpvbox') && !e.target.closest('.dshwv-bubpal') && !e.target.closest('[data-pal]')) return
    e.preventDefault()
    e.stopPropagation()
  } catch (err) {}
}, true)
function bubblePvFont(level) {
  // 预览字号(px 近似):与真实 --dshw-u 倍数保持比例观感
  var mult = bubbleModuleFontU(level)
  return Math.max(10, Math.round(mult * 0.42))
}
function renderBubblePv() {
  bubblePvEl.innerHTML = ''
  var it = bubbleEditTarget()
  if (!it) return
  var rows = bubbleRowsOf(it.modules || [])
  for (var r = 0; r < rows.length; r++) {
    (function (ri, rowMods) {
      var isImgRow = bubbleIsImgMod(rowMods[0])
      var bar = document.createElement('div')
      bar.className = 'dshwv-pvrow dshwv-pvrowline'
      bar.title = isImgRow ? (bubbleModuleSummary(rowMods[0]) + ' 独占一行:拖 ⠿ 可整行排序') : '同一行模块并排(≤6):拖 ⠿ 整行排序;拖模块块到某行左/右边缘=并入该行,上/下=另起一行'
      // 行手柄:整行上下移动
      var grip = document.createElement('span')
      grip.className = 'dshwv-pvdrag'
      grip.textContent = '⠿'
      grip.title = '按住拖动整行排序'
      grip.draggable = true
      grip.addEventListener('dragstart', function (e) {
        e.stopPropagation()
        try { e.dataTransfer.setData('text/plain', 'prow:' + ri) } catch (err) {}
        bubbleRowDragIdx = ri
        bubbleModDrag = null
        bubblePvZone = ''
      })
      grip.addEventListener('dragend', function () { bubbleRowDragIdx = null; bubblePvZone = '' })
      bar.appendChild(grip)
      // 行内模块块:每块自带 ✎/✕/拖动
      for (var mi = 0; mi < rowMods.length; mi++) {
        (function (m, mIdx) {
          var blk = document.createElement('div')
          blk.className = 'dshwv-pvmod' + (bubbleIsImgMod(m) ? ' dshwv-pvimg' : '')
          blk.draggable = true
          blk.title = isImgRow ? (bubbleModuleSummary(m) + '(独占一行,可整行排序)') : '拖动到某行:左/右边缘=并入该行首/尾,上/下=另起一行'
          blk.addEventListener('dragstart', function (e) {
            e.stopPropagation()
            try { e.dataTransfer.setData('text/plain', 'mod:' + ri + ':' + mIdx) } catch (err) {}
            bubbleRowDragIdx = null
            bubbleModDrag = { ri: ri, mi: mIdx }
            bubblePvZone = ''
          })
          blk.addEventListener('dragend', function () { bubbleModDrag = null; bubblePvZone = '' })
          var lab = document.createElement('span')
          lab.className = 'dshwv-pvlab'
          lab.textContent = bubbleModuleListLabel(m)
          lab.title = '点击编辑该模块(内容/样式)'
          lab.addEventListener('click', function (e) {
            e.stopPropagation()
            bubbleModuleEdit(m, lab)
          })
          blk.appendChild(lab)
          var ed = document.createElement('button')
          ed.type = 'button'
          ed.className = 'dshwv-bubmini'
          ed.textContent = '✎'
          ed.title = '编辑该模块(内容/样式)'
          ed.addEventListener('click', function (e) {
            e.stopPropagation()
            bubbleModuleEdit(m, ed)
          })
          blk.appendChild(ed)
          var del = document.createElement('button')
          del.type = 'button'
          del.className = 'dshwv-bubmini'
          del.textContent = '✕'
          del.title = '删除该模块'
          del.addEventListener('click', function (e) { e.stopPropagation(); bubblePvDelBlock(ri, mIdx) })
          blk.appendChild(del)
          bar.appendChild(blk)
        })(rowMods[mi], mi)
      }
      // 行尾 ➕:往本行加模块(图片行不加:图片独占一行)
      if (!isImgRow) {
        var add = document.createElement('button')
        add.type = 'button'
        add.className = 'dshwv-pvadd'
        add.textContent = '+'
        add.title = '把模块加入同一行(默认文本,并排显示;其余类型可把上方色板拖进本行)'
        add.addEventListener('click', function (e) { e.stopPropagation(); bubblePvAddToRow(ri) })
        bar.appendChild(add)
      }
      // —— 行作为拖放目标 ——
      function highlight(zone) {
        bubblePvZone = zone
        bar.style.boxShadow = ''
        bar.style.outline = ''
        if (zone === 'join') bar.style.outline = '2px solid rgba(32,49,112,.55)'
        else {
          var sh = bubbleDropShadow(zone)
          if (sh) bar.style.boxShadow = sh
        }
      }
      bar.addEventListener('dragover', function (e) {
        try {
          var isMod = !!bubbleModDrag
          var isRow = (bubbleRowDragIdx !== null && bubbleRowDragIdx !== undefined)
          var isPal = !isMod && !isRow && !!bubbleDragKey
          if (!isMod && !isRow && !isPal) return
          e.preventDefault()
          e.dataTransfer.dropEffect = 'move'
          var rc = bar.getBoundingClientRect()
          if (!rc.width) return
          if (isPal) { highlight('join'); return }
          var x = e.clientX - rc.left
          if (isMod && x < rc.width * 0.2) { highlight('pairL'); return }
          if (isMod && x > rc.width * 0.8) { highlight('pairR'); return }
          highlight((e.clientY - rc.top) < rc.height / 2 ? 'before' : 'after')
        } catch (err) {}
      })
      bar.addEventListener('dragleave', function () { bubblePvZone = ''; bar.style.boxShadow = ''; bar.style.outline = '' })
      bar.addEventListener('drop', function (e) {
        try {
          e.preventDefault()
          e.stopPropagation()
          var zone = bubblePvZone
          bubblePvZone = ''
          bar.style.boxShadow = ''
          bar.style.outline = ''
          if (bubbleRowDragIdx !== null && bubbleRowDragIdx !== undefined) {
            var fromRow = bubbleRowDragIdx
            bubbleRowDragIdx = null
            if (fromRow !== ri) bubblePvMoveRow(fromRow, ri, zone)
            return
          }
          if (bubbleModDrag) {
            var md = bubbleModDrag
            bubbleModDrag = null
            if (zone === 'join') zone = 'after'
            bubblePvDropBlock(md.ri, md.mi, ri, zone)
            return
          }
          if (bubbleDragKey) {
            var key = bubbleDragKey
            bubbleDragKey = null
            bubblePvPaletteToRow(key, ri)
            return
          }
        } catch (err) {}
      })
      bubblePvEl.appendChild(bar)
    })(r, rows[r])
  }
  // 真实泡泡形预览
  try {
    var rw = (root && (root.offsetWidth || root.getBoundingClientRect().width)) || 280
    bubblePreviewInto(bubblePvPrevEl, it.modules || [], Math.min(rw, 408))
  } catch (err) {}
}
var bubbleRowDragIdx = null
// 「取消」确认:把该步还原为打开 W2 时的快照并关闭(未保存的编辑/恢复默认全部丢弃)
function bubbleItemDiscard() {
  try {
    var idx = bubbleEditItemIdx
    var snap = bubbleItemSnap
    bubbleItemSnap = null
    if (idx >= 0 && idx < bubbleEditItems.length && snap) bubbleEditItems[idx] = snap
  } catch (err) {}
  closeBubbleItem()
  renderBubbleFirst()
  renderBubbleMore()
}
function bubbleItemSave() {
  showConfirm('保存该泡泡内容?', function () {
    var it = bubbleEditTarget()
    if (it) {
      it.kind = 'custom'
      if (!it.modules) it.modules = []
    }
    bubbleItemSnap = null
    closeBubbleItem()
    renderBubbleFirst()
    renderBubbleMore()
  })
}
function bubbleItemResetToDefault() {
  // 用户 v630 确认:任何"第n次点击"泡(含并列 A/B)恢复默认 = 还原成首次点击泡的内容(靛蓝余额卡)
  showConfirm('恢复该泡泡为默认内容(与首次点击泡一致)?', function () {
    var it = bubbleEditTarget()
    if (it) {
      var mods = []
      try {
        var src = null
        // 优先取当前生效配置的首次点击泡;无配置/为并列时回退到 v615 冻结出厂默认第 1 泡
        if (bubbleCfg && Array.isArray(bubbleCfg.items) && bubbleCfg.items.length) {
          var f0 = bubbleCfg.items[0]
          if (f0 && !bubbleIsChoice(f0) && Array.isArray(f0.modules)) src = f0
        }
        if (!src && BUBBLE_DEFAULT_ITEMS && BUBBLE_DEFAULT_ITEMS.length) {
          var d0 = BUBBLE_DEFAULT_ITEMS[0]
          if (d0 && !bubbleIsChoice(d0) && Array.isArray(d0.modules)) src = d0
        }
        if (src) mods = JSON.parse(JSON.stringify(src.modules))
      } catch (err) {}
      it.kind = 'custom'
      it.modules = mods
    }
    renderBubblePv()
  })
}
bubbleMask = document.createElement('div')
bubbleMask.className = 'dshwv-bubmask'
bubbleMask.style.display = 'none'
var bubbleCard = document.createElement('div')
bubbleCard.className = 'dshwv-bubcard'
var bubbleTitle = document.createElement('div')
bubbleTitle.className = 'dshwv-bubtitle'
bubbleTitle.textContent = '自定义泡泡'
bubbleCard.appendChild(bubbleTitle)
// 首次点击(固定一项)
var bubbleSecFirst = document.createElement('div')
bubbleSecFirst.className = 'dshwv-bubsec dshwv-bubsec-first'
bubbleSecFirst.textContent = '首次点击弹出内容'
bubbleCard.appendChild(bubbleSecFirst)
var bubbleFirstRow = document.createElement('div')
bubbleFirstRow.className = 'dshwv-bubrow'
bubbleFirstChipEl = document.createElement('div')
bubbleFirstChipEl.className = 'dshwv-bubchip'
bubbleFirstChipEl.title = '点击编辑该泡泡的内容模块'
bubbleFirstChipEl.addEventListener('click', function (e) { e.stopPropagation(); openBubbleItem(0) })
bubbleFirstRow.appendChild(bubbleFirstChipEl)
bubbleCard.appendChild(bubbleFirstRow)
// 再次点击(可多条、可排序)
var bubbleSecMore = document.createElement('div')
bubbleSecMore.className = 'dshwv-bubsec'
bubbleSecMore.textContent = '再次点击弹出内容'
bubbleCard.appendChild(bubbleSecMore)
bubbleMoreListEl = document.createElement('div')
bubbleMoreListEl.addEventListener('dragover', function (e) {
  try {
    if (e.target && e.target.closest && e.target.closest('.dshwv-bubrow-drag')) return
    e.preventDefault()
  } catch (err) {}
})
bubbleMoreListEl.addEventListener('drop', function (e) {
  try {
    if (e.target && e.target.closest && e.target.closest('.dshwv-bubrow-drag')) return
    e.preventDefault()
    bubbleDropToEnd()
  } catch (err) {}
})
bubbleCard.appendChild(bubbleMoreListEl)
var bubbleAddBtn = document.createElement('button')
bubbleAddBtn.type = 'button'
bubbleAddBtn.className = 'dshwv-bubadd'
bubbleAddBtn.textContent = '+ 添加泡泡(点完上一个后显示下一个)'
bubbleAddBtn.addEventListener('click', bubbleAddMore)
bubbleCard.appendChild(bubbleAddBtn)
// v727：点按角色推进泡泡队列（设置只在「自定义泡泡」窗口里，随本窗口的「保存」一起落盘）
var bubbleTapAdvRow = document.createElement('div')
bubbleTapAdvRow.className = 'dshwv-bubsec'
bubbleTapAdvRow.style.display = 'flex'
bubbleTapAdvRow.style.alignItems = 'center'
bubbleTapAdvRow.style.flexWrap = 'wrap'
bubbleTapAdvRow.style.gap = '4px 6px'
bubbleTapAdvRow.style.color = '#203170'
var bubbleTapAdvChk = document.createElement('input')
bubbleTapAdvChk.type = 'checkbox'
bubbleTapAdvChk.className = 'dshwv-check'
bubbleTapAdvChk.id = 'dshwv-tapadv'
// v777e（2026-09-27 用户要求）：本行复选框**不再挂原生 tooltip**（原来那句"开启后：点一下角色=往后推进一项…"
// 与问号圈圈里的说明重复）。说明统一收进 ? 圈圈的弹层里（与「提示与音效设置」面板底部那行一致）。
bubbleTapAdvChk.title = ''
var bubbleTapAdvLab = document.createElement('label')
bubbleTapAdvLab.setAttribute('for', 'dshwv-tapadv')
bubbleTapAdvLab.style.cursor = 'pointer'
bubbleTapAdvLab.style.fontSize = '12px'
bubbleTapAdvLab.textContent = '点按角色推进泡泡队列'
// v777（用户要求）：原来跟在标签后面的那一行说明字（「关闭＝点角色回到第 1 个泡泡；开启＝点一下往后一个」）
// 收进问号圈圈里 —— 复用 v647 的 dshwvAskDot（悬停即显示、点击固定、触摸端点按开关），与
// 「提示与音效设置」面板底部那一行同款。说明里顺带把原来 title 里的完整描述也写进去。
// v777b（2026-09-27 追加要求）：? 圈圈放在**文字后面**，并把与文字的间距收紧（同面板那处）。
// v777d：「再稍稍右移」⇒ margin-left 由 -3px 改成 0（间距 = 行的自然列间距 6px）。
var bubbleTapAdvAsk = dshwvAskDot(
  '关闭（默认）：<b>点角色</b>＝回到第 1 个泡泡（点泡泡才是往后推进）。<br><br>' +
  '开启：<b>点一下角色 = 往后推进一项</b>（不再回到首次点击泡泡）；走到最后一项再点＝收起泡泡。'
)
bubbleTapAdvAsk.style.marginLeft = '0'
bubbleTapAdvAsk.style.marginRight = '0'
// v777e：与面板底部那行同款处理 —— 清掉问号圈圈自带的「查看说明」原生 tooltip，
//        说明只在悬停/点击问号时以自绘弹层出现（本行鼠标悬浮不再出现浏览器小黄条）。
bubbleTapAdvAsk.title = ''
bubbleTapAdvRow.appendChild(bubbleTapAdvChk)
bubbleTapAdvRow.appendChild(bubbleTapAdvLab)
bubbleTapAdvRow.appendChild(bubbleTapAdvAsk)
bubbleCard.appendChild(bubbleTapAdvRow)
// 按钮行
var bubbleBtns = document.createElement('div')
bubbleBtns.className = 'dshwv-bubbtns'
function bubbleBtn(label, cls, fn) {
  var b = document.createElement('button')
  b.type = 'button'
  b.className = 'dshwv-bubbtn ' + cls
  b.textContent = label
  b.addEventListener('click', function (e) { e.stopPropagation(); fn() })
  return b
}
bubbleBtns.appendChild(bubbleBtn('取消', 'dshwv-bubbtn-no', function () {
  // 未做任何修改:直接关闭,不再二次确认
  if (bubbleEditorDirty()) showConfirm('放弃未保存的更改?', function () { closeBubbleEditor() })
  else closeBubbleEditor()
}))
bubbleBtns.appendChild(bubbleBtn('重置', 'dshwv-bubbtn-no', bubbleEditorReset))
bubbleBtns.appendChild(bubbleBtn('保存', 'dshwv-bubbtn-ok', bubbleEditorSave))
bubbleCard.appendChild(bubbleBtns)
bubbleMask.appendChild(bubbleCard)
dshwBodyAppend(bubbleMask)

// ===== W2 单泡编辑窗口 =====
bubbleItemMask = document.createElement('div')
bubbleItemMask.className = 'dshwv-bubmask'
bubbleItemMask.style.display = 'none'
var bubbleItemCard = document.createElement('div')
bubbleItemCard.className = 'dshwv-bubcard'
bubbleItemTitleEl = document.createElement('div')
bubbleItemTitleEl.className = 'dshwv-bubtitle'
bubbleItemCard.appendChild(bubbleItemTitleEl)
bubbleItemSideEl = document.createElement('div')
bubbleItemSideEl.className = 'dshwv-sidebar'
bubbleItemSideEl.style.display = 'none'
bubbleItemCard.appendChild(bubbleItemSideEl)
var bubbleSecPal = document.createElement('div')
bubbleSecPal.className = 'dshwv-bubsec dshwv-bubsec-first dshwv-bubsec-withq'
bubbleSecPal.textContent = '可选模块'
// 原「可选模块(点击或拖入…)」与「泡泡内容预览(同一行并排…拖 ⠿ 整行排序)」的说明收进「?」圈
bubbleSecPal.insertBefore(dshwvAskDot(
  '<div style="font-weight:600;margin-bottom:4px">可选模块 &amp; 泡泡内容预览</div>' +
  '<div>点击「可选模块」即可加入泡泡;桌面端也可直接拖到下方泡泡框。</div>' +
  '<div style="margin-top:4px">同一行模块并排显示(≤6 个):</div>' +
  '<div>· 拖模块块到某行左/右边缘 = 并入该行</div>' +
  '<div>· 拖到某行上/下 = 另起一行(拖回本行上下 = 拆行)</div>' +
  '<div>· 拖 ⠿ 手柄 = 整行排序</div>' +
  '<div style="margin-top:4px;opacity:.75">手机端:长按约 0.4 秒进入拖动</div>'
), bubbleSecPal.firstChild)
bubbleItemCard.appendChild(bubbleSecPal)
bubblePalEl = document.createElement('div')
bubblePalEl.className = 'dshwv-bubpal'
bubbleItemCard.appendChild(bubblePalEl)
var bubbleSecPv = document.createElement('div')
bubbleSecPv.className = 'dshwv-bubsec'
bubbleSecPv.textContent = '泡泡内容预览'
bubbleItemCard.appendChild(bubbleSecPv)
bubblePvEl = document.createElement('div')
bubblePvEl.className = 'dshwv-bubpvbox'
bubblePvEl.addEventListener('dragover', function (e) {
  try {
    // 行内(含块/行)由行自身处理;空白区负责“拖到底部另起一行/新增”
    if (e.target && e.target.closest && e.target.closest('.dshwv-pvrow')) return
    e.preventDefault()
  } catch (err) {}
})
bubblePvEl.addEventListener('drop', function (e) {
  try {
    if (e.target && e.target.closest && e.target.closest('.dshwv-pvrow')) return
    e.preventDefault()
    if (bubbleRowDragIdx !== null && bubbleRowDragIdx !== undefined) {
      var fr = bubbleRowDragIdx
      bubbleRowDragIdx = null
      bubblePvMoveRowEnd(fr)
      return
    }
    if (bubbleModDrag) {
      var mdd = bubbleModDrag
      bubbleModDrag = null
      bubblePvDropBlockEnd(mdd.ri, mdd.mi)
      return
    }
    var key = bubbleDragKey
    if (!key) return
    bubbleDragKey = null
    if (key === 'image') { bubblePickImageToAdd(); return }
    if (key === 'wizard') { bubbleModuleWizard(); return }
    var m = bubblePaletteModule(key)
    if (m) bubbleModuleAdd(m)
  } catch (err) {}
})
bubbleItemCard.appendChild(bubblePvEl)
// 真实泡泡形预览(模块行按 --dshw-u 等比绘制)
var bubblePvPrevEl = document.createElement('div')
bubblePvPrevEl.className = 'dshwv-bubprev'
bubbleItemCard.appendChild(bubblePvPrevEl)
var bubbleItemBtns = document.createElement('div')
bubbleItemBtns.className = 'dshwv-bubbtns'
bubbleItemBtns.appendChild(bubbleBtn('取消', 'dshwv-bubbtn-no', function () {
  showConfirm('放弃该泡泡的未保存修改?', function () { bubbleItemDiscard() })
}))
bubbleItemBtns.appendChild(bubbleBtn('恢复默认', 'dshwv-bubbtn-no', bubbleItemResetToDefault))
bubbleItemBtns.appendChild(bubbleBtn('保存', 'dshwv-bubbtn-ok', bubbleItemSave))
bubbleItemCard.appendChild(bubbleItemBtns)
bubbleItemMask.appendChild(bubbleItemCard)
dshwBodyAppend(bubbleItemMask)

// ===== W3 模块编辑器(文本/随机/图片 + 样式) =====
var moduleMask = null
var moduleEditRef = null
var moduleOnSave = null
var moduleEditNew = false
var moduleTitleEl = null
var moduleTypeLabelEl = null
var moduleBodyEl = null
var moduleColorEl = null
var moduleSizeEl = null
var moduleImgListEl = null
var moduleImgSelect = null
var moduleImgPreviewEl = null
var bubbleImgList = []
function loadBubbleImgs(cb) {
  try {
    fetch('/dsh-whale/bubble-imgs.json', { cache: 'no-store' })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (d && d.ok && Array.isArray(d.images)) { bubbleImgList = d.images; if (cb) cb() }
      })
      .catch(function () {})
  } catch (err) {}
}
function bubbleUploadImg(file, cb) {
  try {
    var fr = new FileReader()
    fr.onload = function () {
      var data = String(fr.result || '')
      if (data.indexOf('data:image/png;base64,') !== 0 && data.indexOf('data:image/gif;base64,') !== 0) { if (cb) cb(false); return }
      fetch('/dsh-whale/bubble-img-upload.json', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'upload', name: file.name || '', data: data }),
      })
        .then(function (r) { return r.json() })
        .then(function (d) {
          if (d && d.ok && Array.isArray(d.images)) { bubbleImgList = d.images; if (cb) cb(true) }
          else if (cb) cb(false)
        })
        .catch(function () { if (cb) cb(false) })
    }
    fr.onerror = function () { if (cb) cb(false) }
    fr.readAsDataURL(file)
  } catch (err) { if (cb) cb(false) }
}
// 颜色解析/合成:支持 #hex 与 rgb(r,g,b)
function cssToRgb(css) {
  var s = String(css || '').trim()
  var m = /^#([0-9a-fA-F]{6})$/.exec(s)
  if (m) { var n = parseInt(m[1], 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255] }
  m = /^rgb(s*(d+)s*,s*(d+)s*,s*(d+)s*)$/i.exec(s)
  if (m) return [Number(m[1]), Number(m[2]), Number(m[3])]
  return [32, 49, 112]
}
function rgbToCss(r, g, b) { return 'rgb(' + Math.max(0, Math.min(255, Math.round(r))) + ',' + Math.max(0, Math.min(255, Math.round(g))) + ',' + Math.max(0, Math.min(255, Math.round(b))) + ')' }
// 原生取色版(覆盖上面的自绘版):原生 color 输入更直观,默认色按钮放其旁边
function bubbleColorEdit(container, getCss, setCss, label) {
  var row = document.createElement('div')
  row.className = 'dshwv-audiorow'
  var lb = document.createElement('span')
  lb.textContent = label || '颜色'
  row.appendChild(lb)
  var inp = document.createElement('input')
  inp.type = 'color'
  inp.className = 'dshwv-colnat'
  row.appendChild(inp)
  var def = document.createElement('button')
  def.type = 'button'
  def.className = 'dshwv-snapbtn dshwv-snapbtn-no'
  def.textContent = '默认'
  def.title = '恢复默认颜色'
  def.addEventListener('click', function () { setCss(''); sync() })
  row.appendChild(def)
  container.appendChild(row)
  function sync() {
    try {
      var v = getCss()
      if (/^#[0-9a-fA-F]{6}$/.test(v || '')) { inp.value = v; return }
      if (v) {
        var a = cssToRgb(v)
        inp.value = '#' + ((1 << 24) + (a[0] << 16) + (a[1] << 8) + a[2]).toString(16).slice(1)
        return
      }
      inp.value = '#203170'
    } catch (err) {}
  }
  inp.addEventListener('input', function () { setCss(inp.value) })
  inp.addEventListener('change', function () { setCss(inp.value) })
  sync()
}
// 跑马灯方案选择:'' 无 | 'macaron' 马卡龙 | 'candy' 糖果(true 旧数据视为马卡龙)
// 自绘下拉:选项多(14 种)时原生 select 弹出列表过长,改为按钮+限高列表(可滚轮)
var bubbleRgbOpenMenu = null // 当前展开的 .dshwv-rgbmenu
var bubbleFontOpenMenu = null // 当前展开的字体下拉(复用 rgbmenu 类,额外限高)
var bubbleColorOpenMenu = null // 当前展开的颜色下拉(纯色/跑马灯,限高)
// —— 弹层/菜单层级统一助手(所有下拉与弹窗据此取高于当前可见层,避免互相压制) ——
// ============================================================================
// 层级（z-index）分层表 —— **改层之前先读这里**
// 所有浮层都直接挂在 <body> 上，谁盖谁完全由 z-index 决定。历史上出过
// 「子窗口被父窗口盖住点不到」「窗口关不掉」这类问题，所以这里把层段定死，并约定三条铁律：
//   ① 新增浮层**先在下面的表里选一个层段**，不要随手写数字；
//   ② 凡是「可能从别的窗口里被打开」的窗口，必须用 dshwLayerUp(el, 本段起点)，
//      这样父窗口在 29000 时子窗口自动落到 29010，永远不会被盖住；
//   ③ visibleTopZ() 的候选表必须包含**全部**浮层节点（漏一个就会低估"当前最高层"），
//      唯一例外是 toast —— 它是刻意的最顶层，不参与追赶。
//
//   层段              用途                          载体
//   ---------------------------------------------------------------------------
//   1 – 999          挂件本体内部元素               .dshwv-pop(1) / .dshwv-menu-btn(2) / 拖拽把手(3-5)
//   30 / 60          面板内的自绘下拉               .dshwv-colpop(30) / .dshwv-rgbmenu(60)
//   9999             挂件本体                       .dshwv-root
//   10000 – 19999    主菜单与其列表                 .dshwv-menu(10000) / .dshwv-rolelist·audiolist(10001)
//   20000 – 20999    一级编辑器                     .dshwv-cropmask·gifmask(20000) / .dshwv-resmask(20300) /
//                                                   .dshwv-audiomask·bubmask(20500) / .dshwv-slotlist(20600)
//   21000 – 21999    对话框基础层                   .dshwv-confirmmask(21000，实际被 showConfirm 提到 40000)
//   22000 – 22999    记账 / 吸附窗口                .dshwv-snapmask·usage-mask(22000)
//   26000 – 26999    小浮层                         .dshwv-qedit(26000) / .dshwv-usagepanel(26020) /
//                                                   .dshwv-tplhelp·动态提示(26080+)
//   29000 – 29999    模型子菜单 / 模型设置           JS 显式写入（refreshModelList / openApiModelMenu）
//   30000 – 30999    提醒·额度·余额校正编辑器        JS 显式写入（含 openModelQuotaEditor / openBalanceAdjustment）
//   31000 – 31999    提醒编辑期间需提到顶层的浮层    JS 显式写入（moduleMask / qedit / token 提示）
//   32000 – 32999    提醒编辑期间统一置顶的遮罩      remindZStyle 的 !important 规则
//   40000            确认对话框（永远在所有窗口之上） showConfirm() 的 !important
//   2147483600       提示条 toast（最高，且不参与 visibleTopZ）
// ============================================================================
function visibleTopZ() {
  var top = 20500
  // 注意：这里必须列全 —— 少一个浮层，dshwLayerUp/下拉/提示就会低估"当前最高层"而被盖住。
  // toast 刻意不列入（它是永远的最顶层，不该让别的层去追它）。
  // v750（issue #131 的教训）：每个候选写成**独立取值函数**并各自 try/catch。
  // 0.3.8 曾在这里写了一个并不存在的 `usageMask` —— 数组字面量在**构造时**就抛 ReferenceError，
  // 于是 visibleTopZ() 每次调用都失败、又被调用方的空 catch 吞掉：自绘下拉的层级停在下拉样式表里的
  // 60（被父窗口整层盖住，表现为"点了没反应"），dshwLayerUp() 也一起静默失效。
  // 现在单个名字写错最多只少算那一层，不会让整条层级链失效；`_z-audit-check.mjs` 会核对
  // 候选表里的每个标识符都真的声明过（防止同样的错再犯）。
  var getters = [
    function () { return bubbleMask }, function () { return bubbleItemMask },
    function () { return moduleMask }, function () { return moduleNamePromptMask },
    function () { return cropMask }, function () { return gifMask },
    function () { return audioCropMask }, function () { return audioEditMask },
    function () { return resMaskEl }, function () { return confirmMask },
    function () { return snapMask }, function () { return usageMoreMask },
    function () { return qeditEl }, function () { return dshwvTplHelpEl },
    function () { return dshwvHintEl }, function () { return apiModelMaskEl },
    function () { return accountingMask }, function () { return window.__dshwRemindMask }
  ]
  function eff(el) {
    try {
      if (!el) return 0
      if (el.style && el.style.display === 'none') return 0
      var s = el.style ? (el.style.zIndex || '') : ''
      if (!s) {
        var cs = window.getComputedStyle ? window.getComputedStyle(el) : null
        if (cs) s = cs.zIndex
      }
      var n = parseFloat(s)
      return isFinite(n) ? n : 0
    } catch (err) { return 0 }
  }
  for (var i = 0; i < getters.length; i++) {
    var n = 0
    try { n = eff(getters[i]()) } catch (err) { n = 0 }
    if (n > top) top = n
  }
  // v756（issue #142）：给第三方 fork / 以后的扩展模块留一个**运行时登记口** ——
  // 它们新增的浮层不必再回上游源码里插一行，只要 `window.dshwRegisterMask(el)` 登记一次，
  // 这里就会把它算进"当前可见最高层"（隐藏的会被 eff() 判成 0）。返回一个注销函数。
  try {
    var extra = dshwExtraMasks || []
    for (var k = 0; k < extra.length; k++) {
      var n2 = 0
      try { n2 = eff(extra[k]) } catch (err) { n2 = 0 }
      if (n2 > top) top = n2
    }
  } catch (err) {}
  return top
}
// 运行时登记的额外浮层（issue #142）。只存引用，不持有任何别的东西；登记失败的入口一律静默忽略。
var dshwExtraMasks = []
try {
  window.dshwRegisterMask = function (el) {
    try {
      if (!el || dshwExtraMasks.indexOf(el) >= 0) return function () {}
      dshwExtraMasks.push(el)
      return function () {
        try {
          var i = dshwExtraMasks.indexOf(el)
          if (i >= 0) dshwExtraMasks.splice(i, 1)
        } catch (err) {}
      }
    } catch (err) { return function () {} }
  }
} catch (err) {}
// 「永远在打开它的那个窗口之上」：取 本层段起点 与 当前可见最高层+10 的较大值。
// 用在裁剪 / GIF / 音频裁剪 / 模块编辑器这些**既可能从主菜单(10000)打开、也可能从资源管理(20300)、
// 泡泡编辑器(20500)、模型设置(29000)里打开**的窗口上 —— 固定层号在后者场景会被父窗口盖住。
function dshwLayerUp(el, base) {
  var z = base
  try { z = Math.max(base, Math.round(visibleTopZ()) + 10) } catch (err) { z = base }
  try { if (el && el.style) el.style.zIndex = String(z) } catch (err) {}
  return el
}
// 打开主要编辑器前清理可能残留的临时层级(提醒会话遗留的 moduleMask/qedit 提升与样式)
function whaleZClean() {
  try {
    var zsEl = document.getElementById('dshw-remind-overlay-z')
    if (zsEl) { try { document.head.removeChild(zsEl) } catch (err) {} }
    if (!window.__dshwRemindMask) {
      if (moduleMask && moduleMask.style.zIndex) moduleMask.style.zIndex = ''
      if (qeditEl && qeditEl.style.zIndex) qeditEl.style.zIndex = ''
    }
  } catch (err) {}
}
// 把 dropdown 移植到 document.body 并以 fixed 定位到触发按钮下方,避免被滚动容器裁剪;
// 菜单宽度与触发按钮保持一致(原样式 min-width:100% 在 body 下会按视口撑满,须归零)
function dshwDropOpen(menuEl, anchorEl) {
  try {
    if (menuEl.parentNode !== document.body) dshwBodyAppend(menuEl)
    menuEl.style.position = 'fixed'
    menuEl.style.minWidth = '0px'
    menuEl.style.left = '0px'
    menuEl.style.top = '0px'
    menuEl.classList.add('dshwv-rgbopen')
    var r = anchorEl.getBoundingClientRect()
    var vp = viewport()
    // 选择框与下拉菜单等宽:直接取触发按钮宽度,不做 150 保底放大,
    // 也不受 CSS 固定宽度/最大宽度(字体 220/配色 160/颜色 190 等)影响
    var w = Math.max(20, Math.round(r.width))
    // 颜色/底色下拉:把同一行内色板(取色器+默认钮)占的宽度补进弹层——
    // 纯色模式下弹层宽度与选跑马灯(无色板)时下拉占宽一致
    if (menuEl.classList && menuEl.classList.contains('dshwv-qcolmenu')) {
      try {
        var rowHost = anchorEl && anchorEl.parentNode ? anchorEl.parentNode.parentNode : null
        if (rowHost && rowHost.querySelector) {
          var swEl = rowHost.querySelector('.dshwv-qcolorhost')
          if (swEl && swEl.offsetWidth > 0) w = Math.max(w, Math.round(r.width + swEl.offsetWidth))
        }
      } catch (err) {}
    }
    if (w > vp.w - 16) w = Math.max(20, vp.w - 16)
    menuEl.style.width = w + 'px'
    menuEl.style.maxWidth = 'none'
    var left = r.left
    if (left + w > vp.w - 8) left = Math.max(8, vp.w - w - 8)
    menuEl.style.left = Math.round(left) + 'px'
    menuEl.style.top = Math.round(r.bottom + 2) + 'px'
    // 下拉层级:高于当前所有可见弹窗/窗口(兜底不低于 26010)
    // v750（issue #131）：visibleTopZ() 单独 try/catch —— 算不出最高层也必须把层级抬上去，
    // 并且打一条 warn（原先异常被外层空 catch 吞掉，表现为"下拉点了没反应"且毫无线索）
    var vTop = 20500
    try { vTop = visibleTopZ() } catch (err) { try { console.warn('[dsh-whale] visibleTopZ 失败，下拉改用兜底层级：', err) } catch (e2) {} }
    menuEl.style.zIndex = String(Math.max(26010, Math.round(vTop) + 10))
  } catch (err) {}
}
// 尽力读取系统字体列表(依赖浏览器 Font Access 能力与用户授权;不可用时仅内置清单)
var bubbleSysFontList = []
var bubbleSysFontTried = false
function refreshSystemFonts(onDone) {
  if (bubbleSysFontTried) { if (onDone) onDone(); return }
  bubbleSysFontTried = true
  if (!window.queryLocalFonts) { if (onDone) onDone(); return }
  try {
    window.queryLocalFonts().then(function (list) {
      try {
        var seen = {}
        var out = []
        if (list && list.length) {
          for (var i = 0; i < list.length; i++) {
            var fam = String((list[i] && list[i].family) || '')
            var lab = String((list[i] && (list[i].fullName || list[i].family)) || fam)
            if (!fam) continue
            if (fam.indexOf('"') >= 0 || fam.indexOf(',') >= 0) continue
            var key = fam.toLowerCase()
            if (seen[key]) continue
            seen[key] = true
            out.push({ v: '"' + fam + '"', l: lab })
          }
          out.sort(function (a, b) { return a.l < b.l ? -1 : a.l > b.l ? 1 : 0 })
        }
        bubbleSysFontList = out
      } catch (err) {}
      if (onDone) onDone()
    }).catch(function () { if (onDone) onDone() })
  } catch (err) { if (onDone) onDone() }
}
function bubbleRgbSelect(current, cb) {
  var wrap = document.createElement('div')
  wrap.className = 'dshwv-rgbwrap'
  var cur = current === true ? 'macaron' : (current || '')
  var opts = [
    ['', '无'],
    ['macaron', '马卡龙'],
    ['candy', '糖果'],
    ['rouge', '酒红'],
    ['bamboo', '翠青'],
    ['aurora', '极光幻彩'],
    ['deepsea', '深海蓝调'],
    ['sunset', '落日熔金'],
    ['forest', '森林秘语'],
    ['champagne', '香槟鎏金'],
    ['lavender', '薰衣草梦境'],
    ['mint', '薄荷汽水'],
    ['lava', '岩浆熔岩'],
    ['galaxy', '银河星紫'],
    ['ink', '墨韵黑白'],
    ['indigo', '靛蓝夜曲'],
    ['blaze', '火红烈焰'],
    ['amber', '警示橙黄'],
  ]
  function labelOf(v) {
    for (var i = 0; i < opts.length; i++) if (opts[i][0] === v) return opts[i][1]
    return '无'
  }
  var head = document.createElement('button')
  head.type = 'button'
  head.className = 'dshwv-rgbhead'
  head.textContent = labelOf(cur)
  head.title = '选择跑马灯方案'
  wrap.appendChild(head)
  var menu = document.createElement('div')
  menu.className = 'dshwv-rgbmenu'
  function fillMenu() {
    menu.innerHTML = ''
    for (var i = 0; i < opts.length; i++) {
      (function (v, lab) {
        var o = document.createElement('div')
        o.className = 'dshwv-rgbopt' + (v === cur ? ' dshwv-rgbcur' : '')
        o.textContent = lab
        o.addEventListener('click', function (e) {
          e.stopPropagation()
          cur = v
          head.textContent = labelOf(cur)
          closeRgbMenu()
          cb(v)
        })
        menu.appendChild(o)
      })(opts[i][0], opts[i][1])
    }
  }
  fillMenu()
  wrap.appendChild(menu)
  head.addEventListener('click', function (e) {
    e.stopPropagation()
    if (bubbleRgbOpenMenu === menu) { closeRgbMenu(); return }
    closeRgbMenu()
    menu.classList.add('dshwv-rgbopen')
    bubbleRgbOpenMenu = menu
  })
  function closeRgbMenu() {
    if (bubbleRgbOpenMenu) bubbleRgbOpenMenu.classList.remove('dshwv-rgbopen')
    bubbleRgbOpenMenu = null
  }
  // 外点关闭(仅绑定一次)
  if (!window.__dshwRgbDocBound) {
    window.__dshwRgbDocBound = true
    document.addEventListener('pointerdown', function (e) {
      if (!bubbleRgbOpenMenu) return
      try {
        if (e.target && e.target.closest && e.target.closest('.dshwv-rgbwrap')) return
      } catch (err) {}
      closeRgbMenu()
    }, true)
  }
  return wrap
}
function bubbleFontEditRow(getVal, setVal) {
  // 字体选择:系统字体下拉(仅可选项,去掉自定义字体名),列表限高滚动
  var FONT_OPTIONS = [
    ['', '默认字体'],
    ['"Microsoft YaHei",sans-serif', '微软雅黑'],
    ['"PingFang SC","Microsoft YaHei",sans-serif', '苹方/雅黑'],
    ['DengXian,"Microsoft YaHei",sans-serif', '等线'],
    ['SimSun,serif', '宋体'],
    ['SimHei,sans-serif', '黑体'],
    ['KaiTi,serif', '楷体'],
    ['FangSong,serif', '仿宋'],
    ['STKaiti,KaiTi,serif', '华文楷体'],
    ['"Noto Sans SC",sans-serif', 'Noto Sans SC'],
    ['"Source Han Sans SC",sans-serif', '思源黑体'],
    ['"Segoe UI",sans-serif', 'Segoe UI'],
    ['Arial,Helvetica,sans-serif', 'Arial'],
    ['Helvetica,Arial,sans-serif', 'Helvetica'],
    ['Verdana,sans-serif', 'Verdana'],
    ['Tahoma,sans-serif', 'Tahoma'],
    ['"Trebuchet MS",sans-serif', 'Trebuchet MS'],
    ['"Times New Roman",serif', 'Times New Roman'],
    ['Georgia,serif', 'Georgia'],
    ['"Courier New",monospace', 'Courier New'],
    ['Consolas,monospace', 'Consolas'],
    ['Impact,fantasy', 'Impact'],
    ['"Comic Sans MS",cursive', 'Comic Sans MS'],
  ]
  var row = document.createElement('div')
  row.className = 'dshwv-audiorow'
  var fl = document.createElement('span')
  fl.textContent = '字体'
  row.appendChild(fl)
  var box = document.createElement('div')
  box.className = 'dshwv-rgbwrap dshwv-fontwrap'
  var head = document.createElement('button')
  head.type = 'button'
  head.className = 'dshwv-rgbhead'
  head.title = '选择系统字体'
  box.appendChild(head)
  var menu = document.createElement('div')
  menu.className = 'dshwv-rgbmenu dshwv-fontmenu'
  function currentVal() { return getVal ? String(getVal() || '') : '' }
  function labelOf(v) {
    for (var i = 0; i < FONT_OPTIONS.length; i++) if (FONT_OPTIONS[i][0] === v) return FONT_OPTIONS[i][1]
    if (v) return String(v).slice(0, 14)
    return '默认字体'
  }
  function syncHead() {
    var v = currentVal()
    head.textContent = labelOf(v)
    head.style.fontFamily = v || ''
    head.title = '当前: ' + (v || '默认字体') + ';点击选择系统字体'
  }
  function optionList() {
    var list = []
    var seen = {}
    for (var i = 0; i < FONT_OPTIONS.length; i++) { list.push(FONT_OPTIONS[i]); seen[FONT_OPTIONS[i][0]] = true }
    for (var s = 0; s < bubbleSysFontList.length; s++) {
      if (!seen[bubbleSysFontList[s].v]) { seen[bubbleSysFontList[s].v] = true; list.push([bubbleSysFontList[s].v, bubbleSysFontList[s].l]) }
    }
    return list
  }
  function fill() {
    menu.innerHTML = ''
    var cur = currentVal()
    var all = optionList()
    for (var i = 0; i < all.length; i++) {
      (function (fv, flab) {
        var o = document.createElement('div')
        o.className = 'dshwv-rgbopt' + (fv === cur ? ' dshwv-rgbcur' : '')
        o.textContent = flab
        o.style.fontFamily = fv || ''
        o.addEventListener('click', function () {
          if (setVal) setVal(fv)
          closeFontMenu()
          syncHead()
          fill()
        })
        menu.appendChild(o)
      })(all[i][0], all[i][1])
    }
  }
  box.appendChild(menu)
  function closeFontMenu() {
    menu.classList.remove('dshwv-rgbopen')
    bubbleFontOpenMenu = null
  }
  head.addEventListener('click', function (e) {
    e.stopPropagation()
    if (bubbleFontOpenMenu === menu) { closeFontMenu(); return }
    if (bubbleFontOpenMenu) bubbleFontOpenMenu.classList.remove('dshwv-rgbopen')
    fill()
    bubbleFontOpenMenu = menu
    dshwDropOpen(menu, head)
    // 尽力补入系统字体(授权成功后若菜单仍开着则即时刷新)
    refreshSystemFonts(function () {
      if (bubbleFontOpenMenu === menu && menu.classList.contains('dshwv-rgbopen')) {
        fill()
        dshwDropOpen(menu, head)
      }
    })
  })
  if (!window.__dshwFontDocBound) {
    window.__dshwFontDocBound = true
    document.addEventListener('pointerdown', function (e) {
      if (!bubbleFontOpenMenu) return
      try { if (e.target && e.target.closest && (e.target.closest('.dshwv-fontwrap') || e.target.closest('.dshwv-rgbmenu'))) return } catch (err) {}
      bubbleFontOpenMenu.classList.remove('dshwv-rgbopen')
      bubbleFontOpenMenu = null
    }, true)
  }
  row.appendChild(box)
  syncHead()
  fill()
  return row
}
function moduleTypeName(t, m) {
  if (t === 'balance') return '总余额(充值金额+赠金)'
  // v782：赠金 / 充值余额两个数值模块的类型名（v787 改显示文案）
  if (t === 'bonus') return '赠金'
  if (t === 'recharge') return '充值余额'
  if (t === 'today') return '今日已用'
  if (t === 'session') return '对话名'
  if (t === 'peak' || t === 'nextpeak') {
    // 峰谷模块按显示样式给名(倒计时/简洁等);无模块对象时退回通用名
    if (m) return bubblePeakModuleLabel(m)
    return t === 'nextpeak' ? '时段倒计时' : '峰谷时段'
  }
  if (t === 'image') return '图片/动图'
  if (t === 'randimg') return '随机图片'
  if (t === 'random') return '随机语句'
  return '文本'
}
function renderModuleEditor() {
  var m = moduleEditRef
  // 旧版独立「时段倒计时」模块 → 峰谷模块的倒计时样式(编辑即归一)
  if (m && m.type === 'nextpeak') {
    m.type = 'peak'
    if (!m.peakStyle) m.peakStyle = 'count'
  }
  // 新建模块默认值:文本模块默认加粗(与随机语句一致)
  if (moduleEditNew && m.type === 'text' && m.bold === undefined) m.bold = true
  moduleColorEl = null
  moduleSizeEl = null
  moduleBodyEl.innerHTML = ''
  moduleTitleEl.textContent = (moduleEditNew ? '新增模块: ' : '编辑模块: ') + moduleTypeName(m.type, m)
  // 内容模板行:留空=默认自动文案;占位符按模块给英文(可扩展其他 API 字段)
  function moduleTplRow() {
    var row = document.createElement('div')
    row.className = 'dshwv-audiorow'
    var lab = document.createElement('span')
    lab.textContent = '内容'
    lab.style.flex = '0 0 auto'
    row.appendChild(lab)
    var inp = document.createElement('input')
    inp.type = 'text'
    inp.style.flex = '1'
    inp.style.minWidth = '0'
    inp.style.boxSizing = 'border-box'
    inp.style.border = '1px solid rgba(32,49,112,.4)'
    inp.style.borderRadius = '6px'
    inp.style.padding = '3px 6px'
    inp.style.fontSize = '12px'
    inp.style.color = '#203170'
    inp.style.background = '#fff'
    inp.value = m.tpl || ''
    function hintOf() {
      if (m.type === 'balance') return '例: {balance_ds}'
      if (m.type === 'bonus') return '例: {bonus_ds} 或 赠金 {bonus_ds}'
      if (m.type === 'recharge') return '例: {recharge_ds} 或 余额 {recharge_ds}'
      if (m.type === 'today') return '例: 今日已用 {expense_ds}'
      if (m.type === 'session') return '例: {session} 或 当前对话 {session}'
      if (bubbleIsPeakCount(m)) return '例: 距空闲 {countdown}'
      return '例: 当前 {status}'
    }
    var hp = hintOf()
    inp.placeholder = hp
    inp.title = '输入内容;右侧 ? 查看可用占位符'
    inp.addEventListener('input', function () { m.tpl = inp.value })
    row.appendChild(inp)
    var qb = document.createElement('button')
    qb.type = 'button'
    qb.className = 'dshwv-tplq'
    qb.textContent = '?'
    qb.title = '可用占位符用法'
    qb.addEventListener('click', function (e) { e.stopPropagation(); bubbleTplHelpToggle(m, qb) })
    row.appendChild(qb)
    moduleBodyEl.appendChild(row)
  }
  // 新建模块:类型选择
  if (moduleEditNew) {
    var tr = document.createElement('div')
    tr.className = 'dshwv-audiorow'
    var tl = document.createElement('span')
    tl.textContent = '类型'
    tr.appendChild(tl)
    var tsel = document.createElement('select')
    tsel.className = 'dshwv-sound'
    var topts = [
      ['text', '文本'],
      ['random', '随机语句'],
      ['image', '图片/动图'],
      ['randimg', '随机图片'],
    ]
    for (var ti2 = 0; ti2 < topts.length; ti2++) {
      var o2 = document.createElement('option')
      o2.value = topts[ti2][0]
      o2.textContent = topts[ti2][1]
      tsel.appendChild(o2)
    }
    tsel.value = m.type
    tsel.addEventListener('change', function () {
      m.type = tsel.value
      if (m.type === 'random' && !Array.isArray(m.lines)) m.lines = []
      if (m.type === 'random' && m.bold === undefined) m.bold = true // 随机语句默认加粗
      if (m.type === 'text' && m.bold === undefined) m.bold = true // 文本模块默认加粗
      if (m.type === 'image' && !m.imgId) m.imgId = ''
      if (m.type === 'randimg') {
        if (!Array.isArray(m.imgs)) m.imgs = []
        if (m.imgScale === undefined) m.imgScale = 1
      }
      renderModuleEditor()
    })
    tr.appendChild(tsel)
    dshwCustSel(tsel)
    moduleBodyEl.appendChild(tr)
  }
  // 从模块库载入
  if (bubbleLib.length) {
    var lr2 = document.createElement('div')
    lr2.className = 'dshwv-bubsec'
    lr2.textContent = '从模块库载入'
    moduleBodyEl.appendChild(lr2)
    for (var li3 = 0; li3 < bubbleLib.length; li3++) {
      (function (lb) {
        var lrow = document.createElement('div')
        lrow.className = 'dshwv-bublibrow'
        var lbtn = document.createElement('button')
        lbtn.type = 'button'
        lbtn.className = 'dshwv-bubnewbtn'
        lbtn.textContent = lb.name
        lbtn.title = '将该模块配置载入当前编辑'
        lbtn.addEventListener('click', function () {
          var c = bubbleCloneModule(lb.module)
          var oldKeys = Object.keys(m)
          for (var kk = 0; kk < oldKeys.length; kk++) { try { delete m[oldKeys[kk]] } catch (err) {} }
          var nk = Object.keys(c)
          for (var j2 = 0; j2 < nk.length; j2++) m[nk[j2]] = c[nk[j2]]
          moduleColorEl = null
          moduleSizeEl = null
          renderModuleEditor()
        })
        lrow.appendChild(lbtn)
        var ldel = document.createElement('button')
        ldel.type = 'button'
        ldel.className = 'dshwv-bubmini'
        ldel.textContent = '✕'
        ldel.title = '从模块库删除'
        ldel.addEventListener('click', function () {
          showConfirm('从模块库删除「' + lb.name + '」?', function () {
            bubbleLibDel(lb.id)
            renderModuleEditor()
            if (bubblePalEl) renderBubblePal()
          })
        })
        lrow.appendChild(ldel)
        moduleBodyEl.appendChild(lrow)
      })(bubbleLib[li3])
    }
  }
  // 内容区
  if (m.type === 'text') {
    var ti = document.createElement('input')
    ti.type = 'text'
    ti.className = 'dshwv-cropname'
    ti.maxLength = 60
    ti.value = m.text || ''
    ti.placeholder = '文本内容'
    ti.addEventListener('input', function () { m.text = ti.value || ' ' })
    moduleBodyEl.appendChild(ti)
  } else if (m.type === 'random') {
    // 表头:权重 | 内容 | 操作
    var hint = document.createElement('div')
    hint.className = 'dshwv-linehead'
    var hw = document.createElement('span')
    hw.className = 'dshwv-lhw'
    hw.textContent = '权重'
    hint.appendChild(hw)
    var hc = document.createElement('span')
    hc.className = 'dshwv-lhc'
    hc.textContent = '内容'
    hint.appendChild(hc)
    var ho = document.createElement('span')
    ho.className = 'dshwv-lho'
    ho.textContent = '操作'
    hint.appendChild(ho)
    moduleBodyEl.appendChild(hint)
    if (!Array.isArray(m.lines)) m.lines = []
    var listEl = document.createElement('div')
    // 列表容器限高,语句多时在列表内滚动,不挡住下方按钮
    listEl.className = 'dshwv-listbox'
    listEl.style.maxHeight = '240px'
    listEl.style.overflowY = 'auto'
    listEl.style.paddingRight = '2px'
    moduleBodyEl.appendChild(listEl)
    function linePanel(l, box) {
      box.innerHTML = ''
      // 行未显式设置时继承模块默认(随机语句模块级样式区已隐藏,行编辑是唯一入口)
      function lineVal(lv, mv, dft) { return (lv !== undefined && lv !== null) ? lv : ((mv !== undefined && mv !== null) ? mv : dft) }
      // 第一行:字号(字体大小)
      var r2 = document.createElement('div')
      r2.className = 'dshwv-audiorow'
      var c2 = document.createElement('span')
      c2.textContent = '字号'
      r2.appendChild(c2)
      var ps = document.createElement('input')
      ps.type = 'range'
      ps.min = '1'
      ps.max = '50'
      ps.step = '1'
      ps.className = 'dshwv-cropzoom'
      ps.value = String(lineVal(l.size, m.size, 3))
      r2.appendChild(ps)
      var psNum = document.createElement('span')
      psNum.className = 'dshwv-volpct'
      psNum.textContent = String(lineVal(l.size, m.size, 3))
      ps.addEventListener('input', function () { l.size = Math.round(Number(ps.value) || 3); psNum.textContent = ps.value })
      r2.appendChild(psNum)
      box.appendChild(r2)
      // 自定义字体选择(写入本句 fontFamily)
      box.appendChild(bubbleFontEditRow(function () { return lineVal(l.fontFamily, m.fontFamily, '') }, function (v) { l.fontFamily = v || '' }))
      // 颜色(有跑马灯时不显示颜色行)
      if (!l.rgb) bubbleColorEdit(box, function () { return lineVal(l.color, m.color, '') }, function (v) { l.color = v }, '颜色')
      // 底色(仅该句;默认无;纯色/跑马灯)
      var lbgV = (l.bgRgb) ? l.bgRgb : (l.bg ? 'solid' : (m.bgRgb ? m.bgRgb : (m.bg ? 'solid' : 'none')))
      var lbgHex0 = l.bg || m.bg || '#dbe4f5'
      var lbgc = qColorSelectBuild(lbgV, function (v) {
        if (v === 'none') { l.bgRgb = ''; l.bg = '' }
        else if (v === 'solid') { l.bgRgb = ''; if (!l.bg) l.bg = lbgHex0 }
        else { l.bgRgb = v; l.bg = '' }
        lbgc.sync(v === 'none' ? 'none' : v, l.bg || lbgHex0, function (h) { l.bg = h })
      }, { label: '底色', defaultHex: lbgHex0, defaultText: '默认', allowNone: true })
      box.appendChild(lbgc.row)
      lbgc.sync(lbgV, lbgHex0, function (h) { l.bg = h })
      // 字形:加粗/斜体/下划线 + 跑马灯
      var r3 = document.createElement('div')
      r3.className = 'dshwv-audiorow'
      function lb2(label, key) {
        var la = document.createElement('label')
        la.style.display = 'inline-flex'
        la.style.alignItems = 'center'
        la.style.gap = '3px'
        la.style.marginRight = '10px'
        var cb = document.createElement('input')
        cb.type = 'checkbox'
        // 勾选状态以实际生效样式为准(行未显式设置时继承模块)
        cb.checked = !!lineVal(l[key], m[key], false)
        cb.addEventListener('change', function () { l[key] = cb.checked })
        var tx = document.createElement('span')
        tx.textContent = label
        la.appendChild(cb)
        la.appendChild(tx)
        return la
      }
      r3.appendChild(lb2('加粗', 'bold'))
      r3.appendChild(lb2('斜体', 'italic'))
      r3.appendChild(lb2('下划线', 'ul'))
      var r3l = document.createElement('span')
      r3l.textContent = '跑马灯'
      r3.appendChild(r3l)
      r3.appendChild(bubbleRgbSelect(l.rgb, function (v) { l.rgb = v; linePanel(l, box) }))
      box.appendChild(r3)
    }
    function renderLines() {
      listEl.innerHTML = ''
      // 还没有语句时不显示「权重 内容 操作」表头
      hint.style.display = m.lines.length ? 'flex' : 'none'
      for (var i = 0; i < m.lines.length; i++) {
        (function (idx) {
          var l = m.lines[idx]
          if (!l) return
          var wrap = document.createElement('div')
          wrap.className = 'dshwv-linerow'
          var lr = document.createElement('div')
          lr.className = 'dshwv-audiorow'
          var wt = document.createElement('input')
          wt.type = 'number'
          wt.min = '1'
          wt.max = '99'
          wt.className = 'dshwv-linew'
          wt.value = String(l.w || 1)
          wt.title = '权重'
          wt.addEventListener('input', function () { l.w = Math.max(1, Math.round(Number(wt.value) || 1)) })
          lr.appendChild(wt)
          var tx = document.createElement('input')
          tx.type = 'text'
          tx.className = 'dshwv-linetx'
          tx.value = l.t
          tx.placeholder = '句子'
          tx.addEventListener('input', function () { l.t = tx.value || ' ' })
          lr.appendChild(tx)
          var ed = document.createElement('button')
          ed.type = 'button'
          ed.className = 'dshwv-bubmini'
          ed.textContent = '✎'
          ed.title = '该句悬浮样式编辑(句子/字号/字体/颜色/字形)'
          ed.addEventListener('click', function (e) { e.stopPropagation(); openQuickSentenceEditor(l, m, tx, ed) })
          lr.appendChild(ed)
          var cp = document.createElement('button')
          cp.type = 'button'
          cp.className = 'dshwv-bubmini'
          cp.textContent = '⧉'
          cp.title = '复制该行(含样式)'
          cp.addEventListener('click', function () {
            m.lines.splice(idx + 1, 0, JSON.parse(JSON.stringify(l)))
            renderLines()
          })
          lr.appendChild(cp)
          var del = document.createElement('button')
          del.type = 'button'
          del.className = 'dshwv-linedel'
          del.textContent = '✕'
          del.title = '删除该句'
          del.addEventListener('click', function () { m.lines.splice(idx, 1); renderLines() })
          lr.appendChild(del)
          wrap.appendChild(lr)
          listEl.appendChild(wrap)
        })(i)
      }
    }
    renderLines()
    var addL = document.createElement('button')
    addL.type = 'button'
    addL.className = 'dshwv-addline'
    addL.textContent = '+ 添加语句'
    addL.addEventListener('click', function () { m.lines.push({ t: '新句子', w: 1 }); renderLines() })
    moduleBodyEl.appendChild(addL)
  } else if (m.type === 'image') {
    moduleImgSelect = document.createElement('select')
    moduleImgSelect.className = 'dshwv-sound'
    moduleBodyEl.appendChild(moduleImgSelect)
    moduleImgDrop = dshwCustSel(moduleImgSelect)
    moduleImgPreviewEl = document.createElement('img')
    moduleImgPreviewEl.className = 'dshwv-bubimgprev'
    moduleImgPreviewEl.alt = ''
    moduleBodyEl.appendChild(moduleImgPreviewEl)
    function fillImgSel() {
      moduleImgSelect.innerHTML = ''
      var opt0 = document.createElement('option')
      opt0.value = ''
      opt0.textContent = '— 选择泡泡图库图片 —'
      moduleImgSelect.appendChild(opt0)
      for (var i = 0; i < bubbleImgList.length; i++) {
        var o = document.createElement('option')
        o.value = bubbleImgList[i].id
        o.textContent = bubbleImgList[i].name
        moduleImgSelect.appendChild(o)
      }
      if (m.imgId) moduleImgSelect.value = m.imgId
      moduleImgSelect.dispatchEvent(new Event('change'))
      if (moduleImgDrop) moduleImgDrop.refresh()
    }
    moduleImgSelect.addEventListener('change', function () {
      m.imgId = moduleImgSelect.value
      if (m.imgId) { moduleImgPreviewEl.src = '/dsh-whale/bubble-img.png?id=' + encodeURIComponent(m.imgId); moduleImgPreviewEl.style.display = 'block' }
      else moduleImgPreviewEl.style.display = 'none'
    })
    var fileInput = document.createElement('input')
    fileInput.type = 'file'
    fileInput.accept = 'image/png,image/gif'
    fileInput.style.display = 'none'
    var upBtn = document.createElement('button')
    upBtn.type = 'button'
    upBtn.className = 'dshwv-snapbtn dshwv-snapbtn-no'
    upBtn.textContent = '上传图片(png/gif)'
    upBtn.addEventListener('click', function () { fileInput.click() })
    fileInput.addEventListener('change', function () {
      var f = fileInput.files && fileInput.files[0]
      if (!f) return
      bubbleUploadImg(f, function (ok) {
        if (ok) fillImgSel()
        fileInput.value = ''
      })
    })
    var upRow = document.createElement('div')
    upRow.className = 'dshwv-uproll'
    upRow.appendChild(upBtn)
    upRow.appendChild(fileInput)
    moduleBodyEl.appendChild(upRow)
    // 显示大小:相对最大宽 540u 的百分比(10%–100%)
    var scRow = document.createElement('div')
    scRow.className = 'dshwv-audiorow'
    var scL = document.createElement('span')
    scL.textContent = '显示大小'
    scRow.appendChild(scL)
    var scInit = Number(m.imgScale)
    if (!isFinite(scInit) || scInit <= 0) scInit = 1
    var scInp = document.createElement('input')
    scInp.type = 'range'
    scInp.min = '10'
    scInp.max = '100'
    scInp.step = '5'
    scInp.className = 'dshwv-cropzoom'
    scInp.style.flex = '1'
    scInp.value = String(Math.round(scInit * 100))
    scInp.addEventListener('input', function () {
      m.imgScale = Math.max(0.1, Math.min(1, Number(scInp.value) / 100))
      scVal.textContent = scInp.value + '%'
      try { if (moduleImgPreviewEl) moduleImgPreviewEl.style.maxWidth = Math.round(120 * m.imgScale) + 'px' } catch (err) {}
    })
    scRow.appendChild(scInp)
    var scVal = document.createElement('span')
    scVal.className = 'dshwv-volpct'
    scVal.textContent = scInp.value + '%'
    scRow.appendChild(scVal)
    moduleBodyEl.appendChild(scRow)
    try { if (moduleImgPreviewEl) moduleImgPreviewEl.style.maxWidth = Math.round(120 * scInit) + 'px' } catch (err) {}
    if (!bubbleImgList.length) loadBubbleImgs(fillImgSel)
    else fillImgSel()
  } else if (m.type === 'randimg') {
    // —— 随机图片:与随机语句同款列表(权重 + 图片),按权重抽 1 张且不连续重复 ——
    var rhint = document.createElement('div')
    rhint.className = 'dshwv-linehead'
    var rhw = document.createElement('span')
    rhw.className = 'dshwv-lhw'
    rhw.textContent = '权重'
    rhint.appendChild(rhw)
    var rhc = document.createElement('span')
    rhc.className = 'dshwv-lhc'
    rhc.textContent = '图片'
    rhint.appendChild(rhc)
    var rho = document.createElement('span')
    rho.className = 'dshwv-lho'
    rho.textContent = '操作'
    // 与行内尾部控件(缩略图26+⧉22+✕18+3个8px间距=82)对齐,表头列宽随之内缩
    rho.style.flex = '0 0 82px'
    rhint.appendChild(rho)
    moduleBodyEl.appendChild(rhint)
    if (!Array.isArray(m.imgs)) m.imgs = []
    var rlistEl = document.createElement('div')
    rlistEl.className = 'dshwv-listbox'
    rlistEl.style.maxHeight = '240px'
    rlistEl.style.overflowY = 'auto'
    rlistEl.style.paddingRight = '2px'
    moduleBodyEl.appendChild(rlistEl)
    function renderImgItems() {
      // 重绘前先收起已展开的自绘下拉,避免下拉浮层留在已删除的行上
      try { dshwCustSelClose() } catch (err) {}
      rlistEl.innerHTML = ''
      // 还没有图片时不显示「权重 图片 操作」表头
      rhint.style.display = m.imgs.length ? 'flex' : 'none'
      for (var ii = 0; ii < m.imgs.length; ii++) {
        (function (idx2) {
          var it2 = m.imgs[idx2] || (m.imgs[idx2] = { imgId: '', w: 1 })
          var wrap2 = document.createElement('div')
          wrap2.className = 'dshwv-linerow'
          var lr2 = document.createElement('div')
          lr2.className = 'dshwv-audiorow'
          var wt2 = document.createElement('input')
          wt2.type = 'number'
          wt2.min = '1'
          wt2.max = '99'
          wt2.className = 'dshwv-linew'
          wt2.value = String(it2.w || 1)
          wt2.title = '权重(越大越容易被抽到)'
          wt2.addEventListener('input', function () { it2.w = Math.max(1, Math.round(Number(wt2.value) || 1)) })
          lr2.appendChild(wt2)
          var sel2 = document.createElement('select')
          sel2.className = 'dshwv-sound'
          sel2.style.flex = '1 1 auto'
          sel2.style.minWidth = '0'
          var oo0 = document.createElement('option')
          oo0.value = ''
          oo0.textContent = '— 选择泡泡图库图片 —'
          sel2.appendChild(oo0)
          for (var bi2 = 0; bi2 < bubbleImgList.length; bi2++) {
            var ob2 = document.createElement('option')
            ob2.value = bubbleImgList[bi2].id
            ob2.textContent = bubbleImgList[bi2].name || bubbleImgList[bi2].id
            sel2.appendChild(ob2)
          }
          if (it2.imgId) sel2.value = it2.imgId
          lr2.appendChild(sel2)
          var drop2 = dshwCustSel(sel2)
          var th2 = document.createElement('img')
          th2.className = 'dshwv-rimthumb'
          th2.alt = ''
          function syncTh2() {
            try {
              // 未选图时保留空占位(带底色/边框),避免该行尾部宽度变化导致表头列错位
              if (it2.imgId) th2.src = '/dsh-whale/bubble-img.png?id=' + encodeURIComponent(it2.imgId)
              else th2.removeAttribute('src')
              th2.style.display = 'block'
            } catch (err) {}
          }
          syncTh2()
          lr2.appendChild(th2)
          sel2.addEventListener('change', function () { it2.imgId = sel2.value; syncTh2() })
          var cp2 = document.createElement('button')
          cp2.type = 'button'
          cp2.className = 'dshwv-bubmini'
          cp2.textContent = '⧉'
          cp2.title = '复制该项'
          cp2.addEventListener('click', function () { m.imgs.splice(idx2 + 1, 0, { imgId: it2.imgId, w: it2.w }); renderImgItems() })
          lr2.appendChild(cp2)
          var del2 = document.createElement('button')
          del2.type = 'button'
          del2.className = 'dshwv-linedel'
          del2.textContent = '✕'
          del2.title = '删除该图片'
          del2.addEventListener('click', function () { m.imgs.splice(idx2, 1); renderImgItems() })
          lr2.appendChild(del2)
          wrap2.appendChild(lr2)
          rlistEl.appendChild(wrap2)
        })(ii)
      }
      if (!m.imgs.length) {
        var emptyImg = document.createElement('div')
        emptyImg.className = 'dshwv-bubhint'
        emptyImg.textContent = '还没有图片:点「添加图片」从图库选,或「上传图片(png/gif)」直接加入'
        rlistEl.appendChild(emptyImg)
      }
    }
    renderImgItems()
    var addImgBtn = document.createElement('button')
    addImgBtn.type = 'button'
    addImgBtn.className = 'dshwv-addline'
    addImgBtn.textContent = '+ 添加图片(从图库选)'
    addImgBtn.addEventListener('click', function () { m.imgs.push({ imgId: '', w: 1 }); renderImgItems() })
    moduleBodyEl.appendChild(addImgBtn)
    var upRow2 = document.createElement('div')
    upRow2.className = 'dshwv-uproll'
    var fileInput2 = document.createElement('input')
    fileInput2.type = 'file'
    fileInput2.accept = 'image/png,image/gif'
    fileInput2.style.display = 'none'
    var upBtn2 = document.createElement('button')
    upBtn2.type = 'button'
    upBtn2.className = 'dshwv-snapbtn dshwv-snapbtn-no'
    upBtn2.textContent = '上传图片(png/gif)'
    upBtn2.title = '上传后自动加入本随机图片列表(同时进入泡泡图库)'
    upBtn2.addEventListener('click', function () { fileInput2.click() })
    fileInput2.addEventListener('change', function () {
      var f2 = fileInput2.files && fileInput2.files[0]
      if (!f2) return
      var before2 = {}
      for (var z2 = 0; z2 < bubbleImgList.length; z2++) before2[bubbleImgList[z2].id] = 1
      bubbleUploadImg(f2, function (ok) {
        fileInput2.value = ''
        if (!ok) return
        var newId2 = ''
        for (var z3 = 0; z3 < bubbleImgList.length; z3++) if (!before2[bubbleImgList[z3].id]) { newId2 = bubbleImgList[z3].id; break }
        if (!newId2 && bubbleImgList.length) newId2 = bubbleImgList[bubbleImgList.length - 1].id
        if (newId2) m.imgs.push({ imgId: newId2, w: 1 })
        renderImgItems()
      })
    })
    upRow2.appendChild(upBtn2)
    upRow2.appendChild(fileInput2)
    moduleBodyEl.appendChild(upRow2)
    // 显示大小(与图片模块一致:imgScale 0.1–1)
    var scRow2 = document.createElement('div')
    scRow2.className = 'dshwv-audiorow'
    var scL2 = document.createElement('span')
    scL2.textContent = '显示大小'
    scRow2.appendChild(scL2)
    var scInit2 = Number(m.imgScale)
    if (!isFinite(scInit2) || scInit2 <= 0) scInit2 = 1
    var scInp2 = document.createElement('input')
    scInp2.type = 'range'
    scInp2.min = '10'
    scInp2.max = '100'
    scInp2.step = '5'
    scInp2.className = 'dshwv-cropzoom'
    scInp2.style.flex = '1'
    scInp2.value = String(Math.round(scInit2 * 100))
    var scVal2 = document.createElement('span')
    scVal2.className = 'dshwv-volpct'
    scVal2.textContent = scInp2.value + '%'
    scInp2.addEventListener('input', function () {
      m.imgScale = Math.max(0.1, Math.min(1, Number(scInp2.value) / 100))
      scVal2.textContent = scInp2.value + '%'
    })
    scRow2.appendChild(scInp2)
    scRow2.appendChild(scVal2)
    moduleBodyEl.appendChild(scRow2)
    if (!bubbleImgList.length) loadBubbleImgs(renderImgItems)
  } else {
    if (m.type === 'peak' || m.type === 'nextpeak') {
      // 峰谷模块:显示样式(默认/梁文峰谷/!?强强?!/倒计时/简洁),不再依赖主菜单全局设置
      if (m.type === 'peak') {
        var stRow = document.createElement('div')
        stRow.className = 'dshwv-audiorow'
        var stL = document.createElement('span')
        stL.textContent = '显示样式'
        stRow.appendChild(stL)
        var stSel = document.createElement('select')
        stSel.className = 'dshwv-sound'
        for (var sti = 0; sti < BUBBLE_PEAK_STYLE_OPTS.length; sti++) {
          var so = document.createElement('option')
          so.value = BUBBLE_PEAK_STYLE_OPTS[sti][0]
          so.textContent = BUBBLE_PEAK_STYLE_OPTS[sti][1]
          stSel.appendChild(so)
        }
        stSel.value = bubblePeakStyleOf(m)
        stSel.addEventListener('change', function () { m.peakStyle = stSel.value || 'default' })
        moduleBodyEl.appendChild(stRow)
        stRow.appendChild(stSel)
        dshwCustSel(stSel)
      }
      // F1:峰谷模块的整句文案模板(留空=默认;状态样式可用 {状态},倒计时可用 {时间})
      moduleTplRow()
      // 峰谷/倒计时:高峰/空闲 各占一行 = 左[状态色下拉(纯色时带色板/默认钮)] 右[底色下拉],
      // 底色标签固定为「底色」,两侧各约占一半宽
      var peakStates = [
        { label: '高峰色', colorKey: 'peakColor', rgbKey: 'peakRgb', defaultHex: '#e0433f', defaultText: '默认红', bgKey: 'peakBg', bgRgbKey: 'peakBgRgb', bgHex: '#fbe7e6' },
        { label: '空闲色', colorKey: 'offColor', rgbKey: 'offRgb', defaultHex: '#2fa24c', defaultText: '默认绿', bgKey: 'offBg', bgRgbKey: 'offBgRgb', bgHex: '#e4f3e7' },
      ]
      if (!m.peakColor) m.peakColor = '#e0433f'
      if (!m.offColor) m.offColor = '#2fa24c'
      for (var psi = 0; psi < peakStates.length; psi++) {
        (function (st) {
          var grp = document.createElement('div')
          grp.className = 'dshwv-peakrow'
          var curPeak = m[st.rgbKey] ? m[st.rgbKey] : 'solid'
          var ccRow = qColorSelectBuild(curPeak, function (v) {
            if (v === 'solid') { m[st.rgbKey] = ''; if (!m[st.colorKey]) m[st.colorKey] = st.defaultHex }
            else { m[st.rgbKey] = v; m[st.colorKey] = '' }
            ccRow.sync(v === 'solid' ? 'solid' : v, m[st.colorKey], function (hex) { m[st.colorKey] = hex })
          }, { label: st.label, defaultHex: st.defaultHex, defaultText: st.defaultText })
          grp.appendChild(ccRow.row)
          var stBgV = m[st.bgRgbKey] ? m[st.bgRgbKey] : (m[st.bgKey] ? 'solid' : 'none')
          var stBgHex0 = m[st.bgKey] || st.bgHex
          var bgRowC = qColorSelectBuild(stBgV, function (v) {
            if (v === 'none') { m[st.bgRgbKey] = ''; m[st.bgKey] = '' }
            else if (v === 'solid') { m[st.bgRgbKey] = ''; if (!m[st.bgKey]) m[st.bgKey] = stBgHex0 }
            else { m[st.bgRgbKey] = v; m[st.bgKey] = '' }
            bgRowC.sync(v === 'none' ? 'none' : v, m[st.bgKey] || stBgHex0, function (hex) { m[st.bgKey] = hex })
          }, { label: '底色', defaultHex: stBgHex0, defaultText: '默认', allowNone: true })
          grp.appendChild(bgRowC.row)
          moduleBodyEl.appendChild(grp)
          ccRow.sync(curPeak, m[st.colorKey] || st.defaultHex, function (hex) { m[st.colorKey] = hex })
          bgRowC.sync(stBgV, stBgHex0, function (hex) { m[st.bgKey] = hex })
        })(peakStates[psi])
      }
    } else {
      var note = document.createElement('div')
      note.className = 'dshwv-bubhint'
      note.textContent = m.type === 'session'
        ? '该模块自动显示**当前对话名**（占位符 {session}），可调下方颜色 / 字号'
        : '该模块为内置数值,内容自动获取,可调下方颜色/字号'
      moduleBodyEl.appendChild(note)
      // F1:整句文案模板(如「今日已用 {值}」;留空=默认)
      moduleTplRow()
      // v768/v769：对话名模块额外给「保留长度」（与悬浮编辑器共用同一份实现；
      // 悬浮窗改了会实时刷新预览，这里点保存才落盘）
      if (m.type === 'session') {
        moduleBodyEl.appendChild(sessionLenRowBuild(function () {
          var d = document.createElement('div')
          d.className = 'dshwv-audiorow'
          return d
        }, function (t) {
          var s = document.createElement('span')
          s.textContent = t
          s.style.flex = '0 0 auto'
          return s
        }, m, null))
      }
    }
  }
  // 公共样式区(图片/动图、随机语句模块不显示:随机语句样式在每句单独编辑中设置)
  // 顺序与悬浮窗一致:字体 → 字号 → 加粗/斜体/下划线 → 颜色(峰谷颜色在上方单独设置)
  if (!bubbleIsImgMod(m) && m.type !== 'random') {
  var isPeak = m.type === 'peak' || m.type === 'nextpeak'
  var sec = document.createElement('div')
  sec.className = 'dshwv-bubsec'
  sec.textContent = '样式'
  moduleBodyEl.appendChild(sec)
  // 字体
  moduleBodyEl.appendChild(bubbleFontEditRow(function () { return m.fontFamily || '' }, function (v) { m.fontFamily = v || '' }))
  // 字号
  var sizeRow = document.createElement('div')
  sizeRow.className = 'dshwv-audiorow'
  var sl = document.createElement('span')
  sl.textContent = '字号'
  sizeRow.appendChild(sl)
  moduleSizeEl = document.createElement('input')
  moduleSizeEl.type = 'range'
  moduleSizeEl.min = '1'
  moduleSizeEl.max = '50'
  moduleSizeEl.step = '1'
  moduleSizeEl.className = 'dshwv-cropzoom'
  moduleSizeEl.value = String(m.size || 6)
  sizeRow.appendChild(moduleSizeEl)
  var sizeNum = document.createElement('span')
  sizeNum.className = 'dshwv-volpct'
  sizeNum.textContent = String(m.size || 6)
  moduleSizeEl.addEventListener('input', function () { sizeNum.textContent = moduleSizeEl.value })
  sizeRow.appendChild(sizeNum)
  moduleBodyEl.appendChild(sizeRow)
  // 字形:加粗/斜体/下划线
  var glyphRow = document.createElement('div')
  glyphRow.className = 'dshwv-audiorow'
  function glyphBox(label, key) {
    var lab = document.createElement('label')
    lab.style.display = 'inline-flex'
    lab.style.alignItems = 'center'
    lab.style.gap = '3px'
    lab.style.marginRight = '10px'
    var cb = document.createElement('input')
    cb.type = 'checkbox'
    cb.checked = !!m[key]
    cb.addEventListener('change', function () { m[key] = cb.checked })
    var tx = document.createElement('span')
    tx.textContent = label
    lab.appendChild(cb)
    lab.appendChild(tx)
    return lab
  }
  glyphRow.appendChild(glyphBox('加粗', 'bold'))
  glyphRow.appendChild(glyphBox('斜体', 'italic'))
  glyphRow.appendChild(glyphBox('下划线', 'ul'))
  moduleBodyEl.appendChild(glyphRow)
  // 颜色(峰谷的通用色省略:高峰/空闲色已在顶部各自设置)
  if (!isPeak) {
    var w3cc = qColorSelectBuild(m.rgb ? m.rgb : 'solid', function (v) {
      if (v === 'solid') { m.rgb = ''; if (!m.color) m.color = '#203170' }
      else { m.rgb = v; m.color = '' }
      var mode2 = v === 'solid' ? 'solid' : v
      w3cc.sync(mode2, m.color, function (hex) { m.color = hex })
    })
    moduleBodyEl.appendChild(w3cc.row)
    w3cc.sync(m.rgb ? m.rgb : 'solid', m.color || '#203170', function (hex) { m.color = hex })
  }
  if (!isPeak) {
    // 文字底色(默认无;纯色或跑马灯)。峰谷/倒计时不在此设:两状态各自有「高峰底色/空闲底色」
    var bgCur = m.bgRgb ? m.bgRgb : (m.bg ? 'solid' : 'none')
    var bgcc = qColorSelectBuild(bgCur, function (v) {
      if (v === 'none') { m.bgRgb = ''; m.bg = '' }
      else if (v === 'solid') { m.bgRgb = ''; if (!m.bg) m.bg = '#dbe4f5' }
      else { m.bgRgb = v; m.bg = '' }
      bgcc.sync(v === 'none' ? 'none' : v, m.bg, function (hex) { m.bg = hex })
    }, { label: '底色', defaultHex: '#dbe4f5', defaultText: '默认', allowNone: true })
    moduleBodyEl.appendChild(bgcc.row)
    bgcc.sync(bgCur, m.bg || '#dbe4f5', function (hex) { m.bg = hex })
  }
  }
}
var moduleNamePromptModule = null
function openModuleNamePrompt(m) {
  try {
    moduleNamePromptModule = m || moduleEditRef || null
    moduleNameInput.value = ''
    moduleNamePromptMask.style.display = 'flex'
    setTimeout(function () { try { moduleNameInput.focus() } catch (err) {} }, 30)
  } catch (err) {}
}
function closeModuleNamePrompt() {
  moduleNamePromptMask.style.display = 'none'
  moduleNamePromptModule = null
}
function saveModuleNamePrompt() {
  var m = moduleNamePromptModule || moduleEditRef
  if (!m) { closeModuleNamePrompt(); return }
  bubbleLibAdd(moduleNameInput.value, m)
  moduleNameInput.value = ''
  closeModuleNamePrompt()
  if (bubblePalEl) renderBubblePal()
}
function openModuleEditor(m, onSave, isNew) {
  try {
    whaleZClean()
    moduleEditRef = m
    moduleOnSave = onSave || null
    moduleEditNew = !!isNew
    renderModuleEditor()
    // v744：模块编辑器从泡泡编辑器(20500)/资源管理(20300)里打开时，固定 20500 会与父窗口同层
    // （只靠 DOM 顺序决定谁在上面），这里统一抬到"当前最高层之上"
    dshwLayerUp(moduleMask, 20500)
    moduleMask.style.display = 'flex'
  } catch (err) {}
}
function closeModuleEditor(saved) {
  try {
    if (saved && moduleEditRef) {
      if (moduleColorEl) moduleEditRef.color = moduleColorEl.value === '#203170' && !moduleEditRef.color ? '' : moduleColorEl.value
      if (moduleSizeEl) moduleEditRef.size = Math.max(1, Math.min(50, Math.round(Number(moduleSizeEl.value) || 6)))
      if (moduleEditRef.type === 'image' && !moduleEditRef.imgId) {
        // 未选图不允许保存
        showConfirm('请先选择或上传一张图片', function () {})
        return
      }
      if (moduleEditRef.type === 'randimg') {
        // 随机图片:至少要有一张已选图片,否则泡泡里什么都不会显示
        var anyImg2 = false
        var arr2 = Array.isArray(moduleEditRef.imgs) ? moduleEditRef.imgs : []
        for (var qi2 = 0; qi2 < arr2.length; qi2++) if (arr2[qi2] && arr2[qi2].imgId) { anyImg2 = true; break }
        if (!anyImg2) {
          showConfirm('随机图片还没有图片:请先「添加图片」或「上传图片」', function () {})
          return
        }
      }
      if (moduleOnSave) moduleOnSave(moduleEditRef)
    }
    moduleMask.style.display = 'none'
    moduleEditRef = null
    moduleOnSave = null
    moduleEditNew = false
  } catch (err) { moduleMask.style.display = 'none' }
}
moduleMask = document.createElement('div')
moduleMask.className = 'dshwv-bubmask'
moduleMask.style.display = 'none'
var moduleCard = document.createElement('div')
moduleCard.className = 'dshwv-bubcard'
moduleTitleEl = document.createElement('div')
moduleTitleEl.className = 'dshwv-bubtitle'
moduleCard.appendChild(moduleTitleEl)
moduleBodyEl = document.createElement('div')
moduleCard.appendChild(moduleBodyEl)
var moduleBtns = document.createElement('div')
moduleBtns.className = 'dshwv-bubbtns'
moduleBtns.appendChild(bubbleBtn('取消', 'dshwv-bubbtn-no', function () { closeModuleEditor(false) }))
// 「另存」= 存为可选模块,置于取消与保存中间;悬浮显示含义
var saveAsBtn = document.createElement('button')
saveAsBtn.type = 'button'
saveAsBtn.className = 'dshwv-bubbtn dshwv-bubbtn-no'
saveAsBtn.textContent = '另存'
saveAsBtn.title = '另存为可选模块:把当前模块存入模块库,可在任意泡泡里复用'
saveAsBtn.addEventListener('click', function (e) { e.stopPropagation(); openModuleNamePrompt(moduleEditRef) })
moduleBtns.appendChild(saveAsBtn)
moduleBtns.appendChild(bubbleBtn('保存', 'dshwv-bubbtn-ok', function () { closeModuleEditor(true) }))
moduleCard.appendChild(moduleBtns)
moduleMask.appendChild(moduleCard)
dshwBodyAppend(moduleMask)
// 存为可选模块:名称输入弹窗
var moduleNamePromptMask = document.createElement('div')
moduleNamePromptMask.className = 'dshwv-confirmmask'
moduleNamePromptMask.style.display = 'none'
var moduleNamePromptCard = document.createElement('div')
moduleNamePromptCard.className = 'dshwv-audiowin'
var moduleNamePromptTitle = document.createElement('div')
moduleNamePromptTitle.className = 'dshwv-audiotitle'
moduleNamePromptTitle.textContent = '存为可选模块'
moduleNamePromptCard.appendChild(moduleNamePromptTitle)
var moduleNameInput = document.createElement('input')
moduleNameInput.type = 'text'
moduleNameInput.className = 'dshwv-audionameinput'
moduleNameInput.maxLength = 20
moduleNameInput.placeholder = '模块名称(留空自动编号)'
moduleNamePromptCard.appendChild(moduleNameInput)
var moduleNameBtns = document.createElement('div')
moduleNameBtns.className = 'dshwv-cropbtns'
var moduleNameCancel = document.createElement('button')
moduleNameCancel.type = 'button'
moduleNameCancel.className = 'dshwv-cropbtn dshwv-cropbtn-no'
moduleNameCancel.textContent = '取消'
moduleNameCancel.addEventListener('click', closeModuleNamePrompt)
var moduleNameOk = document.createElement('button')
moduleNameOk.type = 'button'
moduleNameOk.className = 'dshwv-cropbtn dshwv-cropbtn-ok'
moduleNameOk.textContent = '保存'
moduleNameOk.addEventListener('click', saveModuleNamePrompt)
moduleNameBtns.appendChild(moduleNameCancel)
moduleNameBtns.appendChild(moduleNameOk)
moduleNamePromptCard.appendChild(moduleNameBtns)
moduleNamePromptMask.appendChild(moduleNamePromptCard)
dshwBodyAppend(moduleNamePromptMask)
// 回车保存/关闭
moduleNameInput.addEventListener('keydown', function (e) {
  try {
    if (e.key === 'Enter') saveModuleNamePrompt()
    else if (e.key === 'Escape') closeModuleNamePrompt()
  } catch (err) {}
})

// —— 导入裁剪弹窗 ——
var CROP_BOX = 260
var cropMask = document.createElement('div')
cropMask.className = 'dshwv-cropmask'
cropMask.style.display = 'none'
var cropCard = document.createElement('div')
cropCard.className = 'dshwv-cropwin'
var cropTitle = document.createElement('div')
cropTitle.className = 'dshwv-croptitle'
cropTitle.textContent = '裁剪角色图片'
var cropBox = document.createElement('div')
cropBox.className = 'dshwv-cropbox'
var cropCanvas = document.createElement('canvas')
cropCanvas.width = CROP_BOX
cropCanvas.height = CROP_BOX
cropBox.appendChild(cropCanvas)
var cropZoom = document.createElement('input')
cropZoom.type = 'range'
cropZoom.min = '0.3'
cropZoom.max = '3'
cropZoom.step = '0.01'
cropZoom.value = '1'
cropZoom.className = 'dshwv-cropzoom'
// 缩放行：标签 + 滑块 + 数字框（百分比，50–300）
var cropZoomWrap = document.createElement('div')
cropZoomWrap.className = 'dshwv-cropctrl'
var cropZoomLabel = document.createElement('span')
cropZoomLabel.className = 'dshwv-croplabel'
cropZoomLabel.textContent = '缩放'
var cropZoomNum = document.createElement('input')
cropZoomNum.type = 'number'
cropZoomNum.min = '30'
cropZoomNum.max = '300'
cropZoomNum.step = '1'
cropZoomNum.value = '100'
cropZoomNum.className = 'dshwv-cropnum'
cropZoomWrap.appendChild(cropZoomLabel)
cropZoomWrap.appendChild(cropZoom)
cropZoomWrap.appendChild(cropZoomNum)
var cropNameInput = document.createElement('input')
cropNameInput.type = 'text'
cropNameInput.className = 'dshwv-cropname'
cropNameInput.maxLength = 16
cropNameInput.placeholder = '角色名称'
// 旋转行：标签 + 水平翻转 + 竖直翻转 + 滑块 + 数值框（滑块撑满剩余宽度，右侧与缩放行右对齐）
var cropAngleWrap = document.createElement('div')
cropAngleWrap.className = 'dshwv-cropctrl'
var cropAngleLabel = document.createElement('span')
cropAngleLabel.className = 'dshwv-croplabel'
cropAngleLabel.textContent = '旋转'
var cropFlipHBtn = document.createElement('button')
cropFlipHBtn.type = 'button'
cropFlipHBtn.className = 'dshwv-cropflip'
cropFlipHBtn.textContent = '⇋'
cropFlipHBtn.title = '水平翻转'
var cropFlipVBtn = document.createElement('button')
cropFlipVBtn.type = 'button'
cropFlipVBtn.className = 'dshwv-cropflip'
cropFlipVBtn.textContent = '⇅'
cropFlipVBtn.title = '垂直翻转'
var cropAngle = document.createElement('input')
cropAngle.type = 'range'
cropAngle.min = '-360'
cropAngle.max = '360'
cropAngle.step = '1'
cropAngle.value = '0'
cropAngle.className = 'dshwv-cropzoom'
var cropAngleNum = document.createElement('input')
cropAngleNum.type = 'number'
cropAngleNum.min = '-360'
cropAngleNum.max = '360'
cropAngleNum.step = '1'
cropAngleNum.value = '0'
cropAngleNum.className = 'dshwv-cropnum'
cropAngleWrap.appendChild(cropAngleLabel)
cropAngleWrap.appendChild(cropFlipHBtn)
cropAngleWrap.appendChild(cropFlipVBtn)
cropAngleWrap.appendChild(cropAngle)
cropAngleWrap.appendChild(cropAngleNum)
var cropBtns = document.createElement('div')
cropBtns.className = 'dshwv-cropbtns'
var cropCancelBtn = document.createElement('button')
cropCancelBtn.type = 'button'
cropCancelBtn.className = 'dshwv-cropbtn dshwv-cropbtn-no'
cropCancelBtn.textContent = '取消'
var cropResetBtn = document.createElement('button')
cropResetBtn.type = 'button'
cropResetBtn.className = 'dshwv-cropbtn dshwv-cropbtn-no'
cropResetBtn.textContent = '重置'
cropResetBtn.title = '重置缩放、旋转和位置'
var cropOkBtn = document.createElement('button')
cropOkBtn.type = 'button'
cropOkBtn.className = 'dshwv-cropbtn dshwv-cropbtn-ok'
cropOkBtn.textContent = '确认'
cropBtns.appendChild(cropCancelBtn)
cropBtns.appendChild(cropResetBtn)
cropBtns.appendChild(cropOkBtn)
cropCard.appendChild(cropTitle)
cropCard.appendChild(cropBox)
cropCard.appendChild(cropNameInput)
cropCard.appendChild(cropZoomWrap)
cropCard.appendChild(cropAngleWrap)
cropCard.appendChild(cropBtns)
cropMask.appendChild(cropCard)
dshwBodyAppend(cropMask)
cropBox.addEventListener('pointerdown', onCropDown)
cropBox.addEventListener('pointermove', onCropMove)
cropBox.addEventListener('pointerup', onCropUp)
cropBox.addEventListener('pointercancel', onCropUp)
cropBox.addEventListener('pointerleave', onCropUp)
cropBox.addEventListener('wheel', onCropWheel, { passive: false })
cropAngle.addEventListener('input', function () {
  if (cropState) {
    cropState.rotation = clampAngle(Number(cropAngle.value))
    cropAngleNum.value = String(cropState.rotation)
    positionCrop()
  }
})
cropAngleNum.addEventListener('input', function () {
  if (cropState) {
    var v = Math.round(Number(cropAngleNum.value))
    if (!isFinite(v)) v = 0
    cropState.rotation = clampAngle(v)
    cropAngle.value = String(cropState.rotation)
    positionCrop()
  }
})
cropAngleNum.addEventListener('change', function () {
  if (cropState) cropAngleNum.value = String(cropState.rotation)
})
cropZoom.addEventListener('input', function () {
  if (cropState) {
    cropState.zoom = Number(cropZoom.value)
    cropZoomNum.value = String(Math.round(cropState.zoom * 100))
    positionCrop()
  }
})
cropZoomNum.addEventListener('input', function () {
  if (cropState) {
    var pct = Number(cropZoomNum.value)
    if (!isFinite(pct)) pct = 100
    cropState.zoom = Math.min(3, Math.max(0.3, pct / 100))
    cropZoom.value = String(cropState.zoom)
    positionCrop()
  }
})
cropZoomNum.addEventListener('change', function () {
  if (cropState) cropZoomNum.value = String(Math.round(cropState.zoom * 100))
})
cropCancelBtn.addEventListener('click', function () { hideCropModal() })
cropResetBtn.addEventListener('click', function () { resetCrop() })
cropOkBtn.addEventListener('click', function () { confirmCrop() })
// 翻转：给图片容器加一个带过渡的镜像动画，动画结束后落到画布重绘
cropFlipHBtn.addEventListener('click', function (e) {
  e.stopPropagation()
  flipCrop('H')
})
cropFlipVBtn.addEventListener('click', function (e) {
  e.stopPropagation()
  flipCrop('V')
})

// —— GIF 动图角色导入弹窗（不支持裁剪，原样上传） ——
var gifMask = document.createElement('div')
gifMask.className = 'dshwv-gifmask'
gifMask.style.display = 'none'
var gifCard = document.createElement('div')
gifCard.className = 'dshwv-gifwin'
var gifTitle = document.createElement('div')
gifTitle.className = 'dshwv-giftitle'
gifTitle.textContent = '导入动图角色'
var gifPreviewBox = document.createElement('div')
gifPreviewBox.className = 'dshwv-gifpreview'
var gifPreviewImg = document.createElement('img')
gifPreviewImg.className = 'dshwv-gifpreviewimg'
gifPreviewImg.alt = '动图预览'
gifPreviewImg.draggable = false
gifPreviewBox.appendChild(gifPreviewImg)
var gifHint = document.createElement('div')
gifHint.className = 'dshwv-gifhint'
gifHint.textContent = 'GIF 动图不支持裁剪，将按原始尺寸原样导入'
var gifNameInput = document.createElement('input')
gifNameInput.type = 'text'
gifNameInput.className = 'dshwv-gifname'
gifNameInput.maxLength = 16
gifNameInput.placeholder = '角色名称'
var gifBtns = document.createElement('div')
gifBtns.className = 'dshwv-cropbtns'
var gifCancelBtn = document.createElement('button')
gifCancelBtn.type = 'button'
gifCancelBtn.className = 'dshwv-cropbtn dshwv-cropbtn-no'
gifCancelBtn.textContent = '取消'
var gifOkBtn = document.createElement('button')
gifOkBtn.type = 'button'
gifOkBtn.className = 'dshwv-cropbtn dshwv-cropbtn-ok'
gifOkBtn.textContent = '确认'
gifBtns.appendChild(gifCancelBtn)
gifBtns.appendChild(gifOkBtn)
gifCard.appendChild(gifTitle)
gifCard.appendChild(gifPreviewBox)
gifCard.appendChild(gifHint)
gifCard.appendChild(gifNameInput)
gifCard.appendChild(gifBtns)
gifMask.appendChild(gifCard)
dshwBodyAppend(gifMask)
gifCancelBtn.addEventListener('click', hideGifRoleModal)
gifOkBtn.addEventListener('click', confirmGifRole)
var gifRoleDataUrl = null
var gifRoleFileName = ''
var gifRoleAnimType = 'gif' // 'gif' | 'apng'
function openGifRoleModal(dataUrl, fileName, animType) {
  gifRoleDataUrl = dataUrl
  gifRoleAnimType = animType === 'apng' ? 'apng' : 'gif'
  gifRoleFileName = (fileName || '').replace(/.[^.]+$/, '') || '新角色'
  // 标题/提示按格式区分
  if (gifRoleAnimType === 'apng') {
    gifTitle.textContent = '导入 APNG 动图角色'
    gifHint.textContent = 'APNG 动图不支持裁剪，将按原始尺寸原样导入'
  } else {
    gifTitle.textContent = '导入 GIF 动图角色'
    gifHint.textContent = 'GIF 动图不支持裁剪，将按原始尺寸原样导入'
  }
  gifNameInput.value = ''
  gifPreviewImg.src = dataUrl
  dshwLayerUp(gifMask, 20000) // v744：同上，GIF 窗口也可能从资源管理/泡泡编辑器里打开
  gifMask.style.display = 'flex'
}
function hideGifRoleModal() {
  gifMask.style.display = 'none'
  gifRoleDataUrl = null
  gifRoleFileName = ''
  gifRoleAnimType = 'gif'
  gifPreviewImg.src = ''
}
function confirmGifRole() {
  try {
    // 名字留空 → 统一「新角色」（不回落文件名，与裁剪导入一致）
    var name = (gifNameInput.value || '').trim().slice(0, 16) || '新角色'
    if (!gifRoleDataUrl) { hideGifRoleModal(); return }
    fetch(ROLE_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      // format 显式传给 host：APNG 的 dataURL 是 image/png，host 无法自行区分
      body: JSON.stringify({ name: name, image: gifRoleDataUrl, format: gifRoleAnimType }),
    })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (d && d.ok && Array.isArray(d.roles)) {
          roleList = d.roles
          renderRolePanel()
          // 自动切换到刚导入的角色
          var newest = null
          for (var i = 0; i < roleList.length; i++) {
            if (roleList[i].id !== 'default' && (!newest || roleList[i].createdAt > newest.createdAt)) newest = roleList[i]
          }
          if (newest) applyRole(newest.id, newest.name, roleUrl(newest.id))
        }
      })
      .catch(function () {})
      .finally(function () { hideGifRoleModal() })
  } catch (err) { try { hideGifRoleModal() } catch (err2) {} }
}

// —— 删除确认弹窗 ——
var confirmMask = document.createElement('div')
confirmMask.className = 'dshwv-confirmmask'
confirmMask.style.display = 'none'
var confirmCard = document.createElement('div')
confirmCard.className = 'dshwv-confirmwin'
var confirmText = document.createElement('div')
confirmText.className = 'dshwv-confirmtext'
var confirmBtns = document.createElement('div')
confirmBtns.className = 'dshwv-confirmbtns'
var confirmNoBtn = document.createElement('button')
confirmNoBtn.type = 'button'
confirmNoBtn.className = 'dshwv-cropbtn dshwv-cropbtn-no'
confirmNoBtn.textContent = '取消'
var confirmYesBtn = document.createElement('button')
confirmYesBtn.type = 'button'
confirmYesBtn.className = 'dshwv-cropbtn dshwv-cropbtn-ok'
confirmYesBtn.textContent = '删除'
confirmBtns.appendChild(confirmNoBtn)
confirmBtns.appendChild(confirmYesBtn)
confirmCard.appendChild(confirmText)
confirmCard.appendChild(confirmBtns)
confirmMask.appendChild(confirmCard)
dshwBodyAppend(confirmMask)
confirmNoBtn.addEventListener('click', hideConfirm)
confirmYesBtn.addEventListener('click', function () {
  var cb = confirmCb
  hideConfirm()
  if (cb) cb()
})

// —— 音频组编辑弹窗 ——
var audioEditMask = document.createElement('div')
audioEditMask.className = 'dshwv-audiomask'
audioEditMask.style.display = 'none'
var audioEditCard = document.createElement('div')
audioEditCard.className = 'dshwv-audiowin'
var audioEditTitle = document.createElement('div')
audioEditTitle.className = 'dshwv-audiotitle'
audioEditTitle.textContent = '音效组'
var audioEditName = document.createElement('input')
audioEditName.type = 'text'
audioEditName.className = 'dshwv-audionameinput'
audioEditName.maxLength = 20
audioEditName.placeholder = '预设名称'
var audioEditPressRow = document.createElement('div')
audioEditPressRow.className = 'dshwv-audiorow'
var audioEditPressLabel = document.createElement('span')
audioEditPressLabel.className = 'dshwv-audioslotlabel'
audioEditPressLabel.textContent = '按压'
var audioEditPressWrap = document.createElement('div')
audioEditPressWrap.className = 'dshwv-slotwrap'
var audioEditPressBtn = document.createElement('button')
audioEditPressBtn.type = 'button'
audioEditPressBtn.className = 'dshwv-slotbtn'
audioEditPressBtn.textContent = '小黄鸭·按下'
var audioEditPressPanel = document.createElement('div')
audioEditPressPanel.className = 'dshwv-slotlist'
audioEditPressWrap.appendChild(audioEditPressBtn)
audioEditPressWrap.appendChild(audioEditPressPanel)
var audioEditPressImport = document.createElement('button')
audioEditPressImport.type = 'button'
audioEditPressImport.className = 'dshwv-audiosmallimport'
audioEditPressImport.textContent = '导入'
audioEditPressImport.title = '导入并裁剪按压音'
audioEditPressRow.appendChild(audioEditPressLabel)
audioEditPressRow.appendChild(audioEditPressWrap)
audioEditPressRow.appendChild(audioEditPressImport)
var audioEditReleaseRow = document.createElement('div')
audioEditReleaseRow.className = 'dshwv-audiorow'
var audioEditReleaseLabel = document.createElement('span')
audioEditReleaseLabel.className = 'dshwv-audioslotlabel'
audioEditReleaseLabel.textContent = '松开'
var audioEditReleaseWrap = document.createElement('div')
audioEditReleaseWrap.className = 'dshwv-slotwrap'
var audioEditReleaseBtn = document.createElement('button')
audioEditReleaseBtn.type = 'button'
audioEditReleaseBtn.className = 'dshwv-slotbtn'
audioEditReleaseBtn.textContent = '小黄鸭·松开'
var audioEditReleasePanel = document.createElement('div')
audioEditReleasePanel.className = 'dshwv-slotlist'
audioEditReleaseWrap.appendChild(audioEditReleaseBtn)
audioEditReleaseWrap.appendChild(audioEditReleasePanel)
var audioEditReleaseImport = document.createElement('button')
audioEditReleaseImport.type = 'button'
audioEditReleaseImport.className = 'dshwv-audiosmallimport'
audioEditReleaseImport.textContent = '导入'
audioEditReleaseImport.title = '导入并裁剪松开音'
audioEditReleaseRow.appendChild(audioEditReleaseLabel)
audioEditReleaseRow.appendChild(audioEditReleaseWrap)
audioEditReleaseRow.appendChild(audioEditReleaseImport)
var audioEditBtns = document.createElement('div')
audioEditBtns.className = 'dshwv-cropbtns'
var audioEditCancel = document.createElement('button')
audioEditCancel.type = 'button'
audioEditCancel.className = 'dshwv-cropbtn dshwv-cropbtn-no'
audioEditCancel.textContent = '取消'
var audioEditPlay = document.createElement('button')
audioEditPlay.type = 'button'
audioEditPlay.className = 'dshwv-cropbtn dshwv-cropbtn-no'
audioEditPlay.textContent = '试听'
audioEditPlay.title = '按住播放按压音，松开播放松开音（模拟点击挂件）'
var audioEditSave = document.createElement('button')
audioEditSave.type = 'button'
audioEditSave.className = 'dshwv-cropbtn dshwv-cropbtn-ok'
audioEditSave.textContent = '保存'
audioEditBtns.appendChild(audioEditCancel)
audioEditBtns.appendChild(audioEditPlay)
audioEditBtns.appendChild(audioEditSave)
audioEditCard.appendChild(audioEditTitle)
audioEditCard.appendChild(audioEditName)
audioEditCard.appendChild(audioEditPressRow)
audioEditCard.appendChild(audioEditReleaseRow)
audioEditCard.appendChild(audioEditBtns)
audioEditMask.appendChild(audioEditCard)
dshwBodyAppend(audioEditMask)
audioEditCancel.addEventListener('click', hideAudioEditor)
audioEditPlay.addEventListener('pointerdown', function (e) { e.stopPropagation(); audioEditPreviewDown() })
audioEditPlay.addEventListener('pointerup', function (e) { e.stopPropagation(); audioEditPreviewUp() })
audioEditPlay.addEventListener('pointercancel', audioEditPreviewUp)
audioEditPlay.addEventListener('pointerleave', audioEditPreviewUp)
audioEditSave.addEventListener('click', saveAudioGroup)
// 点击遮罩空白处关闭片段面板（不关闭编辑器）
audioEditMask.addEventListener('click', function (e) {
  if (e.target === audioEditMask || e.target === audioEditCard) closeAudioSlotPanels()
})

// —— 音频裁剪弹窗 ——
var audioCropMask = document.createElement('div')
audioCropMask.className = 'dshwv-audiomask'
audioCropMask.style.display = 'none'
var audioCropCard = document.createElement('div')
audioCropCard.className = 'dshwv-audiowin'
var audioCropTitle = document.createElement('div')
audioCropTitle.className = 'dshwv-audiotitle'
audioCropTitle.textContent = '裁剪音频'
var audioCropCanvas = document.createElement('canvas')
audioCropCanvas.width = 300
audioCropCanvas.height = 120
audioCropCanvas.className = 'dshwv-audiocropcanvas'
var audioCropStart = document.createElement('input')
audioCropStart.type = 'range'
audioCropStart.min = '0'
audioCropStart.max = '100'
audioCropStart.step = '0.001'
audioCropStart.value = '0'
audioCropStart.className = 'dshwv-cropzoom'
var audioCropStartNum = document.createElement('input')
audioCropStartNum.type = 'number'
audioCropStartNum.min = '0'
audioCropStartNum.step = '0.001'
audioCropStartNum.value = '0'
audioCropStartNum.className = 'dshwv-cropnum'
audioCropStartNum.title = '起始时间（秒）'
var audioCropStartRow = document.createElement('div')
audioCropStartRow.className = 'dshwv-audiosliderrow'
audioCropStartRow.appendChild(audioCropStart)
audioCropStartRow.appendChild(audioCropStartNum)
var audioCropEnd = document.createElement('input')
audioCropEnd.type = 'range'
audioCropEnd.min = '0'
audioCropEnd.max = '100'
audioCropEnd.step = '0.001'
audioCropEnd.value = '100'
audioCropEnd.className = 'dshwv-cropzoom'
var audioCropEndNum = document.createElement('input')
audioCropEndNum.type = 'number'
audioCropEndNum.min = '0'
audioCropEndNum.step = '0.001'
audioCropEndNum.value = '0'
audioCropEndNum.className = 'dshwv-cropnum'
audioCropEndNum.title = '结束时间（秒）'
var audioCropEndRow = document.createElement('div')
audioCropEndRow.className = 'dshwv-audiosliderrow'
audioCropEndRow.appendChild(audioCropEnd)
audioCropEndRow.appendChild(audioCropEndNum)
// 合并后的起止双滑块：一条轨道上两个 thumb（起点/终点），原滑块保留为隐藏数据源
audioCropStart.style.display = 'none'
audioCropEnd.style.display = 'none'
var audioCropDual = document.createElement('div')
audioCropDual.className = 'dshwv-dualrange'
var audioCropDualTrack = document.createElement('div')
audioCropDualTrack.className = 'dshwv-dualrange-track'
var audioCropDualFill = document.createElement('div')
audioCropDualFill.className = 'dshwv-dualrange-fill'
var audioCropDualStart = document.createElement('div')
audioCropDualStart.className = 'dshwv-dualrange-thumb'
var audioCropDualEnd = document.createElement('div')
audioCropDualEnd.className = 'dshwv-dualrange-thumb'
audioCropDual.appendChild(audioCropDualTrack)
audioCropDual.appendChild(audioCropDualFill)
audioCropDual.appendChild(audioCropDualStart)
audioCropDual.appendChild(audioCropDualEnd)
// 合并行：起点数字框 + 双滑块轨道 + 终点数字框
var audioCropDualRow = document.createElement('div')
audioCropDualRow.className = 'dshwv-audiosliderrow'
audioCropDualRow.appendChild(audioCropStartNum)
audioCropDualRow.appendChild(audioCropDual)
audioCropDualRow.appendChild(audioCropEndNum)
// 波形缩放条：缩放倍数标签 + 滑块 + 数值输入
var audioCropZoomLabel = document.createElement('span')
audioCropZoomLabel.className = 'dshwv-zoomlabel'
audioCropZoomLabel.textContent = '缩放倍数'
var audioCropZoomRange = document.createElement('input')
audioCropZoomRange.type = 'range'
audioCropZoomRange.min = '1'
audioCropZoomRange.max = '50'
audioCropZoomRange.step = '1'
audioCropZoomRange.value = '1'
audioCropZoomRange.className = 'dshwv-cropzoom'
audioCropZoomRange.title = '波形放大倍数'
var audioCropZoomNum = document.createElement('input')
audioCropZoomNum.type = 'number'
audioCropZoomNum.min = '1'
audioCropZoomNum.max = '50'
audioCropZoomNum.step = '1'
audioCropZoomNum.value = '1'
audioCropZoomNum.className = 'dshwv-cropnum'
audioCropZoomNum.title = '放大倍数'
var audioCropZoomRow = document.createElement('div')
audioCropZoomRow.className = 'dshwv-audiosliderrow'
audioCropZoomRow.appendChild(audioCropZoomLabel)
audioCropZoomRow.appendChild(audioCropZoomRange)
audioCropZoomRow.appendChild(audioCropZoomNum)
var audioCropTime = document.createElement('div')
audioCropTime.className = 'dshwv-audiotime'
audioCropTime.textContent = '0.0s – 0.0s'
var audioCropNameRow = document.createElement('div')
audioCropNameRow.className = 'dshwv-audiosliderrow'
audioCropNameRow.style.justifyContent = 'center'
audioCropNameRow.style.margin = '2px 0'
var audioCropName = document.createElement('input')
audioCropName.type = 'text'
audioCropName.maxLength = 30
audioCropName.placeholder = '音频片段名称'
audioCropName.title = '裁剪后片段的名称（可改名，留空用源文件名）'
audioCropName.style.cssText = 'width:min(60%,240px);flex:0 1 auto;margin:0;text-align:center;border:1px solid rgba(32,49,112,.4);border-radius:6px;padding:2px 6px;font-size:12px;color:#203170;background:#fff;box-sizing:border-box'
audioCropNameRow.appendChild(audioCropName)
audioCropNameRow.style.marginBottom = '12px'
function updateAudioCropOkState() { try { audioCropOk.disabled = false } catch (err) {} }
audioCropName.addEventListener('input', updateAudioCropOkState)
var audioCropBtns = document.createElement('div')
audioCropBtns.className = 'dshwv-cropbtns'
var audioCropCancel = document.createElement('button')
audioCropCancel.type = 'button'
audioCropCancel.className = 'dshwv-cropbtn dshwv-cropbtn-no'
audioCropCancel.textContent = '取消'
var audioCropPlay = document.createElement('button')
audioCropPlay.type = 'button'
audioCropPlay.className = 'dshwv-cropbtn dshwv-cropbtn-no'
audioCropPlay.textContent = '试听'
audioCropPlay.title = '试听裁剪后的片段'
var audioCropOk = document.createElement('button')
audioCropOk.type = 'button'
audioCropOk.className = 'dshwv-cropbtn dshwv-cropbtn-ok'
audioCropOk.textContent = '确认'
audioCropOk.disabled = false
audioCropBtns.appendChild(audioCropCancel)
audioCropBtns.appendChild(audioCropPlay)
audioCropBtns.appendChild(audioCropOk)
audioCropCard.appendChild(audioCropTitle)
audioCropCard.appendChild(audioCropCanvas)
audioCropCard.appendChild(audioCropDualRow)
audioCropCard.appendChild(audioCropZoomRow)
audioCropCard.appendChild(audioCropTime)
audioCropCard.appendChild(audioCropNameRow)
audioCropCard.appendChild(audioCropBtns)
audioCropMask.appendChild(audioCropCard)
dshwBodyAppend(audioCropMask)
audioCropCancel.addEventListener('click', hideAudioCrop)
audioCropOk.addEventListener('click', confirmAudioCrop)
audioCropPlay.addEventListener('click', previewAudioCrop)
audioCropStart.addEventListener('input', onAudioCropStartInput)
audioCropEnd.addEventListener('input', onAudioCropEndInput)
audioCropStartNum.addEventListener('input', onAudioCropStartNumInput)
audioCropStartNum.addEventListener('change', onAudioCropStartNumChange)
audioCropEndNum.addEventListener('input', onAudioCropEndNumInput)
audioCropEndNum.addEventListener('change', onAudioCropEndNumChange)
audioCropZoomRange.addEventListener('input', onAudioCropZoomInput)
audioCropZoomNum.addEventListener('input', onAudioCropZoomNumInput)
audioCropZoomNum.addEventListener('change', onAudioCropZoomNumChange)
audioCropCanvas.addEventListener('wheel', onAudioCropWheel, { passive: false })
audioCropCanvas.addEventListener('pointerdown', onAudioCropSelDown)
audioCropCanvas.addEventListener('pointermove', onAudioCropSelMove)
audioCropCanvas.addEventListener('pointerup', onAudioCropSelUp)
audioCropCanvas.addEventListener('pointercancel', onAudioCropSelUp)
// 双滑块：按下判定拖哪个 thumb，拖动更新起止值
audioCropDual.addEventListener('pointerdown', onAudioCropDualDown)
audioCropDual.addEventListener('pointermove', onAudioCropDualMove)
audioCropDual.addEventListener('pointerup', onAudioCropDualUp)
audioCropDual.addEventListener('pointercancel', onAudioCropDualUp)

var textBox = document.createElement('div')
textBox.className = 'dshwv-text'
var labelEl = document.createElement('div')
labelEl.className = 'dshwv-label'
labelEl.textContent = 'Codex 用量'
var amountEl = document.createElement('div')
amountEl.className = 'dshwv-amount'
var hintEl = document.createElement('div')
hintEl.className = 'dshwv-hint'
textBox.appendChild(labelEl)
textBox.appendChild(amountEl)
textBox.appendChild(hintEl)

var bubbleBox = document.createElement('div')
bubbleBox.className = 'dshwv-pop'
bubbleBox.innerHTML = '<svg viewBox="0 0 1026 700" preserveAspectRatio="xMidYMid meet" xmlns="http://www.w3.org/2000/svg">' +
  '<path class="dshwv-bshape" fill="#FFFFFF" stroke="#203170" stroke-width="18" stroke-linejoin="round" stroke-linecap="round" d="M 827 248 A 373 232 0 1 0 81 246 A 373 232 0 0 0 301 465 A 57 32 10 0 0 413 484 A 373 232 0 0 0 827 248 Z"/>' +
  '<ellipse class="dshwv-b1" cx="352" cy="561" rx="37.5" ry="26" fill="#FFFFFF" stroke="#203170" stroke-width="18"/>' +
  '<ellipse class="dshwv-b2" cx="442" cy="646" rx="24.5" ry="18" fill="#FFFFFF" stroke="#203170" stroke-width="18"/>' +
  '</svg>'
var gifEl = document.createElement('img')
gifEl.className = 'dshwv-gif'
gifEl.src = GIF_URL
gifEl.alt = ''
gifEl.draggable = false
bubbleBox.appendChild(gifEl)
var gifFailed = false
gifEl.onerror = function () { gifFailed = true }
bubbleBox.appendChild(textBox)
bubbleBox.addEventListener('click', function (e) {
  e.stopPropagation()
  if (!bubbleShown) return
  if (costBubbleActive) {
    // 消耗金额泡泡：点击关闭（确认）
    hideCostBubble()
    return
  }
  // 点击泡泡 = 跳到下一项；已是最后一项则关闭（点鲸鱼才是从队列开头开始）
  bubbleNext()
})

// ===== v785：桌面客户端「窗口控件带」不进入挂件的可移动范围 =====
// 背景（用户 2026-10-02 反馈）：客户端右上角的最小化/最大化/关闭会盖在泡泡上方。
// 查证：DSH 桌面端产品窗口用 Electron `titleBarStyle:'hidden'` + **`titleBarOverlay`**
//   （主进程写死 height:40；macOS 是 `hiddenInset` + 左上 traffic light）—— 那三个键由**浏览器引擎**
//   画在所有页面内容之上 ⇒ **CSS/z-index 永远盖不过去**（不是我们层级写低了）。
// 修法（用户指定）：不做层级对抗、也不平移泡泡，而是**把整条顶带排除在挂件的可移动范围之外**：
//   挂件顶边（root.top）一律 ≥ 叠层下沿 ⇒ 画在 root 内部的泡泡自然也在带下，
//   而鲸鱼与泡泡的相对位置**完全不变**（比"泡泡自己下移"更干净）。
var DESKTOP_TITLEBAR_FALLBACK = 44 // 主进程 titleBarOverlay.height = 40，+4 余量
function desktopTitlebarBottom() {
  var wco = null
  try { wco = navigator.windowControlsOverlay } catch (err) { wco = null }
  // 没有叠层对象（普通浏览器 / 手机浏览器）或叠层不可见 ⇒ 不需要排除
  if (!wco || wco.visible === false) return 0
  // 有叠层：优先用官方矩形；取不到（旧内核/异常）就退回主进程里那个高度，**不能返回 0**
  try {
    if (typeof wco.getTitlebarAreaRect === 'function') {
      var r = wco.getTitlebarAreaRect()
      if (r && isFinite(r.y) && isFinite(r.height) && r.height > 0) return Math.max(0, Math.round(r.y + r.height))
    }
  } catch (err) {}
  return DESKTOP_TITLEBAR_FALLBACK
}
// 挂件顶边的**下限**（0 = 不限制，普通浏览器就是 0）
function widgetTopMin() {
  try { return Math.max(0, desktopTitlebarBottom()) } catch (err) { return 0 }
}
// 把「挂件顶边」夹进可移动范围：[顶带下沿, 视口内上限]
function clampWidgetTop(v, maxT) {
  var min = widgetTopMin()
  var t = Number(v)
  if (!isFinite(t)) t = min
  var hi = Infinity
  if (maxT !== undefined && isFinite(Number(maxT))) hi = Math.max(min, Number(maxT))
  return Math.max(min, Math.min(t, hi))
}
var body = document.createElement('div')
body.className = 'dshwv-body'
body.appendChild(img)
body.appendChild(bubbleBox)
root.appendChild(body)
root.appendChild(menuBtn)
dshwBodyAppend(root)
dshwBodyAppend(menuBox)

// ===== PR #105 后半：DOM 守护（SPA 切路由 / 别的插件替换 body 子树时把节点摘掉）=====
// 背景：DSH 是 SPA，切到会话列表 / 设置 / 插件市场再回来、或其它客户端插件整体替换
// document.body 的子树时，挂件节点会被顺带移除，而它不会自己回来（脚本只初始化一次）。
// 做法：暴露 window.__dshWhaleRoot 供外部定位/调试，并用一个 MutationObserver 盯着；
// 一旦发现节点已不在文档里就**把同一个节点补挂回 body**（不重建、不重新初始化，
// 位置/设置/状态全部保留）。
// v743：补挂范围从 root+menuBox 扩到**全部登记过的 body 节点**（见 dshwBodyNodes）——
// 否则被整体替换后，遮罩/面板/隐藏 file input 会变成孤儿，功能静默失效；
// 另外 root 还在、只有个别节点被摘掉的"局部移除"也要能自愈，所以做了节流的全量核对。
try { window.__dshWhaleRoot = root } catch (err) {}
function dshwReattachRoot() {
  try {
    for (var i = 0; i < dshwBodyNodes.length; i++) {
      var el = dshwBodyNodes[i]
      if (el && !dshwConnected(el)) dshwBodyAppend(el)
    }
  } catch (err) {}
}
try {
  if (typeof MutationObserver === 'function') {
    var dshwGuardLastFull = 0
    var dshwRootGuard = new MutationObserver(function () {
      try {
        if (!root) return
        if (!dshwConnected(root)) { dshwReattachRoot(); dshwGuardLastFull = Date.now(); return }
        // root 正常时也定期全量核对一次（最多 1.5 秒一次）：覆盖"只有个别浮层被摘掉"的情况
        var now = Date.now()
        if (now - dshwGuardLastFull > 1500) { dshwGuardLastFull = now; dshwReattachRoot() }
      } catch (err) {}
    })
    dshwRootGuard.observe(document.documentElement, { childList: true, subtree: true })
  }
} catch (err) {}

// 泡泡内容整体与视觉中心对齐:
// 读取 SVG 主体(bshape)的包围盒,取其中点作为文字内容区的视觉中心,
// 写入 --dshw-vx/--dshw-vy(相对 .dshwv-pop 尺寸的百分比)。
// 这样任意字号/行数的内容都围绕泡泡主体视觉中心整体居中,
// 不再靠“固定 top% + translate(-50%)”猜锚点,避免大字号与普通字号互相影响。
var dshwCenterX = 44.25 // 兜底:SVG viewBox 水平中心(454/1026)
var dshwCenterY = 36 // 兜底:主体视觉中心(旧经验值)
function measureBubbleCenter() {
  try {
    var svg = bubbleBox && bubbleBox.querySelector('svg')
    var shape = svg && svg.querySelector('.dshwv-bshape')
    if (!shape || typeof shape.getBBox !== 'function') return
    var bb = shape.getBBox()
    if (!bb || !isFinite(bb.x + bb.y + bb.width + bb.height) || bb.width <= 0 || bb.height <= 0) return
    // SVG viewBox 为 0 0 1026 700,box 中心即视觉中心
    var cx = (bb.x + bb.width / 2) / 1026 * 100
    var cy = (bb.y + bb.height / 2) / 700 * 100
    if (!isFinite(cx) || !isFinite(cy)) return
    dshwCenterX = cx
    dshwCenterY = cy
    try {
      // v751（PR #119）：写到挂件自己的 root 上，**不写 document.documentElement**。
      // 未注册的自定义属性写在 <html> 上会让 Blink 保守失效整棵子树样式（PR 实测 12k 节点会话
      // 一次约 160ms），而本函数在初始化、rAF、load 以及每次窗口 resize 都会跑。
      // 消费这两个变量的 .dshwv-text/.dshwv-gif 都在 root 内（继承即可）；
      // 编辑器预览在 root 之外，所以在 bubblePreviewInto() 里补写了两行。
      var s = root.style
      s.setProperty('--dshw-vx', cx + '%')
      s.setProperty('--dshw-vy', cy + '%')
    } catch (err) {}
  } catch (err) {}
}
// 立即尝试(此时 bubbleBox 已在 DOM);布局未就绪则等下一次再测
measureBubbleCenter()
try {
  if (typeof requestAnimationFrame === 'function') requestAnimationFrame(measureBubbleCenter)
} catch (err) {}
setTimeout(measureBubbleCenter, 120)
try {
  window.addEventListener('load', function () { measureBubbleCenter() })
} catch (err) {}

// Position model: the widget is ALWAYS expressed in left/top px (so edge snaps
// animate smoothly via the CSS transition on both sides — switching to
// right/auto cannot transition and flashes). The anchor info (h/v + offsets)
// lives in state and is used by settle() to recompute coordinates on window
// resize and size changes, keeping the widget glued to its anchored edge.
var state = {
  scale: 1.5,
  h: 'right',
  hOff: 0,
  v: 'bottom',
  vOff: 0,
  left: 0,
  top: 0,
  balance: null,
  currency: null,
  // v781：余额的两个分量（赠金 / 充值余额）。宿主在 API key 路与 DSH 账号路都会下发；
  //   取不到时保持 null ⇒ 模块显示 `—`（见 bubbleBonusText/bubbleRechargeText）。
  bonusBalance: null,
  rechargeBalance: null,
  todayUsage: null,
  todayUsageCurrency: 'CNY',
  usageLabel: '本地估算',
  isPeak: false,
  peakNextChangeAt: null,
  peakHolidays: null,
  status: 'loading',
  message: '',
  flip: false
}
// —— 吸附与翻转自定义配置 ——
// mode: 'ratio' 比例吸附 / 'px' 绝对吸附 / 'off' 关闭（无吸附、无翻转，纯自由）
// ratio.L/T/R/B: 各边吸附区占视口宽/高的百分比（距各自边缘的宽度/高度）
// ratio.F: 翻转线距左侧的百分比（翻转区 = 翻转线左侧；约束 L <= F <= 100-R）
// px.L/T/R/B: 各边吸附区距各自边缘的像素；px.F: 翻转线距左侧的像素
// px.F 为 -1 表示该档从未初始化（首次切到绝对档时按当前视口半宽补默认值）
var SNAP_KEY = 'dshw-snap'
// 配置版本：v2 = 默认吸附区改为 上下15%、左右10%（旧 v1 默认 25%）；v3 = 上吸附默认改为 0（禁用）。
// 旧版本配置整档覆盖为新默认。
var SNAP_VER = 3
var SNAP_DEFAULTS = {
  mode: 'ratio',
  ratio: { L: 10, T: 0, R: 10, B: 15, F: 50 },
  px: { L: 80, T: 0, R: 80, B: 80, F: -1 }
}
var snapConfig = null
function cloneSnap(c) {
  return {
    mode: c.mode,
    ratio: { L: c.ratio.L, T: c.ratio.T, R: c.ratio.R, B: c.ratio.B, F: c.ratio.F },
    px: { L: c.px.L, T: c.px.T, R: c.px.R, B: c.px.B, F: c.px.F }
  }
}
// 单键钳制到合法范围（与另一条线/翻转线的约束），val 为目标值
function clampSnapKey(mode, key, val, vp) {
  try {
    if (mode === 'px') {
      var w = Math.max(1, vp.w), h = Math.max(1, vp.h)
      var p = snapEdit ? snapEdit.px : snapConfig.px
      if (key === 'L') return Math.max(0, Math.min(val, p.F >= 0 ? p.F : w / 2))
      if (key === 'R') return Math.max(0, Math.min(val, Math.max(0, w - (p.F >= 0 ? p.F : w / 2))))
      if (key === 'T') return Math.max(0, Math.min(val, Math.max(0, h - p.B)))
      if (key === 'B') return Math.max(0, Math.min(val, Math.max(0, h - p.T)))
      if (key === 'F') return Math.max(p.L, Math.min(val, Math.max(p.L, w - p.R)))
    } else {
      var r = snapEdit ? snapEdit.ratio : snapConfig.ratio
      if (key === 'L') return Math.max(0, Math.min(val, r.F))
      if (key === 'R') return Math.max(0, Math.min(val, 100 - r.F))
      if (key === 'T') return Math.max(0, Math.min(val, 100 - r.B))
      if (key === 'B') return Math.max(0, Math.min(val, 100 - r.T))
      if (key === 'F') return Math.max(r.L, Math.min(val, 100 - r.R))
    }
  } catch (err) {}
  return val
}
// 整体修复（加载/确认时调用）：把越界/非法值拉回合法范围；mode 可指定修复哪一档
function fixSnapConfig(cfg, mode) {
  try {
    var m = mode || cfg.mode
    if (m === 'off') return
    var vp = viewport()
    if (m === 'px') {
      var w = Math.max(1, vp.w), h = Math.max(1, vp.h)
      if (!(cfg.px.F >= 0)) { cfg.px.F = Math.round(w / 2); if (!(cfg.px.L > 0)) cfg.px.L = 80; if (!(cfg.px.R > 0)) cfg.px.R = 80; cfg.px.B = Math.max(0, cfg.px.B > 0 ? cfg.px.B : 80); cfg.px.T = Math.max(0, Math.min(cfg.px.T, Math.max(0, h - cfg.px.B))) }      cfg.px.L = Math.max(0, Math.min(cfg.px.L, cfg.px.F))
      cfg.px.R = Math.max(0, Math.min(cfg.px.R, Math.max(0, w - cfg.px.F)))
      cfg.px.F = Math.max(cfg.px.L, Math.min(cfg.px.F, Math.max(cfg.px.L, w - cfg.px.R)))
      cfg.px.L = Math.max(0, Math.min(cfg.px.L, cfg.px.F))
      cfg.px.R = Math.max(0, Math.min(cfg.px.R, Math.max(0, w - cfg.px.F)))
      cfg.px.T = Math.max(0, Math.min(cfg.px.T, Math.max(0, h - cfg.px.B)))
      cfg.px.B = Math.max(0, Math.min(cfg.px.B, Math.max(0, h - cfg.px.T)))
      cfg.px.T = Math.max(0, Math.min(cfg.px.T, Math.max(0, h - cfg.px.B)))
    } else {
      cfg.ratio.L = Math.max(0, Math.min(cfg.ratio.L, cfg.ratio.F))
      cfg.ratio.R = Math.max(0, Math.min(cfg.ratio.R, 100 - cfg.ratio.F))
      cfg.ratio.F = Math.max(cfg.ratio.L, Math.min(cfg.ratio.F, 100 - cfg.ratio.R))
      cfg.ratio.L = Math.max(0, Math.min(cfg.ratio.L, cfg.ratio.F))
      cfg.ratio.R = Math.max(0, Math.min(cfg.ratio.R, 100 - cfg.ratio.F))
      cfg.ratio.T = Math.max(0, Math.min(cfg.ratio.T, 100 - cfg.ratio.B))
      cfg.ratio.B = Math.max(0, Math.min(cfg.ratio.B, 100 - cfg.ratio.T))
      cfg.ratio.T = Math.max(0, Math.min(cfg.ratio.T, 100 - cfg.ratio.B))
    }
  } catch (err) {}
}
function loadSnapConfig() {
  snapConfig = cloneSnap(SNAP_DEFAULTS)
  try {
    var raw = localStorage.getItem(SNAP_KEY)
    if (raw) {
      var d = JSON.parse(raw)
      if (d && d.v === SNAP_VER) {
        // 仅认当前版本：旧版（含无版本）配置整档覆盖为新默认（默认吸附区已改为 上下15/左右10）
        if (d.mode === 'ratio' || d.mode === 'px' || d.mode === 'off') snapConfig.mode = d.mode
        var keys = ['L', 'T', 'R', 'B', 'F']
        var i, k
        if (d.ratio) for (i = 0; i < keys.length; i++) { k = keys[i]; if (typeof d.ratio[k] === 'number' && isFinite(d.ratio[k])) snapConfig.ratio[k] = d.ratio[k] }
        if (d.px) for (i = 0; i < keys.length; i++) { k = keys[i]; if (typeof d.px[k] === 'number' && isFinite(d.px[k])) snapConfig.px[k] = d.px[k] }
      }
    }
  } catch (err) {}
  fixSnapConfig(snapConfig, 'ratio')
  fixSnapConfig(snapConfig, 'px')
}
function saveSnapConfig() {
  try {
    var out = cloneSnap(snapConfig)
    out.v = SNAP_VER
    localStorage.setItem(SNAP_KEY, JSON.stringify(out))
  } catch (err) {}
}
loadSnapConfig()

var busy = false
var settleTimer = null
var animDelayTimer = null
var drag = null
var shown = null
var animId = null
var bubbleShown = false
var bubbleTimer = null
var bubbleRandomActive = false
var bubbleRandomLines = null
var BUBBLE_STYLE_CLASS = { A: 'dshwv-label', B: 'dshwv-amount', P: 'dshwv-period', C: 'dshwv-hint' }
function pickOne(arr) { return arr[Math.floor(Math.random() * arr.length)] }
function singleCenter(style, text, color, wrap) { return [null, { t: text, s: style, c: color || '', w: !!wrap }, null] }
var RANDOM_GROUPS = [
  { w: 7, lines: function () { return singleCenter('B', pickOne(['好模型... ↓', '好女孩...↓'])) } },
  { w: 7, lines: function () { return singleCenter('A', pickOne(['不知道用户有什么用，先赶走吧~', '我...我...我也要挣钱吗？', '我去吃饭啦，测完叫我', '压力一只蓝色大肥鱼？！', 'DeepSleep...', '坏了...用户彻底怒了！']), '', true) } },
  { w: 10, lines: function () { return { gif: true } } },
  { w: 3, lines: function () { return singleCenter('A', pickOne(['你目录里的dsh是什么...大烧货吗...?', '恭喜你实现token自由！token全跑了！', '真当我是便宜货啊...']), '', true) } },
  { w: 1, lines: function () { return singleCenter('B', '哦鲸鲸... ') } },
]
function pickRandomLines() {
  var total = 0
  for (var i = 0; i < RANDOM_GROUPS.length; i++) total += RANDOM_GROUPS[i].w
  var r = Math.random() * total
  for (var i = 0; i < RANDOM_GROUPS.length; i++) {
    r -= RANDOM_GROUPS[i].w
    if (r < 0) return RANDOM_GROUPS[i].lines()
  }
  return RANDOM_GROUPS[RANDOM_GROUPS.length - 1].lines()
}
function applyBubbleLines(lines) {
  if (lines && lines.gif) {
    // gif 台词组：只显示 gif，隐藏三行文字（display 必须显式覆盖 CSS 的 none）
    if (gifFailed) {
      // gif 加载失败/路由缺失：降级为文字台词，避免空白白色气泡
      lines = singleCenter('A', pickOne(['gif 加载失败了...', '今天没有动图给你看~', '呜呜 动图不见了...']), '', true)
    } else {
      if (gifFadeTimer) { clearTimeout(gifFadeTimer); gifFadeTimer = null }
      gifEl.style.display = 'block'
      gifEl.style.opacity = ''
      labelEl.style.display = 'none'
      amountEl.style.display = 'none'
      hintEl.style.display = 'none'
      return
    }
  }
  if (gifFadeTimer) { clearTimeout(gifFadeTimer); gifFadeTimer = null }
  gifEl.style.display = 'none'
  gifEl.style.opacity = ''
  var els = [labelEl, amountEl, hintEl]
  for (var i = 0; i < 3; i++) {
    var el = els[i]
    var ln = lines && lines[i]
    if (ln) {
      el.style.display = ''
      el.className = (BUBBLE_STYLE_CLASS[ln.s] || 'dshwv-label') + (ln.w ? ' dshwv-wrap' : '')
      el.textContent = ln.t
      el.style.color = ln.c || ''
    } else {
      el.style.display = 'none'
      el.textContent = ''
      el.style.color = ''
    }
  }
}
var bubbleSwapTimer = null
var hintFadeTimer = null
var gifFadeTimer = null
var lastHintText = null
function setHint(text) {
  // 首次/恢复（lastHintText===null）时直接写文本，不做淡出淡入——否则
  // 气泡打开或按压重开时会先淡出再淡入，造成「消失一下又出现」。
  // 只有气泡打开期间的内容变化（加载中→今日已用）才走动画。
  if (text === lastHintText) return
  var first = lastHintText === null
  lastHintText = text
  if (first || !bubbleShown) {
    hintEl.textContent = text
    return
  }
  hintEl.style.transition = 'opacity .18s ease'
  hintEl.style.opacity = '0'
  hintFadeTimer = setTimeout(function () {
    hintFadeTimer = null
    hintEl.textContent = text
    hintEl.style.opacity = '1'
    setTimeout(function () {
      hintEl.style.transition = ''
      hintEl.style.opacity = ''
    }, 220)
  }, 190)
}
function swapBubbleContent(applyFn) {
  if (bubbleSwapTimer) { clearTimeout(bubbleSwapTimer); bubbleSwapTimer = null }
  textBox.style.transition = 'opacity .18s ease'
  textBox.style.opacity = '0'
  bubbleSwapTimer = setTimeout(function () {
    bubbleSwapTimer = null
    applyFn()
    textBox.style.opacity = '1'
    setTimeout(function () {
      textBox.style.transition = ''
      textBox.style.opacity = ''
    }, 220)
  }, 190)
}
function restoreBubbleLines() {
  if (bubbleSwapTimer) { clearTimeout(bubbleSwapTimer); bubbleSwapTimer = null }
  if (hintFadeTimer) { clearTimeout(hintFadeTimer); hintFadeTimer = null }
  if (gifFadeTimer) { clearTimeout(gifFadeTimer); gifFadeTimer = null }
  lastHintText = null
  textBox.style.transition = ''
  textBox.style.opacity = ''
  gifEl.style.display = 'none'
  gifEl.style.opacity = ''
  // 必须重置三行文字的 opacity/transition：若上次 setHint 淡出动画被中断
  // （restoreBubbleLines 清了 hintFadeTimer），opacity 会卡在 '0'，
  // 后续 setHint 首次直写分支不碰 opacity → 今日已用整行透明消失。
  labelEl.style.display = ''
  labelEl.className = 'dshwv-label'
  labelEl.textContent = 'Codex 用量'
  labelEl.style.color = ''
  labelEl.style.opacity = ''
  amountEl.style.display = ''
  amountEl.className = 'dshwv-amount'
  amountEl.style.color = ''
  amountEl.style.opacity = ''
  hintEl.style.display = ''
  hintEl.className = 'dshwv-hint'
  hintEl.style.color = ''
  hintEl.style.opacity = ''
  render()
}

// ===== 泡泡统一核心 + 手动点击序列(新;覆盖旧 showBubble/hideBubble/showCostBubble/hideCostBubble) =====
// 原则:一切泡泡内容都是“场景(scene)”,经 sceneOpen 统一调度(单 TTL、统一清场),
// 消灭分散 flag/timer 的竞态。手动点击 = 序列推进(默认序列=老体验两项:
// 第1项 余额默认内容, 第2项 随机台词段; 再点关闭); 事件泡泡(余额变化=showBubble、
// 消耗=showCostBubble)独立触发, 消耗优先级最高可顶掉当前并暂停手动轮。
var costBubbleTimer = null // 旧版计时器保留声明(新版 bubbleClearAll 仍清理)
var bubbleTtlTimer = null
// v789（issue #188/#172）：泡泡的自动收起**不能只靠 setTimeout** —— 页面被遮挡 / 最小化 / 切到后台时，
// 浏览器会把定时器节流甚至挂起，于是"5 秒后收起"变成几十秒甚至永远不收（用户实测：点击泡一直不消失、
// 刷新即恢复）。所以同时记一个**截止时刻**，在页面重新可见 / 拿到焦点时结算一次；再配一个每秒巡检兜底。
var bubbleTtlDeadline = 0
function bubbleTtlArmed(ttlMs) {
  try { if (ttlMs > 0) bubbleTtlDeadline = Date.now() + ttlMs; else bubbleTtlDeadline = 0 } catch (err) { bubbleTtlDeadline = 0 }
}
function bubbleTtlClear() { bubbleTtlDeadline = 0 }
// 巡检：只在"本该已经到期"且计时器已经不在了（被节流/挂起）时补收一次
function bubbleTtlSweep() {
  try {
    if (bubbleTtlTimer || !bubbleTtlDeadline) return
    if (!bubbleScene || !(bubbleScene.ttlMs > 0)) { bubbleTtlDeadline = 0; return }
    if (Date.now() < bubbleTtlDeadline) return
    bubbleTtlDeadline = 0
    bubbleAutoClose()
  } catch (err) {}
}
// 三条触发路径：页面重新可见（切标签回来）、窗口重新拿到焦点（最小化还原）、以及每秒一次的兜底巡检。
// 正常前台运行时计时器按时收敛，巡检永远提前返回（不会误收下一个泡泡）。
try {
  document.addEventListener('visibilitychange', function () { try { if (!document.hidden) bubbleTtlSweep() } catch (err) {} })
  window.addEventListener('focus', function () { bubbleTtlSweep() })
  window.addEventListener('pageshow', function () { bubbleTtlSweep() })
  setInterval(bubbleTtlSweep, 1000)
} catch (err) {}
var bubbleScene = null // { kind:'normal'|'random'|'cost', ttlMs }
var bubbleSeq = bubbleDefaultQueue() // 默认序列 = 与开发者线上生效一致(首次=余额,再次=随机语句);有配置后由 applyBubbleCfgSeq 覆盖
var bubbleSeqIdx = 0 // 下一项下标(手动轮内推进)
var bubbleRoundOn = false // 手动轮是否进行中(被消耗顶掉时保留, 结束后归零)
var bubbleCfg = null // 服务端自定义配置 {v:1, items:[...], lib:[...], tapAdvance:bool}; null=未配置
var bubbleLib = [] // 可选模块库(当前会话编辑用,随配置保存):[{id,name,module}]
// v727「点按角色推进泡泡队列」（仅「自定义泡泡」菜单可设，存在泡泡配置里）：
//   关闭（默认）= 点鲸鱼时，正在看第 2 项及以后 → 回到第 1 项（旧行为）
//   开启       = 点鲸鱼 = 往后推进一项；已是最后一项 → 收起泡泡（与「点泡泡」一致），下次点按从第 1 项开始
var bubbleTapAdvance = false
function bubbleCloneModule(m) {
  return JSON.parse(JSON.stringify(m || {}))
}
function bubbleLibAdd(name, module) {
  try {
    if (!module) return null
    var id = 'bmod_' + Date.now().toString(36) + '_' + Math.random().toString(36).slice(2, 6)
    var it = { id: id, name: String(name || '').trim().slice(0, 20) || '模块' + (bubbleLib.length + 1), module: bubbleCloneModule(module) }
    bubbleLib.push(it)
    return it
  } catch (err) { return null }
}
function bubbleLibDel(id) {
  bubbleLib = bubbleLib.filter(function (x) { return x.id !== id })
}
// 用配置的 items 覆盖运行序列(kind: normal|random|custom(modules))
function applyBubbleCfgSeq() {
  try {
    if (!bubbleCfg || !Array.isArray(bubbleCfg.items) || !bubbleCfg.items.length) return
    var seq = []
    for (var i = 0; i < bubbleCfg.items.length; i++) {
      var it = bubbleCfg.items[i]
      if (it && it.kind === 'choice' && Array.isArray(it.options)) {
        var opts = []
        for (var ci = 0; ci < it.options.length && ci < 2; ci++) {
          var co = it.options[ci] || {}
          var cit = co.item || {}
          var citem = null
          if (cit.kind === 'custom' && Array.isArray(cit.modules)) citem = { kind: 'custom', modules: cit.modules }
          else if (cit.kind === 'random') citem = { kind: 'random' }
          else citem = { kind: 'normal' }
          opts.push({ w: bubbleChoiceWeight(co), item: citem })
        }
        if (opts.length) { seq.push({ kind: 'choice', options: opts }); continue }
      }
      if (it && it.kind === 'custom' && Array.isArray(it.modules)) seq.push({ kind: 'custom', modules: it.modules })
      else if (it && it.kind === 'random') seq.push({ kind: 'random' })
      else seq.push({ kind: 'normal' })
    }
    if (seq.length) bubbleSeq = seq
  } catch (err) {}
}
function loadBubbleCfg() {
  try {
    fetch(BUBBLE_URL, { cache: 'no-store' })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (d && d.ok && d.config) {
          bubbleCfg = d.config
          bubbleLib = (d.config.lib && Array.isArray(d.config.lib)) ? JSON.parse(JSON.stringify(d.config.lib)) : []
          bubbleTapAdvance = d.config.tapAdvance === true // v727
          applyBubbleCfgSeq()
          maybeBubbleMigratePeak()
        }
      })
      .catch(function () {})
  } catch (err) {}
}
function saveBubbleCfg(cfg, okFn) {
  try {
    fetch(BUBBLE_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(cfg),
    })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (d && d.ok && d.config) {
          bubbleCfg = d.config
          applyBubbleCfgSeq()
          if (okFn) okFn()
        } else if (okFn) okFn(false)
      })
      .catch(function () { if (okFn) okFn(false) })
  } catch (err) { if (okFn) okFn(false) }
}
// —— 峰谷模块归一化:旧「时段倒计时」模块(nextpeak)→ 峰谷模块倒计时样式;
//    旧全局峰谷称呼(梁文峰谷/!?强强?!)一次性写入尚未设置的峰谷模块 ——
function bubbleEachPeakMod(cfg, fn) {
  try {
    if (!cfg || !Array.isArray(cfg.items)) return
    function walk(step) {
      if (!step || typeof step !== 'object') return
      if (Array.isArray(step.modules)) {
        for (var i = 0; i < step.modules.length; i++) {
          var md = step.modules[i]
          if (md && (md.type === 'peak' || md.type === 'nextpeak')) fn(md)
        }
      }
      if (step.kind === 'choice' && Array.isArray(step.options)) {
        for (var j = 0; j < step.options.length; j++) {
          var o = step.options[j]
          walk(o && o.item)
        }
      }
    }
    for (var k = 0; k < cfg.items.length; k++) walk(cfg.items[k])
  } catch (err) {}
}
function maybeBubbleMigratePeak() {
  try {
    var legacy = window.__dshwLegacyPeak
    if (!legacy && !bubbleCfg) return
    if (!bubbleCfg || !Array.isArray(bubbleCfg.items)) return
    if (!legacy && !hasNextPeakMod(bubbleCfg)) return
    window.__dshwLegacyPeak = null
    var changed = false
    bubbleEachPeakMod(bubbleCfg, function (md) {
      if (md.type === 'nextpeak') {
        // 旧倒计时模块并入峰谷模块
        md.type = 'peak'
        md.peakStyle = 'count'
        changed = true
        return
      }
      if (!md.peakStyle) {
        md.peakStyle = legacy || 'default'
        changed = true
      }
    })
    if (changed) saveBubbleCfg(bubbleCfg)
  } catch (err) {}
}
function hasNextPeakMod(cfg) {
  var found = false
  bubbleEachPeakMod(cfg, function (md) { if (md.type === 'nextpeak') found = true })
  return found
}
// ===== F2「一行多模块」数据层(§7 第 1 步):行分组表示 =====
// 行分组标记(向后兼容旧配置):
// - bubble 的 modules[] 维持平铺、按自上而下显示顺序排列;可给任意模块加正整数 `row`(行号标记)。
// - 旧配置(模块无 row / row 非正整数)→ 每模块独占一行,与旧版本行为完全一致。
// - 行合并规则:仅当「相邻」两模块都带同一个正整数 row 时并入同一行;
//   其余情况(row 不同 / 某侧无 row / 不相邻)一律另起一行 —— 无 row 的旧数据绝不会被误合并。
// - image 模块不受 row 影响,永远单独占一整行(行数/每行模块数上限由渲染层控制)。
// - 配置加载/保存/应用全程对模块对象直传(见 loadBubbleCfg / saveBubbleCfg / applyBubbleCfgSeq),
//   本层只提供「读行标记 + 平铺→行」分解;渲染层(bubbleRowsTo)与编辑器(F2 第 3 步)共用。
// 行标记读取:只认正整数(字符串/布尔等一律视为无标记,防脏值误合并)
function bubbleRowKeyOf(m) {
  try {
    m = m || {}
    var n = m.row
    if (typeof n === 'number' && isFinite(n) && Math.round(n) === n && n > 0) return n
  } catch (err) {}
  return null
}
// 平铺 modules[] → 视觉行数组 [[m…], [m…]](保持原模块顺序;图片各占一行)
// 图片类模块(图片/动图 与 随机图片):独占一整行,且一个泡泡只允许一个
function bubbleIsImgMod(m) { return !!m && (m.type === 'image' || m.type === 'randimg') }
function bubbleRowsOf(mods) {
  var out = []
  if (!Array.isArray(mods)) return out
  var cur = null
  for (var i = 0; i < mods.length; i++) {
    var m = mods[i] || {}
    if (bubbleIsImgMod(m)) {
      // 图片单独占一整行,并打断与前后模块的行合并
      out.push([m])
      cur = null
      continue
    }
    if (cur && cur.key !== null && bubbleRowKeyOf(m) === cur.key) {
      cur.row.push(m)
      continue
    }
    cur = { key: bubbleRowKeyOf(m), row: [m] }
    out.push(cur.row)
  }
  return out
}
// F2 行模型→平铺:把「行数组(模块引用)」规范写回平铺 modules[](同行的模块写同行键;单模块行/图片行不写键)
function bubbleRowsFlat(rows) {
  var flat = []
  try {
    if (!Array.isArray(rows)) return flat
    for (var r = 0; r < rows.length; r++) {
      var row = rows[r]
      if (!row || !row.length) continue
      var multi = row.length > 1
      for (var i = 0; i < row.length; i++) {
        var m = row[i]
        if (!m || typeof m !== 'object') continue
        if (multi) m.row = r + 1
        else try { delete m.row } catch (err) {}
        flat.push(m)
      }
    }
  } catch (err) {}
  return flat
}
// 就地把 modules[] 规范化:以当前相邻分组为准,重写/清除行键(旧数据=每模块一行 → 全部无键)
function bubbleRowsCanon(mods) {
  try {
    if (!Array.isArray(mods)) return
    var flat = bubbleRowsFlat(bubbleRowsOf(mods))
    mods.length = 0
    for (var i = 0; i < flat.length; i++) mods.push(flat[i])
  } catch (err) {}
}
function bubbleClearAll() {
  try { if (bubbleTtlTimer) { clearTimeout(bubbleTtlTimer); bubbleTtlTimer = null } } catch (err) {}
  try { if (bubbleTimer) { clearTimeout(bubbleTimer); bubbleTimer = null } } catch (err) {}
  try { if (bubbleSwapTimer) { clearTimeout(bubbleSwapTimer); bubbleSwapTimer = null } } catch (err) {}
  try { if (hintFadeTimer) { clearTimeout(hintFadeTimer); hintFadeTimer = null } } catch (err) {}
  try { if (gifFadeTimer) { clearTimeout(gifFadeTimer); gifFadeTimer = null } } catch (err) {}
  try { if (costBubbleTimer) { clearTimeout(costBubbleTimer); costBubbleTimer = null } } catch (err) {}
  try { if (animId) { cancelAnimationFrame(animId); animId = null } } catch (err) {}
  try { if (animDelayTimer) { clearTimeout(animDelayTimer); animDelayTimer = null } } catch (err) {}
  try { if (settleTimer) { clearTimeout(settleTimer); settleTimer = null } } catch (err) {}
}
function bubbleCloseVisual() {
  // v789（issue #188）：视觉收起是"泡泡真的收掉了"的唯一出口 ⇒ 顺带把截止时刻清零（三处对齐之二）
  bubbleTtlClear()
  // v771：泡泡一旦真的收起，等待标记必须一起复位。`waitShown` 平时由 bubbleRenderModules 按场景重算，
  // 但"整泡关闭"（hideBubble/hideCostBubble/hideWaitBubble…）不走渲染 ⇒ 不复位就会残留成 true，
  // 之后 showWaitBubble 会误判"已经在显示等待泡泡"（历史上正是这类不同步把等待泡泡卡死的）。
  waitShown = false
  try { bubbleBox.classList.remove('dshwv-pop-open') } catch (err) {}
  try { textBox.style.transition = ''; textBox.style.opacity = '' } catch (err) {}
  try { hintEl.style.transition = ''; hintEl.style.opacity = '' } catch (err) {}
  // gif 靠 CSS opacity 淡出;display:none 会跳过过渡,须等淡出完成再隐藏
  try { if (gifFadeTimer) { clearTimeout(gifFadeTimer); gifFadeTimer = null } } catch (err) {}
  gifFadeTimer = setTimeout(function () {
    gifFadeTimer = null
    try { gifEl.style.display = 'none' } catch (err) {}
  }, 240)
}
// 打开场景:统一清场 → 重置内容层 → 渲染 → 开框 → 布 TTL
// 清掉 textBox 里遗留的模块行(自定义场景切走/关闭时,防止叠到默认/消耗内容上)
function bubbleClearModuleRows() {
  try {
    var els = textBox.querySelectorAll('.dshwv-trow, .dshwv-mimg')
    for (var i = 0; i < els.length; i++) { try { textBox.removeChild(els[i]) } catch (err) {} }
  } catch (err) {}
}
function sceneOpen(kind, renderFn, ttlMs) {
  var wasOpen = bubbleShown
  bubbleClearAll()
  bubbleClearModuleRows()
  try { gifEl.style.display = 'none'; gifEl.style.opacity = '' } catch (err) {}
  try { textBox.style.transition = ''; textBox.style.opacity = '' } catch (err) {}
  bubbleScene = { kind: kind, ttlMs: ttlMs || 0 }
  bubbleShown = true
  costBubbleActive = (kind === 'cost')
  bubbleRandomActive = (kind === 'random')
  lastHintText = null // 内容整体重置,提示行走“首次直写”,避免半途淡出残留
  function finish() {
    try { renderFn() } catch (err) {}
    try { bubbleBox.classList.add('dshwv-pop-open') } catch (err) {}
    // 内容替换(泡泡已开着)时淡入新文字;首次打开不加内联透明度,
    // 文字显隐交给 CSS(.dshwv-pop-open 才显示,带 .36s 延时跟随泡泡成形)
    if (wasOpen) {
      try {
        textBox.style.transition = 'opacity .16s ease'
        textBox.style.opacity = '1'
        bubbleSwapTimer = setTimeout(function () {
          bubbleSwapTimer = null
          try { textBox.style.transition = ''; textBox.style.opacity = '' } catch (err) {}
        }, 180)
      } catch (err) {}
    }
    if (ttlMs > 0) { bubbleTtlArmed(ttlMs); bubbleTtlTimer = setTimeout(bubbleAutoClose, ttlMs) }
    else bubbleTtlClear()
  }
  if (wasOpen) {
    // 内容切换:旧内容先淡出,再换新内容并淡入(可被新场景/关闭随时打断)
    try {
      textBox.style.transition = 'opacity .12s ease'
      textBox.style.opacity = '0'
      bubbleSwapTimer = setTimeout(function () {
        bubbleSwapTimer = null
        finish()
      }, 120)
    } catch (err) {
      finish()
    }
  } else {
    finish()
  }
}
function bubbleAutoClose() {
  bubbleTtlTimer = null
  // v789：走完这条就说明本次 TTL 已经结算，截止时刻必须清零，否则巡检会误收下一个泡泡
  bubbleTtlClear()
  if (bubbleScene && bubbleScene.kind === 'cost') { if (whaleSysSwapNext()) return; hideCostBubble(); return }
  if (bubbleScene && bubbleScene.kind === 'alert') { if (whaleSysSwapNext()) return; hideUsageAlertBubble(); return }
  // v761（#161 C5）：等待交互泡泡是**常驻**的（sceneOpen 收到 ttl 0 不布计时器），正常永远走不到这里；
  // 这一支是防御性的（例如用户先点开等待泡泡再改设置导致 ttl 被重新布上），语义仍与其他系统泡泡一致。
  if (bubbleScene && bubbleScene.kind === 'wait') { if (whaleSysSwapNext()) return; hideWaitBubble(); return }
  hideBubble()
}
// 重置当前泡泡的留存计时(点鲸鱼给第 1 泡续时,不清内容)
function bubbleResetTtl() {
  try { if (bubbleTtlTimer) { clearTimeout(bubbleTtlTimer); bubbleTtlTimer = null } } catch (err) {}
  // v789：续时也要同步截止时刻，否则巡检会在"刚续过"的泡泡上提前动作（issue #188 提醒的三处对齐之一）
  if (bubbleScene && bubbleScene.ttlMs > 0) { bubbleTtlArmed(bubbleScene.ttlMs); bubbleTtlTimer = setTimeout(bubbleAutoClose, bubbleScene.ttlMs) }
  else bubbleTtlClear()
}
// 默认内容视图 = 现在待机内容(余额/今日已用),由 render/restore 维护
function bubbleRenderDefault() { restoreBubbleLines() }
// 随机台词段:每次展示按权重重抽(老 RANDOM_GROUPS),含 gif 槽与失败降级
function bubbleRenderRandom(lines) {
  if (lines && lines.gif) {
    if (gifFailed) {
      lines = singleCenter('A', pickOne(['gif 加载失败了...', '今天没有动图给你看~', '呜呜 动图不见了...']), '', true)
    } else {
      try { gifEl.style.display = 'block'; gifEl.style.opacity = '' } catch (err) {}
      labelEl.style.display = 'none'
      amountEl.style.display = 'none'
      hintEl.style.display = 'none'
      return
    }
  }
  applyBubbleLines(lines)
}
// 每轮消耗提示(v720):内容改为可编辑的模块列表,与余额预警/预算同一条渲染链路。
// 旧版是写死的三段(label/amount/hint),现在的默认内容(usageTurnCostDefaultLines)复刻它的观感。
function bubbleRenderCostMods(amount) {
  bubbleRenderModules(codexWhaleHost.completionModules(usageAlertModsResolved(usageTurnCostLines(), null, null, codexWhaleHost.completionCost(amount))))
}
// 等待交互提示（v761 / #161 C5）：内容取 events.<kind>.lines（缺失/空 → 默认模板），
// 走与预警/预算/消耗同一条模块渲染链路。`{session}` 在这里就替换成当前对话名
// （usageAlertModsResolved → usageFillText 的会话名回落），所以真实泡泡显示的是对话名而不是 `{session}`。
function bubbleRenderWaitMods(kind) {
  var k = (kind === 'approval') ? 'approval' : 'question'
  var cfg = {}
  try { cfg = ((usageSet || {}).events || {})[k] || {} } catch (err) { cfg = {} }
  bubbleRenderModules(usageAlertModsResolved(usageWaitLinesOf(cfg, k), null, null, null))
}
// 并列步骤:每轮到这一步时独立按权重抽一个候选泡(不记忆上次,允许连续几轮同泡)
function bubblePickChoiceStep(step) {
  var opts = bubbleChoiceOptions(step)
  if (!opts.length) return null
  var total = 0
  for (var i = 0; i < opts.length; i++) total += bubbleChoiceWeight(opts[i])
  var r = Math.random() * total
  var acc = 0
  for (var j = 0; j < opts.length; j++) {
    acc += bubbleChoiceWeight(opts[j])
    if (r < acc) return (opts[j] && opts[j].item) || null
  }
  var last = opts[opts.length - 1]
  return (last && last.item) || null
}
function bubbleShowSeqNext() {
  var step = bubbleSeq[bubbleSeqIdx]
  if (!step) { hideBubble(); return }
  bubbleSeqIdx++
  var item = bubbleIsChoice(step) ? bubblePickChoiceStep(step) : step
  if (!item) { hideBubble(); return }
  if (item.kind === 'random') {
    var lines = pickRandomLines()
    bubbleRandomLines = lines
    sceneOpen('random', function () { bubbleRenderRandom(lines) }, BUBBLE_MS)
  } else if (item.kind === 'custom') {
    sceneOpen('custom', function () { bubbleRenderModules(item.modules || []) }, BUBBLE_MS)
  } else {
    sceneOpen('normal', bubbleRenderDefault, BUBBLE_MS)
  }
}
// ===== 模块渲染引擎(B1) =====
// 模块:{type:'text'|'balance'|'bonus'|'recharge'|'today'|'peak'|'session'|'image'|'random', text?, imgId?, color?, size?(1..8 档),
//        lines?:[{t,w}] (random 自带句子列表), tpl?(占位符模板), len?(session 的「保留长度」，0=不截断)}
//   v782 新增 'bonus'（赠金）与 'recharge'（充值余额）：与 'balance' 并列的内置数值模块，
//   分别用占位符 {bonus_ds} / {recharge_ds}；**'balance' / '{balance_ds}' 的名字与行为一律不动**
//   （历史泡泡里存的是这些字面量，改名会让老内容显示不出来）。
// 字号档位 1..50,线性细分(1→40u … 50→240u,相对 --dshw-u 的倍数)
function bubbleModuleFontU(level) {
  var n = Number(level) || 6
  n = Math.max(1, Math.min(50, Math.round(n)))
  return Math.round(40 + (n - 1) * 200 / 49)
}
function bubbleAmountText() {
  var v = shown !== null ? shown : (state.balance !== null ? state.balance : null)
  if (v === null) return '…'
  return fmt(v, state.currency)
}
function bubbleTodayText() {
  return (state.usageLabel || '今日已用') + ' ' + (state.todayUsage !== null && state.todayUsage !== undefined ? fmt(state.todayUsage, state.todayUsageCurrency || state.currency) : '--')
}
// v782：赠金 / 充值余额两个数值模块的取数（与「总余额(充值金额+赠金)」同源：宿主余额返回体）。
//   ⚠️ 两个数字**可能拿不到**（厂商没有该字段、或未登录账号）⇒ 显示 `—`，**不显示 0**
//   —— 0 的意思是"确实没有了"，与"读不到"是两件事（记账也依赖这个区分）。
function bubbleBonusText() {
  var v = state.bonusBalance
  if (v === null || v === undefined || !isFinite(Number(v))) return '—'
  return fmt(Number(v), state.currency)
}
function bubbleRechargeText() {
  var v = state.rechargeBalance
  if (v === null || v === undefined || !isFinite(Number(v))) return '—'
  return fmt(Number(v), state.currency)
}
// 模块「内容」模板:占位符统一英文(便于兼容其他模型 API 时区分来源/字段):
//   {expense_ds} 今日已用金额 · {balance_ds} 余额 · {status} 高峰/空闲状态字 · {countdown} 倒计时
// —— 自定义 API 模型的余额/今日已用（Phase 3）——
function apiModelBalanceInfo(modelId) {
  var m = apiModelById(modelId)
  if (!m) return null
  return { name: m.name, balance: m.balance, today: m.todayUsage, currency: m.currency, todayCurrency: apiTodayCur(m), mode: m.balanceMode || (m.hasBalanceApi === false ? 'events' : 'api') }
}
function apiModelBalanceText(modelId) {
  var i = apiModelBalanceInfo(modelId)
  if (!i) return '--'
  if (i.balance === null || i.balance === undefined) return '—'
  return apiFmtMoney(i.balance, i.currency)
}
function apiModelTodayText(modelId) {
  if (modelId === 'codex') return codexWhaleHost.tokens('today') + ' tokens'
  var i = apiModelBalanceInfo(modelId)
  if (!i) return '--'
  // 今日已用按它自己的币种显示（会话事件金额＝CNY，余额差＝厂商币种）
  return apiFmtMoney(i.today, i.todayCurrency || i.currency)
}
// —— 手动额度（订阅 / 资源包：厂商没有额度接口，总量与已用由用户在面板里填）——
function apiQuotaOf(modelId) {
  var m = apiModelById(modelId)
  var q = (m && m.quota) || null
  if (!q && usageSet && usageSet.models && usageSet.models[modelId]) q = usageSet.models[modelId].quota || null
  return q || null
}
function apiQuotaInfo(modelId) {
  var q = apiQuotaOf(modelId)
  if (!q) return null
  var total = Number(q.total) || 0
  // 自动模式：已用由 host 按会话 token 统计（q.autoUsed）；Codex 模式：按 Codex 本地会话 token；手动模式：面板里填的 q.used
  var isAuto = (q.mode === 'codex') || ((q.mode !== 'manual') && (q.mode === 'auto' || q.autoUsed !== undefined))
  var rawUsed = isAuto ? q.autoUsed : q.used
  var used = rawUsed == null || rawUsed === '' ? null : Math.max(0, Number(rawUsed) || 0)
  if (!total && !used) return null
  var left = used == null ? null : Math.max(0, total - used)
  return {
    total: total, used: used, left: left,
    pct: used == null ? null : (total > 0 ? Math.min(100, used / total * 100) : 0),
    unit: q.unit === 'money' ? 'money' : 'tokens',
    reset: q.reset || 'none',
    mode: q.mode === 'codex' ? 'codex' : (isAuto ? 'auto' : 'manual'),
  }
}
function apiFmtQuotaNum(n) {
  if (n === null || n === undefined || n === '') return '—'
  n = Number(n) || 0
  if (n >= 100000000) return (n / 100000000).toFixed(2).replace(/\.?0+$/, '') + '亿'
  if (n >= 10000) return (n / 10000).toFixed(n >= 1000000 ? 0 : 1).replace(/\.0$/, '') + '万'
  return String(Math.round(n))
}
function apiQuotaUnitSuffix(i) { return i && i.unit === 'money' ? ' 元' : (i ? ' tokens' : '') }
function apiQuotaUsedText(modelId) {
  var i = apiQuotaInfo(modelId)
  if (!i) return '--'
  return (i.unit === 'money' ? '¥' : '') + apiFmtQuotaNum(i.used) + (i.unit === 'money' ? '' : '')
}
function apiQuotaLeftText(modelId) {
  var i = apiQuotaInfo(modelId)
  if (!i) return '--'
  return (i.unit === 'money' ? '¥' : '') + apiFmtQuotaNum(i.left) + ''
}
function apiQuotaTotalText(modelId) {
  var i = apiQuotaInfo(modelId)
  if (!i) return '--'
  return (i.unit === 'money' ? '¥' : '') + apiFmtQuotaNum(i.total) + ''
}
function apiQuotaPctText(modelId) {
  var i = apiQuotaInfo(modelId)
  if (!i) return '--'
  return i.pct == null ? '—' : i.pct.toFixed(1).replace(/\.0$/, '') + '%'
}
function apiQuotaResetText(modelId) {
  var i = apiQuotaInfo(modelId)
  if (!i) return '--'
  if (i.reset === 'none') return '不重置'
  var now = new Date()
  var nx = i.reset === 'monthly'
    ? new Date(now.getFullYear(), now.getMonth() + 1, 1)
    : new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1)
  var ms = Math.max(0, nx.getTime() - now.getTime())
  var h = Math.floor(ms / 3600000)
  var d = Math.floor(h / 24)
  if (d > 0) return d + '天' + (h % 24) + '小时后重置'
  return h + '小时' + Math.floor((ms % 3600000) / 60000) + '分后重置'
}
// 面板/子菜单里显示的一行摘要
function apiQuotaSummary(modelId) {
  var q = apiQuotaOf(modelId)
  if (!q || !q.on) return '已关闭'
  var i = apiQuotaInfo(modelId)
  if (!i) return '已开启（未填总量）'
  if (i.used == null) return '统计未完成 · 用量未知'
  return (i.mode === 'codex' ? 'Codex · ' : (i.mode === 'auto' ? '自动 · ' : '手动 · ')) + '已用 ' + apiQuotaPctText(modelId) + ' · 剩 ' + apiFmtQuotaNum(i.left) + apiQuotaUnitSuffix(i)
}
// —— Codex 模式：本地会话统计（host 读 ~/.codex/sessions 得到，机器级数据）——
function apiCodexOf(modelId) {
  var m = apiModelById(modelId)
  return (m && m.codex) || null
}
function apiFmtTokens(n) {
  if (n == null || n === '') return '—'
  n = Number(n) || 0
  if (n >= 100000000) return (n / 100000000).toFixed(2).replace(/\.?0+$/, '') + '亿'
  if (n >= 10000) return (n / 10000).toFixed(n >= 1000000 ? 0 : 1).replace(/\.0$/, '') + '万'
  return String(Math.round(n))
}
function apiCodexDays7(c) {
  var s = 0
  if (c && Array.isArray(c.days7)) for (var i = 0; i < c.days7.length; i++) s += Number(c.days7[i].tokens) || 0
  return s
}
// 列表行摘要
function apiCodexRowText(am) {
  var c = am && am.codex
  if (!c || !c.ok) {
    // issue #116：把「被关掉 / 找不到目录 / 出错」区分开，不再是一句笼统的失败
    if (c && c.disabled) return 'Codex 统计已关闭'
    return c && c.error ? ('⚠ ' + c.error) : '无 Codex 数据'
  }
  var txt = 'Codex 今日 ' + apiFmtTokens(c.todayTokens) + ' · 近7天 ' + apiFmtTokens(apiCodexDays7(c)) + ' tokens'
  // 护栏的实际情况要能看见（issue #116：原报告抱怨"无开关、无报错、无提示"）
  var notes = []
  if (Number(c.skipped) > 0) notes.push('已跳过 ' + c.skipped + ' 个超大日志')
  if (Number(c.deferred) > 0) notes.push('统计更新中')
  if (notes.length) txt += '（' + notes.join(' · ') + '）'
  return txt
}
// 第二期：订阅窗口（5h / 周）。host 已把 rate_limits 归一成 { primary, secondary, planType }
function apiCodexWinLabel(w, idx) {
  if (w && w.windowMinutes) {
    if (w.windowMinutes >= 1440) return Math.round(w.windowMinutes / 1440) + '天窗口'
    if (w.windowMinutes >= 60) return Math.round(w.windowMinutes / 60) + 'h'
    return w.windowMinutes + '分钟'
  }
  return idx === 0 ? '5h' : '周'
}
function apiCodexWindowReset(w) {
  if (!w || !w.resetAt) return ''
  var left = Number(w.resetAt) - Date.now()
  if (!isFinite(left)) return ''
  if (left <= 0) return '即将重置'
  var h = Math.floor(left / 3600000)
  var d = Math.floor(h / 24)
  if (d > 0) return d + '天' + (h % 24) + '小时后重置'
  return h + '小时' + Math.floor((left % 3600000) / 60000) + '分后重置'
}
function apiCodexWindowsText(c) {
  var w = c && c.windows
  if (!w) return ''
  var pct = function (v) { return (v === null || v === undefined) ? '--' : ((Number(v) || 0).toFixed(1).replace(/\.0$/, '') + '%') }
  var parts = []
  if (w.primary) parts.push(apiCodexWinLabel(w.primary, 0) + ' 已用 ' + pct(w.primary.usedPct) + (apiCodexWindowReset(w.primary) ? ' · ' + apiCodexWindowReset(w.primary) : ''))
  if (w.secondary) parts.push(apiCodexWinLabel(w.secondary, 1) + ' 已用 ' + pct(w.secondary.usedPct) + (apiCodexWindowReset(w.secondary) ? ' · ' + apiCodexWindowReset(w.secondary) : ''))
  if (w.planType) parts.push(w.planType)
  return parts.join(' | ')
}
// 详细摘要（子菜单只读行 / 编辑器提示）
function apiCodexDetailText(modelId) {
  var c = apiCodexOf(modelId)
  if (!c || !c.ok) return c && c.error ? c.error : '无 Codex 数据'
  var s = '今日 ' + apiFmtTokens(c.todayTokens) + ' · 本月 ' + apiFmtTokens(c.monthTokens) +
    ' · 累计 ' + apiFmtTokens(c.totalTokens) + ' · 近7天 ' + apiFmtTokens(apiCodexDays7(c)) +
    '（' + (c.sessions || 0) + ' 个会话文件）'
  var w = apiCodexWindowsText(c)
  if (w) s += ' · ' + w
  return s
}
// —— 厂商订阅额度（kind='quota'：智谱 / Kimi Coding / MiniMax Coding 等，由 host 归一化下发）——
function apiPlanOf(modelId) {
  var m = apiModelById(modelId)
  return (m && m.plan) || null
}
function apiPlanSupport(modelId) {
  var m = apiModelById(modelId)
  if (!m) return false
  // host 判定「账号没有该订阅套餐」时标记 hide：这类行对没订阅的用户没有意义
  if (m.plan && m.plan.hide) return false
  return !!(m.planSupport || m.plan)
}
function apiPlanPctText(v) {
  if (v === null || v === undefined) return '--'
  return (Number(v) || 0).toFixed(1).replace(/\.0$/, '') + '%'
}
// —— 多窗口订阅额度（如 OpenCode Go 的 rolling / weekly / monthly）——
// 模块的「显示样式」= 选哪个时间窗口：all 全部三窗口 / rolling 5h / weekly 周 / monthly 月。
// 选中具体窗口时输出会带上窗口标签（如 `5h 2%`），避免看不出是哪个时间段。
var BUBBLE_PLAN_WIN_OPTS = [
  ['all', '全部（5h / 周 / 月）'],
  ['rolling', '5h'],
  ['weekly', '周'],
  ['monthly', '月'],
]
function bubblePlanWinOf(m) {
  m = m || {}
  var w = String(m.planWin || 'all')
  for (var i = 0; i < BUBBLE_PLAN_WIN_OPTS.length; i++) { if (BUBBLE_PLAN_WIN_OPTS[i][0] === w) return w }
  return 'all'
}
function bubblePlanWinLabel(w) {
  for (var i = 0; i < BUBBLE_PLAN_WIN_OPTS.length; i++) { if (BUBBLE_PLAN_WIN_OPTS[i][0] === w) return BUBBLE_PLAN_WIN_OPTS[i][1] }
  return BUBBLE_PLAN_WIN_OPTS[0][1]
}
// 订阅额度模块在编辑列表 / 模块库里的名字（带所选窗口，便于区分同一个模型的多个模块）
function bubblePlanModuleLabel(m) {
  m = m || {}
  var nm = (apiModelById(m.modelId) || {}).name || m.modelId
  var w = bubblePlanWinOf(m)
  return '额度·' + nm + (w === 'all' ? '' : '（' + bubblePlanWinLabel(w) + '）')
}
// 该模型的厂商模板是否声明了多窗口额度（决定编辑器里要不要给「显示样式」下拉）
function apiPlanMultiWin(modelId) {
  var am = apiModelById(modelId)
  if (!am) return false
  var list = apiTemplates || []
  for (var i = 0; i < list.length; i++) {
    if (list[i] && list[i].id === am.provider) {
      var q = list[i].quota
      return !!(q && q.json && q.json.windows && q.json.windows.length)
    }
  }
  return false
}
function apiPlanWinList(modelId) {
  var p = apiPlanOf(modelId)
  return (p && p.ok && p.windows && p.windows.length) ? p.windows : null
}
// 按窗口取「已用% / 剩余%」文本；非多窗口厂商返回 null（走原有单窗口逻辑）
function apiPlanPctWinText(modelId, win, left) {
  var list = apiPlanWinList(modelId)
  if (!list) return null
  function one(w) {
    var v = w.usedPct
    if (v === null || v === undefined) return '--'
    var pct = left ? Math.max(0, 100 - Number(v)) : Number(v)
    return (w.label ? (w.label + ' ') : '') + apiPlanPctText(pct)
  }
  if (win && win !== 'all') {
    for (var i = 0; i < list.length; i++) { if (list[i].key === win) return one(list[i]) }
    return '--'
  }
  var parts = []
  for (var j = 0; j < list.length; j++) parts.push(one(list[j]))
  return parts.join(' · ')
}
// 按窗口取「重置倒计时」文本；非多窗口厂商返回 null
function apiPlanResetWinText(modelId, win) {
  var list = apiPlanWinList(modelId)
  if (!list) return null
  function one(w, short) {
    var ms = apiPlanResetMs(w.resetAt)
    if (ms === null) return '--'
    var t = short ? apiPlanCountdownShortText(ms) : apiPlanCountdownText(ms)
    return t || '--'
  }
  if (win && win !== 'all') {
    for (var i = 0; i < list.length; i++) { if (list[i].key === win) return one(list[i], true) }
    return '--'
  }
  var parts = []
  for (var j = 0; j < list.length; j++) parts.push(one(list[j], true))
  return parts.join(' · ')
}
function apiPlanUsedText(modelId, win) {
  var t = apiPlanPctWinText(modelId, win, false)
  if (t !== null) return t
  var p = apiPlanOf(modelId)
  if (!p || !p.ok) return '--'
  return apiPlanPctText(p.usedPct)
}
function apiPlanLeftText(modelId, win) {
  var t = apiPlanPctWinText(modelId, win, true)
  if (t !== null) return t
  var p = apiPlanOf(modelId)
  if (!p || !p.ok) return '--'
  return apiPlanPctText(p.remainPct)
}
// 重置时间归一成毫秒（秒 / 毫秒 / 日期字符串都兼容）；解析不出来返回 null
function apiPlanResetMs(t) {
  if (t === null || t === undefined || t === '') return null
  if (typeof t === 'number') return t < 1e12 ? t * 1000 : t // 秒 / 毫秒都兼容
  var pd = Date.parse(String(t))
  return isFinite(pd) ? pd : null
}
// 倒计时文本（长写法，单窗口用）
function apiPlanCountdownText(ms) {
  var left = ms - Date.now()
  if (!isFinite(left)) return ''
  if (left <= 0) return '即将重置'
  var h = Math.floor(left / 3600000)
  var d = Math.floor(h / 24)
  if (d > 0) return d + '天' + (h % 24) + '小时后重置'
  return h + '小时' + Math.floor((left % 3600000) / 60000) + '分后重置'
}
// 倒计时文本（紧凑写法，多窗口用：5d21h / 3h53m）——单位统一用 d/h/m，不掺中文
// 不带「后重置」字样：多窗口模块里窗口标签已说明它是什么（如 `5h 2% · 3h53m`）
function apiPlanCountdownShortText(ms) {
  var left = ms - Date.now()
  if (!isFinite(left)) return ''
  if (left <= 0) return '即将重置'
  var m = Math.floor(left / 60000)
  var h = Math.floor(m / 60)
  var d = Math.floor(h / 24)
  if (d > 0) return d + 'd' + (h % 24) + 'h'
  if (h > 0) return h + 'h' + (m % 60) + 'm'
  return m + 'm'
}
function apiPlanResetText(modelId, win) {
  var t = apiPlanResetWinText(modelId, win)
  if (t !== null) return t
  var p = apiPlanOf(modelId)
  if (!p || !p.ok || p.resetAt === null || p.resetAt === undefined || p.resetAt === '') return '--'
  var ms = apiPlanResetMs(p.resetAt)
  if (ms === null) return String(p.resetAt)
  var txt = apiPlanCountdownText(ms)
  return txt || String(p.resetAt)
}
function apiPlanSummary(modelId) {
  var p = apiPlanOf(modelId)
  if (!p) return '读取中…'
  if (!p.ok) return p.hide ? '--' : (p.error || '读取失败')
  // v0.3.1：多窗口额度（如 OpenCode Go 的 5h / 周 / 月）逐窗口展示：`5h 12.5% · 2h55m后重置 | 周 …`
  if (p.windows && p.windows.length) {
    var parts = []
    for (var i = 0; i < p.windows.length; i++) {
      var w = p.windows[i] || {}
      var seg = w.label ? (w.label + ' ') : ''
      seg += apiPlanPctText(w.usedPct)
      var ms = apiPlanResetMs(w.resetAt)
      if (ms !== null) {
        var rt = apiPlanCountdownShortText(ms)
        if (rt) seg += ' · ' + rt
      }
      parts.push(seg)
    }
    if (p.level) parts.push(p.level)
    return parts.join(' | ')
  }
  var s = '已用 ' + apiPlanUsedText(modelId) + ' · ' + apiPlanResetText(modelId)
  if (p.weeklyUsedPct !== null && p.weeklyUsedPct !== undefined) s += ' · 周 ' + apiPlanPctText(p.weeklyUsedPct)
  if (p.level) s += ' · ' + p.level
  return s
}
function bubbleIsModelMod(m) { return !!(m && m.modelId && m.modelId !== 'deepseek') }
function bubbleContentTokenMap(m) {
  m = m || {}
  var v = ''
  var map = {}
  for (var period of ['today','week','month','all']) map['tokens_' + period] = codexWhaleHost.tokens(period)
  // 自定义模型模块：{balance}/{today} 取该模型自己的数据，并兼容旧的 _ds 写法
  if (bubbleIsModelMod(m) && (m.type === 'balance' || m.type === 'today' || m.type === 'quota' || m.type === 'plan')) {
    map['balance'] = apiModelBalanceText(m.modelId)
    map['today'] = apiModelTodayText(m.modelId)
    map['balance_ds'] = map['balance']
    map['expense_ds'] = map['today']
    // 手动额度占位符（订阅/资源包模型）
    map['quota'] = apiQuotaPctText(m.modelId)
    map['quota_pct'] = map['quota']
    map['quota_used'] = apiQuotaUsedText(m.modelId)
    map['quota_left'] = apiQuotaLeftText(m.modelId)
    map['quota_total'] = apiQuotaTotalText(m.modelId)
    map['quota_reset'] = apiQuotaResetText(m.modelId)
    // 厂商订阅额度占位符（kind='quota' 的厂商）：按模块「显示样式」选的窗口取数
    var pwin = bubblePlanWinOf(m)
    map['plan'] = apiPlanUsedText(m.modelId, pwin)
    map['plan_used'] = map['plan']
    map['plan_left'] = apiPlanLeftText(m.modelId, pwin)
    map['plan_reset'] = apiPlanResetText(m.modelId, pwin)
    return map
  }
  if (m.type === 'balance') {
    v = bubbleAmountText()
    map['balance_ds'] = v
  } else if (m.type === 'bonus') {
    // v782：赠金模块（{bonus_ds}）。取不到数 = '—'，不显示 0。
    map['bonus_ds'] = bubbleBonusText()
  } else if (m.type === 'recharge') {
    // v782：充值余额模块（{recharge_ds}）。
    map['recharge_ds'] = bubbleRechargeText()
  } else if (m.type === 'today') {
    v = (state.todayUsage !== null && state.todayUsage !== undefined ? fmt(state.todayUsage, state.todayUsageCurrency || state.currency) : '--')
    map['expense_ds'] = v
  } else if (m.type === 'session') {
    // v768：对话名模块 —— {session} = 当前对话标题（按模块的「保留长度」截断）
    map['session'] = bubbleSessionText(m)
  } else if (bubbleIsPeakCount(m)) {
    v = bubbleCountdownText()
    map['countdown'] = v
  } else {
    v = bubblePeakText(m)
    map['status'] = v
  }
  return map
}
// 各模块可用的英文占位符说明(? 按钮内容)
function bubbleTplHelpItems(m) {
  m = m || {}
  var arr = []
  function add(k, d) { arr.push({ k: '{' + k + '}', d: d }) }
  if (m.modelId === 'codex') for (var p of ['today','week','month','all']) add('tokens_' + p, 'Codex ' + p + ' token 用量')
  if (bubbleIsModelMod(m) && (m.type === 'balance' || m.type === 'today' || m.type === 'quota' || m.type === 'plan')) {
    if (m.type !== 'quota' && m.type !== 'plan') {
      add('balance', '该模型的余额')
      add('today', '该模型今日已用')
    }
    if (m.type !== 'plan') {
      add('quota', '额度已用百分比')
      add('quota_used', '额度已用值')
      add('quota_left', '额度剩余值')
      add('quota_total', '额度总量')
      add('quota_reset', '额度重置倒计时')
    }
    add('plan', '订阅额度已用百分比（多窗口厂商按「显示样式」所选窗口，带窗口标签）')
    add('plan_left', '订阅额度剩余百分比（同上）')
    add('plan_reset', '订阅额度刷新倒计时（同上；全部窗口时为紧凑倒计时）')
    return arr
  }
  if (m.type === 'balance') add('balance_ds', '总余额(充值金额+赠金)')
  if (m.type === 'bonus') add('bonus_ds', '赠金余额（未过期的赠送余额；取不到显示 —）')
  if (m.type === 'recharge') add('recharge_ds', '充值余额（不含赠金；取不到显示 —）')
  else if (m.type === 'today') add('expense_ds', '今日已用金额')
  else if (m.type === 'session') add('session', '当前对话名（超过「保留长度」会截断为 前N字...）')
  else if (m.type === 'peak' || m.type === 'nextpeak') {
    if (bubbleIsPeakCount(m)) add('countdown', '距下一时段倒计时 (HH:MM:SS)')
    else add('status', '高峰/空闲 状态文字(随显示样式变化)')
  }
  return arr
}
var dshwvTplHelpEl = null
function bubbleTplHelpToggle(m, anchor) {
  try {
    if (!dshwvTplHelpEl) {
      dshwvTplHelpEl = document.createElement('div')
      dshwvTplHelpEl.className = 'dshwv-tplhelp'
      dshwBodyAppend(dshwvTplHelpEl)
      document.addEventListener('pointerdown', function (e) {
        if (!dshwvTplHelpEl || dshwvTplHelpEl.style.display === 'none') return
        try {
          if (e.target && e.target.closest && (e.target.closest('.dshwv-tplq') || e.target.closest('.dshwv-tplhelp'))) return
        } catch (err) {}
        dshwvTplHelpEl.style.display = 'none'
      }, true)
      document.addEventListener('keydown', function (e) { if (dshwvComposingNow()) return; if (e.key === 'Escape') dshwvTplHelpEl.style.display = 'none' })
    }
    if (dshwvTplHelpEl.style.display === 'block') { dshwvTplHelpEl.style.display = 'none'; return }
    var items = bubbleTplHelpItems(m)
    var html = '<div style="font-weight:600;margin-bottom:4px">可用占位符(替换到内容里)</div>'
    if (!items.length) html += '<div style="opacity:.8">该模块无自动内容占位</div>'
    for (var i = 0; i < items.length; i++) html += '<div style="margin:1px 0"><b style="color:#2f4488">' + items[i].k + '</b> — ' + items[i].d + '</div>'
    html += '<div style="margin-top:5px;opacity:.65">其余文字原样显示;留空=默认自动内容</div>'
    dshwvTplHelpEl.innerHTML = html
    dshwvTplHelpEl.style.display = 'block'
    // 与 dshwvHintShow 同理:占位符说明弹层也要抬到当前可见窗口之上,否则在提醒编辑器里会被遮罩盖住
    try { dshwvTplHelpEl.style.zIndex = String(Math.max(26080, Math.round(visibleTopZ()) + 10)) } catch (err) {}
    var r = anchor ? anchor.getBoundingClientRect() : { left: 60, top: 120, right: 180, width: 100 }
    var w = 252
    var vp = viewport()
    var left = Math.max(4, Math.min(r.right - w, vp.w - w - 4))
    var top = r.bottom + 4
    var h = dshwvTplHelpEl.offsetHeight || 120
    if (top + h > vp.h - 4) top = Math.max(4, r.top - h - 4)
    dshwvTplHelpEl.style.left = Math.round(left) + 'px'
    dshwvTplHelpEl.style.top = Math.round(top) + 'px'
  } catch (err) {}
}
// —— 通用「?」说明圈(v647):桌面悬浮即显示,点击固定/收起;触摸端点按开关 ——
var dshwvHintEl = null
var dshwvHintPinned = false
var dshwvHintShownAt = 0
function dshwvHintHide() {
  dshwvHintPinned = false
  if (dshwvHintEl) dshwvHintEl.style.display = 'none'
}
function dshwvHintEnsure() {
  if (dshwvHintEl) return dshwvHintEl
  dshwvHintEl = document.createElement('div')
  dshwvHintEl.className = 'dshwv-tplhelp dshwv-hintbox'
  dshwvHintEl.style.display = 'none'
  dshwBodyAppend(dshwvHintEl)
  document.addEventListener('pointerdown', function (e) {
    if (!dshwvHintEl || dshwvHintEl.style.display === 'none') return
    try {
      if (e.target && e.target.closest && (e.target.closest('.dshwv-askq') || e.target.closest('.dshwv-hintbox'))) return
    } catch (err) {}
    dshwvHintHide()
  }, true)
  document.addEventListener('keydown', function (e) { if (dshwvComposingNow()) return; if (e.key === 'Escape') dshwvHintHide() })
  return dshwvHintEl
}
function dshwvHintShow(html, anchor, pinned) {
  var el = dshwvHintEnsure()
  try { if (dshwvTplHelpEl) dshwvTplHelpEl.style.display = 'none' } catch (err) {} // 同一时间只留一个说明弹层
  el.innerHTML = html
  el.style.display = 'block'
  // 说明弹层挂在 body 上(CSS 默认 z-index:26080),在弹窗里会被窗口遮罩(提醒编辑器 30000)整体盖住,
  // 看起来就像"问号里的内容没了" → 显示时按当前最顶层可见弹窗抬高一层(与下拉菜单 dshwDropOpen 同一套做法)
  try { el.style.zIndex = String(Math.max(26080, Math.round(visibleTopZ()) + 10)) } catch (err) {}
  dshwvHintPinned = !!pinned
  dshwvHintShownAt = Date.now()
  try {
    var r = anchor ? anchor.getBoundingClientRect() : { left: 60, top: 120, right: 180, bottom: 140 }
    var vp = viewport()
    var w = el.offsetWidth || 320
    var h = el.offsetHeight || 120
    var left = Math.max(4, Math.min(r.left, vp.w - w - 4))
    var top = r.bottom + 6
    if (top + h > vp.h - 4) top = Math.max(4, r.top - h - 6)
    el.style.left = Math.round(left) + 'px'
    el.style.top = Math.round(top) + 'px'
  } catch (err) {}
}
function dshwvAskDot(html) {
  var b = document.createElement('button')
  b.type = 'button'
  b.className = 'dshwv-askq dshwv-tplq'
  b.textContent = '?'
  b.title = '查看说明'
  b.addEventListener('mouseenter', function () {
    if (dshwvHintPinned && dshwvHintEl && dshwvHintEl.style.display === 'block') return
    dshwvHintShow(html, b, false)
  })
  b.addEventListener('mouseleave', function () { if (!dshwvHintPinned) dshwvHintHide() })
  b.addEventListener('click', function (e) {
    e.stopPropagation()
    if (dshwvHintEl && dshwvHintEl.style.display === 'block' && dshwvHintPinned && (Date.now() - dshwvHintShownAt > 350)) { dshwvHintHide(); return }
    dshwvHintShow(html, b, true)
  })
  return b
}
function bubbleContentText(m, autoTxt) {
  m = m || {}
  if (!m.tpl || !String(m.tpl).length) return autoTxt
  var map = bubbleContentTokenMap(m)
  var s = String(m.tpl)
  var keys = Object.keys(map).sort(function (a, b) { return b.length - a.length })
  for (var i = 0; i < keys.length; i++) s = s.split('{' + keys[i] + '}').join(String(map[keys[i]]))
  return s
}
// 倒计时样式文案(含模板;供渲染与每秒 ticker 共用)
function bubbleCountTextOf(m) { return bubbleContentText(m, bubbleCountdownText()) }
// 跑马灯动画时长:每次渲染随机 1.5s~4.5s,各行速度不同(预览/真实共用同一渲染器)
function bubbleMarqueeDur() { return Math.round(1500 + Math.random() * 3000) + 'ms' }
// 峰谷模块「显示样式」:默认 / 梁文峰谷 / !?强强?! / 倒计时 / 简洁(峰·谷)
// 称呼样式按模块自身 peakStyle,不再依赖主菜单全局设置(已下线)
var BUBBLE_PEAK_STYLE_OPTS = [
  ['default', '默认'],
  ['liangwen', '梁文峰谷'],
  ['qiangqiang', '!?强强?!'],
  ['count', '倒计时'],
  ['mini', '简洁(峰/谷)'],
]
function bubblePeakStyleOf(m) {
  m = m || {}
  if (m.type === 'nextpeak') return 'count' // 旧版独立倒计时模块视为倒计时样式
  var s = String(m.peakStyle || 'default')
  for (var i = 0; i < BUBBLE_PEAK_STYLE_OPTS.length; i++) {
    if (BUBBLE_PEAK_STYLE_OPTS[i][0] === s) return s
  }
  return 'default'
}
function bubbleIsPeakCount(m) { return bubblePeakStyleOf(m) === 'count' }
function bubblePeakModuleLabel(m) {
  var s = bubblePeakStyleOf(m)
  if (s === 'count') return '时段倒计时'
  if (s === 'mini') return '简洁(峰/谷)'
  return '峰谷时段'
}
// 服务端峰谷数据是否已就绪(isPeak 来自余额接口)
function bubblePeakReady() {
  return state && state.balance !== null && state.status !== 'loading' && state.status !== 'error'
}
// 当前是否为高峰(供非倒计时样式用):服务端就绪时以其 isPeak 为准;
// 未就绪(首拉加载/失败)时先用本地北京规则(bubbleCountdownIsPeak,与倒计时同源)先行显示,
// 避免错误显示「谷/空闲」或出现加载圈;就绪后若结果不一致由渲染重绘校正为服务端值
function bubbleIsPeakNow() {
  if (bubblePeakReady()) return !!state.isPeak
  return bubbleCountdownIsPeak()
}
// 峰谷状态文字(count 倒计时样式不在此生成,由倒计时引擎逐秒输出)
function bubblePeakText(m) {
  if (state.peakSupported !== true) return '未配置峰谷'
  m = m || {}
  var st = bubblePeakStyleOf(m)
  var peak = bubbleIsPeakNow()
  if (st === 'liangwen') return peak ? '梁文峰' : '梁文谷'
  if (st === 'qiangqiang') return peak ? '!?峰峰?!' : '!?谷谷?!'
  if (st === 'mini') return peak ? '峰' : '谷'
  return peak ? '高峰时段' : '空闲时段'
}
// 按权重抽 1 条,avoidIdx 用于“不连续重复上一句”
function bubblePickLine(lines, avoidIdx) {
  if (!Array.isArray(lines) || !lines.length) return null
  if (lines.length === 1) return 0
  var total = 0
  for (var i = 0; i < lines.length; i++) total += Math.max(1, Number(lines[i].w) || 1)
  var pick
  for (var guard = 0; guard < 6; guard++) {
    var r = Math.random() * total
    var acc = 0
    pick = lines.length - 1
    for (var j = 0; j < lines.length; j++) {
      acc += Math.max(1, Number(lines[j].w) || 1)
      if (r < acc) { pick = j; break }
    }
    if (pick !== avoidIdx) break
  }
  return pick
}
function bubbleModuleText(m, avoidIdx) {
  var t = m.text || ''
  if (m.type === 'random' && Array.isArray(m.lines)) {
    var idx = bubblePickLine(m.lines, avoidIdx)
    var ln = m.lines[idx]
    if (ln) {
      m._lastPick = idx
      return ln.t
    }
    return ''
  }
  return t
}
// —— 下个时段倒计时模块:按“工作日9-12/14-18为高峰,周末全天谷价”推算下一时段切换 ——
// v746：法定节假日同样全天谷价。前端无法自己知道放假安排，节假日清单由宿主随余额接口下发
// （state.peakHolidays，单一来源），切换点也优先用宿主算好的绝对值（state.peakNextChangeAt）。
var bubbleHolidaySet = null
var bubbleHolidaySetKey = ''
function bubbleHolidaySetOf() {
  var list = (state && state.peakHolidays) || null
  var key = Array.isArray(list) ? list.join(',') : ''
  if (key !== bubbleHolidaySetKey) {
    bubbleHolidaySetKey = key
    bubbleHolidaySet = {}
    if (Array.isArray(list)) for (var i = 0; i < list.length; i++) bubbleHolidaySet[String(list[i])] = 1
  }
  return bubbleHolidaySet
}
function bubbleBJHolidayKey(bj) {
  try { return bj.toISOString().slice(0, 10) } catch (err) { return '' }
}
function bubbleCountdownIsPeak(sec) {
  sec = isFinite(Number(sec)) ? Number(sec) : Math.floor(Date.now() / 1000)
  var bj = new Date((sec + 8 * 3600) * 1000)
  var hs = bubbleHolidaySetOf()
  if (hs && hs[bubbleBJHolidayKey(bj)]) return false // 法定节假日全天谷价
  var dow = bj.getUTCDay()
  var h = bj.getUTCHours()
  if (dow === 0 || dow === 6) return false
  return (h >= 9 && h < 12) || (h >= 14 && h < 18)
}
function bubbleCountdownNextChange(sec) {
  sec = isFinite(Number(sec)) ? Number(sec) : Math.floor(Date.now() / 1000)
  // 宿主已就绪时直接用其算好的切换点（与计费同源，含法定节假日）；
  // 只接受未来 12 天内的值——切过去之后该值会过期，回退到本地推算，避免倒计时卡在 00:00:00
  var hostCand = Number(state && state.peakNextChangeAt)
  if (isFinite(hostCand) && hostCand > sec + 1 && hostCand - sec <= 12 * 86400) return hostCand
  var cur = bubbleCountdownIsPeak(sec)
  var bjDay0 = Math.floor((sec + 8 * 3600) / 86400) * 86400
  var best = null
  for (var d = 0; d <= 12 && best === null; d++) {
    var dayStartBj = bjDay0 + d * 86400
    var edges = [0, 9 * 3600, 12 * 3600, 14 * 3600, 18 * 3600]
    for (var i = 0; i < edges.length; i++) {
      var cand = dayStartBj + edges[i] - 8 * 3600
      if (cand <= sec + 1) continue
      if (bubbleCountdownIsPeak(cand) !== cur) { best = cand; break }
    }
  }
  if (best === null) return sec + 86400 // 兜底
  return best
}
function bubbleCountdownText() {
  if (state.peakSupported !== true) return '未配置峰谷'
  var now = Math.floor(Date.now() / 1000)
  var cand = bubbleCountdownNextChange(now)
  var remain = Math.max(0, cand - now)
  var p = function (n) { return String(n).padStart(2, '0') }
  var hh = Math.floor(remain / 3600)
  var mm = Math.floor((remain % 3600) / 60)
  var ss = remain % 60
  // 只显示倒计时时间:HH:MM:SS(时段切换点随高峰/空闲状态自动取)
  return p(hh) + ':' + p(mm) + ':' + p(ss)
}
var bubbleCountdownTicker = null
var bubbleCountdownRows = [] // {el, mod}
function bubbleCountdownApplyStyle(el, mod, peak) {
  try {
    var rgb = peak ? (mod.peakRgb || '') : (mod.offRgb || '')
    var col = peak ? (mod.peakColor || '') : (mod.offColor || '')
    if (!rgb && !col) col = mod.color || ''
    // 前缀清除该行上所有跑马灯渐变类(含 dshwv-rgb 本体),再按当前状态重设
    var cls = Array.prototype.slice.call(el.classList || [])
    for (var i = 0; i < cls.length; i++) {
      if (String(cls[i]).indexOf('dshwv-rgb') === 0) { try { el.classList.remove(cls[i]) } catch (err) {} }
    }
    el.style.color = ''
    if (rgb) {
      var scheme = rgb === true ? 'macaron' : String(rgb || 'macaron')
      el.classList.add('dshwv-rgb')
      if (bubbleRgbSchemeOk(scheme)) el.classList.add('dshwv-rgb-' + scheme)
    } else if (col) {
      el.style.color = col
    }
  } catch (err) {}
}
function bubbleCountdownTick() {
  try {
    bubbleCountdownRows = bubbleCountdownRows.filter(function (x) { return x && x.el && x.el.isConnected })
    var cur = bubbleCountdownIsPeak()
    for (var i = 0; i < bubbleCountdownRows.length; i++) {
      var x = bubbleCountdownRows[i]
      try {
        x.el.textContent = bubbleCountTextOf(x.mod)
        bubbleCountdownApplyStyle(x.el, x.mod, cur)
      } catch (err) {}
    }
    // v733：非倒计时的峰谷行共用这个 ticker（只在状态真的变了时才动 DOM，不打断跑马灯）
    bubblePeakRows = bubblePeakRows.filter(function (y) { return y && y.row && y.row.isConnected })
    if (bubblePeakRows.length) {
      var peakNow = bubbleIsPeakNow()
      for (var k = 0; k < bubblePeakRows.length; k++) {
        var y2 = bubblePeakRows[k]
        if (y2.peak !== peakNow) {
          y2.peak = peakNow
          try { bubblePeakRowApply(y2, peakNow) } catch (err) {}
        }
      }
    }
    if (!bubbleCountdownRows.length && !bubblePeakRows.length && bubbleCountdownTicker) {
      clearInterval(bubbleCountdownTicker)
      bubbleCountdownTicker = null
    }
  } catch (err) {}
}
function bubbleCountdownRegister(el, mod) {
  bubbleCountdownRows.push({ el: el, mod: mod })
  if (!bubbleCountdownTicker) bubbleCountdownTicker = setInterval(bubbleCountdownTick, 1000)
}
// —— v733：峰谷状态实时跟随 ——
// count / nextpeak 样式由倒计时引擎逐秒刷新；其余峰谷样式（默认「高峰时段 / 空闲时段」、
// 梁文峰谷、!?峰峰?!、简洁峰/谷）原来只在渲染那一刻取一次状态 —— 泡泡显示期间跨过峰谷切换点，
// 文字与配色会一直停在旧状态。这里把这类行登记进同一个 1s ticker，状态变化时**原地**改写
// 文字 / 配色 / 底色（不做整泡重绘，保持「泡泡显示期间内容稳定」的既有设计）。
var bubblePeakRows = []
// 文字是直接放在行上还是包一层内层 span，取决于该状态有没有底色 ——
// 只有「高峰 / 空闲两态底色有无一致」时，DOM 结构才不随状态变化，才能原地改写
function bubblePeakRowStable(m) {
  return !!(m.peakBg || m.peakBgRgb) === !!(m.offBg || m.offBgRgb)
}
function bubblePeakRowRegister(row, tx, mod, peakNow) {
  try {
    if (!mod || mod.type !== 'peak' || bubbleIsPeakCount(mod)) return
    if (!bubblePeakRowStable(mod)) return
    bubblePeakRows.push({
      row: row, tx: tx, mod: mod, peak: !!peakNow,
      gradEl: (mod.peakBg || mod.peakBgRgb) ? tx : row,
    })
    if (!bubbleCountdownTicker) bubbleCountdownTicker = setInterval(bubbleCountdownTick, 1000)
  } catch (err) {}
}
// 跑马灯配色方案白名单（与 blockOf() / bubbleCountdownApplyStyle 用的那套一致）
// v770：新增配色时**只改这一处**（其余调用点已改成调它，不再各自维护一串 || 判断）
function bubbleRgbSchemeOk(s) {
  return s === 'candy' || s === 'rouge' || s === 'bamboo' || s === 'aurora' || s === 'deepsea' ||
    s === 'sunset' || s === 'forest' || s === 'champagne' || s === 'lavender' || s === 'mint' ||
    s === 'lava' || s === 'galaxy' || s === 'ink' || s === 'indigo' ||
    s === 'blaze' || s === 'amber'
}
function bubblePeakRowClearClass(el, prefix) {
  try {
    var cls = Array.prototype.slice.call(el.classList || [])
    for (var i = 0; i < cls.length; i++) {
      if (String(cls[i]).indexOf(prefix) === 0) { try { el.classList.remove(cls[i]) } catch (err) {} }
    }
  } catch (err) {}
}
// 原地改写一行峰谷：顺序与 blockOf() 的峰谷分支一致（底色 → 文字跑马灯 → 纯色）
function bubblePeakRowApply(x, peak) {
  var m = x.mod
  try { x.tx.textContent = bubbleContentText(m, bubblePeakText(m)) } catch (err) {}
  // ① 底色（高峰底色 / 空闲底色，跑马灯优先于纯色）
  try {
    var effBgRgb = peak ? String(m.peakBgRgb || '') : String(m.offBgRgb || '')
    var effBg = effBgRgb ? '' : (peak ? String(m.peakBg || '') : String(m.offBg || ''))
    if (effBgRgb === 'true') effBgRgb = 'macaron'
    bubblePeakRowClearClass(x.row, 'dshwv-bgrgb')
    x.row.style.background = ''
    if (effBgRgb) {
      x.row.classList.add('dshwv-bgrgb')
      if (bubbleRgbSchemeOk(effBgRgb) || effBgRgb === 'macaron') x.row.classList.add('dshwv-bgrgb-' + effBgRgb)
      x.row.style.animationDuration = bubbleMarqueeDur()
    } else if (effBg) {
      x.row.style.background = effBg
    }
  } catch (err) {}
  // ② 文字：本状态跑马灯 > 模块跑马灯 > 本状态纯色 > 模块纯色
  try {
    var marquee = (peak ? String(m.peakRgb || '') : String(m.offRgb || '')) || String(m.rgb || '')
    bubblePeakRowClearClass(x.gradEl, 'dshwv-rgb')
    x.row.style.color = ''
    if (marquee) {
      var scheme = marquee === true ? 'macaron' : String(marquee || 'macaron')
      x.gradEl.classList.add('dshwv-rgb')
      if (bubbleRgbSchemeOk(scheme)) x.gradEl.classList.add('dshwv-rgb-' + scheme)
      x.gradEl.style.animationDuration = bubbleMarqueeDur()
    } else {
      var pcol = peak ? String(m.peakColor || '') : String(m.offColor || '')
      if (pcol) x.row.style.color = pcol
      else if (m.color) x.row.style.color = String(m.color)
    }
  } catch (err) {}
}
// v209: 计算单个模块要显示的行文本(与随机选中行),每次全新计算、不跨行复用状态
function bubbleRowContentOf(mod) {
  mod = mod || {}
  if (mod.type === 'nextpeak' || (mod.type === 'peak' && bubbleIsPeakCount(mod))) return { txt: bubbleCountTextOf(mod), line: null }
  if (mod.type === 'balance') {
    var bv = bubbleIsModelMod(mod) ? apiModelBalanceText(mod.modelId) : bubbleAmountText()
    return { txt: bubbleContentText(mod, bv), line: null }
  }
  // v782：赠金 / 充值余额两个数值模块（与「总余额(充值金额+赠金)」并列，取不到数时自动文案是 —）
  if (mod.type === 'bonus') return { txt: bubbleContentText(mod, bubbleBonusText()), line: null }
  if (mod.type === 'recharge') return { txt: bubbleContentText(mod, bubbleRechargeText()), line: null }
  if (mod.type === 'today') {
    var tv2 = bubbleIsModelMod(mod) ? apiModelTodayText(mod.modelId) : (state.todayUsage !== null && state.todayUsage !== undefined ? fmt(state.todayUsage, state.todayUsageCurrency || state.currency) : '--')
    return { txt: bubbleContentText(mod, '今日已用 ' + tv2), line: null }
  }
  if (mod.type === 'session') {
    // v768：对话名模块（自动内容 = 截断后的对话名；留空 tpl 时直接用自动内容）
    return { txt: bubbleContentText(mod, bubbleSessionText(mod)), line: null }
  }
  if (mod.type === 'quota') {
    var qi = apiQuotaInfo(mod.modelId)
    var qt = qi
      ? ('额度 ' + apiQuotaPctText(mod.modelId) + ' · 剩 ' + apiFmtQuotaNum(qi.left) + apiQuotaUnitSuffix(qi))
      : '额度 --'
    return { txt: bubbleContentText(mod, qt), line: null }
  }
  if (mod.type === 'plan') {
    return { txt: bubbleContentText(mod, '额度 ' + apiPlanSummary(mod.modelId)), line: null }
  }
  if (mod.type === 'peak') {
    var pw = bubblePeakText(mod)
    return { txt: bubbleContentText(mod, pw), line: null }
  }
  if (mod.type === 'random' && Array.isArray(mod.lines)) {
    var pi = bubblePickLine(mod.lines, mod._lastPick)
    if (pi !== null && pi !== undefined && mod.lines[pi]) {
      mod._lastPick = pi
      return { txt: mod.lines[pi].t, line: mod.lines[pi] }
    }
    return { txt: '', line: null }
  }
  return { txt: mod.text || '', line: null }
}
// 渲染一组模块为泡泡内容(F2 渲染层:按行分组 → 行内 flex 并排(≤6 模块/行)→ 超宽自动折行;
// 图片单独一整行;泡泡行数上限维持 6)
// 把模块追加为行到指定父容器(真实泡泡 textBox 与编辑器预览共用;旧配置每模块一行 → 与旧版逐像素一致)
function bubbleRowsTo(parentEl, mods) {
  if (!parentEl || !Array.isArray(mods)) return
  var old = parentEl.querySelectorAll('.dshwv-trow, .dshwv-mimg')
  for (var i = 0; i < old.length; i++) { try { parentEl.removeChild(old[i]) } catch (err) {} }
  var ROW_MAX = 6 // 泡泡行数上限(维持旧版)
  var MOD_MAX = 6 // 每行模块数上限(F2)
  // —— 单个模块 → 行内块:样式/底色/跑马灯逐模块独立,与旧版“整行样式”完全一致 ——
  function blockOf(m, rowContent) {
    var line = rowContent ? rowContent.line : null
    var fSize = m.size
    var fColor = m.color
    var fBold = m.bold
    var fItalic = m.italic
    var fUl = m.ul
    var fRgb = m.rgb
    if (line) {
      if (line.size) fSize = line.size
      if (line.color) fColor = line.color
      // 行级字形:显式 false 允许取消(模块级/随机默认加粗不强行覆盖)
      if (line.bold === false) fBold = false
      else if (line.bold === true) fBold = true
      if (line.italic === false) fItalic = false
      else if (line.italic === true) fItalic = true
      if (line.ul === false) fUl = false
      else if (line.ul === true) fUl = true
      if (line.rgb) fRgb = line.rgb
    }
    // 随机语句默认加粗:仅当没有行级显式取消(bold===false)时才视为加粗
    if (m.type === 'random' && fBold !== false) fBold = true
    // 当前是否高峰(倒计时样式用本地北京规则逐秒判定,其余经 bubbleIsPeakNow:
    // 服务端 isPeak 就绪则用它,未就绪先用本地规则,状态字与底色始终一致)
    var curPeak = bubbleIsPeakCount(m) ? bubbleCountdownIsPeak() : bubbleIsPeakNow()
    // 文字底色:峰谷/倒计时按当前状态分别取「高峰底色/空闲底色」;其余模块行级优先,其次模块级
    var effBg = ''
    var effBgRgb = ''
    if (m.type === 'peak' || m.type === 'nextpeak') {
      effBgRgb = curPeak ? String(m.peakBgRgb || '') : String(m.offBgRgb || '')
      effBg = effBgRgb ? '' : (curPeak ? String(m.peakBg || '') : String(m.offBg || ''))
    } else {
      if (line && line.bgRgb) effBgRgb = String(line.bgRgb)
      else if (line && line.bg) effBg = String(line.bg)
      if (!effBgRgb && !effBg) {
        if (m.bgRgb) effBgRgb = String(m.bgRgb)
        else if (m.bg) effBg = String(m.bg)
      }
    }
    if (effBgRgb === 'true') effBgRgb = 'macaron' // 兼容旧布尔数据
    var needBg = !!(effBgRgb || effBg)
    var row = document.createElement('div')
    row.className = 'dshwv-trow'
    // 有底色时:文字放入内层 span,底色作为 row 的独立底层(纯色/跑马灯都在 row 上)——
    // 底色与文字(含文字跑马灯/颜色)不再互相占用 background,可共存,文字不必转白
    var tx = row
    if (needBg) {
      row.style.padding = '1px 6px'
      row.style.borderRadius = '7px'
      row.style.textShadow = 'none'
      tx = document.createElement('span')
      tx.className = 'dshwv-trowtx'
      row.appendChild(tx)
    }
    tx.textContent = String(rowContent.txt)
    // 峰谷/时段倒计时块不受自动换行影响:保持单行显示
    if (m.type === 'peak' || m.type === 'nextpeak') {
      row.style.whiteSpace = 'nowrap'
      row.style.maxWidth = 'none'
    }
    row.style.fontSize = 'calc(var(--dshw-u) * ' + bubbleModuleFontU(fSize) + ')'
    // 加粗:普通模块 700;余额模块默认本就 800(重档),勾选加粗须用更重 900,避免“700 < 默认800”反而变细
    if (fBold) row.style.fontWeight = (m.type === 'balance') ? '900' : '700'
    else if (m.type === 'balance') row.style.fontWeight = '800'
    if (fItalic) row.style.fontStyle = 'italic'
    if (fUl) row.style.textDecoration = 'underline'
    var fFont = m.fontFamily || ''
    if (line && line.fontFamily) fFont = line.fontFamily
    if (fFont) row.style.fontFamily = fFont
    // 倒计时:数字等宽(tabular-nums),每秒变化时字符宽度不变 → 底色长度稳定
    if (bubbleIsPeakCount(m)) row.style.fontVariantNumeric = 'tabular-nums'
    var marquee = fRgb
    // 高峰/空闲可分别选跑马灯(优先级高于各自颜色)
    if (m.type === 'peak' || m.type === 'nextpeak') {
      var peakStateMarquee = curPeak ? (m.peakRgb || '') : (m.offRgb || '')
      if (peakStateMarquee) marquee = peakStateMarquee
    }
    function applyTextGradient(target, g) {
      // 跑马灯渐变:true 兼容旧数据=马卡龙;支持 macaron/candy/rouge/bamboo…
      target.classList.add('dshwv-rgb')
      var scheme = g === true ? 'macaron' : String(g || 'macaron')
      if (bubbleRgbSchemeOk(scheme)) target.classList.add('dshwv-rgb-' + scheme)
    }
    if (marquee) {
      // 文字跑马灯装在内层 span(有底色时)或行上;底色在 row 层,两者可叠加;
      // 每次渲染给该条随机一个动画时长(1.5~4.5s),各行速度各异
      var mt = needBg ? tx : row
      applyTextGradient(mt, marquee)
      mt.style.animationDuration = bubbleMarqueeDur()
    } else if (m.type === 'peak' || m.type === 'nextpeak') {
      var pcol = curPeak ? (m.peakColor || '') : (m.offColor || '')
      if (pcol) row.style.color = pcol
      else if (fColor) row.style.color = fColor
    } else if (fColor) {
      row.style.color = fColor
    }
    // 底色层:画在 row 上(纯色直接 background;跑马灯用 bgrgb 渐变类),位于文字下方
    if (needBg) {
      if (effBgRgb) {
        row.classList.add('dshwv-bgrgb')
        if (bubbleRgbSchemeOk(effBgRgb) || effBgRgb === 'macaron') row.classList.add('dshwv-bgrgb-' + effBgRgb)
        row.style.animationDuration = bubbleMarqueeDur()
      } else if (effBg) {
        row.style.background = effBg
      }
    }
    return { el: row, tx: tx, fSize: fSize, mod: m, peak: (m.type === 'peak' || m.type === 'nextpeak'), bg: needBg, curPeak: curPeak }
  }
  // 超宽判定(与旧版一致):先单行渲染,测量实际超出泡泡内宽(560u)才允许该块内折行——
  // 预览与真实共用此逻辑,保证两者一致
  function maybeWrap(blk) {
    if (!blk || blk.peak) return
    try {
      var compFs = window.getComputedStyle ? parseFloat(window.getComputedStyle(blk.el).fontSize) : 0
      var multNow = bubbleModuleFontU(blk.fSize)
      var capPx = compFs && multNow ? 560 * compFs / multNow : 0
      if (capPx > 0 && blk.el.scrollWidth > capPx + 2) {
        blk.el.style.maxWidth = capPx + 'px'
        blk.el.style.whiteSpace = 'normal'
        blk.el.style.overflowWrap = 'anywhere'
        blk.el.style.wordBreak = 'break-word'
      } else {
        blk.el.style.whiteSpace = 'nowrap'
      }
    } catch (err) {}
  }
  // 倒计时块:注册文字节点(带底色时为内层 span),由每秒 ticker 刷新文案与配色
  function registerIfCountdown(blk) {
    if (bubbleIsPeakCount(blk.mod)) bubbleCountdownRegister(blk.tx, blk.mod)
    // v733：非 count 的峰谷行登记到同一个 ticker，状态切换时原地刷新（原来只在渲染时取一次）
    else if (blk.mod && blk.mod.type === 'peak') bubblePeakRowRegister(blk.el, blk.tx, blk.mod, blk.curPeak)
  }
  // 超链接模块:仅真实泡泡(textBox)内可点击,新标签页打开;预览/编辑不弹窗
  function enableLinkRun(blk2) {
    try {
      var lmd = blk2 && blk2.mod
      if (!lmd || lmd.type !== 'link') return
      if (!parentEl || parentEl !== textBox) return
      var u0 = String(lmd.url || '').trim()
      if (!/^https?:\/\//i.test(u0)) return
      var lel = blk2.el
      lel.style.cursor = 'pointer'
      lel.style.pointerEvents = 'auto'
      lel.title = u0
      lel.addEventListener('click', function (e) {
        try { e.preventDefault() } catch (err) {}
        try { e.stopPropagation() } catch (err) {}
        try { window.open(u0, '_blank', 'noopener') } catch (err) {}
      })
    } catch (err) {}
  }
  // 数据层行分组(bubbleRowsOf):平铺 modules[] → 视觉行;旧配置每模块一行
  var groups = bubbleRowsOf(mods)
  var rows = 0
  var imgDone = false
  for (var g = 0; g < groups.length; g++) {
    var grp = groups[g]
    if (!grp || !grp.length) continue
    if (bubbleIsImgMod(grp[0])) {
      // 图片/随机图片模块单独占一整行(旧版即全局唯一、独占一行,位置随模块顺序)
      var md = grp[0]
      var pickId = md.imgId || ''
      if (md.type === 'randimg') {
        // 随机图片:按权重抽 1 张,且不连续重复(与随机语句同规则)
        if (!Array.isArray(md.imgs)) md.imgs = []
        var pool = []
        for (var pi0 = 0; pi0 < md.imgs.length; pi0++) {
          var it0 = md.imgs[pi0] || {}
          if (it0.imgId) pool.push({ imgId: it0.imgId, w: it0.w })
        }
        if (!pool.length) continue
        var pickIdx = bubblePickLine(pool, md._lastPickImg)
        if (pickIdx === null || pickIdx === undefined || !pool[pickIdx]) continue
        md._lastPickImg = pickIdx
        pickId = pool[pickIdx].imgId
      }
      if (imgDone || !pickId) continue
      var im = document.createElement('img')
      im.className = 'dshwv-mimg'
      // 显示大小:imgScale(0.1–1,相对最大宽 540u);未设置=满宽
      var scV2 = Number(md.imgScale)
      if (isFinite(scV2) && scV2 > 0) im.style.maxWidth = 'calc(var(--dshw-u) * ' + (540 * Math.max(0.1, Math.min(1, scV2))) + ')'
      im.src = '/dsh-whale/bubble-img.png?id=' + encodeURIComponent(pickId)
      im.alt = ''
      im.draggable = false
      parentEl.appendChild(im)
      imgDone = true
      continue
    }
    // 一行超过 MOD_MAX 个模块时拆成多行(F2 每行至多 6;数据由编辑器保证,此处仅防御)
    for (var s = 0; s < grp.length && rows < ROW_MAX; s += MOD_MAX) {
      var chunk = []
      for (var c = s; c < grp.length && c < s + MOD_MAX; c++) {
        var cm = grp[c] || {}
        // v209: 每行内容由独立函数一次算出(文本+随机选中行),不在行间复用状态
        var rowContent = bubbleRowContentOf(cm)
        if (!rowContent || rowContent.txt === '' || rowContent.txt === undefined || rowContent.txt === null) continue
        chunk.push(blockOf(cm, rowContent))
      }
      if (!chunk.length) continue
      if (chunk.length === 1) {
        // 单模块行:保持旧版 DOM 结构(直接作为 textBox 的行),布局逐像素一致
        var blk1 = chunk[0]
        parentEl.appendChild(blk1.el)
        enableLinkRun(blk1)
        registerIfCountdown(blk1)
        maybeWrap(blk1)
        rows++
        continue
      }
      // 多模块行:文本流式排布 —— 模块依次紧排,到 560u 换行限宽才在文字内折行续行;
      // 字号不同按文字基线对齐;底色块随行断开延续(box-decoration-break:clone);峰谷/倒计时视为不拆行的整块
      var capPx2 = 0
      try {
        var uCss2 = window.getComputedStyle ? window.getComputedStyle(parentEl).getPropertyValue('--dshw-u') : ''
        var uVal2 = parseFloat(uCss2)
        if (uVal2 > 0) capPx2 = 560 * uVal2
      } catch (err) {}
      var para = document.createElement('div')
      para.className = 'dshwv-trow dshwv-trowline'
      para.style.textAlign = 'center'
      if (capPx2 > 0) para.style.maxWidth = capPx2 + 'px'
      for (var p = 0; p < chunk.length; p++) {
        var pr = chunk[p]
        var pe = pr.el
        // 块 → 行内流(run):跟随前一个模块同行续排;仅峰谷/倒计时保持不折行整块
        pe.style.display = 'inline'
        pe.style.verticalAlign = 'baseline'
        pe.style.margin = '0 calc(var(--dshw-u) * 6) 0 0'
        // 高度统一:无底色模块补上与底色模块相同的纵向盒(padding-top/bottom 1px,透明),
        // 横向不补,避免撑大模块间距
        if (!pr.bg) {
          pe.style.padding = '1px 0'
        }
        if (pr.bg) {
          pe.style.boxDecorationBreak = 'clone'
          pe.style.webkitBoxDecorationBreak = 'clone'
        }
        if (!pr.peak) {
          pe.style.whiteSpace = 'normal'
          pe.style.overflowWrap = 'anywhere'
          pe.style.wordBreak = 'break-word'
          pe.style.maxWidth = ''
        }
        para.appendChild(pe)
        enableLinkRun(pr)
      }
      parentEl.appendChild(para)
      for (var p2 = 0; p2 < chunk.length; p2++) {
        registerIfCountdown(chunk[p2])
      }
      rows++
    }
  }
}
// 当前泡泡正在显示的模块列表(供场景切换时记录;显示期间不再即时整泡重绘,
// 避免用户观看时内容被刷新打断——更新统一在“泡泡消失→下一次显示”的渲染采用最新 state)
var bubbleLiveMods = null
// v761（#161 C5）：当前是否正显示「等待交互」的常驻泡泡。只有真正渲染出等待内容时才置位，
// 因此拿它做幂等判断（同名再次入队直接丢弃）不会把"排了队但还没轮到"的项误判成已显示。
var waitShown = false
function bubbleRenderModules(mods) {
  try {
    // v761（#161 C5）：等待交互泡泡（kind:'wait'）从布场景到渲染都是同步完成的，所以真正的
    // "已显示"标记放在这里取；其它场景/关闭路径会把 bubbleScene 置空或换 kind，因此
    // 该标记不可能残留成"其实没显示却以为在显示"。
    waitShown = !!(bubbleScene && bubbleScene.kind === 'wait')
    bubbleLiveMods = Array.isArray(mods) ? mods.slice() : null
    // 清掉旧模块行,隐藏老三行与 gif(它们仍保留在 DOM 供普通场景使用)
    gifEl.style.display = 'none'
    labelEl.style.display = 'none'
    amountEl.style.display = 'none'
    hintEl.style.display = 'none'
    bubbleRowsTo(textBox, mods)
  } catch (err) {}
}
// 编辑器真实泡泡预览:用与真实相同的宽度 B 排版(保证换行判定一致),
// 再整体 CSS 缩放放入卡片,因此预览与实际逐像素同排版
function bubblePreviewInto(container, mods, widthPx) {
  try {
    if (!container) return
    container.innerHTML = ''
    // 真实泡泡当前宽度(挂件缩放后)
    var B = Math.max(120, (root && (root.offsetWidth || root.getBoundingClientRect().width)) || 300)
    // 按父容器可视宽度等比显示(不再用固定 408 缩放),视觉上在卡片内居中
    var hostW = Math.max(120, (container.parentNode && (container.parentNode.clientWidth || container.parentNode.getBoundingClientRect().width)) || 408)
    var W = Math.max(120, Math.min(B, hostW))
    container.style.width = W + 'px'
    container.style.transform = 'none'
    container.style.transformOrigin = ''
    container.style.setProperty('--dshw-u', (W / 1026) + 'px')
    // v751（PR #119）：预览节点在挂件 root 之外，拿不到 root 上的 --dshw-vx/--dshw-vy（以前写在 <html> 上才吃得到），
    // 这里补一份，保证编辑器里的预览与真实泡泡同排版。
    try {
      container.style.setProperty('--dshw-vx', dshwCenterX + '%')
      container.style.setProperty('--dshw-vy', dshwCenterY + '%')
    } catch (err) {}
    // 视觉右移 10px:用左右 margin 的非对称(右侧少让),避免溢出撑出横向滚动条
    var halfGap = Math.max(0, (hostW - W) / 2)
    var shiftR = Math.min(10, Math.max(0, Math.round(halfGap)))
    container.style.marginLeft = (Math.max(0, Math.round(halfGap)) + shiftR) + 'px'
    container.style.marginRight = Math.max(0, Math.round(halfGap) - shiftR) + 'px'
    var pop = document.createElement('div')
    pop.className = 'dshwv-minipop'
    pop.style.aspectRatio = 'auto'
    // 预览仅保留大泡泡主体区域:裁掉底部(两个小泡泡已隐藏),下方按钮随之靠上
    var cropTop = Math.max(2, Math.round(W * 0.012)) // 大泡泡上边距
    pop.style.height = (Math.round(W * 560 / 1026) + cropTop) + 'px'
    pop.style.overflow = 'hidden'
    var stage = document.createElement('div')
    stage.style.position = 'absolute'
    stage.style.left = '0'
    stage.style.top = cropTop + 'px'
    stage.style.width = '100%'
    stage.style.height = Math.round(W * 700 / 1026) + 'px'
    try {
      var svgEl = bubbleBox.querySelector('svg')
      if (svgEl) stage.innerHTML = svgEl.outerHTML
    } catch (err) {}
    // 预览:隐藏底部两个小泡泡(尾巴),仅预览不影响真实泡泡
    try {
      var tailEls = stage.querySelectorAll('.dshwv-b1, .dshwv-b2')
      for (var t1 = 0; t1 < tailEls.length; t1++) { try { tailEls[t1].style.display = 'none' } catch (err) {} }
    } catch (err) {}
    var tb = document.createElement('div')
    // 与真实泡泡完全一致:复用 .dshwv-text,位置/字号/换行全用真实 CSS 与 u
    tb.className = 'dshwv-text'
    tb.style.opacity = '1'
    tb.style.transition = 'none'
    stage.appendChild(tb)
    pop.appendChild(stage)
    container.appendChild(pop)
    bubbleRowsTo(tb, mods || [])
  } catch (err) {}
}
// 点击鲸鱼:
//  - 消耗泡泡存在时优先级最高:忽略点击(点泡泡才关)
//  - 当前无泡泡:开新轮从第 1 次点击开始
//  - 正显示第 2 次及以后的泡泡:回到序列开头(重开第 1 次点击泡泡)
//  - 正显示第 1 次点击泡泡:不切换内容,仅重置留存计时
function whaleClick() {
  try {
    if (!bubbleOn) return
    if (bubbleScene && (bubbleScene.kind === 'cost' || bubbleScene.kind === 'alert')) return // 消耗/预警提醒期间点鲸鱼不动作(点泡泡才关)
    if (!bubbleShown) {
      bubbleRoundOn = true
      bubbleSeqIdx = 0
      bubbleShowSeqNext()
      return
    }
    // v777：等待交互提示期间点角色 —— 开关「点按角色关闭提示气泡」打开时，点角色＝收起这条等待提示
    //（默认关：保持"点角色不动等待提示"，只有点泡泡才关）。收起来后 pollWaitState 不会把它弹回来。
    if (bubbleScene && bubbleScene.kind === 'wait') {
      if (waitCharCloseOn()) dismissWaitBubble()
      return
    }
    // v727：开启「点按角色推进泡泡队列」→ 点角色＝往后推进一项（不再回到第 1 项）；
    // 已是最后一项时与「点泡泡」一致：收起泡泡，下次点按从第 1 项开始。
    if (bubbleTapAdvance) { bubbleNext(); return }
    if (!bubbleRoundOn) return // 非手动轮场景:不动
    if (bubbleSeqIdx <= 1) {
      // 正显示第 1 次点击泡泡:仅重置留存计时(续时)
      bubbleResetTtl()
      return
    }
    // 第 2 次及以后 → 回到序列开头(第 1 次点击)
    bubbleSeqIdx = 0
    bubbleShowSeqNext()
  } catch (err) {}
}
// 点击泡泡:跳到下一项;已是最后一项则关闭
function bubbleNext() {
  try {
    if (!bubbleShown) return
    if (bubbleScene && bubbleScene.kind === 'cost') { hideCostBubble(); return }
    if (bubbleScene && bubbleScene.kind === 'alert') { hideUsageAlertBubble(); return }
    // v777：等待交互泡泡**可以点掉**（用户反馈「无法点击关闭」）。原来的实现在这里是 `return`
    //（设计意图是"别让误点把'正在等待你回答'收掉"），但真机上用户就是想把它点掉 ⇒ 改成：
    // 点泡泡 = 收起，并记住"这条挂起已被点掉"，pollWaitState 不再自动弹回（回答/批准或下一次新挂起照常）。
    if (bubbleScene && bubbleScene.kind === 'wait') { dismissWaitBubble(); return }
    // v761（#161 C5）曾在此处"等待交互泡泡点它不关"（必须等被回答/批准）——
    // v777 按用户反馈改成"可点关"，语义与实现见上面的 dismissWaitBubble()；这里不再拦截。
    if (bubbleRoundOn && bubbleSeqIdx < bubbleSeq.length) { bubbleShowSeqNext(); return }
    hideBubble()
  } catch (err) {}
}
// —— 覆盖旧实现 ——
function showBubble() {
  // 通用打开(余额变化等事件):当作“开新轮从第1项”,打断当前手动/随机内容
  if (!bubbleOn) return
  if (costBubbleActive) return
  bubbleRoundOn = true
  bubbleSeqIdx = 0
  bubbleShowSeqNext()
}
function hideBubble() {
  bubbleClearAll()
  bubbleRoundOn = false
  bubbleSeqIdx = 0
  bubbleScene = null
  bubbleShown = false
  bubbleRandomActive = false
  bubbleRandomLines = null
  lastHintText = null
  bubbleCloseVisual()
}
function showCostBubble(amount) {
  if (!bubbleOn || !turnCostOn) return
  // v778：②区「冒泡提示」勾选框（events.turnCost.bubbleOn）以前**只写不读** —— 取消勾选照样弹。
  //   同类问题（"控件有值、没有消费方"）与本次修的音量滑块同源，一并接上：
  //   缺省（未设置/旧数据）= 照常冒泡，只有**显式**取消勾选才不弹 ⇒ 老用户行为不变。
  try {
    var tcCfg = soundEventCfg('turnCost') || {}
    if (tcCfg.bubbleOn === false) return
  } catch (err) {}
  // 进入系统泡泡队列(等级3):若同批有预警/预算,则排在它们之后展示
  whaleSysPush({ kind: 'cost', amount: amount, rank: 3 })
}
function hideCostBubble() {
  // 与手动点击“第 n 次点击”一致:有下一项时保持打开淡切,无下一项才收起
  if (whaleSysSwapNext()) return
  bubbleClearAll()
  costBubbleActive = false
  bubbleScene = null
  bubbleShown = false
  bubbleRandomActive = false
  bubbleRandomLines = null
  bubbleCloseVisual()
  whaleSysDone()
}
// —— 系统泡泡优先队列:今日预算(1) > 余额预警(2) > 本轮消耗(3) ——
// 同批(约 30ms 窗口内)触发的多个系统泡泡按等级依次展示;已展示项结束(关闭/超时)再播下一项
var USAGE_ALERT_TTL = 6500 // 提醒泡泡停留毫秒数
var whaleSysQueue = []
var whaleSysItem = null // 当前正展示的 {kind, mods/amount, rank}
var whaleSysTimer = null
// 队列插入（v772 起抽成函数，供 push / 抢占退回 共用）：
//   rank 小的先出（1 今日预算 / 2 余额预警 / 3 每轮消耗 / 4 等待交互）；
//   **同档的「每轮消耗」后入先出** —— 新的一轮消耗插到已有消耗项之前 ⇒ 新的置顶，旧的排队；
//   其它档位保持先入先出（预警/预算按发生顺序看更合理），等待交互项永远排在最前。
function whaleSysQueueInsert(item) {
  var rank = Number(item.rank)
  if (!(rank >= 1)) rank = 2
  item.rank = rank
  var pos = whaleSysQueue.length
  for (var i = 0; i < whaleSysQueue.length; i++) {
    var q = whaleSysQueue[i] || {}
    var qr = Number(q.rank)
    if (!(qr >= 1)) qr = 2
    if (qr > rank || (item.kind === 'cost' && q.kind === 'cost' && qr === rank)) { pos = i; break }
  }
  whaleSysQueue.splice(pos, 0, item)
  return pos
}
// 展示一项（v772 抽成函数：tick / 淡切下一项 / 抢占都走这一份，避免三处各写一遍又不一致）
function whaleSysOpenItem(item) {
  whaleSysItem = item
  if (item.kind === 'cost') {
    sceneOpen('cost', function () { bubbleRenderCostMods(item.amount) }, turnCostCloseMs > 0 ? turnCostCloseMs : 0)
  } else if (item.kind === 'wait') {
    // 等待交互：常驻（ttl 0 = sceneOpen 不布自动关闭计时器），内容在渲染时按当前设置取
    sceneOpen('wait', function () { bubbleRenderWaitMods(item.waitKind) }, 0)
  } else {
    sceneOpen('alert', function () { bubbleRenderModules(item.mods || []) }, (item && item.ttlMs != null) ? item.ttlMs : USAGE_ALERT_TTL)
  }
}
function whaleSysPush(item) {
  try {
    if (!bubbleOn || !bubbleBox || !textBox) return false
    if (!item || !item.kind) return false
    // v772：**等待交互（授权/提问）享有最高优先级，永远不会被"消耗泡泡正开着"挡住**。
    // 整轮对话都卡在等你回答/批准，它必须立刻可见；正在展示的其它系统泡泡（消耗/预警）退回队列，
    // 等挂起解除后照旧继续 —— 用户真机反馈「已有每轮消耗提示时，授权与提问不会显示」。
    // 顺带兜住历史上的一个坑：`costBubbleActive` 万一残留成 true 而 `whaleSysItem` 为空，
    // 下面的 `costBubbleActive && !whaleSysItem` 早退会把所有入队都吃掉（等待泡泡一起被丢）。
    if (item.kind === 'wait') {
      if (whaleSysItem && whaleSysItem.kind === 'wait') {
        if (whaleSysItem.waitKind === item.waitKind) return true // 已在显示同一类：内容无需重渲染
        whaleSysItem = item
        sceneOpen('wait', function () { bubbleRenderWaitMods(item.waitKind) }, 0)
        return true
      }
      if (whaleSysItem) {
        var pre = whaleSysItem
        whaleSysItem = null
        whaleSysQueueInsert(pre) // 退回队列（按 rank + 消耗 LIFO 规则就位）
      }
      whaleSysOpenItem(item)
      return true
    }
    // 正在展示消耗泡泡时,新提醒退化为居中卡片(避免与消耗内容抢层);排队中的则按序等
    if (costBubbleActive && !whaleSysItem) return false
    // v772：**每轮消耗改后入先出** —— 新一轮消耗直接置顶，原来那条退回队列（点击时按"新→旧"回看）。
    if (item.kind === 'cost' && whaleSysItem && whaleSysItem.kind === 'cost') {
      var pre2 = whaleSysItem
      whaleSysItem = null
      whaleSysQueueInsert(pre2)
      whaleSysOpenItem(item)
      return true
    }
    whaleSysQueueInsert(item)
    if (whaleSysTimer) { clearTimeout(whaleSysTimer); whaleSysTimer = null }
    whaleSysTimer = setTimeout(whaleSysTick, 30)
    return true
  } catch (err) { return false }
}
function whaleSysTick() {
  whaleSysTimer = null
  try {
    if (!bubbleOn || !bubbleBox || !textBox) { whaleSysQueue = []; whaleSysItem = null; return }
    if (whaleSysItem) return
    if (!whaleSysQueue.length) return
    // 存在尚未入队的正展示系统泡?仅在鲸鱼空闲且无我们自己队列项时取下一个
    // v761（#161 C5）：waitShown 也必须算"正被占着"——等待泡泡是常驻的（whaleSysItem 会一直是它），
    // 正常走不到这里；但万一 item 被别的路径清掉而场景还停在等待内容上，也不能抢它的层。
    if (costBubbleActive || waitShown || (bubbleScene && bubbleScene.kind === 'alert')) return
    var item = whaleSysQueue.shift()
    if (!item) return
    whaleSysOpenItem(item) // v772：展示逻辑抽成一份（tick / 淡切 / 抢占共用）
  } catch (err) {}
}
function whaleSysDone() {
  try {
    whaleSysItem = null
    if (whaleSysTimer) { clearTimeout(whaleSysTimer); whaleSysTimer = null }
    whaleSysTick()
  } catch (err) {}
}
// 自动超时且有下一项时:泡泡保持打开,内容平滑切换(复用 sceneOpen 的淡出/淡入)
function whaleSysSwapNext() {
  try {
    if (!whaleSysQueue.length || !bubbleOn || !bubbleShown) return false
    var item = whaleSysQueue.shift()
    if (!item) return false
    whaleSysOpenItem(item) // v772：与 tick / 抢占同一份展示逻辑
    return true
  } catch (err) { return false }
}
function hideUsageAlertBubble() {
  // 与手动点击“第 n 次点击”一致:有下一项时保持打开淡切,无下一项才收起
  if (whaleSysSwapNext()) return
  bubbleClearAll()
  bubbleScene = null
  bubbleShown = false
  bubbleRandomActive = false
  bubbleRandomLines = null
  bubbleLiveMods = null
  bubbleCloseVisual()
  whaleSysDone()
}
// ===== v761（#161 C5）：等待交互的**常驻**泡泡（提问 / 授权）=====
// 语义与消耗(3)/预警(2)/预算(1)三档系统泡泡不同：那几档是"显示 N 秒自动关"，而挂起泡泡要
// **在挂起存在期间一直显示**，直到宿主在 wait.json 里说挂起解除了 → pollWaitState() 调 hideWaitBubble()。
// 复核后的实现方式（改动面最小的那一版）：
//   · 复用既有系统泡泡链路（whaleSysPush / whaleSysTick / whaleSysSwapNext / whaleSysDone），
//     不另造浮层；新增第四档 rank = 4（排在消耗之后，理由见 showWaitBubble 注释）；
//   · 常驻 = **ttlMs 传 0**。sceneOpen 里 `if (ttlMs > 0) bubbleTtlTimer = setTimeout(...)`，
//     所以 0 天然"不布自动关闭计时器"；bubbleResetTtl 也带 `ttlMs > 0` 守卫，不会给它补上计时器。
//     既有 cost/alert 的取值（turnCostCloseMs / usageRemindTtlMs）完全没动。
//   · 队列项 kind = 'cost' | 'alert' | 'wait'；'wait' 这一支只出现在 whaleSysTick /
//     whaleSysSwapNext / bubbleAutoClose / bubbleNext 的**新增分支**里，alert/budget/cost 路径逐字未改。
// 幂等 + 换类型（**v771 修掉真机卡死**）：等待泡泡**永远只有一条**，内容 = 宿主当前上报的那一条挂起。
// 依据：宿主 `waitState` 也只有一个槽位（`notePendingEvent` 里每次 `waitState.pending = {...}` 覆盖），
// 所以客户端"镜像最新一条"就与宿主同口径，不需要（也不能）排队多条常驻泡泡。
// ⚠️ 旧实现遇到"正在显示 A 类、又来了 B 类"时：把 `whaleSysItem` 置空 → 把 A 项 `unshift` 回队首 → 再 push B。
//    可屏幕上**仍在显示等待场景**（`waitShown` 依旧 true）⇒
//      ① `whaleSysTick` 的 `if (waitShown) return` 守卫**永远**拦住队列（B 永不上屏、A 也不换）；
//      ② `whaleSysItem` 已是 null ⇒ `hideWaitBubble()` 的 `if (whaleSysItem && kind==='wait')` 分支走不到
//         ⇒ 之后宿主说"没有挂起了"也**关不掉** ⇒ 泡泡一直停在 A，直到刷新页面。
//    真机复现：先授权、再提问（1 秒轮询很可能错过中间"无挂起"的那一瞬）⇒ 泡泡停在授权、提问泡泡不出现、
//    对话结束也不消失。
function showWaitBubble(kind, p) {
  try {
    if (!bubbleOn) return false // 全局「泡泡总开关」为假 → 一律不显示
    var k = (kind === 'approval') ? 'approval' : 'question'
    var cfg = waitEventCfg(k)
    // 门控：该事件总开关 + 该事件「冒泡提示」开关（任一为假都不冒泡）；pollWaitState 也会先判一次
    if (cfg.on === false || cfg.bubbleOn === false) return false
    if (!bubbleBox || !textBox) return false
    var pid = (p && p.id) ? String(p.id) : ''
    // "正显示等待泡泡"看**场景**（权威）或已占位的队列项（次之）—— 二者取或，避免状态半同步时漏判
    var showingWait = !!((bubbleScene && bubbleScene.kind === 'wait') || (whaleSysItem && whaleSysItem.kind === 'wait'))
    if (showingWait) {
      // 同一类：幂等丢弃（内容一样）。只是 pendingId 变了就顺手更新，便于排查。
      if (whaleSysItem && whaleSysItem.kind === 'wait' && whaleSysItem.waitKind === k) {
        if (pid) whaleSysItem.pendingId = pid
        return false
      }
      // 不同类（或状态半同步）：**原地替换** —— 不动 waitShown、不置空 whaleSysItem、不往队列塞，
      // 直接复用 sceneOpen 的内容淡切把这一条常驻泡泡换成新的挂起内容。
      whaleSysQueue = whaleSysQueue.filter(function (q) { return !(q && q.kind === 'wait') })
      whaleSysItem = { kind: 'wait', waitKind: k, rank: 4, ttlMs: 0, pendingId: pid }
      sceneOpen('wait', function () { bubbleRenderWaitMods(k) }, 0)
      return true
    }
    // 还没在显示等待泡泡（可能正在放消耗/预警，或压根没泡泡）：队列里只保留"最新一条"等待项
    whaleSysQueue = whaleSysQueue.filter(function (q) { return !(q && q.kind === 'wait') })
    // rank 4：1 今日预算 / 2 余额预警 / 3 每轮消耗 / 4 等待交互。挂在消耗之后是刻意的 ——
    // 等待泡泡一旦显示就**常驻**（不会自己让位），所以必须让"来了就要看、会自己消失"的消耗泡泡先走完；
    // 否则一次消耗提醒会被常驻的等待泡泡永久挡在队里。既有 30ms 合批排序逻辑不用改。
    return whaleSysPush({ kind: 'wait', waitKind: k, rank: 4, ttlMs: 0, pendingId: pid })
  } catch (err) { return false }
}
// 挂起解除（被回答 / 被批准 / 切换对话）：收起等待泡泡。
// 队列里还有别的项时保持打开、淡切到下一项 —— 与 hideCostBubble / hideUsageAlertBubble 同语义，
// 复用它们的做法：**不**把正在显示的那一项摘掉，交给 whaleSysSwapNext() 去换或去清场。
function hideWaitBubble() {
  try {
    // v771：判断"当前是不是正显示等待泡泡"必须**同时看场景与队列项**。历史上只看 whaleSysItem，
    // 一旦它与场景不同步（showWaitBubble 换类型的旧实现就会造成）就永远走不到关闭分支 ⇒ 泡泡卡死。
    var showingWait = !!((bubbleScene && bubbleScene.kind === 'wait') || (whaleSysItem && whaleSysItem.kind === 'wait') || waitShown)
    var beforeLen = whaleSysQueue.length
    // 排队中但还没轮到的等待项作废（等待项永远只保留最新一条，见 showWaitBubble）
    whaleSysQueue = whaleSysQueue.filter(function (q) { return !(q && q.kind === 'wait') })
    if (showingWait) {
      if (whaleSysSwapNext()) return // 队列里还有别的项 → 淡切过去（waitShown 由渲染侧重算）
      waitShown = false
      whaleSysItem = null
      bubbleClearAll()
      bubbleScene = null
      bubbleShown = false
      bubbleRandomActive = false
      bubbleRandomLines = null
      bubbleLiveMods = null
      bubbleCloseVisual()
      whaleSysDone()
      return
    }
    // 没在显示等待泡泡：只清掉了排队中的等待项 → 补一次 tick，让别的项继续走
    if (whaleSysQueue.length !== beforeLen) whaleSysTick()
  } catch (err) {}
}
// v777（用户反馈「授权提示/提问提示无法点击关闭」）：
//   · 默认：**点泡泡**即可关掉等待气泡（点角色不管，除非打开下面的开关）；
//   · 开关「点按角色关闭提示气泡」打开后：**点角色**也能关；
//   · 关掉的是"这条挂起"——宿主每秒还会下发同一条挂起（pollWaitState 是每分钟…不是，是每秒对齐一次），
//     所以必须记住"用户把这条点掉了、别再自动弹回"，否则关掉后 1 秒内又回来 = 看起来根本关不掉。
var waitDismissedId = '' // 被用户点掉的挂起 id（'' = 没有）
var waitPendingId = '' // 宿主最近一次上报的挂起 id（用于"点掉的就是当前这条"）
function waitCharCloseOn() {
  try { return !!(((usageSet || {}).wait || {}).charClose) } catch (err) { return false }
}
// 用户主动收起等待气泡：记住这条挂起已被点掉，然后收起
function dismissWaitBubble() {
  try {
    if (waitPendingId) waitDismissedId = waitPendingId
    hideWaitBubble()
  } catch (err) {}
}
function clamp(v, lo, hi) { return v < lo ? lo : (v > hi ? hi : v) }
function viewport() {
  return {
    w: window.innerWidth || document.documentElement.clientWidth || 1280,
    h: window.innerHeight || document.documentElement.clientHeight || 800
  }
}
function rightGap() {
  // 开关关闭：贴边（不避让滚动条）
  if (!scrollGapOn) return 0
  // 开启：用用户填写的像素；填 0 也贴边
  return scrollGapPx > 0 ? scrollGapPx : 0
}
function fmt(balance, currency) {
  if (balance === null || balance === undefined || balance === '') return '—'
  var num = Number(balance)
  var fixed = isFinite(num) ? num.toFixed(2) : '--'
  return currency === 'CNY' ? '¥ ' + fixed : fixed + ' ' + currency
}
function animateAmount(from, to, currency, duration) {
  // 消耗金额泡泡显示期间，余额数字滚动不触碰金额行
  if (costBubbleActive) return
  if (animId) cancelAnimationFrame(animId)
  if (from === null || !isFinite(from)) from = to
  if (from === to) {
    shown = to
    amountEl.textContent = fmt(to, currency)
    return
  }
  var startTime = null
  function step(ts) {
    // 帧级保护：成本泡泡出现后立即停止滚动，避免后续帧把余额写进金额行
    if (costBubbleActive) {
      animId = null
      return
    }
    if (startTime === null) startTime = ts
    var t = Math.min(1, (ts - startTime) / duration)
    var eased = 1 - Math.pow(1 - t, 3)
    var val = from + (to - from) * eased
    amountEl.textContent = fmt(val, currency)
    if (t < 1) {
      animId = requestAnimationFrame(step)
    } else {
      animId = null
      shown = to
      amountEl.textContent = fmt(to, currency)
    }
  }
  animId = requestAnimationFrame(step)
}
function render() {
  // 消耗金额泡泡显示期间，余额渲染不覆盖其内容（金额行/标题行/提示行）
  if (costBubbleActive) return
  var amount, hint
  if (state.status === 'error') {
    amount = shown !== null ? fmt(shown, state.currency) : '--'
    hint = state.message ? state.message.slice(0, 14) : '获取失败 · 点击重试'
  } else if (state.balance === null) {
    amount = shown !== null ? fmt(shown, state.currency) : '…'
    hint = '加载中…'
  } else {
    amount = shown !== null ? fmt(shown, state.currency) : fmt(state.balance, state.currency)
    hint = (state.usageLabel || '今日已用') + ' ' + (state.todayUsage !== null && state.todayUsage !== undefined ? fmt(state.todayUsage, state.todayUsageCurrency || state.currency) : '--')
  }
  amountEl.textContent = amount
  if (bubbleRandomActive && bubbleRandomLines) {
    applyBubbleLines(bubbleRandomLines)
  } else {
    setHint(hint)
  }
  // 注意:不在泡泡显示期间整泡重绘(内容在用户观看时保持稳定);
  // 数值/峰谷等更新由“泡泡消失→下一次显示”时的渲染自然采用最新 state
}
function express() {
  root.style.right = 'auto'
  root.style.bottom = 'auto'
  root.style.left = state.left + 'px'
  // v785：最后一道保险 —— 所有"写 state.top"的路径最终都会走到这里，统一把顶边夹出"窗口控件带"
  // （拖动/吸附/settle 各自也夹了一次；这里兜住启动、自愈、外部直接改 state 等路径）。
  // 见 clampWidgetTop：下限 = 叠层下沿（普通浏览器为 0 ⇒ 与以前完全一致）。
  state.top = clampWidgetTop(state.top)
  root.style.top = state.top + 'px'
  root.classList.toggle('dshwv-left', !!state.flip)
}
function settle() {
  var vp = viewport()
  var w = root.offsetWidth || root.getBoundingClientRect().width || 0
  var h = root.offsetHeight || root.getBoundingClientRect().height || 0
  if (drag && drag.active) {
    // mid-drag resize: keep the pointer-follow position, just clamp into view
    state.left = clamp(state.left, 0, Math.max(0, vp.w - w - rightGap()))
    state.top = clampWidgetTop(state.top, Math.max(0, vp.h - h))
    express()
    return
  }
  // issue #102：锚点分支过去只有下限（左/顶锚甚至完全不夹），脏锚点或尺寸竞态会把挂件算到
  // 视口外 —— 症状是「启动闪一下 → 向下滑出/镜像 → 消失」，而且 saveConfig 会把负的离边距离
  // 写回 localStorage，于是刷新也恢复不了。四个锚点分支统一夹到可视区。
  var maxLFree = Math.max(0, vp.w - w - rightGap())
  var maxLAnchor = Math.max(0, vp.w - w)
  var maxT = Math.max(0, vp.h - h)
  var outOfRange = false
  if (state.h === 'right') {
    var rawR = vp.w - w - state.hOff - rightGap()
    state.left = clamp(rawR, 0, maxLAnchor)
    if (state.left !== rawR) outOfRange = true
  } else if (state.h === 'left') {
    var rawL = state.hOff
    state.left = clamp(rawL, 0, maxLAnchor)
    if (state.left !== rawL) outOfRange = true
  } else {
    state.left = clamp(state.left, 0, maxLFree)
  }
  if (state.v === 'bottom') {
    var rawB = vp.h - h - state.vOff
    state.top = clampWidgetTop(rawB, maxT)
    if (state.top !== rawB) outOfRange = true
  } else if (state.v === 'top') {
    var rawT = state.vOff
    state.top = clampWidgetTop(rawT, maxT)
    if (state.top !== rawT) outOfRange = true
  } else {
    state.top = clampWidgetTop(state.top, maxT)
  }
  // 偏移确实越界（脏数据 / 视口变小）：先把偏移夹回合法范围再落盘，下次启动不再复现。
  // 只在真的越界时写，正常 resize 不会产生额外的 localStorage 写入。
  if (outOfRange) {
    if (state.h === 'right') state.hOff = clamp(state.hOff, 0, Math.max(0, vp.w - w - rightGap()))
    else if (state.h === 'left') state.hOff = clamp(state.hOff, 0, maxLAnchor)
    if (state.v === 'bottom') state.vOff = clamp(state.vOff, 0, Math.max(0, maxT - widgetTopMin()))
    else if (state.v === 'top') state.vOff = clamp(state.vOff, widgetTopMin(), maxT)
    try { saveAnchorPos() } catch (err) {}
  }
  refreshFlip()
}
// 阈值坐标（px）：按当前配置计算四条吸附区边界与翻转线的屏幕坐标；'off' 返回全屏默认。
function snapBounds(vp) {
  var b = { L: 0, T: 0, R: vp.w, B: vp.h, F: vp.w / 2 }
  try {
    var cfg = snapConfig
    if (!cfg || cfg.mode === 'off') return b
    if (cfg.mode === 'px') {
      b.L = cfg.px.L
      b.T = cfg.px.T
      b.R = vp.w - cfg.px.R
      b.B = vp.h - cfg.px.B
      b.F = cfg.px.F
    } else {
      b.L = vp.w * cfg.ratio.L / 100
      b.T = vp.h * cfg.ratio.T / 100
      b.R = vp.w * (100 - cfg.ratio.R) / 100
      b.B = vp.h * (100 - cfg.ratio.B) / 100
      b.F = vp.w * cfg.ratio.F / 100
    }
  } catch (err) {}
  return b
}
// 吸附判定。判定点：cx = 图像中心 x（左右吸附区 + 翻转线）；
// cyBox = 挂件盒中心 y（顶吸附区）；cyImg = 图像中心 y（底吸附区）。
// 返回 { zH:'left'|'right'|null, zV:'top'|'bottom'|null, flip:boolean }；'off' 全自由且不翻转。
function snapZones(cx, cyBox, cyImg, vp) {
  var out = { zH: null, zV: null, flip: false }
  try {
    var cfg = snapConfig
    if (!cfg || cfg.mode === 'off') return out
    var b = snapBounds(vp)
    out.flip = cx < b.F
    if (cx < b.L) out.zH = 'left'
    else if (cx > b.R) out.zH = 'right'
    if (cyBox < b.T) out.zV = 'top'
    else if (cyImg > b.B) out.zV = 'bottom'
  } catch (err) {}
  return out
}
// 依据当前挂件几何重算翻转态并应用（express 内部按 state.flip 切 dshwv-left）。
// 贴左/贴右锚定时翻转跟随吸附边（贴左必翻、贴右不翻，保证朝向屏幕内）；
// 自由摆放时按“图像中心”与翻转线判定（判断点 = 图像中心而非挂件盒中心）。
function refreshFlip() {
  try {
    if (state.h === 'left') {
      state.flip = true
    } else if (state.h === 'right') {
      state.flip = false
    } else {
      var w = root.offsetWidth || root.getBoundingClientRect().width || 0
      var h = root.offsetHeight || root.getBoundingClientRect().height || 0
      var ac = artCenterAt(state.left, state.top, w, h, !!state.flip)
      var vp = viewport()
      // 自由摆放：翻转只看图像中心的 x 与翻转线
      state.flip = ac.cx < snapBounds(vp).F
    }
    express()
  } catch (err) {}
}
// 计算图像（鲸鱼 img 元素）的中心点：img 占挂件盒右下 59.45%（盒为正方形），
// 翻转时整个盒镜像，图像视觉中心随之翻到左侧。
// flipped=false → 图像在盒内右侧；flipped=true → 镜像后在盒内左侧。
function artCenterAt(left, top, w, h, flipped) {
  var iw = Math.max(1, w * 0.5945)
  var cx = flipped ? left + iw / 2 : left + w - iw / 2
  var cy = top + h - iw / 2
  return { cx: cx, cy: cy }
}
// v739（用户反馈「每次新实例的第一次余额请求都失败」）：冷启动时凭据服务可能还没就绪，
// 首次 DNS+TLS 也最慢；而客户端 25s 超时会先于宿主的两段重试结束 —— 结果是第一次必失败、
// 只能干等 60 秒后的下一轮。这里失败后快速重试两次（1.5s / 3s），成功即重置计数。
var balanceRetryLeft = 2
function balanceRetryLater() {
  if (balanceRetryLeft <= 0) return
  var delay = balanceRetryLeft === 2 ? 1500 : 3000
  balanceRetryLeft--
  setTimeout(function () { try { refresh(false) } catch (err) {} }, delay)
}
function refresh(manual) {
  if (busy) return
  busy = true
  if (animDelayTimer) { clearTimeout(animDelayTimer); animDelayTimer = null }
  if (manual || state.balance === null) { state.status = 'loading'; render() }
  var ctrl = null
  var timer = null
  try {
    ctrl = new AbortController()
    timer = setTimeout(function () { try { ctrl.abort() } catch (err) {} }, FETCH_TIMEOUT_MS)
  } catch (err) {}
  fetch(BALANCE_URL + (manual ? '?refresh=1' : ''), { cache: 'no-store', signal: ctrl ? ctrl.signal : undefined })
    .then(function (r) { return r.json() })
    .then(function (data) {
      if (data && data.ok) {
        var nb = data.totalBalance == null ? null : Number(data.totalBalance)
        var nc = String(data.currency || 'CNY')
        var changed = state.balance !== null && (nb !== state.balance || nc !== state.currency)
        var currencyChanged = state.currency !== null && nc !== state.currency
        state.balance = nb
        state.currency = nc
        state.message = ''
        balanceRetryLeft = 2
        state.todayUsage = data.todayUsage !== undefined ? data.todayUsage : null
        state.todayUsageCurrency = data.todayUsageCurrency || data.currency || 'CNY'
        // v781：赠金 / 充值余额（宿主两条路都下发；缺失给 null ⇒ 模块显示 —）
        state.bonusBalance = data.bonusBalance != null && isFinite(Number(data.bonusBalance)) ? Number(data.bonusBalance) : null
        state.rechargeBalance = data.rechargeBalance != null && isFinite(Number(data.rechargeBalance)) ? Number(data.rechargeBalance) : null
        state.usageLabel = data.usageLabel || '本地估算'
        if (data.stale) state.usageLabel += ' · 余额未刷新'
        state.peakSupported = data.peakSupported === true
        state.isPeak = data.isPeak === null ? null : !!data.isPeak
        state.peakNextChangeAt = isFinite(Number(data.peakNextChangeAt)) ? Number(data.peakNextChangeAt) : null
        state.peakHolidays = Array.isArray(data.peakHolidays) ? data.peakHolidays : null
        checkUsageAlerts(nb, state.todayUsage)
        if (changed && !currencyChanged) {
          if (!manual) {
            showBubble()
            state.status = 'changing'
            // balance-change bubble: wait 0.3s after it floats out, then roll the number
            if (animDelayTimer) clearTimeout(animDelayTimer)
            animDelayTimer = setTimeout(function () {
              animDelayTimer = null
              animateAmount(shown, nb, nc, ANIM_MS)
            }, 300)
            if (settleTimer) clearTimeout(settleTimer)
            settleTimer = setTimeout(function () {
              settleTimer = null
              if (state.status === 'changing') { state.status = 'ok'; render() }
            }, CHANGE_MS + 300)
          } else {
            animateAmount(shown, nb, nc, ANIM_MS)
            state.status = 'ok'
            render()
          }
        } else {
          if (animId === null) shown = nb
          state.status = 'ok'
          render()
        }
      } else {
        state.status = 'error'
        state.message = (data && data.error) ? String(data.error) : '获取失败'
        // v781：把宿主的错误码也打到控制台 —— "只登录客户端读不到余额"这类反馈里，
        //   界面只显示一句人类可读的文案，而 code（NO_KEY / BOTH_FAILED / SHAPE …）才是排查入口。
        try { console.warn('[dsh-whale] 余额读取失败', (data && data.code) || '', state.message) } catch (err) {}
        render()
        balanceRetryLater()
      }
    })
    .catch(function () {
      state.status = 'error'
      state.message = '获取失败'
      render()
      balanceRetryLater()
    })
    .finally(function () {
      busy = false
      if (timer) clearTimeout(timer)
    })
}
var soundOn = codexWhaleHost.initialSound()
var soundVol = 0.9
var soundSet = 'duck'
var usageMode = 'ledger'
var peakMode = 'default'
var bubbleOn = true
var turnCostOn = true
var turnCostCloseMs = 5000
var costBubbleActive = false
var scrollGapOn = false
var scrollGapPx = 17
var menuBtnHide = false // 主菜单开关:隐藏挂件菜单按钮,改为右键小鲸鱼唤出菜单
// issue #116：Codex 本机统计开关。关掉后宿主完全不扫 ~/.codex/sessions（适合会话日志很大的机器）。
// v748：默认值仍是「开」，但**宿主会先看有没有 Codex 模型** —— 没配就一律按关闭处理，
// 所以这里保持默认 true 是安全的（不会让没配 Codex 的用户被扫盘），也不会因为一次普通保存
// 就把老用户的开关写成 false。开关 UI 现只在 Codex 模型的设置子菜单里（见 codexStatsCheckbox）。
var codexStatsOn = true
// —— v734（issue #97 / #88）：设置保存的「防覆盖 + 失败可见」——
// #97 根因：首次 GET 还没落地就 PUT，会把内存里的默认值整包写进服务端（重启后设置被洗成默认值）。
// #88 根因：这个 PUT 以前是 fire-and-forget，服务端 500 / {ok:false} 完全没人读。
var configLoaded = false        // 首次 GET 应用完成前，一律不 PUT
var configSavePending = false   // 加载期间被挡下的保存，加载完成后补一次
var dshwvToastEl = null
var dshwvToastTimer = null
// 固定定位的小提示条（自动消失；同一时刻只留一条）
function dshwvToast(msg) {
  try {
    if (!dshwvToastEl || !dshwvToastEl.parentNode) {
      var el = document.createElement('div')
      el.style.cssText = 'position:fixed;left:50%;bottom:28px;transform:translateX(-50%);z-index:2147483600;' +
        'max-width:min(560px,calc(100vw - 32px));box-sizing:border-box;padding:10px 14px;border-radius:10px;' +
        'background:#8a1f1f;color:#fff;font-size:13px;line-height:1.6;box-shadow:0 6px 20px rgba(0,0,0,.28);' +
        'pointer-events:none;text-align:center'
      dshwBodyAppend(el)
      dshwvToastEl = el
    }
    dshwvToastEl.innerHTML = msg
    if (dshwvToastTimer) clearTimeout(dshwvToastTimer)
    dshwvToastTimer = setTimeout(function () {
      // v744：toast 也是登记过的 body 节点，必须 detach（否则 DOM 守护会把它补挂回来 → 提示条永不消失）
      try { if (dshwvToastEl) dshwBodyDetach(dshwvToastEl) } catch (err) {}
      dshwvToastEl = null
    }, 8000)
  } catch (err) {}
}
function configSaveFailNotice(detail) {
  try { console.error('[dsh-whale] 设置保存失败:', detail) } catch (err) {}
  // v756（issue #143）：toast 是按 HTML 设计的（其余调用点传的字面量里带 `<br>`），而这里的 detail
  // 是**动态值**（服务端 JSON 回包的 error 字段 / fetch 异常消息），直接拼进 innerHTML 就是一处
  // 「动态数据进 HTML 位置」。当前来源都在本地信任边界内、构造不出真实利用，但把 provider 回包、
  // 模型名之类接进同一个 toast 时它会立刻变成真洞 —— 所以只转义这一个动态值，toast 自身的
  // `<br>` 保留。
  var safe = String(detail || '').slice(0, 120)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  dshwvToast('⚠ 设置保存失败：' + safe +
    '<br>已自动重试一次。若持续失败，请检查 Codex++ 数据目录是否可写。')
}
function configPayload() {
  return JSON.stringify({ scale: state.scale, sound: soundOn, vol: soundVol, soundSet: soundSet, usageMode: usageMode, peakMode: peakMode, bubbleOn: bubbleOn, turnCostOn: turnCostOn, turnCostCloseMs: turnCostCloseMs, scrollGapOn: scrollGapOn, scrollGapPx: scrollGapPx, menuBtnHide: menuBtnHide, codexStatsOn: codexStatsOn })
}
// 真正的 PUT：读响应 → 失败（网络异常 / HTTP!=200 / {ok:false}）静默重试一次 → 仍失败才提示
function configPut(payload, retried) {
  return fetch(SIZE_URL, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: payload })
    .then(function (r) {
      return r.json().catch(function () { return null }).then(function (d) { return { ok: r.ok, status: r.status, d: d } })
    })
    .then(function (x) {
      if (x.ok && (!x.d || x.d.ok !== false)) return true
      if (!retried) return new Promise(function (res) { setTimeout(function () { res(configPut(payload, true)) }, 900) })
      configSaveFailNotice((x.d && x.d.error) || ('HTTP ' + x.status))
      return false
    })
    .catch(function (err) {
      if (!retried) return new Promise(function (res) { setTimeout(function () { res(configPut(payload, true)) }, 900) })
      configSaveFailNotice((err && err.message) || err)
      return false
    })
}
function saveConfig() {
  // issue #97：加载完成前只记待办，绝不 PUT（否则把默认值整包写进服务端）
  if (!configLoaded) { configSavePending = true; return null }
  try {
    // 返回 Promise（v743：Codex 统计开关需要"等服务端确认后再刷新"）。
    // configPut 自带重试与失败提示、且链尾有 catch，所以不会有 unhandled rejection。
    var p = configPut(configPayload(), false)
    // 锚点位置记忆：记录相对边框的离边距离，窗口 resize 后保持（localStorage）。
    saveAnchorPos()
    return p
  } catch (err) { return null }
}
// 单独抽出：只写 localStorage 锚点（不碰尺寸设置）；applyAnchorPos / settle 自愈时也要用。
// v:2 = 净距离格式（剥离避让距离），v:1 旧格式含避让距离，恢复时废弃旧格式。
function saveAnchorPos() {
  try {
    var vp = viewport()
    var w = root.offsetWidth || root.getBoundingClientRect().width || 0
    var h = root.offsetHeight || root.getBoundingClientRect().height || 0
    var leftDist = isFinite(state.left) ? state.left : 0
    var rightDist = vp.w - leftDist - w
    var topDist = isFinite(state.top) ? state.top : 0
    var bottomDist = vp.h - topDist - h
    var hAnchor = leftDist <= rightDist ? 'left' : 'right'
    // issue #102：离边距离必须非负。挂件一旦被算到屏幕外，min(leftDist, rightDist) 就是负数，
    // 存进去会变成"永久坏锚点"，此后每次启动都复现（刷新也恢复不了）。
    var hDistRaw = Math.max(0, Math.round(Math.min(leftDist, rightDist)))
    var hDist = hAnchor === 'right' && scrollGapOn ? Math.max(0, hDistRaw - rightGap()) : hDistRaw
    localStorage.setItem('dshw-pos', JSON.stringify({
      v: 2,
      hAnchor: hAnchor,
      hDist: hDist,
      vAnchor: topDist <= bottomDist ? 'top' : 'bottom',
      vDist: Math.max(0, Math.round(Math.min(topDist, bottomDist)))
    }))
  } catch (err) {}
}
function setUsageMode(v) {
  usageMode = 'ledger' // 小鲸鱼记账为唯一记账方式(token 模式已下线)
  saveConfig()
  refresh(false)
}
function setBubbleOn(v) {
  bubbleOn = !!v
  bubbleToggle.checked = bubbleOn
  saveConfig()
  // 必须走 hideCostBubble：残留的 costBubbleActive 会让 render()/showBubble() 永久早退
  if (!bubbleOn) hideCostBubble()
}
function setTurnCostOn(v) {
  turnCostOn = !!v
  turnCostToggle.checked = turnCostOn
  turnCostCloseInput.disabled = !turnCostOn
  saveConfig()
  if (!turnCostOn) hideCostBubble()
}
// v720:「自定义提示」窗口打开期间,秒数先只改内存(turnCostCloseDefer),取消可还原
var turnCostCloseDefer = false
var turnCostCloseDeferSnap = 0
function setTurnCostClose(v) {
  // v720:去掉"提示关闭时不许改秒数"的早退 —— 「自定义提示」窗口里该输入是启用的,
  // 值要能存下来(启用开关在菜单行,开关关闭时该值只是暂不生效)。
  var n = Math.max(0, Math.round(Number(v) || 0))
  turnCostCloseMs = n * 1000
  turnCostCloseInput.value = String(n)
  if (turnCostCloseDefer) return // 窗口内:等「保存」再落盘
  saveConfig()
}
function setScrollGapOn(v) {
  scrollGapOn = !!v
  scrollGapToggle.checked = scrollGapOn
  scrollGapInput.disabled = !scrollGapOn
  saveConfig()
  settle()
}
function setScrollGapPx(v) {
  if (!scrollGapOn) return
  var n = Math.max(0, Math.round(Number(v) || 0))
  scrollGapPx = n
  scrollGapInput.value = String(n)
  saveConfig()
  settle()
}
// issue #91 缺陷2：触屏设备上没有任何进菜单的路径 —— ☰ 按钮默认 opacity:0，只由
// pointermove 命中鲸鱼时才加 dshwv-menu-btn-visible，而触摸端没有 hover；长按唤出又只在
// 「隐藏菜单按钮」开启时挂计时（默认关闭）。这里判定「主输入是否为无 hover 的触摸」：
// 只用 (hover: none)，或 (pointer: coarse) + 有触点。触屏笔记本（主输入是鼠标）不受影响。
function dshwvTouchUI() {
  try {
    if (typeof window !== 'undefined' && window.matchMedia) {
      if (window.matchMedia('(hover: none)').matches) return true
      if (window.matchMedia('(pointer: coarse)').matches && (navigator.maxTouchPoints || 0) > 0) return true
    }
    if (typeof window !== 'undefined' && typeof navigator !== 'undefined') {
      return (navigator.maxTouchPoints || 0) > 0 && ('ontouchstart' in window)
    }
  } catch (err) {}
  return false
}
function applyMenuBtnHideUI() {
  try {
    menuBtn.classList.toggle('dshwv-menu-btn-hidden', menuBtnHide)
    if (menuBtnHide) menuBtn.classList.remove('dshwv-menu-btn-visible')
    // 触屏：没有 hover 可以显形 → 常显（仍受上面的 hidden 开关控制）
    else if (dshwvTouchUI()) menuBtn.classList.add('dshwv-menu-btn-visible')
  } catch (err) {}
}
function setMenuBtnHide(v) {
  menuBtnHide = !!v
  if (menuHideToggle) menuHideToggle.checked = menuBtnHide
  saveConfig()
  applyMenuBtnHideUI()
}
// issue #116：Codex 本机统计开关。关掉后宿主不扫描 ~/.codex/sessions；
// 打开/关闭都要重取一次模型列表，让 Codex 那行的统计/提示立刻跟着变。
function setCodexStatsOn(v) {
  codexStatsOn = v !== false
  if (codexStatsToggle) codexStatsToggle.checked = codexStatsOn
  var after = function () {
    // v748：这里原来调的是 openApiModelPanel 内部的局部函数 refreshModelList()，
    // 在顶层作用域里根本不存在 → 异常被 try/catch 吞掉，"重取模型列表"其实从没发生过。
    // 现在按别处同一套做法刷新：模型列表 + （若开着）记账子界面，再原地刷新 Codex 用量行
    // （用量行的文字来自刚取回的模型列表，所以要等回调，不能立刻读旧缓存）
    try { loadApiModels(refreshOpenCodexRow, true) } catch (err) { refreshOpenCodexRow() }
    try { if (usagePanelOpen && usageSet !== null) rebuildUsageSubShell() } catch (err) {}
  }
  var p = null
  try { p = saveConfig() } catch (err) {}
  // 等服务端确认落盘后再重取模型列表：否则可能读到旧配置，那一行会先显示上一次的状态再跳变
  if (p && typeof p.then === 'function') p.then(after, after)
  else after()
}
function scaleToDisplay(s) {
  return Math.round((s - MIN_SCALE) / ((MAX_SCALE - MIN_SCALE) / 19)) + 1
}
function setScale(v) {
  var next = Math.round(Math.min(MAX_SCALE, Math.max(MIN_SCALE, Number(v))) * 10) / 10
  // 缩放测量需要 left/top 立即到位：临时禁用过渡（滚轮/数字框路径没有
  // 滑块 pointerdown 的 transition:none，否则 r2 测的是过渡起点导致错锚点）
  var prevTrans = root.style.transition
  root.style.transition = 'none'
  var rect = root.getBoundingClientRect()
  // fixed point: the whale's corner — bottom-right when unflipped, bottom-left
  // when flipped. Growing extends the widget up-left / up-right from that
  // corner; shrinking pulls it back toward the corner. The whale always hugs
  // its corner while scaling.
  var fx = state.flip ? rect.left : rect.right
  var fy = rect.bottom
  state.scale = next
  root.style.setProperty('--dshw-scale', String(next))
  scaleInput.value = String(next)
  scaleNumber.value = String(scaleToDisplay(next))
  saveConfig()
  // keep the corner fixed while resizing; the position correction applies
  // instantly because the caller disables the transition for the whole drag
  var r2 = root.getBoundingClientRect()
  var vp = viewport()
  if (state.flip) {
    state.left = Math.min(Math.max(fx, 0), Math.max(0, vp.w - r2.width))
  } else {
    state.left = Math.min(Math.max(fx - r2.width, 0), Math.max(0, vp.w - r2.width))
  }
  state.top = clampWidgetTop(Math.min(Math.max(fy - r2.height, 0), Math.max(0, vp.h - r2.height)))
  express()
  // 恢复过渡必须延迟到下一帧：本帧 left/top 已在 none 下设置并提交，
  // 立即恢复会让浏览器对「刚改过的 left/top」重新评估并播放过渡动画
  // （翻转时叠加 transform .3s 更明显，表现为抽搐）。
  requestAnimationFrame(function () {
    root.style.transition = prevTrans
  })
}
function setVol(v) {
  var next = Math.round(Math.min(1, Math.max(0, Number(v))) * 100) / 100
  soundVol = next
  // v753（issue #135）：**不再由音量派生 soundOn**。原来"拉到 0 就等于关音效"是个没写明的隐含行为，
  // 既让想临时静音的人被迫牺牲原来的音量值，也让"音效开关"这件事在界面上无迹可寻。
  // 现在开关自己说了算（见 setSoundOn），音量只表示音量。
  volInput.value = String(next)
  volPct.textContent = Math.round(next * 100) + '%'
  try {
    if (pressAudio) pressAudio.volume = next
    if (releaseAudio) releaseAudio.volume = next
  } catch (err) {}
  saveConfig()
}
// v753（issue #135）：显式音效总开关（菜单「音量」右边那个勾选框）。
// 它同时管三件事：① 出声与否（播放路径都看 soundOn）；② 关掉时**立刻挂起** AudioContext，
// 把系统睡眠交还给用户，而不是等 1 分钟空闲；③ 重新打开时立刻预热，恢复"点按即响"的跟手度。
function setSoundOn(v) {
  soundOn = v !== false
  try { soundToggle.checked = soundOn } catch (err) {}
  if (!soundOn) {
    dshwvAudioSuspendNow()
  } else {
    try { applySoundSet() } catch (err) {}
  }
  try {
    var p = saveConfig()
    if (p && typeof p.then === 'function') p.catch(function () {})
  } catch (err) {}
}
function setSoundSet(v) {
  // 支持预设组（duck/fx1）和自定义组 id
  soundSet = typeof v === 'string' && v ? v : 'duck'
  setAudioBtnText(audioGroupName(soundSet))
  applySoundSet()
  saveConfig()
}
var SQUISH = 'scaleY(0.88) scaleX(1.05)'
var pressAudio = null
var releaseAudio = null
var pressing = false
var pressEnded = false
var releasePlayed = false
// v745：不再需要 releaseTimer —— 点按时由 playReleaseAt() 在**音频线程**排期（见 pressUp）
// —— v752：本体按压/松开音改走「片段路由」——
// 起因：issue 里那台机器上 /dsh-whale/sound/press.mp3?set=… 会被本机的一层东西（代理/安全软件）
// 拦成**空的 204**（响应头里没有 Date、还多出 pragma: no-cache，不是本进程发出的），
// 0 字节送进 decodeAudioData 就抛 EncodingError → 只剩静音；
// 而同一台机器上 /dsh-whale/audio-fragment.wav?id=<片段> 是 200 且能解码。
// 两边的字节是**同一个文件**（预设 ya1→assets/Ya1.mp3；自定义组→它引用的那个片段），
// 所以音色、时长、衔接时序都不变；同时少一层"组→片段"的宿主侧间接。
// 返回 { url, alt, empty }：url=首选（片段路由）、alt=兜底（老的声音组路由，失败时自动切换）、
// empty=true 表示该槽显式留空（'' = 该事件静音，按设计不出声）。
function soundSlotUrls(slot, groupId) {
  var gid = String(groupId || soundSet || 'duck')
  if (audioGroupSlotEmpty(gid, slot)) return { url: '', alt: '', empty: true }
  var frag = ''
  for (var i = 0; i < audioGroups.length; i++) {
    var g = audioGroups[i]
    if (g && g.id === gid) { frag = String(g[slot] || ''); break }
  }
  var legacy = '/dsh-whale/sound/' + (slot === 'press' ? 'press' : 'release') + '.mp3?set=' + encodeURIComponent(gid)
  // 组信息还没到（audio.json 未返回）或找不到组 → 与旧行为完全一致，走老路由
  if (!frag) return { url: legacy, alt: '', empty: false }
  return { url: '/dsh-whale/audio-fragment.wav?id=' + encodeURIComponent(frag), alt: legacy, empty: false }
}
function applySoundSet() {
  try {
    // v729：切音效组 / 开关音效时把本轮播放状态一并复位，
    // 避免残留 releasePlayed=true 把新组的松开音整体吃掉（与 playPress 的修复配套）
    pressEnded = false
    releasePlayed = false
    // 槽位显式留空(该事件静音)时,对应音频元素置空;playPress/playRelease 已判空
    var pu = soundSlotUrls('press')
    var ru = soundSlotUrls('release')
    if (pu.empty) { pressAudio = null } else {
      pressAudio = dshwvSound(pu.url)
      pressAudio._alt = pu.alt || ''
      pressAudio.preload = 'auto'
      pressAudio.volume = soundVol
    }
    if (ru.empty) { releaseAudio = null } else {
      releaseAudio = dshwvSound(ru.url)
      releaseAudio._alt = ru.alt || ''
      releaseAudio.preload = 'auto'
      releaseAudio.volume = soundVol
    }
    // v745：把这组的按压/松开音**预取+预解码**（Web Audio 下 preload='auto' 不解码）。
    // 预热过之后，起播走 dshwvSound 的同步路径（pointerdown 同一任务里 start），手感才贴手。
    // ⚠️ 不变式：这里预热的 URL 必须与上面建元素用的 URL **逐字符相同**（同一个变量，不再各写一遍字面量）——
    //    一旦"预热 A、播放 B"，点按就会退回异步解码路径，手感立刻变钝（0.3.3 那次"变钝"的根因）。
    dshwvWarm([pu.empty ? '' : pu.url, ru.empty ? '' : ru.url])
  } catch (err) {}
}
function playPress() {
  if (!soundOn) return
  // v729 修复：**本轮状态复位必须放在「按压槽留空」分支之前**。
  // 原实现里 pressAudio 为空时直接 return，跳过了 releasePlayed = false；而 playRelease()
  // 一旦把 releasePlayed 置为 true 就再没有任何地方复位它 → 结果是只有第一次松开有声音，
  // 之后每次点击都静音（用户实测：新建音效组只填松开音时复现）。
  if (releaseAudio) {
    releaseAudio.pause()
    releaseAudio.currentTime = 0
  }
  pressEnded = false
  releasePlayed = false
  if (!pressAudio) {
    // 按压槽留空:按压事件静音,但状态已复位 → 每次松开都还能正常发声
    pressEnded = true
    return
  }
  try {
    pressAudio.onended = function () {
      pressEnded = true
      // fallback (duration unknown): click → Ya2 right after Ya1 ends
      if (!pressing && !releasePlayed) playRelease()
      // hold: still pressed → wait for pressUp()
    }
    pressAudio.currentTime = 0
    var p = pressAudio.play()
    if (p && typeof p.catch === 'function') p.catch(function () {})
  } catch (err) {}
}
// 点按时"松开音提前多少毫秒进场"（v745 把它做成**可调参数**，按耳朵微调即可）：
//   0  = 松开音正好接在按压音结束那一刻（无缝、不重叠）
//   30 / 50 = 轻微交叠（更"黏"）
//   100 = 明显重叠（听感上可能像"重复播放"。0.3.0 代码里写的是 100，但它的主线程 setTimeout
//         经常迟到、实际几乎听不到重叠；我们用音频线程精确排期，所以别照抄 100）
// 当前取值：**40**（用户在 30/50 之间试听后选定）
var RELEASE_LEAD_MS = 40
function playRelease() {
  if (releasePlayed || !releaseAudio || !soundOn) return
  releasePlayed = true
  try {
    releaseAudio.currentTime = 0
    var p = releaseAudio.play()
    if (p && typeof p.catch === 'function') p.catch(function () {})
  } catch (err) {}
}
// 把松开音**排期到 delaySec 秒之后**（音频线程时间轴）：起播时刻精确、不受主线程抖动影响。
function playReleaseAt(delaySec) {
  if (releasePlayed || !releaseAudio || !soundOn) return
  releasePlayed = true
  try {
    releaseAudio.currentTime = 0 // 复位（同时停掉上一条已排期/在播的松开音）
    if (delaySec > 0 && typeof releaseAudio.playAt === 'function') {
      releaseAudio.playAt(delaySec)
      return
    }
    var p = releaseAudio.play()
    if (p && typeof p.catch === 'function') p.catch(function () {})
  } catch (err) {}
}
function pressDown() {
  body.style.transform = SQUISH
  pressing = true
  playPress()
}
function pressUp() {
  body.style.transform = 'scaleY(1) scaleX(1)'
  pressing = false
  if (pressEnded) {
    // hold (or released after Ya1 finished) → Ya2 now
    playRelease()
    return
  }
  // click：把松开音排到"按压音结束前 RELEASE_LEAD_MS 毫秒"（默认 0 = 正好接上）
  var durKnown = false
  var remainSec = 0
  try {
    var dur = pressAudio ? pressAudio.duration : 0
    if (isFinite(dur) && dur > 0) {
      durKnown = true
      remainSec = Math.max(0, dur - pressAudio.currentTime)
    }
  } catch (err) {}
  if (durKnown) {
    playReleaseAt(Math.max(0, remainSec - RELEASE_LEAD_MS / 1000))
    return
  }
  // 时长未知（预热失败/解码异常）→ 交给 pressAudio.onended 兜底（见 playPress）
}
var menuOpen = false
var menuClosedAt = 0 // 最近一次关闭菜单的时刻(用于避免"关掉后同一次手势又把它长按打开")
function toggleMenu() {
  menuOpen = !menuOpen
  if (menuOpen) positionMenu()
  menuBox.classList.toggle('dshwv-menu-open', menuOpen)
  if (menuOpen && !menuBtnHide) menuBtn.classList.add('dshwv-menu-btn-visible')
  // 汉堡按钮关闭菜单时同样复位用量子界面(用量态时滑回主菜单态,
  // 主菜单态时 hideUsageSub 守卫直接返回),避免下次打开仍停留在用量视图
  if (!menuOpen) closeUsagePanel()
}
function closeMenu() {
  menuOpen = false
  menuClosedAt = Date.now()
  menuBox.classList.remove('dshwv-menu-open')
  closeRolePanel()
  closeAudioGroupPanel()
  closeUsagePanel()
  root.style.transition = ''
  snapCheck()
}
function snapCheck() {
  if (!snapConfig || snapConfig.mode === 'off') return
  var rect = root.getBoundingClientRect()
  var vp = viewport()
  var w = rect.width, h = rect.height
  var left = rect.left, top = rect.top
  // 判定点：左右吸附/翻转 = 图像中心 x，下吸附 = 图像中心 y，上吸附 = 挂件盒中心 y
  var ac = artCenterAt(left, top, w, h, !!state.flip)
  var z = snapZones(ac.cx, top + h / 2, ac.cy, vp)
  var moved = false
  if (z.zH === 'left') {
    state.h = 'left'
    state.hOff = 0
    left = 0
    moved = true
  } else if (z.zH === 'right') {
    state.h = 'right'
    state.hOff = 0
    left = vp.w - w - rightGap()
    moved = true
  } else {
    state.h = null
    state.hOff = left
  }
  if (z.zV === 'top') {
    state.v = 'top'
    state.vOff = 0
    top = 0
    moved = true
  } else if (z.zV === 'bottom') {
    state.v = 'bottom'
    state.vOff = 0
    top = Math.max(0, vp.h - h)
    moved = true
  } else {
    state.v = 'bottom'
    state.vOff = Math.max(0, vp.h - top - h)
  }
  state.flip = z.flip
  if (moved) {
    state.left = left
    state.top = top
    settle()
  } else {
    express()
  }
}
function positionMenu() {
  try {
    var r = root.getBoundingClientRect()
    var b = menuBtn.getBoundingClientRect()
    var vp = viewport()
    var onLeft = r.left + r.width / 2 < vp.w / 2
    // 菜单出现在按钮上方，锚定在按钮一侧：
    // 右侧 → 菜单右下角对齐按钮右上角；左侧 → 菜单左下角对齐按钮左上角
    if (onLeft) {
      menuBox.style.left = b.left + 'px'
      menuBox.style.right = 'auto'
      menuBox.style.transformOrigin = 'bottom left'
    } else {
      menuBox.style.right = (vp.w - b.right) + 'px'
      menuBox.style.left = 'auto'
      menuBox.style.transformOrigin = 'bottom right'
    }
    // 菜单底边悬在鲸鱼素材顶部上方：素材占挂件底部 59.45% 高度，
    // 其顶部位于 root 底部往上 59.45% 处（= 距顶部 40.55%）。按钮顶边在
    // 素材顶部下方 4px，若菜单底边对齐按钮顶边会压住素材顶部，导致被菜单遮挡。
    var assetTop = r.bottom - r.height * 0.5945
    menuBox.style.bottom = (vp.h - assetTop) + 6 + 'px'
    menuBox.style.top = 'auto'
  } catch (err) {}
}

// —— 自定义角色：列表 / 选择 / 置顶 / 删除 / 导入裁剪 ——
var ROLE_URL = '/dsh-whale/roles.json'
var currentRole = { id: 'default', name: '小鲸鱼', url: IMG_URL }
var roleList = []
function loadRoles() {
  try {
    fetch(ROLE_URL, { cache: 'no-store' })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (!d || !d.ok || !Array.isArray(d.roles)) return
        roleList = d.roles
        renderRolePanel()
        // 恢复上次选择的角色
        var saved = ''
        try { saved = localStorage.getItem('dshw-role') || '' } catch (err) {}
        var found = null
        for (var i = 0; i < roleList.length; i++) {
          if (roleList[i].id === saved) { found = roleList[i]; break }
        }
        if (found) {
          // 角色仍在：若初始已按此角色渲染（initRoleUrl 一致）则不重复切换，只补全 name/panel
          if (currentRole.id !== found.id) applyRole(found.id, found.name, found.url)
          else renderRolePanel()
        } else if (saved && saved !== 'default') {
          // 保存的角色已被删除：回退默认鲸鱼娘
          applyRole('default', '小鲸鱼', IMG_URL)
        }
        // 若 saved 为空或为 default：保持当前（initRoleUrl 已是 IMG_URL）
      })
      .catch(function () {})
  } catch (err) {}
}
function setRoleBtnText(t) { try { roleBtnLabel.textContent = t } catch (err) {} }
function setAudioBtnText(t) { try { audioGroupBtnLabel.textContent = t } catch (err) {} }
// 名称悬停循环滚动：仅当文本溢出容器时，悬停到该行后名称无限循环滚动露出全名，
// 移开停止并回位。双副本无缝循环：滚动距离 = 单份文本+间距，跳回起点时画面相同。
// item 为行容器（鼠标事件源），nameEl 必须是包了 .dshwv-nameinner 的外层。
var MARQ_SPEED = 40 // 滚动速度 px/s
function bindNameMarquee(item, nameEl) {
  try {
    if (!item || !nameEl) return
    var timer = null
    function stop() {
      try {
        if (timer) { clearTimeout(timer); timer = null }
        var t = nameEl.querySelector('.dshwv-nameinner')
        if (!t) return
        t.style.transitionTimingFunction = ''
        t.style.transitionDuration = ''
        t.style.transform = ''
        // 空闲态只保留单份副本：移除滚动时临时补的那份
        while (t.children && t.children.length > 1) t.removeChild(t.children[t.children.length - 1])
      } catch (err) {}
    }
    function distOf() {
      try {
        var t = nameEl.querySelector('.dshwv-nameinner')
        if (!t) return 0
        return t.scrollWidth / 2 // 单份副本 + 尾部间距的宽度
      } catch (err) { return 0 }
    }
    function textWOf() {
      try {
        var t = nameEl.querySelector('.dshwv-nameinner')
        if (!t || !t.children || !t.children.length) return 0
        return t.children[0].offsetWidth || 0 // 单份文本本身宽度
      } catch (err) { return 0 }
    }
    // 循环滚动前补一份副本（默认只有一份）；仅在真正溢出时才需要
    function ensureDup() {
      var t = nameEl.querySelector('.dshwv-nameinner')
      if (!t || !t.children || t.children.length >= 2) return t
      var first = t.children[0]
      var c = document.createElement('span')
      c.className = 'dshwv-namecopy'
      c.textContent = first.textContent
      t.appendChild(c)
      return t
    }
    function cycle() {
      var dist = distOf()
      var dur = Math.max(200, (dist / MARQ_SPEED) * 1000)
      timer = setTimeout(function () { try { cycle() } catch (err) {} }, dur + 40)
      var t = nameEl.querySelector('.dshwv-nameinner')
      if (!t) return
      // 无过渡瞬间回到起点（与 -dist 处画面一致，视觉无缝），再滚动下一轮
      t.style.transitionTimingFunction = 'linear'
      t.style.transitionDuration = '0ms'
      t.style.transform = 'translateX(0px)'
      void t.offsetWidth // 强制回流，保证下一帧按新时长生效
      t.style.transitionDuration = dur + 'ms'
      t.style.transform = 'translateX(' + (-dist) + 'px)'
    }
    item.addEventListener('mouseenter', function () {
      try {
        stop()
        if (textWOf() <= nameEl.clientWidth + 1) return // 未溢出不滚
        ensureDup() // 溢出才补副本供循环滚动
        cycle()
      } catch (err) {}
    })
    item.addEventListener('mouseleave', stop)
  } catch (err) {}
}
// 创建名称 span：外层 .dshwv-rolename/.dshwv-audioname（overflow hidden）
// + 内层 .dshwv-nameinner 轨道。默认只放一份文本，避免短名字“放得下两遍”时
// 把第二份副本也露出来（重复显示）；需要循环滚动时由 bindNameMarquee 临时补副本。
function makeNameCell(className, text) {
  var outer = document.createElement('span')
  outer.className = className
  var inner = document.createElement('span')
  inner.className = 'dshwv-nameinner'
  var c = document.createElement('span')
  c.className = 'dshwv-namecopy'
  c.textContent = text
  inner.appendChild(c)
  outer.appendChild(inner)
  return outer
}
function applyRole(id, name, url) {
  currentRole = { id: id, name: name, url: url }
  img.src = url
  setRoleBtnText(name)
  try { localStorage.setItem('dshw-role', id) } catch (err) {}
  hitReady = false
  hitFailed = false
  setupHitTest(url)
  closeRolePanel()
  renderRolePanel()
}
function roleUrl(id) {
  if (id === 'default') return IMG_URL
  return '/dsh-whale/role-image.png?id=' + encodeURIComponent(id)
}
function toggleRolePanel() {
  if (rolePanel.classList.contains('dshwv-rolelist-open')) { closeRolePanel(); return }
  try {
    var b = roleBtn.getBoundingClientRect()
    var vp = viewport()
    // 固定面板宽度（与触发按钮一致，最小 200px）：防止长名称把面板撑宽，
    // 超出部分由名称 ellipsis 省略 / 面板横向滚动
    var panelW = Math.max(200, Math.round(b.width))
    rolePanel.style.width = panelW + 'px'
    rolePanel.style.left = Math.max(4, Math.min(b.left, vp.w - panelW - 4)) + 'px'
    rolePanel.style.top = (b.bottom + 6) + 'px'
    rolePanel.style.display = 'block'
    rolePanel.classList.add('dshwv-rolelist-open')
  } catch (err) {}
}
function closeRolePanel() {
  rolePanel.classList.remove('dshwv-rolelist-open')
  rolePanel.style.display = 'none'
}
function renderRolePanel() {
  try {
    rolePanel.innerHTML = ''
    roleList.forEach(function (r) {
      var item = document.createElement('div')
      item.className = 'dshwv-roleitem' + (currentRole.id === r.id ? ' dshwv-roleitem-cur' : '')
      var thumb = document.createElement('img')
      thumb.className = 'dshwv-rolethumb'
      thumb.src = r.url
      thumb.alt = ''
      thumb.draggable = false
      var name = makeNameCell('dshwv-rolename', r.name)
      // 动图标签 + 名称包进同一容器（总宽受控）：标签在前固定宽，名称超长省略，
      // 不会被长名称挤出/撑宽面板
      var nameWrap = document.createElement('span')
      nameWrap.className = 'dshwv-rolenamewrap'
      if (r.format === 'gif' || r.format === 'apng') {
        var gifTag = document.createElement('span')
        gifTag.className = 'dshwv-roleGifTag'
        gifTag.textContent = r.format === 'apng' ? 'APNG' : 'GIF'
        nameWrap.appendChild(gifTag)
      }
      nameWrap.appendChild(name)
      item.appendChild(thumb)
      item.appendChild(nameWrap)
      var pin = document.createElement('button')
      pin.type = 'button'
      pin.className = 'dshwv-rolepin' + (r.pinned ? ' on' : '')
      pin.textContent = '📌'
      pin.title = r.pinned ? '取消置顶' : '置顶'
      pin.addEventListener('click', function (e) {
        e.stopPropagation()
        togglePin(r.id, !r.pinned)
      })
      item.appendChild(pin)
      if (r.id !== 'default') {
        var del = document.createElement('button')
        del.type = 'button'
        del.className = 'dshwv-roledel'
        del.textContent = '✕'
        del.title = '删除角色'
        del.addEventListener('click', function (e) {
          e.stopPropagation()
          deleteRole(r.id)
        })
        item.appendChild(del)
      }
      item.addEventListener('click', function () {
        applyRole(r.id, r.name, roleUrl(r.id))
      })
      bindNameMarquee(item, name)
      rolePanel.appendChild(item)
    })
  } catch (err) {}
}
function togglePin(id, pinned) {
  try {
    fetch('/dsh-whale/role-pin.json', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: id, pinned: pinned }),
    })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (d && d.ok && Array.isArray(d.roles)) {
          roleList = d.roles
          renderRolePanel()
        }
      })
      .catch(function () {})
  } catch (err) {}
}
var confirmCb = null
function showConfirm(text, cb, okLabel) {
  confirmCb = cb || null
  // 确认弹窗永远在最上层：模型面板(29000)/提醒编辑器(30000)等都在其下
  try { confirmMask.style.setProperty('z-index', '40000', 'important') } catch (err) {}
  confirmText.textContent = text
  // 默认确定键文案:文本含“删除”则用“删除”,否则“确定”(可显式传入覆盖)
  try {
    if (!okLabel) okLabel = String(text || '').indexOf('删除') !== -1 ? '删除' : '确定'
    confirmYesBtn.textContent = okLabel
  } catch (err) {}
  confirmMask.style.display = 'flex'
}
function hideConfirm() {
  confirmMask.style.display = 'none'
  confirmCb = null
}
function deleteRole(id) {
  var r = null
  for (var i = 0; i < roleList.length; i++) if (roleList[i].id === id) { r = roleList[i]; break }
  showConfirm('确定删除角色「' + (r ? r.name : id) + '」吗？', function () {
    try {
      fetch('/dsh-whale/role-delete.json', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: id }),
      })
        .then(function (res) { return res.json() })
        .then(function (d) {
          if (d && d.ok && Array.isArray(d.roles)) {
            roleList = d.roles
            renderRolePanel()
            if (currentRole.id === id) applyRole('default', '小鲸鱼', IMG_URL)
          }
        })
        .catch(function () {})
    } catch (err) {}
  })
}
// —— 导入：文件 → 裁剪弹窗 → 上传 ——
var cropState = null
// 手写 GIF 帧数解析：返回图像分隔符(0x2C)数量（1 = 静态，>1 = 动图）
function countGifFrames(bytes) {
  try {
    // 校验 GIF 头
    var head = ''
    for (var i = 0; i < 6; i++) head += String.fromCharCode(bytes[i])
    if (head !== 'GIF87a' && head !== 'GIF89a') return 1
    // 逻辑屏幕描述符：offset 6..13；bit7 全局颜色表标志，bit0-2 色表大小
    var flags = bytes[10]
    var hasGct = (flags & 0x80) !== 0
    var gctSize = 3 * (1 << ((flags & 0x07) + 1))
    var pos = 13 + (hasGct ? gctSize : 0)
    var frames = 0
    while (pos + 1 < bytes.length) {
      var b = bytes[pos]
      if (b === 0x3b) break // 结尾
      if (b === 0x2c) { // 图像分隔符（一帧）
        frames++
        // 图像描述符 9B + 局部颜色表
        var lctFlag = bytes[pos + 9] & 0x80
        var lctSize = lctFlag ? 3 * (1 << ((bytes[pos + 9] & 0x07) + 1)) : 0
        pos += 10 + lctSize
        // 跳过 LZW 最小码长 + 子块数据
        if (pos >= bytes.length) break
        pos++ // LZW min code size
        while (pos < bytes.length) {
          var sz = bytes[pos]
          pos++
          if (sz === 0) break
          pos += sz
        }
      } else if (b === 0x21) { // 扩展块
        pos += 2 // label + 块大小
        while (pos < bytes.length) {
          var sz2 = bytes[pos]
          pos++
          if (sz2 === 0) break
          pos += sz2
        }
      } else {
        break // 异常，停止
      }
    }
    return Math.max(1, frames)
  } catch (err) { return 1 }
}
// 手写 APNG 检测：PNG 文件存在 acTL chunk 即为动图（后缀仍是 .png / MIME image/png）
function isAnimatedPng(bytes) {
  try {
    // PNG 签名：89 50 4E 47 0D 0A 1A 0A
    if (bytes.length < 8) return false
    for (var s = 0; s < 8; s++) {
      if (bytes[s] !== [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a][s]) return false
    }
    // chunk 结构：[4B 长度][4B 类型][数据][4B CRC]，从 offset 8 开始
    var pos = 8
    while (pos + 8 <= bytes.length) {
      var len = (bytes[pos] << 24) | (bytes[pos + 1] << 16) | (bytes[pos + 2] << 8) | bytes[pos + 3]
      var type = ''
      for (var i = 0; i < 4; i++) type += String.fromCharCode(bytes[pos + 4 + i])
      if (type === 'acTL') return true // 动画控制 chunk → APNG
      if (type === 'IEND') return false
      pos += 12 + len // 长度4 + 类型4 + CRC4 + 数据
    }
    return false
  } catch (err) { return false }
}
function onRoleFileChosen(input) {
  try {
    var f = input && input.files && input.files[0]
    input.value = ''
    if (!f) return
    if (!/^image\//.test(f.type)) return
    var reader = new FileReader()
    reader.onload = function () {
      try {
        var buf = new Uint8Array(reader.result)
        var isGifFile = /^image\/gif$/i.test(f.type) || /.gif$/i.test(f.name)
        var isPngFile = /^image\/png$/i.test(f.type) || /.png$/i.test(f.name)
        var animType = null // 'gif' | 'apng' | null
        if (isGifFile) {
          // GIF 动图（>1 帧）→ 专用窗口；单帧 GIF 当静态走裁剪
          if (countGifFrames(buf) > 1) animType = 'gif'
        } else if (isPngFile) {
          // APNG（含 acTL）→ 专用窗口；静态 PNG 走裁剪
          if (isAnimatedPng(buf)) animType = 'apng'
        }
        if (animType) {
          // 动图：转 dataURL 走专用窗口（不裁剪，原样上传）
          var fr = new FileReader()
          fr.onload = function () { openGifRoleModal(fr.result, f.name, animType) }
          fr.readAsDataURL(f)
          return
        }
      } catch (err) {}
      // 静态图（PNG/JPG/单帧GIF 等）→ 裁剪流程
      var dr = new FileReader()
      dr.onload = function () {
        openCropModal(dr.result, f.name)
      }
      dr.readAsDataURL(f)
    }
    reader.readAsArrayBuffer(f)
  } catch (err) {}
}
function openCropModal(dataUrl, fileName) {
  try {
    var imgEl = new Image()
    imgEl.onload = function () {
      cropState = {
        img: imgEl,
        zoom: 1,
        ox: 0, // 图片中心相对画布中心的横向偏移（画布 px）
        oy: 0, // 图片中心相对画布中心的纵向偏移
        rotation: 0, // 旋转角度（度）
        flipH: false, // 水平翻转
        flipV: false, // 垂直翻转
        baseScale: Math.max(CROP_BOX / imgEl.width, CROP_BOX / imgEl.height),
      }
      // 名称默认留空，以便显示占位文本「角色名称」；确认时为空则回落为「新角色」
      cropNameInput.value = ''
      cropZoom.value = '1'
      cropZoomNum.value = '100'
      cropAngle.value = '0'
      cropAngleNum.value = '0'
      positionCrop()
      // v744：裁剪窗口既可能从主菜单打开，也可能从资源管理(20300)/泡泡编辑器(20500)里打开，
      // 固定 20000 在后者会被父窗口盖住 → 动态抬层（见 visibleTopZ 上方的分层表）
      dshwLayerUp(cropMask, 20000)
      cropMask.style.display = 'flex'
    }
    imgEl.onerror = function () {}
    imgEl.src = dataUrl
  } catch (err) {}
}
// 旋转角度钳制到 [-360, 360]（允许负向旋转）
function clampAngle(v) {
  var n = Number(v)
  if (!isFinite(n)) return 0
  return Math.min(360, Math.max(-360, Math.round(n)))
}
// 旋转后图片在画布上的显示包围盒（画布 px）。
// 任意角度 θ：包围盒宽 = w·|cosθ| + h·|sinθ|，高 = w·|sinθ| + h·|cosθ|（90 倍数时退化回宽高互换）
function cropDisplaySize() {
  var s = cropState.baseScale * cropState.zoom
  var w = cropState.img.width * s
  var h = cropState.img.height * s
  var rad = cropState.rotation * Math.PI / 180
  var c = Math.abs(Math.cos(rad))
  var sn = Math.abs(Math.sin(rad))
  return { w: w * c + h * sn, h: w * sn + h * c, s: s }
}
function positionCrop() {
  if (!cropState) return
  var d = cropDisplaySize()
  // 图片中心偏移范围：若该方向图片比框大则可在 ±(size-box)/2 内移动（保证覆盖），
  // 若比框小则只能居中（cover 保证另一方向一定大于框，不会露出边缘）
  var maxOx = Math.max(0, (d.w - CROP_BOX) / 2)
  var maxOy = Math.max(0, (d.h - CROP_BOX) / 2)
  cropState.ox = Math.min(maxOx, Math.max(-maxOx, cropState.ox))
  cropState.oy = Math.min(maxOy, Math.max(-maxOy, cropState.oy))
  drawCrop()
}
function drawCrop() {
  try {
    if (!cropState) return
    var ctx = cropCanvas.getContext('2d')
    var s = cropState.baseScale * cropState.zoom
    var rad = cropState.rotation * Math.PI / 180
    var w = cropState.img.width * s
    var h = cropState.img.height * s
    ctx.clearRect(0, 0, CROP_BOX, CROP_BOX)
    ctx.save()
    // 以图片中心为旋转原点；ox/oy 是中心相对画布中心的偏移
    ctx.translate(CROP_BOX / 2 + cropState.ox, CROP_BOX / 2 + cropState.oy)
    ctx.rotate(rad)
    // 翻转：水平/垂直镜像（在旋转之后应用，翻转的是旋转后的图像）
    ctx.scale(cropState.flipH ? -1 : 1, cropState.flipV ? -1 : 1)
    ctx.drawImage(cropState.img, -w / 2, -h / 2, w, h)
    ctx.restore()
  } catch (err) {}
}
var cropDrag = null
function onCropDown(e) {
  if (!cropState) return
  try { e.preventDefault(); e.stopPropagation() } catch (err) {}
  cropDrag = { x: e.clientX, y: e.clientY, ox: cropState.ox, oy: cropState.oy }
}
function onCropMove(e) {
  if (!cropDrag || !cropState) return
  cropState.ox = cropDrag.ox + (e.clientX - cropDrag.x)
  cropState.oy = cropDrag.oy + (e.clientY - cropDrag.y)
  positionCrop()
}
function onCropUp() { cropDrag = null }
function onCropWheel(e) {
  if (!cropState) return
  try { e.preventDefault(); e.stopPropagation() } catch (err) {}
  // 滚轮缩放：向上放大、向下缩小（步进 0.05，范围 30%–300%）
  var delta = (e.deltaY > 0 ? -1 : 1) * 0.05
  cropState.zoom = Math.min(3, Math.max(0.3, cropState.zoom + delta))
  cropZoom.value = String(Math.round(cropState.zoom * 100) / 100)
  cropZoomNum.value = String(Math.round(cropState.zoom * 100))
  positionCrop()
}
function resetCrop() {
  if (!cropState) return
  // 重置缩放/旋转/位置/翻转到初始状态
  cropState.zoom = 1
  cropState.ox = 0
  cropState.oy = 0
  cropState.rotation = 0
  cropState.flipH = false
  cropState.flipV = false
  cropZoom.value = '1'
  cropZoomNum.value = '100'
  cropAngle.value = '0'
  cropAngleNum.value = '0'
  cropFlipHBtn.classList.remove('dshwv-cropflip-on')
  cropFlipVBtn.classList.remove('dshwv-cropflip-on')
  positionCrop()
}
// 翻转：给图片容器（canvas）一个带过渡的镜像动画（视觉平滑翻转），
// 动画结束后把翻转落到画布重绘，并复位 canvas 变换。
// 只翻转 canvas，不翻转裁剪框的虚线边框/背景。
var cropFlipTimer = null
function flipCrop(axis) {
  if (!cropState) return
  try { if (cropFlipTimer) { clearTimeout(cropFlipTimer); cropFlipTimer = null } } catch (err) {}
  var flipTarget = axis === 'H' ? 'scaleX(-1)' : 'scaleY(-1)'
  cropCanvas.style.transition = 'transform .3s ease'
  cropCanvas.style.transform = flipTarget
  cropFlipTimer = setTimeout(function () {
    cropFlipTimer = null
    try {
      if (axis === 'H') { cropState.flipH = !cropState.flipH; cropFlipHBtn.classList.toggle('dshwv-cropflip-on', cropState.flipH) }
      else { cropState.flipV = !cropState.flipV; cropFlipVBtn.classList.toggle('dshwv-cropflip-on', cropState.flipV) }
      positionCrop()
      // 复位 canvas 变换（延迟到下一帧，避免闪回）
      requestAnimationFrame(function () {
        cropCanvas.style.transition = ''
        cropCanvas.style.transform = ''
      })
    } catch (err) {}
  }, 300)
}
function confirmCrop() {
  try {
    if (!cropState) return
    var name = (cropNameInput.value || '').trim().slice(0, 16) || '新角色'
    // 用与预览一致的变换把当前可视画面放大导出为 610×610，
    // 保证所见即所得（含旋转/缩放/位移），且保留透明背景
    var s = cropState.baseScale * cropState.zoom
    var k = 610 / CROP_BOX // 放大系数
    var rad = cropState.rotation * Math.PI / 180
    var w = cropState.img.width * s * k
    var h = cropState.img.height * s * k
    var out = document.createElement('canvas')
    out.width = 610
    out.height = 610
    var octx = out.getContext('2d')
    octx.translate(305 + cropState.ox * k, 305 + cropState.oy * k)
    octx.rotate(rad)
    // 与预览一致的翻转（旋转后应用）
    octx.scale(cropState.flipH ? -1 : 1, cropState.flipV ? -1 : 1)
    // 直接绘制，不铺底色——保留原图透明背景（PNG 透明素材不会被染成白色）
    octx.drawImage(cropState.img, -w / 2, -h / 2, w, h)
    var dataUrl = out.toDataURL('image/png')
    fetch(ROLE_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: name, image: dataUrl }),
    })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (d && d.ok && Array.isArray(d.roles)) {
          roleList = d.roles
          renderRolePanel()
          // 自动切换到刚导入的角色（取最新的非 default 角色）
          var newest = null
          for (var i = 0; i < roleList.length; i++) {
            if (roleList[i].id !== 'default' && (!newest || roleList[i].createdAt > newest.createdAt)) newest = roleList[i]
          }
          if (newest) applyRole(newest.id, newest.name, roleUrl(newest.id))
        }
      })
      .catch(function () {})
      .finally(function () { hideCropModal() })
  } catch (err) { try { hideCropModal() } catch (err2) {} }
}
function hideCropModal() {
  cropMask.style.display = 'none'
  cropState = null
  cropDrag = null
}

// —— 自定义音效组：列表 / 选择 / 置顶 / 删除 / 编辑 / 音频裁剪 ——
var AUDIO_URL = '/dsh-whale/audio.json'
var audioGroups = [] // [{id,name,press,release,preset,pinned}]
var audioFragments = [] // [{id,name,preset}]
var audioGroupPanelOpen = false
function loadAudio() {
  try {
    fetch(AUDIO_URL, { cache: 'no-store' })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (!d || !d.ok) return
        if (Array.isArray(d.groups)) audioGroups = d.groups
        if (Array.isArray(d.fragments)) audioFragments = d.fragments
        // 若当前 soundSet 是已删除的自定义组，回退预设
        var exists = false
        for (var i = 0; i < audioGroups.length; i++) if (audioGroups[i].id === soundSet) { exists = true; break }
        if (!exists) setSoundSet('duck')
        else {
          setAudioBtnText(audioGroupName(soundSet))
          // 组槽数据就绪后重建按压/松开音频元素(自定义组槽位留空=静音需此时生效)
          try { applySoundSet() } catch (err) {}
        }
        renderAudioGroupPanel()
        refreshTaskEndAfterAudio()
      })
      .catch(function () {})
  } catch (err) {}
}
function audioGroupName(id) {
  for (var i = 0; i < audioGroups.length; i++) if (audioGroups[i].id === id) return audioGroups[i].name
  return id === 'duck' ? '小黄鸭' : (id === 'fx1' ? '音效1' : id)
}
// 组槽位是否显式留空('' = 该事件静音);null/undefined/预设=视为有声
function audioGroupSlotEmpty(id, slot) {
  try {
    for (var i = 0; i < audioGroups.length; i++) {
      var g = audioGroups[i]
      if (g && g.id === id) return g[slot] === ''
    }
  } catch (err) {}
  return false
}
function toggleAudioGroupPanel() {
  if (audioGroupPanelOpen) { closeAudioGroupPanel(); return }
  try {
    renderAudioGroupPanel() // 每次打开重建:反映最新 soundSet 的选中高亮
    var b = audioGroupBtn.getBoundingClientRect()
    var vp = viewport()
    // 固定面板宽度（与触发按钮一致，最小 200px）
    var panelW = Math.max(200, Math.round(b.width))
    audioGroupPanel.style.width = panelW + 'px'
    audioGroupPanel.style.left = Math.max(4, Math.min(b.left, vp.w - panelW - 4)) + 'px'
    audioGroupPanel.style.top = (b.bottom + 6) + 'px'
    audioGroupPanel.style.display = 'block'
    audioGroupPanel.classList.add('dshwv-audiolist-open')
    audioGroupPanelOpen = true
  } catch (err) {}
}
function closeAudioGroupPanel() {
  audioGroupPanel.classList.remove('dshwv-audiolist-open')
  audioGroupPanel.style.display = 'none'
  audioGroupPanelOpen = false
}
function renderAudioGroupPanel() {
  try {
    audioGroupPanel.innerHTML = ''
    audioGroups.forEach(function (g) {
      var item = document.createElement('div')
      item.className = 'dshwv-audioitem' + (soundSet === g.id ? ' dshwv-audioitem-cur' : '')
      var thumb = document.createElement('span')
      thumb.className = 'dshwv-audiothumb'
      thumb.textContent = '🎵'
      item.appendChild(thumb)
      var name = makeNameCell('dshwv-audioname', g.name)
      item.appendChild(name)
      if (g.preset) {
        var tag = document.createElement('span')
        tag.className = 'dshwv-audiopreset'
        tag.textContent = '预设'
        item.appendChild(tag)
      } else {
        var pin = document.createElement('button')
        pin.type = 'button'
        pin.className = 'dshwv-audiopin' + (g.pinned ? ' on' : '')
        pin.textContent = '📌'
        pin.title = g.pinned ? '取消置顶' : '置顶'
        pin.addEventListener('click', function (e) {
          e.stopPropagation()
          audioPinGroup(g.id, !g.pinned)
        })
        item.appendChild(pin)
        var del = document.createElement('button')
        del.type = 'button'
        del.className = 'dshwv-audiodel'
        del.textContent = '✕'
        del.title = '删除音效组'
        del.addEventListener('click', function (e) {
          e.stopPropagation()
          audioDeleteGroup(g.id)
        })
        item.appendChild(del)
      }
      item.addEventListener('click', function () {
        setSoundSet(g.id)
        closeAudioGroupPanel()
      })
      bindNameMarquee(item, name)
      audioGroupPanel.appendChild(item)
    })
  } catch (err) {}
}
function audioPinGroup(id, pinned) {
  try {
    fetch(AUDIO_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'pin-group', id: id, pinned: pinned }),
    })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (d && d.ok && Array.isArray(d.groups)) {
          audioGroups = d.groups
          renderAudioGroupPanel()
          refreshTaskEndAfterAudio()
        }
      })
      .catch(function () {})
  } catch (err) {}
}
function audioDeleteGroup(id) {
  var g = null
  for (var i = 0; i < audioGroups.length; i++) if (audioGroups[i].id === id) { g = audioGroups[i]; break }
  showConfirm('确定删除音效组「' + (g ? g.name : id) + '」吗？', function () {
    try {
      fetch(AUDIO_URL, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'delete-group', id: id }),
      })
        .then(function (r) { return r.json() })
        .then(function (d) {
          if (d && d.ok && Array.isArray(d.groups)) {
            audioGroups = d.groups
            renderAudioGroupPanel()
            refreshTaskEndAfterAudio()
            if (soundSet === id) setSoundSet('duck')
          }
        })
        .catch(function () {})
    } catch (err) {}
  })
}
// —— 组编辑弹窗 ——
var editingAudioGroupId = null
var activeSlotPanel = null // 当前打开的面板（press/release）
function audioSlotValue(slot) {
  return slot === 'press' ? (audioEditPressVal || 'ya1') : (audioEditReleaseVal || 'ya2')
}
var audioEditPressVal = 'ya1'
var audioEditReleaseVal = 'ya2'
function audioSlotName(id) {
  for (var i = 0; i < audioFragments.length; i++) if (audioFragments[i].id === id) return audioFragments[i].name
  return id
}
// 槽位按钮文案:空值=留空(该事件不发声),灰字提示;非空显示片段名
function audioSlotBtnText(btn, id) {
  try {
    var txt = audioSlotName(id)
    btn.style.opacity = txt ? '' : '.55'
    btn.textContent = txt || '留空·不发声'
    btn.title = '留空则该事件不发声;点击可选择音频片段'
  } catch (err) {}
  return btn
}
function openAudioGroupEditor(group) {
  editingAudioGroupId = group && group.id ? group.id : null
  audioEditTitle.textContent = editingAudioGroupId ? '编辑音效组' : '新建音效组'
  audioEditName.value = group && group.name ? group.name : ''
  // 槽位可为空(留空=该事件静音);新建默认留空待选;
  // 编辑旧组时:字段为字符串(含'')按存储原样,缺失字段才回落预设(兼容更早版本)
  var isNewGroup = !(group && group.id)
  audioEditPressVal = group && typeof group.press === 'string' ? group.press : (isNewGroup ? '' : 'ya1')
  audioEditReleaseVal = group && typeof group.release === 'string' ? group.release : (isNewGroup ? '' : 'ya2')
  audioEditPressBtn.textContent = audioSlotName(audioEditPressVal)
  audioEditReleaseBtn.textContent = audioSlotName(audioEditReleaseVal)
  audioSlotBtnText(audioEditPressBtn, audioEditPressVal)
  audioSlotBtnText(audioEditReleaseBtn, audioEditReleaseVal)
  // 预创建试听音频并预加载（与挂件 applySoundSet 的 preload=auto 一致），
  // 首次按下即响，避免现场下载/解码造成的听感延迟
  audioEditPreviewEnsure()
  audioEditMask.style.display = 'flex'
}
function toggleAudioSlotPanel(slot) {
  var btn = slot === 'press' ? audioEditPressBtn : audioEditReleaseBtn
  var panel = slot === 'press' ? audioEditPressPanel : audioEditReleasePanel
  if (activeSlotPanel === panel) { closeAudioSlotPanels(); return }
  closeAudioSlotPanels()
  activeSlotPanel = panel
  renderAudioSlotPanel(slot)
  try {
    var b = btn.getBoundingClientRect()
    var vp = viewport()
    // 面板宽度与选择框（按钮）一致：按按钮实际宽度设置
    var panelW = Math.max(120, Math.round(b.width))
    panel.style.width = panelW + 'px'
    panel.style.left = Math.max(4, Math.min(b.left, vp.w - panelW - 4)) + 'px'
    panel.style.top = (b.bottom + 4) + 'px'
    panel.style.display = 'block'
  } catch (err) {}
}
function closeAudioSlotPanels() {
  if (audioEditPressPanel) audioEditPressPanel.style.display = 'none'
  if (audioEditReleasePanel) audioEditReleasePanel.style.display = 'none'
  activeSlotPanel = null
}
function renderAudioSlotPanel(slot) {
  try {
    var panel = slot === 'press' ? audioEditPressPanel : audioEditReleasePanel
    var current = slot === 'press' ? audioEditPressVal : audioEditReleaseVal
    panel.innerHTML = ''
    // 顶部：留空（该事件不发声）
    var emptyItem = document.createElement('div')
    emptyItem.className = 'dshwv-audioitem' + (!current ? ' dshwv-audioitem-cur' : '')
    var emptyIcon = document.createElement('span')
    emptyIcon.className = 'dshwv-audiothumb'
    emptyIcon.textContent = '🚫'
    emptyItem.appendChild(emptyIcon)
    var emptyName = makeNameCell('dshwv-audioname', '留空（不发声）')
    emptyItem.appendChild(emptyName)
    emptyItem.addEventListener('click', function () {
      if (slot === 'press') { audioEditPressVal = ''; audioSlotBtnText(audioEditPressBtn, '') }
      else { audioEditReleaseVal = ''; audioSlotBtnText(audioEditReleaseBtn, '') }
      audioEditPreviewEnsure(true)
      closeAudioSlotPanels()
    })
    bindNameMarquee(emptyItem, emptyName)
    panel.appendChild(emptyItem)
    audioFragments.forEach(function (f) {
      var item = document.createElement('div')
      item.className = 'dshwv-audioitem' + (current === f.id ? ' dshwv-audioitem-cur' : '')
      var thumb = document.createElement('span')
      thumb.className = 'dshwv-audiothumb'
      thumb.textContent = '🎵'
      item.appendChild(thumb)
      var name = makeNameCell('dshwv-audioname', f.name)
      item.appendChild(name)
      if (f.preset) {
        var tag = document.createElement('span')
        tag.className = 'dshwv-audiopreset'
        tag.textContent = '预设'
        item.appendChild(tag)
      } else {
        var del = document.createElement('button')
        del.type = 'button'
        del.className = 'dshwv-audiodel'
        del.textContent = '✕'
        del.title = '删除该音频'
        del.addEventListener('click', function (e) {
          e.stopPropagation()
          audioDeleteFragmentInSlot(slot, f.id)
        })
        item.appendChild(del)
      }
      item.addEventListener('click', function () {
        if (slot === 'press') { audioEditPressVal = f.id; audioSlotBtnText(audioEditPressBtn, f.id) }
        else { audioEditReleaseVal = f.id; audioSlotBtnText(audioEditReleaseBtn, f.id) }
        // 片段变化后重建试听音频（保持预加载指向最新片段）
        audioEditPreviewEnsure(true)
        closeAudioSlotPanels()
      })
      bindNameMarquee(item, name)
      panel.appendChild(item)
    })
  } catch (err) {}
}
function hideAudioEditor() {
  stopAudioEditPreview()
  audioEditMask.style.display = 'none'
  editingAudioGroupId = null
  closeAudioSlotPanels()
}
// 组编辑弹窗试听：完全模拟挂件按压交互。
// pointerdown（按下）→ 播放按压片段，进入"按住"状态（按压音播完仍按着则静默等待，不重复）；
// pointerup/cancel/leave（松开）→ 若按压音已结束则立即播松开片段，
// 否则把松开音**排期到按压音正好放完**的那一刻（音频线程排期；与挂件本体同一套做法，不重叠）。
var audioEditPreviewEl = null
var audioEditPreviewRelease = null
var audioEditPreviewReady = false
var audioEditPreviewPressing = false // 按住状态
var audioEditPreviewPressEnded = false // 按压音已播完
var audioEditPreviewReleasePlayed = false // 松开音已播
function audioEditPreviewEnsure(force) {
  try {
    if (!force && audioEditPreviewReady) return true
    // 槽位可为空(留空=该事件静音):空则不建对应音频元素,播放阶段直接跳过
    var pressId = audioEditPressVal || ''
    var releaseId = audioEditReleaseVal || ''
    // 重建前先停止：清重叠定时器与按压状态，防止旧定时器触发时指向新元素造成错播
    try { stopAudioEditPreview() } catch (err) {}
    // 再销毁旧元素
    try {
      if (audioEditPreviewEl) { audioEditPreviewEl.pause(); audioEditPreviewEl = null }
      if (audioEditPreviewRelease) { audioEditPreviewRelease.pause(); audioEditPreviewRelease = null }
    } catch (err) {}
    if (pressId) {
      audioEditPreviewEl = dshwvSound('/dsh-whale/audio-fragment.wav?id=' + encodeURIComponent(pressId))
      audioEditPreviewEl.preload = 'auto'
      audioEditPreviewEl.volume = soundVol
    }
    if (releaseId) {
      audioEditPreviewRelease = dshwvSound('/dsh-whale/audio-fragment.wav?id=' + encodeURIComponent(releaseId))
      audioEditPreviewRelease.preload = 'auto'
      audioEditPreviewRelease.volume = soundVol
    }
    // v745：试听也要预热（否则第一次点按试听同样是"按下音慢半拍"）
    dshwvWarm([
      pressId ? '/dsh-whale/audio-fragment.wav?id=' + encodeURIComponent(pressId) : '',
      releaseId ? '/dsh-whale/audio-fragment.wav?id=' + encodeURIComponent(releaseId) : '',
    ])
    audioEditPreviewReady = true
    return true
  } catch (err) { return false }
}
function audioEditPreviewDown() {
  try {
    stopAudioEditPreview()
    if (!audioEditPreviewEnsure()) return
    audioEditPreviewPressing = true
    audioEditPreviewPressEnded = false
    audioEditPreviewReleasePlayed = false
    // 按压槽留空:无按压音,视为“按压音立即结束”,松开时(若有松开音)直接播松开
    if (!audioEditPreviewEl) { audioEditPreviewPressEnded = true; return }
    // 按下：播放按压音
    audioEditPreviewEl.onended = function () {
      audioEditPreviewPressEnded = true
      // fallback：松手发生在按压音结束**之前**（= 直接点按）时，按压音播完立刻补上松开音。
      // ⚠️ 这里必须直接调 audioEditPreviewPlayRelease()，**不能**调 audioEditPreviewUp()：
      // 后者开头是 `if (!audioEditPreviewPressing) return`，而点按松手时 pressing 已经是 false
      // → 直接 return，松开音永远不响（这就是"直接点按只有按下音、长按才听到松开音"的根因）。
      if (!audioEditPreviewPressing && !audioEditPreviewReleasePlayed) audioEditPreviewPlayRelease()
    }
    var p = audioEditPreviewEl.play()
    if (p && typeof p.catch === 'function') p.catch(function () {})
  } catch (err) {}
}
function audioEditPreviewUp() {
  try {
    if (!audioEditPreviewPressing) return
    audioEditPreviewPressing = false
    if (!audioEditPreviewEl) { audioEditPreviewPlayRelease(); return } // 无按压音,松开即播松开(若有)
    if (audioEditPreviewPressEnded) {
      // 松手时按压音已播完 → 立即播松开
      audioEditPreviewPlayRelease()
      return
    }
    // 快速点击（松手时按压音未播完）：与本体一致 —— 排到"按压音结束前 RELEASE_LEAD_MS"
    // （默认 0 = 正好接上；音频线程排期，不依赖主线程定时器）
    var durKnown = false
    var remainSec = 0
    try {
      var dur = audioEditPreviewEl ? audioEditPreviewEl.duration : 0
      if (isFinite(dur) && dur > 0) {
        durKnown = true
        remainSec = Math.max(0, dur - audioEditPreviewEl.currentTime)
      }
    } catch (err) {}
    if (durKnown) {
      audioEditPreviewPlayRelease(Math.max(0, remainSec - RELEASE_LEAD_MS / 1000))
      return
    }
    // duration 未知 → onended fallback 播放松开
  } catch (err) {}
}
// 兜底里必须**直接**调它（不能调 audioEditPreviewUp() —— 后者以 pressing 为前置，
// 点按松手时已为 false 会直接 return，导致"直接点按只有按下音"）。delaySec>0 时走音频线程排期。
function audioEditPreviewPlayRelease(delaySec) {
  try {
    if (audioEditPreviewReleasePlayed || !audioEditPreviewRelease) return
    audioEditPreviewReleasePlayed = true
    audioEditPreviewRelease.currentTime = 0
    if (delaySec > 0 && typeof audioEditPreviewRelease.playAt === 'function') {
      audioEditPreviewRelease.playAt(delaySec)
      return
    }
    var p = audioEditPreviewRelease.play()
    if (p && typeof p.catch === 'function') p.catch(function () {})
  } catch (err) {}
}
function stopAudioEditPreview() {
  try {
    // v745：松开音改由音频线程排期，没有主线程定时器要清
    if (audioEditPreviewEl) {
      audioEditPreviewEl.pause()
      audioEditPreviewEl.currentTime = 0
    }
    if (audioEditPreviewRelease) {
      audioEditPreviewRelease.pause()
      audioEditPreviewRelease.currentTime = 0
    }
    audioEditPreviewPressing = false
    audioEditPreviewPressEnded = false
    audioEditPreviewReleasePlayed = false
  } catch (err) {}
}
function saveAudioGroup() {
  try {
    stopAudioEditPreview()
    var name = (audioEditName.value || '').trim().slice(0, 20)
    if (!name) { audioEditName.focus(); return }
    fetch(AUDIO_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        action: 'save-group',
        id: editingAudioGroupId || '',
        name: name,
        // 槽位可显式留空('' = 该事件静音);host 端对空串原样保存
        press: audioEditPressVal || '',
        release: audioEditReleaseVal || '',
      }),
    })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (d && d.ok && Array.isArray(d.groups)) {
          audioGroups = d.groups
          renderAudioGroupPanel()
          refreshTaskEndAfterAudio()
          // 新组保存后自动选中
          if (!editingAudioGroupId) {
            var newest = d.groups.filter(function (x) { return !x.preset }).sort(function (a, b) { return (b.pinnedAt || 0) - (a.pinnedAt || 0) })[0]
            if (newest) setSoundSet(newest.id)
          } else if (soundSet === editingAudioGroupId) {
            // 编辑的是当前正在用的组:槽位变化后重建音频元素(留空=静音即时生效)
            try { applySoundSet() } catch (err) {}
          }
          hideAudioEditor()
        }
      })
      .catch(function () {})
  } catch (err) {}
}
// —— 音频裁剪 ——
var audioCropFileInput = document.createElement('input')
audioCropFileInput.type = 'file'
audioCropFileInput.accept = 'audio/*'
audioCropFileInput.style.display = 'none'
dshwBodyAppend(audioCropFileInput)
var audioCropTarget = null // 'press' | 'release'
var audioCropCtx = null // AudioContext
var audioCropBuffer = null // AudioBuffer
var audioCropFileBase = '' // 源文件名（作为片段名）
var audioCropZoom = 1 // 波形水平缩放倍数
var audioCropOffset = 0 // 波形视窗左偏移（0..1，相对总时长比例）
audioEditPressImport.addEventListener('click', function () {
  audioCropTarget = 'press'
  audioCropFileInput.click()
})
audioEditReleaseImport.addEventListener('click', function () {
  audioCropTarget = 'release'
  audioCropFileInput.click()
})
audioCropFileInput.addEventListener('change', function () {
  var f = audioCropFileInput.files && audioCropFileInput.files[0]
  audioCropFileInput.value = ''
  if (!f) return
  var reader = new FileReader()
  reader.onload = function () {
    openAudioCrop(reader.result, f.name)
  }
  reader.readAsArrayBuffer(f)
})
function openAudioCrop(arrayBuf, fileName) {
  try {
    audioCropFileBase = (fileName || '音频片段').replace(/.[^.]+$/, '')
    audioCropZoom = 1
    audioCropOffset = 0
    // 重新导入新音频时立即停止旧试听，避免旧片段继续响
    stopAudioCropPreview()
    if (!audioCropCtx) {
      try { audioCropCtx = new (window.AudioContext || window.webkitAudioContext)() } catch (err) { audioCropCtx = null }
    }
    if (!audioCropCtx) { alert('当前浏览器不支持音频解码'); return }
    audioCropCtx.decodeAudioData(arrayBuf, function (buf) {
      audioCropBuffer = buf
      audioCropStart.value = '0'
      audioCropEnd.value = '100'
      audioCropStartNum.max = buf.duration.toFixed(3)
      audioCropEndNum.max = buf.duration.toFixed(3)
      audioCropZoomRange.value = '1'
      audioCropZoomNum.value = '1'
      drawAudioCrop()
      try { audioCropName.value = '' } catch (err) {}
      updateAudioCropOkState()
      dshwLayerUp(audioCropMask, 20500) // v744：音频裁剪也可能从资源管理(20300)里打开
      audioCropMask.style.display = 'flex'
    }, function () { alert('音频解码失败') })
  } catch (err) {}
}
function audioCropRange() {
  var b = audioCropBuffer
  if (!b) return null
  var s = Number(audioCropStart.value) / 100
  var e = Number(audioCropEnd.value) / 100
  if (e < s) { var t = s; s = e; e = t }
  return { start: b.duration * s, end: b.duration * e, s: s, e: e }
}
// 滚轮平移浏览波形：像浏览器滚轮滚动长页面一样，滚轮左右移动视窗（放大后才有平移空间）
function onAudioCropWheel(e) {
  if (!audioCropBuffer) return
  try { e.preventDefault(); e.stopPropagation() } catch (err) {}
  var maxOff = Math.max(0, 1 - 1 / audioCropZoom)
  if (maxOff <= 0) return
  // deltaY（纵向滚轮）映射为横向平移；触控板横向 deltaX 也可
  var dx = (e.deltaY || 0) + (e.deltaX || 0)
  if (dx === 0) return
  var move = dx / audioCropCanvas.width / audioCropZoom // 平移量（比例）
  audioCropOffset = Math.min(maxOff, Math.max(0, audioCropOffset + move))
  drawAudioCrop()
}
function drawAudioCrop(skipSync) {
  try {
    if (!audioCropBuffer) return
    var r = audioCropRange()
    audioCropTime.textContent = r.start.toFixed(1) + 's – ' + r.end.toFixed(1) + 's' + '（共 ' + audioCropBuffer.duration.toFixed(1) + 's，缩放 x' + audioCropZoom.toFixed(1) + '）'
    var ctx = audioCropCanvas.getContext('2d')
    var W = audioCropCanvas.width, H = audioCropCanvas.height
    ctx.clearRect(0, 0, W, H)
    ctx.fillStyle = '#f3f5fb'
    ctx.fillRect(0, 0, W, H)
    var ch = audioCropBuffer.getChannelData(0)
    var totalLen = ch.length
    var viewStart = audioCropOffset // 视口起点（0..1）
    var viewSpan = 1 / audioCropZoom // 视口宽度（比例）
    // 波形：逐像素采样视口内的最大振幅（用 min/max 更准确）
    ctx.strokeStyle = '#9fb0d9'
    ctx.lineWidth = 1
    ctx.beginPath()
    for (var x = 0; x < W; x++) {
      var g0 = viewStart + (x / W) * viewSpan
      var g1 = viewStart + ((x + 1) / W) * viewSpan
      var i0 = Math.max(0, Math.floor(g0 * totalLen))
      var i1 = Math.max(i0 + 1, Math.min(totalLen - 1, Math.ceil(g1 * totalLen)))
      var mn = 0, mx = 0
      for (var i = i0; i < i1; i++) {
        var v = ch[i]
        if (v < mn) mn = v
        if (v > mx) mx = v
      }
      var yTop = H / 2 - mx * H / 2
      var yBot = H / 2 - mn * H / 2
      ctx.moveTo(x, yTop)
      ctx.lineTo(x, yBot)
    }
    ctx.stroke()
    // 选中区：把 start/end 换算到视口坐标
    var vx0 = (r.s - viewStart) / viewSpan * W
    var vx1 = (r.e - viewStart) / viewSpan * W
    // 只绘制与视口相交的部分
    var drawX0 = Math.max(0, vx0)
    var drawX1 = Math.min(W, vx1)
    if (drawX1 > drawX0) {
      ctx.fillStyle = 'rgba(32,49,112,.25)'
      ctx.fillRect(drawX0, 0, drawX1 - drawX0, H)
      ctx.strokeStyle = '#203170'
      ctx.lineWidth = 2
      ctx.strokeRect(drawX0 + 0.5, 0.5, drawX1 - drawX0, H - 1)
    }
    // 视口边缘指示
    ctx.strokeStyle = 'rgba(32,49,112,.3)'
    ctx.lineWidth = 1
    ctx.beginPath()
    ctx.moveTo(0, H - 1); ctx.lineTo(W, H - 1)
    ctx.stroke()
    // 同步时间数字输入框（0.001s 精度）；skipSync 时（用户正在输入）不覆盖
    if (!skipSync) {
      audioCropStartNum.value = r.start.toFixed(3)
      audioCropEndNum.value = r.end.toFixed(3)
      audioCropZoomRange.value = String(audioCropZoom)
      audioCropZoomNum.value = String(Math.round(audioCropZoom * 100) / 100)
    }
    // 双滑块 thumb 与区间高亮定位（相对轨道；轨道有 4px 左右内缩）
    try {
      var trackW = audioCropDual.clientWidth || 200
      var usable = Math.max(1, trackW - 8)
      audioCropDualStart.style.left = (4 + r.s * usable) + 'px'
      audioCropDualEnd.style.left = (4 + r.e * usable) + 'px'
      audioCropDualFill.style.left = (4 + r.s * usable) + 'px'
      audioCropDualFill.style.width = Math.max(0, (r.e - r.s) * usable) + 'px'
    } catch (err) {}
  } catch (err) {}
}
// 起始滑条：滑块 → 时间数字框
function onAudioCropStartInput() {
  syncAudioCropStartNum()
  drawAudioCrop()
}
function syncAudioCropStartNum() {
  var b = audioCropBuffer
  if (!b) return
  audioCropStartNum.value = (b.duration * Number(audioCropStart.value) / 100).toFixed(3)
}
// 结束滑条：滑块 → 时间数字框
function onAudioCropEndInput() {
  syncAudioCropEndNum()
  drawAudioCrop()
}
function syncAudioCropEndNum() {
  var b = audioCropBuffer
  if (!b) return
  audioCropEndNum.value = (b.duration * Number(audioCropEnd.value) / 100).toFixed(3)
}
// 起始时间数字框：input 时只更新滑块（不覆盖输入，便于连续输入）；change 时规范化
function onAudioCropStartNumInput() {
  var b = audioCropBuffer
  if (!b) return
  var v = Number(audioCropStartNum.value)
  if (!isFinite(v)) return
  v = Math.min(Math.max(0, v), b.duration)
  var endSec = b.duration * Number(audioCropEnd.value) / 100
  if (v > endSec - 0.001) v = Math.max(0, endSec - 0.001)
  audioCropStart.value = String(v / b.duration * 100)
  drawAudioCrop(true)
}
function onAudioCropStartNumChange() {
  var b = audioCropBuffer
  if (!b) return
  var v = Number(audioCropStartNum.value)
  if (!isFinite(v)) v = 0
  v = Math.min(Math.max(0, v), b.duration)
  var endSec = b.duration * Number(audioCropEnd.value) / 100
  if (v > endSec - 0.001) v = Math.max(0, endSec - 0.001)
  audioCropStart.value = String(v / b.duration * 100)
  drawAudioCrop()
}
// 结束时间数字框：input 只更新滑块；change 规范化
function onAudioCropEndNumInput() {
  var b = audioCropBuffer
  if (!b) return
  var v = Number(audioCropEndNum.value)
  if (!isFinite(v)) return
  v = Math.min(Math.max(0, v), b.duration)
  var startSec = b.duration * Number(audioCropStart.value) / 100
  if (v < startSec + 0.001) v = Math.min(b.duration, startSec + 0.001)
  audioCropEnd.value = String(v / b.duration * 100)
  drawAudioCrop(true)
}
function onAudioCropEndNumChange() {
  var b = audioCropBuffer
  if (!b) return
  var v = Number(audioCropEndNum.value)
  if (!isFinite(v)) v = b.duration
  v = Math.min(Math.max(0, v), b.duration)
  var startSec = b.duration * Number(audioCropStart.value) / 100
  if (v < startSec + 0.001) v = Math.min(b.duration, startSec + 0.001)
  audioCropEnd.value = String(v / b.duration * 100)
  drawAudioCrop()
}
// 波形缩放条：滑块 + 数值（放大倍数）
// 统一缩放入口：以选中区间（start~end）的中点为锚点缩放，缩放后中点保持不动
function applyAudioCropZoom(v, skipSync) {
  if (!audioCropBuffer) return
  var next = Number(v)
  if (!isFinite(next) || next < 1) next = 1
  if (next > 50) next = 50
  var r = audioCropRange()
  var mid = (r.s + r.e) / 2 // 选中区间中点（全局 0..1）
  var viewSpan = 1 / audioCropZoom
  var midInView = viewSpan > 0 ? (mid - audioCropOffset) / viewSpan : 0.5 // 中点相对视口起点的比例
  audioCropZoom = next
  var newSpan = 1 / audioCropZoom
  audioCropOffset = mid - midInView * newSpan
  audioCropOffset = Math.min(1 - 1 / audioCropZoom, Math.max(0, audioCropOffset))
  audioCropZoomRange.value = String(audioCropZoom)
  audioCropZoomNum.value = String(Math.round(audioCropZoom * 100) / 100)
  drawAudioCrop(skipSync)
}
function onAudioCropZoomInput() {
  // 读滑块自身的值（之前误读数字框导致滑块不可拖动）
  applyAudioCropZoom(audioCropZoomRange.value)
}
// 波形缩放条数值框：input 只更新滑块（不覆盖输入）；change 规范化
function onAudioCropZoomNumInput() {
  var v = Number(audioCropZoomNum.value)
  if (!isFinite(v) || v < 1) return
  if (v > 50) return
  applyAudioCropZoom(v, true)
}
function onAudioCropZoomNumChange() {
  applyAudioCropZoom(audioCropZoomNum.value)
}
// 鼠标按下拖动选择音频段：按下起点 → 拖动设置 start/end → 松开结束
var audioCropSelDrag = null
function onAudioCropSelDown(e) {
  if (!audioCropBuffer) return
  try { e.preventDefault(); e.stopPropagation() } catch (err) {}
  var rect = audioCropCanvas.getBoundingClientRect()
  var W = audioCropCanvas.width
  var globalRatio = audioCropOffset + ((e.clientX - rect.left) / W) * (1 / audioCropZoom)
  globalRatio = Math.min(1, Math.max(0, globalRatio))
  audioCropSelDrag = { startRatio: globalRatio, moved: false }
  // 按住即把起始点设到该处
  audioCropStart.value = String(globalRatio * 100)
  syncAudioCropStartNum()
  drawAudioCrop()
}
function onAudioCropSelMove(e) {
  if (!audioCropSelDrag || !audioCropBuffer) return
  var rect = audioCropCanvas.getBoundingClientRect()
  var W = audioCropCanvas.width
  var globalRatio = audioCropOffset + ((e.clientX - rect.left) / W) * (1 / audioCropZoom)
  globalRatio = Math.min(1, Math.max(0, globalRatio))
  if (Math.abs(globalRatio - audioCropSelDrag.startRatio) > 0.002) audioCropSelDrag.moved = true
  if (audioCropSelDrag.moved) {
    // 拖动：小的为 start，大的为 end
    var s = Math.min(audioCropSelDrag.startRatio, globalRatio)
    var en = Math.max(audioCropSelDrag.startRatio, globalRatio)
    audioCropStart.value = String(s * 100)
    audioCropEnd.value = String(en * 100)
    syncAudioCropStartNum()
    syncAudioCropEndNum()
    drawAudioCrop()
  }
}
function onAudioCropSelUp() {
  audioCropSelDrag = null
}
// —— 双滑块：一条轨道上的起点/终点 thumb ——
var audioCropDualDrag = null // { side: 'start'|'end' }
function onAudioCropDualDown(e) {
  if (!audioCropBuffer) return
  try { e.preventDefault(); e.stopPropagation() } catch (err) {}
  var ratio = audioCropDualRatio(e)
  var startV = Number(audioCropStart.value) / 100
  var endV = Number(audioCropEnd.value) / 100
  // 靠近哪个 thumb 拖哪个；中间区域选更近的
  var dStart = Math.abs(ratio - startV)
  var dEnd = Math.abs(ratio - endV)
  audioCropDualDrag = { side: dStart <= dEnd ? 'start' : 'end' }
  audioCropDualMoveTo(ratio)
}
function onAudioCropDualMove(e) {
  if (!audioCropDualDrag || !audioCropBuffer) return
  audioCropDualMoveTo(audioCropDualRatio(e))
}
// 计算点击/拖动在轨道上的 0..1 比例（对齐 4px 内缩）
function audioCropDualRatio(e) {
  var rect = audioCropDual.getBoundingClientRect()
  var usable = Math.max(1, rect.width - 8)
  var ratio = (e.clientX - rect.left - 4) / usable
  return Math.min(1, Math.max(0, ratio))
}
function audioCropDualMoveTo(ratio) {
  if (!audioCropDualDrag) return
  var startV = Number(audioCropStart.value) / 100
  var endV = Number(audioCropEnd.value) / 100
  if (audioCropDualDrag.side === 'start') {
    // 起点不能超过终点
    if (ratio >= endV) ratio = Math.max(0, endV - 0.0001)
    audioCropStart.value = String(ratio * 100)
    syncAudioCropStartNum()
  } else {
    if (ratio <= startV) ratio = Math.min(1, startV + 0.0001)
    audioCropEnd.value = String(ratio * 100)
    syncAudioCropEndNum()
  }
  drawAudioCrop()
}
function onAudioCropDualUp() {
  audioCropDualDrag = null
}
function hideAudioCrop() {
  stopAudioCropPreview()
  audioCropMask.style.display = 'none'
  audioCropBuffer = null
  audioCropTarget = null
  audioCropFileBase = ''
  // v753（issue #135）：裁剪面板自己的 AudioContext 关闭面板就销毁 —— 它不复用，
  // 留着同样是"一条一直开着的系统音频流"（重新打开面板时导入音频会重新创建，见上面的 if (!audioCropCtx)）。
  try {
    if (audioCropCtx) audioCropCtx.close()
  } catch (err) {}
  audioCropCtx = null
  try { audioCropName.value = '' } catch (err) {}
  updateAudioCropOkState()
}
var audioCropPreviewNode = null
function stopAudioCropPreview() {
  try {
    if (audioCropPreviewNode) {
      audioCropPreviewNode.stop()
      audioCropPreviewNode.disconnect()
      audioCropPreviewNode = null
    }
  } catch (err) {}
}
function previewAudioCrop() {
  try {
    if (!audioCropBuffer || !audioCropCtx) return
    stopAudioCropPreview()
    var r = audioCropRange()
    // 从源 buffer 切片试听
    var len = Math.floor((r.end - r.start) * audioCropBuffer.sampleRate)
    if (len < 1) return
    var slice = audioCropCtx.createBuffer(audioCropBuffer.numberOfChannels, len, audioCropBuffer.sampleRate)
    for (var c = 0; c < audioCropBuffer.numberOfChannels; c++) {
      var src = audioCropBuffer.getChannelData(c)
      var dst = slice.getChannelData(c)
      var off = Math.floor(r.start * audioCropBuffer.sampleRate)
      for (var i = 0; i < len; i++) dst[i] = src[off + i] || 0
    }
    var srcNode = audioCropCtx.createBufferSource()
    srcNode.buffer = slice
    srcNode.connect(audioCropCtx.destination)
    audioCropPreviewNode = srcNode
    srcNode.onended = function () { if (audioCropPreviewNode === srcNode) audioCropPreviewNode = null }
    srcNode.start()
  } catch (err) {}
}
// 编码 WAV（16-bit PCM，支持多声道）——纯手写，零依赖
function encodeWav(buffer) {
  var numCh = buffer.numberOfChannels
  var sampleRate = buffer.sampleRate
  var len = buffer.length
  var bytesPerSample = 2
  var blockAlign = numCh * bytesPerSample
  var dataSize = len * blockAlign
  var ab = new ArrayBuffer(44 + dataSize)
  var dv = new DataView(ab)
  function writeStr(offset, s) {
    for (var i = 0; i < s.length; i++) dv.setUint8(offset + i, s.charCodeAt(i))
  }
  writeStr(0, 'RIFF')
  dv.setUint32(4, 36 + dataSize, true)
  writeStr(8, 'WAVE')
  writeStr(12, 'fmt ')
  dv.setUint32(16, 16, true)
  dv.setUint16(20, 1, true) // PCM
  dv.setUint16(22, numCh, true)
  dv.setUint32(24, sampleRate, true)
  dv.setUint32(28, sampleRate * blockAlign, true)
  dv.setUint16(32, blockAlign, true)
  dv.setUint16(34, 16, true)
  writeStr(36, 'data')
  dv.setUint32(40, dataSize, true)
  var offset = 44
  for (var i = 0; i < len; i++) {
    for (var c = 0; c < numCh; c++) {
      var v = buffer.getChannelData(c)[i]
      var s = Math.max(-1, Math.min(1, v))
      dv.setInt16(offset, s < 0 ? s * 0x8000 : s * 0x7fff, true)
      offset += 2
    }
  }
  return new Blob([ab], { type: 'audio/wav' })
}
function confirmAudioCrop() {
  try {
    if (!audioCropBuffer) return
    stopAudioCropPreview()
    var fragName = String(audioCropName.value || '').trim()
    if (!fragName) {
      // 空名提醒:红框 + 聚焦,不执行保存
      try {
        audioCropName.style.borderColor = '#e0433f'
        audioCropName.style.boxShadow = '0 0 0 2px rgba(224,67,63,.25)'
        audioCropName.focus()
        setTimeout(function () {
          audioCropName.style.borderColor = 'rgba(32,49,112,.4)'
          audioCropName.style.boxShadow = 'none'
        }, 1200)
      } catch (err) {}
      return
    }
    var r = audioCropRange()
    var len = Math.floor((r.end - r.start) * audioCropBuffer.sampleRate)
    if (len < 1) { alert('所选片段为空'); return }
    var slice = audioCropCtx.createBuffer(audioCropBuffer.numberOfChannels, len, audioCropBuffer.sampleRate)
    for (var c = 0; c < audioCropBuffer.numberOfChannels; c++) {
      var src = audioCropBuffer.getChannelData(c)
      var dst = slice.getChannelData(c)
      var off = Math.floor(r.start * audioCropBuffer.sampleRate)
      for (var i = 0; i < len; i++) dst[i] = src[off + i] || 0
    }
    var blob = encodeWav(slice)
    var reader = new FileReader()
    reader.onload = function () {
      uploadAudioFragment(reader.result, fragName, function (ok) {
        if (ok) {
          // 上传成功后自动选中新片段到对应槽位并更新按钮
          if (audioCropTarget === 'press') {
            audioEditPressVal = lastUploadedFragmentId || audioEditPressVal
            audioSlotBtnText(audioEditPressBtn, audioEditPressVal)
          } else if (audioCropTarget === 'release') {
            audioEditReleaseVal = lastUploadedFragmentId || audioEditReleaseVal
            audioSlotBtnText(audioEditReleaseBtn, audioEditReleaseVal)
          }
          // 资源窗口导入:片段入库后刷新其音频列表(不绑定槽位)
          var wasResImport = audioCropTarget === '__library__'
          // 重建试听音频：槽位值已变，若不重建试听仍指向旧音效（bug：导入后试听不更新）
          audioEditPreviewEnsure(true)
          hideAudioCrop()
          if (wasResImport) {
            try { openResManager() } catch (err) {}
          }
        }
      })
    }
    reader.readAsDataURL(blob)
  } catch (err) {}
}
var lastUploadedFragmentId = null
function uploadAudioFragment(dataUrl, name, cb) {
  try {
    fetch(AUDIO_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'upload-fragment', name: name, audio: dataUrl }),
    })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (d && d.ok && Array.isArray(d.fragments)) {
          audioFragments = d.fragments
          lastUploadedFragmentId = d.id || null
          refreshTaskEndAfterAudio()
          if (cb) cb(true)
        } else {
          if (cb) cb(false)
        }
      })
      .catch(function () { if (cb) cb(false) })
  } catch (err) { if (cb) cb(false) }
}
// 片段面板里删除自定义片段（预设不可删）
function audioDeleteFragmentInSlot(slot, id) {
  if (!id) return
  var f = null
  for (var i = 0; i < audioFragments.length; i++) if (audioFragments[i].id === id) { f = audioFragments[i]; break }
  if (!f || f.preset) return
  showConfirm('确定删除音频「' + f.name + '」吗？', function () {
    try {
      fetch(AUDIO_URL, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'delete-fragment', id: id }),
      })
        .then(function (r) { return r.json() })
        .then(function (d) {
          if (d && d.ok && Array.isArray(d.fragments)) {
            audioFragments = d.fragments
            if (Array.isArray(d.groups)) audioGroups = d.groups
            // 若当前槽位引用被删，回退预设
            if (slot === 'press' && audioEditPressVal === id) { audioEditPressVal = 'ya1'; audioSlotBtnText(audioEditPressBtn, 'ya1') }
            if (slot === 'release' && audioEditReleaseVal === id) { audioEditReleaseVal = 'ya2'; audioSlotBtnText(audioEditReleaseBtn, 'ya2') }
            // 重建试听音频：引用片段被删后若仍指向旧 URL 会失效（404/无声）
            audioEditPreviewEnsure(true)
            renderAudioGroupPanel()
            if (activeSlotPanel) renderAudioSlotPanel(slot)
            refreshTaskEndAfterAudio()
          }
        })
        .catch(function () {})
    } catch (err) {}
  })
}
// 槽位按钮：点击弹出片段面板
audioEditPressBtn.addEventListener('click', function (e) { e.stopPropagation(); toggleAudioSlotPanel('press') })
audioEditReleaseBtn.addEventListener('click', function (e) { e.stopPropagation(); toggleAudioSlotPanel('release') })

var hitCanvas = null
var hitReady = false
var hitFailed = false // 命中图加载失败（如图片 404/资源缺失）：退回矩形区域判定，绝不吞掉整页事件
function setupHitTest(url) {
  try {
    hitCanvas = document.createElement('canvas')
    hitCanvas.width = 610
    hitCanvas.height = 610
    hitReady = false
    hitFailed = false
    var probe = new Image()
    probe.onload = function () {
      try {
        var ctx = hitCanvas.getContext('2d')
        // 按 object-fit:contain + object-position:right bottom 的实际几何绘制，
        // 使非正方形（如 GIF 动图）的命中区域与显示一致
        var iw = probe.width || 610
        var ih = probe.height || 610
        var scale = Math.min(610 / iw, 610 / ih)
        var dw = iw * scale
        var dh = ih * scale
        var dx = 610 - dw // right bottom
        var dy = 610 - dh
        ctx.drawImage(probe, dx, dy, dw, dh)
        // v756（issue #144）：画布读回可能被浏览器**抹白** —— Firefox 开启「隐私保护 / resistFingerprinting」
        // 时 getImageData 会返回全透明（有些版本直接抛错）。那样 isWhaleHit 的 `data[3] > 10` 处处为假，
        // 而 hitReady 仍为 true ⇒ 走不到矩形回退分支 ⇒ **整只挂件点不动**。
        // 这里建立命中图时就扫一遍格子：一个不透明像素都取不到 → 判为命中图不可用，退回矩形判定。
        var opaque = 0
        for (var gy = 0; gy < 6; gy++) {
          for (var gx = 0; gx < 6; gx++) {
            var px = Math.min(609, Math.floor(dx + dw * (gx + 0.5) / 6))
            var py = Math.min(609, Math.floor(dy + dh * (gy + 0.5) / 6))
            if (ctx.getImageData(px, py, 1, 1).data[3] > 10) opaque++
          }
        }
        if (!opaque) { hitReady = false; hitFailed = true; applyHitClip(''); return }
        hitReady = true
        // v757（issue #147）：命中图可用 → 用不透明区域的凸包裁掉 img 的透明边距，
        // 这样"身体自己接事件"不会把透明角落的点击也吞掉。
        applyHitClip(buildHitClipPath())
      } catch (err) {
        hitFailed = true
        applyHitClip('')
      }
    }
    probe.onerror = function () {
      // 图片加载失败：不能把整页当成鲸鱼命中区吞掉事件（会全页面点不动），
      // 标记失败，命中判定退回图像矩形区域。
      hitFailed = true
      applyHitClip('')
    }
    probe.src = url || IMG_URL
  } catch (err) {}
}
// 退回「图像矩形」判定：命中图不可用（加载失败 / 画布读不出来）时只认挂件图片的矩形区域，
// 绝不把整页当成命中区（那会让全页面点不动）。
function whaleRectHit(e) {
  try {
    var fr = img.getBoundingClientRect()
    if (!fr || fr.width <= 0 || fr.height <= 0) return false
    return e.clientX >= fr.left && e.clientX <= fr.right && e.clientY >= fr.top && e.clientY <= fr.bottom
  } catch (err) { return false }
}
// —— v757（issue #147）：把命中图的不透明区域做成 clip-path ——
// `img` 现在常驻 `pointer-events:auto`（压在任何 iframe 上都能接事件），所以要靠 clip-path
// 把透明边距裁掉，保住"点到透明处穿透到下层"。用**凸包**而不是逐像素轮廓：凸包天然包含全部
// 不透明像素（不会裁到角色本身），点数少（几十个），且左右镜像由外层的 scaleX(-1) 一起变换。
function applyHitClip(clip) {
  try {
    img.style.clipPath = clip || ''
    img.style.webkitClipPath = clip || ''
  } catch (err) {}
}
function convexHull(pts) {
  try {
    var p = pts.slice().sort(function (a, b) { return a[0] - b[0] || a[1] - b[1] })
    if (p.length < 3) return null
    var cross = function (o, a, b) { return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]) }
    var lower = [], upper = [], i
    for (i = 0; i < p.length; i++) {
      while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], p[i]) <= 0) lower.pop()
      lower.push(p[i])
    }
    for (i = p.length - 1; i >= 0; i--) {
      while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], p[i]) <= 0) upper.pop()
      upper.push(p[i])
    }
    lower.pop(); upper.pop()
    return lower.concat(upper)
  } catch (err) { return null }
}
function buildHitClipPath() {
  try {
    var c = hitCanvas && hitCanvas.getContext ? hitCanvas.getContext('2d') : null
    if (!c) return ''
    var data = c.getImageData(0, 0, 610, 610).data // 只读一次（1.5MB），别逐行读
    var pts = []
    for (var y = 0; y < 610; y++) {
      var l = -1, r = -1
      var base = y * 610 * 4
      for (var x = 0; x < 610; x++) {
        if (data[base + x * 4 + 3] > 10) { if (l < 0) l = x; r = x }
      }
      if (l >= 0) { pts.push([l, y]); pts.push([r, y]) }
    }
    if (pts.length < 8) return ''
    var hull = convexHull(pts)
    if (!hull || hull.length < 3) return ''
    var parts = []
    for (var k = 0; k < hull.length; k++) {
      parts.push((hull[k][0] / 610 * 100).toFixed(2) + '% ' + (hull[k][1] / 610 * 100).toFixed(2) + '%')
    }
    return 'polygon(' + parts.join(', ') + ')'
  } catch (err) {
    return '' // 读不出来 → 不裁（退回矩形行为，与旧版一致）
  }
}
function isWhaleHit(e) {
  // 命中图未就绪/失败时：绝不默认“全屏都是鲸鱼”。
  // 加载中 → 返回 false（不拦截页面）；加载失败 → 退回图像矩形区域，仅挂件区域可拖。
  if (!hitCanvas || !hitReady) {
    if (!hitFailed) return false
    return whaleRectHit(e)
  }
  try {
    var r = img.getBoundingClientRect()
    if (!r || r.width <= 0 || r.height <= 0) return false
    var lx = (e.clientX - r.left) / r.width * 610
    var ly = (e.clientY - r.top) / r.height * 610
    if (lx < 0 || ly < 0 || lx >= 610 || ly >= 610) return false
    if (state.flip) lx = 610 - lx
    var data
    try {
      data = hitCanvas.getContext('2d').getImageData(Math.floor(lx), Math.floor(ly), 1, 1).data
    } catch (err) {
      // v756（issue #144）：画布读不出来（隐私保护/跨域污染）→ 立刻降级为矩形判定，
      // 并且**记住**这个状态，后续调用直接走矩形，不再每次抛错。
      hitReady = false
      hitFailed = true
      return whaleRectHit(e)
    }
    return data[3] > 10
  } catch (err) {
    return false
  }
}
function onDocPointerDown(e) {
  if (e.target && e.target.closest) {
    if (e.target.closest('.dshwv-pop') || e.target.closest('.dshwv-menu-btn')) return
    // 点击在面板/弹窗内部：交给面板自身逻辑处理
    if (e.target.closest('.dshwv-rolelist') || e.target.closest('.dshwv-audiolist') ||
        e.target.closest('.dshwv-cropmask') || e.target.closest('.dshwv-confirmmask') ||
        e.target.closest('.dshwv-audiomask') || e.target.closest('.dshwv-snapmask') ||
        e.target.closest('.dshwv-bubmask') || e.target.closest('.dshwv-qedit') || e.target.closest('.dshwv-usagepanel') || e.target.closest('.dshwv-usage-mask') ||
        e.target.closest('.dshwv-resmask') ||
        e.target.closest('.dshwv-custmenu') || e.target.closest('.dshwv-custbtn')) return
    // 菜单内的下拉切换按钮/导入按钮：它们自己的 click 负责开合，pointerdown 不干预
    if (e.target.closest('.dshwv-rolebtn') || e.target.closest('.dshwv-audiobtn') ||
        e.target.closest('.dshwv-roleimport') || e.target.closest('.dshwv-audioimport')) return
    // 点击在菜单内但不在任何面板/切换按钮上：收起所有下拉面板，菜单保持打开
    if (e.target.closest('.dshwv-menu')) {
      closeRolePanel()
      closeAudioGroupPanel()
      return
    }
  }
  // 右键(非左键鼠标)不在 pointerdown 阶段收起菜单：交给随后的 contextmenu 用 toggleMenu 做"开/关"，
  // 否则 pointerdown 先关、contextmenu 再开 —— 右键就只能唤出、无法关闭。
  var rightMouse = (e.pointerType === 'mouse' && e.button !== 0)
  if (menuOpen && !rightMouse) {
    closeMenu()
    return
  }
  if (e.button !== 0 && e.pointerType === 'mouse') return
  if (!isWhaleHit(e)) return
  try { e.preventDefault(); e.stopPropagation() } catch (err) {}
  var vp = viewport()
  var rect = root.getBoundingClientRect()
  drag = { active: true, startX: e.clientX, startY: e.clientY, origLeft: rect.left, origTop: rect.top, w: rect.width, h: rect.height, moved: false, vp: vp }
  root.classList.add('dshwv-dragging')
  pressDown()
  setWidgetCursor('grabbing')
  document.addEventListener('pointermove', onDocPointerMove, true)
  document.addEventListener('pointerup', onDocPointerUp, true)
  document.addEventListener('pointercancel', onDocPointerCancel, true)
}
function onDocPointerMove(e) {
  if (!drag || !drag.active) return
  var dx = e.clientX - drag.startX
  var dy = e.clientY - drag.startY
  if (dx * dx + dy * dy >= CLICK_SQ) drag.moved = true
  // Keep the pre-drag flip orientation while dragging (state.h/v stay as they
  // were); on release endDrag() recomputes the anchors and settle() flips the
  // class with a smooth transition instead of reverting instantly.
  state.left = clamp(drag.origLeft + dx, 0, Math.max(0, drag.vp.w - drag.w))
  // v785：拖拽时就把"桌面端窗口控件带"排除在可移动范围外（顶边下限 = 叠层下沿）
  state.top = clampWidgetTop(drag.origTop + dy, Math.max(0, drag.vp.h - drag.h))
  express()
}
function onDocPointerUp(e) {
  // 拦截鲸鱼区域内的 pointerup：防止下方元素（如文件行）监听 pointerup 穿透误触发
  try { if (isWhaleHit(e)) { e.preventDefault(); e.stopPropagation() } } catch (err) {}
  endDrag(e, true)
}
function onDocPointerCancel(e) { endDrag(e, false, true) }
function onDocClickStopper(e) {
  // 只在鲸鱼命中区域拦截 click（保持透明区 pass-through）。
  // 持久注册（不随 endDrag 移除）——click 在 pointerup 之后派发，
  // 若在 endDrag 移除会导致 click 穿透到下方元素（如误打开文件）。
  // 必须豁免挂件自身 UI：按钮/菜单/气泡/角色面板/裁剪/确认弹窗——
  // 自定义角色是不透明正方形图时，这些元素位于非透明像素上，
  // 若不豁免会被 isWhaleHit 判定命中而吞掉 click，导致点不动按钮/气泡。
  if (e.target && e.target.closest) {
    if (e.target.closest('.dshwv-pop') || e.target.closest('.dshwv-menu') || e.target.closest('.dshwv-menu-btn') ||
        e.target.closest('.dshwv-rolelist') || e.target.closest('.dshwv-cropmask') || e.target.closest('.dshwv-confirmmask') ||
        e.target.closest('.dshwv-audiolist') || e.target.closest('.dshwv-audiomask') ||
        e.target.closest('.dshwv-snapmask') || e.target.closest('.dshwv-bubmask') || e.target.closest('.dshwv-qedit') || e.target.closest('.dshwv-usagepanel') || e.target.closest('.dshwv-usage-mask') ||
        e.target.closest('.dshwv-resmask') ||
        e.target.closest('.dshwv-custmenu') || e.target.closest('.dshwv-custbtn')) return
  }
  if (!isWhaleHit(e)) return
  try { e.preventDefault(); e.stopPropagation() } catch (err) {}
}
// 右键小鲸鱼唤出菜单(仅在开启「隐藏菜单按钮」时生效;菜单位置与按钮唤出一致)
function onDocContextMenu(e) {
  try {
    if (!menuBtnHide) return
    // 触摸端:原生 contextmenu(部分 Android 约 500ms 就补发)一律吞掉不触发,
    // 唤出时机统一由我们的长按计时决定,避免早于设定时长或重复开合
    if (touchDrag || (touchEndedAt > 0 && Date.now() - touchEndedAt < 1500) || longPressRecent()) {
      e.preventDefault()
      return
    }
    if (e.target && e.target.closest) {
      if (e.target.closest('.dshwv-pop') || e.target.closest('.dshwv-menu') || e.target.closest('.dshwv-menu-btn') ||
          e.target.closest('.dshwv-rolelist') || e.target.closest('.dshwv-audiolist') || e.target.closest('.dshwv-cropmask') ||
          e.target.closest('.dshwv-confirmmask') || e.target.closest('.dshwv-audiomask') || e.target.closest('.dshwv-snapmask') ||
          e.target.closest('.dshwv-bubmask') || e.target.closest('.dshwv-qedit') || e.target.closest('.dshwv-usagepanel') || e.target.closest('.dshwv-usage-mask') ||
          e.target.closest('.dshwv-resmask') ||
          e.target.closest('.dshwv-custmenu') || e.target.closest('.dshwv-custbtn')) return
    }
    if (!isWhaleHit(e)) return
    e.preventDefault()
    toggleMenu()
  } catch (err) {}
}
document.addEventListener('pointerdown', onDocPointerDown, true)
document.addEventListener('click', onDocClickStopper, true)
document.addEventListener('contextmenu', onDocContextMenu, true)

// ===== 移动端触摸支持(v631) =====
// 鲸鱼是 pointer-events:none 的穿透层,手指真正按到的其实是下层页面元素(通常是可滚动区),
// 浏览器会把这次手势当成页面滚动 → 派发 pointercancel 掐断拖拽,表现为"手机上拖不动"。
// 这里只在"触摸起点命中鲸鱼"的这一次手势上阻止滚动(非 passive 监听),让 pointer 事件得以持续;
// 桌面鼠标不产生 touch 事件,原有 pointerdown/move/up 逻辑与透明区穿透完全不受影响。
function widgetUiHit(target) {
  if (!target || !target.closest) return false
  return !!(target.closest('.dshwv-pop') || target.closest('.dshwv-menu') || target.closest('.dshwv-menu-btn') ||
    target.closest('.dshwv-rolelist') || target.closest('.dshwv-audiolist') || target.closest('.dshwv-cropmask') ||
    target.closest('.dshwv-confirmmask') || target.closest('.dshwv-audiomask') || target.closest('.dshwv-snapmask') ||
    target.closest('.dshwv-bubmask') || target.closest('.dshwv-qedit') || target.closest('.dshwv-usagepanel') ||
    target.closest('.dshwv-usage-mask') || target.closest('.dshwv-resmask') || target.closest('.dshwv-custmenu') ||
    target.closest('.dshwv-custbtn') || target.closest('.dshwv-rolebtn') || target.closest('.dshwv-audiobtn') ||
    target.closest('.dshwv-roleimport') || target.closest('.dshwv-audioimport'))
}
var touchDrag = null // 正在接管滚动的触摸(仅"起点命中鲸鱼"的那一次手势)
// —— 移动端长按唤出菜单(v632):仅当开启「隐藏菜单按钮」时生效,替代电脑端的右键唤出 ——
var TOUCH_LONG_PRESS_MS = 1500
var TOUCH_LONG_PRESS_SLOP = 10 // 位移超过该像素即视为拖拽,取消长按
var touchStartPt = null
var touchNowPt = null
var touchLongPressTimer = null
var longPressFiredAt = 0
var touchEndedAt = 0 // 鲸鱼手势结束时刻(用于吞掉随之补发的原生 contextmenu)
function longPressRecent() { return longPressFiredAt > 0 && (Date.now() - longPressFiredAt) < 800 }
function cancelTouchLongPress() {
  if (touchLongPressTimer) { clearTimeout(touchLongPressTimer); touchLongPressTimer = null }
}
function fireTouchLongPressMenu() {
  cancelTouchLongPress()
  longPressFiredAt = Date.now()
  try { if (navigator && navigator.vibrate) navigator.vibrate(10) } catch (err) {}
  // 长按只用于唤出菜单:就地收尾当前拖拽(不移动鲸鱼),随后的抬手也不再触发 whaleClick
  try { if (drag && drag.active) endDrag({ clientX: touchNowPt ? touchNowPt.x : 0, clientY: touchNowPt ? touchNowPt.y : 0 }, false) } catch (err) {}
  toggleMenu()
}
function onDocTouchStart(e) {
  try {
    if (touchDrag) return
    if (!e.touches || e.touches.length !== 1) return
    // 挂件自身 UI(气泡/菜单/各类面板)不拦截:它们各有自己的点击与滚动需求
    if (widgetUiHit(e.target)) return
    var t = e.touches[0]
    if (!isWhaleHit({ clientX: t.clientX, clientY: t.clientY })) return
    touchDrag = { id: t.identifier }
    touchStartPt = { x: t.clientX, y: t.clientY }
    touchNowPt = { x: t.clientX, y: t.clientY }
    // 长按唤出菜单：原本只在「隐藏菜单按钮」开启且菜单未打开时挂计时(位移超标即取消)。
    // 触屏上默认配置(按钮永远不显形)就完全没有进菜单的路径 → 无 hover 的设备一律允许长按
    // 唤出（issue #91 缺陷2）；桌面端行为不变。
    if ((menuBtnHide || dshwvTouchUI()) && !menuOpen && (Date.now() - menuClosedAt > 600)) {
      cancelTouchLongPress()
      touchLongPressTimer = setTimeout(fireTouchLongPressMenu, TOUCH_LONG_PRESS_MS)
    }
    try { e.preventDefault() } catch (err) {}
    document.addEventListener('touchmove', onDocTouchMove, { capture: true, passive: false })
    document.addEventListener('touchend', onDocTouchEnd, true)
    document.addEventListener('touchcancel', onDocTouchEnd, true)
  } catch (err) {}
}
function onDocTouchMove(e) {
  // 多指(双指缩放)立刻放行,不干扰用户操作
  if (e.touches && e.touches.length > 1) { onDocTouchEnd(); return }
  try {
    var t = e.touches && e.touches[0]
    if (t) {
      touchNowPt = { x: t.clientX, y: t.clientY }
      if (touchLongPressTimer && touchStartPt) {
        var dx = t.clientX - touchStartPt.x
        var dy = t.clientY - touchStartPt.y
        if (dx * dx + dy * dy > TOUCH_LONG_PRESS_SLOP * TOUCH_LONG_PRESS_SLOP) cancelTouchLongPress()
      }
    }
  } catch (err) {}
  try { e.preventDefault() } catch (err) {}
}
function onDocTouchEnd() {
  cancelTouchLongPress()
  if (touchDrag) touchEndedAt = Date.now()
  touchDrag = null
  touchStartPt = null
  touchNowPt = null
  document.removeEventListener('touchmove', onDocTouchMove, true)
  document.removeEventListener('touchend', onDocTouchEnd, true)
  document.removeEventListener('touchcancel', onDocTouchEnd, true)
}
document.addEventListener('touchstart', onDocTouchStart, { capture: true, passive: false })

var widgetCursor = ''
function setWidgetCursor(v) {
  if (v === widgetCursor) return
  widgetCursor = v
  // v751（PR #119）：光标不写 document.body.style.cursor —— cursor 是可继承属性，写 <body> 会让 Blink
  // 失效整棵文档树的样式；而本函数在点击链路上按下/抬手各写一次，紧接着 isWhaleHit() 的
  // getBoundingClientRect() 与泡泡行测量的 getComputedStyle()/scrollWidth 会强制刷新样式+布局 ——
  // 整页重算就被算进了点击里。实测（见 PR #119）：5.7k 节点会话约 35ms/次、13.4k 节点约 179ms/次，
  // 而同样一次写入只落在挂件自己子树上约 1.3ms。所以改成切换挂件自己的类，光标由 CSS 承担。
  // 两个类必须互斥切换（只 add 不摘旧类会让光标一直停在 grabbing）。
  try {
    root.classList.toggle('dshwv-cursor-grab', v === 'grab')
    root.classList.toggle('dshwv-cursor-grabbing', v === 'grabbing')
  } catch (err) {}
}
// 鲸鱼图接过指针期间（见上面的 dshwv-cursor-* 类），滚轮要转交给「指针下方真正可滚动的容器」，
// 否则桌宠会变成一块滚不动的实心区域。只做一次临时让位取元素，全程只写挂件自己的内联样式，
// 不碰页面级样式；找不到可滚动祖先时什么都不做（此时浏览器仍按默认把滚动交给页面滚动容器）。
function onWhaleWheel(e) {
  try {
    // v757（issue #147）：img 现在**常驻** pointer-events:auto，所以不再用 widgetCursor 当闸门 ——
    // 这个监听挂在 root 上，只有挂件自己的子元素接住了事件才会触发；命中就转交。
    if (!e || !e.target) return
    var hit = e.target
    if (hit !== img && !(root && root.contains && root.contains(hit))) return
    var prev = hit.style ? hit.style.pointerEvents : ''
    var under = null
    try {
      if (hit.style) hit.style.pointerEvents = 'none' // 临时让开，取出指针下方真正的页面元素
      under = document.elementFromPoint(e.clientX, e.clientY)
    } catch (err) {}
    try { if (hit.style) hit.style.pointerEvents = prev } catch (err) {}
    var sc = under
    for (var hop = 0; sc && sc !== document.body && sc !== document.documentElement && hop < 12; hop++) {
      var st = null
      try { st = window.getComputedStyle(sc) } catch (err) {}
      if (st && (st.overflowY === 'auto' || st.overflowY === 'scroll' || st.overflowY === 'overlay') &&
          sc.scrollHeight > sc.clientHeight + 1) break
      sc = sc.parentElement
    }
    if (sc && sc !== document.body && sc !== document.documentElement) {
      var k = (e.deltaMode === 1) ? 16 : 1 // 1 = 行模式，按 16px 折算
      if (e.deltaY) sc.scrollTop += e.deltaY * k
      if (e.deltaX) sc.scrollLeft += e.deltaX * k
      e.preventDefault()
    }
  } catch (err) {}
}
try { root.addEventListener('wheel', onWhaleWheel, { passive: false }) } catch (err) {}
function onDocPointerMoveCursor(e) {
  if (drag && drag.active) {
    // v751（PR #119）：按钮已经松开却还在"拖动中" = 这一次 pointerup 丢了（例如松手时指针在窗口外）：
    // 不能继续强推 grabbing（光标会一直卡在"抓紧"），顺手补一次 endDrag 收尾，避免挂件继续黏着鼠标。
    // 鼠标/触摸/笔拖动期间 e.buttons 都是 1，buttons===0 只可能意味着真的松手了。
    // endDrag 内部还有 drag.moved 判定，所以这里传 clickAllowed=true 不会误触发点按。
    if (!e.buttons) { try { endDrag(e, true) } catch (err) {} ; return }
    setWidgetCursor('grabbing')
    return
  }
  var el = null
  try { el = document.elementFromPoint(e.clientX, e.clientY) } catch (err) {}
  if (el && el.closest && (el.closest('.dshwv-pop') || el.closest('.dshwv-menu') || el.closest('.dshwv-menu-btn') || el.closest('.dshwv-rolelist') || el.closest('.dshwv-cropmask') || el.closest('.dshwv-confirmmask') || el.closest('.dshwv-audiolist') || el.closest('.dshwv-audiomask') || el.closest('.dshwv-snapmask') || el.closest('.dshwv-bubmask') || el.closest('.dshwv-qedit') || el.closest('.dshwv-usagepanel') || el.closest('.dshwv-usage-mask') || el.closest('.dshwv-resmask') || el.closest('.dshwv-custmenu') || el.closest('.dshwv-custbtn'))) {
    setWidgetCursor('')
    if (!menuBtnHide) menuBtn.classList.add('dshwv-menu-btn-visible')
    return
  }
  var over = isWhaleHit(e)
  setWidgetCursor(over ? 'grab' : '')
  // 触屏上必须把 dshwvTouchUI() 也当作"该显示"：拖动鲸鱼时 pointermove 的 over 为 false，
  // 否则这一次 toggle 会把常显状态撤掉（issue #91 缺陷2 修完又被自己抹掉）。
  if (!menuBtnHide) menuBtn.classList.toggle('dshwv-menu-btn-visible', over || menuOpen || dshwvTouchUI())
}
document.addEventListener('pointermove', onDocPointerMoveCursor, true)
// 启动即应用一次菜单按钮可见性：触屏上 ☰ 常显（issue #91 缺陷2）。
// 配置读回来之后还会再应用一次，这里是配置请求失败时的兜底。
try { applyMenuBtnHideUI() } catch (err) {}

function endDrag(e, clickAllowed, cancelled) {
  if (!drag || !drag.active) return
  drag.active = false
  document.removeEventListener('pointermove', onDocPointerMove, true)
  document.removeEventListener('pointerup', onDocPointerUp, true)
  document.removeEventListener('pointercancel', onDocPointerCancel, true)
  pressUp()
  root.classList.remove('dshwv-dragging')
  // issue #79 缺陷2：pointercancel（Android 把手势判成页面滚动、或系统抢走手势时派发）的
  // clientX/clientY 常常是 0，而 endDrag 又是「按坐标收尾 + saveConfig() 落盘」——
  // 于是位移被算成"一口气拖到了 (0,0)"，归边判定吃进左上角，损坏锚点被写进 localStorage。
  // 0.3.2 的「非法距离自愈」只治负数 / 超出视口，救不回这个**合法的 (0,0)**，所以必须在这里拦住。
  // 处理：取消的手势一律回到按下前的位置、并且**不落盘**（取消不该提交位置）。
  var noCoord = (!e || typeof e.clientX !== 'number' || typeof e.clientY !== 'number' ||
                 !isFinite(e.clientX) || !isFinite(e.clientY))
  var zeroBoth = (e && e.clientX === 0 && e.clientY === 0 && drag.moved)
  if (cancelled || noCoord || zeroBoth) {
    try { state.left = drag.origLeft; state.top = drag.origTop } catch (err) {}
    setWidgetCursor('')
    settle()
    return
  }
  setWidgetCursor(isWhaleHit(e) ? 'grab' : '')
  if (clickAllowed && !drag.moved) {
    // 长按刚唤出菜单:这次抬手不再当作点击(避免顺带弹出余额泡)
    if (longPressRecent()) return
    whaleClick()
    refresh(true)
    return
  }
  var dx = e.clientX - drag.startX
  var dy = e.clientY - drag.startY
  var left = clamp(drag.origLeft + dx, 0, Math.max(0, drag.vp.w - drag.w))
  // v785：落位计算也要先排除"窗口控件带"（否则吸附判定点会用到带内的 top）
  var top = clampWidgetTop(drag.origTop + dy, Math.max(0, drag.vp.h - drag.h))
  // 自定义吸附区（比例/绝对/关闭）。判定点：左右吸附/翻转 = 图像中心 x，
  // 下吸附 = 图像中心 y，上吸附 = 挂件盒中心 y。
  // 解析计算，避免拖动结束过渡期强制布局。
  var ac = artCenterAt(left, top, drag.w, drag.h, !!state.flip)
  var z = snapZones(ac.cx, top + drag.h / 2, ac.cy, drag.vp)
  if (z.zH === 'left') {
    state.h = 'left'
    state.hOff = 0
  } else if (z.zH === 'right') {
    state.h = 'right'
    state.hOff = 0
  } else {
    state.h = null
    state.hOff = left
  }
  if (z.zV === 'top') {
    state.v = 'top'
    state.vOff = 0
  } else if (z.zV === 'bottom') {
    state.v = 'bottom'
    state.vOff = 0
  } else {
    state.v = null
    state.vOff = top
  }
  // 贴边侧固定朝向；自由摆放按图像中心 vs 翻转线
  state.flip = z.zH === 'left' ? true : (z.zH === 'right' ? false : z.flip)
  state.left = left
  state.top = top
  settle()
  // 拖拽结束立即保存锚点位置（否则刷新/关闭后位置回退到上次改菜单时）
  saveConfig()
}
// 窗口尺寸变化时：自由位置的鲸鱼按相对边框锚点重算（保持离边距离，窗口恢复原状即回原位）；
// 贴边吸附的鲸鱼走 settle()（保持贴边）
function applyAnchorPos() {
  try {
    var a = JSON.parse(localStorage.getItem('dshw-pos') || 'null')
    if (!a || a.v !== 2 || (a.hAnchor !== 'left' && a.hAnchor !== 'right') || typeof a.hDist !== 'number' ||
        (a.vAnchor !== 'top' && a.vAnchor !== 'bottom') || typeof a.vDist !== 'number') return false
    var vp = viewport()
    var w = root.offsetWidth || root.getBoundingClientRect().width || 0
    var h = root.offsetHeight || root.getBoundingClientRect().height || 0
    var maxOffH = Math.max(0, vp.w - w)
    var maxOffV = Math.max(0, vp.h - h)
    // issue #102 自愈：非法距离（负数 = 存进去时挂件已在屏幕外；超出视口 = 窗口变小/脏数据）
    // 一律夹回合法范围并把修正结果落盘 —— 老用户中了脏数据也能自己恢复，无需手清 localStorage。
    var hDist = isFinite(a.hDist) ? clamp(a.hDist, 0, maxOffH) : 0
    var vDist = isFinite(a.vDist) ? clamp(a.vDist, 0, maxOffV) : 0
    var healed = (hDist !== a.hDist || vDist !== a.vDist)
    // 与加载恢复一致：锚点存净距离，右锚点按当前避让开关叠加
    var effectiveRightDist = a.hAnchor === 'right' ? hDist + (scrollGapOn ? rightGap() : 0) : hDist
    var l = a.hAnchor === 'left' ? hDist : vp.w - effectiveRightDist - w
    var t = a.vAnchor === 'top' ? vDist : vp.h - vDist - h
    state.left = clamp(l, 0, maxOffH)
    // v785：锚点自愈/恢复也要排除"窗口控件带"（这条路径不一定紧跟 express，所以就地夹）
    state.top = clampWidgetTop(clamp(t, 0, maxOffV), maxOffV)
    state.h = a.hAnchor
    // 净距离直接还给 hOff/vOff：settle() 对锚定状态从偏移量重算，
    // 若置 0 会把刚恢复的距离覆盖成贴边（issue #43）
    state.hOff = hDist
    state.v = a.vAnchor
    state.vOff = vDist
    refreshFlip()
    if (healed) { try { saveAnchorPos() } catch (err) {} }
    return true
  } catch (err) { return false }
}
window.addEventListener('resize', function () {
  if (state.h === null && state.v === null && applyAnchorPos()) return
  settle()
})

var rect0 = root.getBoundingClientRect()
state.left = rect0.left
state.top = rect0.top
codexWhaleHost.bind({ root: root, menu: menuBox, refresh: refresh, showBubble: showBubble, playTaskEndSound: playTaskEndSound, pollWaitState: pollWaitState, pollLastTurn: pollLastTurn, openSoundSettings: openSoundSettingsPanel, openBubbleEditor: openBubbleEditor, openResources: openResManager })
express()
render()
applySoundSet()
setupHitTest(initRoleUrl)
loadRoles()
loadAudio()
// 用量设置(任务结束音/预警/预算)加载,并据此初始化主菜单“任务结束”行
loadUsageSettings(function () {
  try {
    if (usageSet && usageSet.taskEnd) {
      taskEndToggle.checked = !!usageSet.taskEnd.on
      taskEndSel.disabled = !usageSet.taskEnd.on
    }
    fillTaskEndOptions(usageSet && usageSet.taskEnd)
    // 音效片段异步就绪后若仍是预设默认,优先切到已导入的 entity
    setTimeout(function () {
      try {
        if (audioFragments && audioFragments.length) fillTaskEndOptions(usageSet && usageSet.taskEnd)
      } catch (err) {}
    }, 1500)
  } catch (err) {}
})
loadBubbleCfg()
fetch(SIZE_URL, { cache: 'no-store' })
  .then(function (r) { return r.json() })
  .then(function (d) {
    if (d && typeof d.scale === 'number' && d.scale >= MIN_SCALE - 0.1 && d.scale <= MAX_SCALE + 0.1) {
      state.scale = d.scale
      root.style.setProperty('--dshw-scale', String(d.scale))
      scaleInput.value = String(d.scale)
      scaleNumber.value = String(scaleToDisplay(d.scale))
      settle()
    }
    if (d && typeof d.vol === 'number') {
      soundVol = d.vol
      // v753（issue #135）：旧配置没有 sound 字段 → 沿用"音量>0 即开"的老语义；有新字段就以它为准
      if (typeof d.sound !== 'boolean') soundOn = soundVol > 0
      volInput.value = String(soundVol)
      volPct.textContent = Math.round(soundVol * 100) + '%'
      try {
        if (pressAudio) pressAudio.volume = soundVol
        if (releaseAudio) releaseAudio.volume = soundVol
      } catch (err) {}
    }
    // v753：音效总开关（configPayload 一直在写 sound 字段，但以前加载时被"音量>0"覆盖掉了 → 白存）
    if (d && typeof d.sound === 'boolean') {
      soundOn = d.sound
      try { soundToggle.checked = soundOn } catch (err) {}
    }
    if (d && typeof d.soundSet === 'string' && d.soundSet) {
      // 支持预设组（duck/fx1）和自定义组 id；loadAudio() 会校验自定义组是否存在
      soundSet = d.soundSet
      setAudioBtnText(audioGroupName(soundSet))
      applySoundSet()
    }
    if (d && typeof d.usageMode === 'string') {
      usageMode = 'ledger' // 旧 token 配置自动视为小鲸鱼记账
    }
    if (d && typeof d.peakMode === 'string' && (d.peakMode === 'liangwen' || d.peakMode === 'qiangqiang')) {
      // 旧版全局峰谷称呼:暂存一次,等泡泡配置就绪后批量写入各峰谷模块
      window.__dshwLegacyPeak = d.peakMode
      maybeBubbleMigratePeak()
    }
    peakMode = 'default'
    if (d && typeof d.bubbleOn === 'boolean') {
      bubbleOn = d.bubbleOn
      bubbleToggle.checked = bubbleOn
    }
    if (d && typeof d.turnCostOn === 'boolean') {
      turnCostOn = d.turnCostOn
      turnCostToggle.checked = turnCostOn
      turnCostCloseInput.disabled = !turnCostOn
    }
    if (d && typeof d.turnCostCloseMs === 'number') {
      turnCostCloseMs = d.turnCostCloseMs > 0 ? d.turnCostCloseMs : 0
      turnCostCloseInput.value = String(Math.round(turnCostCloseMs / 1000))
    }
    if (d && typeof d.scrollGapOn === 'boolean') {
      scrollGapOn = d.scrollGapOn
      scrollGapToggle.checked = scrollGapOn
      scrollGapInput.disabled = !scrollGapOn
    }
    if (d && typeof d.scrollGapPx === 'number') {
      scrollGapPx = d.scrollGapPx > 0 ? Math.round(d.scrollGapPx) : 0
      scrollGapInput.value = String(scrollGapPx)
    }
    if (d && typeof d.menuBtnHide === 'boolean') {
      menuBtnHide = d.menuBtnHide
      if (menuHideToggle) menuHideToggle.checked = menuBtnHide
    }
    // issue #116：Codex 本机统计开关（老配置里没有这个键 → 保持默认「开」）
    if (d && typeof d.codexStatsOn === 'boolean') {
      codexStatsOn = d.codexStatsOn
      if (codexStatsToggle) codexStatsToggle.checked = codexStatsOn
    }
    // 无论服务端带没带这个键都要应用一次：触屏上 ☰ 需要常显（issue #91 缺陷2），
    // 而旧写法只在键存在时才调用，空配置下按钮永远是透明的。
    applyMenuBtnHideUI()
    // 相对边框恢复（localStorage 锚点）：窗口变化后保持离边距离。
    // 仅认 v:2 净距离格式；旧格式（含避让距离）废弃，挂件保持默认右下角吸附。
    // issue #102：这里原与 applyAnchorPos() 各写了一份恢复逻辑（两份都只做下限夹紧），
    // 现在统一走 applyAnchorPos()，避免"只修一处、另一处仍复现"。
    try {
      if (applyAnchorPos()) {
        settle()
        // 恢复时 offsetWidth 可能还是 0（字体/图片尚未就绪），尺寸就绪后再夹一次（issue #102）
        setTimeout(function () { try { settle() } catch (err) {} }, 500)
      }
    } catch (err) {}
    refresh(false)
    // v734（issue #97）：首次 GET 应用完成 —— 从这一刻起才允许 saveConfig() 落盘
    configLoaded = true
    if (configSavePending) { configSavePending = false; try { saveConfig() } catch (err) {} }
  })
  .catch(function () {
    // 读取失败：绝不能拿内存里的默认值去 PUT（那正是「设置被洗成默认值」）
    refresh(false)
    dshwvToast('⚠ 设置读取失败，已暂停保存以免覆盖你的原有设置<br>请刷新页面重试')
  })
setInterval(function () { refresh(false); try { loadApiModels(null, true) } catch (err) {} }, REFRESH_MS)

// —— 每轮对话消耗检测：轮询 last-turn.json，出现新 seq 时弹消耗金额泡泡 ——
var LAST_TURN_URL = '/dsh-whale/last-turn.json'
var lastCostSeq = 0
var lastCostAligned = false
// 用 localStorage 记忆已看过的 seq：刷新页面后不吞新轮次（配合 host 端 seq 持久化）
try {
  var lastCostStored = Number(localStorage.getItem('dshw-last-seq') || 0)
  if (isFinite(lastCostStored) && lastCostStored >= 0) lastCostSeq = lastCostStored
} catch (err) {}
// v761（issue #161 / 全局音效设置）：**等待用户交互**的轮询。
// 并入既有的每秒周期（不新增 interval）；宿主 wait.json 给出「有没有挂起 / 哪一类 / 对话名」。这里负责：
//   ① 记住对话名，供提示内容里的 {session} 占位符使用；
//   ② 挂起**首次出现**时播对应音效（提问音 / 授权音），并在开启「冒泡提示」时弹泡泡；
//   ③ 用 localStorage 记住已响过的挂起 id ⇒ **刷新页面不会为同一个未回答的提问重复响**。
var WAIT_URL = '/dsh-whale/wait.json'
var WAIT_SOUND_KEY = 'dshw-wait-sound'
var waitSeenId = ''
function waitEventCfg(kind) {
  try { return ((usageSet || {}).events || {})[kind] || {} } catch (err) { return {} }
}
function pollWaitState() {
  try {
    fetch(WAIT_URL, { cache: 'no-store' })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (!d || !d.ok) return
        if (typeof d.sessionName === 'string') waitSessionName = d.sessionName
        var p = d.pending || null
        if (!p || !p.kind) {
          // 挂起已解除：收起泡泡，并允许下一次挂起照常响
          waitSeenId = ''
          waitPendingId = '' // v777：挂起解除 ⇒ "被点掉"的标记一并作废（下一次挂起照常冒泡）
          waitDismissedId = ''
          if (typeof hideWaitBubble === 'function') hideWaitBubble()
          return
        }
        var kind = p.kind === 'approval' ? 'approval' : 'question'
        var ev = waitEventCfg(kind)
        // v777：这条挂起已经被用户点掉了 ⇒ 不再自动弹回（但仍然照常响铃判定与去重；回答/批准后
        // 上面的 !p 分支会把标记清掉，所以"下一次新的提问/授权"照常冒泡）。
        waitPendingId = String(p.id || '')
        if (waitPendingId && waitPendingId === waitDismissedId) {
          // 什么都不做：既不显示，也不清 waitSeenId（同一挂起不重复响）
        } else {
          if (ev.bubbleOn !== false && typeof showWaitBubble === 'function') showWaitBubble(kind, p)
        }
        // v771：**每秒都把泡泡与宿主的挂起状态对齐一次**（showWaitBubble 幂等：同类直接返回 false，
        // 不会重渲染）。旧实现只在"新 id"时才调显示，于是一旦出现瞬时不同步（换类型 / 泡泡被别的
        // 场景顶掉 / 某次渲染失败），泡泡就会卡在那个状态再也回不来 —— 这正是真机那次卡死的另一半原因。
        // 现在任何不同步都会在 1 秒内自愈：该换内容就换、该收就收（收在下面 !p 分支）。
        // 门控照旧：该事件的「冒泡提示」关掉就不冒泡（与声音各自独立判）。
        // v777：显示那一步已上移到"被点掉的挂起"判定里（见上面的 waitDismissedId 分支），这里不再重复调用。
        // 声音：同一个未回答的挂起只响一次（刷新页面也不重复响）
        var key = kind + ':' + String(p.id || '')
        var remembered = ''
        try { remembered = localStorage.getItem(WAIT_SOUND_KEY) || '' } catch (err) {}
        if (key === waitSeenId || key === remembered) return
        waitSeenId = key
        try { localStorage.setItem(WAIT_SOUND_KEY, key) } catch (err) {}
        var on = ev.on !== false
        var sel = ev.sel || ''
        // v762：音效行新增的「是否播这个音效」开关（events.<kind>.soundOn，缺省视为开）。
        // 门控 = 该事件总开关 on + 选了音效 sel（下拉已无「静音」，正常不为空）+ soundOn。
        // 任务结束音的播放判定不在这里，仍读既有的 usageSet.taskEnd.on（见 playTaskEndSound）。
        if (on && sel && ev.soundOn !== false) playBindingSound({ on: true, sel: sel }, ev)
      })
      .catch(function () {})
  } catch (err) {}
}
function pollLastTurn() {
  try {
    fetch(LAST_TURN_URL, { cache: 'no-store' })
      .then(function (r) { return r.json() })
      .then(function (d) {
        if (!d || !d.ok || typeof d.seq !== 'number') return
        if (!lastCostAligned) {
          // 首次拿到数据：以本地记忆的 seq 为基准，只弹比它更新的轮次
          lastCostAligned = true
          if (d.seq > lastCostSeq) {
            lastCostSeq = d.seq
            try { localStorage.setItem('dshw-last-seq', String(lastCostSeq)) } catch (err) {}
            playTaskEndSound()
            if (d.turn !== null) { codexWhaleHost.complete(d, showCostBubble) }
          } else {
            // 记忆值已过期（host 端重置/文件丢失）：拉齐到当前值，避免永久不弹
            lastCostSeq = d.seq
            try { localStorage.setItem('dshw-last-seq', String(lastCostSeq)) } catch (err) {}
          }
          return
        }
        if (d.seq > lastCostSeq) {
          lastCostSeq = d.seq
          try { localStorage.setItem('dshw-last-seq', String(lastCostSeq)) } catch (err) {}
          playTaskEndSound()
          if (d.turn !== null) { codexWhaleHost.complete(d, showCostBubble) }
        }
      })
      .catch(function () {})
  } catch (err) {}
}
setInterval(function () { pollLastTurn(); pollWaitState() }, 1000)
}
// 主界面检测通过（或稍后由 MutationObserver 检测到）后执行挂件初始化；非主界面不启动
try { dshwTryStart(true) } catch (err) {}
})()
