#!/usr/bin/env node
// 检查本机安装的 Codex 能否打上原生 Fast 补丁：直接读 app.asar（只读），
// 按运行时同样的流程从 index.html 的入口发现模块、匹配四个门控，报告每个门控用的是严格结构还是核心结构。
// Codex 每次更新后跑一次，就能提前知道 Fast 是否还适配。
//
// 用法：node scripts/check-native-fast.cjs [app.asar 路径]
//   默认 macOS：/Applications/ChatGPT.app/Contents/Resources/app.asar
//   Windows 商店版可传 WindowsApps 下 OpenAI.Codex_*\app\resources\app.asar
"use strict";

const fs = require("node:fs");
const path = require("node:path");

function openAsar(file) {
  const fd = fs.openSync(file, "r");
  const head = Buffer.alloc(16);
  fs.readSync(fd, head, 0, 16, 0);
  const headerSize = head.readUInt32LE(4);
  const json = Buffer.alloc(head.readUInt32LE(12));
  fs.readSync(fd, json, 0, json.length, 16);
  const root = JSON.parse(json.toString("utf8"));
  const base = 8 + headerSize;
  const lookup = (relative) => {
    let node = root;
    for (const part of relative.split("/")) {
      node = node?.files?.[part];
      if (!node) return null;
    }
    return node.files ? null : node;
  };
  return {
    read(relative) {
      const entry = lookup(relative);
      if (!entry) return null;
      if (entry.unpacked) return fs.readFileSync(`${file}.unpacked/${relative}`, "utf8");
      const buffer = Buffer.alloc(entry.size);
      fs.readSync(fd, buffer, 0, entry.size, base + Number(entry.offset));
      return buffer.toString("utf8");
    },
  };
}

async function main() {
  const asarPath = process.argv[2] || "/Applications/ChatGPT.app/Contents/Resources/app.asar";
  if (!fs.existsSync(asarPath)) {
    console.error(`找不到 ${asarPath}，请传入 Codex 的 app.asar 路径。`);
    process.exit(2);
  }
  const asar = openAsar(asarPath);
  const html = asar.read("webview/index.html") ?? "";
  const entries = [...html.matchAll(/<script[^>]+src="\.?\/?(assets\/index-[\w-]+\.js)"/g)].map((match) => `app://-/${match[1]}`);
  if (!entries.length) {
    console.error("webview/index.html 里没有找到 assets/index-*.js 入口。");
    process.exit(1);
  }

  const api = require(path.join(__dirname, "../assets/inject/native-fast-unlock.js"));
  api.runtime.update({ enhancementsEnabled: true, codexAppNativeFastUnlock: true });
  const fetched = [];
  const fetchText = async (url) => {
    const relative = `webview${new URL(url).pathname}`;
    fetched.push(relative);
    return asar.read(relative);
  };
  const result = await api.prepareDocumentModules({
    location: new URL("app://-/index.html"),
    document: { scripts: entries.map((src) => ({ src })) },
    fetchText,
  });
  console.log(`入口：${entries.join(", ")}`);
  console.log(`读取模块：${fetched.length} 个`);
  console.log(`结果：${result.ok ? "可以打补丁" : "无法打补丁"}（${result.diagnostics.reason}）`);
  console.log(`改写器：${JSON.stringify(result.diagnostics.rewriters)}`);
  console.log(`发现方式：${JSON.stringify(result.diagnostics.discovery ?? {})}`);
  const fast = result.modules?.find((item) => item.patchedSource.includes('__codexPlusNativeFastCompiled="v1"'));
  if (fast) {
    const source = asar.read(`webview${new URL(fast.url).pathname}`);
    const plan = api.plan(source, fast.url);
    console.log(`目标块：${fast.url}`);
    console.log(`门控：${JSON.stringify(plan.diagnostics.strategies)}`);
  }
  process.exit(result.ok ? 0 : 1);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
