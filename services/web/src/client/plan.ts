/// The plan in the browser (ov-775, plan-763 slice 12; ov-872; ov-1016): the
/// runner's plan as every device reads it (`plan_json`, pinned by
/// test/fixtures/plan.json, which is also `farcooler plan --json`), the board
/// that polls it, the edits a person made (a theme's Next line, a ruling), and
/// the words for why a refusal was one.
///
/// Which ticket is in which section, and every word a card says, is not here:
/// crates/client's `plan_view` decides them, and plan-present.ts asks it. What
/// is parsed here is only what the page edits or looks up itself.
///
/// Nothing here touches the page or wasm, so a test runs it with a runner it
/// plays by hand. The DOM is plan-view.ts.

import { planFailure, planRefusal } from './copy'

// MARK: Reading

/// A card on the plan: its key and the task it is.
export interface CardRef {
  task: string
  key: string
}

/// What a card's counts say, as the board's statuses.
export interface Counts {
  backlog: number
  todo: number
  needsDecision: number
  inProgress: number
  inReview: number
  done: number
}

/// A card as the plan read it: for a title and a status the page hasn't.
export interface PlanCard {
  task: string
  key: string
  title: string
  /// The board's status, as `Status`'s word; empty when the plan names none.
  status: string
}

/// What an agent is, in the runner's words.
export type AgentRole = 'orchestrator' | 'builder' | 'reviewer' | 'lander' | 'agent'

/// An agent that still exists on the board: only what the page needs to find
/// the orchestrator's pane. Every word about an agent is the view model's.
export interface PlanAgent {
  id: string
  role: AgentRole
  /// Its pane, to open its chat.
  terminal: string | null
}

export interface Theme {
  id: string
  name: string
  /// Its Next line as the runner has it, for editing.
  next: string
  /// What waits on the owner, for Needs You's row; empty when nothing does.
  ask: string
  /// The tasks of its cards, to say which theme a listed ticket is for.
  tasks: string[]
}

export type RulingState = 'standing' | 'confirmed' | 'reversed' | 'unknown'

export interface Ruling {
  id: string
  short: string
  number: number
  decision: string
  why: string
  reversal: string
  theme: string
  state: RulingState
  note: string
  settledAt: number | null
  reversedSha: string | null
}


export interface Plan {
  nowMs: number
  themes: Theme[]
  rulings: Ruling[]
  /// Whether the runner sent rulings at all (`board_rulings`).
  hasRulings: boolean
  /// Every agent that still exists, the orchestrator included.
  agents: PlanAgent[]
  /// The cards the plan carries, with their titles and statuses.
  cards: PlanCard[]
  /// The board's cards by status; null from a runner that doesn't say.
  boardCounts: Counts | null
}

const text = (v: unknown): string => (typeof v === 'string' ? v : '')
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null)
const list = (v: unknown): any[] => (Array.isArray(v) ? v : [])

const countsOf = (c: any): Counts => ({
  backlog: num(c?.backlog) ?? 0,
  todo: num(c?.todo) ?? 0,
  needsDecision: num(c?.needs_decision) ?? 0,
  inProgress: num(c?.in_progress) ?? 0,
  inReview: num(c?.in_review) ?? 0,
  done: num(c?.done) ?? 0,
})

const AGENT_ROLES = new Set<string>(['orchestrator', 'builder', 'reviewer', 'lander'])
const RULING_STATES = new Set<string>(['standing', 'confirmed', 'reversed'])

/// The plan in the runner's `plan_json`, or null when it isn't one. A row that
/// isn't an object with an id is left out rather than failing the plan, and a
/// missing optional key reads as none.
export function parsePlan(json: string): Plan | null {
  let raw: any
  try {
    raw = JSON.parse(json)
  } catch {
    return null
  }
  if (!raw || typeof raw !== 'object' || !Array.isArray(raw.lines) || !Array.isArray(raw.themes)) return null

  const themes: Theme[] = []
  for (const t of raw.themes) {
    if (!t || typeof t.id !== 'string') continue
    themes.push({ id: t.id, name: text(t.name), next: text(t.next), ask: text(t.owner_ask).trim(), tasks: list(t.cards).map(c => text(c?.task)).filter(Boolean) })
  }

  const rulings: Ruling[] = []
  for (const r of list(raw.rulings)) {
    if (!r || typeof r.id !== 'string' || typeof r.decision !== 'string') continue
    rulings.push({
      id: r.id,
      short: text(r.short) || `R-${num(r.number) ?? 0}`,
      number: num(r.number) ?? 0,
      decision: r.decision,
      why: text(r.why),
      reversal: text(r.reversal),
      theme: text(r.theme),
      state: RULING_STATES.has(r.state) ? r.state : 'unknown',
      note: text(r.note),
      settledAt: num(r.settled_at),
      reversedSha: typeof r.reversed_sha === 'string' && r.reversed_sha ? r.reversed_sha : null,
    })
  }

  const agents: PlanAgent[] = []
  for (const a of list(raw.agents)) {
    if (!a || typeof a.id !== 'string') continue
    agents.push({
      id: a.id,
      role: AGENT_ROLES.has(a.role) ? a.role : 'agent',
      terminal: typeof a.terminal === 'string' && a.terminal ? a.terminal : null,
    })
  }
  const cards: PlanCard[] = list(raw.cards)
    .filter(c => c && typeof c.task === 'string' && c.task)
    .map(c => ({ task: c.task, key: text(c.key), title: text(c.title), status: text(c.status) }))

  return {
    nowMs: num(raw.now_ms) ?? 0,
    themes,
    rulings,
    hasRulings: Array.isArray(raw.rulings),
    agents,
    cards,
    boardCounts: raw.board_counts && typeof raw.board_counts === 'object' ? countsOf(raw.board_counts) : null,
  }
}

