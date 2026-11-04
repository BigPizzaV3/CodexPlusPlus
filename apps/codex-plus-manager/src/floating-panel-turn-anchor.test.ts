import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, describe, it } from "node:test";

describe("floating-panel turn anchors with goal continuations", () => {
  type Turn = { turnKey: string; userText: string; node: string; assistantMessage: { text: string; node: string } | null };
  type Anchor = { sessionId: string; turnKey: string; assistantText: string; userText: string } | null;
  let nextAnchor: (previous: Anchor, turns: Turn[], session: string) => Anchor;
  before(async () => {
    const source = await readFile(new URL("../../../assets/inject/floating-panel/runtime/answer-context.js", import.meta.url), "utf8");
    const start = source.indexOf("  function compareConversationTurnKeys(");
    const end = source.indexOf("  function updateLatestTurnAnchor(", start);
    assert.ok(start >= 0 && end > start);
    nextAnchor = new Function(source.slice(start, end) + "\nreturn nextLatestTurnAnchor;")();
  });
  const turn = (key: string, answer: string | null = "Completed assistant answer"): Turn => ({
    turnKey: key, userText: "User request", node: key,
    assistantMessage: answer === null ? null : { text: answer, node: `assistant:${key}` },
  });
  const older = "01900000-0000-7000-8000-000000000001";
  const latest = "01900000-0000-7000-8000-000000000002";

  it("selects the completed goal continuation after indexed history", () => {
    const history = turn("turn-index-23", null);
    const previous = nextAnchor(null, [history], "goal");
    const result = nextAnchor(previous, [history, turn(older), turn(latest)], "goal");
    assert.equal(result?.turnKey, latest);
    assert.equal(result?.assistantText, "Completed assistant answer");
  });

  it("orders indexed history numerically across digit boundaries", () => {
    const previous = nextAnchor(null, [turn("turn-index-9")], "goal");
    const result = nextAnchor(previous, [turn("turn-index-9"), turn("turn-index-10")], "goal");
    assert.equal(result?.turnKey, "turn-index-10");
  });

  it("does not replace a completed continuation when only old indexed history remounts", () => {
    const previous = nextAnchor(null, [turn(latest)], "goal");
    assert.equal(nextAnchor(previous, [turn("turn-index-23")], "goal"), previous);
  });

  it("uses mounted timeline order when a newer indexed turn follows a UUID turn", () => {
    const previous = nextAnchor(null, [turn(latest)], "goal");
    const result = nextAnchor(previous, [turn(latest), turn("turn-index-24", null)], "goal");
    assert.equal(result?.turnKey, "turn-index-24");
    assert.equal(result?.assistantText, "");
  });

  it("waits for a new answer and retains it across a transient remount", () => {
    const previous = nextAnchor(null, [turn(older)], "goal");
    const pending = nextAnchor(previous, [turn(older), turn(latest, null)], "goal");
    assert.equal(pending?.assistantText, "");
    const completed = nextAnchor(pending, [turn(latest)], "goal");
    assert.equal(completed?.assistantText, "Completed assistant answer");
    assert.deepEqual(nextAnchor(completed, [turn(latest, null)], "goal"), completed);
  });

  it("keeps UUID history stable and resets anchors when switching sessions", () => {
    const previous = nextAnchor(null, [turn(latest)], "goal");
    assert.equal(nextAnchor(previous, [turn(older)], "goal"), previous);
    assert.equal(nextAnchor(previous, [turn("turn-index-1")], "other")?.turnKey, "turn-index-1");
  });
});
