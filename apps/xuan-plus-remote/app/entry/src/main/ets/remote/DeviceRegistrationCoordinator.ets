import cryptoFramework from '@ohos.security.cryptoFramework';
import util from '@ohos.util';
import { BusinessError } from '@kit.BasicServicesKit';
import { pushService } from '@kit.PushKit';
import { DeviceIdentityCoordinator } from './DeviceIdentityCoordinator';
import { RemoteApiClient } from './RemoteApiClient';
import { RemoteEnvironmentConfig } from './RemoteEnvironmentConfig';

const CONTRACT_VERSION = '1.5';
const DEVICE_KEY_ALGORITHM = 'ed25519';
const APP_DISPLAY_NAME = '轩++远程';
const APP_VERSION = '0.1.9-dev';
const OPAQUE_ID_PATTERN = /^[A-Za-z0-9_-]{16,128}$/;

interface DeviceChallengeRequest {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  environment: string;
  sentAt: string;
  appDeviceId: string;
  deviceKeyAlgorithm: string;
  devicePublicKey: string;
}

interface DeviceChallengeResponse {
  schemaVersion: string;
  messageType: string;
  requestMessageId: string;
  environment: string;
  appDeviceId: string;
  challengeId: string;
  challenge: string;
  expiresAt: string;
}

interface DeviceRegistrationRequest {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  environment: string;
  sentAt: string;
  appDeviceId: string;
  deviceKeyAlgorithm: string;
  devicePublicKey: string;
  challengeId: string;
  challenge: string;
  registrationSignature: string;
  pushProvider: string;
  pushToken: string;
  appDisplayName: string;
  appVersion: string;
}

interface DeviceRegistrationResponse {
  schemaVersion: string;
  messageType: string;
  requestMessageId: string;
  environment: string;
  appDeviceId: string;
  deviceKeyId: string;
  registrationState: string;
}

export class DeviceRegistrationResult {
  appDeviceId: string;
  deviceKeyId: string;

  constructor(appDeviceId: string, deviceKeyId: string) {
    this.appDeviceId = appDeviceId;
    this.deviceKeyId = deviceKeyId;
  }
}

export class DeviceRegistrationCoordinator {
  static async registerCurrentDevice(): Promise<DeviceRegistrationResult> {
    RemoteEnvironmentConfig.assertReady();
    const publicKey: Uint8Array = await DeviceIdentityCoordinator.exportPublicKey();
    const publicKeyDigest: string = await DeviceRegistrationCoordinator.sha256Hex(publicKey);
    const appDeviceId: string = `app_device_${publicKeyDigest}`;
    const encodedPublicKey: string = DeviceRegistrationCoordinator.base64Url(publicKey);
    const pushToken: string = await DeviceRegistrationCoordinator.readPushToken();

    const challengeMessageId: string = DeviceRegistrationCoordinator.randomOpaqueId();
    const challengeRequest: DeviceChallengeRequest = {
      schemaVersion: CONTRACT_VERSION,
      messageType: 'app/device-registration-challenge',
      messageId: challengeMessageId,
      environment: RemoteEnvironmentConfig.environment,
      sentAt: new Date().toISOString(),
      appDeviceId: appDeviceId,
      deviceKeyAlgorithm: DEVICE_KEY_ALGORITHM,
      devicePublicKey: encodedPublicKey
    };
    const challenge: DeviceChallengeResponse = await RemoteApiClient.publicPost(
      '/v1/app-devices/challenges', challengeRequest) as DeviceChallengeResponse;
    DeviceRegistrationCoordinator.validateChallenge(challenge, challengeMessageId, appDeviceId);

    const registrationProof: string = [
      'workagents-device-registration-v1',
      RemoteEnvironmentConfig.environment,
      appDeviceId,
      challenge.challengeId,
      challenge.challenge,
      publicKeyDigest
    ].join('\n');
    const signature: Uint8Array = await DeviceIdentityCoordinator.sign(
      new util.TextEncoder().encodeInto(registrationProof));
    const registrationMessageId: string = DeviceRegistrationCoordinator.randomOpaqueId();
    const registrationRequest: DeviceRegistrationRequest = {
      schemaVersion: CONTRACT_VERSION,
      messageType: 'app/device-register',
      messageId: registrationMessageId,
      environment: RemoteEnvironmentConfig.environment,
      sentAt: new Date().toISOString(),
      appDeviceId: appDeviceId,
      deviceKeyAlgorithm: DEVICE_KEY_ALGORITHM,
      devicePublicKey: encodedPublicKey,
      challengeId: challenge.challengeId,
      challenge: challenge.challenge,
      registrationSignature: DeviceRegistrationCoordinator.base64Url(signature),
      pushProvider: 'huawei_push_kit',
      pushToken: pushToken,
      appDisplayName: APP_DISPLAY_NAME,
      appVersion: APP_VERSION
    };
    const registered: DeviceRegistrationResponse = await RemoteApiClient.publicPost(
      '/v1/app-devices', registrationRequest) as DeviceRegistrationResponse;
    DeviceRegistrationCoordinator.validateRegistration(
      registered, registrationMessageId, appDeviceId);
    return new DeviceRegistrationResult(appDeviceId, registered.deviceKeyId);
  }

