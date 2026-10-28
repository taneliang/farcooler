/// One pane: xterm.js showing a runner terminal's bytes, and sending what is
/// typed back through the runner's control session.
///
/// The canvas renderer is loaded BEFORE the terminal opens, so xterm never
/// builds its DOM renderer: that renderer writes `<style>` elements and
/// `style` attributes, which this page's CSP (`style-src 'self'`) refuses.
/// Scrollback is xterm's own buffer.

import { CanvasAddon } from '@xterm/addon-canvas'
import { FitAddon } from '@xterm/addon-fit'
import { Terminal } from '@xterm/xterm'
import type { WebPane, WebRunner } from 'fc-web'

import type { Pane } from './panes'
import { paneOutput, Replacement } from './reconnect'

const SCROLLBACK = 5000
/// The most typed before a pane is live that is held for it.
const EARLY_LIMIT = 64 * 1024

export class PaneView implements Pane<WebRunner> {
  private readonly term: Terminal
  private readonly fit = new FitAddon()
  private pane: WebPane | null = null
  /// What was typed while a pane is still attaching, sent in order once it is
  /// (null when nothing is attaching). Typing into a pane that just appeared
  /// used to be dropped without a word until the runner answered the attach.
  private typedEarly: Uint8Array[] | null = null
  private typedEarlyBytes = 0
  private generation = 0
  /// Attached, or attaching: holding a channel, or about to (`Pane.active`).
  private holding = false
  /// Has shown a terminal, so the screen is worth keeping (`Pane.used`).
  private shownBefore = false
  /// A resumed pane keeps showing what it had until the runner's replay
  /// arrives, and is cleared only then (`resume`).
  private replacement = new Replacement(false)
  private resizing = 0
  private readonly encoder = new TextEncoder()
  private readonly watching = new ResizeObserver(() => this.refit())

  constructor(private readonly host: HTMLElement, private readonly onProblem: (word: string) => void) {
    this.term = new Terminal({
      scrollback: SCROLLBACK,
      cursorBlink: true,
      fontFamily: 'ui-monospace, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace',
      fontSize: 13,
      allowProposedApi: false,
    })
    this.term.loadAddon(new CanvasAddon())
    this.term.loadAddon(this.fit)
    this.term.open(host)
    this.term.onData(text => this.send(this.encoder.encode(text)))
    // Bytes that aren't text (some mouse reports): one byte per character.
    this.term.onBinary(text => this.send(Uint8Array.from(text, c => c.charCodeAt(0) & 0xff)))
    this.watching.observe(host)
  }

  get active(): boolean {
    return this.holding
  }

  get used(): boolean {
    return this.shownBefore
  }

  /// Put this pane in front, or out of sight. A pane out of sight keeps its
  /// terminal, its scrollback and its channel; xterm measures nothing while
  /// its host is not displayed, so the size is taken again when it returns.
  show(on: boolean): void {
    this.host.hidden = !on
    if (!on) return
    this.refit()
    this.term.focus()
  }

  /// Take the terminal off the page, with whatever pane it showed.
  dispose(): void {
    this.detach()
    this.watching.disconnect()
    this.term.dispose()
    this.host.remove()
  }

  /// Show `terminal` from `runner`, replacing whatever was shown.
  ///
  /// A pane's stream can still deliver a few bytes after the page moved on,
  /// so each attach has a generation, and only the current one writes.
  ///
  /// The terminal takes focus at once, and what is typed before the runner has
  /// accepted the attach is held and sent first, in order. `data-attached` on
  /// the host says the pane is live.
  async attach(runner: WebRunner, terminal: string, ended: () => void, resuming = false): Promise<void> {
    this.detach()
    this.holding = true
    this.shownBefore = true
    // A fresh pane starts empty. A resumed one (after a dropped connection)
    // keeps its scrollback on screen until the runner's replay arrives, so the
    // person is never looking at a blank terminal, and the replay, which
    // carries the pane's history of its own, replaces it in one step.
    this.replacement = new Replacement(resuming)
    if (!resuming) this.term.reset()
    const generation = ++this.generation
    const replacement = this.replacement
    const current = () => this.generation === generation
    this.typedEarly = []
    this.typedEarlyBytes = 0
    this.term.focus()
    let pane: WebPane
    try {
      pane = await runner.attach(
        terminal,
        paneOutput(replacement, current, () => this.term.reset(), bytes => this.write(bytes)),
        () => {
          if (!current()) return
          // The stream stopped, so no channel is held; a set may attach it
          // again, and a reconnect does.
          this.holding = false
          ended()
        },
      )
    } catch (e) {
      if (current()) {
        this.typedEarly = null
        this.holding = false
      }
      throw e
    }
    if (!current()) {
      pane.close()
      pane.free()
      return
    }
    this.pane = pane
    const early = this.typedEarly ?? []
    this.typedEarly = null
    for (const bytes of early) this.send(bytes)
    this.host.dataset.attached = 'true'
    this.refit(true)
    this.term.focus()
  }

  /// Let the channel go. The terminal keeps what it shows.
  detach(): void {
    this.generation++
    this.holding = false
    this.typedEarly = null
    delete this.host.dataset.attached
    // close() hands the channel's closing to a task of its own, so the wasm
    // object can go at once.
    this.pane?.close()
    this.pane?.free()
    this.pane = null
  }

  /// Output onto the screen. The page's measuring harness
  /// (test/browser/csp-check.mjs) sets `__fcMeasure` to hear when each chunk
  /// has been parsed and whether this pane was in front; nothing else does.
  private write(bytes: Uint8Array): void {
    const measure = (globalThis as { __fcMeasure?: (inFront: boolean) => void }).__fcMeasure
    if (!measure) return void this.term.write(bytes)
    this.term.write(bytes, () => measure(!this.host.hidden))
  }

  private send(bytes: Uint8Array): void {
    if (this.typedEarly) {
      // Held for the pane that's attaching, up to a paste's worth; past that
      // the person is told rather than having keystrokes vanish.
      if (this.typedEarlyBytes + bytes.length > EARLY_LIMIT) return this.onProblem('still_attaching')
      this.typedEarly.push(bytes)
      this.typedEarlyBytes += bytes.length
      return
    }
    // Not awaited between keystrokes: each write is queued in call order
    // before this returns (crates/web, Control::call).
    this.pane?.write(bytes).catch((e: Error) => this.onProblem(e.message))
  }

  /// Fit to the window, and tell the runner the new size once it settles.
  private refit(now = false): void {
    try {
      this.fit.fit()
    } catch {
      return
    }
    clearTimeout(this.resizing)
    const tell = () => this.pane?.resize(this.term.cols, this.term.rows).catch(() => {})
    if (now) tell()
    else this.resizing = window.setTimeout(tell, 150)
  }
}
