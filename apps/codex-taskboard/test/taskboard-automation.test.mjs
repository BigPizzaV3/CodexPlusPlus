import assert from "node:assert/strict";
import { test } from "node:test";

import {
  buildTaskboardAutomationName,
  buildTaskboardAutomationPrompt,
  buildTaskboardAutomationSpec,
  parseTaskboardAutomationHostRequest,
  reconcileTaskboardAutomation,
} from "../shared/taskboard-automation.mjs";
import {
  AUTOMATION_MODELS,
  isSupportedModelEffort,
  withAutomationModel,
} from "../shared/taskboard-automation-options.mjs";

const baseRequest = {
  id: "host-request-1",
  action: "automation",
  requestId: "iframe-request-1",
  operation: "ensure-active",
  taskboardProjectId: "ppt-skill",
  codexProjectId: "codex-project-123",
  projectName: "PPT Skill",
  workspacePath: "/Users/example/Documents/ppt-skill",
  skillPath: "/Users/example/taskboard/skills/manage-taskboard/SKILL.md",
  intervalMinutes: 5,
  model: "gpt-5.5",
  reasoningEffort: "high",
};

test("the automation model catalog matches Codex and normalizes unsupported efforts", () => {
  assert.deepEqual(AUTOMATION_MODELS, [
    {
      label: "5.6 Sol",
      slug: "gpt-5.6-sol",
      defaultEffort: "low",
      efforts: ["low", "medium", "high", "xhigh", "max", "ultra"],
    },
    {
      label: "5.6 Terra",
      slug: "gpt-5.6-terra",
      defaultEffort: "medium",
      efforts: ["low", "medium", "high", "xhigh", "max", "ultra"],
    },
    {
      label: "5.6 Luna",
      slug: "gpt-5.6-luna",
      defaultEffort: "medium",
      efforts: ["low", "medium", "high", "xhigh", "max"],
    },
    {
      label: "5.5",
      slug: "gpt-5.5",
      defaultEffort: "medium",
      efforts: ["low", "medium", "high", "xhigh"],
    },
    {
      label: "5.4",
      slug: "gpt-5.4",
      defaultEffort: "medium",
      efforts: ["low", "medium", "high", "xhigh"],
    },
    {
      label: "5.4 Mini",
      slug: "gpt-5.4-mini",
      defaultEffort: "medium",
      efforts: ["low", "medium", "high", "xhigh"],
    },
  ]);

  const current = {
    status: "ACTIVE",
    intervalMinutes: 5,
    model: "gpt-5.6-sol",
    reasoningEffort: "ultra",
  };
  assert.deepEqual(withAutomationModel(current, "gpt-5.6-terra"), {
    ...current,
    model: "gpt-5.6-terra",
  });
  assert.deepEqual(withAutomationModel(current, "gpt-5.6-luna"), {
    ...current,
    model: "gpt-5.6-luna",
    reasoningEffort: "medium",
  });
});

test("the automation host request accepts only whitelisted project automation options", () => {
  assert.deepEqual(parseTaskboardAutomationHostRequest(baseRequest), baseRequest);
  assert.equal(
    parseTaskboardAutomationHostRequest({ ...baseRequest, operation: "delete" }),
    null,
  );
  assert.equal(
    parseTaskboardAutomationHostRequest({ ...baseRequest, method: "automation-delete" }),
    null,
  );
  assert.equal(
    parseTaskboardAutomationHostRequest({ ...baseRequest, prompt: "arbitrary" }),
    null,
  );
  assert.deepEqual(
    parseTaskboardAutomationHostRequest({ ...baseRequest, intervalMinutes: 10 }),
    { ...baseRequest, intervalMinutes: 10 },
  );
  assert.equal(
    parseTaskboardAutomationHostRequest({ ...baseRequest, intervalMinutes: 7 }),
    null,
  );
  assert.equal(
    parseTaskboardAutomationHostRequest({
      ...baseRequest,
      model: "gpt-5.6-sol",
      reasoningEffort: "ultra",
    })?.reasoningEffort,
    "ultra",
  );
  assert.equal(
    parseTaskboardAutomationHostRequest({ ...baseRequest, model: "gpt-future" }),
    null,
  );
  assert.equal(
    parseTaskboardAutomationHostRequest({ ...baseRequest, reasoningEffort: "xhigh" })?.reasoningEffort,
    "xhigh",
  );
  assert.equal(
    parseTaskboardAutomationHostRequest({
      ...baseRequest,
      model: "gpt-5.4",
      reasoningEffort: "ultra",
    }),
    null,
  );
  const allEfforts = ["low", "medium", "high", "xhigh", "max", "ultra"];
  for (const intervalMinutes of [5, 10, 15, 30, 60]) {
    for (const model of AUTOMATION_MODELS) {
      for (const effort of allEfforts) {
        assert.equal(
          parseTaskboardAutomationHostRequest({
            ...baseRequest,
            intervalMinutes,
            model: model.slug,
            reasoningEffort: effort,
          }) !== null,
          model.efforts.includes(effort),
          `${intervalMinutes}m/${model.slug}/${effort}`,
        );
      }
    }
  }
  assert.equal(isSupportedModelEffort("gpt-5.6-luna", "max"), true);
  assert.equal(isSupportedModelEffort("gpt-5.6-luna", "ultra"), false);
  assert.equal(
    parseTaskboardAutomationHostRequest({ ...baseRequest, workspacePath: "relative/path" }),
    null,
  );
});

