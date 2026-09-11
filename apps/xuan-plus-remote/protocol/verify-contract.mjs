import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const schemaPath = join(scriptDirectory, "remote-control.schema.json");
const fixturesPath = join(scriptDirectory, "contract-fixtures.json");
const schema = JSON.parse(readFileSync(schemaPath, "utf8"));
const fixtures = JSON.parse(readFileSync(fixturesPath, "utf8"));

const failures = [];
const assert = (condition, message) => {
  if (!condition) failures.push(message);
};
const byteLength = (value) => Buffer.byteLength(value, "utf8");
const isOpaqueId = (value) => typeof value === "string" && /^[A-Za-z0-9_-]{16,128}$/.test(value);
const isDigest = (value) => typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
const isTimestamp = (value) =>
  typeof value === "string" &&
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(value) &&
  Number.isFinite(Date.parse(value));
const hasOnlyKeys = (value, allowedKeys, label) => {
  const unexpected = Object.keys(value).filter((key) => !allowedKeys.includes(key));
  assert(unexpected.length === 0, `${label} 包含未声明字段：${unexpected.join(", ")}`);
};
const assertHttpRequest = (value, messageType, label) => {
  assert(value.schemaVersion === "1.5", `${label} 必须使用 1.5`);
  assert(value.messageType === messageType, `${label} messageType 不正确`);
  assert(value.environment === "dev" || value.environment === "prod", `${label} environment 无效`);
  assert(isOpaqueId(value.messageId), `${label} messageId 必须是不透明 ID`);
  assert(isTimestamp(value.sentAt), `${label} sentAt 必须是 UTC RFC 3339 时间`);
};
const assertHttpResponse = (value, messageType, requestMessageId, label) => {
  assert(value.schemaVersion === "1.5", `${label} 必须使用 1.5`);
  assert(value.messageType === messageType, `${label} messageType 不正确`);
  assert(value.requestMessageId === requestMessageId, `${label} 没有关联正确的请求`);
  assert(value.environment === "dev" || value.environment === "prod", `${label} environment 无效`);
  assert(isOpaqueId(value.messageId), `${label} messageId 必须是不透明 ID`);
  assert(isTimestamp(value.serverReceivedAt), `${label} serverReceivedAt 必须是 UTC RFC 3339 时间`);
};

assert(schema.$schema === "https://json-schema.org/draft/2020-12/schema", "必须使用 JSON Schema 2020-12");
assert(schema.$id === "https://workagents.invalid/contracts/remote-control/1.5/schema.json", "Schema ID 必须升级到 1.5");
assert(schema["x-contractVersion"] === "1.5", "契约版本必须为 1.5");
assert(
  JSON.stringify(schema["x-compatibleVersions"]) === JSON.stringify(["1.0", "1.1", "1.2", "1.3", "1.4", "1.5"]),
  "必须显式声明 1.0 到 1.5 的兼容集合",
);

