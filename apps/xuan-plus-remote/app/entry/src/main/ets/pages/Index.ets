import { PairingQrPayload, QrPairingCoordinator } from '../remote/QrPairingCoordinator';
import { DeviceRegistrationCoordinator, DeviceRegistrationResult } from '../remote/DeviceRegistrationCoordinator';
import { BindingSummary, ModelConfigSummary, PairingApiCoordinator, PairingPendingResponse,
  PcDeviceListResponse, PcDeviceSummary } from '../remote/PairingApiCoordinator';
import { CommandStatus, PairingState, PcConnectionState, RemoteEnvironment, ReplyState, utf8ByteLength } from '../remote/RemoteModels';
import { RemoteCommandCoordinator, RemoteCommandRecord } from '../remote/RemoteCommandCoordinator';
import { RemoteApiClient, RemoteFailureNotice } from '../remote/RemoteApiClient';
import { RemoteTaskSnapshot, TaskListResponse, TaskSyncCoordinator } from '../remote/TaskSyncCoordinator';
import { PushNotificationCoordinator } from '../remote/PushNotificationCoordinator';
import { PushRefreshCoordinator, PushRefreshTarget } from '../remote/PushRefreshCoordinator';
import { AppLifecycleCoordinator } from '../remote/AppLifecycleCoordinator';
import { LiveReplyStreamCoordinator, LiveReplyUpdate } from '../remote/LiveReplyStreamCoordinator';
import { MarkdownPreview } from '../components/MarkdownPreview';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { KeyboardAvoidMode } from '@kit.ArkUI';

const DOMAIN = 0x5741;
const TAG = 'XuanPlusRemote';
const TASK_REFRESH_INTERVAL_MS = 3000;
const CHINA_TIME_OFFSET_MS: number = 8 * 60 * 60 * 1000;
const FONT_SYSTEM_SANS = 'HarmonyOS Sans';
const COLOR_INK: ResourceColor = $r('app.color.remote_ink');
const COLOR_INK_STRONG: ResourceColor = $r('app.color.remote_ink_strong');
const COLOR_INK_BODY: ResourceColor = $r('app.color.remote_ink_body');
const COLOR_INK_SECONDARY: ResourceColor = $r('app.color.remote_ink_secondary');
const COLOR_MUTED: ResourceColor = $r('app.color.remote_muted');
const COLOR_MUTED_LIGHT: ResourceColor = $r('app.color.remote_muted_light');
const COLOR_MUTED_FAINT: ResourceColor = $r('app.color.remote_muted_faint');
const COLOR_DISABLED: ResourceColor = $r('app.color.remote_disabled');
const COLOR_ACCENT: ResourceColor = $r('app.color.remote_accent');
const COLOR_ACCENT_SOFT: ResourceColor = $r('app.color.remote_accent_soft');
const COLOR_SUCCESS: ResourceColor = $r('app.color.remote_success');
const COLOR_SUCCESS_SOFT: ResourceColor = $r('app.color.remote_success_soft');
const COLOR_WARNING: ResourceColor = $r('app.color.remote_warning');
const COLOR_WARNING_TEXT: ResourceColor = $r('app.color.remote_warning_text');
const COLOR_WARNING_SOFT: ResourceColor = $r('app.color.remote_warning_soft');
const COLOR_DANGER: ResourceColor = $r('app.color.remote_danger');
const COLOR_DANGER_SOFT: ResourceColor = $r('app.color.remote_danger_soft');
const COLOR_RULE: ResourceColor = $r('app.color.remote_rule');
const COLOR_RULE_INPUT: ResourceColor = $r('app.color.remote_rule_input');
const COLOR_SURFACE: ResourceColor = $r('app.color.remote_surface');
const COLOR_SURFACE_SUBDUED: ResourceColor = $r('app.color.remote_surface_subdued');
const COLOR_SURFACE_INPUT: ResourceColor = $r('app.color.remote_surface_input');
const COLOR_SURFACE_CONTROL: ResourceColor = $r('app.color.remote_surface_control');
const COLOR_PAGE: ResourceColor = $r('app.color.remote_page');
const PC_DEVICE_REVOKE_ACTION_WIDTH: number = 112;
const PC_DEVICE_REVOKE_REVEAL_THRESHOLD: number = PC_DEVICE_REVOKE_ACTION_WIDTH / 2;
// The selected binding is local presentation state only. It never becomes a remote fact or command input.
PersistentStorage.persistProp('workagents.remote.activePcDeviceKey', '');

type TaskFilter = 'all' | 'running' | 'attention' | 'ended';
type HomeSection = 'overview' | 'tasks';
type TaskPresentationStatus = 'created' | 'queued' | 'running' | 'paused' | 'stopped' | 'completed' | 'failed' |
  'archived';

function twoDigits(value: number): string {
  return value < 10 ? `0${value}` : `${value}`;
}

function formatChinaDateTime(value: string): string {
  const timestamp: number = Date.parse(value);
  if (Number.isNaN(timestamp)) {
    return '时间不可用';
  }
  const chinaDate: Date = new Date(timestamp + CHINA_TIME_OFFSET_MS);
  return `${chinaDate.getUTCFullYear()}-${twoDigits(chinaDate.getUTCMonth() + 1)}-${twoDigits(chinaDate.getUTCDate())} ` +
    `${twoDigits(chinaDate.getUTCHours())}:${twoDigits(chinaDate.getUTCMinutes())}:${twoDigits(chinaDate.getUTCSeconds())}`;
}

interface WorkspaceChoice {
  name: string;
  sourceTaskId: string;
  taskCount: number;
}

interface TaskWorkspaceGroup {
  name: string;
  tasks: RemoteTaskSnapshot[];
  runningTaskCount: number;
}

interface PcDeviceCardLayout {
  deviceKey: string;
  height: number;
}

@Entry
@Component
struct Index {
  @State private environment: RemoteEnvironment = RemoteEnvironment.DEV;
  @State private connectionState: PcConnectionState = PcConnectionState.OFFLINE;
  @State private pairingState: PairingState = PairingState.UNBOUND;
  @State private scanning: boolean = false;
  @State private registeringDevice: boolean = false;
  @State private deviceRegistered: boolean = false;
  @State private refreshingBinding: boolean = false;
  @State private pcDevices: PcDeviceSummary[] = [];
  @StorageLink('workagents.remote.activePcDeviceKey') private activePcDeviceKey: string = '';
  @State private deviceListExpanded: boolean = false;
  @State private revokingPcDeviceKey: string = '';
  @State private revokeConfirmationPcDeviceKey: string = '';
  @State private swipingPcDeviceKey: string = '';
  @State private pcDeviceSwipeOffset: number = 0;
  @State private revealedPcDeviceKey: string = '';
  @State private pcDeviceCardLayouts: PcDeviceCardLayout[] = [];
  @State private connectionDetailsExpanded: boolean = false;
  @State private bindingRefreshError: string = '';
  @State private pairingError: string = '';
  @State private bindingFailure?: RemoteFailureNotice = undefined;
  @State private pairingFailure?: RemoteFailureNotice = undefined;
  @State private pendingSummary?: BindingSummary = undefined;
  private pendingPcDeviceId?: string = undefined;
  private deviceRegistration?: DeviceRegistrationResult = undefined;
  private bindingMonitorGeneration: number = 0;
  private bindingMonitorTask?: Promise<void> = undefined;
  @State private tasks: RemoteTaskSnapshot[] = [];
  @State private refreshingTasks: boolean = false;
  @State private loadingMoreTasks: boolean = false;
  @State private nextTaskCursor: string | null = null;
  @State private taskSearchQuery: string = '';
  @State private taskFilter: TaskFilter = 'all';
  @State private expandedTaskWorkspaces: string[] = [];
  @State private lastTaskSyncAt: string = '';
  @State private taskNotice: string = '绑定电脑后将自动同步任务';
  @State private taskFailure?: RemoteFailureNotice = undefined;
  @State private activePcDevice?: PcDeviceSummary = undefined;
  private taskVisibilityGeneration: number = 0;
  private taskRefreshPending: boolean = false;
  private taskCursorsSeen: string[] = [];
  private taskSyncGeneration: number = 0;
  private taskSyncTimer: number = -1;
  private taskDetailGeneration: number = 0;
  private refreshingSelectedTask: boolean = false;
  private selectedTaskRefreshPending: boolean = false;
  private pageMounted: boolean = true;
  private replyScroller: Scroller = new Scroller();
  private replyFollowBottom: boolean = true;
  private replyScrollTimer?: number = undefined;
  private previousKeyboardAvoidMode?: KeyboardAvoidMode = undefined;
  @State private selectedTaskId: string = '';
  @State private activePage: string = 'home';
  @State private homeSection: HomeSection = 'overview';
  @State private viewportWidth: number = 375;
  @State private selectedWorkspaceSourceTaskId: string = '';
  @State private selectedModelConfigId: string = '';
  @State private newTaskInstruction: string = '';
  @State private createNotice: string = '先选择工作区，再编写第一条指令';
  @State private createFailure?: RemoteFailureNotice = undefined;
  @State private instruction: string = '';
  @State private commandSubmitting: boolean = false;
  @State private commandNotice: string = '选择任务后可发送消息或停止任务';
  @State private commandFailure?: RemoteFailureNotice = undefined;
  @State private liveReplyText: string = '';
  @State private liveReplyActive: boolean = false;
  @State private liveReplyAwaitingFinalSnapshot: boolean = false;
  @State private liveReplyConnected: boolean = false;
  @State private liveReplyNotice: string = '';
  private pendingLiveReplyText: string = '';
  private liveReplyRenderTimer?: number = undefined;
  private liveReplyReconnectTimer?: number = undefined;
  private liveReplyGeneration: number = 0;
  private liveReplyReconnectAttempt: number = 0;
  private liveReplyObservedAt: string = '';
  @State private notice: string = '请先在电脑端开启远程控制并完成绑定';
  @State private registrationError: string = '';
  @State private registrationFailure?: RemoteFailureNotice = undefined;
  @State private notificationPermissionKnown: boolean = false;
  @State private notificationEnabled: boolean = false;
  @State private notificationPermissionChecking: boolean = false;
  private taskPageBackTouchStartX: number = -1;
  private taskPageBackTouchStartY: number = -1;
  private taskPageBackGestureHandled: boolean = false;
  private pcDeviceRevokeTouchStartX: number = -1;
  private pcDeviceRevokeTouchStartY: number = -1;
  private pcDeviceRevokeTouchStartOffset: number = 0;

  aboutToAppear(): void {
    this.pageMounted = true;
    PushNotificationCoordinator.setRefreshHandler(async (refreshRef: string): Promise<boolean> => {
      return await this.handlePushRefresh(refreshRef);
    });
    PushNotificationCoordinator.setNotificationPermissionHandler((enabled: boolean, checkFailed: boolean): void => {
      this.notificationPermissionKnown = !checkFailed;
      this.notificationEnabled = enabled;
      this.notificationPermissionChecking = false;
    });
    AppLifecycleCoordinator.setForegroundHandler(async (): Promise<void> => {
      if (!this.pageMounted) {
        return;
      }
      const generation: number = this.taskDetailGeneration;
      await this.refreshBindingStatus(false);
      if (!this.pageMounted || generation !== this.taskDetailGeneration ||
        !AppLifecycleCoordinator.isForeground()) {
        return;
      }
      if (this.pcIsOnline()) {
        await this.refreshTasks(false);
      }
      PushNotificationCoordinator.retryPendingRefresh();
      if (this.activePage === 'detail') {
        this.startTaskSyncLoop();
        this.startLiveReplyStream(true);
        void this.refreshSelectedTask(this.selectedTaskId);
      }
    });
    AppLifecycleCoordinator.setBackgroundHandler((): void => {
      this.invalidateDetailRequest();
      this.taskVisibilityGeneration++;
      this.stopTaskSyncLoop();
      this.stopLiveReplyStream(true);
      this.cancelReplyScroll();
    });
    this.registerDevice();
  }

  aboutToDisappear(): void {
    this.pageMounted = false;
    this.taskVisibilityGeneration++;
    this.invalidateDetailRequest();
    AppLifecycleCoordinator.clearForegroundHandler();
    AppLifecycleCoordinator.clearBackgroundHandler();
    this.stopTaskSyncLoop();
    this.stopLiveReplyStream(true);
    this.cancelReplyScroll();
    this.restoreKeyboardAvoidMode();
    PushNotificationCoordinator.setRefreshHandler(async (_refreshRef: string): Promise<boolean> => false);
    PushNotificationCoordinator.setNotificationPermissionHandler((): void => {});
    this.bindingMonitorGeneration++;
    this.bindingMonitorTask = undefined;
  }

  onBackPress(): boolean {
    if (this.activePage === 'detail' || this.activePage === 'create') {
      this.leaveTaskPage();
      return true;
    }
    if (this.activePage === 'home' && this.homeSection === 'tasks') {
      this.showOverview();
      return true;
    }
    return false;
  }

  private async registerDevice(): Promise<void> {
    if (this.registeringDevice) {
      return;
    }
    this.registeringDevice = true;
    this.deviceRegistered = false;
    this.registrationError = '';
    this.registrationFailure = undefined;
    this.notice = '正在通过开发云端登记手机设备…';
    try {
      this.deviceRegistration = await DeviceRegistrationCoordinator.registerCurrentDevice();
      this.deviceRegistered = true;
      this.notice = '手机设备已在开发云端登记；等待电脑端 Gateway 配对入口';
      await this.refreshBindingStatus(false);
      PushNotificationCoordinator.retryPendingRefresh();
    } catch (error) {
      const failure: RemoteFailureNotice = RemoteApiClient.describeFailure(
        error, '手机设备登记失败，请稍后重试', '按提示处理后点击“连接开发云端”重试。');
      this.registrationFailure = failure;
      this.registrationError = failure.title;
      this.notice = failure.title;
    } finally {
      this.registeringDevice = false;
    }
  }

  private isConnected(): boolean {
    return this.connectionState === PcConnectionState.ONLINE;
  }

  private pcDeviceKey(device: PcDeviceSummary): string {
    return `${device.pcDeviceId}:${device.installationId}:${device.bindingEpoch}`;
  }

  private connectionStateText(state: PcConnectionState): string {
    if (state === PcConnectionState.ONLINE) {
      return '连接正常';
    }
    if (state === PcConnectionState.STALE) {
      return '连接不稳定';
    }
    return '设备未连接';
  }

  private connectionStateColor(state: PcConnectionState): ResourceColor {
    if (state === PcConnectionState.ONLINE) {
      return COLOR_SUCCESS;
    }
    if (state === PcConnectionState.STALE) {
      return COLOR_WARNING;
    }
    return COLOR_DANGER;
  }

  private connectionHelpText(): string {
    if (this.bindingRefreshError.length > 0) {
      return '无法从云端刷新设备状态。请检查手机网络后重试；当前展示可能是上次成功同步的状态。';
    }
    if (this.connectionState === PcConnectionState.STALE) {
      return '电脑心跳已有延迟。请确认电脑未休眠、轩++远程仍在运行，并检查两端网络。';
    }
    if (this.connectionState === PcConnectionState.OFFLINE) {
      return '电脑当前未连接云端。请在电脑上启动轩++远程并确认远程控制仍已开启。';
    }
    return '电脑与云端连接正常。重新进入应用、收到通知或点击检测后将刷新任务。';
  }

  private async scanPairingQr(): Promise<void> {
    if (!this.deviceRegistered || this.deviceRegistration === undefined) {
      this.notice = this.registrationError.length > 0 ? this.registrationError : '请先连接开发云端，再扫描电脑二维码';
      return;
    }
    if (!QrPairingCoordinator.isSupported()) {
      this.pairingState = PairingState.FAILED;
      this.notice = '当前设备不支持系统扫码，无法启动安全绑定流程';
      return;
    }
    const hostContext = this.getUIContext().getHostContext();
    if (hostContext === undefined) {
      this.pairingState = PairingState.FAILED;
      this.notice = '当前无法启动系统扫码，请稍后重试';
      return;
    }

    const monitorGeneration: number = ++this.bindingMonitorGeneration;
    this.scanning = true;
    this.pairingError = '';
    this.pairingFailure = undefined;
    this.notice = '正在启动系统二维码扫描…';
    try {
      const payload: PairingQrPayload = await QrPairingCoordinator.scan(hostContext, this.environment);
      if (this.pcDevices.length === 0) {
        this.pairingState = PairingState.PENDING_LOCAL_CONFIRMATION;
      }
      this.notice = '二维码已校验，正在登记设备并使用 HUKS 持钥签名…';
      if (this.deviceRegistration === undefined) {
        this.deviceRegistration = await DeviceRegistrationCoordinator.registerCurrentDevice();
        this.deviceRegistered = true;
      }
      const pending: PairingPendingResponse = await PairingApiCoordinator.consume(
        this.deviceRegistration, payload);
      this.pendingSummary = pending.bindingSummary;
      this.pendingPcDeviceId = pending.pcDeviceId;
      if (this.pcDevices.length === 0) {
        this.pairingState = PairingState.PENDING_LOCAL_CONFIRMATION;
      }
      this.notice = '配对消费成功，电脑端正在自动确认并激活绑定…';
      hilog.info(DOMAIN, TAG, '%{public}s', 'Pairing consume accepted; waiting for automatic activation');
      const activated: boolean = await this.waitForBindingActive(
        pending.pcDeviceId, pending.confirmationExpiresAt, monitorGeneration);
      if (!activated && monitorGeneration === this.bindingMonitorGeneration) {
        this.notice = '自动确认仍在后台继续；可检查网络，或扫描电脑端最新二维码替代本次绑定';
        hilog.warn(DOMAIN, TAG, '%{public}s', 'Initial binding activation polling timed out');
        this.bindingMonitorTask = this.monitorBindingUntilResolved(
          pending.pcDeviceId, pending.confirmationExpiresAt, monitorGeneration);
      }
    } catch (error) {
      if (monitorGeneration === this.bindingMonitorGeneration) {
        this.bindingMonitorGeneration++;
        this.pendingSummary = undefined;
        this.pendingPcDeviceId = undefined;
        this.pairingState = this.pcDevices.length === 0 ? PairingState.FAILED : PairingState.ACTIVE;
        const failure: RemoteFailureNotice = RemoteApiClient.describeFailure(
          error, '绑定失败，请重新生成电脑端二维码后重试', '按提示处理后重新扫描电脑端最新二维码。');
        this.pairingFailure = failure;
        this.notice = failure.title;
        this.pairingError = failure.title;
        hilog.error(DOMAIN, TAG, '%{public}s', 'Pairing flow failed before activation');
      }
    } finally {
      this.scanning = false;
    }
  }