test("the stable name and generated prompt are project-scoped and encode the claim protocol", () => {
  assert.equal(
    buildTaskboardAutomationName(baseRequest),
    "Taskboard 自动认领 · ppt-skill",
  );

  const prompt = buildTaskboardAutomationPrompt(baseRequest);
  assert.match(
    prompt,
    /\[\$manage-taskboard\]\(\/Users\/example\/taskboard\/skills\/manage-taskboard\/SKILL\.md\)/,
  );
  assert.match(prompt, /\[\$manage-taskboard\]\([^)]*\) e-taskboard /);
  assert.match(prompt, /PPT Skill/);
  assert.match(prompt, /每 5 分钟检查/);
  assert.match(prompt, /ppt-skill/);
  assert.match(prompt, /\/Users\/example\/Documents\/ppt-skill/);
  assert.match(prompt, /每次仅处理一个 todo/);
  assert.match(prompt, /issue get/);
  assert.match(prompt, /comment list/);
  assert.match(prompt, /最新 version/);
  assert.match(prompt, /in_progress/);
  assert.match(prompt, /版本冲突.*跳过/);
  assert.match(prompt, /关键改动、验证结果、执行结果和剩余风险/);
  assert.match(prompt, /in_review/);
  assert.match(prompt, /已绑定.*branch.*worktree/);
});

test("the generated cron spec uses the selected whitelisted local Codex options", () => {
  assert.deepEqual(buildTaskboardAutomationSpec(baseRequest), {
    kind: "cron",
    name: "Taskboard 自动认领 · ppt-skill",
    prompt: buildTaskboardAutomationPrompt(baseRequest),
    projectId: "codex-project-123",
    executionEnvironment: "local",
    localEnvironmentConfigPath: null,
    model: "gpt-5.5",
    reasoningEffort: "high",
    rrule: "RRULE:FREQ=MINUTELY;INTERVAL=5",
  });
  assert.deepEqual(buildTaskboardAutomationSpec({
    ...baseRequest,
    intervalMinutes: 30,
    model: "gpt-5.4",
    reasoningEffort: "medium",
  }), {
    ...buildTaskboardAutomationSpec(baseRequest),
    prompt: buildTaskboardAutomationPrompt({ ...baseRequest, intervalMinutes: 30 }),
    model: "gpt-5.4",
    reasoningEffort: "medium",
    rrule: "RRULE:FREQ=MINUTELY;INTERVAL=30",
  });
});

test("ensure-active updates a matching automation by id with a complete active spec", async () => {
  const existing = {
    id: "automation-1",
    status: "PAUSED",
    kind: "cron",
    name: "Taskboard 自动认领 · ppt-skill",
    prompt: "old prompt",
    projectId: "old-project",
    executionEnvironment: "local",
    localEnvironmentConfigPath: null,
    model: "gpt-5.5",
    reasoningEffort: "medium",
    rrule: "FREQ=HOURLY",
    createdAt: "2026-07-25T00:00:00.000Z",
    internalRevision: 4,
  };
  const calls = [];
  const response = await reconcileTaskboardAutomation(
    { ...baseRequest, automationId: "automation-1" },
    async (method, params) => {
      calls.push({ method, params });
      if (method === "list-automations") return { items: [existing] };
      return { item: params };
    },
  );

  const spec = buildTaskboardAutomationSpec(baseRequest);
  assert.deepEqual(calls, [
    { method: "list-automations", params: {} },
    {
      method: "automation-update",
      params: {
        ...spec,
        id: "automation-1",
        status: "ACTIVE",
      },
    },
  ]);
  assert.deepEqual(response, {
    item: { ...spec, id: "automation-1", status: "ACTIVE" },
  });
});

test("ensure-active is idempotent when the listed automation already matches", async () => {
  const existing = {
    id: "automation-1",
    status: "ACTIVE",
    ...buildTaskboardAutomationSpec(baseRequest),
    createdAt: "2026-07-25T00:00:00.000Z",
  };
  const calls = [];
  const response = await reconcileTaskboardAutomation(
    { ...baseRequest, automationId: "automation-1" },
    async (method, params) => {
      calls.push({ method, params });
      return { items: [existing] };
    },
  );

  assert.deepEqual(calls, [{ method: "list-automations", params: {} }]);
  assert.deepEqual(response, { item: existing });
});

