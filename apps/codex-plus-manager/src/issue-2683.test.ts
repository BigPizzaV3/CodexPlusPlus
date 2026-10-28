import assert from "node:assert/strict";
import test from "node:test";
import {
  modelMultiAgentMode,
  parseModelMetadataDocument,
  synchronizeModelMetadataMultiAgentVersion,
} from "./model-metadata.ts";

// #2683：每模型 multi_agent_version 的下拉同步与解析校验。
// 上游枚举只有 disabled/v1/v2，「不写该键」才是 Auto/继承——inherit 必须从文档
// 删除键而不是写 "auto" 字面值（schema 会拒绝）。

test("multi agent sync edits only the selected model and round-trips inherit", () => {
  const original = { models: [{ slug: "one", custom: 42 }, { slug: "two", multi_agent_version: "v2" }] };

  const changed = synchronizeModelMetadataMultiAgentVersion(JSON.stringify(original), "one", "v1");
  assert.ok(changed);
  assert.equal(changed.preview.metadata.multi_agent_version, "v1");
  assert.equal(modelMultiAgentMode(changed.preview.metadata), "v1");
  // 其它模型的声明与自定义字段原样保留。
  assert.deepEqual(JSON.parse(changed.document).models[1], original.models[1]);
  assert.equal(JSON.parse(changed.document).models[0].custom, 42);

  // inherit = 删除键，完整回到原文档。
  const reset = synchronizeModelMetadataMultiAgentVersion(changed.document, "one", "inherit");
  assert.ok(reset);
  assert.deepEqual(JSON.parse(reset.document), original);
  assert.equal(modelMultiAgentMode(reset.preview.metadata), "inherit");

  // disabled 是合法显式值。
  const disabled = synchronizeModelMetadataMultiAgentVersion(JSON.stringify(original), "one", "disabled");
  assert.ok(disabled);
  assert.equal(disabled.preview.metadata.multi_agent_version, "disabled");

  // 同 slug 多条目时拒绝改写（与 Lite 同规）。
  assert.equal(synchronizeModelMetadataMultiAgentVersion('{"models":[{"slug":"one"},{"slug":"one"}]}', "one", "v1"), null);
});

test("parse rejects multi_agent_version values outside the upstream enum", () => {
  const bad = parseModelMetadataDocument('{"models":[{"slug":"one","multi_agent_version":"auto"}]}', "one");
  assert.ok(!bad.ok);
  assert.match(bad.ok ? "" : bad.error, /multi_agent_version/);

  for (const value of ["v1", "v2", "disabled"]) {
    const ok = parseModelMetadataDocument(`{"models":[{"slug":"one","multi_agent_version":"${value}"}]}`, "one");
    assert.ok(ok.ok, value);
    assert.equal(modelMultiAgentMode(ok.ok ? ok.value.metadata : null), value);
  }
});
