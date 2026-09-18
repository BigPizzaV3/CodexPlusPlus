import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const executableSource = await readFile(
  new URL("../server/executable.mjs", import.meta.url),
  "utf8",
);
const serverSource = await readFile(new URL("../server/app.mjs", import.meta.url), "utf8");

test("server child processes hide their console window on Windows", () => {
  assert.match(executableSource, /windowsHide: process\.platform === "win32"/);
  assert.match(executableSource, /spawn\(command\.executable, command\.args, childProcessOptions\(options\)\)/);
  assert.match(executableSource, /execFileAsync\(command\.executable, command\.args, childProcessOptions\(options\)\)/);
});

test("development context Git scans use the hidden-window executable wrapper", () => {
  assert.doesNotMatch(serverSource, /const execFileAsync =/);
  assert.equal(
    [...serverSource.matchAll(/execFileExecutable\("git"/g)].length,
    3,
  );
});
