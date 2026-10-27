/// A dead tunnel, and the way back (ov-772, plan-763 slice 9).
///
/// The wasm client gives a connection up after nine silent seconds
/// (`Liveness::TUNNEL`, crates/web/src/liveness.rs) and calls `onLost`. From
/// there this decides what the page does, with every browser call handed in so
/// a test runs it with a clock it moves by hand:
///
/// - **Reconnecting is shown at once.** The old connection is let go and the
///   page says so; nothing waits on the person to find out.
/// - **The tunnel is dialed straight away, the sign-in waits for a click.**
///   The dial needs no touch, so it runs while the notice is up. The SSH login
///   needs the passkey, and Safari prompts for a passkey only inside a click,
///   so the person selects Reconnect and touches once. With the tunnel already
///   up, that is one handshake and one touch, not a dial too.
/// - **A dialed tunnel goes stale.** One held for `PREDIAL_MAX_AGE_MS` is
///   dropped and dialed again at the click, since the runner closes a tunnel
///   whose SSH never starts.
/// - **It never hangs.** A refused or unanswered dial or login ends in a
///   `failed` state with the dial's own word, and Reconnect tries again.
/// - **A tab that comes back asks.** `recheck` (visibility, the browser's
///   `online`) probes the runner right away rather than waiting out the
///   interval; a laptop that slept finds out the moment it wakes.

/// How long a tunnel dialed ahead of the click is kept.
export const PREDIAL_MAX_AGE_MS = 30_000

/// What the page is showing about the connection.
export type LinkView =
  | { kind: 'up' }
  /// Waiting for Reconnect. `tunnel` says whether the dial is already done.
  | { kind: 'reconnecting'; tunnel: 'dialing' | 'ready' | 'failed' }
  /// The click was made; the dial and the login are under way.
  | { kind: 'connecting' }
  /// Reconnect did not work. `word` is the stable failure word.
  | { kind: 'failed'; word: string }

/// The connected runner, as far as this needs it.
export interface Linked {
  check(): Promise<boolean>
  free(): void
}

export interface Deps<R extends Linked, C> {
  /// Dial the tunnel. Rejects with an Error whose message is the dial word.
  dial(): Promise<C>
  /// Close a dialed tunnel that was never used.
  discard(conn: C): void
  /// SSH in over `conn`, signing with the passkey (a touch). `onLost` is
  /// passed to the wasm client, which calls it once if this connection dies.
  connect(conn: C, onLost: () => void): Promise<R>
  /// Put the pane the person was in back on `runner`.
  resume(runner: R): Promise<void>
  show(view: LinkView): void
  now(): number
}

export class Reconnector<R extends Linked, C> {
  private runner: R | null = null
  /// Each connection has a number; a stale one's news is ignored.
  private generation = 0
  /// Runners the connection was lost under, kept here because a freed wasm
  /// object cannot be asked: its `lost` getter throws once it is freed.
  private readonly gone = new WeakSet<object>()
  private view: LinkView = { kind: 'up' }
  private ahead: { at: number; conn: Promise<C | null> } | null = null

  constructor(private readonly deps: Deps<R, C>) {}

  /// Make the first connection (or a later one) and adopt it.
  async open(conn: C): Promise<R> {
    const generation = ++this.generation
    const runner = await this.deps.connect(conn, () => this.lost(generation))
    if (generation !== this.generation) {
      runner.free()
      throw new Error('closed')
    }
    this.runner = runner
    this.set({ kind: 'up' })
    return runner
  }

  get current(): R | null {
    return this.runner
  }

  /// Whether the connection `runner` was on has been lost.
  hasLost(runner: object): boolean {
    return this.gone.has(runner)
  }

  /// Whether the page is showing a working connection.
  get up(): boolean {
    return this.view.kind === 'up'
  }

  private lost(generation: number): void {
    if (generation !== this.generation) return
    if (this.runner) this.gone.add(this.runner)
    this.runner?.free()
    this.runner = null
    this.dialAhead()
    this.set({ kind: 'reconnecting', tunnel: 'dialing' })
  }

  private dialAhead(): void {
    this.dropAhead()
    const at = this.deps.now()
    const conn = this.deps.dial().then(
      conn => {
        if (this.view.kind === 'reconnecting') this.set({ kind: 'reconnecting', tunnel: 'ready' })
        return conn
      },
      () => {
        if (this.view.kind === 'reconnecting') this.set({ kind: 'reconnecting', tunnel: 'failed' })
        return null
      },
    )
    this.ahead = { at, conn }
  }

  private dropAhead(): void {
    const ahead = this.ahead
    this.ahead = null
    void ahead?.conn.then(conn => conn && this.deps.discard(conn))
  }

  /// The tab is visible again, or the browser says it is online.
  async recheck(): Promise<void> {
    const runner = this.runner
    if (!runner || !this.up) return
    // A false answer has already told the wasm client, which calls `lost`.
    await runner.check().catch(() => {})
  }

  /// The Reconnect button. Must be called from the click itself.
  async reconnect(): Promise<void> {
    // The view is the guard: a second click lands on `connecting`.
    if (this.view.kind === 'up' || this.view.kind === 'connecting') return
    this.set({ kind: 'connecting' })
    try {
      const conn = await this.tunnel()
      let runner: R
      try {
        runner = await this.open(conn)
      } catch (e) {
        // A tunnel the login never used must not be left dialed.
        this.deps.discard(conn)
        throw e
      }
      await this.deps.resume(runner)
    } catch (e) {
      this.set({ kind: 'failed', word: (e as Error).message })
    }
  }

  /// The tunnel to sign in over: the one dialed ahead when it is fresh, else
  /// a new one.
  private async tunnel(): Promise<C> {
    const ahead = this.ahead
    this.ahead = null
    if (ahead && this.deps.now() - ahead.at <= PREDIAL_MAX_AGE_MS) {
      const conn = await ahead.conn
      if (conn) return conn
    } else if (ahead) {
      void ahead.conn.then(conn => conn && this.deps.discard(conn))
    }
    return await this.deps.dial()
  }

  private set(view: LinkView): void {
    this.view = view
    this.deps.show(view)
  }
}

/// When a resumed pane clears what it was showing: on the first byte of the
/// runner's replay, once, and never for a pane that was not resumed.
///
/// The replay a runner sends on attach carries the pane's history of its own
/// (crates/daemon/src/resync.rs), so the old scrollback is replaced by the
/// runner's rather than added to; clearing at the first byte, not at the click,
/// keeps the terminal on screen through the reconnect.
export class Replacement {
  constructor(private pending: boolean) {}

  /// True for the first byte of a resumed pane's replay, false after.
  first(): boolean {
    const now = this.pending
    this.pending = false
    return now
  }
}

/// What a pane does with each chunk the runner sends: nothing once the page
/// has moved on, the clear before a resumed pane's first chunk, and the write.
/// One function so the wiring terminal.ts relies on is the one tested.
export function paneOutput(
  replacement: Replacement,
  current: () => boolean,
  reset: () => void,
  write: (bytes: Uint8Array) => void,
): (bytes: Uint8Array) => void {
  return bytes => {
    if (!current()) return
    if (replacement.first()) reset()
    write(bytes)
  }
}
