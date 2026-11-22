import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { it } from "node:test";
import { EN_PLAIN, EN_TEMPLATE } from "./i18n-en.ts";
import {
  aiBlockedHint,
  aiStatusView,
  currentSourceNote,
  CURRENT_SOURCE_MISSING,
  findingLevelLabel,
  FINDING_LEVELS,
  localSourceNote,
  sourceOptions,
  summaryLine,
  type AiReport,
} from "./config-doctor-model.ts";

const panel = readFileSync(new URL("./ConfigDoctorPanel.tsx", import.meta.url), "utf8");

it("配置体检面板的界面文字都有英文", () => {
  for (const match of panel.matchAll(/\bt\("([^"]+)"\)/g)) assert.ok(EN_PLAIN[match[1]], `缺英文：${match[1]}`);
  for (const match of panel.matchAll(/\btf\("([^"]+)"/g)) assert.ok(EN_TEMPLATE[match[1]], `缺英文模板：${match[1]}`);
  assert.ok(EN_PLAIN["配置体检"], "设置分类「配置体检」要有英文");
});

it("配置体检的模型层文字（来源、分组、汇总、AI 状态）都有英文", () => {
  const plain = [
    ...sourceOptions(null).flatMap((option) => [option.label, option.hint]),
    currentSourceNote(null).label,
    CURRENT_SOURCE_MISSING,
    localSourceNote(null).label,
    ...FINDING_LEVELS.map(findingLevelLabel),
    ...(["ok", "failed", "skipped"] as const).map((status) => aiStatusView({ status, message: "", model: "", endpoint: "", elapsedMs: 0, summary: "", items: [] } as AiReport)).flatMap((view) => [view.label, view.detail].filter(Boolean)),
  ];
  for (const text of plain) assert.ok(EN_PLAIN[text], `缺英文：${text}`);
  const templates = [
    summaryLine({ error: 1, warn: 1, info: 1 })!.text,
    aiBlockedHint({ enabled: false, source: "custom", baseUrl: "", model: "" })!.text,
    aiBlockedHint({ enabled: true, source: "custom", baseUrl: "", model: "" })!.text,
  ];
  for (const text of templates) assert.ok(EN_TEMPLATE[text], `缺英文模板：${text}`);
  // 后端拼好的来源说明也过一遍 t()，有英文就显示英文。
  assert.match(panel, /\{t\(currentNote\.label\)\}/);
  assert.match(panel, /\{t\(localNote\.label\)\}/);
});