  private async waitForBindingActive(pcDeviceId: string, confirmationExpiresAt: string,
    monitorGeneration: number): Promise<boolean> {
    const confirmationExpiry: number = Date.parse(confirmationExpiresAt);
    for (let attempt: number = 0; attempt < 30; attempt++) {
      if (monitorGeneration !== this.bindingMonitorGeneration) {
        return false;
      }
      if (Date.now() >= confirmationExpiry) {
        this.finishExpiredBindingMonitor(monitorGeneration);
        return false;
      }
      if (await this.refreshBindingStatus(false, pcDeviceId)) {
        return true;
      }
      await this.waitForNextBindingPoll(1000);
    }
    return false;
  }

  private async monitorBindingUntilResolved(pcDeviceId: string, confirmationExpiresAt: string,
    monitorGeneration: number): Promise<void> {
    const confirmationExpiry: number = Date.parse(confirmationExpiresAt);
    hilog.info(DOMAIN, TAG, '%{public}s', 'Background binding activation polling resumed');
    while (monitorGeneration === this.bindingMonitorGeneration && this.pendingPcDeviceId === pcDeviceId) {
      if (Date.now() >= confirmationExpiry) {
        this.finishExpiredBindingMonitor(monitorGeneration);
        return;
      }
      await this.waitForNextBindingPoll(5000);
      if (monitorGeneration !== this.bindingMonitorGeneration) {
        return;
      }
      if (Date.now() >= confirmationExpiry) {
        this.finishExpiredBindingMonitor(monitorGeneration);
        return;
      }
      if (await this.refreshBindingStatus(false, pcDeviceId)) {
        return;
      }
    }
  }

  private finishExpiredBindingMonitor(monitorGeneration: number): void {
    if (monitorGeneration !== this.bindingMonitorGeneration || this.pendingPcDeviceId === undefined) {
      return;
    }
    this.bindingMonitorGeneration++;
    this.bindingMonitorTask = undefined;
    this.pendingSummary = undefined;
    this.pendingPcDeviceId = undefined;
    this.pairingState = this.pcDevices.length === 0 ? PairingState.FAILED : PairingState.ACTIVE;
    this.notice = '本次自动绑定未在有效期内完成；请在电脑端生成最新二维码后重试';
    this.pairingError = this.notice;
    this.pairingFailure = new RemoteFailureNotice(
      '自动绑定已超时', '电脑端未在本次确认窗口内完成自动绑定。',
      '配对确认窗口已结束 · 可重新发起', '请在电脑端生成最新二维码后重新扫描。', false);
    hilog.warn(DOMAIN, TAG, '%{public}s', 'Binding activation expired before completion');
  }

  private async waitForNextBindingPoll(delayMilliseconds: number): Promise<void> {
    await new Promise<void>((resolve: () => void) => {
      setTimeout(resolve, delayMilliseconds);
    });
  }

  private async refreshBindingStatus(showProgress: boolean = true,
    expectedPcDeviceId?: string): Promise<boolean> {
    if (this.refreshingBinding || this.deviceRegistration === undefined) {
      return false;
    }
    this.refreshingBinding = true;
    if (showProgress) {
      this.notice = '正在使用手机设备持钥签名刷新云端绑定状态…';
    }
    try {
      const response: PcDeviceListResponse = await PairingApiCoordinator.listPcDevices(this.deviceRegistration);
      this.bindingRefreshError = '';
      this.bindingFailure = undefined;
      this.pcDevices = response.pcDevices.slice();
      let activatedDevice: PcDeviceSummary | undefined = undefined;
      let selectedDevice: PcDeviceSummary | undefined = undefined;
      for (let index = 0; index < response.pcDevices.length; index++) {
        const candidate: PcDeviceSummary = response.pcDevices[index];
        if (expectedPcDeviceId !== undefined && candidate.pcDeviceId === expectedPcDeviceId) {
          activatedDevice = candidate;
        }
        if (this.pcDeviceKey(candidate) === this.activePcDeviceKey) {
          selectedDevice = candidate;
        }
      }
      if (activatedDevice !== undefined) {
        selectedDevice = activatedDevice;
        this.pendingSummary = undefined;
        this.pendingPcDeviceId = undefined;
        this.pairingError = '';
        this.pairingFailure = undefined;
        this.notice = `新电脑已绑定：${activatedDevice.displayName}`;
        hilog.info(DOMAIN, TAG, '%{public}s', 'Active binding observed from cloud state');
      }
      if (selectedDevice === undefined && response.pcDevices.length > 0) {
        selectedDevice = this.preferredPcDevice(response.pcDevices);
      }
      if (selectedDevice !== undefined) {
        this.pairingState = PairingState.ACTIVE;
        this.selectPcDevice(selectedDevice, false);
      } else {
        this.clearActivePcDevice();
        this.pairingState = this.pendingPcDeviceId === undefined ? PairingState.UNBOUND :
          PairingState.PENDING_LOCAL_CONFIRMATION;
      }
      if (showProgress && activatedDevice === undefined) {
        if (this.pendingPcDeviceId !== undefined) {
          this.notice = '云端仍在处理自动确认，可稍后安全重试扫码';
        } else if (selectedDevice !== undefined) {
          this.notice = `设备状态已刷新：${selectedDevice.displayName}`;
        } else {
          this.notice = '当前手机尚无已激活的电脑绑定';
        }
      }
      return expectedPcDeviceId === undefined ? selectedDevice !== undefined : activatedDevice !== undefined;
    } catch (error) {
      const failure: RemoteFailureNotice = RemoteApiClient.describeFailure(
        error, '刷新云端绑定状态失败', '检查连接后点击“重新检测连接”。');
      this.bindingFailure = failure;
      this.bindingRefreshError = failure.title;
      this.notice = failure.title;
      hilog.warn(DOMAIN, TAG, '%{public}s', 'Binding status refresh failed');
      return false;
    } finally {
      this.refreshingBinding = false;
    }
  }

  private async handlePushRefresh(refreshRef: string): Promise<boolean> {
    const registration: DeviceRegistrationResult | undefined = this.deviceRegistration;
    if (registration === undefined) {
      return false;
    }
    this.taskNotice = '已收到任务状态提示，正在安全刷新最新状态…';
    const target: PushRefreshTarget = await PushRefreshCoordinator.resolve(refreshRef, registration);
    await this.refreshBindingStatus(false, target.pcDeviceId);
    let targetPc: PcDeviceSummary | undefined = undefined;
    for (let index: number = 0; index < this.pcDevices.length; index++) {
      const candidate: PcDeviceSummary = this.pcDevices[index];
      if (candidate.pcDeviceId === target.pcDeviceId &&
        candidate.installationId === target.installationId &&
        candidate.bindingEpoch === target.bindingEpoch) {
        targetPc = candidate;
        break;
      }
    }
    if (targetPc === undefined) {
      return false;
    }
    this.selectPcDevice(targetPc, false);
    let requiresRefresh: boolean = true;
    for (let index: number = 0; index < this.tasks.length; index++) {
      const current: RemoteTaskSnapshot = this.tasks[index];
      if (current.remoteTaskId === target.remoteTaskId &&
        current.stateVersion >= target.terminalStateVersion) {
        requiresRefresh = false;
        break;
      }
    }
    if (requiresRefresh) {
      await this.refreshTasks(false);
    }
    for (let index: number = 0; index < this.tasks.length; index++) {
      const task: RemoteTaskSnapshot = this.tasks[index];
      if (task.remoteTaskId === target.remoteTaskId &&
        task.stateVersion >= target.terminalStateVersion) {
        this.openTaskDetail(task);
        this.taskNotice = '已通过任务结束通知刷新到最新状态';
        return true;
      }
    }
    return false;
  }

  private preferredPcDevice(devices: PcDeviceSummary[]): PcDeviceSummary {
    for (let index: number = 0; index < devices.length; index++) {
      if (devices[index].pcConnectionState === PcConnectionState.ONLINE) {
        return devices[index];
      }
    }
    return devices[0];
  }

  private selectPcDevice(pcDevice: PcDeviceSummary, userInitiated: boolean): void {
    if (userInitiated && this.commandSubmitting) {
      return;
    }
    const currentKey: string = this.activePcDeviceKey;
    const nextKey: string = this.pcDeviceKey(pcDevice);
    const wasOnline: boolean = currentKey === nextKey && this.connectionState === PcConnectionState.ONLINE;
    this.activePcDevice = pcDevice;
    this.activePcDeviceKey = nextKey;
    this.connectionState = pcDevice.pcConnectionState;
    if (this.activePage === 'create') {
      this.refreshCreateState();
    }
    if (currentKey === nextKey) {
      if (!this.pcIsOnline()) {
        this.hideTasksForDisconnectedPc();
      } else if (!wasOnline) {
        this.taskNotice = '电脑已恢复连接，正在刷新任务…';
        void this.refreshTasks(false);
      }
      return;
    }
    this.newTaskInstruction = '';
    this.selectedWorkspaceSourceTaskId = '';
    this.selectedModelConfigId = '';
    this.connectionDetailsExpanded = false;
    this.clearTaskProjection(this.connectionState === PcConnectionState.ONLINE ?
      '正在同步当前电脑的任务…' : '电脑未连接；重新进入应用、收到通知或点击检测后将刷新任务');
    this.commandNotice = userInitiated ? `已切换到：${pcDevice.displayName}` :
      '选择任务后可发送消息或停止任务';
    if (this.pcIsOnline()) {
      void this.refreshTasks(false);
    } else {
      this.hideTasksForDisconnectedPc();
    }
  }

  private clearActivePcDevice(): void {
    this.activePcDevice = undefined;
    this.activePcDeviceKey = '';
    this.connectionState = PcConnectionState.OFFLINE;
    this.clearTaskProjection('绑定电脑后将自动同步任务');
  }

  private async revokePcDevice(device: PcDeviceSummary): Promise<void> {
    if (this.revokingPcDeviceKey.length > 0 || this.deviceRegistration === undefined) {
      return;
    }
    const deviceKey: string = this.pcDeviceKey(device);
    this.revokingPcDeviceKey = deviceKey;
    this.bindingFailure = undefined;
    this.bindingRefreshError = '';
    this.notice = `正在撤销绑定：${device.displayName}`;
    try {
      await PairingApiCoordinator.revokeBinding(this.deviceRegistration, device.bindingId);
      const retainedDevices: PcDeviceSummary[] = [];
      for (let index: number = 0; index < this.pcDevices.length; index++) {
        const candidate: PcDeviceSummary = this.pcDevices[index];
        if (candidate.bindingId !== device.bindingId) {
          retainedDevices.push(candidate);
        }
      }
      this.pcDevices = retainedDevices;
      if (this.activePcDeviceKey === deviceKey) {
        if (retainedDevices.length > 0) {
          this.selectPcDevice(this.preferredPcDevice(retainedDevices), false);
        } else {
          this.clearActivePcDevice();
          this.pairingState = PairingState.UNBOUND;
        }
      }
      if (retainedDevices.length <= 1) {
        this.deviceListExpanded = false;
      }
      this.notice = `已撤销绑定：${device.displayName}`;
      hilog.info(DOMAIN, TAG, '%{public}s', 'PC binding revoked from device list');
    } catch (error) {
      const failure: RemoteFailureNotice = RemoteApiClient.describeFailure(
        error, '撤销电脑绑定失败', '请检查网络后重试；设备仍会保留在列表中。');
      this.bindingFailure = failure;
      this.bindingRefreshError = failure.title;
      this.notice = failure.title;
      hilog.warn(DOMAIN, TAG, '%{public}s', 'PC binding revocation failed');
    } finally {
      this.revokingPcDeviceKey = '';
    }
  }

  private pcIsOnline(): boolean {
    return this.activePcDevice !== undefined &&
      this.activePcDevice.pcConnectionState === PcConnectionState.ONLINE &&
      this.connectionState === PcConnectionState.ONLINE;
  }

  private hideTasksForDisconnectedPc(): void {
    this.clearTaskProjection('电脑未连接。重新进入应用、收到通知或点击检测后将刷新任务');
  }

  private async requestNotificationPermission(): Promise<void> {
    if (this.notificationPermissionChecking) {
      return;
    }
    this.notificationPermissionChecking = true;
    const enabled: boolean = await PushNotificationCoordinator.requestNotificationPermission();
    if (enabled) {
      this.notice = '任务结束通知已开启';
    } else {
      this.notice = '通知尚未开启；可在系统通知设置中允许“轩++远程”通知后再次检测。';
    }
  }

  private async refreshNotificationPermission(): Promise<void> {
    if (this.notificationPermissionChecking) {
      return;
    }
    this.notificationPermissionChecking = true;
    const enabled: boolean = await PushNotificationCoordinator.refreshNotificationPermission();
    if (enabled) {
      this.notice = '任务结束通知已开启';
    } else {
      this.notice = '通知尚未开启；可前往系统通知设置恢复。';
    }
    this.notificationPermissionChecking = false;
  }

  private async confirmRevokePcDevice(device: PcDeviceSummary): Promise<void> {
    const deviceKey: string = this.pcDeviceKey(device);
    if (this.commandSubmitting || this.revokingPcDeviceKey.length > 0 ||
      this.revokeConfirmationPcDeviceKey.length > 0) {
      return;
    }
    this.revokeConfirmationPcDeviceKey = deviceKey;
    try {
      const response = await this.getUIContext().getPromptAction().showDialog({
        title: '撤销电脑绑定',
        message: `确定要撤销此手机与“${device.displayName}”的绑定吗？撤销后，该电脑将不再向此手机同步任务。`,
        buttons: [
          { text: '取消', color: $r('app.color.remote_ink_secondary'), primary: true },
          { text: '撤销绑定', color: $r('app.color.remote_danger') }
        ]
      });
      if (response.index === 1) {
        void this.revokePcDevice(device);
      } else {
        this.notice = '已取消撤销绑定';
      }
    } catch (_error) {
      this.notice = '无法显示撤销确认；请稍后重试。';
    } finally {
      if (this.revokeConfirmationPcDeviceKey === deviceKey) {
        this.revokeConfirmationPcDeviceKey = '';
      }
    }
  }

  private clearTaskProjection(notice: string): void {
    this.invalidateDetailRequest();
    this.stopTaskSyncLoop();
    this.cancelReplyScroll();
    this.restoreKeyboardAvoidMode();
    if (this.activePage === 'detail') {
      this.stopLiveReplyStream(false);
      this.activePage = 'home';
    }
    this.taskVisibilityGeneration++;
    this.taskRefreshPending = false;
    this.tasks = [];
    this.expandedTaskWorkspaces = [];
    this.selectedTaskId = '';
    this.selectedWorkspaceSourceTaskId = '';
    this.taskFailure = undefined;
    this.nextTaskCursor = null;
    this.lastTaskSyncAt = '';
    this.taskCursorsSeen = [];
    this.taskNotice = notice;
  }

  private startTaskSyncLoop(): void {
    this.stopTaskSyncLoop();
    const generation: number = this.taskSyncGeneration;
    this.scheduleTaskSync(generation);
  }

  private stopTaskSyncLoop(): void {
    this.taskSyncGeneration++;
    if (this.taskSyncTimer >= 0) {
      clearTimeout(this.taskSyncTimer);
      this.taskSyncTimer = -1;
    }
  }

  private scheduleTaskSync(generation: number): void {
    if (generation !== this.taskSyncGeneration || !this.pageMounted ||
      this.activePage !== 'detail' || !AppLifecycleCoordinator.isForeground()) {
      return;
    }
    if (this.taskSyncTimer >= 0) {
      clearTimeout(this.taskSyncTimer);
    }
    this.taskSyncTimer = setTimeout((): void => {
      this.taskSyncTimer = -1;
      void this.runTaskSync(generation);
    }, TASK_REFRESH_INTERVAL_MS);
  }

  private async runTaskSync(generation: number): Promise<void> {
    if (generation !== this.taskSyncGeneration || !this.pageMounted ||
      this.activePage !== 'detail' || !AppLifecycleCoordinator.isForeground()) {
      return;
    }
    try {
      if (this.pcIsOnline()) {
        await this.refreshSelectedTask(this.selectedTaskId);
      }
    } finally {
      this.scheduleTaskSync(generation);
    }
  }

