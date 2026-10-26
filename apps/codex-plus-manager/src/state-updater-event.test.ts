import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

/**
 * 函数式 setState 的 updater 不能读事件对象（#2705）。
 *
 * updater 会延迟到渲染阶段执行，那时 `event.currentTarget` 已被 React 置空，
 * 读 `.value` 直接抛 TypeError；管理器没有错误边界，整页白屏。
 * 正确写法是先在事件处理函数里取值，再把值传进 updater。
 */

const root = path.resolve(import.meta.dirname, "..");
const sources = readdirSync(path.join(root, "src"))
  .filter((name) => name.endsWith(".tsx"))
  .map((name) => ({ name, text: readFileSync(path.join(root, "src", name), "utf8") }));

test("setState updater 里不读取 event.currentTarget / event.target", () => {
  const offenders: string[] = [];
  const updater = /set[A-Z]\w*\(\s*\((\w+)\)\s*=>/g;
  for (const { name, text } of sources) {
    for (const match of text.matchAll(updater)) {
      // 只看 updater 开头一小段：足够覆盖单行和换行写法的对象字面量
      const body = text.slice(match.index, match.index + 240).split(/\)\s*\)?\s*[;}\n]\s*\n/)[0];
      if (/\bevent\.(currentTarget|target)\b/.test(body)) {
        const line = text.slice(0, match.index).split("\n").length;
        offenders.push(`${name}:${line}`);
      }
    }
  }
  assert.deepEqual(offenders, []);
});
