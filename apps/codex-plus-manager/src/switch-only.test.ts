/** 开关行只有开关本身可点。 */
import assert from "node:assert/strict";
import test from "node:test";
import { blocksSwitchRowClick } from "./switch-only.ts";

type Node = { tag: string; classes?: string[]; role?: string; parent?: Node; children?: Node[] };

/** 只实现用到的 closest / querySelector / contains，选择器按逗号拆开逐个匹配。 */
function element(node: Node) {
  const matches = (candidate: Node, selector: string) => selector.split(",").map((part) => part.trim()).some((part) => {
    if (part.startsWith(".")) return candidate.classes?.includes(part.slice(1)) ?? false;
    if (part.startsWith("[role=")) return candidate.role === part.slice(7, -2);
    return candidate.tag === part;
  });
  const wrap = (current: Node): unknown => ({
    node: current,
    closest(selector: string) {
      for (let cursor: Node | undefined = current; cursor; cursor = cursor.parent) if (matches(cursor, selector)) return wrap(cursor);
      return null;
    },
    querySelector(selector: string) {
      const walk = (candidate: Node): Node | null => {
        for (const child of candidate.children ?? []) {
          if (matches(child, selector)) return child;
          const found = walk(child);
          if (found) return found;
        }
        return null;
      };
      const found = walk(current);
      return found ? wrap(found) : null;
    },
    contains(other: { node: Node }) {
      for (let cursor: Node | undefined = other.node; cursor; cursor = cursor.parent) if (cursor === current) return true;
      return false;
    },
  });
  return wrap(node) as EventTarget;
}

function tree(node: Node, parent?: Node): Node {
  node.parent = parent;
  for (const child of node.children ?? []) tree(child, node);
  return node;
}

function switchRow() {
  const title: Node = { tag: "strong" };
  const link: Node = { tag: "a" };
  const detail: Node = { tag: "small", children: [link] };
  const input: Node = { tag: "input" };
  const thumb: Node = { tag: "span", classes: ["toggle-switch-thumb"] };
  const visual: Node = { tag: "span", classes: ["toggle-switch-visual"], children: [thumb] };
  const label = tree({ tag: "label", classes: ["switch-row"], children: [input, { tag: "span", children: [title, detail] }, visual] });
  return { label, title, detail, link, input, visual, thumb };
}

test("点开关行的标题、说明、行内空白：拦掉，不切换", () => {
  const row = switchRow();
  assert.equal(blocksSwitchRowClick(element(row.title)), true);
  assert.equal(blocksSwitchRowClick(element(row.detail)), true);
  assert.equal(blocksSwitchRowClick(element(row.label)), true);
});

test("点开关图形（含滑块）、checkbox 本身、说明里的链接：放行", () => {
  const row = switchRow();
  assert.equal(blocksSwitchRowClick(element(row.visual)), false);
  assert.equal(blocksSwitchRowClick(element(row.thumb)), false);
  // label 被点后浏览器会再给 checkbox 派发一次点击，这一次不能拦。
  assert.equal(blocksSwitchRowClick(element(row.input)), false);
  assert.equal(blocksSwitchRowClick(element(row.link)), false);
});

test("不带开关图形的 label（勾选框、表单字段）和 label 之外的点击不受影响", () => {
  const text: Node = { tag: "span" };
  tree({ tag: "label", classes: ["mg-check-line"], children: [{ tag: "input" }, text] });
  assert.equal(blocksSwitchRowClick(element(text)), false);
  assert.equal(blocksSwitchRowClick(element(tree({ tag: "div" }))), false);
  assert.equal(blocksSwitchRowClick(null), false);
});

test("开关行放在按钮之类的容器里：行外的可点祖先不算，点文字仍然拦掉", () => {
  const row = switchRow();
  tree({ tag: "button", children: [row.label] });
  assert.equal(blocksSwitchRowClick(element(row.title)), true);
});
