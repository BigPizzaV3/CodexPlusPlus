import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { it } from "node:test";

it("dictation records, inserts, retries, cancels and safely sends through the native composer", async () => {
  const { stdout } = await promisify(execFile)(process.execPath, [
    fileURLToPath(new URL("../../../assets/inject/dictation.test.cjs", import.meta.url)),
  ], { timeout: 10000 });
  assert.match(stdout, /dictation recording, insertion, cancellation, retry and send contracts passed/);
});
