import assert from "node:assert/strict";
import { test } from "node:test";
import { applyPluginInstallation, filterMarketPlugins, mergePluginMarketCatalog, pluginMarketPage, PluginMarketRequests, type MarketCatalogs, type MarketPlugin, type MarketResult } from "./plugin-market-model.ts";

function plugin(id: string, overrides: Partial<MarketPlugin> = {}): MarketPlugin {
  return {
    id, name: id, displayName: id, description: "", version: "1.0", author: "", tags: [],
    license: "MIT", path: `plugins/${id}`, tree: "tree", bytes: 100, skills: 1,
    requiresAuth: false, installed: false, installedVersion: "", updateAvailable: false,
    ...overrides,
  };
}

function catalog(source: "public" | "full", plugins: MarketPlugin[]): MarketResult {
  return { status: "ok", message: "", source, repository: "owner/cache", total: plugins.length, plugins };
}

test("search matches multiple terms across plugin metadata without changing entries", () => {
  const entries = [
    plugin("data-browser", { displayName: "数据浏览", description: "Browse SQL records", author: "Example", tags: ["analytics"] }),
    plugin("other", { description: "Browse files", tags: ["files"] }),
  ];
  for (const query of [" DATA-BROWSER ", "数据浏览", "SQL analytics", "example records"]) {
    assert.deepEqual(filterMarketPlugins(entries, query, "all"), [entries[0]]);
  }
  assert.deepEqual(filterMarketPlugins(entries, "SQL files", "all"), []);
  assert.deepEqual(filterMarketPlugins(entries, " \n ", "all"), entries);
  assert.equal(entries[0].installed, false);
});

test("installation, update and authorization filters remain independent", () => {
  const entries = [
    plugin("new"),
    plugin("installed", { installed: true, installedVersion: "1.0" }),
    plugin("outdated", { installed: true, installedVersion: "0.9", updateAvailable: true }),
    plugin("authorization", { requiresAuth: true }),
  ];
  assert.deepEqual(filterMarketPlugins(entries, "", "available").map((entry) => entry.id), ["new", "authorization"]);
  assert.deepEqual(filterMarketPlugins(entries, "", "installed").map((entry) => entry.id), ["installed", "outdated"]);
  assert.deepEqual(filterMarketPlugins(entries, "", "updates").map((entry) => entry.id), ["outdated"]);
  assert.deepEqual(filterMarketPlugins(entries, "", "auth").map((entry) => entry.id), ["authorization"]);
  assert.deepEqual(filterMarketPlugins(entries, "outdated", "available"), []);
});

test("50-entry pages clamp after a search or installation removes rows", () => {
  const entries = Array.from({ length: 101 }, (_, index) => plugin(String(index)));
  assert.equal(pluginMarketPage(entries, 0).visible.length, 50);
  assert.equal(pluginMarketPage(entries, 1).visible[0].id, "50");
  assert.deepEqual(pluginMarketPage(entries, 2).visible.map((entry) => entry.id), ["100"]);
  const narrowed = pluginMarketPage(entries.slice(0, 12), 2);
  assert.equal(narrowed.currentPage, 0);
  assert.equal(narrowed.totalPages, 1);
  assert.equal(narrowed.visible.length, 12);
  assert.deepEqual(pluginMarketPage([], 20), { currentPage: 0, totalPages: 1, visible: [] });
  assert.equal(pluginMarketPage(entries, -1).currentPage, 0);
  assert.equal(pluginMarketPage(entries, Number.NaN).currentPage, 0);
});

test("successful installation updates only its source and ID, preserving same-name variants", () => {
  const first = plugin("first", { name: "same-name" });
  const variant = plugin("second", { name: "same-name" });
  const catalogs: MarketCatalogs = { public: catalog("public", [first]), full: catalog("full", [first, variant]) };
  const installed = { ...first, installed: true, installedVersion: "1.0" };
  const next = applyPluginInstallation(catalogs, "full", installed);
  assert.equal(next.full?.plugins[0].installed, true);
  assert.equal(next.full?.plugins[1], variant);
  assert.equal(next.full?.plugins[1].installed, false);
  assert.equal(next.public, catalogs.public);
  assert.equal(next.public?.plugins[0].installed, false);
  assert.equal(catalogs.full?.plugins[0].installed, false);
});

