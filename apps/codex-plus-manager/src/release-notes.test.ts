import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ReleaseNotes } from "./ReleaseNotes.ts";

function render(markdown: string): string {
  return renderToStaticMarkup(createElement(ReleaseNotes, { markdown, onOpenUrl: () => {} }));
}

test("release headings, bold, lists and inline code render as Markdown", () => {
  const html = render("本次更新带来原生 Fast 解锁。\r\n\r\n### 新增\r\n\r\n- **原生 Fast 解锁**：支持 `priority`\r\n- *外部 Agent* 协作\r\n\r\n### 修复\r\n\r\n1. 更新说明\r\n2. 版本提示");
  assert.match(html, /<h3>新增<\/h3>/);
  assert.match(html, /<strong>原生 Fast 解锁<\/strong>/);
  assert.match(html, /<code>priority<\/code>/);
  assert.match(html, /<em>外部 Agent<\/em>/);
  assert.match(html, /<ul><li>/);
  assert.match(html, /<ol start="1"><li>/);
  assert.doesNotMatch(html, /###|\*\*/);
});

test("nested lists, quotes, tables, tasks and fenced code retain structure", () => {
  const html = render("- 外部 Agent\n  - **Codex**\n  - DeepSeek\n\n> 需要重启\n\n| 功能 | 状态 |\n| --- | --- |\n| Fast | 支持 |\n\n- [x] 完成\n\n```ts\nconst value = '<safe>';\n```\n\n---\n\n~~旧版~~");
  assert.match(html, /<ul><li><p>外部 Agent<\/p><ul>/);
  assert.match(html, /<blockquote><p>需要重启<\/p><\/blockquote>/);
  assert.match(html, /<table><thead><tr><th>功能<\/th>/);
  assert.match(html, /<td>支持<\/td>/);
  assert.match(html, /type="checkbox".*disabled="".*checked=""/);
  assert.match(html, /<pre><code>const value = &#x27;&lt;safe&gt;&#x27;;<\/code><\/pre>/);
  assert.match(html, /<hr\/>/);
  assert.match(html, /<del>旧版<\/del>/);
});

test("remote HTML stays text and links allow only HTTP(S)", () => {
  const html = render('<script>alert(1)</script>\n\n[文档](https://example.test/notes) [危险](javascript:alert) [本地](file:///tmp/secret) <https://example.test>\n\n![远端图片](https://example.test/pixel.png)');
  assert.match(html, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
  assert.match(html, /href="https:\/\/example.test\/notes"/);
  assert.doesNotMatch(html, /href="(?:javascript|file):|<script|<img/);
});

test("empty and unclosed Markdown still renders without dropping content", () => {
  assert.equal(render(""), '<div class="release-notes"></div>');
  assert.match(render("```\n**literal**\n<script>"), /<pre><code>\*\*literal\*\*\n&lt;script&gt;<\/code>/);
  assert.match(render("\\*literal\\*"), /<p>\*literal\*<\/p>/);
});