  private async refreshTasks(showProgress: boolean = true): Promise<void> {
    if (!this.pageMounted || !AppLifecycleCoordinator.isForeground()) {
      return;
    }
    if (this.refreshingTasks) {
      this.taskRefreshPending = this.pcIsOnline();
      return;
    }
    if (this.deviceRegistration === undefined || this.activePcDevice === undefined || !this.pcIsOnline()) {
      if (this.activePcDevice !== undefined && !this.pcIsOnline()) {
        this.hideTasksForDisconnectedPc();
      }
      return;
    }
    const requestedPcDeviceKey: string = this.pcDeviceKey(this.activePcDevice);
    const requestedVisibilityGeneration: number = this.taskVisibilityGeneration;
    this.refreshingTasks = true;
    if (showProgress) {
      this.taskNotice = '正在使用手机持钥签名刷新任务…';
    }
    try {
      const response: TaskListResponse = await TaskSyncCoordinator.listTasks(
        this.deviceRegistration, this.activePcDevice);
      if (!this.pageMounted || !AppLifecycleCoordinator.isForeground() ||
        this.activePcDevice === undefined || this.pcDeviceKey(this.activePcDevice) !== requestedPcDeviceKey ||
        !this.pcIsOnline() || requestedVisibilityGeneration !== this.taskVisibilityGeneration) {
        return;
      }
      for (let index: number = 0; index < response.tasks.length; index++) {
        if (response.tasks[index].pcConnectionState !== PcConnectionState.ONLINE) {
          this.connectionState = response.tasks[index].pcConnectionState;
          this.hideTasksForDisconnectedPc();
          return;
        }
      }
      const selected: RemoteTaskSnapshot | undefined = this.selectedTask();
      const retained: RemoteTaskSnapshot[] = this.tasks.filter((current: RemoteTaskSnapshot): boolean =>
        response.tasks.some((incoming: RemoteTaskSnapshot): boolean =>
          incoming.remoteTaskId === current.remoteTaskId));
      // 首页没有选中任务不代表它已删除，详情可能来自后续分页。
      if (this.activePage === 'detail' && selected !== undefined && response.nextCursor !== null &&
        !retained.some((current: RemoteTaskSnapshot): boolean => current.remoteTaskId === selected.remoteTaskId)) {
        retained.push(selected);
      }
      this.tasks = this.mergeTaskPage(retained, response.tasks);
      if (this.expandedTaskWorkspaces.length === 0 && this.tasks.length > 0) {
        const initialGroups: TaskWorkspaceGroup[] = this.taskWorkspaceGroups();
        const initialExpanded: string[] = [];
        for (let index: number = 0; index < initialGroups.length && index < 2; index++) {
          initialExpanded.push(initialGroups[index].name);
        }
        this.expandedTaskWorkspaces = initialExpanded;
      }
      this.calibrateLiveReply();
      this.nextTaskCursor = response.nextCursor;
      this.taskCursorsSeen = [];
      this.lastTaskSyncAt = response.serverReceivedAt;
      this.taskFailure = undefined;
      if (this.activePage === 'create' && this.selectedWorkspaceSource() === undefined) {
        const workspaces: WorkspaceChoice[] = this.workspaceChoices();
        this.selectedWorkspaceSourceTaskId = workspaces.length === 0 ? '' : workspaces[0].sourceTaskId;
      }
      if (this.activePage === 'create') {
        this.refreshCreateState();
      }
      let selectionExists: boolean = false;
      for (let index: number = 0; index < this.tasks.length; index++) {
        if (this.tasks[index].remoteTaskId === this.selectedTaskId) {
          selectionExists = true;
          break;
        }
      }
      if (!selectionExists) {
        if (this.activePage === 'detail') {
          this.leaveTaskDetail();
        }
        this.selectedTaskId = response.tasks.length === 0 ? '' : response.tasks[0].remoteTaskId;
      }
      this.taskNotice = response.tasks.length === 0 ?
        '电脑已连接，暂未收到任务快照；首次同步最多等待一个心跳周期' :
        `已同步 ${response.tasks.length} 个任务${response.nextCursor === null ? '，已到底' : '，可继续加载'}`;
      PushNotificationCoordinator.retryPendingRefresh();
    } catch (error) {
      if (!this.pageMounted || !AppLifecycleCoordinator.isForeground()) {
        return;
      }
      if (!this.pcIsOnline() || requestedVisibilityGeneration !== this.taskVisibilityGeneration) {
        this.hideTasksForDisconnectedPc();
        return;
      }
      const failure: RemoteFailureNotice = RemoteApiClient.describeFailure(
        error, '刷新任务列表失败', '保留当前页面，检查连接后点击“刷新任务”。');
      this.taskFailure = failure;
      this.taskNotice = '任务同步未完成';
      hilog.warn(DOMAIN, TAG, '%{public}s', 'Task list refresh failed');
    } finally {
      if (this.pageMounted) {
        this.refreshingTasks = false;
      }
      if (this.taskRefreshPending) {
        const shouldRefresh: boolean = this.pageMounted && AppLifecycleCoordinator.isForeground() && this.pcIsOnline();
        this.taskRefreshPending = false;
        if (shouldRefresh) {
          await this.refreshTasks(false);
        }
      }
    }
  }

  private async loadMoreTasks(): Promise<void> {
    const cursor: string | null = this.nextTaskCursor;
    if (cursor === null || this.loadingMoreTasks || this.refreshingTasks || this.deviceRegistration === undefined ||
      this.activePcDevice === undefined || !this.pcIsOnline()) {
      return;
    }
    if (this.taskCursorsSeen.includes(cursor)) {
      this.nextTaskCursor = null;
      this.taskNotice = '任务分页游标重复，已保留已加载任务；请刷新后重试。';
      return;
    }
    const requestedPcDeviceKey: string = this.pcDeviceKey(this.activePcDevice);
    const requestedVisibilityGeneration: number = this.taskVisibilityGeneration;
    this.loadingMoreTasks = true;
    this.taskFailure = undefined;
    this.taskNotice = '正在加载更多任务…';
    try {
      const response: TaskListResponse = await TaskSyncCoordinator.listTasks(
        this.deviceRegistration, this.activePcDevice, cursor);
      if (this.activePcDevice === undefined || this.pcDeviceKey(this.activePcDevice) !== requestedPcDeviceKey ||
        !this.pcIsOnline() || requestedVisibilityGeneration !== this.taskVisibilityGeneration) {
        return;
      }
      this.taskCursorsSeen.push(cursor);
      if (response.nextCursor !== null && this.taskCursorsSeen.includes(response.nextCursor)) {
        this.nextTaskCursor = null;
        this.taskNotice = '任务分页游标重复，已保留已加载任务；请刷新后重试。';
        return;
      }
      this.tasks = this.mergeTaskPage(this.tasks, response.tasks);
      this.nextTaskCursor = response.nextCursor;
      this.lastTaskSyncAt = response.serverReceivedAt;
      this.taskNotice = `已加载 ${this.tasks.length} 个任务${response.nextCursor === null ? '，已到底' : '，可继续加载'}`;
    } catch (error) {
      if (!this.pcIsOnline() || requestedVisibilityGeneration !== this.taskVisibilityGeneration) {
        return;
      }
      const failure: RemoteFailureNotice = RemoteApiClient.describeFailure(
        error, '加载更多任务失败', '已保留当前任务；检查连接后点击“加载更多”重试。');
      this.taskFailure = failure;
      this.taskNotice = '加载更多任务未完成';
    } finally {
      this.loadingMoreTasks = false;
    }
  }

  private mergeTaskPage(existingTasks: RemoteTaskSnapshot[], incomingTasks: RemoteTaskSnapshot[]): RemoteTaskSnapshot[] {
    const merged: RemoteTaskSnapshot[] = existingTasks.slice();
    for (let incomingIndex: number = 0; incomingIndex < incomingTasks.length; incomingIndex++) {
      const incoming: RemoteTaskSnapshot = incomingTasks[incomingIndex];
      let existingIndex: number = -1;
      for (let index: number = 0; index < merged.length; index++) {
        if (merged[index].remoteTaskId === incoming.remoteTaskId) {
          existingIndex = index;
          break;
        }
      }
      if (existingIndex < 0) {
        merged.push(incoming);
      } else if (incoming.stateVersion >= merged[existingIndex].stateVersion) {
        merged[existingIndex] = incoming;
      }
    }
    return this.sortTasks(merged);
  }

  private sortTasks(tasks: RemoteTaskSnapshot[]): RemoteTaskSnapshot[] {
    return tasks.sort((left: RemoteTaskSnapshot, right: RemoteTaskSnapshot): number => {
      const rightUpdatedAt: number = Date.parse(right.serverReceivedAt);
      const leftUpdatedAt: number = Date.parse(left.serverReceivedAt);
      if (rightUpdatedAt !== leftUpdatedAt) {
        return rightUpdatedAt - leftUpdatedAt;
      }
      return right.stateVersion - left.stateVersion;
    });
  }

  private taskPresentationStatus(task: RemoteTaskSnapshot): TaskPresentationStatus {
    if (task.taskStatus === 'failed' || task.turnStatus === 'failed') {
      return 'failed';
    }
    if (task.taskStatus === 'archived') {
      return 'archived';
    }
    if (task.turnStatus === 'completed') {
      return 'completed';
    }
    if (task.taskStatus === 'stopping' || task.taskStatus === 'stopped' || task.turnStatus === 'stopped' ||
      task.turnStatus === 'interrupted') {
      return 'stopped';
    }
    if (task.taskStatus === 'pausing' || task.taskStatus === 'paused') {
      return 'paused';
    }
    if (task.taskStatus === 'queued' || task.taskStatus === 'starting') {
      return 'queued';
    }
    if (task.taskStatus === 'running' || this.taskHasActiveTurn(task)) {
      return 'running';
    }
    return 'created';
  }

  private taskStatusText(task: RemoteTaskSnapshot): string {
    const status: TaskPresentationStatus = this.taskPresentationStatus(task);
    if (status === 'running') {
      return '运行中';
    }
    if (status === 'queued') {
      return '等待启动';
    }
    if (status === 'paused') {
      return '已暂停';
    }
    if (status === 'stopped') {
      return '已停止';
    }
    if (status === 'completed') {
      return '已完成';
    }
    if (status === 'failed') {
      return '失败';
    }
    if (status === 'archived') {
      return '已归档';
    }
    return '已创建';
  }

  private taskStatusColor(task: RemoteTaskSnapshot): ResourceColor {
    const status: TaskPresentationStatus = this.taskPresentationStatus(task);
    if (status === 'failed') {
      return COLOR_DANGER;
    }
    if (status === 'running' || status === 'queued') {
      return COLOR_ACCENT;
    }
    if (status === 'paused') {
      return COLOR_WARNING;
    }
    return COLOR_SUCCESS;
  }

  private taskStatusBackground(task: RemoteTaskSnapshot): ResourceColor {
    const status: TaskPresentationStatus = this.taskPresentationStatus(task);
    if (status === 'failed') {
      return COLOR_DANGER_SOFT;
    }
    if (status === 'running' || status === 'queued') {
      return COLOR_ACCENT_SOFT;
    }
    if (status === 'paused') {
      return COLOR_WARNING_SOFT;
    }
    return COLOR_SUCCESS_SOFT;
  }

  private turnStatusText(task: RemoteTaskSnapshot): string {
    if (task.turnStatus === 'running' || task.turnStatus === 'starting') {
      return '回合执行中';
    }
    if (task.turnStatus === 'failed') {
      return '回合失败';
    }
    if (task.turnStatus === 'completed') {
      return '回合已完成';
    }
    if (task.turnStatus === 'interrupted') {
      return '回合已中断';
    }
    return '回合空闲';
  }

  private selectedTask(): RemoteTaskSnapshot | undefined {
    for (let index: number = 0; index < this.tasks.length; index++) {
      if (this.tasks[index].remoteTaskId === this.selectedTaskId) {
        return this.tasks[index];
      }
    }
    return undefined;
  }

  private selectedTaskHasActiveTurn(): boolean {
    const task: RemoteTaskSnapshot | undefined = this.selectedTask();
    return task !== undefined && this.taskIsRunning(task);
  }

  private detailExecutionText(task: RemoteTaskSnapshot): string {
    if (this.taskIsRunning(task)) {
      return '当前回合正在执行，回复内容会在下方持续更新';
    }
    if (task.taskStatus === 'failed' || task.turnStatus === 'failed') {
      return '当前任务或回合已失败，请查看状态与错误信息';
    }
    if (task.turnStatus === 'completed') {
      return '当前回合已完成';
    }
    if (task.taskStatus === 'stopped' || task.turnStatus === 'stopped' || task.turnStatus === 'interrupted') {
      return '当前回合已停止或中断';
    }
    return '当前回合尚未执行';
  }

  private replyHeading(task: RemoteTaskSnapshot): string {
    return this.taskIsRunning(task) ? '回复记录（持续更新）' : '回复记录';
  }

  private detailReplyText(task: RemoteTaskSnapshot): string {
    if (task.lastReplyState === ReplyState.WITHHELD) {
      return '';
    }
    const snapshotText: string = task.lastReply?.text ?? '';
    if (!this.liveReplyActive && this.liveReplyObservedAt.length > 0 &&
      Date.parse(task.pcObservedAt) > Date.parse(this.liveReplyObservedAt)) {
      return snapshotText;
    }
    // 实时流与快照携带同一份回复记录，快照已追平时不重复展示，也不退回较短的流内容。
    if ((this.liveReplyActive || this.liveReplyAwaitingFinalSnapshot) && this.liveReplyText.length > 0 &&
      !snapshotText.startsWith(this.liveReplyText)) {
      return this.liveReplyText;
    }
    return snapshotText;
  }

  private calibrateLiveReply(): void {
    const task: RemoteTaskSnapshot | undefined = this.selectedTask();
    if (task === undefined || !this.liveReplyAwaitingFinalSnapshot) {
      return;
    }
    const snapshotText: string = task.lastReply?.text ?? '';
    if (task.lastReplyState !== ReplyState.WITHHELD &&
      (this.taskHasActiveTurn(task) || !snapshotText.startsWith(this.pendingLiveReplyText))) {
      return;
    }
    this.liveReplyText = '';
    this.pendingLiveReplyText = '';
    this.liveReplyAwaitingFinalSnapshot = false;
    this.liveReplyNotice = '';
  }

  private replyStateTitle(task: RemoteTaskSnapshot): string {
    if (task.lastReplyState === ReplyState.WITHHELD) {
      return '答复不可展示';
    }
    if (task.lastReplyState === ReplyState.ABSENT) {
      return '尚无最终答复';
    }
    if (task.lastReplyState === ReplyState.TRUNCATED) {
      return '答复已截断';
    }
    return '完整回复';
  }

  private replyStateDescription(task: RemoteTaskSnapshot): string {
    if (task.lastReplyState === ReplyState.WITHHELD) {
      return '该最终答复因远程安全策略不可展示。';
    }
    if (task.lastReplyState === ReplyState.ABSENT) {
      return '该回合尚未提供可展示的回复。';
    }
    if (task.lastReplyState === ReplyState.TRUNCATED) {
      const bytes: number = task.lastReply?.byteLength ?? 0;
      return `手机端仅展示最近 ${bytes} B；完整记录仍保留在电脑端。`;
    }
    return `电脑观测：${formatChinaDateTime(task.pcObservedAt)}`;
  }

  private selectedModelConfig(): ModelConfigSummary | undefined {
    const configs: ModelConfigSummary[] = this.activePcDevice?.modelConfigs ?? [];
    for (let index: number = 0; index < configs.length; index++) {
      if (configs[index].id === this.selectedModelConfigId) {
        return configs[index];
      }
    }
    return undefined;
  }

  private taskWorkspaceGroups(): TaskWorkspaceGroup[] {
    const groups: TaskWorkspaceGroup[] = [];
    const visibleTasks: RemoteTaskSnapshot[] = this.filteredTasks();
    for (let taskIndex: number = 0; taskIndex < visibleTasks.length; taskIndex++) {
      const task: RemoteTaskSnapshot = visibleTasks[taskIndex];
      const workspaceName: string = task.workspaceName.trim().length === 0 ? '未命名工作区' : task.workspaceName;
      let existingGroup: TaskWorkspaceGroup | undefined = undefined;
      for (let groupIndex: number = 0; groupIndex < groups.length; groupIndex++) {
        if (groups[groupIndex].name === workspaceName) {
          existingGroup = groups[groupIndex];
          break;
        }
      }
      if (existingGroup === undefined) {
        existingGroup = { name: workspaceName, tasks: [], runningTaskCount: 0 };
        groups.push(existingGroup);
      }
      existingGroup.tasks.push(task);
    }
    for (let taskIndex: number = 0; taskIndex < this.tasks.length; taskIndex++) {
      const task: RemoteTaskSnapshot = this.tasks[taskIndex];
      if (!this.taskIsRunning(task)) {
        continue;
      }
      const workspaceName: string = task.workspaceName.trim().length === 0 ? '未命名工作区' : task.workspaceName;
      for (let groupIndex: number = 0; groupIndex < groups.length; groupIndex++) {
        if (groups[groupIndex].name === workspaceName) {
          groups[groupIndex].runningTaskCount += 1;
          break;
        }
      }
    }
    const runningGroups: TaskWorkspaceGroup[] = [];
    const idleGroups: TaskWorkspaceGroup[] = [];
    for (let groupIndex: number = 0; groupIndex < groups.length; groupIndex++) {
      if (groups[groupIndex].runningTaskCount > 0) {
        runningGroups.push(groups[groupIndex]);
      } else {
        idleGroups.push(groups[groupIndex]);
      }
    }
    return runningGroups.concat(idleGroups);
  }

