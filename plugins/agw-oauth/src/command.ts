import { startDevice, waitForApproval, type DeviceStart } from './oauth.js'

export interface CommandResult {
  kind: 'success' | 'error'
  text: string
  openUrl?: string
  userCode?: string
}

export interface LoginWatch {
  status: 'waiting' | 'ok' | 'error'
  detail?: string
  openUrl?: string
  userCode?: string
}

let watch: LoginWatch | undefined

export function resetLoginWatch(): void {
  watch = undefined
}

export function currentWatch(): LoginWatch | undefined {
  return watch
}

export function usageText(): string {
  return [
    'Usage:',
    '  /agw status',
    '  /agw login',
    '  /agw logout',
    '',
    'Open the verification URL, sign into AI-GateWay, and approve.',
    'No model config file is required after login.',
  ].join('\n')
}

export async function startLogin(
  origin: string,
  persist: (apiKey: string, origin: string) => Promise<void>,
  signal?: AbortSignal,
): Promise<CommandResult> {
  if (watch?.status === 'waiting') {
    return {
      kind: 'success',
      text: waitingText(watch),
      ...watch.openUrl === undefined ? {} : { openUrl: watch.openUrl },
      ...watch.userCode === undefined ? {} : { userCode: watch.userCode },
    }
  }
  if (origin.length === 0) {
    return { kind: 'error', text: 'Set the gateway URL in Settings → AGW Oauth, or set AGW_ORIGIN.' }
  }
  let started: DeviceStart
  try {
    started = await startDevice(origin, signal)
  } catch (error) {
    return { kind: 'error', text: error instanceof Error ? error.message : String(error) }
  }
  watch = {
    status: 'waiting',
    openUrl: started.verificationUriComplete,
    userCode: started.userCode,
  }
  void waitForApproval(origin, started).then(
    async (approved) => {
      await persist(approved.apiKey, approved.origin)
      if (watch !== undefined) {
        watch.status = 'ok'
        watch.detail = 'Logged in to AI-GateWay.'
      }
    },
    (error: unknown) => {
      if (watch !== undefined) {
        watch.status = 'error'
        watch.detail = error instanceof Error ? error.message : String(error)
      }
    },
  )
  return {
    kind: 'success',
    text: waitingText(watch),
    openUrl: started.verificationUriComplete,
    userCode: started.userCode,
  }
}

function waitingText(current: LoginWatch): string {
  return [
    `Open this URL:\n${current.openUrl ?? ''}`,
    `Enter code: ${current.userCode ?? ''}`,
    '',
    'Finish signing in to AI-GateWay in the browser.',
    'This command has returned so the UI is not stuck; login continues in the background.',
    'When you are done, run /agw status.',
  ].join('\n')
}
