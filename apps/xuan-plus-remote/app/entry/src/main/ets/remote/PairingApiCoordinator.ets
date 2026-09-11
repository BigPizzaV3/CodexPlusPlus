import { DeviceRegistrationResult } from './DeviceRegistrationCoordinator';
import { PairingQrPayload } from './QrPairingCoordinator';
import { RemoteApiClient } from './RemoteApiClient';
import { RemoteEnvironmentConfig } from './RemoteEnvironmentConfig';
import { PcConnectionState } from './RemoteModels';

const CONTRACT_VERSION = '1.5';
const OPAQUE_ID_PATTERN = /^[A-Za-z0-9_-]{16,128}$/;
const DIGEST_PATTERN = /^[a-f0-9]{64}$/;
const SAFETY_PHRASE_PATTERN = /^[a-z]{3,16}(?:-[a-z]{3,16}){2}$/;

interface PairingConsumeRequest {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  environment: string;
  sentAt: string;
  appDeviceId: string;
  pairing: PairingQrPayload;
}

interface BindingRevocationRequest {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  environment: string;
  sentAt: string;
  appDeviceId: string;
  bindingId: string;
}

export interface BindingSummary {
  environment: string;
  pcDisplayName: string;
  appDisplayName: string;
  safetyPhrase: string;
  summaryDigest: string;
}

export interface PairingPendingResponse {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  requestMessageId: string;
  environment: string;
  serverReceivedAt: string;
  bindingId: string;
  pcDeviceId: string;
  installationId: string;
  confirmationNonce: string;
  confirmationExpiresAt: string;
  bindingSummary: BindingSummary;
  bindingState: string;
}

export interface PcDeviceSummary {
  bindingId: string;
  pcDeviceId: string;
  installationId: string;
  bindingEpoch: number;
  displayName: string;
  pcConnectionState: PcConnectionState;
  pcObservedAt: string;
  modelConfigs: ModelConfigSummary[];
}

export interface BindingRevokedResponse {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  requestMessageId: string;
  environment: string;
  serverReceivedAt: string;
  bindingId: string;
  bindingState: string;
  revokedAt: string;
}

export interface ModelConfigSummary {
  id: string;
  model: string;
  provider: string;
}

export interface PcDeviceListResponse {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  requestMessageId: string;
  environment: string;
  serverReceivedAt: string;
  pcDevices: PcDeviceSummary[];
  nextCursor: string | null;
}

export class PairingApiCoordinator {
  static async consume(registration: DeviceRegistrationResult,
    pairing: PairingQrPayload): Promise<PairingPendingResponse> {
    const messageId: string = RemoteApiClient.randomOpaqueId();
    const request: PairingConsumeRequest = {
      schemaVersion: CONTRACT_VERSION,
      messageType: 'pairing/consume',
      messageId: messageId,
      environment: RemoteEnvironmentConfig.environment,
      sentAt: new Date().toISOString(),
      appDeviceId: registration.appDeviceId,
      pairing: pairing
    };
    const response = await RemoteApiClient.signedPost(
      '/v1/pairings/consume', request, registration.deviceKeyId) as PairingPendingResponse;
    PairingApiCoordinator.validatePending(response, messageId);
    return response;
  }

  static async listPcDevices(registration: DeviceRegistrationResult): Promise<PcDeviceListResponse> {
    const messageId: string = RemoteApiClient.randomOpaqueId();
    const sentAt: string = new Date().toISOString();
    const query: string = [
      `schemaVersion=${encodeURIComponent(CONTRACT_VERSION)}`,
      `messageType=${encodeURIComponent('app/pc-devices-query')}`,
      `messageId=${encodeURIComponent(messageId)}`,
      `environment=${encodeURIComponent(RemoteEnvironmentConfig.environment)}`,
      `sentAt=${encodeURIComponent(sentAt)}`,
      `appDeviceId=${encodeURIComponent(registration.appDeviceId)}`
    ].join('&');
    const response = await RemoteApiClient.signedGet(
      `/v1/pc-devices?${query}`, registration.deviceKeyId) as PcDeviceListResponse;
    PairingApiCoordinator.validatePcDevices(response, messageId);
    return response;
  }

  static async revokeBinding(registration: DeviceRegistrationResult,
    bindingId: string): Promise<BindingRevokedResponse> {
    if (!OPAQUE_ID_PATTERN.test(bindingId)) {
      throw new Error('待删除电脑绑定标识无效');
    }
    const messageId: string = RemoteApiClient.randomOpaqueId();
    const request: BindingRevocationRequest = {
      schemaVersion: CONTRACT_VERSION,
      messageType: 'binding/revoke',
      messageId: messageId,
      environment: RemoteEnvironmentConfig.environment,
      sentAt: new Date().toISOString(),
      appDeviceId: registration.appDeviceId,
      bindingId: bindingId
    };
    const response = await RemoteApiClient.signedPost(
      `/v1/bindings/${encodeURIComponent(bindingId)}/revoke`, request,
      registration.deviceKeyId) as BindingRevokedResponse;
    PairingApiCoordinator.validateRevoked(response, messageId, bindingId);
    return response;
  }