  private taskWorkspaceIsExpanded(workspaceName: string): boolean {
    return this.expandedTaskWorkspaces.includes(workspaceName);
  }

  private toggleTaskWorkspace(workspaceName: string): void {
    const expandedIndex: number = this.expandedTaskWorkspaces.indexOf(workspaceName);
    if (expandedIndex >= 0) {
      const nextExpandedWorkspaces: string[] = this.expandedTaskWorkspaces.slice();
      nextExpandedWorkspaces.splice(expandedIndex, 1);
      this.expandedTaskWorkspaces = nextExpandedWorkspaces;
      return;
    }
    this.expandedTaskWorkspaces = this.expandedTaskWorkspaces.concat([workspaceName]);
  }

  private filteredTasks(): RemoteTaskSnapshot[] {
    const query: string = this.taskSearchQuery.trim().toLocaleLowerCase();
    const visible: RemoteTaskSnapshot[] = [];
    for (let index: number = 0; index < this.tasks.length; index++) {
      const task: RemoteTaskSnapshot = this.tasks[index];
      const searchable: string = `${task.name}\n${task.workspaceName}\n${task.modelLabel}`.toLocaleLowerCase();
      if (query.length > 0 && !searchable.includes(query)) {
        continue;
      }
      if (this.taskFilter === 'running' && !this.taskIsRunning(task)) {
        continue;
      }
      if (this.taskFilter === 'attention' && !this.taskNeedsAttention(task)) {
        continue;
      }
      if (this.taskFilter === 'ended' && !this.taskIsEnded(task)) {
        continue;
      }
      visible.push(task);
    }
    return visible;
  }

  private taskIsRunning(task: RemoteTaskSnapshot): boolean {
    const status: TaskPresentationStatus = this.taskPresentationStatus(task);
    return status === 'queued' || status === 'running';
  }

  private taskHasActiveTurn(task: RemoteTaskSnapshot): boolean {
    return task.turnStatus === 'starting' || task.turnStatus === 'running' || task.turnStatus === 'pausing';
  }

  private taskNeedsAttention(task: RemoteTaskSnapshot): boolean {
    return task.taskStatus === 'failed' || task.turnStatus === 'failed' || task.lastError !== null;
  }

  private taskIsEnded(task: RemoteTaskSnapshot): boolean {
    return task.taskStatus === 'stopped' || task.taskStatus === 'archived' ||
      task.turnStatus === 'completed';
  }

  private workspaceChoices(): WorkspaceChoice[] {
    const choices: WorkspaceChoice[] = [];
    for (let taskIndex: number = 0; taskIndex < this.tasks.length; taskIndex++) {
      const task: RemoteTaskSnapshot = this.tasks[taskIndex];
      if (task.workspaceName === '独立任务') {
        continue;
      }
      let existingIndex: number = -1;
      for (let choiceIndex: number = 0; choiceIndex < choices.length; choiceIndex++) {
        if (choices[choiceIndex].name === task.workspaceName) {
          existingIndex = choiceIndex;
          break;
        }
      }
      if (existingIndex >= 0) {
        choices[existingIndex].taskCount++;
      } else {
        choices.push({ name: task.workspaceName, sourceTaskId: task.remoteTaskId, taskCount: 1 });
      }
    }
    return choices;
  }

  private selectedWorkspaceSource(): RemoteTaskSnapshot | undefined {
    for (let index: number = 0; index < this.tasks.length; index++) {
      if (this.tasks[index].remoteTaskId === this.selectedWorkspaceSourceTaskId) {
        return this.tasks[index];
      }
    }
    return undefined;
  }

  private derivedTaskName(): string {
    const trimmed: string = this.newTaskInstruction.trim();
    if (trimmed.length === 0) {
      return '新任务';
    }
    const firstLine: string = trimmed.split('\n')[0].trim();
    return firstLine.length > 36 ? `${firstLine.slice(0, 36)}…` : firstLine;
  }

  private newTaskInstructionBytes(): number {
    return utf8ByteLength(this.newTaskInstruction.trim());
  }

  private openTaskDetail(task: RemoteTaskSnapshot): void {
    this.invalidateDetailRequest();
    this.cancelReplyScroll();
    this.replyFollowBottom = true;
    if (this.previousKeyboardAvoidMode === undefined) {
      this.previousKeyboardAvoidMode = this.getUIContext().getKeyboardAvoidMode();
      this.getUIContext().setKeyboardAvoidMode(KeyboardAvoidMode.RESIZE);
    }
    if (this.selectedTaskId !== task.remoteTaskId) {
      this.instruction = '';
    }
    this.selectedTaskId = task.remoteTaskId;
    this.activePage = 'detail';
    this.commandFailure = undefined;
    this.commandNotice = `当前任务：${task.name.length === 0 ? '未命名任务' : task.name}`;
    this.startLiveReplyStream();
    this.startTaskSyncLoop();
    void this.refreshSelectedTask(task.remoteTaskId);
  }

  private leaveTaskDetail(): void {
    this.invalidateDetailRequest();
    this.stopTaskSyncLoop();
    this.cancelReplyScroll();
    this.restoreKeyboardAvoidMode();
    this.stopLiveReplyStream(false);
    this.activePage = 'home';
    this.homeSection = 'tasks';
  }

  private leaveTaskPage(): void {
    if (this.activePage === 'detail') {
      this.leaveTaskDetail();
      return;
    }
    if (this.activePage === 'create') {
      this.activePage = 'home';
      this.homeSection = 'tasks';
    }
  }

  private invalidateDetailRequest(): void {
    this.taskDetailGeneration++;
    this.refreshingSelectedTask = false;
    this.selectedTaskRefreshPending = false;
  }

  private restoreKeyboardAvoidMode(): void {
    if (this.previousKeyboardAvoidMode !== undefined) {
      this.getUIContext().setKeyboardAvoidMode(this.previousKeyboardAvoidMode);
      this.previousKeyboardAvoidMode = undefined;
    }
  }

  private cancelReplyScroll(): void {
    if (this.replyScrollTimer !== undefined) {
      clearTimeout(this.replyScrollTimer);
      this.replyScrollTimer = undefined;
    }
  }

  private followReplyBottom(): void {
    if (!this.replyFollowBottom || !this.pageMounted || this.activePage !== 'detail' ||
      !AppLifecycleCoordinator.isForeground() || this.replyScrollTimer !== undefined) {
      return;
    }
    this.replyScrollTimer = setTimeout((): void => {
      this.replyScrollTimer = undefined;
      if (this.replyFollowBottom && this.pageMounted && this.activePage === 'detail' &&
        AppLifecycleCoordinator.isForeground()) {
        this.replyScroller.scrollEdge(Edge.Bottom);
      }
    }, 0);
  }

  private trackReplyScroll(yOffset: number, scrollState: ScrollState): void {
    // 仅用户滚动改变跟随意图；布局变化和程序滚动不能误判为向上阅读。
    if (scrollState !== ScrollState.Idle && yOffset !== 0) {
      this.replyFollowBottom = yOffset > 0 && this.replyScroller.isAtEnd();
      if (!this.replyFollowBottom) {
        this.cancelReplyScroll();
      }
    }
  }

  private startLiveReplyStream(preserveContent: boolean = false): void {
    const task: RemoteTaskSnapshot | undefined = this.selectedTask();
    const registration: DeviceRegistrationResult | undefined = this.deviceRegistration;
    if (!this.pageMounted || this.activePage !== 'detail' || task === undefined || registration === undefined ||
      !this.pcIsOnline() || !AppLifecycleCoordinator.isForeground()) {
      return;
    }
    const taskId: string = task.remoteTaskId;
    this.stopLiveReplyStream(preserveContent);
    const generation: number = this.liveReplyGeneration;
    this.liveReplyNotice = '正在连接实时回复…';
    void LiveReplyStreamCoordinator.connect(
      registration,
      task,
      (update: LiveReplyUpdate): void => {
        if (generation !== this.liveReplyGeneration || !this.pageMounted ||
          this.activePage !== 'detail' || this.selectedTaskId !== taskId ||
          !AppLifecycleCoordinator.isForeground()) {
          return;
        }
        this.liveReplyActive = true;
        this.liveReplyAwaitingFinalSnapshot = false;
        this.liveReplyReconnectAttempt = 0;
        this.liveReplyObservedAt = update.observedAt;
        this.pendingLiveReplyText = update.text;
        if (this.liveReplyRenderTimer === undefined) {
          this.liveReplyRenderTimer = setTimeout((): void => {
            this.liveReplyText = this.pendingLiveReplyText;
            this.liveReplyRenderTimer = undefined;
          }, 100);
        }
      },
      (_outcome: string): void => {
        if (generation !== this.liveReplyGeneration || !this.pageMounted ||
          this.activePage !== 'detail' || this.selectedTaskId !== taskId ||
          !AppLifecycleCoordinator.isForeground()) {
          return;
        }
        if (this.liveReplyRenderTimer !== undefined) {
          clearTimeout(this.liveReplyRenderTimer);
          this.liveReplyRenderTimer = undefined;
        }
        this.liveReplyText = this.pendingLiveReplyText;
        this.liveReplyActive = false;
        this.liveReplyAwaitingFinalSnapshot = this.liveReplyText.length > 0;
        this.liveReplyNotice = '本轮生成已结束，正在核对完整最终答复…';
        void this.refreshSelectedTask(taskId);
      },
      (connected: boolean, notice: string): void => {
        if (generation !== this.liveReplyGeneration || !this.pageMounted ||
          this.activePage !== 'detail' || this.selectedTaskId !== taskId ||
          !AppLifecycleCoordinator.isForeground()) {
          return;
        }
        this.liveReplyConnected = connected;
        this.liveReplyNotice = notice;
        if (connected) {
          if (this.liveReplyReconnectTimer !== undefined) {
            clearTimeout(this.liveReplyReconnectTimer);
            this.liveReplyReconnectTimer = undefined;
          }
          // 每次建立连接都补拉快照，覆盖离线期间完成且已不在流缓存中的回复。
          void this.refreshSelectedTask(taskId);
        }
        if (!connected && this.liveReplyReconnectTimer === undefined && this.pcIsOnline() &&
          AppLifecycleCoordinator.isForeground()) {
          // 断流时立即补拉；后续详情轮询不依赖实时连接是否恢复。
          void this.refreshSelectedTask(taskId);
          this.liveReplyReconnectTimer = setTimeout((): void => {
            this.liveReplyReconnectTimer = undefined;
            if (this.activePage === 'detail' && this.selectedTaskId === taskId && this.pcIsOnline() &&
              AppLifecycleCoordinator.isForeground()) {
              this.startLiveReplyStream(true);
            }
          }, Math.min(30000, 1000 * Math.pow(2, this.liveReplyReconnectAttempt++)));
        }
      });
  }

  private stopLiveReplyStream(preservePageState: boolean): void {
    this.liveReplyGeneration++;
    LiveReplyStreamCoordinator.disconnect();
    if (this.liveReplyRenderTimer !== undefined) {
      clearTimeout(this.liveReplyRenderTimer);
      this.liveReplyRenderTimer = undefined;
    }
    if (this.liveReplyReconnectTimer !== undefined) {
      clearTimeout(this.liveReplyReconnectTimer);
      this.liveReplyReconnectTimer = undefined;
    }
    if (preservePageState) {
      this.liveReplyText = this.pendingLiveReplyText || this.liveReplyText;
      this.liveReplyAwaitingFinalSnapshot = this.liveReplyText.length > 0;
    } else {
      this.pendingLiveReplyText = '';
      this.liveReplyText = '';
      this.liveReplyObservedAt = '';
      this.liveReplyReconnectAttempt = 0;
      this.liveReplyAwaitingFinalSnapshot = false;
    }
    this.liveReplyActive = false;
    this.liveReplyConnected = false;
    this.liveReplyNotice = preservePageState ? '应用回到前台后将恢复实时回复' : '';
  }

  private async refreshSelectedTask(taskId: string): Promise<void> {
    const registration: DeviceRegistrationResult | undefined = this.deviceRegistration;
    const task: RemoteTaskSnapshot | undefined = this.selectedTask();
    if (!this.pageMounted || registration === undefined || task === undefined || task.remoteTaskId !== taskId ||
      this.activePage !== 'detail' || !this.pcIsOnline() || !AppLifecycleCoordinator.isForeground()) {
      return;
    }
    if (this.refreshingSelectedTask) {
      this.selectedTaskRefreshPending = true;
      return;
    }
    const generation: number = this.taskDetailGeneration;
    const visibilityGeneration: number = this.taskVisibilityGeneration;
    this.refreshingSelectedTask = true;
    try {
      const response = await TaskSyncCoordinator.getTask(registration, task);
      if (!this.pageMounted || generation !== this.taskDetailGeneration || visibilityGeneration !== this.taskVisibilityGeneration ||
        this.activePage !== 'detail' || this.selectedTaskId !== taskId || !this.pcIsOnline() ||
        !AppLifecycleCoordinator.isForeground()) {
        return;
      }
      if (response.snapshot.pcConnectionState !== PcConnectionState.ONLINE) {
        this.connectionState = response.snapshot.pcConnectionState;
        this.hideTasksForDisconnectedPc();
        return;
      }
      this.tasks = this.mergeTaskPage(this.tasks, [response.snapshot]);
      this.lastTaskSyncAt = response.serverReceivedAt;
      this.calibrateLiveReply();
      if (this.liveReplyAwaitingFinalSnapshot) {
        this.liveReplyNotice = '回复记录仍在同步，将自动重试';
      } else if (!this.liveReplyConnected) {
        this.liveReplyNotice = '回复记录已同步，正在恢复实时连接';
      }
    } catch (_error) {
      if (this.pageMounted && AppLifecycleCoordinator.isForeground() &&
        generation === this.taskDetailGeneration && visibilityGeneration === this.taskVisibilityGeneration &&
        this.activePage === 'detail' && this.selectedTaskId === taskId) {
        this.liveReplyNotice = '任务详情同步暂未完成，将自动重试';
        // 详情可能已被取消同步，通过列表复核删除状态，避免继续操作失效任务。
        await this.refreshTasks(false);
      }
    } finally {
      if (generation === this.taskDetailGeneration) {
        this.refreshingSelectedTask = false;
        if (this.selectedTaskRefreshPending) {
          this.selectedTaskRefreshPending = false;
          void this.refreshSelectedTask(taskId);
        }
      }
    }
  }

  private handleTaskPageBackTouch(event: TouchEvent): void {
    if ((this.activePage !== 'detail' && this.activePage !== 'create') ||
      this.commandSubmitting || event.touches.length === 0) {
      return;
    }
    const touch = event.touches[0];
    if (event.type === TouchType.Down) {
      this.taskPageBackTouchStartX = touch.windowX;
      this.taskPageBackTouchStartY = touch.windowY;
      this.taskPageBackGestureHandled = false;
      return;
    }
    if (event.type !== TouchType.Move || this.taskPageBackGestureHandled) {
      return;
    }
    const horizontalDistance = touch.windowX - this.taskPageBackTouchStartX;
    const verticalDistance = Math.abs(touch.windowY - this.taskPageBackTouchStartY);
    const leftEdgeSwipeRight = this.taskPageBackTouchStartX <= 32 &&
      horizontalDistance >= 96 && horizontalDistance > verticalDistance;
    const swipeLeft = horizontalDistance <= -96 && -horizontalDistance > verticalDistance;
    if (leftEdgeSwipeRight || swipeLeft) {
      this.taskPageBackGestureHandled = true;
      this.leaveTaskPage();
    }
  }

  private pcDeviceRevokeOffset(device: PcDeviceSummary): number {
    const deviceKey: string = this.pcDeviceKey(device);
    if (this.swipingPcDeviceKey === deviceKey) {
      return this.pcDeviceSwipeOffset;
    }
    return this.revealedPcDeviceKey === deviceKey ? -PC_DEVICE_REVOKE_ACTION_WIDTH : 0;
  }

  private pcDeviceCardHeight(device: PcDeviceSummary): number {
    const deviceKey: string = this.pcDeviceKey(device);
    for (let index: number = 0; index < this.pcDeviceCardLayouts.length; index++) {
      const layout: PcDeviceCardLayout = this.pcDeviceCardLayouts[index];
      if (layout.deviceKey === deviceKey) {
        return layout.height;
      }
    }
    return 0;
  }

