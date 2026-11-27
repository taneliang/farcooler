import type { WebRunner, WriterInput } from 'fc-web'

export type { WriterInput } from 'fc-web'
type Runner = Pick<WebRunner, 'terminal_writer_supported' | 'terminal_writer'>
type Action = 'watch' | 'claim' | 'status' | 'take-over' | 'release'
/// How soon a failed lease read is tried again, doubling up to the ceiling.
const RETRY_MS = 2000
const RETRY_CEILING_MS = 30_000

interface State {
  writable: boolean
  label: string
  reason: string
  action: { label: string; intent: 'take-over' | 'claim' } | null
}

/// Renders the runner's lease for one visible pane. Status reads never claim
/// a lease, so a displaced writer stays a viewer until the person acts.
export class TerminalWriter {
  private readonly bar = document.createElement('div')
  private readonly label = document.createElement('strong')
  private readonly reason = document.createElement('span')
  private readonly button = document.createElement('button')
  private runner: Runner | null = null
  private supported = false
  private generation = 0
  private retry = 0
  private retryMs = RETRY_MS
  private readonly pending = new WeakMap<Runner, Promise<void>>()

  constructor(
    host: HTMLElement,
    private readonly terminal: string,
    private readonly setInput: (input: WriterInput) => void,
    private readonly failure: (error: Error) => string | Promise<string>,
    private readonly isConnected: (runner: Runner) => boolean,
  ) {
    this.bar.className = 'terminal-writer'
    this.bar.hidden = true
    this.button.type = 'button'
    const status = document.createElement('span')
    status.className = 'terminal-writer-status'
    status.setAttribute('role', 'status')
    status.append(this.label, this.reason)
    this.bar.append(status, this.button)
    host.prepend(this.bar)
  }

  show(runner: Runner | null): void {
    if (this.runner === runner) return
    const previous = this.runner
    const previousSupported = this.supported
    this.runner = runner
    this.supported = runner?.terminal_writer_supported() ?? false
    this.generation++
    clearTimeout(this.retry)
    this.retryMs = RETRY_MS
    this.bar.hidden = true
    if (runner || previousSupported) {
      // A runner with no lease for this grant answers a write itself, in words.
      this.setInput(!runner ? 'viewing' : this.supported ? 'claiming' : 'writable')
    }
    if (previous && previousSupported && this.isConnected(previous)) {
      this.enqueue(previous, async () => {
        if (!this.isConnected(previous)) return
        try { await previous.terminal_writer(this.terminal, 'release') } catch { /* connection can have ended */ }
      })
    }
    if (runner && this.supported) this.request(runner, 'watch')
  }

  refresh(): void {
    if (this.runner && this.supported) this.request(this.runner, 'status')
  }

  private enqueue(runner: Runner, run: () => Promise<void>): void {
    this.pending.set(runner, (this.pending.get(runner) ?? Promise.resolve()).then(run))
  }

  private request(runner: Runner, action: Action): void {
    const generation = this.generation
    this.enqueue(runner, async () => {
      const current = () => this.generation === generation && this.runner === runner && this.isConnected(runner)
      if (!current()) return
      clearTimeout(this.retry)
      try {
        const view: State | null = JSON.parse(await runner.terminal_writer(this.terminal, action))
        if (!current()) return
        this.retryMs = RETRY_MS
        if (view) this.draw(runner, view)
        else {
          this.supported = false
          this.bar.hidden = true
          this.setInput('writable')
        }
      } catch (e) {
        if (!current()) return
        const reason = await this.failure(e as Error)
        if (!current()) return
        this.draw(runner, { writable: false, label: '', reason, action: null })
        // Nothing else reads a lease that hasn't changed, so one failed read
        // would leave the pane read-only. A takeover is never repeated by itself.
        this.retry = window.setTimeout(() => this.request(runner, action === 'watch' ? 'watch' : 'status'), this.retryMs)
        this.retryMs = Math.min(this.retryMs * 2, RETRY_CEILING_MS)
      }
    })
  }

  private draw(runner: Runner, view: State): void {
    this.setInput(view.writable ? 'writable' : 'viewing')
    if (this.label.textContent !== view.label) this.label.textContent = view.label
    if (this.reason.textContent !== view.reason) this.reason.textContent = view.reason
    this.button.hidden = !view.action
    this.button.textContent = view.action?.label ?? ''
    this.button.disabled = false
    const action = view.action
    this.button.onclick = action ? () => {
      this.button.disabled = true
      this.setInput('claiming')
      this.request(runner, action.intent)
    } : null
    this.bar.hidden = false
  }
}
