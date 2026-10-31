/// An agent's pane with the Mac's view switch (ov-871): its conversation, or
/// the terminal it runs in. The two are one pane of the set (panes.ts), so
/// they share its place in the list, and only the terminal holds a channel:
/// the conversation rides the control session. Switching to the terminal
/// takes a channel as opening any terminal does (the set makes room for it),
/// and switching back gives it up.
///
/// The set drives it through `Pane`; the page switches with `setMode` and then
/// asks the set to open the pane again, which attaches whichever view is in
/// front. What each view is lives behind `Pane`, so a test plays both.

import type { Pane } from './panes'

export type View = 'conversation' | 'terminal'

export class AgentPane<R> implements Pane<R> {
  private view: View = 'conversation'
  private terminal: Pane<R> | null = null
  private front = false

  /// `conversation` is made at once; the terminal's screen is made the first
  /// time it is asked for, since most agents are only ever read.
  constructor(
    private readonly conversation: Pane<R>,
    private readonly makeTerminal: () => Pane<R>,
  ) {}

  get mode(): View {
    return this.view
  }

  /// Holds a channel only while the terminal is in front of it.
  get active(): boolean {
    return this.view === 'terminal' && !!this.terminal?.active
  }

  get used(): boolean {
    return this.conversation.used || !!this.terminal?.used
  }

  show(on: boolean): void {
    this.front = on
    if (this.view === 'conversation') {
      this.terminal?.show(false)
      this.conversation.show(on)
    } else {
      this.conversation.show(false)
      this.terminal?.show(on)
    }
  }

  /// Change the view. Nothing is attached yet: the set's `open` does that,
  /// through `attach`, where a terminal's channel is budgeted for.
  setMode(view: View): void {
    if (view === this.view) return
    if (view === 'terminal') {
      this.conversation.detach()
      this.conversation.show(false)
      this.terminal ??= this.makeTerminal()
    } else {
      this.terminal?.detach()
      this.terminal?.show(false)
    }
    this.view = view
    this.show(this.front)
  }

  async attach(runner: R, terminal: string, ended: () => void, resuming: boolean): Promise<void> {
    if (this.view === 'terminal') {
      this.terminal ??= this.makeTerminal()
      await this.terminal.attach(runner, terminal, ended, resuming && this.terminal.used)
    } else {
      await this.conversation.attach(runner, terminal, ended, resuming)
    }
  }

  detach(): void {
    if (this.view === 'terminal') this.terminal?.detach()
    else this.conversation.detach()
  }

  dispose(): void {
    this.conversation.dispose()
    this.terminal?.dispose()
  }
}