  private updatePcDeviceCardHeight(device: PcDeviceSummary, height: Length): void {
    const measuredHeight: number = typeof height === 'number' ? height : Number.parseFloat(height.toString());
    if (!Number.isFinite(measuredHeight) || measuredHeight <= 0) {
      return;
    }
    const deviceKey: string = this.pcDeviceKey(device);
    const nextLayouts: PcDeviceCardLayout[] = [];
    let updated: boolean = false;
    for (let index: number = 0; index < this.pcDeviceCardLayouts.length; index++) {
      const layout: PcDeviceCardLayout = this.pcDeviceCardLayouts[index];
      if (layout.deviceKey === deviceKey) {
        if (Math.abs(layout.height - measuredHeight) < 0.5) {
          return;
        }
        nextLayouts.push({ deviceKey, height: measuredHeight });
        updated = true;
      } else {
        nextLayouts.push(layout);
      }
    }
    if (!updated) {
      nextLayouts.push({ deviceKey, height: measuredHeight });
    }
    this.pcDeviceCardLayouts = nextLayouts;
  }

  private handlePcDeviceRevokeSwipe(device: PcDeviceSummary, event: TouchEvent): void {
    if (this.commandSubmitting || this.revokingPcDeviceKey.length > 0 ||
      this.revokeConfirmationPcDeviceKey.length > 0) {
      return;
    }
    const deviceKey: string = this.pcDeviceKey(device);
    if (event.type === TouchType.Up || event.type === TouchType.Cancel) {
      const shouldReveal: boolean = event.type === TouchType.Up &&
        this.swipingPcDeviceKey === deviceKey &&
        this.pcDeviceSwipeOffset <= -PC_DEVICE_REVOKE_REVEAL_THRESHOLD;
      this.revealedPcDeviceKey = shouldReveal ? deviceKey : '';
      this.swipingPcDeviceKey = '';
      this.pcDeviceSwipeOffset = 0;
      return;
    }
    if (event.touches.length === 0) {
      return;
    }
    const touch = event.touches[0];
    if (event.type === TouchType.Down) {
      this.revealedPcDeviceKey = this.revealedPcDeviceKey === deviceKey ? deviceKey : '';
      this.swipingPcDeviceKey = deviceKey;
      this.pcDeviceRevokeTouchStartX = touch.windowX;
      this.pcDeviceRevokeTouchStartY = touch.windowY;
      this.pcDeviceRevokeTouchStartOffset = this.revealedPcDeviceKey === deviceKey ?
        -PC_DEVICE_REVOKE_ACTION_WIDTH : 0;
      this.pcDeviceSwipeOffset = this.pcDeviceRevokeTouchStartOffset;
      return;
    }
    if (event.type !== TouchType.Move || this.swipingPcDeviceKey !== deviceKey) {
      return;
    }
    const horizontalDistance: number = touch.windowX - this.pcDeviceRevokeTouchStartX;
    const verticalDistance: number = Math.abs(touch.windowY - this.pcDeviceRevokeTouchStartY);
    if (Math.abs(horizontalDistance) <= verticalDistance) {
      return;
    }
    const nextOffset: number = this.pcDeviceRevokeTouchStartOffset + horizontalDistance;
    this.pcDeviceSwipeOffset = Math.max(-PC_DEVICE_REVOKE_ACTION_WIDTH, Math.min(0, nextOffset));
  }

  private refreshCreateState(): void {
    const modelConfigs: ModelConfigSummary[] = this.activePcDevice?.modelConfigs ?? [];
    const workspaces: WorkspaceChoice[] = this.workspaceChoices();
    if (this.selectedWorkspaceSource() === undefined) {
      this.selectedWorkspaceSourceTaskId = workspaces.length === 0 ? '' : workspaces[0].sourceTaskId;
    }
    if (this.selectedModelConfig() === undefined) {
      this.selectedModelConfigId = modelConfigs.length === 0 ? '' : modelConfigs[0].id;
    }
    this.createNotice = this.activePcDevice === undefined ?
      '当前没有已绑定电脑，请返回首页完成绑定' :
      (this.connectionState !== PcConnectionState.ONLINE ?
      '电脑当前不在线；恢复连接后即可创建任务' :
      (workspaces.length === 0 ?
      '当前还没有可用于远程新建的工作区；请先在电脑端创建一个工作区任务' :
      (modelConfigs.length === 0 ?
      '当前电脑还没有已配置的模型，请先在电脑端配置模型' :
      '选择只保存在当前页面；提交第一条指令后才会创建任务')));
  }

  private async openCreateTask(): Promise<void> {
    this.activePage = 'create';
    this.createFailure = undefined;
    this.refreshCreateState();
    if (this.deviceRegistration === undefined || this.activePcDevice === undefined || !this.pcIsOnline()) {
      return;
    }
    const refreshModels: boolean = this.activePcDevice.modelConfigs.length === 0;
    const refreshWorkspaces: boolean = this.workspaceChoices().length === 0;
    if (refreshModels) {
      this.createNotice = '正在刷新电脑模型配置…';
      await this.refreshBindingStatus(false);
    }
    if (this.activePage !== 'create') {
      return;
    }
    if (refreshWorkspaces && this.pcIsOnline()) {
      this.createNotice = '正在刷新可用工作区…';
      await this.refreshTasks(false);
    }
    if (this.activePage === 'create') {
      this.refreshCreateState();
    }
  }

  private canCreateTask(): boolean {
    return this.deviceRegistration !== undefined && this.activePcDevice !== undefined &&
      this.connectionState === PcConnectionState.ONLINE && !this.commandSubmitting &&
      this.selectedWorkspaceSource() !== undefined &&
      this.newTaskInstructionBytes() >= 1 && this.newTaskInstructionBytes() <= 8192 &&
      this.selectedModelConfig() !== undefined;
  }

  private async createTask(): Promise<void> {
    const registration: DeviceRegistrationResult | undefined = this.deviceRegistration;
    const workspaceSource: RemoteTaskSnapshot | undefined = this.selectedWorkspaceSource();
    const modelConfig: ModelConfigSummary | undefined = this.selectedModelConfig();
    if (registration === undefined || workspaceSource === undefined || modelConfig === undefined || !this.canCreateTask()) {
      this.createNotice = '请连接当前电脑、选择工作区与模型，并填写第一条指令';
      return;
    }
    this.commandSubmitting = true;
    this.createFailure = undefined;
    this.createNotice = '正在创建任务并提交第一条指令…';
    try {
      const accepted: RemoteCommandRecord = await RemoteCommandCoordinator.createTask(
        registration, workspaceSource, modelConfig.id, this.derivedTaskName(), this.newTaskInstruction);
      const result: RemoteCommandRecord = await RemoteCommandCoordinator.waitForTerminal(registration, accepted);
      this.createFailure = RemoteCommandCoordinator.resultFailure(result);
      this.createNotice = this.createFailure === undefined ? RemoteCommandCoordinator.resultNotice(result) :
        '任务尚未创建';
      if (result.status === CommandStatus.COMPLETED) {
        this.newTaskInstruction = '';
        await this.refreshTasks(false);
        this.activePage = 'home';
        this.homeSection = 'tasks';
      }
    } catch (error) {
      const failure: RemoteFailureNotice = RemoteApiClient.describeFailure(
        error, '创建任务失败，请刷新后重试', '当前草稿已保留，按提示处理后点击“创建并发送”重试。');
      this.createFailure = failure;
      this.createNotice = '任务尚未创建';
    } finally {
      this.commandSubmitting = false;
    }
  }

  private canSendSelected(): boolean {
    const task: RemoteTaskSnapshot | undefined = this.selectedTask();
    if (task === undefined || this.deviceRegistration === undefined || this.commandSubmitting ||
      this.pairingState !== PairingState.ACTIVE) {
      return false;
    }
    if (task.taskStatus === 'created') {
      return this.connectionState === PcConnectionState.ONLINE;
    }
    return task.taskStatus === 'running' || task.taskStatus === 'stopped' || task.taskStatus === 'failed';
  }

  private canStopSelected(): boolean {
    const task: RemoteTaskSnapshot | undefined = this.selectedTask();
    if (task === undefined || this.deviceRegistration === undefined || this.commandSubmitting ||
      this.pairingState !== PairingState.ACTIVE || this.connectionState !== PcConnectionState.ONLINE) {
      return false;
    }
    return task.taskStatus === 'queued' || task.taskStatus === 'starting' || task.taskStatus === 'running' ||
      task.taskStatus === 'pausing' || task.taskStatus === 'paused';
  }

  private async sendInstruction(): Promise<void> {
    const task: RemoteTaskSnapshot | undefined = this.selectedTask();
    const registration: DeviceRegistrationResult | undefined = this.deviceRegistration;
    if (task === undefined || registration === undefined || !this.canSendSelected()) {
      this.commandNotice = '请选择当前可接收消息的任务';
      return;
    }
    this.commandSubmitting = true;
    this.commandFailure = undefined;
    this.commandNotice = '正在安全提交消息…';
    try {
      const accepted: RemoteCommandRecord = await RemoteCommandCoordinator.submitText(
        registration, task, this.instruction);
      this.commandNotice = '消息已提交，正在等待电脑处理…';
      const result: RemoteCommandRecord = await RemoteCommandCoordinator.waitForTerminal(registration, accepted);
      this.commandFailure = RemoteCommandCoordinator.resultFailure(result);
      this.commandNotice = this.commandFailure === undefined ? RemoteCommandCoordinator.resultNotice(result) :
        '消息未完成';
      if (result.status === CommandStatus.COMPLETED) {
        this.instruction = '';
      }
      await this.refreshTasks(false);
    } catch (error) {
      const failure: RemoteFailureNotice = RemoteApiClient.describeFailure(
        error, '消息提交失败，请刷新后重试', '当前输入已保留，按提示处理后重新发送。');
      this.commandFailure = failure;
      this.commandNotice = '消息尚未提交';
    } finally {
      this.commandSubmitting = false;
    }
  }

  private async stopSelectedTask(): Promise<void> {
    const task: RemoteTaskSnapshot | undefined = this.selectedTask();
    const registration: DeviceRegistrationResult | undefined = this.deviceRegistration;
    if (task === undefined || registration === undefined || !this.canStopSelected()) {
      this.commandNotice = '请选择当前可停止且电脑在线的任务';
      return;
    }
    this.commandSubmitting = true;
    this.commandFailure = undefined;
    this.commandNotice = '正在安全提交停止请求…';
    try {
      const accepted: RemoteCommandRecord = await RemoteCommandCoordinator.stopTask(registration, task);
      const result: RemoteCommandRecord = await RemoteCommandCoordinator.waitForTerminal(registration, accepted);
      this.commandFailure = RemoteCommandCoordinator.resultFailure(result);
      this.commandNotice = this.commandFailure === undefined ? RemoteCommandCoordinator.resultNotice(result) :
        '停止请求未完成';
      await this.refreshTasks(false);
    } catch (error) {
      const failure: RemoteFailureNotice = RemoteApiClient.describeFailure(
        error, '停止请求失败，请刷新后重试', '刷新任务状态后重新提交停止请求。');
      this.commandFailure = failure;
      this.commandNotice = '停止请求尚未提交';
    } finally {
      this.commandSubmitting = false;
    }
  }

  private runningTaskCount(): number {
    let count: number = 0;
    for (let index: number = 0; index < this.tasks.length; index++) {
      if (this.taskIsRunning(this.tasks[index])) {
        count++;
      }
    }
    return count;
  }

  private attentionTaskCount(): number {
    let count: number = 0;
    for (let index: number = 0; index < this.tasks.length; index++) {
      if (this.taskNeedsAttention(this.tasks[index])) {
        count++;
      }
    }
    return count;
  }

  private pageGutter(): number {
    if (this.viewportWidth >= 1024) {
      return 40;
    }
    if (this.viewportWidth >= 768) {
      return 24;
    }
    return 16;
  }

  private updateViewportWidth(width: Length): void {
    const measuredWidth: number = typeof width === 'number' ? width : Number.parseFloat(width.toString());
    if (Number.isFinite(measuredWidth) && measuredWidth > 0 && Math.abs(this.viewportWidth - measuredWidth) >= 1) {
      this.viewportWidth = measuredWidth;
    }
  }

  private showOverview(): void {
    this.homeSection = 'overview';
  }

  private showTasks(): void {
    this.homeSection = 'tasks';
    if (this.pcIsOnline() && !this.refreshingTasks) {
      void this.refreshTasks(false);
    }
  }

  private homeFailure(): RemoteFailureNotice | undefined {
    if (this.pairingFailure !== undefined) {
      return this.pairingFailure;
    }
    if (this.registrationFailure !== undefined) {
      return this.registrationFailure;
    }
    return this.bindingFailure;
  }

