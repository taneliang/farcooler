/// An agent's rows, held the way the apps hold them (ov-776): the runner's
/// projector folds a transcript into rows with ids that never change, and a
/// client pages the newest and then follows what changed after a revision.
/// This is `AgentRowLedger` and `AgentRowStore.run` (apps/shared/AgentKit)
/// without the main-thread machinery a page has no use for, so the browser and
/// the Mac read one stream the same way.
///
/// No DOM and no wasm in this file: a test plays the runner.

/// One row as `projector::Row` serializes it. Every enum is externally tagged:
/// `kind` is `{"Prose": {...}}`.
export interface Row {
  id: string
  /// Its place, never renumbered: a page's cursor.
  ord: number
  /// The revision that last changed it.
  rev: number
  /// The `Turn` row this one belongs to.
  turn: string | null
  provisional: boolean
  kind: Record<string, any>
}

export interface RowPage {
  epoch: number
  rev: number
  moreBefore: boolean
  rows: Row[]
}

export interface RowChange {
  kind: 'insert' | 'update' | 'remove'
  id: string
  rev: number
  row?: Row
}

export interface RowChanges {
  epoch: number
  rev: number
  reset: boolean
  changes: RowChange[]
}

/// The row the composer's placeholder comes from; never drawn in the transcript.
export const HINT_ID = 'hint:composer'

/// How many rows a page asks for, and how long a follow is held when nothing
/// changes: the apps' numbers (`AgentRowStore.pageSize`, `followWaitMs`).
export const PAGE_ROWS = 100
export const FOLLOW_WAIT_MS = 20_000

/// The name of a row's kind (`Prose`, `Tool`, ...), or `Unknown` for a shape
/// this build can't read. A kind it doesn't know is drawn as nothing.
export function kindOf(row: Row): string {
  const names = row.kind && typeof row.kind === 'object' ? Object.keys(row.kind) : []
  return names.length === 1 ? names[0] : 'Unknown'
}

/// The fields of a row's kind.
export function bodyOf(row: Row): any {
  return row.kind?.[kindOf(row)] ?? {}
}

/// What a change to the held rows did, so a view redraws only that.
export interface Delta {
  /// Ids of rows added or changed.
  changed: string[]
  /// Ids of rows taken back.
  removed: string[]
  /// The order of the rows changed (a row came, went or was put above).
  reordered: boolean
}

const none = (): Delta => ({ changed: [], removed: [], reordered: false })
export const isEmpty = (d: Delta) => d.changed.length === 0 && d.removed.length === 0 && !d.reordered

function same(a: Row, b: Row): boolean {
  return a.rev === b.rev && a.id === b.id && JSON.stringify(a.kind) === JSON.stringify(b.kind) && a.provisional === b.provisional
}

export class RowLedger {
  epoch = 0
  rev = 0
  moreBefore = false
  /// Ids, oldest first.
  order: string[] = []
  readonly rows = new Map<string, Row>()
  private paged = false

  /// Where a follow continues from, or null before the first page.
  get cursor(): { epoch: number; rev: number } | null {
    return this.paged ? { epoch: this.epoch, rev: this.rev } : null
  }

  /// The ord of the oldest row held, for the page above it.
  get oldestOrd(): number | null {
    const first = this.order[0]
    return first === undefined ? null : (this.rows.get(first)?.ord ?? null)
  }

  /// `agent.rows`'s newest page. It replaces what is held, except that rows
  /// below it stay when it is the same projection: `ord` never moves, so they
  /// are still where they were.
  page(page: RowPage): Delta {
    const samePlace = page.epoch === this.epoch && this.order.length > 0
    const floor = page.rows[0]?.ord ?? Number.POSITIVE_INFINITY
    const kept = samePlace ? this.order.filter(id => (this.rows.get(id)?.ord ?? 0) < floor) : []
    const next = new Map<string, Row>()
    for (const id of kept) next.set(id, this.rows.get(id) as Row)
    const delta = none()
    for (const row of page.rows) {
      const held = this.rows.get(row.id)
      next.set(row.id, row)
      if (!held || !same(held, row)) delta.changed.push(row.id)
    }
    const order = [...kept, ...page.rows.map(r => r.id)]
    delta.removed = this.order.filter(id => !next.has(id))
    delta.reordered = delta.removed.length > 0 || order.length !== this.order.length || order.some((id, i) => id !== this.order[i])
    const moreBefore = samePlace && kept.length > 0 ? this.moreBefore : page.moreBefore
    this.rows.clear()
    for (const [id, row] of next) this.rows.set(id, row)
    this.order = order
    this.epoch = page.epoch
    this.rev = page.rev
    this.moreBefore = moreBefore
    this.paged = true
    return delta
  }

