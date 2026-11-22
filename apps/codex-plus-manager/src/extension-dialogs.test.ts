import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");

function functionSource(source: string, name: string) {
  const start = source.indexOf(`function ${name}(`);
  assert.ok(start >= 0, name);
  const end = source.indexOf("\n}\n", start);
  return source.slice(start, end);
}

test("四个拓展弹窗都走 useModalShell：焦点圈在卡片里、Esc 可关、以标题命名，role 在卡片上", () => {
  const dialogs: Array<[string, string]> = [
    ["./CommunityExtensionsPanel.tsx", "InstallDialog"],
    ["./ExtensionCuration.tsx", "CurationReviewDialog"],
    ["./ExtensionPublishDialog.tsx", "ExtensionPublishDialog"],
    ["./MyPublishedPanel.tsx", "DeleteRepoDialog"],
  ];
  for (const [file, name] of dialogs) {
    const body = functionSource(read(file), name);
    assert.match(body, /const titleId = useId\(\);/, name);
    assert.match(body, /useModalShell<HTMLDivElement>\(\{.*labelledBy: titleId \}\);/, name);
    assert.match(body, /<div className="modal-backdrop app-dialog-backdrop" \{\.\.\.backdropProps\}>/, name);
    assert.match(body, /<div className="modal-card[^"]*"[^>]*\{\.\.\.dialogProps\}>/, name);
    assert.match(body, /<h2 id=\{titleId\}>/, name);
    // 遮罩上不再挂 role="dialog"，也不再各自手写 Esc / 遮罩点击。
    assert.doesNotMatch(body, /role="dialog"|useEscapeLayer|onMouseDown=/, name);
  }
});

test("社区拓展不能替换已装的官方 / 精选版本时，安装确认框只给「关闭」", () => {
  const body = functionSource(read("./CommunityExtensionsPanel.tsx"), "InstallDialog");
  // 与后端 extension_discovery.rs 的拒绝条件一致。
  assert.match(body, /const blocked = Boolean\(prepared\.conflict && prepared\.conflict\.source === "github" && prepared\.conflict\.channel\s*&& prepared\.conflict\.channel !== "community" && prepared\.channel === "community"\);/);
  assert.match(body, /\{blocked \? \(\n\s*<Button onClick=\{onCancel\} variant="secondary">\{t\("关闭"\)\}<\/Button>\n\s*\) : \(/);
  const backend = read("../../../crates/codex-plus-core/src/extension_discovery.rs");
  assert.match(backend, /if channel == CHANNEL_COMMUNITY \{\s*if let Some\(existing\) = state\.installs\.get\(&id\)\.filter\(\|record\| record\.channel != CHANNEL_COMMUNITY\)/);
});