const definitions = schema.$defs ?? {};
const requiredDefinitions = [
  "pcHello",
  "pcHeartbeat",
  "snapshotUpsert",
  "snapshotTombstone",
  "syncAck",
  "syncRequest",
  "commandDispatch",
  "commandResult",
  "appCommandRequest",
  "errorResponse",
  "taskSnapshot",
  "remoteTaskSnapshot",
  "commandRecord",
  "appDeviceRegistrationChallengeRequest",
  "appDeviceRegistrationChallengeResponse",
  "appDeviceRegistrationRequest",
  "appDeviceRegistrationResponse",
  "pairingRegistrationRequest",
  "pairingRegistrationResponse",
  "pairingConsumeRequest",
  "pairingPendingResponse",
  "bindingConfirmationRequest",
  "bindingLocalConfirmation",
  "bindingActiveResponse",
  "bindingRevocationRequest",
  "bindingRevokedResponse",
  "pcDeviceListQuery",
  "pcDeviceListResponse",
  "taskListQuery",
  "taskListResponse",
  "pushRefreshQuery",
  "pushRefreshResponse",
  "taskDetailQuery",
  "taskDetailResponse",
  "commandQuery",
  "commandQueryResponse",
  "commandAcceptedResponse",
  "liveReplyStreamQuery",
  "liveReplySubscription",
  "liveReplyReset",
  "liveReplyAppend",
  "liveReplyEnd",
];
for (const definitionName of requiredDefinitions) {
  assert(definitions[definitionName], `缺少契约定义：${definitionName}`);
}
const terminalPushEligibility = definitions.snapshotUpsert.allOf?.[1]?.properties?.terminalPushEligible;
assert(terminalPushEligibility?.type === "boolean", "终态 Push 准入位必须是布尔值");
assert(terminalPushEligibility?.default === false, "旧快照缺少终态 Push 准入位时必须安全默认为 false");
assert(
  !definitions.snapshotUpsert.allOf?.[1]?.required?.includes("terminalPushEligible"),
  "终态 Push 准入位必须保持可选，以便旧持久 outbox 安全重放为不推送",
);

const expectedMessageTypes = [
  "pc/hello",
  "pc/heartbeat",
  "snapshot/upsert",
  "snapshot/tombstone",
  "sync/ack",
  "sync/request",
  "command/dispatch",
  "command/result",
  "app/command",
  "error",
  "app/device-registration-challenge",
  "app/device-registration-challenged",
  "app/device-register",
  "app/device-registered",
  "pairing/register",
  "pairing/registered",
  "pairing/consume",
  "pairing/pending",
  "binding/confirmation-request",
  "binding/local-confirm",
  "binding/active",
  "binding/revoke",
  "binding/revoked",
  "app/pc-devices-query",
  "app/pc-devices",
  "app/task-list-query",
  "app/task-list",
  "app/push-refresh-query",
  "app/push-refresh",
  "app/task-query",
  "app/task",
  "app/command-query",
  "app/command-status",
  "app/command-accepted",
  "app/reply-stream-connect",
  "reply-stream/subscription",
  "reply-stream/reset",
  "reply-stream/append",
  "reply-stream/end",
];
assert(schema.oneOf?.length === expectedMessageTypes.length, "根消息集合数量发生漂移");
for (const messageType of expectedMessageTypes) {
  assert(definitions.messageType?.enum?.includes(messageType), `缺少消息类型：${messageType}`);
}
assert(
  JSON.stringify(definitions.schemaVersion?.enum) === JSON.stringify(["1.0", "1.1", "1.2", "1.3", "1.4", "1.5"]),
  "既有消息必须保留 1.0/1.1/1.2/1.3/1.4 并接受 1.5",
);
assert(definitions.httpSchemaVersion?.const === "1.5", "无账号设备认证 REST envelope 必须固定使用 1.5");

const expectedCommandTypes = ["create_task", "start_task", "send_input", "pause_task", "resume_task", "stop_task"];
assert(
  JSON.stringify(definitions.commandType?.enum) === JSON.stringify(expectedCommandTypes),
  "六种远程命令集合发生漂移",
);

const expectedCommandStatuses = [
  "queued",
  "dispatched",
  "accepted",
  "applying",
  "completed",
  "rejected",
  "failed",
  "expired",
  "cancelled",
  "reconciling",
];
assert(
  definitions.commandPayloadCreate?.properties?.text?.maxLength === 60,
  "create_task 名称上限必须为 60 个字符",
);
assert(
  definitions.commandPayloadCreate?.required?.includes("initialText") &&
    definitions.commandPayloadCreate?.properties?.initialText?.["x-maxUtf8Bytes"] === 8192,
  "1.5 create_task 必须携带受限的第一条指令",
);
assert(
  JSON.stringify(definitions.commandStatus?.enum) === JSON.stringify(expectedCommandStatuses),
  "命令状态机发生漂移",
);

