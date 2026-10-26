import assert from "node:assert/strict";
import { createRequire, Module } from "node:module";
import test from "node:test";
import vm from "node:vm";

const { overrides, patchWorkspaceSource, patchOnLoad, installTimeoutOverrides } = createRequire(import.meta.url)("../../../assets/claude-localization/bash-timeout.js");
const settings = { enabled: true, defaultEnabled: true, defaultSeconds: 120, maxEnabled: true, maxSeconds: 1800 };
const fixture = `(()=>{
var im=120000,am=600000,om=2000;
const t={VA:()=>180000,fat:()=>{const value={maximum:Infinity,description:'',int(){return this},positive(){return this},max(n){this.maximum=n;return this},optional(){return this},describe(n){this.description=n;return this},parse(n){if(n!==undefined&&n>this.maximum)throw Error('too_big');return n}};return value}};
function Tm(){return {description:"Run a shell command in the session's isolated Linux workspace.",schema:{timeout_ms:t.fat().int().positive().max(am).optional().describe(\x60Timeout in milliseconds (default \x24{im}, max \x24{am}). Values above the host's MCP tool-call cap are capped to it.\x60)},call(e){let s=Date.now();let S=()=>Math.min(e.timeout_ms??im,Math.max(1e3,t.VA()-(Date.now()-s)-om));return S()}}}
function launch(){return {MCP_TOOL_TIMEOUT:String(t.VA())}}
globalThis.__codexPlusTimeoutFixture={make:Tm,launch};
})();`;

test("Cowork 超时独立覆盖，最大值不低于默认值且 watchdog 留出余量", () => {
  assert.deepEqual(overrides(settings).limits(120000, 600000), { defaultMs: 120000, maxMs: 1800000 });
  assert.equal(overrides(settings).budget(180000, 120000, 600000), 1860000);
  assert.deepEqual(overrides({ ...settings, maxEnabled: false, defaultSeconds: 900 }).limits(120000, 600000), { defaultMs: 900000, maxMs: 900000 });
  assert.equal(overrides({ ...settings, maxEnabled: false }).budget(180000, 120000, 600000), 180000);
  assert.equal(overrides({ ...settings, maxEnabled: false, defaultSeconds: 900 }).budget(180000, 120000, 600000), 960000);
  assert.deepEqual(overrides({ ...settings, defaultEnabled: false }).limits(120000, 600000), { defaultMs: 120000, maxMs: 1800000 });
  assert.deepEqual(overrides({ ...settings, defaultSeconds: 0, maxSeconds: NaN }).limits(120000, 600000), { defaultMs: 120000, maxMs: 600000 });
  assert.ok(overrides({ ...settings, maxSeconds: Number.MAX_SAFE_INTEGER }).budget(180000, 120000, 600000) <= 2147483647);
});

test("Cowork 适配只改 Bash 参数、Bash 预算和子进程 watchdog，未知结构拒绝修改", () => {
  const source = fixture + "\nfunction unrelated(){return Math.max(1e3,180000)}";
  const patched = patchWorkspaceSource(source);
  assert.ok(patched.endsWith("function unrelated(){return Math.max(1e3,180000)}"));
  assert.throws(() => patchWorkspaceSource("no workspace tool"), /未找到/);
  assert.throws(() => patchWorkspaceSource(source.replace(".max(am)", ".max(600000)")), /参数校验/);
  assert.throws(() => patchWorkspaceSource(source.replace("t.VA()-(Date.now()-s)-om", "10000")), /执行时限/);
  assert.throws(() => patchWorkspaceSource(source.replace("MCP_TOOL_TIMEOUT:String(t.VA())", "MCP_TOOL_TIMEOUT:'60000'")), /MCP/);
  assert.throws(() => patchWorkspaceSource(source + source), /参数校验/);
});

