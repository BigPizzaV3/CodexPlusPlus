import assert from "node:assert/strict";
import test from "node:test";
import {
  abandonResetCardOperation,
  beginResetCardOperation,
  finishResetCardOperation,
  pendingResetCardOperation,
  resetCardUsable,
  resetOutcomeSettled,
  soonestResetCardExpiry,
  usableResetCardCount,
} from "./reset-card-operation.ts";
import type { GatewayPoolEntry } from "./gateway-model.ts";

function storage() {
  const values = new Map<string, string>();
  return { getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); }, removeItem: (key: string) => { values.delete(key); } };
}

test("切页或重建调用方后复用未确认标识，成功后才允许新操作", () => {
  const disk = storage();
  assert.equal(beginResetCardOperation(disk, "a", () => "first"), "first");
  assert.equal(pendingResetCardOperation(disk, "a"), "first");
  assert.equal(beginResetCardOperation(disk, "a", () => "second"), "first");
  assert.equal(beginResetCardOperation(disk, "b", () => "other"), "other");
  finishResetCardOperation(disk, "a", "wrong");
  assert.equal(pendingResetCardOperation(disk, "a"), "first");
  finishResetCardOperation(disk, "a", "first");
  assert.equal(beginResetCardOperation(disk, "a", () => "second"), "second");
});

test("存储失败或损坏时阻止创建新的消费请求", () => {
  const disk = storage();
  disk.setItem("codexpp:reset-card-operation:a", "bad\nid");
  assert.throws(() => beginResetCardOperation(disk, "a", () => "new"));
  const unavailable = { ...storage(), setItem: () => { throw Error("unavailable"); } };
  assert.throws(() => beginResetCardOperation(unavailable, "a", () => "new"));
});

test("到期时不再信任旧 usable，未知张数保持 null", () => {
  const card = { id: "one", status: "available", usable: true, expiresAtMs: 2000 };
  assert.equal(resetCardUsable(card, 1999), true);
  assert.equal(resetCardUsable(card, 2000), false);
  const entry = { resetCredits: { observedAtMs: 1, credits: [card] } } as GatewayPoolEntry;
  assert.equal(usableResetCardCount(entry, 1999), 1);
  assert.equal(usableResetCardCount(entry, 2000), 0);
  assert.equal(soonestResetCardExpiry(entry, 2000), null);
  entry.resetCredits!.credits = [];
  assert.equal(usableResetCardCount(entry), null);
  entry.resetCredits!.availableCount = 0;
  assert.equal(usableResetCardCount(entry), 0);
});

test("确定结果结束操作，结果未知或没发出请求时保留", () => {
  for (const outcome of ["reset", "alreadyRedeemed", "noCredit", "nothingToReset", "rejected"]) {
    assert.equal(resetOutcomeSettled(outcome), true, outcome);
  }
  for (const outcome of ["unknown", undefined, null, "", "no_credit"]) {
    assert.equal(resetOutcomeSettled(outcome), false, String(outcome));
  }
});

test("放弃后另起新标识，损坏的记录也能清掉", () => {
  const disk = storage();
  assert.equal(beginResetCardOperation(disk, "a", () => "first"), "first");
  abandonResetCardOperation(disk, "a");
  assert.equal(pendingResetCardOperation(disk, "a"), null);
  assert.equal(beginResetCardOperation(disk, "a", () => "second"), "second");
  disk.setItem("codexpp:reset-card-operation:b", "bad\nid");
  assert.throws(() => pendingResetCardOperation(disk, "b"));
  abandonResetCardOperation(disk, "b");
  assert.equal(beginResetCardOperation(disk, "b", () => "fresh"), "fresh");
});
