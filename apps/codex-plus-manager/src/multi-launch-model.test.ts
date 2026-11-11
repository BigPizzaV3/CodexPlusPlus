import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import {
  multiLaunchInstanceCanRestart,
  multiLaunchInstanceCanStop,
  multiLaunchInstanceFailure,
  multiLaunchInstanceIsMain,
  multiLaunchInstanceLabel,
  multiLaunchInstanceRunning,
  multiLaunchInstanceStatus,
  multiLaunchInstanceVisible,
  runningInstancesForProfile,
  type MultiLaunchInstance,
} from "./multi-launch-model.ts";

const instance = (patch: Partial<MultiLaunchInstance> = {}): MultiLaunchInstance => ({
  instanceId: "deepseek",
  status: "running",
  ...patch,
});

describe("Codex 多开运行实例模型", () => {
  it("启动期间禁止结束和重启，其他状态保留原有操作", () => {
    const starting = instance({ status: " STARTING ", running: true });
    assert.equal(multiLaunchInstanceCanStop(starting), false);
    assert.equal(multiLaunchInstanceCanRestart(starting), false);
    for (const status of ["running", "unknown", "failed"]) {
      assert.equal(multiLaunchInstanceCanStop(instance({ status })), true);
      assert.equal(multiLaunchInstanceCanRestart(instance({ status })), true);
    }
    assert.equal(multiLaunchInstanceCanStop(instance({ status: "stopped" })), false);
    assert.equal(multiLaunchInstanceCanRestart(instance({ status: "stopped" })), true);
  });

  // #3025: the main row used to send "main" to the multi-instance RPCs, which always reject it.
  it("主 Codex 不能从多开页停止或重启", () => {
    const runningMain = instance({ instanceId: "main", isMain: true, status: "running", running: true });
    assert.equal(multiLaunchInstanceIsMain(runningMain), true);
    assert.equal(multiLaunchInstanceCanStop(runningMain), false);
    assert.equal(multiLaunchInstanceCanRestart(runningMain), false);

    assert.equal(multiLaunchInstanceIsMain(instance({ instanceId: " main " })), true);
    assert.equal(multiLaunchInstanceCanStop(instance({ instanceId: " main ", status: "running", running: true })), false);
    assert.equal(multiLaunchInstanceCanRestart(instance({ instanceId: "main", status: "stopped", running: false })), false);
    assert.equal(multiLaunchInstanceCanStop(instance({ isMain: true, status: "failed", running: false })), false);
    assert.equal(multiLaunchInstanceCanRestart(instance({ isMain: true, status: "unknown" })), false);

    assert.equal(multiLaunchInstanceIsMain(instance({ instanceId: "mainland" })), false);
    assert.equal(multiLaunchInstanceCanStop(instance({ instanceId: "mainland", status: "running" })), true);
    assert.equal(multiLaunchInstanceCanRestart(instance({ instanceId: "mainland", status: "stopped" })), true);
  });
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
    assert.equal(multiLaunchInstanceRunning(instance({ status: "starting" })), false);
    assert.equal(multiLaunchInstanceRunning(instance({ status: "running_degraded" })), true);
    assert.equal(multiLaunchInstanceRunning(instance({ status: "stopped", running: true })), true);
    assert.equal(multiLaunchInstanceRunning(instance({ status: "stopped" })), false);
  });

  it("实例列表保留运行中的主实例和未确认状态，只隐藏已停止实例", () => {
    const instances = [
      instance({ instanceId: "main", isMain: true, status: "stopped", running: true }),
      instance({ instanceId: "running", status: "running", running: true }),
      instance({ instanceId: "starting", status: "starting", running: false }),
      instance({ instanceId: "unknown", status: "unknown", running: false }),
      instance({ instanceId: "failed", status: "failed", running: false }),
      instance({ instanceId: "stopped", status: "stopped", running: false }),
      instance({ instanceId: "legacy-stopped", status: " STOPPED " }),
    ];
    assert.deepEqual(instances.filter(multiLaunchInstanceVisible).map((item) => item.instanceId), [
      "main", "running", "starting", "unknown", "failed",
    ]);
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
      multiLaunchInstanceFailure(instance({
        status: "unknown", running: false,
        launchResult: { status: "cleanup_failed", message: "实例清理未完成" },
      })),
      "实例清理未完成",
    );

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

  // #2801：多开页删除供应商前，要提醒还有哪些实例在用它。
  it("按供应商找出正在运行的多开实例，不含主 Codex 和已停止的", () => {
    const instances = [
      instance({ instanceId: "main", isMain: true, profileId: "ccs-a" }),
      instance({ instanceId: "a-1", profileId: " ccs-a " }),
      instance({ instanceId: "a-2", profileId: "ccs-a", status: "stopped", running: false }),
      instance({ instanceId: "b-1", profileId: "ccs-b" }),
    ];
    assert.deepEqual(runningInstancesForProfile(instances, "ccs-a").map((item) => item.instanceId), ["a-1"]);
    assert.deepEqual(runningInstancesForProfile(instances, "missing"), []);
    assert.deepEqual(runningInstancesForProfile(instances, "  "), []);
  });
});

describe("多开页主 Codex 的停止和重启接到真实入口", () => {
  const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  const commands = readFileSync(new URL("../src-tauri/src/commands.rs", import.meta.url), "utf8");

  it("主行置灰并说明侧栏停止、顶部重启，处理函数不会把 main 交给多开 RPC", () => {
    const screenStart = app.indexOf("function CodexMultiLaunchScreen(");
    const screen = app.slice(screenStart, app.indexOf("\nfunction ", screenStart + 1));
    assert.match(screen, /multiLaunchInstanceIsMain\(instance\)/);
    assert.match(screen, /主 Codex 请使用顶部的「重启 Codex」。/);
    assert.match(screen, /主 Codex 请使用侧栏的「停止 Codex」。/);
    assert.match(screen, /pointerEvents: "none"/);

    const restartStart = app.indexOf("const restartMultiLaunchInstance = async");
    const stopStart = app.indexOf("const stopMultiLaunchInstance = async");
    const launchStart = app.indexOf("const launch = async");
    const restartFn = app.slice(restartStart, stopStart);
    const stopFn = app.slice(stopStart, launchStart);
    assert.ok(restartFn.includes("multiLaunchInstanceIsMain(instance)"));
    assert.ok(restartFn.indexOf("multiLaunchInstanceIsMain(instance)") < restartFn.indexOf("restart_codex_multi_instance"));
    assert.ok(stopFn.includes("multiLaunchInstanceIsMain(instance)"));
    assert.ok(stopFn.indexOf("multiLaunchInstanceIsMain(instance)") < stopFn.indexOf("appConfirm"));
    assert.ok(stopFn.indexOf("multiLaunchInstanceIsMain(instance)") < stopFn.indexOf("stop_codex_multi_instance"));
  });

  it("多开 RPC 拒绝 main 时指向同样的入口", () => {
    assert.match(commands, /主 Codex 请使用侧栏的「停止 Codex」。/);
    assert.match(commands, /主 Codex 请使用顶部的「重启 Codex」。/);
    assert.doesNotMatch(commands, /主进程请使用重启 Codex 或退出管理器来结束/);
    assert.doesNotMatch(commands, /主进程请使用顶部的重启 Codex/);
  });
});
