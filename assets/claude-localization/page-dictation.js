/**
 * Claude Desktop 语音输入 —— 页面侧脚本
 *
 * 由 hook.js 通过 CDP 注入（Page.addScriptToEvaluateOnNewDocument + 已加载页面补一次
 * Runtime.evaluate，所以必须幂等）。它与 page-shim.js 分工不同：那边改界面语言，
 * 这边在输入框右下角加一个麦克风入口。
 *
 * 为什么不点亮官方听写：官方听写的开关来自部署配置，服务地址固定为
 * `<inference.baseUrl>/user/dictation`（WebSocket），3p/gateway 部署下由第三方网关决定，
 * 我们既没有那个协议，也不该把聊天入口指到别处。所以自己录音，交给 Codex++ 已经配好的
 * ASR（本地 SenseVoice 或用户配置的 OpenAI 兼容接口），只借用官方的输入框。
 *
 * 通道：页面 → window.__claudeCnDictationCall（CDP binding）→ 主进程 hook →
 *       Codex++ helper /dictation/transcribe → 文本 → window.__claudeCnDictationDeliver。
 *
 * 生命周期：宿主是 React 应用，输入框和工具条会被反复重建。这里不缓存 DOM，靠
 * MutationObserver 重挂；注册的状态都挂在 window 上，注入多次只有第一份生效。
 */

