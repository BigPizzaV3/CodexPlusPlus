import { http } from '@kit.NetworkKit';
import { cryptoFramework } from '@kit.CryptoArchitectureKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { util } from '@kit.ArkTS';
import { DeviceIdentityCoordinator } from './DeviceIdentityCoordinator';
import { RemoteEnvironmentConfig } from './RemoteEnvironmentConfig';

interface CloudErrorResponse {
  errorCode?: string;
  retryable?: boolean;
  serverReceivedAt?: string;
}

export interface SignedRequestHeaders {
  Accept: string;
  'X-WorkAgents-Device-Key-Id': string;
  'X-WorkAgents-Device-Timestamp': string;
  'X-WorkAgents-Device-Nonce': string;
  'X-WorkAgents-Device-Signature': string;
}

const EMPTY_SHA256_HEX = 'e3b0c44298fc1c149afbf4c8996fb924' +
  '27ae41e4649b934ca495991b7852b855';
const LOG_DOMAIN = 0xD002021;
const LOG_TAG = 'RemoteApi';

export class RemoteFailureNotice {
  title: string;
  reason: string;
  status: string;
  action: string;
  retryable: boolean;

  constructor(title: string, reason: string, status: string, action: string, retryable: boolean) {
    this.title = title;
    this.reason = reason;
    this.status = status;
    this.action = action;
    this.retryable = retryable;
  }
}

export class RemoteRequestError extends Error {
  failure: RemoteFailureNotice;

  constructor(failure: RemoteFailureNotice) {
    super(failure.reason);
    this.name = 'RemoteRequestError';
    this.failure = failure;
  }
}

export class RemoteApiClient {
  static async publicPost(path: string, body: Object): Promise<Object> {
    RemoteEnvironmentConfig.assertReady();
    if (!/^\/v1\/[A-Za-z0-9_\/-]{1,180}$/.test(path)) {
      throw new Error('云端登记请求参数无效');
    }
    const request: http.HttpRequest = http.createHttp();
    try {
      let response: http.HttpResponse;
      try {
        response = await request.request(
          `${RemoteEnvironmentConfig.apiBaseUrl}${path}`,
          {
            method: http.RequestMethod.POST,
            header: {
              'Content-Type': 'application/json',
              'Accept': 'application/json'
            },
            extraData: JSON.stringify(body),
            expectDataType: http.HttpDataType.STRING,
            usingProxy: false,
            connectTimeout: 10000,
            readTimeout: 15000
          });
      } catch (_error) {
        throw RemoteApiClient.networkFailure();
      }
      return RemoteApiClient.parseResponse(response, path);
    } finally {
      request.destroy();
    }
  }

  static async signedPost(path: string, body: Object, deviceKeyId: string): Promise<Object> {
    RemoteEnvironmentConfig.assertReady();
    if (!/^\/v1\/[A-Za-z0-9_\/-]{1,180}$/.test(path) ||
      !/^[A-Za-z0-9_-]{16,128}$/.test(deviceKeyId)) {
      throw new Error('设备签名请求参数无效');
    }
    const bodyText: string = JSON.stringify(body);
    const timestamp: string = new Date().toISOString();
    const nonce: string = RemoteApiClient.randomOpaqueId();
    const bodyDigest: string = await RemoteApiClient.sha256Hex(
      new util.TextEncoder().encodeInto(bodyText));
    const proof: string = [
      'workagents-device-request-v1',
      'POST',
      path,
      RemoteEnvironmentConfig.environment,
      timestamp,
      nonce,
      bodyDigest
    ].join('\n');
    const signature: Uint8Array = await DeviceIdentityCoordinator.sign(
      new util.TextEncoder().encodeInto(proof));
    const request: http.HttpRequest = http.createHttp();
    try {
      let response: http.HttpResponse;
      try {
        response = await request.request(
          `${RemoteEnvironmentConfig.apiBaseUrl}${path}`,
          {
            method: http.RequestMethod.POST,
            header: {
              'Content-Type': 'application/json',
              'Accept': 'application/json',
              'X-WorkAgents-Device-Key-Id': deviceKeyId,
              'X-WorkAgents-Device-Timestamp': timestamp,
              'X-WorkAgents-Device-Nonce': nonce,
              'X-WorkAgents-Device-Signature': RemoteApiClient.base64Url(signature)
            },
            extraData: bodyText,
            expectDataType: http.HttpDataType.STRING,
            usingProxy: false,
            connectTimeout: 10000,
            readTimeout: 15000
          });
      } catch (_error) {
        throw RemoteApiClient.networkFailure();
      }
      return RemoteApiClient.parseResponse(response, path);
    } finally {
      request.destroy();
    }
  }

