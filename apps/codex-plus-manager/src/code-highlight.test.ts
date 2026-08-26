import assert from "node:assert/strict";
import test from "node:test";

import { tokenizeCode, type CodeLanguage, type CodeToken } from "./code-highlight.ts";

function kinds(text: string, language: CodeLanguage): string[][] {
  return tokenizeCode(text, language).map((line) => line.map((token) => `${token.kind}:${token.text}`));
}

function flatten(lines: CodeToken[][]): string {
  return lines.map((line) => line.map((token) => token.text).join("")).join("\n");
}

/** 着色器绝不能吞字符：拼回来必须等于原文。 */
function assertLossless(text: string, language: CodeLanguage): void {
  assert.equal(flatten(tokenizeCode(text, language)), text);
}

test("keeps line count and content identical to the input", () => {
  const toml = 'model = "gpt-5"\n\n[features]\ngoals = true\n';
  assert.equal(tokenizeCode(toml, "toml").length, toml.split("\n").length);
  assertLossless(toml, "toml");
  assertLossless('{\n  "OPENAI_API_KEY": "redacted"\n}\n', "json");
  assertLossless("", "toml");
  assertLossless("no trailing newline", "json");
});

test("colors toml keys, strings, headers and comments", () => {
  assert.deepEqual(kinds('# hi\n[model_providers.demo]\nname = "Demo"\n', "toml"), [
    ["comment:# hi"],
    ["section:[model_providers.demo]"],
    ["key:name", "plain: ", "operator:=", "plain: ", 'string:"Demo"'],
    [],
  ]);
});

test("colors toml scalars by type", () => {
  assert.deepEqual(kinds("a = 12\nb = true\nc = 1979-05-27T07:32:00Z\n", "toml"), [
    ["key:a", "plain: ", "operator:=", "plain: ", "number:12"],
    ["key:b", "plain: ", "operator:=", "plain: ", "boolean:true"],
    ["key:c", "plain: ", "operator:=", "plain: ", "date:1979-05-27T07:32:00Z"],
    [],
  ]);
});

test("treats quoted toml keys as keys and quoted values as strings", () => {
  assert.deepEqual(kinds('"weird key" = "value"\n', "toml"), [
    ['key:"weird key"', "plain: ", "operator:=", "plain: ", 'string:"value"'],
    [],
  ]);
});

test("colors keys inside toml inline tables", () => {
  assert.deepEqual(kinds('x = { a = "1", b = 2 }\n', "toml"), [
    [
      "key:x",
      "plain: ",
      "operator:=",
      "plain: ",
      "punct:{",
      "plain: ",
      "key:a",
      "plain: ",
      "operator:=",
      "plain: ",
      'string:"1"',
      "operator:,",
      "plain: ",
      "key:b",
      "plain: ",
      "operator:=",
      "plain: ",
      "number:2",
      "plain: ",
      "punct:}",
    ],
    [],
  ]);
});

test("spreads toml multiline strings across lines as strings", () => {
  assert.deepEqual(kinds('text = """\nline one\n"""\n', "toml"), [
    ["key:text", "plain: ", "operator:=", "plain: ", 'string:"""'],
    ["string:line one"],
    ['string:"""'],
    [],
  ]);
});

test("does not treat a hash inside a toml string as a comment", () => {
  assert.deepEqual(kinds('base_url = "https://x.dev/#frag"\n', "toml"), [
    ["key:base_url", "plain: ", "operator:=", "plain: ", 'string:"https://x.dev/#frag"'],
    [],
  ]);
});

test("separates json keys from json string values", () => {
  assert.deepEqual(kinds('{\n  "token": "abc",\n  "n": 1\n}\n', "json"), [
    ["punct:{"],
    ["plain:  ", 'key:"token"', "operator::", "plain: ", 'string:"abc"', "operator:,"],
    ["plain:  ", 'key:"n"', "operator::", "plain: ", "number:1"],
    ["punct:}"],
    [],
  ]);
});

test("colors json literals and comments", () => {
  assert.deepEqual(kinds('// note\n[true, false, null]\n', "json"), [
    ["comment:// note"],
    ["punct:[", "boolean:true", "operator:,", "plain: ", "boolean:false", "operator:,", "plain: ", "null:null", "punct:]"],
    [],
  ]);
});

test("survives unterminated json strings without eating the rest of the file", () => {
  assert.deepEqual(kinds('{"a": "oops\n"b": 1}\n', "json"), [
    ["punct:{", 'key:"a"', "operator::", "plain: ", 'string:"oops'],
    ['key:"b"', "operator::", "plain: ", "number:1", "punct:}"],
    [],
  ]);
  assertLossless('{"a": "oops\n"b": 1}\n', "json");
});

test("keeps escaped quotes inside json strings", () => {
  assert.deepEqual(kinds('{"a": "say \\"hi\\""}', "json"), [
    ["punct:{", 'key:"a"', "operator::", "plain: ", 'string:"say \\"hi\\""', "punct:}"],
  ]);
});
