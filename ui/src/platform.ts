import {
  type Application,
  validateApplications,
} from './applications.js'

function launcherModule() {
  if (typeof NativeModules === 'undefined' || !NativeModules.Launcher) {
    throw new Error('Launcher native module is not registered')
  }

  return NativeModules.Launcher
}

export async function getApplications(): Promise<Application[]> {
  const applications = await launcherModule().getApplications()

  return validateApplications(applications)
}

export async function launchApplication(id: string): Promise<void> {
  await launcherModule().launchApplication(id)
}
