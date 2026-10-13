import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

/// 取顶格书写的选择器规则体，空白按 \s+ 容忍，避免误匹配组合选择器。
function ruleBody(selector: string) {
  const pattern = selector
    .split(/\s+/)
    .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
    .join("\\s+");
  const match = styles.match(new RegExp(`^${pattern}\\s*\\{([^}]*)\\}`, "m"));
  assert.ok(match, `missing top-level rule ${selector}`);
  return match[1];
}

test("feature switch cards keep their own height and expanded blocks take a full row", () => {
  // 默认 align-items: stretch 会把同一行的卡片拉到行高，details 一展开左侧卡就被撑大。
  assert.match(ruleBody(".feature-switch-grid"), /align-items:\s*start/);

  // 「API 余额（可选）」这类可展开配置块整行独占，展开只往下长。
  assert.match(
    ruleBody(".enhance-feature-groups .feature-switch-grid > .settings-block"),
    /grid-column:\s*1\s*\/\s*-1/,
  );

  // 回归前提：该配置块在「挂件与桌宠」里排在开关卡之后，才能落到单独一行。
  const whaleGroup = app.slice(app.indexOf('t("挂件与桌宠")'), app.indexOf('</FeatureGroup>', app.indexOf('t("挂件与桌宠")')));
  assert.match(whaleGroup, /<FeatureToggle[^>]*t\("Codex 用量挂件"\)/);
  assert.match(whaleGroup, /<WhaleBalanceSettingsFields/);
  assert.ok(
    whaleGroup.indexOf("<WhaleBalanceSettingsFields") > whaleGroup.indexOf('t("活动进度条右侧布局")'),
    "余额配置块必须排在开关卡之后，否则会在网格里留下空洞",
  );
});