// MARK: Rulings

/// Open rulings, newest first: all that Decided For You shows.
export const openRulings = (plan: Plan): Ruling[] => plan.rulings.filter(r => r.state === 'standing').sort((a, b) => b.number - a.number)

/// Kept and reversed ones, most recently settled first: Past Decisions.
export const pastRulings = (plan: Plan): Ruling[] =>
  plan.rulings.filter(r => r.state !== 'standing').sort((a, b) => (b.settledAt ?? 0) - (a.settledAt ?? 0) || b.number - a.number)

// MARK: Words

export const WORDS = {
  decidedForYou: 'Decided For You',
  pastDecisions: 'Past Decisions',
  nothingPlanned: 'Nothing is planned on this board yet.',
  nothingPlannedDetail: 'The orchestrator writes the plan: themes say why a group of cards exists, tickets are the cards lined up to build, and agents work them.',
}

/// A ruling's state: "Open", "Kept", "Reversed", with the commit that
/// reversed it when the orchestrator named one.
export function rulingStateTitle(r: Ruling): string {
  switch (r.state) {
    case 'standing':
      return 'Open'
    case 'confirmed':
      return 'Kept'
    case 'reversed':
      return r.reversedSha ? `Reversed in ${r.reversedSha.slice(0, 8)}` : 'Reversed'
    case 'unknown':
      return 'Unknown'
  }
}

// MARK: The board

/// How often the plan is read while the tab is in front. The runner pushes
/// nothing a browser listens to yet, so this is what shows an edit made on the
/// Mac inside two seconds.
export const PLAN_POLL_MS = 1500

/// The runner's workspaces, as the page needs them.
export interface Workspace {
  id: string
  name: string
  isMain: boolean
}

export function parseWorkspaces(json: string): Workspace[] {
  let raw: any
  try {
    raw = JSON.parse(json)
  } catch {
    return []
  }
  return list(raw)
    .filter(w => w && typeof w.id === 'string')
    .map(w => ({ id: w.id, name: text(w.name) || 'Main', isMain: w.is_main === true }))
}

/// What the screen is showing.
export interface PlanBoardView {
  /// `loading` until the first read lands.
  state: 'loading' | 'ready' | 'old' | 'unreadable'
  plan: Plan | null
  /// The plan as the runner sent it (the view model's input).
  raw: unknown
  workspaces: Workspace[]
  workspace: string | null
  /// The theme whose Next line is being written, and what is typed.
  editing: { theme: string; draft: string } | null
  /// Themes, rulings and "all" being changed right now.
  busy: ReadonlySet<string>
  /// Why a change didn't land, by what it was about (a theme or ruling id, or
  /// `all`).
  notices: ReadonlyMap<string, string>
  /// Said above the plan: a read that failed after a good one, or a runner that
  /// can't show this.
  banner: string
}

/// How a rejection reaches here from wasm: the word as the message, and for a
/// refusal the runner's code and `what`.
export type Failure = Error & { code?: string; what?: string }

export interface PlanDeps {
  runner: string
  /// The workspace to read first (the page's picker), else the main one.
  workspace?: string | null
  workspaces(): Promise<string>
  plan(workspace: string): Promise<string>
  setNext(theme: string, next: string): Promise<void>
  keep(ruling: string): Promise<void>
  keepAll(workspace: string): Promise<number>
  show(view: PlanBoardView): void
}

export class PlanBoard {
  private plan: Plan | null = null
  private raw: unknown = null
  private workspaces: Workspace[] = []
  private workspace: string | null
  private state: PlanBoardView['state'] = 'loading'
  private banner = ''
  private editing: { theme: string; draft: string } | null = null
  private readonly busy = new Set<string>()
  private readonly notices = new Map<string, string>()
  private reading: Promise<void> | null = null
  private dead = false

  constructor(private readonly deps: PlanDeps) {
    this.workspace = deps.workspace ?? null
  }

