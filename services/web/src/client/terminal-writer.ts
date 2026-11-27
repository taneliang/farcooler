import type { WebRunner, WriterInput } from 'fc-web'

export type { WriterInput } from 'fc-web'
type Runner = Pick<WebRunner, 'terminal_writer_supported' | 'terminal_writer'>
type Action = 'watch' | 'claim' | 'status' | 'take-over' | 'release'
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
    this.bar.hidden = true
    if (runner || previousSupported) {
      this.setInput(this.supported ? 'claiming' : 'viewing')
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
      try {
        const view: State | null = JSON.parse(await runner.terminal_writer(this.terminal, action))
        if (!current()) return
        if (view) this.draw(runner, view)
        else {
          this.supported = false
          this.bar.hidden = true
          this.setInput('writable')
        }
      } catch (e) {
        if (!current()) return
        const reason = await this.failure(e as Error)
        if (current()) this.draw(runner, { writable: false, label: '', reason, action: null })
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
