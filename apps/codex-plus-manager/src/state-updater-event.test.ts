import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

/**
 * 函数式 setState 的 updater 不能读事件对象（#2705，供应商池填 API Key 白屏）。
 *
 * updater 会延迟到渲染阶段执行，那时 `event.currentTarget` 已被 React 置空，
 * 读 `.value` 直接抛 TypeError；管理器没有错误边界，整页白屏。
 * 正确写法是先在事件处理函数里取值，再把值传进 updater。
 */

const srcDir = import.meta.dirname;
const sources = readdirSync(srcDir)
  .filter((name) => /\.tsx?$/.test(name) && !name.includes(".test."))
  .map((name) => ({ name, text: readFileSync(path.join(srcDir, name), "utf8") }));

/** 从 `open` 处的左括号起找到配对的右括号，返回括号内文本（只看括号，不解析字符串）。 */
function balancedArguments(text: string, open: number): string {
  let depth = 0;
  for (let index = open; index < text.length; index += 1) {
    if (text[index] === "(") depth += 1;
    else if (text[index] === ")") {
      depth -= 1;
      if (depth === 0) return text.slice(open + 1, index);
    }
  }
  return text.slice(open + 1);
}

test("setState updater 里不读取 event.currentTarget / event.target", () => {
  const offenders: string[] = [];
  // setXxx((previous) => …) / setXxx(previous => …)
  const updater = /\bset[A-Z]\w*(\()\s*\(?\s*\w+\s*\)?\s*=>/g;
  for (const { name, text } of sources) {
    for (const match of text.matchAll(updater)) {
      const body = balancedArguments(text, match.index + match[0].indexOf("("));
      if (/\b(event|e|evt)\.(currentTarget|target)\b/.test(body)) {
        offenders.push(`${name}:${text.slice(0, match.index).split("\n").length}`);
      }
    }
  }
  assert.deepEqual(offenders, []);
});

test("检测本身能抓到单行与换行两种写法", () => {
  const body = (source: string) => {
    const match = /\bset[A-Z]\w*(\()\s*\(?\s*\w+\s*\)?\s*=>/.exec(source)!;
    return balancedArguments(source, match.index + match[0].indexOf("("));
  };
  assert.match(body("onChange={(event) => setDraft((previous) => ({ ...previous, key: event.currentTarget.value }))}"), /event\.currentTarget/);
  assert.match(body("onChange={(event) =>\n  setUrls((current) => ({\n    ...current,\n    [id]: event.target.value,\n  }))\n}"), /event\.target/);
  // 先取值再进 updater 的写法不算。
  assert.doesNotMatch(body("onChange={(event) => { const value = event.currentTarget.value; setDraft((previous) => ({ ...previous, value })); }}"), /event\./);
});
