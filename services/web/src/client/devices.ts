/// The Devices sheet (ov-1490): who can open the runner in front, removing
/// one, and approving a new browser or phone in person.
///
/// Both changes are signed with this browser's passkey, with the person
/// verified, and the runner decides what that signature may do
/// (crates/daemon `step_up`). Approving shows the new device's name and
/// passkey to compare, then a code both screens show; nothing the new device
/// is sent can open anything until its person matches that code.

import type { Granted } from './device'

export interface DeviceRow {
  clientId: string
  label: string
  here: boolean
}

export interface Asking {
  name: string
  fingerprint: string
}

/// One runner this browser is connected to, which it can change.
export interface LiveRunner {
  grant: Granted
  devices: () => Promise<DeviceRow[]>
  /// Answers the tunnel token the new browser dials.
  approve: (code: string) => Promise<string>
  remove: (clientId: string) => Promise<void>
}

export interface DevicesDeps {
  /// The runner in front first, then every other one connected now.
  runners: () => LiveRunner[]
  /// Runners this browser has but isn't connected to, by name.
  offline: () => string[]
  /// crates/web `approvalAsking`; throws an Error with the ceremony's word.
  asking: (code: string) => Asking
  /// crates/web `sealApproval`.
  seal: (code: string, runners: Granted[]) => { sealed: string; compare: string; ceremony: string }
  post: (ceremony: string, sealed: string) => Promise<{ status: number; body: any }>
  failure: (error: Error & { code?: string; what?: string }, runner: string) => string
  scan: (video: HTMLVideoElement, found: (text: string) => boolean) => Promise<{ stop: () => void }>
}

export function codeFailure(word: string): string {
  switch (word) {
    case 'wrong_account':
      return 'That code is for another account. Sign in to this account on the new device, then use its new code.'
    case 'not_a_browser':
      return 'That isn’t a new browser’s code. On the new device, open Far Cooler, sign in and choose Approve From Another Device.'
    case 'wrong_target':
      return 'That’s this browser’s own code. Use the one on the new device.'
    case 'channel':
      return 'That code is from a different Far Cooler release. Open the same one on both devices.'
    case 'version':
      return 'That code is from a newer Far Cooler. Reload this page, then try again.'
    default:
      return 'That code couldn’t be read. Copy all of it, or scan it again.'
  }
}

export class DevicesSheet {
  private camera: { stop: () => void } | null = null
  private removing: string | null = null
  private asked = ''
  private busy = false
  private generation = 0

  constructor(private readonly dialog: HTMLDialogElement, private readonly $: <T extends HTMLElement = HTMLElement>(id: string) => T, private readonly deps: DevicesDeps) {
    this.$('devices-close').onclick = () => dialog.close()
    dialog.addEventListener('close', () => this.reset())
    this.$('device-scan').onclick = () => void this.scan()
    this.$('device-check').onclick = () => this.check(this.$<HTMLTextAreaElement>('device-code').value)
    this.$('device-approve').onclick = () => void this.approve()
    this.$<HTMLTextAreaElement>('device-code').oninput = () => this.forget()
  }

  open(): void {
    this.reset()
    const front = this.deps.runners()[0]
    const name = front?.grant.label ?? 'this runner'
    this.$('devices-title').textContent = `Devices on ${name}`
    this.$('devices-detail').textContent = `These can open ${name}. Remove one you’ve lost or no longer use: it stops opening it at once.`
    this.dialog.showModal()
    void this.list()
  }

  private reset(): void {
    this.generation++
    this.camera?.stop()
    this.camera = null
    this.removing = null
    this.$('device-video').hidden = true
    this.$<HTMLTextAreaElement>('device-code').value = ''
    this.forget()
    this.say('')
  }

  private forget(): void {
    this.asked = ''
    this.$('device-confirm').hidden = true
    this.$('device-compare').hidden = true
  }

  private say(words: string): void {
    const note = this.$('device-result')
    note.textContent = words
    note.hidden = !words
  }

  private async list(): Promise<void> {
    const mine = this.generation
    const list = this.$('devices-list')
    const front = this.deps.runners()[0]
    if (!front) {
      list.replaceChildren()
      this.say('Connect to a runner to see its devices.')
      return
    }
    let rows: DeviceRow[]
    try {
      rows = await front.devices()
    } catch (error) {
      if (mine === this.generation) this.say(this.deps.failure(error as Error, front.grant.label))
      return
    }
    if (mine !== this.generation) return
    list.replaceChildren(
      ...rows.map(row => {
        const item = document.createElement('li')
        const name = document.createElement('span')
        name.textContent = row.here ? `${row.label} (this browser)` : row.label
        item.append(name)
        if (!row.here) {
          const button = document.createElement('button')
          button.type = 'button'
          button.className = this.removing === row.clientId ? 'danger' : 'quiet'
          button.textContent = this.removing === row.clientId ? 'Confirm Remove' : 'Remove'
          button.setAttribute('aria-label', `${button.textContent} ${row.label}`)
          button.onclick = () => {
            if (this.removing !== row.clientId) {
              this.removing = row.clientId
              void this.list()
              return
            }
            void this.remove(row)
          }
          item.append(button)
        }
        return item
      }),
    )
  }

