/// What leaves the conversation view: a message, and Stop (ov-776). Kept apart
/// from the view so a test can play the runner without a page.
///
/// A message is the person's: it goes out as `terminal.compose`, the call the
/// Mac's composer makes, so every device shows it as a turn the person typed.
/// Stop is `terminal.interrupt`, one Esc pressed by the runner. Words that
/// didn't go stay with the caller, who keeps them in the box.

import type { Picked } from './attachments'
import { sendFailure } from './chat-copy'

/// The calls on a connected runner.
export interface Speaker {
  compose(terminal: string, text: string): Promise<boolean>
  composeImages(terminal: string, text: string, images: { mime: string; data: Uint8Array }[]): Promise<boolean>
  interrupt(terminal: string): Promise<void>
}

export type Outcome =
  /// Taken. `queued` is true when the agent was busy and it waits its turn.
  | { ok: true; queued: boolean }
  /// Not taken, with the sentence to show. Nothing was sent when `ignored`.
  | { ok: false; message: string; ignored?: boolean }

export class Outbox {
  private sending = false

  constructor(
    private readonly runner: () => Speaker | null,
    private readonly terminal: string,
    private readonly label: string,
  ) {}

  get busy(): boolean {
    return this.sending
  }

  /// Send `text`, trimmed, and `images` with it (a message may be only
  /// images). One at a time: a second send while the first is out is ignored,
  /// so a double press can't send twice.
  async send(text: string, images: Picked[] = []): Promise<Outcome> {
    const words = text.trim()
    const runner = this.runner()
    if ((!words && images.length === 0) || this.sending || !runner) return { ok: false, message: '', ignored: true }
    this.sending = true
    try {
      const queued = images.length ? await runner.composeImages(this.terminal, words, images.map(i => ({ mime: i.mime, data: i.data }))) : await runner.compose(this.terminal, words)
      return { ok: true, queued }
    } catch (e) {
      return { ok: false, message: sendFailure((e as Error).message, this.label, 'send', (e as { what?: string }).what ?? '') }
    } finally {
      this.sending = false
    }
  }

  /// Stop the turn the agent is on.
  async stop(): Promise<Outcome> {
    const runner = this.runner()
    if (!runner) return { ok: false, message: '', ignored: true }
    try {
      await runner.interrupt(this.terminal)
      return { ok: true, queued: false }
    } catch (e) {
      return { ok: false, message: sendFailure((e as Error).message, this.label, 'stop') }
    }
  }
}
