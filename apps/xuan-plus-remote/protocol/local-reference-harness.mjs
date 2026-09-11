import { createHash } from "node:crypto";

const COMMAND_TYPES = new Set(["create_task", "start_task", "send_input", "pause_task", "resume_task", "stop_task"]);
const CONTROL_COMMANDS = new Set(["create_task", "start_task", "pause_task", "resume_task", "stop_task"]);
const OFFLINE_INPUT_TTL_MS = 10 * 60 * 1000;

const clone = (value) => JSON.parse(JSON.stringify(value));
const stableStringify = (value) => {
  if (Array.isArray(value)) return `[${value.map(stableStringify).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
};
const sha256 = (value) => createHash("sha256").update(stableStringify(value), "utf8").digest("hex");
const makeError = (code, message) => {
  const error = new Error(message);
  error.code = code;
  return error;
};

const taskKey = (binding, remoteTaskId) =>
  `${binding.pcDeviceId}:${binding.installationId}:${binding.bindingEpoch}:${remoteTaskId}`;
const idempotencyKey = ({ bindingId, appDeviceId, remoteTaskId, clientRequestId }) =>
  `${bindingId}:${appDeviceId}:${remoteTaskId}:${clientRequestId}`;
const commandId = (counter) => `command_${String(counter).padStart(12, "0")}`;
const eventId = (counter) => `event_${String(counter).padStart(12, "0")}`;

/**
 * Test-only reference model for R1/R2 contract behavior.
 * It has no network, database, Codex Host, model, or tool execution path.
 */
export class RemoteReferenceHarness {
  constructor({ now = () => new Date() } = {}) {
    this.now = now;
    this.bindings = new Map();
    this.tasks = new Map();
    this.commands = new Map();
    this.idempotency = new Map();
    this.pcInbox = new Map();
    this.events = [];
    this.commandCounter = 0;
    this.eventCounter = 0;
  }

  registerBinding(binding) {
    if (binding.environment !== "dev" && binding.environment !== "prod") {
      throw makeError("environment_mismatch", "未知环境");
    }
    if (!binding.localConfirmed) throw makeError("not_local_confirmed", "PC 尚未本地确认");
    if (!binding.bindingId) throw makeError("invalid_request", "缺少绑定标识");
    this.bindings.set(binding.bindingId, {
      ...clone(binding),
      pcConnectionState: binding.pcConnectionState ?? "offline",
    });
  }

  setPcConnection({ bindingId, pcDeviceId, state }) {
    if (!["online", "stale", "offline"].includes(state)) {
      throw makeError("invalid_request", "连接状态无效");
    }
    const binding = this.#findBinding({ bindingId, pcDeviceId });
    binding.pcConnectionState = state;
    for (const record of this.tasks.values()) {
      if (record.binding.pcDeviceId === pcDeviceId && record.binding.bindingId === bindingId) {
        record.snapshot.pcConnectionState = state;
      }
    }
  }

  upsertSnapshot({ bindingId, pcDeviceId, installationId, bindingEpoch, snapshot, eventId: suppliedEventId }) {
    const binding = this.#findBinding({ bindingId, pcDeviceId, installationId, bindingEpoch });
    if (snapshot.pcDeviceId !== pcDeviceId || snapshot.installationId !== installationId || snapshot.bindingEpoch !== bindingEpoch) {
      throw makeError("stale_installation", "快照代际与绑定不一致");
    }
    const key = taskKey(binding, snapshot.remoteTaskId);
    const digest = sha256(snapshot);
    const previous = this.tasks.get(key);
    if (!previous && snapshot.stateVersion !== 1) {
      return { accepted: false, reason: "version_gap", reconciling: true };
    }
    if (previous) {
      if (snapshot.stateVersion === previous.snapshot.stateVersion && digest === previous.digest) {
        return { accepted: true, duplicate: true, stateVersion: snapshot.stateVersion };
      }
      if (snapshot.stateVersion <= previous.snapshot.stateVersion) {
        return { accepted: false, reason: "stale_version", stateVersion: previous.snapshot.stateVersion };
      }
      if (snapshot.stateVersion > previous.snapshot.stateVersion + 1) {
        return { accepted: false, reason: "version_gap", reconciling: true };
      }
    }
    this.tasks.set(key, { binding: clone(binding), snapshot: clone(snapshot), digest });
    const storedEventId = suppliedEventId ?? eventId(++this.eventCounter);
    this.events.push({ eventId: storedEventId, stateVersion: snapshot.stateVersion, type: "snapshot/upsert" });
    return { accepted: true, duplicate: false, stateVersion: snapshot.stateVersion, eventId: storedEventId };
  }

  tombstone({ bindingId, pcDeviceId, installationId, bindingEpoch, remoteTaskId, reason }) {
    const binding = this.#findBinding({ bindingId, pcDeviceId, installationId, bindingEpoch });
    const key = taskKey(binding, remoteTaskId);
    const previous = this.tasks.get(key);
    if (!previous) return { accepted: true, duplicate: true };
    this.tasks.delete(key);
    const storedEventId = eventId(++this.eventCounter);
    this.events.push({ eventId: storedEventId, stateVersion: previous.snapshot.stateVersion + 1, type: "snapshot/tombstone", reason });
    return { accepted: true, duplicate: false, eventId: storedEventId };
  }

  submitCommand({ bindingId, appDeviceId, environment, remoteTaskId, clientRequestId, commandType, payload, expectedStateVersion }) {
    if (!COMMAND_TYPES.has(commandType)) throw makeError("invalid_request", "未知命令类型");
    const requestKey = idempotencyKey({ bindingId, appDeviceId, remoteTaskId, clientRequestId });
    const payloadDigest = sha256({ commandType, payload, expectedStateVersion });
    const prior = this.idempotency.get(requestKey);
    if (prior) {
      if (prior.payloadDigest !== payloadDigest) throw makeError("payload_digest_conflict", "幂等键对应的摘要冲突");
      return clone(this.commands.get(prior.commandId));
    }

    const task = this.#findTask({ bindingId, appDeviceId, environment, remoteTaskId });
    const binding = this.#findBinding(task.binding);
    task.binding = clone(binding);
    if (commandType === "create_task" &&
      (!payload || typeof payload.text !== "string" || payload.text.trim().length === 0 || payload.text.trim().length > 60)) {
      throw makeError("invalid_request", "新任务名称无效");
    }
    if (commandType === "start_task" && (!payload || typeof payload.text !== "string" || payload.text.trim().length === 0)) {
      throw makeError("missing_initial_input", "启动任务必须携带首条文本");
    }
    if (commandType === "send_input" && (!payload || typeof payload.text !== "string" || payload.text.trim().length === 0)) {
      throw makeError("invalid_request", "输入文本不能为空");
    }
    if (CONTROL_COMMANDS.has(commandType) && binding.pcConnectionState === "offline") {
      throw makeError("pc_offline", "PC 当前离线");
    }
    if (expectedStateVersion !== task.snapshot.stateVersion) {
      throw makeError("state_conflict", "任务状态版本已变化");
    }
    this.#assertPrecondition(commandType, task.snapshot);
    const expiresAt = new Date(this.now().getTime() + OFFLINE_INPUT_TTL_MS).toISOString();
    const record = {
      commandId: commandId(++this.commandCounter),
      clientRequestId,
      remoteTaskId,
      commandType,
      payloadDigest,
      expectedStateVersion,
      status: "queued",
      createdAt: this.now().toISOString(),
      expiresAt,
      offlineQueued: commandType === "send_input" && binding.pcConnectionState === "offline",
      payload: clone(payload),
      binding: clone(binding),
    };
    this.commands.set(record.commandId, record);
    this.idempotency.set(requestKey, { commandId: record.commandId, payloadDigest });
    return clone(record);
  }

  dispatchPending() {
    const dispatched = [];
    for (const record of this.commands.values()) {
      if (record.status !== "queued") continue;
      const binding = this.#findBinding(record.binding);
      if (binding.pcConnectionState === "offline") continue;
      record.status = "dispatched";
      dispatched.push(record.commandId);
    }
    return dispatched;
  }

  acceptOnPc(commandIdValue) {
    const record = this.#getCommand(commandIdValue);
    const prior = this.pcInbox.get(commandIdValue);
    if (prior) return clone(prior);
    if (!["queued", "dispatched"].includes(record.status)) return clone(record);
    const accepted = { commandId: record.commandId, payloadDigest: record.payloadDigest, status: "accepted" };
    this.pcInbox.set(commandIdValue, accepted);
    record.status = "accepted";
    return clone(record);
  }

  applyOnPc(commandIdValue, { uncertain = false, outcome = "completed" } = {}) {
    const record = this.#getCommand(commandIdValue);
    if (["completed", "failed"].includes(record.status)) return clone(record);
    if (!this.pcInbox.has(commandIdValue)) throw makeError("invalid_command_state", "命令尚未写入 PC inbox");
    if (uncertain) {
      record.status = "reconciling";
      return clone(record);
    }
    record.status = outcome === "completed" ? "completed" : "failed";
    record.appliedStateVersion = record.expectedStateVersion;
    return clone(record);
  }

  recoverUncertain(commandIdValue, outcome) {
    const record = this.#getCommand(commandIdValue);
    if (record.status !== "reconciling") throw makeError("invalid_command_state", "命令不在 reconciling 状态");
    if (outcome !== "completed" && outcome !== "failed") return clone(record);
    record.status = outcome;
    record.appliedStateVersion = record.expectedStateVersion;
    return clone(record);
  }

  getCommand(commandIdValue) {
    return clone(this.#getCommand(commandIdValue));
  }

  #findBinding({ bindingId, appDeviceId, pcDeviceId, installationId, bindingEpoch }) {
    const binding = this.bindings.get(bindingId);
    if (!binding) throw makeError("device_not_bound", "设备未绑定");
    if (appDeviceId !== undefined && binding.appDeviceId !== appDeviceId) throw makeError("device_not_bound", "APP 设备不属于当前绑定");
    if (pcDeviceId !== undefined && binding.pcDeviceId !== pcDeviceId) throw makeError("device_not_bound", "PC 设备不属于当前绑定");
    if (installationId !== undefined && binding.installationId !== installationId) throw makeError("stale_installation", "安装代际已失效");
    if (bindingEpoch !== undefined && binding.bindingEpoch !== bindingEpoch) throw makeError("stale_binding_epoch", "绑定代际已失效");
    return binding;
  }

  #findTask({ bindingId, appDeviceId, environment, remoteTaskId }) {
    for (const record of this.tasks.values()) {
      if (record.binding.bindingId !== bindingId || record.binding.appDeviceId !== appDeviceId) continue;
      if (record.binding.environment !== environment) throw makeError("environment_mismatch", "环境不一致");
      if (record.snapshot.remoteTaskId === remoteTaskId) return record;
    }
    throw makeError("not_found", "任务不存在");
  }

  #getCommand(commandIdValue) {
    const record = this.commands.get(commandIdValue);
    if (!record) throw makeError("not_found", "命令不存在");
    return record;
  }

  #assertPrecondition(commandType, snapshot) {
    const allowed = {
      create_task: ["created", "queued", "starting", "running", "pausing", "paused", "stopping", "stopped", "failed", "reconciling", "archived"],
      start_task: ["created", "stopped", "failed"],
      send_input: ["running", "stopped", "failed"],
      pause_task: ["queued", "starting", "running"],
      resume_task: ["paused"],
      stop_task: ["queued", "starting", "running", "pausing", "paused"],
    };
    if (!allowed[commandType].includes(snapshot.taskStatus)) {
      throw makeError("invalid_command_state", `命令 ${commandType} 不允许当前任务状态`);
    }
  }
}
