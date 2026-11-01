import assert from "node:assert/strict";
import test from "node:test";
import { customSkillGroupKey, defaultSkillGroupKey, groupSkills, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName } from "./skill-groups.ts";

const skill = (id: string, repoKey = "", bundled = false) => ({ id, repoKey, bundled });
const labels = { builtin: "Built-in", user: "User", source: (item: ReturnType<typeof skill>) => item.repoKey === "local" ? "Local import" : item.repoKey.split("@", 1)[0] };

test("empty custom groups and pinned groups remain visible without skills", () => {
  const groups = groupSkills([], [], ["Empty"], {}, [], labels);
  assert.deepEqual(groups.map((group) => group.key), ["builtin", "user", "custom:Empty"]);
  assert.deepEqual(groups.map((group) => group.items.length), [0, 0, 0]);
  assert.deepEqual(groupSkills([], [], ["Empty"], {}, [], labels, false), []);
});

test("known repository groups remain empty after their skills move, but disappear while filtering", () => {
  const items = [skill("a", "owner/repo@main:skills"), skill("b", "owner/repo@dev")];
  const groups = groupSkills(items, items, ["Manual"], { a: "Manual", b: "Manual" }, [], labels, true,
    { "repo:owner/repo": "Alias" });
  assert.deepEqual(groups.find((group) => group.key === "repo:owner/repo")?.items, []);
  assert.equal(groups.find((group) => group.key === "repo:owner/repo")?.label, "Alias");
  assert.equal(groups.find((group) => group.key === "custom:Manual")?.items.length, 2);
  const filtered = groupSkills([items[0]], items, ["Manual"], { a: "Manual", b: "Manual" }, [], labels, false);
  assert.deepEqual(filtered.map((group) => group.key), ["custom:Manual"]);
  assert.deepEqual(groupSkills([], [skill("built", "hidden/repo@main", true), skill("local", "local")], [], {}, [], labels)
    .map((group) => group.key), ["builtin", "user"]);
});

test("configured repositories remain visible without scanned skills and are grouped by owner and name", () => {
  const repos = [
    { owner: "owner", name: "repo", branch: "main", subdir: "", enabled: true },
    { owner: "owner", name: "repo", branch: "dev", subdir: "skills", enabled: true },
    { owner: "other", name: "empty", branch: "main", subdir: "", enabled: false },
    { owner: "local", name: "real-repo", branch: "main", subdir: "", enabled: true },
    { owner: "", name: "local", branch: "", subdir: "", enabled: true },
    { owner: "cc-switch", name: "", branch: "", subdir: "", enabled: true },
    { owner: "invalid/owner", name: "repo", branch: "main", subdir: "", enabled: true },
  ];
  const imports = [skill("local-import", "local"), skill("cc-import", "cc-switch")];
  const groups = groupSkills(imports, imports, ["Manual"], {}, [], labels, true,
    { "repo:owner/repo": "Alias" }, {}, repos);
  assert.deepEqual(groups.map((group) => group.key), ["builtin", "user", "repo:owner/repo", "repo:local/real-repo", "custom:Manual", "repo:other/empty"]);
  assert.equal(groups.find((group) => group.key === "repo:owner/repo")?.label, "Alias");
  assert.deepEqual(groups.filter((group) => group.kind === "repo").map((group) => group.items.length), [0, 0, 0]);
  assert.deepEqual(groups.find((group) => group.key === "user")?.items, imports);
  assert.equal(groups.some((group) => group.key === "repo:local" || group.key === "repo:cc-switch"), false);
  assert.deepEqual(groupSkills(imports, imports, [], {}, [], labels, false, {}, {}, repos).map((group) => group.key), ["user"]);
});

test("configured and discovered sources of the same repository create only one group", () => {
  const item = skill("item", "owner/repo@main:skills");
  const repos = [{ owner: "owner", name: "repo", branch: "dev", subdir: "", enabled: true }];
  const groups = groupSkills([item], [item], [], {}, [], labels, true, {}, {}, repos);
  assert.equal(groups.filter((group) => group.key === "repo:owner/repo").length, 1);
  assert.deepEqual(groups.find((group) => group.key === "repo:owner/repo")?.items, [item]);
});