  /// Off every runner connected now that knows it. The ones that aren't
  /// connected are named: they keep letting it in until it's removed there.
  private async remove(row: DeviceRow): Promise<void> {
    if (this.busy) return
    this.busy = true
    const done: string[] = []
    const failed: string[] = []
    try {
      for (const runner of this.deps.runners()) {
        try {
          if (!(await runner.devices()).some(d => d.clientId === row.clientId)) continue
          this.say(`Removing ${row.label} from ${runner.grant.label}. Verify with your passkey.`)
          await runner.remove(row.clientId)
          done.push(runner.grant.label)
        } catch (error) {
          failed.push(this.deps.failure(error as Error, runner.grant.label))
        }
      }
    } finally {
      this.busy = false
      this.removing = null
    }
    const offline = this.deps.offline()
    this.say(
      [
        done.length ? `${row.label} can no longer open ${done.join(', ')}.` : '',
        ...failed,
        done.length && offline.length ? `This browser isn’t connected to ${offline.join(', ')}. If ${row.label} was added there, connect and remove it too.` : '',
      ].filter(Boolean).join(' '),
    )
    await this.list()
  }

  private async scan(): Promise<void> {
    const video = this.$<HTMLVideoElement>('device-video')
    this.camera?.stop()
    this.forget()
    video.hidden = false
    try {
      this.camera = await this.deps.scan(video, text => {
        if (!text.trimStart().startsWith('{')) return false
        this.$<HTMLTextAreaElement>('device-code').value = text
        video.hidden = true
        this.check(text)
        return true
      })
    } catch {
      video.hidden = true
      this.say('This browser couldn’t use a camera. Paste the code instead.')
    }
  }

  /// Show what the code asks for, with the runners to choose.
  private check(code: string): void {
    this.forget()
    let asking: Asking
    try {
      asking = this.deps.asking(code.trim())
    } catch (error) {
      this.say(codeFailure((error as Error).message))
      return
    }
    const runners = this.deps.runners()
    if (!runners.length) {
      this.say('Connect to a runner first.')
      return
    }
    this.say('')
    this.asked = code.trim()
    this.$('device-asking').textContent = `${asking.name} wants to open your runners. Check that it shows this passkey:`
    this.$('device-fingerprint').textContent = asking.fingerprint
    const choices = this.$('device-runners')
    choices.replaceChildren(
      ...runners.map(runner => {
        const label = document.createElement('label')
        const box = document.createElement('input')
        box.type = 'checkbox'
        box.value = runner.grant.id
        box.checked = true
        label.append(box, ` ${runner.grant.label}`)
        return label
      }),
    )
    const offline = this.deps.offline()
    if (offline.length) {
      const note = document.createElement('p')
      note.textContent = `Not connected now: ${offline.join(', ')}. To include one, open it first.`
      choices.append(note)
    }
    this.$('device-confirm').hidden = false
  }

  private async approve(): Promise<void> {
    const code = this.asked
    if (!code || this.busy) return
    const chosen = new Set([...this.$('device-runners').querySelectorAll<HTMLInputElement>('input:checked')].map(box => box.value))
    const runners = this.deps.runners().filter(r => chosen.has(r.grant.id))
    if (!runners.length) {
      this.say('Choose at least one runner.')
      return
    }
    this.busy = true
    const mine = this.generation
    const granted: Granted[] = []
    const failed: string[] = []
    try {
      for (const runner of runners) {
        this.say(`Adding it to ${runner.grant.label}. Verify with your passkey.`)
        try {
          const token = await runner.approve(code)
          granted.push({ ...runner.grant, token, why: '', cause: '' })
        } catch (error) {
          failed.push(this.deps.failure(error as Error, runner.grant.label))
        }
        if (mine !== this.generation) return
      }
      if (!granted.length) {
        this.say(failed.join(' '))
        return
      }
      const reply = this.deps.seal(code, granted)
      const posted = await this.deps.post(reply.ceremony, reply.sealed).catch(() => null)
      if (posted?.status !== 200) {
        this.say('The runners added it, but Far Cooler couldn’t send it the answer. Make a new code on the new device and approve it again.')
        return
      }
      this.$('device-confirm').hidden = true
      this.asked = ''
      this.$('device-compare').textContent = reply.compare
      this.$('device-compare').hidden = false
      this.say([`Check that the new device shows this code, then select They Match there. If it shows another, remove it above.`, ...failed].join(' '))
      await this.list()
    } finally {
      this.busy = false
    }
  }
}
