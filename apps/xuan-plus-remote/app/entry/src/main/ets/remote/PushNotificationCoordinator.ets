import { common, UIAbility, Want, wantAgent } from '@kit.AbilityKit';
import { notificationManager } from '@kit.NotificationKit';
import { pushCommon, pushService } from '@kit.PushKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { DeviceIdentityCoordinator } from './DeviceIdentityCoordinator';

const DOMAIN = 0x5741;
const TAG = 'XuanPlusRemote';
const MESSAGE_TYPE = 'task-refresh';
const OPAQUE_ID_PATTERN = /^[A-Za-z0-9_-]{16,128}$/;
const BUNDLE_NAME = 'com.dyys.workagents.remote.dev';

interface RefreshMessage {
  messageType: string;
  refreshRef: string;
}

type RefreshHandler = (refreshRef: string) => Promise<boolean>;
type NotificationPermissionHandler = (enabled: boolean, checkFailed: boolean) => void;

export class PushNotificationCoordinator {
  private static context?: common.UIAbilityContext;
  private static pendingRefreshRefs: string[] = [];
  private static refreshHandler?: RefreshHandler;
  private static initialized: boolean = false;
  private static flushing: boolean = false;
  private static foreground: boolean = false;
  private static retryAttempt: number = 0;
  private static retryTimer: number = -1;
  private static notificationPermissionHandler?: NotificationPermissionHandler;
  private static notificationEnabled?: boolean;

  static initialize(ability: UIAbility): void {
    const context: common.UIAbilityContext = ability.context;
    PushNotificationCoordinator.context = context;
    if (PushNotificationCoordinator.initialized) {
      return;
    }
    try {
      pushService.receiveMessage('DEFAULT', ability, (message: pushCommon.PushPayload) => {
        void PushNotificationCoordinator.handlePushMessage(message);
      });
      PushNotificationCoordinator.initialized = true;
      hilog.info(DOMAIN, TAG, '%{public}s', 'Notification push receiver registered');
    } catch (_error) {
      hilog.warn(DOMAIN, TAG, '%{public}s', 'Push message receiver registration failed');
    }
    void PushNotificationCoordinator.refreshNotificationPermission();
  }

  static setRefreshHandler(handler: RefreshHandler): void {
    PushNotificationCoordinator.refreshHandler = handler;
    void PushNotificationCoordinator.flushPendingRefresh();
  }

  static setNotificationPermissionHandler(handler: NotificationPermissionHandler): void {
    PushNotificationCoordinator.notificationPermissionHandler = handler;
    if (PushNotificationCoordinator.notificationEnabled !== undefined) {
      handler(PushNotificationCoordinator.notificationEnabled, false);
    }
  }

  static async refreshNotificationPermission(): Promise<boolean> {
    try {
      const enabled: boolean = await notificationManager.isNotificationEnabled();
      PushNotificationCoordinator.notificationEnabled = enabled;
      PushNotificationCoordinator.publishNotificationPermission(enabled, false);
      return enabled;
    } catch (_error) {
      PushNotificationCoordinator.publishNotificationPermission(false, true);
      return false;
    }
  }

  static async requestNotificationPermission(): Promise<boolean> {
    const context = PushNotificationCoordinator.context;
    if (context === undefined) {
      PushNotificationCoordinator.publishNotificationPermission(false, true);
      return false;
    }
    try {
      await notificationManager.requestEnableNotification(context);
      return await PushNotificationCoordinator.refreshNotificationPermission();
    } catch (_error) {
      PushNotificationCoordinator.publishNotificationPermission(false, true);
      return false;
    }
  }

  static retryPendingRefresh(): void {
    void PushNotificationCoordinator.flushPendingRefresh();
  }

  static setForeground(foreground: boolean): void {
    PushNotificationCoordinator.foreground = foreground;
    if (foreground) {
      void PushNotificationCoordinator.flushPendingRefresh();
    } else if (PushNotificationCoordinator.retryTimer >= 0) {
      clearTimeout(PushNotificationCoordinator.retryTimer);
      PushNotificationCoordinator.retryTimer = -1;
    }
  }

  static handleWant(want: Want): void {
    const parameters = want.parameters;
    if (parameters === undefined) {
      return;
    }
    const directMessage: RefreshMessage | undefined = PushNotificationCoordinator.validateRefreshMessage({
      messageType: String(parameters['messageType'] ?? ''),
      refreshRef: String(parameters['refreshRef'] ?? '')
    });
    const extraData = parameters['extraData'];
    const extraMessage: RefreshMessage | undefined = directMessage === undefined && typeof extraData === 'string' ?
      PushNotificationCoordinator.parseRefreshMessage(extraData) : undefined;
    const refreshMessage: RefreshMessage | undefined = directMessage ?? extraMessage;
    if (refreshMessage === undefined) {
      return;
    }
    PushNotificationCoordinator.enqueueRefreshRef(refreshMessage.refreshRef);
    void PushNotificationCoordinator.flushPendingRefresh();
  }

  private static async handlePushMessage(message: pushCommon.PushPayload): Promise<void> {
    const parsed: RefreshMessage | undefined = PushNotificationCoordinator.parseRefreshMessage(message.data);
    if (parsed === undefined) {
      hilog.warn(DOMAIN, TAG, '%{public}s', 'Unrecognized push message ignored');
      return;
    }
    if (PushNotificationCoordinator.foreground) {
      PushNotificationCoordinator.enqueueRefreshRef(parsed.refreshRef);
      await PushNotificationCoordinator.flushPendingRefresh();
    } else {
      await PushNotificationCoordinator.publishNotification(parsed.refreshRef);
    }
  }

