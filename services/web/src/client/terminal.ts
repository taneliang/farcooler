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

const SCROLLBACK = 5000

export class PaneView {
  private readonly term: Terminal
  private readonly fit = new FitAddon()
  private pane: WebPane | null = null
  private resizing = 0
  private readonly encoder = new TextEncoder()

  constructor(host: HTMLElement, private readonly onProblem: (word: string) => void) {
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
    new ResizeObserver(() => this.refit()).observe(host)
  }

  /// Show `terminal` from `runner`, replacing whatever was shown.
  async attach(runner: WebRunner, terminal: string, ended: () => void): Promise<void> {
    this.detach()
    this.term.reset()
    const pane = await runner.attach(
      terminal,
      bytes => this.term.write(bytes),
      () => {
        if (this.pane === pane) ended()
      },
    )
    this.pane = pane
    this.refit(true)
    this.term.focus()
  }

  detach(): void {
    this.pane?.close()
    this.pane = null
  }

  private send(bytes: Uint8Array): void {
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