  /// A page above the oldest row held. One from another projection is
  /// dropped: the follow will reset.
  older(page: RowPage): Delta {
    const delta = none()
    if (page.epoch !== this.epoch) return delta
    const floor = this.oldestOrd ?? Number.POSITIVE_INFINITY
    const fresh = page.rows.filter(r => r.ord < floor && !this.rows.has(r.id))
    this.moreBefore = page.moreBefore
    if (fresh.length === 0) return delta
    for (const row of fresh) this.rows.set(row.id, row)
    this.order = [...fresh.map(r => r.id), ...this.order]
    delta.changed = fresh.map(r => r.id)
    delta.reordered = true
    return delta
  }

  /// `agent.rows_follow`'s answer, applied by id. `null` means the runner
  /// can't say what changed since the cursor (another projection, or too far
  /// behind): page again.
  follow(answer: RowChanges): Delta | null {
    if (answer.reset || answer.epoch !== this.epoch) return null
    const delta = none()
    const appended: Row[] = []
    const newest = this.order.length ? (this.rows.get(this.order[this.order.length - 1])?.ord ?? -1) : -1
    for (const change of answer.changes) {
      if (change.kind === 'remove') {
        if (!this.rows.delete(change.id)) continue
        delta.removed.push(change.id)
        continue
      }
      const row = change.row
      if (!row) continue
      const held = this.rows.get(row.id)
      if (held) {
        if (same(held, row)) continue
        this.rows.set(row.id, row)
        delta.changed.push(row.id)
      } else if (row.ord > newest) {
        // New, below everything held. A row older than the window is someone
        // else's page and isn't drawn yet.
        this.rows.set(row.id, row)
        appended.push(row)
        delta.changed.push(row.id)
      }
    }
    if (delta.removed.length) {
      const gone = new Set(delta.removed)
      this.order = this.order.filter(id => !gone.has(id))
    }
    if (appended.length) {
      appended.sort((a, b) => a.ord - b.ord)
      this.order = [...this.order, ...appended.map(r => r.id)]
    }
    delta.reordered = delta.removed.length > 0 || appended.length > 0
    this.rev = answer.rev
    return delta
  }

  /// The hint's words: what claude's empty box suggests, for the composer's
  /// placeholder. Empty when there are none.
  get hint(): string {
    const row = this.rows.get(HINT_ID)
    return row ? String(bodyOf(row).text ?? '') : ''
  }

  /// The newest turn, which says whether the agent is working.
  get newestTurn(): Row | null {
    for (let i = this.order.length - 1; i >= 0; i--) {
      const row = this.rows.get(this.order[i])
      if (row && kindOf(row) === 'Turn') return row
    }
    return null
  }
}

// MARK: Grouping

/// What a transcript draws: a row, or a run of tool calls folded into one
/// line that opens to them (`ConversationItem`).
export type Item = { kind: 'row'; id: string } | { kind: 'tools'; id: string; rows: string[] }

type Role = 'tool' | 'thinking' | 'other'

function roleOf(row: Row | undefined): Role {
  switch (row ? kindOf(row) : '') {
    case 'Tool':
      return 'tool'
    case 'Thinking':
      return 'thinking'
    default:
      return 'other'
  }
}

/// `order` as items: each run of two or more tool calls, with the thinking
/// between them, is one `tools` item; everything else, a lone tool call
/// included, is a `row`. Text, a message, a subagent, a question or a task
/// list ends a run. Thinking before a run's first call or after its last stays
/// a row of its own (`AgentConversation.items`). The hint is the composer's.
export function itemsOf(order: string[], rows: Map<string, Row>): Item[] {
  const items: Item[] = []
  let run: string[] = []
  let calls = 0
  let trailing: string[] = []
  const close = () => {
    if (calls >= 2 && run.length) items.push({ kind: 'tools', id: `tools:${run[0]}`, rows: run })
    else for (const id of run) items.push({ kind: 'row', id })
    for (const id of trailing) items.push({ kind: 'row', id })
    run = []
    trailing = []
    calls = 0
  }
  for (const id of order) {
    if (id === HINT_ID) continue
    switch (roleOf(rows.get(id))) {
      case 'tool':
        run.push(...trailing)
        trailing = []
        run.push(id)
        calls++
        break
      case 'thinking':
        if (run.length === 0) items.push({ kind: 'row', id })
        else trailing.push(id)
        break
      default:
        close()
        items.push({ kind: 'row', id })
    }
  }
  close()
  return items
}

// MARK: What the view draws

/// One thing in the transcript: an item, or the line that closes a turn.
export type ViewNode =
  | { kind: 'item'; id: string; item: Item }
  /// The note under a finished turn's last row ("Took 1:05", "Interrupted").
  | { kind: 'end'; id: string; turn: string }

