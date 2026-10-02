import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const require = createRequire(import.meta.url);
const { inspectService, inspectEntry } = require("../../../assets/native-browser/inspect-service.cjs");

// A small protocol specimen, not a copy of the distributed runtime.
const specimen = readFileSync(new URL(
  "../../../assets/native-browser/fixtures/service-specimen.mjs", import.meta.url,
), "utf8");

test("structural admission ignores formatting, comments and minifier identifiers", () => {
  const result = inspectService(specimen);
  assert.equal(Buffer.from(specimen).subarray(result.start, result.end).toString(), "policy");
  const renamed = specimen.replace("var Browser", "var RenamedBrowser")
    .replace("new Browser", "new RenamedBrowser")
    .replace("function metadata", "function renamedMetadata")
    .replace("()=>metadata", "()=>renamedMetadata")
    .replace("function policy", "function renamedPolicy")
    .replace(",policy)", ",renamedPolicy)");
  const current = inspectService(`/* non-ASCII offset: \u4e2d */\n${renamed}`);
  assert.equal(current.metadata, "renamedMetadata");
  assert.equal(current.policy, "renamedPolicy");
  assert.equal(current.contractSha, result.contractSha);
});

test("structural admission refuses protocol, metadata, handshake and constructor drift", () => {
  for (const source of [
    specimen.replace("o.agent_request_header_enabled=i", "o.agent_request_header_enabled=false"),
    specimen.replace("this.sendRequest(r,{...n,...o})", "this.sendRequest(r,{...n})"),
    specimen.replace("await this.readRequestHeaderEnabled()", "this.readRequestHeaderEnabled()"),
    specimen.replace("this.sendRequest(r,{...n,...o})", "this.sendRequest(r,{...o,...n})"),
    specimen.replace("this.sendRequest(r,{...n,...o})", "this.sendRequest(r,{...n,...o,agent_request_header_enabled:false})"),
    specimen.replace("o.agent_request_header_enabled=i,", "i=false;o.agent_request_header_enabled=i,"),
    specimen.replace("x-codex-turn-metadata", "untrusted-metadata"),
    specimen.replace("this.readRequestHeaderEnabled=s", "this.readRequestHeaderEnabled=n"),
    specimen.replace("this.clientInfo=r,r", "this.clientInfo={},r"),
    specimen.replace("return await identity,", "return false,"),
    specimen.replace("return e;", "return {};"),
    specimen.replace("this.runtime),this.turnEndedTracker,policy",
      "this.runtime),this.turnEndedTracker,()=>policy()"),
    `${specimen}\nfunction second(){return new Browser(r,this.clientApi,()=>metadata(this.runtime),this.turnEndedTracker,policy);}`,
    `${specimen}\nfunction cppNativeIdentificationReader(){}`,
    specimen.replace("function refresh() {", "function refresh() { const metadata=()=>({});"),
    specimen.replace("function refresh() {", "function refresh() { const policy=()=>false;"),
    specimen.replace("var Browser", "let Browser") + "\nBrowser = Other;",
    "/* new Browser(...this.turnEndedTracker,policy) */",
    "this is not JavaScript",
  ]) assert.throws(() => inspectService(source));
});

test("unknown method shapes qualify by binding and header data flow, not a hash list", () => {
  for (const source of [
    specimen.replace("let o=this.getSessionParams();", "this.metrics=(this.metrics||0)+1;let o=this.getSessionParams();"),
    specimen.replaceAll("getUserTabs", "getUserTabs2"),
    specimen.replaceAll("sendSessionRequest", "sendSessionRequest2"),
    specimen.replace("constructor(r,n,o,i,s) {", "constructor(r,n,o,i,s) { this.metrics=0;"),
  ]) assert.doesNotThrow(() => inspectService(source));
});

test("entry admission accepts formatting and diagnostics, but requires the bound launch signature", () => {
  const entry = 'import * as native from "@oai/cua-repl"; await native.launch();';
  assert.doesNotThrow(() => inspectEntry(entry));
  assert.doesNotThrow(() => inspectEntry(`#!/usr/bin/env node\n// updated entry\n${entry}\nconsole.debug("started");`));
  for (const changed of [
    entry.replace("@oai/cua-repl", "other-package"),
    entry.replace("await native.launch()", "await native.launch(1)"),
    entry.replace("await native.launch()", "native.launch()"),
    `${entry}\nawait native.launch();`,
    `${entry}\nfunction shadow(native){return native.launch();}\nnative=other;`,
    "not JavaScript",
  ]) assert.throws(() => inspectEntry(changed));
});

test("a generated bundle and source inspector are both shipped", () => {
  const source = readFileSync(new URL("../../../assets/native-browser/inspect-service.mjs", import.meta.url), "utf8");
  const bundle = readFileSync(new URL("../../../assets/native-browser/inspect-service.cjs", import.meta.url), "utf8");
  assert.ok(source.includes("protocolShapes"));
  assert.ok(bundle.includes(`Inspector source SHA256: ${createHash("sha256").update(source).digest("hex")}`));
  assert.ok(bundle.includes("@babel/parser 7.29.3"));
  assert.ok(bundle.includes("Permission is hereby granted"));
});

test("real local service fixtures retain their structural contract", {
  skip: !process.env.CPP_BROWSER_SERVICE_FIXTURES,
}, () => {
  const paths: string[] = JSON.parse(process.env.CPP_BROWSER_SERVICE_FIXTURES!);
  assert.ok(paths.length >= 3);
  for (const path of paths) {
    const source = readFileSync(path, "utf8");
    const binding = inspectService(source);
    const bytes = Buffer.from(source);
    assert.equal(bytes.subarray(binding.start, binding.end).toString(), binding.policy);
    const helper = readFileSync(new URL("../../../assets/native-browser/require-identification.mjs", import.meta.url), "utf8");
    const callback = `cppNativeIdentificationReader(this.runtime,${binding.policy},${binding.metadata},"C:/fixture/control.json")`;
    const candidate = Buffer.concat([
      bytes.subarray(0, binding.start), Buffer.from(callback), bytes.subarray(binding.end),
      Buffer.from(`\n${helper}`),
    ]).toString();
    // Parse only: no proprietary browser, policy or approval implementation runs.
    require("@babel/parser").parse(candidate, { sourceType: "module" });
    assert.throws(() => inspectService(candidate));
    assert.throws(() => inspectService(source.replaceAll(
      "codex_browser_use_agent_request_header", "unrelated_feature",
    )));
  }
});
