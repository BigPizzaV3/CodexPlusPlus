import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { historyRecoveryStatusLabel } from "./history-recovery-report.ts";

test("session index repair report keeps abort and truncation notices outside issue details", async () => {
  const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  assert.match(source, /issuesTruncated\?: number;/);
  assert.match(source, /abortedReason\?: string \| null;/);
  assert.match(source, /warnings\?: string\[\];/);

  const reportStart = source.indexOf("{sessionIndexRepairReport ? (");
  const reportEnd = source.indexOf("function SessionsScreen", reportStart);
  const report = source.slice(reportStart, reportEnd);

  const abortNotice = report.indexOf('role="alert"');
  const warningNotice = report.indexOf("{sessionIndexRepairReport.warnings?.map");
  const issueDetails = report.indexOf('{sessionIndexRepairReport.issues.length ? (');
  const truncationNotice = report.indexOf('{sessionIndexRepairReport.issuesTruncated ? (');

  assert.ok(reportStart >= 0 && reportEnd > reportStart, "repair report markup should exist");
  assert.ok(abortNotice >= 0, "abortedReason should render as an alert");
  assert.match(report, /role="alert"[^>]*>.*sessionIndexRepairReport\.abortedReason/s);
  assert.ok(issueDetails > abortNotice, "abortedReason alert should stay outside the issue details disclosure");
  assert.ok(warningNotice > abortNotice, "warnings should render independently from abortedReason");
  assert.ok(issueDetails > warningNotice, "warnings should stay outside the issue details disclosure");
  assert.match(report, /role="alert"[^>]*><strong>\{t\("修复警告："\)\}<\/strong>\{warning\}/);
  assert.ok(truncationNotice > issueDetails, "truncation count should stay outside the issue details disclosure");
  assert.match(report, /tf\("另有 \{0\} 条检查详情因报告上限未显示。"/);
});

test("frontend recovery reports distinguish pending and every terminal result without database totals", async () => {
  for (const [status, expected] of [
    ["pending", "后台补载中"], ["completed", "已完成"], ["partial", "部分完成"],
    ["unavailable", "不可用"], ["skipped", "已跳过"],
  ] as const) {
    assert.equal(historyRecoveryStatusLabel({ status, loadedPages: 3, metadataPages: 2 }), expected);
  }
  const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const start = source.indexOf("{sessionIndexRepairReport.frontendRecovery ? (");
  const end = source.indexOf(") : null}", start);
  const report = source.slice(start, end);
  assert.ok(start >= 0);
  assert.match(report, /role="status"/);
  assert.match(report, /historyRecoveryStatusLabel\(sessionIndexRepairReport.frontendRecovery\)/);
  assert.match(report, /frontendRecovery\.loadedPages/);
  assert.match(report, /frontendRecovery\.metadataPages/);
  assert.doesNotMatch(report, /repairedItems|scannedFiles/);
});
