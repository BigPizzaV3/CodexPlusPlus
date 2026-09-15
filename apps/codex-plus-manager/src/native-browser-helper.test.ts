import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile, rm, rmdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

// Execute only our helper, not the proprietary service, identity or policy implementations.
const source = await readFile(new URL("../../../assets/native-browser/require-identification.mjs", import.meta.url), "utf8");
const { cppNativeIdentificationReader: reader } = await import(
  `data:text/javascript;base64,${Buffer.from(`${source}\nexport { cppNativeIdentificationReader };`).toString("base64")}`
);

test("local opt-in uses real Edge metadata and preserves the original fallback", async () => {
  const dir = await mkdtemp(join(tmpdir(), "cpp-browser-helper-"));
  const path = join(dir, "control.json");
  const turn = { session_id: "fixture-session", turn_id: "fixture-turn" };
  const info = {
    type: "extension", family: "edge", agentRequestHeaderEnabled: false,
    metadata: { extensionId: "odlomjlbamekndcpllcnffbgeohgkmjh" },
  };
  const client = { clientInfo: info };
  let calls = 0;
  const fallback = () => { calls++; throw new Error("original decision unavailable"); };
  const decide = reader({}, fallback, () => turn, path);
  try {
    await assert.rejects(decide.call(client), /original decision unavailable/);
    await writeFile(path, JSON.stringify({ schema: 1, requireIdentification: true }));
    assert.equal(await decide.call(client), true);
    assert.equal(calls, 1);
    for (const change of [
      { family: "chrome" }, { type: "other" }, { agentRequestHeaderEnabled: undefined },
      { metadata: { extensionId: "unknown" } },
    ]) {
      await assert.rejects(decide.call({ clientInfo: { ...info, ...change } }), /original decision unavailable/);
    }
    for (const control of [
      {}, { schema: 2, requireIdentification: true }, { schema: 1, requireIdentification: false },
      { schema: 1, requireIdentification: "true" },
    ]) {
      await writeFile(path, JSON.stringify(control));
      await assert.rejects(decide.call(client), /original decision unavailable/);
    }
    await writeFile(path, "bad json");
    await assert.rejects(decide.call(client), /original decision unavailable/);
    await writeFile(path, JSON.stringify({ schema: 1, requireIdentification: true }));
    let reads = 0;
    const changedTurn = reader({}, fallback, () => ++reads === 1 ? turn : { ...turn, turn_id: "next" }, path);
    await assert.rejects(changedTurn.call(client), /original decision unavailable/);
    const noTurn = reader({}, fallback, () => null, path);
    await assert.rejects(noTurn.call(client), /original decision unavailable/);
  } finally {
    await rm(path, { force: true });
    await rmdir(dir);
  }
});
