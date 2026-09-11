import { RemoteEnvironment } from './RemoteModels';

export class RemoteEnvironmentConfig {
  static readonly enabled: boolean = true;
  static readonly environment: RemoteEnvironment = RemoteEnvironment.DEV;
  static readonly apiBaseUrl: string = 'https://www.meishijiankang.com/workagents-remote/api';
  static readonly replyStreamWssBaseUrl: string =
    'wss://www.meishijiankang.com/workagents-remote/stream/v1/tasks';

  static assertReady(): void {
    if (!RemoteEnvironmentConfig.enabled) {
      throw new Error('尚未配置轩++远程服务，暂时无法连接电脑');
    }
    if (RemoteEnvironmentConfig.environment !== RemoteEnvironment.DEV ||
      !RemoteEnvironmentConfig.apiBaseUrl.startsWith('https://') ||
      RemoteEnvironmentConfig.apiBaseUrl.endsWith('/') ||
      !RemoteEnvironmentConfig.replyStreamWssBaseUrl.startsWith('wss://') ||
      RemoteEnvironmentConfig.replyStreamWssBaseUrl.endsWith('/')) {
      throw new Error('远程开发环境配置无效');
    }
  }
}