const expectedErrorCodes = [
  "invalid_request",
  "unsupported_schema_version",
  "unknown_message_type",
  "unauthorized",
  "device_not_bound",
  "environment_mismatch",
  "stale_binding_epoch",
  "stale_installation",
  "not_found",
  "pc_offline",
  "state_conflict",
  "invalid_command_state",
  "missing_initial_input",
  "payload_digest_conflict",
  "command_expired",
  "command_not_cancellable",
  "reconciling",
  "unsupported_operation",
  "rate_limited",
  "not_local_confirmed",
  "pairing_expired",
  "pairing_replayed",
  "invalid_pairing_qr",
  "pairing_code_invalid",
  "pairing_summary_mismatch",
  "device_key_invalid",
  "device_proof_invalid",
  "device_challenge_expired",
  "device_signature_replayed",
  "storage_unavailable",
  "internal",
];
assert(
  JSON.stringify(definitions.errorCode?.enum) === JSON.stringify(expectedErrorCodes),
  "统一错误码集合发生漂移",
);

const replyProperties = definitions.reply?.properties ?? {};
assert(!Object.hasOwn(replyProperties.text ?? {}, "maxLength") &&
  !Object.hasOwn(replyProperties.text ?? {}, "x-maxUtf8Bytes"), "回复正文不得设置截断上限");
assert(
  definitions.commandPayloadStart?.properties?.text?.["x-maxUtf8Bytes"] === 8192,
  "start_task 首条文本 UTF-8 字节上限必须为 8 KiB",
);
assert(
  definitions.commandPayloadInput?.properties?.text?.["x-maxUtf8Bytes"] === 8192,
  "send_input 文本 UTF-8 字节上限必须为 8 KiB",
);
assert(definitions.taskSnapshot?.properties?.pcDeviceId, "快照必须保留 PC 路由代际字段");
assert(definitions.taskSnapshot?.properties?.modelLabel, "快照必须提供当前设备模型展示名");
assert(definitions.taskSnapshot?.properties?.bindingEpoch, "快照必须保留 bindingEpoch");
assert(definitions.taskSnapshot?.properties?.stateVersion, "快照必须保留 stateVersion");
assert(
  definitions.remoteTaskSnapshot?.allOf?.[1]?.required?.includes("serverReceivedAt"),
  "APP 可读快照必须要求 serverReceivedAt",
);
assert(
  definitions.taskSnapshot?.properties?.lastReply?.oneOf?.some((entry) => entry.type === "null"),
  "无可导出回复时 lastReply 必须允许为 null",
);
assert(schema["x-transport"]?.push === "refresh-only", "Push 只能作为刷新提示");
assert(schema["x-transport"]?.app?.includes("wss"), "APP 任务详情实时回复必须使用受管 WSS");
assert(schema["x-appDeviceAuthentication"]?.mode === "device-proof-of-possession", "APP 必须使用设备持钥认证");
assert(schema["x-appDeviceAuthentication"]?.algorithm === "ed25519", "设备持钥算法必须固定为 Ed25519");
assert(schema["x-appDeviceAuthentication"]?.accountLogin === "forbidden", "无账号模式不得恢复账号登录");
assert(
  schema["x-appDeviceAuthentication"]?.requestProofHeaders?.length === 4,
  "设备签名请求头必须固定且完整",
);
assert(
  schema["x-appDeviceAuthentication"]?.registrationProofCanonicalFormat?.includes("{challenge}"),
  "设备注册持钥证明必须覆盖服务端挑战值",
);
assert(definitions.devicePublicKey?.["x-format"] === "spki-der-base64url", "设备公钥格式必须固定");
assert(definitions.deviceSignature?.pattern === "^[A-Za-z0-9_-]{86}$", "Ed25519 签名长度必须固定");
assert(definitions.pairingQrPayload?.additionalProperties === false, "二维码 payload 必须拒绝未知字段");
assert(!definitions.shortPairingCode, "1.5 契约不得继续定义手动短码");
assert(
  definitions.pairingQrPayload?.["x-forbiddenProperties"]?.includes("url") &&
    definitions.pairingQrPayload?.["x-forbiddenProperties"]?.includes("endpoint"),
  "二维码必须明确禁止 URL 和 endpoint",
);
assert(definitions.pairingConsumeRequest?.allOf?.[2]?.oneOf?.length === 2, "扫码配对必须强制二维码环境与请求环境一致");
for (const definitionName of ["appCommandRequest", "pairingConsumeRequest", "bindingLocalConfirmation", "taskDetailQuery"]) {
  assert(definitions[definitionName]?.unevaluatedProperties === false, `${definitionName} 必须拒绝未知字段`);
}

