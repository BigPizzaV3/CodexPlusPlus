import assert from "node:assert";
import { describe, it } from "node:test";
import { vlmTestTranslation } from "./vlm-test-translation.ts";

const tr = (zh: string, params?: string[]) =>
  params ? zh.replace("{0}", params[0]) : zh;

describe("vlmTestTranslation", () => {
  it("ok 含识别成功", () => {
    assert.ok(vlmTestTranslation("ok", 200, 2300, tr).includes("识别成功"));
  });
  it("http_error 401 含认证失败", () => {
    assert.ok(vlmTestTranslation("http_error", 401, 0, tr).includes("认证失败"));
  });
  it("http_error 404 含接口不存在", () => {
    assert.ok(vlmTestTranslation("http_error", 404, 0, tr).includes("接口不存在"));
  });
  it("http_error 429 含限流", () => {
    assert.ok(vlmTestTranslation("http_error", 429, 0, tr).includes("限流"));
  });
  it("timeout 含超时", () => {
    assert.ok(vlmTestTranslation("timeout", undefined, 0, tr).includes("超时"));
  });
  it("send_error 含网络", () => {
    assert.ok(vlmTestTranslation("send_error", undefined, 0, tr).includes("网络"));
  });
  it("no_text 含未找到描述文本", () => {
    assert.ok(vlmTestTranslation("no_text", 200, 0, tr).includes("未找到描述文本"));
  });
  it("json_error 含解析失败", () => {
    assert.ok(vlmTestTranslation("json_error", 200, 0, tr).includes("解析失败"));
  });
  it("未知 status 兜底", () => {
    assert.ok(vlmTestTranslation("weird", 500, 0, tr).includes("未知错误"));
  });
});
