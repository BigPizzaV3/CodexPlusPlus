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
  it("http_error 403 也走认证失败并渲染状态码", () => {
    const msg = vlmTestTranslation("http_error", 403, 0, tr);
    assert.ok(msg.includes("认证失败"));
    assert.ok(msg.includes("HTTP 403"));
  });
  it("http_error 404 含接口不存在", () => {
    assert.ok(vlmTestTranslation("http_error", 404, 0, tr).includes("接口不存在"));
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
  it("ok 渲染耗时占位", () => {
    assert.ok(vlmTestTranslation("ok", 200, 2300, tr).includes("2.3s"));
  });
  it("http_error 500 走通用分支含 HTTP 码", () => {
    assert.ok(vlmTestTranslation("http_error", 500, 0, tr).includes("HTTP 500"));
  });
  it("parse_error 含批量描述解析失败", () => {
    assert.ok(vlmTestTranslation("parse_error", undefined, 0, tr).includes("批量描述解析失败"));
  });
  it("client_error 含客户端构建失败", () => {
    assert.ok(vlmTestTranslation("client_error", undefined, 0, tr).includes("客户端构建失败"));
  });
  it("http_error 无状态码走 ? 兜底", () => {
    assert.ok(vlmTestTranslation("http_error", undefined, 0, tr).includes("HTTP ?"));
  });
  it("429 文案逐字锁定", () => {
    assert.equal(vlmTestTranslation("http_error", 429, 0, tr), "❌ 被限流，稍后再试（HTTP 429）");
  });
  it("timeout 文案逐字锁定", () => {
    assert.equal(vlmTestTranslation("timeout", undefined, 0, tr), "❌ 请求超时：VLM 响应过慢或网络不通");
  });
  it("401 文案逐字锁定（含模型名归因）", () => {
    assert.equal(vlmTestTranslation("http_error", 401, 0, tr), "❌ 认证失败（HTTP 401）：API Key 或模型名可能不正确");
  });
  it("invalid_image 提示图片无效与大小上限", () => {
    assert.ok(vlmTestTranslation("invalid_image", undefined, 0, tr).includes("测试图片无效"));
    assert.ok(vlmTestTranslation("invalid_image", undefined, 0, tr).includes("10MB"));
  });
});