  @Builder
  failureCard(failure: RemoteFailureNotice) {
    Column({ space: 7 }) {
      Row({ space: 8 }) {
        Text(failure.title)
          .fontFamily(FONT_SYSTEM_SANS)
          .fontSize(15)
          .fontWeight(FontWeight.Bold)
          .fontColor(COLOR_DANGER)
          .layoutWeight(1)
        Text(failure.retryable ? '可重试' : '需处理')
          .fontFamily(FONT_SYSTEM_SANS)
          .fontSize(12)
          .fontWeight(FontWeight.Medium)
          .fontColor(failure.retryable ? COLOR_WARNING_TEXT : COLOR_DANGER)
          .backgroundColor(failure.retryable ? COLOR_WARNING_SOFT : COLOR_DANGER_SOFT)
          .borderRadius(8)
          .padding({ left: 8, right: 8, top: 3, bottom: 3 })
      }
      .width('100%')

      Text(failure.reason)
        .fontFamily(FONT_SYSTEM_SANS)
        .fontSize(14)
        .fontColor(COLOR_INK_BODY)
        .width('100%')

      Text(failure.status)
        .fontFamily(FONT_SYSTEM_SANS)
        .fontSize(12)
        .fontWeight(FontWeight.Medium)
        .fontColor(COLOR_WARNING_TEXT)
        .width('100%')

      Text(`下一步：${failure.action}`)
        .fontFamily(FONT_SYSTEM_SANS)
        .fontSize(13)
        .fontColor(COLOR_INK_SECONDARY)
        .width('100%')
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
    .padding({ left: 14, right: 14, top: 12, bottom: 12 })
    .backgroundColor(COLOR_DANGER_SOFT)
    .border({ width: { left: 4 }, color: COLOR_DANGER })
    .borderRadius(0)
  }

  @Builder
  private bottomNavigation() {
    Column() {
      Divider()
        .color(COLOR_RULE)
        .strokeWidth(1)
      Row({ space: 8 }) {
        Button('概览')
          .fontFamily(FONT_SYSTEM_SANS)
          .type(ButtonType.Normal)
          .layoutWeight(1)
          .height(52)
          .fontSize(14)
          .fontWeight(this.homeSection === 'overview' ? FontWeight.Bold : FontWeight.Medium)
          .fontColor(this.homeSection === 'overview' ? COLOR_ACCENT : COLOR_INK_SECONDARY)
          .backgroundColor(this.homeSection === 'overview' ? COLOR_ACCENT_SOFT : Color.Transparent)
          .borderRadius(6)
          .stateEffect(true)
          .accessibilityText(this.homeSection === 'overview' ? '概览，当前页面' : '打开概览')
          .onClick(() => {
            this.showOverview();
          })
        Button('任务列表')
          .fontFamily(FONT_SYSTEM_SANS)
          .type(ButtonType.Normal)
          .layoutWeight(1)
          .height(52)
          .fontSize(14)
          .fontWeight(this.homeSection === 'tasks' ? FontWeight.Bold : FontWeight.Medium)
          .fontColor(this.homeSection === 'tasks' ? COLOR_ACCENT : COLOR_INK_SECONDARY)
          .backgroundColor(this.homeSection === 'tasks' ? COLOR_ACCENT_SOFT : Color.Transparent)
          .borderRadius(6)
          .stateEffect(true)
          .accessibilityText(this.homeSection === 'tasks' ? `任务列表，当前页面，共 ${this.tasks.length} 个任务` :
            `打开任务列表，共 ${this.tasks.length} 个任务`)
          .onClick(() => {
            this.showTasks();
          })
        Button('新建')
          .fontFamily(FONT_SYSTEM_SANS)
          .type(ButtonType.Normal)
          .layoutWeight(1)
          .height(52)
          .fontSize(14)
          .fontWeight(FontWeight.Medium)
          .fontColor(COLOR_SURFACE)
          .backgroundColor(COLOR_ACCENT)
          .borderRadius(6)
          .stateEffect(true)
          .enabled(!this.commandSubmitting)
          .accessibilityText('新建任务')
          .onClick(() => {
            this.openCreateTask();
          })
      }
      .width('100%')
      .constraintSize({ maxWidth: 840 })
      .padding({ left: this.pageGutter(), right: this.pageGutter(), top: 8, bottom: 8 })
    }
    .alignItems(HorizontalAlign.Center)
    .width('100%')
    .backgroundColor(COLOR_SURFACE)
  }

  @Builder
  private pageContent() {
      Column({ space: 20 }) {
        Row() {
          Column({ space: 4 }) {
            Text(this.activePage === 'home' ? (this.homeSection === 'overview' ? '轩++远程' : '任务') :
              (this.activePage === 'detail' ? (this.selectedTask()?.name || '未命名任务') : '新建任务'))
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(this.activePage === 'detail' ? 20 : 30)
              .fontWeight(FontWeight.Bold)
              .fontColor(COLOR_INK)
              .maxLines(2)
              .textOverflow({ overflow: TextOverflow.Ellipsis })
            if (this.activePage === 'home') {
            Text(this.homeSection === 'overview' ? '开发环境 · API 20+ · 0.1.9-dev' :
              `${this.runningTaskCount()} 个运行中 · ${this.attentionTaskCount()} 个需要关注`)
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(13)
              .fontColor(COLOR_MUTED)
            }
          }
          .alignItems(HorizontalAlign.Start)
          .layoutWeight(1)

          if (this.activePage !== 'home') {
            Button('返回')
              .fontFamily(FONT_SYSTEM_SANS)
              .type(ButtonType.Normal)
              .fontSize(14)
              .height(48)
              .fontColor(COLOR_ACCENT)
              .backgroundColor(COLOR_ACCENT_SOFT)
              .borderRadius(6)
              .stateEffect(true)
              .accessibilityText(this.activePage === 'detail' ? '返回任务列表' : '返回任务列表')
              .margin({ right: 8 })
              .enabled(!this.commandSubmitting)
              .onClick(() => {
                this.leaveTaskPage();
              })
          }

          if (this.activePage === 'home' && this.homeSection === 'overview') {
          Text(this.environment === RemoteEnvironment.DEV ? 'DEV' : 'PROD')
            .fontFamily(FONT_SYSTEM_SANS)
            .fontSize(12)
            .fontWeight(FontWeight.Bold)
            .fontColor(COLOR_SURFACE)
            .backgroundColor(COLOR_ACCENT)
            .borderRadius(2)
            .padding({ left: 10, right: 10, top: 5, bottom: 5 })
          }
        }
        .width('100%')
        .alignItems(VerticalAlign.Center)
        .flexShrink(0)

        if (this.activePage === 'home') {
        if (this.homeSection === 'overview') {
        Column({ space: 12 }) {
          Column() {
            Row({ space: 12 }) {
              Text('01')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(12)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_ACCENT)
                .width(32)
              Column({ space: 3 }) {
                Text('手机身份')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .fontSize(13)
                  .fontColor(COLOR_MUTED)
                Text(this.deviceRegistered ? '已连接开发云端' :
                  (this.registeringDevice ? '正在登记设备' : '尚未连接'))
                  .fontFamily(FONT_SYSTEM_SANS)
                  .fontSize(16)
                  .fontWeight(FontWeight.Bold)
                  .fontColor(this.deviceRegistered ? COLOR_INK : COLOR_WARNING_TEXT)
              }
              .alignItems(HorizontalAlign.Start)
              .layoutWeight(1)
            }
            .width('100%')
            .constraintSize({ minHeight: 64 })
            .padding({ top: 12, bottom: 12 })
            .border({ width: { bottom: 1 }, color: COLOR_RULE })

            Row({ space: 12 }) {
              Text('02')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(12)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_ACCENT)
                .width(32)
              Column({ space: 3 }) {
                Text('当前电脑')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .fontSize(13)
                  .fontColor(COLOR_MUTED)
                Text(this.activePcDevice === undefined ? '尚未绑定' :
                  `${this.activePcDevice.displayName} · ${this.connectionStateText(this.connectionState)}`)
                  .fontFamily(FONT_SYSTEM_SANS)
                  .fontSize(16)
                  .fontWeight(FontWeight.Bold)
                  .fontColor(this.activePcDevice === undefined ? COLOR_WARNING_TEXT :
                    this.connectionStateColor(this.connectionState))
                  .maxLines(2)
              }
              .alignItems(HorizontalAlign.Start)
              .layoutWeight(1)
              Text(this.activePcDevice === undefined ? '绑定' : '查看')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_ACCENT)
            }
            .width('100%')
            .constraintSize({ minHeight: 72 })
            .padding({ top: 12, bottom: 12 })
            .border({ width: { bottom: 1 }, color: COLOR_RULE })
            .accessibilityText(this.activePcDevice === undefined ? '尚未绑定电脑，点击开始绑定' :
              `当前电脑 ${this.activePcDevice.displayName}，${this.connectionStateText(this.connectionState)}，点击查看状态`)
            .onClick(() => {
              if (this.activePcDevice === undefined) {
                void this.scanPairingQr();
              } else {
                this.connectionDetailsExpanded = !this.connectionDetailsExpanded;
              }
            })

            Row({ space: 12 }) {
              Text('03')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(12)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_ACCENT)
                .width(32)
              Column({ space: 3 }) {
                Text('任务同步')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .fontSize(13)
                  .fontColor(COLOR_MUTED)
                Text(`${this.tasks.length} 个任务 · ${this.runningTaskCount()} 个运行中`)
                  .fontFamily(FONT_SYSTEM_SANS)
                  .fontSize(16)
                  .fontWeight(FontWeight.Bold)
                  .fontColor(COLOR_INK)
              }
              .alignItems(HorizontalAlign.Start)
              .layoutWeight(1)
              Text('打开')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_ACCENT)
            }
            .width('100%')
            .constraintSize({ minHeight: 72 })
            .padding({ top: 12, bottom: 12 })
            .accessibilityText(`打开任务，共 ${this.tasks.length} 个，${this.runningTaskCount()} 个运行中`)
            .onClick(() => {
              this.showTasks();
            })
          }
          .alignItems(HorizontalAlign.Start)
          .width('100%')
          .padding({ left: 14, right: 14 })
          .backgroundColor(COLOR_SURFACE)
          .border({ width: 1, color: COLOR_RULE })
          .borderRadius(2)

          Text('电脑与通知')
            .fontFamily(FONT_SYSTEM_SANS)
            .fontSize(14)
            .fontWeight(FontWeight.Bold)
            .fontColor(COLOR_INK)
            .width('100%')
            .margin({ top: 8 })

          Row({ space: 10 }) {
            Column({ space: 3 }) {
              Text(this.activePcDevice === undefined ? '当前电脑' : `当前电脑 · ${this.activePcDevice.displayName}`)
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(18)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_INK)
                .maxLines(2)
                .accessibilityText(this.activePcDevice === undefined ? '当前电脑，尚未绑定' :
                  `当前电脑，${this.activePcDevice.displayName}`)
              Row({ space: 6 }) {
                Blank()
                  .width(4)
                  .height(18)
                  .backgroundColor(this.activePcDevice === undefined ? COLOR_DISABLED :
                    this.connectionStateColor(this.connectionState))
                Text(this.activePcDevice === undefined ? '尚未绑定电脑' :
                  (this.bindingRefreshError.length > 0 ? '状态刷新失败' :
                    this.connectionStateText(this.connectionState)))
                  .fontFamily(FONT_SYSTEM_SANS)
                  .fontSize(13)
                  .fontColor(this.activePcDevice === undefined ? COLOR_MUTED :
                    this.connectionStateColor(this.connectionState))
              }
            }
            .alignItems(HorizontalAlign.Start)
            .layoutWeight(1)
          }
          .width('100%')
          .alignItems(VerticalAlign.Center)

          if (this.activePcDevice !== undefined || this.pcDevices.length > 1) {
            Row({ space: 8 }) {
              if (this.activePcDevice !== undefined) {
                Button(this.connectionDetailsExpanded ? '收起状态' : '连接状态')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .type(ButtonType.Normal)
                  .borderRadius(6)
                  .fontSize(13)
                  .height(48)
                  .layoutWeight(1)
                  .accessibilityText(this.connectionDetailsExpanded ? '收起电脑连接状态' : '查看电脑连接状态')
                  .fontColor(COLOR_INK_SECONDARY)
                  .backgroundColor(COLOR_SURFACE_CONTROL)
                  .onClick(() => {
                    this.connectionDetailsExpanded = !this.connectionDetailsExpanded;
                  })
              }
              if (this.pcDevices.length > 1) {
                Button(this.deviceListExpanded ? '完成' : '切换设备')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .type(ButtonType.Normal)
                  .borderRadius(6)
                  .fontSize(13)
                  .height(48)
                  .layoutWeight(1)
                  .accessibilityText(this.deviceListExpanded ? '完成设备切换' : '打开电脑切换列表')
                  .enabled(!this.commandSubmitting)
                  .onClick(() => {
                    this.deviceListExpanded = !this.deviceListExpanded;
                  })
              }
            }
            .width('100%')
          }

          if (this.activePcDevice !== undefined &&
            (this.connectionDetailsExpanded || !this.isConnected() || this.bindingRefreshError.length > 0)) {
            Column({ space: 7 }) {
              Text(this.connectionHelpText())
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontColor(this.isConnected() && this.bindingRefreshError.length === 0 ? COLOR_INK_SECONDARY : COLOR_WARNING_TEXT)
                .width('100%')
              Text(`电脑最近观测：${formatChinaDateTime(this.activePcDevice.pcObservedAt)}`)
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(12)
                .fontColor(COLOR_MUTED_LIGHT)
                .width('100%')
              Button(this.refreshingBinding ? '刷新中…' : '重新检测连接')
                .fontFamily(FONT_SYSTEM_SANS)
                .type(ButtonType.Normal)
                .borderRadius(6)
                .fontSize(12)
                .height(48)
                .enabled(!this.refreshingBinding)
                .onClick(() => {
                  this.refreshBindingStatus(true);
                })
            }
            .width('100%')
            .padding(12)
            .backgroundColor(this.isConnected() && this.bindingRefreshError.length === 0 ? COLOR_SURFACE_INPUT : COLOR_WARNING_SOFT)
            .border({ width: { left: 4 }, color: this.isConnected() && this.bindingRefreshError.length === 0 ?
              COLOR_ACCENT : COLOR_WARNING })
            .borderRadius(0)
          }

