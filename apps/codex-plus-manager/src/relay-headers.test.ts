/**
 * @description 自定义请求头校验与规范化单测（Node 内置 test runner，与 aggregate-routes.test.ts 同风格）
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-09-23
 */

import assert from "node:assert";
import { describe, it } from "node:test";
import {
  hasCustomAuthorization,
  normalizeRelayHeaders,
  relayHeadersValidationMessage,
  serializeRelayHeaders,
} from "./relay-headers.ts";

describe("normalizeRelayHeaders", () => {
  it("丢掉两侧都空的行", () => {
    assert.deepStrictEqual(
      normalizeRelayHeaders([
        { key: "", value: "" },
        { key: "  ", value: "  " },
      ]),
      [],
    );
  });

  it("压缩名称与值的首尾空白", () => {
    assert.deepStrictEqual(normalizeRelayHeaders([{ key: " X-Tenant ", value: " acme " }]), [
      { key: "X-Tenant", value: "acme" },
    ]);
  });

  it("保留只有名称的行（值留空由用户后续补）", () => {
    assert.deepStrictEqual(normalizeRelayHeaders([{ key: "X-Ok", value: "" }]), [
      { key: "X-Ok", value: "" },
    ]);
  });
});

describe("relayHeadersValidationMessage", () => {
  it("合法配置返回 null", () => {
    assert.strictEqual(
      relayHeadersValidationMessage([{ key: "X-Tenant", value: "acme" }]),
      null,
    );
  });

  it("拒绝传输头（不区分大小写）", () => {
    for (const name of ["Host", "content-length", "Transfer-Encoding", "Connection"]) {
      const message = relayHeadersValidationMessage([{ key: name, value: "x" }]);
      assert.ok(message && message.includes("不允许覆盖"), `${name} 应被拒绝：${message}`);
    }
  });

  it("拒绝非法名称", () => {
    const message = relayHeadersValidationMessage([{ key: "bad name", value: "1" }]);
    assert.ok(message && message.includes("不是合法的 HTTP 头名称"));
  });

  it("拒绝重复名称（大小写不敏感）", () => {
    const message = relayHeadersValidationMessage([
      { key: "X-A", value: "1" },
      { key: "x-a", value: "2" },
    ]);
    assert.ok(message && message.includes("重复配置"));
  });

  it("拒绝值里的换行（防 header 注入）", () => {
    const message = relayHeadersValidationMessage([{ key: "X-A", value: "1\r\nHost: evil" }]);
    assert.ok(message && message.includes("不能包含换行"));
  });

  it("报错不回显值", () => {
    const message = relayHeadersValidationMessage([
      { key: "X-Secret-Token", value: "sk-super-secret\r\nX-Inject: 1" },
    ]);
    assert.ok(message && message.includes("X-Secret-Token"));
    assert.ok(!message.includes("sk-super-secret"));
  });
});

describe("hasCustomAuthorization", () => {
  it("识别显式 Authorization", () => {
    assert.strictEqual(hasCustomAuthorization([{ key: "authorization", value: "Bearer x" }]), true);
  });

  it("空值的 Authorization 不算配置", () => {
    assert.strictEqual(hasCustomAuthorization([{ key: "Authorization", value: "  " }]), false);
  });
});

describe("serializeRelayHeaders", () => {
  it("保存时去掉空行", () => {
    assert.deepStrictEqual(
      serializeRelayHeaders([
        { key: "X-A", value: "1" },
        { key: "", value: "" },
      ]),
      [{ key: "X-A", value: "1" }],
    );
  });
});
