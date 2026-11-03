import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

/**
 * 供应商分页的顺序是刻意排的：直连与模型聚合最常用，排在最前面。
 * 重排 JSX 时很容易顺手挪回去，这里用一条静态断言守住。
 */
const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

test("provider view tabs keep model aggregation right after direct", () => {
  // 分页是「Codex 连接」页直连供应商一节的筛选胶囊，顺序就是这个数组。
  const match = source.match(/items=\{\(\[([^\]]+)\] as ProviderViewKind\[\]\)\.map/);
  assert.ok(match, "App.tsx 里找不到供应商分页胶囊");
  const order = [...match[1].matchAll(/"([A-Za-z]+)"/g)].map((item) => item[1]);

  assert.deepEqual(order, ["direct", "modelAggregation", "routing", "failover"]);
  assert.match(source, /onChange=\{selectProviderView\}/);
});
