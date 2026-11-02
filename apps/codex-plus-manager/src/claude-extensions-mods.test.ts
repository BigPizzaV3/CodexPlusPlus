import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const template = readFileSync(new URL("../../../assets/claude-extensions/register.js", import.meta.url), "utf8");
const extension = (key = "user:demo") => ({
  key, id: "demo", name: "Demo",
  buttons: [{ id: "inspect", label: "Inspect", method: "status" }],
  events: {
    "session.start": "observe",
    "turn.start": "observe",
    "turn.complete": "observe",
    "session.end": "observe",
  },
  tools: [{
    name: "status", description: "Read status", method: "status",
    inputSchema: { type: "object", properties: { detail: { type: "boolean" } }, additionalProperties: false },
  }],
});

function harness(extensions = [extension()]) {
  const handlers: Array<{ event: string; matcher: any; handler: any }> = [];
  const calls: Array<{ server: string; tool: string; params: any }> = [];
  const connections: string[] = [];
  const registered: any[] = [];
  const logs: string[] = [];
  const toasts: string[] = [];
  let sessionId = "session-one";
  let response: any = { content: [{ type: "text", text: '{"text":"Backend connected"}' }], isError: false };
  let fail: ((params: any) => boolean) | undefined;
  let registrationFails = false;
  const config = {
    serverName: "plugin:codexpp-extension-host:codexpp-extensions",
    toolName: "codexpp_extension_call",
    extensions,
  };
  const source = template.replace("/*CODEXPP_CONFIG*/{}", JSON.stringify(config)).replace("export function register", "function register");
  const context = vm.createContext({});
  vm.runInContext(source, context);
  context.register((event: string, matcher: any, handler?: any) => {
    handlers.push({ event, matcher: handler ? matcher : null, handler: handler || matcher });
  });
  const $ = {
    mcp: {
      connect: async (server: string) => {
        connections.push(server);
        return { isConnected: true, server: config.serverName };
      },
      call: async (server: string, tool: string, params: any) => {
        calls.push({ server, tool, params: structuredClone(params) });
        if (fail?.(params)) throw new Error("backend unavailable");
        return structuredClone(response);
      },
    },
    session: { id: async () => sessionId },
    tool: {
      register: async (tool: any) => {
        if (registrationFails) throw new Error("tool refused");
        registered.push(structuredClone(tool));
      },
    },
    ui: {
      log: async (text: string) => { logs.push(text); },
      toast: async (text: string) => { toasts.push(text); },
      resolve: () => ({
        Box: (props: any) => ({ type: "box", ...props }),
        Button: (props: any) => ({ type: "button", ...props }),
      }),
    },
  };
  return {
    calls, connections, registered, logs, toasts, $,
    setSession: (value: string) => { sessionId = value; },
    setResponse: (value: any) => { response = value; },
    failCalls: (value: (params: any) => boolean) => { fail = value; },
    failRegistration: () => { registrationFails = true; },
    async fire(event: string, data: any, downstream: any = { type: "downstream" }) {
      const entry = handlers.find(item => item.event === event &&
        (!item.matcher || Object.entries(item.matcher).every(([key, value]) =>
          Array.isArray(value) ? value.includes(data[key]) : value === data[key])));
      assert.ok(entry, "registered event: " + event);
      let nextCalls = 0;
      const result = await entry.handler($, data, async (received: any) => {
        assert.equal(received, data, "pass the original event unchanged");
        nextCalls++;
        return downstream;
      });
      return { result, nextCalls };
    },
  };
}

test("Claude buttons preserve other Mods and use the scoped persistent backend", async () => {
  const host = harness();
  const downstream = { type: "another-mod" };
  const rendered = await host.fire("ui.render", { component: "AbovePrompt", surface: "desktop" }, downstream);
  assert.equal(rendered.nextCalls, 1);
  assert.equal(rendered.result.children[0], downstream);
  const button = rendered.result.children[1].children[0];
  assert.equal(button.label, "Inspect");
  await button.onPress();
  assert.deepEqual(host.connections, ["codexpp-extensions"]);
  assert.deepEqual(host.calls, [{
    server: "plugin:codexpp-extension-host:codexpp-extensions",
    tool: "codexpp_extension_call",
    params: {
      key: "user:demo", method: "status", sessionId: "session-one",
      params: { host: "claudeDesktop", source: "button", buttonId: "inspect", sessionId: "session-one" },
    },
  }]);
  assert.deepEqual(host.toasts, ["Backend connected"]);
});

