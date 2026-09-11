import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const root = path.join(import.meta.dirname);
const marketplacePath = path.resolve(root, "..", ".agents", "plugins", "marketplace.json");
const pluginNames = ["xuan-workspace-search", "xuan-usage", "xuan-polish"];
const pluginVersions = { "xuan-workspace-search": "0.1.2", "xuan-usage": "0.1.2", "xuan-polish": "0.1.2" };
const userScripts = {
  "xuan-workspace-search": "workspace-search.user.js",
  "xuan-usage": "usage-header.user.js",
  "xuan-polish": "polish-composer.user.js"
};

test("all Xuan plugin manifests use the official extension fields", () => {
  for (const name of pluginNames) {
    const manifest = JSON.parse(fs.readFileSync(path.join(root, name, ".codex-plugin", "plugin.json"), "utf8"));
    assert.equal(manifest.name, name);
    assert.equal(manifest.version, pluginVersions[name]);
    assert.equal(manifest.skills, "./skills/");
    assert.equal(manifest.mcpServers, "./.mcp.json");
    assert.ok(Array.isArray(manifest.interface.capabilities));
  }
});

test("marketplace contains exactly the three plugin packages", () => {
  const marketplace = JSON.parse(fs.readFileSync(marketplacePath, "utf8"));
  assert.equal(marketplace.name, "xuan-curated");
  assert.equal(marketplace.interface.displayName, "Xuan Plugins");
  assert.deepEqual(marketplace.plugins.map((plugin) => plugin.name), pluginNames);
  for (const plugin of marketplace.plugins) {
    assert.deepEqual(plugin.source, {
      source: "local",
      path: `./plugins/${plugin.name}`
    });
    assert.equal(plugin.policy.installation, "AVAILABLE");
    assert.equal(plugin.policy.authentication, "ON_INSTALL");
    assert.equal(typeof plugin.category, "string");
  }
});

test("each plugin package is self contained", () => {
  for (const name of pluginNames) {
    const server = fs.readFileSync(path.join(root, name, "server.mjs"), "utf8");
    assert.match(server, /\.\/lib\/mcp-server\.mjs/);
    assert.doesNotMatch(server, /\.\.\/shared/);
    assert.ok(fs.existsSync(path.join(root, name, "lib", "mcp-server.mjs")));
    assert.ok(fs.existsSync(path.join(root, name, "scripts", userScripts[name])));
  }
});
