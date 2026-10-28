import assert from "node:assert/strict";
import test from "node:test";
import { noticeDuration, noticeTone } from "./notice-model.ts";

test("后端状态归为四种提示语气", () => {
  assert.equal(noticeTone(undefined), "success");
  assert.equal(noticeTone("ok"), "success");
  assert.equal(noticeTone("failed"), "error");
  assert.equal(noticeTone("FAILED"), "error");
  assert.equal(noticeTone("warning"), "warning");
  assert.equal(noticeTone("partial"), "warning");
  assert.equal(noticeTone("not_checked"), "info");
  assert.equal(noticeTone("accepted"), "success");
  assert.equal(noticeTone("anything-else"), "success");
});

test("出错的提示停留更久，长文本加时但有上限", () => {
  assert.equal(noticeDuration("success", "短消息"), 4200);
  assert.ok(noticeDuration("error", "短消息") > noticeDuration("warning", "短消息"));
  assert.ok(noticeDuration("success", "字".repeat(200)) > 4200);
  assert.equal(noticeDuration("error", "字".repeat(5000)), 15_000);
});
