import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const skillSource = await readFile(
  new URL("../skills/manage-taskboard/SKILL.md", import.meta.url),
  "utf8",
);

test("the taskboard skill coordinates safe issue execution and review handoff", () => {
  assert.match(skillSource, /read the latest issue content and all comments/i);
  assert.match(skillSource, /completed work.*returned|returned.*completed work/i);
  assert.match(skillSource, /claim.*`todo`.*`in_progress`.*`--if-version`/is);
  assert.match(skillSource, /version conflict.*skip the issue.*do not implement/is);

  assert.match(
    skillSource,
    /after implementation[^\n]*add a comment[^\n]*key changes[^\n]*verification[^\n]*result[^\n]*risks[^\n]*then move[^\n]*`in_review`/i,
  );
});