test("import channels default to the user group and both generations of custom assignments are preserved", () => {
  const items = [skill("builtin", "", true), skill("user"), skill("local", "local"), skill("cc", "cc-switch"),
    skill("repository", "owner/repo@main:skills"), skill("old"), skill("new"), skill("assigned-builtin", "", true)];
  const assignments = { old: "Manual", new: "custom:Manual", "assigned-builtin": "Manual" };
  const groups = groupSkills(items, items, ["Manual"], assignments, [], labels);
  assert.equal(defaultSkillGroupKey(items[0]), "builtin");
  assert.equal(defaultSkillGroupKey(items[1]), "user");
  assert.deepEqual(groups.find((group) => group.key === "user")?.items.map((item) => item.id), ["user", "local", "cc"]);
  assert.equal(groups.some((group) => group.key === "repo:local" || group.key === "repo:cc-switch"), false);
  assert.deepEqual(groups.find((group) => group.key === "repo:owner/repo")?.items.map((item) => item.id), ["repository"]);
  assert.deepEqual(groups.find((group) => group.key === "custom:Manual")?.items.map((item) => item.id), ["old", "new"]);
  assert.deepEqual(groups.find((group) => group.key === "builtin")?.items.map((item) => item.id), ["builtin", "assigned-builtin"]);
  assert.deepEqual(groups.slice(0, 2).map((group) => group.key), ["builtin", "user"]);
});

test("only valid repository keys create repository groups", () => {
  for (const repoKey of ["", "local", "cc-switch", "owner/repo", "owner/repo@", "/repo@main", "owner/@main"]) {
    assert.equal(defaultSkillGroupKey(skill("id", repoKey)), "user", repoKey);
  }
  assert.equal(defaultSkillGroupKey(skill("id", "owner/repo@main")), "repo:owner/repo");
  assert.equal(defaultSkillGroupKey(skill("id", "owner/repo@main:skills/nested")), "repo:owner/repo");
});

test("old and new assignments never move bundled skills, including a shared ID", () => {
  const bundled = skill("shared", "owner/bundled@main", true);
  const imported = skill("shared", "local");
  for (const assignment of ["Manual", "custom:Manual", "user", "repo:other/repo"]) {
    const stored = readSkillGroupStorage(JSON.stringify({ names: ["Manual"], assignments: { shared: assignment } }));
    for (const items of [[bundled, imported], [imported, bundled]]) {
      const groups = groupSkills(items, items, stored.names, stored.assignments, [], labels);
      assert.deepEqual(groups.find((group) => group.key === "builtin")?.items, [bundled]);
      const target = assignment === "Manual" ? "custom:Manual" : assignment;
      assert.ok(groups.find((group) => group.key === target)?.items.includes(imported));
      assert.deepEqual(stored.assignments, { shared: assignment });
    }
  }
});

test("same-ID non-bundled entries keep the legacy ID-level assignment across input order", () => {
  const first = skill("shared", "first/repo@main");
  const second = skill("shared", "second/repo@dev:skills");
  for (const items of [[first, second], [second, first]]) {
    const defaults = groupSkills(items, items, ["Manual"], {}, [], labels);
    assert.deepEqual(defaults.find((group) => group.key === "repo:first/repo")?.items, [first]);
    assert.deepEqual(defaults.find((group) => group.key === "repo:second/repo")?.items, [second]);
    const stored = readSkillGroupStorage(JSON.stringify({ names: ["Manual"], assignments: { shared: "Manual" }, future: "keep" }));
    const assigned = groupSkills(items, items, stored.names, stored.assignments, [], labels);
    assert.deepEqual(new Set(assigned.find((group) => group.key === "custom:Manual")?.items), new Set([first, second]));
    assert.deepEqual(stored.assignments, { shared: "Manual" });
    assert.equal(stored.future, "keep");
  }
});

test("tuple overrides are independent from old IDs and remain stable under reordering", () => {
  const first = skill("shared", "first/repo@main");
  const second = skill("shared", "second/repo@main");
  const bundled = skill("shared", "", true);
  const assignments = { shared: "Manual" };
  const overrides = { [skillIdentityKey(first)]: "custom:Other", [skillIdentityKey(second)]: "" };
  for (const items of [[first, second, bundled], [bundled, second, first]]) {
    const groups = groupSkills(items, items, ["Manual", "Other"], assignments, [], labels, true, {}, overrides);
    assert.deepEqual(groups.find((group) => group.key === "custom:Other")?.items, [first]);
    assert.deepEqual(groups.find((group) => group.key === "repo:second/repo")?.items, [second]);
    assert.deepEqual(groups.find((group) => group.key === "builtin")?.items, [bundled]);
    assert.deepEqual(groups.find((group) => group.key === "custom:Manual")?.items, []);
  }
  assert.deepEqual(assignments, { shared: "Manual" });
});

