import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const require = createRequire(import.meta.url);
const playwrightPath = process.env.SKILLS_TEST_PLAYWRIGHT;
const browserExecutable = process.env.SKILLS_TEST_BROWSER;
let esbuild: typeof import("esbuild") | undefined;
let playwright: any;
try {
  esbuild = require("esbuild");
  playwright = require(playwrightPath || "playwright");
} catch { /* Optional browser tooling is supplied by the caller. */ }

async function buildPage() {
  return esbuild!.build({
    stdin: { resolveDir: dirname(fileURLToPath(import.meta.url)), sourcefile: "skills-batch-entry.tsx", loader: "tsx", contents: `
      import React from 'react';
      import { createRoot } from 'react-dom/client';
      import { SkillsPage } from './ManagementExtra';
      let root;
      window.mountPage = () => { root = createRoot(document.getElementById('root')); root.render(<SkillsPage notify={async (...args) => window.notices.push(args)} />); };
      window.unmountPage = () => root.unmount();
    ` },
    bundle: true, write: false, outdir: "out", format: "iife", platform: "browser",
    define: { "import.meta.url": '"http://skills-fixture.invalid/main.js"' },
    plugins: [{ name: "fixture-tauri", setup(build) {
      build.onResolve({ filter: /^@tauri-apps\/api\/core$/ }, () => ({ path: "core", namespace: "fixture" }));
      build.onResolve({ filter: /^@tauri-apps\/plugin-dialog$/ }, () => ({ path: "dialog", namespace: "fixture" }));
      build.onLoad({ filter: /.*/, namespace: "fixture" }, (args) => ({ loader: "js", contents: args.path === "core"
        ? "export const invoke = (...args) => window.fixtureInvoke(...args); export const convertFileSrc = value => value; export class Channel {}"
        : "export const open = async () => null; export const save = async () => null; export const message = async () => {}; export const ask = async () => false; export const confirm = async () => false;" }));
    } }],
  });
}

async function fixture(browser: any) {
  const bundle = await buildPage();
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors: string[] = [];
  page.on("pageerror", (error: Error) => errors.push(error.message));
  await page.route("http://skills-fixture.invalid/", (route: any) => route.fulfill({ status: 200, contentType: "text/html", body: '<div data-topbar-actions></div><div id="root" style="display:flex;height:90vh"></div>' }));
  await page.goto("http://skills-fixture.invalid/");
  await page.evaluate(() => {
    const w = window as any;
    w.notices = [];
    w.fixture = { calls: [] as any[], pending: [] as any[], skills: ["alpha", "beta"].map((id) => ({ id, name: id, description: "", repoKey: "a/repo@main", installed: true, enabled: false, bundled: false, updateAvailable: false, agents: [], managed: true })) };
    w.fixtureInvoke = async (command: string, args: any = {}) => {
      w.fixture.calls.push({ command, args });
      const snapshot = () => ({ status: "ok", message: "ok", skills: structuredClone(w.fixture.skills), repos: [], repositorySkills: [], repoCounts: {}, repoErrors: [], skillsDir: "", codexSkillsDir: "" });
      if (command === "list_installed_skills" || command === "refresh_skill_catalog") return snapshot();
      if (command === "import_ccs_skills") {
        w.fixture.skills.push({ id: "gamma", name: "gamma", description: "", repoKey: "cc-switch", installed: true, enabled: false, bundled: false, updateAvailable: false, agents: [], managed: true });
        return snapshot();
      }
      if (command === "set_skill_enabled_for") return new Promise((resolve) => w.fixture.pending.push(() => {
        const target = w.fixture.skills.find((item: any) => item.id === args.id);
        const fail = args.agent === "codex" && args.id === "beta";
        if (!fail) {
          target.agents = target.agents.filter((value: string) => value !== args.agent);
          if (args.enabled) target.agents.push(args.agent);
          if (args.agent === "codex") target.enabled = args.enabled;
        }
        const result = snapshot();
        if (fail) { result.status = "failed"; result.message = "fixture failure"; }
        resolve(result);
      }));
      throw new Error(`unexpected command: ${command}`);
    };
  });
  const css = bundle.outputFiles.find((file) => file.path.endsWith(".css"));
  if (css) await page.addStyleTag({ content: css.text });
  await page.addScriptTag({ content: bundle.outputFiles.find((file) => file.path.endsWith(".js"))!.text });
  await page.evaluate(() => (window as any).mountPage());
  await page.getByRole("button", { name: /已安装/ }).click();
  await page.getByLabel("a/repo · Codex · 全选").waitFor();
  return { page, errors };
}