test("a foreign automation id never grants control outside the project", async () => {
  const foreign = {
    id: "foreign-automation",
    status: "ACTIVE",
    ...buildTaskboardAutomationSpec({
      ...baseRequest,
      taskboardProjectId: "another-project",
    }),
  };
  const ensureCalls = [];
  await reconcileTaskboardAutomation(
    { ...baseRequest, automationId: foreign.id },
    async (method, params) => {
      ensureCalls.push({ method, params });
      if (method === "list-automations") return { items: [foreign] };
      return { item: params };
    },
  );
  assert.deepEqual(ensureCalls, [
    { method: "list-automations", params: {} },
    { method: "automation-create", params: buildTaskboardAutomationSpec(baseRequest) },
  ]);

  const pauseCalls = [];
  const paused = await reconcileTaskboardAutomation(
    { ...baseRequest, operation: "pause", automationId: foreign.id },
    async (method, params) => {
      pauseCalls.push({ method, params });
      return { items: [foreign] };
    },
  );
  assert.deepEqual(pauseCalls, [{ method: "list-automations", params: {} }]);
  assert.deepEqual(paused, { error: "not-found" });
});

test("ensure-active falls back to the stable name and otherwise creates", async () => {
  const matching = {
    id: "automation-by-name",
    status: "PAUSED",
    ...buildTaskboardAutomationSpec(baseRequest),
  };
  const updateCalls = [];
  await reconcileTaskboardAutomation(baseRequest, async (method, params) => {
    updateCalls.push({ method, params });
    if (method === "list-automations") return { items: [matching] };
    return { item: params };
  });
  assert.equal(updateCalls[1].method, "automation-update");
  assert.equal(updateCalls[1].params.id, "automation-by-name");

  const createCalls = [];
  const created = await reconcileTaskboardAutomation(baseRequest, async (method, params) => {
    createCalls.push({ method, params });
    if (method === "list-automations") return { items: [] };
    return { item: { id: "created-1", status: "ACTIVE", ...params } };
  });
  assert.deepEqual(createCalls, [
    { method: "list-automations", params: {} },
    { method: "automation-create", params: buildTaskboardAutomationSpec(baseRequest) },
  ]);
  assert.equal(created.item.id, "created-1");
});

test("pause never creates and list returns only sanitized matching project automations", async () => {
  const matching = {
    id: "matching",
    status: "ACTIVE",
    ...buildTaskboardAutomationSpec(baseRequest),
    untrustedListField: "must not be echoed into an update",
  };
  const unrelated = {
    id: "unrelated",
    status: "ACTIVE",
    ...buildTaskboardAutomationSpec({
      ...baseRequest,
      taskboardProjectId: "another-project",
    }),
  };

  const pauseCalls = [];
  const paused = await reconcileTaskboardAutomation(
    { ...baseRequest, operation: "pause" },
    async (method, params) => {
      pauseCalls.push({ method, params });
      if (method === "list-automations") return { items: [unrelated, matching] };
      return { item: params };
    },
  );
  assert.deepEqual(pauseCalls, [
    { method: "list-automations", params: {} },
    {
      method: "automation-update",
      params: {
        ...buildTaskboardAutomationSpec(baseRequest),
        id: "matching",
        status: "PAUSED",
      },
    },
  ]);
  assert.deepEqual(paused, {
    item: {
      ...buildTaskboardAutomationSpec(baseRequest),
      id: "matching",
      status: "PAUSED",
    },
  });

  const notFoundCalls = [];
  const notFound = await reconcileTaskboardAutomation(
    { ...baseRequest, operation: "pause", taskboardProjectId: "missing" },
    async (method, params) => {
      notFoundCalls.push({ method, params });
      return { items: [matching, unrelated] };
    },
  );
  assert.deepEqual(notFoundCalls, [{ method: "list-automations", params: {} }]);
  assert.deepEqual(notFound, { error: "not-found" });

  const listed = await reconcileTaskboardAutomation(
    { ...baseRequest, operation: "list" },
    async () => ({ items: [unrelated, matching] }),
  );
  assert.deepEqual(listed, {
    items: [{
      id: "matching",
      status: "ACTIVE",
      model: "gpt-5.5",
      reasoningEffort: "high",
      rrule: "RRULE:FREQ=MINUTELY;INTERVAL=5",
    }],
  });

  const invalidPair = {
    ...matching,
    id: "invalid-pair",
    model: "gpt-5.4",
    reasoningEffort: "ultra",
  };
  const invalidListed = await reconcileTaskboardAutomation(
    { ...baseRequest, operation: "list" },
    async () => ({ items: [invalidPair] }),
  );
  assert.deepEqual(invalidListed, { items: [] });
});

test("pause is idempotent for an already paused matching automation", async () => {
  const matching = {
    id: "matching",
    status: "PAUSED",
    ...buildTaskboardAutomationSpec(baseRequest),
  };
  const calls = [];
  const response = await reconcileTaskboardAutomation(
    { ...baseRequest, operation: "pause" },
    async (method, params) => {
      calls.push({ method, params });
      return { items: [matching] };
    },
  );
  assert.deepEqual(calls, [{ method: "list-automations", params: {} }]);
  assert.deepEqual(response, { item: matching });
});
