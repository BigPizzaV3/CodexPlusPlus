import assert from "node:assert/strict";
import { RemoteReferenceHarness } from "./local-reference-harness.mjs";

const clock = { value: Date.parse("2026-08-18T00:00:00Z") };
const harness = new RemoteReferenceHarness({ now: () => new Date(clock.value) });
const binding = {
  bindingId: "binding_00000001",
  appDeviceId: "app_device_0001",
  pcDeviceId: "pc_device_000001",
  installationId: "installation_001",
  bindingEpoch: 1,
  environment: "dev",
  localConfirmed: true,
  pcConnectionState: "online",
};
harness.registerBinding(binding);

const baseSnapshot = {
  remoteTaskId: "remote_task_0001",
  pcDeviceId: binding.pcDeviceId,
  installationId: binding.installationId,
  bindingEpoch: binding.bindingEpoch,
  name: "测试任务",
  workspaceName: "隔离工作区",
  taskStatus: "created",
  turnStatus: "idle",
  lastTurnOutcome: "none",
  lastReply: null,
  lastReplyState: "absent",
  lastReplyVersion: null,
  lastError: null,
  pcObservedAt: "2026-08-18T00:00:00Z",
  stateVersion: 1,
  pcConnectionState: "online",
};
assert.equal(
  harness.upsertSnapshot({ ...binding, snapshot: baseSnapshot }).accepted,
  true,
  "初始快照必须接受",
);

const startRequest = {
  bindingId: binding.bindingId,
  appDeviceId: binding.appDeviceId,
  environment: "dev",
  remoteTaskId: baseSnapshot.remoteTaskId,
  clientRequestId: "client_req_00001",
  commandType: "start_task",
  payload: { text: "启动测试任务" },
  expectedStateVersion: 1,
};
const firstCommand = harness.submitCommand(startRequest);
const duplicateCommand = harness.submitCommand(startRequest);
assert.equal(duplicateCommand.commandId, firstCommand.commandId, "同幂等键和摘要必须返回同一 commandId");
assert.throws(
  () => harness.submitCommand({ ...startRequest, payload: { text: "不同正文" } }),
  (error) => error.code === "payload_digest_conflict",
  "同幂等键不同摘要必须拒绝",
);
assert.throws(
  () => harness.submitCommand({ ...startRequest, clientRequestId: "client_req_00002", payload: {} }),
  (error) => error.code === "missing_initial_input",
  "start_task 缺少首条文本必须拒绝",
);

const stoppedSnapshot = { ...baseSnapshot, stateVersion: 2, taskStatus: "stopped" };
assert.equal(
  harness.upsertSnapshot({ ...binding, snapshot: stoppedSnapshot }).accepted,
  true,
  "进入 stopped 的连续快照必须接受",
);
harness.setPcConnection({ bindingId: binding.bindingId, pcDeviceId: binding.pcDeviceId, state: "offline" });
const offlineInput = harness.submitCommand({
  ...startRequest,
  clientRequestId: "client_req_00003",
  commandType: "send_input",
  payload: { text: "离线后续指令" },
  expectedStateVersion: 2,
});
assert.equal(offlineInput.status, "queued", "离线 send_input 必须进入短期队列");
assert.equal(offlineInput.offlineQueued, true, "离线 send_input 必须标记为短期排队");
assert.throws(
  () => harness.submitCommand({ ...startRequest, clientRequestId: "client_req_00004", commandType: "pause_task", payload: {}, expectedStateVersion: 2 }),
  (error) => error.code === "pc_offline",
  "离线控制命令必须拒绝",
);

harness.setPcConnection({ bindingId: binding.bindingId, pcDeviceId: binding.pcDeviceId, state: "online" });
assert.equal(harness.dispatchPending().length, 2, "上线后两个排队命令都应进入 dispatched");
const accepted = harness.acceptOnPc(firstCommand.commandId);
assert.equal(accepted.status, "accepted", "PC 必须先持久化 inbox 并受理命令");
assert.equal(harness.acceptOnPc(firstCommand.commandId).commandId, firstCommand.commandId, "重复投递必须返回同一 inbox 结果");
assert.equal(harness.applyOnPc(firstCommand.commandId, { uncertain: true }).status, "reconciling", "调用证据不确定必须进入 reconciling");
assert.equal(harness.recoverUncertain(firstCommand.commandId, "completed").status, "completed", "恢复证据确定后才能结束命令");

assert.equal(
  harness.upsertSnapshot({ ...binding, snapshot: { ...baseSnapshot, stateVersion: 0 } }).reason,
  "stale_version",
  "旧版本快照不得覆盖新事实",
);
assert.equal(
  harness.upsertSnapshot({ ...binding, snapshot: { ...baseSnapshot, stateVersion: 4 } }).reason,
  "version_gap",
  "版本跳跃必须请求补偿并进入 reconciling",
);
assert.equal(
  harness.upsertSnapshot({ ...binding, snapshot: { ...baseSnapshot, stateVersion: 3 } }).accepted,
  true,
  "补齐缺口后的连续版本必须接受",
);
assert.equal(
  harness.tombstone({ ...binding, remoteTaskId: baseSnapshot.remoteTaskId, reason: "deleted" }).accepted,
  true,
  "任务删除必须通过 tombstone 清除读模型",
);

console.log(
  JSON.stringify(
    {
      ok: true,
      checks: [
        "idempotent_command",
        "payload_digest_conflict",
        "offline_input_queue",
        "offline_control_rejection",
        "inbox_replay",
        "reconciling_recovery",
        "version_gap",
        "tombstone",
      ],
    },
    null,
    2,
  ),
);