test("real page retains pending batch on remount and preserves unrelated controls", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser);
    const codex = page.getByLabel("a/repo · Codex · 全选");
    const claude = page.getByLabel("a/repo · Claude Code · 全选");
    await codex.check();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    assert.equal(await page.getByRole("button", { name: "从本地目录导入" }).isEnabled(), true);
    assert.equal(await page.getByRole("button", { name: "从 cc-switch 导入" }).isEnabled(), true);
    assert.equal(await claude.isEnabled(), true);
    await claude.check();
    assert.equal(await page.getByLabel("a/repo · Grok · 全选").isEnabled(), true);
    await page.evaluate(() => { (window as any).unmountPage(); (window as any).mountPage(); });
    await page.getByRole("button", { name: /已安装/ }).click();
    await page.waitForFunction(() => (document.querySelector('input[aria-label="a/repo · Codex · 全选"]') as HTMLInputElement | null)?.disabled);
    assert.equal(await claude.isDisabled(), true);
    for (let i = 0; i < 4; i++) {
      await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
      await page.evaluate(() => (window as any).fixture.pending.shift()());
      if (i === 0) await page.waitForFunction(() => (document.querySelector('input[aria-label="a/repo · Codex · 全选"]') as HTMLInputElement | null)?.checked === true);
    }
    await page.waitForFunction(() => { const input = document.querySelector('input[aria-label="a/repo · Codex · 全选"]') as HTMLInputElement | null; return input && !input.disabled && input.indeterminate; });
    await page.waitForFunction(() => { const input = document.querySelector('input[aria-label="a/repo · Claude Code · 全选"]') as HTMLInputElement | null; return input && !input.disabled && input.checked; });
    assert.equal(await page.evaluate(() => (window as any).fixture.calls.filter((call: any) => call.command === "set_skill_enabled_for").length), 4);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

test("group checkboxes stay aligned with their Agent columns at wide and narrow widths", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser);
    for (const width of [1280, 560]) {
      await page.setViewportSize({ width, height: 800 });
      const positions = await page.evaluate(() => {
        const center = (element: Element) => { const box = element.getBoundingClientRect(); return box.left + box.width / 2; };
        const headings = [...document.querySelectorAll(".mg-skill-matrix .mg-matrix-head .mg-agent-column")].map(center);
        const checks = [...document.querySelectorAll(".mg-skill-group-row")].find((row) => row.querySelector('input[aria-label="a/repo · Codex · 全选"]'))!;
        return { headings, checks: [...checks.querySelectorAll(".mg-skill-batch-cell input")].map(center) };
      });
      assert.equal(positions.headings.length, positions.checks.length);
      assert.ok(positions.headings.length > 0);
      positions.headings.forEach((value: number, index: number) => assert.ok(Math.abs(value - positions.checks[index]) < 1));
    }
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

test("filtered selection changes only visible Skills and queues a clicked import", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser);
    await page.getByPlaceholder("搜索 Skills 或仓库").fill("alpha");
    await page.getByLabel("a/repo · Codex · 全选").check();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    await page.getByRole("button", { name: "从 cc-switch 导入" }).click();
    assert.equal(await page.evaluate(() => (window as any).fixture.calls.some((call: any) => call.command === "import_ccs_skills")), false);
    await page.evaluate(() => { (window as any).unmountPage(); (window as any).mountPage(); });
    await page.getByRole("button", { name: /已安装/ }).click();
    await page.evaluate(() => (window as any).fixture.pending.shift()());
    await page.waitForFunction(() => (window as any).fixture.calls.some((call: any) => call.command === "import_ccs_skills"));
    await page.getByText("gamma", { exact: true }).waitFor();
    assert.deepEqual(await page.evaluate(() => (window as any).fixture.calls.filter((call: any) => call.command === "set_skill_enabled_for").map((call: any) => call.args.id)), ["alpha"]);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});