  static async signedGet(pathAndQuery: string, deviceKeyId: string): Promise<Object> {
    RemoteEnvironmentConfig.assertReady();
    if (!/^\/v1\/[A-Za-z0-9_\/-]{1,180}\?[A-Za-z0-9_.~%=&:+\/-]{1,900}$/.test(pathAndQuery) ||
      !/^[A-Za-z0-9_-]{16,128}$/.test(deviceKeyId)) {
      throw new Error('设备签名查询参数无效');
    }
    const headers: SignedRequestHeaders = await RemoteApiClient.signedGetHeaders(pathAndQuery, deviceKeyId);
    const request: http.HttpRequest = http.createHttp();
    try {
      let response: http.HttpResponse;
      try {
        response = await request.request(
          `${RemoteEnvironmentConfig.apiBaseUrl}${pathAndQuery}`,
          {
            method: http.RequestMethod.GET,
            header: headers,
            expectDataType: http.HttpDataType.STRING,
            usingProxy: false,
            connectTimeout: 10000,
            readTimeout: 15000
          });
      } catch (_error) {
        throw RemoteApiClient.networkFailure();
      }
      return RemoteApiClient.parseResponse(response, pathAndQuery);
    } finally {
      request.destroy();
    }
  }

  static async signedGetHeaders(pathAndQuery: string, deviceKeyId: string): Promise<SignedRequestHeaders> {
    RemoteEnvironmentConfig.assertReady();
    if (!/^\/v1\/[A-Za-z0-9_\/-]{1,180}\?[A-Za-z0-9_.~%=&:+\/-]{1,900}$/.test(pathAndQuery) ||
      !/^[A-Za-z0-9_-]{16,128}$/.test(deviceKeyId)) {
      throw new Error('设备签名查询参数无效');
    }
    const timestamp: string = new Date().toISOString();
    const nonce: string = RemoteApiClient.randomOpaqueId();
    const bodyDigest: string = await RemoteApiClient.sha256Hex(new Uint8Array(0));
    const proof: string = [
      'workagents-device-request-v1',
      'GET',
      pathAndQuery,
      RemoteEnvironmentConfig.environment,
      timestamp,
      nonce,
      bodyDigest
    ].join('\n');
    const signature: Uint8Array = await DeviceIdentityCoordinator.sign(
      new util.TextEncoder().encodeInto(proof));
    return {
      'Accept': 'application/json',
      'X-WorkAgents-Device-Key-Id': deviceKeyId,
      'X-WorkAgents-Device-Timestamp': timestamp,
      'X-WorkAgents-Device-Nonce': nonce,
      'X-WorkAgents-Device-Signature': RemoteApiClient.base64Url(signature)
    };
  }

  static randomOpaqueId(): string {
    try {
      const random = cryptoFramework.createRandom().generateRandomSync(24);
      return RemoteApiClient.base64Url(random.data);
    } catch (_error) {
      throw new Error('安全随机标识生成失败');
    }
  }

  static describeFailure(error: Object, fallbackReason: string, fallbackAction: string): RemoteFailureNotice {
    if (error instanceof RemoteRequestError) {
      return error.failure;
    }
    const reason: string = RemoteApiClient.safeLocalReason(error, fallbackReason);
    return new RemoteFailureNotice(
      '操作未完成', reason, '手机端未收到可确认的完成状态', fallbackAction, false);
  }