const assertCommandEnvelope = (value, label) => {
  assert(["1.0", "1.1", "1.2", "1.3", "1.4", "1.5"].includes(value.schemaVersion), `${label} 版本不兼容`);
  assert(value.messageType === "app/command", `${label} messageType 不正确`);
  assert(value.environment === "dev", `${label} fixture environment 应为 dev`);
  assert(isOpaqueId(value.messageId), `${label} messageId 必须是不透明 ID`);
  assert(isOpaqueId(value.remoteTaskId), `${label} remoteTaskId 必须是不透明 ID`);
  assert(isOpaqueId(value.clientRequestId), `${label} clientRequestId 必须是不透明 ID`);
  assert(Number.isSafeInteger(value.expectedStateVersion) && value.expectedStateVersion >= 1, `${label} 状态版本无效`);
  assert(isTimestamp(value.expiresAt), `${label} expiresAt 必须是 UTC RFC 3339 时间`);
  assert(expectedCommandTypes.includes(value.commandType), `${label} 命令类型无效`);
};
for (const [fixtureName, commandType] of [
  ["validStartCommand", "start_task"],
  ["validSendInputCommand", "send_input"],
  ["validPauseCommandV12", "pause_task"],
]) {
  const fixture = fixtures[fixtureName];
  assertCommandEnvelope(fixture, fixtureName);
  assert(fixture.commandType === commandType, `${fixtureName} 命令类型漂移`);
}
assert(
  fixtures.validStartCommand.payload.text.length > 0 && byteLength(fixtures.validStartCommand.payload.text) <= 8192,
  "合法 start_task 夹具必须包含不超过 8 KiB 的首条文本",
);
assert(
  fixtures.validSendInputCommand.payload.text.length > 0 && byteLength(fixtures.validSendInputCommand.payload.text) <= 8192,
  "合法 send_input 夹具必须包含不超过 8 KiB 的文本",
);
assert(Object.keys(fixtures.validPauseCommandV12.payload).length === 0, "pause_task 必须使用空 payload");
assert(
  fixtures.invalidStartWithoutText.commandType === "start_task" && !fixtures.invalidStartWithoutText.payload.text,
  "缺少首条文本的非法夹具未保持非法形态",
);
assert(Object.hasOwn(fixtures.invalidModelOverride, "model"), "模型覆盖非法夹具缺少应拒绝的模型字段");
assert(
  !Object.hasOwn(fixtures.validStartCommand, "model") &&
    !Object.hasOwn(fixtures.validStartCommand, "cwd") &&
    !Object.hasOwn(fixtures.validStartCommand, "threadId"),
  "合法远程命令夹具暴露了内部或模型字段",
);

