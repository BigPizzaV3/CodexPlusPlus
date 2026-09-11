import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const root = path.join(import.meta.dirname);
const pluginNames = ["xuan-workspace-search", "xuan-usage", "xuan-polish"];

test("all Xuan plugin manifests use the official extension fields", () => {
  for (const name of pluginNames) {
    const manifest = JSON.parse(fs.readFileSync(path.join(root, name, ".codex-plugin", "plugin.json"), "utf8"));
    assert.equal(manifest.name, name);
    assert.match(manifest.version, /^0\.1\.0$/);
    assert.equal(manifest.skills, "./skills/");
    assert.equal(manifest.mcpServers, "./.mcp.json");
    assert.ok(Array.isArray(manifest.interface.capabilities));
  }
});

test("marketplace contains exactly the three plugin packages", () => {
  const marketplace = JSON.parse(fs.readFileSync(path.join(root, "marketplace.json"), "utf8"));
  assert.equal(marketplace.name, "xuan-curated");
  assert.deepEqual(marketplace.plugins.map((plugin) => plugin.name), pluginNames);
});