(function () {
  'use strict';

  var MARK = 'data-claude-cn-dictation';
  var BUTTON_ID = 'claude-cn-dictation-button';
  var STYLE_ID = 'claude-cn-dictation-style';
  /** 录到这么长自动停，避免用户忘了点停止一直占着麦克风。 */
  var MAX_RECORD_MS = 5 * 60 * 1000;

  try {
    var isClaudeHost = /(^|\.)claude\.(ai|com)$/.test(location.hostname);
    // 本地主界面：主界面是 Resources/ion-dist 里的本地前端，用 app://localhost 提供
    var isLocalUi = location.protocol === 'app:' && location.hostname === 'localhost';
    if (!isClaudeHost && !isLocalUi) return;
    if (window.__claudeCnDictationInstalled) return;
    window.__claudeCnDictationInstalled = true;

    var state = {
      phase: 'idle', // idle | recording | transcribing
      editor: null,
      dock: null,
      button: null,
      stream: null,
      recorder: null,
      chunks: [],
      timer: 0,
      pending: {},
      notice: null,
      noticeTimer: 0,
    };

    // ---------- 通道 ----------

    /** 调用主进程。CDP binding 可能还没就绪，这里等一小会儿再放弃。 */
    function callHost(message, attempt) {
      return new Promise(function (resolve, reject) {
        var id = 'd' + Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
        state.pending[id] = { resolve: resolve, reject: reject };
        var send = function () {
          var call = window.__claudeCnDictationCall;
          if (typeof call !== 'function') {
            if ((attempt || 0) < 20) {
              setTimeout(send, 150);
              return;
            }
            delete state.pending[id];
            reject(new Error('语音通道未就绪，请重开 Claude 副本'));
            return;
          }
          try {
            call(JSON.stringify(Object.assign({ id: id }, message)));
          } catch (e) {
            delete state.pending[id];
            reject(new Error('语音请求发送失败'));
            return;
          }
          setTimeout(function () {
            if (!state.pending[id]) return;
            delete state.pending[id];
            reject(new Error('语音服务没有响应'));
          }, 180000);
        };
        send();
      });
    }

    /** 主进程回投结果（hook.js deliver 调用）。 */
    window.__claudeCnDictationDeliver = function (id, message) {
      var entry = state.pending[id];
      if (!entry) return;
      delete state.pending[id];
      if (message && message.ok) entry.resolve(message);
      else entry.reject(new Error((message && message.message) || '语音转写失败'));
    };

    // ---------- 样式 ----------

    function installStyle() {
      if (document.getElementById(STYLE_ID)) return;
      var style = document.createElement('style');
      style.id = STYLE_ID;
      style.setAttribute(MARK, 'style');
      style.textContent =
        '#' + BUTTON_ID + '{display:inline-flex;align-items:center;justify-content:center;' +
        // align-self 是必需的：插进 Cowork 那一行时，宿主容器的默认对齐会把固定高度的
        // 按钮顶到上沿，看起来比发送键高半格。
        'align-self:center;width:24px;height:24px;padding:0;border:0;border-radius:6px;background:transparent;' +
        'color:inherit;opacity:.72;cursor:pointer;flex:0 0 auto;transition:opacity .15s,background .15s}' +
        '#' + BUTTON_ID + ':hover{opacity:1;background:color-mix(in srgb,currentColor 12%,transparent)}' +
        '#' + BUTTON_ID + '[data-phase="recording"]{opacity:1;color:#e5484d;background:color-mix(in srgb,#e5484d 16%,transparent)}' +
        '#' + BUTTON_ID + '[data-phase="transcribing"]{opacity:.85;cursor:progress}' +
        '#' + BUTTON_ID + '[data-phase="transcribing"] svg{animation:claude-cn-dictation-spin 1s linear infinite}' +
        '#' + BUTTON_ID + '[disabled]{opacity:.4;cursor:not-allowed}' +
        '@keyframes claude-cn-dictation-spin{from{transform:rotate(0)}to{transform:rotate(360deg)}}' +
        '.claude-cn-dictation-notice{position:fixed;left:50%;bottom:96px;transform:translateX(-50%);' +
        'z-index:2147483000;max-width:min(420px,80vw);padding:8px 14px;border-radius:10px;' +
        'font-size:13px;line-height:18px;background:rgba(28,28,30,.94);color:#fff;' +
        'box-shadow:0 6px 24px rgba(0,0,0,.28);pointer-events:none}';
      (document.head || document.documentElement).appendChild(style);
    }

    // ---------- 图标与按钮 ----------

    function micIcon() {
      var svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      svg.setAttribute('viewBox', '0 0 24 24');
      svg.setAttribute('width', '16');
      svg.setAttribute('height', '16');
      svg.setAttribute('fill', 'none');
      svg.setAttribute('stroke', 'currentColor');
      svg.setAttribute('stroke-width', '1.8');
      svg.setAttribute('stroke-linecap', 'round');
      svg.setAttribute('stroke-linejoin', 'round');
      var path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
      path.setAttribute('d', 'M12 3.5a3 3 0 0 0-3 3v5a3 3 0 0 0 6 0v-5a3 3 0 0 0-3-3Z');
      var arc = document.createElementNS('http://www.w3.org/2000/svg', 'path');
      arc.setAttribute('d', 'M5.5 11.5a6.5 6.5 0 0 0 13 0M12 18v2.5');
      svg.appendChild(path);
      svg.appendChild(arc);
      return svg;
    }

    function spinnerIcon() {
      var svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      svg.setAttribute('viewBox', '0 0 24 24');
      svg.setAttribute('width', '16');
      svg.setAttribute('height', '16');
      svg.setAttribute('fill', 'none');
      svg.setAttribute('stroke', 'currentColor');
      svg.setAttribute('stroke-width', '2');
      svg.setAttribute('stroke-linecap', 'round');
      var path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
      path.setAttribute('d', 'M12 3.5a8.5 8.5 0 1 0 8.5 8.5');
      svg.appendChild(path);
      return svg;
    }

    var LABELS = {
      idle: '语音输入',
      recording: '停止录音',
      transcribing: '正在识别…',
    };

    function renderButton() {
      var button = state.button;
      if (!button) return;
      button.dataset.phase = state.phase;
      button.replaceChildren(state.phase === 'transcribing' ? spinnerIcon() : micIcon());
      button.setAttribute('aria-label', LABELS[state.phase]);
      button.setAttribute('title', LABELS[state.phase]);
      button.disabled = state.phase === 'transcribing';
    }

    // ---------- 提示 ----------

    function notify(message) {
      try {
        if (state.notice && state.notice.parentNode) state.notice.parentNode.removeChild(state.notice);
        var node = document.createElement('div');
        node.className = 'claude-cn-dictation-notice';
        node.setAttribute(MARK, 'notice');
        node.textContent = String(message);
        (document.body || document.documentElement).appendChild(node);
        state.notice = node;
        clearTimeout(state.noticeTimer);
        state.noticeTimer = setTimeout(function () {
          if (node.parentNode) node.parentNode.removeChild(node);
          if (state.notice === node) state.notice = null;
        }, 4000);
      } catch (e) {}
    }

    // ---------- 定位输入框 ----------

    function visible(node) {
      if (!node || !node.isConnected) return false;
      var rect = node.getBoundingClientRect();
      if (!rect.width || !rect.height) return false;
      return !node.closest('[inert], [aria-hidden="true"]');
    }

    function findEditor() {
      var nodes = document.querySelectorAll(
        '.ProseMirror[contenteditable="true"], [contenteditable="true"][role="textbox"], textarea:not([readonly])',
      );
      var first = null;
      for (var i = 0; i < nodes.length; i++) {
        var node = nodes[i];
        if (!visible(node)) continue;
        if (node.closest('[' + MARK + ']')) continue;
        // 主对话输入框优先：编辑历史消息时页面里会有第二个编辑器，入口只跟着主输入框走。
        if (node.closest('[data-testid="composer-line-v1"]')) return node;
        if (!first) first = node;
      }
      return first;
    }

    /**
     * 找工具条容器：按钮最终落在输入框右下角那一排。
     *
     * 官方结构随版本变化，这里按「明确的 composer 行 → 最近的含按钮祖先 → 编辑器本身」
     * 三级回退，任一级都不保证永远命中，所以插不进工具条时退化成编辑器下方的独立入口。
     */
    function findDock(editor) {
      var marked = editor.closest('[data-testid="composer-line-v1"], [data-testid="chat-footer-row"]');
      if (marked) return marked;
      var node = editor.parentElement;
      for (var depth = 0; depth < 5 && node && node !== document.body; depth++) {
        if (node.querySelector('button') && node.contains(editor)) return node;
        node = node.parentElement;
      }
      return editor.parentElement || null;
    }

    /**
     * 找发送键所在的那一排按钮，返回插入位置。
     *
     * 两套界面的发送键完全不同：Code 模式是 `code-prompt-send` /「发送」，Cowork 模式是
     * `chat-input-send` /「开始任务」。所以先按 testid 命中，再按动作词兜底，最后退到
     * 「发送键的父容器」——实测 Cowork 的父容器就是一个只放发送键的 flex，插在它前面
     * 正好落在同一排。官方自己的动作按钮都排在带 gap 的 flex 里，优先用那种容器。
     */
    function findActionRow(scope) {
      var send = scope.querySelector(
        '[data-testid="code-prompt-send"], [data-testid="chat-input-send"], [data-testid="epitaxy-cds-send"]',
      );
      if (!send) {
        var buttons = scope.querySelectorAll('button');
        for (var i = 0; i < buttons.length; i++) {
          var label = (buttons[i].getAttribute('aria-label') || buttons[i].title || '').trim();
          if (/^(发送|开始任务|新建任务|Send|Start|运行|Run|停止|Stop)/.test(label)) {
            send = buttons[i];
            break;
          }
        }
      }
      if (!send) return null;
      // 从发送键往上找「这一行的容器」：横向 flex 且带间距，说明官方在这一行排按钮。
      // 注意要往上够深：Cowork 的发送键外面裹了 4 层单元素 div，第 5 层才是那一行，
      // 上溯不够就会退化成「塞进发送键的父容器」，结果贴着左边的模型选择器、还贴顶。
      var node = send;
      for (var depth = 0; depth < 8 && node && node.parentElement; depth++) {
        var parent = node.parentElement;
        var style = window.getComputedStyle(parent);
        if (style.display.indexOf('flex') >= 0 && style.gap && style.gap !== 'normal') {
          // 插到「含发送键的那个子元素」之前，而不是容器最前面——容器的第一个子元素
          // 可能是模型选择器之类的左侧内容。
          return { row: parent, before: node, spaced: true };
        }
        node = parent;
      }
      return send.parentElement ? { row: send.parentElement, before: send, spaced: false } : null;
    }

    // ---------- 录音 ----------

    function pickMime() {
      var candidates = ['audio/webm;codecs=opus', 'audio/webm', 'audio/mp4'];
      if (typeof MediaRecorder === 'undefined') return '';
      for (var i = 0; i < candidates.length; i++) {
        try {
          if (MediaRecorder.isTypeSupported(candidates[i])) return candidates[i];
        } catch (e) {}
      }
      return '';
    }

    function stopTracks() {
      try {
        if (state.stream) state.stream.getTracks().forEach(function (track) { track.stop(); });
      } catch (e) {}
      state.stream = null;
    }

    /** 停止录音并等 MediaRecorder 收尾；某些实现不派发 stop，所以用超时兜底。 */
    function stopRecording() {
      return new Promise(function (resolve) {
        var recorder = state.recorder;
        if (!recorder) {
          resolve(null);
          return;
        }
        var settled = false;
        var finish = function (blob) {
          if (settled) return;
          settled = true;
          state.recorder = null;
          state.chunks = [];
          stopTracks();
          resolve(blob);
        };
        recorder.addEventListener('stop', function () {
          var type = recorder.mimeType || 'audio/webm';
          finish(state.chunks.length ? new Blob(state.chunks, { type: type }) : null);
        });
        try {
          recorder.stop();
        } catch (e) {
          finish(null);
          return;
        }
        setTimeout(function () { finish(null); }, 4000);
      });
    }

    async function startRecording() {
      if (typeof MediaRecorder === 'undefined' || !navigator.mediaDevices?.getUserMedia) {
        notify('当前环境不支持录音');
        return;
      }
      var stream;
      try {
        stream = await navigator.mediaDevices.getUserMedia({
          audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
        });
      } catch (error) {
        notify('无法使用麦克风：' + (error && error.message ? error.message : '权限被拒绝'));
        return;
      }
      state.stream = stream;
      state.chunks = [];
      var mime = pickMime();
      var recorder;
      try {
        recorder = mime ? new MediaRecorder(stream, { mimeType: mime }) : new MediaRecorder(stream);
      } catch (error) {
        stopTracks();
        notify('录音初始化失败');
        return;
      }
      state.recorder = recorder;
      recorder.addEventListener('dataavailable', function (event) {
        if (event.data && event.data.size) state.chunks.push(event.data);
      });
      recorder.start(1000);
      state.phase = 'recording';
      renderButton();
      clearTimeout(state.timer);
      state.timer = setTimeout(function () {
        if (state.phase === 'recording') finishRecording();
      }, MAX_RECORD_MS);
    }

    /**
     * 把任意浏览器能解码的录音转成 16 kHz 单声道 WAV。
     *
     * 本地 SenseVoice 只吃 WAV（后端会校验 RIFF 头），走 WebAudio 重采样比让后端依赖
     * ffmpeg 更省事；外部 OpenAI 兼容接口同样接受 wav。
     */
    async function toWav(blob) {
      var AudioContextCtor = window.AudioContext || window.webkitAudioContext;
      var OfflineContextCtor = window.OfflineAudioContext || window.webkitOfflineAudioContext;
      if (!AudioContextCtor || !OfflineContextCtor) return blob;
      var context = new AudioContextCtor();
      try {
        var decoded = await context.decodeAudioData(await blob.arrayBuffer());
        var frames = Math.max(1, Math.ceil(decoded.duration * 16000));
        var offline = new OfflineContextCtor(1, frames, 16000);
        var source = offline.createBufferSource();
        source.buffer = decoded;
        source.connect(offline.destination);
        source.start();
        var rendered = await offline.startRendering();
        var samples = rendered.getChannelData(0);
        var bytes = new ArrayBuffer(44 + samples.length * 2);
        var view = new DataView(bytes);
        var write = function (offset, text) {
          for (var i = 0; i < text.length; i++) view.setUint8(offset + i, text.charCodeAt(i));
        };
        write(0, 'RIFF');
        view.setUint32(4, 36 + samples.length * 2, true);
        write(8, 'WAVE');
        write(12, 'fmt ');
        view.setUint32(16, 16, true);
        view.setUint16(20, 1, true);
        view.setUint16(22, 1, true);
        view.setUint32(24, 16000, true);
        view.setUint32(28, 32000, true);
        view.setUint16(32, 2, true);
        view.setUint16(34, 16, true);
        write(36, 'data');
        view.setUint32(40, samples.length * 2, true);
        for (var i = 0; i < samples.length; i++) {
          var sample = Math.max(-1, Math.min(1, samples[i]));
          view.setInt16(44 + i * 2, sample < 0 ? sample * 0x8000 : sample * 0x7fff, true);
        }
        return new Blob([bytes], { type: 'audio/wav' });
      } catch (e) {
        return blob;
      } finally {
        try { context.close && context.close(); } catch (e) {}
      }
    }

    function base64Of(blob) {
      return new Promise(function (resolve, reject) {
        var reader = new FileReader();
        reader.onload = function () {
          var text = String(reader.result || '');
          var comma = text.indexOf(',');
          resolve(comma >= 0 ? text.slice(comma + 1) : text);
        };
        reader.onerror = function () { reject(new Error('录音读取失败')); };
        reader.readAsDataURL(blob);
      });
    }

    // ---------- 回填输入框 ----------

    /** 取当前可写入的编辑器：录音期间宿主可能已重建，旧引用会断开。 */
    function usableEditor() {
      if (state.editor && state.editor.isConnected) return state.editor;
      return findEditor();
    }

    /**
     * 把识别结果写进编辑器。
     *
     * 优先走 ProseMirror 事务：直接改 DOM 不会同步 React/编辑器状态，草稿、附件、
     * 撤销栈都会错位。拿不到 view 时再退到 execCommand。
     */
    function insertText(editor, text) {
      if (!editor || !text) return false;
      if (!editor.isConnected) return false;
      try {
        editor.focus();
        var view = editor.pmViewDesc && editor.pmViewDesc.view;
        if (view && view.state && view.state.tr && typeof view.dispatch === 'function') {
          var selection = view.state.selection;
          var from = selection.from;
          var before = view.state.doc.textBetween(Math.max(0, from - 1), from, '\n');
          var prefix = before && !/\s$/.test(before) ? ' ' : '';
          view.dispatch(view.state.tr.insertText(prefix + text, from, selection.to).scrollIntoView());
          return true;
        }
        return document.execCommand('insertText', false, text) === true;
      } catch (e) {
        return false;
      }
    }

    // ---------- 主流程 ----------

    async function finishRecording() {
      clearTimeout(state.timer);
      var blob = await stopRecording();
      if (!blob || !blob.size) {
        state.phase = 'idle';
        renderButton();
        notify('没有录到声音');
        return;
      }
      state.phase = 'transcribing';
      renderButton();
      try {
        var wav = await toWav(blob);
        var audioBase64 = await base64Of(wav);
        var result = await callHost({
          action: 'transcribe',
          audioBase64: audioBase64,
          mimeType: wav.type || 'audio/wav',
          filename: 'dictation.wav',
        });
        var text = String((result && result.text) || '').trim();
        if (!text) {
          notify('未识别到文字，请重试');
        } else if (!insertText(usableEditor(), text)) {
          notify('识别成功，但写入输入框失败：' + text);
        }
      } catch (error) {
        notify((error && error.message) || '语音识别失败');
      } finally {
        state.phase = 'idle';
        renderButton();
      }
    }

    function cancelRecording() {
      clearTimeout(state.timer);
      try {
        if (state.recorder) state.recorder.stop();
      } catch (e) {}
      state.recorder = null;
      state.chunks = [];
      stopTracks();
      state.phase = 'idle';
      renderButton();
    }

    function onButtonClick() {
      if (state.phase === 'recording') {
        finishRecording();
        return;
      }
      if (state.phase === 'transcribing') return;
      startRecording();
    }

    function makeButton() {
      var button = document.createElement('button');
      button.type = 'button';
      button.id = BUTTON_ID;
      button.setAttribute(MARK, 'button');
      button.dataset.phase = 'idle';
      button.addEventListener('click', function (event) {
        event.preventDefault();
        event.stopPropagation();
        onButtonClick();
      });
      // 输入框工具条里的按钮默认会抢焦点、触发提交快捷键，这里全部拦住。
      button.addEventListener('mousedown', function (event) { event.preventDefault(); });
      button.addEventListener('keydown', function (event) { event.stopPropagation(); });
      return button;
    }

    /**
     * 往上找输入框外壳：发送键不一定和编辑器同容器（实测它在外层动作区里），
     * 所以从编辑器逐层向上，取第一个能查到发送键的祖先。
     */
    function composerScope(editor) {
      var node = editor;
      for (var depth = 0; depth < 8 && node && node !== document.body; depth++) {
        if (
          node.querySelector &&
          node.querySelector(
            '[data-testid="code-prompt-send"], [data-testid="chat-input-send"], [data-testid="epitaxy-cds-send"]',
          )
        ) {
          return node;
        }
        node = node.parentElement;
      }
      return null;
    }

    /** 挂载入口；每次调用都从当前 DOM 重新定位，不保留旧引用。 */
    function mount() {
      var editor = findEditor();
      if (!editor) return;
      var dock = findDock(editor);
      if (!dock) return;
      // 优先落在发送键左侧的动作区；拿不到动作区就退回编辑器所在的容器。
      var action = findActionRow(composerScope(editor) || dock) || { row: dock, before: null, spaced: true };
      var existing = document.getElementById(BUTTON_ID);
      if (existing && existing.parentNode === action.row) {
        state.editor = editor;
        state.dock = action.row;
        state.button = existing;
        // 官方容器自带 gap 时不要额外留白；没有 gap 的容器（Cowork 的发送键父层）自己补。
        existing.style.marginRight = action.spaced ? '' : '6px';
        return;
      }
      installStyle();
      // 录音/识别期间也可以重挂：宿主重建后按钮会消失，用户就没法停止录音了。
      // 状态留在 state 里，新按钮按当前 phase 重画即可。
      if (existing && existing.parentNode) existing.parentNode.removeChild(existing);
      var button = makeButton();
      button.style.marginRight = action.spaced ? '' : '6px';
      if (action.before && action.before.parentNode === action.row) {
        action.row.insertBefore(button, action.before);
      } else {
        action.row.appendChild(button);
      }
      state.editor = editor;
      state.dock = action.row;
      state.button = button;
      renderButton();
    }

    function scan() {
      try {
        mount();
      } catch (e) {}
    }

    // 宿主重建很频繁：观察整棵树的增删，节流到一帧后再扫描。
    var scheduled = false;
    var schedule = function () {
      if (scheduled) return;
      scheduled = true;
      requestAnimationFrame(function () {
        scheduled = false;
        scan();
      });
    };
    try {
      new MutationObserver(schedule).observe(document.documentElement, {
        childList: true,
        subtree: true,
      });
    } catch (e) {}
    document.addEventListener('DOMContentLoaded', schedule);
    setTimeout(schedule, 0);
    setTimeout(schedule, 1500);
    setTimeout(schedule, 5000);
    // 低频兜底：在 Code / Cowork 之间切换、开关侧栏时整块 composer 可能被重建，
    // 实测有 MutationObserver 没抓到的情形，靠这一轮保证入口一定会回来。
    setInterval(schedule, 2500);

    return 'ok';
  } catch (e) {
    return 'err:' + (e && e.message);
  }
})();
