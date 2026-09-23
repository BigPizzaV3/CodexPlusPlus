import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

import {
  buildIssueUrl,
  readIssueIdentifier,
} from "../web/src/issueRoute.ts";

const appSource = await readFile(new URL("../web/src/App.tsx", import.meta.url), "utf8");

test("issue detail URLs preserve project context and unrelated query parameters", () => {
  const url = buildIssueUrl(
    "http://127.0.0.1:47823/?host=codex&project=other&status=todo",
    "local",
    "LOCAL-72",
  );

  assert.equal(url.searchParams.get("project"), "local");
  assert.equal(url.searchParams.get("issue"), "LOCAL-72");
  assert.equal(url.searchParams.get("host"), "codex");
  assert.equal(url.searchParams.get("status"), "todo");
  assert.equal(readIssueIdentifier(url.search), "LOCAL-72");
});

test("closing issue detail removes only the issue route", () => {
  const url = buildIssueUrl(
    "http://127.0.0.1:47823/?host=codex&project=local&issue=LOCAL-72&label=缺陷",
    "local",
    null,
  );

  assert.equal(url.searchParams.has("issue"), false);
  assert.equal(url.searchParams.get("project"), "local");
  assert.equal(url.searchParams.get("host"), "codex");
  assert.deepEqual(url.searchParams.getAll("label"), ["缺陷"]);
});

test("the app restores issue detail from the URL and follows browser history", () => {
  assert.match(
    appSource,
    /useState<string \| null>\(\s*\(\) => readIssueIdentifier\(window\.location\.search\),?\s*\)/,
  );
  assert.match(appSource, /task\.identifier === detailTaskIdentifier/);
  assert.match(appSource, /window\.addEventListener\("popstate", syncRouteFromLocation\)/);
  assert.match(appSource, /window\.removeEventListener\("popstate", syncRouteFromLocation\)/);
  const openTaskSource = appSource.slice(
    appSource.indexOf("function openTaskDetail"),
    appSource.indexOf("function closeTaskDetail"),
  );
  assert.match(openTaskSource, /buildIssueUrl\(window\.location\.href, task\.projectId, null\)/);
  assert.match(openTaskSource, /window\.history\.replaceState/);
  assert.match(openTaskSource, /window\.history\.pushState/);
  assert.match(appSource, /function closeTaskDetail\(\)[\s\S]*?window\.history\.replaceState/);
  assert.match(appSource, /onEdit=\{openTaskDetail\}/);
});
