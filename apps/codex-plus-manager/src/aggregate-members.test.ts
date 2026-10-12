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
