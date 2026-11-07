import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8");

test("Agent 接入：状态用短标签、整行点击展开", () => {
  const row = source.slice(source.indexOf('className="gw-agent-row cx-row"'), source.indexOf("<ModelPicker", source.indexOf('className="gw-agent-row cx-row"')));
  assert.match(row, /className="gw-agent-badge"/);
  assert.match(row, /onToggleExpand\(\);/);
  assert.doesNotMatch(source, /打开「接入」后|的模型列表里有 \{1\} 个模型|tf\("模型 \{0\}"/);
});

test("模型下拉底部只有一个「用默认（不经 Codex++）」：它就是断开并还原，不再重复放一个同效果的按钮", () => {
  const source = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(source, /t\("断开并还原配置"\)/);
  assert.equal(source.match(/onClick=\{onDisconnect\}/g)?.length, 2, "下拉里一个、展开详情里一个");
});

test("Agent 默认模型标签属于选择按钮，配置路径使用当前 Agent 定位", () => {
  const picker = source.slice(source.indexOf("function ModelPicker("), source.indexOf("/** 没接入时 Agent"));
  assert.match(picker, /<button[^>]*className="gw-picker-button"[^>]*>[\s\S]*?<span className="gw-picker-label">\{t\("默认模型"\)\}<\/span>/);
  const revealButton = source.split("\n").find((line) => line.includes('aria-label={t("在文件管理器中显示配置文件")}')) ?? "";
  assert.match(revealButton, /"reveal_gateway_agent_config", \{ agent: agent\.agent \}/);
  assert.match(source, /className="gw-detail-label gw-detail-file" title=\{agent\.configPath \?\? ""\}/);
  assert.match(readFileSync(new URL("./i18n-en.ts", import.meta.url), "utf8"), /"在文件管理器中显示配置文件": "Show configuration file in file manager"/);
});

test("详情折叠标题保留原生交互，路径与模型选择器可伸缩", () => {
  const providers = readFileSync(new URL("./GatewayProviders.tsx", import.meta.url), "utf8");
  const styles = readFileSync(new URL("./cx-pages-gateway.css", import.meta.url), "utf8");
  assert.match(source, /<details className="gw-detail-advanced">\s*<summary>/);
  assert.match(providers, /<details className="gw-detail-advanced gw-pool-more">\s*<summary>/);
  assert.match(styles, /gw-detail-advanced > summary svg \{[^}]*right: 100%;/);
  assert.match(styles, /gw-detail-row\.gw-detail-foot \{[^}]*grid-template-columns: minmax\(0, 1fr\) auto;/);
  assert.match(styles, /gw-agent-row\.cx-row \{[^}]*flex-wrap: wrap;/);
  assert.match(styles, /gw-picker-button > \.gw-picker-label \{[^}]*flex: none;/);
});
