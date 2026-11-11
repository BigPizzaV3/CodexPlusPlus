import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { it } from "node:test";

const asset = (name: string) => readFile(new URL(`../../../assets/inject/${name}`, import.meta.url), "utf8");

for (const platform of ["windows"]) {
  it(`${platform}: removes stylesheet :has() dependencies from base and community themes`, async () => {
    const renderer = await asset(`upstream/dream-skin/${platform}/renderer-inject.js`);
    const css = [
      await asset(`upstream/dream-skin/${platform}/dream-skin.css`),
      await asset("dream-skin-native-shell-compat.css"),
      await asset("dream-skin-community-compat.css"),
    ].join("\n");
    const start = renderer.indexOf("  const CSS_PREDICATES =");
    const end = renderer.indexOf("  const ART =", start);
    assert.ok(start >= 0 && end > start);
    const compile = new Function("cssText", `${renderer.slice(start, end)}\nreturn cssText;`);
    const compiled = compile(css);
    // 不仅检查新规则：防止添加样式后遗漏预计算表，重新引入常驻依赖。
    assert.doesNotMatch(compiled.replace(/\/\*[\s\S]*?\*\//g, ""), /:has\(/);
  });
}

it("keeps the popup parent rule installed only while the manager page is open", async () => {
  const styleSource = await asset("renderer-inject/10-style.js");
  const pageSource = await asset("renderer-inject/40-backend-settings.js");
  const extract = (source: string, name: string) => {
    const match = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"));
    assert.ok(match, name);
    return match[0];
  };
  let open = false;
  let writes = 0;
  type StyleNode = { id: string; textContent: string; setAttribute(): void; remove(): void };
  const nodes = new Map<string, StyleNode>();
  const attributes = new Map<string, string>();
  const document = {
    getElementById: (id: string) => nodes.get(id),
    querySelector: (selector: string) => selector === ".codex-plus-page-overlay" && open ? {} : null,
    createElement: (): StyleNode => ({
      id: "", textContent: "", setAttribute() {},
      remove() { nodes.delete(this.id); },
    }),
    documentElement: {
      appendChild(node: StyleNode) { nodes.set(node.id, node); writes++; },
      setAttribute: (name: string, value: string) => attributes.set(name, value),
      removeAttribute: (name: string) => attributes.delete(name),
      style: { setProperty() {}, removeProperty() {} },
    },
  };
  const sync = new Function("document", `
    const styleId = "codex-delete-style";
    const codexPlusIsWindowsPlatform = true;
    const codexPlusPageClass = "codex-plus-page-overlay";
    const codexPlusRailSelector = "nav[data-app-navigation-rail]";
    const codexPlusRailDestinationSelector = "[data-sidebar-destination]";
    ${extract(styleSource, "syncCodexPlusPagePopupStyle")}
    ${extract(pageSource, "syncCodexPlusRailNativeSelection")}
    return syncCodexPlusRailNativeSelection;
  `)(document);
  sync();
  assert.equal(nodes.size, 0);
  open = true;
  sync();
  sync();
  assert.equal(writes, 1, "repeated scans must not recreate the stylesheet");
  assert.equal(attributes.has("data-codex-plus-page-open"), true);
  assert.match(nodes.get("codex-delete-style-page-popups")!.textContent, /div:has\(/);
  open = false;
  sync();
  assert.equal(nodes.size, 0, "closing removes the CSS rule, not just its root condition");
  assert.equal(attributes.has("data-codex-plus-page-open"), false);
  open = true;
  sync();
  assert.equal(nodes.size, 1, "reopening restores popup stacking");
});

it("keeps the original popup stylesheet behavior on non-Windows platforms", async () => {
  const source = await asset("renderer-inject/10-style.js");
  const install = source.match(/^  function installStyle\([^]*?^  \}/m)![0];
  const names = [...new Set(["styleId", "codexDeleteStyleVersion",
    ...Array.from(install.matchAll(/\$\{([A-Za-z_$][A-Za-z0-9_$]*)/g), m => m[1])])];
  for (const windows of [false, true]) {
    let text = "";
    const document = {
      getElementById: () => null,
      createElement: () => ({id: "", dataset: {}, textContent: ""}),
      documentElement: {appendChild: (style: {textContent: string}) => {text = style.textContent;}},
    };
    new Function("document", ...names, `${install}\ninstallStyle();`)(document,
      ...names.map(name => name === "codexPlusIsWindowsPlatform" ? windows : name === "selectors" ? {} : name));
    assert.equal(text.includes("div:has("), !windows,
      "only Windows should stop installing the persistent parent rule");
  }
});
