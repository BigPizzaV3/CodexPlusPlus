import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { describe, it } from "node:test";

// Windows 检出（core.autocrlf）下源码是 CRLF，多行断言必须先归一化换行。
const readText = async (name: string) =>
  (await readFile(new URL(name, import.meta.url), "utf8")).replace(/\r\n/g, "\n");

const app = await readText("./App.tsx");
const styles = await readText("./styles.css");

/**
 * 「从第三方导入」菜单的整段源码：从菜单容器开始，到下一个同级区块
 * （供应商分页 tab）之前。菜单内部结构变了这里会立刻失败，属于有意的契约。
 */
function importMenuSource() {
  const start = app.indexOf('className="third-party-import-menu"');
  assert.ok(start > 0, "找不到 third-party-import-menu");
  const end = app.indexOf('className="provider-view-tabs"', start);
  assert.ok(end > start, "找不到菜单之后的供应商分页 tab");
  return app.slice(start, end);
}

function styleBlock(selector: string) {
  const start = styles.indexOf(`${selector} {`);
  assert.ok(start >= 0, `缺少样式 ${selector}`);
  const end = styles.indexOf("}", start);
  assert.ok(end > start);
  return styles.slice(start, end);
}

describe("从第三方导入菜单的可达性", () => {
  // issue #1906：cc-switch 供应商几十个时，导入按钮掉到窗口下方，点了没反应。
  it("位置与限高由 resolveAnchoredMenuLayout 按触发按钮算出来，并写进内联样式", () => {
    // 坐标在打开菜单时算（openThirdPartyImport 里），菜单只负责消费算好的结果。
    assert.match(
      app,
      /resolveAnchoredMenuLayout\(rect, \{ width: window\.innerWidth, height: window\.innerHeight \}\)/,
    );
    const menu = importMenuSource();
    assert.match(menu, /position: "fixed"/);
    // 只传 top/right 不传 maxHeight 就是修复前的形态：菜单会溢出窗口下沿。
    assert.match(menu, /maxHeight: thirdPartyImportPosition\.maxHeight/);
  });

  it("供应商列表单独滚动，导入/刷新按钮在滚动区之外", () => {
    const menu = importMenuSource();
    const list = menu.indexOf('className="third-party-import-list"');
    const actions = menu.indexOf('className="third-party-import-actions"');
    assert.ok(list > 0, "供应商列表缺少独立的滚动容器");
    assert.ok(actions > list, "导入按钮区必须排在供应商列表之后");
    const listBody = menu.slice(list, actions);
    assert.match(listBody, /providers\.map\(/, "供应商行要放在滚动容器里");
    assert.match(menu.slice(actions), /importCcsProviders\(/, "导入按钮不能在滚动容器里");
  });

  it("样式里滚动交给列表区，菜单只在极矮窗口兜底滚动", () => {
    const menuStyle = styleBlock(".third-party-import-menu");
    assert.match(menuStyle, /display: flex/);
    assert.match(menuStyle, /flex-direction: column/);
    // 正常情况只有列表滚；菜单自身的 overflow 只作为「连头尾都放不下」时的兜底，
    // 若改成 hidden，矮窗口会把导入按钮裁掉（#2648 review）。
    assert.match(menuStyle, /overflow-y: auto/);
    assert.match(styleBlock(".third-party-import-list"), /overflow-y: auto/);
    assert.match(styleBlock(".third-party-import-list"), /min-height: 0/);
  });

  it("只有列表参与伸缩，其余子项不被压扁", () => {
    // 回归：flex column + max-height 下「实际来源：…」那一行被压到 4px 高。
    assert.match(
      styles,
      /\.third-party-import-menu > \*:not\(\.third-party-import-list\) \{\n {2}flex: none;/,
    );
    assert.match(styleBlock(".third-party-import-list"), /flex: 1 1 auto/);
  });

  it("列表自己滚动时不会把菜单关掉", () => {
    // 回归：scroll 事件的 capture 监听挂在 window 上，列表内滚轮/拖滚动条同样会冒上去，
    // 以前一律 close()，表现为「滚一下就关、列表根本没法滚」。
    assert.match(
      app,
      /const handleScroll = \(event: Event\) => \{\n\s+if \(thirdPartyImportTriggerRef\.current\?\.contains\(event\.target as Node\)\) return;\n\s+close\(\);\n\s+\};/,
    );
    assert.match(app, /addEventListener\("scroll", handleScroll, \{ capture: true, passive: true \}\)/);
    assert.doesNotMatch(app, /addEventListener\("scroll", close, \{ capture: true, passive: true \}\)/);
  });
});
