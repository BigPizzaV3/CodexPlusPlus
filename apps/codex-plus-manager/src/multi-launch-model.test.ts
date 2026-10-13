import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  multiLaunchInstanceFailure,
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

  // Windows 上「点了多开没窗口」曾经完全静默，现在至少要把原因显示出来。
  it("启动失败时给出 worker 记录的原因", () => {
    const failed = instance({
      status: "failed",
      running: false,
      launchResult: { status: "failed", message: "调试端口 9456 未就绪" },
    });
    assert.equal(multiLaunchInstanceFailure(failed), "调试端口 9456 未就绪");

    assert.equal(
      multiLaunchInstanceFailure(instance({ status: "failed", running: false, launchResult: { status: "failed" } })),
      "Codex 实例启动失败，原因未记录。",
    );
  });

  it("没有失败结果时不给失败提示", () => {
    assert.equal(multiLaunchInstanceFailure(instance({ status: "stopped", running: false })), "");
    assert.equal(
      multiLaunchInstanceFailure(instance({ status: "stopped", running: false, launchResult: { status: "ready" } })),
      "",
    );
    // 已经跑起来的实例不该再显示上一次的失败结果。
    assert.equal(
      multiLaunchInstanceFailure(
        instance({ status: "running", running: true, launchResult: { status: "failed", message: "旧失败" } }),
      ),
      "",
    );
  });
});
