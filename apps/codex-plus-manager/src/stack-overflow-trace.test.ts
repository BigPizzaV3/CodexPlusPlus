import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

// #3036 / #3078：发往 app-server 的包装捕获到栈溢出时只记录诊断，原样抛出，不改变行为。
const prelude = readFileSync(
  new URL("../../../assets/inject/renderer-inject/00-prelude.js", import.meta.url), "utf8");

function functionSource(name: string) {
  const start = prelude.indexOf(`  function ${name}(`);
  assert.ok(start >= 0, `prelude 里要有 ${name}`);
  let depth = 0;
  for (let index = prelude.indexOf("{", start); index < prelude.length; index++) {
    if (prelude[index] === "{") depth++;
    if (prelude[index] === "}" && --depth === 0) return prelude.slice(start, index + 1);
  }
  throw new Error(`${name} 截取失败`);
}

function runtime() {
  const logged: Array<{ event: string; detail: any }> = [];
  const body = [
    "const codexPlusStackOverflowSeen = new Set();",
    functionSource("codexPlusIsStackOverflow"),
    functionSource("codexPlusNoteStackOverflow"),
    functionSource("codexPlusTraceStackOverflow"),
    "return { trace: codexPlusTraceStackOverflow, note: codexPlusNoteStackOverflow };",
  ].join("\n");
  const api = new Function("sendCodexPlusDiagnostic", body)(
    (event: string, detail: any) => logged.push({ event, detail }));
  return { ...api, logged };
}

function overflow(): never {
  const recurse = (): number => recurse() + 1;
  recurse();
  throw new Error("unreachable");
}

describe("stack overflow trace (#3036)", () => {
  it("keeps return value, this and own markers of the traced function", async () => {
    const { trace, logged } = runtime();
    const pending = Promise.resolve("ok");
    const target: any = {
      name: "client",
      send(method: string) { return method === "sync" ? this.name : pending; },
    };
    target.send.__codexPlusPluginNativeTransport = "7";
    assert.equal(trace(target, "send", "client.sendRequest"), true);
    assert.equal(target.send("sync"), "client", "this 不变");
    assert.equal(target.send("async"), pending, "不替换返回的 Promise");
    assert.equal(await target.send("async"), "ok");
    assert.equal(target.send.__codexPlusPluginNativeTransport, "7", "原函数上的标记属性保留");
    assert.equal(trace(target, "send", "client.sendRequest"), false, "已经套过不再套");
    assert.deepEqual(logged, []);
  });

  it("logs a stack overflow once per entry and rethrows it unchanged", async () => {
    const { trace, logged } = runtime();
    const target: any = {
      sync() { overflow(); },
      async later() { overflow(); },
      other() { throw new TypeError("not an overflow"); },
    };
    for (const key of ["sync", "later", "other"]) trace(target, key, `client.${key}`);

    let thrown: unknown;
    try { target.sync("thread/start"); } catch (error) { thrown = error; }
    assert.ok(thrown instanceof RangeError, "原样抛出");
    assert.throws(() => target.sync("thread/start"), RangeError);
    await assert.rejects(target.later("thread/start"), RangeError);
    assert.throws(() => target.other("thread/start"), TypeError);
    await new Promise((resolve) => setTimeout(resolve, 0));

    assert.deepEqual(logged.map((entry) => [entry.event, entry.detail.where, entry.detail.method]), [
      ["stack_overflow", "client.sync", "thread/start"],
      ["stack_overflow", "client.later", "thread/start"],
    ], "同一入口只记一次，非栈溢出不记");
    assert.match(logged[0].detail.stack, /RangeError: Maximum call stack size exceeded/);
    assert.ok(logged[0].detail.stack.split("\n").length <= 40, "只留栈的前 40 行");
  });

  it("ignores non-function targets and swallows its own failures", () => {
    const { trace, note } = runtime();
    assert.equal(trace({ send: 1 }, "send", "x"), false);
    assert.equal(trace(null, "send", "x"), false);
    assert.equal(trace(Object.freeze({ send() {} }), "send", "x"), false);
    assert.doesNotThrow(() => note("x", "", { name: "RangeError", get message() { throw new Error("boom"); } }));
  });
});
