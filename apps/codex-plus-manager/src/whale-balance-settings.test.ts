import assert from "node:assert/strict";
import { test } from "node:test";
import { defaultWhaleBalanceSettings, normalizeWhaleBalanceSettings, whaleBalanceSettingsIssue } from "./whale-settings.ts";

const custom = () => ({ ...defaultWhaleBalanceSettings(), codexAppWhaleWidgetEnabled: true, codexAppWhaleBalanceProtocol: "custom" as const, codexAppWhaleBalancePath: "/account/balance", codexAppWhaleBalanceField: "data.balance" });

test("default and disabled widgets do not require an API balance configuration", () => {
  assert.equal(defaultWhaleBalanceSettings().codexAppWhaleWidgetEnabled, false);
  assert.equal(whaleBalanceSettingsIssue(defaultWhaleBalanceSettings()), null);
  assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleWidgetEnabled: false, codexAppWhaleBalancePath: "" }), null);
  assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalanceProtocol: "off", codexAppWhaleBalancePath: "" }), null);
});

test("custom balance requests cannot select another origin or include credential query parameters", () => {
  for (const path of ["https://elsewhere.example/balance", "//elsewhere.example/balance", "/balance?key=private", "/balance#secret", "\\\\elsewhere.example", "/a\nb"]) {
    assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalancePath: path }), "path", path);
  }
  for (const path of ["/api/balance", "account/balance"]) assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalancePath: path }), null);
});

test("explicit currency, scale and JSON array paths retain the selected provider units", () => {
  const settings = normalizeWhaleBalanceSettings({ ...custom(), codexAppWhaleBalanceCurrency: " cny ", codexAppWhaleBalanceField: " data.accounts[0].balance ", codexAppWhaleBalanceScale: 0.000002 });
  assert.equal(whaleBalanceSettingsIssue(settings), null);
  assert.equal(settings.codexAppWhaleBalanceCurrency, "CNY");
  assert.equal(settings.codexAppWhaleBalanceScale, 0.000002);
  for (const scale of [0, -1, NaN, Infinity]) assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalanceScale: scale }), "scale");
  assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalanceCurrency: "quota" }), "currency");
  assert.equal(whaleBalanceSettingsIssue({ ...custom(), codexAppWhaleBalanceField: "data..amount" }), "field");
});
