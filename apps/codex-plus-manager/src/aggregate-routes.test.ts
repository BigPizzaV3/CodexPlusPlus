/**
 * @description 聚合供应商路由规则纯函数单测（Node 内置 test runner，与 model-windows.test.ts 同风格）
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-08-05
 */

import assert from "node:assert";
import { describe, it } from "node:test";
import {
  clampAggregateRoutePriority,
  normalizeAggregateRoutes,
  validateAggregateRoutes,
  type AggregateRouteLike,
} from "./aggregate-routes.ts";

describe("clampAggregateRoutePriority", () => {
  it("NaN 归零", () => {
    assert.strictEqual(clampAggregateRoutePriority(NaN), 0);
  });
  it("负数归零", () => {
    assert.strictEqual(clampAggregateRoutePriority(-5), 0);
    assert.strictEqual(clampAggregateRoutePriority(-0.1), 0);
  });
  it("超过 999 钳到 999", () => {
    assert.strictEqual(clampAggregateRoutePriority(1500), 999);
  });
  it("小数四舍五入", () => {
    assert.strictEqual(clampAggregateRoutePriority(3.4), 3);
    assert.strictEqual(clampAggregateRoutePriority(3.6), 4);
  });
  it("边界值保持不变", () => {
    assert.strictEqual(clampAggregateRoutePriority(0), 0);
    assert.strictEqual(clampAggregateRoutePriority(999), 999);
  });
});

describe("normalizeAggregateRoutes", () => {
  it("trim pattern 并 clamp priority", () => {
    const routes: AggregateRouteLike[] = [
      { pattern: "  deepseek-*  ", profileId: "member-a", priority: 1200 },
      { pattern: "gpt-*", profileId: "member-b", priority: -3 },
    ];
    const result = normalizeAggregateRoutes(routes);
    assert.deepStrictEqual(result, [
      { pattern: "deepseek-*", profileId: "member-a", priority: 999 },
      { pattern: "gpt-*", profileId: "member-b", priority: 0 },
    ]);
  });
  it("默认保留空 pattern 规则（不再静默删除）", () => {
    const routes: AggregateRouteLike[] = [
      { pattern: "   ", profileId: "member-a", priority: 1 },
      { pattern: "", profileId: "", priority: 2 },
    ];
    const result = normalizeAggregateRoutes(routes);
    assert.strictEqual(result.length, 2);
    assert.strictEqual(result[0]!.pattern, "");
    assert.strictEqual(result[1]!.pattern, "");
  });
  it("dropEmptyPattern 时过滤空 pattern 规则", () => {
    const routes: AggregateRouteLike[] = [
      { pattern: "   ", profileId: "member-a", priority: 1 },
      { pattern: "deepseek-*", profileId: "member-b", priority: 2 },
    ];
    const result = normalizeAggregateRoutes(routes, { dropEmptyPattern: true });
    assert.deepStrictEqual(result, [{ pattern: "deepseek-*", profileId: "member-b", priority: 2 }]);
  });
  it("memberIds 过滤非成员规则", () => {
    const routes: AggregateRouteLike[] = [
      { pattern: "deepseek-*", profileId: "member-a", priority: 1 },
      { pattern: "gpt-*", profileId: "removed-provider", priority: 2 },
    ];
    const result = normalizeAggregateRoutes(routes, { memberIds: new Set(["member-a"]) });
    assert.deepStrictEqual(result, [{ pattern: "deepseek-*", profileId: "member-a", priority: 1 }]);
  });
  it("clampPriority false 时保留原始 priority", () => {
    const routes: AggregateRouteLike[] = [{ pattern: "deepseek-*", profileId: "member-a", priority: -7 }];
    const result = normalizeAggregateRoutes(routes, { clampPriority: false });
    assert.deepStrictEqual(result, [{ pattern: "deepseek-*", profileId: "member-a", priority: -7 }]);
  });
  it("空数组返回空数组", () => {
    assert.deepStrictEqual(normalizeAggregateRoutes([]), []);
  });
});

describe("validateAggregateRoutes", () => {
  const memberIds = new Set(["member-a", "member-b"]);

  it("空 pattern（有目标）报 emptyPattern", () => {
    const issues = validateAggregateRoutes([{ pattern: "  ", profileId: "member-a", priority: 1 }], memberIds);
    assert.ok(issues);
    assert.strictEqual(issues[0]!.code, "emptyPattern");
  });
  it("pattern 与 profileId 全空的行跳过", () => {
    const issues = validateAggregateRoutes([{ pattern: "", profileId: "", priority: 1 }], memberIds);
    assert.strictEqual(issues, null);
  });
  it("非整数 priority 报 invalidPriority", () => {
    const issues = validateAggregateRoutes([{ pattern: "deepseek-*", profileId: "member-a", priority: 1.5 }], memberIds);
    assert.ok(issues);
    assert.strictEqual(issues[0]!.code, "invalidPriority");
    assert.strictEqual(issues[0]!.pattern, "deepseek-*");
  });
  it("NaN priority 报 invalidPriority", () => {
    const issues = validateAggregateRoutes([{ pattern: "deepseek-*", profileId: "member-a", priority: NaN }], memberIds);
    assert.ok(issues);
    assert.strictEqual(issues[0]!.code, "invalidPriority");
  });
  it("负数 priority 报 invalidPriority", () => {
    const issues = validateAggregateRoutes([{ pattern: "deepseek-*", profileId: "member-a", priority: -1 }], memberIds);
    assert.ok(issues);
    assert.strictEqual(issues[0]!.code, "invalidPriority");
  });
  it("非成员 profileId 报 notMember", () => {
    const issues = validateAggregateRoutes(
      [{ pattern: "gpt-*", profileId: "removed-provider", priority: 1 }],
      memberIds,
    );
    assert.ok(issues);
    assert.strictEqual(issues[0]!.code, "notMember");
    assert.strictEqual(issues[0]!.pattern, "gpt-*");
  });
  it("返回全部错误而非仅第一个", () => {
    const issues = validateAggregateRoutes(
      [
        { pattern: "a-*", profileId: "removed-1", priority: 1 },
        { pattern: "b-*", profileId: "removed-2", priority: 2 },
        { pattern: "ok-*", profileId: "member-a", priority: 3 },
      ],
      memberIds,
    );
    assert.ok(issues);
    assert.strictEqual(issues.length, 2);
  });
  it("合法规则返回 null", () => {
    const issues = validateAggregateRoutes(
      [
        { pattern: "deepseek-*", profileId: "member-a", priority: 10 },
        { pattern: "gpt-*", profileId: "member-b", priority: 0 },
      ],
      memberIds,
    );
    assert.strictEqual(issues, null);
  });
});

describe("priority 上限", () => {
  it("超过 999 报 invalidPriority", () => {
    const issues = validateAggregateRoutes(
      [{ pattern: "deepseek-*", profileId: "member-a", priority: 1000 }],
      new Set(["member-a"]),
    );
    assert.ok(issues);
    assert.strictEqual(issues[0]!.code, "invalidPriority");
    assert.strictEqual(issues[0]!.pattern, "deepseek-*");
  });
  it("边界 999 合法", () => {
    const issues = validateAggregateRoutes(
      [{ pattern: "deepseek-*", profileId: "member-a", priority: 999 }],
      new Set(["member-a"]),
    );
    assert.strictEqual(issues, null);
  });
});
