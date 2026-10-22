/**
 * Claude 中文化 —— 主进程 hook（本地化副本专用）
 *
 * 由 inject-cn.js 通过 Node inspector 在副本主进程里 require 进来：
 *   require('/Users/mac/.claude-cn/hook.js')
 *
 * 作用：给默认 session 上导航到 claude.ai / claude.com 的 webContents 挂调试器，
 *      用 Page.addScriptToEvaluateOnNewDocument 注入 page-shim.js（在任何页面脚本之前运行），
 *      让 SPA 把界面语言当成 zh-Hans，从而下载官方中文词表。
 *
 * 内置浏览器面板、预览窗口等 App 自己要用调试器的页面一概不碰。
 * 日志：~/.claude-cn/hook.log
 */

'use strict';

const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');

const DIR = __dirname;
const LOG = path.join(DIR, 'hook.log');
const SHIM_FILE = path.join(DIR, 'page-shim.js');
const DICTATION_FILE = path.join(DIR, 'page-dictation.js');

/**
 * Codex++ launcher 写的听写凭据：helper 端口 + 能力令牌。
 * 文件名必须与 crates/codex-plus-core/src/launcher.rs 的 DICTATION_HELPER_RECEIPT 一致。
 *
 * launcher 把它写在状态目录（default_app_state_dir），而本脚本装在
 * `<状态目录>/claude-localization`（claude_localization.rs 的 runtime_dir），所以先读上一级。
 * 状态目录可被 CODEX_PLUS_STATE_DIR 改掉（本地构建、多开实例），写死家目录会读不到；
 * 家目录那份只作手工部署（hook 不在 runtime_dir 下）时的兜底。
 */
const RECEIPT_NAME = 'dictation-helper.json';
const RECEIPTS = [
  path.join(path.dirname(DIR), RECEIPT_NAME),
  path.join(os.homedir(), '.codex-session-delete', RECEIPT_NAME),
].filter((file, index, all) => all.indexOf(file) === index);
/** 页面调用主进程的 binding 名（page-dictation.js 按同名字面量调用）。 */
const BINDING = '__claudeCnDictationCall';

function log(msg) {
  try {
    if (fs.existsSync(LOG) && fs.statSync(LOG).size > 2 * 1024 * 1024) fs.writeFileSync(LOG, '');
    fs.appendFileSync(LOG, new Date().toISOString() + ' ' + msg + '\n');
  } catch (e) {}
}

/** 读取 launcher 落下的听写凭据；helper 没运行时返回 null。 */
function readReceipt() {
  for (const file of RECEIPTS) {
    try {
      const value = JSON.parse(fs.readFileSync(file, 'utf8'));
      if (
        value &&
        Number.isInteger(value.port) &&
        value.port > 0 &&
        value.port < 65536 &&
        typeof value.token === 'string' &&
        value.token
      ) {
        return value;
      }
    } catch (e) {}
  }
  return null;
}

/** 文件名与 MIME 会拼进 multipart 头，按白名单收敛，别让页面塞进换行。 */
function safeFilename(name) {
  const text = String(name || '');
  return /^[A-Za-z0-9._-]{1,64}$/.test(text) ? text : 'dictation.wav';
}

function safeMime(type) {
  const text = String(type || '').trim().toLowerCase();
  return /^[a-z]+\/[a-z0-9.+-]{1,32}$/.test(text) ? text : 'audio/wav';
}

/**
 * 把页面传来的音频交给 Codex++ 的 helper 转写（本地 SenseVoice 或用户配置的 ASR）。
 *
 * 走 node:http 而不是全局 fetch：主进程可能挂着系统代理，loopback 请求不该被代理接管。
 */
