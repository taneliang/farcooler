/// Account-owned recovery copies. Ciphertext and credential ids are untrusted;
/// nothing is rendered as HTML, and clearing one needs no passkey or PRF.
import type { Vault, VaultCall } from './vault'

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

export class SavedCopies {
  constructor(private readonly dialog: HTMLDialogElement, private readonly call: VaultCall, private readonly current: () => string | undefined) {}

  async open(): Promise<void> {
    if (!this.dialog.open) this.dialog.showModal()
    this.dialog.replaceChildren()
    const heading = document.createElement('h2')
    heading.textContent = 'Saved Copies' // platform copy: browser passkey recovery-copy management
    const explanation = document.createElement('p')
    explanation.textContent = 'Each passkey keeps one sealed recovery copy shared by its browsers. Clearing a copy removes recovery for those browsers; their local runners and passkey stay. An active browser may save a copy again after a touch.' // platform copy: browser passkey recovery-copy management
    const close = document.createElement('button')
    close.textContent = 'Close'
    close.onclick = () => this.dialog.close()
    const message = document.createElement('p')
    message.setAttribute('role', 'status')
    message.textContent = 'Loading saved copies…' // platform copy: browser passkey recovery-copy management
    const list = document.createElement('ul')
    this.dialog.append(heading, explanation, close, message, list)
    const answer = await this.call('web/vault', {}).catch(() => null)
    if (answer?.status !== 200 || !Array.isArray(answer.body?.vaults)) {
      message.textContent = 'Far Cooler couldn’t load your saved copies. Close this window and try again.' // platform copy: browser passkey recovery-copy management
      return
    }
    const vaults = answer.body.vaults as Vault[]
    message.textContent = vaults.length ? `${vaults.length} saved ${vaults.length === 1 ? 'copy' : 'copies'}.` : 'No saved copies.' // platform copy: browser passkey recovery-copy management
    for (const vault of vaults) {
      const row = document.createElement('li')
      const id = document.createElement('code')
      id.textContent = vault.credential
      const label = document.createElement('p')
      label.textContent = vault.credential === this.current() ? 'This browser’s passkey' : 'Another passkey' // platform copy: browser passkey recovery-copy management
      const clear = document.createElement('button')
      clear.textContent = 'Clear Copy' // platform copy: browser passkey recovery-copy management
      clear.onclick = async () => {
        clear.disabled = true
        if (await deleteCopy(this.call, vault.credential)) {
          row.remove()
          message.textContent = list.children.length ? 'Saved copy cleared.' : 'No saved copies.' // platform copy: browser passkey recovery-copy management
        } else {
          message.textContent = 'Far Cooler couldn’t clear that saved copy. Try again.' // platform copy: browser passkey recovery-copy management
          clear.disabled = false
        }
      }
      row.append(label, id, clear)
      list.append(row)
    }
  }
}
