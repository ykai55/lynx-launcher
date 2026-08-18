export interface Application {
  id: string
  name: string
  iconUri?: string | null
}

export interface ApplicationCardState {
  application: Application
  visible: boolean
  visiblePosition: number | null
}

export function validateApplications(value: unknown): Application[] {
  if (!Array.isArray(value)) {
    throw new Error('Launcher.getApplications() must resolve to an array')
  }

  const applicationIndexes = new Map<string, number>()

  value.forEach((application, index) => {
    if (
      typeof application !== 'object'
      || application === null
      || Array.isArray(application)
    ) {
      throw new Error(
        `Launcher.getApplications() item at index ${index} must be an object`,
      )
    }

    const record = application as Record<string, unknown>
    if (typeof record.id !== 'string' || record.id.trim().length === 0) {
      throw new Error(
        `Launcher.getApplications() item at index ${index} has an invalid id; expected a non-empty string`,
      )
    }
    const firstIndex = applicationIndexes.get(record.id)
    if (firstIndex !== undefined) {
      throw new Error(
        `Launcher.getApplications() has duplicate id "${record.id}" at index ${index}; first seen at index ${firstIndex}`,
      )
    }
    applicationIndexes.set(record.id, index)

    if (typeof record.name !== 'string' || record.name.trim().length === 0) {
      throw new Error(
        `Launcher.getApplications() item at index ${index} has an invalid name; expected a non-empty string`,
      )
    }
    if (
      record.iconUri !== undefined
      && record.iconUri !== null
      && typeof record.iconUri !== 'string'
    ) {
      throw new Error(
        `Launcher.getApplications() item at index ${index} has an invalid iconUri; expected a string, null, or undefined`,
      )
    }
  })

  return value as Application[]
}

export function filterApplications(
  applications: readonly Application[],
  query: string,
): readonly Application[] {
  const normalizedQuery = query.trim().toLowerCase()

  if (!normalizedQuery) {
    return applications
  }

  return applications.filter(application =>
    application.name.toLowerCase().includes(normalizedQuery)
  )
}

export function getApplicationCardStates(
  applications: readonly Application[],
  query: string,
): ApplicationCardState[] {
  const normalizedQuery = query.trim().toLowerCase()
  let visiblePosition = 0

  return applications.map(application => {
    const visible = !normalizedQuery
      || application.name.toLowerCase().includes(normalizedQuery)
    if (visible) {
      visiblePosition += 1
    }

    return {
      application,
      visible,
      visiblePosition: visible ? visiblePosition : null,
    }
  })
}

export function hasApplicationIcon(application: Application): boolean {
  return typeof application.iconUri === 'string'
    && application.iconUri.trim().length > 0
}

export function getApplicationInitial(name: string): string {
  const trimmedName = name.trim()

  if (!trimmedName) {
    return '?'
  }

  return [...trimmedName][0]?.toUpperCase() ?? '?'
}

export function getApplicationAccent(application: Application): number {
  const seed = application.id || application.name
  let hash = 0

  for (let index = 0; index < seed.length; index += 1) {
    hash = ((hash << 5) - hash + seed.charCodeAt(index)) | 0
  }

  return Math.abs(hash) % 4
}