const registrationChallenge = fixtures.validAppDeviceRegistrationChallenge;
assertHttpRequest(registrationChallenge, "app/device-registration-challenge", "APP 设备挑战请求");
hasOnlyKeys(
  registrationChallenge,
  ["schemaVersion", "messageType", "messageId", "environment", "sentAt", "appDeviceId", "deviceKeyAlgorithm", "devicePublicKey"],
  "APP 设备挑战请求",
);
assert(registrationChallenge.deviceKeyAlgorithm === "ed25519", "APP 设备挑战必须固定使用 Ed25519");
assert(/^[A-Za-z0-9_-]{40,512}$/.test(registrationChallenge.devicePublicKey), "APP 设备公钥必须使用 SPKI DER base64url");
const registrationChallenged = fixtures.validAppDeviceRegistrationChallenged;
assertHttpResponse(
  registrationChallenged,
  "app/device-registration-challenged",
  registrationChallenge.messageId,
  "APP 设备挑战响应",
);
assert(registrationChallenged.appDeviceId === registrationChallenge.appDeviceId, "APP 设备挑战响应设备不一致");
assert(isOpaqueId(registrationChallenged.challengeId) && isOpaqueId(registrationChallenged.challenge), "APP 设备挑战必须使用不透明随机值");
assert(isTimestamp(registrationChallenged.expiresAt), "APP 设备挑战必须带过期时间");

const registration = fixtures.validAppDeviceRegistration;
assertHttpRequest(registration, "app/device-register", "APP 注册请求");
hasOnlyKeys(
  registration,
  [
    "schemaVersion",
    "messageType",
    "messageId",
    "environment",
    "sentAt",
    "appDeviceId",
    "deviceKeyAlgorithm",
    "devicePublicKey",
    "challengeId",
    "challenge",
    "registrationSignature",
    "pushProvider",
    "pushToken",
    "appDisplayName",
    "appVersion",
  ],
  "APP 注册请求",
);
assert(isOpaqueId(registration.appDeviceId), "APP 注册必须携带不透明 appDeviceId");
assert(registration.challengeId === registrationChallenged.challengeId, "APP 注册必须回应同一挑战");
assert(registration.challenge === registrationChallenged.challenge, "APP 注册必须回传同一挑战值供摘要核验");
assert(registration.devicePublicKey === registrationChallenge.devicePublicKey, "APP 注册不得替换挑战阶段公钥");
assert(/^[A-Za-z0-9_-]{86}$/.test(registration.registrationSignature), "APP 注册必须携带固定长度 Ed25519 持钥证明");
assert(registration.pushProvider === "huawei_push_kit" && registration.pushToken.length > 0, "APP 注册必须携带 Push Kit Token");
assertHttpResponse(fixtures.validAppDeviceRegistered, "app/device-registered", registration.messageId, "APP 注册响应");
assert(fixtures.validAppDeviceRegistered.appDeviceId === registration.appDeviceId, "APP 注册响应设备不一致");
assert(isOpaqueId(fixtures.validAppDeviceRegistered.deviceKeyId), "APP 注册响应必须返回不透明 deviceKeyId");

const pairingRegistration = fixtures.validPairingRegistration;
assertHttpRequest(pairingRegistration, "pairing/register", "PC 配对登记请求");
hasOnlyKeys(
  pairingRegistration,
  ["schemaVersion", "messageType", "messageId", "environment", "sentAt", "pcDeviceId", "installationId", "pairing", "pcDisplayName", "expiresAt"],
  "PC 配对登记请求",
);
assert(pairingRegistration.pairing.environment === pairingRegistration.environment, "PC 配对登记不得跨环境");
assert(isOpaqueId(pairingRegistration.pairing.pairingHandle), "PC 配对登记必须携带不透明句柄");
assertHttpResponse(
  fixtures.validPairingRegistered,
  "pairing/registered",
  pairingRegistration.messageId,
  "PC 配对登记响应",
);
assert(
  fixtures.validPairingRegistered.pairingHandle === pairingRegistration.pairing.pairingHandle,
  "云端不得替换 PC 已登记的配对句柄",
);

