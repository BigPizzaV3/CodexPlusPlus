import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

// Optional browser tooling is supplied by the caller; no packages are installed by this test.
const require = createRequire(import.meta.url);
const dependencyRoot = process.env.SKILLS_TEST_NODE_MODULES;
const browserRoot = process.env.SKILLS_TEST_PLAYWRIGHT;
const browserExecutable = process.env.SKILLS_TEST_BROWSER;
let esbuild: typeof import("esbuild") | undefined;
let playwright: any;
try {
  esbuild = require(dependencyRoot ? resolve(dependencyRoot, "esbuild") : "esbuild");
  playwright = require(browserRoot || "playwright");
} catch { /* Optional integration test dependencies. */ }

async function buildSkillsPageBundle() {
  const sourceDir = dirname(fileURLToPath(import.meta.url));
  return esbuild!.build({
    stdin: { resolveDir: sourceDir, sourcefile: "skills-lifecycle-entry.tsx", loader: "tsx", contents: `
      import React from 'react';
      import { createRoot } from 'react-dom/client';
      import { SkillsPage } from './ManagementExtra';
      const el = document.getElementById('root');
      let root;
      window.mountPage = () => { root = createRoot(el); root.render(<SkillsPage notify={async () => {}} />); };
      window.unmountPage = () => root.unmount();
    ` },
    bundle: true, write: false, format: "iife", platform: "browser", nodePaths: dependencyRoot ? [dependencyRoot] : [],
    define: { "import.meta.url": '"http://skills-fixture.invalid/main.js"' },
    loader: { ".css": "empty" },
    plugins: [{ name: "fixture-tauri", setup(build) {
      build.onResolve({ filter: /^@tauri-apps\/api\/core$/ }, () => ({ path: "core", namespace: "fixture" }));
      build.onResolve({ filter: /^@tauri-apps\/plugin-dialog$/ }, () => ({ path: "dialog", namespace: "fixture" }));
      build.onLoad({ filter: /.*/, namespace: "fixture" }, (args) => ({ loader: "js", contents: args.path === "core"
        ? "export const invoke = (...args) => window.fixtureInvoke(...args); export const convertFileSrc = value => value; export class Channel {}"
        : "export const open = async () => null; export const save = async () => null; export const message = async () => {}; export const ask = async () => false; export const confirm = async () => false;" }));
    } }],
  });
}

