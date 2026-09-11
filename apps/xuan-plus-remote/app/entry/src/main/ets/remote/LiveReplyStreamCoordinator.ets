import { webSocket } from '@kit.NetworkKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { DeviceRegistrationResult } from './DeviceRegistrationCoordinator';
import { RemoteApiClient, SignedRequestHeaders } from './RemoteApiClient';
import { RemoteEnvironmentConfig } from './RemoteEnvironmentConfig';
import { utf8ByteLength } from './RemoteModels';
import { RemoteTaskSnapshot } from './TaskSyncCoordinator';

const CONTRACT_VERSION: string = '1.5';
const OPAQUE_ID_PATTERN: RegExp = /^[A-Za-z0-9_-]{16,128}$/;
const LOG_DOMAIN: number = 0x5741;
const LOG_TAG: string = 'XuanPlusRemote';

export interface LiveReplyFrame {
  schemaVersion: string;
  messageType: string;
  messageId: string;
  environment: string;
  pcDeviceId: string;
  installationId: string;
  bindingEpoch: number;
  remoteTaskId: string;
  streamId: string;
  streamSeq: number;
  sentAt: string;
  serverReceivedAt: string;
  text?: string;
  outcome?: string;
}

export interface LiveReplyUpdate {
  text: string;
  streamId: string;
  streamSeq: number;
  observedAt: string;
}

type UpdateHandler = (update: LiveReplyUpdate) => void;
type EndHandler = (outcome: string) => void;
type StatusHandler = (connected: boolean, notice: string) => void;

export class LiveReplyStreamCoordinator {
  private static socket?: webSocket.WebSocket;
  private static generation: number = 0;
  private static streamId: string = '';
  private static streamSeq: number = 0;
  private static text: string = '';
  private static streamStartedAt: number = 0;
  private static retiredStreamIds: string[] = [];
  private static connectTimer?: number = undefined;

