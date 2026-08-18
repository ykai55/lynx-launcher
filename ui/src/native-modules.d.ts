import '@lynx-js/types/background'

interface LauncherNativeModule {
  getApplications(): Promise<unknown>
  launchApplication(id: string): Promise<void>
}

declare module '@lynx-js/types/background' {
  interface NativeModules {
    Launcher?: LauncherNativeModule
  }
}