function transcribe(payload) {
  return new Promise((resolve, reject) => {
    const receipt = readReceipt();
    if (!receipt) {
      reject(new Error('Codex++ 语音服务未运行，请先启动 Codex++'));
      return;
    }
    let audio;
    try {
      audio = Buffer.from(String(payload.audioBase64 || ''), 'base64');
    } catch (e) {
      reject(new Error('录音数据无法解码'));
      return;
    }
    if (!audio.length) {
      reject(new Error('录音为空'));
      return;
    }
    const boundary = 'codex-plus-claude-' + Date.now().toString(16);
    const head = Buffer.from(
      `--${boundary}\r\nContent-Disposition: form-data; name="file"; filename="${safeFilename(payload.filename)}"\r\n` +
        `Content-Type: ${safeMime(payload.mimeType)}\r\n\r\n`,
    );
    const tail = Buffer.from(`\r\n--${boundary}--\r\n`);
    const body = Buffer.concat([head, audio, tail]);
    const request = http.request(
      {
        host: '127.0.0.1',
        port: receipt.port,
        method: 'POST',
        path: '/dictation/transcribe',
        headers: {
          'Content-Type': `multipart/form-data; boundary=${boundary}`,
          'Content-Length': body.length,
          'X-Codex-Plus-Dictation-Token': receipt.token,
        },
      },
      (response) => {
        const chunks = [];
        response.on('data', (chunk) => chunks.push(chunk));
        response.on('end', () => {
          const text = Buffer.concat(chunks).toString('utf8');
          let parsed = null;
          try {
            parsed = JSON.parse(text);
          } catch (e) {}
          if (response.statusCode !== 200) {
            reject(new Error((parsed && parsed.message) || `语音服务返回 ${response.statusCode}`));
            return;
          }
          if (!parsed || typeof parsed.text !== 'string') {
            reject(new Error('语音服务没有返回文本'));
            return;
          }
          resolve(parsed.text);
        });
      },
    );
    request.on('error', (error) => reject(new Error('无法连接语音服务：' + (error && error.message))));
    request.setTimeout(180000, () => request.destroy(new Error('语音转写超时')));
    request.end(body);
  });
}

/** 把结果投回页面；页面按 id 匹配自己那一次请求。 */
function deliver(wc, id, message) {
  try {
    if (!wc || wc.isDestroyed()) {
      log('deliver skipped ' + id + ': webContents gone');
      return;
    }
    const expression =
      'globalThis.__claudeCnDictationDeliver && globalThis.__claudeCnDictationDeliver(' +
      JSON.stringify(id) +
      ',' +
      JSON.stringify(message) +
      ')';
    // 用 electron 的 executeJavaScript 而不是 wc.debugger.sendCommand：后者会往
    // debugger 会话里插命令，实测在 bindingCalled 的回调里发出后页面收不到。
    Promise.resolve(wc.executeJavaScript(expression)).catch((error) =>
      log('deliver failed ' + id + ': ' + ((error && error.message) || error)),
    );
  } catch (e) {
    log('deliver error ' + id + ': ' + (e && e.message));
  }
}

/**
 * 放行麦克风。
 *
 * Claude 自己（3p 部署）会拒绝媒体权限，这里显式放开；其余权限维持 Electron 的默认
 * 放行语义，避免替换 handler 后把 Claude 的通知、剪贴板等能力一起关掉。
 */
function allowMicrophone(target) {
  try {
    // 麦克风必须显式放行（3p 部署下 Claude 自己会拒绝媒体权限）。其余权限一并放行，
    // 保持 Electron 未设置 handler 时的默认语义，避免替换 handler 后把通知、剪贴板关掉。
    target.setPermissionRequestHandler((_wc, _permission, callback) => callback(true));
    target.setPermissionCheckHandler(() => true);
  } catch (e) {
    log('microphone permission handler failed: ' + (e && e.message));
  }
}

/**
 * Computer Use 授权自检（仅 macOS、仅勾选了 Computer Use 制作的副本）。
 *
 * 由副本自己查自己的授权：isTrustedAccessibilityClient / getMediaAccessStatus
 * 查的是调用进程本身，不需要读 TCC.db，也就不需要「完全磁盘访问」。
 * 缺授权时 Computer Use 用不了，弹窗引导用户去系统设置；每次启动最多提醒一次。
 */
const CU_MARKER = '.cu-patched';

function computerUsePermissions(electron) {
  const { systemPreferences } = electron;
  let accessibility = false;
  let screen = false;
  try { accessibility = systemPreferences.isTrustedAccessibilityClient(false) === true; } catch (e) {}
  try { screen = systemPreferences.getMediaAccessStatus('screen') === 'granted'; } catch (e) {}
  return { accessibility, screen };
}

/** 让系统把副本登记进隐私列表并弹出官方提示，再打开对应设置页。 */
function requestComputerUsePermissions(electron, missing) {
  const { systemPreferences, desktopCapturer, shell } = electron;
  if (missing.accessibility) {
    // 传 true 会让 macOS 弹「去系统设置」提示，同时把副本加进辅助功能列表。
    try { systemPreferences.isTrustedAccessibilityClient(true); } catch (e) {}
  }
  if (missing.screen) {
    // 主动取一次屏幕源：系统会弹屏幕录制提示，并把副本加进列表。
    try {
      Promise.resolve(desktopCapturer.getSources({ types: ['screen'], thumbnailSize: { width: 1, height: 1 } }))
        .catch(() => {});
    } catch (e) {}
  }
  const anchor = missing.accessibility ? 'Privacy_Accessibility' : 'Privacy_ScreenCapture';
  shell.openExternal('x-apple.systempreferences:com.apple.preference.security?' + anchor).catch(() => {});
}

