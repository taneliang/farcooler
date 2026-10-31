/// The size a terminal had before this page reshaped it, to hand back.
///
/// A tmux pane has one size, shared by every screen showing it. Fitting the
/// browser's window sends that size to the runner (`terminal.resize`), and
/// nothing sent the Mac's size back: the Mac only resizes when its own view
/// changes, so a pane opened here stayed browser-shaped on the Mac. The phone
/// apps hand the pane back when they stop showing it (`releasePane` in
/// TerminalSession.swift); this is the same rule.

export interface Size {
  columns: number
  rows: number
}

export class SizeLease {
  private before: Size | null = null
  private reshaped = false

  /// A pane was attached; `before` is the size it had just then, if known.
  begin(before: Size | null): void {
    this.before = before
    this.reshaped = false
  }

  /// This page sent the pane a size.
  reshape(): void {
    this.reshaped = true
  }

  /// The size to give back, once, when this page stops showing the pane: null
  /// when it never reshaped it, or the old size is not known.
  release(): Size | null {
    const back = this.reshaped ? this.before : null
    this.reshaped = false
    return back
  }
}

/// How long after claiming the pane's size a typed key does not claim it
/// again. The runner answers a size it already has at once, but a key a
/// character would otherwise be a round trip a character.
export const CLAIM_EVERY_MS = 2000

/// Whose size the pane has, when the person is using it here. A browser tab
/// left open keeps the size it fitted, so the Mac was garbled until the tab
/// closed; the Mac claims its size when its terminal becomes active, and this
/// page does the same when its pane gains focus or is typed into, so whoever
/// last used the terminal sees it right.
export class Claim {
  private last = Number.NEGATIVE_INFINITY

  /// Whether to send this page's size now. Focus claims always; typing
  /// claims once per `CLAIM_EVERY_MS`.
  due(now: number, focus: boolean): boolean {
    if (!focus && now - this.last < CLAIM_EVERY_MS) return false
    this.last = now
    return true
  }
}
