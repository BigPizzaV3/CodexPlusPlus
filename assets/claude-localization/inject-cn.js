#!/usr/bin/env node
/**
 * Claude 中文化 —— 启动 + 注入器（本地化副本专用，不改 app.asar）
 *
 * 流程：
 *   1. 用 LaunchServices 启动副本，并带上 --inspect=127.0.0.1:<随机端口>
 *      （副本的 Electron Framework 里 EnableNodeCliInspectArguments 这一个 fuse 位被打开，
 *        所以打包版也允许 --inspect；该开关不在 App 自己的调试开关黑名单里）
 *   2. 等 Node inspector 起来，连上去
 *   3. 校验身份（process.execPath 必须是这个副本，防止注错进程）
 *   4. require 主进程 hook（~/.claude-cn/hook.js），由它给 claude.ai 页面注入 page-shim
 *   5. 关掉 inspector（inspector.close()），不留调试端口
 *
 * 用法：
 *   node inject-cn.js                 # 启动副本并汉化
 *   node inject-cn.js --attach 9411   # 只注入到已经开着 --inspect 的实例
 *   node inject-cn.js --no-launch     # 不启动，只等 inspector 出现
 *   node inject-cn.js --app <path> --port 9411 --timeout 60
 */

'use strict';

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');

const HOME = os.homedir();
const DEFAULT_APP = process.env.CODEX_PLUS_CLAUDE_COPY;
const DEFAULT_HOOK = path.join(__dirname, 'hook.js');
const HOOK_LOG = path.join(__dirname, 'hook.log');

function argOf(name, def) {
  const i = process.argv.indexOf(name);
  if (i < 0) return def;
  const v = process.argv[i + 1];
  return v === undefined || v.startsWith('--') ? true : v;
}

const APP = String(argOf('--app', DEFAULT_APP));
const HOOK = String(argOf('--hook', DEFAULT_HOOK));
const TIMEOUT = Number(argOf('--timeout', 45)) * 1000;
const NO_LAUNCH = !!argOf('--no-launch', false);
const ATTACH = argOf('--attach', false);
const PORT_ARG = argOf('--port', null);
const PROFILE = argOf('--user-data-dir', null);
const LOCALE = String(argOf('--locale', 'zh-Hans'));
if (!/^[a-z]{2,3}(?:-[A-Za-z0-9]{2,8})*$/.test(LOCALE)) throw new Error('无效的语言代码');

const EXEC = path.join(APP, 'Contents', 'MacOS', 'Claude');

function log(msg) {
  console.log(msg);
}

function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const p = srv.address().port;
      srv.close(() => resolve(p));
    });
  });
}

function runningPids() {
  const out = spawnSync('pgrep', ['-f', 'MacOS/Claude'], { encoding: 'utf8' }).stdout || '';
  const pids = out.split('\n').map((s) => s.trim()).filter(Boolean);
  const mine = [];
  for (const pid of pids) {
    const cmd = spawnSync('ps', ['-o', 'command=', '-p', pid], { encoding: 'utf8' }).stdout || '';
    if (cmd.includes('--type=')) continue; // 只关心主进程
    mine.push({ pid, cmd: cmd.trim() });
  }
  return mine;
}

async function listTargets(port) {
  const res = await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(2000) });
  return res.json();
}

async function waitForTarget(port, deadline) {
  while (Date.now() < deadline) {
    try {
      const list = await listTargets(port);
      const t = (list || []).find((x) => x.type === 'node');
      if (t && t.webSocketDebuggerUrl) return t;
    } catch (e) {}
    await new Promise((r) => setTimeout(r, 500));
  }
  return null;
}

function connect(url) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    const connectTimer = setTimeout(() => { ws.close(); reject(new Error('连接 inspector 超时')); }, 5000);
    let id = 0;
    const pending = new Map();
    ws.addEventListener('open', () => {
      clearTimeout(connectTimer);
      resolve({
        ws,
        send(method, params) {
          return new Promise((res, rej) => {
            const i = ++id;
            const timer = setTimeout(() => {
              if (pending.has(i)) {
                pending.delete(i);
                rej(new Error(method + ' timeout'));
              }
            }, 30000);
            pending.set(i, {
              res: (v) => { clearTimeout(timer); res(v); },
              rej: (e) => { clearTimeout(timer); rej(e); },
            });
            ws.send(JSON.stringify({ id: i, method, params: params || {} }));
          });
        },
        close: () => ws.close(),
      });
    });
    ws.addEventListener('error', (e) => reject(new Error('WebSocket 连接失败: ' + (e && e.message))));
    ws.addEventListener('message', (ev) => {
      let m;
      try {
        m = JSON.parse(ev.data);
      } catch (e) {
        return;
      }
      if (m.id && pending.has(m.id)) {
        const { res, rej } = pending.get(m.id);
        pending.delete(m.id);
        m.error ? rej(new Error(JSON.stringify(m.error))) : res(m.result);
      }
    });
  });
}

async function evaluateRetry(c, expression, tries) {
  let last;
  for (let i = 0; i < (tries || 4); i++) {
    try {
      return await evaluate(c, expression);
    } catch (e) {
      last = e;
      await new Promise((r) => setTimeout(r, 2000));
    }
  }
  throw last;
}

