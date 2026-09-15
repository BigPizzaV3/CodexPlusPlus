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

const browsers = [
  { family: "edge", extensionId: "odlomjlbamekndcpllcnffbgeohgkmjh" },
  { family: "chrome", extensionId: "hehggadaopoacecdllhhajmbjkdcmajg" },
];

for (const browser of browsers) {
  test(`local opt-in accepts ${browser.family} metadata and preserves the original fallback`, async () => {
    const dir = await mkdtemp(join(tmpdir(), "cpp-browser-helper-"));
    const path = join(dir, "control.json");
    const turn = { session_id: "fixture-session", turn_id: "fixture-turn" };
    const info = {
      type: "extension", family: browser.family, agentRequestHeaderEnabled: false,
      metadata: { extensionId: browser.extensionId, extensionInstanceId: "fixture-extension" },
    };
    const client = { clientInfo: info };
    let calls = 0;
    const fallback = () => { calls++; throw new Error("original decision unavailable"); };
    const decide = reader({}, fallback, () => turn, path);
    try {
      await assert.rejects(decide.call(client), /original decision unavailable/);
      await writeFile(path, JSON.stringify({ schema: 1, requireIdentification: true }));
      assert.equal(await decide.call(client), true);
      assert.equal(await decide.call({ clientInfo: { ...info, agentRequestHeaderEnabled: true } }), true);
      assert.equal(calls, 1);
      for (const change of [
        { family: "firefox" }, { type: "other" }, { agentRequestHeaderEnabled: undefined },
        { agentRequestHeaderEnabled: "false" }, { agentRequestHeaderEnabled: 0 },
        { metadata: { extensionId: "unknown" } },
        { metadata: { ...info.metadata, extensionInstanceId: "" } },
        { metadata: { ...info.metadata, extensionId: "lfkehkpjohcoelkpembgemeipeppanef" } },
        { family: browsers.find((other) => other.family !== browser.family)!.family },
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
      let mutationReads = 0;
      const mutatedTurn = reader({}, fallback, () => {
        if (++mutationReads > 1) turn.turn_id = "mutated-in-place";
        return turn;
      }, path);
      await assert.rejects(mutatedTurn.call(client), /original decision unavailable/);
    } finally {
      await rm(path, { force: true });
      await rmdir(dir);
    }
  });

  test(`${browser.family} decision rechecks client, browser pair, metadata and turn after I/O`, async (t) => {
    const dir = await mkdtemp(join(tmpdir(), "cpp-browser-race-"));
    const path = join(dir, "control.json");
    try {
      await writeFile(path, JSON.stringify({ schema: 1, requireIdentification: true }));
      for (const mutation of [
        "client", "type", "family", "extension", "supported-pair", "instance", "capability",
        "session", "turn",
      ]) {
        await t.test(mutation, async () => {
          const turn = { session_id: "fixture-session", turn_id: "fixture-turn" };
          const info = {
            type: "extension", family: browser.family, agentRequestHeaderEnabled: false as boolean | undefined,
            metadata: { extensionId: browser.extensionId, extensionInstanceId: "fixture-extension" },
          };
          const client = { clientInfo: info };
          const other = browsers.find((item) => item.family !== browser.family)!;
          let reads = 0;
          let calls = 0;
          const decide = reader({}, () => { calls++; return false; }, () => {
            if (++reads === 2) {
              switch (mutation) {
                case "client": client.clientInfo = { ...info }; break;
                case "type": info.type = "other"; break;
                case "family": info.family = other.family; break;
                case "extension": info.metadata.extensionId = other.extensionId; break;
                case "supported-pair":
                  info.family = other.family;
                  info.metadata.extensionId = other.extensionId;
                  break;
                case "instance": info.metadata.extensionInstanceId = "replacement"; break;
                case "capability": info.agentRequestHeaderEnabled = undefined; break;
                case "session": turn.session_id = "next-session"; break;
                case "turn": turn.turn_id = "next-turn"; break;
              }
            }
            return turn;
          }, path);
          assert.equal(await decide.call(client), false);
          assert.equal(calls, 1);
        });
      }
    } finally {
      await rm(path, { force: true });
      await rmdir(dir);
    }
  });
}

test("unsupported clients retain the original asynchronous result and error", async () => {
  const turn = { session_id: "fixture-session", turn_id: "fixture-turn" };
  for (const clientInfo of [
    undefined, null, { type: "iab" },
    { type: "extension", family: "chrome", metadata: {} },
  ]) {
    for (const result of [true, false]) {
      let calls = 0;
      const decide = reader({}, async () => { calls++; return result; }, () => turn, "unused");
      assert.equal(await decide.call({ clientInfo }), result);
      assert.equal(calls, 1);
    }
    const originalError = new Error("original fallback failure");
    const decide = reader({}, async () => { throw originalError; }, () => turn, "unused");
    await assert.rejects(decide.call({ clientInfo }), (error: Error) => error === originalError);
  }
});

test("invalid control files never opt in either browser", async () => {
  const dir = await mkdtemp(join(tmpdir(), "cpp-browser-control-"));
  const path = join(dir, "control.json");
  try {
    for (const browser of browsers) {
      const clientInfo = {
        type: "extension", family: browser.family, agentRequestHeaderEnabled: false,
        metadata: { extensionId: browser.extensionId, extensionInstanceId: "fixture-extension" },
      };
      const metadata = () => ({ session_id: "fixture-session", turn_id: "fixture-turn" });
      const directoryControl = reader({}, () => false, metadata, dir);
      assert.equal(await directoryControl.call({ clientInfo }), false);
      await writeFile(path, `${JSON.stringify({ schema: 1, requireIdentification: true })}${" ".repeat(1024)}`);
      const oversizedControl = reader({}, () => false, metadata, path);
      assert.equal(await oversizedControl.call({ clientInfo }), false);
    }
  } finally {
    await rm(path, { force: true });
    await rmdir(dir);
  }
});
