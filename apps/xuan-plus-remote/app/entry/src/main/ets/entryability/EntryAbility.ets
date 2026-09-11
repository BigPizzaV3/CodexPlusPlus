import { AbilityConstant, ConfigurationConstant, UIAbility, Want } from '@kit.AbilityKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { window } from '@kit.ArkUI';
import { DeviceIdentityCoordinator } from '../remote/DeviceIdentityCoordinator';
import { PushNotificationCoordinator } from '../remote/PushNotificationCoordinator';
import { AppLifecycleCoordinator } from '../remote/AppLifecycleCoordinator';
import { RemoteEnvironmentConfig } from '../remote/RemoteEnvironmentConfig';

const DOMAIN = 0x5741;
const TAG = 'XuanPlusRemote';

export default class EntryAbility extends UIAbility {
  onCreate(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    try {
      this.context.getApplicationContext().setColorMode(ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET);
    } catch (error) {
      hilog.warn(DOMAIN, TAG, '%{public}s', 'Unable to apply the system color mode');
    }
    this.initializeDeviceIdentity();
    if (RemoteEnvironmentConfig.enabled) {
      PushNotificationCoordinator.initialize(this);
      PushNotificationCoordinator.handleWant(want);
    }
    hilog.info(DOMAIN, TAG, '%{public}s', 'Remote development client created');
  }

  onNewWant(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    PushNotificationCoordinator.handleWant(want);
  }

  onForeground(): void {
    AppLifecycleCoordinator.setForeground(true);
    PushNotificationCoordinator.setForeground(true);
  }

  onBackground(): void {
    PushNotificationCoordinator.setForeground(false);
    AppLifecycleCoordinator.setForeground(false);
  }

  private async initializeDeviceIdentity(): Promise<void> {
    try {
      await DeviceIdentityCoordinator.ensureDeviceKey();
      hilog.info(DOMAIN, TAG, '%{public}s', 'Device identity is ready');
    } catch (_error) {
      hilog.error(DOMAIN, TAG, '%{public}s', 'Device identity initialization failed; remote pairing remains disabled');
    }
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    windowStage.loadContent('pages/Index', (error) => {
      if (error.code) {
        hilog.error(DOMAIN, TAG, 'Failed to load main page: %{public}s', JSON.stringify(error));
        return;
      }
      hilog.info(DOMAIN, TAG, '%{public}s', 'Main page loaded');
    });
  }
}
