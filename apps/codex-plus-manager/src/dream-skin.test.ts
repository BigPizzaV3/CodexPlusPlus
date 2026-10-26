import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { readFile } from "node:fs/promises";
import ts from "typescript";

import {
  defaultDreamSkinTheme,
  isDreamSkinDraftDirty,
  normalizeDreamSkinTheme,
  resolveDreamSkinStylePreset,
  runExclusiveDreamSkinInstall,
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

  it("locks theme installation synchronously and releases the lock after completion", async () => {
    const lock = { current: false };
    const pending: boolean[] = [];
    let finish!: (value: boolean) => void;
    let runs = 0;
    const task = () => {
      runs += 1;
      return new Promise<boolean>((resolve) => { finish = resolve; });
    };

    const first = runExclusiveDreamSkinInstall(lock, (value) => pending.push(value), task);
    const duplicate = await runExclusiveDreamSkinInstall(lock, (value) => pending.push(value), task);

    assert.equal(duplicate, null);
    assert.equal(runs, 1);
    assert.equal(lock.current, true);
    assert.deepEqual(pending, [true]);

    finish(true);
    assert.equal(await first, true);
    assert.equal(lock.current, false);
    assert.deepEqual(pending, [true, false]);
  });

  it("releases the theme installation lock when the request throws", async () => {
    const lock = { current: false };
    const pending: boolean[] = [];

    await assert.rejects(
      runExclusiveDreamSkinInstall(lock, (value) => pending.push(value), async () => {
        throw new Error("network failed");
      }),
      /network failed/,
    );

    assert.equal(lock.current, false);
    assert.deepEqual(pending, [true, false]);
  });

  it("keeps the real task locked when the initiating view unmounts and remounts", async () => {
    const appLock = { current: false };
    let runs = 0;
    let finish!: (value: boolean) => void;
    const task = () => {
      runs += 1;
      return new Promise<boolean>((resolve) => { finish = resolve; });
    };
    const mountView = () => () => runExclusiveDreamSkinInstall(appLock, () => {}, task);

    const firstView = mountView();
    const activeTask = firstView();
    const remountedView = mountView();
    const duplicate = await remountedView();

    assert.equal(duplicate, null);
    assert.equal(runs, 1);
    assert.equal(appLock.current, true);

    finish(true);
    assert.equal(await activeTask, true);
    assert.equal(appLock.current, false);
  });

  it("keeps global indeterminate install state across theme views and every install entry", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const css = await readFile(new URL("./styles.css", import.meta.url), "utf8");

    assert.match(app, /const \[dreamSkinInstallOperation, setDreamSkinInstallOperation\] = useState<DreamSkinInstallOperation \| null>\(null\)/);
    assert.match(app, /const dreamSkinInstallLockRef = useRef\(false\)/);
    assert.match(app, /runExclusiveDreamSkinInstall\([\s\S]*dreamSkinInstallLockRef,[\s\S]*setDreamSkinInstallOperation/);
    for (const source of ["market", "community", "pending-community", "import"]) {
      assert.match(app, new RegExp(`source: "${source}"`));
    }
    assert.doesNotMatch(app, /const installLock = useRef\(false\)/);
    assert.match(app, /const installing = installOperation\?\.source === "market"/);
    assert.match(app, /const installing = installOperation\?\.source === "community"/);
    assert.match(app, /disabled=\{installBlocked\}/);
    assert.match(app, /disabled=\{installBlocked \|\| !theme\.applyCompatible\}/);
    assert.match(app, /aria-busy=\{installing\}/);
    assert.match(app, /installOperation=\{dreamSkinInstallOperation\}/);
    assert.match(app, /DreamSkinInstallProgress label=\{dreamSkinInstallOperationLabel\(installOperation\)\}/);
    assert.match(app, /installPending=\{dreamSkinInstallOperation\?\.source === "pending-community"\}/);
    assert.match(app, /disabled=\{Boolean\(installOperation\)\} variant="outline" onClick=\{\(\) => void actions\.importDreamSkinThemePackage\(\)\}/);
    assert.match(app, /className="dream-skin-install-progress-bar" role="progressbar"/);
    assert.doesNotMatch(app, /dream-skin-install-progress-bar[^>]*aria-valuenow/);
    assert.match(css, /\.dream-skin-install-progress-bar > div\s*\{[^}]*animation:\s*task-progress-indeterminate/s);
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
      companion: {
        dataUrl: "data:image/webp;base64,UklGRg==",
        width: 96,
        side: "right",
      },
      customTargetField: { nested: true },
    });

    assert.equal(theme.colors, undefined);
    assert.equal(theme.stylePreset, undefined);
    assert.deepEqual(theme.art, { focusX: 0.72, focusY: 0.45, safeArea: "left", taskMode: "ambient" });
    assert.deepEqual(theme.palette, { accent: "#123456", custom: "keep" });
    assert.deepEqual(theme.companion, {
      dataUrl: "data:image/webp;base64,UklGRg==",
      width: 96,
      side: "right",
    });
    assert.deepEqual(theme.customTargetField, { nested: true });
  });

  it("renders an optional image companion beside the visible composer", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /codex-dream-skin-companion/);
    assert.match(renderer, /theme\.companion/);
    assert.match(renderer, /\.composer-footer/);
    assert.match(renderer, /\.composer-surface-chrome/);
    assert.match(renderer, /data:image\/(?:png|jpeg|webp|gif);base64/);
    assert.match(renderer, /removeDreamSkinCompanion/);
    assert.match(renderer, /ensureDreamSkinCompanion\(\s*window\.__CODEX_PLUS_DREAM_SKIN_THEME__/);
  });

  it("aligns tall companion images by rendered height with a wider vertical offset range", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");

    assert.match(renderer, /companion\.naturalWidth/);
    assert.match(renderer, /companion\.naturalHeight/);
    assert.match(renderer, /composer\.rect\.bottom - renderedHeight \+ config\.offsetY/);
    assert.match(renderer, /window\.innerHeight - renderedHeight - edge/);
    assert.match(renderer, /const offsetY = Math\.max\(-160, Math\.min\(Number\(companion\.offsetY\) \|\| 0, 160\)\)/);
    assert.match(app, /min=\{-160\}/);
    assert.match(app, /max=\{160\}/);
    assert.match(app, /Math\.max\(-160, Math\.min\(160, Number\(event\.currentTarget\.value\) \|\| 0\)\)/);
  });

  it("uses the current shared selector contract without requiring a visible sidebar", async () => {
    const renderer = await readFile(
      new URL("../../../assets/inject/upstream/dream-skin/windows/renderer-inject.js", import.meta.url),
      "utf8",
    );
    const compatibility = await readFile(
      new URL("../../../assets/inject/renderer-inject.js", import.meta.url),
      "utf8",
    );
    const assets = await readFile(
      new URL("../../../crates/codex-plus-core/src/assets.rs", import.meta.url),
      "utf8",
    );

    assert.match(renderer, /codex-dream-skin-selectors\/1/);
    assert.match(renderer, /data-app-shell-main-surface/);
    assert.match(renderer, /_MainContentSurface_/);
    assert.match(renderer, /data-ds-part/);
    assert.match(renderer, /MutationObserver/);
    assert.match(compatibility, /main\[class\*="_MainContentSurface_"\]/);
    assert.match(compatibility, /shellMain\.classList\.add\("main-surface"\)/);
    assert.match(compatibility, /data-codex-plus-dream-skin-main-surface/);
    assert.match(compatibility, /clearDreamSkinMainSurfaceCompatibility\(\)/);
    assert.match(assets, /DREAM_SKIN_RENDERER_REVISION: &str = "27-dream-skin-1.5.20-wallpaper-shell"/);
    assert.match(assets, /DREAM_SKIN_COMMUNITY_COMPAT_CSS/);
    assert.match(assets, /format!\("\{compiled\}\\n\{DREAM_SKIN_COMMUNITY_COMPAT_CSS\}"\)/);
  });

  it("covers the current shell, composer, message, and top-fade contracts", async () => {
    const renderer = await readFile(
      new URL("../../../assets/inject/upstream/dream-skin/windows/renderer-inject.js", import.meta.url),
      "utf8",
    );
    const css = await readFile(
      new URL("../../../assets/inject/upstream/dream-skin/windows/dream-skin.css", import.meta.url),
      "utf8",
    );

    assert.match(renderer, /_ComposerLayoutRoot_/);
    assert.match(renderer, /data-local-conversation-final-assistant/);
    assert.match(renderer, /data-local-conversation-user-anchor/);
    assert.match(css, /_MainContentTopFade_/);
    assert.match(css, /_ComposerLayoutBody_/);
    assert.match(css, /data-markdown-table="true"/);
    assert.match(css, /_Toolbar_.*> \*/s);
    assert.match(css, /data-composer-home-utility-bar-position="above"[\s\S]*box-sizing:\s*border-box/);
    assert.match(css, /_StickyActionBar_/);
    assert.match(css, /data-ds-part="home"[\s\S]*overflow:\s*hidden\s*!important/);
  });

  it("exposes companion image controls in the theme editor", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");

    assert.match(app, /dream-skin-companion-controls/);
    assert.match(app, /FileReader/);
    assert.match(app, /companion\.dataUrl/);
    assert.match(app, /companion\?\.offsetX/);
    assert.match(app, /companion\?\.offsetY/);
    assert.match(app, /companionEnabled/);
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
      "应用所选主题",
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

  it("restores the original appearance as pending without reloading or restarting Codex", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const commands = await readFile(new URL("../src-tauri/src/commands.rs", import.meta.url), "utf8");
    const restoreStart = app.indexOf("const restoreDreamSkin = async () =>");
    const restoreEnd = app.indexOf("const verifyDreamSkin", restoreStart);
    const restoreHandler = app.slice(restoreStart, restoreEnd);
    const commandStart = commands.indexOf("pub async fn restore_dream_skin");
    const commandEnd = commands.indexOf("pub fn reset_dream_skin_theme", commandStart);
    const restoreCommand = commands.slice(commandStart, commandEnd);

    assert.match(restoreHandler, /Codex 原始外观/);
    assert.match(restoreHandler, /setPendingDreamSkinRestart/);
    assert.doesNotMatch(restoreHandler, /window\.confirm|await restart\(\)/);
    assert.doesNotMatch(restoreCommand, /reload_dream_skin_live/);
    assert.match(restoreCommand, /pending_restart\(false, false\)/);
  });

  it("renders responsive three-column theme grids with platform guidance", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const css = await readFile(new URL("./styles.css", import.meta.url), "utf8");

    assert.match(app, /dream-skin-theme-library/);
    assert.match(app, /Windows 使用亮暗模式、图片取色和可选强调色/);
    assert.match(css, /\.dream-skin-market-grid\s*\{[^}]*grid-template-columns:\s*repeat\(3,/s);
    assert.match(css, /\.dream-skin-theme-list\s*\{[^}]*grid-template-columns:\s*repeat\(3,/s);
    assert.match(css, /@media \(max-width:\s*760px\)[\s\S]*\.dream-skin-theme-list\s*\{[^}]*grid-template-columns:\s*1fr/s);
  });

  it("keeps advanced editing collapsed with one explicit theme activation action", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const source = ts.createSourceFile("App.tsx", app, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    const screen = source.statements.find((node): node is ts.FunctionDeclaration =>
      ts.isFunctionDeclaration(node) && node.name?.text === "DreamSkinScreen",
    );
    assert.ok(screen);

    // 用 JSX 结构定位完整区域，避免新增嵌套 details 后截在第一个结束标签。
    const elements = new Map<string, ts.JsxElement>();
    const visit = (node: ts.Node): void => {
      if (ts.isJsxElement(node)) {
        const className = node.openingElement.attributes.properties.find((attribute) =>
          ts.isJsxAttribute(attribute) && attribute.name.getText(source) === "className",
        );
        if (className && ts.isJsxAttribute(className) && className.initializer && ts.isStringLiteral(className.initializer)) {
          for (const name of className.initializer.text.split(/\s+/)) elements.set(name, node);
        }
      }
      ts.forEachChild(node, visit);
    };
    visit(screen);
    const customizerNode = elements.get("dream-skin-customizer");
    const libraryNode = elements.get("dream-skin-theme-library");
    const overviewNode = elements.get("dream-skin-overview");
    assert.ok(customizerNode);
    assert.ok(libraryNode);
    assert.ok(overviewNode);
    const customizer = customizerNode.getText(source);
    const library = libraryNode.getText(source);
    const overview = overviewNode.getText(source);

    assert.doesNotMatch(customizerNode.openingElement.getText(source), /\sopen(?:=|\s|>)/);
    assert.match(overview, /应用所选主题/);
    assert.match(overview, /actions\.activateDreamSkinTheme\(\)/);
    assert.equal((screen.getText(source).match(/actions\.activateDreamSkinTheme\(\)/g) ?? []).length, 1);
    assert.match(library, /actions\.selectDreamSkinTheme\(item\)/);
    assert.doesNotMatch(library, /actions\.activateDreamSkinTheme\(\)/);
    assert.doesNotMatch(library, /t\("(?:从图片创建|保存主题|恢复 Dream Skin 默认主题)"\)/);
    assert.match(customizer, /t\("从图片创建"\)/);
    assert.match(customizer, /t\("保存主题"\)/);
    assert.match(customizer, /t\("恢复 Dream Skin 默认主题"\)/);
    assert.match(customizer, /t\("恢复 Codex 默认配色"\)/);
  });

  it("exposes only effective Windows appearance and accent controls", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");

    assert.match(app, /dream-skin-windows-theme-controls/);
    assert.match(app, /自动/);
    assert.match(app, /亮色/);
    assert.match(app, /暗色/);
    assert.match(app, /跟随图片配色/);
    assert.match(app, /isWindowsPlatform \? \([\s\S]*dream-skin-windows-theme-controls[\s\S]*dream-skin-colors/);
    assert.match(app, /if \(isWindowsPlatform\) \{[\s\S]*delete config\.colors;[\s\S]*delete config\.palette;/);
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

  it("shows a loading spinner instead of 0 while theme lists are loading", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const css = await readFile(new URL("./styles.css", import.meta.url), "utf8");

    assert.match(app, /dreamSkinLibraryLoading/);
    assert.match(app, /dreamSkinMarketLoading/);
    assert.match(app, /dreamSkinCommunityLoading/);
    // tab 计数在加载中渲染转圈图标，而不是渲染 0
    assert.match(app, /libraryPending \? <Loader2[^/]*className="h-3 w-3 animate-spin" \/> : libraryCount/);
    assert.match(app, /communityPending \? <Loader2[^/]*className="h-3 w-3 animate-spin" \/> : communityCount/);
    assert.match(app, /marketPending \? <Loader2[^/]*className="h-3 w-3 animate-spin" \/> : marketCount/);
    assert.doesNotMatch(app, /<span>\{library\?\.themes\.length \?\? 0\}<\/span>/);
    assert.doesNotMatch(app, /<span>\{community\?\.items\.length \?\? 0\}<\/span>/);
    assert.doesNotMatch(app, /<span>\{market\?\.themes\.length \?\? 0\}<\/span>/);
    // 各区块的加载文案也带转圈
    assert.match(app, /className="dream-skin-loading" role="status"/);
    assert.match(css, /\.dream-skin-view-tabs button > span\.is-loading/);
    assert.match(css, /\.dream-skin-loading\s*\{/);
  });

  it("allows the webview to load managed theme images only", async () => {
    const raw = await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8");
    const config = JSON.parse(raw);
    const assetProtocol = config.app?.security?.assetProtocol;

    assert.equal(assetProtocol?.enable, true);
    assert.deepEqual(assetProtocol?.scope, ["$HOME/.codex-session-delete/dream-skin/**"]);
  });
});
