import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, describe, it } from "node:test";

describe("floating-panel message role compatibility", () => {
  class ElementFixture {
    parentElement: ElementFixture | null = null;
    labels: ElementFixture[] = [];
    classList = new Set<string>();
    textContent: string;
    role: string | null;
    constructor(textContent = "", role: string | null = null) {
      this.textContent = textContent;
      this.role = role;
    }
    getAttribute(name: string) { return name === "data-conversation-role" ? this.role : null; }
    querySelectorAll(selector: string) {
      assert.equal(selector, "h4.sr-only");
      return this.labels;
    }
  }

  let roleFromLabel: (label: ElementFixture | null) => string;
  let containerForRole: (turn: ElementFixture, role: string) => ElementFixture | null;
  before(async () => {
    const source = await readFile(new URL("../../../assets/inject/floating-panel/runtime/answer-context.js", import.meta.url), "utf8");
    const start = source.indexOf("  function roleFromMessageLabel(");
    const end = source.indexOf("  function labeledMessageText(", start);
    assert.ok(start >= 0 && end > start);
    const api = new Function("Element", "normalizeText", "classTokenMatch",
      source.slice(start, end) + "\nreturn { roleFromMessageLabel, labeledMessageContainer };",
    )(ElementFixture, (text: unknown) => String(text ?? "").trim(),
      (node: ElementFixture, token: string) => node.classList.has(token));
    roleFromLabel = api.roleFromMessageLabel;
    containerForRole = api.labeledMessageContainer;
  });

  function turnWithLabel(text: string, role: string | null, containerClass: string) {
    const turn = new ElementFixture();
    const container = new ElementFixture();
    container.classList.add(containerClass);
    const label = new ElementFixture(text, role);
    label.parentElement = container;
    turn.labels.push(label);
    return { turn, container };
  }

  it("finds completed assistant and user containers after accessible labels change", () => {
    for (const [text, role, token] of [["最新一条回复", "assistant", "group"], ["用户输入", "user", "items-end"]]) {
      const { turn, container } = turnWithLabel(text, role, token);
      assert.equal(containerForRole(turn, role), container);
    }
  });

  it("prefers semantic roles over conflicting translated label text", () => {
    assert.equal(roleFromLabel(new ElementFixture("You said:", "assistant")), "assistant");
    assert.equal(roleFromLabel(new ElementFixture("Codex said:", "user")), "user");
  });

  it("keeps legacy labels working when the role attribute is absent or unknown", () => {
    for (const [text, role, token] of [["Codex said:", "assistant", "group"], ["你说：", "user", "items-end"]]) {
      for (const attribute of [null, "unknown"]) {
        const { turn, container } = turnWithLabel(text, attribute, token);
        assert.equal(containerForRole(turn, role), container);
      }
    }
    assert.equal(roleFromLabel(null), "");
    assert.equal(roleFromLabel(new ElementFixture("其他内容", "tool")), "");
  });

  it("still rejects containers that do not have the expected message structure", () => {
    const { turn } = turnWithLabel("最新一条回复", "assistant", "other");
    assert.equal(containerForRole(turn, "assistant"), null);
  });
});
