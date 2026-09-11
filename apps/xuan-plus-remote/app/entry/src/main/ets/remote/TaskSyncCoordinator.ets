import { DeviceRegistrationResult } from './DeviceRegistrationCoordinator';
import { PcDeviceSummary } from './PairingApiCoordinator';
import { RemoteApiClient } from './RemoteApiClient';
import { RemoteEnvironmentConfig } from './RemoteEnvironmentConfig';
import { PcConnectionState, ReplyState, TaskStatus, TurnStatus, utf8ByteLength } from './RemoteModels';

const CONTRACT_VERSION = '1.5';
const OPAQUE_ID_PATTERN = /^[A-Za-z0-9_-]{16,128}$/;

export interface RemoteReplySnapshot {
  state: ReplyState;
  text: string | null;
  byteLength: number;
  truncated: boolean;
}

export interface RemoteTaskSnapshot {
  remoteTaskId: string;
  pcDeviceId: string;
  installationId: string;
  bindingEpoch: number;
  name: string;
  workspaceName: string;
  modelLabel: string;
  taskStatus: TaskStatus;
  turnStatus: TurnStatus;
  lastTurnOutcome: string;
  lastReply: RemoteReplySnapshot | null;
  lastReplyState: ReplyState;
  lastReplyVersion: number | null;
  lastError: string | null;
  pcObservedAt: string;
  serverReceivedAt: string;
  stateVersion: number;
  pcConnectionState: PcConnectionState;
}

export interface TaskListResponse {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  requestMessageId: string;
  environment: string;
  serverReceivedAt: string;
  pcDeviceId: string;
  installationId: string;
  bindingEpoch: number;
  tasks: RemoteTaskSnapshot[];
  nextCursor: string | null;
}

export interface TaskDetailResponse {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  requestMessageId: string;
  environment: string;
  serverReceivedAt: string;
  snapshot: RemoteTaskSnapshot;
}

export class TaskSyncCoordinator {
  static async getTask(registration: DeviceRegistrationResult,
    task: RemoteTaskSnapshot): Promise<TaskDetailResponse> {
    if (!OPAQUE_ID_PATTERN.test(task.remoteTaskId)) {
      throw new Error('当前任务标识无效');
    }
    const messageId: string = RemoteApiClient.randomOpaqueId();
    const query: string = [
      `schemaVersion=${encodeURIComponent(CONTRACT_VERSION)}`,
      `messageType=${encodeURIComponent('app/task-query')}`,
      `messageId=${encodeURIComponent(messageId)}`,
      `environment=${encodeURIComponent(RemoteEnvironmentConfig.environment)}`,
      `sentAt=${encodeURIComponent(new Date().toISOString())}`,
      `appDeviceId=${encodeURIComponent(registration.appDeviceId)}`,
      `remoteTaskId=${encodeURIComponent(task.remoteTaskId)}`
    ].join('&');
    const response = await RemoteApiClient.signedGet(
      `/v1/tasks/${task.remoteTaskId}?${query}`,
      registration.deviceKeyId) as TaskDetailResponse;
    if (response.schemaVersion !== CONTRACT_VERSION || response.messageType !== 'app/task' ||
      response.requestMessageId !== messageId ||
      response.environment !== RemoteEnvironmentConfig.environment ||
      !OPAQUE_ID_PATTERN.test(response.messageId) ||
      Number.isNaN(Date.parse(response.serverReceivedAt)) ||
      response.snapshot.remoteTaskId !== task.remoteTaskId) {
      throw new Error('云端任务详情响应无效');
    }
    TaskSyncCoordinator.validateTask(response.snapshot, task);
    return response;
  }

  static async listTasks(registration: DeviceRegistrationResult,
    pcDevice: PcDeviceSummary, cursor?: string): Promise<TaskListResponse> {
    if (!OPAQUE_ID_PATTERN.test(pcDevice.pcDeviceId)) {
      throw new Error('当前电脑绑定标识无效');
    }
    const messageId: string = RemoteApiClient.randomOpaqueId();
    const queryParts: string[] = [
      `schemaVersion=${encodeURIComponent(CONTRACT_VERSION)}`,
      `messageType=${encodeURIComponent('app/task-list-query')}`,
      `messageId=${encodeURIComponent(messageId)}`,
      `environment=${encodeURIComponent(RemoteEnvironmentConfig.environment)}`,
      `sentAt=${encodeURIComponent(new Date().toISOString())}`,
      `appDeviceId=${encodeURIComponent(registration.appDeviceId)}`,
      `pcDeviceId=${encodeURIComponent(pcDevice.pcDeviceId)}`,
      'limit=100'
    ];
    if (cursor !== undefined) {
      if (cursor.length === 0 || TaskSyncCoordinator.utf8Length(cursor) > 256) {
        throw new Error('任务分页游标无效');
      }
      queryParts.push(`cursor=${encodeURIComponent(cursor)}`);
    }
    const query: string = queryParts.join('&');
    const response = await RemoteApiClient.signedGet(
      `/v1/pc-devices/${pcDevice.pcDeviceId}/tasks?${query}`,
      registration.deviceKeyId) as TaskListResponse;
    TaskSyncCoordinator.validateResponse(response, messageId, pcDevice);
    return response;
  }

