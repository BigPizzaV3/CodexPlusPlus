import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { readFile } from "node:fs/promises";

import {
  defaultDreamSkinTheme,
  isDreamSkinDraftDirty,
  normalizeDreamSkinTheme,
  resolveDreamSkinStylePreset,
} from "./dream-skin.ts";

describe("dream skin theme helpers", () => {
  it("uses the Codex-Dream-Skin theme.json defaults", () => {
    const theme = defaultDreamSkinTheme();

    assert.equal(theme.schemaVersion, 1);
    assert.equal(theme.name, "Dream Skin");
    assert.equal(theme.stylePreset, undefined);
    assert.equal(theme.brandSubtitle, "CODEX DREAM SKIN");
    assert.equal(theme.colors!.accent, "#E25563");
  });

  it("restores an invalid color without dropping valid text", () => {
    const base = defaultDreamSkinTheme();
    const theme = normalizeDreamSkinTheme({
      ...base,
      name: "My Theme",
      colors: { ...base.colors!, accent: "javascript:bad" },
    });

    assert.equal(theme.name, "My Theme");
    assert.equal(theme.colors!.accent, "#E25563");
  });

  it("maps legacy market themes to their layout presets", () => {
    assert.equal(resolveDreamSkinStylePreset("preset-cyber-neon", undefined), "cyber-neon");
    assert.equal(resolveDreamSkinStylePreset("codex-snow-skin", "dream-original"), "codex-snow");
    assert.equal(resolveDreamSkinStylePreset("custom-theme", undefined), "dream-original");
  });

  it("runs the unmodified target renderers instead of the rewritten theme packages", async () => {
    const assets = await readFile(new URL("../../../crates/codex-plus-core/src/assets.rs", import.meta.url), "utf8");
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(assets, /upstream\/dream-skin/);
    assert.match(assets, /upstream\/cidala-tiger/);
    assert.match(assets, /upstream\/snow-skin/);
    assert.match(assets, /upstream\/glass-vision/);
    assert.doesNotMatch(assets, /assets\/inject\/themes\//);
    assert.match(assets, /__DREAM_SKIN_CSS_JSON__/);
    assert.match(assets, /__GLASS_VISION_CSS_JSON__/);
    assert.match(renderer, /__CODEX_PLUS_EXTERNAL_DREAM_SKIN_RUNTIME__/);
    assert.match(renderer, /__CODEX_PLUS_CLEAR_DREAM_SKIN__/);
  });

  it("preserves target-only theme fields without rewriting them", () => {
    const theme = normalizeDreamSkinTheme({
      schemaVersion: 1,
      id: "target-theme",
      name: "Target Theme",
      brandSubtitle: "TARGET",
      tagline: "Target tagline",
      projectPrefix: "project · ",
      projectLabel: "Select project",
      statusText: "ONLINE",
      quote: "EXACT",
      appearance: "dark",
      art: { focusX: 0.72, focusY: 0.45, safeArea: "left", taskMode: "ambient" },
      palette: { accent: "#123456", custom: "keep" },
      promoTitle: "Sponsor",
      promoSub: "sponsor.example",
      promoUrl: "https://sponsor.example",
      customTargetField: { nested: true },
    });

    assert.equal(theme.colors, undefined);
    assert.equal(theme.stylePreset, undefined);
    assert.deepEqual(theme.art, { focusX: 0.72, focusY: 0.45, safeArea: "left", taskMode: "ambient" });
    assert.deepEqual(theme.palette, { accent: "#123456", custom: "keep" });
    assert.deepEqual(theme.customTargetField, { nested: true });
  });

  it("detects text, color, and image draft changes", () => {
    const draft = {
      config: defaultDreamSkinTheme(),
      imagePath: "current.png",
      builtin: false,
    };

    assert.equal(isDreamSkinDraftDirty(draft, draft), false);
    assert.equal(isDreamSkinDraftDirty(draft, {
      ...draft,
      config: { ...draft.config, name: "Changed" },
    }), true);
    assert.equal(isDreamSkinDraftDirty(draft, {
      ...draft,
      config: { ...draft.config, stylePreset: "cyber-neon" },
    }), true);
    assert.equal(isDreamSkinDraftDirty(draft, {
      ...draft,
      config: {
        ...draft.config,
        colors: { ...draft.config.colors!, accent: "#112233" },
      },
    }), true);
    assert.equal(isDreamSkinDraftDirty(draft, { ...draft, imagePath: "other.png" }), true);
  });

  it("visibly credits the source project and exposes complete controls", async () => {
    const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");

    for (const text of [
      "Fei-Away/Codex-Dream-Skin",
      "MIT License",
      "第三方图片",
      "主题名称",
      "品牌副标题",
      "主题标语",
      "项目前缀",
      "项目按钮文字",
      "状态文字",
      "引用文字",
      "应用皮肤",
      "恢复 Codex 外观",
      "实机验证",
      "保存截图",
    ]) {
      assert.match(source, new RegExp(text));
    }
    assert.doesNotMatch(source, /暂停皮肤|继续皮肤|pauseDreamSkin/);
  });

  it("keeps the image preview inside its grid column in a compact window", async () => {
    const styles = await readFile(new URL("./styles.css", import.meta.url), "utf8");
    const previewRule = styles.match(/\.dream-skin-preview\s*\{([^}]*)\}/)?.[1] ?? "";

    assert.match(previewRule, /width:\s*100%/);
    assert.doesNotMatch(previewRule, /min-height:\s*220px/);
  });

  it("uses the current launcher debug port for live skin operations", async () => {
    const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");

    assert.match(source, /overview\?\.latest_launch\?\.debug_port\s*\?\?/);
  });

  it("keeps theme draft separate from backend settings until explicit activation", async () => {
    const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");

    assert.match(source, /selectedDreamSkinTheme/);
    assert.match(source, /dreamSkinThemeDraft/);
    assert.match(source, /activate_dream_skin_theme/);
    assert.match(source, /DreamSkinUnsavedDialog/);
    assert.match(source, /pendingDreamSkinRestart/);
    assert.match(source, /重启并应用/);
    assert.doesNotMatch(source, /当前 Codex 无法实时切换完整主题，需要重启 Codex\+\+。是否立即重启/);
  });

  it("renders responsive three-column theme grids with platform guidance", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const css = await readFile(new URL("./styles.css", import.meta.url), "utf8");

    assert.match(app, /dream-skin-theme-library/);
    assert.match(app, /Windows 原版样式固定/);
    assert.match(css, /\.dream-skin-market-grid\s*\{[^}]*grid-template-columns:\s*repeat\(3,/s);
    assert.match(css, /\.dream-skin-theme-list\s*\{[^}]*grid-template-columns:\s*repeat\(3,/s);
    assert.match(css, /@media \(max-width:\s*760px\)[\s\S]*\.dream-skin-theme-list\s*\{[^}]*grid-template-columns:\s*1fr/s);
  });

  it("keeps advanced theme editing collapsed outside the theme switcher", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const customizerStart = app.indexOf('<details className="dream-skin-customizer">');
    const customizerEnd = app.indexOf("</details>", customizerStart);
    const libraryStart = app.indexOf('<section className="dream-skin-theme-library">');
    const libraryEnd = app.indexOf("</section>", libraryStart);

    assert.ok(customizerStart >= 0);
    assert.ok(customizerEnd > customizerStart);
    assert.ok(libraryStart >= 0);
    assert.ok(libraryEnd > libraryStart);

    const customizer = app.slice(customizerStart, customizerEnd);
    const library = app.slice(libraryStart, libraryEnd);

    assert.doesNotMatch(app.slice(customizerStart, customizerStart + 80), /\sopen(?:=|\s|>)/);
    assert.match(library, /应用主题/);
    assert.doesNotMatch(library, /t\("(?:从图片创建|保存主题|恢复 Dream Skin 默认主题)"\)/);
    assert.match(customizer, /t\("从图片创建"\)/);
    assert.match(customizer, /t\("保存主题"\)/);
    assert.match(customizer, /t\("恢复 Dream Skin 默认主题"\)/);
  });

  it("separates the remote marketplace from local theme editing", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const css = await readFile(new URL("./styles.css", import.meta.url), "utf8");

    assert.match(app, /refresh_dream_skin_market/);
    assert.match(app, /install_dream_skin_market_theme/);
    assert.match(app, /主题市场/);
    assert.match(app, /投稿主题/);
    assert.match(app, /onInstalled=\{\(\) => setThemeView\("local"\)\}/);
    assert.match(css, /\.dream-skin-market-grid\s*\{[^}]*grid-template-columns:\s*repeat\(3,/s);
    assert.match(css, /\.dream-skin-market-preview\s*\{[^}]*aspect-ratio:\s*16 \/ 9/s);
  });

  it("allows the webview to load managed theme images only", async () => {
    const raw = await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8");
    const config = JSON.parse(raw);
    const assetProtocol = config.app?.security?.assetProtocol;

    assert.equal(assetProtocol?.enable, true);
    assert.deepEqual(assetProtocol?.scope, ["$HOME/.codex-session-delete/dream-skin/**"]);
  });
});
