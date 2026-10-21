import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { nextAggregateProfileName } from "./aggregate-profile-name.ts";

const chineseName = (number: number) => `聚合供应商 ${number}`;

describe("nextAggregateProfileName", () => {
  it("没有供应商时从 1 开始", () => {
    assert.equal(nextAggregateProfileName([], chineseName), "聚合供应商 1");
  });

  it("连续名称取下一编号，删除 4 后再次使用 4", () => {
    const names = [1, 2, 3, 4].map(chineseName);
    assert.equal(nextAggregateProfileName(names, chineseName), "聚合供应商 5");
    assert.equal(nextAggregateProfileName(names.slice(0, -1), chineseName), "聚合供应商 4");
  });

  it("复用中间空缺，不受较大编号和无关名称影响", () => {
    assert.equal(
      nextAggregateProfileName(["聚合供应商 1", "聚合供应商 3", "聚合供应商 99", "自定义聚合", "API"], chineseName),
      "聚合供应商 2",
    );
  });

  it("检查所有传入名称，去掉首尾空格并容忍重复名称", () => {
    assert.equal(
      nextAggregateProfileName([" 聚合供应商 1 ", "聚合供应商 1", "聚合供应商 2"], chineseName),
      "聚合供应商 3",
    );
  });

  it("沿用调用方的 i18n 格式，不把其他语言的名称当作当前名称", () => {
    assert.equal(
      nextAggregateProfileName(
        ["Aggregate provider 1", "Aggregate provider 3", "聚合供应商 2"],
        (number) => `Aggregate provider ${number}`,
      ),
      "Aggregate provider 2",
    );
  });

  it("非标准编号和相似名称不占用默认名称", () => {
    assert.equal(
      nextAggregateProfileName(["聚合供应商 01", "聚合供应商 0", "聚合供应商 1 备份"], chineseName),
      "聚合供应商 1",
    );
  });
});
