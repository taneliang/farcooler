import type { WebRunner } from 'fc-web'

type Runner = Pick<WebRunner, 'terminal_writer_supported' | 'terminal_writer'>
type Action = 'claim' | 'status' | 'take-over' | 'release'
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
  private timer = 0
  private readonly pending = new WeakMap<Runner, Promise<void>>()

  constructor(
    host: HTMLElement,
    private readonly terminal: string,
    private readonly setWritable: (writable: boolean) => void,
    private readonly failure: (error: Error) => string | Promise<string>,
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
    this.runner = runner
    clearTimeout(this.timer)
    this.bar.hidden = true
    const supported = runner?.terminal_writer_supported()
    if (runner || previous?.terminal_writer_supported()) this.setWritable(!!runner && !supported)
    if (previous?.terminal_writer_supported()) {
      this.enqueue(previous, async () => {
        try { await previous.terminal_writer(this.terminal, 'release') } catch { /* connection can have ended */ }
      })
    }
    if (runner && supported) this.request(runner, 'claim')
  }

  private enqueue(runner: Runner, run: () => Promise<void>): void {
    this.pending.set(runner, (this.pending.get(runner) ?? Promise.resolve()).then(run))
  }

  private request(runner: Runner, action: Action): void {
    clearTimeout(this.timer)
    this.enqueue(runner, async () => {
      if (this.runner !== runner) return
      try {
        const view: State | null = JSON.parse(await runner.terminal_writer(this.terminal, action))
        if (this.runner !== runner) return
        if (view) this.draw(runner, view)
        else {
          this.bar.hidden = true
          this.setWritable(true)
        }
      } catch (e) {
        const reason = await this.failure(e as Error)
        if (this.runner === runner) this.draw(runner, { writable: false, label: '', reason, action: null })
      } finally {
        if (this.runner === runner && runner.terminal_writer_supported()) {
          clearTimeout(this.timer)
          this.timer = window.setTimeout(() => this.request(runner, 'status'), 1000)
        }
      }
    })
  }

  private draw(runner: Runner, view: State): void {
    this.setWritable(view.writable)
    if (this.label.textContent !== view.label) this.label.textContent = view.label
    if (this.reason.textContent !== view.reason) this.reason.textContent = view.reason
    this.button.hidden = !view.action
    this.button.textContent = view.action?.label ?? ''
    this.button.disabled = false
    const action = view.action
    this.button.onclick = action ? () => {
      this.button.disabled = true
      this.request(runner, action.intent)
    } : null
    this.bar.hidden = false
  }
}
