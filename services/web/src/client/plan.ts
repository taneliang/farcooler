/// The plan in the browser (ov-775, plan-763 slice 12): the runner's trains and
/// lanes, its themes with their Next lines, and Decided For You, as the Mac
/// shows them.
///
/// The plan is the runner's own, in the shape every device reads (`plan_json`,
/// pinned by test/fixtures/plan.json, which is also `farcooler plan --json`).
/// The daemon decides what is live, what is ranked and who may change what.
/// What is here is only what a page does with it: read it, sort it into the
/// Mac's sections with the Mac's words (AgentKit's PlanSections, TrainPhase
/// and PlanRulings), send the edits a person made, and say in words why a
/// refusal was one.
///
/// Nothing here touches the page or wasm, so a test runs it with a runner it
/// plays by hand. The DOM is plan-view.ts.

import { planFailure, planRefusal } from './copy'

// MARK: Reading

export interface Spend {
  cost: number | null
  tokens: number
}

export interface TrainLane {
  id: string
  name: string
}

export interface Train {
  id: string
  name: string
  /// "Train 9", else the name.
  heading: string
  summary: string
  phase: Phase
  phaseSince: number | null
  state: string
  lanes: TrainLane[]
  card: string | null
}

export type LaneState = 'queued' | 'building' | 'review' | 'fixing' | 'landing' | 'landed' | 'dropped' | 'unknown'

export interface Lane {
  id: string
  name: string
  heading: string
  reason: string
  state: LaneState
  stateSince: number | null
  rank: number | null
  train: string | null
  cards: string[]
  stale: boolean
  agent: string | null
}

export interface Theme {
  id: string
  name: string
  outcome: string
  story: string
  next: string
  ask: string
  state: string
  ordinal: number
  cards: number
  done: number
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
  trains: Train[]
  lanes: Lane[]
  /// Lane ids in the plan's ranking.
  order: string[]
  /// Lane ids named like a live train: that train's own agent, left out.
  laneIsTrain: string[]
  themes: Theme[]
  rulings: Ruling[]
  /// Whether the runner sent rulings at all (`board_rulings`).
  hasRulings: boolean
}

const text = (v: unknown): string => (typeof v === 'string' ? v : '')
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null)
const list = (v: unknown): any[] => (Array.isArray(v) ? v : [])

/// A phase of a train, in the runner's words.
export type Phase =
  | 'waiting_for_previous_train'
  | 'waiting_for_main'
  | 'waiting_for_lanes'
  | 'integrating'
  | 'gating_locally'
  | 'waiting_for_ci'
  | 'ci_failed'
  | 'ready_to_land'
  | 'landed'
  | 'stalled'
  | 'dropped'
  | 'unknown'

const PHASES = new Set<string>([
  'waiting_for_previous_train',
  'waiting_for_main',
  'waiting_for_lanes',
  'integrating',
  'gating_locally',
  'waiting_for_ci',
  'ci_failed',
  'ready_to_land',
  'landed',
  'stalled',
  'dropped',
])
const LANE_STATES = new Set<string>(['queued', 'building', 'review', 'fixing', 'landing', 'landed', 'dropped'])
const RULING_STATES = new Set<string>(['standing', 'confirmed', 'reversed'])

/// A train without a phase reads from its state, as AgentKit's
/// `PlanTrain.phase(in:)` does: pushed, red, green, landed and dropped are one
/// each; gating is the local run; integrating is the lanes still gathering.
function phaseOf(raw: any): Phase {
  if (PHASES.has(raw.phase)) return raw.phase
  switch (raw.state) {
    case 'pushed':
      return 'waiting_for_ci'
    case 'red':
      return 'ci_failed'
    case 'green':
      return 'ready_to_land'
    case 'landed':
      return 'landed'
    case 'dropped':
      return 'dropped'
    case 'gating':
      return 'gating_locally'
    case 'integrating':
      return 'integrating'
    default:
      return 'unknown'
  }
}