  static async connect(registration: DeviceRegistrationResult, task: RemoteTaskSnapshot,
    onUpdate: UpdateHandler, onEnd: EndHandler, onStatus: StatusHandler): Promise<void> {
    LiveReplyStreamCoordinator.disconnect();
    if (!OPAQUE_ID_PATTERN.test(task.remoteTaskId) ||
      !OPAQUE_ID_PATTERN.test(registration.appDeviceId) ||
      !OPAQUE_ID_PATTERN.test(registration.deviceKeyId)) {
      LiveReplyStreamCoordinator.log('connect-rejected: invalid local subscription parameters');
      onStatus(false, '实时回复订阅参数无效');
      return;
    }
    const generation: number = ++LiveReplyStreamCoordinator.generation;
    const messageId: string = RemoteApiClient.randomOpaqueId();
    const query: string = [
      `schemaVersion=${encodeURIComponent(CONTRACT_VERSION)}`,
      `messageType=${encodeURIComponent('app/reply-stream-connect')}`,
      `messageId=${encodeURIComponent(messageId)}`,
      `environment=${encodeURIComponent(RemoteEnvironmentConfig.environment)}`,
      `sentAt=${encodeURIComponent(new Date().toISOString())}`,
      `appDeviceId=${encodeURIComponent(registration.appDeviceId)}`
    ].join('&');
    const canonicalPath: string = `/v1/tasks/${task.remoteTaskId}/reply-stream?${query}`;
    let headers: SignedRequestHeaders;
    try {
      headers = await RemoteApiClient.signedGetHeaders(canonicalPath, registration.deviceKeyId);
    } catch (_error) {
      LiveReplyStreamCoordinator.log('connect-rejected: signing failed');
      if (generation === LiveReplyStreamCoordinator.generation) {
        onStatus(false, '无法创建实时回复设备签名');
      }
      return;
    }
    if (generation !== LiveReplyStreamCoordinator.generation) {
      return;
    }
    const socket: webSocket.WebSocket = webSocket.createWebSocket();
    LiveReplyStreamCoordinator.socket = socket;
    LiveReplyStreamCoordinator.streamId = '';
    LiveReplyStreamCoordinator.streamSeq = 0;
    LiveReplyStreamCoordinator.text = '';
    LiveReplyStreamCoordinator.streamStartedAt = 0;
    socket.on('open', (_error: BusinessError): void => {
      if (generation === LiveReplyStreamCoordinator.generation) {
        if (LiveReplyStreamCoordinator.connectTimer !== undefined) {
          clearTimeout(LiveReplyStreamCoordinator.connectTimer);
          LiveReplyStreamCoordinator.connectTimer = undefined;
        }
        LiveReplyStreamCoordinator.log('socket-open');
        onStatus(true, '正在接收电脑端实时回复');
      }
    });
    socket.on('message', (error: BusinessError, value: string | ArrayBuffer): void => {
      if (generation !== LiveReplyStreamCoordinator.generation || error.code !== 0 ||
        typeof value !== 'string') {
        return;
      }
      let frame: LiveReplyFrame;
      try {
        frame = JSON.parse(value) as LiveReplyFrame;
      } catch (_error) {
        LiveReplyStreamCoordinator.log('frame-rejected: invalid-json');
        onStatus(false, '实时回复数据无法验证');
        LiveReplyStreamCoordinator.disconnect();
        return;
      }
      if (frame === null || typeof frame !== 'object' || !LiveReplyStreamCoordinator.validFrame(frame, task)) {
        LiveReplyStreamCoordinator.log('frame-rejected: invalid-contract');
        onStatus(false, '实时回复协议校验失败');
        LiveReplyStreamCoordinator.disconnect();
        return;
      }
      // 重放帧不应追加两次；同一流的旧 reset 也不能把已收到的内容回退。
      if (frame.streamId === LiveReplyStreamCoordinator.streamId &&
        frame.streamSeq <= LiveReplyStreamCoordinator.streamSeq) {
        return;
      }
      if (LiveReplyStreamCoordinator.retiredStreamIds.includes(frame.streamId)) {
        return;
      }
      if (frame.messageType === 'reply-stream/reset') {
        if (Date.parse(frame.sentAt) < LiveReplyStreamCoordinator.streamStartedAt) {
          return;
        }
        LiveReplyStreamCoordinator.logFrame('frame-accepted', frame);
        if (frame.streamId !== LiveReplyStreamCoordinator.streamId &&
          LiveReplyStreamCoordinator.streamId.length > 0) {
          LiveReplyStreamCoordinator.retiredStreamIds.push(LiveReplyStreamCoordinator.streamId);
          if (LiveReplyStreamCoordinator.retiredStreamIds.length > 32) {
            LiveReplyStreamCoordinator.retiredStreamIds.shift();
          }
        }
        LiveReplyStreamCoordinator.streamStartedAt = Date.parse(frame.sentAt);
        LiveReplyStreamCoordinator.streamId = frame.streamId;
        LiveReplyStreamCoordinator.streamSeq = frame.streamSeq;
        LiveReplyStreamCoordinator.text = frame.text ?? '';
        onUpdate({
          text: LiveReplyStreamCoordinator.text,
          streamId: frame.streamId,
          streamSeq: frame.streamSeq,
          observedAt: frame.sentAt
        });
        return;
      }
      if (frame.messageType === 'reply-stream/append') {
        if (frame.streamId !== LiveReplyStreamCoordinator.streamId ||
          frame.streamSeq !== LiveReplyStreamCoordinator.streamSeq + 1) {
          LiveReplyStreamCoordinator.logFrame('frame-rejected: sequence-gap', frame);
          onStatus(false, '实时回复出现序列缺口，正在等待重新同步');
          LiveReplyStreamCoordinator.disconnect();
          return;
        }
        const text: string = `${LiveReplyStreamCoordinator.text}${frame.text ?? ''}`;
        LiveReplyStreamCoordinator.text = text;
        LiveReplyStreamCoordinator.streamSeq = frame.streamSeq;
        LiveReplyStreamCoordinator.logFrame('frame-accepted', frame);
        onUpdate({ text: text, streamId: frame.streamId, streamSeq: frame.streamSeq,
          observedAt: frame.sentAt });
        return;
      }
      if (frame.streamId !== LiveReplyStreamCoordinator.streamId ||
        frame.streamSeq !== LiveReplyStreamCoordinator.streamSeq + 1) {
        LiveReplyStreamCoordinator.logFrame('frame-rejected: end-sequence-gap', frame);
        onStatus(false, '实时回复结束帧与当前流不一致，正在等待重新同步');
        LiveReplyStreamCoordinator.disconnect();
        return;
      }
      LiveReplyStreamCoordinator.streamSeq = frame.streamSeq;
      LiveReplyStreamCoordinator.logFrame('frame-accepted', frame);
      onEnd(frame.outcome ?? 'stopped');
    });
    socket.on('close', (_error: BusinessError): void => {
      if (generation === LiveReplyStreamCoordinator.generation) {
        LiveReplyStreamCoordinator.log('socket-close');
        LiveReplyStreamCoordinator.disconnect();
        onStatus(false, '实时回复连接已断开；最终答复仍会通过任务快照同步');
      }
    });
    socket.on('error', (_error: BusinessError): void => {
      if (generation === LiveReplyStreamCoordinator.generation) {
        LiveReplyStreamCoordinator.log('socket-error');
        onStatus(false, '实时回复连接暂不可用；最终答复仍会通过任务快照同步');
        LiveReplyStreamCoordinator.disconnect();
      }
    });
    try {
      LiveReplyStreamCoordinator.connectTimer = setTimeout((): void => {
        if (generation === LiveReplyStreamCoordinator.generation) {
          onStatus(false, '实时连接超时，正在重新连接');
          LiveReplyStreamCoordinator.disconnect();
        }
      }, 15000);
      await socket.connect(
        `${RemoteEnvironmentConfig.replyStreamWssBaseUrl}/${task.remoteTaskId}/reply-stream?${query}`,
        { header: headers });
    } catch (_error) {
      if (generation === LiveReplyStreamCoordinator.generation) {
        LiveReplyStreamCoordinator.log('connect-rejected: socket-connect failed');
        onStatus(false, '无法连接实时回复服务；最终答复仍会通过任务快照同步');
        LiveReplyStreamCoordinator.disconnect();
      }
    }
  }