  private static async readPushToken(): Promise<string> {
    try {
      const pushToken: string = await pushService.getToken();
      if (pushToken.length === 0 || pushToken.length > 4096) {
        throw new Error('Push Token 长度无效');
      }
      return pushToken;
    } catch (error) {
      const pushErrorCode: string = DeviceRegistrationCoordinator.errorCode(error);
      if (pushErrorCode === '1000900010') {
        throw new Error('当前设备未提供 Huawei Push Kit，暂时无法完成云端登记；模拟器仍可用于界面预览');
      }
      throw new Error('推送服务暂不可用，无法完成云端登记；请确认设备支持 Huawei Push Kit，并检查应用签名');
    }
  }

  private static validateChallenge(response: DeviceChallengeResponse, requestMessageId: string,
    appDeviceId: string): void {
    if (response.schemaVersion !== CONTRACT_VERSION ||
      response.messageType !== 'app/device-registration-challenged' ||
      response.requestMessageId !== requestMessageId ||
      response.environment !== RemoteEnvironmentConfig.environment ||
      response.appDeviceId !== appDeviceId ||
      !OPAQUE_ID_PATTERN.test(response.challengeId) ||
      !OPAQUE_ID_PATTERN.test(response.challenge) ||
      Number.isNaN(Date.parse(response.expiresAt)) ||
      Date.parse(response.expiresAt) <= Date.now()) {
      throw new Error('云端设备挑战响应无效');
    }
  }

  private static validateRegistration(response: DeviceRegistrationResponse,
    requestMessageId: string, appDeviceId: string): void {
    if (response.schemaVersion !== CONTRACT_VERSION ||
      response.messageType !== 'app/device-registered' ||
      response.requestMessageId !== requestMessageId ||
      response.environment !== RemoteEnvironmentConfig.environment ||
      response.appDeviceId !== appDeviceId ||
      response.registrationState !== 'registered' ||
      !OPAQUE_ID_PATTERN.test(response.deviceKeyId)) {
      throw new Error('云端设备注册响应无效');
    }
  }

  private static async sha256Hex(value: Uint8Array): Promise<string> {
    try {
      const digest = cryptoFramework.createMd('SHA256');
      await digest.update({ data: value });
      const result: cryptoFramework.DataBlob = await digest.digest();
      let hex = '';
      for (let index = 0; index < result.data.length; index++) {
        hex += result.data[index].toString(16).padStart(2, '0');
      }
      return hex;
    } catch (_error) {
      throw new Error('设备公钥摘要计算失败');
    }
  }

  private static randomOpaqueId(): string {
    try {
      const random = cryptoFramework.createRandom().generateRandomSync(24);
      return DeviceRegistrationCoordinator.base64Url(random.data);
    } catch (_error) {
      throw new Error('安全随机标识生成失败');
    }
  }

  private static base64Url(value: Uint8Array): string {
    return new util.Base64Helper()
      .encodeToStringSync(value, util.Type.BASIC_URL_SAFE)
      .replace(/=+$/, '');
  }

  private static errorCode(error: Object): string {
    const businessError = error as BusinessError;
    if (typeof businessError.code === 'number' || typeof businessError.code === 'string') {
      return businessError.code.toString();
    }
    const safeDescription = String(error).replace(/[^A-Za-z0-9_-]/g, '').slice(0, 80);
    return safeDescription.length > 0 ? safeDescription : 'unknown';
  }
}
