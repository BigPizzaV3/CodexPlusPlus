import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

test("价格行外面有一键同步，不用打开弹窗", () => {
  assert.match(read("GatewayProviders.tsx"), /"sync_gateway_provider_prices", \{ id: entry\.id \}/);
});

test("价格和代理是两个弹窗，各自保存时不动另一项", () => {
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /setExtrasOpen\("price"\)/);
  assert.match(providers, /setExtrasOpen\("proxy"\)/);
  const dialog = read("ProviderExtrasDialog.tsx");
  assert.match(dialog, /onSave\(entry\.price \?\? null, null, proxy\.trim\(\)\)/);
  assert.match(dialog, /modelPricesFromRows\(perModel, currency\), entry\.proxy \?\? ""\)/);
});
