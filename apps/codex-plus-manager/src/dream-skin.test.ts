import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { readFile } from "node:fs/promises";

import {
  defaultDreamSkinTheme,
  normalizeDreamSkinTheme,
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

  it("keeps only the rescue entry and the dreamskin:// confirmation in the manager", async () => {
    // 皮肤界面已移到 Codex 里（Codex++ → 设置 → 皮肤）；管理器不再列主题、激活主题。
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");

    assert.doesNotMatch(app, /"list_dream_skin_themes"/);
    assert.doesNotMatch(app, /"activate_dream_skin_theme"/);
    assert.match(app, /"restore_dream_skin"/);
    assert.match(app, /"activate_installed_dream_skin_theme"/);
    assert.match(app, /"confirm_pending_dream_skin_community"/);
    assert.match(app, /皮肤已移到 Codex 里：打开 Codex 左侧的 Codex\+\+ → 设置 → 皮肤/);
    // 网页一键换肤确认框的安装中状态：进度条不带具体数值（不确定进度）。
    assert.match(app, /installPending=\{dreamSkinCommunityInstalling\}/);
    assert.match(app, /className="dream-skin-install-progress-bar" role="progressbar"/);
    assert.doesNotMatch(app, /dream-skin-install-progress-bar[^>]*aria-valuenow/);
    const css = await readFile(new URL("./styles.css", import.meta.url), "utf8");
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

    assert.match(renderer, /companion\.naturalWidth/);
    assert.match(renderer, /companion\.naturalHeight/);
    assert.match(renderer, /composer\.rect\.bottom - renderedHeight \+ config\.offsetY/);
    assert.match(renderer, /window\.innerHeight - renderedHeight - edge/);
    assert.match(renderer, /const offsetY = Math\.max\(-160, Math\.min\(Number\(companion\.offsetY\) \|\| 0, 160\)\)/);
    // 编辑界面在 Codex 的皮肤页里（96-dream-skin-page.js），垂直偏移同样限制在 ±160。
    assert.match(renderer, /\{ width: \[48, 160\], offsetX: \[-48, 48\], offsetY: \[-160, 160\] \}/);
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
    // 适配层先按属性找主内容区（与上游 shell-main 契约一致），类名只做兜底（#2893）。
    const ensureMain = compatibility.match(/function ensureDreamSkinMainSurface\(\) \{[^]*?\n  \}/)?.[0] ?? "";
    assert.ok(
      ensureMain.indexOf("main[data-app-shell-main-surface]") > -1
        && ensureMain.indexOf("main[data-app-shell-main-surface]") < ensureMain.indexOf("_MainContentSurface_"),
      "先认 data-app-shell-main-surface，再认类名",
    );
    assert.match(compatibility, /shellMain\.classList\.add\("main-surface"\)/);
    assert.match(compatibility, /data-codex-plus-dream-skin-main-surface/);
    assert.match(compatibility, /clearDreamSkinMainSurfaceCompatibility\(\)/);
    assert.match(assets, /DREAM_SKIN_RENDERER_REVISION: &str = "28-dream-skin-local-video"/);
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

  it("exposes companion image controls in the Codex skin page editor", async () => {
    // 主题编辑在 Codex 的 Codex++ 设置页「皮肤」里，不在管理器。
    const page = await readFile(new URL("../../../assets/inject/renderer-inject/96-dream-skin-page.js", import.meta.url), "utf8");

    assert.match(page, /data-skin-file="companion"/);
    assert.match(page, /FileReader/);
    assert.match(page, /companion\?\.dataUrl/);
    assert.match(page, /\["width", "offsetX", "offsetY"\]/);
    assert.match(page, /companion-toggle/);
  });

  it("visibly credits the source project and exposes complete controls", async () => {
    const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const page = await readFile(new URL("../../../assets/inject/renderer-inject/96-dream-skin-page.js", import.meta.url), "utf8");

    // 管理器的皮肤急救页保留来源说明。
    for (const text of ["Fei-Away/Codex-Dream-Skin", "MIT License", "第三方图片"]) {
      assert.match(source, new RegExp(text));
    }
    // 主题编辑字段与恢复入口在 Codex 的皮肤页里。
    for (const text of ["主题名称", "品牌副标题", "主题标语", "项目前缀", "项目按钮文字", "状态文字", "引用文字", "恢复 Codex 原始外观"]) {
      assert.match(page, new RegExp(text));
    }
    assert.doesNotMatch(source, /暂停皮肤|继续皮肤|pauseDreamSkin/);
  });

  it("uses the current launcher debug port for live skin operations", async () => {
    const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");

    assert.match(source, /overview\?\.latest_launch\?\.debug_port\s*\?\?/);
  });

  it("restores the original appearance as pending without reloading or restarting Codex", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const commands = await readFile(new URL("../src-tauri/src/commands.rs", import.meta.url), "utf8");
    const restoreStart = app.indexOf("const restoreDreamSkin = async () =>");
    const restoreEnd = app.indexOf("const actions = useMemo", restoreStart);
    const restoreHandler = app.slice(restoreStart, restoreEnd);
    const commandStart = commands.indexOf("pub async fn restore_dream_skin");
    const commandEnd = commands.indexOf("pub fn reset_dream_skin_theme", commandStart);
    const restoreCommand = commands.slice(commandStart, commandEnd);

    assert.ok(restoreStart > -1 && restoreEnd > restoreStart);
    assert.match(restoreHandler, /call<DreamSkinRuntimeResult>\("restore_dream_skin", dreamSkinRequest\(\)\)/);
    assert.match(restoreHandler, /showResultNotice\(/);
    // 恢复只记为待应用；急救页给出「重启 Codex」按钮，由用户决定何时重启。
    assert.match(restoreHandler, /setDreamSkinRestartPending\(true\)/);
    assert.doesNotMatch(restoreHandler, /window\.confirm|await restart\(\)/);
    assert.match(app, /restartPending=\{dreamSkinRestartPending\}/);
    assert.doesNotMatch(restoreCommand, /reload_dream_skin_live/);
    assert.match(restoreCommand, /pending_restart\(false, false\)/);
  });

  it("exposes only effective Windows appearance and accent controls in the Codex skin page", async () => {
    const page = await readFile(new URL("../../../assets/inject/renderer-inject/96-dream-skin-page.js", import.meta.url), "utf8");

    assert.match(page, /codexPlusDreamSkinPlatform === "windows"/);
    assert.match(page, /\[\["auto", "自动"\], \["light", "亮色"\], \["dark", "暗色"\]\]/);
    assert.match(page, /跟随图片配色/);
  });

  it("keeps the remote marketplace and community catalogs in the Codex skin page", async () => {
    const page = await readFile(new URL("../../../assets/inject/renderer-inject/96-dream-skin-page.js", import.meta.url), "utf8");

    assert.match(page, /"installMarket"/);
    assert.match(page, /"installCommunity"/);
    assert.match(page, /主题市场/);
    assert.match(page, /DreamSkin 社区/);
  });

  it("keeps the skin bridge private to the built-in page and never sends local paths", async () => {
    const page = await readFile(new URL("../../../assets/inject/renderer-inject/96-dream-skin-page.js", import.meta.url), "utf8");
    const extensionApi = await readFile(new URL("../../../assets/inject/renderer-inject/91-extension-api.js", import.meta.url), "utf8");

    // 写操作不进拓展白名单：第三方拓展拿不到改皮肤、装主题、重启 Codex 的口子。
    assert.doesNotMatch(extensionApi, /"\/dream-skin"/);
    assert.deepEqual(page.match(/postJson\("[^"]*"/g), ['postJson("/dream-skin"']);
    // 文件以内容上传，由管理器落到自己的目录；页面不传、也不收本机路径。
    assert.doesNotMatch(page, /path/i);
    assert.match(page, /readAsDataURL/);
  });

  it("no longer opens the asset protocol now that skin images are shown inside Codex", async () => {
    const raw = await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8");
    const config = JSON.parse(raw);
    const cargo = await readFile(new URL("../src-tauri/Cargo.toml", import.meta.url), "utf8");

    assert.equal(config.app?.security?.assetProtocol, undefined);
    assert.doesNotMatch(cargo, /protocol-asset/);
  });
});