const pairing = fixtures.validPairingConsume;
assertHttpRequest(pairing, "pairing/consume", "配对消费请求");
hasOnlyKeys(
  pairing,
  ["schemaVersion", "messageType", "messageId", "environment", "sentAt", "appDeviceId", "pairing"],
  "配对消费请求",
);
hasOnlyKeys(pairing.pairing, ["pairingQrVersion", "environment", "pairingHandle"], "二维码 payload");
assert(pairing.pairing.pairingQrVersion === "2", "二维码版本必须为 2");
assert(pairing.pairing.environment === pairing.environment, "二维码环境必须与请求环境一致");
assert(isOpaqueId(pairing.pairing.pairingHandle), "二维码句柄必须是不透明 ID");
assert(!Object.hasOwn(pairing, "shortCode"), "默认扫码消费不得携带旧版短码");

const pending = fixtures.validPairingPending;
assertHttpResponse(pending, "pairing/pending", pairing.messageId, "待 PC 确认响应");
assert(pending.bindingState === "pending_local_confirmation", "扫码后必须先处于待 PC 本地确认状态");
assert(isOpaqueId(pending.bindingId) && isOpaqueId(pending.confirmationNonce), "待确认响应缺少不透明绑定关联");
assert(isOpaqueId(pending.pcDeviceId) && isOpaqueId(pending.installationId), "待确认响应必须返回精确 PC 身份供自动轮询");
assert(isTimestamp(pending.confirmationExpiresAt), "待确认响应缺少过期时间");
assert(pending.bindingSummary.environment === pairing.environment, "绑定摘要环境不一致");
assert(isDigest(pending.bindingSummary.summaryDigest), "绑定摘要必须带摘要校验值");
assert(/^[a-z]{3,16}(?:-[a-z]{3,16}){2}$/.test(pending.bindingSummary.safetyPhrase), "安全短语格式无效");

const confirmationRequest = fixtures.validBindingConfirmationRequest;
assertHttpResponse(confirmationRequest, "binding/confirmation-request", pairingRegistration.messageId, "PC 绑定确认请求");
assert(confirmationRequest.pcPairingMessageId === pairingRegistration.messageId, "PC 自动确认必须关联当前二维码登记消息");
assert(confirmationRequest.bindingId === pending.bindingId, "PC 确认请求绑定不一致");
assert(confirmationRequest.confirmationNonce === pending.confirmationNonce, "PC 确认请求 nonce 不一致");
assert(
  JSON.stringify(confirmationRequest.bindingSummary) === JSON.stringify(pending.bindingSummary),
  "APP 与 PC 必须收到相同的绑定摘要",
);

const localConfirmation = fixtures.validLocalBindingConfirmation;
assertHttpRequest(localConfirmation, "binding/local-confirm", "PC 本地确认请求");
assert(localConfirmation.bindingId === pending.bindingId, "PC 本地确认绑定不一致");
assert(localConfirmation.confirmationNonce === confirmationRequest.confirmationNonce, "PC 本地确认 nonce 不一致");
assert(localConfirmation.summaryDigest === confirmationRequest.bindingSummary.summaryDigest, "PC 本地确认摘要不一致");
assert(localConfirmation.confirmed === true, "绑定必须由 PC Gateway 持钥自动确认");
assertHttpResponse(fixtures.validBindingActive, "binding/active", localConfirmation.messageId, "绑定激活响应");
assert(fixtures.validBindingActive.bindingState === "active", "本地确认后绑定必须显式激活");
assert(fixtures.validBindingActive.bindingId === pending.bindingId, "激活响应绑定不一致");

assertHttpRequest(fixtures.validBindingRevocation, "binding/revoke", "绑定撤销请求");
assert(fixtures.validBindingRevocation.bindingId === pending.bindingId, "撤销请求绑定不一致");
assertHttpResponse(fixtures.validBindingRevoked, "binding/revoked", fixtures.validBindingRevocation.messageId, "绑定撤销响应");
assert(fixtures.validBindingRevoked.bindingState === "revoked", "撤销响应必须显式为 revoked");

