const assert = require("node:assert/strict");
const { test } = require("node:test");
const fs = require("node:fs");
const vm = require("node:vm");

const source = fs.readFileSync(require.resolve("./site.js"), "utf8");
const releasesUrl = "https://github.com/BigPizzaV3/CodexPlusPlus/releases/latest";

const asset = (name) => ({
  name,
  size: 10 * 1024 * 1024,
  browser_download_url: `https://github.com/BigPizzaV3/CodexPlusPlus/releases/download/published-tag/${name}`,
});
const windowsAsset = asset("CodexPlusPlus-9.8.7-windows-x64-setup.exe");
const macAsset = asset("CodexPlusPlus-9.8.7-macos-universal.dmg");

async function loadSite({ release, status = 200, offline = false }) {
  const links = { windows: { href: releasesUrl }, macos: { href: releasesUrl } };
  const metadata = { windows: { textContent: "" }, macos: { textContent: "" } };
  const versions = [{ textContent: "" }, { textContent: "" }];
  const buttons = Object.fromEntries(["zh", "en"].map((language) => {
    const button = {
      dataset: { language },
      setAttribute() {},
      addEventListener(event, callback) { if (event === "click") this.click = callback; },
    };
    return [language, button];
  }));
  const elements = new Map(Object.keys(links).flatMap((platform) => [
    [`[data-download="${platform}"]`, links[platform]],
    [`[data-asset-meta="${platform}"]`, metadata[platform]],
  ]));
  const document = {
    documentElement: { dataset: { language: "zh" } },
    body: { classList: { contains: () => false } },
    createTreeWalker: () => ({ nextNode: () => null }),
    querySelector: (selector) => elements.get(selector) || null,
    querySelectorAll: (selector) => {
      if (selector === "[data-version]") return versions;
      if (selector === ".language-switch [data-language]") return Object.values(buttons);
      return [];
    },
  };
  const context = vm.createContext({
    document,
    navigator: { platform: "Win32" },
    window: { addEventListener() {}, scrollY: 0 },
    NodeFilter: { SHOW_TEXT: 4 },
    HTMLCanvasElement: class {},
    localStorage: { setItem() {} },
    URL,
    console: { info() {} },
    fetch: async (url) => {
      if (!url.endsWith("/releases/latest")) return { ok: false };
      if (offline) throw new Error("Offline");
      return { ok: status === 200, status, json: async () => release };
    },
  });
  vm.runInContext(source, context, { filename: "site.js" });
  // 等待完整页面初始化中的异步 Release 请求完成。
  await new Promise(setImmediate);
  return { links, metadata, versions, switchLanguage: (language) => buttons[language].click() };
}

function assertLanguageRoundTrip(page, expectedLinks, version = null) {
  for (const [language, fallback] of [["zh", "GitHub 最新发布"], ["en", "Latest GitHub release"], ["zh", "GitHub 最新发布"]]) {
    page.switchLanguage(language);
    assert.deepEqual(Object.fromEntries(Object.entries(page.links).map(([platform, link]) => [platform, link.href])), expectedLinks);
    assert.deepEqual(page.versions.map((element) => element.textContent), [version || fallback, version || fallback]);
  }
}

test("downloads use actual Windows and universal macOS assets across language switches", async () => {
  const page = await loadSite({ release: {
    tag_name: "v9.8.7",
    assets: [asset("CodexPlusPlus-9.8.7-macos-arm64.dmg"), windowsAsset, macAsset],
  } });
  assertLanguageRoundTrip(page, { windows: windowsAsset.browser_download_url, macos: macAsset.browser_download_url }, "v9.8.7");
  assert.match(page.metadata.macos.textContent, /通用安装包 · 10.0 MB/);
  page.switchLanguage("en");
  assert.match(page.metadata.macos.textContent, /Universal installer · 10.0 MB/);
});

test("legacy split macOS packages lead to Releases with an architecture selection prompt", async () => {
  const page = await loadSite({ release: {
    tag_name: "v1.2.46",
    assets: [windowsAsset, asset("CodexPlusPlus-1.2.46-macos-arm64.dmg"), asset("CodexPlusPlus-1.2.46-macos-x64.dmg")],
  } });
  assertLanguageRoundTrip(page, { windows: windowsAsset.browser_download_url, macos: releasesUrl }, "v1.2.46");
  assert.equal(page.metadata.macos.textContent, "前往 Releases 选择对应架构");
  page.switchLanguage("en");
  assert.equal(page.metadata.macos.textContent, "Choose your architecture on Releases");
});

for (const [label, options] of [["offline", { offline: true }], ["rate-limited", { status: 403 }]]) {
  test(`${label} API falls back to Releases without presenting a bundled version`, async () => {
    const page = await loadSite(options);
    assertLanguageRoundTrip(page, { windows: releasesUrl, macos: releasesUrl });
    assert.equal(page.metadata.windows.textContent, "前往 Releases 查看安装包");
    page.switchLanguage("en");
    assert.equal(page.metadata.macos.textContent, "View installers on Releases");
  });
}

test("a missing platform asset falls back while keeping the available asset and release version", async () => {
  const page = await loadSite({ release: { tag_name: "v9.8.7", assets: [macAsset] } });
  assertLanguageRoundTrip(page, { windows: releasesUrl, macos: macAsset.browser_download_url }, "v9.8.7");
  assert.equal(page.metadata.windows.textContent, "前往 Releases 查看安装包");
});

test("invalid download URLs fall back to Releases", async (t) => {
  for (const url of [
    "not a URL",
    "http://github.com/BigPizzaV3/CodexPlusPlus/releases/download/tag/setup.exe",
    "https://example.com/BigPizzaV3/CodexPlusPlus/releases/download/tag/setup.exe",
    "https://github.com/other/project/releases/download/tag/setup.exe",
  ]) {
    await t.test(url, async () => {
      const page = await loadSite({ release: {
        tag_name: "v9.8.7",
        assets: [{ ...windowsAsset, browser_download_url: url }, macAsset],
      } });
      assertLanguageRoundTrip(page, { windows: releasesUrl, macos: macAsset.browser_download_url }, "v9.8.7");
    });
  }
});
