import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";

const { installOverlay } = createRequire(import.meta.url)("../../../assets/claude-localization/overlay.js");

function fixture() {
  const calls: any[][] = [];
  return {
    calls,
    files: {
      readFile(input: any, ...args: any[]) { calls.push(["readFile", input, ...args]); return Promise.resolve("native content"); },
      stat(input: any, ...args: any[]) { calls.push(["stat", input, ...args]); return Promise.resolve({ isFile: () => true }); },
    },
    net: { fetch(input: any, options: any) { calls.push(["fetch", input, options]); return Promise.resolve("native response"); } },
  };
}

test("资源覆盖只重定向清单内本地文件，保留官方协议及 API 请求", async () => {
  const root = mkdtempSync(path.join(os.tmpdir(), "claude-overlay-"));
  mkdirSync(path.join(root, "i18n"));
  writeFileSync(path.join(root, "index.html"), "<head></head>");
  writeFileSync(path.join(root, "i18n/zh-Hans.json"), '{"language":"zh-Hans"}');
  writeFileSync(path.join(root, "manifest.json"), "\uFEFF" + JSON.stringify({ files: [{ path: "index.html" }, { path: "i18n/zh-Hans.json" }] }));
  const target = fixture();
  const ion = path.join(root, "official/ion-dist");
  installOverlay(target, root, ion);
  await target.files.readFile(path.join(ion, "index.html"), "utf8");
  assert.deepEqual(target.calls.at(-1), ["readFile", path.join(root, "index.html"), "utf8"]);
  const options = { bigint: true };
  await target.files.stat(path.join(ion, "i18n/zh-Hans.json"), options);
  assert.deepEqual(target.calls.at(-1), ["stat", path.join(root, "i18n/zh-Hans.json"), options]);
  await target.net.fetch(pathToFileURL(path.join(ion, "i18n/zh-Hans.json")).href, undefined);
  assert.equal(target.calls.at(-1)![1], pathToFileURL(path.join(root, "i18n/zh-Hans.json")).href);
  const request = { method: "PUT", body: "unchanged" };
  await target.net.fetch("https://claude.ai/api/account_profile", request);
  assert.deepEqual(target.calls.at(-1), ["fetch", "https://claude.ai/api/account_profile", request]);
  await target.net.fetch("app://localhost/api/bootstrap", undefined);
  assert.equal(target.calls.at(-1)![1], "app://localhost/api/bootstrap");
  const privateFile = path.join(root, "user-data/settings.json");
  await target.files.readFile(privateFile);
  assert.equal(target.calls.at(-1)![1], privateFile);
  const sibling = path.join(ion, "assets/original.js");
  await target.files.stat(sibling);
  assert.equal(target.calls.at(-1)![1], sibling);
  const buffer = Buffer.from("original");
  await target.files.stat(buffer);
  assert.equal(target.calls.at(-1)![1], buffer);
});

test("资源覆盖拒绝越界清单和缺失文件，失败时不改写运行时", () => {
  const root = mkdtempSync(path.join(os.tmpdir(), "claude-overlay-invalid-"));
  writeFileSync(path.join(root, "manifest.json"), JSON.stringify({ files: [{ path: "../private.json" }] }));
  const target = fixture();
  const original = target.files.readFile;
  assert.throws(() => installOverlay(target, root, root), /增强资源/);
  assert.equal(target.files.readFile, original);
  writeFileSync(path.join(root, "manifest.json"), JSON.stringify({ files: [{ path: "index.html" }] }));
  assert.throws(() => installOverlay(target, root, root), /ENOENT/);
  assert.equal(target.files.readFile, original);
  assert.equal(target.calls.length, 0);
});
