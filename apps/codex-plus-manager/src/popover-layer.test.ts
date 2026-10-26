import assert from "node:assert";
import { describe, it } from "node:test";
import {
  isTriggerVisible,
  openPopoverCount,
  registerOpenPopover,
  releaseOpenPopover,
  resetOpenPopovers,
  resolveAnchoredMenuLayout,
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

describe("resolveAnchoredMenuLayout", () => {
  // issue #1906：「从第三方导入」菜单里 cc-switch 供应商有几十行时，
  // 最后一排按钮（ccswitch 导入 / 刷新列表）会掉到窗口外面点不到。
  it("限高按触发按钮的位置扣掉，菜单底边不会超出视口", () => {
    const trigger = rect({ top: 200, left: 900 });
    const layout = resolveAnchoredMenuLayout(trigger, viewport);
    assert.equal(layout.top, trigger.bottom + 8);
    assert.equal(layout.maxHeight, viewport.height - layout.top - 12);
    assert.ok(layout.top + layout.maxHeight + 12 <= viewport.height, "菜单必须留在视口内");
  });

  it("供应商越多、触发按钮越靠下，限高必须比裸 100vh 更紧", () => {
    const low = resolveAnchoredMenuLayout(rect({ top: 420, left: 900 }), viewport);
    assert.ok(low.maxHeight < viewport.height - 24);
    // 34 行供应商约 1900px 内容，滚动区只可能拿到限高内的空间。
    assert.ok(low.maxHeight < 1900);
  });

  it("右边缘对齐触发按钮并至少留出安全距离", () => {
    const aligned = resolveAnchoredMenuLayout(rect({ top: 100, left: 1000, width: 200 }), viewport);
    assert.equal(aligned.right, 12);
    const wide = resolveAnchoredMenuLayout(rect({ top: 100, left: 100, width: 200 }), viewport);
    assert.equal(wide.right, viewport.width - 300);
  });

  it("触发按钮贴顶时向下展开，top 不小于安全距离", () => {
    const atTop = resolveAnchoredMenuLayout(rect({ top: 0, left: 100 }), viewport);
    assert.equal(atTop.placement, "bottom");
    assert.equal(atTop.top, 40 + 8);
    assert.ok(atTop.maxHeight > 0);
  });

  // #2648 review：触发按钮贴着窗口底部时，只扣 top 会把高度算成 0，
  // 配合 overflow 就是「整个菜单消失」。必须向上展开，不能返回 0 高。
  it("触发按钮贴底时向上展开，菜单仍在视口内", () => {
    const trigger = rect({ top: 780, left: 900 });
    const layout = resolveAnchoredMenuLayout(trigger, { width: 1200, height: 820 });
    assert.equal(layout.placement, "top");
    assert.ok(layout.maxHeight > 0, "不能返回 0 高度把菜单藏起来");
    assert.ok(layout.top >= 12);
    assert.ok(layout.top + layout.maxHeight <= 820 - 12);
    assert.ok(layout.top + layout.maxHeight <= trigger.top, "向上展开不能盖住触发按钮");
  });

  it("上下都放不下时保底最小可用高度，并整体夹进视口", () => {
    const layout = resolveAnchoredMenuLayout(rect({ top: 150, left: 100 }), { width: 1200, height: 200 });
    assert.ok(layout.maxHeight >= 0 && layout.maxHeight <= 200 - 24);
    assert.ok(layout.top >= 12);
    assert.ok(layout.top + layout.maxHeight <= 200 - 12);
    assert.ok(layout.maxHeight > 0, "再矮也要留出可操作高度");
  });

  it("任何触发位置都不会算出 0 高菜单", () => {
    for (let top = 0; top <= 800; top += 40) {
      for (const height of [400, 600, 820]) {
        const layout = resolveAnchoredMenuLayout(rect({ top, left: 100 }), { width: 1200, height });
        assert.ok(layout.maxHeight > 0, `top=${top} height=${height} 得到 0 高`);
        assert.ok(layout.top >= 12);
        assert.ok(layout.top + layout.maxHeight <= height - 12, `top=${top} height=${height} 超出视口`);
      }
    }
  });
});
