import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";

const root = path.resolve(import.meta.dirname, "../../..");
const read = (relative: string) => readFileSync(path.join(root, relative), "utf8");
const injector = read("assets/claude-localization/inject-cn.js");

/** 执行真实启动器，在启动子进程后停下；测试不启动 Claude，也不打开调试端口。 */
async function launch(platform: "darwin" | "win32", pluginDirs?: string) {
  const calls: { program: string; args: string[]; options: Record<string, any> }[] = [];
  const env: Record<string, string> = {
    CODEX_PLUS_CLAUDE_COPY: platform === "darwin" ? "/Managed/Claude Enhanced.app" : "C:\\Managed\\Claude.exe",
  };
  if (pluginDirs !== undefined) env.CLAUDE_CODE_PLUGIN_DIRS = pluginDirs;
  const children = {
    spawnSync: (program: string, args: string[], options: Record<string, any> = {}) => {
      calls.push({ program, args, options });
      return { status: 0, stdout: "", stderr: "" };
    },
    spawn: (program: string, args: string[], options: Record<string, any> = {}) => {
      calls.push({ program, args, options });
      return { unref() {} };
    },
  };
  const server = {
    on() { return server; },
    listen(_port: number, _host: string, callback: () => void) { callback(); },
    address: () => ({ port: 19411 }),
    close(callback: () => void) { callback(); },
  };
  const modules: Record<string, unknown> = {
    "node:child_process": children,
    "node:fs": { existsSync: () => true },
    "node:os": { homedir: () => "/Home" },
    "node:path": path,
    "node:net": { createServer: () => server },
  };
  await vm.runInNewContext(injector, {
    require: (name: string) => {
      assert.ok(name in modules, `测试未允许模块 ${name}`);
      return modules[name];
    },
    __dirname: "/runtime/claude-localization",
    process: { argv: ["node", "inject-cn.js"], env, platform, exit() {} },
    console: {
      log(message: string) {
        if (message.startsWith("→ 等待 inspector")) throw new Error("test launch captured");
      },
      error() {},
    },
  });
  return { calls, env };
}

test("macOS 把插件路径原样传给本次 LaunchServices 启动，位于 --args 之前", async () => {
  const dirs = "/Another/plugin:/Managed plugins/codexpp";
  const { calls } = await launch("darwin", dirs);
  const opened = calls.find((call) => call.program === "open");
  assert.ok(opened);
  const envIndex = opened.args.indexOf("--env");
  assert.ok(envIndex > 0 && envIndex < opened.args.indexOf("--args"));
  assert.equal(opened.args[envIndex + 1], `CLAUDE_CODE_PLUGIN_DIRS=${dirs}`);
  assert.equal(calls.some((call) => call.program === "launchctl"), false);
});

test("没有插件目录时保持原来的 macOS 启动参数", async () => {
  const { calls } = await launch("darwin");
  const opened = calls.find((call) => call.program === "open");
  assert.ok(opened);
  assert.deepEqual(Array.from(opened.args), [
    "-n", "-a", "/Managed/Claude Enhanced.app", "--args", "--inspect=127.0.0.1:19411",
  ]);
});

test("Windows 启动的 Claude 主进程继承完整插件目录列表", async () => {
  const dirs = "C:\\Another plugin;C:\\Managed\\codexpp";
  const { calls, env } = await launch("win32", dirs);
  const spawned = calls.find((call) => call.program === env.CODEX_PLUS_CLAUDE_COPY);
  assert.ok(spawned);
  assert.equal(spawned.options.env.CLAUDE_CODE_PLUGIN_DIRS, dirs);
  assert.equal(spawned.options.detached, true);
});

test("开关仅通过管理器命令保存，插件生成只在补丁版启动分支执行", () => {
  const core = read("crates/codex-plus-core/src/claude_localization.rs");
  const branchStart = core.indexOf('if script.starts_with("inject-cn.")');
  const launchBranch = core.slice(branchStart, core.indexOf("let mut child = command", branchStart));
  assert.match(launchBranch, /if extensions_enabled\(\)/);
  assert.match(launchBranch, /extension_claude::prepare_managed_plugin\(\)/);
  assert.match(launchBranch, /merged_plugin_dirs\(std::env::var_os\("CLAUDE_CODE_PLUGIN_DIRS"\)/);
  assert.equal(core.match(/extension_claude::prepare_managed_plugin\(\)/g)?.length, 1);
  const commands = read("apps/codex-plus-manager/src-tauri/src/commands.rs");
  assert.match(commands, /fn claude_localization_set_extensions\(enabled: bool\)/);
  assert.match(commands, /claude_localization::set_extensions_enabled\(enabled\)/);
  assert.match(read("apps/codex-plus-manager/src-tauri/src/lib.rs"), /commands::claude_localization_set_extensions/);
  assert.doesNotMatch(read("crates/codex-plus-core/src/routes.rs"), /set_extensions_enabled|claude_localization_set_extensions/);
});

test("重建本地管理器时保留客户端持有的 Claude 拓展 MCP", () => {
  const relaunch = read("scripts/relaunch-local.mjs");
  const flags = relaunch.match(/const CLIENT_OWNED_FLAGS = \[([\s\S]*?)\];/)?.[1];
  assert.ok(flags);
  assert.match(flags, /"--codex-plus-claude-extension-mcp"/);
  assert.match(flags, /"--codex-plus-extension-mcp"/);
  assert.match(relaunch, /!CLIENT_OWNED_FLAGS\.some/);
});