assertHttpRequest(fixtures.validPcDeviceListQuery, "app/pc-devices-query", "PC 列表查询");
assertHttpResponse(fixtures.validPcDeviceListEmpty, "app/pc-devices", fixtures.validPcDeviceListQuery.messageId, "PC 列表空结果");
assert(Array.isArray(fixtures.validPcDeviceListEmpty.pcDevices) && fixtures.validPcDeviceListEmpty.pcDevices.length === 0, "PC 空结果必须显式为空数组");
assert(fixtures.validPcDeviceListEmpty.nextCursor === null, "PC 空结果必须显式结束分页");

assertHttpRequest(fixtures.validTaskListQuery, "app/task-list-query", "任务列表查询");
assertHttpResponse(fixtures.validTaskListEmpty, "app/task-list", fixtures.validTaskListQuery.messageId, "任务列表空结果");
assert(Array.isArray(fixtures.validTaskListEmpty.tasks) && fixtures.validTaskListEmpty.tasks.length === 0, "任务空结果必须显式为空数组");
assertHttpRequest(fixtures.validPushRefreshQuery, "app/push-refresh-query", "推送刷新引用查询");
assertHttpResponse(fixtures.validPushRefreshResponse, "app/push-refresh", fixtures.validPushRefreshQuery.messageId, "推送刷新引用响应");
assert(isOpaqueId(fixtures.validPushRefreshResponse.remoteTaskId), "推送刷新响应必须返回不透明远程任务标识");
assert(fixtures.validPushRefreshResponse.terminalStateVersion >= 1, "推送刷新响应必须返回有效终态版本");
assertHttpRequest(fixtures.validTaskDetailQuery, "app/task-query", "任务详情查询");
assertHttpResponse(fixtures.validTaskDetailResponse, "app/task", fixtures.validTaskDetailQuery.messageId, "任务详情响应");
const snapshot = fixtures.validTaskDetailResponse.snapshot;
assert(snapshot.lastReply === null && snapshot.lastReplyState === "absent", "无可导出回复必须以 null/absent 表示");
assert(isTimestamp(snapshot.pcObservedAt) && isTimestamp(snapshot.serverReceivedAt), "任务快照必须区分 PC 观测和云端接收时间");
assert(snapshot.pcObservedAt <= snapshot.serverReceivedAt, "云端接收时间不得早于 PC 观测时间");

assertHttpRequest(fixtures.validCommandQuery, "app/command-query", "命令查询");
for (const [fixtureName, messageType, requestMessageId] of [
  ["validCommandStatus", "app/command-status", fixtures.validCommandQuery.messageId],
  ["validCommandAccepted", "app/command-accepted", fixtures.validStartCommand.messageId],
]) {
  const fixture = fixtures[fixtureName];
  assertHttpResponse(fixture, messageType, requestMessageId, fixtureName);
  assert(isOpaqueId(fixture.command.commandId), `${fixtureName} 必须返回不透明 commandId`);
  assert(isDigest(fixture.command.payloadDigest), `${fixtureName} payloadDigest 无效`);
  assert(expectedCommandStatuses.includes(fixture.command.status), `${fixtureName} 命令状态无效`);
}

assert(Object.hasOwn(fixtures.invalidPairingQrWithUrl.pairing, "url"), "二维码 URL 非法夹具缺少 URL");
assert(
  !Object.hasOwn(fixtures.validPairingConsume.pairing, "url") && !Object.hasOwn(fixtures.validPairingConsume.pairing, "endpoint"),
  "合法二维码不得包含 URL 或 endpoint",
);
assert(
  fixtures.invalidPairingEnvironmentMismatch.environment !== fixtures.invalidPairingEnvironmentMismatch.pairing.environment,
  "跨环境二维码非法夹具未保持环境不一致",
);
assert(fixtures.invalidPairingQrVersion.pairing.pairingQrVersion !== "2", "未知二维码版本非法夹具未保持错误版本");
assert(
  Object.hasOwn(fixtures.invalidPairingShortCode, "shortCode"),
  "旧版短码字段非法夹具必须保留待拒绝字段",
);
assert(
  fixtures.invalidPairingSummaryMismatch.summaryDigest !== pending.bindingSummary.summaryDigest,
  "摘要不一致非法夹具未保持错误摘要",
);
assert(Object.hasOwn(fixtures.invalidTaskQueryWithPath, "cwd"), "路径非法夹具缺少 cwd 字段");

