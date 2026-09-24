const assert = require("node:assert/strict");
const { permitsExternalApi } = require("./api-quota-gate.js");

const profile = { id: "api", relayMode: "official", officialMixApiKey: true, upstreamBaseUrl: "https://proxy.example/v1" };
const settings = { relayProfilesEnabled: true, activeRelayId: "api", relayProfiles: [profile], activeRelaySessionProvider: "openai" };
assert.equal(permitsExternalApi(settings, "local"), true, "API transport may keep openai session identity");
for (const host of ["remote", "durable", "", null]) assert.equal(permitsExternalApi(settings, host), false);
for (const patch of [{relayProfilesEnabled:false}, {activeRelayId:"missing"}, {relayProfiles:null}, {relayProfiles:{}}]) {
  assert.equal(permitsExternalApi({...settings,...patch}, "local"), false);
}
for (const upstreamBaseUrl of [
  "https://api.openai.com/v1",
  "https://chatgpt.com/backend-api",
  "https://sub.openai.com/v1",
  "https://proxy.example/v1",
  "file:///tmp",
  "",
]) assert.equal(permitsExternalApi({...settings,relayProfiles:[{...profile,upstreamBaseUrl}]}, "local"), true);
for (const patch of [
  {officialMixApiKey:false}, {relayMode:"pureApi"},
]) assert.equal(permitsExternalApi({...settings,relayProfiles:[{...profile,...patch}]}, "local"), false);
console.log("external relay official-mix policy passed");
