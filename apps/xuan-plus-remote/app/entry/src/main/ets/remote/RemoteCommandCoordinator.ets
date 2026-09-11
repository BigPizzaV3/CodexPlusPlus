import { DeviceRegistrationResult } from './DeviceRegistrationCoordinator';
import { CommandStatus, CommandType, utf8ByteLength } from './RemoteModels';
import { RemoteApiClient, RemoteFailureNotice } from './RemoteApiClient';
import { RemoteEnvironmentConfig } from './RemoteEnvironmentConfig';
import { RemoteTaskSnapshot } from './TaskSyncCoordinator';
import { hilog } from '@kit.PerformanceAnalysisKit';

const CONTRACT_VERSION = '2.0';
const OPAQUE_ID_PATTERN = /^[A-Za-z0-9_-]{16,128}$/;
const DIGEST_PATTERN = /^[a-f0-9]{64}$/;
const COMMAND_TTL_MS = 9 * 60 * 1000;
const LOG_DOMAIN = 0xD002021;
const LOG_TAG = 'RemoteCommand';

interface CommandPayload {
  text?: string;
  initialText?: string;
  modelConfigId?: string;
}

interface AppCommandRequest {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  environment: string;
  remoteTaskId: string;
  clientRequestId: string;
  expectedStateVersion: number;
  expiresAt: string;
  commandType: CommandType;
  payload: CommandPayload;
}

export interface RemoteCommandRecord {
  commandId: string;
  clientRequestId: string;
  remoteTaskId: string;
  commandType: CommandType;
  payloadDigest: string;
  expectedStateVersion: number;
  status: CommandStatus;
  createdAt: string;
  expiresAt: string;
  appliedStateVersion?: number;
  errorCode?: string;
}

interface CommandAcceptedResponse {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  requestMessageId: string;
  environment: string;
  serverReceivedAt: string;
  command: RemoteCommandRecord;
}

interface CommandQueryResponse {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  requestMessageId: string;
  environment: string;
  serverReceivedAt: string;
  command: RemoteCommandRecord;
}

export class RemoteCommandCoordinator {
  static async createTask(registration: DeviceRegistrationResult, workspaceSource: RemoteTaskSnapshot,
    modelConfigId: string, name: string, initialText: string): Promise<RemoteCommandRecord> {
    const trimmed: string = name.trim();
    if (trimmed.length < 1 || trimmed.length > 60) {
      throw new Error('任务名称不能为空且不超过 60 个字符');
    }
    const trimmedInitialText: string = initialText.trim();
    const initialTextLength: number = utf8ByteLength(trimmedInitialText);
    if (initialTextLength < 1 || initialTextLength > 8192) {
      throw new Error('第一条指令必须为 1–8192 个 UTF-8 字节');
    }
    const normalizedModelConfigId: string = modelConfigId.trim();
    if (!DIGEST_PATTERN.test(normalizedModelConfigId)) {
      throw new Error('当前模型配置标识无效，请刷新电脑配置后重试');
    }
    return RemoteCommandCoordinator.submit(
      registration, workspaceSource, CommandType.CREATE_TASK,
      { text: trimmed, initialText: trimmedInitialText, modelConfigId: normalizedModelConfigId });
  }

  static async submitText(registration: DeviceRegistrationResult, task: RemoteTaskSnapshot,
    text: string): Promise<RemoteCommandRecord> {
    const trimmed: string = text.trim();
    const length: number = utf8ByteLength(trimmed);
    if (length < 1 || length > 8192) {
      throw new Error('消息必须为 1–8192 个 UTF-8 字节');
    }
    const commandType: CommandType = task.taskStatus === 'created' ?
      CommandType.START_TASK : CommandType.SEND_INPUT;
    return RemoteCommandCoordinator.submit(registration, task, commandType, { text: trimmed });
  }

  static async stopTask(registration: DeviceRegistrationResult,
    task: RemoteTaskSnapshot): Promise<RemoteCommandRecord> {
    return RemoteCommandCoordinator.submit(registration, task, CommandType.STOP_TASK, {});
  }

  static async waitForTerminal(registration: DeviceRegistrationResult,
    initial: RemoteCommandRecord): Promise<RemoteCommandRecord> {
    let current: RemoteCommandRecord = initial;
    for (let attempt: number = 0; attempt < 20; attempt++) {
      if (RemoteCommandCoordinator.isTerminal(current.status)) {
        return current;
      }
      await new Promise<void>((resolve: () => void) => {
        setTimeout(resolve, 1000);
      });
      current = await RemoteCommandCoordinator.query(registration, current.commandId);
    }
    return current;
  }

