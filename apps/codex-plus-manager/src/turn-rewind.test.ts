import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

for (const name of ["turn-rewind", "turn-rewind-export", "turn-rewind-menu"]) {
  test(`${name}: renderer behavior`, () => {
    const script = fileURLToPath(new URL(`../../../assets/inject/${name}.test.cjs`, import.meta.url));
    const result = spawnSync(process.execPath, [script], { encoding: "utf8", timeout: 15000 });
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}\n${result.error || ""}`);
  });
}
