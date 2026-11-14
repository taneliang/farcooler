/// Account-owned recovery copies. Ciphertext and credential ids are untrusted;
/// nothing is rendered as HTML, and clearing one needs no passkey or PRF.
import type { Vault, VaultCall } from './vault'
import bundledView from '../../../../crates/client/src/testdata/recovery-view.json'

export async function deleteCopy(call: VaultCall, credential: string, waitMs = 2_000): Promise<boolean> {
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    return await Promise.race([
      call('web/vault/delete', { credential }).then(answer => answer.status === 200, () => false),
      new Promise<boolean>(resolve => { timer = setTimeout(() => resolve(false), waitMs) }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

export interface RecoveryView {
  title: string; explanation: string; close: string; loading: string; loadFailed: string
  summary: string; passkey: string; clear: string; cleared: string; clearFailed: string
  signingOut: string; deleteTitle: string; deleteDetail: string; retry: string; anyway: string
}
export type RecoveryPresenter = (count: number, current: boolean) => Promise<RecoveryView>

export class SavedCopies {
  constructor(private readonly dialog: HTMLDialogElement, private readonly call: VaultCall, private readonly current: () => string | undefined, private readonly present: RecoveryPresenter) {}

  async open(): Promise<void> {
    const presented = await this.present(0, false).catch(() => null)
    const words = presented ?? bundledView
    if (!this.dialog.open) this.dialog.showModal()
    this.dialog.replaceChildren()
    const heading = document.createElement('h2')
    heading.textContent = words.title
    const explanation = document.createElement('p')
    explanation.textContent = words.explanation
    const close = document.createElement('button')
    close.textContent = words.close
    close.onclick = () => this.dialog.close()
    const message = document.createElement('p')
    message.setAttribute('role', 'status')
    message.textContent = words.loading
    const list = document.createElement('ul')
    this.dialog.append(heading, explanation, close, message, list)
    if (!presented) {
      message.textContent = words.loadFailed
      return
    }
    const answer = await this.call('web/vault', {}).catch(() => null)
    if (answer?.status !== 200 || !Array.isArray(answer.body?.vaults)) {
      message.textContent = words.loadFailed
      return
    }
    const vaults = answer.body.vaults as Vault[]
    message.textContent = (await this.present(vaults.length, false)).summary
    for (const vault of vaults) {
      const row = document.createElement('li')
      const id = document.createElement('code')
      id.textContent = vault.credential
      const label = document.createElement('p')
      label.textContent = (await this.present(vaults.length, vault.credential === this.current())).passkey
      const clear = document.createElement('button')
      clear.textContent = words.clear
      clear.onclick = async () => {
        clear.disabled = true
        if (await deleteCopy(this.call, vault.credential)) {
          row.remove()
          message.textContent = (await this.present(list.children.length, false)).cleared
        } else {
          message.textContent = words.clearFailed
          clear.disabled = false
        }
      }
      row.append(label, id, clear)
      list.append(row)
    }
  }
}
