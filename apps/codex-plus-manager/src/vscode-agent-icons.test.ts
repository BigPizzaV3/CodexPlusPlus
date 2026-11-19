/** #2970：Agent 接入页里 VS Code / VS Code Insiders 用各自的官方图标（蓝 / 绿）。 */
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";
import { BRAND_ICON_URLS } from "./provider-brand-icons.ts";
import { brandIconIsColor } from "./provider-brands.ts";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

test("VS Code、Insiders 映射到各自的彩色官方图标，不再是首字母", () => {
  const ui = read("gateway-ui.tsx");
  assert.match(ui, /\n {2}vscode: "vscode-color",\n/);
  assert.match(ui, /\n {2}"vscode-insiders": "vscode-insiders-color",\n/);
  // 和其它彩色 Agent 图标一样按 <img> 原样显示（-color 后缀），已接入 / 未接入都走同一套渲染。
  for (const key of ["vscode-color", "vscode-insiders-color"]) {
    assert.ok(brandIconIsColor(key), key);
    assert.ok(BRAND_ICON_URLS[key]?.endsWith(`/assets/providers/${key}.svg`), key);
    assert.ok(existsSync(new URL(`./assets/providers/${key}.svg`, import.meta.url)), key);
  }
});

test("图标是官方配色：VS Code 蓝、Insiders 绿，两者形状相同", () => {
  const stable = read("assets/providers/vscode-color.svg");
  const insiders = read("assets/providers/vscode-insiders-color.svg");
  for (const color of ["#0065A9", "#007ACC", "#1F9CF0"]) assert.ok(stable.includes(color), color);
  for (const color of ["#009A7C", "#00B294", "#24BFA5"]) assert.ok(insiders.includes(color), color);
  assert.ok(!insiders.includes("#007ACC"), "Insiders 不能还是蓝色");
  assert.match(stable, /viewBox="0 0 100 100"/);
  const shape = (svg: string) => svg.replace(/#[0-9A-F]{6}/g, "").replace(/<title>[^<]*<\/title>/, "");
  assert.equal(shape(insiders), shape(stable));
  // 不内联脚本、不引用外部资源。
  for (const svg of [stable, insiders]) assert.doesNotMatch(svg, /<script|href="http/i);
});

test("图标来源写进了 LICENSE", () => {
  const license = read("assets/providers/LICENSE.txt");
  assert.match(license, /vscode-color\.svg、vscode-insiders-color\.svg/);
});