/// The transcript's nodes, oldest first: `items`, with a turn's closing line
/// after the last row that belongs to it. A turn that is still open, one
/// claude wrote for itself and a turn that ended with no words get none.
export function nodesOf(items: Item[], rows: Map<string, Row>, closes: (turn: Row) => boolean): ViewNode[] {
  const nodes: ViewNode[] = []
  let open: Row | null = null
  const end = () => {
    if (open && closes(open)) nodes.push({ kind: 'end', id: `end:${open.id}`, turn: open.id })
    open = null
  }
  for (const item of items) {
    const first = rows.get(item.kind === 'row' ? item.id : item.rows[0])
    if (first && kindOf(first) === 'Turn') {
      end()
      open = first
    }
    nodes.push({ kind: 'item', id: item.id, item })
  }
  end()
  return nodes
}

// MARK: Following

/// Where the rows come from: `agent.rows` and `agent.rows_follow` on a runner.
export interface RowSource {
  page(before: number | null, limit: number): Promise<RowPage>
  follow(epoch: number, afterRev: number, waitMs: number): Promise<RowChanges>
}

export type Phase =
  /// Nothing to draw yet.
  | { kind: 'loading' }
  /// Following the runner.
  | { kind: 'live' }
  /// The runner doesn't serve rows (too old), so there is nothing to retry.
  | { kind: 'unavailable' }
  /// The last call failed and is being retried.
  | { kind: 'trouble'; word: string }

export interface Hooks {
  /// Rows changed: draw them.
  changed(delta: Delta): void
  phase(phase: Phase): void
}

export interface Timing {
  /// Resolves after `ms`; a test plays it.
  sleep(ms: number): Promise<void>
  /// The first wait after a failed call, doubled on each failure in a row.
  retryMs: number
  maxRetryMs: number
}

export const REAL_TIMING: Timing = {
  sleep: ms => new Promise(resolve => setTimeout(resolve, ms)),
  retryMs: 500,
  maxRetryMs: 10_000,
}

/// The loop (`AgentRowStore.run`): page the newest, then follow, and page
/// again whenever the runner says it can't diff and after any failure, since a
/// call that failed may have lost changes nobody can name. A change is handed
/// to the page the moment its answer arrives, with no wait of this loop's own
/// between a change and the next call, so a row is drawn as soon as the runner
/// says it changed.
export class RowFollower {
  private generation = 0

  constructor(
    private readonly ledger: RowLedger,
    private readonly source: RowSource,
    private readonly hooks: Hooks,
    private readonly timing: Timing = REAL_TIMING,
  ) {}

  /// Start following; a running follow is replaced, and its answers dropped.
  start(): void {
    const generation = ++this.generation
    this.following = true
    void this.run(generation)
  }

  stop(): void {
    this.generation++
    this.following = false
  }

  /// Started and not stopped. A run that ended on its own (the runner is too
  /// old) is still "following" until stopped: the page decides, not the loop.
  following = false

  /// The page above the oldest row held, once.
  async older(): Promise<void> {
    const before = this.ledger.oldestOrd
    if (!this.ledger.moreBefore || before === null) return
    const generation = this.generation
    const page = await this.source.page(before, PAGE_ROWS)
    if (generation !== this.generation) return
    const delta = this.ledger.older(page)
    this.hooks.changed(delta)
  }

  private async run(generation: number): Promise<void> {
    const current = () => generation === this.generation
    let needsPage = this.ledger.cursor === null
    let waitMs = 0
    let backoff = this.timing.retryMs
    while (current()) {
      try {
        let delta: Delta | null = null
        if (needsPage) {
          const page = await this.source.page(null, PAGE_ROWS)
          if (!current()) return
          delta = this.ledger.page(page)
          needsPage = false
        } else {
          const cursor = this.ledger.cursor as { epoch: number; rev: number }
          const answer = await this.source.follow(cursor.epoch, cursor.rev, waitMs)
          if (!current()) return
          delta = this.ledger.follow(answer)
          if (delta === null) {
            needsPage = true
            continue
          }
        }
        waitMs = FOLLOW_WAIT_MS
        backoff = this.timing.retryMs
        this.hooks.phase({ kind: 'live' })
        if (!isEmpty(delta)) this.hooks.changed(delta)
      } catch (e) {
        if (!current()) return
        const word = (e as Error)?.message ?? 'protocol'
        if (word === 'runner_too_old') {
          this.hooks.phase({ kind: 'unavailable' })
          return
        }
        this.hooks.phase({ kind: 'trouble', word })
        needsPage = true
        await this.timing.sleep(backoff)
        backoff = Math.min(backoff * 2, this.timing.maxRetryMs)
      }
    }
  }
}
