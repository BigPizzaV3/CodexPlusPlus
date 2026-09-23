import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const typesSource = await readFile(new URL("../web/src/types.ts", import.meta.url), "utf8");
const detailSource = await readFile(new URL("../web/src/components/TaskDetail.tsx", import.meta.url), "utf8");
const avatarSource = await readFile(new URL("../web/src/components/ActorAvatar.tsx", import.meta.url), "utf8");
const apiSource = await readFile(new URL("../web/src/api.ts", import.meta.url), "utf8");
const appSource = await readFile(new URL("../web/src/App.tsx", import.meta.url), "utf8");
const injectSource = await readFile(new URL("../inject/codex-taskboard.user.js", import.meta.url), "utf8");
const styles = await readFile(new URL("../web/src/styles.css", import.meta.url), "utf8");

test("task and comment contracts expose persisted user or agent identities", () => {
  assert.match(typesSource, /export type ActorType = "user" \| "agent"/);
  assert.match(typesSource, /export interface ActorIdentity/);
  assert.match(typesSource, /avatarUrl: string \| null/);
  assert.match(typesSource, /creatorType: ActorType/);
  assert.match(typesSource, /creatorId: string/);
  assert.match(typesSource, /creatorName: string/);
  assert.match(typesSource, /creatorAvatarUrl: string \| null/);
  assert.match(typesSource, /authorType: ActorType/);
  assert.match(typesSource, /authorId: string/);
  assert.match(typesSource, /authorName: string/);
  assert.match(typesSource, /authorAvatarUrl: string \| null/);
});

test("issue activity renders distinct avatars, IDs, and styles for users and agents", () => {
  assert.match(avatarSource, /function ActorAvatar/);
  assert.match(avatarSource, /actor-avatar-\$\{actor\.type\}/);
  assert.match(avatarSource, /actor\.type === "agent"/);
  assert.match(avatarSource, /className="actor-avatar-image actor-avatar-agent-image"/);
  assert.match(avatarSource, /src="\/codex-agent-logo\.png"/);
  assert.match(avatarSource, /actor\.avatarUrl/);
  assert.match(detailSource, /currentTask\.creatorType/);
  assert.match(detailSource, /currentTask\.creatorId/);
  assert.match(detailSource, /currentTask\.creatorAvatarUrl/);
  assert.match(detailSource, /comment\.authorType/);
  assert.match(detailSource, /comment\.authorId/);
  assert.match(detailSource, /comment\.authorAvatarUrl/);
  assert.match(detailSource, /currentUser\.name/);
  assert.match(detailSource, /currentUser\.id/);
  assert.match(detailSource, /className="actor-id"/);
  assert.match(styles, /\.actor-avatar-agent/);
  assert.match(styles, /\.actor-avatar-user/);
  assert.match(styles, /\.actor-avatar-image/);
  assert.match(
    styles,
    /\.actor-avatar-agent\s*\{[^}]*overflow:\s*visible;[^}]*border:\s*0;[^}]*border-radius:\s*0;[^}]*background:\s*transparent;/s,
  );
  assert.match(styles, /\.actor-avatar-agent-image\s*\{[^}]*object-fit:\s*contain;/s);
  assert.doesNotMatch(styles, /\.comment-entry\.is-agent \.comment-card/);
  assert.match(styles, /\.actor-id/);
});

test("agent avatar asset is a transparent PNG logo", async () => {
  const logo = await readFile(new URL("../web/public/codex-agent-logo.png", import.meta.url));
  assert.deepEqual([...logo.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
  assert.equal(logo[25], 6);
});

test("comment metadata separators never become avatar content", () => {
  assert.match(detailSource, /className="comment-edited"/);
  assert.doesNotMatch(styles, /\.comment-header > span(?::before)?/);
  assert.match(
    styles,
    /\.comment-header \.comment-edited::before\s*\{[^}]*content:\s*"·"/s,
  );
  assert.match(
    styles,
    /\.actor-avatar-image\s*\{[^}]*display:\s*block;[^}]*width:\s*100%;[^}]*height:\s*100%/s,
  );
});

test("Codex host identity is forwarded to user-authored taskboard mutations", () => {
  assert.match(injectSource, /function readCodexUser\(\)/);
  assert.match(injectSource, /cdn\.auth0\.com\/avatars/);
  assert.match(injectSource, /user: readCodexUser\(\)/);
  assert.match(typesSource, /user\?: ActorIdentity/);
  assert.match(apiSource, /export function setCurrentUserActor/);
  assert.match(apiSource, /X-Taskboard-User-Id/);
  assert.match(apiSource, /X-Taskboard-User-Name/);
  assert.match(apiSource, /X-Taskboard-User-Avatar/);
  assert.match(appSource, /setCurrentUserActor\(payload\.user\)/);
});