test("identity keys cannot collide with other tuples, legacy IDs, or inherited property names", () => {
  const first = skill("b:c", "a");
  const second = skill("c", "a:b");
  assert.notEqual(skillIdentityKey(first), skillIdentityKey(second));
  const firstKey = skillIdentityKey(first);
  const sameTextAsKey = skill(firstKey, "local");
  const legacy = Object.fromEntries([[firstKey, "Legacy"], ["__proto__", "Legacy"], ["constructor", "Legacy"]]);
  const overrides = { [firstKey]: "custom:Other" };
  assert.equal(skillGroupAssignment(first, ["Legacy", "Other"], legacy, overrides), "custom:Other");
  assert.equal(skillGroupAssignment(sameTextAsKey, ["Legacy", "Other"], legacy, overrides), "custom:Legacy");
  assert.equal(skillGroupAssignment(skill("__proto__", "local"), ["Legacy"], legacy, {}), "custom:Legacy");
  assert.equal(skillGroupAssignment(skill("constructor", "local"), ["Legacy"], legacy, {}), "custom:Legacy");
  assert.equal(skillGroupAssignment(skill("toString", "local"), ["Legacy"], {}, {}), "");
});

test("changed backend identity does not silently inherit another tuple's override", () => {
  const original = skill("shared", "owner/repo@main");
  const changed = skill("shared", "owner/repo@other");
  const legacy = { shared: "Legacy" };
  const overrides = { [skillIdentityKey(original)]: "custom:Other" };
  assert.equal(skillGroupAssignment(original, ["Legacy", "Other"], legacy, overrides), "custom:Other");
  assert.equal(skillGroupAssignment(changed, ["Legacy", "Other"], legacy, overrides), "custom:Legacy");
  assert.deepEqual(overrides, { [skillIdentityKey(original)]: "custom:Other" });
});

test("legacy explicit assignments and source aliases keep their stable identities", () => {
  const items = [skill("first", "one/repo@main"), skill("second", "other/repo@main"), skill("third")];
  const groups = groupSkills(items, items, ["user"], { first: "repo:other/repo", second: "user", third: "custom:user" }, [], labels, true,
    { "repo:other/repo": "Renamed" });
  assert.equal(groups.find((group) => group.key === "repo:other/repo")?.label, "Renamed");
  assert.deepEqual(groups.find((group) => group.key === "repo:other/repo")?.items.map((item) => item.id), ["first"]);
  assert.deepEqual(groups.find((group) => group.key === "custom:user")?.items.map((item) => item.id), ["second", "third"]);
});

test("name directions and manual order never move the pinned groups", () => {
  const items = [skill("a", "a/repo@main"), skill("z", "z/repo@main")];
  const groups = groupSkills(items, items, ["middle"], {}, ["repo:z/repo", "custom:middle", "repo:a/repo"], labels);
  assert.deepEqual(groups.map((group) => group.key), ["builtin", "user", "repo:z/repo", "custom:middle", "repo:a/repo"]);
  assert.deepEqual(sortSkillGroupKeys(groups, "asc"), ["repo:a/repo", "custom:middle", "repo:z/repo"]);
  assert.deepEqual(sortSkillGroupKeys(groups, "desc"), ["repo:z/repo", "custom:middle", "repo:a/repo"]);
  assert.deepEqual(sortSkillGroupsByName(groups, "desc").slice(0, 2).map((group) => group.key), ["builtin", "user"]);
  assert.deepEqual(moveSkillGroup(groups, "repo:a/repo", "repo:z/repo"), ["repo:a/repo", "repo:z/repo", "custom:middle"]);
  assert.deepEqual(moveSkillGroup(groups, "builtin", "repo:z/repo"), ["repo:z/repo", "custom:middle", "repo:a/repo"]);
  assert.equal(nextSkillGroupSortDirection(null), "asc");
  assert.equal(nextSkillGroupSortDirection("asc"), "desc");
  assert.equal(nextSkillGroupSortDirection("desc"), "asc");
  assert.equal(customSkillGroupKey("middle"), "custom:middle");
});

test("storage reads old data and retains unknown fields while rejecting malformed settings", () => {
  const restored = readSkillGroupStorage(JSON.stringify({ names: ["Empty", "", 4], assignments: { id: "Empty", bad: 3 },
    assignmentsByIdentity: { '["local","id"]': "", bad: false }, order: ["custom:Empty", false], sortDirection: "desc", aliases: { "repo:a": "Alias" }, future: { keep: true } }));
  assert.deepEqual(restored.names, ["Empty"]);
  assert.deepEqual(restored.assignments, { id: "Empty" });
  assert.deepEqual(restored.assignmentsByIdentity, { '["local","id"]': "" });
  assert.deepEqual(restored.order, ["custom:Empty"]);
  assert.equal(restored.sortDirection, "desc");
  assert.deepEqual(restored.aliases, { "repo:a": "Alias" });
  assert.deepEqual(restored.future, { keep: true });
  assert.deepEqual(readSkillGroupStorage("broken"), { names: [], assignments: {}, assignmentsByIdentity: {}, order: [], sortDirection: null, aliases: {} });
});
