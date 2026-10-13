import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

/**
 * 供应商分页的顺序是刻意排的：直连与模型聚合最常用，排在最前面。
 * 重排 JSX 时很容易顺手挪回去，这里用一条静态断言守住。
 */
const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

test("provider view tabs keep model aggregation right after direct", () => {
  const start = source.indexOf('className="provider-view-tabs"');
  assert.ok(start >= 0, "App.tsx 里找不到 provider-view-tabs");

  const tabs = source.slice(start);
  const block = tabs.slice(0, tabs.indexOf("</div>"));
  const order = [...block.matchAll(/selectProviderView\("([A-Za-z]+)"\)/g)].map((match) => match[1]);

  assert.deepEqual(order, ["direct", "modelAggregation", "routing", "failover"]);
});
