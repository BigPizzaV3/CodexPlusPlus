import huks from '@ohos.security.huks';
import { BusinessError } from '@kit.BasicServicesKit';

const DEVICE_KEY_ALIAS = 'workagents_remote_dev_ed25519_v3';
const ED25519_RAW_PUBLIC_KEY_LENGTH = 32;
const ED25519_SPKI_PUBLIC_KEY_LENGTH = 44;
const ED25519_SPKI_PREFIX: Uint8Array = new Uint8Array([
  0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00
]);

const GENERATE_OPTIONS: huks.HuksOptions = {
  properties: [
    {
      tag: huks.HuksTag.HUKS_TAG_ALGORITHM,
      value: huks.HuksKeyAlg.HUKS_ALG_ED25519
    },
    {
      tag: huks.HuksTag.HUKS_TAG_PURPOSE,
      value: huks.HuksKeyPurpose.HUKS_KEY_PURPOSE_SIGN | huks.HuksKeyPurpose.HUKS_KEY_PURPOSE_VERIFY
    },
    {
      tag: huks.HuksTag.HUKS_TAG_KEY_SIZE,
      value: huks.HuksKeySize.HUKS_CURVE25519_KEY_SIZE_256
    },
    {
      tag: huks.HuksTag.HUKS_TAG_DIGEST,
      value: huks.HuksKeyDigest.HUKS_DIGEST_NONE
    }
  ]
};

const SIGN_OPTIONS: huks.HuksOptions = {
  properties: [
    {
      tag: huks.HuksTag.HUKS_TAG_ALGORITHM,
      value: huks.HuksKeyAlg.HUKS_ALG_ED25519
    },
    {
      tag: huks.HuksTag.HUKS_TAG_PURPOSE,
      value: huks.HuksKeyPurpose.HUKS_KEY_PURPOSE_SIGN
    },
    {
      tag: huks.HuksTag.HUKS_TAG_DIGEST,
      value: huks.HuksKeyDigest.HUKS_DIGEST_NONE
    }
  ]
};

export class DeviceIdentityCoordinator {
  static async ensureDeviceKey(): Promise<void> {
    try {
      const exists: boolean = await huks.hasKeyItem(DEVICE_KEY_ALIAS, {});
      if (!exists) {
        await huks.generateKeyItem(DEVICE_KEY_ALIAS, GENERATE_OPTIONS);
      }
    } catch (error) {
      throw new Error(`设备密钥初始化失败（${DeviceIdentityCoordinator.errorCode(error)}）`);
    }
  }

  static async exportPublicKey(): Promise<Uint8Array> {
    try {
      await DeviceIdentityCoordinator.ensureDeviceKey();
      const result: huks.HuksReturnResult = await huks.exportKeyItem(DEVICE_KEY_ALIAS, {});
      if (result.outData !== undefined && result.outData.length > 0) {
        return DeviceIdentityCoordinator.normalizePublicKey(result.outData);
      }
      if (result.properties !== undefined) {
        for (const property of result.properties) {
          if (property.tag === huks.HuksTag.HUKS_TAG_ASYMMETRIC_PUBLIC_KEY_DATA &&
            property.value instanceof Uint8Array && property.value.length > 0) {
            return DeviceIdentityCoordinator.normalizePublicKey(property.value);
          }
        }
      }
      const propertyCount = result.properties === undefined ? 0 : result.properties.length;
      throw new Error(`设备公钥为空（properties=${propertyCount}）`);
    } catch (error) {
      if (error instanceof Error && error.message.startsWith('设备公钥')) {
        throw error;
      }
      throw new Error(`设备公钥导出失败（${DeviceIdentityCoordinator.errorCode(error)}）`);
    }
  }

  static async sign(payload: Uint8Array): Promise<Uint8Array> {
    if (payload.length === 0) {
      throw new Error('设备签名载荷不能为空');
    }
    try {
      await DeviceIdentityCoordinator.ensureDeviceKey();
      const session: huks.HuksSessionHandle = await huks.initSession(DEVICE_KEY_ALIAS, SIGN_OPTIONS);
      const result: huks.HuksReturnResult = await huks.finishSession(session.handle, { inData: payload });
      if (result.outData === undefined || result.outData.length !== 64) {
        throw new Error('设备签名长度无效');
      }
      return result.outData;
    } catch (error) {
      throw new Error(`设备签名失败（${DeviceIdentityCoordinator.errorCode(error)}）`);
    }
  }

  private static normalizePublicKey(publicKey: Uint8Array): Uint8Array {
    if (publicKey.length === ED25519_RAW_PUBLIC_KEY_LENGTH) {
      const spki = new Uint8Array(ED25519_SPKI_PUBLIC_KEY_LENGTH);
      spki.set(ED25519_SPKI_PREFIX, 0);
      spki.set(publicKey, ED25519_SPKI_PREFIX.length);
      return spki;
    }
    if (publicKey.length === ED25519_SPKI_PUBLIC_KEY_LENGTH) {
      for (let index = 0; index < ED25519_SPKI_PREFIX.length; index++) {
        if (publicKey[index] !== ED25519_SPKI_PREFIX[index]) {
          throw new Error(`设备公钥格式无效（length=${publicKey.length}）`);
        }
      }
      return publicKey;
    }
    throw new Error(`设备公钥格式无效（length=${publicKey.length}）`);
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
