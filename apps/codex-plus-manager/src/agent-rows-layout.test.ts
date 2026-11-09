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

test("Codex 行有启动 / 停止键：只有主窗口传了 codexRun 才显示，缺省不显示", () => {
  // 主窗口把侧栏那份运行时对象透传下来；总览里的接入弹窗不传，就只留模型与开关。
  assert.match(source, /export type CodexRunControls = \{/);
  assert.match(source, /codexRun\?: CodexRunControls;/);
  assert.match(source, /\{isCodex && codexRun \? \(/);
  assert.match(source, /aria-label=\{codexRun\.running \? t\("停止 Codex"\) : t\("启动 Codex"\)\}/);
  // 未运行 Play「启动 Codex」、运行中 Square「停止 Codex」、busy 转圈；不可用时置灰并把原因放进 title。
  assert.match(source, /disabled=\{codexRun\.busy \|\| \(!codexRun\.running && Boolean\(codexRun\.unavailableReason\)\)\}/);
  assert.match(source, /title=\{codexRun\.busy \? t\("处理中…"\) : codexRun\.running \? t\("停止 Codex"\) : codexRun\.unavailableReason \?\? t\("启动 Codex"\)\}/);
  assert.match(source, /\(codexRun\.running \? codexRun\.onStop : codexRun\.onStart\)\(\);/);
  assert.match(source, /<Switch disabled=\{busy !== null\} label=\{tf\("\{0\} 接入 Codex\+\+"/);
  // 和 Claude 行同一个位置：模型选择器和接入开关之间。
  const controls = source.slice(source.indexOf('<div className="cx-row-controls">'), source.indexOf("<Switch", source.indexOf('<div className="cx-row-controls">')));
  assert.match(controls, /<ModelPicker/);
  assert.match(controls, /isCodex && codexRun/);
  assert.match(controls, /canLaunchClaudeDesktop && claudeRun/);
});

test("Claude Desktop 行有本地启动 / 停止键，其它 Agent 不显示", () => {
  // 只有 Claude Desktop 有可启停的本地启动器；Codex 的启动入口在侧栏和总览，Grok 没有本地启动器。
  // 绑定 id 是 `claude-desktop`（`claude` 是 Claude Code 命令行），两者都要认成 Claude。
  assert.match(source, /const isClaude = agent\.agent === "claude" \|\| agent\.agent === "claude-desktop";/);
  assert.match(source, /const canLaunchClaudeDesktop = agent\.agent === "claude-desktop";/);
  assert.match(source, /\{canLaunchClaudeDesktop && claudeRun \? \(/);
  assert.doesNotMatch(source, /\{isClaude && claudeRun \? \(/, "Claude Code 行不该出现本地启动键");
  assert.match(source, /if \(canLaunchClaudeDesktop\) void refreshClaudeRun\(\);/);
  assert.match(source, /className="gw-icon-button gw-agent-run"/);
  assert.match(source, /data-running=\{claudeRun\.running\}/);
  assert.match(source, /invoke<ClaudeLocalRunStatus>\("claude_local_status"\)/);
  assert.match(source, /invoke<ClaudeLocalRunStatus>\("claude_local_action", \{ action \}\)/);
  // 放在模型选择器和接入开关之间，和页面里其它图标按钮同尺寸。
  const controls = source.slice(source.indexOf('<div className="cx-row-controls">'), source.indexOf("<Switch", source.indexOf('<div className="cx-row-controls">')));
  assert.match(controls, /<ModelPicker/);
  assert.match(controls, /gw-agent-run/);
  assert.doesNotMatch(source, /agent\.agent === "grok" && claudeRun/);
  // 失败走页面已有的 notify，不自己弹 alert。
  assert.match(source, /await notify\(t\("Claude 本地模式"\), result\.message \?\? t\("操作失败"\), "failed"\)/);
});
