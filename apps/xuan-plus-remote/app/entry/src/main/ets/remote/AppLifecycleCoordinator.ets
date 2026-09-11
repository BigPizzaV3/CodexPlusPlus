type ForegroundHandler = () => Promise<void>;
type BackgroundHandler = () => void;

export class AppLifecycleCoordinator {
  private static foreground: boolean = false;
  private static foregroundHandler?: ForegroundHandler;
  private static backgroundHandler?: BackgroundHandler;

  static setForegroundHandler(handler: ForegroundHandler): void {
    AppLifecycleCoordinator.foregroundHandler = handler;
    if (AppLifecycleCoordinator.foreground) {
      void handler();
    }
  }

  static clearForegroundHandler(): void {
    AppLifecycleCoordinator.foregroundHandler = undefined;
  }

  static setBackgroundHandler(handler: BackgroundHandler): void {
    AppLifecycleCoordinator.backgroundHandler = handler;
  }

  static clearBackgroundHandler(): void {
    AppLifecycleCoordinator.backgroundHandler = undefined;
  }

  static setForeground(foreground: boolean): void {
    const becameForeground: boolean = foreground && !AppLifecycleCoordinator.foreground;
    const becameBackground: boolean = !foreground && AppLifecycleCoordinator.foreground;
    AppLifecycleCoordinator.foreground = foreground;
    if (becameForeground && AppLifecycleCoordinator.foregroundHandler !== undefined) {
      void AppLifecycleCoordinator.foregroundHandler();
    }
    if (becameBackground && AppLifecycleCoordinator.backgroundHandler !== undefined) {
      AppLifecycleCoordinator.backgroundHandler();
    }
  }

  static isForeground(): boolean {
    return AppLifecycleCoordinator.foreground;
  }
}
