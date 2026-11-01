import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { schemasteryFields } from "./extension-config-schema.ts";

describe("dsh 插件 Config → 设置表单字段", () => {
  it("按 Schemastery JSON 生成字段：文本框、密钥、数字范围、下拉、数组、嵌套对象", () => {
    const fields = schemasteryFields({
      type: "object",
      dict: {
        text: { type: "string", meta: { description: "人设", role: "textarea" } },
        key: { type: "string", meta: { required: true, role: "secret" } },
        n: { type: "number", meta: { default: 2, min: 1, max: 5 } },
        level: { type: "union", list: [{ type: "const", value: "a" }, { type: "const", value: "b", meta: { description: "B 档" } }], meta: { default: "a" } },
        tags: { type: "array", inner: { type: "string" } },
        nested: { type: "object", dict: { on: { type: "boolean", meta: { default: true } } } },
        mixed: { type: "union", list: [{ type: "string" }, { type: "number" }] },
        hidden: { type: "string", meta: { hidden: true } },
        any: { type: "any" },
      },
    });
    assert.deepEqual(fields.map((field) => [field.key, field.type]), [
      ["text", "text"], ["key", "string"], ["n", "number"], ["level", "select"], ["tags", "array"], ["nested", "object"],
    ]);
    assert.equal(fields[0].description, "人设");
    assert.equal(fields[1].secret, true);
    assert.deepEqual([fields[2].min, fields[2].max, fields[2].default], [1, 5, 2]);
    assert.deepEqual(fields[3].options, [{ value: "a", label: "a" }, { value: "b", label: "B 档" }]);
    assert.equal(fields[4].items?.type, "string");
    assert.deepEqual(fields[4].default, []);
    assert.equal(fields[5].properties?.[0].type, "boolean");
  });

  it("顶层不是 object 或没有字段时返回空数组（退回 JSON 编辑）", () => {
    assert.deepEqual(schemasteryFields(undefined), []);
    assert.deepEqual(schemasteryFields({ type: "string" }), []);
    assert.deepEqual(schemasteryFields({ type: "object", dict: { any: { type: "any" } } }), []);
  });
});
