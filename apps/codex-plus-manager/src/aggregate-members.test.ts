/**
 * @description 聚合成员候选纯函数单测（Node 内置 test runner，与 aggregate-routes.test.ts 同风格）
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-10-12
 */

import assert from "node:assert";
import { describe, it } from "node:test";
import {
  aggregateMemberCandidates,
  authContentsHaveChatgptTokens,
  isApiMemberProfile,
  isOfficialLoginMemberProfile,
  normalizeAggregateDefaultRelayId,
  normalizeAggregateMembers,
  type AggregateMemberProfileLike,
} from "./aggregate-members.ts";

function profile(overrides: Partial<AggregateMemberProfileLike> & { id: string }): AggregateMemberProfileLike {
  return { relayMode: "pureApi", ...overrides };
}

const chatgptAuth = JSON.stringify({
  auth_mode: "chatgpt",
  tokens: { access_token: "access", refresh_token: "refresh", account_id: "acct-1" },
});

describe("authContentsHaveChatgptTokens", () => {
  it("识别带 tokens 的登录快照", () => {
    assert.strictEqual(authContentsHaveChatgptTokens(chatgptAuth), true);
  });

  it("只有空 tokens 不算登录", () => {
    assert.strictEqual(authContentsHaveChatgptTokens(JSON.stringify({ tokens: {} })), false);
    assert.strictEqual(
      authContentsHaveChatgptTokens(JSON.stringify({ tokens: { access_token: "  " } })),
      false,
    );
  });

  it("API Key 型 auth.json 不算官方登录", () => {
    assert.strictEqual(authContentsHaveChatgptTokens(JSON.stringify({ OPENAI_API_KEY: "sk-1" })), false);
    assert.strictEqual(authContentsHaveChatgptTokens(""), false);
  });

  it("损坏 JSON 用兜底正则", () => {
    assert.strictEqual(authContentsHaveChatgptTokens('{"tokens": {"access_token": "a"'), true);
    assert.strictEqual(authContentsHaveChatgptTokens("{broken"), false);
  });
});

describe("isOfficialLoginMemberProfile", () => {
  it("纯官方 profile 且有登录凭据时可作为成员", () => {
    assert.strictEqual(
      isOfficialLoginMemberProfile(profile({ id: "official", relayMode: "official", authContents: chatgptAuth })),
      true,
    );
  });

  it("混合 API Key 的官方 profile 不走官方登录分支", () => {
    assert.strictEqual(
      isOfficialLoginMemberProfile(
        profile({ id: "official-mix", relayMode: "official", officialMixApiKey: true, authContents: chatgptAuth }),
      ),
      false,
    );
  });

  it("没有登录凭据的官方 profile 不可作为成员", () => {
    assert.strictEqual(
      isOfficialLoginMemberProfile(profile({ id: "official-empty", relayMode: "official" })),
      false,
    );
  });

  it("chat/completions 协议的官方 profile 不走官方登录通道", () => {
    assert.strictEqual(
      isOfficialLoginMemberProfile(
        profile({ id: "official-chat", relayMode: "official", protocol: "chatCompletions", authContents: chatgptAuth }),
      ),
      false,
    );
  });
});

describe("aggregateMemberCandidates", () => {
  const apiMember = profile({ id: "api", baseUrl: "https://api.example/v1", apiKey: "sk-1" });
  const officialMember = profile({ id: "official", relayMode: "official", authContents: chatgptAuth });
  const emptyOfficial = profile({ id: "official-empty", relayMode: "official" });
  const aggregate = profile({ id: "agg-2", relayMode: "aggregate" });
  const self = profile({ id: "agg-1", relayMode: "aggregate", aggregate: { members: [] } });
  const noKey = profile({ id: "api-no-key", baseUrl: "https://api.example/v1" });

  it("同时包含 API 成员与官方登录成员，排除聚合/缺凭据/自身", () => {
    const candidates = aggregateMemberCandidates(
      [self, apiMember, officialMember, emptyOfficial, aggregate, noKey],
      "agg-1",
    );
    assert.deepStrictEqual(
      candidates.map((item) => item.id),
      ["api", "official"],
    );
  });

  it("isApiMemberProfile 要求 URL 与 Key 同时非空", () => {
    assert.strictEqual(isApiMemberProfile(apiMember), true);
    assert.strictEqual(isApiMemberProfile(noKey), false);
    assert.strictEqual(isApiMemberProfile(profile({ id: "blank", baseUrl: "  ", apiKey: " " })), false);
  });
});

describe("normalizeAggregateDefaultRelayId", () => {
  const members = new Set(["api", "official"]);

  it("成员集合内的值原样保留（含 trim）", () => {
    assert.strictEqual(normalizeAggregateDefaultRelayId("official", members), "official");
    assert.strictEqual(normalizeAggregateDefaultRelayId("  api  ", members), "api");
  });

  it("空值/未设置清空为空串", () => {
    assert.strictEqual(normalizeAggregateDefaultRelayId(undefined, members), "");
    assert.strictEqual(normalizeAggregateDefaultRelayId(null, members), "");
    assert.strictEqual(normalizeAggregateDefaultRelayId("   ", members), "");
  });

  it("成员被移出后默认值不再保留", () => {
    assert.strictEqual(normalizeAggregateDefaultRelayId("removed", members), "");
  });
});

describe("normalizeAggregateMembers", () => {
  const member = (profileId: string, weight = 1) => ({ profileId, weight });

  it("供应商还在就保留，去重并 trim", () => {
    const kept = normalizeAggregateMembers(
      [member("api"), member("  api  "), member("official")],
      new Set(["api", "official"]),
    );
    assert.deepStrictEqual(kept, [member("api"), member("official")]);
  });

  it("供应商已被删除时丢掉", () => {
    const kept = normalizeAggregateMembers([member("api"), member("ghost")], new Set(["api"]));
    assert.deepStrictEqual(kept, [member("api")]);
  });

  it("拿不到完整供应商列表时一律保留成员（宁可留着也不替用户删配置）", () => {
    const kept = normalizeAggregateMembers([member("api"), member("ghost")], null);
    assert.deepStrictEqual(kept, [member("api"), member("ghost")]);
  });

  it("关键回归：成员没变、但供应商的 Key/凭据缺失时，成员不被清掉", () => {
    // 历史事故：成员规范化直接拿"可勾选候选"过滤，而候选要求 URL+Key 齐备 / 官方
    // 登录快照有 tokens。供应商编辑中或快照未回填时判定会翻转，于是一次普通保存就把
    // 正在用的聚合成员删光，聚合当场报「成员供应商不存在」。
    const editing = profile({ id: "api", baseUrl: "https://api.example/v1" });
    const notReady = profile({ id: "official", relayMode: "official" });
    assert.strictEqual(isApiMemberProfile(editing), false, "编辑中的供应商不是候选");
    assert.strictEqual(isOfficialLoginMemberProfile(notReady), false, "未回填的官方成员不是候选");
    assert.deepStrictEqual(
      aggregateMemberCandidates([editing, notReady], "agg").map((item) => item.id),
      [],
      "两个都不是可勾选候选",
    );
    // 但它们仍然存在于供应商列表里 → 成员必须保留。
    const kept = normalizeAggregateMembers(
      [member("api"), member("official")],
      new Set(["api", "official", "agg"]),
    );
    assert.deepStrictEqual(kept, [member("api"), member("official")]);
  });
});
