const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

// 最小合成 ASAR，仅用于检查命令行诊断，不包含官方应用资产。
function run(t, html, modules = {}) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "codex-fast-check-"));
  const file = path.join(directory, "app.asar");
  t.after(() => {
    if (fs.existsSync(file)) fs.unlinkSync(file);
    fs.rmdirSync(directory);
  });
  const root = { files: {} };
  const buffers = [];
  let offset = 0;
  for (const [name, source] of Object.entries({ "webview/index.html": html, ...modules })) {
    const parts = name.split("/");
    let node = root;
    for (const part of parts.slice(0, -1)) node = node.files[part] ??= { files: {} };
    const buffer = Buffer.from(source);
    node.files[parts.at(-1)] = { size: buffer.length, offset: String(offset) };
    buffers.push(buffer);
    offset += buffer.length;
  }
  const json = Buffer.from(JSON.stringify(root));
  const padded = Math.ceil((json.length + 4) / 4) * 4;
  const header = Buffer.alloc(16);
  header.writeUInt32LE(4, 0);
  header.writeUInt32LE(padded + 4, 4);
  header.writeUInt32LE(padded, 8);
  header.writeUInt32LE(json.length, 12);
  // JSON 紧随 16 字节头，padding 位于 JSON 后。
  fs.writeFileSync(file, Buffer.concat([header, json, Buffer.alloc(padded - 4 - json.length), ...buffers]));
  const result = spawnSync(process.execPath, [path.join(__dirname, "../check-native-fast.cjs"), file], {
    encoding: "utf8", windowsHide: true, timeout: 10000,
  });
  assert.ifError(result.error);
  return result;
}

test("Rspack static check reports runtime validation required, not unsupported", t => {
  const result = run(t, '<script type="module" src="./assets/123.abcdef.js"></script>', {
    "webview/assets/123.abcdef.js": 'const chunk={};chunk.__rspack_esm_ids;chunk.__webpack_modules__;const req={};export{req as __webpack_require__};',
  });
  assert.equal(result.status, 2);
  assert.match(result.stdout, /rspack-runtime-check-required/);
  assert.doesNotMatch(result.stdout + result.stderr, /无法打补丁|没有找到/);
});

test("Legacy entries still use the static patch planner", t => {
  const result = run(t, '<script type="module" src="./assets/index-test.js"></script>', {
    "webview/assets/index-test.js": "export const unrelated = true;",
  });
  assert.equal(result.status, 1);
  assert.match(result.stdout, /无法打补丁/);
  assert.doesNotMatch(result.stdout, /rspack-runtime-check-required/);
});

test("Unrecognized entries remain a discovery failure", t => {
  const result = run(t, '<script type="module" src="https://example.com/assets/123.abcdef.js"></script>');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /没有找到可识别的模块入口/);
});
