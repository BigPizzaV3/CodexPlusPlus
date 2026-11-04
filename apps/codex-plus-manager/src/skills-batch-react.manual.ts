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
      window.mountPage = () => { root = createRoot(document.getElementById('root')); root.render(<SkillsPage notify={async (...args) => { window.notice = args; }} />); };
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

async function fixture(browser: any, initialSkills?: any[]) {
  const bundle = await buildPage();
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors: string[] = [];
  page.on("pageerror", (error: Error) => errors.push(error.message));
  await page.route("http://skills-fixture.invalid/", (route: any) => route.fulfill({ status: 200, contentType: "text/html", body: '<div data-topbar-actions></div><div id="root" style="display:flex;height:90vh"></div>' }));
  await page.goto("http://skills-fixture.invalid/");
  await page.evaluate((items: any[] | undefined) => {
    const w = window as any;
    w.notice = null;
    w.fixture = { calls: [] as any[], pending: [] as any[], skills: items ?? ["alpha", "beta"].map((id) => ({ id, name: id, description: "", repoKey: "a/repo@main", installed: true, enabled: false, bundled: false, updateAvailable: false, agents: [], managed: true })) };
    w.fixtureInvoke = async (command: string, args: any = {}) => {
      w.fixture.calls.push({ command, args });
      const snapshot = () => ({ status: "ok", message: "ok", skills: structuredClone(w.fixture.skills), repos: [], repositorySkills: [], repoCounts: {}, repoErrors: [], skillsDir: "", codexSkillsDir: "" });
      if (command === "list_installed_skills" || command === "refresh_skill_catalog") return snapshot();
      if (command === "import_ccs_skills") {
        w.fixture.skills.push({ id: "gamma", name: "gamma", description: "", repoKey: "cc-switch", installed: true, enabled: false, bundled: false, updateAvailable: false, agents: [], managed: true });
        return snapshot();
      }
      if (command === "update_skill") return new Promise((resolve) => w.fixture.pending.push(() => {
        const target = w.fixture.skills.find((item: any) => item.id === args.id && item.repoKey === args.repoKey);
        const fail = w.fixture.failIds ? w.fixture.failIds.includes(args.id) : args.id === "beta";
        if (!fail) target.updateAvailable = false;
        const result = snapshot();
        if (w.fixture.staleSkills) { result.skills = w.fixture.staleSkills; w.fixture.staleSkills = null; }
        if (fail) { result.status = "failed"; result.message = w.fixture.failureMessage || "fixture update failure"; }
        resolve(result);
      }));
      if (command === "install_skill") return new Promise((resolve) => w.fixture.pending.push(() => {
        const target = w.fixture.skills.find((item: any) => item.id === args.id && item.repoKey === args.repoKey);
        target.installed = true;
        resolve(snapshot());
      }));
      if (command === "uninstall_skill") return new Promise((resolve) => w.fixture.pending.push(() => {
        const target = w.fixture.skills.find((item: any) => item.id === args.id && !item.bundled);
        if (target) { target.installed = false; target.updateAvailable = false; }
        resolve(snapshot());
      }));
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
  }, initialSkills);
  const css = bundle.outputFiles.find((file) => file.path.endsWith(".css"));
  if (css) await page.addStyleTag({ content: css.text });
  await page.addScriptTag({ content: bundle.outputFiles.find((file) => file.path.endsWith(".js"))!.text });
  await page.evaluate(() => (window as any).mountPage());
  await page.getByRole("button", { name: /已安装/ }).click();
  await page.getByLabel("a/repo · Codex · 全选").waitFor();
  return { page, errors };
}

test("group toolbar icons stay visible and operate on the real component", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser);
    const actions = [
      ["新建分组", ".lucide-folder-plus"],
      ["全部折叠", ".lucide-chevrons-up"],
      ["全部展开", ".lucide-chevrons-down"],
    ];
    for (const width of [1280, 560, 375]) {
      await page.setViewportSize({ width, height: 800 });
      if (process.env.SKILLS_TEST_SCREENSHOT_DIR) {
        await page.screenshot({ path: `${process.env.SKILLS_TEST_SCREENSHOT_DIR}/toolbar-${width}.png` });
      }
      for (const [label, icon] of actions) {
        const button = page.getByRole("button", { name: label, exact: true });
        assert.equal(await button.getAttribute("title"), label);
        assert.equal(await button.locator(icon).count(), 1);
        const box = await button.boundingBox();
        assert.ok(box && box.x >= 0 && box.x + box.width <= width, `${label} is outside the ${width}px viewport: ${JSON.stringify(box)}`);
      }
      await page.getByRole("button", { name: "全部折叠", exact: true }).click();
      assert.ok(await page.locator(".mg-skill-group-row .mg-group-head[aria-expanded='false']").count() > 0);
      await page.getByRole("button", { name: "全部展开", exact: true }).click();
      assert.equal(await page.locator(".mg-skill-group-row .mg-group-head[aria-expanded='false']").count(), 0);
      if (width === 375) {
        const scroll = await page.locator(".mg-skill-matrix").evaluate((matrix: HTMLElement) => {
          matrix.scrollLeft = matrix.scrollWidth;
          return { visible: matrix.clientWidth, content: matrix.scrollWidth, position: matrix.scrollLeft };
        });
        assert.ok(scroll.content > scroll.visible && scroll.position > 0, "wide Skill columns must remain scrollable");
      }
    }
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

test("group updates queue eligible Skills across collapsed groups and distinguish duplicate IDs", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const make = (id: string, repoKey: string, updateAvailable = true) => ({ id, name: id, description: "", repoKey, installed: true, enabled: false, bundled: false, updateAvailable, agents: [], managed: true });
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser, [make("alpha", "a/repo@main"), make("beta", "a/repo@main"), make("gamma", "a/repo@main"), make("no-update", "a/repo@main", false), make("alpha", "z/repo@main"), make("zeta", "z/repo@main"), { ...make("alpha", ""), bundled: true }]);
    await page.getByRole("button", { name: "全部展开", exact: true }).click();
    const groupA = page.locator(".mg-skill-group").filter({ has: page.getByLabel("a/repo · Codex · 全选") });
    const groupZ = page.locator(".mg-skill-group").filter({ has: page.getByLabel("z/repo · Codex · 全选") });
    const builtin = page.locator(".mg-skill-group").filter({ has: page.getByLabel("内置 · Codex · 全选") });
    const empty = page.locator(".mg-skill-group").filter({ has: page.getByLabel("用户 · Codex · 全选") });
    assert.equal(await builtin.getByRole("button", { name: /更新组内可更新 Skill/ }).count(), 0);
    assert.equal(await empty.getByRole("button", { name: /更新组内可更新 Skill/ }).count(), 0);
    await groupA.locator(".mg-matrix-row").filter({ hasText: "alpha" }).getByRole("button", { name: "更新", exact: true }).click();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    assert.equal(await builtin.locator(".mg-matrix-row").filter({ hasText: "alpha" }).getByRole("button", { name: /同 ID 操作中|更新中|排队中/ }).count(), 0);
    assert.equal(await builtin.getByRole("button", { name: /更新组内可更新 Skill/ }).count(), 0);
    await groupZ.locator(".mg-matrix-row").filter({ hasText: "alpha" }).getByRole("button", { name: "同 ID 操作中" }).waitFor();
    assert.equal(await groupZ.locator(".mg-matrix-row").filter({ hasText: "alpha" }).getByRole("button", { name: "更新中" }).count(), 0);
    await groupA.locator(".mg-group-head").click();
    assert.equal(await groupA.locator(".mg-matrix-row").count(), 0);
    await groupA.getByRole("button", { name: /a\/repo · 更新组内可更新 Skill（3）/ }).click();
    assert.match((await page.evaluate(() => (window as any).notice))[1], /跳过 1 个已在操作中的 Skill/);
    await groupZ.getByRole("button", { name: /z\/repo · 更新组内可更新 Skill（2）/ }).click();
    assert.match((await page.evaluate(() => (window as any).notice))[1], /跳过 1 个已在操作中的 Skill/);
    for (let i = 0; i < 4; i++) {
      await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
      await page.evaluate(() => (window as any).fixture.pending.shift()());
    }
    await page.waitForFunction(() => (window as any).fixture.calls.filter((call: any) => call.command === "update_skill").length === 4);
    assert.deepEqual(await page.evaluate(() => (window as any).fixture.calls.filter((call: any) => call.command === "update_skill").map((call: any) => [call.args.repoKey, call.args.id])), [["a/repo@main", "alpha"], ["a/repo@main", "beta"], ["a/repo@main", "gamma"], ["z/repo@main", "zeta"]]);
    await page.waitForFunction(() => (window as any).notice?.[2] === "failed");
    assert.match((await page.evaluate(() => (window as any).notice))[1], /beta.*fixture update failure/);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

test("failure summary distinguishes duplicate IDs by repository and updates a retry", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const make = (repoKey: string) => ({ id: "alpha", name: "alpha", description: "", repoKey, installed: true, enabled: false, bundled: false, updateAvailable: true, agents: [], managed: true });
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser, [make("a/repo@main"), make("z/repo@main")]);
    await page.evaluate(() => { (window as any).fixture.failIds = ["alpha"]; });
    for (const repo of ["a/repo", "z/repo", "a/repo"]) {
      const group = page.locator(".mg-skill-group").filter({ has: page.getByLabel(`${repo} · Codex · 全选`) });
      await page.evaluate((message: string) => { (window as any).fixture.failureMessage = message; }, repo === "a/repo" ? "a failure" : "z failure");
      if (repo === "a/repo" && await page.getByRole("region", { name: "Skill 操作失败摘要" }).count()) await page.evaluate(() => { (window as any).fixture.failureMessage = "latest failure"; });
      await group.getByRole("button", { name: /更新组内可更新 Skill/ }).click();
      await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
      await page.evaluate(() => (window as any).fixture.pending.shift()());
      await page.waitForFunction(() => (window as any).fixture.pending.length === 0);
      await page.getByRole("region", { name: "Skill 操作失败摘要" }).getByText(`${repo === "a/repo" ? "a/repo@main" : "z/repo@main"}`, { exact: true }).waitFor();
    }
    const summary = page.getByRole("region", { name: "Skill 操作失败摘要" });
    await summary.getByText("3 次 Skill 操作失败").waitFor();
    assert.equal(await summary.locator("li").count(), 2);
    assert.match(await summary.locator("li").filter({ hasText: "a/repo@main" }).innerText(), /alpha：latest failure/);
    assert.match(await summary.locator("li").filter({ hasText: "z/repo@main" }).innerText(), /alpha：z failure/);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

test("update rechecks installed source at execution time", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const make = (id: string) => ({ id, name: id, description: "", repoKey: "a/repo@main", installed: true, enabled: false, bundled: false, updateAvailable: true, agents: [], managed: true });
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser, [make("alpha"), make("gamma")]);
    await page.getByRole("button", { name: /a\/repo · 更新组内可更新 Skill（2）/ }).click();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    await page.evaluate(() => { const f = (window as any).fixture; f.staleSkills = structuredClone(f.skills); f.skills.find((item: any) => item.id === "gamma").repoKey = "other/repo@main"; f.pending.shift()(); });
    await page.waitForFunction(() => (window as any).notice?.[2] === "failed");
    assert.match((await page.evaluate(() => (window as any).notice))[1], /gamma.*来源已变更/);
    assert.deepEqual(await page.evaluate(() => (window as any).fixture.calls.filter((call: any) => call.command === "update_skill").map((call: any) => call.args.id)), ["alpha"]);
    await page.getByLabel("other/repo · Codex · 全选").waitFor();
    const otherGroup = page.locator(".mg-skill-group").filter({ has: page.getByLabel("other/repo · Codex · 全选") });
    assert.equal(await otherGroup.locator(".mg-group-head").getAttribute("aria-expanded"), "false");
    await otherGroup.locator(".mg-group-head").click();
    assert.equal(await otherGroup.locator(".mg-matrix-row").filter({ hasText: "gamma" }).count(), 1);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

test("queued uninstall blocks updates after remount while other Skills remain usable", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const make = (id: string) => ({ id, name: id, description: "", repoKey: "a/repo@main", installed: true, enabled: false, bundled: false, updateAvailable: true, agents: [], managed: true });
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser, [make("alpha"), make("beta"), make("gamma")]);
    const row = (id: string) => page.locator(".mg-matrix-row").filter({ hasText: id });
    await page.getByRole("button", { name: "全部展开", exact: true }).click();
    await row("alpha").getByRole("button", { name: "更新", exact: true }).click();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    await row("beta").getByRole("button", { name: "更多操作" }).click();
    await row("beta").getByRole("menuitem", { name: "卸载" }).click();
    await row("beta").getByRole("button", { name: "卸载排队中" }).waitFor();
    await page.evaluate(() => { (window as any).unmountPage(); (window as any).mountPage(); });
    await page.getByRole("button", { name: /已安装/ }).click();
    await page.getByRole("button", { name: "全部展开", exact: true }).click();
    await row("beta").getByRole("button", { name: "卸载排队中" }).waitFor();
    assert.equal(await row("beta").getByRole("button", { name: "更新", exact: true }).count(), 0);
    await page.getByRole("button", { name: /a\/repo · 更新组内可更新 Skill（3）/ }).click();
    assert.match((await page.evaluate(() => (window as any).notice))[1], /跳过 2 个已在操作中的 Skill/);
    assert.equal(await row("gamma").getByRole("button", { name: "排队中" }).isDisabled(), true);
    for (let i = 0; i < 3; i++) {
      await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
      await page.evaluate(() => (window as any).fixture.pending.shift()());
    }
    await page.waitForFunction(() => (window as any).fixture.calls.some((call: any) => call.command === "update_skill" && call.args.id === "gamma"));
    assert.deepEqual(await page.evaluate(() => (window as any).fixture.calls.filter((call: any) => call.command === "update_skill").map((call: any) => call.args.id)), ["alpha", "gamma"]);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

test("group failure remains in the single notice after later success", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const make = (id: string) => ({ id, name: id, description: "", repoKey: "a/repo@main", installed: true, enabled: false, bundled: false, updateAvailable: true, agents: [], managed: true });
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser, [make("beta"), make("gamma")]);
    await page.getByRole("button", { name: /a\/repo · 更新组内可更新 Skill（2）/ }).click();
    for (let i = 0; i < 2; i++) {
      await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
      await page.evaluate(() => (window as any).fixture.pending.shift()());
    }
    await page.waitForFunction(() => (window as any).notice?.[2] === "failed");
    const notice = await page.evaluate(() => (window as any).notice);
    assert.match(notice[1], /已完成 1 个 Skill 操作/);
    assert.match(notice[1], /beta.*fixture update failure/);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

test("persistent bounded failure summary survives uninstall success and remount until dismissed", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const make = (id: string, updateAvailable = true) => ({ id, name: id, description: "", repoKey: "a/repo@main", installed: true, enabled: false, bundled: false, updateAvailable, agents: [], managed: true });
  const ids = ["beta", "fail-one", "fail-two", "fail-three", "fail-four"];
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser, [...ids.map((id) => make(id)), make("gamma", false)]);
    await page.getByRole("button", { name: "全部展开", exact: true }).click();
    await page.evaluate((failIds: string[]) => { (window as any).fixture.failIds = failIds; (window as any).fixture.failureMessage = "failure " + "x".repeat(500); }, ids);
    await page.getByRole("button", { name: /a\/repo · 更新组内可更新 Skill（5）/ }).click();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    const row = page.locator(".mg-matrix-row").filter({ hasText: "gamma" });
    await row.getByRole("button", { name: "更多操作" }).click();
    await row.getByRole("menuitem", { name: "卸载" }).click();
    for (let i = 0; i < 6; i++) { await page.waitForFunction(() => (window as any).fixture.pending.length === 1); await page.evaluate(() => (window as any).fixture.pending.shift()()); }
    await page.waitForFunction(() => (window as any).notice?.[2] === "ok");
    const summary = page.getByRole("region", { name: "Skill 操作失败摘要" });
    await summary.getByText("5 次 Skill 操作失败").waitFor();
    assert.equal(await summary.locator("li").count(), 5);
    for (const id of ids) {
      const item = summary.locator("li").filter({ has: page.getByText(id, { exact: true }) });
      assert.equal(await item.count(), 1);
      assert.ok((await item.innerText()).length <= "a/repo@main · ".length + id.length + 241);
    }
    const list = summary.getByRole("list", { name: "失败的 Skill 列表" });
    assert.equal(await list.evaluate((el: HTMLElement) => el.scrollHeight > el.clientHeight && getComputedStyle(el).overflowY === "auto"), true);
    await page.evaluate(() => { (window as any).unmountPage(); (window as any).mountPage(); });
    await page.getByRole("region", { name: "Skill 操作失败摘要" }).getByText("5 次 Skill 操作失败").waitFor();
    await page.getByRole("button", { name: /已安装/ }).click();
    await page.getByRole("button", { name: "全部展开", exact: true }).click();
    const beta = page.locator(".mg-matrix-row").filter({ hasText: "beta" });
    await page.evaluate(() => { (window as any).fixture.failureMessage = "latest error"; });
    await beta.getByRole("button", { name: "更新", exact: true }).click();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    await page.evaluate(() => (window as any).fixture.pending.shift()());
    await summary.getByText("6 次 Skill 操作失败").waitFor();
    assert.equal(await summary.locator("li").count(), 5);
    assert.match(await summary.locator("li").filter({ has: page.getByText("beta", { exact: true }) }).innerText(), /^a\/repo@main · beta：latest error$/);
    await page.evaluate(() => { (window as any).fixture.failIds = []; });
    await beta.getByRole("button", { name: "更新", exact: true }).click();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    await page.evaluate(() => (window as any).fixture.pending.shift()());
    await page.waitForFunction(() => (window as any).notice?.[2] === "ok" && (window as any).fixture.skills.find((item: any) => item.id === "beta").updateAvailable === false);
    await page.evaluate(() => { (window as any).unmountPage(); (window as any).mountPage(); });
    await summary.getByText("6 次 Skill 操作失败").waitFor();
    for (const id of ids) assert.equal(await summary.locator("li").filter({ has: page.getByText(id, { exact: true }) }).count(), 1);
    await page.getByRole("button", { name: "关闭 Skill 失败摘要" }).click();
    assert.equal(await page.getByRole("region", { name: "Skill 操作失败摘要" }).count(), 0);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

test("single failure followed by group success shares the final notice", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const make = (id: string) => ({ id, name: id, description: "", repoKey: "a/repo@main", installed: true, enabled: false, bundled: false, updateAvailable: true, agents: [], managed: true });
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser, [make("beta"), make("gamma")]);
    await page.getByRole("button", { name: "全部展开", exact: true }).click();
    await page.locator(".mg-matrix-row").filter({ hasText: "beta" }).getByRole("button", { name: "更新", exact: true }).click();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    await page.getByRole("button", { name: /a\/repo · 更新组内可更新 Skill（2）/ }).click();
    assert.match((await page.evaluate(() => (window as any).notice))[1], /跳过 1 个已在操作中的 Skill/);
    for (let i = 0; i < 2; i++) { await page.waitForFunction(() => (window as any).fixture.pending.length === 1); await page.evaluate(() => (window as any).fixture.pending.shift()()); }
    await page.waitForFunction(() => (window as any).notice?.[2] === "failed");
    assert.match((await page.evaluate(() => (window as any).notice))[1], /beta.*fixture update failure/);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

test("group failure followed by single install shares the final notice", { skip: !esbuild || !playwright || !browserExecutable }, async () => {
  const make = (id: string, installed: boolean) => ({ id, name: id, description: "", repoKey: "a/repo@main", installed, enabled: false, bundled: false, updateAvailable: installed, agents: [], managed: true });
  const browser = await playwright.chromium.launch({ executablePath: browserExecutable, headless: true });
  try {
    const { page, errors } = await fixture(browser, [make("beta", true), make("delta", false)]);
    await page.getByRole("button", { name: /a\/repo · 更新组内可更新 Skill（1）/ }).click();
    await page.waitForFunction(() => (window as any).fixture.pending.length === 1);
    await page.getByRole("button", { name: /发现/ }).click();
    await page.getByRole("button", { name: "全部展开", exact: true }).click();
    await page.locator(".mg-matrix-row").filter({ hasText: "delta" }).getByRole("button", { name: "安装", exact: true }).click();
    for (let i = 0; i < 2; i++) { await page.waitForFunction(() => (window as any).fixture.pending.length === 1); await page.evaluate(() => (window as any).fixture.pending.shift()()); }
    await page.waitForFunction(() => (window as any).notice?.[2] === "failed");
    const notice = await page.evaluate(() => (window as any).notice);
    assert.match(notice[1], /beta.*fixture update failure/);
    assert.match(notice[1], /已完成 1 个 Skill 操作/);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});

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
    await page.waitForFunction(() => [...document.querySelectorAll(".mg-skill-group-row")].some((row) =>
      row.querySelector("strong")?.textContent === "用户" && row.querySelector("small")?.textContent?.includes("1 个 Skill")));
    await page.getByRole("button", { name: "全部展开", exact: true }).click();
    await page.getByText("gamma", { exact: true }).waitFor();
    assert.deepEqual(await page.evaluate(() => (window as any).fixture.calls.filter((call: any) => call.command === "set_skill_enabled_for").map((call: any) => call.args.id)), ["alpha"]);
    assert.deepEqual(errors, []);
    await page.evaluate(() => (window as any).unmountPage());
  } finally { await browser.close(); }
});
