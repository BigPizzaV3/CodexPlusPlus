/**
 * Claude Desktop 主界面汉化 —— 页面侧脚本（在 claude.ai 页面里、任何页面脚本之前运行）
 *
 * 由 inject.js 通过 CDP 的 Page.addScriptToEvaluateOnNewDocument 注入。
 * 作用：让 claude.ai 的前端（SPA）把界面语言当成 zh-Hans，从而去下载官方中文词表
 *      https://claude.ai/i18n/zh-Hans.json（约 3 万条，官方翻译）。
 *
 * 依据（社区已反向验证的事实，本项目按同样事实自行实现）：
 *   1. SPA 登录后按启动数据 /edge-api/bootstrap…（或 /api/bootstrap）响应顶层的 locale
 *      字段决定界面语言，并写进 localStorage["spa:locale"]；
 *   2. 语言选择器列出哪些扩展语言，由 GrowthBook 特性 witty_scone_main 的 released 列表决定；
 *   3. 账号语言接口 PUT /api/account_profile 不接受 zh-Hans（400），所以选中文时在本地直接应答成功。
 *
 * 用户显式选过别的语言（localStorage["claude-zh:locale"] 有值且不是 zh-Hans）就完全放手。
 */

(function () {
  'use strict';
  var LOCALE = 'zh-Hans';
  var PREF_KEY = 'claude-zh:locale';
  var GATE = 'witty_scone_main';

  try {
    var isClaudeHost = /(^|\.)claude\.(ai|com)$/.test(location.hostname);
    // 本地 UI：主界面是 Resources/ion-dist 里的本地前端，用 app://localhost 提供
    var isLocalUi = location.protocol === 'app:' && location.hostname === 'localhost';
    if (!isClaudeHost && !isLocalUi) return 'skip:host';
    if (window.__claude_zh_locale__) return 'skip:dup';
    window.__claude_zh_locale__ = 1;
    // 记下运行时文档状态：'loading' = 我们赶在了页面脚本之前（主进程据此决定要不要补一次重载）
    window.__claude_zh_locale_at__ = document.readyState;

    var ls = function (k, v) {
      try {
        if (v === undefined) return localStorage.getItem(k);
        localStorage.setItem(k, v);
      } catch (e) {}
      return null;
    };

    // 管理器选择语言时同步本地偏好，语言选择器后续仍可切换。
    ls(PREF_KEY, LOCALE);
    // 没选过语言、或选的就是中文 → 强制中文
    var wantZh = function () {
      var p = ls(PREF_KEY);
      return !p || p === LOCALE;
    };

    if (wantZh()) {
      ls('spa:locale', LOCALE);
      // 标记为「用户明确选择」，否则本地 UI 会跟随账号语言把它改回去
      ls('spa:locale-source', 'user_choice');
    }

    // 本地 UI 也必须改写启动数据：它里面的 locale 是账号语言，会把界面语言拉回去
    var origFetch = window.fetch;
    if (typeof origFetch !== 'function') return 'skip:nofetch';

    var pathOf = function (input) {
      try {
        var u = typeof input === 'string' ? input : (input && input.url) || String(input);
        return new URL(u, location.href).pathname;
      } catch (e) {
        return '';
      }
    };

    var djb2 = function (s) {
      var h = 0;
      for (var i = 0; i < s.length; i++) {
        h = (h << 5) - h + s.charCodeAt(i);
        h = h & h;
      }
      return String(h >>> 0);
    };

    // 递归找 released 数组，把 zh-Hans 补进去
    var addReleased = function (obj) {
      var n = 0;
      (function walk(x) {
        if (!x || typeof x !== 'object') return;
        if (Object.prototype.toString.call(x.released) === '[object Array]' && x.released.indexOf(LOCALE) < 0) {
          x.released.push(LOCALE);
          n++;
        }
        for (var k in x) {
          if (Object.prototype.hasOwnProperty.call(x, k) && x[k] && typeof x[k] === 'object') walk(x[k]);
        }
      })(obj);
      return n;
    };

    var jsonResponse = function (body, orig) {
      var headers = new Headers();
      if (orig) {
        orig.headers.forEach(function (v, k) {
          if (!/^(content-length|content-encoding)$/i.test(k)) headers.append(k, v);
        });
      } else {
        headers.set('content-type', 'application/json');
      }
      var res = new Response(JSON.stringify(body), {
        status: orig ? orig.status : 200,
        statusText: orig ? orig.statusText : 'OK',
        headers: headers,
      });
      if (orig) {
        try { Object.defineProperty(res, 'url', { value: orig.url }); } catch (e) {}
        try { Object.defineProperty(res, 'redirected', { value: orig.redirected }); } catch (e) {}
      }
      return res;
    };

    var rewrite = function (res) {
      if (!res || !res.ok) return res;
      var ct = (res.headers && res.headers.get('content-type')) || '';
      if (ct.indexOf('json') < 0) return res;
      return res.clone().text().then(function (text) {
        var j = JSON.parse(text);
        var hits = 0;
        if (!j || typeof j !== 'object' || Object.prototype.toString.call(j) === '[object Array]') return res;
        if (wantZh() && typeof j.locale === 'string' && j.locale !== LOCALE) {
          j.locale = LOCALE;
          hits++;
        }
        // GrowthBook 特性载荷：让「中文（简体）」常驻语言选择器
        var gb = j.growthbook;
        var features = gb && gb.features;
        if (features && typeof features === 'object') {
          var key = gb.hashing_algorithm === 'djb2' ? djb2(GATE) : GATE;
          if (features[key]) hits += addReleased(features[key]);
          else { features[key] = { defaultValue: { released: [LOCALE] } }; hits++; }
        }
        if (!hits) return res;
        window.__claude_zh_locale_fixed__ = (window.__claude_zh_locale_fixed__ || 0) + 1;
        return jsonResponse(j, res);
      }).catch(function () { return res; });
    };

    var wrapped = function (input, init) {
      var p = pathOf(input);
      try {
        var method = String((init && init.method) || (input && input.method) || 'GET').toUpperCase();
        // 语言选择器：选中文时账号接口会 400，这里就地应答成功
        if (p === '/api/account_profile' && method === 'PUT' && init && typeof init.body === 'string') {
          var body = JSON.parse(init.body);
          if (body && typeof body.locale === 'string') {
            ls(PREF_KEY, body.locale);
            if (body.locale === LOCALE && LOCALE === 'zh-Hans') {
              ls('spa:locale', LOCALE);
              delete body.locale;
              if (!Object.keys(body).length) return Promise.resolve(jsonResponse({ locale: LOCALE }));
              return origFetch.call(window, input, Object.assign({}, init, { body: JSON.stringify(body) }));
            }
          }
        }
      } catch (e) {}
      var p2 = origFetch.apply(window, arguments);
      try {
        if (/^\/(?:edge-)?api\/bootstrap(\/|$)/.test(p)) return p2.then(rewrite);
      } catch (e) {}
      return p2;
    };

    try { Object.defineProperty(wrapped, 'name', { value: 'fetch' }); } catch (e) {}
    window.fetch = wrapped;
    return 'ok';
  } catch (e) {
    return 'err:' + (e && e.message);
  }
})();
