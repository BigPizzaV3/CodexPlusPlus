import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../", import.meta.url);

async function source(pathname) {
  return readFile(new URL(pathname, root), "utf8");
}

test("issue model and editor preserve a concrete assignee identity", async () => {
  const [typesSource, appSource, editorSource] = await Promise.all([
    source("web/src/types.ts"),
    source("web/src/App.tsx"),
    source("web/src/components/TaskEditor.tsx"),
  ]);

  assert.match(typesSource, /export type AssigneeTarget = "current-user" \| "codex-agent"/);
  assert.match(typesSource, /assignee: ActorIdentity/);
  assert.match(typesSource, /export interface TaskDraft \{[\s\S]*?assigneeTarget\?: AssigneeTarget/);
  assert.match(appSource, /assigneeTarget/);
  assert.match(editorSource, /currentUser: ActorIdentity/);
  assert.match(editorSource, /aria-label="负责人"/);
  assert.match(editorSource, /CODEX_AGENT_ACTOR/);
});

test("issue detail and cards expose the same assignee identity", async () => {
  const [detailSource, cardSource, avatarSource, styles] = await Promise.all([
    source("web/src/components/TaskDetail.tsx"),
    source("web/src/components/TaskCard.tsx"),
    source("web/src/components/ActorAvatar.tsx"),
    source("web/src/styles.css"),
  ]);

  assert.match(detailSource, /detail-property-label">负责人/);
  assert.match(detailSource, /saveTask\(\{ assigneeTarget:/);
  assert.match(cardSource, /card-assignee-avatar/);
  assert.match(avatarSource, /codex-agent-logo\.png/);
  assert.match(avatarSource, /actor-avatar-\$\{actor\.type\}/);
  assert.match(styles, /\.card-assignee-avatar/);
});

test("issue card assignee avatar stays compact inside the identifier row", async () => {
  const styles = await source("web/src/styles.css");
  const baseAvatarRule = styles.indexOf(".actor-avatar {");
  const cardAvatarRule = styles.indexOf(".actor-avatar.card-assignee-avatar {");
  const cardActionsRule = styles.match(/\.card-actions\s*\{([^}]*)\}/)?.[1] ?? "";

  assert.ok(baseAvatarRule >= 0);
  assert.ok(cardAvatarRule > baseAvatarRule);
  assert.match(
    styles.slice(cardAvatarRule),
    /\.actor-avatar\.card-assignee-avatar\s*\{[\s\S]*?width:\s*16px;[\s\S]*?height:\s*16px;/,
  );
  assert.match(styles, /\.card-topline\s*\{[\s\S]*?height:\s*16px;/);
  assert.match(cardActionsRule, /position:\s*absolute;/);
  assert.match(cardActionsRule, /right:\s*0;/);
  assert.match(cardActionsRule, /pointer-events:\s*none;/);
  assert.match(
    styles,
    /\.task-card:hover \.card-assignee-avatar,[\s\S]*?\.task-card\.is-context-open \.card-assignee-avatar\s*\{[\s\S]*?opacity:\s*0;/,
  );
});