  private static validateResponse(response: TaskListResponse, requestMessageId: string,
    pcDevice: PcDeviceSummary): void {
    if (response.schemaVersion !== CONTRACT_VERSION ||
      response.messageType !== 'app/task-list' ||
      response.requestMessageId !== requestMessageId ||
      response.environment !== RemoteEnvironmentConfig.environment ||
      response.pcDeviceId !== pcDevice.pcDeviceId ||
      response.installationId !== pcDevice.installationId ||
      response.bindingEpoch !== pcDevice.bindingEpoch ||
      !OPAQUE_ID_PATTERN.test(response.messageId) ||
      Number.isNaN(Date.parse(response.serverReceivedAt)) ||
      !Array.isArray(response.tasks) || response.tasks.length > 100 ||
      (response.nextCursor !== null && (typeof response.nextCursor !== 'string' ||
        response.nextCursor.length === 0 || TaskSyncCoordinator.utf8Length(response.nextCursor) > 256))) {
      throw new Error('云端任务列表响应无效');
    }
    for (let index: number = 0; index < response.tasks.length; index++) {
      TaskSyncCoordinator.validateTask(response.tasks[index], pcDevice);
    }
  }

  private static validateTask(task: RemoteTaskSnapshot,
    pcDevice: PcDeviceSummary | RemoteTaskSnapshot): void {
    if (!OPAQUE_ID_PATTERN.test(task.remoteTaskId) ||
      task.pcDeviceId !== pcDevice.pcDeviceId ||
      task.installationId !== pcDevice.installationId ||
      task.bindingEpoch !== pcDevice.bindingEpoch ||
      !Number.isSafeInteger(task.stateVersion) || task.stateVersion < 1 ||
      TaskSyncCoordinator.utf8Length(task.name) > 256 ||
      TaskSyncCoordinator.utf8Length(task.workspaceName) > 256 ||
      task.modelLabel.trim().length === 0 || TaskSyncCoordinator.utf8Length(task.modelLabel) > 256 ||
      !TaskSyncCoordinator.validTaskStatus(task.taskStatus) ||
      !TaskSyncCoordinator.validTurnStatus(task.turnStatus) ||
      !TaskSyncCoordinator.validTurnOutcome(task.lastTurnOutcome) ||
      !TaskSyncCoordinator.validReplyState(task.lastReplyState) ||
      !TaskSyncCoordinator.validConnectionState(task.pcConnectionState) ||
      Number.isNaN(Date.parse(task.pcObservedAt)) ||
      Number.isNaN(Date.parse(task.serverReceivedAt)) ||
      (task.lastError !== null && TaskSyncCoordinator.utf8Length(task.lastError) > 512) ||
      (task.lastReplyVersion !== null &&
        (!Number.isSafeInteger(task.lastReplyVersion) || task.lastReplyVersion < 1))) {
      throw new Error('云端任务快照字段无效');
    }
    if (task.lastReply === null) {
      if (task.lastReplyState !== ReplyState.ABSENT && task.lastReplyState !== ReplyState.WITHHELD) {
        throw new Error('云端任务回复状态无效');
      }
      return;
    }
    const reply: RemoteReplySnapshot = task.lastReply;
    const replyLength: number = reply.text === null ? 0 : TaskSyncCoordinator.utf8Length(reply.text);
    if (reply.state !== task.lastReplyState ||
      (reply.state !== ReplyState.AVAILABLE && reply.state !== ReplyState.TRUNCATED) ||
      reply.text === null || reply.byteLength !== replyLength ||
      !Number.isSafeInteger(reply.byteLength) || reply.byteLength < 0 ||
      task.lastReplyVersion === null) {
      throw new Error('云端任务安全回复无效');
    }
  }

  private static utf8Length(value: string): number {
    return utf8ByteLength(value);
  }

  private static validTaskStatus(value: TaskStatus): boolean {
    return value === TaskStatus.CREATED || value === TaskStatus.QUEUED || value === TaskStatus.STARTING ||
      value === TaskStatus.RUNNING || value === TaskStatus.PAUSING || value === TaskStatus.PAUSED ||
      value === TaskStatus.STOPPING || value === TaskStatus.STOPPED || value === TaskStatus.FAILED ||
      value === TaskStatus.RECONCILING || value === TaskStatus.ARCHIVED;
  }

  private static validTurnStatus(value: TurnStatus): boolean {
    return value === TurnStatus.IDLE || value === TurnStatus.STARTING || value === TurnStatus.RUNNING ||
      value === TurnStatus.PAUSING || value === TurnStatus.COMPLETED || value === TurnStatus.FAILED ||
      value === TurnStatus.INTERRUPTED || value === TurnStatus.STOPPED || value === TurnStatus.RECONCILING;
  }

  private static validTurnOutcome(value: string): boolean {
    return value === 'none' || value === 'completed' || value === 'failed' || value === 'interrupted' ||
      value === 'stopped' || value === 'reconciling';
  }

  private static validReplyState(value: ReplyState): boolean {
    return value === ReplyState.AVAILABLE || value === ReplyState.ABSENT ||
      value === ReplyState.WITHHELD || value === ReplyState.TRUNCATED;
  }

  private static validConnectionState(value: PcConnectionState): boolean {
    return value === PcConnectionState.ONLINE || value === PcConnectionState.STALE ||
      value === PcConnectionState.OFFLINE;
  }
}