  private static parseResponse(response: http.HttpResponse, path: string): Object {
    if (response.responseCode < 200 || response.responseCode >= 300) {
      throw RemoteApiClient.httpFailure(response.responseCode, response.result, path);
    }
    if (typeof response.result !== 'string') {
      throw RemoteApiClient.protocolFailure(response.responseCode, '云端响应格式无法验证');
    }
    try {
      return JSON.parse(response.result) as Object;
    } catch (_error) {
      throw RemoteApiClient.protocolFailure(response.responseCode, '云端响应不是有效 JSON');
    }
  }

  private static httpFailure(statusCode: number, result: Object | string, path: string): RemoteRequestError {
    let errorCode: string = '';
    let serverRetryable: boolean | undefined = undefined;
    if (typeof result === 'string') {
      try {
        const cloudError: CloudErrorResponse = JSON.parse(result) as CloudErrorResponse;
        if (typeof cloudError.errorCode === 'string' && /^[a-z0-9_]{3,64}$/.test(cloudError.errorCode)) {
          errorCode = cloudError.errorCode;
        }
        if (typeof cloudError.retryable === 'boolean') {
          serverRetryable = cloudError.retryable;
        }
      } catch (_error) {
        errorCode = '';
      }
    }
    hilog.warn(LOG_DOMAIN, LOG_TAG, '%{public}s', JSON.stringify({
      event: 'http_failure',
      route: RemoteApiClient.safeRouteName(path),
      statusCode: statusCode,
      errorCode: RemoteApiClient.publicErrorCode(errorCode)
    }));
    return new RemoteRequestError(
      RemoteApiClient.failureForStatus(errorCode, statusCode, serverRetryable));
  }

  private static safeRouteName(path: string): string {
    if (/^\/v1\/tasks\/[A-Za-z0-9_-]+\/commands$/.test(path)) {
      return 'task-command';
    }
    if (/^\/v1\/commands\/[A-Za-z0-9_-]+\?/.test(path)) {
      return 'command-query';
    }
    if (path.includes('/tasks')) {
      return 'task-sync';
    }
    if (path.includes('/bindings') || path.includes('/pairing')) {
      return 'binding';
    }
    return 'other';
  }

