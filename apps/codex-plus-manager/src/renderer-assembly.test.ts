import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

const assembler = new URL("../../../scripts/assemble-renderer-inject.mjs", import.meta.url);

// 在临时仓库运行真实组装命令，避免只测试清单内部的一致性。
async function fixture(names = ["00-prelude.js", "99-end.js"]) {
  const root = await mkdtemp(path.join(os.tmpdir(), "codex-renderer-assembly-"));
  const scriptDir = path.join(root, "scripts");
  const fragmentDir = path.join(root, "assets/inject/renderer-inject");
  await mkdir(scriptDir, { recursive: true });
  await mkdir(fragmentDir, { recursive: true });
  const script = path.join(scriptDir, "assemble-renderer-inject.mjs");
  await writeFile(script, await readFile(assembler));
  const sources: Record<string, string> = {
    "00-prelude.js": "(() => {\n",
    "99-end.js": "})();\n",
    "32-thread-goal.js": "  function installCodexThreadGoalForwarding() {}\n",
  };
  for (const [name, source] of Object.entries(sources)) {
    await writeFile(path.join(fragmentDir, name), source);
  }
  const manifestPath = path.join(fragmentDir, "manifest.json");
  const manifest = JSON.stringify({ fragments: names.map((name) => ({ name, lines: 1 })) }, null, 2) + "\n";
  await writeFile(manifestPath, manifest);
  const artifactPath = path.join(root, "assets/inject/renderer-inject.js");
  const artifact = names.map((name) => sources[name] || "").join("");
  await writeFile(artifactPath, artifact);
  return {
    run: (check: boolean) => spawnSync(process.execPath, [script, ...(check ? ["--check"] : [])], { encoding: "utf8" }),
    unchanged: async () => {
      assert.equal(await readFile(manifestPath, "utf8"), manifest);
      assert.equal(await readFile(artifactPath, "utf8"), artifact);
    },
  };
}

describe("renderer assembly inventory guard", () => {
  for (const check of [false, true]) {
    it(`rejects an omitted fragment even when the manifest and artifact agree (${check ? "check" : "write"})`, async () => {
      const f = await fixture();
      const result = f.run(check);
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /未登记的分片: 32-thread-goal\.js/);
      await f.unchanged();
    });
  }

  it("accepts all fragments explicitly listed in execution order", async () => {
    const f = await fixture(["00-prelude.js", "32-thread-goal.js", "99-end.js"]);
    const result = f.run(true);
    assert.equal(result.status, 0, result.stderr);
    await f.unchanged();
  });

  it("rejects duplicate registrations", async () => {
    const f = await fixture(["00-prelude.js", "32-thread-goal.js", "32-thread-goal.js", "99-end.js"]);
    const result = f.run(true);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /不得重复引用同一分片/);
    await f.unchanged();
  });

  it("reports a listed fragment missing from disk", async () => {
    const f = await fixture(["00-prelude.js", "32-thread-goal.js", "missing.js", "99-end.js"]);
    const result = f.run(true);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /不存在的分片: missing\.js/);
    await f.unchanged();
  });
});