  static resultNotice(command: RemoteCommandRecord): string {
    if (command.status === CommandStatus.COMPLETED) {
      if (command.commandType === CommandType.CREATE_TASK) {
        return '任务已创建并提交第一条指令，回复会自动同步';
      }
      return command.commandType === CommandType.STOP_TASK ?
        '停止请求已由电脑处理，任务终态以同步状态为准' :
        '消息已由电脑任务接收，回复将自动同步到手机';
    }
    if (command.status === CommandStatus.QUEUED || command.status === CommandStatus.DISPATCHED ||
      command.status === CommandStatus.ACCEPTED || command.status === CommandStatus.APPLYING) {
      return '命令已提交，正在等待电脑处理';
    }
    if (command.errorCode === 'state_conflict') {
      return '任务状态已变化，请刷新后重新提交';
    }
    if (command.errorCode === 'command_expired') {
      return '命令已过期，请重新提交';
    }
    if (command.errorCode === 'invalid_command_state') {
      return '当前任务状态不允许执行该操作，请刷新后重试';
    }
    if (command.status === CommandStatus.RECONCILING) {
      return '电脑正在校准命令结果，请等待任务状态刷新';
    }
    return '电脑未能处理本次命令，请刷新任务状态后重试';
  }

  static resultFailure(command: RemoteCommandRecord): RemoteFailureNotice | undefined {
    if (command.status !== CommandStatus.REJECTED && command.status !== CommandStatus.FAILED &&
      command.status !== CommandStatus.EXPIRED && command.status !== CommandStatus.CANCELLED) {
      return undefined;
    }
    const errorCode: string = RemoteCommandCoordinator.publicCommandErrorCode(command.errorCode);
    const status: string = `命令状态：${RemoteCommandCoordinator.commandStatusText(command.status)}` +
      `${errorCode.length > 0 ? ` · ${errorCode}` : ''}`;
    if (command.errorCode === 'pc_offline') {
      return new RemoteFailureNotice('电脑当前离线', '云端已接收命令，但目标电脑未能处理。',
        `${status} · 可重试`, '电脑恢复在线后重新提交；当前输入仍保留。', true);
    }
    if (command.errorCode === 'state_conflict') {
      return new RemoteFailureNotice('任务状态已变化', '电脑拒绝使用过期任务状态执行命令。',
        `${status} · 需刷新`, '刷新任务状态后重新提交；当前输入仍保留。', false);
    }
    if (command.errorCode === 'invalid_command_state') {
      return new RemoteFailureNotice('当前状态不允许此操作', '目标任务的当前状态不接受这类命令。',
        `${status} · 需调整操作`, '刷新任务状态，并选择当前可用的操作。', false);
    }
    if (command.errorCode === 'command_expired' || command.status === CommandStatus.EXPIRED) {
      return new RemoteFailureNotice('远程命令已过期', '电脑未能在命令有效期内完成处理。',
        `${status} · 可重新提交`, '确认电脑在线后重新提交当前操作。', false);
    }
    return new RemoteFailureNotice('电脑未能处理命令', '命令已安全结束，但没有完成用户请求。',
      `${status} · 未完成`, '刷新任务状态后重试；持续失败时在电脑端查看任务状态。', false);
  }

  private static async submit(registration: DeviceRegistrationResult, task: RemoteTaskSnapshot,
    commandType: CommandType, payload: CommandPayload): Promise<RemoteCommandRecord> {
    if (!OPAQUE_ID_PATTERN.test(task.remoteTaskId) ||
      !Number.isSafeInteger(task.stateVersion) || task.stateVersion < 1) {
      throw new Error('当前任务快照不能用于远程命令');
    }
    const messageId: string = RemoteApiClient.randomOpaqueId();
    const clientRequestId: string = RemoteApiClient.randomOpaqueId();
    const request: AppCommandRequest = {
      schemaVersion: CONTRACT_VERSION,
      messageType: 'app/command',
      messageId: messageId,
      environment: RemoteEnvironmentConfig.environment,
      remoteTaskId: task.remoteTaskId,
      clientRequestId: clientRequestId,
      expectedStateVersion: task.stateVersion,
      expiresAt: new Date(Date.now() + COMMAND_TTL_MS).toISOString(),
      commandType: commandType,
      payload: payload
    };
    const modelConfigId: string = payload.modelConfigId ?? '';
    hilog.info(LOG_DOMAIN, LOG_TAG, '%{public}s', JSON.stringify({
      event: 'command_submit',
      commandType: commandType,
      hasText: payload.text !== undefined,
      hasInitialText: payload.initialText !== undefined,
      modelConfigIdLength: modelConfigId.length,
      modelConfigIdValid: modelConfigId.length === 0 || DIGEST_PATTERN.test(modelConfigId),
      ttlSeconds: COMMAND_TTL_MS / 1000
    }));
    const response = await RemoteApiClient.signedPost(
      `/v1/tasks/${task.remoteTaskId}/commands`, request, registration.deviceKeyId) as CommandAcceptedResponse;
    RemoteCommandCoordinator.validateResponse(
      response, 'app/command-accepted', messageId, clientRequestId, task.remoteTaskId,
      commandType, task.stateVersion);
    return response.command;
  }