  /// Read the plan now. The runner's answer replaces what is shown. While a
  /// Next line is being typed the newest plan is kept but not drawn, so a
  /// poll never takes the field from under the person's hands.
  refresh(): Promise<void> {
    if (this.dead) return Promise.resolve()
    this.reading ??= this.read().finally(() => {
      this.reading = null
    })
    return this.reading
  }

  private async read(): Promise<void> {
    try {
      if (this.workspaces.length === 0) {
        this.workspaces = parseWorkspaces(await this.deps.workspaces())
      }
      const chosen = this.workspaces.find(w => w.id === this.workspace) ?? this.workspaces.find(w => w.isMain) ?? this.workspaces[0]
      if (!chosen) {
        this.plan = null
        this.raw = null
        this.state = 'ready'
        this.banner = ''
      } else {
        this.workspace = chosen.id
        const text = await this.deps.plan(chosen.id)
        const plan = parsePlan(text)
        if (!plan) throw new Error('protocol')
        this.plan = plan
        this.raw = JSON.parse(text)
        this.state = 'ready'
        this.banner = ''
        for (const id of [...this.notices.keys()]) if (id !== 'all' && !this.knows(id)) this.notices.delete(id)
      }
    } catch (e) {
      if (this.dead) return
      const word = (e as Error).message
      if (word === 'runner_too_old') {
        this.plan = null
        this.raw = null
        this.state = 'old'
      } else if (this.state !== 'ready') {
        this.state = 'unreadable'
      }
      this.banner = planFailure(word, this.deps.runner)
    }
    if (!this.editing) this.publish()
  }

  private knows(id: string): boolean {
    return !!this.plan && (this.plan.themes.some(t => t.id === id) || this.plan.rulings.some(r => r.id === id))
  }

  /// Pick another board.
  choose(workspace: string): void {
    if (this.dead || workspace === this.workspace) return
    this.workspace = workspace
    this.plan = null
    this.raw = null
    this.state = 'loading'
    this.editing = null
    this.notices.clear()
    this.publish()
    void this.refresh()
  }

  /// Start writing a theme's Next line, from what it says now.
  edit(theme: string): void {
    const current = this.plan?.themes.find(t => t.id === theme)
    if (this.dead || !current || this.busy.has(theme)) return
    this.editing = { theme, draft: current.next }
    this.notices.delete(theme)
    this.publish()
  }

  /// What is typed. Not drawn: the field already shows it.
  type(draft: string): void {
    if (this.editing) this.editing = { ...this.editing, draft }
  }

  cancel(): void {
    if (!this.editing) return
    this.editing = null
    this.publish()
    void this.refresh()
  }

  /// Send the Next line. The plan is read again either way, so a refusal
  /// leaves the screen as the runner has it, with its reason under the line.
  async save(): Promise<void> {
    const editing = this.editing
    if (this.dead || !editing || this.busy.has(editing.theme)) return
    const next = editing.draft.trim()
    if (next === (this.plan?.themes.find(t => t.id === editing.theme)?.next ?? '')) {
      this.cancel()
      return
    }
    await this.change(editing.theme, () => this.deps.setNext(editing.theme, next), 'The Next line')
  }

  async keep(ruling: string): Promise<void> {
    if (this.dead || this.busy.has(ruling)) return
    await this.change(ruling, () => this.deps.keep(ruling), 'That ruling')
  }

  async keepAll(): Promise<void> {
    if (this.dead || !this.workspace || this.busy.has('all')) return
    const workspace = this.workspace
    await this.change('all', () => this.deps.keepAll(workspace), 'The rulings')
  }

  private async change(id: string, send: () => Promise<unknown>, what: string): Promise<void> {
    this.busy.add(id)
    this.notices.delete(id)
    this.publish()
    let landed = true
    try {
      await send()
    } catch (e) {
      landed = false
      const failure = e as Failure
      this.notices.set(id, planRefusal({ word: failure.message, code: failure.code, what: failure.what }, this.deps.runner, what))
    }
    this.busy.delete(id)
    // A refused Next line stays open with what was typed, so it can be fixed
    // or given up; a landed one closes.
    if (landed && this.editing?.theme === id) this.editing = null
    await this.refreshNow()
  }

  private async refreshNow(): Promise<void> {
    const keep = this.editing
    this.editing = null
    await this.refresh()
    this.editing = keep
    this.publish()
  }

  /// Draw what is known again: for what the page adds to the plan.
  redraw(): void {
    if (!this.editing) this.publish()
  }

  stop(): void {
    this.dead = true
  }

  private publish(): void {
    if (this.dead) return
    this.deps.show({
      state: this.state,
      plan: this.plan,
      raw: this.raw,
      workspaces: this.workspaces,
      workspace: this.workspace,
      editing: this.editing,
      busy: new Set(this.busy),
      notices: new Map(this.notices),
      banner: this.banner,
    })
  }
}