/// "Train 9" stays "Train 9"; a name is shown as it is.
const heading = (title: string, name: string): string => title || name

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
  if (!raw || typeof raw !== 'object' || !Array.isArray(raw.lanes) || !Array.isArray(raw.themes)) return null

  const lanes: Lane[] = []
  for (const l of raw.lanes) {
    if (!l || typeof l.id !== 'string') continue
    const live = list(l.agents).filter(a => a && a.ended_at == null)
    lanes.push({
      id: l.id,
      name: text(l.name),
      heading: heading(text(l.title), text(l.name)),
      reason: text(l.reason),
      state: LANE_STATES.has(l.state) ? l.state : 'unknown',
      stateSince: num(l.state_since),
      rank: num(l.plan_rank),
      train: typeof l.train === 'string' && l.train ? l.train : null,
      cards: list(l.cards).map(c => text(c?.key)).filter(Boolean),
      stale: l.stale === true,
      agent: live.length ? [text(live[0].role), text(live[0].model)].filter(Boolean).join(' · ') || null : null,
    })
  }

  const trains: Train[] = []
  for (const t of list(raw.trains)) {
    if (!t || typeof t.id !== 'string') continue
    trains.push({
      id: t.id,
      name: text(t.name),
      heading: heading(text(t.title), text(t.name)),
      summary: text(t.summary),
      phase: phaseOf(t),
      phaseSince: num(t.phase_since) ?? num(t.state_since),
      state: text(t.state),
      lanes: list(t.lanes)
        .filter(x => x && typeof x.lane === 'string')
        .map(x => ({ id: x.lane, name: text(x.name) })),
      card: t.card && typeof t.card.key === 'string' ? t.card.key : null,
    })
  }

  const themes: Theme[] = []
  for (const t of raw.themes) {
    if (!t || typeof t.id !== 'string') continue
    const counts = t.counts ?? {}
    themes.push({
      id: t.id,
      name: text(t.name),
      outcome: text(t.outcome),
      story: text(t.story),
      next: text(t.next),
      ask: text(t.owner_ask),
      state: text(t.state),
      ordinal: num(t.ordinal) ?? 0,
      cards: list(t.cards).length,
      done: num(counts.done) ?? 0,
    })
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

  return {
    nowMs: num(raw.now_ms) ?? 0,
    trains,
    lanes,
    order: list(raw.order).filter(x => typeof x === 'string'),
    laneIsTrain: list(raw.lane_is_train).map(x => text(x?.lane)).filter(Boolean),
    themes,
    rulings,
    hasRulings: Array.isArray(raw.rulings),
  }
}

// MARK: Sections

const isLiveTrain = (t: Train): boolean => t.phase !== 'landed' && t.phase !== 'dropped'

export interface Sections {
  landing: { train: Train; lanes: Lane[] }[]
  inReview: Lane[]
  building: Lane[]
  /// Queued lanes the plan ranks, in its order.
  nextUp: Lane[]
  /// Queued lanes it doesn't.
  notRanked: Lane[]
}

/// Which lane is in which section, by AgentKit's PlanSections rules. A lane on
/// a live train is in that train's card whatever its state; one in review,
/// fixing or landing with no live train is In Review; one building is
/// Building; one queued is Next Up when ranked, else Not Ranked. Landed and
/// dropped lanes are history, and a lane named like a live train is that
/// train's own agent (ov-461): left out.
export function sectionsOf(plan: Plan): Sections {
  const live = plan.trains.filter(isLiveTrain)
  const onTrain = (l: Lane): Train | undefined => live.find(t => t.name === l.train || t.lanes.some(x => x.id === l.id))
  const out: Sections = { landing: [], inReview: [], building: [], nextUp: [], notRanked: [] }
  const rest: Lane[] = []
  for (const l of plan.lanes) {
    if (plan.laneIsTrain.includes(l.id)) continue
    if (l.state === 'landed' || l.state === 'dropped') continue
    if (onTrain(l)) continue
    rest.push(l)
  }
  out.landing = live.map(train => ({ train, lanes: plan.lanes.filter(l => !plan.laneIsTrain.includes(l.id) && onTrain(l) === train) }))
  for (const l of rest) {
    if (l.state === 'review' || l.state === 'fixing' || l.state === 'landing') out.inReview.push(l)
    else if (l.state === 'building') out.building.push(l)
    else if (l.state === 'queued') (l.rank !== null || plan.order.includes(l.id) ? out.nextUp : out.notRanked).push(l)
    else out.building.push(l)
  }
  const place = (l: Lane): number => {
    const i = plan.order.indexOf(l.id)
    return i >= 0 ? i : l.rank ?? Number.MAX_SAFE_INTEGER
  }
  out.nextUp.sort((a, b) => place(a) - place(b))
  return out
}

/// The themes still in play, in the plan's order.
export const liveThemes = (plan: Plan): Theme[] => plan.themes.filter(t => t.state !== 'dropped').sort((a, b) => a.ordinal - b.ordinal)

/// Open rulings, newest first: all that Decided For You shows.
export const openRulings = (plan: Plan): Ruling[] => plan.rulings.filter(r => r.state === 'standing').sort((a, b) => b.number - a.number)

/// Kept and reversed ones, most recently settled first: Past Decisions.
export const pastRulings = (plan: Plan): Ruling[] =>
  plan.rulings.filter(r => r.state !== 'standing').sort((a, b) => (b.settledAt ?? 0) - (a.settledAt ?? 0) || b.number - a.number)