const liveQuery = fixtures.validLiveReplyStreamQuery;
assertHttpRequest(liveQuery, "app/reply-stream-connect", "实时回复连接查询");
hasOnlyKeys(
  liveQuery,
  ["schemaVersion", "messageType", "messageId", "environment", "sentAt", "appDeviceId"],
  "实时回复连接查询",
);
const liveSubscription = fixtures.validLiveReplySubscription;
assert(liveSubscription.messageType === "reply-stream/subscription", "PC 订阅通知类型无效");
assert(liveSubscription.active === true, "详情页订阅必须显式激活");
assert(!Object.hasOwn(liveSubscription, "text"), "订阅通知不得携带回复正文");
const liveReset = fixtures.validLiveReplyReset;
const liveAppend = fixtures.validLiveReplyAppend;
const liveEnd = fixtures.validLiveReplyEnd;
assert(liveReset.streamSeq === 1 && liveReset.messageType === "reply-stream/reset", "实时 reset 序列无效");
assert(
  liveAppend.streamId === liveReset.streamId && liveAppend.streamSeq === liveReset.streamSeq + 1,
  "实时 append 必须使用同一 streamId 和连续 streamSeq",
);
assert(
  liveEnd.streamId === liveReset.streamId && liveEnd.streamSeq === liveAppend.streamSeq + 1,
  "实时 end 必须连续结束当前 stream",
);
assert(!Object.hasOwn(liveReset, "stateVersion"), "实时流不得复用持久任务 stateVersion");
assert(fixtures.invalidLiveReplyGap.streamSeq !== liveAppend.streamSeq + 1, "序列缺口夹具未保持缺口");
assert(!Object.hasOwn(definitions.liveReplyReset?.allOf?.[1]?.properties?.text ?? {}, "maxLength"),
  "实时 reset 正文不得设置截断上限");
assert(!Object.hasOwn(definitions.liveReplyAppend?.allOf?.[1]?.properties?.text ?? {}, "maxLength"),
  "实时 append 正文不得设置截断上限");

const boundaryText = "界".repeat(4096);
assert(byteLength(boundaryText) === 12288, "UTF-8 边界样本计算异常");
assert(byteLength("a".repeat(8192)) === 8192, "ASCII 文本字节样本计算异常");

const forbiddenRemoteFields = [
  "model_reasoning_effort",
  "model_context_window",
  "modelProvider",
  "cwd",
  "apiKey",
  "accessKey",
  "threadId",
  "turnId",
];
const serializedSchema = JSON.stringify(schema);
for (const fieldName of forbiddenRemoteFields) {
  assert(!serializedSchema.includes(`\"${fieldName}\"`), `远程契约不得暴露内部/模型字段：${fieldName}`);
}

if (failures.length > 0) {
  console.error(JSON.stringify({ ok: false, failures }, null, 2));
  process.exitCode = 1;
} else {
  console.log(
    JSON.stringify(
      {
        ok: true,
        contractVersion: schema["x-contractVersion"],
        compatibleVersions: schema["x-compatibleVersions"],
        messages: schema.oneOf.length,
        commandTypes: expectedCommandTypes,
        commandStatuses: expectedCommandStatuses.length,
        errorCodes: expectedErrorCodes.length,
        fixtures: Object.keys(fixtures).length,
        restContract: "app-device-challenge,device-proof,pairing,binding,queries",
        replyMaxUtf8Bytes: replyProperties.text["x-maxUtf8Bytes"],
      },
      null,
      2,
    ),
  );
}
