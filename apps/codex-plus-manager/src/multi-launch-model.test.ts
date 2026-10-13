import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  multiLaunchInstanceLabel,
  multiLaunchInstanceRunning,
  multiLaunchInstanceStatus,
  type MultiLaunchInstance,
} from "./multi-launch-model.ts";

const instance = (patch: Partial<MultiLaunchInstance> = {}): MultiLaunchInstance => ({
  instanceId: "deepseek",
  status: "running",
  ...patch,
});

describe("Codex 多开运行实例模型", () => {
  it("主 Codex 优先使用主实例标签", () => {
    assert.equal(multiLaunchInstanceLabel(instance({ isMain: true, profileName: "旧名称" })), "主 Codex");
  });

  it("多开实例按供应商名称回退到实例 ID", () => {
    assert.equal(multiLaunchInstanceLabel(instance({ profileName: "  Zhipu GLM  " })), "Zhipu GLM");
    assert.equal(multiLaunchInstanceLabel(instance({ profileName: "", label: "GLM" })), "GLM");
    assert.equal(multiLaunchInstanceLabel(instance({ profileName: "", label: "" })), "deepseek");
  });

  it("状态归一化并识别可结束的运行态", () => {
    assert.equal(multiLaunchInstanceStatus(instance({ status: "  RUNNING " })), "running");
    assert.equal(multiLaunchInstanceRunning(instance({ status: "starting" })), true);
    assert.equal(multiLaunchInstanceRunning(instance({ status: "running_degraded" })), true);
    assert.equal(multiLaunchInstanceRunning(instance({ status: "stopped", running: true })), true);
    assert.equal(multiLaunchInstanceRunning(instance({ status: "stopped" })), false);
  });
});
