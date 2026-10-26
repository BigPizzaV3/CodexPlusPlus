import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const root = new URL("./", import.meta.url);
const page = readFileSync(new URL("ComputerUsePage.tsx", root), "utf8");
const dictionary = readFileSync(new URL("i18n-en.ts", root), "utf8");

test("Computer Use exposes an explicit native/Codex++ switch for Codex", () => {
  assert.match(page, /const codexInjected = codexTarget\?\.state === "registered"/);
  assert.match(page, /mutateTarget\("codex", event\.currentTarget\.checked \? "install" : "uninstall"\)/);
  assert.match(page, /在 Codex 中使用 Codex\+\+ Computer Use/);
  assert.match(page, /关闭时只使用 Codex 原生 Computer Use/);
  assert.match(dictionary, /"在 Codex 中使用 Codex\+\+ Computer Use"/);
});
