import assert from "node:assert";
import { describe, it } from "node:test";
import {
  isTriggerVisible,
  openPopoverCount,
  registerOpenPopover,
  releaseOpenPopover,
  resetOpenPopovers,
  resolvePopoverLayout,
  type PopoverRect,
} from "./popover-layer.ts";

const viewport = { width: 1200, height: 800 };

function rect(partial: Partial<PopoverRect> & { top: number; left: number }): PopoverRect {
  const width = partial.width ?? 200;
  const height = partial.height ?? 40;
  return {
    top: partial.top,
    left: partial.left,
    width,
    height,
    right: partial.right ?? partial.left + width,
    bottom: partial.bottom ?? partial.top + height,
  };
}

describe("resolvePopoverLayout", () => {
  it("贴在触发器下方并按触发器宽度铺满", () => {
    const layout = resolvePopoverLayout({
      trigger: rect({ top: 100, left: 40, width: 260 }),
      viewport,
      menuHeight: 180,
    });
    assert.equal(layout.placement, "bottom");
    assert.equal(layout.top, 146);
    assert.equal(layout.left, 40);
    assert.equal(layout.width, 260);
    assert.equal(layout.maxHeight, 180);
  });

  it("下方空间不足且上方更宽松时向上翻转", () => {
    const layout = resolvePopoverLayout({
      trigger: rect({ top: 700, left: 40 }),
      viewport,
      menuHeight: 300,
    });
    assert.equal(layout.placement, "top");
    assert.equal(layout.top, 700 - 6 - 300);
    assert.equal(layout.maxHeight, 300);
  });

  it("两侧都放不下时选空间更大的一侧并压缩 maxHeight", () => {
    const layout = resolvePopoverLayout({
      trigger: rect({ top: 380, left: 40 }),
      viewport,
      menuHeight: 400,
      maxHeight: 400,
    });
    // 下方 800-8-(420+6)=366 < 400，上方 380-6-8=366，两侧相同取下方。
    assert.equal(layout.placement, "bottom");
    assert.equal(layout.maxHeight, 366);
    assert.equal(layout.top, 426);
  });

  it("end 对齐时菜单右边缘贴住触发器右边缘", () => {
    const layout = resolvePopoverLayout({
      trigger: rect({ top: 100, left: 900, width: 200 }),
      viewport,
      menuHeight: 120,
      align: "end",
      menuWidth: 240,
    });
    assert.equal(layout.width, 240);
    assert.equal(layout.left, 1100 - 240);
  });

  it("宽度被视口夹紧，固定宽度弹层也遵守 minWidth", () => {
    const clipped = resolvePopoverLayout({
      trigger: rect({ top: 100, left: 0, width: 1300 }),
      viewport,
      menuHeight: 100,
    });
    assert.equal(clipped.width, 1200 - 16);
    assert.equal(clipped.left, 8);

    const widened = resolvePopoverLayout({
      trigger: rect({ top: 100, left: 20, width: 60 }),
      viewport,
      menuHeight: 100,
      minWidth: 180,
    });
    assert.equal(widened.width, 180);
  });

  it("窗口很矮时按剩余空间限高，菜单不会溢出视口下沿", () => {
    const layout = resolvePopoverLayout({
      trigger: rect({ top: 100, left: 40 }),
      viewport: { width: 1200, height: 300 },
      menuHeight: 500,
      maxHeight: 420,
    });
    assert.equal(layout.placement, "bottom");
    assert.equal(layout.maxHeight, 300 - 8 - 146);
    assert.ok(layout.top + layout.maxHeight <= 300 - 8);
  });
});

describe("isTriggerVisible", () => {
  it("触发器完全滚出视口时判定为不可见", () => {
    assert.equal(isTriggerVisible(rect({ top: -80, bottom: -40, left: 20 }), viewport), false);
    assert.equal(isTriggerVisible(rect({ top: 820, bottom: 860, left: 20 }), viewport), false);
    assert.equal(isTriggerVisible(rect({ top: 100, left: 20 }), viewport), true);
    assert.equal(isTriggerVisible(rect({ top: 790, bottom: 830, left: 20 }), viewport), true);
  });
});

describe("open popover registry", () => {
  it("打开新弹层时自动收起之前打开的弹层", () => {
    resetOpenPopovers();
    const closed: string[] = [];
    registerOpenPopover("a", () => closed.push("a"));
    assert.equal(openPopoverCount(), 1);
    registerOpenPopover("b", () => closed.push("b"));
    assert.deepEqual(closed, ["a"]);
    assert.equal(openPopoverCount(), 1);
    registerOpenPopover("c", () => closed.push("c"));
    assert.deepEqual(closed, ["a", "b"]);
    assert.equal(openPopoverCount(), 1);

    releaseOpenPopover("c");
    assert.equal(openPopoverCount(), 0);
  });

  it("重复打开同一个弹层不会把自己关掉", () => {
    resetOpenPopovers();
    const closed: string[] = [];
    registerOpenPopover("a", () => closed.push("a"));
    registerOpenPopover("a", () => closed.push("a-again"));
    assert.deepEqual(closed, []);
    assert.equal(openPopoverCount(), 1);
    resetOpenPopovers();
  });
});