  private static async query(registration: DeviceRegistrationResult,
    commandId: string): Promise<RemoteCommandRecord> {
    if (!OPAQUE_ID_PATTERN.test(commandId)) {
      throw new Error('远程命令标识无效');
    }
    const messageId: string = RemoteApiClient.randomOpaqueId();
    const sentAt: string = new Date().toISOString();
    const query: string = [
      `schemaVersion=${encodeURIComponent(CONTRACT_VERSION)}`,
      `messageType=${encodeURIComponent('app/command-query')}`,
      `messageId=${encodeURIComponent(messageId)}`,
      `environment=${encodeURIComponent(RemoteEnvironmentConfig.environment)}`,
      `sentAt=${encodeURIComponent(sentAt)}`,
      `appDeviceId=${encodeURIComponent(registration.appDeviceId)}`,
      `commandId=${encodeURIComponent(commandId)}`
    ].join('&');
    const response = await RemoteApiClient.signedGet(
      `/v1/commands/${commandId}?${query}`, registration.deviceKeyId) as CommandQueryResponse;
    RemoteCommandCoordinator.validateResponse(
      response, 'app/command-status', messageId, undefined, undefined, undefined, undefined);
    if (response.command.commandId !== commandId) {
      throw new Error('云端命令查询关联不一致');
    }
    return response.command;
  }

  private static validateResponse(response: CommandAcceptedResponse | CommandQueryResponse,
    messageType: string, requestMessageId: string, clientRequestId?: string,
    remoteTaskId?: string, commandType?: CommandType, expectedStateVersion?: number): void {
    const command: RemoteCommandRecord = response.command;
    if (response.schemaVersion !== CONTRACT_VERSION || response.messageType !== messageType ||
      response.requestMessageId !== requestMessageId ||
      response.environment !== RemoteEnvironmentConfig.environment ||
      !OPAQUE_ID_PATTERN.test(response.messageId) ||
      Number.isNaN(Date.parse(response.serverReceivedAt)) ||
      !OPAQUE_ID_PATTERN.test(command.commandId) ||
      !OPAQUE_ID_PATTERN.test(command.clientRequestId) ||
      !OPAQUE_ID_PATTERN.test(command.remoteTaskId) ||
      !DIGEST_PATTERN.test(command.payloadDigest) ||
      !Number.isSafeInteger(command.expectedStateVersion) || command.expectedStateVersion < 1 ||
      Number.isNaN(Date.parse(command.createdAt)) || Number.isNaN(Date.parse(command.expiresAt)) ||
      !RemoteCommandCoordinator.validStatus(command.status) ||
      (clientRequestId !== undefined && command.clientRequestId !== clientRequestId) ||
      (remoteTaskId !== undefined && command.remoteTaskId !== remoteTaskId) ||
      (commandType !== undefined && command.commandType !== commandType) ||
      (expectedStateVersion !== undefined && command.expectedStateVersion !== expectedStateVersion) ||
      (command.appliedStateVersion !== undefined &&
        (!Number.isSafeInteger(command.appliedStateVersion) || command.appliedStateVersion < 1))) {
      throw new Error('云端命令响应字段或关联无效');
    }
  }

  private static isTerminal(status: CommandStatus): boolean {
    return status === CommandStatus.COMPLETED || status === CommandStatus.REJECTED ||
      status === CommandStatus.FAILED || status === CommandStatus.EXPIRED ||
      status === CommandStatus.CANCELLED || status === CommandStatus.RECONCILING;
  }

  private static validStatus(status: CommandStatus): boolean {
    return status === CommandStatus.QUEUED || status === CommandStatus.DISPATCHED ||
      status === CommandStatus.ACCEPTED || status === CommandStatus.APPLYING ||
      status === CommandStatus.COMPLETED || status === CommandStatus.REJECTED ||
      status === CommandStatus.FAILED || status === CommandStatus.EXPIRED ||
      status === CommandStatus.CANCELLED || status === CommandStatus.RECONCILING;
  }

  private static publicCommandErrorCode(errorCode?: string): string {
    if (errorCode === 'pc_offline' || errorCode === 'state_conflict' || errorCode === 'invalid_command_state' ||
      errorCode === 'command_expired' || errorCode === 'payload_digest_conflict') {
      return errorCode;
    }
    return '';
  }

  private static commandStatusText(status: CommandStatus): string {
    if (status === CommandStatus.REJECTED) {
      return '已拒绝';
    }
    if (status === CommandStatus.EXPIRED) {
      return '已过期';
    }
    if (status === CommandStatus.CANCELLED) {
      return '已取消';
    }
    return '处理失败';
  }
}