// MARK: Words

export const WORDS = {
  landing: 'Landing',
  inReview: 'In Review',
  building: 'Building',
  nextUp: 'Next Up',
  notRanked: 'Not Ranked',
  themes: 'Themes',
  decidedForYou: 'Decided For You',
  pastDecisions: 'Past Decisions',
  nothingLanding: 'Nothing is landing right now.',
  nothingBuilding: 'Nothing is building.',
  nothingQueued: 'Nothing is queued.',
  nothingPlanned: 'Nothing is planned on this board yet.',
  nothingPlannedDetail: 'The orchestrator writes the plan: themes say why a group of cards exists, and lanes are the agents working them.',
}

/// A train's phase in the owner's words (AgentKit's TrainPhase.title): the
/// two local steps are one, "Checking Here".
export function phaseTitle(phase: Phase): string {
  switch (phase) {
    case 'waiting_for_lanes':
      return 'Waiting for Lanes'
    case 'waiting_for_previous_train':
      return 'Waiting for the Earlier Landing'
    case 'waiting_for_main':
      return 'Waiting for Main’s Checks to Pass'
    case 'integrating':
    case 'gating_locally':
      return 'Checking Here'
    case 'waiting_for_ci':
      return 'Checking on CI'
    case 'ci_failed':
      return 'CI Failed'
    case 'ready_to_land':
      return 'Ready to Land'
    case 'landed':
      return 'Landed'
    case 'stalled':
      return 'Stalled'
    case 'dropped':
      return 'Dropped'
    case 'unknown':
      return 'Unknown'
  }
}

/// The ones drawn in amber: CI failed, or nothing has moved.
export const needsAttention = (phase: Phase): boolean => phase === 'ci_failed' || phase === 'stalled'

export function laneStateTitle(state: LaneState): string {
  switch (state) {
    case 'queued':
      return 'Queued'
    case 'building':
      return 'Building'
    case 'review':
      return 'In Review'
    case 'fixing':
      return 'Fixing'
    case 'landing':
      return 'Landing'
    case 'landed':
      return 'Landed'
    case 'dropped':
      return 'Dropped'
    case 'unknown':
      return 'Unknown'
  }
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

/// How long, coarsely: "Just now", "4 min", "3 hr", "2 d"; nothing when the
/// runner didn't say.
export function since(at: number | null, now: number): string {
  if (at === null) return ''
  const minutes = Math.floor(Math.max(0, now - at) / 60_000)
  if (minutes < 1) return 'Just now'
  if (minutes < 60) return `${minutes} min`
  if (minutes < 1440) return `${Math.floor(minutes / 60)} hr`
  return `${Math.floor(minutes / 1440)} d`
}

/// "3 cards, 1 done".
export function cardsLine(theme: Theme): string {
  const cards = `${theme.cards} ${theme.cards === 1 ? 'card' : 'cards'}`
  return theme.done > 0 ? `${cards}, ${theme.done} done` : cards
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
  workspaces(): Promise<string>
  plan(workspace: string): Promise<string>
  setNext(theme: string, next: string): Promise<void>
  keep(ruling: string): Promise<void>
  keepAll(workspace: string): Promise<number>
  show(view: PlanBoardView): void
}

export class PlanBoard {
  private plan: Plan | null = null
  private workspaces: Workspace[] = []
  private workspace: string | null = null
  private state: PlanBoardView['state'] = 'loading'
  private banner = ''
  private editing: { theme: string; draft: string } | null = null
  private readonly busy = new Set<string>()
  private readonly notices = new Map<string, string>()
  private reading: Promise<void> | null = null
  private dead = false

  constructor(private readonly deps: PlanDeps) {}

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
        this.state = 'ready'
        this.banner = ''
      } else {
        this.workspace = chosen.id
        const plan = parsePlan(await this.deps.plan(chosen.id))
        if (!plan) throw new Error('protocol')
        this.plan = plan
        this.state = 'ready'
        this.banner = ''
        for (const id of [...this.notices.keys()]) if (id !== 'all' && !this.knows(id)) this.notices.delete(id)
      }
    } catch (e) {
      if (this.dead) return
      const word = (e as Error).message
      if (word === 'runner_too_old') {
        this.plan = null
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

  stop(): void {
    this.dead = true
  }

  private publish(): void {
    if (this.dead) return
    this.deps.show({
      state: this.state,
      plan: this.plan,
      workspaces: this.workspaces,
      workspace: this.workspace,
      editing: this.editing,
      busy: new Set(this.busy),
      notices: new Map(this.notices),
      banner: this.banner,
    })
  }
}
