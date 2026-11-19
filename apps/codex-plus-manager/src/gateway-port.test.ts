/** #3082：网关设置里的「本地网关端口」入口与端口校验。 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { MAX_GATEWAY_PORT, MIN_GATEWAY_PORT, parseGatewayPort } from "./gateway-model.ts";
import { EN_PLAIN, EN_TEMPLATE } from "./i18n-en.ts";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

test("端口只接受 1024–65535 的整数", () => {
  assert.equal(MIN_GATEWAY_PORT, 1024);
  assert.equal(MAX_GATEWAY_PORT, 65535);
  for (const bad of ["", " ", "0", "80", "1023", "65536", "99999", "123456", "57321.5", "-1", "abc", "5e4", "0x1000"]) {
    assert.equal(parseGatewayPort(bad), null, bad);
  }
  assert.equal(parseGatewayPort("1024"), 1024);
  assert.equal(parseGatewayPort(" 57400 "), 57400);
  assert.equal(parseGatewayPort("65535"), 65535);
});

test("网关设置的「网关选项」里有本地网关端口一行，读写走专用命令", () => {
  const page = read("GatewayPage.tsx");
  const options = page.slice(page.indexOf('<CxSection title={t("网关选项")}>'));
  assert.match(options, /<GatewayPortRow notify=\{notify\} \/>/, "端口一行放在网关选项里");
  assert.match(page, /invoke<GatewayPortResult>\("load_gateway_port"\)/);
  assert.match(page, /invoke<GatewayPortResult>\("save_gateway_port", \{ port: parsed \}\)/);
  // 填错不能保存；环境变量指定了端口时输入框禁用并说明原因。
  assert.match(page, /disabled=\{!dirty \|\| saving \|\| envLocked\}/);
  assert.match(page, /disabled=\{envLocked \|\| saving\}/);
  assert.match(page, /CODEX_PLUS_PROTOCOL_PROXY_PORT 指定为 \{0\}，这里不能修改/);
  // 保存后明确提示要重启才生效。
  assert.match(page, /需要重启 Codex\+\+ 并重新接入 Agent 后生效/);
});

test("后端命令已登记，payload 是结构体", () => {
  const lib = read("../src-tauri/src/lib.rs");
  assert.match(lib, /gateway_commands::load_gateway_port,/);
  assert.match(lib, /gateway_commands::save_gateway_port,/);
  const commands = read("../src-tauri/src/gateway_commands.rs");
  assert.match(commands, /pub fn load_gateway_port\(\) -> CommandResult<GatewayPortPayload>/);
  assert.match(commands, /pub async fn save_gateway_port\(port: u32\) -> CommandResult<GatewayPortPayload>/);
  assert.match(commands, /pub struct GatewayPortPayload \{/);
});

test("端口一行的文案都有英文", () => {
  for (const key of ["本地网关端口", "待重启", "本地网关端口已保存", "保存本地网关端口失败"]) assert.ok(EN_PLAIN[key], key);
  for (const key of [
    "已由环境变量 CODEX_PLUS_PROTOCOL_PROXY_PORT 指定为 {0}，这里不能修改。",
    "端口要在 {0}–{1} 之间。",
    "已改为 {0}，当前仍在 {1} 上监听：需要重启 Codex++ 并重新接入 Agent 后生效。",
    "和其它程序端口冲突时换一个（默认 {0}）。保存后需要重启 Codex++ 并重新接入 Agent 后生效。",
  ]) {
    assert.ok(EN_TEMPLATE[key], key);
  }
});
