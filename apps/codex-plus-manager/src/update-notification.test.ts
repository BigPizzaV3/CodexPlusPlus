import assert from "node:assert/strict";
import test from "node:test";
import { UPDATE_MUTED_DAY_KEY, UpdateNotifications, isVersionSkipped, updatePreferencesFrom } from "./update-notification.ts";

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

test("skipping a version persists through saved preferences and only silences that version or older", () => {
  const saved = storage();
  const notifications = new UpdateNotifications(() => saved);
  assert.equal(notifications.preferencesLoaded, false);
  assert.equal(notifications.autoCheckEnabled(), true, "unknown preferences keep the default automatic check");
  notifications.skipVersion(release.latestVersion);
  assert.equal(notifications.shouldPrompt(release), false);
  assert.equal(notifications.shouldPrompt(release, true), true, "manual checks still show the skipped release");
  assert.equal(notifications.shouldPrompt({ ...release, latestVersion: "v1.11.1" }), true);
  assert.equal(notifications.shouldPrompt({ ...release, latestVersion: "1.12" }), true);

  // 重启后从 settings.json 读回同样的偏好。
  const restarted = new UpdateNotifications(() => saved);
  restarted.setPreferences(updatePreferencesFrom({ updateAutoCheck: true, updateSkippedVersion: "v1.11.0" }));
  assert.equal(restarted.preferencesLoaded, true);
  assert.equal(restarted.shouldPrompt(release), false);
  assert.equal(restarted.shouldPrompt({ ...release, latestVersion: "v1.10.9" }), false);
  assert.equal(restarted.shouldPrompt({ ...release, latestVersion: "v1.12.0" }), true);
  restarted.setPreferences(updatePreferencesFrom({ updateAutoCheck: true, updateSkippedVersion: "" }));
  assert.equal(restarted.shouldPrompt(release), true, "clearing the skipped version restores reminders");
});

test("update preferences default to automatic checks for old settings and parse versions like the backend", () => {
  assert.deepEqual(updatePreferencesFrom(undefined), { autoCheck: true, skippedVersion: "" });
  assert.deepEqual(updatePreferencesFrom({}), { autoCheck: true, skippedVersion: "" });
  assert.deepEqual(updatePreferencesFrom({ updateAutoCheck: false, updateSkippedVersion: " v2.0.0 " }), { autoCheck: false, skippedVersion: "v2.0.0" });
  const saved = storage();
  const notifications = new UpdateNotifications(() => saved);
  notifications.setPreferences({ autoCheck: false, skippedVersion: "" });
  assert.equal(notifications.autoCheckEnabled(), false);
  notifications.skipVersion("v1.0.0");
  assert.equal(notifications.autoCheckEnabled(), false, "skipping keeps the auto-check switch untouched");

  assert.equal(isVersionSkipped("v1.11.0", ""), false);
  assert.equal(isVersionSkipped("v1.11.0", "1.11"), true);
  assert.equal(isVersionSkipped("V1.11.0-beta", "v1.11.0"), true);
  assert.equal(isVersionSkipped("v1.11.10", "v1.11.9"), false);
  assert.equal(isVersionSkipped("nightly", "nightly"), true);
  assert.equal(isVersionSkipped("nightly-2", "nightly"), false);
});
