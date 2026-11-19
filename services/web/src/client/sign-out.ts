import { b64url, type VaultCall } from './vault'
import { deleteCopy } from './saved-copies'
// Keep emergency sign-out available when the shared WASM module cannot load.
import words from '../../../../crates/client/src/testdata/recovery-view.json'

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
export function installSignOut(form: HTMLFormElement, dialog: HTMLDialogElement, deps: Deps, waitMs = 2_000): () => boolean {
  const title = dialog.querySelector<HTMLHeadingElement>('#sign-out-title')!
  const detail = dialog.querySelector<HTMLParagraphElement>('#sign-out-detail')!
  const retry = dialog.querySelector<HTMLButtonElement>('#sign-out-retry')!
  const anyway = dialog.querySelector<HTMLButtonElement>('#sign-out-anyway')!
  retry.textContent = words.retry
  anyway.textContent = words.anyway
  dialog.addEventListener('cancel', event => event.preventDefault())
  let running = false
  form.addEventListener('submit', event => {
    event.preventDefault()
    if (running) return
    running = true
    deps.dropSecret()
    const active = deps.session()
    if (active) active.signingOut = true
    const attempt = async () => {
      title.textContent = words.signingOut
      detail.textContent = ''
      retry.hidden = anyway.hidden = true
      if (!dialog.open) dialog.showModal()
      const ready = await settled(active?.copyWork ?? Promise.resolve(), waitMs)
      const passkey = active?.device.passkey
      const deleted = ready && (!passkey || await deleteCopy(deps.call, b64url(passkey.id), waitMs))
      // The Worker's Clear-Site-Data also covers a storage call that stalls.
      await settled(deps.forget(), waitMs)
      if (deleted) return form.submit()
      title.textContent = words.deleteTitle
      detail.textContent = words.deleteDetail
      retry.onclick = () => void attempt()
      anyway.onclick = () => form.submit()
      retry.hidden = anyway.hidden = false
      retry.focus()
    }
    void attempt()
  })
  return () => running
}
