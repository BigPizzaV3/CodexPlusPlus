const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const { chromium } = require("playwright");

const source = fs.readFileSync(path.join(__dirname, "../assets/inject/renderer-inject/70-model-catalog.js"), "utf8");
const functions = source.slice(source.indexOf("  function codexRememberedThreadModels()"),
  source.indexOf("  function codexPerModelContextEnabled()"));
const html = `<!doctype html><meta charset="utf-8"><title>Thread effort regression fixture</title>
<button data-codex-intelligence-trigger>Model</button>
<button data-reasoning-slider>Power</button><output id="effort"></output>
<script>
const codexPlusBackendSettingsLoaded = true;
const scenario = new URLSearchParams(location.search).get("profile") || "a";
const aggregate = scenario.startsWith("aggregate");
const codexPlusBackendSettings = { relayProfiles: [{ id: "a", relayMode: "pureApi" }] };
const codexRemoteSessionActiveProfile = () => ({ id: scenario, relayMode: aggregate ? "aggregate" : "pureApi" });
const codexModelCatalog = { status: "ok", codex_plus_default_group_id: "b" };
const names = scenario === "missing" ? ["other"] : aggregate ? ["sol", "cpx-a/sol"] : ["sol"];
const codexPlusModelNames = () => names;
const modelReasoningEfforts = () => (scenario === "limited" ? ["low", "medium"] : ["low", "high", "xhigh"])
  .map(reasoningEffort => ({ reasoningEffort }));
const codexPlusModelMetadata = model => ({ defaultReasoningEffort: "medium",
  codex_plus_group_id: aggregate ? (model.startsWith("cpx-a/") ? "a" : "b") : undefined });
${functions}
const owner = { conversationId: location.pathname, permissionsHostId: "local" };
const picker = {
  model: names[0], reasoningEffort: "low", models: names.map(model => ({ model })),
  onSelectModel(model, effort) {
    this.model = model; this.reasoningEffort = effort; render();
  }
};
const fiber = { memoizedProps: picker, return: { memoizedProps: owner } };
for (const node of document.querySelectorAll("button")) node.__reactFiber$fixture = fiber;
function render() {
  document.querySelector("output").textContent = picker.reasoningEffort;
  document.querySelector("output").dataset.model = picker.model;
}
const slider = document.querySelector("[data-reasoning-slider]");
slider.addEventListener("pointerup", () => picker.onSelectModel("sol", "high"));
slider.addEventListener("keydown", event => {
  if (event.key === "ArrowRight") picker.onSelectModel("sol", "xhigh");
});
render();
syncCodexRetainedThreadModels();
</script>`;

(async () => {
  const server = http.createServer((_req, res) => {
    res.setHeader("Content-Type", "text/html; charset=utf-8");
    res.end(html);
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  let browser;
  let attachedPage, attachedUrl;
  try {
    const origin = `http://127.0.0.1:${server.address().port}`;
    const endpoint = process.env.CODEX_TEST_CDP_ENDPOINT;
    browser = endpoint ? await chromium.connectOverCDP(endpoint)
      : await chromium.launch({ channel: "msedge", headless: true });
    let context = endpoint ? browser.contexts()[0] : await browser.newContext();
    let page = endpoint ? context.pages()[0] : await context.newPage();
    if (endpoint) {
      attachedPage = page;
      attachedUrl = page.url();
      if (!attachedUrl.startsWith("http://tauri.localhost/")) throw new Error("Expected isolated manager WebView");
    }
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.goto(origin + "/one");
    await page.locator("[data-reasoning-slider]").click();
    await page.waitForFunction(() => JSON.parse(localStorage.getItem("codexPlus.threadModels.v1") || "[]")[0]?.effort === "high");
    await page.reload();
    await page.waitForFunction(() => document.querySelector("output").textContent === "high");
    await page.locator("[data-reasoning-slider]").press("ArrowRight");
    await page.waitForFunction(() => JSON.parse(localStorage.getItem("codexPlus.threadModels.v1"))[0].effort === "xhigh");
    await page.goto(origin + "/two");
    assert.equal(await page.locator("output").textContent(), "low");
    await page.goto(origin + "/one");
    await page.waitForFunction(() => document.querySelector("output").textContent === "xhigh");
    for (const profile of ["aggregate-1", "aggregate-2", "a", "b", "limited", "missing", "a"]) {
      await page.goto(origin + "/one?profile=" + profile);
      const effort = profile === "limited" ? "medium" : profile === "missing" ? "low" : "xhigh";
      const model = profile.startsWith("aggregate") ? "cpx-a/sol" : profile === "missing" ? "other" : "sol";
      await page.waitForFunction(({ effort, model }) => {
        const output = document.querySelector("output");
        return output.textContent === effort && output.dataset.model === model;
      }, { effort, model });
    }
    const storageState = await context.storageState();
    if (attachedPage) await attachedPage.goto(attachedUrl);
    await browser.close();
    browser = await chromium.launch({ channel: "msedge", headless: true });
    context = await browser.newContext({ storageState });
    page = await context.newPage();
    page.on("pageerror", error => errors.push(error.message));
    await page.goto(origin + "/one");
    await page.waitForFunction(() => document.querySelector("output").textContent === "xhigh");
    assert.deepEqual(errors, []);
    console.log("PASS: pointer/keyboard, reload, navigation, direct/aggregate/source/name/effort/default fallbacks, browser recreation");
  } finally {
    if (attachedPage && !attachedPage.isClosed() && attachedPage.url() !== attachedUrl) {
      await attachedPage.goto(attachedUrl).catch(() => {});
    }
    await browser?.close();
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
