/// Needs You across every runner this browser is enrolled on (ov-861).
///
/// The Mac and the phones list every enrolled runner's items together, each
/// row naming its runner. The page does the same: one `Board` per runner, each
/// reading its own runner's list over a connection of its own, merged here.
///
/// - **Rows keep each runner's order**, the runners in the order they were
///   enrolled. A row names its runner once there is more than one.
/// - **An answer goes to the runner the item is on.** The merged item's id is
///   namespaced by runner, so two runners that number their items alike never
///   share a busy flag or a refusal, and `press` strips it again and hands the
///   original item to that runner's board.
/// - **A runner that can't be reached is a line of its own.** It never hides
///   the others: their items stay, and the line says which runner and why.
///
/// Nothing here touches the page or wasm, so a test plays the runners by hand.

import { Board, type BoardDeps, type BoardView, type Item, type Press, type RunnerNote } from './needs'

/// What reads and answers for one runner. `list` and the answers reject with
/// `closed` while the runner has no connection.
export type RunnerDeps = Pick<BoardDeps, 'list' | 'ask' | 'decide'>

interface Member {
  id: string
  label: string
  board: Board | null
  view: BoardView | null
  /// Why the runner can't be read right now, and what to offer.
  down: { text: string; retry?: string } | null
  /// Connecting: no read has been tried yet.
  connecting: boolean
}

export class Fleet {
  private readonly members = new Map<string, Member>()
  private dead = false

  constructor(
    private readonly show: (view: BoardView) => void,
    private readonly openTerminal: (runner: string, terminal: string) => void,
    /// Whether a terminal on `runner` is one the page lists.
    private readonly canOpen: (runner: string, terminal: string) => boolean,
  ) {}

  /// A runner the page will read. Its board reads nothing until `refresh`.
  add(id: string, label: string, deps: RunnerDeps): void {
    const member: Member = { id, label, board: null, view: null, down: null, connecting: false }
    member.board = new Board({
      runner: label,
      ...deps,
      open: terminal => this.openTerminal(id, terminal),
      show: view => {
        member.view = view
        this.publish()
      },
    })
    this.members.set(id, member)
  }

  /// A runner this browser can't reach at all (an address-only one), listed
  /// as such. Nothing can be done about it here, so no button.
  addUnreachable(id: string, label: string, text: string): void {
    this.members.set(id, { id, label, board: null, view: null, down: { text }, connecting: false })
  }

  /// The runner is being connected to.
  connecting(id: string): void {
    const m = this.members.get(id)
    if (!m) return
    m.connecting = true
    m.down = null
    this.publish()
  }

  /// The runner is connected: its list is read now.
  up(id: string): void {
    const m = this.members.get(id)
    if (!m) return
    m.connecting = false
    m.down = null
    this.publish()
    void m.board?.refresh()
  }

  /// The runner can't be reached. Its last list, if it had one, stays under
  /// the note.
  unreachable(id: string, text: string, retry?: string): void {
    const m = this.members.get(id)
    if (!m) return
    m.connecting = false
    m.down = { text, retry }
    this.publish()
  }

  /// Read every connected runner's list again.
  async refresh(): Promise<void> {
    await Promise.all([...this.members.values()].filter(m => !m.down && !m.connecting).map(m => m.board?.refresh()))
  }

  /// Press an item's button on the runner the item is on.
  async press(item: Item, press: Press): Promise<void> {
    const m = item.runner ? this.members.get(item.runner) : undefined
    if (!m?.board) return
    await m.board.press({ ...item, id: item.id.slice(m.id.length + 1) }, press)
  }

  stop(): void {
    this.dead = true
    for (const m of this.members.values()) m.board?.stop()
  }

  private publish(): void {
    if (this.dead) return
    const many = this.members.size > 1
    const items: Item[] = []
    const busy = new Set<string>()
    const notices = new Map<string, string>()
    const runners: RunnerNote[] = []
    let ready = false
    let waiting = false
    for (const m of this.members.values()) {
      const key = (id: string) => `${m.id}|${id}`
      const view = m.view
      if (view) {
        if (view.state === 'ready') ready = true
        for (const i of view.items) items.push({ ...i, id: key(i.id), runner: m.id, ...(many ? { runnerName: m.label } : {}) })
        for (const id of view.busy) busy.add(key(id))
        for (const [id, text] of view.notices) notices.set(key(id), text)
      }
      if (m.down) runners.push({ runner: m.id, text: m.down.text, retry: m.down.retry })
      else if (m.connecting) runners.push({ runner: m.id, text: `Connecting to ${m.label}…` })
      else if (view?.banner) runners.push({ runner: m.id, text: view.banner })
      if (!view && !m.down) waiting = true
    }
    this.show({ state: ready ? 'ready' : waiting ? 'loading' : 'unreadable', items, busy, notices, banner: '', runners })
  }

  /// Whether a merged item's Open can go to a terminal this page lists.
  canOpenItem(terminal: string, item: Item): boolean {
    return item.runner !== undefined && this.canOpen(item.runner, terminal)
  }
}