  private static parseRefreshMessage(rawData: string): RefreshMessage | undefined {
    try {
      const parsed = JSON.parse(rawData) as RefreshMessage;
      if (parsed === null || typeof parsed !== 'object' || Object.keys(parsed).length !== 2) {
        return undefined;
      }
      return PushNotificationCoordinator.validateRefreshMessage(parsed);
    } catch (_error) {
      return undefined;
    }
  }

  private static validateRefreshMessage(message: RefreshMessage): RefreshMessage | undefined {
    if (message.messageType !== MESSAGE_TYPE || !OPAQUE_ID_PATTERN.test(message.refreshRef)) {
      return undefined;
    }
    return message;
  }

  private static async publishNotification(refreshRef: string): Promise<void> {
    const context = PushNotificationCoordinator.context;
    if (context === undefined) {
      return;
    }
    try {
      const agentInfo: wantAgent.WantAgentInfo = {
        wants: [{
          deviceId: '',
          bundleName: BUNDLE_NAME,
          abilityName: 'EntryAbility',
          parameters: { messageType: MESSAGE_TYPE, refreshRef: refreshRef }
        }],
        operationType: wantAgent.OperationType.START_ABILITY,
        requestCode: 1,
        wantAgentFlags: [wantAgent.WantAgentFlags.UPDATE_PRESENT_FLAG]
      };
      const agent = await wantAgent.getWantAgent(agentInfo);
      await notificationManager.publish({
        id: 1001,
        content: {
          notificationContentType: notificationManager.ContentType.NOTIFICATION_CONTENT_BASIC_TEXT,
          normal: {
            title: '轩++远程',
            text: '电脑上的任务已结束，点击查看最新状态'
          }
        },
        wantAgent: agent
      });
      hilog.info(DOMAIN, TAG, '%{public}s', 'Task refresh notification published');
    } catch (_error) {
      hilog.warn(DOMAIN, TAG, '%{public}s', 'Unable to publish task refresh notification');
    }
  }

  private static publishNotificationPermission(enabled: boolean, checkFailed: boolean): void {
    PushNotificationCoordinator.notificationPermissionHandler?.(enabled, checkFailed);
  }

  private static enqueueRefreshRef(refreshRef: string): void {
    if (PushNotificationCoordinator.pendingRefreshRefs.includes(refreshRef)) {
      return;
    }
    if (PushNotificationCoordinator.pendingRefreshRefs.length >= 32) {
      PushNotificationCoordinator.pendingRefreshRefs = [];
    }
    PushNotificationCoordinator.pendingRefreshRefs.push(refreshRef);
  }

  private static async flushPendingRefresh(): Promise<void> {
    const handler = PushNotificationCoordinator.refreshHandler;
    if (PushNotificationCoordinator.pendingRefreshRefs.length === 0 || handler === undefined ||
      PushNotificationCoordinator.flushing || !PushNotificationCoordinator.foreground) {
      return;
    }
    PushNotificationCoordinator.flushing = true;
    try {
      await DeviceIdentityCoordinator.ensureDeviceKey();
      while (PushNotificationCoordinator.foreground && PushNotificationCoordinator.pendingRefreshRefs.length > 0) {
        const refreshRef: string = PushNotificationCoordinator.pendingRefreshRefs[0];
        const handled: boolean = await handler(refreshRef);
        if (!handled) {
          hilog.info(DOMAIN, TAG, '%{public}s', 'Task refresh notification remains pending');
          PushNotificationCoordinator.scheduleRetry();
          break;
        }
        if (PushNotificationCoordinator.pendingRefreshRefs[0] === refreshRef) {
          PushNotificationCoordinator.pendingRefreshRefs.shift();
        } else {
          PushNotificationCoordinator.pendingRefreshRefs =
            PushNotificationCoordinator.pendingRefreshRefs.filter((value: string) => value !== refreshRef);
        }
        hilog.info(DOMAIN, TAG, '%{public}s', 'Task refresh notification handled');
      }
    } catch (_error) {
      hilog.warn(DOMAIN, TAG, '%{public}s', 'Task refresh notification deferred');
      PushNotificationCoordinator.scheduleRetry();
    } finally {
      PushNotificationCoordinator.flushing = false;
      if (PushNotificationCoordinator.pendingRefreshRefs.length === 0) {
        PushNotificationCoordinator.retryAttempt = 0;
      }
    }
  }

  private static scheduleRetry(): void {
    if (!PushNotificationCoordinator.foreground || PushNotificationCoordinator.retryTimer >= 0 ||
      PushNotificationCoordinator.retryAttempt >= 3) {
      return;
    }
    const delays: number[] = [1000, 3000, 10000];
    const delay: number = delays[PushNotificationCoordinator.retryAttempt];
    PushNotificationCoordinator.retryAttempt++;
    PushNotificationCoordinator.retryTimer = setTimeout(() => {
      PushNotificationCoordinator.retryTimer = -1;
      void PushNotificationCoordinator.flushPendingRefresh();
    }, delay);
  }
}