test("real lifecycle events forward metadata and refresh the session on resume", async () => {
  const host = harness();
  for (const event of ["session.start", "turn.start", "turn.complete", "session.end"]) {
    const data = {
      turnId: "turn-one", agentId: "agent-one", isAborted: false,
      reason: "resume", text: "private prompt", answer: "private answer",
    };
    assert.equal((await host.fire(event, data)).nextCalls, 1);
  }
  assert.equal(host.registered.length, 1);
  assert.equal(host.registered[0].name, "cpx_0_status");
  assert.deepEqual(host.calls.map(call => call.params.params.event), [
    "session.start", "turn.start", "turn.complete", "session.end",
  ]);
  for (const call of host.calls) {
    assert.equal(call.params.sessionId, "session-one");
    assert.equal(call.params.params.turnId, "turn-one");
    assert.ok(!("text" in call.params.params));
    assert.ok(!("answer" in call.params.params));
  }
  host.setSession("session-two");
  assert.equal((await host.fire("classic.SessionStart", { source: "resume" })).nextCalls, 1);
  assert.equal(host.calls.at(-1)?.params.sessionId, "session-two");
  assert.equal(host.calls.at(-1)?.params.params.event, "session.start");
  assert.equal(host.calls.at(-1)?.params.params.source, "resume");
});

test("model tools send schema inputs and leave unrelated Claude tools unchanged", async () => {
  const host = harness();
  host.setResponse({ content: [{ type: "text", text: '{"count":3}' }], isError: false });
  const own = await host.fire("tool.call", {
    tool: "mcp__codexpp-extension-host__cpx_0_status",
    detail: true, agentId: "agent-private", tool_use_id: "call-private", toolUseId: "call-private", extra: "not declared",
  });
  assert.deepEqual(structuredClone(own.result), { result: '{"count":3}' });
  assert.equal(own.nextCalls, 0);
  assert.deepEqual(host.calls[0].params.params, { detail: true });
  const before = host.calls.length;
  const builtin = await host.fire("tool.call", { tool: "Bash", command: "pwd" });
  assert.equal(builtin.nextCalls, 1);
  assert.equal(host.calls.length, before);
});

test("one broken extension cannot interrupt session events or the remaining extensions", async () => {
  const host = harness([extension("user:broken"), extension("user:healthy")]);
  host.failCalls(params => params.key === "user:broken");
  host.failRegistration();
  assert.equal((await host.fire("session.start", {})).nextCalls, 1);
  assert.deepEqual(host.calls.map(call => call.params.key), ["user:broken", "user:healthy"]);
  assert.equal(host.logs.length, 3);
  assert.equal((await host.fire("turn.complete", {})).nextCalls, 1);
  assert.equal(host.calls.at(-1)?.params.key, "user:healthy");
});

test("backend errors produce a useful button or model result", async () => {
  const host = harness();
  host.setResponse({ isError: true, content: [{ type: "text", text: "Extension was disabled" }] });
  const render = await host.fire("ui.render", { component: "AbovePrompt", surface: "desktop" });
  await render.result.children[1].children[0].onPress();
  assert.match(host.toasts[0], /Extension was disabled/);
  const tool = await host.fire("tool.call", { tool: "mcp__codexpp-extension-host__cpx_0_status" });
  assert.match(tool.result.result, /Extension was disabled/);
  host.$.mcp.connect = async () => ({
    isConnected: false, server: "", message: "MCP connection was refused",
  });
  const refused = await host.fire("tool.call", { tool: "mcp__codexpp-extension-host__cpx_0_status" });
  assert.match(refused.result.result, /MCP connection was refused/);
});

test("empty registrations and other surfaces preserve the downstream interface", async () => {
  for (const [extensions, surface] of [[[], "desktop"], [[extension()], "terminal"]] as const) {
    const host = harness([...extensions]);
    const downstream = { type: "engine" };
    const { result, nextCalls } = await host.fire("ui.render", { component: "AbovePrompt", surface }, downstream);
    assert.equal(result, downstream);
    assert.equal(nextCalls, 1);
    assert.equal(host.calls.length, 0);
  }
});
