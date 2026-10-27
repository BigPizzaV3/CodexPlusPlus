import assert from "node:assert/strict";
import test from "node:test";
import { rebaseSettingsEdits } from "./settings-rebase.ts";

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
