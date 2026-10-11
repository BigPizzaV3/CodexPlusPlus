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
const path = require('node:path');

const DIR = __dirname;
const LOG = path.join(DIR, 'hook.log');
const SHIM_FILE = path.join(DIR, 'page-shim.js');

function log(msg) {
  try {
    if (fs.existsSync(LOG) && fs.statSync(LOG).size > 2 * 1024 * 1024) fs.writeFileSync(LOG, '');
    fs.appendFileSync(LOG, new Date().toISOString() + ' ' + msg + '\n');
  } catch (e) {}
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
  try {
    SHIM = fs.readFileSync(SHIM_FILE, 'utf8');
    const locale = globalThis.__claudeCnTargetLocale || 'zh-Hans';
    SHIM = SHIM.replace("var LOCALE = 'zh-Hans';", 'var LOCALE = ' + JSON.stringify(locale) + ';');
  } catch (e) {
    log('cannot read ' + SHIM_FILE + ': ' + (e && e.message));
    return;
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
            if ((at === null || at === undefined) && !RELOADED.has(wc)) {
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

  const arm = (wc, why) => {
    try {
      if (!wc || wc.isDestroyed() || ARMED.has(wc)) return;
      if (!isDefaultSession(wc) || !hostOf(wc.getURL())) return;
      ARMED.add(wc);
      const dbg = wc.debugger;
      if (!dbg.isAttached()) dbg.attach('1.3');
      dbg.sendCommand('Page.enable').catch(() => {});
      dbg
        .sendCommand('Page.addScriptToEvaluateOnNewDocument', { source: SHIM })
        .then(() => {
          log('armed (' + why + ')');
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
    app.on('web-contents-created', (_e, wc) => watch(wc));
    electron.webContents.getAllWebContents().forEach((wc) => watch(wc));
    globalThis.__claudeCnHookInstalled = true;
    log('hook installed (Claude ' + app.getVersion() + ', pid ' + process.pid + ')');
  } catch (e) {
    log('wire failed: ' + (e && e.message));
  }
}