test("real SkillsPage keeps batch protection and refreshes after remount", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const bundle = await buildSkillsPageBundle();
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const page = await browser.newPage();
    const errors: string[] = [];
    page.on("pageerror", (error: Error) => errors.push(error.message));
    await page.route("http://skills-fixture.invalid/", (route: any) => route.fulfill({ status: 200, contentType: "text/html", body: '<div id="root"></div>' }));
    await page.goto("http://skills-fixture.invalid/");
    await page.evaluate(() => {
      const w = window as any;
      w.fixture = { calls: [] as any[], pending: [] as any[], skills: ["a", "b"].map((id) => ({ id, name: id, description: "", repoKey: "a/repo@main", installed: true, enabled: false, bundled: false, agents: [], managed: true })) };
      w.fixtureInvoke = async (command: string, args: any) => {
        w.fixture.calls.push({ command, args });
        const snapshot = () => ({ status: "ok", message: "ok", skills: structuredClone(w.fixture.skills), repos: [], pendingImportedRepos: [], repositorySkills: [], repoCounts: {}, repoErrors: [], skillsDir: "", codexSkillsDir: "" });
        if (command === "list_installed_skills") return snapshot();
        if (command === "set_skill_enabled_for") return new Promise((resolve) => w.fixture.pending.push(() => {
          if (args.id === "a") w.fixture.skills.find((item: any) => item.id === args.id).agents = [args.agent];
          const result = snapshot();
          if (args.id === "b") { result.status = "failed"; result.message = "fixture failure"; }
          resolve(result);
        }));
        throw new Error(`unexpected command: ${command}`);
      };
    });
    await page.addScriptTag({ content: bundle.outputFiles[0].text });
    await page.evaluate(() => (window as any).mountPage());
    const codex = page.locator('input[aria-label^="a/repo · Codex ·"]');
    const claude = page.locator('input[aria-label^="a/repo · Claude Code ·"]');
    await codex.waitFor();
    const initialLoads = await page.evaluate(() => (window as any).fixture.calls.filter((call: any) => call.command === "list_installed_skills").length);
    await codex.check();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    const loadsBefore = await page.evaluate(() => (window as any).fixture.calls.filter((call: any) => call.command === "list_installed_skills").length);
    assert.equal(loadsBefore, initialLoads);
    await page.evaluate(() => { (window as any).unmountPage(); (window as any).mountPage(); });
    await codex.waitFor();
    await page.waitForFunction(() => (document.querySelector('input[aria-label^="a/repo · Codex ·"]') as HTMLInputElement)?.disabled);
    assert.equal(await claude.isDisabled(), false);
    await page.evaluate(() => (window as any).fixture.pending.shift()());
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    await page.waitForFunction((previous: number) => (window as any).fixture.calls.filter((call: any) => call.command === "list_installed_skills").length >= previous + 2, loadsBefore);
    await page.evaluate(() => (window as any).fixture.pending.shift()());
    await page.waitForFunction(() => { const input = document.querySelector('input[aria-label^="a/repo · Codex ·"]') as HTMLInputElement | null; return input && !input.disabled; });
    assert.equal(await codex.evaluate((node: HTMLInputElement) => node.indeterminate), true);
    assert.equal(await page.evaluate(() => (window as any).fixture.calls.filter((call: any) => call.command === "set_skill_enabled_for").length), 2);
    const ownerLoads = await page.evaluate(() => (window as any).fixture.calls.filter((call: any) => call.command === "list_installed_skills").length);
    await claude.check();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    await page.evaluate(() => (window as any).fixture.pending.shift()());
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    assert.equal(await page.evaluate(() => (window as any).fixture.calls.filter((call: any) => call.command === "list_installed_skills").length), ownerLoads);
    await page.evaluate(() => (window as any).fixture.pending.shift()());
    await page.waitForFunction((previous: number) => (window as any).fixture.calls.filter((call: any) => call.command === "list_installed_skills").length === previous + 1, ownerLoads);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

test("real import drawer resets incomplete comparison confirmation after rechecking", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const bundle = await buildSkillsPageBundle();
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const page = await browser.newPage();
    const errors: string[] = [];
    page.on("pageerror", (error: Error) => errors.push(error.message));
    await page.route("http://skills-fixture.invalid/", (route: any) => route.fulfill({ status: 200, contentType: "text/html", body: '<div data-topbar-actions></div><div id="root"></div>' }));
    await page.goto("http://skills-fixture.invalid/");
    await page.evaluate(() => {
      const w = window as any;
      w.fixture = { calls: [] as any[], previewCount: 0 };
      w.fixtureInvoke = async (command: string, args: any) => {
        w.fixture.calls.push({ command, args });
        if (command === "list_installed_skills" || command === "apply_skill_import") return {
          status: "ok", message: "ok", skills: [], repos: [], pendingImportedRepos: [],
          repositorySkills: [], repoCounts: {}, repoErrors: [], skillsDir: "", codexSkillsDir: "",
        };
        if (command === "preview_skill_import") return {
          status: "ok", message: "ok", warning: "", candidates: [{
            id: "incoming", name: "Incoming", description: "", sourceHash: "source", repoKey: "", repoPath: "",
            matches: [], duplicateCandidates: [], error: null,
            incompleteTargets: [{ id: "existing", name: "Existing", error: "unreadable", fingerprint: `snapshot-${++w.fixture.previewCount}` }],
          }],
        };
        throw new Error(`unexpected command: ${command}`);
      };
    });
    await page.addScriptTag({ content: bundle.outputFiles[0].text });
    await page.evaluate(() => (window as any).mountPage());
    await page.getByRole("button", { name: "从 cc-switch 导入" }).click();
    const drawer = page.getByRole("dialog", { name: "选择要导入的 Skills" });
    const confirmation = drawer.getByLabel("我已了解比较不完整，仍要导入此项");
    const submit = drawer.getByRole("button", { name: "导入所选 1 项" });
    await confirmation.waitFor();
    assert.equal(await submit.isDisabled(), true);
    await confirmation.check();
    assert.equal(await submit.isEnabled(), true);
    await drawer.getByRole("button", { name: "重新检查" }).click();
    await page.waitForFunction(() => (window as any).fixture.previewCount === 2);
    await page.waitForFunction(() => {
      const checkbox = document.querySelector('.mg-skill-import-row .mg-warning input') as HTMLInputElement | null;
      return checkbox && !checkbox.checked;
    });
    assert.equal(await submit.isDisabled(), true);
    await confirmation.check();
    await submit.click();
    await page.waitForFunction(() => (window as any).fixture.calls.some((call: any) => call.command === "apply_skill_import"));
    const call = await page.evaluate(() => (window as any).fixture.calls.find((item: any) => item.command === "apply_skill_import"));
    assert.deepEqual(call.args.incompleteConfirmations, { incoming: [{ id: "existing", name: "Existing", error: "unreadable", fingerprint: "snapshot-2" }] });
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});
