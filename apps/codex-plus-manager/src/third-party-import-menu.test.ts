/**
 * 「从第三方导入」：原来是挂在按钮下面的一个小弹层（只有 cc-switch、只能全部导入），
 * 现在是对话框（ThirdPartyImportDialog）：来源切换、逐条勾选、已导入的标出来。
 * 这里按源码守住它的结构；数据整理的逻辑在 third-party-import-model.test.ts。
 */
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { describe, it } from "node:test";

// Windows 检出（core.autocrlf）下源码是 CRLF，多行断言必须先归一化换行。
const readText = async (name: string) =>
  (await readFile(new URL(name, import.meta.url), "utf8")).replace(/\r\n/g, "\n");

const dialog = await readText("./ThirdPartyImportDialog.tsx");
const relayScreen = await readText("./RelayScreen.tsx");
const styles = await readText("./styles.css");
const gatewayCss = await readText("./GatewayPage.css");

describe("从第三方导入对话框", () => {
  it("供应商页只留一个按钮，点开是对话框；旧弹层的定位、外点关闭、滚动关闭都不再需要", () => {
    assert.match(relayScreen, /\{thirdPartyImportOpen \? \(\n\s*<ThirdPartyImportDialog actions=\{actions\} ccsProviders=\{ccsProviders\} form=\{normalized\} onClose=\{\(\) => setThirdPartyImportOpen\(false\)\} \/>/);
    assert.doesNotMatch(relayScreen, /third-party-import-menu|resolveAnchoredMenuLayout|thirdPartyImportTriggerRef|thirdPartyImportPosition/);
    assert.doesNotMatch(styles, /third-party-import/);
  });

  it("和共享供应商的「导入」是同一套弹窗：挂到 body，Esc、点遮罩关闭，条目样式共用", () => {
    assert.match(dialog, /useModalShell<HTMLDivElement>\(\{ onClose, labelledBy: titleId \}\)/);
    assert.match(dialog, /createPortal\(\n\s*<div className="gw-modal-backdrop gw-portal" \{\.\.\.backdropProps\}>\n\s*<div \{\.\.\.dialogProps\} className="gw-modal gw-import-modal tpi-modal">/);
    assert.match(dialog, /<div className="gw-import-item tpi-row" data-on=\{on\} key=\{key\}>/);
    assert.match(dialog, /const renderRow = \(item: ThirdPartyItem\) => \{/);
    assert.match(dialog, /<span className="gw-import-check">/);
    assert.match(dialog, /<footer className="gw-import-foot">/);
  });

  it("来源在顶部切换：CC Switch、Magpie，各带供应商数", () => {
    const model = dialog.includes("THIRD_PARTY_SOURCES.map((key) => (");
    assert.ok(model, "来源页签按 THIRD_PARTY_SOURCES 渲染");
    assert.match(dialog, /<div aria-label=\{t\("导入来源"\)\} className="mg-segmented tpi-sources" role="tablist">/);
    assert.match(dialog, /<span className="tpi-count">\{items\.filter\(\(item\) => item\.source === key && !item\.upstream\)\.length\}<\/span>/);
  });

  // issue #1906：cc-switch 供应商几十个时，导入按钮掉到窗口下方，点了没反应。
  it("列表在弹窗内容区里滚动，导入 / 刷新按钮在滚动区之外，始终看得到", () => {
    const body = dialog.indexOf('<div className="gw-modal-body">');
    const list = dialog.indexOf('<section className="tpi-list">');
    const foot = dialog.indexOf('<footer className="gw-import-foot">');
    assert.ok(body > 0 && list > body && foot > list, "列表在内容区里，底栏排在内容区之后");
    assert.match(dialog.slice(list, foot), /\{copyItems\.map\(renderRow\)\}/, "供应商行在滚动的内容区里");
    assert.match(dialog.slice(foot), /onClick=\{\(\) => void submit\(\)\}/, "导入按钮在底栏");
    assert.match(dialog.slice(foot), /onClick=\{\(\) => void reload\(\)\}/, "刷新按钮在底栏");
    // 弹窗限高、内容区滚动、底栏不参与滚动。
    assert.match(gatewayCss, /\.gw-modal \{[^}]*max-height: 100%;[^}]*overflow: hidden;/);
    assert.match(gatewayCss, /\.gw-modal-body \{[^}]*overflow-y: auto;/);
    assert.match(gatewayCss, /\.gw-portal \.tpi-modal \.gw-modal-body \{\n  min-height: 0;/);
  });

  it("逐条勾选：已导入的标出来、不能勾；两个来源勾的一起导入，cc-switch 只导勾着的", () => {
    assert.match(dialog, /disabled=\{busy \|\| item\.imported \|\| item\.upstream \|\| \(item\.source === "ccswitch" && ccsPathDirty\)\}/);
    assert.match(dialog, /\{item\.imported \? <em className="gw-import-badge">\{t\("已导入"\)\}<\/em> : null\}/);
    assert.match(dialog, /if \(ccsChosen\.length\) await actions\.importCcsProviders\(bindings, ccsChosen\);\n\s*if \(plan\.magpie\.length\) await actions\.importMagpieProviders\(plan\.magpie\);/);
    // 路径改了还没应用：cc-switch 那一栏的列表是旧库的，不算进这次导入。
    assert.match(dialog, /const ccsChosen = ccsPathDirty \? \[\] : plan\.ccswitch;/);
    assert.match(dialog, /disabled=\{busy \|\| total === 0\}/);
  });

  it("打开时两个来源都重新读一次；Magpie 的列表不带 Key", () => {
    assert.match(dialog, /await Promise\.all\(\[actions\.refreshCcsProviders\(true\), loadMagpie\(\)\]\);/);
    assert.match(dialog, /invoke<MagpieProvidersResult>\("load_magpie_providers"\)/);
    const types = dialog.match(/MagpieProvidersResult/g)?.length ?? 0;
    assert.ok(types >= 2);
  });

  it("Magpie 两种导入方式二选一：「把 Magpie 当作一个上游」（默认）还是「复制它的供应商」，一次只导选中的那种", () => {
    // 两个选项是单选；没手动选过时跟着数据走，默认当作上游，排在前面。
    assert.match(dialog, /const \[chosenMagpieMode, setMagpieMode\] = useState<MagpieImportMode \| null>\(null\);/);
    assert.match(dialog, /const magpieMode = chosenMagpieMode \?\? defaultMagpieMode\(items\);/);
    assert.ok(dialog.indexOf('<strong>{t("把 Magpie 当作一个上游")}</strong>') < dialog.indexOf('<strong>{t("复制它的供应商")}</strong>'), "当作上游排在复制前面");
    assert.match(dialog, /<div aria-label=\{t\("导入方式"\)\} className="tpi-modes" role="radiogroup">/);
    assert.match(dialog, /<button aria-checked=\{!upstreamMode\} className="tpi-mode"[^>]*onClick=\{\(\) => setMagpieMode\("copy"\)\} role="radio" type="button">\n\s*<strong>\{t\("复制它的供应商"\)\}<\/strong>/);
    assert.match(dialog, /<button aria-checked=\{upstreamMode\} className="tpi-mode"[^>]*onClick=\{\(\) => setMagpieMode\("upstream"\)\} role="radio" type="button">\n\s*<strong>\{t\("把 Magpie 当作一个上游"\)\}<\/strong>/);
    // 别的来源（没有「当作上游」这一条）不出这一排。
    assert.match(dialog, /\{upstreamItem \? \(\n\s*<div aria-label=\{t\("导入方式"\)\}/);
    // 下面只列选中那种方式要导的：当作上游只有一条，复制是逐条勾选的列表。
    assert.match(dialog, /\{upstreamMode && upstreamItem \? renderRow\(upstreamItem\) : \(/);
    // 导入计划带上方式；当作上游那一条不靠勾选、也不能取消。
    assert.match(dialog, /const plan = importPlan\(items, picked, magpieMode\);/);
    assert.match(dialog, /const on = !item\.imported && \(item\.upstream \|\| picked\.has\(key\)\);/);
    assert.match(dialog, /disabled=\{busy \|\| item\.imported \|\| item\.upstream \|\|/);
    // 「全选未导入的」只在复制方式下管复制的那些；页签上的数字也只数可复制的供应商。
    assert.match(dialog, /const pending = upstreamMode \? \[\] : copyItems\.filter\(\(item\) => !item\.imported\);/);
    assert.match(dialog, /items\.filter\(\(item\) => item\.source === key && !item\.upstream\)\.length/);
    // 当作上游的那一条直接显示完整地址。
    assert.match(dialog, /\{item\.upstream \? item\.baseUrl : \[protocolLabel\(item\.protocol\)/);
    assert.match(gatewayCss, /\.gw-portal \.tpi-mode\[aria-checked="true"\] \{\n  border-color: hsl\(var\(--primary\)\);/);
  });

  it("行本身不是按钮（里面有「导入为」下拉）：勾选区是行里的按钮", () => {
    assert.match(dialog, /<button\n\s*aria-pressed=\{on\}\n\s*className="tpi-pick"/);
    assert.match(dialog, /\{item\.source === "ccswitch" && on \? \(\n\s*<AppSelect/);
    assert.match(gatewayCss, /\.gw-portal \.tpi-pick \{[^}]*flex: 1 1 auto;[^}]*cursor: pointer;/);
  });
});
