import { common } from '@kit.AbilityKit';
import { scanBarcode, scanCore } from '@kit.ScanKit';
import { RemoteEnvironment } from './RemoteModels';

export interface PairingQrPayload {
  pairingQrVersion: string;
  environment: RemoteEnvironment;
  pairingHandle: string;
}

export class QrPairingCoordinator {
  static isSupported(): boolean {
    return canIUse('SystemCapability.Multimedia.Scan.Core') &&
      canIUse('SystemCapability.Multimedia.Scan.ScanBarcode');
  }

  static async scan(context: common.Context, expectedEnvironment: RemoteEnvironment): Promise<PairingQrPayload> {
    const options: scanBarcode.ScanOptions = {
      scanTypes: [scanCore.ScanType.QR_CODE],
      enableMultiMode: false,
      enableAlbum: true
    };
    let result: scanBarcode.ScanResult;
    try {
      result = await scanBarcode.startScanForResult(context, options);
    } catch (error) {
      throw new Error('扫码已取消或系统扫码暂不可用');
    }
    return QrPairingCoordinator.parse(result.originalValue, expectedEnvironment);
  }

  static parse(value: string, expectedEnvironment: RemoteEnvironment): PairingQrPayload {
    if (value.length === 0 || value.length > 512) {
      throw new Error('二维码无效：载荷为空或超过长度限制');
    }

    let payload: PairingQrPayload;
    try {
      payload = JSON.parse(value) as PairingQrPayload;
    } catch (error) {
      throw new Error('二维码无效：不是合法的绑定载荷');
    }
    const keys: string[] = Object.keys(payload).sort();
    if (keys.length !== 3 ||
      keys[0] !== 'environment' ||
      keys[1] !== 'pairingHandle' ||
      keys[2] !== 'pairingQrVersion') {
      throw new Error('二维码无效：字段不完整或包含未允许字段');
    }
    if (payload.pairingQrVersion !== '2' ||
      payload.environment !== expectedEnvironment ||
      !/^[A-Za-z0-9_-]{16,128}$/.test(payload.pairingHandle)) {
      throw new Error('二维码无效：版本、环境或一次性句柄不匹配');
    }
    return payload;
  }
}