function checkComputerUsePermissions(electron) {
  if (process.platform !== 'darwin') return;
  if (!fs.existsSync(path.join(process.resourcesPath || '', CU_MARKER))) return;
  const { app, dialog, BrowserWindow } = electron;
  const granted = computerUsePermissions(electron);
  log('computer use permissions: accessibility=' + granted.accessibility + ' screen=' + granted.screen);
  if (granted.accessibility && granted.screen) return;
  const missing = { accessibility: !granted.accessibility, screen: !granted.screen };
  const names = [];
  if (missing.accessibility) names.push('辅助功能');
  if (missing.screen) names.push('屏幕录制');
  const parent = BrowserWindow.getFocusedWindow() || BrowserWindow.getAllWindows()[0];
  const options = {
    type: 'warning',
    buttons: ['去授权', '稍后'],
    defaultId: 0,
    cancelId: 1,
    message: 'Computer Use 还缺少系统授权，授权前无法使用',
    detail:
      '缺少：' + names.join('、') + '。\n\n' +
      '点「去授权」后在系统设置里打开「' + app.getName() + '」的开关。' +
      // 副本与官方版同一个 bundle id：列表里那一行可能是给官方版授的，副本照样过不了。
      '如果开关已经是打开的仍然提示，先用「−」删掉这一行，再重新打开本应用授权。' +
      (missing.screen ? '\n\n屏幕录制授权后需要退出并重新打开本应用才生效。' : ''),
  };
  const shown = parent ? dialog.showMessageBox(parent, options) : dialog.showMessageBox(options);
  Promise.resolve(shown)
    .then((result) => {
      if (result && result.response === 0) requestComputerUsePermissions(electron, missing);
    })
    .catch((e) => log('permission dialog failed: ' + (e && e.message)));
}

if (globalThis.__claudeCnHookInstalled) {
  log('already installed, skip');
} else {
  install();
}

