// 从已安装 Codex 的 app.asar 里读出 IPC 方法版本表。
// 用法：node ipc-versions.mjs <app.asar>
//
// 版本表是主进程入口 .vite/build/early-bootstrap.js 里的一个对象字面量：
//   {"thread-stream-state-changed":13, ..., "thread-follower-start-turn":2, ...}
// asar 头：u32 魔数、u32 headerSize、u32、u32 jsonSize，JSON 目录从偏移 16 开始，
// 文件数据从 8 + headerSize 开始（不是 16 + headerSize）。
import fs from "node:fs";

export function readAsarFile(asarPath, innerPath) {
  const fd = fs.openSync(asarPath, "r");
  try {
    const head = Buffer.alloc(16);
    fs.readSync(fd, head, 0, 16, 0);
    const headerSize = head.readUInt32LE(4);
    const jsonSize = head.readUInt32LE(12);
    const json = Buffer.alloc(jsonSize);
    fs.readSync(fd, json, 0, jsonSize, 16);
    let node = JSON.parse(json.toString("utf8"));
    for (const part of innerPath.split("/")) {
      node = node.files?.[part];
      if (!node) throw new Error(`asar 里没有 ${innerPath}`);
    }
    if (node.unpacked) throw new Error(`${innerPath} 在 app.asar.unpacked 里`);
    const data = Buffer.alloc(node.size);
    fs.readSync(fd, data, 0, node.size, 8 + headerSize + Number(node.offset));
    return data.toString("utf8");
  } finally {
    fs.closeSync(fd);
  }
}

export function extractIpcVersions(source) {
  const match = source.match(/\{[^{}]*"thread-follower-start-turn":\d+[^{}]*\}/);
  if (!match) throw new Error("没找到 IPC 版本表（Codex 改了实现）");
  return JSON.parse(match[0]);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const asar = process.argv[2];
  if (!asar) {
    console.error("用法：node ipc-versions.mjs <ChatGPT.app/Contents/Resources/app.asar>");
    process.exit(2);
  }
  console.log(JSON.stringify(extractIpcVersions(readAsarFile(asar, ".vite/build/early-bootstrap.js")), null, 2));
}