          if (this.activePcDevice !== undefined && (!this.notificationPermissionKnown || !this.notificationEnabled)) {
            Column({ space: 8 }) {
              Text('开启任务结束通知')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(15)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_INK_STRONG)
              Text(this.notificationPermissionKnown ?
                '通知未开启。通知只提示任务状态变化，不包含任务名称、答复或内部标识。' :
                '暂时无法确认通知设置。任务结束通知只提示状态变化，不包含任务名称、答复或内部标识。')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontColor(COLOR_INK_SECONDARY)
                .width('100%')
              Button(this.notificationPermissionChecking ? '正在检查通知设置…' : '前往系统通知设置')
                .fontFamily(FONT_SYSTEM_SANS)
                .type(ButtonType.Normal)
                .borderRadius(6)
                .fontSize(13)
                .height(48)
                .width('100%')
                .fontColor(COLOR_ACCENT)
                .backgroundColor(COLOR_ACCENT_SOFT)
                .enabled(!this.notificationPermissionChecking)
                .accessibilityText('前往系统通知设置，开启任务结束通知')
                .onClick(() => {
                  this.requestNotificationPermission();
                })
              Button('重新检测通知设置')
                .fontFamily(FONT_SYSTEM_SANS)
                .type(ButtonType.Normal)
                .borderRadius(6)
                .fontSize(13)
                .height(48)
                .width('100%')
                .fontColor(COLOR_INK_SECONDARY)
                .backgroundColor(COLOR_SURFACE_CONTROL)
                .enabled(!this.notificationPermissionChecking)
                .accessibilityText('重新检测任务结束通知设置')
                .onClick(() => {
                  this.refreshNotificationPermission();
                })
            }
            .alignItems(HorizontalAlign.Start)
            .width('100%')
            .padding(12)
            .backgroundColor(COLOR_WARNING_SOFT)
            .border({ width: { left: 4 }, color: COLOR_WARNING })
            .borderRadius(0)
          }

          if (this.deviceListExpanded) {
            Column({ space: 8 }) {
              Text(`已绑定设备 · ${this.pcDevices.length} 台`)
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_INK_SECONDARY)
                .width('100%')
              Text('向左滑动电脑卡片显示撤销绑定按钮')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(12)
                .fontColor(COLOR_MUTED)
                .width('100%')
              Column({ space: 8 }) {
                ForEach(this.pcDevices, (device: PcDeviceSummary) => {
                  Stack({ alignContent: Alignment.End }) {
                    if (this.pcDeviceRevokeOffset(device) < 0 && this.pcDeviceCardHeight(device) > 0) {
                      Button(this.revokingPcDeviceKey === this.pcDeviceKey(device) ? '正在撤销…' : '撤销绑定')
                        .fontFamily(FONT_SYSTEM_SANS)
                        .type(ButtonType.Normal)
                        .fontSize(14)
                        .fontWeight(FontWeight.Medium)
                        .fontColor(COLOR_SURFACE)
                        .backgroundColor(COLOR_DANGER)
                        .height(this.pcDeviceCardHeight(device))
                        .width(PC_DEVICE_REVOKE_ACTION_WIDTH)
                        .borderRadius(6)
                        .enabled(!this.commandSubmitting && this.revokingPcDeviceKey.length === 0 &&
                          this.revokeConfirmationPcDeviceKey.length === 0)
                        .accessibilityText(`撤销此手机与电脑 ${device.displayName} 的绑定`)
                        .onClick(() => {
                          void this.confirmRevokePcDevice(device);
                        })
                    }
                    Column({ space: 10 }) {
                      Row({ space: 9 }) {
                        Blank()
                          .width(4)
                          .height(28)
                          .backgroundColor(this.connectionStateColor(device.pcConnectionState))
                        Column({ space: 2 }) {
                        Text(device.displayName)
                          .fontFamily(FONT_SYSTEM_SANS)
                          .fontSize(14)
                          .fontWeight(FontWeight.Medium)
                          .fontColor(COLOR_INK)
                          .maxLines(2)
                          .accessibilityText(`电脑名称：${device.displayName}`)
                        Text(this.connectionStateText(device.pcConnectionState))
                          .fontFamily(FONT_SYSTEM_SANS)
                          .fontSize(12)
                          .fontColor(this.connectionStateColor(device.pcConnectionState))
                          .accessibilityText(`连接状态：${this.connectionStateText(device.pcConnectionState)}`)
                        }
                        .alignItems(HorizontalAlign.Start)
                        .layoutWeight(1)
                        if (this.pcDeviceKey(device) === this.activePcDeviceKey) {
                          Text('当前电脑')
                            .fontFamily(FONT_SYSTEM_SANS)
                            .fontSize(12)
                            .fontWeight(FontWeight.Medium)
                            .fontColor(COLOR_ACCENT)
                        }
                      }
                      .width('100%')
                      if (this.pcDeviceKey(device) !== this.activePcDeviceKey) {
                        Button('切换到此电脑')
                          .fontFamily(FONT_SYSTEM_SANS)
                          .type(ButtonType.Normal)
                          .borderRadius(6)
                          .fontSize(14)
                          .height(48)
                          .width('100%')
                          .fontColor(COLOR_SURFACE)
                          .backgroundColor(COLOR_ACCENT)
                          .enabled(!this.commandSubmitting && this.revokingPcDeviceKey.length === 0)
                          .accessibilityText(`切换到电脑：${device.displayName}`)
                          .onClick(() => {
                            this.selectPcDevice(device, true);
                            this.deviceListExpanded = false;
                          })
                      }
                      Button(this.revokingPcDeviceKey === this.pcDeviceKey(device) ? '正在撤销…' : '撤销绑定')
                        .fontFamily(FONT_SYSTEM_SANS)
                        .type(ButtonType.Normal)
                        .borderRadius(6)
                        .fontSize(14)
                        .height(48)
                        .width('100%')
                        .fontColor(COLOR_DANGER)
                        .backgroundColor(COLOR_DANGER_SOFT)
                        .enabled(!this.commandSubmitting && this.revokingPcDeviceKey.length === 0 &&
                          this.revokeConfirmationPcDeviceKey.length === 0)
                        .accessibilityText(`撤销此手机与电脑 ${device.displayName} 的绑定`)
                        .onClick(() => {
                          void this.confirmRevokePcDevice(device);
                        })
                    }
                    .width('100%')
                    .padding(12)
                    .backgroundColor(this.pcDeviceKey(device) === this.activePcDeviceKey ? COLOR_ACCENT_SOFT : COLOR_SURFACE_SUBDUED)
                    .border({ width: 1, color: this.pcDeviceKey(device) === this.activePcDeviceKey ? COLOR_ACCENT : COLOR_RULE })
                    .borderRadius(2)
                    .translate({ x: this.pcDeviceRevokeOffset(device) })
                    .onAreaChange((_oldValue: Area, newValue: Area) => {
                      this.updatePcDeviceCardHeight(device, newValue.height);
                    })
                    .accessibilityText(`电脑：${device.displayName}，可切换或撤销绑定；向左滑动也可显示撤销按钮`)
                    .onTouch((event: TouchEvent) => {
                      this.handlePcDeviceRevokeSwipe(device, event);
                    })
                  }
                  .width('100%')
                  .clip(true)
                }, (device: PcDeviceSummary) => this.pcDeviceKey(device))
              }
              .width('100%')
            }
            .width('100%')
          }

          if (this.activePcDevice === undefined || this.pendingSummary !== undefined ||
            this.bindingRefreshError.length > 0 || this.pairingError.length > 0) {
            if (this.homeFailure() !== undefined) {
              this.failureCard(this.homeFailure()!)
            } else {
              Text(this.notice)
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(14)
                .fontColor(COLOR_MUTED)
                .width('100%')
            }
          }

          if (!this.deviceRegistered || this.registeringDevice || this.bindingRefreshError.length > 0) {
            Button(this.registeringDevice ? '正在登记设备…' :
              (this.deviceRegistered ? '重新连接云端' : '连接开发云端'))
              .fontFamily(FONT_SYSTEM_SANS)
              .type(ButtonType.Normal)
              .borderRadius(6)
              .fontSize(14)
              .fontColor(COLOR_ACCENT)
              .backgroundColor(COLOR_ACCENT_SOFT)
              .width('100%')
              .height(48)
              .enabled(!this.registeringDevice)
              .onClick(() => {
                this.registerDevice();
              })
          }

          if (this.activePcDevice === undefined) {
            Button(this.scanning ? '正在绑定新电脑…' :
              (this.deviceRegistered ? '扫描二维码绑定新电脑' : '请先连接开发云端'))
              .fontFamily(FONT_SYSTEM_SANS)
              .type(ButtonType.Normal)
              .borderRadius(6)
              .fontSize(15)
              .fontWeight(FontWeight.Medium)
              .backgroundColor(COLOR_ACCENT)
              .width('100%')
              .height(50)
              .enabled(!this.scanning && this.deviceRegistered)
              .onClick(() => {
                this.scanPairingQr();
              })
          } else {
            Row() {
              Text(`${this.pcDevices.length} 台设备`)
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(12)
                .fontColor(COLOR_MUTED_LIGHT)
                .layoutWeight(1)
              Button(this.scanning ? '正在绑定…' : '绑定新电脑')
                .fontFamily(FONT_SYSTEM_SANS)
                .type(ButtonType.Normal)
                .borderRadius(6)
                .fontSize(13)
                .fontColor(COLOR_ACCENT)
                .backgroundColor(COLOR_ACCENT_SOFT)
                .height(48)
                .enabled(!this.scanning)
                .onClick(() => {
                  this.scanPairingQr();
                })
            }
            .width('100%')
            .alignItems(VerticalAlign.Center)
          }

          if (this.pcDevices.length === 0) {
            Text('每台电脑都需扫描其一次性二维码完成绑定；二维码不包含地址、端口、密钥或长期凭据。')
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(12)
              .fontColor(COLOR_MUTED_LIGHT)
              .width('100%')
          }

          if (this.pendingSummary !== undefined) {
            Column({ space: 8 }) {
              Text('正在绑定新电脑')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(16)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_WARNING)
              Text(`电脑：${this.pendingSummary.pcDisplayName}`)
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(14)
                .fontColor(COLOR_INK)
              Text(`手机：${this.pendingSummary.appDisplayName}`)
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(14)
                .fontColor(COLOR_INK)
              Text(this.pendingSummary.safetyPhrase)
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(22)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_ACCENT)
                .letterSpacing(1)
              Text('安全短语仅作为本次绑定审计摘要；电脑端会校验当前二维码关联后自动确认。')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(12)
                .fontColor(COLOR_MUTED_LIGHT)
            }
            .width('100%')
            .padding(14)
            .backgroundColor(COLOR_WARNING_SOFT)
            .border({ width: { left: 4 }, color: COLOR_WARNING })
            .borderRadius(0)
          }
        }
        .width('100%')
        .padding(0)
        .backgroundColor(COLOR_PAGE)
        .borderRadius(0)

        }

        if (this.homeSection === 'tasks') {

        Column({ space: 12 }) {
          Row() {
            Column({ space: 0 }) {
              Text(`${this.tasks.length}`)
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(44)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_INK)
              Text('全部任务')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontColor(COLOR_MUTED)
            }
            .alignItems(HorizontalAlign.Start)
            .layoutWeight(1)
            Button('新建')
              .fontFamily(FONT_SYSTEM_SANS)
              .type(ButtonType.Normal)
              .borderRadius(6)
              .fontSize(13)
              .fontColor(COLOR_SURFACE)
              .backgroundColor(COLOR_ACCENT)
              .height(48)
              .width(88)
              .stateEffect(true)
              .accessibilityText('新建任务')
              .onClick(() => {
                this.openCreateTask();
              })
          }
          .width('100%')
          .alignItems(VerticalAlign.Bottom)

          Row() {
            Text(this.taskNotice)
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(13)
              .fontColor(COLOR_MUTED)
              .layoutWeight(1)
            Button(this.refreshingTasks ? '刷新中…' : '刷新任务')
              .fontFamily(FONT_SYSTEM_SANS)
              .type(ButtonType.Normal)
              .borderRadius(6)
              .fontSize(12)
              .height(48)
              .enabled(!this.refreshingTasks && this.pairingState === PairingState.ACTIVE &&
                this.connectionState === PcConnectionState.ONLINE)
              .onClick(() => {
                this.refreshTasks(true);
              })
          }
          .width('100%')

          if (this.lastTaskSyncAt.length > 0) {
            Text(`任务最近同步：${formatChinaDateTime(this.lastTaskSyncAt)}`)
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(12)
              .fontColor(COLOR_MUTED_LIGHT)
              .width('100%')
          }

          if (this.connectionState === PcConnectionState.ONLINE) {
            Column({ space: 8 }) {
              Text('查找任务')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(14)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_INK_SECONDARY)
                .width('100%')
              TextInput({ placeholder: '搜索任务、工作区或模型', text: this.taskSearchQuery })
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(15)
                .height(48)
                .backgroundColor(COLOR_SURFACE_INPUT)
                .border({ width: 1, color: COLOR_RULE_INPUT })
                .borderRadius(2)
                .accessibilityText('搜索任务、工作区或模型')
                .onChange((value: string) => {
                  this.taskSearchQuery = value;
                })
              Row({ space: 8 }) {
                Button('全部')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .type(ButtonType.Normal)
                  .borderRadius(6)
                  .layoutWeight(1)
                  .height(48)
                  .fontColor(this.taskFilter === 'all' ? COLOR_SURFACE : COLOR_INK_SECONDARY)
                  .backgroundColor(this.taskFilter === 'all' ? COLOR_ACCENT : COLOR_SURFACE_CONTROL)
                  .accessibilityText('筛选全部任务')
                  .onClick(() => {
                    this.taskFilter = 'all';
                  })
                Button('运行中')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .type(ButtonType.Normal)
                  .borderRadius(6)
                  .layoutWeight(1)
                  .height(48)
                  .fontColor(this.taskFilter === 'running' ? COLOR_SURFACE : COLOR_INK_SECONDARY)
                  .backgroundColor(this.taskFilter === 'running' ? COLOR_ACCENT : COLOR_SURFACE_CONTROL)
                  .accessibilityText('筛选运行中的任务')
                  .onClick(() => {
                    this.taskFilter = 'running';
                  })
              }
              .width('100%')
              Row({ space: 8 }) {
                Button('需要关注')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .type(ButtonType.Normal)
                  .borderRadius(6)
                  .layoutWeight(1)
                  .height(48)
                  .fontColor(this.taskFilter === 'attention' ? COLOR_SURFACE : COLOR_INK_SECONDARY)
                  .backgroundColor(this.taskFilter === 'attention' ? COLOR_ACCENT : COLOR_SURFACE_CONTROL)
                  .accessibilityText('筛选失败或有错误的任务')
                  .onClick(() => {
                    this.taskFilter = 'attention';
                  })
                Button('已结束')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .type(ButtonType.Normal)
                  .borderRadius(6)
                  .layoutWeight(1)
                  .height(48)
                  .fontColor(this.taskFilter === 'ended' ? COLOR_SURFACE : COLOR_INK_SECONDARY)
                  .backgroundColor(this.taskFilter === 'ended' ? COLOR_ACCENT : COLOR_SURFACE_CONTROL)
                  .accessibilityText('筛选已结束的任务')
                  .onClick(() => {
                    this.taskFilter = 'ended';
                  })
              }
              .width('100%')
              if (this.taskSearchQuery.length > 0 || this.taskFilter !== 'all') {
                Button('清除搜索与筛选')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .type(ButtonType.Normal)
                  .borderRadius(6)
                  .height(48)
                  .fontSize(13)
                  .fontColor(COLOR_INK_SECONDARY)
                  .backgroundColor(COLOR_SURFACE_CONTROL)
                  .width('100%')
                  .accessibilityText('清除任务搜索与状态筛选')
                  .onClick(() => {
                    this.taskSearchQuery = '';
                    this.taskFilter = 'all';
                  })
              }
            }
            .width('100%')
            .padding(12)
            .backgroundColor(COLOR_SURFACE_SUBDUED)
            .border({ width: { top: 1, bottom: 1 }, color: COLOR_RULE })
            .borderRadius(0)
          }

          if (this.connectionState === PcConnectionState.ONLINE && this.taskFailure !== undefined) {
            this.failureCard(this.taskFailure!)
          }

          if (this.activePcDevice === undefined) {
            Column({ space: 8 }) {
              Text('连接电脑后，任务会自动出现在这里')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(16)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_INK_STRONG)
              Text('先完成上方设备绑定，无需手动导入任务或复制会话。')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontColor(COLOR_MUTED)
            }
            .alignItems(HorizontalAlign.Start)
            .width('100%')
            .padding({ top: 22, right: 16, bottom: 22, left: 16 })
            .backgroundColor(COLOR_SURFACE_SUBDUED)
            .border({ width: { left: 4 }, color: COLOR_ACCENT })
            .borderRadius(0)
          } else if (this.connectionState !== PcConnectionState.ONLINE) {
            Column({ space: 8 }) {
              Text('电脑未连接')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(16)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_INK_STRONG)
              Text('离线期间不会刷新或展示任务；电脑恢复连接后会自动重新同步。')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontColor(COLOR_MUTED)
            }
            .alignItems(HorizontalAlign.Start)
            .width('100%')
            .padding({ top: 22, right: 16, bottom: 22, left: 16 })
            .backgroundColor(COLOR_SURFACE_SUBDUED)
            .border({ width: { left: 4 }, color: COLOR_WARNING })
            .borderRadius(0)
          } else if (this.tasks.length === 0) {
            Column({ space: 8 }) {
              Text('暂时没有同步任务')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(16)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_INK_STRONG)
              Text('电脑在线时首次同步最多等待一个心跳周期；也可以在电脑端先创建任务后刷新。')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontColor(COLOR_MUTED)
            }
            .alignItems(HorizontalAlign.Start)
            .width('100%')
            .padding({ top: 22, right: 16, bottom: 22, left: 16 })
            .backgroundColor(COLOR_SURFACE_SUBDUED)
            .border({ width: { left: 4 }, color: COLOR_ACCENT })
            .borderRadius(0)
          } else {
            if (this.filteredTasks().length === 0) {
              Column({ space: 8 }) {
                Text('当前筛选无结果')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .fontSize(16)
                  .fontWeight(FontWeight.Medium)
                  .fontColor(COLOR_INK_STRONG)
                Text('已保留当前电脑的任务；可清除搜索或筛选后继续查看。')
                  .fontFamily(FONT_SYSTEM_SANS)
                  .fontSize(13)
                  .fontColor(COLOR_MUTED)
              }
              .alignItems(HorizontalAlign.Start)
              .width('100%')
              .padding({ top: 22, right: 16, bottom: 22, left: 16 })
              .backgroundColor(COLOR_SURFACE_SUBDUED)
              .border({ width: { left: 4 }, color: COLOR_RULE_INPUT })
              .borderRadius(0)
            } else {
              ForEach(this.taskWorkspaceGroups(), (workspaceGroup: TaskWorkspaceGroup) => {
              Column({ space: 8 }) {
                Row({ space: 8 }) {
                  Row({ space: 6 }) {
                    Text(workspaceGroup.name)
                      .fontFamily(FONT_SYSTEM_SANS)
                      .fontSize(15)
                      .fontWeight(FontWeight.Bold)
                      .fontColor(COLOR_INK_STRONG)
                      .layoutWeight(1)
                      .maxLines(2)
                    if (workspaceGroup.runningTaskCount > 0) {
                      Text(`${workspaceGroup.runningTaskCount}`)
                        .fontFamily(FONT_SYSTEM_SANS)
                        .fontSize(11)
                        .fontWeight(FontWeight.Bold)
                        .fontColor(COLOR_SURFACE)
                        .textAlign(TextAlign.Center)
                        .width(20)
                        .height(20)
                        .borderRadius(10)
                        .backgroundColor(COLOR_SUCCESS)
                    }
                  }
                  .layoutWeight(1)
                  Text(`${workspaceGroup.tasks.length} 个任务`)
                    .fontFamily(FONT_SYSTEM_SANS)
                    .fontSize(12)
                    .fontColor(COLOR_MUTED)
                  Text(this.taskWorkspaceIsExpanded(workspaceGroup.name) ? '收起' : '展开')
                    .fontFamily(FONT_SYSTEM_SANS)
                    .fontSize(13)
                    .fontWeight(FontWeight.Medium)
                    .fontColor(COLOR_ACCENT)
                    .constraintSize({ minWidth: 48, minHeight: 48 })
                    .textAlign(TextAlign.Center)
                }
                .width('100%')
                .accessibilityText(this.taskWorkspaceIsExpanded(workspaceGroup.name) ?
                  `收起项目 ${workspaceGroup.name}` : `展开项目 ${workspaceGroup.name}`)
                .onClick(() => {
                  this.toggleTaskWorkspace(workspaceGroup.name);
                })
                if (this.taskWorkspaceIsExpanded(workspaceGroup.name)) {
                  ForEach(workspaceGroup.tasks, (task: RemoteTaskSnapshot) => {
                    Column({ space: 7 }) {
                      Row() {
                        Text(task.name.length === 0 ? '未命名任务' : task.name)
                          .fontFamily(FONT_SYSTEM_SANS)
                          .fontSize(16)
                          .fontWeight(FontWeight.Bold)
                          .fontColor(COLOR_INK)
                          .maxLines(2)
                          .layoutWeight(1)
                        Text(this.taskStatusText(task))
                          .fontFamily(FONT_SYSTEM_SANS)
                          .fontSize(12)
                          .fontWeight(FontWeight.Medium)
                          .fontColor(this.taskStatusColor(task))
                          .backgroundColor(this.taskStatusBackground(task))
                          .borderRadius(2)
                          .padding({ left: 8, right: 8, top: 3, bottom: 3 })
                      }
                      .width('100%')
                      Text(this.turnStatusText(task))
                        .fontFamily(FONT_SYSTEM_SANS)
                        .fontSize(13)
                        .fontColor(COLOR_MUTED)
                        .width('100%')
                      Text(`电脑观测：${formatChinaDateTime(task.pcObservedAt)}`)
                        .fontFamily(FONT_SYSTEM_SANS)
                        .fontSize(12)
                        .fontColor(COLOR_MUTED_FAINT)
                        .width('100%')
                      if (task.lastError !== null) {
                        Text(task.lastError)
                          .fontFamily(FONT_SYSTEM_SANS)
                          .fontSize(13)
                          .fontColor(COLOR_DANGER)
                          .maxLines(3)
                          .width('100%')
                      }
                    }
                    .width('100%')
                    .padding(14)
                    .backgroundColor(COLOR_SURFACE)
                    .border({
                      width: { bottom: 1 },
                      color: COLOR_RULE
                    })
                    .borderRadius(0)
                    .onClick(() => {
                      this.openTaskDetail(task);
                    })
                  }, (task: RemoteTaskSnapshot) => task.remoteTaskId)
                }
              }
              .width('100%')
              .padding({ top: 12, bottom: 12 })
              .backgroundColor(COLOR_PAGE)
              .border({ width: { top: 1 }, color: COLOR_RULE })
              .borderRadius(0)
              }, (workspaceGroup: TaskWorkspaceGroup) => workspaceGroup.name)
            }
            if (this.nextTaskCursor !== null) {
              Button(this.loadingMoreTasks ? '正在加载更多…' : '加载更多任务')
                .fontFamily(FONT_SYSTEM_SANS)
                .type(ButtonType.Normal)
                .borderRadius(6)
                .fontSize(14)
                .height(48)
                .width('100%')
                .fontColor(COLOR_ACCENT)
                .backgroundColor(COLOR_ACCENT_SOFT)
                .enabled(!this.loadingMoreTasks && !this.refreshingTasks)
                .accessibilityText('加载更多任务')
                .onClick(() => {
                  this.loadMoreTasks();
                })
            } else if (this.tasks.length >= 100) {
              Text('任务已全部加载')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontColor(COLOR_MUTED_LIGHT)
                .textAlign(TextAlign.Center)
                .width('100%')
            }
          }
        }
        .width('100%')
        .padding(0)
        .backgroundColor(COLOR_PAGE)
        .borderRadius(0)

        }
        }

        if (this.activePage === 'detail') {
        Column({ space: 8 }) {
          Scroll(this.replyScroller) {
          Column({ space: 12 }) {
          Text(this.selectedTask() === undefined ? '请返回任务列表重新选择' :
            `${this.selectedTask()!.workspaceName} · ${this.selectedTask()!.modelLabel} · ${this.taskStatusText(this.selectedTask()!)}`)
            .fontFamily(FONT_SYSTEM_SANS)
            .fontSize(13)
            .fontColor(COLOR_MUTED)
            .width('100%')

          if (this.selectedTask() !== undefined) {
            Column({ space: 5 }) {
              Text('当前执行状态')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(14)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_INK_SECONDARY)
              Text(this.detailExecutionText(this.selectedTask()!))
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(15)
                .fontWeight(FontWeight.Medium)
                .fontColor(this.selectedTaskHasActiveTurn() ? COLOR_WARNING_TEXT : COLOR_INK_BODY)
              Text(`电脑观测：${formatChinaDateTime(this.selectedTask()!.pcObservedAt)}`)
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(12)
                .fontColor(COLOR_MUTED_LIGHT)
            }
            .alignItems(HorizontalAlign.Start)
            .width('100%')
            .padding(14)
            .backgroundColor(this.selectedTaskHasActiveTurn() ? COLOR_WARNING_SOFT : COLOR_SURFACE_INPUT)
            .border({ width: { left: 4 }, color: this.selectedTaskHasActiveTurn() ? COLOR_WARNING : COLOR_ACCENT })
            .borderRadius(0)
            .accessibilityText(`当前执行状态：${this.detailExecutionText(this.selectedTask()!)}`)
          }

          if (this.liveReplyActive || this.liveReplyText.length > 0 || this.liveReplyNotice.length > 0) {
            Text(this.liveReplyActive ? '正在生成回复' : '回复同步状态')
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(16)
              .fontWeight(FontWeight.Bold)
              .fontColor(COLOR_INK)
              .width('100%')
            Text(this.liveReplyNotice.length > 0 ? this.liveReplyNotice :
              (this.liveReplyConnected ? '内容由电脑端实时同步' : '实时连接正在恢复'))
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(12)
              .fontColor(this.liveReplyConnected ? COLOR_MUTED_LIGHT : COLOR_WARNING_TEXT)
              .width('100%')
            if (this.liveReplyActive && this.liveReplyText.length === 0) {
              Text('模型正在准备回复…')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(14)
                .fontColor(COLOR_MUTED)
                .width('100%')
                .padding(14)
                .backgroundColor(COLOR_ACCENT_SOFT)
                .border({ width: { left: 4 }, color: COLOR_ACCENT })
                .borderRadius(0)
            }
          }

          Text(this.selectedTask() === undefined ? '回复记录' : this.replyHeading(this.selectedTask()!))
            .fontFamily(FONT_SYSTEM_SANS)
            .fontSize(16)
            .fontWeight(FontWeight.Bold)
            .fontColor(COLOR_INK)
            .width('100%')

          if (this.selectedTask() !== undefined && this.detailReplyText(this.selectedTask()!).length > 0) {
            if (this.selectedTask()!.lastReplyState === ReplyState.TRUNCATED) {
              Text(this.replyStateDescription(this.selectedTask()!))
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontColor(COLOR_WARNING_TEXT)
                .width('100%')
                .padding(12)
                .backgroundColor(COLOR_WARNING_SOFT)
                .borderRadius(0)
                .accessibilityText(`答复状态：${this.replyStateDescription(this.selectedTask()!)}`)
            }
            MarkdownPreview({ content: this.detailReplyText(this.selectedTask()!) })
              .width('100%')
              .padding(14)
              .backgroundColor(COLOR_SURFACE_INPUT)
              .border({ width: { left: 1 }, color: COLOR_RULE })
              .borderRadius(0)
            if (this.selectedTask()!.lastReplyState === ReplyState.TRUNCATED) {
              Text('当前内容尚不完整，正在等待电脑端同步完整回复记录。')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontColor(COLOR_WARNING_TEXT)
                .width('100%')
                .padding(12)
                .backgroundColor(COLOR_WARNING_SOFT)
                .borderRadius(0)
            }
          } else if (this.liveReplyAwaitingFinalSnapshot && this.liveReplyText.length > 0) {
            Column({ space: 5 }) {
              Text('完整最终答复仍在同步')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(15)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_INK_BODY)
              Text('当前已显示本轮实时内容；它不会被当作最终答复，直到电脑端安全快照完成校准。')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(13)
                .fontColor(COLOR_MUTED)
            }
            .alignItems(HorizontalAlign.Start)
            .width('100%')
            .padding(14)
            .backgroundColor(COLOR_SURFACE_INPUT)
            .border({ width: { left: 4 }, color: COLOR_ACCENT })
            .borderRadius(0)
            .accessibilityText('完整最终答复仍在同步，当前已显示本轮实时内容')
          } else {
            Column({ space: 5 }) {
              Text(this.selectedTask() === undefined ? '请返回任务列表重新选择。' :
                this.replyStateTitle(this.selectedTask()!))
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(15)
                .fontWeight(FontWeight.Medium)
                .fontColor(COLOR_INK_BODY)
              if (this.selectedTask() !== undefined) {
                Text(this.replyStateDescription(this.selectedTask()!))
                  .fontFamily(FONT_SYSTEM_SANS)
                  .fontSize(13)
                  .fontColor(COLOR_MUTED)
              }
            }
            .alignItems(HorizontalAlign.Start)
            .width('100%')
            .padding(14)
            .backgroundColor(COLOR_SURFACE_INPUT)
            .border({ width: { left: 4 }, color: COLOR_RULE_INPUT })
            .borderRadius(0)
            .accessibilityText(this.selectedTask() === undefined ? '任务不可用' :
              `答复状态：${this.replyStateTitle(this.selectedTask()!)}，${this.replyStateDescription(this.selectedTask()!)}`)
          }

          if (this.commandFailure !== undefined) {
            this.failureCard(this.commandFailure!)
          }
          }
          .width('100%')
          .padding({ top: 4, bottom: 12 })
          .onAreaChange(() => {
            this.followReplyBottom();
          })
          }
          .width('100%')
          .layoutWeight(1)
          .align(Alignment.TopStart)
          .clip(true)
          .edgeEffect(EdgeEffect.None)
          .onDidScroll((_xOffset: number, yOffset: number, state: ScrollState) => {
            this.trackReplyScroll(yOffset, state);
          })
          .onAreaChange(() => {
            this.followReplyBottom();
          })

          Column({ space: 8 }) {
            Text('输入消息')
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(14)
              .fontWeight(FontWeight.Bold)
              .fontColor(COLOR_INK)
              .width('100%')
            Text(this.commandNotice)
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(13)
              .fontColor(COLOR_INK_BODY)
              .width('100%')
              .maxLines(2)
              .textOverflow({ overflow: TextOverflow.Ellipsis })

          TextArea({
            placeholder: '连接电脑并选择任务后输入指令',
            text: this.instruction
          })
            .fontFamily(FONT_SYSTEM_SANS)
            .height(80)
            .fontSize(15)
            .backgroundColor(COLOR_SURFACE_INPUT)
            .border({ width: 1, color: COLOR_RULE_INPUT })
            .borderRadius(2)
            .enabled(this.canSendSelected())
            .accessibilityText('输入发送给当前任务的消息')
            .onChange((value: string) => {
              this.instruction = value;
            })

          Row({ space: 10 }) {
            Button(this.commandSubmitting ? '处理中…' : '发送消息')
              .fontFamily(FONT_SYSTEM_SANS)
              .type(ButtonType.Normal)
              .borderRadius(6)
              .layoutWeight(1)
              .height(48)
              .fontWeight(FontWeight.Medium)
              .backgroundColor(COLOR_ACCENT)
              .enabled(this.canSendSelected() && this.instruction.trim().length > 0)
              .onClick(() => {
                this.sendInstruction();
              })
            Button('停止')
              .fontFamily(FONT_SYSTEM_SANS)
              .type(ButtonType.Normal)
              .borderRadius(6)
              .width(88)
              .height(48)
              .fontColor(COLOR_DANGER)
              .backgroundColor(COLOR_DANGER_SOFT)
              .enabled(this.canStopSelected())
              .onClick(() => {
                this.stopSelectedTask();
              })
          }
          .width('100%')
          }
          .width('100%')
          .padding({ top: 8, bottom: 4 })
          .flexShrink(0)
          .backgroundColor(COLOR_PAGE)
          .border({ width: { top: 1 }, color: COLOR_RULE })
        }
        .width('100%')
        .layoutWeight(1)
        .padding(0)
        .backgroundColor(COLOR_PAGE)
        .borderRadius(0)
        }

        if (this.activePage === 'create') {
        Column({ space: 20 }) {
          Column({ space: 6 }) {
            Text('提交前不会创建任务')
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(18)
              .fontWeight(FontWeight.Bold)
              .fontColor(COLOR_INK)
              .width('100%')
            Text(`目标电脑 · ${this.activePcDevice?.displayName ?? '未连接'}`)
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(13)
              .fontColor(COLOR_MUTED)
              .width('100%')
          }
          .alignItems(HorizontalAlign.Start)
          .width('100%')

          Column({ space: 10 }) {
            Row({ space: 12 }) {
              Text('01')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(24)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_ACCENT)
                .width(40)
              Text('选择工作区')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(16)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_INK)
                .layoutWeight(1)
            }
            .width('100%')
            .alignItems(VerticalAlign.Center)
            if (this.workspaceChoices().length === 0) {
              Text('当前没有已同步的工作区。请先在电脑端为目标工作区创建一个任务。')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(14)
                .fontColor(COLOR_WARNING_TEXT)
                .width('100%')
                .padding(14)
                .backgroundColor(COLOR_WARNING_SOFT)
                .border({ width: { left: 4 }, color: COLOR_WARNING })
                .borderRadius(0)
            } else {
              ForEach(this.workspaceChoices(), (workspace: WorkspaceChoice) => {
                Row({ space: 12 }) {
                  Column({ space: 3 }) {
                    Text(workspace.name)
                      .fontFamily(FONT_SYSTEM_SANS)
                      .fontSize(15)
                      .fontWeight(FontWeight.Medium)
                      .fontColor(COLOR_INK)
                      .maxLines(2)
                      .width('100%')
                    Text(`${workspace.taskCount} 个已同步任务`)
                      .fontFamily(FONT_SYSTEM_SANS)
                      .fontSize(12)
                      .fontColor(COLOR_MUTED)
                      .width('100%')
                  }
                  .alignItems(HorizontalAlign.Start)
                  .layoutWeight(1)
                  Text(this.selectedWorkspaceSourceTaskId === workspace.sourceTaskId ? '已选择' : '选择')
                    .fontFamily(FONT_SYSTEM_SANS)
                    .fontSize(12)
                    .fontWeight(FontWeight.Medium)
                    .fontColor(this.selectedWorkspaceSourceTaskId === workspace.sourceTaskId ? COLOR_ACCENT : COLOR_MUTED)
                }
                .width('100%')
                .padding(12)
                .backgroundColor(this.selectedWorkspaceSourceTaskId === workspace.sourceTaskId ? COLOR_ACCENT_SOFT : COLOR_SURFACE_SUBDUED)
                .border({ width: 1, color: this.selectedWorkspaceSourceTaskId === workspace.sourceTaskId ? COLOR_ACCENT : COLOR_RULE })
                .borderRadius(2)
                .enabled(!this.commandSubmitting)
                .accessibilityText(this.selectedWorkspaceSourceTaskId === workspace.sourceTaskId ?
                  `工作区 ${workspace.name}，已选择` : `选择工作区 ${workspace.name}`)
                .onClick(() => {
                  this.selectedWorkspaceSourceTaskId = workspace.sourceTaskId;
                })
              }, (workspace: WorkspaceChoice) => `workspace-${workspace.sourceTaskId}`)
            }
          }
          .alignItems(HorizontalAlign.Start)
          .width('100%')

          Column({ space: 10 }) {
            Row({ space: 12 }) {
              Text('02')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(24)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_ACCENT)
                .width(40)
              Text('选择模型')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(16)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_INK)
                .layoutWeight(1)
            }
            .width('100%')
            .alignItems(VerticalAlign.Center)
            if ((this.activePcDevice?.modelConfigs ?? []).length === 0) {
              Text('当前电脑没有可用模型配置，请先在电脑端完成配置。')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(14)
                .fontColor(COLOR_WARNING_TEXT)
                .width('100%')
                .padding(14)
                .backgroundColor(COLOR_WARNING_SOFT)
                .border({ width: { left: 4 }, color: COLOR_WARNING })
                .borderRadius(0)
            } else {
              ForEach(this.activePcDevice?.modelConfigs ?? [], (config: ModelConfigSummary) => {
                Row({ space: 12 }) {
                  Column({ space: 3 }) {
                    Text(config.model)
                      .fontFamily(FONT_SYSTEM_SANS)
                      .fontSize(15)
                      .fontWeight(FontWeight.Medium)
                      .fontColor(COLOR_INK)
                      .maxLines(2)
                      .width('100%')
                    Text(config.provider)
                      .fontFamily(FONT_SYSTEM_SANS)
                      .fontSize(12)
                      .fontColor(COLOR_MUTED)
                      .maxLines(1)
                      .width('100%')
                  }
                  .alignItems(HorizontalAlign.Start)
                  .layoutWeight(1)
                  Text(this.selectedModelConfigId === config.id ? '已选择' : '选择')
                    .fontFamily(FONT_SYSTEM_SANS)
                    .fontSize(12)
                    .fontWeight(FontWeight.Medium)
                    .fontColor(this.selectedModelConfigId === config.id ? COLOR_ACCENT : COLOR_MUTED)
                }
                .width('100%')
                .padding(12)
                .backgroundColor(this.selectedModelConfigId === config.id ? COLOR_ACCENT_SOFT : COLOR_SURFACE_SUBDUED)
                .border({ width: 1, color: this.selectedModelConfigId === config.id ? COLOR_ACCENT : COLOR_RULE })
                .borderRadius(2)
                .enabled(!this.commandSubmitting)
                .accessibilityText(this.selectedModelConfigId === config.id ?
                  `模型 ${config.model}，提供方 ${config.provider}，已选择` :
                  `选择模型 ${config.model}，提供方 ${config.provider}`)
                .onClick(() => {
                  this.selectedModelConfigId = config.id;
                })
              }, (config: ModelConfigSummary) => `model-${config.id}`)
            }
          }
          .alignItems(HorizontalAlign.Start)
          .width('100%')

          Column({ space: 8 }) {
            Row({ space: 12 }) {
              Text('03')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(24)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_ACCENT)
                .width(40)
              Text('编写第一条指令')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(16)
                .fontWeight(FontWeight.Bold)
                .fontColor(COLOR_INK)
                .layoutWeight(1)
            }
            .width('100%')
            .alignItems(VerticalAlign.Center)
            TextArea({ placeholder: '描述需要在所选工作区完成的工作', text: this.newTaskInstruction })
              .fontFamily(FONT_SYSTEM_SANS)
              .height(132)
              .fontSize(15)
              .backgroundColor(COLOR_SURFACE_INPUT)
              .border({ width: 1, color: COLOR_RULE_INPUT })
              .borderRadius(2)
              .enabled(!this.commandSubmitting)
              .accessibilityText('编写新任务的第一条指令')
              .onChange((value: string) => {
                this.newTaskInstruction = value;
              })
            Row() {
              Text('任务名称将从第一行生成')
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(12)
                .fontColor(COLOR_MUTED_LIGHT)
                .layoutWeight(1)
              Text(`${this.newTaskInstructionBytes()} / 8192 B`)
                .fontFamily(FONT_SYSTEM_SANS)
                .fontSize(12)
                .fontColor(this.newTaskInstructionBytes() > 8192 ? COLOR_DANGER : COLOR_MUTED_LIGHT)
            }
            .width('100%')
          }
          .alignItems(HorizontalAlign.Start)
          .width('100%')

          if (this.createFailure !== undefined) {
            this.failureCard(this.createFailure!)
          } else {
            Text(this.createNotice)
              .fontFamily(FONT_SYSTEM_SANS)
              .fontSize(13)
              .fontColor(COLOR_INK_SECONDARY)
              .width('100%')
          }

          Button(this.commandSubmitting ? '正在创建并发送…' : '创建并发送')
            .fontFamily(FONT_SYSTEM_SANS)
            .type(ButtonType.Normal)
            .borderRadius(6)
            .width('100%')
            .height(50)
            .fontSize(15)
            .fontWeight(FontWeight.Medium)
            .backgroundColor(COLOR_ACCENT)
            .enabled(this.canCreateTask())
            .onClick(() => {
              this.createTask();
            })
        }
        .width('100%')
        .padding(16)
        .backgroundColor(COLOR_SURFACE)
        .border({ width: 1, color: COLOR_RULE })
        .borderRadius(2)
        }

        if (this.activePage !== 'detail') {
          Text('手机只提交受限命令；模型与工具始终由电脑端 Codex 官方客户端执行。')
          .fontFamily(FONT_SYSTEM_SANS)
          .fontSize(12)
          .fontColor(COLOR_MUTED_LIGHT)
          .textAlign(TextAlign.Center)
          .width('100%')
          .padding({ left: 10, right: 10, bottom: 14 })
        }
      }
      .width('100%')
      .height(this.activePage === 'detail' ? '100%' : undefined)
      .constraintSize({ minHeight: '100%', maxWidth: 840 })
      .padding({ left: this.pageGutter(), right: this.pageGutter(), top: 16,
        bottom: this.activePage === 'detail' ? 8 : 24 })
  }

  build() {
    Column() {
      if (this.activePage === 'detail') {
        this.pageContent()
      } else if (this.activePage === 'create') {
        Scroll() {
          this.pageContent()
        }
        .width('100%')
        .height('100%')
        .align(Alignment.Top)
        .edgeEffect(EdgeEffect.Spring)
      } else {
        Scroll() {
          this.pageContent()
        }
        .width('100%')
        .layoutWeight(1)
        .align(Alignment.Top)
        .edgeEffect(EdgeEffect.Spring)
        this.bottomNavigation()
      }
    }
    .onTouch((event: TouchEvent) => {
      this.handleTaskPageBackTouch(event);
    })
    .width('100%')
    .height('100%')
    .backgroundColor(COLOR_PAGE)
    .onAreaChange((_oldValue: Area, newValue: Area) => {
      this.updateViewportWidth(newValue.width);
    })
  }
}
