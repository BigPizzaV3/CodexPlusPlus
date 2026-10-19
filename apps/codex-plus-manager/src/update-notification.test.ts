import assert from "node:assert/strict";
import test from "node:test";
import { UPDATE_MUTED_DAY_KEY, UpdateNotifications } from "./update-notification.ts";

const release = { status: "ok", updateAvailable: true, latestVersion: "v1.11.0" };

function storage() {
  const entries = new Map<string, string>();
  return { getItem: (key: string) => entries.get(key) ?? null, setItem: (key: string, value: string) => { entries.set(key, value); } };
}

test("only available releases prompt; closing suppresses this version until next session", () => {
  const saved = storage();
  const notifications = new UpdateNotifications(() => saved);
  for (const result of [{ ...release, status: "failed" }, { ...release, updateAvailable: false }, { ...release, latestVersion: null }]) {
    assert.equal(notifications.shouldPrompt(result, true), false);
  }
  assert.equal(notifications.shouldPrompt(release), true);
  notifications.dismiss(release.latestVersion);
  assert.equal(notifications.shouldPrompt(release), false);
  assert.equal(notifications.shouldPrompt(release, true), true);
  assert.equal(notifications.shouldPrompt({ ...release, latestVersion: "v1.12.0" }), true);
  assert.equal(new UpdateNotifications(() => saved).shouldPrompt(release), true);
});

test("today's mute persists across restart, applies to every release, and expires at local midnight", () => {
  const saved = storage();
  const notifications = new UpdateNotifications(() => saved);
  const late = new Date(2026, 9, 18, 23, 59);
  notifications.muteToday(late);
  assert.equal(saved.getItem(UPDATE_MUTED_DAY_KEY), "2026-10-18");
  const restarted = new UpdateNotifications(() => saved);
  assert.equal(restarted.shouldPrompt(release, false, late), false);
  assert.equal(restarted.shouldPrompt({ ...release, latestVersion: "v1.12.0" }, false, late), false);
  assert.equal(restarted.shouldPrompt(release, true, late), true);
  assert.equal(restarted.shouldPrompt(release, false, new Date(2026, 9, 19)), true);
  assert.equal(restarted.shouldPrompt(release, false, new Date(2026, 10, 18)), true);
});

test("blocked localStorage still supports in-session mute and tomorrow's reminder", () => {
  const notifications = new UpdateNotifications(() => { throw new Error("storage disabled"); });
  const today = new Date(2026, 9, 18);
  assert.equal(notifications.shouldPrompt(release, false, today), true);
  notifications.muteToday(today);
  assert.equal(notifications.shouldPrompt(release, false, today), false);
  assert.equal(notifications.shouldPrompt(release, true, today), true);
  assert.equal(notifications.shouldPrompt(release, false, new Date(2026, 9, 19)), true);
});
