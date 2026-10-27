import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { describe, it } from "node:test";

// Windows 检出（core.autocrlf）下源码是 CRLF，多行契约断言必须先把换行归一化，
// 否则同一份代码在 macOS 绿、在 Windows CI 红。
const readText = async (name: string) =>
  (await readFile(new URL(name, import.meta.url), "utf8")).replace(/\r\n/g, "\n");

const app = await readText("./App.tsx");
const hookSource = await readText("./use-popover-layer.ts");
const styles = await readText("./styles.css");
// AppSelect 抽成了独立模块，全应用共用。
const selectSource = await readText("./AppSelect.tsx");

function componentSource(name: string) {
  // 终止行必须是顶格单独一个 }（`}: {` 这种行首是 } 的不算）。
  const source = name === "AppSelect" ? selectSource : app;
  const match = source.match(new RegExp(`^(?:export )?function ${name}[<(][^]*?^\\}$`, "m"));
  assert.ok(match, `missing ${name}`);
  return match[0];
}

describe("manager popover layer contract", () => {
  // 回归：「点其他地方收不起来 / 多个下拉同时展开」。
  // WKWebView 里点非可聚焦元素不触发 blur，relatedTarget 判定靠不住。
  it("下拉不再用 onBlur + relatedTarget 收", () => {
    for (const name of ["AppSelect", "ModelReasoningLevelsCell"]) {
      assert.doesNotMatch(componentSource(name), /relatedTarget/, name);
    }
  });

  it("usePopoverLayer 用 document 上的 pointerdown 判定外点关闭并登记单开", () => {
    const hook = hookSource;
    assert.match(hook, /document\.addEventListener\("pointerdown", handlePointerDown, true\)/);
    assert.match(hook, /rootRef\.current\?\.contains\(target\) \|\| menuRef\.current\?\.contains\(target\)/);
    assert.match(hook, /registerOpenPopover\(layerId, close\)/);
    assert.match(hook, /releaseOpenPopover\(layerId\)/);
    assert.match(hook, /event\.key !== "Escape"/);
    assert.match(hook, /isTriggerVisible\(trigger/);
  });

  it("菜单 portal 到 body 并用 fixed 内联坐标，摆脱父级 overflow 裁剪", () => {
    assert.match(hookSource, /position: "fixed"/);

    const select = componentSource("AppSelect");
    assert.match(select, /createPortal\(/);
    assert.match(select, /app-select-menu app-select-menu-portal/);
    assert.match(select, /document\.body/);
    assert.doesNotMatch(select, /className="app-select-menu"/);

    const reasoning = componentSource("ModelReasoningLevelsCell");
    assert.match(reasoning, /app-select-menu app-select-menu-portal relay-reasoning-menu/);
    assert.match(reasoning, /document\.body/);
  });

  it("搜索型下拉同样走 portal 弹层，不再各自维护外点监听", () => {
    for (const name of ["SearchablePathPicker", "SearchableSessionPicker"]) {
      const source = componentSource(name);
      assert.match(source, /usePopoverLayer\(\{ open, onOpenChange: setOpen/, name);
      assert.match(source, /weixin-search-menu weixin-search-menu-portal/, name);
      assert.match(source, /document\.body/, name);
      assert.doesNotMatch(source, /document\.addEventListener\("pointerdown"/, name);
    }
  });

  it("样式里保留 portal 弹层的层级与视口边界", () => {
    assert.match(styles, /\.app-select-menu-portal,\n\.weixin-search-menu-portal \{/);
    assert.match(styles, /z-index: 240/);
    assert.match(styles, /max-width: calc\(100vw - 16px\)/);
    // 菜单搬出单元格后，字号规则要挂在菜单自身的类名上，否则会退回默认字号。
    assert.match(styles, /\.relay-reasoning-menu \.app-select-option \{/);
    assert.match(styles, /\.app-select-menu\.text-xs \.app-select-option \{/);
  });
});
