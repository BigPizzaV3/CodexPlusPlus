import assert from "node:assert/strict";
import test from "node:test";

import { resolveProviderName, tomlSectionStringValue } from "./provider-name.ts";

// issue #1886：用户把 name 改成 OpenAi，保存/重启后不能被改回 custom
test("keeps a user-customized provider name", () => {
  const contents = `model_provider = "openai"

[model_providers.custom]
name = "OpenAi"
wire_api = "responses"
`;
  assert.equal(resolveProviderName(contents, "custom"), "OpenAi");
});

test("falls back to the table id when name is missing", () => {
  const contents = `model_provider = "custom"

[model_providers.custom]
wire_api = "responses"
`;
  assert.equal(resolveProviderName(contents, "custom"), "custom");
});

test("falls back to the table id when name is blank", () => {
  const contents = `[model_providers.custom]
name = "   "
`;
  assert.equal(resolveProviderName(contents, "custom"), "custom");
});

test("falls back when the provider table does not exist yet", () => {
  assert.equal(resolveProviderName("model = \"gpt-5.6\"\n", "custom"), "custom");
});

test("only reads the name from the requested section", () => {
  const contents = `[model_providers.other]
name = "Other"

[model_providers.custom]
name = "Mine"
`;
  assert.equal(resolveProviderName(contents, "custom"), "Mine");
  assert.equal(resolveProviderName(contents, "other"), "Other");
});

test("does not leak a root-level name into the section lookup", () => {
  const contents = `name = "root-level"

[model_providers.custom]
wire_api = "responses"
`;
  assert.equal(tomlSectionStringValue(contents, "model_providers.custom", "name"), "");
  assert.equal(resolveProviderName(contents, "custom"), "custom");
});

test("handles single quotes and escaped characters", () => {
  const contents = `[model_providers.custom]
name = 'My \\'Relay\\''
`;
  assert.equal(resolveProviderName(contents, "custom"), "My 'Relay'");
});
