import { DeviceRegistrationResult } from './DeviceRegistrationCoordinator';
import { RemoteApiClient } from './RemoteApiClient';
import { RemoteEnvironmentConfig } from './RemoteEnvironmentConfig';

const CONTRACT_VERSION = '1.5';
const OPAQUE_ID_PATTERN = /^[A-Za-z0-9_-]{16,128}$/;

export interface PushRefreshTarget {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  requestMessageId: string;
  environment: string;
  serverReceivedAt: string;
  pcDeviceId: string;
  installationId: string;
  bindingEpoch: number;
  remoteTaskId: string;
  terminalStateVersion: number;
}

export class PushRefreshCoordinator {
  static async resolve(refreshRef: string,
    registration: DeviceRegistrationResult): Promise<PushRefreshTarget> {
    if (!OPAQUE_ID_PATTERN.test(refreshRef)) {
      throw new Error('推送刷新引用无效');
    }
    const messageId: string = RemoteApiClient.randomOpaqueId();
    const query: string = [
      `schemaVersion=${encodeURIComponent(CONTRACT_VERSION)}`,
      `messageType=${encodeURIComponent('app/push-refresh-query')}`,
      `messageId=${encodeURIComponent(messageId)}`,
      `environment=${encodeURIComponent(RemoteEnvironmentConfig.environment)}`,
      `sentAt=${encodeURIComponent(new Date().toISOString())}`,
      `appDeviceId=${encodeURIComponent(registration.appDeviceId)}`
    ].join('&');
    const response = await RemoteApiClient.signedGet(
      `/v1/push-refresh/${refreshRef}?${query}`, registration.deviceKeyId) as PushRefreshTarget;
    PushRefreshCoordinator.validateResponse(response, messageId);
    return response;
  }

  private static validateResponse(response: PushRefreshTarget, requestMessageId: string): void {
    if (response.schemaVersion !== CONTRACT_VERSION ||
      response.messageType !== 'app/push-refresh' ||
      response.requestMessageId !== requestMessageId ||
      response.environment !== RemoteEnvironmentConfig.environment ||
      !OPAQUE_ID_PATTERN.test(response.messageId) ||
      !OPAQUE_ID_PATTERN.test(response.pcDeviceId) ||
      !OPAQUE_ID_PATTERN.test(response.installationId) ||
      !OPAQUE_ID_PATTERN.test(response.remoteTaskId) ||
      !Number.isSafeInteger(response.bindingEpoch) || response.bindingEpoch < 1 ||
      !Number.isSafeInteger(response.terminalStateVersion) || response.terminalStateVersion < 1 ||
      Number.isNaN(Date.parse(response.serverReceivedAt))) {
      throw new Error('云端推送刷新响应无效');
    }
  }
}