  static disconnect(): void {
    LiveReplyStreamCoordinator.generation++;
    if (LiveReplyStreamCoordinator.connectTimer !== undefined) {
      clearTimeout(LiveReplyStreamCoordinator.connectTimer);
      LiveReplyStreamCoordinator.connectTimer = undefined;
    }
    const socket: webSocket.WebSocket | undefined = LiveReplyStreamCoordinator.socket;
    LiveReplyStreamCoordinator.socket = undefined;
    LiveReplyStreamCoordinator.streamId = '';
    LiveReplyStreamCoordinator.streamSeq = 0;
    LiveReplyStreamCoordinator.text = '';
    LiveReplyStreamCoordinator.streamStartedAt = 0;
    LiveReplyStreamCoordinator.retiredStreamIds = [];
    if (socket !== undefined) {
      socket.off('open');
      socket.off('message');
      socket.off('close');
      socket.off('error');
      try {
        void socket.close({ code: 1000, reason: 'detail_closed' }).catch((): void => {
          LiveReplyStreamCoordinator.log('socket-close: already unavailable');
        });
      } catch (_error) {
        LiveReplyStreamCoordinator.log('socket-close: already unavailable');
      }
    }
  }

  private static validFrame(frame: LiveReplyFrame, task: RemoteTaskSnapshot): boolean {
    const hasText: boolean = frame.messageType === 'reply-stream/reset' ||
      frame.messageType === 'reply-stream/append';
    const text: string | undefined = frame.text;
    return frame.schemaVersion === CONTRACT_VERSION &&
      (frame.messageType === 'reply-stream/reset' || frame.messageType === 'reply-stream/append' ||
        frame.messageType === 'reply-stream/end') &&
      OPAQUE_ID_PATTERN.test(frame.messageId) && OPAQUE_ID_PATTERN.test(frame.streamId) &&
      frame.environment === RemoteEnvironmentConfig.environment &&
      frame.pcDeviceId === task.pcDeviceId && frame.installationId === task.installationId &&
      frame.bindingEpoch === task.bindingEpoch && frame.remoteTaskId === task.remoteTaskId &&
      Number.isSafeInteger(frame.streamSeq) && frame.streamSeq > 0 &&
      !Number.isNaN(Date.parse(frame.sentAt)) && !Number.isNaN(Date.parse(frame.serverReceivedAt)) &&
      (hasText ? typeof text === 'string' :
        text === undefined && (frame.outcome === 'completed' || frame.outcome === 'failed' ||
          frame.outcome === 'interrupted' || frame.outcome === 'stopped'));
  }

  private static utf8Length(value: string): number {
    return utf8ByteLength(value);
  }

  private static log(message: string): void {
    hilog.info(LOG_DOMAIN, LOG_TAG, '%{public}s', message);
  }

  private static logFrame(stage: string, frame: LiveReplyFrame): void {
    if (stage === 'frame-accepted' && frame.messageType === 'reply-stream/append' &&
      frame.streamSeq !== 2 && frame.streamSeq % 20 !== 0) {
      return;
    }
    const textBytes: number = typeof frame.text === 'string' ?
      LiveReplyStreamCoordinator.utf8Length(frame.text) : 0;
    LiveReplyStreamCoordinator.log(`${stage}: type=${frame.messageType}, seq=${frame.streamSeq}, textBytes=${textBytes}`);
  }
}
