import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  findRelayModelRouteIssue,
  modelRouteSaveRequiresRestart,
  PROTOCOL_PROXY_BASE_URL,
  settingsRequireLocalHelper,
  type RelayModelRouteProfile,
  type RelayModelRouteSettings,
} from "./model-routes.ts";

function profile(id: string, patch: Partial<RelayModelRouteProfile> = {}): RelayModelRouteProfile {
  return {
    id,
    name: id.toUpperCase(),
    baseUrl: `https://${id}.example/v1`,
    apiKey: `sk-${id}`,
    protocol: "responses",
    relayMode: "pureApi",
    officialMixApiKey: false,
    modelRoutes: [],
    ...patch,
  };
}

function settings(
  relayProfiles: RelayModelRouteProfile[],
  patch: Partial<RelayModelRouteSettings> = {},
): RelayModelRouteSettings {
  return {
    relayProfilesEnabled: true,
    enhancementsEnabled: false,
    activeRelayId: relayProfiles[0]?.id ?? "",
    relayProfiles,
    ...patch,
  };
}

test("model route inputs keep focus while editing and label the example placeholder", async () => {
  const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");

  assert.match(source, /key=\{`model-route-\$\{index\}`\}/);
  assert.doesNotMatch(source, /key=\{`\$\{route\.model\}-\$\{index\}`\}/);
  assert.match(source, /placeholder=\{t\("例：gpt-5\.6-luna"\)\}/);
  assert.match(source, /relayModelRoutesSettingsValidation\(validationSettings\)/);
  assert.match(
    source,
    /if \(requiresRestart\) \{[\s\S]*?actions\.restart\(true\)[\s\S]*?return;[\s\S]*?actions\.switchRelayProfile\(savedSettings, savedSettings\.activeRelayId\)/,
  );
});

test("saving a route target validates every reverse reference against the proposed settings", () => {
  const source = profile("source", {
    modelRoutes: [{ model: "gpt-5.6-luna", targetRelayId: "target", targetModel: "" }],
  });
  const secondSource = profile("second-source", {
    modelRoutes: [{ model: "gpt-5.6-terra", targetRelayId: "target", targetModel: "" }],
  });

  assert.equal(
    findRelayModelRouteIssue([source, secondSource], [source, secondSource, profile("target")]),
    null,
  );
  assert.equal(
    findRelayModelRouteIssue(
      [source, secondSource],
      [source, secondSource, profile("target", { protocol: "chatCompletions" })],
    )?.kind,
    "targetProtocol",
  );
  assert.equal(
    findRelayModelRouteIssue([source], [source, profile("target", { baseUrl: "" })])?.kind,
    "targetCredentials",
  );
  assert.equal(
    findRelayModelRouteIssue([source], [source, profile("target", { apiKey: "" })])?.kind,
    "targetCredentials",
  );
  assert.equal(
    findRelayModelRouteIssue([source], [source, profile("target", {
      relayMode: "official",
      officialMixApiKey: false,
    })])?.kind,
    "targetCredentials",
  );
});

test("the first active model route transitions from no helper to a required helper", () => {
  const target = profile("target");
  const source = profile("source");
  const before = settings([source, target]);
  const after = {
    ...before,
    relayProfiles: [
      profile("source", {
        modelRoutes: [{ model: "gpt-5.6-luna", targetRelayId: "target", targetModel: "" }],
      }),
      target,
    ],
  };

  assert.equal(settingsRequireLocalHelper(before), false);
  assert.equal(settingsRequireLocalHelper(after), true);
  assert.equal(modelRouteSaveRequiresRestart(before, after, source.baseUrl), true);
});

test("an interrupted first-route restart remains retryable until live config uses the proxy", () => {
  const target = profile("target");
  const routed = profile("source", {
    modelRoutes: [{ model: "gpt-5.6-luna", targetRelayId: "target", targetModel: "" }],
  });
  const saved = settings([routed, target]);

  assert.equal(modelRouteSaveRequiresRestart(saved, saved, routed.baseUrl), true);
  assert.equal(modelRouteSaveRequiresRestart(saved, saved, PROTOCOL_PROXY_BASE_URL), false);
});

test("the first active route restarts conservatively even when settings claim a helper mode", () => {
  const target = profile("target");
  const routed = profile("source", {
    modelRoutes: [{ model: "gpt-5.6-luna", targetRelayId: "target", targetModel: "" }],
  });
  const routedSettings = settings([routed, target]);

  assert.equal(
    modelRouteSaveRequiresRestart(
      settings([profile("source"), target], { enhancementsEnabled: true }),
      { ...routedSettings, enhancementsEnabled: true },
      routed.baseUrl,
    ),
    true,
  );
  assert.equal(settingsRequireLocalHelper(settings([profile("chat", { protocol: "chatCompletions" })])), true);
  assert.equal(settingsRequireLocalHelper(settings([profile("mixed", {
    relayMode: "official",
    officialMixApiKey: true,
  })])), true);
  assert.equal(settingsRequireLocalHelper(settings([profile("aggregate", {
    relayMode: "aggregate",
  })])), true);
});

test("disabled provider switching never rewrites live config for a first route", () => {
  const target = profile("target");
  const before = settings([profile("source"), target], { relayProfilesEnabled: false });
  const after = settings([
    profile("source", {
      modelRoutes: [{ model: "gpt-5.6-luna", targetRelayId: "target", targetModel: "" }],
    }),
    target,
  ], { relayProfilesEnabled: false });

  assert.equal(modelRouteSaveRequiresRestart(before, after, profile("source").baseUrl), false);
});
