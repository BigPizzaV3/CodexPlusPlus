import assert from "node:assert/strict";
import test from "node:test";
import { mergeRotatedAuthContents, rebaseSettingsEdits } from "./settings-rebase.ts";

type Settings = { revision: string; a: boolean; b: boolean; list: string[] };

test("没有本地编辑时直接采用外部改动后的最新设置", () => {
  const previous: Settings = { revision: "r1", a: true, b: false, list: ["x"] };
  const latest: Settings = { revision: "r2", a: false, b: false, list: ["x"] };
  assert.deepEqual(rebaseSettingsEdits({ ...previous }, previous, latest), latest);
});

test("保留本地未保存的编辑，其余字段与 revision 取最新", () => {
  const previous: Settings = { revision: "r1", a: true, b: false, list: ["x"] };
  const form: Settings = { ...previous, b: true, list: ["x", "y"] };
  const latest: Settings = { revision: "r2", a: false, b: false, list: ["x"] };
  assert.deepEqual(rebaseSettingsEdits(form, previous, latest), { revision: "r2", a: false, b: true, list: ["x", "y"] });
});

test("两边改了同一项时以本地编辑为准，交给随后的自动保存写回", () => {
  const previous: Settings = { revision: "r1", a: true, b: false, list: [] };
  const form: Settings = { ...previous, a: false };
  const latest: Settings = { revision: "r2", a: true, b: true, list: [] };
  assert.deepEqual(rebaseSettingsEdits(form, previous, latest), { revision: "r2", a: false, b: true, list: [] });
});

type ProfileSettings = {
  revision: string;
  relayProfiles: { id: string; name: string; authContents: string; apiKey: string }[];
};

const profileBase = { id: "p1", name: "官方", authContents: "auth-old", apiKey: "sk-1" };

test("外部只轮换 token 时，用户编辑的供应商列表采用最新 authContents", () => {
  const previous: ProfileSettings = { revision: "r1", relayProfiles: [profileBase] };
  // 用户改了 apiKey，authContents 保持旧值；外部把 token 轮换成 auth-new。
  const form: ProfileSettings = {
    revision: "r1",
    relayProfiles: [{ ...profileBase, apiKey: "sk-2" }],
  };
  const latest: ProfileSettings = {
    revision: "r2",
    relayProfiles: [{ ...profileBase, authContents: "auth-new" }],
  };
  const merged = mergeRotatedAuthContents(form, previous, latest);
  assert.deepEqual(merged.relayProfiles, [
    { ...profileBase, apiKey: "sk-2", authContents: "auth-new" },
  ]);
});

test("用户自己粘贴过新凭据的 profile 不被外部 token 覆盖", () => {
  const previous: ProfileSettings = { revision: "r1", relayProfiles: [profileBase] };
  const form: ProfileSettings = {
    revision: "r1",
    relayProfiles: [{ ...profileBase, authContents: "auth-user-pasted" }],
  };
  const latest: ProfileSettings = {
    revision: "r2",
    relayProfiles: [{ ...profileBase, authContents: "auth-new" }],
  };
  const merged = mergeRotatedAuthContents(form, previous, latest);
  assert.equal(merged.relayProfiles[0].authContents, "auth-user-pasted");
});

test("外部没有动 authContents 时合并结果保持不变", () => {
  const previous: ProfileSettings = { revision: "r1", relayProfiles: [profileBase] };
  const form: ProfileSettings = {
    revision: "r1",
    relayProfiles: [{ ...profileBase, apiKey: "sk-2" }],
  };
  const latest: ProfileSettings = {
    revision: "r2",
    relayProfiles: [profileBase],
  };
  const merged = mergeRotatedAuthContents(form, previous, latest);
  assert.deepEqual(merged, form);
});
