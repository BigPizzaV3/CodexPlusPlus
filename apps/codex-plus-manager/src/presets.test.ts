import assert from "node:assert";
import { describe, it, test } from "node:test";
import { PRESETS, sponsorPresets } from "./presets.ts";

describe("provider presets", () => {
  it("keeps MiniMax China and global credentials in separate presets", () => {
    const china = PRESETS.find((preset) => preset.id === "minimax");
    const global = PRESETS.find((preset) => preset.id === "minimax-global");

    assert.deepStrictEqual(china, {
      id: "minimax",
      name: "MiniMax (China)",
      websiteUrl: "https://platform.minimaxi.com",
      apiKeyUrl: "https://platform.minimaxi.com/subscribe/coding-plan",
      category: "cn_official",
      baseUrl: "https://api.minimaxi.com/v1",
      protocol: "chatCompletions",
      model: "MiniMax-M3",
      modelList: ["MiniMax-M3", "MiniMax-M2.7"],
    });

    assert.deepStrictEqual(global, {
      id: "minimax-global",
      name: "MiniMax (Global)",
      websiteUrl: "https://platform.minimax.io",
      apiKeyUrl: "https://platform.minimax.io/subscribe/coding-plan",
      category: "official",
      baseUrl: "https://api.minimax.io/v1",
      protocol: "chatCompletions",
      model: "MiniMax-M3",
      modelList: ["MiniMax-M3", "MiniMax-M2.7"],
    });
  });

test("DeepSeek preset uses the official Responses integration", () => {
  const preset = PRESETS.find((candidate) => candidate.id === "deepseek");
  assert.ok(preset);
  assert.equal(preset.baseUrl, "https://api.deepseek.com/");
  assert.equal(preset.protocol, "responses");
  assert.equal(preset.model, "deepseek-v4-flash");
  assert.deepEqual(preset.modelList, ["deepseek-v4-flash", "deepseek-v4-pro"]);
});

// GrooRoute 赞助已于 2026-10-15 到期：预设与推广链接一并下线。
test("expired GrooRoute sponsor preset is removed", () => {
  assert.equal(PRESETS.some((candidate) => candidate.id === "grooroute"), false);
});

test("sponsor presets are not built in", () => {
  for (const id of ["jojocode-max", "runapi", "apikeyfun", "cccccc-api", "tokenmy", "vegaapi"]) {
    assert.equal(PRESETS.some((candidate) => candidate.id === id), false, id);
  }
});
});

describe("sponsor presets from the remote ad list", () => {
  const ad = (id: string, preset: unknown, extra: Record<string, unknown> = {}) => ({
    id: `${id}-ad`, type: "sponsor", title: id, description: "d", url: "https://example.test", preset, ...extra,
  });
  const preset = (id: string, extra: Record<string, unknown> = {}) => ({
    id, name: id.toUpperCase(), baseUrl: `https://${id}.example/v1`, protocol: "responses", model: "", ...extra,
  });

  it("returns nothing when the list did not load", () => {
    assert.deepEqual(sponsorPresets(null), []);
    assert.deepEqual(sponsorPresets({ ads: [] }), []);
  });

  it("reads the preset of each sponsor ad, top slots first, as an aggregator preset", () => {
    const presets = sponsorPresets({
      topAds: [ad("jojo", preset("jojocode-max", { websiteUrl: "https://max2.jojocode.com/" }))],
      ads: [
        ad("apikey", preset("apikeyfun", { model: "gpt-5.5", modelList: ["gpt-5.5", " ", 3], apiKeyUrl: "javascript:alert(1)" })),
        ad("plain", undefined),
        ad("jojo-again", preset("jojocode-max")),
        { ...ad("normal", preset("normal")), type: "normal" },
      ],
    });
    assert.deepEqual(presets, [
      { id: "jojocode-max", name: "JOJOCODE-MAX", websiteUrl: "https://max2.jojocode.com/", category: "aggregator", baseUrl: "https://jojocode-max.example/v1", protocol: "responses", model: "" },
      { id: "apikeyfun", name: "APIKEYFUN", category: "aggregator", baseUrl: "https://apikeyfun.example/v1", protocol: "responses", model: "gpt-5.5", modelList: ["gpt-5.5"] },
    ]);
  });

  it("drops presets with a bad address or protocol", () => {
    const presets = sponsorPresets({
      ads: [
        ad("a", preset("a", { baseUrl: "javascript:alert(1)" })),
        ad("b", preset("b", { protocol: "anthropicMessages" })),
        ad("c", preset("c", { name: " " })),
        ad("d", preset("d", { protocol: "chatCompletions" })),
      ],
    });
    assert.deepEqual(presets.map((item) => [item.id, item.protocol]), [["d", "chatCompletions"]]);
  });

  it("keeps a sponsor through its expiry day and drops it the day after", () => {
    const payload = { ads: [ad("apikey", preset("apikeyfun"), { expires_at: "2026-12-22" })] };
    assert.equal(sponsorPresets(payload, new Date(2026, 11, 22, 23, 59)).length, 1);
    assert.equal(sponsorPresets(payload, new Date(2026, 11, 23)).length, 0);
  });
});
