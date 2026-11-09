import assert from "node:assert/strict";
import test from "node:test";
import { nativeAuthSelection, savedAuthLabel, type NativeAuthState } from "./native-auth-model.ts";

const state: NativeAuthState = { managed: false, selection: null, actualAccountRef: "a", actualLabel: "A", accounts: [], drift: null };

test("已发现的实际登录不被自动当作用户的明确选择", () => {
  assert.equal(nativeAuthSelection(state), "");
});

test("原生无登录选择和具体账号选择明确分离", () => {
  assert.equal(nativeAuthSelection({ ...state, managed: true, selection: { mode: "none" } }), "none");
  assert.equal(nativeAuthSelection({ ...state, managed: true, selection: { mode: "account", accountRef: "a" } }), "account:a");
  assert.equal(nativeAuthSelection({ ...state, managed: true, selection: { mode: "account", accountRef: "b" } }), "account:b");
});

test("已保存账号显示工作区以区分同名账号", () => {
  assert.equal(savedAuthLabel({ id: "a", name: "A", email: "a@example.test", workspaceId: "workspace-1", source: "gateway" }), "A · a@example.test · workspace-1");
});