async function evaluate(c, expression) {
  const r = await c.send('Runtime.evaluate', { expression, returnByValue: true, includeCommandLineAPI: true, awaitPromise: false });
  if (r.exceptionDetails) {
    throw new Error('页面/进程异常: ' + (r.exceptionDetails.exception?.description || JSON.stringify(r.exceptionDetails)).slice(0, 300));
  }
  return r.result ? r.result.value : undefined;
}

(async function main() {
  if (!fs.existsSync(EXEC)) {
    console.error(`✗ 找不到副本：${EXEC}\n  先运行 build-cn-copy.py 生成。`);
    process.exit(1);
  }
  if (!fs.existsSync(HOOK)) {
    console.error(`✗ 找不到 hook：${HOOK}`);
    process.exit(1);
  }

  let port = PORT_ARG ? Number(PORT_ARG) : null;

  if (!ATTACH && !NO_LAUNCH) {
    if (!PROFILE) {
      // 指定了 --user-data-dir（沙箱测试）时跳过"另一个 Claude 在跑"的检查
      const running = runningPids();
      const mineRunning = running.find((r) => r.cmd.includes(APP));
      if (mineRunning) {
        console.error('✗ 本地化副本已经在运行了（pid ' + mineRunning.pid + '），但启动时没有带 --inspect，无法注入。');
        console.error('  请先 ⌘Q 退出「Claude CN」，再重新运行本启动器。');
        process.exit(2);
      }
      const other = running[0];
      if (other) {
        console.error('✗ 检测到另一个 Claude 正在运行（pid ' + other.pid + '）：');
        // 不把用户进程的命令行参数写进日志。
        console.error('  官方版和副本共用同一个用户数据目录，同时只能跑一个。请先退出它。');
        process.exit(2);
      }
    }
    port = await freePort();
    log(`→ 启动副本（inspector 端口 ${port}）`);
    const openArgs = ['-n', '-a', APP, '--args', `--inspect=127.0.0.1:${port}`];
    if (PROFILE) openArgs.push(`--user-data-dir=${PROFILE}`); // 必须用 = 形式，空格分隔 Chromium 不认
    const r = spawnSync('open', openArgs, { encoding: 'utf8' });
    if (r.status !== 0) {
      console.error('✗ open 失败：' + (r.stderr || r.stdout));
      process.exit(1);
    }
  } else if (!port) {
    port = 9411;
  }

  log(`→ 等待 inspector（127.0.0.1:${port}，最多 ${TIMEOUT / 1000}s）`);
  const target = await waitForTarget(port, Date.now() + TIMEOUT);
  if (!target) {
    console.error('✗ inspector 没出现。可能原因：副本没起来 / fuse 没打开 / 端口被占。');
    console.error('  排查：pgrep -fl "MacOS/Claude" ；codesign --verify --deep --strict "' + APP + '"');
    process.exit(1);
  }

  const c = await connect(target.webSocketDebuggerUrl);
  try {
    // 启动期主线程可能被占用，evaluate 会排队等待，这里给几次重试
    const execPath = await evaluateRetry(c, 'process.execPath');
    if (fs.realpathSync(execPath) !== fs.realpathSync(EXEC)) {
      console.error(`✗ 目标进程不是这个副本（${execPath}），放弃注入。`);
      process.exit(1);
    }
    const version = await evaluateRetry(c, "require('electron').app.getVersion()");
    log(`→ 已连上主进程（Claude ${version}, pid ${await evaluateRetry(c, 'process.pid')}）`);

    await evaluate(c, `globalThis.__claudeCnTargetLocale = ${JSON.stringify(LOCALE)}; require(${JSON.stringify(HOOK)})`);
    const okFlag = await evaluate(c, 'globalThis.__claudeCnHookInstalled === true');
    if (!okFlag) {
      console.error('✗ hook 没有装载成功，看看 ' + HOOK_LOG);
      process.exit(1);
    }
    log('→ 主进程 hook 已装载，页面注入就绪');

    try {
      // 这一句会把 inspector 和当前连接一起关掉，所以大概率等不到返回值，属正常
      await evaluate(c, "setImmediate(()=>require('node:inspector').close()); 'closing'");
    } catch (e) {}
    try { c.close(); } catch (e) {}
    await new Promise((r) => setTimeout(r, 1200));
    let stillOpen = true;
    try {
      await listTargets(port);
    } catch (e) {
      stillOpen = false;
    }
    log(stillOpen ? `（提示：${port} 端口可能仍开着）` : '→ inspector 已关闭（不留调试端口）');
  } finally {
    try { c.close(); } catch (e) {}
  }

  await new Promise((r) => setTimeout(r, 1500));
  if (false && fs.existsSync(HOOK_LOG)) {
    const lines = fs.readFileSync(HOOK_LOG, 'utf8').trim().split('\n').slice(-6);
    log('--- hook.log 末尾 ---');
    lines.forEach((l) => log('  ' + l));
  }
  log('启动注入已完成。');
})().catch((e) => {
  console.error('✗ ' + (e && e.message));
  process.exit(1);
});
