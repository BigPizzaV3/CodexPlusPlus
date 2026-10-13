import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

/// 取顶格书写的选择器规则体，避免误匹配组合选择器。
function ruleBody(selector: string) {
  const pattern = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = styles.match(new RegExp(`^${pattern}\\s*\\{([^}]*)\\}`, "m"));
  assert.ok(match, `missing top-level rule ${selector}`);
  return match[1];
}

test("feature switch grid keeps cards at content height so an expanded block cannot stretch them", () => {
  // 同一行里 details 展开会把行高撑高；默认的 align-items: stretch 会把相邻的
  // 功能开关卡一起拉大（用量挂件卡曾被「API 余额（可选）」撑成两倍高）。
  assert.match(ruleBody(".feature-switch-grid"), /align-items:\s*start/);

  // 回归前提：可展开的余额配置块与用量挂件卡确实同处一个功能开关网格。
  const whaleGroup = app.slice(app.indexOf('t("挂件与桌宠")'), app.indexOf('t("活动进度条右侧布局")'));
  assert.match(whaleGroup, /<FeatureToggle[^>]*t\("Codex 用量挂件"\)/);
  assert.match(whaleGroup, /<WhaleBalanceSettingsFields/);
});
