import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { isFavoriteModel, ownPickerSection, pickerSections, type GatewayPayload, type OwnModels, type PickerSection } from "./gateway-model.ts";

const source = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8").replaceAll("\r\n", "\n");
const start = "  const sections = useMemo(() => {\n";
const end = "\n  }, [payload, query, exclude, favoritesOnly, sectionOnly, favorites, skipGroup, ownList]);";
const begin = source.indexOf(start);
const finish = source.indexOf(end, begin);
assert.ok(begin >= 0 && finish > begin, "找不到选择器的分组筛选逻辑");
const select = new Function(
  "payload", "query", "exclude", "skipGroup", "ownList", "sectionOnly", "favoritesOnly", "favorites",
  "withOwnSection", "pickerSections", "ownPickerSection", "avatarOf", "isFavoriteModel",
  source.slice(begin + start.length, finish),
);
const ownStart = source.indexOf("function withOwnSection(");
const bodyStart = source.indexOf("{\n", ownStart);
const bodyEnd = source.indexOf("\n}", bodyStart);
assert.ok(ownStart >= 0 && bodyStart > ownStart && bodyEnd > bodyStart, "找不到直连模型分组逻辑");
const withOwnSection = new Function("sections", "own", source.slice(bodyStart + 2, bodyEnd));

function payload(): GatewayPayload {
  return {
    gateway: { version: 1, bindings: {}, keyPolicy: { mode: "sticky" }, restoreOnExit: false },
    port: 57321, helperRunning: true,
    pool: Array.from({ length: 8 }, (_, index) => ({
      id: `p${index + 1}`, name: `Provider ${index + 1}`, baseUrl: "", protocol: "responses",
      official: false, models: index < 2 ? ["gpt-6.1-terra"] : [`model-${index + 1}`],
      keyCount: 1, enabled: true, source: "",
    })),
    groups: [{ id: "daily", name: "Daily", mode: "inOrder", members: [{ model: "p1/gpt-6.1-terra", weight: 1 }] }],
    agents: [], health: { keys: [], providers: [] }, codexProviders: [], claudeProviders: [], grokProfiles: [],
  } as GatewayPayload;
}

const own: OwnModels = { vendor: "Direct", current: "", models: [{ id: "direct-luna", label: "Direct Luna" }] };

function sections(query: string, sectionOnly: string | null = null, favoritesOnly = false, direct: OwnModels | null = null, exclude: string[] = [], favorites: string[] = []): PickerSection[] {
  return select(payload(), query, exclude, undefined, direct, sectionOnly, favoritesOnly, favorites,
    withOwnSection, pickerSections, ownPickerSection, (name: string) => name, isFavoriteModel);
}

test("已知模型的模糊、完整、大小写及带供应商前缀搜索不追加自定义项", () => {
  for (const query of ["terra", " GPT-6.1-TERRA ", "p1/gpt-6.1-terra", "provider 1", "daily"]) {
    const result = sections(query);
    assert.ok(result.length, query);
    assert.ok(!result.some((section) => section.key === "custom"), query);
  }
  assert.equal(sections("").some((section) => section.key === "custom"), false);
  assert.equal(sections("no match").some((section) => section.key === "custom"), false);
});

test("未知模型仍提供各目标供应商，可输入带斜杠及完整供应商前缀的 ID", () => {
  const custom = sections("kimi-k3").find((section) => section.key === "custom");
  assert.deepEqual(custom?.items.map((item) => item.model), ["p1/kimi-k3", "p2/kimi-k3", "p3/kimi-k3", "p4/kimi-k3", "p5/kimi-k3", "p6/kimi-k3"]);
  assert.equal(custom?.items[0].sub, "Provider 1");
  assert.equal(sections("vendor/new").find((section) => section.key === "custom")?.items[0].model, "p1/vendor/new");
  assert.deepEqual(sections("p7/new-model").find((section) => section.key === "custom")?.items.map((item) => item.model), ["p7/new-model"]);
  assert.equal(sections("kimi-k3", null, false, null, ["p1"]).find((section) => section.key === "custom")?.items[0].model, "p2/kimi-k3");
});

test("供应商筛选只给该供应商生成候选，包括第七家；其他范围不混入候选", () => {
  assert.deepEqual(sections("unknown", "provider:p7").find((section) => section.key === "custom")?.items.map((item) => item.model), ["p7/unknown"]);
  assert.deepEqual(sections("unknown", "provider:p2").find((section) => section.key === "custom")?.items.map((item) => item.model), ["p2/unknown"]);
  assert.equal(sections("unknown", "group").some((section) => section.key === "custom"), false);
  assert.equal(sections("unknown", "own", false, own).some((section) => section.key === "custom"), false);
  assert.equal(sections("unknown", null, true).some((section) => section.key === "custom"), false);
  assert.deepEqual(sections("unknown", null, true, null, [], ["p2/unknown"]).find((section) => section.key === "custom")?.items.map((item) => item.model), ["p2/unknown"]);
  assert.equal(sections("unknown", "provider:p7", false, null, ["p7"]).some((section) => section.key === "custom"), false);
  assert.equal(sections("p7/unknown", "provider:p2").some((section) => section.key === "custom"), false);
});

test("直连已知模型匹配时不追加自定义项", () => {
  assert.deepEqual(sections("luna", null, false, own).map((section) => section.key), ["own"]);
});

test("自定义候选显示目标供应商与自定义标识", () => {
  assert.match(source, /item\.custom\s*\?\s*<small>[^<]*\{item\.sub\}[^<]*\{t\("自定义模型 ID"\)\}<\/small>/);
  assert.ok(source.includes('tf("{0} 个自定义候选", [customCount])'));
});
