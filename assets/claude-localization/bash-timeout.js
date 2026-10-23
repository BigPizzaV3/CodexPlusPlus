'use strict';

const GLOBAL = '__codexPlusBashTimeout';
const RESERVE_MS = 60000;
const MAX_MS = 2147483647 - RESERVE_MS;

function overrides(settings) {
  const milliseconds = (enabled, seconds) => enabled && Number.isSafeInteger(seconds) && seconds > 0
    ? Math.min(seconds * 1000, MAX_MS) : undefined;
  const defaultMs = milliseconds(settings.defaultEnabled, settings.defaultSeconds);
  const maxMs = milliseconds(settings.maxEnabled, settings.maxSeconds);
  return {
    limits(nativeDefault, nativeMax) {
      const value = defaultMs ?? nativeDefault;
      return { defaultMs: value, maxMs: Math.max(value, maxMs ?? nativeMax) };
    },
    budget(nativeBudget, nativeDefault, nativeMax) {
      const target = maxMs === undefined ? defaultMs : this.limits(nativeDefault, nativeMax).maxMs;
      return target === undefined ? nativeBudget : Math.max(nativeBudget, target + RESERVE_MS);
    },
  };
}

// 按工具语义定位，不依赖每次发布都会变化的 chunk 文件名；所有锚点验证完才修改。
function patchWorkspaceSource(source) {
  if (!source.includes("Run a shell command in the session's isolated Linux workspace.")) {
    throw new Error('未找到 Cowork 工作区 Bash 工具');
  }
  const id = '[A-Za-z_$][\\w$]*';
  const schema = new RegExp('timeout_ms:(?<root>' + id + ')\\.(?<number>' + id + ')\\(\\)\\.int\\(\\)\\.positive\\(\\)\\.max\\((?<max>' + id + ')\\)\\.optional\\(\\)\\.describe\\(`Timeout in milliseconds \\(default \\$\\{(?<def>' + id + ')\\}, max \\$\\{\\k<max>\\}\\)\\. Values above the host\'s MCP tool-call cap are capped to it\\.`\\)', 'g');
  const matches = [...source.matchAll(schema)];
  if (matches.length !== 1) throw new Error('不支持的 Cowork Bash 参数校验结构');
  const { root, number, max, def } = matches[0].groups;
  const escape = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const budget = new RegExp('Math\\.min\\((?<args>' + id + ')\\.timeout_ms\\?\\?' + escape(def) + ',Math\\.max\\(1e3,(?<get>' + escape(root) + '\\.' + id + '\\(\\))-(?<elapsed>\\(Date\\.now\\(\\)-' + id + '\\))-(?<reserve>' + id + ')\\)\\)', 'g');
  const budgets = [...source.matchAll(budget)];
  if (budgets.length !== 1) throw new Error('不支持的 Cowork Bash 执行时限结构');
  const { args, get, elapsed, reserve } = budgets[0].groups;
  const spawn = `MCP_TOOL_TIMEOUT:String(${get})`;
  if (source.split(spawn).length !== 2) throw new Error('不支持的 Cowork MCP 等待时限结构');
  const helper = `globalThis.${GLOBAL}`;
  const limits = `${helper}.limits(${def},${max})`;
  const outer = `${helper}.budget(${get},${def},${max})`;
  return source
    .replace(matches[0][0], 'timeout_ms:' + root + '.' + number + '().int().positive().max(' + limits + '.maxMs).optional().describe(`Timeout in milliseconds (default ${' + limits + '.defaultMs}, max ${' + limits + '.maxMs}). Values above the host\'s MCP tool-call cap are capped to it.`)')
    .replace(budgets[0][0], `Math.min(${args}.timeout_ms??${limits}.defaultMs,Math.max(1e3,${outer}-${elapsed}-${reserve}))`)
    // 只提高 Cowork 子进程的外层 watchdog；其他工具自身的执行时限保持原样。
    .replace(spawn, `MCP_TOOL_TIMEOUT:String(${helper}.budget(${get},120000,600000))`);
}

function patchOnLoad(Module, filename, source, report) {
  const original = Module._extensions['.js'];
  const handler = function (module, file) {
    if (file !== filename) return original.call(this, module, file);
    if (Module._extensions['.js'] === handler) Module._extensions['.js'] = original;
    module._compile(source, file);
    report();
  };
  Module._extensions['.js'] = handler;
}

async function installTimeoutOverrides(settings, resourcesPath = process.resourcesPath) {
  if (!settings?.enabled || (!settings.defaultEnabled && !settings.maxEnabled)) return { enabled: false };
  if (globalThis[GLOBAL]) throw new Error('Bash 时间限制已经安装，请重启 Claude 后应用新设置');
  const fs = require('node:fs');
  const path = require('node:path');
  const root = path.join(resourcesPath, 'app.asar', '.vite', 'build');
  const candidates = fs.readdirSync(root).filter((name) => name.endsWith('.js')).map((name) => {
    const file = path.join(root, name);
    const source = fs.readFileSync(file, 'utf8');
    return source.includes("Run a shell command in the session's isolated Linux workspace.") ? { file, source } : null;
  }).filter(Boolean);
  if (candidates.length !== 1) throw new Error('当前 Claude 版本不支持 Cowork Bash 时间限制，请关闭此增强后重试');
  const candidate = candidates[0];
  const patched = patchWorkspaceSource(candidate.source);
  const Module = require('node:module');
  const helper = overrides(settings);
  const result = { enabled: true, ...helper.limits(120000, 600000), loaded: false };
  // 启动器在主程序执行前安装；不热更新或重建已加载的会话管理器。
  if (!require.cache[candidate.file]) {
    globalThis[GLOBAL] = helper;
    patchOnLoad(Module, candidate.file, patched, () => { result.loaded = true; });
    return result;
  }
  throw new Error('Cowork 模块已加载，请完全退出 Claude 后从 Codex++ 重新启动');
}

module.exports = { overrides, patchWorkspaceSource, patchOnLoad, installTimeoutOverrides };
