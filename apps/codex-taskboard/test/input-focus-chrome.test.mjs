import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const globalStyles = await readFile(new URL("../web/src/styles.css", import.meta.url), "utf8");
const workflowStyles = await readFile(new URL("../web/src/components/workflow.css", import.meta.url), "utf8");

function blocksForSelector(styles, selector) {
  return [...styles.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter(([, selectors]) => selectors.split(",").some((candidate) => candidate.trim() === selector))
    .map(([, , declarations]) => declarations);
}

function assertNoFocusChrome(styles, selector) {
  for (const declarations of blocksForSelector(styles, selector)) {
    assert.doesNotMatch(
      declarations,
      /(?:^|;)\s*(?:border(?:-color)?|background(?:-color)?|box-shadow|outline)\s*:/,
      `${selector} must not alter border, background, shadow or outline`,
    );
  }
}

test("editable controls suppress native focus outlines without disabling non-text focus feedback", () => {
  assert.match(
    globalStyles,
    /:where\([\s\S]*?input:not\(\[type="checkbox"\]\)[\s\S]*?:not\(\[type="radio"\]\)[\s\S]*?:not\(\[type="range"\]\)[\s\S]*?:not\(\[type="file"\]\)[\s\S]*?:not\(\[type="color"\]\)[\s\S]*?:not\(\[type="button"\]\)[\s\S]*?:not\(\[type="submit"\]\)[\s\S]*?:not\(\[type="reset"\]\)[\s\S]*?:not\(\[type="image"\]\)[\s\S]*?textarea,[\s\S]*?select,[\s\S]*?\[contenteditable="true"\],[\s\S]*?\[contenteditable="plaintext-only"\][\s\S]*?\):is\(:focus, :focus-visible\)\s*\{\s*outline:\s*0;\s*\}/,
  );
  assert.doesNotMatch(globalStyles, /(?:^|,)\s*(?:input|textarea|select):focus-visible\s*(?:,|\{)/m);
  assert.doesNotMatch(globalStyles, /\*:focus(?:-visible)?/);
  assert.match(
    globalStyles,
    /input:is\([\s\S]*?\[type="checkbox"\][\s\S]*?\[type="radio"\][\s\S]*?\[type="range"\][\s\S]*?\[type="file"\][\s\S]*?\):focus-visible/,
  );
});

test("editable-control wrappers do not add focus chrome", () => {
  for (const selector of [
    ".project-card:focus-within",
    ".search-field:focus-within",
    ".detail-property-row:focus-within",
    ".workflow-node-search:focus-within",
    ".comment-composer:focus-within",
  ]) {
    assertNoFocusChrome(globalStyles, selector);
  }

  assert.equal(blocksForSelector(globalStyles, ".workflow-tab.is-renaming input:focus").length, 0);
  assert.equal(blocksForSelector(globalStyles, ".workflow-config-section input:focus").length, 0);
  assert.equal(blocksForSelector(globalStyles, ".workflow-config-section textarea:focus").length, 0);
  assert.equal(blocksForSelector(globalStyles, ".workflow-config-section select:focus").length, 0);
});

test("workflow-local styles do not reintroduce input focus chrome", () => {
  assertNoFocusChrome(workflowStyles, ".workflow-step-picker-search:focus-within");
  assert.equal(blocksForSelector(workflowStyles, ".workflow-tab.is-renaming input:focus").length, 0);
  assert.equal(blocksForSelector(workflowStyles, ".workflow-config-section input:focus").length, 0);
  assert.equal(blocksForSelector(workflowStyles, ".workflow-config-section textarea:focus").length, 0);
  assert.equal(blocksForSelector(workflowStyles, ".workflow-config-section select:focus").length, 0);
});