  private static failureForStatus(errorCode: string, statusCode: number,
    serverRetryable?: boolean): RemoteFailureNotice {
    const retryable: boolean = serverRetryable === undefined ?
      statusCode === 429 || statusCode >= 500 : serverRetryable;
    const codeStatus: string = RemoteApiClient.publicErrorCode(errorCode);
    const status: string = `HTTP ${statusCode}${codeStatus.length > 0 ? ` · ${codeStatus}` : ''} · ` +
      `${retryable ? '可安全重试' : '请按提示处理'}`;

    if (errorCode === 'invalid_request' || errorCode === 'unsupported_operation') {
      return new RemoteFailureNotice('请求校验失败',
        errorCode === 'unsupported_operation' ? '云端不支持当前客户端请求的操作。' :
          '云端已收到请求，但请求字段、格式或协议版本未通过校验。',
        status, '请确认手机端与开发云端均已发布协议 2.0；修正后重新提交。', false);
    }
    if (errorCode === 'environment_mismatch') {
      return new RemoteFailureNotice('环境不匹配', '手机端连接的云端环境与请求环境不一致。',
        status, '请使用同一开发环境的手机端、电脑端和云端服务后重试。', false);
    }
    if (errorCode === 'device_key_invalid' || errorCode === 'device_proof_invalid' ||
      errorCode === 'device_authentication_failed') {
      return new RemoteFailureNotice('设备鉴权失败', '云端未能验证当前手机的设备身份或持钥证明。',
        status, '请重新连接云端完成设备登记；持续失败时检查应用签名与云端版本。', false);
    }
    if (errorCode === 'device_not_bound') {
      return new RemoteFailureNotice('绑定鉴权已失效', '云端确认当前手机与电脑之间没有有效绑定。',
        status, '请在电脑端生成新二维码，并在手机上重新完成绑定。', false);
    }
    if (errorCode === 'device_challenge_expired') {
      return new RemoteFailureNotice('设备登记挑战已过期', '设备登记未在云端挑战有效期内完成。',
        status, '重新连接开发云端即可获取新的登记挑战。', false);
    }
    if (errorCode === 'device_request_replayed') {
      return new RemoteFailureNotice('签名请求已处理', '云端拒绝重复使用同一次设备签名请求。',
        status, '直接重新执行当前操作，手机会生成新的签名请求。', false);
    }
    if (errorCode === 'pairing_expired' || errorCode === 'pairing_confirmation_expired') {
      return new RemoteFailureNotice('配对已过期', '本次二维码或电脑确认窗口已经结束。',
        status, '请在电脑端生成最新二维码后重新扫描。', false);
    }
    if (errorCode === 'pairing_replayed') {
      return new RemoteFailureNotice('二维码已使用', '该一次性配对二维码已经被消费，不能再次绑定。',
        status, '请在电脑端生成新的二维码后重新扫描。', false);
    }
    if (errorCode === 'invalid_pairing_qr' || errorCode === 'pairing_code_invalid' ||
      errorCode === 'pairing_summary_mismatch') {
      return new RemoteFailureNotice('配对校验失败', '二维码无效，或电脑与手机核对的绑定摘要不一致。',
        status, '请确认两端处于同一环境，并扫描电脑端最新生成的二维码。', false);
    }
    if (errorCode === 'device_limit_reached') {
      return new RemoteFailureNotice('设备数量已达上限', '云端无法继续登记新的手机设备。',
        status, '请清理停用设备；若此前尚未发布该能力，请先更新开发云端。', false);
    }
    if (errorCode === 'pc_offline') {
      return new RemoteFailureNotice('电脑当前离线', '云端已收到请求，但目标电脑当前无法接收远程命令。',
        status, '启动电脑端轩++远程并恢复远程连接后原位重试。', true);
    }
    if (errorCode === 'state_conflict') {
      return new RemoteFailureNotice('任务状态已变化', '提交命令时使用的任务状态已不是最新版本。',
        status, '刷新任务状态后重新提交，当前输入会保留。', false);
    }
    if (errorCode === 'invalid_command_state') {
      return new RemoteFailureNotice('当前状态不允许此操作', '目标任务的当前状态不接受这类远程命令。',
        status, '刷新任务状态，并选择当前可用的操作。', false);
    }
    if (errorCode === 'payload_digest_conflict') {
      return new RemoteFailureNotice('请求内容发生冲突', '同一次命令重试携带了与原请求不同的内容。',
        status, '返回后重新发起一次新命令，不要复用旧请求。', false);
    }
    if (errorCode === 'command_expired') {
      return new RemoteFailureNotice('远程命令已过期', '电脑未能在命令有效期内完成处理。',
        status, '确认电脑在线后重新提交当前操作。', false);
    }
    if (errorCode === 'not_found') {
      return new RemoteFailureNotice('目标已不存在', '云端找不到当前任务、命令或绑定记录。',
        status, '刷新当前页面；若绑定已失效，请重新扫码绑定。', false);
    }
    if (errorCode === 'storage_unavailable') {
      return new RemoteFailureNotice('云端服务暂不可用', '云端存储当前无法完成请求。',
        status, '稍后原位重试；持续失败时检查开发云端服务状态。', true);
    }
    if (statusCode === 404) {
      return new RemoteFailureNotice('服务尚未发布或版本不匹配',
        '当前云端没有客户端所请求的接口，常见原因是开发云端尚未发布对应版本。',
        status, '请发布协议 2.0 云端并确认公开健康检查通过，然后重新连接。', false);
    }
    if (statusCode === 401 || statusCode === 403) {
      return new RemoteFailureNotice('云端鉴权失败', '云端拒绝了当前请求的身份或环境证明。',
        status, '重新登记手机设备或重新扫码绑定后再试。', false);
    }
    if (statusCode === 400 || statusCode === 422) {
      return new RemoteFailureNotice('请求校验失败', '云端已响应，但没有返回可识别的公开错误代码。',
        status, '确认手机端与云端协议版本一致；持续失败时由开发人员核对请求契约。', false);
    }
    if (statusCode === 409 || statusCode === 410) {
      return new RemoteFailureNotice('请求状态已失效', '云端拒绝了过期、重复或与当前状态冲突的请求。',
        status, '刷新当前状态后重新执行操作。', false);
    }
    if (statusCode === 429) {
      return new RemoteFailureNotice('请求过于频繁', '云端正在限制当前设备的请求频率。',
        status, '稍候片刻再原位重试。', true);
    }
    if (statusCode >= 500) {
      return new RemoteFailureNotice('云端服务暂不可用', '云端网关或服务当前未能完成请求。',
        status, '稍后原位重试；持续失败时检查服务发布与运行状态。', true);
    }
    return new RemoteFailureNotice('云端请求未完成', '云端拒绝了请求，但未返回可识别的公开失败原因。',
      status, '刷新状态后重试；持续失败时检查手机端与云端协议版本。', retryable);
  }

