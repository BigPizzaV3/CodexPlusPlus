import assert from "node:assert/strict";
import { test } from "node:test";
import { defaultWhaleBalanceSettings, normalizeWhaleBalanceSettings, whaleBalanceSettingsIssue } from "./whale-settings.ts";

const custom = () => ({ ...defaultWhaleBalanceSettings(), codexAppWhaleBalanceProtocol: "custom" as const, codexAppWhaleBalancePath: "/account/balance", codexAppWhaleBalanceField: "data.balance" });

test("default and disabled widgets do not require an API balance configuration", () => {
  assert.equal(whaleBalanceSettingsIssue(defaultWhaleBalanceSettings(), true), null);
  assert.equal(whaleBalanceSettingsIssue(defaultWhaleBalanceSettings(), false), null);
  // 没选大肥鱼（或桌宠关着）时，填了一半的自定义接口不拦保存。
  assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalancePath: "" }, false), null);
  assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalanceProtocol: "off", codexAppWhaleBalancePath: "" }, true), null);
});

test("custom balance requests cannot select another origin or include credential query parameters", () => {
  for (const path of ["https://elsewhere.example/balance", "//elsewhere.example/balance", "/balance?key=private", "/balance#secret", "\\\\elsewhere.example", "/a\nb"]) {
    assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalancePath: path }, true), "path", path);
  }
  for (const path of ["/api/balance", "account/balance"]) assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalancePath: path }, true), null);
});

test("explicit currency, scale and JSON array paths retain the selected provider units", () => {
  const settings = normalizeWhaleBalanceSettings({ ...custom(), codexAppWhaleBalanceCurrency: " cny ", codexAppWhaleBalanceField: " data.accounts[0].balance ", codexAppWhaleBalanceScale: 0.000002 });
  assert.equal(whaleBalanceSettingsIssue(settings, true), null);
  assert.equal(settings.codexAppWhaleBalanceCurrency, "CNY");
  assert.equal(settings.codexAppWhaleBalanceScale, 0.000002);
  for (const scale of [0, -1, NaN, Infinity]) assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalanceScale: scale }, true), "scale");
  assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalanceCurrency: "quota" }, true), "currency");
  assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalanceField: "data..amount" }, true), "field");
});
