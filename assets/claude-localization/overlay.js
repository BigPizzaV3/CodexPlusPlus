'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { fileURLToPath, pathToFileURL } = require('node:url');

// 保留官方 app:// 协议、动态配置及 CSP，只重定向清单内的本地文件读取。
function installOverlay({ files, net }, root, ionRoot) {
  const manifest = JSON.parse(fs.readFileSync(path.join(root, 'manifest.json'), 'utf8').replace(/^\uFEFF/, ''));
  const entries = new Map();
  for (const entry of manifest.files) {
    const relative = entry.path;
    if (typeof relative !== 'string' || relative.includes('\\') || relative.split('/').includes('..') || path.isAbsolute(relative)) {
      throw new Error('无效的增强资源路径');
    }
    const file = path.resolve(root, relative);
    if (!file.startsWith(path.resolve(root) + path.sep)) throw new Error('增强资源越界');
    if (!fs.statSync(file).isFile()) throw new Error('增强资源不是文件');
    entries.set(path.resolve(ionRoot, relative), file);
  }
  const mapped = (input) => {
    try {
      const file = input instanceof URL ? fileURLToPath(input) : typeof input === 'string' ? input : null;
      return file === null ? undefined : entries.get(path.resolve(file));
    } catch { return undefined; }
  };
  for (const name of ['readFile', 'stat']) {
    const original = files[name];
    files[name] = function (input, ...args) { return original.call(this, mapped(input) || input, ...args); };
  }
  const fetch = net.fetch;
  net.fetch = function (input, options) {
    let target;
    try {
      const url = input instanceof URL ? input : typeof input === 'string' ? new URL(input) : null;
      if (url?.protocol === 'file:') target = mapped(url);
    } catch {}
    return fetch.call(this, target ? pathToFileURL(target).href : input, options);
  };
}

module.exports = { installOverlay };
