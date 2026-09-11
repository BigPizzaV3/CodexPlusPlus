import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const root = import.meta.dirname;
const contract = JSON.parse(fs.readFileSync(path.join(root, "mobile-bridge-contract.json"), "utf8"));
const expected = ["status", "pair", "confirm", "tasks", "send-input", "stop"];
assert.deepEqual(Object.keys(contract.endpoints), expected);
assert.equal(contract.security.deviceKeysLeaveDevice, false);
for (const [name, endpoint] of Object.entries(contract.endpoints)) {
  assert.match(endpoint.path, /^\/v1\/mobile\//, name);
}
console.log(JSON.stringify({ ok: true, contractVersion: contract.contractVersion, endpoints: expected.length }));