  private static validatePending(response: PairingPendingResponse, requestMessageId: string): void {
    const summary = response.bindingSummary;
    if (response.schemaVersion !== CONTRACT_VERSION ||
      response.messageType !== 'pairing/pending' ||
      response.requestMessageId !== requestMessageId ||
      response.environment !== RemoteEnvironmentConfig.environment ||
      (response.bindingState !== 'pending_local_confirmation' && response.bindingState !== 'active') ||
      !OPAQUE_ID_PATTERN.test(response.messageId) ||
      !OPAQUE_ID_PATTERN.test(response.bindingId) ||
      !OPAQUE_ID_PATTERN.test(response.pcDeviceId) ||
      !OPAQUE_ID_PATTERN.test(response.installationId) ||
      !OPAQUE_ID_PATTERN.test(response.confirmationNonce) ||
      Number.isNaN(Date.parse(response.serverReceivedAt)) ||
      Number.isNaN(Date.parse(response.confirmationExpiresAt)) ||
      (response.bindingState === 'pending_local_confirmation' &&
        Date.parse(response.confirmationExpiresAt) <= Date.now()) ||
      summary.environment !== RemoteEnvironmentConfig.environment ||
      summary.pcDisplayName.length === 0 || summary.pcDisplayName.length > 128 ||
      summary.appDisplayName.length === 0 || summary.appDisplayName.length > 128 ||
      !SAFETY_PHRASE_PATTERN.test(summary.safetyPhrase) ||
      !DIGEST_PATTERN.test(summary.summaryDigest)) {
      throw new Error('云端配对待确认响应无效');
    }
  }

  private static validatePcDevices(response: PcDeviceListResponse, requestMessageId: string): void {
    if (response.schemaVersion !== CONTRACT_VERSION ||
      response.messageType !== 'app/pc-devices' ||
      response.requestMessageId !== requestMessageId ||
      response.environment !== RemoteEnvironmentConfig.environment ||
      !OPAQUE_ID_PATTERN.test(response.messageId) ||
      Number.isNaN(Date.parse(response.serverReceivedAt)) ||
      !Array.isArray(response.pcDevices) || response.pcDevices.length > 100 ||
      (response.nextCursor !== null && typeof response.nextCursor !== 'string')) {
      throw new Error('云端电脑绑定列表响应无效');
    }
    for (let index = 0; index < response.pcDevices.length; index++) {
      const device: PcDeviceSummary = response.pcDevices[index];
      if (!OPAQUE_ID_PATTERN.test(device.bindingId) || !OPAQUE_ID_PATTERN.test(device.pcDeviceId) ||
        !OPAQUE_ID_PATTERN.test(device.installationId) ||
        !Number.isSafeInteger(device.bindingEpoch) || device.bindingEpoch < 1 ||
        device.displayName.length === 0 || device.displayName.length > 128 ||
        (device.pcConnectionState !== 'online' && device.pcConnectionState !== 'stale' &&
          device.pcConnectionState !== 'offline') ||
        Number.isNaN(Date.parse(device.pcObservedAt)) ||
        !Array.isArray(device.modelConfigs) || device.modelConfigs.length > 100) {
        throw new Error('云端电脑绑定摘要无效');
      }
      for (let modelIndex: number = 0; modelIndex < device.modelConfigs.length; modelIndex++) {
        const config: ModelConfigSummary = device.modelConfigs[modelIndex];
        if (!DIGEST_PATTERN.test(config.id) || config.model.trim().length === 0 ||
          config.model.length > 256 || config.provider.trim().length === 0 || config.provider.length > 128) {
          throw new Error('云端电脑模型目录无效');
        }
      }
    }
  }

  private static validateRevoked(response: BindingRevokedResponse, requestMessageId: string,
    bindingId: string): void {
    if (response.schemaVersion !== CONTRACT_VERSION || response.messageType !== 'binding/revoked' ||
      response.requestMessageId !== requestMessageId ||
      response.environment !== RemoteEnvironmentConfig.environment ||
      !OPAQUE_ID_PATTERN.test(response.messageId) || response.bindingId !== bindingId ||
      response.bindingState !== 'revoked' || Number.isNaN(Date.parse(response.serverReceivedAt)) ||
      Number.isNaN(Date.parse(response.revokedAt))) {
      throw new Error('云端删除电脑绑定响应无效');
    }
  }
}
