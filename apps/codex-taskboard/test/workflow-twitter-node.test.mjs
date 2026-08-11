import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const board = await readFile(new URL("../web/src/components/WorkflowBoard.tsx", import.meta.url), "utf8");
const inspector = await readFile(new URL("../web/src/components/WorkflowInspector.tsx", import.meta.url), "utf8");
const node = await readFile(new URL("../web/src/components/WorkflowNode.tsx", import.meta.url), "utf8");
const picker = await readFile(new URL("../web/src/components/WorkflowStepPicker.tsx", import.meta.url), "utf8");
const catalog = await readFile(new URL("../web/src/components/workflowCatalog.ts", import.meta.url), "utf8");

test("Twitter publishing is a third-party workflow choice with the official transparent X logo", () => {
  assert.match(catalog, /import xLogo from "\.\.\/assets\/x-logo-black\.png"/);
  assert.match(
    catalog,
    /group: "第三方集成",[\s\S]*?title: "发布到 Twitter"[\s\S]*?kind: "twitter-post"[\s\S]*?logo: xLogo[\s\S]*?logoMonochrome: true/,
  );
  assert.match(picker, /filteredItems\.filter\(\(item\) => item\.group === group\)/);
  assert.match(picker, /<WorkflowMark[\s\S]*?logo=\{item\.data\.logo\}/);
  assert.doesNotMatch(catalog, /twitterUser|twitterAccount|twitterCredential|twitterToken/);
});

test("Twitter publishing owns only one shared node-data configuration field", () => {
  assert.match(node, /twitterPostContent\?: string/);
  assert.match(
    inspector,
    /data\.kind === "twitter-post"[\s\S]*?<h2>发布到 Twitter<\/h2>[\s\S]*?aria-label="Twitter 发布内容"[\s\S]*?value=\{data\.twitterPostContent \?\? ""\}[\s\S]*?onChange=\{\(event\) => onChange\(\{ twitterPostContent: event\.target\.value \}\)\}/,
  );
  assert.doesNotMatch(
    inspector,
    /data\.kind === "twitter-post"[\s\S]*?(账号|凭据|回复|线程|媒体上传)/,
  );
});

test("Twitter node title, summary and configured state derive from actual post content", () => {
  assert.match(
    catalog,
    /if \(data\.kind === "twitter-post"\)[\s\S]*?twitterPostContent\?\.trim\(\)[\s\S]*?尚未填写发布内容/,
  );
  assert.match(
    catalog,
    /if \(data\.kind === "twitter-post"\)[\s\S]*?formatActionTitle\(data\.title/,
  );
  assert.match(
    catalog,
    /if \(data\.kind === "twitter-post"\)[\s\S]*?Boolean\(data\.twitterPostContent\?\.trim\(\)\)/,
  );
});

test("Twitter post content follows the shared workspace autosave path", () => {
  assert.match(
    board,
    /node\.id === selectedNodeId[\s\S]*?data: \{ \.\.\.node\.data, \.\.\.changes \}/,
  );
  assert.match(board, /nodes: snapshot\.nodes/);
  assert.doesNotMatch(inspector, /localStorage|sessionStorage/);
});