  private static publicErrorCode(errorCode: string): string {
    const allowed: string[] = [
      'invalid_request', 'environment_mismatch', 'device_key_invalid', 'device_proof_invalid',
      'device_challenge_expired', 'device_authentication_failed', 'device_request_replayed',
      'pairing_expired', 'pairing_replayed', 'invalid_pairing_qr', 'pairing_code_invalid',
      'pairing_confirmation_expired', 'pairing_summary_mismatch', 'device_limit_reached',
      'device_not_bound', 'pc_offline', 'state_conflict', 'payload_digest_conflict',
      'command_expired', 'unsupported_operation', 'invalid_command_state', 'not_found',
      'storage_unavailable'
    ];
    return allowed.includes(errorCode) ? errorCode : '';
  }

  private static networkFailure(): RemoteRequestError {
    return new RemoteRequestError(new RemoteFailureNotice(
      '无法连接云端服务', '手机未能与远程开发云端建立连接，可能是网络中断、超时或服务未运行。',
      '未收到 HTTP 响应 · 可安全重试', '检查手机网络和开发云端服务状态后原位重试。', true));
  }

  private static protocolFailure(statusCode: number, reason: string): RemoteRequestError {
    return new RemoteRequestError(new RemoteFailureNotice(
      '云端响应无法验证', reason, `HTTP ${statusCode} · 响应格式异常 · 请勿原样继续`,
      '请确认开发云端已发布协议 2.0；持续出现时联系开发人员。', false));
  }

  private static safeLocalReason(error: Object, fallbackReason: string): string {
    if (!(error instanceof Error)) {
      return fallbackReason;
    }
    const message: string = error.message.trim();
    const lowered: string = message.toLowerCase();
    const sensitive: boolean = lowered.includes('http://') || lowered.includes('https://') ||
      lowered.includes('authorization') || lowered.includes('cookie') || lowered.includes('signature=') ||
      lowered.includes('token=') || lowered.includes('nonce=') || lowered.includes('device_id') ||
      lowered.includes('binding_id') || lowered.includes('task_id') || lowered.includes('sql') ||
      /^[A-Za-z]:\\/.test(message) || message.length > 180;
    return message.length > 0 && !sensitive ? message : fallbackReason;
  }

  private static async sha256Hex(value: Uint8Array): Promise<string> {
    if (value.length === 0) {
      return EMPTY_SHA256_HEX;
    }
    try {
      const digest = cryptoFramework.createMd('SHA256');
      await digest.update({ data: value });
      const result: cryptoFramework.DataBlob = await digest.digest();
      let hex: string = '';
      for (let index = 0; index < result.data.length; index++) {
        hex += result.data[index].toString(16).padStart(2, '0');
      }
      return hex;
    } catch (_error) {
      throw new Error('设备请求摘要计算失败');
    }
  }

  private static base64Url(value: Uint8Array): string {
    return new util.Base64Helper()
      .encodeToStringSync(value, util.Type.BASIC_URL_SAFE)
      .replace(/=+$/, '');
  }
}
