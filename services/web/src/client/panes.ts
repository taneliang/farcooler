/// Many panes on one SSH connection (ov-774, plan-763 slice 11).
///
/// A pane's bytes ride an SSH channel of their own, and the runner's sshd
/// allows `MaxSessions` (10) channels on a connection, one of which is the
/// control channel every call and keystroke uses. So at most
/// `MAX_LIVE_PANES` panes are attached at once: nine would fill the
/// connection, and the last of them would be refused with a channel nobody
/// can free; eight leaves one channel of room while a closing channel is
/// still being acknowledged.
///
/// - **A pane stays attached when it is hidden**, so a build running in one
///   keeps scrolling there while another is in front.
/// - **Hidden panes are detached first, least recently shown first**, to make
///   room for the one being opened. A detached pane keeps its screen; going
///   back to it attaches it again, the runner's replay replaces what it
///   showed (`PaneView.resume`), and it is never blank in between.
/// - **A reconnect puts every attached pane back**, the one in front first.
///
/// What a pane is lives behind `Pane`, so a test plays the terminals and
/// counts the channels with no browser.

/// The most panes attached at once on one connection.
export const MAX_LIVE_PANES = 8

/// The part of a pane view the set drives.
export interface Pane<R> {
  /// Attached, or attaching: it holds a channel, or is about to.
  readonly active: boolean
  /// It has shown a terminal before, so its screen is worth keeping.
  readonly used: boolean
  /// Put it in front (true) or out of sight (false).
  show(on: boolean): void
  /// Attach `terminal` from `runner`. `resuming` keeps what the screen shows
  /// until the runner's replay arrives.
  attach(runner: R, terminal: string, ended: () => void, resuming: boolean): Promise<void>
  /// Let the channel go. The screen stays.
  detach(): void
  dispose(): void
}

export class PaneSet<R> {
  private readonly panes = new Map<string, Pane<R>>()
  /// Terminal ids, least recently put in front first.
  private recent: string[] = []
  private front: string | null = null
  /// Panes this set detached to make room: not put back by a reconnect.
  private readonly letGo = new Set<string>()

  constructor(
    private readonly make: (terminal: string) => Pane<R>,
    private readonly max = MAX_LIVE_PANES,
  ) {}

  /// The terminal in front.
  get shown(): string | null {
    return this.front
  }

  /// The terminals holding a channel, or about to.
  get live(): string[] {
    return [...this.panes].filter(([, pane]) => pane.active).map(([id]) => id)
  }

  /// Put `terminal` in front, attached. `ended` is told when its stream stops
  /// while it is in front.
  async open(terminal: string, runner: R, ended: () => void): Promise<void> {
    const before = this.front
    if (before && before !== terminal) this.panes.get(before)?.show(false)
    this.front = terminal
    this.recent = [...this.recent.filter(id => id !== terminal), terminal]
    let pane = this.panes.get(terminal)
    if (!pane) {
      pane = this.make(terminal)
      this.panes.set(terminal, pane)
    }
    pane.show(true)
    if (pane.active) return
    await this.attach(terminal, pane, runner, ended)
  }

  /// A new connection: attach again every pane that was attached, the one in
  /// front first. A pane that can't be (its terminal ended) is left as it is;
  /// the one in front reports why.
  async resume(runner: R, ended: () => void): Promise<void> {
    const ids = [...this.panes.keys()].filter(id => !this.letGo.has(id) && this.panes.get(id)?.used)
    ids.sort((a, b) => Number(b === this.front) - Number(a === this.front))
    for (const id of ids) {
      const pane = this.panes.get(id) as Pane<R>
      try {
        await this.attach(id, pane, runner, ended)
      } catch (e) {
        if (id === this.front) throw e
      }
    }
  }

  dispose(): void {
    for (const pane of this.panes.values()) pane.dispose()
    this.panes.clear()
    this.recent = []
    this.letGo.clear()
    this.front = null
  }

  private async attach(id: string, pane: Pane<R>, runner: R, ended: () => void): Promise<void> {
    this.letGo.delete(id)
    const told = () => {
      if (this.front === id) ended()
    }
    this.makeRoom(id)
    try {
      await pane.attach(runner, id, told, pane.used)
    } catch (e) {
      // A channel the runner refused: every other pane may be holding one it
      // doesn't count (a stream that ended, a channel still closing). Give
      // back the least recent and try once more.
      if ((e as Error).message !== 'exec_failed' || !this.releaseOne(id)) throw e
      await pane.attach(runner, id, told, pane.used)
    }
  }

  /// Detach hidden panes, least recently shown first, until `id` fits.
  private makeRoom(id: string): void {
    while (this.live.filter(other => other !== id).length >= this.max) {
      if (!this.releaseOne(id)) return
    }
  }

  private releaseOne(keep: string): boolean {
    const victim = this.recent.find(id => id !== keep && this.panes.get(id)?.active)
    if (!victim) return false
    this.panes.get(victim)?.detach()
    this.letGo.add(victim)
    return true
  }
}
