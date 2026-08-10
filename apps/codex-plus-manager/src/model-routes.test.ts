import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("model route inputs keep focus while editing and label the example placeholder", async () => {
  const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");

  assert.match(source, /key=\{`model-route-\$\{index\}`\}/);
  assert.doesNotMatch(source, /key=\{`\$\{route\.model\}-\$\{index\}`\}/);
  assert.match(source, /placeholder=\{t\("例：gpt-5\.6-luna"\)\}/);
});