function install() {
  let electron;
  try {
    electron = require('electron');
  } catch (e) {
    log('require("electron") failed: ' + (e && e.message));
    return;
  }
  const { app, session } = electron;
  if (!app) {
    log('no app object');
    return;
  }

  let SHIM = '';
  let DICTATION = '';
  try {
    SHIM = fs.readFileSync(SHIM_FILE, 'utf8');
    const locale = globalThis.__claudeCnTargetLocale || 'zh-Hans';
    SHIM = SHIM.replace("var LOCALE = 'zh-Hans';", 'var LOCALE = ' + JSON.stringify(locale) + ';');
  } catch (e) {
    log('cannot read ' + SHIM_FILE + ': ' + (e && e.message));
    return;
  }
  try {
    DICTATION = fs.readFileSync(DICTATION_FILE, 'utf8');
  } catch (e) {
    // 语音脚本缺失不该影响汉化本身，只记一条日志。
    log('cannot read ' + DICTATION_FILE + ': ' + (e && e.message));
  }
  // 管理器的「增强功能」开关：语音输入关掉时不注入入口，
  // 界面汉化关掉时不注入中文界面那一段（副本按原语言跑）。
  if (globalThis.__claudeCnVoiceInput === false) {
    DICTATION = '';
    log('voice input disabled by settings');
  }
  if (globalThis.__claudeCnLocalization === false) {
    SHIM = '';
    log('localization disabled by settings');
  }

  const HOST = /(^|\.)claude\.(ai|com)$/;
  const hostOf = (url) => {
    try {
      const u = new URL(url);
      if (HOST.test(u.hostname)) return true;
      // 本地主界面：app://localhost/...
      return u.protocol === 'app:' && u.hostname === 'localhost';
    } catch (e) {
      return false;
    }
  };

  const ARMED = new WeakSet();
  const RELOADED = new WeakSet();

  const isDefaultSession = (wc) => {
    try {
      return wc.session === session.defaultSession;
    } catch (e) {
      return false;
    }
  };

  const verify = (wc, tag) => {
    setTimeout(() => {
      try {
        if (wc.isDestroyed() || !wc.debugger.isAttached()) return;
        wc.debugger
          .sendCommand('Runtime.evaluate', {
            expression:
              'JSON.stringify({at:window.__claude_zh_locale_at__||null,' +
              'fixed:window.__claude_zh_locale_fixed__||0,' +
              'spa:localStorage.getItem("spa:locale"),' +
              'pref:localStorage.getItem("claude-zh:locale"),' +
              'lang:document.documentElement.lang})',
            returnByValue: true,
          })
          .then((r) => {
            const v = String((r && r.result && r.result.value) || '');
            log(tag + ' verify ' + v);
            let at = null;
            try {
              at = JSON.parse(v).at;
            } catch (e) {}
            if (SHIM && (at === null || at === undefined) && !RELOADED.has(wc)) {
              RELOADED.add(wc);
              log(tag + ' page script missed the document, reloading once');
              try { wc.reload(); } catch (e) {}
            }
          })
          .catch((e) => log(tag + ' verify failed: ' + (e && e.message)));
      } catch (e) {
        log(tag + ' verify error: ' + (e && e.message));
      }
    }, 3000);
  };

  /** 页面请求一次转写：解析 payload、转写、把结果投回去。 */
  const handleDictationCall = (wc, raw) => {
    let message = null;
    try {
      message = JSON.parse(String(raw || ''));
    } catch (e) {}
    const id = message && typeof message.id === 'string' ? message.id : '';
    if (!id) return;
    if (message.action === 'status') {
      deliver(wc, id, { ok: true, available: !!readReceipt() });
      return;
    }
    transcribe(message)
      .then((text) => {
        log('dictation ok ' + id + ' chars=' + text.length);
        deliver(wc, id, { ok: true, text });
      })
      .catch((error) => {
        log('dictation failed ' + id + ': ' + ((error && error.message) || error));
        deliver(wc, id, { ok: false, message: String((error && error.message) || error) });
      });
  };

  const arm = (wc, why) => {
    try {
      if (!wc || wc.isDestroyed() || ARMED.has(wc)) return;
      if (!isDefaultSession(wc) || !hostOf(wc.getURL())) return;
      ARMED.add(wc);
      const dbg = wc.debugger;
      if (!dbg.isAttached()) dbg.attach('1.3');
      // Runtime 域必须先打开，bindingCalled 事件才会派发。
      dbg.sendCommand('Runtime.enable').catch(() => {});
      // 语音输入关掉时连通道都不注册：页面里不会出现那个可调用的全局函数。
      if (DICTATION) {
        dbg.sendCommand('Runtime.addBinding', { name: BINDING }).catch((e) => {
          log('dictation binding failed: ' + (e && e.message));
        });
        dbg.on('message', (_event, method, params) => {
          if (method !== 'Runtime.bindingCalled' || !params || params.name !== BINDING) return;
          try {
            handleDictationCall(wc, params.payload);
          } catch (e) {
            log('dictation dispatch error: ' + (e && e.message));
          }
        });
      }
      dbg.sendCommand('Page.enable').catch(() => {});
      dbg
        .sendCommand('Page.addScriptToEvaluateOnNewDocument', {
          source: SHIM + '\n;\n' + (DICTATION || ''),
        })
        .then(() => {
          log('armed (' + why + ')');
          // 已经加载完的页面不会再走 addScriptToEvaluateOnNewDocument，这里补一次注入；
          // 脚本自身有幂等守卫，重复注入没有副作用。
          if (DICTATION) {
            dbg.sendCommand('Runtime.evaluate', { expression: DICTATION }).catch(() => {});
          }
          verify(wc, '(armed)');
        })
        .catch((e) => {
          ARMED.delete(wc);
          log('arm failed: ' + (e && e.message));
        });
    } catch (e) {
      log('arm error: ' + (e && e.message));
    }
  };

  const watch = (wc) => {
    try {
      if (!wc || wc.isDestroyed() || !isDefaultSession(wc)) return;
      arm(wc, 'created');
      wc.on('did-start-navigation', (_e, url, _ip, main) => {
        if (main && hostOf(url)) arm(wc, 'navigation');
      });
      wc.on('did-finish-load', () => {
        if (hostOf(wc.getURL())) verify(wc, '(load)');
      });
    } catch (e) {
      log('watch error: ' + (e && e.message));
    }
  };

  try {
    // 注入器连得够快时 hook 会在 app ready 之前装载，这时读 session.defaultSession 会抛
    // 「Session can only be received when app is ready」，整条 hook 跟着失败。
    // 监听器先挂上（webContents 都在 ready 之后才创建），麦克风放行等 ready 再做。
    app.on('web-contents-created', (_e, wc) => watch(wc));
    electron.webContents.getAllWebContents().forEach((wc) => watch(wc));
    globalThis.__claudeCnHookInstalled = true;
    log('hook installed (Claude ' + app.getVersion() + ', pid ' + process.pid + ')');
    app.whenReady()
      .then(() => allowMicrophone(session.defaultSession))
      .catch((e) => log('microphone permission handler failed: ' + (e && e.message)));
    // 等主窗口出来再查，弹窗才能挂在窗口上而不是孤零零浮在桌面。
    app.whenReady()
      .then(() => new Promise((resolve) => setTimeout(resolve, 2500)))
      .then(() => checkComputerUsePermissions(electron))
      .catch((e) => log('permission check failed: ' + (e && e.message)));
  } catch (e) {
    log('wire failed: ' + (e && e.message));
  }
}