test("installation status updates keep catalog metadata and ignore unknown IDs", () => {
  const entry = plugin("existing", { version: "2.0", tags: ["search"], updateAvailable: true });
  const catalogs = { public: catalog("public", [entry]) };
  const next = applyPluginInstallation(catalogs, "public", { ...entry, description: "different", installed: true, installedVersion: "2.0", updateAvailable: false });
  assert.equal(next.public?.plugins[0].description, entry.description);
  assert.equal(next.public?.plugins[0].tags, entry.tags);
  assert.equal(next.public?.plugins[0].updateAvailable, false);
  assert.equal(applyPluginInstallation(catalogs, "public", plugin("unknown")), catalogs);
  assert.equal(applyPluginInstallation(catalogs, "full", entry), catalogs);
});

test("reading the first catalog cannot start a plugin installation", () => {
  const requests = new PluginMarketRequests();
  assert.equal(requests.beginCatalog(), null);
  assert.equal(requests.beginInstall(), null);
  requests.activate();
  const catalogRequest = requests.beginCatalog()!;
  assert.equal(requests.beginInstall(), null);
  assert.equal(requests.isCurrentCatalog(catalogRequest), true);
  requests.finishCatalog(catalogRequest);
  assert.notEqual(requests.beginInstall(), null);
});

test("a source change invalidates older results and cannot clear the newer loading state", () => {
  const requests = new PluginMarketRequests();
  requests.activate();
  const publicRequest = requests.beginCatalog()!;
  requests.invalidateCatalog();
  assert.equal(requests.isCurrentCatalog(publicRequest), false);
  const fullRequest = requests.beginCatalog()!;
  assert.equal(requests.finishCatalog(publicRequest), false);
  assert.equal(requests.beginInstall(), null);
  assert.equal(requests.isCurrentCatalog(fullRequest), true);
  assert.equal(requests.finishCatalog(fullRequest), true);
  assert.equal(requests.isCurrentCatalog(fullRequest), false);
});

test("installations serialize and prevent catalog refresh or source changes even before a repaint", () => {
  const requests = new PluginMarketRequests();
  requests.activate();
  const first = requests.beginInstall()!;
  assert.equal(requests.beginInstall(), null);
  assert.equal(requests.beginCatalog(), null);
  assert.equal(requests.canSelectSource(), false);
  assert.equal(requests.finishInstall(first), true);
  assert.equal(requests.canSelectSource(), true);
  assert.equal(requests.finishInstall(first), false);
  const second = requests.beginInstall()!;
  assert.equal(requests.isCurrentInstall(first), false);
  assert.equal(requests.isCurrentInstall(second), true);
});

test("route disposal invalidates late catalog and installation responses, including reactivation", () => {
  const requests = new PluginMarketRequests();
  requests.activate();
  const oldCatalog = requests.beginCatalog()!;
  requests.dispose();
  requests.activate();
  const currentCatalog = requests.beginCatalog()!;
  assert.equal(requests.isCurrentCatalog(oldCatalog), false);
  assert.equal(requests.finishCatalog(oldCatalog), false);
  assert.equal(requests.isCurrentCatalog(currentCatalog), true);
  requests.finishCatalog(currentCatalog);
  const oldInstall = requests.beginInstall()!;
  requests.dispose();
  requests.activate();
  const currentInstall = requests.beginInstall()!;
  assert.equal(requests.isCurrentInstall(oldInstall), false);
  assert.equal(requests.finishInstall(oldInstall), false);
  assert.equal(requests.isCurrentInstall(currentInstall), true);
});

test("a failed refresh finishes its request without changing already-installed catalog state", () => {
  const installed = plugin("local", { installed: true, installedVersion: "1.0" });
  const catalogs = { public: catalog("public", [installed]) };
  const requests = new PluginMarketRequests();
  requests.activate();
  const refresh = requests.beginCatalog()!;
  const failed = { ...catalog("public", []), status: "failed", message: "network unavailable" };
  const result = mergePluginMarketCatalog(catalogs, "public", failed);
  assert.equal(result, catalogs);
  assert.equal(mergePluginMarketCatalog(catalogs, "public", catalog("full", [])), catalogs);
  assert.equal(requests.finishCatalog(refresh), true);
  assert.equal(result.public?.plugins[0], installed);
  assert.equal(result.public?.plugins[0].installed, true);
  const fresh = catalog("public", [installed, plugin("new")]);
  assert.equal(mergePluginMarketCatalog(catalogs, "public", fresh).public, fresh);
  assert.notEqual(requests.beginInstall(), null);
});
