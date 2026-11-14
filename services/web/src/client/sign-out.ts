import { b64url, type VaultCall } from './vault'
import { deleteCopy } from './saved-copies'

interface Session {
  device: { passkey?: { id: Uint8Array } }
  copyWork?: Promise<void>
  signingOut?: boolean
}

interface Deps {
  session(): Session | null
  call: VaultCall
  forget(): Promise<unknown>
  dropSecret(): void
  waiting(): void
  failed(retry: () => void, anyway: () => void): void
}

async function settled(work: Promise<unknown>, waitMs: number): Promise<boolean> {
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    return await Promise.race([
      work.then(() => true, () => true),
      new Promise<boolean>(resolve => { timer = setTimeout(() => resolve(false), waitMs) }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

/// Finish outstanding writes before deleting the shared copy. An unconfirmed
/// delete requires either retry or an explicit choice to sign out anyway.
export function installSignOut(form: HTMLFormElement, deps: Deps, waitMs = 2_000): void {
  let running = false
  form.addEventListener('submit', event => {
    event.preventDefault()
    if (running) return
    running = true
    deps.dropSecret()
    const active = deps.session()
    if (active) active.signingOut = true
    const attempt = async () => {
      deps.waiting()
      const ready = await settled(active?.copyWork ?? Promise.resolve(), waitMs)
      const passkey = active?.device.passkey
      const deleted = ready && (!passkey || await deleteCopy(deps.call, b64url(passkey.id), waitMs))
      // The Worker's Clear-Site-Data also covers a storage call that stalls.
      await settled(deps.forget(), waitMs)
      if (deleted) return form.submit()
      deps.failed(() => void attempt(), () => form.submit())
    }
    void attempt()
  })
}
