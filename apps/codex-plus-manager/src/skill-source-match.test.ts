import assert from "node:assert/strict";
import { test } from "node:test";
import { groupSourceMatches, preferredSourceMatch, selectedSourceMatches, sourceMatchKey, sourceMatchLabel, type SkillSourceMatch } from "./skill-source-match.ts";

const match = (localId: string, repoPath: string, exact: boolean): SkillSourceMatch => ({ localId, localName: localId, localHash: "local", snapshot: "snapshot", repoKey: "owner/repo@main", repoPath, remoteName: repoPath, remoteHash: "remote", exact });

test("source labels distinguish branches for the same repository path", () => {
  const main = match("a", "skills/alpha", true);
  const dev = { ...main, repoKey: "owner/repo@feature/dev" };
  assert.equal(sourceMatchLabel(main), "owner/repo@main / skills/alpha");
  assert.equal(sourceMatchLabel(dev), "owner/repo@feature/dev / skills/alpha");
  assert.notEqual(sourceMatchLabel(main), sourceMatchLabel(dev));
});

test("source labels preserve configured subdirectories and the full skill path", () => {
  const curated = { ...match("a", "skills/.curated/alpha", true), repoKey: "owner/repo@main:skills/.curated" };
  const all = { ...curated, repoKey: "owner/repo@main:skills" };
  assert.equal(sourceMatchLabel(curated), "owner/repo@main:skills/.curated / skills/.curated/alpha");
  assert.equal(sourceMatchLabel(all), "owner/repo@main:skills / skills/.curated/alpha");
  assert.notEqual(sourceMatchLabel(curated), sourceMatchLabel(all));
});

test("prefers a unique content match without guessing among multiple exact candidates", () => {
  const exact = match("a", "one", true);
  const weak = match("a", "two", false);
  assert.equal(preferredSourceMatch([weak, exact]), sourceMatchKey(exact));
  assert.equal(preferredSourceMatch([exact, match("a", "three", true)]), "");
  assert.equal(preferredSourceMatch([weak, match("a", "three", false)]), "");
});

test("rejects missing selections and assigning one repository path to multiple skills", () => {
  const a = match("a", "one", true);
  const b = match("b", "one", true);
  const groups = groupSourceMatches([a, b]);
  assert.equal(selectedSourceMatches(groups, { a: "" }), null);
  assert.equal(selectedSourceMatches(groups, { a: sourceMatchKey(a), b: sourceMatchKey(b) }), null);
  assert.deepEqual(selectedSourceMatches(groups, { a: sourceMatchKey(a) }), [a]);
});

test("rejects duplicate origins across configured subdirectories without changing selection keys", () => {
  const root = match("a", "skills/.curated/alpha", true);
  const subdir = { ...match("b", root.repoPath, true), repoKey: "Owner/Repo@main:skills" };
  const nested = { ...match("c", root.repoPath, true), repoKey: "owner/repo@main:skills/.curated" };
  const groups = groupSourceMatches([root, subdir, nested]);

  assert.notEqual(sourceMatchKey(root), sourceMatchKey(subdir));
  assert.equal(selectedSourceMatches(groups, { a: sourceMatchKey(root), b: sourceMatchKey(subdir) }), null);
  assert.equal(selectedSourceMatches(groups, { b: sourceMatchKey(subdir), c: sourceMatchKey(nested) }), null);
  assert.deepEqual(selectedSourceMatches(groups, { b: sourceMatchKey(subdir) }), [subdir]);
});

test("keeps different repositories, branches, and case-sensitive paths independent", () => {
  const root = match("a", "skills/alpha", true);
  const distinct = [
    { ...match("b", "skills/beta", true), repoKey: "owner/repo@main:skills" },
    { ...match("c", "skills/Alpha", true), repoKey: "owner/repo@main:skills" },
    { ...match("d", root.repoPath, true), repoKey: "owner/repo@Main:skills" },
    { ...match("e", root.repoPath, true), repoKey: "owner/another@main:skills" },
  ];
  const groups = groupSourceMatches([root, ...distinct]);
  const selected = Object.fromEntries([root, ...distinct].map((item) => [item.localId, sourceMatchKey(item)]));
  assert.deepEqual(selectedSourceMatches(groups, selected), [root, ...distinct]);
});

test("recognizes legacy default branches but leaves unknown source keys distinct", () => {
  const legacy = { ...match("a", "skills/alpha", true), repoKey: "OWNER/REPO" };
  const configured = { ...match("b", legacy.repoPath, true), repoKey: "owner/repo@main:skills" };
  const local = { ...match("c", legacy.repoPath, true), repoKey: "local" };
  const other = { ...match("d", legacy.repoPath, true), repoKey: "cc-switch" };
  const groups = groupSourceMatches([legacy, configured, local, other]);
  assert.equal(selectedSourceMatches(groups, { a: sourceMatchKey(legacy), b: sourceMatchKey(configured) }), null);
  assert.deepEqual(selectedSourceMatches(groups, { c: sourceMatchKey(local), d: sourceMatchKey(other) }), [local, other]);
});

test("deduplicates repository candidates and keeps exact matches first", () => {
  const weak = match("a", "two", false);
  const exact = match("a", "one", true);
  assert.deepEqual(groupSourceMatches([weak, exact, exact]).get("a"), [exact, weak]);
});