for (const wrapped of [false, true]) test(`启动期编译适配（${wrapped ? 'Electron CJS' : 'Node'}）：1800 秒通过校验及预算`, () => {
  let elapsedMs = 0;
  let clockReads = 0;
  // 每次工具调用各读取一次起点与当前时间；只控制 fixture 的时钟，不改真实全局时钟。
  const context = vm.createContext({
    Date: { now: () => 1_000_000 + (clockReads++ % 2 === 0 ? 0 : elapsedMs) },
  });
  const global = context as any;
  const compile = (source: string) => {
    const compiled = vm.runInContext(wrapped ? Module.wrap(source) : source, context, { filename: "codex-plus-timeout-fixture.js" });
    if (wrapped) compiled({}, () => {}, {}, '', '');
  };
  compile(fixture);
  const original = global.__codexPlusTimeoutFixture.make;
  assert.throws(() => original().schema.timeout_ms.parse(1800000), /too_big/);
  assert.equal(original().call({ timeout_ms: 600000 }), 178000);
  elapsedMs = 1;
  assert.equal(original().call({ timeout_ms: 600000 }), 177999);
  elapsedMs = 0;
  global.__codexPlusBashTimeout = overrides(settings);
  const loader = { _extensions: { '.js': () => { throw Error('Unexpected original loader'); } } };
  patchOnLoad(loader, 'workspace.js', patchWorkspaceSource(fixture), () => {});
  (loader._extensions['.js'] as any)({ _compile: compile }, 'workspace.js');
  const make = global.__codexPlusTimeoutFixture.make;
  const tool = make();
  assert.equal(tool.schema.timeout_ms.parse(1800000), 1800000);
  assert.match(tool.schema.timeout_ms.description, /max 1800000/);
  assert.equal(tool.call({ timeout_ms: 1800000 }), 1800000);
  assert.equal(tool.call({}), 120000);
  assert.equal(global.__codexPlusTimeoutFixture.launch().MCP_TOOL_TIMEOUT, "1860000");
  assert.throws(() => tool.schema.timeout_ms.parse(1800001), /too_big/);
  elapsedMs = 61000;
  assert.equal(tool.call({ timeout_ms: 1800000 }), 1797000, "外层预算扣除已耗时间与工具余量");
  elapsedMs = 1860000;
  assert.equal(tool.call({ timeout_ms: 1800000 }), 1000, "外层预算耗尽后仍保留原生最低等待值");
  elapsedMs = 0;
  global.__codexPlusBashTimeout = overrides({ ...settings, maxSeconds: 3, defaultSeconds: 1 });
  assert.equal(make().call({ timeout_ms: 3000 }), 3000);
  assert.equal(make().call({}), 1000);
});

test("总开关或两个子项关闭时，不装调试器、不修改运行时", async () => {
  assert.deepEqual(await installTimeoutOverrides(null), { enabled: false });
  assert.deepEqual(await installTimeoutOverrides({ ...settings, enabled: false }), { enabled: false });
  assert.deepEqual(await installTimeoutOverrides({ ...settings, defaultEnabled: false, maxEnabled: false }), { enabled: false });
  assert.equal((globalThis as any).__codexPlusBashTimeout, undefined);
});

test("延迟模块只在目标首次编译时适配，随后还原加载器，不拦截其他模块", () => {
  const calls: any[] = [];
  const original = (...args: any[]) => calls.push(args);
  const loader = { _extensions: { '.js': original } };
  let loaded = 0;
  patchOnLoad(loader, 'workspace.js', 'patched', () => loaded++);
  const module = { _compile: (...args: any[]) => calls.push(args) };
  loader._extensions['.js'](module, 'unrelated.js');
  assert.deepEqual(calls.pop(), [module, 'unrelated.js']);
  assert.equal(loaded, 0);
  loader._extensions['.js'](module, 'workspace.js');
  assert.deepEqual(calls.pop(), ['patched', 'workspace.js']);
  assert.equal(loaded, 1);
  assert.equal(loader._extensions['.js'], original);
});
