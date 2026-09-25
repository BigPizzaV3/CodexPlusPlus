import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

const packageJson = JSON.parse(
  await readFile(new URL("../package.json", import.meta.url), "utf8"),
);
const detailSource = await readFile(
  new URL("../web/src/components/TaskDetail.tsx", import.meta.url),
  "utf8",
);
const styles = await readFile(
  new URL("../web/src/styles.css", import.meta.url),
  "utf8",
);

test("issue detail renders descriptions and comments with GFM markdown", () => {
  assert.equal(typeof packageJson.dependencies["react-markdown"], "string");
  assert.equal(typeof packageJson.dependencies["remark-gfm"], "string");
  assert.match(detailSource, /import ReactMarkdown from "react-markdown";/);
  assert.match(detailSource, /import remarkGfm from "remark-gfm";/);
  assert.match(
    detailSource,
    /<ReactMarkdown[\s\S]*remarkPlugins=\{\[remarkGfm\]\}[\s\S]*>\s*\{value\}\s*<\/ReactMarkdown>/,
  );
  assert.match(
    detailSource,
    /\{description \? <DescriptionDocument value=\{description\} \/> : "添加描述…"\}/,
  );
  assert.match(
    detailSource,
    /comment\.body && <div className="comment-body"><DescriptionDocument value=\{comment\.body\} \/><\/div>/,
  );
  assert.doesNotMatch(detailSource, /value\.split\("\\n"\)/);
});

test("issue detail markdown styles cover rich document elements", () => {
  for (const selector of [
    ".issue-description-document blockquote",
    ".issue-description-document pre",
    ".issue-description-document table",
    ".issue-description-document a",
    ".issue-description-document img",
    ".issue-description-document input[type=\"checkbox\"]",
  ]) {
    assert.match(styles, new RegExp(selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }
});

test("the configured markdown renderer produces CommonMark and GFM elements", () => {
  const markdown = [
    "**粗体**和[链接](https://example.com)",
    "",
    "> 引用",
    "",
    "- [x] 已完成",
    "- [ ] 未完成",
    "",
    "~~删除线~~",
    "",
    "| 名称 | 状态 |",
    "| --- | --- |",
    "| Taskboard | Ready |",
    "",
    "```js",
    "const ready = true;",
    "```",
  ].join("\n");
  const html = renderToStaticMarkup(
    createElement(ReactMarkdown, { remarkPlugins: [remarkGfm] }, markdown),
  );

  for (const element of ["strong", "a", "blockquote", "input", "del", "table", "pre", "code"]) {
    assert.match(html, new RegExp(`<${element}(?: |>)`));
  }
});
